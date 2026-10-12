//! The other EVM chains the buyer can buy on (`docs/v4/eth-nft-canister.md` §3–4, §7–8): their configuration, its
//! validation, the per-chain pin with the chain's own clock, the fee and gas caps, the one-purchase-in-flight line, and Relay's
//! quote for bringing ETH there.
//!
//! Pure: no network, no keys. Nothing here names a particular chain. A chain is a [`EvmChain`] entry in the install argument;
//! the async half is `nftnet.rs`.

use candid::CandidType;
use serde::{Deserialize, Serialize};

/// One chain, as installed. Every field is public.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct EvmChain {
    /// Checked against `eth_chainId` on every use, and the chain id every transaction for it is signed with.
    pub chain_id: u64,
    /// At least two, distinct; all must agree (docs/v2/pinned-reads.md).
    pub rpc_urls: Vec<String>,
    /// Heads are rounded to this many blocks: ~90–130 s of this chain's blocks (§3).
    pub bucket: u64,
    /// Must be [`crate::seaport::SEAPORT_1_6`].
    pub seaport: String,
    /// keccak of Seaport's runtime code on THIS chain (it differs per chain: chain id and domain are immutables).
    pub seaport_codehash: String,
    /// Hard cap on `maxFeePerGas`.
    pub max_fee_wei: u64,
    pub priority_fee_wei: u64,
    /// Hard cap on the gas limit.
    pub gas_cap: u64,
    /// Marketplace fee recipients an order may pay (besides the offerer and the collection's royalty receiver).
    pub fee_recipients: Vec<String>,
    /// All payments to others, at most this share of the total.
    pub max_fee_bps: u32,
    /// An order must stay valid this long past the pinned block's timestamp.
    pub min_order_life_s: u64,
    /// Restricted orders are bought only through these zones (OpenSea's SignedZone), each pinned by its code hash on THIS
    /// chain (docs/v4/eth-nft-canister.md §12). Optional so an install or state from before it still decodes; `None`: none.
    pub signed_zones: Option<Vec<AllowedZone>>,
    /// Transfer validators a zone's substandard 7/8/9 may name (Limit Break's, for creator-token collections).
    pub zone_registries: Option<Vec<String>>,
}

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct AllowedZone {
    pub zone: String,
    pub codehash: String,
}

impl EvmChain {
    pub fn seaport_addr(&self) -> Result<[u8; 20], String> {
        crate::evmtx::parse_address(&self.seaport)
    }
    pub fn codehash(&self) -> Result<[u8; 32], String> {
        hex::decode(self.seaport_codehash.trim_start_matches("0x")).map_err(|_| "seaport_codehash is not hex".to_string())?
            .try_into().map_err(|_| "seaport_codehash is not 32 bytes".into())
    }
    /// The allowed zones with their code hashes, and the allowed registries.
    pub fn zones(&self) -> Result<(Vec<([u8; 20], [u8; 32])>, crate::seaport::ZoneRules), String> {
        let mut zones = Vec::new();
        for z in self.signed_zones.iter().flatten() {
            let h: [u8; 32] = hex::decode(z.codehash.trim_start_matches("0x")).map_err(|_| "a zone code hash is not hex".to_string())?
                .try_into().map_err(|_| "a zone code hash is not 32 bytes".to_string())?;
            zones.push((crate::evmtx::parse_address(&z.zone)?, h));
        }
        let registries = self.zone_registries.iter().flatten().map(|a| crate::evmtx::parse_address(a)).collect::<Result<Vec<_>, _>>()?;
        Ok((zones.clone(), crate::seaport::ZoneRules { zones: zones.into_iter().map(|(a, _)| a).collect(), registries }))
    }
    pub fn rules(&self) -> Result<crate::seaport::Rules, String> {
        Ok(crate::seaport::Rules {
            fee_recipients: self.fee_recipients.iter().map(|a| crate::evmtx::parse_address(a)).collect::<Result<_, _>>()?,
            max_fee_bps: self.max_fee_bps,
            min_order_life_s: self.min_order_life_s,
        })
    }
}

/// Audit C3-02: Ethereum's block time, in milliseconds (the slot: 12 s).
pub const ETHEREUM_BLOCK_MS: u64 = 12_000;

/// Audit C3-02: a chain's block time in milliseconds, for every count of blocks the NFT floor cuts on it (`ethnft::sale_blocks`):
/// Ethereum's 12 s slot; Robinhood Chain's 0.104 s, measured over ~1,000 blocks (docs/v4/eth-nft-canister.md §2–3), its testnet's alike;
/// Base's 2 s. `None`: a chain whose block time this build does not know (`ethnft::validate` refuses an NFT collection on it).
pub fn block_ms(chain_id: u64) -> Option<u64> {
    match chain_id {
        1 => Some(ETHEREUM_BLOCK_MS),
        4663 | 46630 => Some(104),
        8453 => Some(2_000),
        _ => None,
    }
}

/// Every rule an install or upgrade argument's chain list must satisfy. `home` is the vault's chain: its nonce line belongs
/// to the vault loop, and the same key has the same address there, so it may not be listed (two writers, one nonce line).
/// The EVM chain that is the home, for [`validate`]: the install's `chain_id` on the EVM home; none (0, which no chain may be) on the
/// Solana home (phase B2), where the vault loop's chain is not a home and its nonce line is no one's to protect.
pub fn evm_home(chain_id: u64, solana_home: bool) -> u64 {
    if solana_home { 0 } else { chain_id }
}

/// Every rule `evm_chains` must satisfy. `home`: the EVM home's chain id (`evm_home`), never listed here; 0 on the Solana home.
pub fn validate(chains: &[EvmChain], home: u64) -> Result<(), String> {
    for (i, c) in chains.iter().enumerate() {
        let id = c.chain_id;
        if id == 0 {
            return Err("an EVM chain with chain id 0".into());
        }
        if id == home {
            return Err(format!("chain {id} is the vault's own chain: its nonce line belongs to the vault loop"));
        }
        if chains[..i].iter().any(|d| d.chain_id == id) {
            return Err(format!("chain {id} is listed twice"));
        }
        if c.rpc_urls.len() < 2 {
            return Err(format!("chain {id}: two RPC providers at least; one is not a check"));
        }
        if c.rpc_urls.iter().enumerate().any(|(k, u)| c.rpc_urls[..k].contains(u)) {
            return Err(format!("chain {id}: the RPC providers must be distinct"));
        }
        if c.bucket == 0 {
            return Err(format!("chain {id}: a bucket of zero blocks"));
        }
        if c.seaport_addr()? != crate::seaport::SEAPORT_1_6 {
            return Err(format!("chain {id}: {} is not the canonical Seaport 1.6", c.seaport));
        }
        c.codehash().map_err(|e| format!("chain {id}: {e}"))?;
        if c.max_fee_wei == 0 || c.gas_cap == 0 {
            return Err(format!("chain {id}: the fee and gas caps must be above zero"));
        }
        if c.priority_fee_wei > c.max_fee_wei {
            return Err(format!("chain {id}: the priority fee is above the fee cap"));
        }
        if c.max_fee_bps > 10_000 {
            return Err(format!("chain {id}: max_fee_bps above 100%"));
        }
        c.rules().map_err(|e| format!("chain {id}: a fee recipient: {e}"))?;
        c.zones().map_err(|e| format!("chain {id}: signed_zones / zone_registries: {e}"))?;
    }
    Ok(())
}

// ---- the pin, with the chain's clock -----------------------------------------------------------------------------------------

/// A pinned block as [`crate::pin::block_view`] projects it, plus its timestamp: an order's `endTime` is judged against the
/// chain's clock, because Seaport judges it against `block.timestamp`, not the IC's.
pub fn block_view_ts(v: &serde_json::Value) -> serde_json::Value {
    let mut out = crate::pin::block_view(v);
    if let Some(o) = out.as_object_mut() {
        let ts = v.get("timestamp").and_then(|x| x.as_str()).map(|x| serde_json::Value::String(x.to_ascii_lowercase()));
        o.insert("timestamp".into(), ts.unwrap_or(serde_json::Value::Null));
    }
    out
}

/// The transform body for `eth_getBlockByNumber(pin)` on another chain.
pub fn block_body_ts(body: &[u8]) -> Vec<u8> {
    match serde_json::from_slice::<serde_json::Value>(body).ok().and_then(|v| v.get("result").cloned()) {
        Some(b) => serde_json::json!({ "result": block_view_ts(&b) }).to_string().into_bytes(),
        None => br#"{"error":"unusable reply"}"#.to_vec(),
    }
}

/// [`crate::pin::agree_block`], and the agreed block's timestamp.
pub fn agree_block_ts(views: &[serde_json::Value], want: u64) -> Result<(crate::pin::Pin, u64), String> {
    let pin = crate::pin::agree_block(views, want)?;
    let ts = views[0].get("timestamp").and_then(|x| x.as_str()).and_then(|x| x.strip_prefix("0x"))
        .and_then(|x| u64::from_str_radix(x, 16).ok()).ok_or("the pinned block has no timestamp")?;
    Ok((pin, ts))
}

// ---- fees and gas ------------------------------------------------------------------------------------------------------------

/// `maxFeePerGas` from the pinned base fee (the loop's [`crate::looper::FEE_MULTIPLE`] margin, the same as on 4663) plus the
/// priority fee, refused above the chain's cap; and the gas limit, the estimate plus 20%, refused above its cap.
pub fn fees_and_gas(c: &EvmChain, base_fee: u128, estimate: u64) -> Result<(u128, u128, u64), String> {
    let prio = c.priority_fee_wei as u128;
    let max_fee = base_fee.saturating_mul(crate::looper::FEE_MULTIPLE).saturating_add(prio);
    if max_fee > c.max_fee_wei as u128 {
        return Err(format!("refused (fee cap): the pinned base fee {base_fee} needs a max fee of {max_fee} wei, over this chain's \
            cap of {}", c.max_fee_wei));
    }
    let gas = estimate.saturating_add(estimate / 5);
    if gas > c.gas_cap {
        return Err(format!("refused (gas cap): {gas} gas is over this chain's cap of {}", c.gas_cap));
    }
    Ok((max_fee, prio, gas))
}

// ---- the line: one purchase in flight per chain ------------------------------------------------------------------------------

/// A purchase signed on a chain and not yet settled. Saved **before** anything is sent.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct InFlight {
    pub nonce: u64,
    /// keccak of `raw`: the only hash we ever watch.
    pub tx_hash: String,
    pub raw: Vec<u8>,
    pub collection: String,
    pub token_id: String,
    pub total_wei: String,
    pub order_hash: String,
}

/// How a purchase ended, kept after its line is cleared.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Mined, succeeded, and `ownerOf` at a pin at or above its block is us.
    Arrived { block: u64 },
    /// Mined and reverted: the gas is spent, the NFT is not ours.
    Reverted { block: u64 },
    /// The pinned nonce moved past ours with no receipt for our hash: another transaction took the nonce.
    Superseded,
}

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct Line {
    pub in_flight: Option<InFlight>,
    pub done: Vec<(InFlight, Outcome)>,
}

/// A recorded target: the one token to buy on a chain, and the most to pay, recorded before any order exists.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct NftTarget {
    pub chain_id: u64,
    pub collection: String,
    /// Decimal.
    pub token_id: String,
    /// Decimal wei.
    pub cap_wei: String,
}

/// Everything the NFT side keeps in stable memory.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct NftState {
    pub lines: Vec<(u64, Line)>,
    pub targets: Vec<NftTarget>,
}

impl NftState {
    pub fn line(&self, chain_id: u64) -> Line {
        self.lines.iter().find(|(c, _)| *c == chain_id).map(|(_, l)| l.clone()).unwrap_or_default()
    }
    pub fn line_mut(&mut self, chain_id: u64) -> &mut Line {
        if let Some(i) = self.lines.iter().position(|(c, _)| *c == chain_id) {
            return &mut self.lines[i].1;
        }
        self.lines.push((chain_id, Line::default()));
        &mut self.lines.last_mut().expect("just pushed").1
    }
    /// Chains with a purchase in flight: an upgrade may not drop any of them.
    pub fn busy_chains(&self) -> Vec<u64> {
        self.lines.iter().filter(|(_, l)| l.in_flight.is_some()).map(|(c, _)| *c).collect()
    }
}

/// Rule of the line: nothing new is signed while a purchase is in flight on that chain.
pub fn may_sign(line: &Line) -> Result<(), String> {
    match &line.in_flight {
        Some(f) => Err(format!("refused (in flight): {} (nonce {}) is not settled on this chain yet", f.tx_hash, f.nonce)),
        None => Ok(()),
    }
}

/// What the read-back says about the in-flight purchase, and what the line does about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadBack {
    /// A receipt at or below the pin.
    Mined { block: u64, success: bool },
    /// No receipt at the pin; the nonce at the pin.
    NotMined { pinned_nonce: u64 },
}

/// Settle the line from the read-back. `owner_is_us` is `ownerOf(tokenId)` at the pin, asked only when mined. Returns the
/// outcome when the line is cleared, `None` while it must wait.
pub fn settle(line: &mut Line, seen: ReadBack, owner_is_us: Option<bool>) -> Result<Option<Outcome>, String> {
    let Some(f) = line.in_flight.clone() else { return Ok(None) };
    let outcome = match seen {
        ReadBack::Mined { block, success: true } => match owner_is_us {
            Some(true) => Outcome::Arrived { block },
            Some(false) => return Err(format!("mined in block {block}, but the NFT's owner at the pin is not us: nothing is recorded; \
                look before anything else is signed on this chain")),
            None => return Ok(None),
        },
        ReadBack::Mined { block, success: false } => Outcome::Reverted { block },
        ReadBack::NotMined { pinned_nonce } if pinned_nonce > f.nonce => Outcome::Superseded,
        ReadBack::NotMined { .. } => return Ok(None),
    };
    line.in_flight = None;
    line.done.push((f, outcome.clone()));
    Ok(Some(outcome))
}

// ---- Relay: bringing ETH from the vault's chain, judged and never signed ----------------------------------------------------

/// A Relay deposit that passed every check: what would be signed on the home chain, and what arrives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmBridgeQuote {
    pub to: [u8; 20],
    pub value: u128,
    pub data: Vec<u8>,
    pub request_id: String,
    pub wei_out: u128,
}

/// Judge Relay's `/quote` for native ETH from `home` to `dest`, paid to `ours` on `dest`: the checks of
/// [`crate::relayquote::judge`], with an EVM destination. Arrival is proven only by reading `dest` at a pin.
pub fn judge_relay_evm(body: &[u8], home: u64, dest: u64, ours: [u8; 20], amount_wei: u128, floor_wei: u128)
    -> Result<EvmBridgeQuote, String> {
    use crate::relayquote::RELAY_DEPOSITORY_4663;
    let q: serde_json::Value = serde_json::from_slice(body).map_err(|e| format!("not JSON: {e}"))?;
    let get = |v: &serde_json::Value, p: &[&str]| -> Result<serde_json::Value, String> {
        p.iter().try_fold(v.clone(), |x, k| x.get(*k).cloned().ok_or_else(|| format!("the quote has no {}", p.join("."))))
    };
    let num = |v: &serde_json::Value, p: &[&str]| -> Result<u128, String> {
        let x = get(v, p)?;
        x.as_str().map(|s| s.to_string()).or_else(|| x.as_u64().map(|n| n.to_string()))
            .and_then(|s| s.parse().ok()).ok_or_else(|| format!("{} is not a whole number", p.join(".")))
    };
    let addr = |v: &serde_json::Value, p: &[&str]| -> Result<[u8; 20], String> {
        crate::evmtx::parse_address(get(v, p)?.as_str().ok_or_else(|| format!("{} is not a string", p.join(".")))?)
    };
    let steps = get(&q, &["steps"])?;
    let steps = steps.as_array().ok_or("steps is not a list")?;
    if steps.len() != 1 || get(&steps[0], &["id"])? != "deposit" || get(&steps[0], &["kind"])? != "transaction" {
        return Err("not a single deposit transaction".into());
    }
    let items = get(&steps[0], &["items"])?;
    let items = items.as_array().ok_or("items is not a list")?;
    if items.len() != 1 {
        return Err(format!("{} items: exactly one transaction is signed", items.len()));
    }
    let d = get(&items[0], &["data"])?;
    if num(&d, &["chainId"])? != home as u128 {
        return Err("the deposit is not on the vault's chain".into());
    }
    let to = addr(&d, &["to"])?;
    if to != RELAY_DEPOSITORY_4663 {
        return Err(format!("the deposit goes to 0x{}, not Relay's depository", hex::encode(to)));
    }
    let value = num(&d, &["value"])?;
    if value != amount_wei {
        return Err(format!("the deposit sends {value} wei, not {amount_wei}"));
    }
    let data = hex::decode(get(&d, &["data"])?.as_str().unwrap_or("").trim_start_matches("0x")).map_err(|_| "calldata is not hex")?;
    if data.len() != 68 || data[..4] != crate::bridgecall::RELAY_DEPOSIT_NATIVE {
        return Err("the calldata is not depositNative(address, bytes32)".into());
    }
    if data[4..16].iter().any(|b| *b != 0) || data[16..36] != ours {
        return Err("the calldata names another depositor".into());
    }
    let details = get(&q, &["details"])?;
    if addr(&details, &["recipient"])? != ours {
        return Err("the quote's recipient is not our own address".into());
    }
    let out = get(&details, &["currencyOut"])?;
    if num(&out, &["currency", "chainId"])? != dest as u128 {
        return Err("the quote pays out on another chain".into());
    }
    if addr(&out, &["currency", "address"])? != [0u8; 20] {
        return Err("the quote does not pay out native ETH".into());
    }
    let wei_out = num(&out, &["amount"])?;
    if wei_out < floor_wei {
        return Err(format!("{wei_out} wei out is under our floor of {floor_wei}"));
    }
    Ok(EvmBridgeQuote { to, value, data, request_id: get(&steps[0], &["requestId"])?.as_str().unwrap_or("").into(), wei_out })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn chain(id: u64) -> EvmChain {
        EvmChain {
            chain_id: id,
            rpc_urls: vec!["https://a".into(), "https://b".into()],
            bucket: 8,
            seaport: "0x0000000000000068F116a894984e2DB1123eB395".into(),
            seaport_codehash: "0x74499ac0cce14428e4b41541d5e44f28f5a6882a1051d0118867c2a93cd5aec0".into(),
            max_fee_wei: 200_000_000_000,
            priority_fee_wei: 1_000_000_000,
            gas_cap: 400_000,
            fee_recipients: vec!["0x0000a26b00c1F0DF003000390027140000fAa719".into()],
            max_fee_bps: 1_500,
            min_order_life_s: 300,
            signed_zones: None,
            zone_registries: None,
        }
    }

    #[test]
    fn a_chain_list_is_validated_rule_by_rule() {
        assert_eq!(validate(&[chain(1), chain(8453)], 4663), Ok(()));
        assert_eq!(validate(&[], 4663), Ok(()), "no chains: no NFT buying, as before");
        let bad = |f: &dyn Fn(&mut EvmChain), why: &str| {
            let mut c = chain(1);
            f(&mut c);
            let e = validate(&[c], 4663).unwrap_err();
            assert!(e.contains(why), "{why}: {e}");
        };
        bad(&|c| c.chain_id = 0, "chain id 0");
        bad(&|c| c.chain_id = 4663, "vault's own chain");
        bad(&|c| c.rpc_urls.truncate(1), "two RPC providers");
        bad(&|c| c.rpc_urls[1] = c.rpc_urls[0].clone(), "distinct");
        bad(&|c| c.bucket = 0, "bucket");
        bad(&|c| c.seaport = "0x00000000000000ADc04C56Bf30aC9d3c0aAF14dC".into(), "canonical Seaport");
        bad(&|c| c.seaport_codehash = "0x1234".into(), "32 bytes");
        bad(&|c| c.max_fee_wei = 0, "above zero");
        bad(&|c| c.priority_fee_wei = c.max_fee_wei + 1, "priority fee");
        bad(&|c| c.max_fee_bps = 10_001, "above 100%");
        bad(&|c| c.fee_recipients.push("nope".into()), "fee recipient");
        bad(&|c| c.signed_zones = Some(vec![AllowedZone { zone: "0x000056F7000000EcE9003ca63978907a00FFD100".into(), codehash: "0x12".into() }]),
            "signed_zones");
        bad(&|c| c.zone_registries = Some(vec!["0x12".into()]), "zone_registries");
        let mut z = chain(8453);
        z.signed_zones = Some(vec![AllowedZone { zone: "0x000056F7000000EcE9003ca63978907a00FFD100".into(),
            codehash: "0x9cc021be99f4db02490d261e6fe6c3792bd2e777d10ff799150a2eaecf41c701".into() }]);
        z.zone_registries = Some(vec!["0x721c008fdff27bf06e7e123956e2fe03b63342e3".into()]);
        assert_eq!(validate(&[z.clone()], 4663), Ok(()));
        assert_eq!(z.zones().unwrap().1.zones.len(), 1);
        assert!(validate(&[chain(1), chain(1)], 4663).unwrap_err().contains("listed twice"));
    }

    #[test]
    fn the_pinned_block_carries_the_chains_clock_and_all_providers_must_agree_on_it() {
        let blk = |ts: &str| serde_json::json!({ "number": "0x18d4d67", "hash": format!("0x{}", "ab".repeat(32)),
            "baseFeePerGas": "0x1dcd6500", "timestamp": ts, "transactions": ["0x01"] });
        let body = serde_json::json!({ "jsonrpc": "2.0", "id": 1, "result": blk("0x6ab34c73") }).to_string();
        let v: serde_json::Value = serde_json::from_slice(&block_body_ts(body.as_bytes())).unwrap();
        let (pin, ts) = agree_block_ts(&[v["result"].clone(), v["result"].clone()], 26_037_607).unwrap();
        assert_eq!((pin.number, pin.base_fee, ts), (26_037_607, 500_000_000, 1_790_135_411));
        // a provider with another clock for the same block is a disagreement, not an average
        let other = block_view_ts(&blk("0x6ab34c74"));
        assert!(agree_block_ts(&[v["result"].clone(), other], 26_037_607).unwrap_err().contains("disagree"));
        assert!(agree_block_ts(&[crate::pin::block_view(&blk("0x1"))], 26_037_607).unwrap_err().contains("timestamp"));
    }

    #[test]
    fn a_chain_saved_before_signed_zones_existed_still_decodes() {
        #[derive(CandidType)]
        struct Old { chain_id: u64, rpc_urls: Vec<String>, bucket: u64, seaport: String, seaport_codehash: String, max_fee_wei: u64,
            priority_fee_wei: u64, gas_cap: u64, fee_recipients: Vec<String>, max_fee_bps: u32, min_order_life_s: u64 }
        let c = chain(1);
        let old = Old { chain_id: 1, rpc_urls: c.rpc_urls.clone(), bucket: 8, seaport: c.seaport.clone(), seaport_codehash: c.seaport_codehash.clone(),
            max_fee_wei: c.max_fee_wei, priority_fee_wei: c.priority_fee_wei, gas_cap: c.gas_cap, fee_recipients: c.fee_recipients.clone(),
            max_fee_bps: 1_500, min_order_life_s: 300 };
        let (back,): (Vec<EvmChain>,) = candid::decode_args(&candid::encode_args((vec![old],)).unwrap()).expect("decodes");
        assert_eq!(back, vec![c]);
    }

    #[test]
    fn fees_are_capped_per_chain() {
        let c = chain(1);
        assert_eq!(fees_and_gas(&c, 10_000_000_000, 150_000), Ok((41_000_000_000, 1_000_000_000, 180_000)));
        assert!(fees_and_gas(&c, 60_000_000_000, 150_000).unwrap_err().contains("fee cap"));
        assert!(fees_and_gas(&c, 1, 350_000).unwrap_err().contains("gas cap"));
    }

    fn flight(nonce: u64) -> InFlight {
        InFlight { nonce, tx_hash: "0xaa".into(), raw: vec![2], collection: "0x01".into(), token_id: "1".into(),
            total_wei: "5".into(), order_hash: "0xbb".into() }
    }

    #[test]
    fn one_purchase_in_flight_per_chain_and_only_the_read_back_settles_it() {
        let mut s = NftState::default();
        assert_eq!(may_sign(&s.line(1)), Ok(()));
        s.line_mut(1).in_flight = Some(flight(7));
        assert!(may_sign(&s.line(1)).unwrap_err().contains("in flight"));
        assert_eq!(may_sign(&s.line(8453)), Ok(()), "another chain's line is its own");
        assert_eq!(s.busy_chains(), vec![1]);
        // not mined and the nonce has not moved: wait
        let l = s.line_mut(1);
        assert_eq!(settle(l, ReadBack::NotMined { pinned_nonce: 7 }, None), Ok(None));
        // mined, but arrival not yet read: wait
        assert_eq!(settle(l, ReadBack::Mined { block: 9, success: true }, None), Ok(None));
        // mined and the owner is not us: loud, and the line stays blocked
        assert!(settle(l, ReadBack::Mined { block: 9, success: true }, Some(false)).is_err());
        assert!(l.in_flight.is_some());
        // mined and ours: arrived, the line is free
        assert_eq!(settle(l, ReadBack::Mined { block: 9, success: true }, Some(true)), Ok(Some(Outcome::Arrived { block: 9 })));
        assert!(l.in_flight.is_none() && l.done.len() == 1);
        // reverted, or superseded by another transaction at our nonce
        l.in_flight = Some(flight(8));
        assert_eq!(settle(l, ReadBack::Mined { block: 10, success: false }, None), Ok(Some(Outcome::Reverted { block: 10 })));
        l.in_flight = Some(flight(9));
        assert_eq!(settle(l, ReadBack::NotMined { pinned_nonce: 10 }, None), Ok(Some(Outcome::Superseded)));
        assert!(s.busy_chains().is_empty());
    }

    const QUOTE: &str = include_str!("../tests/fixtures/seaport/relay-quote-4663-8453-2026-09-23.json");
    const QUOTED_FOR: [u8; 20] = [0x11; 20];

    #[test]
    fn a_real_relay_quote_to_base_passes_and_every_checkable_lie_is_refused() {
        let q = judge_relay_evm(QUOTE.as_bytes(), 4663, 8453, QUOTED_FOR, 100_000_000_000_000_000, 99_000_000_000_000_000).unwrap();
        assert_eq!((q.to, q.value, q.wei_out), (crate::relayquote::RELAY_DEPOSITORY_4663, 100_000_000_000_000_000, 99_791_605_776_267_177));
        let j = |dest, ours, amount, floor| judge_relay_evm(QUOTE.as_bytes(), 4663, dest, ours, amount, floor).unwrap_err();
        assert!(j(1, QUOTED_FOR, 100_000_000_000_000_000, 0).contains("another chain"));
        assert!(j(8453, [0x22; 20], 100_000_000_000_000_000, 0).contains("another depositor"));
        assert!(j(8453, QUOTED_FOR, 1, 0).contains("sends"));
        assert!(j(8453, QUOTED_FOR, 100_000_000_000_000_000, 99_900_000_000_000_000).contains("floor"));
        let mut v: serde_json::Value = serde_json::from_str(QUOTE).unwrap();
        v["details"]["currencyOut"]["currency"]["address"] = "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913".into();
        assert!(judge_relay_evm(&serde_json::to_vec(&v).unwrap(), 4663, 8453, QUOTED_FOR, 100_000_000_000_000_000, 0).unwrap_err()
            .contains("native ETH"));
    }
}
