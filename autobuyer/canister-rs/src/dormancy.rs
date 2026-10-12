//! Dormant, wake-on-demand collections (docs/v7/dormant-collections.md), pure: the configuration and its validation, each
//! collection's life cycle (`DORMANT → WAKING → ACTIVE → DORMANT`), the wake queue and its caps, the slow cadence, the backfill's
//! completeness, the "warming up (~N min)" estimate and the fresh-read rule before a buy. No network, no keys, no clock: the flow
//! (`ethflow.rs`) feeds in what the chains said and what time it is.
//!
//! With `eth_nft.dormancy = None` nothing here applies: every collection is `Always` (ACTIVE all the time), as before this change.

use candid::CandidType;
use serde::{Deserialize, Serialize};

use crate::ethflow::SaleScan;

// ---- the cadence (§4.1) ------------------------------------------------------------------------------------------------------------

/// ACTIVE, not hot: an evidence pass (listings, the reading, a post when due) at most this often. With the runner's 20-minute
/// hand-ins that is a pass about every 40 minutes, inside the module's one-hour `FLOOR_TTL` (a stale module refuses burns).
pub const PASS_EVERY_SECS: u64 = 1_800;
/// ACTIVE, not hot: the sale scan inside a pass at most this often, doubled after each scan that counted no new sale, up to
/// `SCAN_BACKOFF_MAX_SECS`, and back to this at a new sale.
pub const SCAN_EVERY_SECS: u64 = 3_600;
pub const SCAN_BACKOFF_MAX_SECS: u64 = 4 * 3_600;
/// ACTIVE, not hot: the module read that decides a new buy at most this often.
pub const VIEW_EVERY_SECS: u64 = 1_800;
/// Hot this long after a module view wanted an item, or a fill waited for a fresh read.
pub const HOT_SECS: u64 = 3_600;
/// The re-dormancy reads, an ACTIVE collection's launch count and the DORMANT watchdog, at most this often per collection.
pub const SLEEP_CHECK_SECS: u64 = 6 * 3_600;
/// At most this many collections' sleep checks (or watchdogs) a tick: their reads are spread over ticks.
pub const CHECKS_PER_TICK: usize = 2;
/// WAKING: receipts judged a pass (the pass's and the day's byte budgets still bound what they reserve).
pub const WAKE_RECEIPTS_PER_PASS: usize = 64;
/// New desk requests read a tick.
pub const DESK_READS_PER_TICK: u64 = 4;
/// Independent review DI-M-1: a desk request's wake starts only while it can still post inside the request's window, with this margin
/// after the wake's timeout for the post's `markServed` to mine: `now + wake_timeout_secs + DESK_START_MARGIN_SECS <= at + window`. Past
/// that the request is dropped from the queue and never started late: its bond goes back unless the collection is posted inside the
/// window by another wake (the desk's rule, independent review 2 D2-L-3: any post inside the window serves the request, whoever caused
/// it, and anyone may record it).
pub const DESK_START_MARGIN_SECS: u64 = 6 * 3_600;
/// ACTIVE after a wake with no position at all for this long (no unit, item, graduation, advance or float on 4663, nothing open in the
/// canister): back to DORMANT (dormant-review DR-H-1: a bare launch costs gas alone, so it is not a position and never keeps a
/// collection awake; its graduation wakes it again, owed).
/// Six hours (dormant-review DR-M-3): a collection that was active once takes launches while it sleeps (its module keeps its
/// `accepted`, so `hasRoomFor` holds), so a wake needs to stay up only for what needs a fresh floor; a wake nobody uses holds its slot
/// (`max_active`) for the backfill and six hours, never a day.
pub const NO_POSITION_SECS: u64 = 6 * 3_600;
/// The fill hunt (§5, dormant-review DR-M-2): the operator's hand-ins (`eth_fill`) first, then only the newest coins of the factory, a few
/// a try, and a try at most every `FILL_RETRY_SECS` (a flood of launched coins can never make the hunt read them all).
pub const COINS_PER_FILL_TICK: u64 = 8;
pub const FILL_RETRY_SECS: u64 = 1_800;
/// Fill hand-ins kept at once.
pub const FILLS_MAX: usize = 8;
/// Owed wakes (a deposit, a queued graduation) may start with this many ACTIVE + WAKING above `max_active` (dormant-review DR-H-1): the
/// slots bought by desk and operator wakes can never keep a wake that is owed from starting.
pub const OWED_RESERVE: usize = 1;
/// An owed wake that timed out waits `cooldown_secs × 2^(fails − 1)` (at most `OWED_BACKOFF_MAX_SECS`) before it may start again, and
/// after `OWED_FAILS_MAX` timed-out wakes a deposit that asked for them goes back to its sender (dormant-review DR-H-2): one NFT sent
/// to the canister can never hold the waking slot for good.
pub const OWED_FAILS_MAX: u8 = 3;
pub const OWED_BACKOFF_MAX_SECS: u64 = 7 * 86_400;
/// The module's `FLOOR_TTL`, and the margin before it at which an ACTIVE collection's pass is due whatever its cadence says
/// (dormant-review DR-M-3: a late or failed hand-in or two never lets a module with holders go stale).
pub const FLOOR_TTL_SECS: u64 = 3_600;
pub const TTL_MARGIN_SECS: u64 = 25 * 60;
/// A backfill is complete with at most this many of its receipts backed off (unreadable or rate-limited after their fast tries):
/// a handful of unreadable receipts never holds a wake until its timeout.
pub const BACKFILL_BACKED_OFF_MAX: usize = 8;

// ---- the fresh read before a buy (§4.3) ------------------------------------------------------------------------------------------

/// The reading a fill is judged against is at most this old ...
pub const BUY_FRESH_SECS: u64 = 900;
/// ... its pass ran the sale scan, which ended at most this many final blocks behind (about an hour) ...
pub const BUY_SCAN_LAG_BLOCKS: u64 = 300;
/// ... it is within this of the module's accepted floor ...
pub const BUY_MISMATCH_BPS: u128 = 500;
/// ... and within this of the floor the buy's advance was sized on.
pub const BUY_DRIFT_BPS: u128 = 1_000;

// ---- the estimate (§4.2) --------------------------------------------------------------------------------------------------------

/// A waking collection's evidence is handed in every runner cycle (a tick, 600 s), and the post mines a tick after the last pass.
pub const WAKE_PASS_SECS: u64 = 600;
/// The moves (transfer transactions) a 7-day window is assumed to hold before any has been read.
pub const WAKE_DEFAULT_MOVES: u64 = 700;

// ---- the configuration (§3.2) ---------------------------------------------------------------------------------------------------

pub const MAX_ACTIVE_CEILING: u8 = 16;
pub const MAX_WAKES_PER_DAY: u8 = 24;
pub const WAKE_TIMEOUT_MIN_SECS: u64 = 3_600;
pub const WAKE_TIMEOUT_MAX_SECS: u64 = 172_800;
pub const COOLDOWN_MIN_SECS: u64 = 3_600;
pub const COOLDOWN_MAX_SECS: u64 = 30 * 86_400;
pub const IDLE_DAYS_MAX: u16 = 60;

// ---- the auto-scaling from the canister's own cycles balance (§11) ---------------------------------------------------------------

/// A billion cycles: the scaling's configuration is in B (whole numbers, no `nat` in the install argument), its arithmetic in cycles.
pub const B: u128 = 1_000_000_000;
/// Bounds of `eth_nft.dormancy.scaling` (§11.2), in B cycles or days. The reserve can never be set under the canister's own scan floor
/// (`ethflow::EVIDENCE_CYCLES_FLOOR`, 8 T): under it the sale scan stops, so no wake can finish its backfill and no buy passes the fresh
/// read; what is under it is the wind-down's (posts, deliveries, returns), never a new wake's.
pub const SCALE_ACTIVE_B_MIN: u64 = 500;
pub const SCALE_ACTIVE_B_MAX: u64 = 100_000;
pub const SCALE_IDLE_B_MAX: u64 = 10_000;
pub const SCALE_WAKE_B_MIN: u64 = 100;
pub const SCALE_WAKE_B_MAX: u64 = 50_000;
pub const SCALE_RUNWAY_DAYS_MIN: u16 = 1;
pub const SCALE_RUNWAY_DAYS_MAX: u16 = 60;
pub const SCALE_RESERVE_B_MAX: u64 = 1_000_000;
pub const SCALE_EXIT_B_MAX: u64 = 100_000;

/// `eth_nft.dormancy.scaling` (§11): the limits scale with the canister's own cycles balance. `max_active` and `wakes_per_day` stay as
/// the hard ceilings (governance, timelocked); under them, a collection may be awake only while the balance pays for every awake one's
/// `runway_days` plus the reserve, and a wake starts only while the balance after its one-time cost still does with one more awake.
/// Every figure is the conservative (high) end of what was measured or estimated (§7), in B cycles.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Scaling {
    /// What one ACTIVE collection burns a day (posts, passes, the scan, views and checks): 2,500 (2.5 T) at launch.
    pub active_b_per_day: u64,
    /// The idle base a day (the pin, the chain check and the desk read, whatever is awake): 300 at launch.
    pub idle_b_per_day: u64,
    /// One wake's one-time cost (the 7-day backfill and its passes): 1,500 at launch.
    pub wake_b: u64,
    /// The days of burn the balance must cover for every awake collection: 7 at launch.
    pub runway_days: u16,
    /// Held back whatever is awake, never spent on a wake or a runway: at least the scan floor (8,000). Under it the canister stops its
    /// own sale scan and serves no new wake or buy; the posts, deliveries and returns that wind a collection down still run.
    pub reserve_b: u64,
    /// Held back for each awake collection on top of the reserve: its exits, returns and owed work (about two days of posts without the
    /// scan, so holders can burn and leave, and its deliveries): 3,000 at launch.
    pub exit_b_per_awake: u64,
}

impl Scaling {
    /// The launch numbers (docs §11.2).
    pub fn launch() -> Self {
        Scaling { active_b_per_day: 2_500, idle_b_per_day: 300, wake_b: 1_500, runway_days: 7, reserve_b: 8_000, exit_b_per_awake: 3_000 }
    }
    fn c(b: u64) -> u128 {
        (b as u128).saturating_mul(B)
    }
    /// The cycles each awake collection takes: its runway and its exit reserve.
    pub fn slot(&self) -> u128 {
        Self::c(self.active_b_per_day).saturating_mul(self.runway_days as u128).saturating_add(Self::c(self.exit_b_per_awake))
    }
    /// The cycles held whatever is awake: the reserve and the idle base's runway.
    pub fn fixed(&self) -> u128 {
        Self::c(self.reserve_b).saturating_add(Self::c(self.idle_b_per_day).saturating_mul(self.runway_days as u128))
    }
    pub fn wake(&self) -> u128 {
        Self::c(self.wake_b)
    }
    /// The balance that keeps `n` collections awake for the runway, with the reserve and every one's exit reserve.
    pub fn need(&self, n: u128) -> u128 {
        self.fixed().saturating_add(self.slot().saturating_mul(n))
    }
    /// The part of the reserve an owed wake may never touch: the reserve and the exit reserve of `n` awake collections (owed work is
    /// what the runways are for; it may use them, never the reserve).
    pub fn reserve_for(&self, n: u128) -> u128 {
        Self::c(self.reserve_b).saturating_add(Self::c(self.exit_b_per_awake).saturating_mul(n))
    }
}

/// The balance the scaling may count: the liquid balance less the one-time cost still to come of each wake already started (a waking
/// collection's backfill is paid over its passes, so a balance read mid-backfill has not paid it yet: counting it twice is the safe side).
pub fn available(s: &Scaling, balance: u128, waking: usize) -> u128 {
    balance.saturating_sub(s.wake().saturating_mul(waking as u128))
}

/// `effective_max_active` (§11.1): `min(ceiling, ⌊(balance − fixed) / slot⌋)`, and 0 when the balance does not cover the fixed part.
/// It is at least 1 exactly when the balance covers one collection's runway, its exit reserve and the reserve. Never overflows: the
/// configuration bounds make `slot` positive, and every step saturates or divides.
pub fn effective_max_active(s: &Scaling, balance: u128, ceiling: u8) -> u8 {
    let fixed = s.fixed();
    if balance < fixed {
        return 0;
    }
    let n = (balance - fixed) / s.slot().max(1);
    n.min(ceiling as u128) as u8
}

/// Whether a wake that is not owed fits now (§11.3): the balance after its one-time cost still keeps `awake + 1` collections for the
/// runway, with the reserve.
pub fn wake_fits(s: &Scaling, balance: u128, awake: usize) -> bool {
    balance >= s.wake() && balance - s.wake() >= s.need(awake as u128 + 1)
}

/// Whether an owed wake (a deposit, a queued graduation) fits: after its one-time cost the balance still holds the reserve and the exit
/// reserve of every awake collection and this one. Owed work may use the runways, never the reserve (§11.1), and at most `OWED_RESERVE`
/// slot above `effective_max_active` (`start_wakes`).
pub fn owed_wake_fits(s: &Scaling, balance: u128, awake: usize) -> bool {
    balance >= s.wake() && balance - s.wake() >= s.reserve_for(awake as u128 + 1)
}

/// The wakes that fit now, one after another (§11.3): the most `k` with `balance − k × wake ≥ need(awake + k)`, within the ceiling.
/// The day's cap (`wakes_per_day`) is separate.
pub fn wake_headroom(s: &Scaling, balance: u128, awake: usize, ceiling: u8) -> u8 {
    let base = s.need(awake as u128);
    if balance < base {
        return 0;
    }
    let k = (balance - base) / s.slot().saturating_add(s.wake()).max(1);
    k.min((ceiling as usize).saturating_sub(awake) as u128) as u8
}

/// What the canister publishes about its capacity each tick (`Dorm.cap`, `eth_nft_capacity()`): the monitor pages on it and the site
/// says "N of M collections can be awake right now".
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct Capacity {
    /// When it was computed, and the liquid cycles balance then (`canister_liquid_cycle_balance128`: the freezing reserve excluded).
    pub at: u64,
    pub balance: u128,
    /// Whether the limits scale (`scaling` set); false: `effective_max_active` is the ceiling.
    pub scaling: bool,
    /// The collections configured, and the governance ceilings (`max_active`, `wakes_per_day`).
    pub collections: u8,
    pub ceiling: u8,
    pub wakes_per_day: u8,
    /// How many may be awake now, from the balance (never above the ceiling).
    pub effective_max_active: u8,
    /// ACTIVE + WAKING now, and of them those holding a position (on 4663 at the last check, or open in the canister): a collection with
    /// a position is never put to sleep, so it stays awake above the limit and counts against it.
    pub awake: u8,
    pub pinned: u8,
    /// The wakes that would start now, one after another (0 while the limit is reached or exceeded), and the day's wakes left.
    pub wake_headroom: u8,
    pub wakes_left_today: u8,
    /// The balance at which one more wake would start (`wake + need(awake + 1)` plus the waking ones' cost still to come).
    pub next_wake_at: u128,
    /// The human text.
    pub status: String,
}

/// The capacity now. `balance`: `None` when the canister could not say (a fake world): with scaling set that fails closed (no wake).
pub fn capacity(p: &Dormancy, d: &Dorm, balance: Option<u128>, collections: usize, pinned: usize, now: u64) -> Capacity {
    let (waking, active) = d.counts();
    let awake = waking + active;
    let today = d.wakes.iter().filter(|t| now < t.saturating_add(86_400)).count();
    let wakes_left = (p.wakes_per_day as usize).saturating_sub(today);
    let (eff, head, next_at) = match (&p.scaling, balance) {
        (None, _) => (p.max_active, (p.max_active as usize).saturating_sub(awake) as u8, 0),
        (Some(_), None) => (0, 0, u128::MAX),
        (Some(s), Some(b)) => {
            let a = available(s, b, waking);
            (effective_max_active(s, a, p.max_active), wake_headroom(s, a, awake, p.max_active),
                s.wake().saturating_mul(waking as u128 + 1).saturating_add(s.need(awake as u128 + 1)))
        }
    };
    let head = if awake >= p.max_active as usize { 0 } else { head };
    let status = if p.scaling.is_some() && balance.is_none() {
        "the cycles balance could not be read: no wake starts".to_string()
    } else if (eff as usize) < pinned {
        format!("{eff} of {collections} collections can be awake on this balance, but {pinned} hold positions: they stay awake; no new wake until a top-up")
    } else if awake > eff as usize {
        format!("{eff} of {collections} collections can be awake on this balance; {awake} are: no new wake until one sleeps or a top-up")
    } else {
        format!("{eff} of {collections} collections can be awake right now ({awake} awake)")
    };
    Capacity { at: now, balance: balance.unwrap_or(0), scaling: p.scaling.is_some(), collections: collections.min(255) as u8, ceiling: p.max_active,
        wakes_per_day: p.wakes_per_day, effective_max_active: eff, awake: awake.min(255) as u8, pinned: pinned.min(255) as u8,
        wake_headroom: head.min(wakes_left.min(255) as u8), wakes_left_today: wakes_left.min(255) as u8, next_wake_at: next_at, status }
}

/// `eth_nft.dormancy`: operational, in the install argument (a change is a published, timelocked act).
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Dormancy {
    /// ACTIVE + WAKING at most: the cycles budget.
    pub max_active: u8,
    /// Backfills at once.
    pub max_waking: u8,
    /// Wakes started in any 24 h, every source.
    pub wakes_per_day: u8,
    /// WAKING longer than this failed: back to DORMANT, with the cooldown.
    pub wake_timeout_secs: u64,
    /// After a failed wake or a return to DORMANT, the collection waits this long before a wake that is not owed.
    pub cooldown_secs: u64,
    /// ACTIVE with no launch this many days, and nothing open, goes DORMANT.
    pub idle_days: u16,
    /// The 4663 `WakeDesk` whose bonded requests the canister reads; `None`: no public path (the operator's `eth_wake` only).
    pub wake_desk: Option<String>,
    /// §11: the limits scale with the canister's cycles balance, under `max_active` and `wakes_per_day` as the hard ceilings; `None`:
    /// the ceilings are the limits (fixed caps, as before §11).
    pub scaling: Option<Scaling>,
}

impl Dormancy {
    /// The launch configuration's numbers (docs §3.2), with the desk given.
    pub fn launch(desk: Option<String>) -> Self {
        Dormancy { max_active: 10, max_waking: 1, wakes_per_day: 12, wake_timeout_secs: 43_200, cooldown_secs: 86_400, idle_days: 3,
            wake_desk: desk, scaling: Some(Scaling::launch()) }
    }
    /// The desk's address as the records key it: an EVM address, or on the Solana home (docs/v9 §4) the first 20 bytes of rook_core's
    /// `["desk"]` PDA named in base58 (`canister::solana_desk_ok` checks it is that PDA).
    pub fn desk(&self) -> Result<Option<[u8; 20]>, String> {
        self.wake_desk.as_deref().map(|w| crate::evmtx::parse_address(w).or_else(|e| match crate::soltx::pubkey(w.trim()) {
            Ok(k) if !w.trim().starts_with("0x") => Ok(k[..20].try_into().expect("20")),
            _ => Err(e),
        })).transpose()
    }
}

/// Every rule `eth_nft.dormancy` must satisfy, with `collections` the number the configuration names.
pub fn validate(d: &Dormancy, collections: usize) -> Result<(), String> {
    if d.max_active == 0 || d.max_active > MAX_ACTIVE_CEILING || d.max_active as usize > collections {
        return Err(format!("eth_nft.dormancy.max_active {} is outside [1, min({MAX_ACTIVE_CEILING}, the {collections} collections)]", d.max_active));
    }
    if d.max_waking == 0 || d.max_waking > d.max_active {
        return Err(format!("eth_nft.dormancy.max_waking {} is outside [1, max_active {}]", d.max_waking, d.max_active));
    }
    if d.wakes_per_day == 0 || d.wakes_per_day > MAX_WAKES_PER_DAY {
        return Err(format!("eth_nft.dormancy.wakes_per_day {} is outside [1, {MAX_WAKES_PER_DAY}]", d.wakes_per_day));
    }
    if !(WAKE_TIMEOUT_MIN_SECS..=WAKE_TIMEOUT_MAX_SECS).contains(&d.wake_timeout_secs) {
        return Err(format!("eth_nft.dormancy.wake_timeout_secs {} is outside [{WAKE_TIMEOUT_MIN_SECS}, {WAKE_TIMEOUT_MAX_SECS}]", d.wake_timeout_secs));
    }
    if !(COOLDOWN_MIN_SECS..=COOLDOWN_MAX_SECS).contains(&d.cooldown_secs) {
        return Err(format!("eth_nft.dormancy.cooldown_secs {} is outside [{COOLDOWN_MIN_SECS}, {COOLDOWN_MAX_SECS}]", d.cooldown_secs));
    }
    if d.idle_days == 0 || d.idle_days > IDLE_DAYS_MAX {
        return Err(format!("eth_nft.dormancy.idle_days {} is outside [1, {IDLE_DAYS_MAX}]", d.idle_days));
    }
    if let Some(s) = &d.scaling {
        validate_scaling(s)?;
    }
    match d.desk() {
        Ok(Some(a)) if a == [0; 20] => return Err("eth_nft.dormancy.wake_desk is the zero address".into()),
        Ok(_) => {}
        Err(e) => return Err(format!("eth_nft.dormancy.wake_desk: {e}")),
    }
    Ok(())
}

/// Every rule `eth_nft.dormancy.scaling` must satisfy (§11.2).
pub fn validate_scaling(s: &Scaling) -> Result<(), String> {
    let floor_b = (crate::ethflow::EVIDENCE_CYCLES_FLOOR / B) as u64;
    let within = |v: u64, lo: u64, hi: u64, what: &str| -> Result<(), String> {
        if (lo..=hi).contains(&v) { Ok(()) } else { Err(format!("eth_nft.dormancy.scaling.{what} {v} is outside [{lo}, {hi}]")) }
    };
    within(s.active_b_per_day, SCALE_ACTIVE_B_MIN, SCALE_ACTIVE_B_MAX, "active_b_per_day")?;
    within(s.idle_b_per_day, 0, SCALE_IDLE_B_MAX, "idle_b_per_day")?;
    within(s.wake_b, SCALE_WAKE_B_MIN, SCALE_WAKE_B_MAX, "wake_b")?;
    within(s.runway_days as u64, SCALE_RUNWAY_DAYS_MIN as u64, SCALE_RUNWAY_DAYS_MAX as u64, "runway_days")?;
    within(s.reserve_b, floor_b, SCALE_RESERVE_B_MAX, "reserve_b (at least the canister's scan floor)")?;
    within(s.exit_b_per_awake, 0, SCALE_EXIT_B_MAX, "exit_b_per_awake")?;
    Ok(())
}

// ---- the life cycle (§2) --------------------------------------------------------------------------------------------------------

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    Dormant,
    /// The backfill runs; `backfilled_at` once the window is read and a confirmed reading made the first post due.
    Waking { since: u64, backfilled_at: Option<u64> },
    Active { since: u64 },
}

/// What asked for a wake.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum WakeBy {
    /// A bonded request on the `WakeDesk` (its id and requester).
    Desk { id: u64, requester: [u8; 20] },
    /// The operator's `eth_wake`.
    Operator,
    /// A valid deposit of the collection (an NFT really sent to us): owed.
    Deposit { tx: [u8; 32] },
    /// A graduation queued at the module (`gradEth() > 0`): owed.
    Graduation,
}

impl WakeBy {
    /// Owed: something of value already waits on this collection (an NFT at our address, a whole raise at the module). Owed wakes go
    /// first and skip the collection's cooldown; every cap still applies.
    pub fn owed(&self) -> bool {
        matches!(self, WakeBy::Deposit { .. } | WakeBy::Graduation)
    }
    pub fn word(&self) -> String {
        match self {
            WakeBy::Desk { id, requester } => format!("the wake desk's request #{id} by 0x{}", hex::encode(requester)),
            WakeBy::Operator => "the operator".into(),
            WakeBy::Deposit { tx } => format!("a deposit (0x{})", hex::encode(tx)),
            WakeBy::Graduation => "a graduation queued at its module".into(),
        }
    }
}

/// Independent review DI-M-1 / DI-M-2: the collection's open `WakeDesk` request as the canister holds it (the desk keeps at most one open
/// a collection). Queued while the collection is DORMANT and the request can still be served inside its window; kept while it is WAKING
/// or ACTIVE (its first or next post serves it, and it is queued again if the collection sleeps unserved before a wake was tried for it).
/// The post that serves it is recorded on the desk (`markServed`), so a later post never decides it.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct DeskAsk {
    pub id: u64,
    pub requester: [u8; 20],
    /// When it was asked (the desk's `at`) and the end of its window (`at + window()`).
    pub at: u64,
    pub until: u64,
    /// A wake was started, or was already under way, since it was asked: an unserved ask whose wake timed out is not queued again.
    pub tried: bool,
    /// When one of our posts mined at or after `at`, inside the window: it is recorded on the desk (`markServed`).
    pub served: Option<u64>,
    /// `markServed` mined (or the desk already holds it, or it cannot be taken any more): nothing more to do.
    pub marked: bool,
}

impl DeskAsk {
    /// Whether a wake started now could still post inside the window, with the margin for `markServed` (DI-M-1).
    pub fn startable(&self, p: &Dormancy, now: u64) -> bool {
        now.saturating_add(p.wake_timeout_secs).saturating_add(DESK_START_MARGIN_SECS) <= self.until
    }
}

/// One collection's life, kept in `EthState.dorm`.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Life {
    pub collection: [u8; 20],
    pub phase: Phase,
    /// When the phase last changed.
    pub since: u64,
    /// What woke it (the current or last wake).
    pub by: Option<WakeBy>,
    /// A wake asked and waiting for room: `(what asked, when)`.
    pub queued: Option<(WakeBy, u64)>,
    /// No wake that is not owed before this.
    pub cooldown_until: u64,
    /// Posted once at least (a launch on its module is then possible while DORMANT: the watchdog reads it).
    pub ever_active: bool,
    /// The module's factory (`factory()`), read once.
    pub factory: Option<[u8; 20]>,
    /// When a position was last seen while ACTIVE (a unit, an item, a graduation, an advance or a float on 4663, or anything open in the
    /// canister), `None`: none since the wake. The idle clock (§5).
    pub position_at: Option<u64>,
    /// The next sleep check (ACTIVE) or watchdog read (DORMANT).
    pub check_at: u64,
    /// The cadence clocks: the last evidence pass, the last sale scan and its back-off, the last module view, hot until.
    pub pass_at: u64,
    pub scan_at: u64,
    pub scan_every: u64,
    pub view_at: u64,
    pub hot_until: u64,
    /// The last reading's scan: `(the reading's time, its pass ran the scan, the scan's lag in final blocks)` (§4.3).
    pub read_scan: Option<(u64, bool, u64)>,
    /// The floor the open buy's advance was sized on: `(buy, floor)` (§4.3).
    pub buy_floor: Option<(u64, u128)>,
    /// ACTIVE: a queued graduation to fill (`Step::Fill`): when the hunt may try next; set when `gradEth()` reads non-zero.
    pub fill_hunt: Option<u64>,
    /// ACTIVE: when `gradEth()` was last read for the hunt (on the view's cadence).
    pub grad_at: u64,
    /// WAKING: the estimate, seconds from `eta_at`.
    pub eta: Option<(u64, u64)>,
    /// The module's last post as the canister knows it (a post of ours mined, or `postedAt()` read at a pass): the TTL margin (DR-M-3).
    pub posted_at: u64,
    /// Owed wakes that timed out in a row, and no owed wake before `owed_after` (DR-H-2). A wake that becomes ACTIVE clears both.
    pub owed_fails: u8,
    pub owed_after: u64,
    /// §11: whether it held a position at its last check (on 4663, or open in the canister): a pinned collection stays awake whatever the
    /// cycles limit says, and counts against it.
    pub pinned: bool,
    pub note: String,
    /// DI-M-1 / DI-M-2: its open desk request, as held (`None`: none).
    #[serde(default)]
    pub desk: Option<DeskAsk>,
}

impl Life {
    pub fn new(collection: [u8; 20], phase: Phase, now: u64) -> Self {
        let ever_active = matches!(phase, Phase::Active { .. });
        Life { collection, phase, since: now, by: None, queued: None, cooldown_until: 0, ever_active, factory: None,
            position_at: None, check_at: now, pass_at: 0, scan_at: 0, scan_every: SCAN_EVERY_SECS, view_at: 0,
            hot_until: 0, read_scan: None, buy_floor: None, fill_hunt: None, grad_at: 0, eta: None, posted_at: 0, owed_fails: 0, owed_after: 0,
            pinned: false, note: String::new(), desk: None }
    }
    pub fn is_dormant(&self) -> bool {
        self.phase == Phase::Dormant
    }
    pub fn is_waking(&self) -> bool {
        matches!(self.phase, Phase::Waking { .. })
    }
    pub fn is_active(&self) -> bool {
        matches!(self.phase, Phase::Active { .. })
    }
}

/// Everything the dormancy keeps (`EthState.dorm`).
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct Dorm {
    pub lives: Vec<Life>,
    /// The next `WakeDesk` request id to read, and the desk it counts on (a new desk, by an upgrade, is read from its start).
    pub desk_cursor: u64,
    pub desk: Option<[u8; 20]>,
    /// When each wake of the last 24 h started (`wakes_per_day`).
    pub wakes: Vec<u64>,
    /// The operator's fill hand-ins (`eth_fill`): `(collection, coin, handed in at)`, tried before the newest coins (DR-M-2).
    pub fills: Vec<([u8; 20], [u8; 20], u64)>,
    /// §11: the capacity as of the last tick (the monitor reads it here).
    pub cap: Option<Capacity>,
    /// DI-M-1: the desk's `window()`, read once (a request's window ends at its `at` plus this).
    #[serde(default)]
    pub desk_window: Option<u64>,
}

/// A collection as the flow treats it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// No dormancy configured, or its life not decided yet (read as ACTIVE: never less than before this change).
    Always,
    Dormant,
    Queued,
    Waking,
    Active,
}

impl Kind {
    /// Its floor is watched and posted, and buys and deposit mints may run.
    pub fn live(self) -> bool {
        matches!(self, Kind::Always | Kind::Active)
    }
    /// Nothing of it is read (no pass, no scan, no module view).
    pub fn asleep(self) -> bool {
        matches!(self, Kind::Dormant | Kind::Queued)
    }
    pub fn word(self) -> &'static str {
        match self {
            Kind::Always => "always-on",
            Kind::Dormant => "dormant",
            Kind::Queued => "queued",
            Kind::Waking => "waking",
            Kind::Active => "active",
        }
    }
}

impl Dorm {
    pub fn life(&self, c: &[u8; 20]) -> Option<&Life> {
        self.lives.iter().find(|l| l.collection == *c)
    }
    pub fn life_mut(&mut self, c: &[u8; 20]) -> Option<&mut Life> {
        self.lives.iter_mut().find(|l| l.collection == *c)
    }
    pub fn kind(&self, c: &[u8; 20]) -> Kind {
        match self.life(c) {
            None => Kind::Always,
            Some(l) => match l.phase {
                Phase::Dormant if l.queued.is_some() => Kind::Queued,
                Phase::Dormant => Kind::Dormant,
                Phase::Waking { .. } => Kind::Waking,
                Phase::Active { .. } => Kind::Active,
            },
        }
    }
    /// `(waking, active)`.
    pub fn counts(&self) -> (usize, usize) {
        (self.lives.iter().filter(|l| l.is_waking()).count(), self.lives.iter().filter(|l| l.is_active()).count())
    }
    /// The collections no longer configured are forgotten (their work is refused by `stranded_by` at the upgrade).
    pub fn retain(&mut self, cols: &[[u8; 20]]) {
        self.lives.retain(|l| cols.contains(&l.collection));
    }
    /// A wake is asked for `c`. A DORMANT collection's request is queued (an owed one takes the place of one that is not, keeping the
    /// earlier time); a WAKING or ACTIVE one needs none.
    pub fn request(&mut self, c: &[u8; 20], by: WakeBy, now: u64) -> Result<String, String> {
        let l = self.life_mut(c).ok_or_else(|| format!("0x{} has no life yet (the next tick decides it)", hex::encode(c)))?;
        match l.phase {
            Phase::Waking { .. } => Ok(format!("0x{} is already warming up", hex::encode(c))),
            Phase::Active { .. } => Ok(format!("0x{} is already active", hex::encode(c))),
            Phase::Dormant => {
                let word = by.word();
                l.queued = Some(match l.queued.take() {
                    Some((old, at)) if old.owed() || !by.owed() => (old, at),
                    Some((_, at)) => (by, at),
                    None => (by, now),
                });
                Ok(format!("0x{}: a wake is queued ({word})", hex::encode(c)))
            }
        }
    }
    /// DI-M-1 / DI-M-2: a desk request read (not closed, its window not over). DORMANT: its wake is queued if it can still post inside the
    /// window (it takes the place of a queued desk wake of an earlier request, at its own read time (D2-I-2); an owed or the operator's
    /// wake stays); WAKING: the wake
    /// under way serves it; ACTIVE: its next post serves it. It is kept on the life in every case (`Life.desk`).
    pub fn desk_ask(&mut self, c: &[u8; 20], mut ask: DeskAsk, p: &Dormancy, now: u64) -> Result<String, String> {
        let l = self.life_mut(c).ok_or_else(|| format!("0x{} has no life yet (the next tick decides it)", hex::encode(c)))?;
        let (id, x) = (ask.id, hex::encode(c));
        let s = match l.phase {
            Phase::Dormant if !ask.startable(p, now) => format!("0x{x}: the wake desk's request #{id} is too late in its window for a wake that \
                posts inside it (it ends at {}): not queued; its bond goes back unless the collection is posted inside the window by another \
                wake", ask.until),
            Phase::Dormant => {
                let by = WakeBy::Desk { id, requester: ask.requester };
                l.queued = Some(match l.queued.take() {
                    Some((old, at)) if !matches!(old, WakeBy::Desk { .. }) => (old, at),
                    // independent review 2 (D2-I-2): a queued desk wake of an earlier request (one closed on the desk, or past its
                    // window, not read again yet) gives up its place: the new request queues at its own read time, never at the time
                    // the old one was queued (the desk holds one open request a collection, so a new id is always a new request)
                    Some((WakeBy::Desk { id: old, .. }, at)) if old == id => (by.clone(), at),
                    Some(_) | None => (by.clone(), now),
                });
                format!("0x{x}: a wake is queued ({})", by.word())
            }
            Phase::Waking { .. } => {
                ask.tried = true;
                format!("0x{x} is already warming up: the wake desk's request #{id} is served by its first post")
            }
            Phase::Active { .. } => format!("0x{x} is already active: the wake desk's request #{id} is served by its next post (queued for a \
                wake if it sleeps first)"),
        };
        l.desk = Some(ask);
        Ok(s)
    }
    /// DI-M-1 / DI-M-2, each tick before the wakes start: a desk request past its window (or recorded on the desk) is forgotten; a queued
    /// desk wake that can no longer post inside its request's window is dropped from the queue (never started late: its bond goes back
    /// unless another wake posts the collection inside the window, D2-L-3); a DORMANT collection's unserved request that no wake was
    /// tried for (it was ACTIVE and slept first) is queued again.
    pub fn desk_upkeep(&mut self, p: &Dormancy, now: u64) -> Vec<String> {
        let mut notes = Vec::new();
        for l in self.lives.iter_mut() {
            let x = hex::encode(l.collection);
            if let Some(a) = l.desk.clone() {
                if a.marked || now >= a.until {
                    if a.served.is_some() && !a.marked {
                        notes.push(format!("0x{x}: the wake desk's request #{} was served at {} but not recorded before its window ended \
                            (the desk decides from the module's latest post)", a.id, a.served.unwrap_or(0)));
                    }
                    l.desk = None;
                }
            }
            if !l.is_dormant() {
                continue;
            }
            if let Some((WakeBy::Desk { id, .. }, _)) = &l.queued {
                let id = *id;
                if !l.desk.as_ref().is_some_and(|a| a.id == id && a.served.is_none() && a.startable(p, now)) {
                    l.queued = None;
                    notes.push(format!("0x{x}: the wake desk's request #{id} can no longer start a wake that posts inside its window: dropped \
                        from the queue (its bond goes back unless the collection is posted inside the window by another wake)"));
                    l.note = format!("dormant: the wake desk's request #{id} cannot start a wake inside its window any more (refunded unless \
                        another wake posts the collection inside it)");
                }
            }
            if l.queued.is_none() {
                if let Some(a) = l.desk.as_ref().filter(|a| a.served.is_none() && !a.tried && a.startable(p, now)) {
                    l.queued = Some((WakeBy::Desk { id: a.id, requester: a.requester }, a.at));
                    notes.push(format!("0x{x}: the wake desk's request #{} was not served before the collection slept: its wake is queued", a.id));
                }
            }
        }
        notes
    }
    /// The queued wakes that fit the caps now start: owed first, then the oldest. Returns the collections that started WAKING.
    /// `balance`: the canister's liquid cycles (§11); with `scaling` set, `None` starts no wake (fail closed).
    pub fn start_wakes(&mut self, p: &Dormancy, now: u64, balance: Option<u128>) -> (Vec<[u8; 20]>, Vec<String>) {
        self.wakes.retain(|t| now < t.saturating_add(86_400));
        let mut order: Vec<(bool, u64, [u8; 20])> = self.lives.iter().filter(|l| l.is_dormant())
            .filter_map(|l| l.queued.as_ref().map(|(by, at)| (!by.owed(), *at, l.collection))).collect();
        order.sort();
        let (mut started, mut notes) = (Vec::new(), Vec::new());
        for (not_owed, _, c) in order {
            let (waking, active) = self.counts();
            // DR-H-1: an owed wake has `OWED_RESERVE` slots above max_active
            let room = p.max_active as usize + if not_owed { 0 } else { OWED_RESERVE };
            if waking + active >= room {
                notes.push(format!("0x{}: queued: {} of max_active {}{} are waking or active", hex::encode(c), waking + active, p.max_active,
                    if not_owed { "" } else { " (+ the owed reserve)" }));
                continue;
            }
            // §11: under the ceiling, the cycles balance decides. A wake that is not owed needs the runway of every awake collection and
            // of this one after its cost; an owed one needs only that the reserve stays whole. Collections with positions stay awake above
            // the limit and count against it, so while it is exceeded no wake that is not owed starts
            if let Some(s) = &p.scaling {
                let Some(b) = balance else {
                    notes.push(format!("0x{}: queued: the canister's cycles balance was not read (no wake starts without it)", hex::encode(c)));
                    continue;
                };
                let a = available(s, b, waking);
                let eff = effective_max_active(s, a, p.max_active);
                if not_owed && !wake_fits(s, a, waking + active) {
                    notes.push(format!("0x{}: queued: the cycles balance ({} B, {} B once the backfills under way are paid) keeps {eff} awake for \
                        {} days with the reserve, {} are: a wake needs {} B available (a top-up raises the limit)", hex::encode(c), b / B, a / B,
                        s.runway_days, waking + active, s.wake().saturating_add(s.need((waking + active) as u128 + 1)) / B));
                    continue;
                }
                // Scaling review S-1: an owed wake has one slot (`OWED_RESERVE`) above what the balance keeps, not above the ceiling: a
                // deposit of each collection could otherwise wake all ten on a balance that pays for one
                if !not_owed && waking + active >= eff as usize + OWED_RESERVE {
                    notes.push(format!("0x{}: queued: an owed wake, but {} are awake and the cycles balance keeps {eff} (+ the owed reserve): \
                        it starts after a top-up or when one sleeps", hex::encode(c), waking + active));
                    continue;
                }
                if !not_owed && !owed_wake_fits(s, a, waking + active) {
                    notes.push(format!("0x{}: queued: an owed wake, but the cycles balance ({} B available) would eat the reserve: it needs {} B \
                        (top it up)", hex::encode(c), a / B, s.wake().saturating_add(s.reserve_for((waking + active) as u128 + 1)) / B));
                    continue;
                }
            }
            if waking >= p.max_waking as usize {
                notes.push(format!("0x{}: queued: {waking} of max_waking {} are warming up", hex::encode(c), p.max_waking));
                continue;
            }
            if self.wakes.len() >= p.wakes_per_day as usize {
                notes.push(format!("0x{}: queued: {} wakes in the last 24 h (wakes_per_day {})", hex::encode(c), self.wakes.len(), p.wakes_per_day));
                continue;
            }
            let l = self.life_mut(&c).expect("listed");
            // DI-M-1: a desk wake starts only while it can still post inside its request's window
            if let Some((WakeBy::Desk { id, .. }, _)) = &l.queued {
                let id = *id;
                if !l.desk.as_ref().is_some_and(|a| a.id == id && a.startable(p, now)) {
                    l.queued = None;
                    notes.push(format!("0x{}: the wake desk's request #{id} can no longer post inside its window: dropped from the queue",
                        hex::encode(c)));
                    continue;
                }
            }
            if not_owed && now < l.cooldown_until {
                notes.push(format!("0x{}: queued: its cooldown runs until {}", hex::encode(c), l.cooldown_until));
                continue;
            }
            if !not_owed && now < l.owed_after {
                notes.push(format!("0x{}: queued: {} owed wake(s) timed out: the next may start at {}", hex::encode(c), l.owed_fails, l.owed_after));
                continue;
            }
            let (by, _) = l.queued.take().expect("queued");
            // DI-M-1: the collection's desk request has had its wake (one try: a timed-out wake is not asked again for it)
            if let Some(a) = l.desk.as_mut() {
                a.tried = true;
            }
            l.phase = Phase::Waking { since: now, backfilled_at: None };
            l.since = now;
            l.note = format!("warming up: woken by {}", by.word());
            l.by = Some(by);
            l.eta = None;
            l.pass_at = 0;
            l.scan_at = 0;
            l.scan_every = SCAN_EVERY_SECS;
            l.read_scan = None;
            l.fill_hunt = None;
            self.wakes.push(now);
            started.push(c);
        }
        (started, notes)
    }
    /// WAKING longer than the timeout goes back to DORMANT, with the cooldown. Returns those collections.
    pub fn timeouts(&mut self, p: &Dormancy, now: u64) -> Vec<[u8; 20]> {
        let mut out = Vec::new();
        for l in self.lives.iter_mut() {
            if let Phase::Waking { since, .. } = l.phase {
                if now >= since.saturating_add(p.wake_timeout_secs) {
                    l.phase = Phase::Dormant;
                    l.since = now;
                    l.cooldown_until = now.saturating_add(p.cooldown_secs);
                    l.eta = None;
                    let mut owed = String::new();
                    if l.by.as_ref().is_some_and(|b| b.owed()) {
                        // DR-H-2: an owed wake that timed out backs off, doubling
                        l.owed_fails = l.owed_fails.saturating_add(1);
                        let wait = p.cooldown_secs.saturating_mul(1u64 << (l.owed_fails - 1).min(16)).min(OWED_BACKOFF_MAX_SECS);
                        l.owed_after = now.saturating_add(wait);
                        owed = format!("; owed wake {} of {OWED_FAILS_MAX} timed out: the next owed wake from {}", l.owed_fails, l.owed_after);
                    }
                    l.note = format!("the wake timed out after {} s (the backfill or a confirmed reading did not come): dormant, cooldown until {}{owed}",
                        p.wake_timeout_secs, l.cooldown_until);
                    out.push(l.collection);
                }
            }
        }
        out
    }
    /// The first post mined: ACTIVE.
    pub fn activate(&mut self, c: &[u8; 20], now: u64) -> bool {
        let Some(l) = self.life_mut(c) else { return false };
        if !l.is_waking() {
            return false;
        }
        l.phase = Phase::Active { since: now };
        l.since = now;
        l.ever_active = true;
        l.owed_fails = 0;
        l.owed_after = 0;
        l.position_at = None;
        l.check_at = now.saturating_add(SLEEP_CHECK_SECS);
        l.eta = None;
        l.pass_at = now;
        l.view_at = 0;
        l.scan_every = SCAN_EVERY_SECS;
        l.note = "active: the first confirmed post mined".into();
        true
    }
    /// Back to DORMANT (idle): the cooldown starts.
    pub fn sleep(&mut self, c: &[u8; 20], p: &Dormancy, now: u64, why: &str) {
        if let Some(l) = self.life_mut(c) {
            l.phase = Phase::Dormant;
            l.pinned = false;
            l.since = now;
            l.cooldown_until = now.saturating_add(p.cooldown_secs);
            l.check_at = now.saturating_add(SLEEP_CHECK_SECS);
            l.read_scan = None;
            l.buy_floor = None;
            l.fill_hunt = None;
            l.hot_until = 0;
            l.note = format!("dormant: {why}");
        }
    }
    /// The queue's order, 1-based, of a queued collection.
    pub fn position(&self, c: &[u8; 20]) -> Option<u32> {
        let mut order: Vec<(bool, u64, [u8; 20])> = self.lives.iter().filter(|l| l.is_dormant())
            .filter_map(|l| l.queued.as_ref().map(|(by, at)| (!by.owed(), *at, l.collection))).collect();
        order.sort();
        order.iter().position(|x| x.2 == *c).map(|i| i as u32 + 1)
    }
}

// ---- the cadence -----------------------------------------------------------------------------------------------------------------

/// Whether an evidence hand-in is a pass now: hot, the cadence, or (DR-M-3) the module's last post within `TTL_MARGIN_SECS` of its
/// `FLOOR_TTL` (a post is then due at that pass).
pub fn pass_due(l: &Life, now: u64, hot: bool) -> bool {
    match l.phase {
        Phase::Dormant => false,
        Phase::Waking { .. } => true,
        Phase::Active { .. } => hot || now >= l.pass_at.saturating_add(PASS_EVERY_SECS)
            || now.saturating_add(TTL_MARGIN_SECS) >= l.posted_at.saturating_add(FLOOR_TTL_SECS),
    }
}

/// DR-M-1: the module's accepted floor has caught up with our reading (after a long dormancy its first posts are clamped by the
/// module's bounds): only then are graduations filled and buys started.
pub fn converged(reading: u128, accepted: u128) -> bool {
    accepted != 0 && within_bps(reading, accepted, BUY_MISMATCH_BPS)
}

/// Whether that pass runs the sale scan.
pub fn scan_due(l: &Life, now: u64, hot: bool) -> bool {
    match l.phase {
        Phase::Dormant => false,
        Phase::Waking { .. } => true,
        Phase::Active { .. } => hot || now >= l.scan_at.saturating_add(l.scan_every.max(SCAN_EVERY_SECS)),
    }
}

/// A scan ran: the back-off doubles after one that counted no new sale, and is reset by a new sale.
pub fn note_scan(l: &mut Life, now: u64, new_sales: bool) {
    l.scan_at = now;
    l.scan_every = if new_sales { SCAN_EVERY_SECS } else { l.scan_every.max(SCAN_EVERY_SECS).saturating_mul(2).min(SCAN_BACKOFF_MAX_SECS) };
}

/// Whether the module view that decides a new buy is read now.
pub fn view_due(l: &Life, now: u64, hot: bool) -> bool {
    l.is_active() && (hot || now >= l.view_at.saturating_add(VIEW_EVERY_SECS))
}

/// Whether `a` is within `bps` of `b` (relative to `b`).
pub fn within_bps(a: u128, b: u128, bps: u128) -> bool {
    a.abs_diff(b).saturating_mul(10_000) <= b.saturating_mul(bps)
}

// ---- back to DORMANT (§5) --------------------------------------------------------------------------------------------------------

/// The idle clock (§5, dormant-review DR-H-1): `idle_days` since a position was last seen, or `NO_POSITION_SECS` since ACTIVE when none
/// was seen since the wake. A bare launch is no position: anyone can launch for gas alone, and a coin still trading needs no fresh floor
/// (its graduation, when it comes, is an owed wake).
pub fn idle_long_enough(l: &Life, p: &Dormancy, now: u64) -> Result<(), String> {
    let Phase::Active { since } = l.phase else { return Err("not active".into()) };
    match l.position_at {
        None if now >= since.saturating_add(NO_POSITION_SECS) => Ok(()),
        None => Err(format!("no position since the wake: it may sleep at {} (six hours after it became active) if none comes", since + NO_POSITION_SECS)),
        Some(t) if now >= t.saturating_add(p.idle_days as u64 * 86_400) => Ok(()),
        Some(t) => Err(format!("its last position was seen at {t}: it may sleep at {}", t + p.idle_days as u64 * 86_400)),
    }
}

/// What 4663 says about a collection that would go DORMANT: every one must be 0 (§5).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OnChain {
    pub total_supply: u128,
    pub held: u128,
    pub rented_held: u128,
    pub grad_eth: u128,
    pub grad_units: u128,
    pub transit_units: u128,
    pub float_units: u128,
}

impl OnChain {
    pub fn open(&self) -> Vec<String> {
        let mut v = Vec::new();
        for (n, x) in [("wX supply", self.total_supply), ("items held", self.held), ("rented items", self.rented_held),
            ("queued graduation ETH", self.grad_eth), ("unreleased graduation units", self.grad_units), ("an advance in transit", self.transit_units),
            ("float units", self.float_units)] {
            if x != 0 {
                v.push(format!("{n} {x}"));
            }
        }
        v
    }
}

// ---- the backfill and the estimate (§2.3, §4.2) ----------------------------------------------------------------------------------

/// What is still missing from a complete 7-day backfill at the final head `top` (empty: complete). `backed_off`: the moves whose
/// receipts are backed off (their fast tries spent); at most `BACKFILL_BACKED_OFF_MAX` of them may remain.
pub fn backfill_gaps(sc: Option<&SaleScan>, top: u64) -> Vec<String> {
    let Some(sc) = sc else { return vec!["the scan has not started".into()] };
    let bottom = top.saturating_sub(crate::ethnft::SALE_WINDOW_BLOCKS);
    let mut v = Vec::new();
    if sc.up == 0 {
        v.push("nothing read yet".into());
        return v;
    }
    if sc.down > bottom {
        v.push(format!("blocks {bottom}..{} not read yet", sc.down));
    }
    if let Some((a, b)) = sc.hole {
        v.push(format!("a re-anchor's hole {a}..{b}"));
    }
    if let Some((a, b)) = sc.lost {
        v.push(format!("blocks {a}..{b} lost (too deep for the providers)"));
    }
    if sc.raw.as_ref().is_some_and(|r| !r.is_empty()) {
        v.push("deferred ranges whose marketplace logs were not read whole".into());
    }
    if sc.skipped.as_ref().is_some_and(|r| !r.is_empty()) {
        v.push("oversized blocks not read yet".into());
    }
    if sc.rescan.is_some() || !sc.todo.is_empty() {
        v.push("blocks to read again".into());
    }
    let backed: std::collections::BTreeSet<[u8; 32]> = sc.rfail.iter().flatten().filter(|x| x.1 >= crate::ethnft::SALE_RECEIPT_FAST_TRIES)
        .map(|x| x.0).chain(sc.rbusy.iter().flatten().map(|x| x.0)).collect();
    let pend: Vec<&crate::ethnft::Move> = sc.pend.iter().flatten().filter(|m| m.block <= top).collect();
    let waiting = pend.iter().filter(|m| !backed.contains(&m.hash)).count();
    let stuck = pend.len() - waiting;
    if waiting > 0 {
        v.push(format!("{waiting} receipt(s) to judge"));
    }
    if stuck > BACKFILL_BACKED_OFF_MAX {
        v.push(format!("{stuck} receipt(s) backed off (over {BACKFILL_BACKED_OFF_MAX})"));
    }
    v
}

/// "warming up (~N min)": the passes still needed, at `WAKE_PASS_SECS` each, plus the post's own tick. `reads_per_pass` log reads of
/// `chunk` blocks a pass; `receipts_per_pass` receipts a pass; `moves_seen` the moves judged or pending in what was read so far.
pub fn wake_eta_secs(sc: Option<&SaleScan>, top: u64, reads_per_pass: u64, receipts_per_pass: u64, moves_seen: u64) -> u64 {
    let window = crate::ethnft::SALE_WINDOW_BLOCKS;
    let bottom = top.saturating_sub(window);
    let (left, read, pend) = match sc {
        Some(s) if s.up != 0 => {
            let hole = s.hole.map_or(0, |(a, b)| b.saturating_sub(a) + 1);
            (s.down.saturating_sub(bottom).saturating_add(hole), s.up.saturating_sub(s.down).saturating_add(1).saturating_sub(hole),
                s.pend.as_ref().map_or(0, |p| p.len() as u64) + s.todo.len() as u64)
        }
        _ => (window, 0, 0),
    };
    let chunk = sc.map_or(crate::ethnft::SALE_SCAN_CHUNK_BLOCKS, |s| s.chunk.max(crate::ethnft::SALE_SCAN_MIN_CHUNK_BLOCKS));
    let per_pass = reads_per_pass.max(1).saturating_mul(chunk).max(1);
    let log_passes = left.div_ceil(per_pass);
    let moves_left = if read == 0 {
        WAKE_DEFAULT_MOVES.saturating_mul(left) / window.max(1)
    } else {
        pend.saturating_add(moves_seen.saturating_mul(left) / read.max(1))
    };
    let receipt_passes = moves_left.div_ceil(receipts_per_pass.max(1));
    (log_passes.max(receipt_passes).max(1) + 1).saturating_mul(WAKE_PASS_SECS)
}

// ---- the API (§4.2) --------------------------------------------------------------------------------------------------------------

/// One collection as `eth_nft_collections()` shows it.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct CollectionLife {
    pub collection: String,
    pub wrapper: String,
    pub module: String,
    /// `always-on`, `dormant`, `queued`, `waking` or `active`.
    pub state: String,
    pub since: u64,
    /// While waking: the estimate, seconds from now.
    pub eta_secs: Option<u64>,
    /// While queued: the place in the queue (1 is next).
    pub queue_position: Option<u32>,
    pub woken_by: Option<String>,
    pub cooldown_until: Option<u64>,
    /// The human text: "dormant: a launch wakes it", "queued (#2)", "warming up (~40 min)", "active".
    pub status: String,
    pub note: String,
}

/// The status line the site shows.
pub fn status_text(k: Kind, eta_secs: Option<u64>, position: Option<u32>, cooldown_until: u64, now: u64) -> String {
    match k {
        Kind::Always => "active".into(),
        Kind::Active => "active".into(),
        Kind::Waking => match eta_secs {
            Some(s) => format!("warming up (~{} min)", s.div_ceil(60).max(1)),
            None => "warming up".into(),
        },
        Kind::Queued => match position {
            Some(p) => format!("queued (#{p}): it warms up when there is room"),
            None => "queued".into(),
        },
        Kind::Dormant if cooldown_until > now => format!("dormant: it can be woken again in ~{} min", (cooldown_until - now).div_ceil(60)),
        Kind::Dormant => "dormant: a launch wakes it".into(),
    }
}

/// The view of one life (or of a collection with no dormancy), for the query.
pub fn view(k: Kind, l: Option<&Life>, d: Option<&Dorm>, now: u64, names: (String, String, String)) -> CollectionLife {
    let eta = l.and_then(|l| l.eta).map(|(secs, at)| secs.saturating_sub(now.saturating_sub(at)).max(60));
    let eta = if k == Kind::Waking { eta } else { None };
    let pos = if k == Kind::Queued { l.and_then(|l| d.and_then(|d| d.position(&l.collection))) } else { None };
    let cooldown = l.map_or(0, |l| l.cooldown_until);
    CollectionLife {
        collection: names.0,
        wrapper: names.1,
        module: names.2,
        state: k.word().into(),
        since: l.map_or(0, |l| l.since),
        eta_secs: eta,
        queue_position: pos,
        woken_by: l.and_then(|l| l.by.as_ref().map(|b| b.word())),
        cooldown_until: (cooldown > now).then_some(cooldown),
        status: status_text(k, eta, pos, cooldown, now),
        note: l.map_or(String::new(), |l| l.note.clone()),
    }
}

// ---- the WakeDesk's ABI (src/floor/WakeDesk.sol) --------------------------------------------------------------------------------

/// `requestCount()`, `requestAt(uint256)` → `(address wrapper, address requester, uint64 at, uint128 bond, bool closed)`.
pub const REQUEST_COUNT: [u8; 4] = [0x5b, 0xad, 0xbe, 0x4c];
pub const REQUEST_AT: [u8; 4] = [0x9d, 0x0a, 0x91, 0x25];
/// DI-M-1: `window()` (read once), `markServed(uint256)` (the post that served a request, recorded inside its window) and
/// `servedAt(uint256)` (what the desk recorded: 0 none).
pub const WINDOW: [u8; 4] = [0x46, 0x16, 0x45, 0xbf];
pub const MARK_SERVED: [u8; 4] = [0x5a, 0x3a, 0x75, 0x54];
pub const SERVED_AT: [u8; 4] = [0x63, 0x57, 0xa0, 0xb1];
/// The reads of a collection that would go DORMANT and of the watchdog (EthNftWrapper, FloorModule, CurveFactory, FrogCurve).
pub const TOTAL_SUPPLY: [u8; 4] = [0x18, 0x16, 0x0d, 0xdd];
pub const HELD: [u8; 4] = [0xa2, 0x85, 0xae, 0xd7];
pub const RENTED_HELD: [u8; 4] = [0xba, 0x64, 0x36, 0xbd];
pub const GRAD_ETH: [u8; 4] = [0x81, 0xd3, 0x76, 0x80];
pub const GRAD_UNITS: [u8; 4] = [0x7d, 0x35, 0xc4, 0x29];
pub const FLOAT_UNITS: [u8; 4] = [0x3a, 0x60, 0xec, 0x4c];
pub const FACTORY: [u8; 4] = [0xc4, 0x5a, 0x01, 0x55];
pub const COIN_COUNT: [u8; 4] = [0x67, 0x8f, 0xd2, 0x89];
/// `coins(uint256)`, `routerOf(address)`, `state()`, `isFilled(address)`, `fill(address)` (the fill hunt).
pub const COINS: [u8; 4] = [0xc6, 0x61, 0x06, 0x57];
pub const ROUTER_OF: [u8; 4] = [0x25, 0x35, 0x6d, 0x2b];
pub const STATE: [u8; 4] = [0xc1, 0x9d, 0x93, 0xfb];
pub const IS_FILLED: [u8; 4] = [0x8c, 0xc8, 0xa4, 0x36];
pub const FILL: [u8; 4] = [0xaa, 0x29, 0xb9, 0x5c];
/// `FrogCurve.State`: Trading, Graduating, Graduated, Refunding.
pub const CURVE_GRADUATING: u128 = 1;
pub const CURVE_FINAL_MIN: u128 = 2;

/// One desk request as read: `(wrapper, requester, at, bond, closed)`.
pub fn decode_request(r: &[u8]) -> Result<([u8; 20], [u8; 20], u64, u128, bool), String> {
    Ok((crate::abi::addr_at(r, 0, 0)?, crate::abi::addr_at(r, 0, 1)?, crate::abi::uint_at(r, 0, 2, 64)? as u64,
        crate::abi::uint_at(r, 0, 3, 128)?, crate::abi::uint_at(r, 0, 4, 8)? != 0))
}

pub fn call_addr(sel: [u8; 4], a: &[u8; 20]) -> Vec<u8> {
    [&sel[..], &crate::seaport::word_addr(a)].concat()
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: [u8; 20] = [0xa0; 20];
    const B: [u8; 20] = [0xb0; 20];
    const C: [u8; 20] = [0xc0; 20];
    const D: [u8; 20] = [0xd0; 20];

    fn p() -> Dormancy {
        Dormancy::launch(None)
    }

    /// A balance no limit binds on (1,000,000 T).
    const RICH: Option<u128> = Some(1_000_000 * 1_000 * super::B);

    fn dorm(n: usize) -> Dorm {
        let mut d = Dorm::default();
        for c in [A, B, C, D].iter().take(n) {
            d.lives.push(Life::new(*c, Phase::Dormant, 0));
        }
        d
    }

    #[test]
    fn the_selectors_are_the_contracts_own() {
        let k = |s: &str| -> [u8; 4] { crate::evmtx::keccak(s.as_bytes())[..4].try_into().unwrap() };
        assert_eq!(REQUEST_COUNT, k("requestCount()"));
        assert_eq!(REQUEST_AT, k("requestAt(uint256)"));
        assert_eq!(WINDOW, k("window()"));
        assert_eq!(MARK_SERVED, k("markServed(uint256)"));
        assert_eq!(SERVED_AT, k("servedAt(uint256)"));
        assert_eq!(TOTAL_SUPPLY, k("totalSupply()"));
        assert_eq!(HELD, k("held()"));
        assert_eq!(RENTED_HELD, k("rentedHeld()"));
        assert_eq!(GRAD_ETH, k("gradEth()"));
        assert_eq!(GRAD_UNITS, k("gradUnits()"));
        assert_eq!(FLOAT_UNITS, k("floatUnits()"));
        assert_eq!(FACTORY, k("factory()"));
        assert_eq!(COIN_COUNT, k("coinCount()"));
        assert_eq!(COINS, k("coins(uint256)"));
        assert_eq!(ROUTER_OF, k("routerOf(address)"));
        assert_eq!(STATE, k("state()"));
        assert_eq!(IS_FILLED, k("isFilled(address)"));
        assert_eq!(FILL, k("fill(address)"));
    }

    #[test]
    fn the_configuration_is_bounded() {
        assert_eq!(validate(&p(), 10), Ok(()));
        let bad = |f: &dyn Fn(&mut Dormancy), n: usize, want: &str| {
            let mut d = p();
            f(&mut d);
            let e = validate(&d, n).unwrap_err();
            assert!(e.contains(want), "{e}");
        };
        bad(&|d| d.max_active = 0, 10, "max_active");
        bad(&|d| d.max_active = 17, 20, "max_active");
        bad(&|d| d.max_active = 3, 2, "max_active");
        bad(&|d| d.max_waking = 0, 10, "max_waking");
        bad(&|d| d.max_waking = 11, 10, "max_waking");
        bad(&|d| d.wakes_per_day = 0, 10, "wakes_per_day");
        bad(&|d| d.wakes_per_day = 25, 10, "wakes_per_day");
        bad(&|d| d.wake_timeout_secs = 3_599, 10, "wake_timeout_secs");
        bad(&|d| d.wake_timeout_secs = 172_801, 10, "wake_timeout_secs");
        bad(&|d| d.cooldown_secs = 10, 10, "cooldown_secs");
        bad(&|d| d.idle_days = 0, 10, "idle_days");
        bad(&|d| d.idle_days = 61, 10, "idle_days");
        bad(&|d| d.wake_desk = Some("nope".into()), 10, "wake_desk");
        bad(&|d| d.wake_desk = Some(format!("0x{}", "00".repeat(20))), 10, "zero");
        // §11.2: the scaling's bounds; the reserve never under the canister's scan floor
        let sc = |f: &dyn Fn(&mut Scaling), want: &str| bad(&|d| f(d.scaling.as_mut().unwrap()), 10, want);
        sc(&|s| s.active_b_per_day = 499, "active_b_per_day");
        sc(&|s| s.active_b_per_day = 100_001, "active_b_per_day");
        sc(&|s| s.idle_b_per_day = 10_001, "idle_b_per_day");
        sc(&|s| s.wake_b = 99, "wake_b");
        sc(&|s| s.runway_days = 0, "runway_days");
        sc(&|s| s.runway_days = 61, "runway_days");
        sc(&|s| s.reserve_b = 7_999, "scan floor");
        sc(&|s| s.reserve_b = 1_000_001, "reserve_b");
        sc(&|s| s.exit_b_per_awake = 100_001, "exit_b_per_awake");
        let mut none = p();
        none.scaling = None;
        assert_eq!(validate(&none, 10), Ok(()), "fixed caps, as before §11");
        let mut ok = p();
        ok.max_active = 3;
        ok.wake_desk = Some(format!("0x{}", "ab".repeat(20)));
        assert_eq!(validate(&ok, 3), Ok(()));
    }

    #[test]
    fn wakes_start_under_every_cap_owed_first() {
        let mut d = dorm(4);
        let mut q = p();
        q.max_active = 2;
        d.request(&A, WakeBy::Operator, 10).unwrap();
        d.request(&B, WakeBy::Operator, 5).unwrap();
        d.request(&C, WakeBy::Deposit { tx: [1; 32] }, 20).unwrap();
        assert_eq!(d.kind(&A), Kind::Queued);
        assert_eq!(d.position(&C), Some(1), "owed first");
        assert_eq!(d.position(&B), Some(2));
        // max_waking 1: one backfill at a time, the owed one
        let (s, notes) = d.start_wakes(&q, 100, RICH);
        assert_eq!(s, vec![C]);
        assert!(notes.iter().any(|n| n.contains("max_waking")), "{notes:?}");
        assert_eq!(d.kind(&C), Kind::Waking);
        // C active: B wakes next (older than A); then max_active 2 holds A
        assert!(d.activate(&C, 200));
        let (s, _) = d.start_wakes(&q, 201, RICH);
        assert_eq!(s, vec![B]);
        assert!(d.activate(&B, 300));
        let (s, notes) = d.start_wakes(&q, 301, RICH);
        assert!(s.is_empty());
        assert!(notes.iter().any(|n| n.contains("max_active")), "{notes:?}");
        // a request for an awake collection needs no queue
        assert!(d.request(&B, WakeBy::Operator, 302).unwrap().contains("already active"));
    }

    /// Independent review DI-M-1 / DI-H-1: a desk request replaces only a queued desk wake (keeping its place); an owed or the operator's
    /// wake stays; a desk wake is never started once it could no longer post inside its window, and a deposit upgrades it to owed.
    #[test]
    fn desk_requests_queue_within_their_window_and_never_displace_an_owed_or_operator_wake() {
        let q = p();
        let ask = |id: u64, at: u64| DeskAsk { id, requester: [id as u8; 20], at, until: at + 7 * 86_400, tried: false, served: None, marked: false };
        let mut d = dorm(3);
        d.desk_ask(&A, ask(0, 10), &q, 10).unwrap();
        assert_eq!(d.life(&A).unwrap().queued, Some((WakeBy::Desk { id: 0, requester: [0; 20] }, 10)));
        // a later request (the first closed on chain) takes the queued desk wake's place at its own read time (D2-I-2: never the
        // earlier request's place in the queue)
        d.desk_ask(&A, ask(1, 50), &q, 55).unwrap();
        assert_eq!(d.life(&A).unwrap().queued, Some((WakeBy::Desk { id: 1, requester: [1; 20] }, 55)));
        // DI-H-1: a deposit upgrades it to owed, keeping its place; a later desk request never downgrades it
        d.request(&A, WakeBy::Deposit { tx: [7; 32] }, 60).unwrap();
        d.desk_ask(&A, ask(2, 70), &q, 70).unwrap();
        assert_eq!(d.life(&A).unwrap().queued, Some((WakeBy::Deposit { tx: [7; 32] }, 55)));
        // the operator's wake stays too
        d.request(&B, WakeBy::Operator, 5).unwrap();
        d.desk_ask(&B, ask(3, 6), &q, 6).unwrap();
        assert_eq!(d.life(&B).unwrap().queued, Some((WakeBy::Operator, 5)));
        // too late in its window: held, not queued; a queued one is dropped once too late, and never started
        let late = 7 * 86_400 - q.wake_timeout_secs - DESK_START_MARGIN_SECS;
        assert!(d.desk_ask(&C, ask(4, 0), &q, late + 1).unwrap().contains("too late"));
        assert_eq!(d.life(&C).unwrap().queued, None);
        d.desk_ask(&C, ask(5, 100), &q, 100).unwrap();
        assert!(d.desk_upkeep(&q, 100 + late).is_empty(), "still in time");
        assert!(d.desk_upkeep(&q, 101 + late).iter().any(|n| n.contains("dropped from the queue")));
        assert_eq!(d.kind(&C), Kind::Dormant);
        let (s, _) = d.start_wakes(&q, 102 + late, RICH);
        assert!(!s.contains(&C));
        // past its window the ask is forgotten
        d.desk_upkeep(&q, 100 + 7 * 86_400);
        assert!(d.life(&C).unwrap().desk.is_none());
    }

    #[test]
    fn wakes_per_day_and_the_cooldown_bound_what_spam_can_spend() {
        let mut d = dorm(4);
        let mut q = p();
        q.max_active = 4;
        q.max_waking = 4;
        q.wakes_per_day = 2;
        for c in [A, B, C] {
            d.request(&c, WakeBy::Operator, 1).unwrap();
        }
        let (s, notes) = d.start_wakes(&q, 10, RICH);
        assert_eq!(s.len(), 2);
        assert!(notes.iter().any(|n| n.contains("wakes_per_day")), "{notes:?}");
        // the two time out: dormant with the cooldown
        assert_eq!(d.timeouts(&q, 10 + q.wake_timeout_secs).len(), 2);
        assert_eq!(d.kind(&A), Kind::Dormant);
        // a day later the third starts; the first two, asked again, wait for their cooldown unless owed
        let t = 10 + 86_400;
        d.request(&A, WakeBy::Operator, t).unwrap();
        let (s, notes) = d.start_wakes(&q, t, RICH);
        assert_eq!(s, vec![C]);
        assert!(notes.iter().any(|n| n.contains("cooldown")), "{notes:?}");
        d.request(&A, WakeBy::Graduation, t).unwrap();
        let (s, _) = d.start_wakes(&q, t + 1, RICH);
        assert_eq!(s, vec![A], "owed: the cooldown does not hold it");
    }

    #[test]
    fn dr_h_2_an_owed_wake_that_times_out_backs_off_and_the_owed_reserve_lets_it_start_past_max_active() {
        let q = Dormancy { max_active: 1, max_waking: 1, wakes_per_day: 24, wake_timeout_secs: 3_600, cooldown_secs: 3_600, idle_days: 1,
            wake_desk: None, scaling: None };
        let mut d = Dorm { lives: vec![Life::new(A, Phase::Active { since: 0 }, 0), Life::new(B, Phase::Dormant, 0), Life::new(C, Phase::Dormant, 0)],
            ..Default::default() };
        // A holds the one slot: a desk wake of C waits; an owed wake of B takes the reserve
        d.request(&C, WakeBy::Operator, 10).unwrap();
        d.request(&B, WakeBy::Deposit { tx: [7; 32] }, 10).unwrap();
        let (s, _) = d.start_wakes(&q, 10, RICH);
        assert_eq!(s, vec![B], "the owed reserve");
        // it times out: the next owed wake waits the cooldown, then twice it, then four times
        let mut t = 10 + 3_600;
        for (n, wait) in [(1u8, 3_600u64), (2, 7_200), (3, 14_400)] {
            assert_eq!(d.timeouts(&q, t), vec![B]);
            let l = d.life(&B).unwrap();
            assert_eq!((l.owed_fails, l.owed_after), (n, t + wait));
            d.request(&B, WakeBy::Deposit { tx: [7; 32] }, t).unwrap();
            let (s, notes) = d.start_wakes(&q, t + wait - 1, RICH);
            assert!(s.is_empty() && notes.iter().any(|x| x.contains("owed wake(s) timed out")), "{notes:?}");
            let (s, _) = d.start_wakes(&q, t + wait, RICH);
            assert_eq!(s, vec![B]);
            t += wait + 3_600;
        }
        assert!(d.life(&C).unwrap().queued.is_some(), "the desk wake still waits for A's slot, never for B");
        // a wake that becomes ACTIVE clears the count
        assert!(d.activate(&B, t));
        assert_eq!((d.life(&B).unwrap().owed_fails, d.life(&B).unwrap().owed_after), (0, 0));
    }

    #[test]
    fn dr_m_1_converged_is_within_the_mismatch_of_a_posted_floor() {
        assert!(converged(1_000, 1_000) && converged(1_050, 1_000) && !converged(1_051, 1_000) && !converged(1_000, 0));
    }

    #[test]
    fn the_cadence_slows_backs_off_and_speeds_up_when_hot() {
        let mut l = Life::new(A, Phase::Active { since: 0 }, 0);
        l.pass_at = 1_000;
        l.posted_at = 1_000;
        assert!(!pass_due(&l, 1_000 + PASS_EVERY_SECS - 1, false));
        assert!(pass_due(&l, 1_000 + PASS_EVERY_SECS, false));
        // DR-M-3: whatever the cadence, a pass is due TTL_MARGIN_SECS before the module's post goes stale
        l.pass_at = 2_500;
        assert!(!pass_due(&l, 1_000 + FLOOR_TTL_SECS - TTL_MARGIN_SECS - 1, false));
        assert!(pass_due(&l, 1_000 + FLOOR_TTL_SECS - TTL_MARGIN_SECS, false));
        l.pass_at = 1_000;
        assert!(pass_due(&l, 1_001, true), "hot: every hand-in");
        note_scan(&mut l, 2_000, false);
        assert_eq!(l.scan_every, 2 * SCAN_EVERY_SECS);
        note_scan(&mut l, 3_000, false);
        note_scan(&mut l, 4_000, false);
        assert_eq!(l.scan_every, SCAN_BACKOFF_MAX_SECS);
        assert!(!scan_due(&l, 4_000 + SCAN_BACKOFF_MAX_SECS - 1, false));
        assert!(scan_due(&l, 4_001, true));
        note_scan(&mut l, 5_000, true);
        assert_eq!(l.scan_every, SCAN_EVERY_SECS, "a new sale resets it");
        let w = Life::new(B, Phase::Waking { since: 0, backfilled_at: None }, 0);
        assert!(pass_due(&w, 1, false) && scan_due(&w, 1, false));
        let z = Life::new(C, Phase::Dormant, 0);
        assert!(!pass_due(&z, u64::MAX / 2, true) && !scan_due(&z, 1, true) && !view_due(&z, 1, true));
    }

    #[test]
    fn idle_is_idle_days_after_the_last_position_or_a_day_with_none() {
        let q = p();
        let mut l = Life::new(A, Phase::Active { since: 1_000 }, 1_000);
        assert!(idle_long_enough(&l, &q, 1_000 + NO_POSITION_SECS - 1).is_err());
        assert!(idle_long_enough(&l, &q, 1_000 + NO_POSITION_SECS).is_ok());
        l.position_at = Some(50_000);
        assert!(idle_long_enough(&l, &q, 50_000 + 3 * 86_400 - 1).is_err());
        assert!(idle_long_enough(&l, &q, 50_000 + 3 * 86_400).is_ok());
    }

    #[test]
    fn the_estimate_counts_passes_of_logs_and_receipts() {
        let top = 26_000_000;
        // nothing read: the whole window, the default moves
        let e0 = wake_eta_secs(None, top, 52, WAKE_RECEIPTS_PER_PASS as u64, 0);
        assert_eq!(e0, (700u64.div_ceil(64) + 1) * WAKE_PASS_SECS);
        // half the window read, 300 moves seen in it, 40 pending: 340 left → 6 passes of receipts; logs 1 pass
        let mut sc = SaleScan { collection: A, down: top - 25_200, up: top, chunk: 500, todo: vec![], pend: Some(vec![crate::ethnft::Move {
            hash: [1; 32], block: top, token: [0; 32], from: [0; 20] }; 40]), rescan: None, judged: None, bad: None, skipped: None,
            done_big: None, stuck: None, spent: None, hole: None, lost: None, behind: None, rfail: None, raw: None, current: None,
            hole_deep: None, rbusy: None, busy: None };
        let e = wake_eta_secs(Some(&sc), top, 52, 64, 300);
        assert_eq!(e, ((40u64 + 300 * 25_200 / 25_201).div_ceil(64) + 1) * WAKE_PASS_SECS);
        assert!(!backfill_gaps(Some(&sc), top).is_empty());
        // read down to the window's start, nothing pending: complete
        sc.down = top - crate::ethnft::SALE_WINDOW_BLOCKS;
        sc.pend = Some(vec![]);
        assert!(backfill_gaps(Some(&sc), top).is_empty(), "{:?}", backfill_gaps(Some(&sc), top));
        assert_eq!(wake_eta_secs(Some(&sc), top, 52, 64, 300), 2 * WAKE_PASS_SECS);
        sc.hole = Some((top - 10_000, top - 9_000));
        assert!(backfill_gaps(Some(&sc), top).iter().any(|g| g.contains("hole")));
    }

    #[test]
    fn status_words() {
        assert_eq!(status_text(Kind::Waking, Some(2_400), None, 0, 0), "warming up (~40 min)");
        assert_eq!(status_text(Kind::Queued, None, Some(2), 0, 0), "queued (#2): it warms up when there is room");
        assert_eq!(status_text(Kind::Dormant, None, None, 0, 5), "dormant: a launch wakes it");
        assert!(status_text(Kind::Dormant, None, None, 3_600, 0).contains("~60 min"));
        assert_eq!(status_text(Kind::Active, None, None, 0, 0), "active");
    }

    #[test]
    fn within_bps_is_relative_to_the_reference() {
        assert!(within_bps(105, 100, 500));
        assert!(!within_bps(106, 100, 500));
        assert!(within_bps(95, 100, 500));
        assert!(!within_bps(94, 100, 500));
    }

    // ---- §11: the auto-scaling from the cycles balance -------------------------------------------------------------------------

    const TT: u128 = 1_000 * super::B;

    #[test]
    fn scaling_the_formula_at_the_launch_numbers() {
        let s = Scaling::launch();
        // slot = 7 × 2.5 T + 3 T exit = 20.5 T; fixed = 8 T reserve + 7 × 0.3 T idle = 10.1 T
        assert_eq!(s.slot(), 20_500 * super::B);
        assert_eq!(s.fixed(), 10_100 * super::B);
        assert_eq!(s.need(1), 30_600 * super::B);
        assert_eq!(s.need(10), 215_100 * super::B);
        // effective = min(ceiling, ⌊(balance − fixed) / slot⌋)
        for (bal, want) in [(0, 0), (10_100 * super::B, 0), (30_600 * super::B - 1, 0), (30_600 * super::B, 1), (51_100 * super::B - 1, 1),
            (51_100 * super::B, 2), (215_100 * super::B, 10), (10_000 * TT, 10)] {
            assert_eq!(effective_max_active(&s, bal, 10), want, "balance {bal}");
        }
        assert_eq!(effective_max_active(&s, 10_000 * TT, 3), 3, "never above the ceiling");
    }

    #[test]
    fn scaling_a_zero_or_low_balance_wakes_nothing_and_its_reserve_is_never_spent() {
        let s = Scaling::launch();
        for bal in [0, 1, s.wake(), s.fixed(), s.need(1) - 1] {
            assert_eq!(effective_max_active(&s, bal, 10), 0);
            assert_eq!(wake_headroom(&s, bal, 0, 10), 0);
            assert!(!wake_fits(&s, bal, 0));
        }
        // an owed wake may use the runways, never the reserve: 8 T + 3 T (this one's exit) + 1.5 T (its cost) = 12.5 T
        assert!(!owed_wake_fits(&s, 12_500 * super::B - 1, 0));
        assert!(owed_wake_fits(&s, 12_500 * super::B, 0));
        assert!(!owed_wake_fits(&s, 15_500 * super::B - 1, 1), "one awake: its exit reserve too");
        assert!(!owed_wake_fits(&s, 0, 0) && !owed_wake_fits(&s, s.wake() - 1, 0));
    }

    #[test]
    fn scaling_a_wake_at_exactly_its_threshold() {
        let s = Scaling::launch();
        // the first wake: 1.5 T + need(1) = 32.1 T; the second with one awake: 1.5 T + need(2) = 52.6 T
        assert!(!wake_fits(&s, 32_100 * super::B - 1, 0));
        assert!(wake_fits(&s, 32_100 * super::B, 0));
        assert!(!wake_fits(&s, 52_600 * super::B - 1, 1));
        assert!(wake_fits(&s, 52_600 * super::B, 1));
        // the headroom is the same rule, one wake after another: k with balance − k × 1.5 ≥ need(awake + k)
        assert_eq!(wake_headroom(&s, 32_100 * super::B - 1, 0, 10), 0);
        assert_eq!(wake_headroom(&s, 32_100 * super::B, 0, 10), 1);
        assert_eq!(wake_headroom(&s, 10_100 * super::B + 2 * 22_000 * super::B, 0, 10), 2);
        assert_eq!(wake_headroom(&s, 10_100 * super::B + 2 * 22_000 * super::B - 1, 0, 10), 1);
        // whatever the headroom says, the per-wake rule agrees with it
        for bal in (0..200u128).map(|x| x * 1_000 * super::B) {
            for awake in 0..5usize {
                assert_eq!(wake_headroom(&s, bal, awake, 10) >= 1, wake_fits(&s, bal, awake), "balance {bal}, awake {awake}");
            }
        }
        let mut d = dorm(2);
        let mut q = p();
        q.max_active = 4;
        d.request(&A, WakeBy::Operator, 1).unwrap();
        let (st, notes) = d.start_wakes(&q, 2, Some(32_100 * super::B - 1));
        assert!(st.is_empty() && notes.iter().any(|n| n.contains("a wake needs 32100 B available")), "{notes:?}");
        assert_eq!(d.start_wakes(&q, 3, Some(32_100 * super::B)).0, vec![A]);
    }

    #[test]
    fn scaling_positions_pin_collections_awake_above_the_limit_and_no_new_wake_starts() {
        let s = Scaling::launch();
        let mut q = p();
        q.max_active = 4;
        // three ACTIVE with positions; the balance keeps one
        let mut d = Dorm { lives: vec![Life::new(A, Phase::Active { since: 0 }, 0), Life::new(B, Phase::Active { since: 0 }, 0),
            Life::new(C, Phase::Active { since: 0 }, 0), Life::new(D, Phase::Dormant, 0)], ..Default::default() };
        let bal = Some(40 * TT);
        let cap = capacity(&q, &d, bal, 4, 3, 10);
        assert_eq!((cap.effective_max_active, cap.awake, cap.pinned, cap.wake_headroom), (1, 3, 3, 0));
        assert!(cap.status.contains("1 of 4 collections can be awake on this balance, but 3 hold positions"), "{}", cap.status);
        // a desk wake waits; so does an owed one: it has one slot above what the balance keeps (1 + 1), and 3 are awake (review S-1)
        d.request(&D, WakeBy::Operator, 5).unwrap();
        let (st, notes) = d.start_wakes(&q, 6, bal);
        assert!(st.is_empty() && notes.iter().any(|n| n.contains("keeps 1 awake")), "{notes:?}");
        d.request(&D, WakeBy::Deposit { tx: [9; 32] }, 7).unwrap();
        let (st, notes) = d.start_wakes(&q, 8, bal);
        assert!(st.is_empty() && notes.iter().any(|n| n.contains("an owed wake, but 3 are awake and the cycles balance keeps 1")), "{notes:?}");
        // one awake on a balance that keeps one: the owed wake takes the slot above it, using the runway, not the reserve
        let mut o = Dorm { lives: vec![Life::new(A, Phase::Active { since: 0 }, 0), Life::new(B, Phase::Dormant, 0), Life::new(C, Phase::Dormant, 0)],
            ..Default::default() };
        o.request(&B, WakeBy::Deposit { tx: [8; 32] }, 1).unwrap();
        o.request(&C, WakeBy::Graduation, 1).unwrap();
        let (st, notes) = o.start_wakes(&Dormancy { max_waking: 2, ..q.clone() }, 2, Some(31 * TT));
        assert_eq!(st, vec![B], "one owed slot above the balance's limit, not two: {notes:?}");
        // nothing awake, the balance under one runway: an owed wake still starts while the reserve stays whole (8 T + 3 T + 1.5 T)
        let mut e = Dorm { lives: vec![Life::new(B, Phase::Dormant, 0)], ..Default::default() };
        e.request(&B, WakeBy::Graduation, 1).unwrap();
        let (st, notes) = e.start_wakes(&q, 2, Some(s.reserve_for(1) + s.wake() - 1));
        assert!(st.is_empty() && notes.iter().any(|n| n.contains("would eat the reserve")), "{notes:?}");
        assert_eq!(e.start_wakes(&q, 3, Some(s.reserve_for(1) + s.wake())).0, vec![B]);
    }

    #[test]
    fn scaling_a_top_up_raises_the_limit_and_the_wakes_already_under_way_are_counted() {
        let mut q = p();
        q.max_active = 10;
        q.max_waking = 3;
        let mut d = dorm(4);
        for c in [A, B, C] {
            d.request(&c, WakeBy::Operator, 1).unwrap();
        }
        // 20 T: nothing
        let (st, _) = d.start_wakes(&q, 2, Some(20 * TT));
        assert!(st.is_empty());
        assert_eq!(capacity(&q, &d, Some(20 * TT), 4, 0, 2).effective_max_active, 0);
        // topped up to 60 T: A wakes (32.1 T); B too (after A's cost 58.5 T counts, and B needs 1.5 T + need(2) = 52.6 T); not C
        // (after two costs 57 T counts, and C needs 1.5 T + need(3) = 73.1 T)
        let (st, notes) = d.start_wakes(&q, 3, Some(60 * TT));
        assert_eq!(st, vec![A, B]);
        assert!(notes.iter().any(|n| n.contains("57000 B once the backfills under way are paid) keeps 2 awake")), "{notes:?}");
        // the next tick, the balance read before the backfills paid: still counted as paid, so C still waits
        let (st, _) = d.start_wakes(&q, 4, Some(60 * TT));
        assert!(st.is_empty());
        // a top-up to 500 T: the rest wake at once, up to max_waking
        let (st, _) = d.start_wakes(&q, 6, Some(500 * TT));
        assert_eq!(st, vec![C]);
        let cap = capacity(&q, &d, Some(500 * TT), 4, 0, 6);
        assert_eq!(cap.effective_max_active, 10);
        assert_eq!(cap.wakes_left_today, 9);
    }

    #[test]
    fn scaling_without_a_balance_fails_closed_and_without_scaling_the_ceiling_is_the_limit() {
        let mut d = dorm(2);
        d.request(&A, WakeBy::Deposit { tx: [1; 32] }, 1).unwrap();
        let (st, notes) = d.start_wakes(&p(), 2, None);
        assert!(st.is_empty() && notes.iter().any(|n| n.contains("balance was not read")), "{notes:?}");
        assert!(capacity(&p(), &d, None, 2, 0, 2).status.contains("could not be read"));
        let mut fixed = p();
        fixed.scaling = None;
        fixed.max_active = 2;
        let cap = capacity(&fixed, &d, None, 2, 0, 2);
        assert_eq!((cap.scaling, cap.effective_max_active, cap.wake_headroom), (false, 2, 2));
        assert_eq!(d.start_wakes(&fixed, 3, None).0, vec![A], "fixed caps need no balance");
    }

    #[test]
    fn scaling_never_overflows_or_underflows() {
        let mut s = Scaling::launch();
        for bal in [0, 1, u128::MAX / 2, u128::MAX - 1, u128::MAX] {
            for awake in [0usize, 1, 9, 10, 255, usize::MAX / 2] {
                let _ = (effective_max_active(&s, bal, 16), wake_fits(&s, bal, awake), owed_wake_fits(&s, bal, awake),
                    wake_headroom(&s, bal, awake, 16), available(&s, bal, awake));
            }
        }
        assert_eq!(effective_max_active(&s, u128::MAX, 16), 16);
        assert_eq!(wake_headroom(&s, u128::MAX, 20, 16), 0, "awake above the ceiling: no headroom, no underflow");
        assert_eq!(available(&s, 1, 3), 0);
        // the largest configuration the bounds allow
        s = Scaling { active_b_per_day: SCALE_ACTIVE_B_MAX, idle_b_per_day: SCALE_IDLE_B_MAX, wake_b: SCALE_WAKE_B_MAX,
            runway_days: SCALE_RUNWAY_DAYS_MAX, reserve_b: SCALE_RESERVE_B_MAX, exit_b_per_awake: SCALE_EXIT_B_MAX };
        assert!(validate_scaling(&s).is_ok());
        assert!(s.need(u128::MAX) == u128::MAX && s.need(16) < u128::MAX / 2);
        assert_eq!(effective_max_active(&s, 0, 16), 0);
        let d = dorm(1);
        let cap = capacity(&Dormancy { scaling: Some(s), ..p() }, &d, Some(u128::MAX), 1, 0, 0);
        assert!(cap.next_wake_at > 0 && cap.effective_max_active == 10);
    }
}
