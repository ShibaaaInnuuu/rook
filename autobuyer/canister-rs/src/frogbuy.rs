//! Buying one frog (one inscription) from the float, for an item sink (a collection's buy queue, a graduating curve) or a
//! mint-desk request (docs/v3/curve.md,
//! "Canister: graduation buys, the mint desk and holder deposits").
//!
//! Pure: no network, no keys, no clock. [`judge`] decides whether a marketplace PSBT for one inscription may be signed, from
//! facts the canister read itself (ord's location of the inscription, the explorers' account of the seller's output) and never
//! from the relayer's word. The property it exists for: **the inscribed sat lands on an output paying custody, and every sat
//! of that output comes from the seller's input or from our own ord-clean inputs**, so custody receives exactly the one
//! inscription (ordinal theory's first-in-first-out assignment). The rest is the money: what leaves the float is at most the
//! cap, the miner fee is within the policy's ceiling, and the ETH cost is within the policy's per-purchase limits.
//!
//! Also here: the frog's record ([`FrogRec`]), sats ↔ wei at the rate recorded with the frog, confirmation depth, and the
//! collection's Merkle proof (`ListingRegistry.isMember`).

use crate::custody::{Outpoint, DUST_P2TR, MAX_DELIVERY_FEE_SATS};
use crate::psbt::{Psbt, Witness};

/// Six Bitcoin blocks: the depth at which a frog is believed in custody (docs/v3/sweep.md, 1a).
pub const CONFIRMATIONS: u64 = 6;
/// The ETH-to-BTC conversion the reimbursement must cover when it is turned back into float (the doc's ≈2%), basis points.
pub const CONVERSION_BPS: u128 = 200;
/// A purchase plus six confirmations, generously: a purchase is started only with more than this (and the mint margin) left.
pub const PURCHASE_LATENCY_SECS: u64 = 3 * 60 * 60;
/// No `wrapDeposit` to a target with less than this left on either clock: at least the sequencer's one-hour lead
/// (src/lib/Wait.sol), so a mint cannot be overtaken by the target's own deadline.
pub const MINT_MARGIN_SECS: u64 = 60 * 60;
/// The parent chain's block time, as `Wait.blocksFor` divides (src/lib/Wait.sol).
pub const PARENT_BLOCK_SECONDS: u64 = 12;

const WEI: u128 = 1_000_000_000_000_000_000;
const BPS: u128 = 10_000;

// ---- money ------------------------------------------------------------------------------------------------------------------

/// The ETH a frog costs us: `⌈sats × 1e18 / rate⌉`, plus [`CONVERSION_BPS`], rounded up. `rate` is sats per ETH.
pub fn wei_for_sats(sats: u64, sats_per_eth: u64) -> Result<u128, String> {
    if sats_per_eth == 0 {
        return Err("no rate".into());
    }
    let base = (sats as u128).checked_mul(WEI).ok_or("wei: overflow")?.div_ceil(sats_per_eth as u128);
    Ok(base.checked_mul(BPS + CONVERSION_BPS).ok_or("wei: overflow")?.div_ceil(BPS))
}

/// The most sats a frog may take from the float when its ETH cost may be at most `wei`: the inverse of [`wei_for_sats`],
/// rounded down, so `wei_for_sats(sats_for_wei(w)) ≤ w` (checked in the tests).
pub fn sats_for_wei(wei: u128, sats_per_eth: u64) -> Result<u64, String> {
    let net = wei.checked_mul(BPS).ok_or("sats: overflow")? / (BPS + CONVERSION_BPS);
    let sats = net.checked_mul(sats_per_eth as u128).ok_or("sats: overflow")? / WEI;
    let mut s = u64::try_from(sats).map_err(|_| "sats: out of range".to_string())?;
    // the two roundings can meet one unit apart; step down until the cost fits
    while s > 0 && wei_for_sats(s, sats_per_eth)? > wei {
        s -= 1;
    }
    Ok(s)
}

/// The most a frog purchase may pay the miner, absolutely: the delivery ceiling less room for the float's conflict-out that may
/// have to replace a stale buy (audit R2, N-5; BIP125 asks the replacement for the buy's fee plus its own ≤ 154 vB), as a bump
/// is capped.
pub const MAX_BUY_FEE_SATS: u64 = MAX_DELIVERY_FEE_SATS - 500;

/// The miner fee a frog purchase may pay: within `policy::MAX_COST_BPS` of the spend once the conversion allowance is taken
/// out of it, and never over [`MAX_BUY_FEE_SATS`].
pub fn fee_bound(spend: u64) -> u64 {
    let share = crate::policy::MAX_COST_BPS.saturating_sub(CONVERSION_BPS);
    ((spend as u128 * share / BPS) as u64).min(MAX_BUY_FEE_SATS)
}

/// `policy.rs`'s per-purchase limits, applied to a frog's ETH cost. Unchanged: the global maximum, for every sink that has no
/// collection limit (docs/v5/floor-feed.md §6.3).
pub fn check_limits(cost_wei: u128) -> Result<(), String> {
    if cost_wei < crate::policy::MIN_PURCHASE_WEI {
        return Err(format!("a frog costing {cost_wei} wei is under the {} minimum", crate::policy::MIN_PURCHASE_WEI));
    }
    if cost_wei > crate::policy::MAX_PURCHASE_WEI {
        return Err(format!("a frog costing {cost_wei} wei is over the {} per-purchase maximum", crate::policy::MAX_PURCHASE_WEI));
    }
    Ok(())
}

/// The per-purchase limits for one collection: the policy's minimum, and the collection's own maximum when a controller set
/// one (`floortrigger::FloorState::set_limit`, at most `PURCHASE_LIMIT_CEILING_WEI`); without one, exactly [`check_limits`].
pub fn check_limits_for(cost_wei: u128, limit: Option<u128>) -> Result<(), String> {
    let Some(max) = limit else { return check_limits(cost_wei) };
    if cost_wei < crate::policy::MIN_PURCHASE_WEI {
        return Err(format!("a purchase costing {cost_wei} wei is under the {} minimum", crate::policy::MIN_PURCHASE_WEI));
    }
    if cost_wei > max.min(crate::floortrigger::PURCHASE_LIMIT_CEILING_WEI) {
        return Err(format!("a purchase costing {cost_wei} wei is over this collection's {max} per-purchase maximum"));
    }
    Ok(())
}

// ---- depth --------------------------------------------------------------------------------------------------------------------

/// Blocks deep a transaction mined at `height` is when the tip is `tip` (1 when it is the tip).
pub fn depth(tip: u64, height: u64) -> u64 {
    if tip >= height { tip - height + 1 } else { 0 }
}

// ---- the collection -----------------------------------------------------------------------------------------------------------

/// `ListingRegistry.leafOf`: `keccak256(bytes.concat(keccak256(bytes(id))))`.
pub fn leaf_of(inscription_id: &str) -> [u8; 32] {
    crate::evmtx::keccak(&crate::evmtx::keccak(inscription_id.as_bytes()))
}

/// OpenZeppelin's `MerkleProof.verify`: sorted pairs, keccak256.
pub fn verify_proof(proof: &[[u8; 32]], root: &[u8; 32], leaf: &[u8; 32]) -> bool {
    let mut h = *leaf;
    for p in proof {
        let (a, b) = if h <= *p { (h, *p) } else { (*p, h) };
        h = crate::evmtx::keccak(&[a, b].concat());
    }
    h == *root
}

// ---- judging an offer ------------------------------------------------------------------------------------------------------

/// Where ord says the inscription is, confirmed by every explorer: the seller's output, the inscribed sat's offset in it, and
/// the output's value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seller {
    pub outpoint: Outpoint,
    pub offset: u64,
    pub value: u64,
}

/// What an offer is judged against. Every field is the canister's own reading.
#[derive(Debug, Clone)]
pub struct Facts<'a> {
    /// The float's script: our inputs carry it, our change returns to it.
    pub float: &'a [u8],
    /// Where the frog must land.
    pub custody: &'a [u8],
    pub seller: Seller,
    /// The most that may leave the float, fee included.
    pub cap_sats: u64,
}

/// What a passed judgement says: which inputs we sign, where the frog lands, and what it cost.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Judged {
    pub ours: Vec<usize>,
    pub custody_vout: u32,
    /// Our inputs less our change: the price, the marketplace's fees and the miner fee.
    pub spend: u64,
    pub fee: u64,
}

/// Why an offer is refused. Each is a reason not to sign.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refused {
    /// An input we were asked to sign does not carry the float's script.
    ForeignInputToSign(usize),
    /// The PSBT does not say what an input spends, so the sat flow cannot be traced.
    UnknownInputValue(usize),
    /// An input pays the custody script: a purchase never spends custody.
    SpendsCustody(usize),
    /// No input spends the seller's outpoint, or more than one claims to.
    SellerInput(usize),
    /// The seller's input is not worth what ord and the explorers say.
    SellerValue { psbt: u64, read: u64 },
    NegativeFee,
    OverCap { spend: u64, cap: u64 },
    FeeTooHigh { fee: u64, bound: u64 },
    /// An `OP_RETURN` output: a runestone could put runes on the frog's output.
    OpReturn(usize),
    /// The inscribed sat does not land on the custody output (`None`: it goes to the miner).
    NotToCustody(Option<usize>),
    /// Not exactly one output pays custody.
    CustodyOutputs(usize),
    /// The custody output would receive sats from a foreign input other than the seller's.
    ForeignSatsToCustody(usize),
    /// The custody output is under the dust limit.
    CustodyDust(u64),
    /// A foreign input whose witness is not a plain key-path or P2WPKH signature (a script path could reveal an inscription).
    ScriptPath(usize),
    /// An input that is not the float's carries no final witness (`PSBT_IN_FINAL_SCRIPTWITNESS`): our signatures alone could
    /// never complete the transaction, so nothing is signed (review S-01; `psbt::finalize` refuses the same, but only after
    /// the float has signed).
    NotFinal(usize),
    /// A float input already carries a final witness: not ours to sign over (`psbt::finalize` refuses it too, after signing).
    InputSigned(usize),
    /// The seller's signature is not `SIGHASH_SINGLE|ANYONECANPAY` (`0x83`), the only listing shape we complete
    /// (docs/v8/ordinals-wave.md §1.3); `0x00` is a 64-byte `SIGHASH_DEFAULT` key-path signature.
    SellerSighash { input: usize, sighash: u8 },
}

fn is_p2tr(s: &[u8]) -> bool {
    s.len() == 34 && s[0] == 0x51 && s[1] == 0x20
}

/// Judge a marketplace PSBT for one inscription. `witnesses` are the PSBT's own final witnesses (the seller's signature),
/// `to_sign` what the venue asks us to sign.
pub fn judge(psbt: &Psbt, witnesses: &[Option<Witness>], to_sign: &[usize], f: &Facts) -> Result<Judged, Refused> {
    let tx = &psbt.tx;
    // 1. every input's value is known: the flow of sats cannot be traced otherwise
    let mut values = Vec::with_capacity(tx.inputs.len());
    for (i, u) in psbt.witness_utxos.iter().enumerate() {
        match u {
            Some(u) => values.push(u.value),
            None => return Err(Refused::UnknownInputValue(i)),
        }
    }
    // 2. ours are the float's; we sign only ours; nothing spends custody
    let ours = psbt.our_inputs(f.float);
    for &i in to_sign {
        if !ours.contains(&i) {
            return Err(Refused::ForeignInputToSign(i));
        }
    }
    for (i, u) in psbt.witness_utxos.iter().enumerate() {
        if u.as_ref().is_some_and(|u| u.script == f.custody) {
            return Err(Refused::SpendsCustody(i));
        }
    }
    // 3. exactly one input is the seller's, worth what was read
    let seller_in: Vec<usize> = tx.inputs.iter().enumerate()
        .filter(|(_, i)| crate::custody::outpoint_of(i) == f.seller.outpoint).map(|(i, _)| i).collect();
    if seller_in.len() != 1 {
        return Err(Refused::SellerInput(seller_in.len()));
    }
    let s = seller_in[0];
    if values[s] != f.seller.value {
        return Err(Refused::SellerValue { psbt: values[s], read: f.seller.value });
    }
    // 4. no OP_RETURN at all
    if let Some(i) = tx.outputs.iter().position(|o| o.script.first() == Some(&0x6a)) {
        return Err(Refused::OpReturn(i));
    }
    // 5. the money
    let in_total: u64 = values.iter().fold(0u64, |a, v| a.saturating_add(*v));
    let out_total: u64 = tx.outputs.iter().fold(0u64, |a, o| a.saturating_add(o.value));
    if out_total > in_total {
        return Err(Refused::NegativeFee);
    }
    let fee = in_total - out_total;
    let ours_in: u64 = ours.iter().fold(0u64, |a, &i| a.saturating_add(values[i]));
    let ours_out: u64 = tx.outputs.iter().filter(|o| o.script == f.float).fold(0u64, |a, o| a.saturating_add(o.value));
    let spend = ours_in.saturating_sub(ours_out);
    if spend > f.cap_sats {
        return Err(Refused::OverCap { spend, cap: f.cap_sats });
    }
    let bound = fee_bound(spend);
    if fee > bound {
        return Err(Refused::FeeTooHigh { fee, bound });
    }
    // 6. exactly one output pays custody, above dust
    let to_custody: Vec<usize> = tx.outputs.iter().enumerate().filter(|(_, o)| o.script == f.custody).map(|(i, _)| i).collect();
    if to_custody.len() != 1 {
        return Err(Refused::CustodyOutputs(to_custody.len()));
    }
    let c = to_custody[0];
    if tx.outputs[c].value < DUST_P2TR {
        return Err(Refused::CustodyDust(tx.outputs[c].value));
    }
    // 7. the sat flow: the inscribed sat's absolute position, and the output whose range holds it
    let seller_start: u64 = values[..s].iter().sum();
    let pos = seller_start + f.seller.offset;
    let mut start = 0u64;
    let mut landed = None;
    for (j, o) in tx.outputs.iter().enumerate() {
        if pos >= start && pos < start + o.value {
            landed = Some(j);
            break;
        }
        start += o.value;
    }
    if landed != Some(c) {
        return Err(Refused::NotToCustody(landed));
    }
    // 8. every input overlapping the custody output's range is the seller's or ours
    let c_start: u64 = tx.outputs[..c].iter().map(|o| o.value).sum();
    let c_end = c_start + tx.outputs[c].value;
    let mut a = 0u64;
    for (i, v) in values.iter().enumerate() {
        let b = a + v;
        if a < c_end && b > c_start && i != s && !ours.contains(&i) {
            return Err(Refused::ForeignSatsToCustody(i));
        }
        a = b;
    }
    // 9. every input not ours is already FINAL, with a plain signature: a key path (one 64/65-byte item) or P2WPKH (a signature
    //    and a 33-byte key); ours carry nothing yet; the seller signed SIGHASH_SINGLE|ANYONECANPAY. Judged here, before any
    //    digest is computed or the float key asked (review S-01): an offer we could sign but never complete is refused, not
    //    signed and then dropped at `psbt::finalize`.
    if witnesses.len() != tx.inputs.len() {
        return Err(Refused::NotFinal(witnesses.len().min(tx.inputs.len())));
    }
    for (i, w) in witnesses.iter().enumerate() {
        if ours.contains(&i) {
            if w.is_some() {
                return Err(Refused::InputSigned(i));
            }
            continue;
        }
        let Some(w) = w else {
            return Err(Refused::NotFinal(i));
        };
        let key_path = w.len() == 1 && (w[0].len() == 64 || w[0].len() == 65);
        let wpkh = w.len() == 2 && w[1].len() == 33 && !psbt.witness_utxos[i].as_ref().is_some_and(|u| is_p2tr(&u.script));
        if !key_path && !wpkh {
            return Err(Refused::ScriptPath(i));
        }
        if i == s {
            // the hash type is the signature's last byte (key path: the 65th; P2WPKH: after the DER signature); 64 bytes: DEFAULT
            let sighash = if key_path && w[0].len() == 64 { 0x00 } else { w[0].last().copied().unwrap_or(0) };
            if sighash != crate::sighash::SIGHASH_SINGLE_ANYONECANPAY {
                return Err(Refused::SellerSighash { input: i, sighash });
            }
        }
    }
    Ok(Judged { ours, custody_vout: c as u32, spend, fee })
}

/// A PSBT's bytes from its parts, with final witnesses where given: what a marketplace hands over, for tests and rehearsals
/// (the canister itself only ever parses).
pub fn psbt_bytes(p: &Psbt, finals: &[Option<Witness>]) -> Vec<u8> {
    fn compact(n: usize) -> Vec<u8> {
        match n {
            0..=0xfc => vec![n as u8],
            0xfd..=0xffff => { let mut v = vec![0xfd]; v.extend_from_slice(&(n as u16).to_le_bytes()); v }
            _ => { let mut v = vec![0xfe]; v.extend_from_slice(&(n as u32).to_le_bytes()); v }
        }
    }
    fn out(o: &crate::psbt::TxOut) -> Vec<u8> {
        let mut b = o.value.to_le_bytes().to_vec();
        b.extend(compact(o.script.len()));
        b.extend_from_slice(&o.script);
        b
    }
    let tx = &p.tx;
    let mut raw = tx.version.to_le_bytes().to_vec();
    raw.extend(compact(tx.inputs.len()));
    for i in &tx.inputs {
        raw.extend_from_slice(&i.txid);
        raw.extend_from_slice(&i.vout.to_le_bytes());
        raw.push(0);
        raw.extend_from_slice(&i.sequence.to_le_bytes());
    }
    raw.extend(compact(tx.outputs.len()));
    for o in &tx.outputs {
        raw.extend(out(o));
    }
    raw.extend_from_slice(&tx.locktime.to_le_bytes());
    let mut b = b"psbt\xff".to_vec();
    b.extend([1u8, 0x00]);
    b.extend(compact(raw.len()));
    b.extend(raw);
    b.push(0);
    for (i, u) in p.witness_utxos.iter().enumerate() {
        if let Some(u) = u {
            b.extend([1u8, 0x01]);
            let v = out(u);
            b.extend(compact(v.len()));
            b.extend(v);
        }
        if let Some(Some(w)) = finals.get(i) {
            let mut v = compact(w.len());
            for item in w {
                v.extend(compact(item.len()));
                v.extend_from_slice(item);
            }
            b.extend([1u8, 0x08]);
            b.extend(compact(v.len()));
            b.extend(v);
        }
        b.push(0);
    }
    for _ in &tx.outputs {
        b.push(0);
    }
    b
}

// ---- the record --------------------------------------------------------------------------------------------------------------

/// Who a frog is for. `None` on the record: inventory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub enum Target {
    /// An item sink (a collection's buy queue, or a graduating curve): minted to it, then reported (`itemsink.rs`).
    Sink([u8; 20]),
    /// A mint-desk request: minted to the desk, then `fulfill(id, cost)`.
    Desk(u64),
}

/// A frog bought: what it is, where it landed, what it cost. Fixed when the offer was accepted.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub struct Bought {
    pub inscription: String,
    /// The custody output, display order.
    pub outpoint: Outpoint,
    pub seller: Outpoint,
    pub spend_sats: u64,
    pub fee_sats: u64,
    /// The reference rate at acceptance, sats per ETH.
    pub rate: u64,
    /// What the report (`recordItem` / `recordFrog` / `fulfill`) is asked for.
    pub cost_wei: u128,
    /// The signed purchase, saved before it was first broadcast: the only bytes ever sent again (a reorganisation or an
    /// eviction sends these, never a new signature).
    pub raw: Vec<u8>,
    /// docs/v8/ow-protected.md §3.3: a protected buy's setup transaction (the float's own coins split in two for the sale), display
    /// order. Every float spend must be one the canister recorded (the monitor's canary pages any other as an emergency), and the
    /// setup is one. None for a classic buy and for a protected one without a setup.
    #[serde(default)]
    pub setup_txid: Option<[u8; 32]>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub enum FPhase {
    /// Waiting for an offer the canister can accept, for at most `cap_wei`.
    Wanted { cap_wei: u128 },
    /// Signed and saved (`buy.raw`); the same bytes go out until Bitcoin shows them. The txid is `buy.outpoint.txid`.
    Signed { buy: Bought },
    /// Bitcoin has it; waiting for six blocks and ord.
    Sent { buy: Bought },
    /// Six deep, and ord shows custody holding exactly it. Not minted: it can still go to any target.
    Held { buy: Bought },
    /// Minted to the target; its report (`recordItem` / `recordFrog` / `fulfill`) is owed.
    Minted { buy: Bought },
    /// Reimbursed. Finished.
    Settled { buy: Bought },
    /// Minted to the desk and its request expired before `fulfill`: an unassigned unit at the desk, ours to use for the next.
    Stray { buy: Bought },
    /// Minted to a sink that could no longer take it before it was reported (a curve that opened refunds): its wFROG is
    /// the sink's, its cost is lost to the float. Loud.
    Stranded { buy: Bought, why: String },
    /// Nothing of ours was spent (no acceptable offer, the listing vanished). Finished.
    Failed { why: String },
    /// A buy that never confirmed within `frogflow::BUY_STALE_SECS` (audit D, L-5): its signed bytes are being killed by spending
    /// one of the float's inputs back to the float (H8's conflict machinery, with the float key). Ends `Failed` (the conflict,
    /// or another spend, confirmed: nothing was bought) or `Sent` (the buy confirmed first: it was bought after all).
    Abandoning { buy: Bought, conflict: crate::conflict::Conflict },
    /// A protected Ordinals Wallet sale (docs/v8/ow-protected.md §3.3), signed on the float's inputs only and handed back to the
    /// runner, which submits it; Ordinals Wallet co-signs and broadcasts it, the canister cannot. `sale_txid` (display order hex,
    /// `buy.outpoint.txid`) is watched each tick: seen, `Sent`; not seen within `frogflow::PROTECTED_STALE_SECS` of `at`
    /// (seconds), abandoned as a stale buy is. Nothing is ever broadcast in this phase. The signed PSBTs and what the stale path
    /// needs are kept beside it (`frogflow::ProtectedRec`). `buy.raw` is empty: the canister never holds the final bytes.
    Handed { buy: Bought, sale_txid: String, at: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub struct FrogRec {
    pub id: u64,
    pub target: Option<Target>,
    pub phase: FPhase,
}

impl FrogRec {
    pub fn finished(&self) -> bool {
        matches!(self.phase, FPhase::Settled { .. } | FPhase::Stranded { .. } | FPhase::Failed { .. })
    }
    /// A Bitcoin purchase in flight: the float is committed (or about to be).
    pub fn acquiring(&self) -> bool {
        matches!(self.phase, FPhase::Wanted { .. } | FPhase::Signed { .. } | FPhase::Sent { .. } | FPhase::Abandoning { .. }
            | FPhase::Handed { .. })
    }
    pub fn bought(&self) -> Option<&Bought> {
        match &self.phase {
            FPhase::Signed { buy } | FPhase::Sent { buy } | FPhase::Held { buy } | FPhase::Minted { buy }
            | FPhase::Settled { buy } | FPhase::Stray { buy } | FPhase::Stranded { buy, .. } | FPhase::Abandoning { buy, .. }
            | FPhase::Handed { buy, .. } => Some(buy),
            _ => None,
        }
    }
    /// A frog minted by us (so the Wrapper counts it held): for finding a deposit id's inscription at redemption.
    pub fn minted(&self) -> bool {
        matches!(self.phase, FPhase::Minted { .. } | FPhase::Settled { .. } | FPhase::Stray { .. } | FPhase::Stranded { .. })
    }
    /// What this frog commits of its target's budget: its cost once known, its cap while wanted.
    pub fn committed_wei(&self) -> u128 {
        match &self.phase {
            FPhase::Wanted { cap_wei } => *cap_wei,
            _ => self.bought().map_or(0, |b| b.cost_wei),
        }
    }
}

// ---- the outspend, for "the listing vanished" -------------------------------------------------------------------------------

/// esplora's `/tx/<txid>/outspend/<vout>`: `None` unspent; else the spending txid (display) and whether it is confirmed.
pub fn parse_outspend_full(status: u16, body: &[u8]) -> Result<Option<([u8; 32], bool)>, String> {
    if status != 200 {
        return Err(format!("explorer: HTTP {status}"));
    }
    let v: serde_json::Value = serde_json::from_slice(body).map_err(|_| "explorer: not JSON")?;
    if !v.get("spent").and_then(|x| x.as_bool()).ok_or("explorer: no spent field")? {
        return Ok(None);
    }
    let t = v.get("txid").and_then(|x| x.as_str()).ok_or("explorer: spent, but no txid")?;
    let txid: [u8; 32] = hex::decode(t).ok().and_then(|b| b.try_into().ok()).ok_or("explorer: an unreadable spending txid")?;
    let confirmed = v.get("status").and_then(|s| s.get("confirmed")).and_then(|c| c.as_bool()).unwrap_or(false);
    Ok(Some((txid, confirmed)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::psbt::{Tx, TxIn, TxOut};

    fn script(tag: u8) -> Vec<u8> {
        let mut s = vec![0x51, 0x20];
        s.extend([tag; 32]);
        s
    }
    const FLOAT: u8 = 1;
    const CUSTODY: u8 = 2;
    const SELLER_PAY: u8 = 3;
    const OTHER: u8 = 4;

    fn inp(tag: u8, vout: u32) -> TxIn {
        TxIn { txid: [tag; 32], vout, sequence: 0xffff_fffd }
    }
    fn seller_op() -> Outpoint {
        crate::custody::outpoint_of(&inp(0xee, 0))
    }
    /// The shape marketplaces use: two small float inputs (padding), the seller's frog, a float payment input; out: the
    /// padding merged to the float, the frog to custody, the seller's payment, change to the float.
    fn offer(frog_offset: u64) -> (Psbt, Vec<Option<Witness>>) {
        let tx = Tx {
            version: 2,
            inputs: vec![inp(0xa1, 0), inp(0xa2, 0), inp(0xee, 0), inp(0xa3, 1)],
            outputs: vec![
                TxOut { value: 1_200, script: script(FLOAT) },
                TxOut { value: 10_000, script: script(CUSTODY) },
                TxOut { value: 2_000_000, script: script(SELLER_PAY) },
                TxOut { value: 7_997_000, script: script(FLOAT) },
            ],
            locktime: 0,
        };
        let _ = frog_offset;
        let utxos = vec![
            Some(TxOut { value: 600, script: script(FLOAT) }),
            Some(TxOut { value: 600, script: script(FLOAT) }),
            Some(TxOut { value: 10_000, script: script(0xee) }),
            Some(TxOut { value: 10_000_000, script: script(FLOAT) }),
        ];
        (Psbt { tx, witness_utxos: utxos }, vec![None, None, Some(vec![seller_sig()]), None])
    }
    /// The seller's key-path signature under `SIGHASH_SINGLE|ANYONECANPAY`: 64 bytes and the hash type.
    fn seller_sig() -> Vec<u8> {
        let mut s = vec![7u8; 65];
        s[64] = 0x83;
        s
    }
    fn facts<'a>(float: &'a [u8], custody: &'a [u8], offset: u64, cap: u64) -> Facts<'a> {
        Facts { float, custody, seller: Seller { outpoint: seller_op(), offset, value: 10_000 }, cap_sats: cap }
    }

    #[test]
    fn an_honest_listing_is_judged_and_the_frog_lands_on_custody() {
        let (p, w) = offer(0);
        let (f, c) = (script(FLOAT), script(CUSTODY));
        let j = judge(&p, &w, &[0, 1, 3], &facts(&f, &c, 0, 2_100_000)).unwrap();
        assert_eq!(j.ours, vec![0, 1, 3]);
        assert_eq!(j.custody_vout, 1);
        // in 10,011,200; out 10,008,200: fee 3,000; ours in 10,001,200, ours back 7,998,200
        assert_eq!((j.spend, j.fee), (2_003_000, 3_000));
        // anywhere in the seller's 10,000 sats still lands in custody
        assert!(judge(&p, &w, &[0], &facts(&f, &c, 9_999, 2_100_000)).is_ok());
    }

    #[test]
    fn the_inscribed_sat_must_land_on_custody_and_nothing_foreign_beside_it() {
        let (f, c) = (script(FLOAT), script(CUSTODY));
        // the padding shrinks to one output of 600: the first 600 sats of the frog now go to the float's output
        let (mut p, w) = offer(0);
        p.tx.outputs[0].value = 1_800;
        p.tx.outputs[3].value -= 600;
        assert_eq!(judge(&p, &w, &[0], &facts(&f, &c, 0, 2_100_000)), Err(Refused::NotToCustody(Some(0))));
        // a foreign input slipped in front of the frog, its sats flowing into custody's range
        let (mut p, mut w) = offer(0);
        p.tx.inputs.insert(2, inp(0xbb, 0));
        p.witness_utxos.insert(2, Some(TxOut { value: 546, script: script(OTHER) }));
        w.insert(2, Some(vec![vec![1u8; 64]]));
        p.tx.outputs[0].value = 1_200;
        p.tx.outputs[1].value = 10_546;
        assert_eq!(judge(&p, &w, &[0], &facts(&f, &c, 0, 2_100_000)), Err(Refused::ForeignSatsToCustody(2)));
        // the frog to the miner: outputs end before it
        let (mut p, w) = offer(0);
        p.tx.outputs.remove(1);
        p.tx.outputs.insert(1, TxOut { value: 5_000, script: script(CUSTODY) });
        assert!(matches!(judge(&p, &w, &[0], &facts(&f, &c, 9_000, u64::MAX)), Err(Refused::NotToCustody(_))));
    }

    #[test]
    fn the_money_is_capped_and_the_fee_is_within_the_policy_ceiling() {
        let (f, c) = (script(FLOAT), script(CUSTODY));
        let (p, w) = offer(0);
        assert_eq!(judge(&p, &w, &[0], &facts(&f, &c, 0, 2_002_999)), Err(Refused::OverCap { spend: 2_003_000, cap: 2_002_999 }));
        // a fee over 3% of the spend (5% less the 2% conversion allowance) is refused
        let (mut p, w) = offer(0);
        p.tx.outputs[3].value -= 80_000;
        let r = judge(&p, &w, &[0], &facts(&f, &c, 0, u64::MAX)).unwrap_err();
        assert!(matches!(r, Refused::FeeTooHigh { fee: 83_000, .. }), "{r:?}");
        assert_eq!(fee_bound(1_000_000), 24_500, "and never over custody's absolute ceiling less room for the abandon (R2 N-5)");
        assert_eq!(fee_bound(100_000), 3_000);
    }

    #[test]
    fn what_is_signed_is_ours_only_and_nothing_of_custody_is_ever_an_input() {
        let (f, c) = (script(FLOAT), script(CUSTODY));
        let (p, w) = offer(0);
        assert_eq!(judge(&p, &w, &[2], &facts(&f, &c, 0, u64::MAX)), Err(Refused::ForeignInputToSign(2)), "the seller's input");
        let (mut p, w) = offer(0);
        p.witness_utxos[1] = Some(TxOut { value: 600, script: script(CUSTODY) });
        assert_eq!(judge(&p, &w, &[0], &facts(&f, &c, 0, u64::MAX)), Err(Refused::SpendsCustody(1)));
        let (mut p, w) = offer(0);
        p.witness_utxos[3] = None;
        assert_eq!(judge(&p, &w, &[0], &facts(&f, &c, 0, u64::MAX)), Err(Refused::UnknownInputValue(3)));
    }

    #[test]
    fn the_seller_input_is_the_one_ord_named_worth_what_ord_said() {
        let (f, c) = (script(FLOAT), script(CUSTODY));
        let (mut p, w) = offer(0);
        p.tx.inputs[2] = inp(0xef, 0);
        assert_eq!(judge(&p, &w, &[0], &facts(&f, &c, 0, u64::MAX)), Err(Refused::SellerInput(0)), "listing vanished or swapped");
        let (mut p, w) = offer(0);
        p.witness_utxos[2] = Some(TxOut { value: 9_000, script: script(0xee) });
        assert_eq!(judge(&p, &w, &[0], &facts(&f, &c, 0, u64::MAX)), Err(Refused::SellerValue { psbt: 9_000, read: 10_000 }));
    }

    #[test]
    fn no_op_return_one_custody_output_above_dust_and_no_script_paths() {
        let (f, c) = (script(FLOAT), script(CUSTODY));
        let (mut p, w) = offer(0);
        p.tx.outputs.push(TxOut { value: 0, script: vec![0x6a, 0x5d, 0x01, 0x00] });
        assert_eq!(judge(&p, &w, &[0], &facts(&f, &c, 0, u64::MAX)), Err(Refused::OpReturn(4)), "a runestone");
        let (mut p, w) = offer(0);
        p.tx.outputs[3].value -= 1_000;
        p.tx.outputs.push(TxOut { value: 1_000, script: script(CUSTODY) });
        assert_eq!(judge(&p, &w, &[0], &facts(&f, &c, 0, u64::MAX)), Err(Refused::CustodyOutputs(2)));
        let (p, mut w) = offer(0);
        w[2] = Some(vec![vec![1u8; 64], vec![0x20; 40], vec![0xc0; 33]]);
        assert_eq!(judge(&p, &w, &[0], &facts(&f, &c, 0, u64::MAX)), Err(Refused::ScriptPath(2)));
    }

    /// Review S-01 (ops/rehearsal/sim, 2026-10-09): every input not ours must already be final, the seller's under `0x83`, and
    /// ours must carry nothing, or the offer is refused before the float key is asked for anything.
    #[test]
    fn an_offer_we_could_sign_but_never_complete_is_refused_by_the_judge() {
        let (f, c) = (script(FLOAT), script(CUSTODY));
        // the seller's witness stripped: a marketplace keeping it server-side
        let (p, mut w) = offer(0);
        w[2] = None;
        assert_eq!(judge(&p, &w, &[0, 1, 3], &facts(&f, &c, 0, u64::MAX)), Err(Refused::NotFinal(2)));
        // a foreign, non-seller input with no witness
        let (mut p, mut w) = offer(0);
        p.tx.inputs.push(inp(0xbb, 0));
        p.witness_utxos.push(Some(TxOut { value: 546, script: script(OTHER) }));
        p.tx.outputs[3].value += 546;
        w.push(None);
        assert_eq!(judge(&p, &w, &[0], &facts(&f, &c, 0, u64::MAX)), Err(Refused::NotFinal(4)));
        w[4] = Some(vec![vec![1u8; 64]]);
        assert!(judge(&p, &w, &[0], &facts(&f, &c, 0, u64::MAX)).is_ok(), "a third party's own final witness is theirs to choose");
        // the seller signed something other than SINGLE|ANYONECANPAY
        let (p, mut w) = offer(0);
        w[2] = Some(vec![vec![7u8; 64]]);
        assert_eq!(judge(&p, &w, &[0], &facts(&f, &c, 0, u64::MAX)), Err(Refused::SellerSighash { input: 2, sighash: 0x00 }));
        let mut all = vec![7u8; 65];
        all[64] = 0x01;
        w[2] = Some(vec![all]);
        assert_eq!(judge(&p, &w, &[0], &facts(&f, &c, 0, u64::MAX)), Err(Refused::SellerSighash { input: 2, sighash: 0x01 }));
        // one of ours already carries a final witness
        let (p, mut w) = offer(0);
        w[1] = Some(vec![vec![1u8; 64]]);
        assert_eq!(judge(&p, &w, &[0], &facts(&f, &c, 0, u64::MAX)), Err(Refused::InputSigned(1)));
        // a witness list that does not cover every input
        let (p, mut w) = offer(0);
        w.pop();
        assert_eq!(judge(&p, &w, &[0], &facts(&f, &c, 0, u64::MAX)), Err(Refused::NotFinal(3)));
    }

    #[test]
    fn a_psbt_written_by_psbt_bytes_parses_back_to_itself() {
        let (p, w) = offer(0);
        let b = psbt_bytes(&p, &w);
        assert_eq!(Psbt::parse(&b).unwrap(), p);
        assert_eq!(crate::psbt::final_witnesses(&b).unwrap(), w);
    }

    #[test]
    fn sats_and_wei_round_against_us_never_against_the_payer() {
        let rate = 3_190_000; // sats per ETH
        // 2,003,000 sats at 3.19M/ETH is 0.6279 ETH; plus 2% is 0.6404 ETH
        let w = wei_for_sats(2_003_000, rate).unwrap();
        assert!(w > 640_000_000_000_000_000 && w < 641_000_000_000_000_000, "{w}");
        for cap in [1u128, 50_000_000_000_000_000, 646_700_000_000_000_000, 5 * WEI] {
            let s = sats_for_wei(cap, rate).unwrap();
            assert!(wei_for_sats(s, rate).unwrap() <= cap, "a cap in sats never costs more than the cap in wei");
            assert!(wei_for_sats(s + 1, rate).unwrap() > cap || s == 0, "and it is the most that fits");
        }
        assert!(wei_for_sats(1, 0).is_err());
    }

    #[test]
    fn the_policy_limits_apply_to_every_frog_and_the_docs_example_is_over_the_maximum() {
        assert!(check_limits(crate::policy::MIN_PURCHASE_WEI - 1).unwrap_err().contains("minimum"));
        assert!(check_limits(crate::policy::MIN_PURCHASE_WEI).is_ok());
        assert!(check_limits(crate::policy::MAX_PURCHASE_WEI).is_ok());
        // docs/v3/curve.md's worked example: ≈0.6467 ETH a frog. Stated in the doc: refused by MAX_PURCHASE_WEI.
        assert!(check_limits(646_700_000_000_000_000).unwrap_err().contains("maximum"));
    }

    #[test]
    fn a_collection_limit_lets_its_own_items_through_and_nothing_else_changes() {
        // no limit: exactly the global policy
        for c in [crate::policy::MIN_PURCHASE_WEI - 1, crate::policy::MIN_PURCHASE_WEI, crate::policy::MAX_PURCHASE_WEI, 646_700_000_000_000_000] {
            assert_eq!(check_limits_for(c, None), check_limits(c), "{c}");
        }
        // a frog collection limited at 0.7 ETH buys the 0.6467 ETH frog; an ape collection at 6 ETH buys a 5.1 ETH ape
        assert!(check_limits_for(646_700_000_000_000_000, Some(700_000_000_000_000_000)).is_ok());
        assert!(check_limits_for(5_100_000_000_000_000_000, Some(6 * WEI)).is_ok());
        assert!(check_limits_for(6 * WEI + 1, Some(6 * WEI)).unwrap_err().contains("this collection's"));
        assert!(check_limits_for(crate::policy::MIN_PURCHASE_WEI - 1, Some(6 * WEI)).unwrap_err().contains("minimum"));
        // and the code ceiling binds whatever limit was stored
        assert!(check_limits_for(crate::floortrigger::PURCHASE_LIMIT_CEILING_WEI + 1, Some(u128::MAX)).is_err());
    }

    #[test]
    fn depth_and_the_merkle_proof_match_the_contracts() {
        assert_eq!((depth(100, 95), depth(100, 100), depth(100, 101)), (6, 1, 0));
        // `cast keccak $(cast keccak "<id>")`
        let id = "6fb976ab49dcec017f1e201e84395983204ae1a7c2abf7ced0a85d692e442799i0";
        assert_eq!(hex::encode(leaf_of(id)), "ca44e38150628e3cf88c87d95f5dde92d124579736dc3005d10543df48ce4f46");
        // a four-leaf tree built the OpenZeppelin way (sorted pairs)
        let leaves: Vec<[u8; 32]> = (0..4).map(|i| leaf_of(&format!("{}i{i}", "ab".repeat(32)))).collect();
        let pair = |a: [u8; 32], b: [u8; 32]| { let (x, y) = if a <= b { (a, b) } else { (b, a) }; crate::evmtx::keccak(&[x, y].concat()) };
        let (l, r) = (pair(leaves[0], leaves[1]), pair(leaves[2], leaves[3]));
        let root = pair(l, r);
        assert!(verify_proof(&[leaves[3], l], &root, &leaves[2]));
        assert!(verify_proof(&[leaves[0], r], &root, &leaves[1]));
        assert!(!verify_proof(&[leaves[0], r], &root, &leaves[2]), "another leaf");
        // an inner node verifies as a "leaf" one level up: that is why the registry hashes an id twice, so no id's leaf is one
        assert!(verify_proof(&[l], &root, &r));
        assert!(!verify_proof(&[l], &root, &leaf_of("not an inner node")));
    }

    #[test]
    fn an_outspend_says_who_spent_it_and_whether_that_is_confirmed() {
        assert_eq!(parse_outspend_full(200, br#"{"spent":false}"#), Ok(None));
        let t = "11".repeat(32);
        let b = format!(r#"{{"spent":true,"txid":"{t}","vin":0,"status":{{"confirmed":true,"block_height":5}}}}"#);
        assert_eq!(parse_outspend_full(200, b.as_bytes()), Ok(Some(([0x11; 32], true))));
        assert!(parse_outspend_full(500, b"").is_err());
    }
}
