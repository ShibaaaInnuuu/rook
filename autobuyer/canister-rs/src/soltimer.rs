//! W14b (docs/v9/canister-only-solana.md §13.1, §5.4 "a canister timer would remove the runner from the latency path"): the canister
//! drives its four Solana send lines and the registry keeper ITSELF, on an `ic-cdk-timers` one-shot timer re-armed after every run.
//! The runner's `tick` stays the entry for everything else (and still ticks the lines too: either alone is enough).
//!
//! - **Cadence.** [`BUSY_SECS`] while anything is out on a line (a transaction to resend, settle, re-sign or burn) or a flow waits its
//!   turn; [`IDLE_SECS`] otherwise ([`next_delay`]). An open incident is not work (only the operator's clear is), so it idles.
//! - **No overlap.** A run takes the tick's flag (`canister::hold_tick`): while the runner's `tick` (or an operator method that writes
//!   ticked state) holds it, the run does nothing and comes back at the busy cadence; while a run holds it, the runner's `tick` is the
//!   one refused. The timer is one-shot and re-armed by its own run, so two runs of it never overlap either.
//! - **A trap cannot stop it.** Each run first arms a FALLBACK at the idle cadence, then does its work, then replaces the fallback with
//!   the cadence it found; a run that traps after an await leaves the fallback armed. Timers do not survive an upgrade: `init` and
//!   `post_upgrade` arm the first run ([`arm_after_install`]).
//! - **Paused, off, no home.** Paused: nothing is signed or sent by the timer (the runner's paused pass keeps custody's duties); the
//!   operator's switch ([`sol_timer`], saved in the lines' state) turns it off; without a Solana home it does nothing. Each re-arms at the
//!   idle cadence, so lifting any of them takes effect within a minute.
//! - **Cluster check.** The run checks the providers' genesis hash (as `tick` does) at most once every [`HOME_CHECK_SECS`].
use std::cell::RefCell;
use std::time::Duration;

/// Seconds between runs while a line has something out (Solana finalizes in ≈ 13 s; a status read costs outcalls).
pub const BUSY_SECS: u64 = 10;
/// Seconds between runs while every line is idle (the keeper's schedule is in hours; a flow's new step comes from the runner).
pub const IDLE_SECS: u64 = 60;
/// The providers' cluster (genesis hash) is checked at most this often by the timer.
pub const HOME_CHECK_SECS: u64 = 3_600;
/// The first run after an install or upgrade.
pub const FIRST_SECS: u64 = 5;

/// What one run found (pure input of [`next_delay`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Run {
    /// It ran the pass; `busy`: something is still out or queued on a line.
    Ran { busy: bool },
    /// The tick's flag was held (the runner's tick, or an operator method): nothing done.
    Held,
    /// Paused, switched off, or no Solana home: nothing done.
    Idle,
    /// The pass could not start (the cluster check, the providers): nothing done.
    Failed,
}

/// The next run's delay after `r` (pure): busy and held come back soon; everything else at the idle cadence.
pub fn next_delay(r: Run) -> u64 {
    match r {
        Run::Ran { busy: true } | Run::Held => BUSY_SECS,
        Run::Ran { busy: false } | Run::Idle | Run::Failed => IDLE_SECS,
    }
}

/// Whether the cluster must be checked again (pure): never checked, or checked more than [`HOME_CHECK_SECS`] ago.
pub fn home_check_due(last_ok: Option<u64>, now: u64) -> bool {
    last_ok.is_none_or(|t| now.saturating_sub(t) >= HOME_CHECK_SECS)
}

thread_local! {
    /// The armed timer (the fallback while a run is in progress).
    static ARMED: RefCell<Option<ic_cdk_timers::TimerId>> = const { RefCell::new(None) };
    /// When the timer last saw the providers serve our cluster.
    static HOME_OK: RefCell<Option<u64>> = const { RefCell::new(None) };
}

fn now_secs() -> u64 {
    ic_cdk::api::time() / 1_000_000_000
}

fn arm(secs: u64) {
    let id = ic_cdk_timers::set_timer(Duration::from_secs(secs), run());
    if let Some(old) = ARMED.with(|a| a.borrow_mut().replace(id)) {
        ic_cdk_timers::clear_timer(old);
    }
    crate::solhome::LINES.with(|l| l.borrow_mut().timer.get_or_insert_with(Default::default).next_at = now_secs() + secs);
}

/// `init` / `post_upgrade`: arm the first run (timers do not survive an upgrade).
pub(crate) fn arm_after_install() {
    arm(FIRST_SECS);
}

async fn run() {
    // the fallback first: a trap after an await below cannot stop the timer
    arm(IDLE_SECS);
    let r = pass().await;
    let now = now_secs();
    crate::solhome::LINES.with(|l| {
        let mut l = l.borrow_mut();
        let t = l.timer.get_or_insert_with(Default::default);
        t.last_run = now;
        t.runs = t.runs.saturating_add(1);
        if r == Run::Held {
            t.skipped = t.skipped.saturating_add(1);
        }
        if let Run::Ran { busy } = r {
            t.busy = busy;
        }
    });
    arm(next_delay(r));
}

async fn pass() -> Run {
    let off = crate::solhome::LINES.with(|l| l.borrow().timer_off == Some(true));
    if off || crate::canister::not_paused().is_err() || crate::canister::cfg().home.is_none() {
        return Run::Idle;
    }
    let Ok(_held) = crate::canister::hold_tick() else { return Run::Held };
    let now = now_secs();
    if home_check_due(HOME_OK.with(|h| *h.borrow()), now) {
        if let Err(e) = crate::canister::assert_home().await {
            crate::canister::note(format!("sol timer: waits: {e}"));
            return Run::Failed;
        }
        HOME_OK.with(|h| *h.borrow_mut() = Some(now));
    }
    let (lines, busy) = lines_pass().await;
    let desk = crate::desk::api::tick().await;
    let coins = crate::coins::api::tick().await;
    let pool = crate::pooldesk::api::tick().await;
    let solnft = crate::solnftflow::api::tick_ic().await;
    let cranks = crate::cranks::tick_ic().await;
    for t in lines.iter().chain(desk.iter()).chain(coins.iter()).chain(pool.iter()).chain(solnft.iter()).chain(cranks.iter()) {
        if !t.contains("waits") {
            crate::canister::note(format!("timer: {t}"));
        }
    }
    Run::Ran { busy: busy || crate::desk::state::busy() }
}

/// The IC world of the lines' pass: the configured providers and the threshold key; every line through `solhome`'s thread-locals.
pub(crate) struct LinesNet {
    sol: Option<crate::solhome::SolNet>,
}

crate::sol_io_via!(LinesNet, sol);

/// One fair pass over the four lines (`solhome::tick_lines`), the cursor saved. Called by the timer and by the runner's `tick` (holding the
/// tick's flag). Returns the log and whether anything is still out.
pub(crate) async fn lines_pass() -> (Vec<String>, bool) {
    let sol = match crate::solhome::SolNet::of_install() {
        Ok(Some(s)) => s,
        Ok(None) => return (vec![], false),
        Err(e) => return (vec![format!("sol lines: waits: {e}")], false),
    };
    let cursor = crate::solhome::LINES.with(|l| l.borrow().next);
    let (out, next, busy) = crate::solhome::tick_lines(&mut LinesNet { sol: Some(sol) }, cursor, now_secs()).await;
    crate::solhome::LINES.with(|l| l.borrow_mut().next = Some(next));
    (out, busy)
}

/// The operator switches the canister's own Solana timer on or off (saved; the runner's `tick` drives the lines either way). Answers the
/// timer's state. Refused mid-tick (the lines' state is ticked state).
#[ic_cdk::update]
pub fn sol_timer(on: bool) -> Result<String, String> {
    crate::canister::only_operator();
    crate::canister::not_paused()?;
    crate::canister::not_ticking()?;
    crate::solhome::LINES.with(|l| l.borrow_mut().timer_off = (!on).then_some(true));
    if on {
        arm(FIRST_SECS);
    }
    Ok(format!("the canister's Solana timer is {}: busy every {BUSY_SECS} s, idle every {IDLE_SECS} s", if on { "on" } else { "off" }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cadence_is_busy_while_work_is_out_and_idle_otherwise() {
        assert_eq!(next_delay(Run::Ran { busy: true }), BUSY_SECS);
        assert_eq!(next_delay(Run::Held), BUSY_SECS, "a run that found the runner's tick comes back soon");
        for r in [Run::Ran { busy: false }, Run::Idle, Run::Failed] {
            assert_eq!(next_delay(r), IDLE_SECS, "{r:?}");
        }
        assert!(BUSY_SECS < IDLE_SECS && FIRST_SECS <= BUSY_SECS);
    }

    #[test]
    fn the_cluster_is_checked_once_an_hour_at_most() {
        assert!(home_check_due(None, 0));
        assert!(!home_check_due(Some(1_000), 1_000 + HOME_CHECK_SECS - 1));
        assert!(home_check_due(Some(1_000), 1_000 + HOME_CHECK_SECS));
        assert!(!home_check_due(Some(5_000), 1_000), "a clock that went back is not a reason to check");
    }

    /// The run arms its fallback BEFORE its first await and replaces it after, takes the tick's flag, and is armed at install and upgrade.
    #[test]
    fn a_run_arms_its_fallback_first_and_never_overlaps_a_tick() {
        let src = include_str!("soltimer.rs");
        let run = src.split("async fn run() {").nth(1).unwrap().split("\n}\n").next().unwrap();
        let (fallback, work, rearm) = (run.find("arm(IDLE_SECS)").unwrap(), run.find("pass().await").unwrap(), run.find("arm(next_delay(r))").unwrap());
        assert!(fallback < work && work < rearm, "{run}");
        let pass = src.split("async fn pass() -> Run {").nth(1).unwrap().split("\n}\n").next().unwrap();
        let (held, first_await) = (pass.find("hold_tick()").unwrap(), pass.find(".await").unwrap());
        assert!(held < first_await, "the flag is taken before anything is awaited");
        let canister = include_str!("canister.rs");
        for f in ["fn init(arg: Init) {", "fn post_upgrade(arg: Init) {"] {
            let body = canister.split(f).nth(1).unwrap().split("\n}\n").next().unwrap();
            assert!(body.contains("soltimer::arm_after_install()"), "{f} arms the timer");
        }
    }
}
