//! A contract that takes items the canister buys and reimburses it per item: a collection's buy queue (`buyqueue.rs`), or a
//! graduating `FrogCurve` as merged at `40735bf` (`curvebuy.rs`). docs/v3/curve.md, "Canister: …".
//!
//! Pure, and deliberately ignorant of what an item is: this module decides from a [`SinkView`] (one pinned read, adapted by
//! the contract's own module) whether a purchase may start, whether a held item may be minted to the sink, what one item may
//! cost, and whether to drive the contract's refund or settlement trigger. The same rules are meant to drive EVM NFT
//! collections later: only the adapter (what to read, what to call) and the acquisition differ.
//!
//! Every deadline is on both clocks, and the nearer one binds (the rule `delivery.rs` applies to a ticket).

use crate::frogbuy::{FrogRec, Target, MINT_MARGIN_SECS, PARENT_BLOCK_SECONDS, PURCHASE_LATENCY_SECS};

/// The two clocks at the pinned block: the chain's time (never behind the canister's) and the parent chain's block number
/// advanced by the lag since the pin at 12 s a block. Both push a deadline nearer: the safe direction for a rule that stops.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub struct Clock {
    pub time: u64,
    pub parent_block: u64,
}

/// Seconds left before a deadline on the NEARER clock (the parent chain's blocks counted at 12 s).
pub fn secs_left(at: u64, at_block: u64, c: &Clock) -> u64 {
    at.saturating_sub(c.time).min(at_block.saturating_sub(c.parent_block).saturating_mul(PARENT_BLOCK_SECONDS))
}

/// Both clocks past the deadline: what `Wait.elapsed` requires.
pub fn both_past(at: u64, at_block: u64, c: &Clock) -> bool {
    c.time >= at && c.parent_block >= at_block
}

/// Which contract a sink is: what to read from it and what to call on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub enum SinkKind {
    /// A collection's buy queue (the design since 2026-09-23; interface ASSUMED, `buyqueue.rs`).
    Queue,
    /// One coin's `FrogCurve` in `Graduating` (as merged at `40735bf`).
    Curve,
    /// A collection's Floor Module (docs/v5/floor-feed.md §6; interface ASSUMED, `floorsink.rs`): buys one item when net
    /// ETH-backed demand reaches one item plus a buffer and the posted floor is fresh.
    Floor,
}

/// One sink as read at one pin, in the terms every rule here uses.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub struct SinkView {
    pub kind: SinkKind,
    /// The contract's own state word, for the page.
    pub status: String,
    /// Taking items now.
    pub open: bool,
    /// Finished for good (a curve past graduating). A queue is never done.
    pub done: bool,
    /// Items recorded so far.
    pub recorded: u64,
    /// How many it takes in all (a curve's N), or none (a queue).
    pub target: Option<u64>,
    /// Items it could pay for now, before ours on the way.
    pub want: u64,
    /// ETH it holds for items, in wei.
    pub budget_left: u128,
    /// A hard per-item maximum, when the contract states one.
    pub item_cap: Option<u128>,
    /// `(at, atBlock)` after which an item can no longer be taken safely, when there is one.
    pub deadline: Option<(u64, u64)>,
    pub clock: Clock,
    /// Anyone may now start the contract's refunds, and the canister drives it.
    pub drive_refunds: bool,
    /// Anyone may now settle the contract (a curve's `graduate`), and the canister drives it.
    pub drive_settle: bool,
    /// When the price the item cap derives from stops being fresh (a Floor Module's `postedAt + ttl`); `None` for sinks whose
    /// cap needs no freshness (a queue, a curve). Optional so a view saved before it existed still decodes.
    #[serde(default)]
    pub price_expiry: Option<u64>,
}

impl SinkView {
    pub fn secs_left(&self) -> Option<u64> {
        self.deadline.map(|(a, b)| secs_left(a, b, &self.clock))
    }
}

/// What the canister keeps per sink.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub struct SinkRec {
    pub kind: SinkKind,
    pub sink: [u8; 20],
    /// The coin, for a curve; zero for a queue.
    pub coin: [u8; 20],
    pub view: Option<SinkView>,
    /// Why this sink is never served (not our payee, not our wrapper), when that was found.
    pub refused: Option<String>,
    /// The refund / settlement trigger sent by us, and the recorded count it was sent at: sent again only once the count
    /// moves (a queue refunds many times; a curve once).
    pub drove_refunds_at: Option<u64>,
    pub drove_settle: bool,
}

impl SinkRec {
    pub fn done(&self) -> bool {
        self.refused.is_some() || self.view.as_ref().is_some_and(|v| v.done)
    }
}

/// Our items for `sink` not yet recorded there (and not finished): how many, and what they commit of its budget.
pub fn assigned(frogs: &[FrogRec], sink: &[u8; 20]) -> (u64, u128) {
    frogs.iter().filter(|f| f.target == Some(Target::Sink(*sink)) && !f.finished())
        .fold((0, 0), |(n, w), f| (n + 1, w + f.committed_wei()))
}

/// Items still to find, beyond ours on the way.
pub fn needed(v: &SinkView, assigned_count: u64) -> u64 {
    v.want.saturating_sub(assigned_count)
}

/// The most the next item may cost: what is left, less what ours already commit, over the items still needed, and never
/// over the contract's own per-item maximum.
pub fn per_item_cap(v: &SinkView, assigned_count: u64, assigned_wei: u128) -> Option<u128> {
    let need = needed(v, assigned_count);
    if need == 0 {
        return None;
    }
    let even = v.budget_left.saturating_sub(assigned_wei) / need as u128;
    Some(v.item_cap.map_or(even, |c| c.min(even)))
}

/// A new purchase may start: open, an item still needed, and more than a purchase's latency, the mint margin and the policy's
/// cap margin left on the nearer clock.
pub fn may_start(v: &SinkView, assigned_count: u64) -> Result<(), String> {
    if !v.open {
        return Err(format!("the {:?} is {}", v.kind, v.status));
    }
    if needed(v, assigned_count) == 0 {
        return Err(format!("it can pay for {} and {assigned_count} of ours are on the way: nothing to buy", v.want));
    }
    // docs/v5/floor-feed.md §6.1: a price-driven sink starts a purchase only on a price with more than the policy's cap margin
    // left, as the vault loop does with the keeper's cap. Only a start: the item was bought against a fresh price.
    if let Some(exp) = v.price_expiry {
        if exp < v.clock.time.saturating_add(crate::policy::CAP_MARGIN_SECS) {
            return Err(format!("the posted price is fresh until {exp}, inside the {} s margin: stale", crate::policy::CAP_MARGIN_SECS));
        }
    }
    let want = PURCHASE_LATENCY_SECS + MINT_MARGIN_SECS + crate::policy::CAP_MARGIN_SECS;
    if let Some(left) = v.secs_left() {
        if left <= want {
            return Err(format!("{left} s left on the nearer clock, not more than the {want} s a purchase needs"));
        }
    }
    Ok(())
}

/// A held item may be minted to the sink: open, not all recorded, more than the mint margin left on the nearer clock, and its
/// cost within what the sink holds and its per-item maximum.
pub fn may_mint(v: &SinkView, cost_wei: u128) -> Result<(), String> {
    if !v.open {
        return Err(format!("the {:?} is {}", v.kind, v.status));
    }
    if v.target.is_some_and(|n| v.recorded >= n) {
        return Err("every item is recorded".into());
    }
    if let Some(left) = v.secs_left() {
        if left <= MINT_MARGIN_SECS {
            return Err(format!("{left} s left on the nearer clock, inside the {MINT_MARGIN_SECS} s mint margin"));
        }
    }
    if cost_wei > v.budget_left {
        return Err(format!("an item costing {cost_wei} wei is over the {} it holds", v.budget_left));
    }
    if let Some(c) = v.item_cap {
        if cost_wei > c {
            return Err(format!("an item costing {cost_wei} wei is over its per-item cap of {c}"));
        }
    }
    Ok(())
}

/// Whether to send the refund trigger now: the contract allows it, and we have not already sent it at this recorded count.
pub fn drive_refunds(r: &SinkRec) -> bool {
    r.view.as_ref().is_some_and(|v| v.drive_refunds && r.drove_refunds_at != Some(v.recorded))
}

pub fn drive_settle(r: &SinkRec) -> bool {
    r.view.as_ref().is_some_and(|v| v.drive_settle) && !r.drove_settle
}

// ---- progress, for the page ----------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub struct Progress {
    pub kind: SinkKind,
    pub sink: String,
    pub coin: String,
    pub status: String,
    /// "k of N": items recorded, and N for a curve (0 for a queue, which has no end).
    pub recorded: u64,
    pub target: u64,
    /// What the sink holds for items, wei (decimal).
    pub budget_left: String,
    /// What the next item may cost, before ours on the way (decimal wei; empty when nothing is needed).
    pub next_cap: String,
    pub deadline_at: u64,
    pub deadline_block: u64,
    /// Our items for this sink on the way (wanted, bought, held, minted and not yet recorded).
    pub in_flight: u64,
    pub refused: Option<String>,
}

pub fn progress(r: &SinkRec, frogs: &[FrogRec]) -> Progress {
    let h = |a: &[u8; 20]| format!("0x{}", hex::encode(a));
    let (in_flight, w) = assigned(frogs, &r.sink);
    let v = r.view.as_ref();
    Progress {
        kind: r.kind,
        sink: h(&r.sink),
        coin: h(&r.coin),
        status: v.map_or("unread".into(), |v| v.status.clone()),
        recorded: v.map_or(0, |v| v.recorded),
        target: v.and_then(|v| v.target).unwrap_or(0),
        budget_left: v.map_or(0, |v| v.budget_left).to_string(),
        next_cap: v.and_then(|v| per_item_cap(v, in_flight, w)).map(|c| c.to_string()).unwrap_or_default(),
        deadline_at: v.and_then(|v| v.deadline).map_or(0, |d| d.0),
        deadline_block: v.and_then(|v| v.deadline).map_or(0, |d| d.1),
        in_flight,
        refused: r.refused.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const E: u128 = 1_000_000_000_000_000_000;
    const T0: u64 = 1_800_000_000;
    const B0: u64 = 26_000_000;

    fn queue(budget: u128, cap: u128, want: u64) -> SinkView {
        SinkView { kind: SinkKind::Queue, status: "open".into(), open: true, done: false, recorded: 3, target: None, want,
            budget_left: budget, item_cap: Some(cap), deadline: Some((T0 + 7 * 86_400, B0 + 7 * 86_400 / 12)),
            clock: Clock { time: T0, parent_block: B0 }, drive_refunds: false, drive_settle: false, price_expiry: None }
    }

    #[test]
    fn the_nearer_clock_binds_and_refunds_need_both() {
        let c = Clock { time: T0, parent_block: B0 };
        assert_eq!(secs_left(T0 + 3_600, B0 + 1_000, &c), 3_600);
        assert_eq!(secs_left(T0 + 36_000, B0 + 100, &c), 1_200, "the parent chain is nearer");
        assert!(!both_past(T0, B0 + 1, &c));
        assert!(both_past(T0, B0, &c));
    }

    #[test]
    fn a_queue_is_bought_for_one_item_at_a_time_under_its_cap() {
        let v = queue(E, E / 2, 1);
        assert_eq!(per_item_cap(&v, 0, 0), Some(E / 2), "the queue's cap binds, not the whole of what it holds");
        assert!(may_start(&v, 0).is_ok());
        assert!(may_start(&v, 1).unwrap_err().contains("nothing to buy"), "one of ours on the way covers it");
        assert_eq!(per_item_cap(&v, 1, E / 2), None);
        assert!(may_start(&queue(E, E / 2, 0), 0).is_err(), "not funded to cap + buffer: the contract says want = 0");
        assert!(may_mint(&v, E / 2).is_ok());
        assert!(may_mint(&v, E / 2 + 1).unwrap_err().contains("per-item cap"));
        let mut thin = queue(E / 4, E / 2, 1);
        thin.item_cap = None;
        assert!(may_mint(&thin, E / 4 + 1).unwrap_err().contains("over the"));
    }

    #[test]
    fn deadlines_stop_purchases_first_and_mints_last() {
        let need = PURCHASE_LATENCY_SECS + MINT_MARGIN_SECS + crate::policy::CAP_MARGIN_SECS;
        let mut v = queue(E, E / 2, 1);
        v.deadline = Some((T0 + need, B0 + 1_000_000));
        assert!(may_start(&v, 0).is_err());
        assert!(may_mint(&v, 1).is_ok());
        v.deadline = Some((T0 + 1_000_000, B0 + MINT_MARGIN_SECS / 12));
        assert!(may_mint(&v, 1).unwrap_err().contains("mint margin"), "the parent chain's clock alone stops a mint");
        v.deadline = None;
        assert!(may_start(&v, 0).is_ok(), "no deadline, no time rule");
    }

    #[test]
    fn a_price_driven_sink_starts_only_on_a_fresh_price_and_mints_whatever_its_age() {
        let mut v = queue(E, E / 2, 1);
        v.kind = SinkKind::Floor;
        v.deadline = None;
        v.price_expiry = Some(T0 + crate::policy::CAP_MARGIN_SECS + 1);
        assert!(may_start(&v, 0).is_ok());
        v.price_expiry = Some(T0 + crate::policy::CAP_MARGIN_SECS - 1);
        assert!(may_start(&v, 0).unwrap_err().contains("stale"));
        v.price_expiry = Some(T0 - 3_600);
        assert!(may_mint(&v, E / 2).is_ok(), "an item bought against a fresh price is minted and claimed; the module bounds the cost");
    }

    #[test]
    fn a_queue_refunds_many_times_but_each_trigger_once_per_count() {
        let mut r = SinkRec { kind: SinkKind::Queue, sink: [1; 20], coin: [0; 20], view: Some(queue(E, E / 2, 0)), refused: None,
            drove_refunds_at: None, drove_settle: false };
        assert!(!drive_refunds(&r));
        r.view.as_mut().unwrap().drive_refunds = true;
        assert!(drive_refunds(&r));
        r.drove_refunds_at = Some(3);
        assert!(!drive_refunds(&r), "sent at this count: the chain says the rest");
        r.view.as_mut().unwrap().recorded = 4;
        assert!(drive_refunds(&r), "the count moved: it may be due again");
    }

    #[test]
    fn progress_reads_as_k_of_n_or_as_a_queue() {
        let r = SinkRec { kind: SinkKind::Queue, sink: [1; 20], coin: [0; 20], view: Some(queue(E, E / 2, 1)), refused: None,
            drove_refunds_at: None, drove_settle: false };
        let p = progress(&r, &[]);
        assert_eq!((p.recorded, p.target, p.in_flight, p.next_cap.as_str()), (3, 0, 0, "500000000000000000"));
    }
}
