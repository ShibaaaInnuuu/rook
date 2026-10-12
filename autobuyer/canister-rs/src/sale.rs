//! Protocol sales and relist tickets (docs/v6/canister-floor-followups.md §3–§4): when a Floor Module's standing reserve is
//! low the canister sells one of the module's inventory items on the protocol's behalf and reports the real net proceeds
//! with `itemSold{value}`; a YOYO vault's `redeemToRelist` ticket is listed at the vault's ask and its sale reported with
//! `recordSale{value}`. An item is sold by listing it: a seller-signed PSBT under `SIGHASH_SINGLE | SIGHASH_ANYONECANPAY`
//! that pays custody the ask, which any ordinals venue takes as a listing.
//!
//! Pure: no network, no keys, no clock of its own. Here: the ABI (selectors pinned against keccak in the tests), the sale
//! policy a controller sets, the sale records and their phases, the reserve-low rule, the ask and the proceeds arithmetic,
//! the listing PSBT and its check, and how a fill is recognised. The sequencing is `saleflow.rs`, the endpoints `salenet.rs`.

use candid::CandidType;
use serde::{Deserialize, Serialize};

use crate::abi::{self, Arg};
use crate::custody::{Frog, Outpoint};
use crate::deposits::TxView;
use crate::floorsink::{FloorExtra, ONE};
use crate::frogbuy::CONVERSION_BPS;
use crate::itemsink::SinkView;
use crate::psbt::{Psbt, Tx, TxIn, TxOut};

// ---- the ABI ----------------------------------------------------------------------------------------------------------------------

/// `itemSold(uint256)` (the module, payable)
pub const ITEM_SOLD: [u8; 4] = [0xf0, 0x59, 0x6a, 0x79];
/// `redeemToRelist()` (both wrappers)
pub const REDEEM_TO_RELIST: [u8; 4] = [0x54, 0x4c, 0x61, 0xb0];
/// `recordSale(uint256)` (a YOYO vault, payable)
pub const RECORD_SALE: [u8; 4] = [0x6f, 0x27, 0xf6, 0x90];
/// `reclaimListing(uint256)` (a YOYO vault)
pub const RECLAIM_LISTING: [u8; 4] = [0xea, 0x5e, 0x13, 0xd4];
/// `listing(uint256)` (a YOYO vault): `(ListingStage stage, address coin, uint256 ask, uint256 proceeds)`
pub const LISTING: [u8; 4] = [0x84, 0xa1, 0x84, 0xde];
/// `wrapper()` (a vault)
pub const VAULT_WRAPPER: [u8; 4] = [0xac, 0x21, 0x0c, 0xc7];

/// `SweepVault.ListingStage`.
pub const LISTING_LISTED: u8 = 1;

// ---- the bounds ------------------------------------------------------------------------------------------------------------------

/// A protocol sale's markup over `price()`, at most this (the YOYO markup's own ceiling, docs/v5/policies-and-fees.md §1.1).
pub const MAX_MARKUP_BPS: u32 = 2_000;
pub const DEFAULT_MARKUP_BPS: u32 = 500;
/// Sales open at once for one module, at most.
pub const MAX_OPEN: u32 = 4;
/// Tickets walked per tick looking for relists; the rest next tick.
pub const MAX_TICKETS_READ_PER_TICK: u64 = 16;
/// What the canister's 4663 balance must hold beyond a report's value: the report's own gas.
pub const REPORT_GAS_MARGIN_WEI: u128 = 2_000_000_000_000_000; // 0.002 ETH
/// A move (a delist's conflicting spend) pays at most custody's own delivery ceiling.
pub const SIGHASH_LISTING: u8 = crate::sighash::SIGHASH_SINGLE_ANYONECANPAY;

const BPS: u128 = 10_000;
const WEI: u128 = 1_000_000_000_000_000_000;

// ---- encoders and decoders --------------------------------------------------------------------------------------------------------

pub fn item_sold(proceeds_wei: u128) -> Vec<u8> {
    abi::encode_call(ITEM_SOLD, &[Arg::Word(abi::uint_word(proceeds_wei))])
}
pub fn record_sale(ticket: u64) -> Vec<u8> {
    abi::encode_call(RECORD_SALE, &[Arg::Word(abi::uint_word(ticket as u128))])
}
pub fn reclaim_listing(ticket: u64) -> Vec<u8> {
    abi::encode_call(RECLAIM_LISTING, &[Arg::Word(abi::uint_word(ticket as u128))])
}
pub fn listing_call(ticket: u64) -> Vec<u8> {
    abi::encode_call(LISTING, &[Arg::Word(abi::uint_word(ticket as u128))])
}

/// A vault's `listing(ticket)`: four static words, returned inline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultListing {
    pub stage: u8,
    pub coin: [u8; 20],
    pub ask_wei: u128,
    pub proceeds_wei: u128,
}

pub fn decode_listing(ret: &[u8]) -> Result<VaultListing, String> {
    Ok(VaultListing { stage: abi::uint_at(ret, 0, 0, 8)? as u8, coin: abi::addr_at(ret, 0, 1)?, ask_wei: abi::uint_at(ret, 0, 2, 128)?,
        proceeds_wei: abi::uint_at(ret, 0, 3, 128)? })
}

/// The `owner` of `ticket(id)`'s returned struct (word 1 after the offset word): who burned the unit.
pub fn ticket_owner(ret: &[u8]) -> Result<[u8; 20], String> {
    let base = abi::uint_at(ret, 0, 0, 32)? as usize;
    abi::addr_at(ret, base, 1)
}

// ---- the policy and the state -------------------------------------------------------------------------------------------------------

/// What a controller (the timelock, after the handover) sets per module before any sale happens for it.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SalePolicy {
    pub module: [u8; 20],
    /// The standing reserve `S` under which the module's inventory is sold.
    pub reserve_floor_wei: u128,
    /// A protocol sale's ask over `price()`, bps.
    pub markup_bps: u32,
    /// Sales open at once for this module.
    pub max_open: u32,
    /// An EVM listing's fee share to the chain's first configured fee recipient, bps (0: no fee item).
    pub fee_bps: u32,
}

/// Where a sale is. Every variant carries what a restart needs.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum SPhase {
    /// A protocol sale decided at a tick; `redeemToRelist` not yet sent.
    Planned,
    /// `redeemToRelist` sent; our ticket is the first at or after `count_before` whose owner is us.
    Redeeming { count_before: u64 },
    /// The ticket exists and is not drawn yet.
    Pinned,
    /// Drawn: the item is `inscription`; the listing is to be signed at `ask_wei`.
    Listing { inscription: String, ask_wei: u128 },
    /// Signed and saved; `recordDelivery(ticket, hash)` not yet mined. The outpoint is watched for a fill.
    Listed { inscription: String, outpoint: Outpoint, ask_sats: u64, ask_wei: u128, rate: u64, listing: Vec<u8>, hash: [u8; 32] },
    /// The delivery-by-listing is recorded on chain. The outpoint is watched for a fill.
    Recorded { inscription: String, outpoint: Outpoint, ask_sats: u64, ask_wei: u128, rate: u64, listing: Vec<u8>, hash: [u8; 32] },
    /// The outpoint was spent six deep by a fill paying custody `sold_sats`; the report waits for the ETH on 4663. `ask_wei`
    /// is kept for a relist's minimum (`ask × 0.95`).
    Sold { inscription: String, fill_txid: [u8; 32], sold_sats: u64, rate: u64, proceeds_wei: u128, ask_wei: u128, hash: [u8; 32] },
    /// `itemSold` / `recordSale` mined. Final.
    Reported { proceeds_wei: u128 },
    /// The operator delisted: the item is to be moved to a fresh custody outpoint (cancelling the listing).
    Delisting { inscription: String, outpoint: Outpoint },
    /// The move is signed, saved and sent; `outpoint` is the item's new home once confirmed.
    Moving { inscription: String, outpoint: Outpoint, txid: [u8; 32], raw: Vec<u8>, fee_input: Outpoint },
    /// Moved and six deep: `wrapDeposit(…, to)` owed (to the vault for a relist, to the canister for a protocol sale).
    Redeposit { inscription: String, outpoint: Outpoint },
    /// Re-deposited to the vault; `reclaimListing(ticket)` owed.
    Reclaiming,
    /// Re-deposited (a protocol sale: the seed is back) or reclaimed (a relist). Final.
    Returned,
    /// Nothing of ours moved. Final.
    Failed { why: String },
    /// Something a person must look at. Final.
    Stuck { why: String },
}

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SaleRec {
    pub id: u64,
    /// A protocol sale: the module whose inventory it sells and reports to.
    pub module: Option<[u8; 20]>,
    /// A YOYO relist: the vault that owns the ticket and takes the report.
    pub vault: Option<[u8; 20]>,
    pub wrapper: [u8; 20],
    pub ticket: Option<u64>,
    pub phase: SPhase,
}

impl SaleRec {
    pub fn finished(&self) -> bool {
        matches!(self.phase, SPhase::Reported { .. } | SPhase::Returned | SPhase::Failed { .. } | SPhase::Stuck { .. })
    }
    /// A protocol sale that still holds, or will hold, the seed's place: counted against `max_open`.
    pub fn open(&self) -> bool {
        !self.finished()
    }
    /// The fee input a move in flight has claimed: no delivery or return may choose it.
    pub fn reserved(&self) -> Option<Outpoint> {
        match &self.phase {
            SPhase::Moving { fee_input, .. } => Some(*fee_input),
            _ => None,
        }
    }
    pub fn listing(&self) -> Option<(&str, &Outpoint, u64, &[u8])> {
        match &self.phase {
            SPhase::Listed { inscription, outpoint, ask_sats, listing, .. } | SPhase::Recorded { inscription, outpoint, ask_sats, listing, .. } =>
                Some((inscription, outpoint, *ask_sats, listing)),
            _ => None,
        }
    }
}

/// Everything this side keeps in stable memory (the upgrade tuple's eleventh element).
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct SaleState {
    pub policies: Vec<SalePolicy>,
    pub sales: Vec<SaleRec>,
    pub next_sale: u64,
    /// How far the wrapper's tickets were walked looking for relists.
    pub ticket_cursor: u64,
    /// Each module's extra reads at the last tick, for the page.
    pub extras: Vec<([u8; 20], FloorExtra)>,
}

impl SaleState {
    pub fn policy(&self, module: &[u8; 20]) -> Option<&SalePolicy> {
        self.policies.iter().find(|p| p.module == *module)
    }
    /// A controller's act: bounded, and replaceable (a policy is a number, not an identity).
    pub fn set_policy(&mut self, p: SalePolicy, chain_max_fee_bps: Option<u32>) -> Result<(), String> {
        if p.markup_bps > MAX_MARKUP_BPS {
            return Err(format!("refused: a markup of {} bps is over the {MAX_MARKUP_BPS} ceiling", p.markup_bps));
        }
        if p.max_open == 0 || p.max_open > MAX_OPEN {
            return Err(format!("refused: max_open must be 1..={MAX_OPEN}"));
        }
        if let Some(m) = chain_max_fee_bps {
            if p.fee_bps > m {
                return Err(format!("refused: a fee share of {} bps is over the chain's {m}", p.fee_bps));
            }
        }
        if p.fee_bps > 10_000 {
            return Err("refused: fee_bps over 100%".into());
        }
        match self.policies.iter_mut().find(|x| x.module == p.module) {
            Some(x) => *x = p,
            None => self.policies.push(p),
        }
        Ok(())
    }
    pub fn sale_mut(&mut self, id: u64) -> Option<&mut SaleRec> {
        self.sales.iter_mut().find(|s| s.id == id)
    }
    pub fn open_for(&self, module: &[u8; 20]) -> usize {
        self.sales.iter().filter(|s| s.module == Some(*module) && s.open()).count()
    }
    pub fn has_ticket(&self, ticket: u64) -> bool {
        self.sales.iter().any(|s| s.ticket == Some(ticket))
    }
    /// Every fee input a move in flight has claimed.
    pub fn reserved(&self) -> Vec<Outpoint> {
        self.sales.iter().filter_map(|s| s.reserved()).collect()
    }
    pub fn set_extra(&mut self, module: [u8; 20], x: FloorExtra) {
        match self.extras.iter_mut().find(|(m, _)| *m == module) {
            Some(e) => e.1 = x,
            None => self.extras.push((module, x)),
        }
    }
    pub fn extra(&self, module: &[u8; 20]) -> Option<&FloorExtra> {
        self.extras.iter().find(|(m, _)| m == module).map(|(_, x)| x)
    }
    fn push(&mut self, module: Option<[u8; 20]>, vault: Option<[u8; 20]>, wrapper: [u8; 20], ticket: Option<u64>, phase: SPhase) -> u64 {
        let id = self.next_sale;
        self.next_sale += 1;
        self.sales.push(SaleRec { id, module, vault, wrapper, ticket, phase });
        id
    }
    /// A protocol sale decided now (§3.1 passed); `redeemToRelist` goes out with the EVM group.
    pub fn plan(&mut self, module: [u8; 20], wrapper: [u8; 20]) -> u64 {
        self.push(Some(module), None, wrapper, None, SPhase::Planned)
    }
    /// A vault's relist ticket, adopted once.
    pub fn adopt_relist(&mut self, wrapper: [u8; 20], ticket: u64, vault: [u8; 20], inscription: String, ask_wei: u128) -> Option<u64> {
        if self.has_ticket(ticket) {
            return None;
        }
        Some(self.push(None, Some(vault), wrapper, Some(ticket), SPhase::Listing { inscription, ask_wei }))
    }
}

// ---- the rules -----------------------------------------------------------------------------------------------------------------------

/// §3.1: whether a protocol sale starts now for `module`. Every condition alone stops it, with its reason.
pub fn sell_now(p: &SalePolicy, v: &SinkView, x: &FloorExtra, seed_units: u128, open: usize) -> Result<(), String> {
    if x.reserve >= p.reserve_floor_wei {
        return Err(format!("the reserve holds {} wei, not under the {} floor", x.reserve, p.reserve_floor_wei));
    }
    if x.float_units < ONE {
        return Err(format!("the module holds {} float units: no inventory to sell", x.float_units));
    }
    if open >= p.max_open as usize {
        return Err(format!("{open} sale(s) open already (at most {})", p.max_open));
    }
    if seed_units < ONE {
        return Err(format!("the canister holds {seed_units} units of the wrapper: no seed to redeem (send it one wX)"));
    }
    if !v.open {
        return Err("no floor posted".into());
    }
    match v.price_expiry {
        Some(exp) if exp >= v.clock.time.saturating_add(crate::policy::CAP_MARGIN_SECS) => Ok(()),
        Some(exp) => Err(format!("the posted price is fresh until {exp}, inside the {} s margin: stale", crate::policy::CAP_MARGIN_SECS)),
        None => Err("no price expiry read".into()),
    }
}

/// A protocol sale's ask: the fills price plus the markup, rounded up.
pub fn ask_wei(price_wei: u128, markup_bps: u32) -> u128 {
    price_wei.saturating_mul(BPS + markup_bps as u128).div_ceil(BPS)
}

/// The sats a listing asks for `ask_wei`: gross (no conversion taken off: the seller is paid this much), rounded up.
pub fn ask_sats(ask_wei: u128, sats_per_eth: u64) -> Result<u64, String> {
    if sats_per_eth == 0 {
        return Err("no rate".into());
    }
    let sats = ask_wei.checked_mul(sats_per_eth as u128).ok_or("sats: overflow")?.div_ceil(WEI);
    u64::try_from(sats).map_err(|_| "sats: out of range".into())
}

/// §2: what a sale is reported at — the sats it paid custody at the rate read when it confirmed, less the BTC→ETH conversion
/// the canister will pay to bring them to 4663. Net, rounded down; never above what the rate says the sats are worth.
pub fn proceeds_wei(sold_sats: u64, sats_per_eth: u64) -> Result<u128, String> {
    if sats_per_eth == 0 {
        return Err("no rate".into());
    }
    let gross = (sold_sats as u128).checked_mul(WEI).ok_or("wei: overflow")? / sats_per_eth as u128;
    Ok(gross.checked_mul(BPS - CONVERSION_BPS).ok_or("wei: overflow")? / BPS)
}

/// `SweepVault`'s allowance under the ask for a relist's sale (`SALE_FEE_ALLOWANCE_BPS`, 5%): what `recordSale` refuses under.
pub const SALE_FEE_ALLOWANCE_BPS: u128 = 500;

pub fn relist_min_proceeds(ask_wei: u128) -> u128 {
    ask_wei.saturating_mul(BPS - SALE_FEE_ALLOWANCE_BPS) / BPS
}

// ---- the listing --------------------------------------------------------------------------------------------------------------------

/// §3.3: the seller PSBT. One input, the item at custody; one output, `ask_sats` to custody. A buyer completes it.
pub fn listing_psbt(item: &Frog, ours: &[u8], ask_sats: u64) -> Result<Psbt, String> {
    if !(ours.len() == 34 && ours[0] == 0x51 && ours[1] == 0x20) {
        return Err("our own script is not a taproot script".into());
    }
    if ask_sats < item.value {
        return Err(format!("an ask of {ask_sats} sats is under the item's own {} sats", item.value));
    }
    let tx = Tx {
        version: 2,
        inputs: vec![TxIn { txid: item.output.internal_txid(), vout: item.output.vout, sequence: 0xffff_ffff }],
        outputs: vec![TxOut { value: ask_sats, script: ours.to_vec() }],
        locktime: 0,
    };
    let p = Psbt { tx, witness_utxos: vec![Some(TxOut { value: item.value, script: ours.to_vec() })] };
    check_listing(&p, item, ours, ask_sats)?;
    Ok(p)
}

/// The listing's invariant, checked on the parsed bytes rather than trusted to the builder: exactly the item in, exactly
/// the ask to custody out, nothing else. (The full outbound verifier judges complete transactions; a listing is half of one.)
pub fn check_listing(p: &Psbt, item: &Frog, ours: &[u8], ask_sats: u64) -> Result<(), String> {
    let tx = &p.tx;
    if tx.inputs.len() != 1 {
        return Err(format!("a listing has one input, not {}", tx.inputs.len()));
    }
    let i0 = &tx.inputs[0];
    if i0.txid != item.output.internal_txid() || i0.vout != item.output.vout {
        return Err("the input is not the item".into());
    }
    match p.witness_utxos.first().and_then(|u| u.as_ref()) {
        Some(u) if u.script == ours && u.value == item.value => {}
        _ => return Err("the input's witness utxo is not custody's output at the item's value".into()),
    }
    if tx.outputs.len() != 1 {
        return Err(format!("a listing has one output, not {}", tx.outputs.len()));
    }
    let o0 = &tx.outputs[0];
    if o0.script != ours {
        return Err("the output does not pay custody".into());
    }
    if o0.value != ask_sats {
        return Err(format!("the output pays {} sats, not the ask of {ask_sats}", o0.value));
    }
    if ask_sats < item.value {
        return Err("the ask is under the item's own value".into());
    }
    Ok(())
}

/// The listing's bytes: the PSBT with the seller's input finalized (`sig ‖ 0x83`).
pub fn listing_bytes(p: &Psbt, sig64: &[u8]) -> Result<Vec<u8>, String> {
    if sig64.len() != 64 {
        return Err(format!("a key-path signature is 64 bytes, got {}", sig64.len()));
    }
    let mut w = sig64.to_vec();
    w.push(SIGHASH_LISTING);
    Ok(crate::frogbuy::psbt_bytes(p, &[Some(vec![w])]))
}

/// What names the listing on chain: `recordDelivery(ticket, keccak256(listing bytes))`.
pub fn listing_hash(bytes: &[u8]) -> [u8; 32] {
    crate::evmtx::keccak(bytes)
}

/// §3.4: whether `tx` (the transaction that spent the item's outpoint) is a fill: the input at index `i` spends the item and
/// the output at the same index pays custody at least the ask. Returns what it paid.
pub fn fill_pays(tx: &TxView, item: &Outpoint, ours: &[u8], ask_sats: u64) -> Result<u64, String> {
    let i = tx.inputs.iter().position(|x| x.prev == *item).ok_or("the transaction does not spend the item")?;
    let o = tx.outputs.get(i).ok_or_else(|| format!("no output at the item's index {i}"))?;
    if o.script != ours {
        return Err(format!("output {i} does not pay custody"));
    }
    if o.value < ask_sats {
        return Err(format!("output {i} pays {} sats, under the ask of {ask_sats}", o.value));
    }
    Ok(o.value)
}

/// §3.3, the delist: the item and a fee input to a fresh custody output, change to custody. Every output is custody's, so
/// `outbound::Purpose::ConflictOut` judges it.
pub fn move_psbt(item: &Frog, fee: &crate::custody::Utxo, ours: &[u8], rate: u64) -> Result<Psbt, String> {
    if fee.outpoint == item.output {
        return Err("the fee input is the item".into());
    }
    let rate = crate::custody::bound_rate(rate);
    let with_change = crate::custody::vsize(2, &[ours.len(), ours.len()]) * rate;
    let (fee_sats, change) = if fee.value >= with_change + crate::custody::DUST_P2TR {
        (with_change, Some(fee.value - with_change))
    } else {
        let alone = crate::custody::vsize(2, &[ours.len()]) * rate;
        if fee.value < alone {
            return Err(format!("the fee input holds {} sats; {alone} are needed at {rate} sat/vB", fee.value));
        }
        (fee.value, None)
    };
    if fee_sats > crate::custody::MAX_DELIVERY_FEE_SATS {
        return Err(format!("a fee of {fee_sats} sats is over the {}-sat ceiling", crate::custody::MAX_DELIVERY_FEE_SATS));
    }
    let mut outputs = vec![TxOut { value: item.value, script: ours.to_vec() }];
    if let Some(c) = change {
        outputs.push(TxOut { value: c, script: ours.to_vec() });
    }
    let tx = Tx {
        version: 2,
        inputs: vec![
            TxIn { txid: item.output.internal_txid(), vout: item.output.vout, sequence: 0xffff_fffd },
            TxIn { txid: fee.outpoint.internal_txid(), vout: fee.outpoint.vout, sequence: 0xffff_fffd },
        ],
        outputs,
        locktime: 0,
    };
    Ok(Psbt { tx, witness_utxos: vec![Some(TxOut { value: item.value, script: ours.to_vec() }), Some(TxOut { value: fee.value, script: ours.to_vec() })] })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deposits::{InView, OutView};
    use crate::itemsink::{Clock, SinkKind};

    const E: u128 = ONE;
    const T0: u64 = 1_800_000_000;
    const M: [u8; 20] = [0x56; 20];

    fn ours() -> Vec<u8> {
        let mut s = vec![0x51, 0x20];
        s.extend([0xc2u8; 32]);
        s
    }
    fn item() -> Frog {
        Frog { output: Outpoint { txid: [0xf0; 32], vout: 1 }, value: 10_000 }
    }
    fn policy() -> SalePolicy {
        SalePolicy { module: M, reserve_floor_wei: E, markup_bps: 500, max_open: 1, fee_bps: 0 }
    }
    fn view(posted: u64) -> SinkView {
        SinkView { kind: SinkKind::Floor, status: "x".into(), open: true, done: false, recorded: 0, target: None, want: 0, budget_left: 0,
            item_cap: None, deadline: None, clock: Clock { time: T0, parent_block: 1 }, drive_refunds: false, drive_settle: false,
            price_expiry: Some(posted + 3_600) }
    }
    fn extra(reserve: u128, float: u128) -> FloorExtra {
        FloorExtra { price: 640 * E / 1_000, float_units: float, min_sale_proceeds: 620 * E / 1_000, reserve }
    }

    #[test]
    fn the_selectors_are_the_signatures_keccak() {
        for (sel, s) in [(ITEM_SOLD, "itemSold(uint256)"), (REDEEM_TO_RELIST, "redeemToRelist()"), (RECORD_SALE, "recordSale(uint256)"),
            (RECLAIM_LISTING, "reclaimListing(uint256)"), (LISTING, "listing(uint256)"), (VAULT_WRAPPER, "wrapper()")] {
            assert_eq!(crate::evmtx::keccak(s.as_bytes())[..4], sel, "{s}");
        }
        assert_eq!(hex::encode(item_sold(7)), format!("f0596a79{:064x}", 7));
        assert_eq!(hex::encode(record_sale(3)), format!("6f27f690{:064x}", 3));
        assert_eq!(hex::encode(reclaim_listing(3)), format!("ea5e13d4{:064x}", 3));
        // `listing(uint256)` returns (uint8, address, uint256, uint256) inline; `ticket(uint256)` a struct behind an offset
        let ret = [abi::uint_word(1), abi::addr_word(&[0x53; 20]), abi::uint_word(5 * E), abi::uint_word(0)].concat();
        assert_eq!(decode_listing(&ret).unwrap(), VaultListing { stage: 1, coin: [0x53; 20], ask_wei: 5 * E, proceeds_wei: 0 });
        let t = [abi::uint_word(32), abi::uint_word(2), abi::addr_word(&[0x44; 20]), abi::uint_word(0)].concat();
        assert_eq!(ticket_owner(&t).unwrap(), [0x44; 20]);
    }

    #[test]
    fn every_condition_of_the_reserve_low_rule_alone_stops_a_sale() {
        let p = policy();
        assert!(sell_now(&p, &view(T0), &extra(E / 2, E), E, 0).is_ok());
        assert!(sell_now(&p, &view(T0), &extra(E, E), E, 0).unwrap_err().contains("not under"), "the reserve is at the floor");
        assert!(sell_now(&p, &view(T0), &extra(E / 2, E - 1), E, 0).unwrap_err().contains("no inventory"));
        assert!(sell_now(&p, &view(T0), &extra(E / 2, E), E - 1, 0).unwrap_err().contains("no seed"));
        assert!(sell_now(&p, &view(T0), &extra(E / 2, E), E, 1).unwrap_err().contains("open already"));
        let stale = view(T0 - 3_600 + crate::policy::CAP_MARGIN_SECS - 1);
        assert!(sell_now(&p, &stale, &extra(E / 2, E), E, 0).unwrap_err().contains("stale"));
        let mut closed = view(T0);
        closed.open = false;
        assert!(sell_now(&p, &closed, &extra(E / 2, E), E, 0).unwrap_err().contains("no floor"));
    }

    #[test]
    fn the_policy_is_bounded_and_the_state_counts_open_sales_per_module() {
        let mut s = SaleState::default();
        assert!(s.set_policy(SalePolicy { markup_bps: MAX_MARKUP_BPS + 1, ..policy() }, None).unwrap_err().contains("markup"));
        assert!(s.set_policy(SalePolicy { max_open: 0, ..policy() }, None).unwrap_err().contains("max_open"));
        assert!(s.set_policy(SalePolicy { max_open: MAX_OPEN + 1, ..policy() }, None).unwrap_err().contains("max_open"));
        assert!(s.set_policy(SalePolicy { fee_bps: 600, ..policy() }, Some(500)).unwrap_err().contains("chain's"));
        s.set_policy(policy(), None).unwrap();
        s.set_policy(SalePolicy { reserve_floor_wei: 2 * E, ..policy() }, None).unwrap();
        assert_eq!((s.policies.len(), s.policy(&M).unwrap().reserve_floor_wei), (1, 2 * E), "replaced, not duplicated");
        let id = s.plan(M, [0xaa; 20]);
        assert_eq!((s.open_for(&M), s.open_for(&[1; 20])), (1, 0));
        s.sale_mut(id).unwrap().phase = SPhase::Reported { proceeds_wei: 1 };
        assert_eq!(s.open_for(&M), 0);
        assert!(s.adopt_relist([0xaa; 20], 4, [0x55; 20], "i".into(), E).is_some());
        assert!(s.adopt_relist([0xaa; 20], 4, [0x55; 20], "i".into(), E).is_none(), "a ticket is adopted once");
    }

    #[test]
    fn the_ask_and_the_proceeds_round_against_us_and_proceeds_are_net_of_the_conversion() {
        assert_eq!(ask_wei(E, 500), 1_050 * E / 1_000);
        assert_eq!(ask_wei(3, 500), 4, "rounded up");
        let rate = 3_190_000;
        assert_eq!(ask_sats(E, rate).unwrap(), rate);
        assert_eq!(ask_sats(E + 1, rate).unwrap(), rate + 1, "a wei more asks a sat more");
        let p = proceeds_wei(rate, rate).unwrap();
        assert_eq!(p, E * (BPS - CONVERSION_BPS) / BPS, "one ETH of sats reports 0.98 ETH");
        assert!(p < E);
        assert!(proceeds_wei(1, 0).is_err());
        assert_eq!(relist_min_proceeds(E), 950 * E / 1_000);
    }

    #[test]
    fn a_listing_is_one_input_one_output_the_ask_to_custody_and_the_check_refuses_every_other_shape() {
        let p = listing_psbt(&item(), &ours(), 900_000).unwrap();
        assert_eq!((p.tx.inputs.len(), p.tx.outputs.len(), p.tx.outputs[0].value, p.tx.inputs[0].sequence), (1, 1, 900_000, 0xffff_ffff));
        assert_eq!(p.tx.outputs[0].script, ours());
        assert!(listing_psbt(&item(), &ours(), 9_999).unwrap_err().contains("under the item"));
        assert!(listing_psbt(&item(), &[0x00, 0x14], 900_000).is_err());
        // the bytes: the seller's input finalized with sig ‖ 0x83, and the parse round-trips into the same transaction
        let bytes = listing_bytes(&p, &[0x5a; 64]).unwrap();
        let back = Psbt::parse(&bytes).unwrap();
        assert_eq!(back, p);
        let w = crate::psbt::final_witnesses(&bytes).unwrap();
        assert_eq!(w[0].as_ref().unwrap()[0].len(), 65);
        assert_eq!(w[0].as_ref().unwrap()[0][64], 0x83);
        assert!(listing_bytes(&p, &[0; 63]).is_err());
        assert!(check_listing(&back, &item(), &ours(), 900_000).is_ok());
        // tampered shapes
        let mut two_in = p.clone();
        two_in.tx.inputs.push(TxIn { txid: [1; 32], vout: 0, sequence: 0 });
        two_in.witness_utxos.push(None);
        assert!(check_listing(&two_in, &item(), &ours(), 900_000).unwrap_err().contains("one input"));
        let mut two_out = p.clone();
        two_out.tx.outputs.push(TxOut { value: 1, script: ours() });
        assert!(check_listing(&two_out, &item(), &ours(), 900_000).unwrap_err().contains("one output"));
        let mut foreign = p.clone();
        foreign.tx.outputs[0].script = vec![0x51, 0x20, 9];
        assert!(check_listing(&foreign, &item(), &ours(), 900_000).unwrap_err().contains("pay custody"));
        let mut under = p.clone();
        under.tx.outputs[0].value = 899_999;
        assert!(check_listing(&under, &item(), &ours(), 900_000).unwrap_err().contains("not the ask"));
        let mut other = p.clone();
        other.tx.inputs[0].vout = 2;
        assert!(check_listing(&other, &item(), &ours(), 900_000).unwrap_err().contains("not the item"));
        let mut lied = p.clone();
        lied.witness_utxos[0].as_mut().unwrap().value = 1;
        assert!(check_listing(&lied, &item(), &ours(), 900_000).unwrap_err().contains("witness utxo"));
    }

    /// BIP341's SigMsg under 0x83, written out by hand from the BIP's text rather than by calling the module, so that the
    /// module's assembly is checked against an independent reading of the specification. (No differential fixture from a
    /// second implementation exists for this type yet: docs/v6/canister-floor-followups.md §9 says so.)
    #[test]
    fn the_listing_digest_is_bip341s_single_anyonecanpay_message() {
        use sha2::{Digest, Sha256};
        let p = listing_psbt(&item(), &ours(), 900_000).unwrap();
        let d = crate::sighash::taproot_key_path_typed(&p, 0, 0x83).unwrap();
        let mut m = vec![0u8, 0x83];
        m.extend_from_slice(&2i32.to_le_bytes());
        m.extend_from_slice(&0u32.to_le_bytes());
        m.push(0); // spend_type
        m.extend_from_slice(&item().output.internal_txid());
        m.extend_from_slice(&1u32.to_le_bytes());
        m.extend_from_slice(&10_000u64.to_le_bytes());
        m.push(34);
        m.extend_from_slice(&ours());
        m.extend_from_slice(&0xffff_ffffu32.to_le_bytes());
        let mut out = 900_000u64.to_le_bytes().to_vec();
        out.push(34);
        out.extend_from_slice(&ours());
        let single: [u8; 32] = Sha256::digest(&out).into();
        m.extend_from_slice(&single);
        let tag: [u8; 32] = Sha256::digest(b"TapSighash").into();
        let want: [u8; 32] = Sha256::digest([&tag[..], &tag[..], &m].concat()).into();
        assert_eq!(d, want);
        assert_ne!(d, crate::sighash::taproot_key_path(&p, 0).unwrap(), "not the default digest");
        assert_eq!(crate::sighash::taproot_key_path_typed(&p, 0, 0x81), Err(crate::sighash::SighashError::UnsupportedHashType(0x81)));
        let mut no_out = p.clone();
        no_out.tx.outputs.clear();
        assert_eq!(crate::sighash::taproot_key_path_typed(&no_out, 0, 0x83), Err(crate::sighash::SighashError::NoOutputAtIndex(0)));
    }

    #[test]
    fn a_fill_pays_custody_the_ask_at_the_items_index() {
        let it = item().output;
        let inp = |o: Outpoint| InView { prev: o, prev_script: vec![], script_sig: vec![], witness: vec![] };
        let tx = |outs: Vec<(u64, Vec<u8>)>| TxView { txid: [1; 32], inputs: vec![inp(Outpoint { txid: [9; 32], vout: 0 }), inp(it)],
            outputs: outs.into_iter().map(|(value, script)| OutView { value, script }).collect(), confirmed: true, height: Some(1), block_hash: None };
        assert_eq!(fill_pays(&tx(vec![(546, vec![9]), (900_000, ours())]), &it, &ours(), 900_000), Ok(900_000));
        assert!(fill_pays(&tx(vec![(900_000, ours()), (546, vec![9])]), &it, &ours(), 900_000).unwrap_err().contains("does not pay custody"));
        assert!(fill_pays(&tx(vec![(546, vec![9]), (899_999, ours())]), &it, &ours(), 900_000).unwrap_err().contains("under the ask"));
        assert!(fill_pays(&tx(vec![(546, vec![9])]), &it, &ours(), 900_000).unwrap_err().contains("no output"));
        assert!(fill_pays(&tx(vec![]), &Outpoint { txid: [7; 32], vout: 0 }, &ours(), 1).unwrap_err().contains("does not spend"));
    }

    #[test]
    fn a_move_pays_everything_to_custody_and_passes_the_conflict_out_purpose() {
        let fee = crate::custody::Utxo { outpoint: Outpoint { txid: [0xfe; 32], vout: 0 }, value: 20_000, confirmed: true };
        let p = move_psbt(&item(), &fee, &ours(), 3).unwrap();
        assert!(p.tx.outputs.iter().all(|o| o.script == ours()));
        assert_eq!(p.tx.outputs[0].value, 10_000);
        let raw = crate::psbt::finalize(&p, &[None, None], &[(0, vec![1; 64]), (1, vec![2; 64])]).unwrap();
        let pre = vec![
            crate::outbound::Prevout { txid: item().output.internal_txid(), vout: 1, value: 10_000, script: ours() },
            crate::outbound::Prevout { txid: fee.outpoint.internal_txid(), vout: 0, value: 20_000, script: ours() },
        ];
        let pol = crate::outbound::BtcPolicy { custody_script: ours(), max_fee_sats: crate::outbound::MAX_FEE_SATS, require_finalized: true, require_event: true };
        assert!(crate::outbound::verify_btc(&raw, &pre, &crate::outbound::Purpose::ConflictOut { spend: pre[0].clone() }, &pol).is_ok());
        assert!(move_psbt(&item(), &crate::custody::Utxo { outpoint: item().output, value: 1, confirmed: true }, &ours(), 3).is_err());
    }
}
