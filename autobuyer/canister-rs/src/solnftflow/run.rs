//! One pass of the Solana NFT flow (docs/v9/solana-nfts-canister.md §1): the money line's outcomes of this flow taken first (the advance
//! link landed or undone, a delivery recorded), the operator's changes past their delay applied, each collection's new evidence read and
//! its floor posted, a landed purchase's custody booked; then, while nothing of ours is out on the money line, ONE new step: a delivery
//! first (redeemers waiting), else a purchase, else the M2 lookup table if an M2 ask waited for it ([`table::pass`]).
use super::*;
use crate::chainlink::driver::DeskStateRef;
use crate::chainlink::ledger::Phase;
use crate::desk::state as desk;
use crate::solhome::{LineId, LineOwner, LineStep};

pub async fn tick<I: SolNftIo>(io: &mut I, now: u64) -> Vec<String> {
    let mut log = vec![];
    if io.sol_home().is_none() {
        return log;
    }
    let colls: Vec<u16> = listings().into_iter().map(|(id, ..)| id).collect();
    if colls.is_empty() {
        return log;
    }
    with_mut(|s| {
        s.roll_day(now);
        for c in &colls {
            let c = s.cols.entry(*c).or_default();
            if let Some(p) = c.pending.clone().filter(|p| now >= p.at) {
                c.premium_bps = p.premium_bps;
                c.m2 = p.m2;
                c.pending = None;
            }
        }
    });
    log.extend(settle(io, now).await);
    for c in &colls {
        match floor::pass(io, *c, now).await {
            Ok(Some(t)) => log.push(t),
            Ok(None) => {}
            Err(e) => log.push(format!("solnft floor {c}: waits: {e}")),
        }
        if let Some(t) = buy::arrived(io, *c, now).await {
            log.push(t);
        }
    }
    // noted-sent deliveries are recorded whatever else holds (no signature)
    for c in &colls {
        log.extend(deliver::pass(io, *c, now, false).await);
    }
    if io.paused() || with(|s| s.off) {
        return log;
    }
    if with(|s| s.out.is_some()) || io.sol_line_of(LineId::Money).pending.as_ref().is_some_and(|p| p.step.owner() == LineOwner::SolNft) {
        return log;
    }
    if with(|s| s.signed_today >= STEPS_PER_DAY) {
        log.push(format!("solnft: waits: {STEPS_PER_DAY} steps today already (the day's budget)"));
        return log;
    }
    for c in &colls {
        log.extend(deliver::pass(io, *c, now, true).await);
        if with(|s| s.out.is_some()) {
            return log;
        }
    }
    table::unwant();
    for c in &colls {
        match buy::plan_and_sign(io, *c, now).await {
            Ok(Some(t)) => {
                log.push(t);
                if with(|s| s.out.is_some()) {
                    return log;
                }
            }
            Ok(None) => {}
            Err(e) => log.push(format!("solnft buy {c}: {e}")),
        }
    }
    // nothing else signed: the M2 lookup table, if an M2 ask waited for it this pass
    if let Some(t) = table::pass(io, now).await {
        log.push(t);
    }
    log
}

/// The money line's outcomes of this flow: a purchase's link landed (its custody checked next: [`buy::arrived`]) or undone (failed on
/// chain or never landed: nothing moved, the advance is back in the books); a delivery's ([`deliver::landed`]).
pub async fn settle<I: SolNftIo>(io: &mut I, now: u64) -> Vec<String> {
    let mut log = vec![];
    let mut l = io.sol_line_of(LineId::Money);
    let mine = l.take(LineOwner::SolNft);
    if !mine.is_empty() {
        io.sol_line_save_of(LineId::Money, &l);
    }
    for o in mine {
        let LineStep::SolNft(step) = &o.step else { continue };
        with_mut(|s| {
            if s.out.as_ref().is_some_and(|p| p.intent == o.intent) {
                s.out = None;
            }
        });
        match step {
            SolNftStep::Buy { coll, asset, cost, link, .. } => match o.ok {
                Some(true) => {
                    desk::with_mut(|s| s.link.landed(&mut s.books, *link, &o.signature, now));
                    with_mut(|s| s.cols.entry(*coll).or_default().arrival = Some(Arrival { asset: *asset, signature: o.signature.clone(), cost: *cost, tries: 0 }));
                    log.push(format!("{} landed ({}): its custody is checked at finalized", step.label(), o.signature));
                }
                ok => {
                    let (phase, why) = match ok {
                        Some(false) => (Phase::Failed, format!("landed and failed on chain ({})", o.signature)),
                        _ => (Phase::Undone, "proven never to land (its nonce burned or its blockhash expired)".to_string()),
                    };
                    desk::with_mut(|s| {
                        let DeskStateRef { books, link: lk } = DeskStateRef::of(s);
                        lk.undo(books, *link, phase, &why, now);
                    });
                    log.push(format!("{}: {why}: the advance is undone in the books (nothing left M[{coll}])", step.label()));
                }
            },
            // audit C1-I6: booked in the same message as the outcome taken (no await between them)
            SolNftStep::Deliver { coll, ticket, .. } => log.push(deliver::landed(*coll, *ticket, o.ok, &o.signature)),
            SolNftStep::Table { .. } => log.push(table::landed(o.ok, &o.signature)),
        }
    }
    // a step this flow saved as out that the line no longer holds nor reports (taken by an operator's clear): free again
    let held = io.sol_line_of(LineId::Money).pending.as_ref().is_some_and(|p| p.step.owner() == LineOwner::SolNft);
    if !held && with(|s| s.out.is_some()) && !io.sol_line_of(LineId::Money).outcomes.iter().any(|o| o.step.owner() == LineOwner::SolNft) {
        with_mut(|s| s.out = None);
    }
    log
}
