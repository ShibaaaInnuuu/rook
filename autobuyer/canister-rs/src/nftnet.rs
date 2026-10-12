//! Buying a listed NFT on another EVM chain (`docs/v4/eth-nft-canister.md`): the canister half of [`crate::seaport`] and
//! [`crate::chains`]. Replicated pinned reads on the chain's own providers, the threshold key, and the per-chain line in
//! stable memory; nothing is decided here.
//!
//! The operator (the relayer) does three things: records a target and its cap ([`nft_set_target`]), hands in an order to be
//! checked ([`nft_preview`]) or checked and signed ([`nft_sign`]), and asks for the saved bytes to be sent and read back
//! ([`nft_send`], [`nft_status`]). It never supplies the destination, the price ceiling at signing time, the calldata sent,
//! or the recipient: those are the target's, the chain's, and ours.

use candid::CandidType;
use ic_cdk::management_canister::{HttpRequestResult, TransformArgs};
use serde::Deserialize;
use std::cell::RefCell;

use crate::canister::{cfg, h0x, hex_bytes, hex_u128, note, only_operator, ours, rpc_once_with, send_raw_once, EvmSeen, EvmSend};
use crate::chains::{EvmChain, InFlight, NftState, NftTarget, ReadBack};
use crate::seaport::{self as sp, Word};

thread_local! {
    /// The per-chain lines and the recorded targets. Saved across upgrades with the rest of the buyer's state.
    pub(crate) static NFT: RefCell<NftState> = RefCell::new(NftState::default());
}

pub(crate) fn chain(id: u64) -> Result<EvmChain, String> {
    cfg().evm_chains.unwrap_or_default().into_iter().find(|c| c.chain_id == id)
        .ok_or_else(|| format!("chain {id} is not one of this canister's evm_chains"))
}

/// Decimal text → a uint256 word. Token ids are often wider than any integer type (hashes, packed ids).
pub fn word_of_dec(s: &str) -> Result<Word, String> {
    let s = s.trim();
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!("{s:?} is not a decimal number"));
    }
    let mut w = [0u8; 32];
    for d in s.bytes() {
        let mut carry = (d - b'0') as u32;
        for byte in w.iter_mut().rev() {
            let v = *byte as u32 * 10 + carry;
            *byte = v as u8;
            carry = v >> 8;
        }
        if carry != 0 {
            return Err(format!("{s} does not fit in 256 bits"));
        }
    }
    Ok(w)
}

// ---- reads on one chain: every provider, replicated, and they must agree -----------------------------------------------------

pub(crate) async fn ask(c: &EvmChain, method: &str, params: serde_json::Value, transform: &str, context: Vec<u8>, max: u64)
    -> Result<Vec<serde_json::Value>, String> {
    let body = serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }).to_string();
    let mut out = Vec::new();
    for url in &c.rpc_urls {
        out.push(rpc_once_with(url, body.clone(), transform, context.clone(), max).await
            .map_err(|e| format!("chain {}: {method}: {e}", c.chain_id))?);
    }
    Ok(out)
}

/// Review round 7 (B7-L-1): `ask` for the sale scan's log reads, where a refusal's CLASS steers the walk. A provider that refuses a
/// read as deeper than it keeps logs no longer decides that alone: the others are asked too, and the read is `deep` only when every
/// provider says so (`pair_deep`). Any other refusal ends the read at once, as `ask` does.
pub(crate) async fn ask_scan(c: &EvmChain, method: &str, params: serde_json::Value, transform: &str, context: Vec<u8>, max: u64)
    -> Result<Vec<serde_json::Value>, String> {
    let body = serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }).to_string();
    let mut answers = Vec::new();
    for url in &c.rpc_urls {
        let a = rpc_once_with(url, body.clone(), transform, context.clone(), max).await.map_err(|e| format!("chain {}: {method}: {e}", c.chain_id));
        let stop = matches!(&a, Err(e) if crate::ethflow::scan_error(e) != crate::ethflow::ScanErr::Deep);
        answers.push(a);
        if stop {
            break;
        }
    }
    pair_deep(answers, c.chain_id, method)
}

/// Review round 7 (B7-L-1): the providers' answers to one scan read, as one: every answer, or the first refusal that is not `deep`,
/// or — when some providers refused it as too deep and others served it — a refusal with NO class (read again; deferred only if it
/// persists), never `deep` on one provider's word. `deep` only when every provider said it.
pub fn pair_deep(answers: Vec<Result<serde_json::Value, String>>, chain: u64, method: &str) -> Result<Vec<serde_json::Value>, String> {
    use crate::ethflow::{scan_error, ScanErr};
    if let Some(e) = answers.iter().find_map(|a| a.as_ref().err().filter(|e| scan_error(e) != ScanErr::Deep)) {
        return Err(e.clone());
    }
    let deep = answers.iter().filter(|a| a.is_err()).count();
    if deep == 0 {
        return Ok(answers.into_iter().filter_map(Result::ok).collect());
    }
    if deep == answers.len() {
        return Err(answers.into_iter().find_map(Result::err).expect("an error"));
    }
    Err(format!("chain {chain}: {method}: {deep} of {} providers refused these logs as deeper than they keep and the others served them: \
        read again, never given up on one provider's word (B7-L-1)", answers.len()))
}

pub(crate) fn one(c: &EvmChain, method: &str, answers: Vec<serde_json::Value>) -> Result<serde_json::Value, String> {
    let first = answers.first().cloned().ok_or("no providers")?;
    if answers.iter().any(|a| *a != first) {
        return Err(format!("chain {}: providers disagree on {method}; nothing is believed", c.chain_id));
    }
    Ok(first)
}

pub(crate) async fn agreed(c: &EvmChain, method: &str, params: serde_json::Value, max: u64) -> Result<serde_json::Value, String> {
    one(c, method, ask(c, method, params, "json_rpc_norm", vec![], max).await?)
}

/// `eth_getBlockByNumber`, projected to `{number, hash, baseFeePerGas, timestamp}`.
#[ic_cdk::query]
fn nft_block_at(args: TransformArgs) -> HttpRequestResult {
    HttpRequestResult { status: args.response.status, body: crate::chains::block_body_ts(&args.response.body), headers: vec![] }
}

/// Go-live prep (2026-09-28, the IC-outcall rehearsal, ops/rehearsal/ic-outcall/): the IC holds `max_response_bytes` to the provider's
/// RAW body, before the transform. An Ethereum block with transaction hashes only (`eth_getBlockByNumber(n, false)`) is 11–51 kB today
/// (measured read-only on both providers of the pair: 19–33 kB over the pin spacing, the finalized block 24 kB), so the 16 kB cap made
/// the pin and the finality read fail on almost every tick ("Http body exceeds size limit of 16000 bytes", reproduced with buyer.wasm on
/// a replica making real outcalls). A full 45 M-gas block of plain transfers is ≈ 150 kB; 128 kB covers every block seen by a wide margin
/// and a rarer one is a failed read, retried next tick. Other chains keep 16 kB (a 4663 block is ≈ 4.3 kB): the reservation is the
/// outcall's price.
pub const ETH_BLOCK_VIEW_BYTES: u64 = 128_000;
pub const BLOCK_VIEW_BYTES: u64 = 16_000;

/// The response cap of an `eth_getBlockByNumber(_, false)` read on `chain_id`.
pub fn block_view_bytes(chain_id: u64) -> u64 {
    if chain_id == 1 { ETH_BLOCK_VIEW_BYTES } else { BLOCK_VIEW_BYTES }
}

/// The chain is the one configured, and the pin: docs/v2/pinned-reads.md with this chain's bucket, plus its clock.
pub(crate) async fn pin_on(c: &EvmChain) -> Result<(crate::pin::Pin, u64), String> {
    use crate::pin::{choose_pin, context_u64};
    let id = u64::from_str_radix(agreed(c, "eth_chainId", serde_json::json!([]), 8_000).await?.as_str().unwrap_or("")
        .trim_start_matches("0x"), 16).map_err(|_| "unreadable chain id")?;
    if id != c.chain_id {
        return Err(format!("refused: these providers are chain {id}, not {}", c.chain_id));
    }
    let heads = ask(c, "eth_blockNumber", serde_json::json!([]), "head_bucket", context_u64(c.bucket), 8_000).await?;
    let rounded = heads.iter().map(|h| hex_u128(h).map(|n| n as u64)).collect::<Result<Vec<_>, _>>()?;
    let n = choose_pin(&rounded, c.bucket)?;
    let views = ask(c, "eth_getBlockByNumber", serde_json::json!([format!("0x{n:x}"), false]), "nft_block_at", vec![],
        block_view_bytes(c.chain_id)).await?;
    crate::chains::agree_block_ts(&views, n)
}

pub(crate) async fn call_at(c: &EvmChain, to: &[u8; 20], data: &[u8], pin: &crate::pin::Pin) -> Result<Vec<u8>, String> {
    hex_bytes(&agreed(c, "eth_call", serde_json::json!([{ "to": h0x(to), "data": h0x(data) }, crate::pin::at(pin)]), 16_000).await?)
}

pub(crate) async fn quantity_at(c: &EvmChain, method: &str, who: &[u8; 20], pin: &crate::pin::Pin) -> Result<u128, String> {
    hex_u128(&agreed(c, method, serde_json::json!([h0x(who), crate::pin::at(pin)]), 8_000).await?)
}

// ---- the check -----------------------------------------------------------------------------------------------------------------

/// What a checked order would do. Every field is public.
#[derive(CandidType, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct NftPlan {
    pub chain_id: u64,
    pub pin: u64,
    pub pin_timestamp: u64,
    pub order_hash: String,
    /// Whether the signature was recovered here (an EOA) or left to the simulation (a contract or bulk signature, or none).
    pub signature_checked_here: bool,
    pub total_wei: String,
    pub to: String,
    pub calldata: String,
    pub from: String,
    pub nonce: u64,
    pub max_fee_wei: String,
    pub priority_fee_wei: String,
    pub gas: u64,
    /// Set by [`nft_sign`]: the saved transaction.
    pub tx_hash: Option<String>,
    pub raw: Option<String>,
}

/// The id an NFT is held under in the custody cap book (H2): `<chain id>:0x<collection>:<token id as a 32-byte hex word>`, so
/// one token has one id however its decimal was written.
pub fn nft_item_id(chain_id: u64, collection: &[u8; 20], token_id: &str) -> Result<String, String> {
    Ok(format!("{}:{}", crate::custodycap::evm_key(chain_id, collection), hex::encode(word_of_dec(token_id)?)))
}

fn target_of(chain_id: u64, collection: &str, token_id: &str) -> Result<sp::Target, String> {
    let collection = crate::evmtx::parse_address(collection)?;
    let token_id = word_of_dec(token_id)?;
    let t = NFT.with(|n| n.borrow().targets.iter().find(|t| t.chain_id == chain_id
        && crate::evmtx::parse_address(&t.collection).ok() == Some(collection)
        && word_of_dec(&t.token_id).ok() == Some(token_id)).cloned())
        .ok_or("no target is recorded for that token on that chain: record it, with its cap, before handing in an order")?;
    Ok(sp::Target { collection, token_id, cap_wei: t.cap_wei.parse().map_err(|_| "unreadable cap")? })
}

/// What the relayer handed in: a basic order (open listings, §6) or an advanced one (restricted, through an allow-listed
/// SignedZone, §12). Told apart by selector; anything else is refused by the basic decoder.
enum Handed {
    Basic(sp::BasicOrder),
    Advanced(sp::AdvancedOrder),
}

impl Handed {
    fn parse(order_hex: &str) -> Result<Handed, String> {
        let b = hex::decode(order_hex.trim().trim_start_matches("0x")).map_err(|e| format!("the order is not hex: {e}"))?;
        if b.starts_with(&sp::FULFILL_ADVANCED_ORDER) { Ok(Handed::Advanced(sp::decode_advanced(&b)?)) } else { Ok(Handed::Basic(sp::decode(&b)?)) }
    }
    /// (offerer, collection, token id, what it would pay in total — for the royalty query only; the verifier computes the real one)
    fn outline(&self) -> ([u8; 20], [u8; 20], Word, u128) {
        match self {
            Handed::Basic(o) => (o.offerer, o.offer_token, o.offer_identifier, o.total().unwrap_or(0)),
            Handed::Advanced(o) => {
                let (tok, id) = o.order.offer.first().map(|i| (i.token, i.identifier)).unwrap_or(([0; 20], [0; 32]));
                let total = o.order.consideration.iter().map(|i| u128::from_be_bytes(i.start[16..].try_into().expect("16"))).fold(0u128, u128::saturating_add);
                (o.order.offerer, tok, id, total)
            }
        }
    }
}

/// Phase 1 (docs/v7/phase1-nft.md §4.3): check a handed-in order for any token of `collection` at most `cap_wei` — the target is
/// the order's own token, the cap the caller's, never the order's.
pub(crate) async fn check_any(c: &EvmChain, collection: [u8; 20], cap_wei: u128, order_hex: &str) -> Result<(NftPlan, Word), String> {
    let (_, coll, token, _) = Handed::parse(order_hex)?.outline();
    if coll != collection {
        return Err(format!("refused (wrong collection): the order sells 0x{}", hex::encode(coll)));
    }
    let t = sp::Target { collection, token_id: token, cap_wei };
    Ok((check(c, &t, order_hex).await?, token))
}

/// Every rule of §6 (and §12 for a restricted order), at one pin: the chain's facts first, then the pure verifier, then the
/// simulation of the exact transaction.
pub(crate) async fn check(c: &EvmChain, t: &sp::Target, order_hex: &str) -> Result<NftPlan, String> {
    let handed = Handed::parse(order_hex)?;
    let (offerer, offer_token, offer_identifier, total_for_royalty) = handed.outline();
    let seaport = c.seaport_addr()?;
    let (pin, ts) = pin_on(c).await?;
    // rule 1: the canonical Seaport, with this chain's code and this chain's domain
    let code = hex_bytes(&agreed(c, "eth_getCode", serde_json::json!([h0x(&seaport), crate::pin::at(&pin)]), 120_000).await?)?;
    if crate::evmtx::keccak(&code) != c.codehash()? {
        return Err(format!("refused (Seaport): the code at {} on chain {} is not the configured code", c.seaport, c.chain_id));
    }
    let (version, domain) = sp::decode_information(&call_at(c, &seaport, &sp::INFORMATION, &pin).await?)?;
    if version != "1.6" || domain != sp::domain_separator(c.chain_id, &seaport) {
        return Err(format!("refused (Seaport): information() says version {version:?} and a domain this canister did not compute"));
    }
    let us = ours().await?.0;
    // rule 6's royalty receiver: ERC-2981 at the pin, if the collection has it. A revert or an unreadable answer is "none",
    // which only makes the check stricter.
    let royalty_call = [&sp::ROYALTY_INFO[..], &offer_identifier, &sp::word_u128(total_for_royalty)].concat();
    let royalty_receiver = match call_at(c, &offer_token, &royalty_call, &pin).await {
        Ok(ret) => sp::decode_address(&ret).ok().filter(|a| *a != [0; 20]),
        Err(_) => None,
    };
    let facts = sp::ChainFacts { ours: us, pin_timestamp: ts, royalty_receiver };
    // rules 2–7 (and §12.3), then the order hash with the offerer's counter at the pin
    let counter = call_at(c, &seaport, &sp::call1(sp::GET_COUNTER, sp::word_addr(&offerer)), &pin).await?;
    let counter: Word = counter.get(..32).ok_or("getCounter returned nothing")?.try_into().expect("32 bytes");
    let (checked, order_hash, signature) = match &handed {
        Handed::Basic(o) => (sp::verify(o, t, &c.rules()?, &facts)?, o.components(counter).hash(), o.signature.clone()),
        Handed::Advanced(o) => {
            let (zones, zr) = c.zones()?;
            // the zone is pinned by its code on this chain, like Seaport
            let (zone, want) = zones.iter().find(|(z, _)| *z == o.order.zone).copied()
                .ok_or_else(|| format!("refused (zone): 0x{} is not an allow-listed SignedZone on this chain", hex::encode(o.order.zone)))?;
            let zcode = hex_bytes(&agreed(c, "eth_getCode", serde_json::json!([h0x(&zone), crate::pin::at(&pin)]), 64_000).await?)?;
            if crate::evmtx::keccak(&zcode) != want {
                return Err(format!("refused (zone): the code at 0x{} on chain {} is not the configured code", hex::encode(zone), c.chain_id));
            }
            let now = ic_cdk::api::time() / 1_000_000_000;
            let (checked, _) = sp::verify_advanced(o, t, &c.rules()?, &zr, &facts, now)?;
            (checked, sp::OrderComponents { counter, ..o.order.clone() }.hash(), o.signature.clone())
        }
    };
    // rule 8: still fillable at the pin
    let (_, cancelled, filled, size) = sp::decode_order_status(&call_at(c, &seaport, &sp::call1(sp::GET_ORDER_STATUS, order_hash), &pin).await?)?;
    if cancelled {
        return Err("refused (cancelled): Seaport records this order as cancelled".into());
    }
    if size > 0 && filled >= size {
        return Err("refused (filled): Seaport records this order as filled".into());
    }
    let owner = sp::decode_address(&call_at(c, &offer_token, &sp::call1(sp::OWNER_OF, offer_identifier), &pin).await?)?;
    if owner != offerer {
        return Err(format!("refused (owner): the token's owner at the pin is 0x{}, not the offerer", hex::encode(owner)));
    }
    // rule 9: the offerer signed it (an EOA or a bulk order), where that can be checked without the chain
    let signature_checked_here = sp::offerer_signed(&order_hash, &offerer, &signature, c.chain_id, &seaport)?;
    // the simulation: the exact transaction, from us, at the pin
    let tx = serde_json::json!({ "from": h0x(&us), "to": h0x(&seaport), "value": format!("0x{:x}", checked.total_wei),
        "data": h0x(&checked.calldata) });
    agreed(c, "eth_call", serde_json::json!([tx.clone(), crate::pin::at(&pin)]), 16_000).await
        .map_err(|e| format!("refused (simulation): the fill reverts at the pin: {e}"))?;
    let mut estimate = 0u128;
    for a in ask(c, "eth_estimateGas", serde_json::json!([tx, format!("0x{:x}", pin.number)]), "json_rpc_norm", vec![], 8_000).await? {
        estimate = estimate.max(hex_u128(&a)?);
    }
    let (max_fee, prio, gas) = crate::chains::fees_and_gas(c, pin.base_fee, estimate as u64)?;
    let balance = quantity_at(c, "eth_getBalance", &us, &pin).await?;
    let need = checked.total_wei.saturating_add(max_fee.saturating_mul(gas as u128));
    if balance < need {
        return Err(format!("refused (balance): {balance} wei at the pin; the fill needs up to {need}"));
    }
    let nonce = quantity_at(c, "eth_getTransactionCount", &us, &pin).await? as u64;
    Ok(NftPlan { chain_id: c.chain_id, pin: pin.number, pin_timestamp: ts, order_hash: h0x(&order_hash), signature_checked_here,
        total_wei: checked.total_wei.to_string(), to: h0x(&seaport), calldata: h0x(&checked.calldata), from: h0x(&us), nonce,
        max_fee_wei: max_fee.to_string(), priority_fee_wei: prio.to_string(), gas, tx_hash: None, raw: None })
}

// ---- the operator's entry points -----------------------------------------------------------------------------------------------

/// Record the one token to buy on a chain and the most to pay for it, before any order exists. The cap is the keeper's (or,
/// once B's registry exists, the registry's); an order never sets it. Replaces an earlier target for the same token.
#[ic_cdk::update]
pub fn nft_set_target(chain_id: u64, collection: String, token_id: String, cap_wei: candid::Nat) -> Result<String, String> {
    only_operator();
    crate::canister::not_paused()?;
    chain(chain_id)?;
    let cap: u128 = cap_wei.0.to_string().parse().map_err(|_| "the cap does not fit in 128 bits")?;
    let t = crate::canister::with_caps(|p, _| NFT.with(|n| record_target(&mut n.borrow_mut(), p, chain_id, &collection, &token_id, cap)))?;
    note(format!("nft target: chain {chain_id} {} #{} cap {cap} wei", t.collection, t.token_id));
    Ok(format!("recorded: chain {chain_id} {} #{} at most {cap} wei", t.collection, t.token_id))
}

/// Record the one token to buy on a chain and its cap, replacing an earlier target for the same token. H2
/// (docs/v5/custody-hardening.md): the collection must be one custody may hold, and the cap within its per-item maximum, so
/// the operator's NFT drain (T5: a junk collection, or any cap up to the balance) is refused here, before any order exists.
pub fn record_target(n: &mut NftState, caps: &crate::custodycap::CapPolicy, chain_id: u64, collection: &str, token_id: &str, cap: u128)
    -> Result<NftTarget, String> {
    let c = crate::evmtx::parse_address(collection)?;
    let id = word_of_dec(token_id)?;
    crate::custodycap::check_target_cap(caps, &crate::custodycap::evm_key(chain_id, &c), cap)
        .map_err(|e| format!("refused (custody cap): {e:?}"))?;
    let t = NftTarget { chain_id, collection: h0x(&c), token_id: token_id.trim().to_string(), cap_wei: cap.to_string() };
    n.targets.retain(|x| !(x.chain_id == chain_id && crate::evmtx::parse_address(&x.collection).ok() == Some(c)
        && word_of_dec(&x.token_id).ok() == Some(id)));
    n.targets.push(t.clone());
    Ok(t)
}

/// Check a handed-in order against the recorded target and the chain, and say what would be signed. Signs nothing.
#[ic_cdk::update]
pub async fn nft_preview(chain_id: u64, collection: String, token_id: String, order_hex: String) -> Result<NftPlan, String> {
    only_operator();
    let c = chain(chain_id)?;
    let t = target_of(chain_id, &collection, &token_id)?;
    check(&c, &t, &order_hex).await
}

/// Check again, then build, sign and **save** the transaction. Nothing is sent: [`nft_send`] sends the saved bytes.
#[ic_cdk::update]
pub async fn nft_sign(chain_id: u64, collection: String, token_id: String, order_hex: String) -> Result<NftPlan, String> {
    only_operator();
    crate::canister::not_paused()?;
    let c = chain(chain_id)?;
    if crate::canister::cfg().eth_nft.as_ref().is_some_and(|e| e.chains().contains(&chain_id)) {
        return Err(format!("refused: chain {chain_id}'s line belongs to the Ethereum NFT side (eth_nft): one writer per nonce line"));
    }
    NFT.with(|n| crate::chains::may_sign(&n.borrow().line(chain_id)))?;
    let t = target_of(chain_id, &collection, &token_id)?;
    let mut plan = check(&c, &t, &order_hex).await?;
    // H3 and H2, before the signing await: the fill's value against the day's limit, then room under the caps, reserved
    let key = crate::custodycap::evm_key(chain_id, &t.collection);
    let total: u128 = plan.total_wei.parse().map_err(|_| "an unreadable total")?;
    let fill = format!("fill:{}", plan.order_hash);
    crate::canister::outflow_check(&fill, &key, crate::outflow::Flow::Value, total)?;
    let flight = format!("nft:{chain_id}:{}", plan.order_hash);
    crate::canister::with_caps(|p, b| crate::custodycap::reserve(p, b, &flight, &key, total))
        .map_err(|e| format!("refused (custody cap): {e:?}"))?;
    let signed = sign_and_save(&c, chain_id, &collection, &token_id, &mut plan).await;
    if signed.is_err() {
        // nothing was saved: the room is free again
        let _ = crate::canister::with_caps(|_, b| crate::custodycap::settle(b, &flight, None));
    } else {
        crate::canister::outflow_record(&fill, &key, crate::outflow::Flow::Value, total);
    }
    signed.map(|_| plan)
}

async fn sign_and_save(c: &EvmChain, chain_id: u64, collection: &str, token_id: &str, plan: &mut NftPlan) -> Result<(), String> {
    let tx = crate::evmtx::Tx1559 {
        chain_id, nonce: plan.nonce, max_priority_fee: plan.priority_fee_wei.parse().expect("ours"),
        max_fee: plan.max_fee_wei.parse().expect("ours"), gas: plan.gas, to: c.seaport_addr()?,
        value: plan.total_wei.parse().expect("ours"), data: hex_bytes(&serde_json::json!(plan.calldata))?,
    };
    let s = crate::canister::sign_evm_recoverable(tx.signing_hash()).await?;
    let b = |h: &str| -> Result<[u8; 32], String> { hex::decode(h).map_err(|e| e.to_string())?.try_into().map_err(|_| "not 32 bytes".into()) };
    let raw = tx.signed(s.recovery_id, &b(&s.r)?, &b(&s.s)?);
    let tx_hash = h0x(&crate::evmtx::keccak(&raw));
    // saved BEFORE anything is sent, and the line re-checked: an await above may have let another sign in
    NFT.with(|n| -> Result<(), String> {
        let mut n = n.borrow_mut();
        crate::chains::may_sign(&n.line(chain_id))?;
        n.line_mut(chain_id).in_flight = Some(InFlight { nonce: plan.nonce, tx_hash: tx_hash.clone(), raw: raw.clone(),
            collection: collection.to_string(), token_id: token_id.to_string(), total_wei: plan.total_wei.clone(),
            order_hash: plan.order_hash.clone() });
        Ok(())
    })?;
    note(format!("nft signed: chain {chain_id} {tx_hash} nonce {} for {} wei", plan.nonce, plan.total_wei));
    plan.tx_hash = Some(tx_hash);
    plan.raw = Some(h0x(&raw));
    Ok(())
}

/// The read-back on one chain: a receipt at or below the pin, every provider agreeing.
pub(crate) async fn seen_on(c: &EvmChain, hash: &str, pin: u64) -> EvmSeen {
    let r = match ask(c, "eth_getTransactionReceipt", serde_json::json!([hash]), "receipt_at_pin", crate::pin::context_u64(pin), 64_000)
        .await.and_then(|rs| one(c, "eth_getTransactionReceipt", rs)) {
        Ok(r) => r,
        Err(e) => return EvmSeen::Unsure(e),
    };
    if r.is_null() {
        return EvmSeen::NotYet;
    }
    let hex = |k: &str| r.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    match u64::from_str_radix(hex("blockNumber").trim_start_matches("0x"), 16) {
        Ok(block) => EvmSeen::Mined { block, success: hex("status") == "0x1", from: hex("from").to_ascii_lowercase() },
        Err(_) => EvmSeen::Unsure("a receipt without a readable blockNumber".into()),
    }
}

/// Send the saved bytes to every provider of the chain (one node each; replies are reports), then read the chain back.
/// Safe to repeat: one nonce, at most one of them is ever mined.
#[ic_cdk::update]
pub async fn nft_send(chain_id: u64) -> Result<EvmSend, String> {
    only_operator();
    crate::canister::not_paused()?;
    let c = chain(chain_id)?;
    let f = NFT.with(|n| n.borrow().line(chain_id).in_flight).ok_or("nothing is signed on this chain")?;
    let signed_for = crate::evmsend::chain_id_of(&f.raw)?;
    if signed_for != chain_id {
        return Err(format!("refused: the saved bytes are signed for chain {signed_for}"));
    }
    let raw_hex = h0x(&f.raw);
    let mut providers = Vec::new();
    for url in &c.rpc_urls {
        providers.push((url.clone(), send_raw_once(url, &raw_hex, &f.tx_hash).await));
    }
    let seen = match pin_on(&c).await { Ok((p, _)) => seen_on(&c, &f.tx_hash, p.number).await, Err(e) => EvmSeen::Unsure(e) };
    Ok(EvmSend { tx_hash: f.tx_hash, providers, seen })
}

/// Read back the purchase in flight on a chain and settle its line: arrived (mined, and `ownerOf` at the pin is us),
/// reverted, superseded, or still waiting.
#[ic_cdk::update]
pub async fn nft_status(chain_id: u64) -> Result<String, String> {
    only_operator();
    crate::canister::not_paused()?;
    let c = chain(chain_id)?;
    let Some(f) = NFT.with(|n| n.borrow().line(chain_id).in_flight) else { return Ok("nothing in flight".into()) };
    let (pin, _) = pin_on(&c).await?;
    let us = ours().await?.0;
    let (seen, owner_is_us) = match seen_on(&c, &f.tx_hash, pin.number).await {
        EvmSeen::Mined { block, success, .. } => {
            let owner = if success {
                let id = word_of_dec(&f.token_id)?;
                let coll = crate::evmtx::parse_address(&f.collection)?;
                Some(sp::decode_address(&call_at(&c, &coll, &sp::call1(sp::OWNER_OF, id), &pin).await?)? == us)
            } else {
                None
            };
            (ReadBack::Mined { block, success }, owner)
        }
        EvmSeen::NotYet | EvmSeen::Pending => (ReadBack::NotMined { pinned_nonce: quantity_at(&c, "eth_getTransactionCount", &us, &pin).await? as u64 }, None),
        EvmSeen::Unsure(e) => return Err(e),
    };
    let out = NFT.with(|n| crate::chains::settle(n.borrow_mut().line_mut(chain_id), seen.clone(), owner_is_us))?;
    // H2: the reservation made at signing becomes the held item, or is freed
    if let Some(o) = &out {
        let flight = format!("nft:{chain_id}:{}", f.order_hash);
        let held = match o {
            crate::chains::Outcome::Arrived { .. } => Some(nft_item_id(chain_id, &crate::evmtx::parse_address(&f.collection)?, &f.token_id)?),
            _ => None,
        };
        if let Err(e) = crate::canister::with_caps(|_, b| crate::custodycap::settle(b, &flight, held.as_deref())) {
            note(format!("nft status: custody book: {e}"));
        }
    }
    let msg = match out {
        Some(o) => format!("{} on chain {chain_id}: {o:?} (pin {})", f.tx_hash, pin.number),
        None => format!("{} on chain {chain_id}: waiting ({seen:?} at pin {})", f.tx_hash, pin.number),
    };
    note(format!("nft status: {msg}"));
    Ok(msg)
}

/// What the canister knows about one chain: the pin, our balance and nonce there, and its line.
#[derive(CandidType, Deserialize, Clone, Debug)]
pub struct NftChainStatus {
    pub chain_id: u64,
    pub pin: u64,
    pub pin_timestamp: u64,
    pub base_fee_wei: String,
    pub address: String,
    pub balance_wei: String,
    pub nonce: u64,
    pub line: crate::chains::Line,
    pub targets: Vec<NftTarget>,
}

#[ic_cdk::update]
pub async fn nft_chain_status(chain_id: u64) -> Result<NftChainStatus, String> {
    only_operator();
    let c = chain(chain_id)?;
    let (pin, ts) = pin_on(&c).await?;
    let us = ours().await?.0;
    Ok(NftChainStatus {
        chain_id, pin: pin.number, pin_timestamp: ts, base_fee_wei: pin.base_fee.to_string(), address: h0x(&us),
        balance_wei: quantity_at(&c, "eth_getBalance", &us, &pin).await?.to_string(),
        nonce: quantity_at(&c, "eth_getTransactionCount", &us, &pin).await? as u64,
        line: NFT.with(|n| n.borrow().line(chain_id)),
        targets: NFT.with(|n| n.borrow().targets.iter().filter(|t| t.chain_id == chain_id).cloned().collect()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_ethereum_block_read_reserves_room_for_a_real_block_and_the_home_chain_keeps_its_price() {
        // measured 2026-09-28, read-only, hashes only, the raw body the IC's cap applies to: 19,309..32,635 bytes over eight blocks at
        // the pin spacing on both providers of the pair, the finalized block 24,279, the largest of 24 blocks 51,161 (the rehearsal)
        for raw in [19_309u64, 24_279, 32_635, 51_161] {
            assert!(block_view_bytes(1) > raw * 2, "{raw}");
            assert!(BLOCK_VIEW_BYTES < raw, "the old 16 kB cap refused {raw}");
        }
        assert_eq!(block_view_bytes(4663), 16_000, "a 4663 block is about 4.3 kB");
        // every Ethereum block read by number or tag goes through block_view_bytes, in both files that make them
        for (name, src) in [("nftnet.rs", include_str!("nftnet.rs")), ("ethnet.rs", include_str!("ethnet.rs"))] {
            for (i, _) in src.match_indices("\"eth_getBlockByNumber\"") {
                let window: String = src[i..].chars().take(400).collect();
                if window.contains("finalized_time") { continue; }   // the HOME chain's clock (4663), not Ethereum
                assert!(window.contains("block_view_bytes("), "{name}: an eth_getBlockByNumber read without block_view_bytes: {}", &window[..120.min(window.len())]);
            }
        }
    }

    #[test]
    fn every_update_method_here_takes_the_operator_guard() {
        let src = include_str!("nftnet.rs");
        let needle = format!("#[ic_cdk::{}]", "update");
        let parts: Vec<&str> = src.split(needle.as_str()).skip(1).collect();
        assert!(parts.len() >= 6, "{} update methods", parts.len());
        for p in parts {
            let head: String = p.chars().take(400).collect();
            assert!(head.contains("only_operator();"), "unguarded: {}", head.lines().nth(1).unwrap_or(""));
        }
    }

    #[test]
    fn token_ids_parse_as_full_width_decimals() {
        assert_eq!(word_of_dec("9820").unwrap(), sp::word_u128(9_820));
        let big = "83285985867244386688011329125555419399256885411704979167199380724258635715412";
        assert_eq!(hex::encode(word_of_dec(big).unwrap()), "b822328e72cfeed48dc9c5412b33c5c406f05c013af4be96ebd1f2d0fd771354");
        assert!(word_of_dec(&"9".repeat(80)).unwrap_err().contains("256 bits"));
        assert!(word_of_dec("0x12").is_err() && word_of_dec("").is_err() && word_of_dec("-1").is_err());
    }
}
