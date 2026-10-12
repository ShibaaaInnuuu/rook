//! The buyer's loop (`docs/v2/buyer-loop.md`): read the chains, ask the state machine, do the one thing it says.
//!
//! Everything the loop can do to the world goes through [`Io`], and nothing else here touches the network, the keys or
//! stable memory. That is the point of the split: `state.rs` decides, this module sequences, and the canister's `Io` only
//! translates. Every decision below is unit-tested (`tests/loop.rs`) against a fake world that can disagree with itself,
//! lag, revert, and die between any two calls.

use crate::bridge::{self, Bridge};
use crate::policy;
use crate::evmtx::{self, Tx1559, VaultPurchase};
use crate::psbt::{self, Psbt, Quote};
use crate::runestone::RuneId;
use crate::sighash;
use crate::state::{Act, EvmStep, Observed, PendingEvm, Phase, Purchase, Stuck, VaultState, DEADLINE_MARGIN_SECS};
use crate::venue::Venue;

/// Gas above the estimate. An estimate is the gas the call used against the state the provider read; ours runs a block
/// later.
pub const GAS_MARGIN_PCT: u64 = 30;
/// The fee cap, as a multiple of the latest base fee. A cap, not a price: the chain charges the actual base fee.
pub const FEE_MULTIPLE: u128 = 4;
/// How long a bridge has to deliver BTC after its transaction is mined, before the purchase is `Stuck(Bridge)`.
pub const BRIDGE_GIVE_UP_SECS: u64 = 6 * 60 * 60;

/// What the EVM chain says about one of our transactions, read replicated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvmView {
    NotYet,
    Pending,
    Mined { success: bool },
    Unsure(String),
}

/// What Bitcoin says about one of our transactions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BtcView {
    NotYet,
    Mempool,
    Confirmed,
    Unsure(String),
}

/// Everything the loop may do to the world. `save` must be durable before the next `await` returns -- on the IC, state
/// written before an outcall's `await` is committed at it, which is what makes "save, then send" mean what it says.
#[allow(async_fn_in_trait)]
pub trait Io {
    fn now(&self) -> u64;
    fn chain_id(&self) -> u64;
    fn vault(&self) -> [u8; 20];
    async fn our_script(&mut self) -> Result<Vec<u8>, String>;
    async fn purchase_count(&mut self) -> Result<u64, String>;
    async fn vault_purchase(&mut self, id: u64) -> Result<VaultPurchase, String>;
    async fn evm_balance(&mut self) -> Result<u128, String>;
    async fn evm_nonce(&mut self) -> Result<u64, String>;
    async fn base_fee(&mut self) -> Result<u128, String>;
    async fn estimate_gas(&mut self, to: [u8; 20], value: u128, data: &[u8]) -> Result<u64, String>;
    /// (yParity, r, s), low-s, parity derived against our own address.
    async fn sign_evm(&mut self, hash: [u8; 32]) -> Result<(u8, [u8; 32], [u8; 32]), String>;
    async fn send_evm(&mut self, raw: &[u8]) -> Result<(), String>;
    async fn evm_seen(&mut self, hash: [u8; 32]) -> EvmView;
    fn save(&mut self, p: &Purchase);
    async fn btc_sats(&mut self) -> Result<u64, String>;
    async fn btc_seen(&mut self, txid: [u8; 32]) -> BtcView;
    async fn btc_send(&mut self, raw: &[u8]) -> Result<(), String>;
    /// Rune units at the quote's recipient from transaction `txid`. Needs a runes indexer.
    async fn delivered_units(&mut self, txid: [u8; 32], quote: &Quote) -> Result<u128, String>;
    async fn sign_taproot(&mut self, sighash: [u8; 32]) -> Result<Vec<u8>, String>;
    /// Belt and braces (docs/v3/wrapper.md, "Custody on its own key"): `Ok` only if the output may be signed by the buyer —
    /// ord shows it carrying no inscription and no rune. Required, with no default, so no world can skip it by omission.
    async fn input_clean(&mut self, o: crate::custody::Outpoint) -> Result<(), String>;
    fn venue(&self) -> Option<&dyn Venue>;
    fn bridge(&self) -> Option<&dyn Bridge>;
    fn bridge_allowlist(&self) -> Vec<[u8; 20]>;
    /// The least BTC `send_wei` is worth by OUR reference rate, less the tolerance: never the bridge's own number.
    async fn reference_floor_sats(&mut self, send_wei: u128) -> Result<u64, String>;
    /// The MOST BTC `wei` is worth by OUR reference rate, plus `rates::TOLERANCE_BPS`: the cap on what a quote may spend
    /// (`docs/v2/trust-points.md` §1). The same rate path as `reference_floor_sats`, never the venue's number.
    async fn reference_cap_sats(&mut self, wei: u128) -> Result<u64, String>;
    /// A replicated `eth_call` (`docs/v2/venue-bridge-policy.md` §1: the policy's reads). Unsupported unless implemented.
    async fn eth_read(&mut self, _to: [u8; 20], _data: &[u8]) -> Result<Vec<u8>, String> {
        Err("this world cannot read contracts".into())
    }
    /// The route's fixed cost for one purchase (bridge fee, Bitcoin fee, gas), in wei, by OUR estimate. Unsupported unless
    /// implemented, and then the policy opens nothing.
    async fn route_cost_wei(&mut self) -> Result<u128, String> {
        Err("no route cost estimate is configured: the policy opens nothing".into())
    }
    /// The installed Bitcoin network's address prefix ("bc", "tb").
    fn btc_hrp(&self) -> Result<&'static str, String> {
        Err("no Bitcoin network is configured".into())
    }
}

/// No purchase is announced unless ETH that leaves the vault has a path to BTC and a place to spend it.
pub fn configured<I: Io>(io: &I) -> Result<(), String> {
    match (io.venue().is_some(), io.bridge().is_some()) {
        (true, true) => Ok(()),
        (v, b) => Err(format!(
            "no {} configured: the buyer announces nothing it could not finish",
            match (v, b) { (false, false) => "venue and no bridge are", (false, true) => "venue is", _ => "bridge is" }
        )),
    }
}

fn terminal(p: &Phase) -> bool {
    matches!(p, Phase::Settled | Phase::Returned | Phase::Stuck { .. })
}

/// A new purchase may be opened only when every other one is finished: one purchase end to end at a time, because the
/// buyer's Bitcoin balance is one balance and the state machine reads "funded" from it.
pub fn can_open(purchases: &[Purchase]) -> Result<(), String> {
    match purchases.iter().find(|p| !terminal(&p.phase)) {
        Some(p) => Err(format!("refused: purchase {} is still {:?}; one purchase at a time", p.id, p.phase)),
        None => Ok(()),
    }
}

/// The vault and its listing, read replicated at one moment (`docs/v2/venue-bridge-policy.md` §1). The registry's listing
/// must name this vault, or nothing is believed.
/// This buyer's listing, as the registry records it, read through the vault's own `registry()` and `listingId()`: the one read
/// the policy's snapshot and the quote check share, so the address a purchase is opened for and the address a quote must pay
/// come from the same place.
pub async fn read_listing<I: Io>(io: &mut I) -> Result<policy::Listing, String> {
    let v = io.vault();
    let registry = policy::ret_address(&io.eth_read(v, &policy::call0(policy::REGISTRY)).await?)?;
    let listing_id = policy::ret_uint(&io.eth_read(v, &policy::call0(policy::LISTING_ID)).await?, 128)?;
    policy::ret_listing(&io.eth_read(registry, &policy::call_uint(policy::LISTING, listing_id)).await?)
}

/// A quote, checked against what the chain says the purchase is for (`docs/v2/trust-points.md` §1): the listing's rune and
/// Bitcoin address, the vault's announced units and released ETH at our reference rate. Called when a quote is recorded and
/// again before anything is signed against it, with fresh reads each time.
pub async fn check_quote_on_chain<I: Io>(io: &mut I, id: u64, q: &Quote) -> Result<(), String> {
    let listing = read_listing(io).await?;
    let v = io.vault_purchase(id).await?;
    let ours = io.our_script().await?;
    let cap = io.reference_cap_sats(v.max_eth.saturating_sub(v.returned)).await?;
    let hrp = io.btc_hrp()?;
    crate::listing::check_quote(q, &crate::listing::QuoteFacts { listing: &listing, our_vault: io.vault(), hrp, purchase: &v,
        our_script: &ours, cap_sats: cap })
}

pub async fn snapshot<I: Io>(io: &mut I) -> Result<policy::Snapshot, String> {
    let v = io.vault();
    let read = |sel| policy::call0(sel);
    let listing = read_listing(io).await?;
    if listing.vault != v {
        return Err(format!("the vault's listing names vault 0x{}, not ours", hex::encode(listing.vault)));
    }
    Ok(policy::Snapshot {
        now: io.now(),
        attributed: policy::ret_uint(&io.eth_read(v, &read(policy::ATTRIBUTED)).await?, 128)?,
        cap_wei: policy::ret_uint(&io.eth_read(v, &read(policy::CAP_WEI)).await?, 128)?,
        cap_expiry: policy::ret_uint(&io.eth_read(v, &read(policy::CAP_EXPIRY)).await?, 64)? as u64,
        max_in_flight: policy::ret_uint(&io.eth_read(v, &read(policy::MAX_IN_FLIGHT)).await?, 128)?,
        in_flight: policy::ret_uint(&io.eth_read(v, &read(policy::IN_FLIGHT)).await?, 128)?,
        open: policy::ret_array_len(&io.eth_read(v, &read(policy::OPEN_PURCHASES)).await?)?,
        blocked: policy::ret_bool_first(&io.eth_read(v, &read(policy::IS_BLOCKED)).await?)?,
        listing,
    })
}

/// The purchase policy: when nothing is unfinished and a venue and a bridge are configured, read the vault and open what the
/// on-chain state says is owed. `Ok(None)` is "nothing to open now"; `Err` is the reason it waits. Nothing here is the
/// operator's word.
pub async fn maybe_open<I: Io>(io: &mut I, purchases: &[Purchase]) -> Result<Option<Purchase>, String> {
    if can_open(purchases).is_err() {
        return Ok(None); // a purchase is running: the policy has nothing to decide
    }
    configured(io)?;
    let s = snapshot(io).await?;
    let cost = io.route_cost_wei().await?;
    let cands = policy::candidates(&s, cost)?;
    let mut payers = Vec::new();
    for a in &cands {
        let p = policy::ret_uint(&io.eth_read(io.vault(), &policy::call_uint(policy::PAYERS_FOR, *a)).await?, 64)? as u64;
        payers.push(p);
        if (1..=policy::MAX_PAYERS).contains(&p) {
            break;
        }
    }
    let amount = policy::first_fundable(&cands[..payers.len()], &payers)
        .ok_or_else(|| format!("no amount from {:?} is chargeable within {} payers ({payers:?})", cands, policy::MAX_PAYERS))?;
    let o = policy::order_for(&s, amount, io.btc_hrp()?)?;
    let id = io.purchase_count().await?;
    Ok(Some(Purchase {
        id,
        phase: Phase::Watching,
        amount_wei: o.amount_wei,
        order: Some(crate::state::Order { rune: o.rune, units: o.units, recipient_script: o.recipient_script, cap_ref: o.cap_ref }),
        pending_evm: None,
    }))
}

pub fn vault_state(status: u8) -> VaultState {
    match status {
        1 => VaultState::Announced,
        2 => VaultState::Pulled,
        3 => VaultState::Settled,
        4 => VaultState::Returned,
        5 => VaultState::WrittenOff,
        _ => VaultState::None,
    }
}

pub fn parse_rune(s: &str) -> Result<RuneId, String> {
    let (b, t) = s.split_once(':').ok_or_else(|| format!("a rune id is block:tx, got {s:?}"))?;
    Ok(RuneId { block: b.trim().parse().map_err(|_| format!("bad block in {s:?}"))?, tx: t.trim().parse().map_err(|_| format!("bad tx in {s:?}"))? })
}

/// Our own purchase, as the vault holds it: what we announced, or somebody else's.
fn is_ours(p: &Purchase, v: &VaultPurchase) -> bool {
    v.max_eth == p.amount_wei && p.order.as_ref().is_some_and(|o| o.units == v.quantity)
}

/// One pass over one purchase. Never throws: a failure is the purchase waiting this tick, with the reason.
pub async fn step<I: Io>(io: &mut I, p: &mut Purchase, other_evm_pending: bool) -> String {
    match step_inner(io, p, other_evm_pending).await {
        Ok(s) => format!("{}: {s}", p.id),
        Err(e) => format!("{}: waits: {e}", p.id),
    }
}

async fn step_inner<I: Io>(io: &mut I, p: &mut Purchase, other_evm_pending: bool) -> Result<String, String> {
    if terminal(&p.phase) {
        return Ok(format!("{:?}: nothing further", p.phase));
    }
    // 1. Our own transaction, if one is out, is settled before anything else is decided.
    if let Some(pe) = p.pending_evm.clone() {
        return resolve_pending(io, p, pe).await;
    }
    // 2. What the chains say, then what the state machine makes of it.
    let (o, vp) = observe(io, p).await?;
    if matches!(p.phase, Phase::Watching) {
        if let Some(v) = &vp {
            if !is_ours(p, v) {
                p.phase = Phase::Stuck { why: Stuck::Mismatch };
                io.save(p);
                return Ok(format!("the vault's purchase {} is not ours (max {} for {}): stuck", p.id, v.max_eth, v.quantity));
            }
        }
    }
    let before = p.phase.clone();
    let act = p.next(&o);
    if p.phase != before {
        io.save(p);
    }
    perform(io, p, act, other_evm_pending).await
}

/// Read the chains for one purchase. Any read that fails or disagrees fails the whole observation: the state machine is
/// never fed a guess.
pub async fn observe<I: Io>(io: &mut I, p: &Purchase) -> Result<(Observed, Option<VaultPurchase>), String> {
    let now = io.now();
    let count = io.purchase_count().await?;
    let vp = if p.id < count { Some(io.vault_purchase(p.id).await?) } else { None };
    let mut o = Observed { now, ..Default::default() };
    if let Some(v) = &vp {
        o.vault = vault_state(v.status);
        o.vault_ready_at = v.ready_at;
        o.vault_deadline = v.deadline;
    }
    match &p.phase {
        // Past the deadline the answer is "return", whatever Bitcoin says, so a Bitcoin read that fails does not hold it.
        Phase::Pulled { deadline } => match io.btc_sats().await {
            Ok(s) => o.btc_sats = s,
            Err(e) if now <= *deadline => return Err(e),
            Err(_) => {}
        },
        Phase::Bridging { give_up_at, .. } => match io.btc_sats().await {
            Ok(s) => o.btc_sats = s,
            Err(e) if now <= *give_up_at => return Err(e),
            Err(_) => {}
        },
        Phase::Broadcast { txid, quote } => match io.btc_seen(*txid).await {
            BtcView::Confirmed => {
                o.btc_tx_confirmed = Some(true);
                o.delivered_units = io.delivered_units(*txid, quote).await?;
            }
            BtcView::NotYet | BtcView::Mempool => {}
            BtcView::Unsure(e) => return Err(e),
        },
        _ => {}
    }
    Ok((o, vp))
}

async fn resolve_pending<I: Io>(io: &mut I, p: &mut Purchase, pe: PendingEvm) -> Result<String, String> {
    let _ = io.send_evm(&pe.raw).await; // the same bytes again; what the provider answers is not believed
    match io.evm_seen(pe.hash).await {
        EvmView::Mined { success: true } => {
            match pe.step {
                EvmStep::Announce => match io.vault_purchase(p.id).await {
                    Ok(v) if is_ours(p, &v) => p.phase = Phase::Announced { ready_at: v.ready_at },
                    Ok(_) => p.phase = Phase::Stuck { why: Stuck::Mismatch },
                    Err(_) => {} // the next observation adopts it from the vault (Watching + Announced)
                },
                EvmStep::Pull => {
                    if let Ok(v) = io.vault_purchase(p.id).await {
                        p.phase = Phase::Pulled { deadline: v.deadline.saturating_sub(DEADLINE_MARGIN_SECS) };
                    }
                }
                // The one step the vault cannot tell us about afterwards: set from our own record, before clearing it.
                EvmStep::Bridge { expect_sats } => {
                    p.phase = Phase::Bridging { sent_tx: pe.hash, expect_sats, give_up_at: io.now() + BRIDGE_GIVE_UP_SECS };
                }
                EvmStep::Settle => p.phase = Phase::Settled,
                EvmStep::Return => {} // the vault says Returned or not; the next observation reads it
            }
            p.pending_evm = None;
            io.save(p);
            Ok(format!("{:?} mined; now {:?}", pe.step, p.phase))
        }
        EvmView::Mined { success: false } => {
            p.pending_evm = None;
            p.phase = Phase::Stuck { why: Stuck::Reverted };
            io.save(p);
            Ok(format!("{:?} was mined and REVERTED: stuck, for the owner to look at", pe.step))
        }
        EvmView::NotYet => {
            let n = io.evm_nonce().await?;
            if n > pe.nonce {
                // Our nonce is used and our transaction is not visible. Only this loop signs for this key, so that is
                // almost surely a provider behind; either way, guessing is how a bridge gets paid twice.
                Err(format!("nonce {} is used but {:?} is not visible yet: waiting rather than guessing", pe.nonce, pe.step))
            } else {
                Ok(format!("{:?} sent again (nonce {}), not seen yet", pe.step, pe.nonce))
            }
        }
        EvmView::Pending => Ok(format!("{:?} pending (nonce {})", pe.step, pe.nonce)),
        EvmView::Unsure(e) => Err(e),
    }
}

/// Build, sign, SAVE, send. The save is committed before the send, so a canister that dies in between wakes holding the
/// bytes and sends the same ones.
async fn send_evm_step<I: Io>(io: &mut I, p: &mut Purchase, step: EvmStep, to: [u8; 20], value: u128, data: Vec<u8>,
    other_evm_pending: bool) -> Result<String, String> {
    if other_evm_pending {
        return Ok(format!("{step:?} waits: another EVM transaction of ours is out (one at a time)"));
    }
    let nonce = io.evm_nonce().await?;
    let base = io.base_fee().await?;
    let est = io.estimate_gas(to, value, &data).await.map_err(|e| format!("{step:?} would not succeed now: {e}"))?;
    let tx = Tx1559 {
        chain_id: io.chain_id(),
        nonce,
        max_priority_fee: 0,
        max_fee: base.saturating_mul(FEE_MULTIPLE),
        gas: est + est * GAS_MARGIN_PCT / 100,
        to,
        value,
        data,
    };
    let (y, r, s) = io.sign_evm(tx.signing_hash()).await?;
    let raw = tx.signed(y, &r, &s);
    let hash = evmtx::keccak(&raw);
    p.pending_evm = Some(PendingEvm { step, raw: raw.clone(), hash, nonce });
    io.save(p);
    let _ = io.send_evm(&raw).await;
    Ok(format!("{step:?} signed, saved and sent (nonce {nonce})"))
}

async fn perform<I: Io>(io: &mut I, p: &mut Purchase, act: Act, other_evm_pending: bool) -> Result<String, String> {
    let vault = io.vault();
    match act {
        Act::Wait => Ok(format!("waits ({:?})", p.phase)),
        Act::Announce { amount_wei } => {
            configured(io)?;
            let o = p.order.clone().ok_or("no order: this purchase was not opened for the loop")?;
            let count = io.purchase_count().await?;
            if count != p.id {
                return Err(format!("the vault's next id is {count}, not {}: refused rather than announce into another id", p.id));
            }
            send_evm_step(io, p, EvmStep::Announce, vault, 0, evmtx::announce(amount_wei, o.units, &o.cap_ref), other_evm_pending).await
        }
        Act::Pull { id } => send_evm_step(io, p, EvmStep::Pull, vault, 0, evmtx::pull(id), other_evm_pending).await,
        Act::Bridge { amount_wei, .. } => {
            if io.bridge().is_none() {
                p.phase = Phase::Stuck { why: Stuck::Unconfigured };
                io.save(p);
                return Ok("no bridge configured any more: stuck".into());
            }
            let ours = io.our_script().await?;
            let floor = io.reference_floor_sats(amount_wei).await?;
            let q = io.bridge().expect("checked above").quote(amount_wei, &ours).map_err(|e| format!("bridge: {e:?}"))?;
            bridge::check(&q, amount_wei, &ours, &io.bridge_allowlist(), floor).map_err(|e| format!("bridge quote refused: {e:?}"))?;
            send_evm_step(io, p, EvmStep::Bridge { expect_sats: q.expect_sats }, q.to, q.send_wei, q.calldata, other_evm_pending).await
        }
        Act::GetQuote { .. } => {
            let Some(v) = io.venue() else {
                p.phase = Phase::Stuck { why: Stuck::Unconfigured };
                io.save(p);
                return Ok("no venue configured any more: stuck".into());
            };
            let o = p.order.clone().ok_or("no order: this purchase was not opened for the loop")?;
            let q = v.quote(&o.rune, o.units, &o.recipient_script).map_err(|e| format!("venue: {e:?}"))?;
            let ours = io.our_script().await?;
            let sats = io.btc_sats().await?;
            if q.rune != parse_rune(&o.rune)? || q.units < o.units || q.recipient_script != o.recipient_script || q.our_script != ours {
                return Err("the venue quoted something other than the order: not recorded".into());
            }
            if q.max_spend_sats > sats {
                return Err(format!("the quote may spend {} sats and we hold {sats}: not recorded", q.max_spend_sats));
            }
            check_quote_on_chain(io, p.id, &q).await.map_err(|e| format!("{e}: not recorded"))?;
            p.record_quote(q)?;
            io.save(p);
            Ok("quote recorded, before any PSBT".into())
        }
        Act::RequestAndVerifyPsbt { quote, .. } => {
            if io.venue().is_none() {
                p.phase = Phase::Stuck { why: Stuck::Unconfigured };
                io.save(p);
                return Ok("no venue configured any more: stuck".into());
            }
            // the chain is asked again before anything is signed: a listing paused, or its address changed, since the quote
            check_quote_on_chain(io, p.id, &quote).await.map_err(|e| format!("{e}: nothing signed"))?;
            let Some(v) = io.venue() else { return Err("no venue".into()) };
            let (bytes, to_sign) = v.psbt(&quote).map_err(|e| format!("venue: {e:?}"))?;
            let psbt = Psbt::parse(&bytes).map_err(|e| format!("the PSBT does not parse: {e:?}"))?;
            let ours = psbt.verify(&quote, &to_sign).map_err(|e| format!("refused: {e:?}"))?;
            let txid = psbt.txid();
            // nothing of ours that carries an inscription or a rune is ever signed away by a purchase, whatever the PSBT says
            for &i in &ours {
                io.input_clean(crate::custody::outpoint_of(&psbt.tx.inputs[i])).await?;
            }
            // one quote, one transaction; bound before any signature is awaited (as `sign_purchase`)
            p.quote_to_sign(&txid)?;
            p.bind(txid);
            io.save(p);
            let mut sigs = Vec::new();
            for i in ours {
                let h = sighash::taproot_key_path(&psbt, i).map_err(|e| format!("digest: {e:?}"))?;
                sigs.push((i, io.sign_taproot(h).await?));
            }
            let others = psbt::final_witnesses(&bytes).map_err(|e| format!("{e:?}"))?;
            let raw = psbt::finalize(&psbt, &others, &sigs)?;
            p.phase = Phase::Signed { txid, raw_tx: raw, quote };
            io.save(p);
            Ok(format!("signed {}; the bytes are saved before anything is broadcast", hex::encode(txid)))
        }
        Act::Broadcast { txid, raw_tx, .. } => {
            let _ = io.btc_send(&raw_tx).await; // the answer is ignored; Bitcoin is read instead
            match io.btc_seen(txid).await {
                BtcView::Mempool | BtcView::Confirmed => {
                    if let Phase::Signed { quote, .. } = p.phase.clone() {
                        p.phase = Phase::Broadcast { txid, quote };
                        io.save(p);
                    }
                    Ok("Bitcoin has it".into())
                }
                BtcView::NotYet => Ok("broadcast; Bitcoin does not show it yet (the same bytes go again next tick)".into()),
                BtcView::Unsure(e) => Err(e),
            }
        }
        Act::ReadBackBitcoin { .. } => Ok("reading Bitcoin until it confirms and the delivery shows".into()),
        Act::Settle { id, txid, units, sats, .. } => {
            send_evm_step(io, p, EvmStep::Settle, vault, 0, evmtx::settle_rune(id, units, txid, sats), other_evm_pending).await
        }
        Act::ReturnFunds { id, .. } => {
            let v = io.vault_purchase(id).await?;
            match vault_state(v.status) {
                // never pulled: a cancellation, with nothing sent
                VaultState::Announced => send_evm_step(io, p, EvmStep::Return, vault, 0, evmtx::return_funds(id), other_evm_pending).await,
                VaultState::Pulled => {
                    let owed = v.max_eth.saturating_sub(v.returned);
                    let bal = io.evm_balance().await?;
                    let base = io.base_fee().await?;
                    let est = io.estimate_gas(vault, 1, &evmtx::return_funds(id)).await?;
                    let reserve = (est + est * GAS_MARGIN_PCT / 100) as u128 * base.saturating_mul(FEE_MULTIPLE);
                    let value = owed.min(bal.saturating_sub(reserve));
                    if value == 0 {
                        return Err(format!("nothing of ours to return: {bal} wei held, {reserve} kept for gas"));
                    }
                    send_evm_step(io, p, EvmStep::Return, vault, value, evmtx::return_funds(id), other_evm_pending).await
                }
                other => Ok(format!("the vault says {other:?}: nothing to return")),
            }
        }
        Act::Halt { why, .. } => Ok(format!("halted: {why:?}; the owner writes off, in public")),
    }
}
