//! The coins' tick (W6): settle what the crank line reported, then at most ONE step signed: a redemption (a holder waits), a graduation step
//! (the backing), the opening of the next collection, a fee cash, a fee claim; discovery of new coins runs on its own interval (reads only).
//! Never queued: a step signs only on a free crank line (`crankline::line_free`), and a refusal by simulation is the step's to wait out, not
//! the line's error.
use crate::chainlink::driver::{DeskIo, SignError};
use crate::chainlink::ledger::Phase;
use crate::desk::state as desk;
use crate::solhome::{self, LineId, LineOwner, LineStep};

use super::state::*;
use super::step::{CoinStep, Planned};
use super::{fees, grad, link, open, redeem};

/// The next look at the collection that is not open yet, when it waited (seconds).
pub const OPEN_EVERY: u64 = 120;

/// One pass of the coins. `now` in unix seconds.
pub async fn tick<I: DeskIo>(io: &mut I, now: u64) -> Vec<String> {
    let mut log = vec![];
    let idle = with(|s| s.off || (s.pins.is_empty() && s.coins.is_empty() && s.redeems.is_empty() && s.receipts.is_empty()));
    if idle {
        return log;
    }
    with_mut(|s| s.roll_day(now));
    for (k, excluded) in with_mut(|s| s.apply_exclusions(now)) {
        log.push(format!("coins: {} is {} the supply a redemption divides by (asked a day ago)", crate::soltx::b58_encode(&k), if excluded { "left out of" } else { "counted in" }));
    }
    settle(io, now, &mut log);
    if io.sol_home().is_none() {
        return log;
    }
    // reads that sign nothing
    if with(|s| s.due("discover", now)) && with(|s| s.cols.values().any(|c| c.stage == ColStage::Open)) {
        log.extend(grad::discover(io, now).await);
    }
    // a step needs the free crank line and the day's budget
    if let Err(e) = crate::crankline::line_free(&io.sol_line_of(LineId::Crank)) {
        if with(|s| s.pins.len() > s.cols.values().filter(|c| c.stage == ColStage::Open).count() || !s.coins.is_empty() || !s.redeems.is_empty()) {
            log.push(format!("coins: waits: {e}"));
        }
        return log;
    }
    if with(|s| s.signed_today >= STEPS_PER_DAY) {
        log.push(format!("coins: waits: {STEPS_PER_DAY} steps today already (the day's budget)"));
        return log;
    }
    if let Some(l) = pick(io, now, &mut log).await {
        log.push(sign(io, l, now).await);
    }
    log
}

/// The first step with work, in priority order. Reads only.
async fn pick<I: DeskIo>(io: &mut I, now: u64, log: &mut Vec<String>) -> Option<Planned> {
    // 1. a holder waiting for a redemption
    match redeem::plan(io, now).await {
        Ok(Some(p)) => return Some(p),
        Ok(None) => {}
        Err(e) => log.push(format!("coins: redemptions wait: {e}")),
    }
    // 2. the backing of a graduated coin (and the migration itself): checked every discovery interval
    if with(|s| s.due("graduate", now)) {
        match grad::plan_graduation(io, now).await {
            Ok(Some(p)) => return Some(p),
            Ok(None) => with_mut(|s| s.set_next("graduate", now + grad::DISCOVER_EVERY)),
            Err(e) => {
                with_mut(|s| s.set_next("graduate", now + 300));
                log.push(format!("coins: graduation waits: {e}"));
            }
        }
    }
    // 3. the next collection
    if with(|s| s.due("open", now)) {
        match open::pass(io, now).await {
            open::Opened::Signed(sig) => {
                // signed inside `open::pass` itself: nothing more this tick
                log.push(format!("coins: a collection's opening step is signed ({sig})"));
                with_mut(|s| s.set_next("open", now));
                return None;
            }
            open::Opened::Open(i) => {
                log.push(format!("coins: collection {i} is open"));
                with_mut(|s| s.set_next("open", now));
            }
            open::Opened::Waits(w) => {
                if with(|s| s.pins.len() > s.cols.values().filter(|c| c.stage == ColStage::Open).count()) {
                    log.push(format!("coins: {w}"));
                }
                with_mut(|s| s.set_next("open", now + OPEN_EVERY));
            }
            open::Opened::Nothing => with_mut(|s| s.set_next("open", now + OPEN_EVERY * 5)),
        }
    }
    // 4. fee wX: cash them
    if with(|s| s.due("cash", now)) {
        match fees::plan_cash(io, now).await {
            Ok(Some(p)) => return Some(p),
            Ok(None) => with_mut(|s| s.set_next("cash", now + fees::CASH_EVERY)),
            Err(e) => {
                with_mut(|s| s.set_next("cash", now + 300));
                log.push(format!("coins: fee cash waits: {e}"));
            }
        }
    }
    // 5. claims
    let (curve, lp) = with(|s| (s.due("curve", now), s.due("lp", now)));
    if curve || lp {
        match fees::plan_claims(io, now, curve, lp).await {
            Ok(Some(p)) => return Some(p),
            Ok(None) => with_mut(|s| {
                if curve {
                    s.set_next("curve", now + fees::CURVE_EVERY);
                }
                if lp {
                    s.set_next("lp", now + fees::LP_EVERY);
                }
            }),
            Err(e) => {
                with_mut(|s| {
                    s.set_next("curve", now + 600);
                    s.set_next("lp", now + 600);
                });
                log.push(format!("coins: fee claims wait: {e}"));
            }
        }
    }
    None
}

/// Signs a planned step on the crank line: a linked one through the gate, a plain one by its instructions alone.
async fn sign<I: DeskIo>(io: &mut I, p: Planned, now: u64) -> String {
    let label = p.step.label();
    let key = p.step.key();
    let before = io.sol_line_of(LineId::Crank).line.error.clone();
    let step = LineStep::Coin(p.step.clone());
    let r = match p.link {
        Some(id) => link::sign_linked(io, LineOwner::Coin, step, id, p.ixs, p.keys, now).await.map_err(|e| match e {
            SignError::Refused(b) => format!("REFUSED by pre_sign: {b}"),
            SignError::Deferred(s) => s,
        }),
        None => link::sign_plain(io, LineOwner::Coin, step, p.ixs, p.keys, now).await,
    };
    match r {
        Ok(sig) => {
            with_mut(|s| {
                s.signed_today += 1;
                if let CoinStep::Redeem { n, link, .. } = &p.step {
                    if let Some(x) = s.redeems.get_mut(n) {
                        x.state = RedeemState::Signed { link: *link };
                        x.signature = sig.clone();
                    }
                }
                s.clear_wait(&key);
            });
            format!("coins: {label} signed and sent ({sig})")
        }
        Err(e) => {
            // a link not signed was undone; a redemption goes back to Requested
            if let CoinStep::Redeem { n, .. } = &p.step {
                redeem::apply_undone(*n);
            }
            if e.contains(crate::solsend::SIMULATION_REFUSED) && !e.contains("insufficient_funds") {
                // a refusal is the step's to wait out (no line error, no alert)
                let mut l = io.sol_line_of(LineId::Crank);
                l.line.error = before;
                io.sol_line_save_of(LineId::Crank, &l);
                with_mut(|s| {
                    s.wait(key, e.clone(), now);
                    if let CoinStep::Surplus { coin } = &p.step {
                        if let Some(c) = s.coins.get_mut(coin) {
                            c.surplus_tries = c.surplus_tries.saturating_add(1);
                        }
                    }
                });
                format!("coins: {label} is refused by simulation, nothing signed: {e}")
            } else {
                // review F10: a short wait for this step (the line not ready, the chain unreadable), not a spin every pass
                with_mut(|s| {
                    s.wait(key, e.clone(), now);
                    if let Some(w) = s.waits.iter_mut().find(|w| w.key == p.step.key()) {
                        w.retry_at = now + 300;
                    }
                });
                format!("coins: {label} waits: {e}")
            }
        }
    }
}

/// What the crank line reported of the coins' steps, and what the links decided: books effects applied once, redemptions and waits updated.
pub fn settle<I: DeskIo>(io: &mut I, now: u64, log: &mut Vec<String>) {
    // 1. the line's verdicts on our steps
    let mut l = io.sol_line_of(LineId::Crank);
    let mine = l.take(LineOwner::Coin);
    if !mine.is_empty() {
        io.sol_line_save_of(LineId::Crank, &l);
    }
    for o in mine {
        let LineStep::Coin(cs) = &o.step else { continue };
        match o.ok {
            Some(true) => {
                if let Some(id) = cs.link() {
                    desk::with_mut(|s| s.link.landed(&mut s.books, id, &o.signature, now));
                }
                grad::apply_landed(cs, now);
                fees::apply_claim(cs);
                with_mut(|s| {
                    s.clear_wait(&cs.key());
                    s.landed(cs.kind(), now, cs.label());
                });
                log.push(format!("coins: {} landed ({})", cs.label(), o.signature));
            }
            Some(false) => {
                if let Some(id) = cs.link() {
                    desk::with_mut(|s| {
                        let crate::chainlink::driver::DeskStateRef { books, link } = crate::chainlink::driver::DeskStateRef::of(s);
                        link.undo(books, id, Phase::Failed, &format!("landed and failed on chain ({})", o.signature), now);
                    });
                }
                with_mut(|s| s.wait(cs.key(), format!("FAILED on chain ({})", o.signature), now));
                log.push(format!("coins: {} FAILED on chain ({}): tried again in {} h", cs.label(), o.signature, WAIT_RETRY_SECS / 3_600));
            }
            None => {
                if let Some(id) = cs.link() {
                    desk::with_mut(|s| {
                        let crate::chainlink::driver::DeskStateRef { books, link } = crate::chainlink::driver::DeskStateRef::of(s);
                        link.undo(books, id, Phase::Undone, "proven never to land (its nonce burned or its blockhash expired)", now);
                    });
                }
                log.push(format!("coins: {} never landed: checked again at its next scan", cs.label()));
            }
        }
    }
    // 2. the receipts: a link decided (by the line's verdict above, or by the desk's own recovery) applies its effect once
    let receipts: Vec<Receipt> = with(|s| s.receipts.clone());
    for r in receipts {
        match link::phase_of(r.link) {
            Some(Phase::Landed) => {
                match &r.step {
                    CoinStep::CashFees { .. } => fees::apply_cash(&r),
                    CoinStep::Redeem { n, coin, amount, payout, .. } => {
                        let sig = desk::with(|s| s.link.get(r.link).map(|l| l.signature.clone()).unwrap_or_default());
                        redeem::apply_landed(*n, coin, *amount, *payout, &sig);
                    }
                    _ => {}
                }
                with_mut(|s| s.receipts.retain(|x| x.link != r.link));
                log.push(format!("coins: link {} applied ({})", r.link, r.step.label()));
            }
            Some(Phase::Failed) | Some(Phase::Undone) | None => {
                if let CoinStep::Redeem { n, .. } = &r.step {
                    redeem::apply_undone(*n);
                }
                with_mut(|s| s.receipts.retain(|x| x.link != r.link));
            }
            _ => {}
        }
    }
    let _ = solhome::LineId::Crank;
}
