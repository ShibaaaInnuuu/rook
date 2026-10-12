//! The pool desk's state (W7): per collection, its DLMM pair and position once opened, the sides and the inventory as last read, the fee
//! income the coins sent it, its arbs and its P&L; the links in flight, the waits, the day's budget. One versioned candid blob
//! ([`StablePool`]) carried across an upgrade in the lines' `pool_state` slot, like `coins::state`. Read and written only in synchronous
//! sections, never across an await.
use std::cell::RefCell;
use std::collections::BTreeMap;

use candid::CandidType;
use serde::{Deserialize, Serialize};

use crate::soltx::Pubkey;

use super::plan;
use super::step::PoolStep;

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct DeskBook {
    /// The pair, once it exists on chain.
    pub pair: Option<Pubkey>,
    /// The position and its bins `[lo, hi]`, once it exists on chain.
    pub position: Option<Pubkey>,
    pub lo: i32,
    pub hi: i32,
    /// The pair's active bin as last read.
    pub active: i32,
    /// The position's sides as last read: wX base units and lamports.
    pub pos_x: u64,
    pub pos_y: u64,
    /// The desk's holdings as last read: native lamports, wSOL, wX base units.
    pub native: u64,
    pub wsol: u64,
    pub wx: u64,
    /// What the coins' fee cashes sent the desk (§5.3): lamports and wX base units, in all.
    pub fee_lamports: u64,
    pub fee_units: u64,
    /// Pool fees claimed (lamports), and what went to the treasury.
    pub claimed: u64,
    pub swept: u64,
    /// Arb times (unix seconds) of the last hour, and the profit the landed arbs quoted (lamports).
    pub arbs: Vec<u64>,
    pub arb_profit: u64,
    /// The pass that last read the chain.
    pub read_at: u64,
    /// §10.4 `pause_desk(i)`: this desk does nothing (reads apart) until resumed.
    #[serde(default)]
    pub off: bool,
}

/// A linked step in flight (a mint, an up-arb, a burn): its books effect is the link's; the desk's own book moves when it lands.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Receipt {
    pub link: u64,
    pub step: PoolStep,
    pub at: u64,
}

/// A step that could not be made, and why: skipped by passes until `retry_at`.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Wait {
    pub key: String,
    pub since: u64,
    pub why: String,
    pub retry_at: u64,
    pub tries: u32,
}

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct PoolState {
    pub desks: BTreeMap<u32, DeskBook>,
    pub receipts: Vec<Receipt>,
    pub waits: Vec<Wait>,
    /// A counter for the idempotence keys of the links this module begins.
    pub ops: u64,
    /// The day (unix / 86,400) the counter below counts.
    pub day: u64,
    pub signed_today: u32,
    /// Each step kind's last landing: `(kind, when, what)`.
    pub last: Vec<(String, u64, String)>,
    /// The last [`HISTORY_KEPT`] steps that landed, oldest first (the monitor's and the tests' record).
    pub history: Vec<(u64, PoolStep)>,
    /// The operator's switch for the whole desk.
    pub off: bool,
}

pub const HISTORY_KEPT: usize = 32;

pub const WAITS_KEPT: usize = 64;
/// A refused or failed step is tried again after this long (the arb's own cadence is the pass's; a failed swap only cost its fee).
pub const WAIT_RETRY_SECS: u64 = 1_800;
/// Steps signed a day, at most (≈ 26 B cycles a signature; 12 arbs an hour per collection would be far more than this: the budget is the
/// bound the operator can raise).
pub const STEPS_PER_DAY: u32 = 288;

impl PoolState {
    pub fn waiting(&self, key: &str, now: u64) -> bool {
        self.waits.iter().any(|w| w.key == key && now < w.retry_at)
    }
    pub fn wait(&mut self, key: String, why: String, now: u64) {
        match self.waits.iter_mut().find(|w| w.key == key) {
            Some(w) => {
                w.why = why;
                w.retry_at = now + WAIT_RETRY_SECS;
                w.tries += 1;
            }
            None => {
                self.waits.push(Wait { key, since: now, why, retry_at: now + WAIT_RETRY_SECS, tries: 1 });
                if self.waits.len() > WAITS_KEPT {
                    self.waits.remove(0);
                }
            }
        }
    }
    pub fn clear_wait(&mut self, key: &str) {
        self.waits.retain(|w| w.key != key);
    }
    pub fn roll_day(&mut self, now: u64) {
        let d = now / 86_400;
        if d != self.day {
            self.day = d;
            self.signed_today = 0;
        }
    }
    pub fn landed(&mut self, step: &PoolStep, now: u64) {
        self.last.retain(|(k, ..)| k != step.kind());
        self.last.push((step.kind().to_string(), now, step.label()));
        self.history.push((now, step.clone()));
        if self.history.len() > HISTORY_KEPT {
            self.history.remove(0);
        }
    }
}

/// The stable layout. A new layout is a new variant.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum StablePool {
    V1(PoolState),
}

pub fn encode_state(s: &PoolState) -> Vec<u8> {
    candid::encode_one(StablePool::V1(s.clone())).expect("the pool state encodes")
}

pub fn decode_state(bytes: &[u8]) -> Result<PoolState, String> {
    match candid::decode_one::<StablePool>(bytes).map_err(|e| format!("the pool state does not decode: {e}"))? {
        StablePool::V1(s) => Ok(s),
    }
}

thread_local! {
    static STATE: RefCell<PoolState> = RefCell::new(PoolState::default());
}

pub fn with<R>(f: impl FnOnce(&PoolState) -> R) -> R {
    STATE.with(|s| f(&s.borrow()))
}

pub fn with_mut<R>(f: impl FnOnce(&mut PoolState) -> R) -> R {
    STATE.with(|s| f(&mut s.borrow_mut()))
}

pub fn restore(bytes: &[u8]) -> Result<(), String> {
    let s = decode_state(bytes)?;
    STATE.with(|c| *c.borrow_mut() = s);
    Ok(())
}

/// A fresh, empty state (tests).
pub fn reset() {
    STATE.with(|c| *c.borrow_mut() = PoolState::default());
}

/// `pre_upgrade`: the desk goes into the saved lines' blob (skipped when empty).
pub fn before_upgrade() {
    let blob = with(|s| (*s != PoolState::default()).then(|| encode_state(s)));
    crate::solhome::LINES.with(|l| l.borrow_mut().pool_state = blob);
}

/// `post_upgrade`, after the lines are restored: the desk comes out of the blob. A blob that does not decode refuses the upgrade.
pub fn after_upgrade() -> Result<(), String> {
    match crate::solhome::LINES.with(|l| l.borrow_mut().pool_state.take()) {
        Some(b) => restore(&b),
        None => {
            reset();
            Ok(())
        }
    }
}

/// §5.3: whether collection `coll`'s desk takes `q_pool` of its fees: while the desk is on, and its pool is not open yet or is under
/// its target depth at the accepted floor.
pub fn wants_fees(coll: u32) -> bool {
    let floor = crate::desk::state::with(|s| s.books.get(coll as u16).map(|c| c.module.accepted).unwrap_or(0));
    with(|s| {
        if s.off {
            return false;
        }
        match s.desks.get(&coll) {
            Some(d) if d.pair.is_some() => plan::under_target(d.pos_x, d.pos_y, floor),
            _ => true,
        }
    })
}

/// A landed fee cash sent the desk `lamports` and `units` wX (`coins::fees::apply_cash`).
pub fn note_fee_income(coll: u32, lamports: u64, units: u64) {
    with_mut(|s| {
        let d = s.desks.entry(coll).or_default();
        d.fee_lamports = d.fee_lamports.saturating_add(lamports);
        d.fee_units = d.fee_units.saturating_add(units);
    });
}
