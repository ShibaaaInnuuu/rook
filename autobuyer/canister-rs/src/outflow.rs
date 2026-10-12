//! Outbound rate limits and the tripwire that freezes custody (docs/v5/custody-hardening.md, H3; docs/v5/custody-threat-model.md,
//! T10, T1, T7).
//!
//! **Wired** (docs/v5/custody-hardening.md, H3): the limits are `Init::outflow_limits` (stage 0 by default), the limiter is the
//! upgrade tuple's ninth element with the cap book, a trip sets the buyer's pause and `set_paused` freezes and unfreezes it.
//! Still pure: no `crate::` import.
//!
//! Pure: every time is passed in. The one idea: **nothing legitimate empties custody in a day.** Holders redeem one item at a
//! time; deposits trickle in. A forged chain read (colluding providers), a bug in a signing path, or a malicious upgrade that has
//! slipped past the timelock all look the same from here: *many* items leaving, or *many* units minted, in a short time. So:
//!
//! - every outbound act (an item delivered or returned, a wrapper unit minted, value sent to anyone but ourselves) is counted in
//!   a sliding 24-hour window, per collection and in all;
//! - an act over its limit **waits** (it is not refused: a delivery that waits past its margin is refused by `delivery::decide`,
//!   and the redeemer's ticket refunds; nothing is lost but time);
//! - when the **distinct** acts asking to go out in one window reach [`Limits::trip_factor`] times a limit, the tripwire
//!   **freezes** custody. The wiring maps the freeze onto the buyer's pause, so lifting it takes what lifting a pause takes after
//!   the handover: k guardians, through the timelock's `resume_target`.
//!
//! A limit is only honest if a bank run still fits inside the timelock's delay: holders who distrust a pending upgrade must be
//! able to take every item out before it lands. [`exit_fits`] is that check.

use serde::{Deserialize, Serialize};

pub const WINDOW_SECS: u64 = 24 * 60 * 60;
/// Stage 0's value a day (H3): 25 ETH since the operator's decision of 2026-09-27 (10 ETH before).
pub const STAGE0_VALUE_WEI: u128 = 25_000_000_000_000_000_000;

/// What leaves.
#[derive(candid::CandidType, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    /// An item leaves custody: a redemption's delivery, a refused deposit's return, a burn delivery. Amount: 1.
    Item,
    /// A wrapper unit is minted on the canister's word (a purchase's arrival, a holder deposit). Amount: 1.
    Mint,
    /// Value leaves to someone other than ourselves (a Seaport fill's price, a bridge deposit). Amount: wei.
    Value,
    /// A refused holder deposit going back, whole, to its own sender (audit D, H-2). Amount: 1. Its own budget, at the item
    /// limits but counted apart from deliveries, and it **never trips the freeze**: anyone can manufacture refused deposits, so
    /// their demand says nothing about custody, and a trip on it would let a stranger freeze deliveries. What returns is still
    /// rate-limited, and H4 checks every one goes whole to the deposit's own sender.
    Return,
}

#[derive(candid::CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Limits {
    pub per_collection_items: u32,
    pub global_items: u32,
    pub per_collection_mints: u32,
    pub global_mints: u32,
    pub global_value_wei: u128,
    /// Distinct acts wanting out in one window, as a multiple of the limit, that trip the freeze. At least 2.
    pub trip_factor: u32,
}

/// One act that went out (or asked to), keyed so that the same act asked every tick counts once.
#[derive(candid::CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Act {
    pub key: String,
    pub collection: String,
    pub flow: Flow,
    pub amount: u128,
    pub at: u64,
}

#[derive(candid::CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum Cause {
    /// Demand in one window reached `trip_factor` × the limit.
    Tripwire { collection: String, flow: Flow, demanded: u128, limit: u128 },
    /// Frozen by a person (the wiring: the guardian's pause) or by an outside canary; `note` says which.
    Manual { note: String },
}

#[derive(candid::CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
pub enum Freeze {
    #[default]
    Open,
    Frozen { since: u64, cause: Cause },
}

#[derive(candid::CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
pub struct Outflow {
    /// Acts that went out, inside the window.
    pub done: Vec<Act>,
    /// Acts that asked to go out, inside the window (a superset of `done`, one entry per key).
    pub asked: Vec<Act>,
    pub freeze: Freeze,
    /// Asks a controller's resume acknowledged (audit D, H-2): they still wait on the limits, but no longer count toward the
    /// trip, so the same backlog does not re-freeze custody one tick after the guardians lift the freeze. Kept for the window.
    /// `None` in a state saved before it existed.
    #[serde(default)]
    pub acked: Option<Vec<Act>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Allow,
    /// Over a limit: ask again at `until` (when the oldest counted act leaves the window).
    Wait { until: u64, why: String },
    Frozen(Cause),
}

impl Limits {
    /// Launch stage 0 (docs/v5/launch-gates.md, G7): 3 items a day per collection and 6 in all, 3 mints a day per collection and
    /// 6 in all, 25 ETH of value a day (the operator, 2026-09-27: 10 ETH before; with J7's 15 ETH custody caps a BAYC advance and
    /// a Pudgy one no longer fit a 10 ETH day), and the tripwire at three times a limit. What an install gets when its argument
    /// names no limits.
    pub fn stage0() -> Limits {
        Limits { per_collection_items: 3, global_items: 6, per_collection_mints: 3, global_mints: 6,
            global_value_wei: STAGE0_VALUE_WEI, trip_factor: 3 }
    }
}

/// An upgrade's limits against the running ones (audit D, I-5): the caps' rule, applied to the limits. Lowering is always
/// allowed; each limit, and the trip factor, may rise at most ×4 per proposal (`custodycap::MAX_RAISE_FACTOR`).
pub fn validate_change(old: &Limits, new: &Limits) -> Result<(), String> {
    let k = crate::custodycap::MAX_RAISE_FACTOR;
    let pairs: [(&str, u128, u128); 6] = [
        ("per_collection_items", old.per_collection_items as u128, new.per_collection_items as u128),
        ("global_items", old.global_items as u128, new.global_items as u128),
        ("per_collection_mints", old.per_collection_mints as u128, new.per_collection_mints as u128),
        ("global_mints", old.global_mints as u128, new.global_mints as u128),
        ("global_value_wei", old.global_value_wei, new.global_value_wei),
        ("trip_factor", old.trip_factor as u128, new.trip_factor as u128),
    ];
    for (name, o, n) in pairs {
        if n > o.max(1).saturating_mul(k) {
            return Err(format!("{name} rises from {o} to {n}: more than ×{k} in one proposal"));
        }
    }
    Ok(())
}

pub fn validate(l: &Limits) -> Result<(), String> {
    if l.trip_factor < 2 {
        return Err("the trip factor must be at least 2: at 1 an ordinary busy day would freeze custody".into());
    }
    if l.per_collection_items > l.global_items || l.per_collection_mints > l.global_mints {
        return Err("a collection's limit may not exceed the global limit".into());
    }
    if l.per_collection_items == 0 {
        return Err("an item limit of zero means nobody can ever redeem: that is a freeze, and a freeze has its own switch".into());
    }
    Ok(())
}

/// Whether holders can take every item of a collection out while a malicious upgrade waits out the timelock's delay: `max_items`
/// items at `per_day` a day must leave within the delay less one day (the last day is for Bitcoin confirmations and the
/// record). If not, the rate limit would trap holders behind a proposal they can see coming.
pub fn exit_fits(max_items: u32, per_day: u32, delay_secs: u64) -> bool {
    if per_day == 0 {
        return false;
    }
    let days = (delay_secs / WINDOW_SECS).saturating_sub(1);
    (max_items as u64).div_ceil(per_day as u64) <= days
}

fn in_window(a: &Act, now: u64) -> bool {
    now.saturating_sub(a.at) < WINDOW_SECS && a.at <= now
}

impl Outflow {
    fn prune(&mut self, now: u64) {
        self.done.retain(|a| in_window(a, now));
        self.asked.retain(|a| in_window(a, now));
        if let Some(k) = self.acked.as_mut() {
            k.retain(|a| in_window(a, now));
        }
    }

    fn is_acked(&self, key: &str) -> bool {
        self.acked.as_ref().is_some_and(|k| k.iter().any(|a| a.key == key))
    }

    fn sum<'a>(v: &'a [Act], pred: impl Fn(&Act) -> bool + 'a) -> u128 {
        v.iter().filter(|a| pred(a)).fold(0u128, |s, a| s.saturating_add(a.amount))
    }

    fn oldest(v: &[Act], pred: impl Fn(&Act) -> bool) -> Option<u64> {
        v.iter().filter(|a| pred(a)).map(|a| a.at).min()
    }

    pub fn frozen(&self) -> Option<&Cause> {
        match &self.freeze {
            Freeze::Open => None,
            Freeze::Frozen { cause, .. } => Some(cause),
        }
    }

    /// Ask whether one act may go out now. Allow does NOT count it: call [`Outflow::record`] once the act is committed (its
    /// signed bytes saved). A Wait or a trip counts the ask, once per key.
    pub fn check(&mut self, l: &Limits, key: &str, collection: &str, flow: Flow, amount: u128, now: u64) -> Verdict {
        self.prune(now);
        if let Some(c) = self.frozen() {
            return Verdict::Frozen(c.clone());
        }
        if !self.asked.iter().any(|a| a.key == key) {
            self.asked.push(Act { key: key.into(), collection: collection.into(), flow, amount, at: now });
        }
        // the limits that apply to this act: (scope, limit)
        let (per, global) = match flow {
            Flow::Item | Flow::Return => (Some(l.per_collection_items as u128), l.global_items as u128),
            Flow::Mint => (Some(l.per_collection_mints as u128), l.global_mints as u128),
            Flow::Value => (None, l.global_value_wei),
        };
        let same_c = |a: &Act| a.flow == flow && a.collection == collection;
        let same_f = |a: &Act| a.flow == flow;

        // the tripwire first: demand, not what went out. Never for returns (see `Flow::Return`); never an ask a resume
        // acknowledged (it still waits on the limits below)
        let trips = if flow == Flow::Return { [(None, true), (None, false)] } else { [(per, true), (Some(global), false)] };
        let fresh_c = |a: &Act| same_c(a) && !self.is_acked(&a.key);
        let fresh_f = |a: &Act| same_f(a) && !self.is_acked(&a.key);
        for (limit, scoped) in trips {
            let Some(limit) = limit else { continue };
            let demanded = if scoped { Self::sum(&self.asked, fresh_c) } else { Self::sum(&self.asked, fresh_f) };
            if demanded >= limit.saturating_mul(l.trip_factor as u128) && demanded > 0 {
                let cause = Cause::Tripwire {
                    collection: if scoped { collection.into() } else { "*".into() },
                    flow, demanded, limit,
                };
                self.freeze = Freeze::Frozen { since: now, cause: cause.clone() };
                return Verdict::Frozen(cause);
            }
        }
        // then the limits: what went out, plus this act
        let out_c = Self::sum(&self.done, same_c);
        let out_f = Self::sum(&self.done, same_f);
        if let Some(per) = per {
            if out_c.saturating_add(amount) > per {
                let until = Self::oldest(&self.done, same_c).map_or(now, |t| t + WINDOW_SECS);
                return Verdict::Wait { until, why: format!("{collection}: {out_c} of {per} {flow:?} in the last 24 h") };
            }
        }
        if out_f.saturating_add(amount) > global {
            let until = Self::oldest(&self.done, same_f).map_or(now, |t| t + WINDOW_SECS);
            return Verdict::Wait { until, why: format!("all collections: {out_f} of {global} {flow:?} in the last 24 h") };
        }
        Verdict::Allow
    }

    /// Count an act that went out. Idempotent per key: recording the same act twice (a restart between saving and recording)
    /// counts it once.
    pub fn record(&mut self, key: &str, collection: &str, flow: Flow, amount: u128, now: u64) {
        self.prune(now);
        if !self.done.iter().any(|a| a.key == key) {
            self.done.push(Act { key: key.into(), collection: collection.into(), flow, amount, at: now });
        }
        if !self.asked.iter().any(|a| a.key == key) {
            self.asked.push(Act { key: key.into(), collection: collection.into(), flow, amount, at: now });
        }
    }

    /// Freeze by hand (the guardian's pause, an outside canary). An existing freeze keeps its first cause.
    pub fn freeze(&mut self, note: &str, now: u64) {
        if matches!(self.freeze, Freeze::Open) {
            self.freeze = Freeze::Frozen { since: now, cause: Cause::Manual { note: note.into() } };
        }
    }

    /// Lift a freeze. `authorized` is the wiring's answer to "is this the controller?": after the handover, the timelock
    /// forwarding k guardians' `resume_target`. The asks that tripped it are acknowledged (audit D, H-2): when they ask again
    /// they wait on the limits like any act, but they no longer count toward the trip, so the same backlog does not re-freeze
    /// custody at once. New acts still do. What went out still counts against the limits.
    pub fn unfreeze(&mut self, authorized: bool) -> Result<(), String> {
        if !authorized {
            return Err("refused: only a controller lifts a freeze (after the handover, k guardians through the timelock)".into());
        }
        self.freeze = Freeze::Open;
        let done: Vec<String> = self.done.iter().map(|a| a.key.clone()).collect();
        let (keep, ack): (Vec<Act>, Vec<Act>) = self.asked.drain(..).partition(|a| done.contains(&a.key));
        self.asked = keep;
        let acked = self.acked.get_or_insert_with(Vec::new);
        for a in ack {
            if !acked.iter().any(|x| x.key == a.key) {
                acked.push(a);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: u64 = WINDOW_SECS;
    const T0: u64 = 1_780_000_000;
    const E: u128 = 1_000_000_000_000_000_000;
    const FROGS: &str = "btc:0x1111111111111111111111111111111111111111";
    const APES: &str = "1:0xbc4ca0eda7647a8ab7c2061c2e118a18a936f13d";

    fn stage1() -> Limits {
        Limits { per_collection_items: 6, global_items: 10, per_collection_mints: 6, global_mints: 10, global_value_wei: 20 * E, trip_factor: 3 }
    }

    fn go(o: &mut Outflow, l: &Limits, key: &str, c: &str, f: Flow, amt: u128, now: u64) -> Verdict {
        let v = o.check(l, key, c, f, amt, now);
        if v == Verdict::Allow {
            o.record(key, c, f, amt, now);
        }
        v
    }

    #[test]
    fn limits_are_validated() {
        assert!(validate(&stage1()).is_ok());
        assert!(validate(&Limits { trip_factor: 1, ..stage1() }).unwrap_err().contains("at least 2"));
        assert!(validate(&Limits { per_collection_items: 11, ..stage1() }).unwrap_err().contains("exceed"));
        assert!(validate(&Limits { per_collection_mints: 11, ..stage1() }).unwrap_err().contains("exceed"));
        assert!(validate(&Limits { per_collection_items: 0, ..stage1() }).unwrap_err().contains("zero"));
    }

    #[test]
    fn the_seventh_delivery_of_a_day_waits_until_the_first_leaves_the_window() {
        let (l, mut o) = (stage1(), Outflow::default());
        for i in 0..6 {
            assert_eq!(go(&mut o, &l, &format!("t{i}"), FROGS, Flow::Item, 1, T0 + i * 60), Verdict::Allow);
        }
        match go(&mut o, &l, "t6", FROGS, Flow::Item, 1, T0 + 3_600) {
            Verdict::Wait { until, why } => {
                assert_eq!(until, T0 + DAY, "the first delivery leaves the window a day after it went");
                assert!(why.contains("6 of 6"), "{why}");
            }
            v => panic!("{v:?}"),
        }
        // one second before the window slides: still waiting; at it: allowed
        assert!(matches!(go(&mut o, &l, "t6", FROGS, Flow::Item, 1, T0 + DAY - 1), Verdict::Wait { .. }));
        assert_eq!(go(&mut o, &l, "t6", FROGS, Flow::Item, 1, T0 + DAY), Verdict::Allow);
    }

    #[test]
    fn a_collection_limit_does_not_spend_another_collections_room_but_the_global_one_is_shared() {
        let (l, mut o) = (stage1(), Outflow::default());
        for i in 0..6 {
            assert_eq!(go(&mut o, &l, &format!("f{i}"), FROGS, Flow::Item, 1, T0), Verdict::Allow);
        }
        for i in 0..4 {
            assert_eq!(go(&mut o, &l, &format!("a{i}"), APES, Flow::Item, 1, T0), Verdict::Allow, "apes have their own 6");
        }
        match go(&mut o, &l, "a4", APES, Flow::Item, 1, T0) {
            Verdict::Wait { why, .. } => assert!(why.contains("all collections: 10 of 10"), "{why}"),
            v => panic!("the global 10 binds first: {v:?}"),
        }
        // mints and items are separate budgets
        assert_eq!(go(&mut o, &l, "m0", FROGS, Flow::Mint, 1, T0), Verdict::Allow);
    }

    #[test]
    fn value_is_counted_in_wei_against_the_global_budget() {
        let (l, mut o) = (stage1(), Outflow::default());
        assert_eq!(go(&mut o, &l, "fill1", APES, Flow::Value, 15 * E, T0), Verdict::Allow);
        assert!(matches!(go(&mut o, &l, "fill2", APES, Flow::Value, 6 * E, T0 + 1), Verdict::Wait { .. }));
        assert_eq!(go(&mut o, &l, "fill3", FROGS, Flow::Value, 5 * E, T0 + 2), Verdict::Allow, "exactly the budget fits");
    }

    #[test]
    fn a_forged_run_of_tickets_trips_the_freeze_and_nothing_goes_out_after() {
        // colluding providers forge 30 resolved tickets naming the attacker (docs/v5/custody-threat-model.md, T10)
        let (l, mut o) = (stage1(), Outflow::default());
        let mut sent = 0;
        let mut verdict = Verdict::Allow;
        for i in 0..30 {
            verdict = go(&mut o, &l, &format!("forged{i}"), FROGS, Flow::Item, 1, T0 + i);
            if verdict == Verdict::Allow {
                sent += 1;
            }
            if matches!(verdict, Verdict::Frozen(_)) {
                break;
            }
        }
        assert_eq!(sent, 6, "the limit, and not one more");
        match verdict {
            Verdict::Frozen(Cause::Tripwire { collection, flow, demanded, limit }) => {
                assert_eq!((collection.as_str(), flow, demanded, limit), (FROGS, Flow::Item, 18, 6), "3 × 6 distinct asks");
            }
            v => panic!("{v:?}"),
        }
        // frozen: every flow, every collection, whatever the window says, a day later too
        assert!(matches!(go(&mut o, &l, "honest", APES, Flow::Item, 1, T0 + 2 * DAY), Verdict::Frozen(_)));
        assert!(matches!(go(&mut o, &l, "mint", APES, Flow::Mint, 1, T0 + 2 * DAY), Verdict::Frozen(_)));
    }

    #[test]
    fn the_same_ticket_asked_every_tick_is_one_ask_not_a_run() {
        let (l, mut o) = (stage1(), Outflow::default());
        for i in 0..6 {
            go(&mut o, &l, &format!("t{i}"), FROGS, Flow::Item, 1, T0);
        }
        // one waiting delivery, asked once a minute for a day, never trips the wire
        for m in 0..(24 * 60) {
            let v = go(&mut o, &l, "t6", FROGS, Flow::Item, 1, T0 + 1 + m * 60);
            assert!(!matches!(v, Verdict::Frozen(_)), "minute {m}: {v:?}");
        }
        assert_eq!(o.asked.len(), 7);
    }

    #[test]
    fn only_a_controller_lifts_a_freeze_and_the_same_demand_does_not_re_trip_at_once() {
        let (l, mut o) = (stage1(), Outflow::default());
        for i in 0..18 {
            go(&mut o, &l, &format!("x{i}"), FROGS, Flow::Item, 1, T0);
        }
        assert!(o.frozen().is_some());
        assert!(o.unfreeze(false).unwrap_err().contains("k guardians"));
        assert!(o.frozen().is_some());
        o.unfreeze(true).unwrap();
        assert!(o.frozen().is_none());
        // what went out still counts: the next one waits, it does not trip
        assert!(matches!(go(&mut o, &l, "x7", FROGS, Flow::Item, 1, T0 + 60), Verdict::Wait { .. }));
    }

    #[test]
    fn a_manual_freeze_keeps_the_first_cause_and_stops_everything() {
        let (l, mut o) = (stage1(), Outflow::default());
        o.freeze("guardian g1 paused: canary saw an unexpected spend from custody", T0);
        o.freeze("a second note", T0 + 1);
        match go(&mut o, &l, "t", FROGS, Flow::Item, 1, T0 + 2) {
            Verdict::Frozen(Cause::Manual { note }) => assert!(note.contains("canary")),
            v => panic!("{v:?}"),
        }
    }

    #[test]
    fn recording_is_idempotent_and_old_acts_leave_the_window() {
        let (l, mut o) = (stage1(), Outflow::default());
        o.record("t0", FROGS, Flow::Item, 1, T0);
        o.record("t0", FROGS, Flow::Item, 1, T0 + 5);
        assert_eq!(o.done.len(), 1, "a restart that records twice counts once");
        let _ = o.check(&l, "t1", FROGS, Flow::Item, 1, T0 + DAY + 1);
        assert!(o.done.is_empty(), "a day later the window is empty again: state stays bounded");
        // a clock that runs backwards (an act dated in the future) is not counted as inside the window
        o.record("future", FROGS, Flow::Item, 1, T0 + 10 * DAY);
        let _ = o.check(&l, "t2", FROGS, Flow::Item, 1, T0 + DAY + 2);
        assert!(o.done.is_empty());
    }

    /// Audit D, H-2: returns are their own budget and never the tripwire; a resume acknowledges the asks that tripped it, so the
    /// same backlog asking again waits instead of re-freezing, while NEW asks still trip it.
    #[test]
    fn returns_never_trip_and_a_resume_does_not_re_freeze_on_the_same_backlog_but_new_demand_still_does() {
        let (l, mut o) = (Limits::stage0(), Outflow::default());
        for i in 0..50 {
            assert!(!matches!(go(&mut o, &l, &format!("return:{i}"), FROGS, Flow::Return, 1, T0 + i), Verdict::Frozen(_)));
        }
        assert_eq!(o.done.iter().filter(|a| a.flow == Flow::Return).count(), 3, "three returns a day, as items");
        assert_eq!(go(&mut o, &l, "ticket:0", FROGS, Flow::Item, 1, T0 + 60), Verdict::Allow, "returns never spend the deliveries' budget");
        // nine distinct delivery asks at stage 0 trip it
        for i in 1..9 {
            go(&mut o, &l, &format!("ticket:{i}"), FROGS, Flow::Item, 1, T0 + 100 + i);
        }
        assert!(matches!(o.frozen(), Some(Cause::Tripwire { flow: Flow::Item, .. })));
        o.unfreeze(true).unwrap();
        // the same eight asking again, every tick: they wait on the limit; they do not re-freeze
        for t in 0..10 {
            for i in 1..9 {
                let v = go(&mut o, &l, &format!("ticket:{i}"), FROGS, Flow::Item, 1, T0 + 1_000 + t * 600);
                assert!(!matches!(v, Verdict::Frozen(_)), "tick {t}, ticket {i}: {v:?}");
            }
        }
        // a new run of distinct asks is demand again
        let mut froze = false;
        for i in 100..120 {
            froze |= matches!(go(&mut o, &l, &format!("ticket:{i}"), FROGS, Flow::Item, 1, T0 + 10_000 + i), Verdict::Frozen(_));
        }
        assert!(froze, "new demand still trips the wire");
        // and the acknowledgement leaves with the window
        let _ = o.check(&l, "x", FROGS, Flow::Mint, 1, T0 + 3 * DAY);
        assert!(o.acked.clone().unwrap_or_default().is_empty());
    }

    /// Audit D, I-5: the limits follow the caps' ×4 rule at upgrade.
    #[test]
    fn a_limit_rises_at_most_four_times_per_upgrade_and_falls_freely() {
        let s0 = Limits::stage0();
        let x4 = Limits { per_collection_items: 12, global_items: 24, per_collection_mints: 12, global_mints: 24,
            global_value_wei: 100 * E, trip_factor: 3 };
        assert_eq!(validate_change(&s0, &x4), Ok(()));
        assert_eq!(validate_change(&x4, &s0), Ok(()), "lowering is free");
        assert!(validate_change(&s0, &Limits { global_value_wei: 100 * E + 1, ..x4.clone() }).unwrap_err().contains("global_value_wei"));
        assert!(validate_change(&s0, &Limits { per_collection_items: 13, global_items: 24, ..x4.clone() }).unwrap_err().contains("×4"));
        assert!(validate_change(&s0, &Limits { trip_factor: 13, ..s0.clone() }).unwrap_err().contains("trip_factor"));
    }

    #[test]
    fn a_bank_run_must_fit_inside_the_timelocks_delay() {
        let three_days = 3 * DAY;
        // stage 1: 12 items a collection at 6 a day leave in 2 days, inside a 72-hour delay less a day
        assert!(exit_fits(12, 6, three_days));
        assert!(!exit_fits(13, 6, three_days), "13 items need 3 days, and only 2 are usable");
        assert!(!exit_fits(1, 0, three_days));
        // stage 3 needs a longer delay, or a higher limit (docs/v5/launch-gates.md)
        assert!(!exit_fits(192, 24, three_days));
        assert!(exit_fits(192, 32, 7 * DAY));
    }
}
