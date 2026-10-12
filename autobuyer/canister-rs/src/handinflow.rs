//! The sequencer's pieces for permissionless offer hand-ins (docs/v6/offer-handins.md §6), over `FrogIo`: read the boards at
//! the tick's pin, offer the cheapest hand-in to a wanted frog, and send the claim or the slash as the one EVM transaction.
//! No network of its own; the decisions are `handin.rs`'s, the judging `frogflow::accept`'s. Tested against a fake world in
//! `tests/handins.rs`.

use crate::frogbuy::{self, CONFIRMATIONS, MINT_MARGIN_SECS, PURCHASE_LATENCY_SECS};
use crate::frogflow::{self, FStep, FrogIo, FrogState};
use crate::handin::{self, Candidate, HandinState, Verdict};
use crate::outflow::Flow;
use crate::custodycap;

fn h(a: &[u8; 20]) -> String {
    format!("0x{}", hex::encode(a))
}

// ---- reads -----------------------------------------------------------------------------------------------------------------------

/// Every board, at this tick's pin: identity once (its wrapper is ours), the params once, then the open ids and the
/// submissions not seen before, bounded per tick.
pub async fn read_boards<I: FrogIo>(io: &mut I, hs: &mut HandinState) -> Vec<String> {
    let mut lines = Vec::new();
    let boards: Vec<[u8; 20]> = hs.boards.iter().filter(|b| b.refused.is_none()).map(|b| b.board).collect();
    for a in boards {
        match read_board(io, hs, a).await {
            Ok(Some(s)) => lines.push(s),
            Ok(None) => {}
            Err(e) => lines.push(format!("board {}: not read: {e}", h(&a))),
        }
    }
    lines
}

async fn read_board<I: FrogIo>(io: &mut I, hs: &mut HandinState, a: [u8; 20]) -> Result<Option<String>, String> {
    if hs.board(&a).and_then(|b| b.params).is_none() {
        let w = frogflow::addr(io, a, handin::WRAPPER).await?;
        if w != io.wrapper() {
            let why = format!("its wrapper is {}, not ours", h(&w));
            if let Some(b) = hs.board_mut(&a) {
                b.refused = Some(why.clone());
            }
            return Ok(Some(format!("board {}: never read: {why}", h(&a))));
        }
        let p = handin::decode_params(&frogflow::word(io, a, &handin::PARAMS).await?)?;
        if let Some(b) = hs.board_mut(&a) {
            b.params = Some(p);
        }
    }
    let ids = handin::decode_ids(&frogflow::word(io, a, &handin::open_ids(0, handin::MAX_OPEN_READ)).await?)?;
    let new_ids: Vec<u64> = ids.iter().copied().filter(|id| hs.seen(&a, *id).is_none()).take(handin::MAX_SUBMISSIONS_READ_PER_TICK).collect();
    let mut new = Vec::with_capacity(new_ids.len());
    for id in &new_ids {
        new.push(handin::decode_submission(&frogflow::word(io, a, &handin::call_uint(handin::SUBMISSION_OF, *id as u128)).await?)?);
    }
    let now = io.now();
    hs.reconcile(a, &ids, new, &new_ids, now);
    Ok(if new_ids.is_empty() { None } else { Some(format!("board {}: {} open, {} new", h(&a), ids.len(), new_ids.len())) })
}

// ---- the choice ----------------------------------------------------------------------------------------------------------------------

/// The cheapest hand-in a wanted frog may try, for at most `cap_wei`: only when one exists are the rate and the clock read.
pub async fn candidate<I: FrogIo>(io: &mut I, hs: &HandinState, cap_wei: u128) -> Result<Option<Candidate>, String> {
    if !hs.any_candidate() {
        return Ok(None);
    }
    let rate = io.rate().await?;
    let cap_sats = frogbuy::sats_for_wei(cap_wei, rate)?;
    let clock = io.clock().await?;
    Ok(hs.cheapest(cap_sats, &clock, PURCHASE_LATENCY_SECS + MINT_MARGIN_SECS + crate::policy::CAP_MARGIN_SECS))
}

// ---- the EVM steps ---------------------------------------------------------------------------------------------------------------------

/// The fifth group of `frogflow::evm_act`: claims for held frogs, then slashes the board allows. One that cannot go now is
/// reported in `errs` and does not hold up the others.
pub async fn evm_act<I: FrogIo>(io: &mut I, st: &mut FrogState, hs: &mut HandinState, errs: &mut Vec<String>) -> Option<String> {
    let now = io.now();
    hs.settle_failed(&st.frogs, now);
    for (board, id, frog) in hs.claims_due(&st.frogs) {
        match claim_one(io, st, hs, board, id, frog).await {
            Ok(Some(s)) => return Some(s),
            Ok(None) => {}
            Err(e) => errs.push(format!("hand-in {id}: {e}")),
        }
    }
    for (board, id) in hs.slashes_due() {
        match slash_one(io, st, board, id).await {
            Ok(Some(s)) => return Some(s),
            Ok(None) => {}
            Err(e) => errs.push(format!("hand-in {id}: {e}")),
        }
    }
    None
}

/// `claim(id)`, only after a fresh read shows the purchase six deep with its block still the block (H3: the reward is value
/// out, against the day's limit).
async fn claim_one<I: FrogIo>(io: &mut I, st: &mut FrogState, hs: &HandinState, board: [u8; 20], id: u64, frog: u64)
    -> Result<Option<String>, String> {
    let buy = st.frogs.iter().find(|f| f.id == frog).and_then(|f| f.bought().cloned()).ok_or("a bought hand-in with nothing bought")?;
    match frogflow::depth(io, buy.outpoint.txid).await? {
        Some(d) if d >= CONFIRMATIONS => {}
        Some(d) => return Err(format!("only {d} deep now (a reorganisation): the claim waits for six")),
        None => return Err("no longer in a block (a reorganisation): the claim waits".into()),
    }
    let reward = hs.board(&board).and_then(|b| b.params).map_or(0, |p| p.reward_wei);
    let collection = custodycap::btc_key(&io.wrapper());
    let key = format!("claim:handin:{}:{id}", hex::encode(board));
    io.outflow_check(&key, &collection, Flow::Value, reward)?;
    let s = frogflow::send(io, st, FStep::Claim { board, id }, board, handin::claim(id)).await?;
    io.outflow_record(&key, &collection, Flow::Value, reward);
    Ok(Some(format!("hand-in {id}: {s}")))
}

/// `slash(id)`, once the board says the delay has passed on both clocks.
async fn slash_one<I: FrogIo>(io: &mut I, st: &mut FrogState, board: [u8; 20], id: u64) -> Result<Option<String>, String> {
    let ok = crate::abi::bool_at(&frogflow::word(io, board, &handin::call_uint(handin::SLASHABLE, id as u128)).await?, 0, 0)?;
    if !ok {
        return Ok(None);
    }
    frogflow::send(io, st, FStep::Slash { board, id }, board, handin::slash(id)).await.map(|s| Some(format!("hand-in {id}: {s}")))
}

/// A mined claim or slash: on success the verdict; on a revert the board's own word (adopted), never a blind retry.
pub async fn mined<I: FrogIo>(io: &mut I, hs: &mut HandinState, step: &FStep, success: bool) -> String {
    let (board, id, done) = match step {
        FStep::Claim { board, id } => (*board, *id, Verdict::Claimed),
        FStep::Slash { board, id } => (*board, *id, Verdict::Slashed),
        _ => return "not a hand-in step".into(),
    };
    let now = io.now();
    if success {
        hs.set_verdict(board, id, done.clone(), now);
        return match done {
            Verdict::Claimed => "claimed: the board credited the submitter its bond and the reward".into(),
            _ => "slashed: its bond went to the reward pool".into(),
        };
    }
    match frogflow::word(io, board, &handin::call_uint(handin::SUBMISSION_OF, id as u128)).await.and_then(|b| handin::decode_submission(&b)) {
        Ok(s) => match s.status {
            handin::STATUS_CLAIMED => {
                hs.set_verdict(board, id, Verdict::Claimed, now);
                "reverted, but the board says claimed: adopted".into()
            }
            handin::STATUS_SLASHED => {
                hs.set_verdict(board, id, Verdict::Slashed, now);
                "reverted, but the board says slashed: adopted".into()
            }
            handin::STATUS_REFUNDED => {
                hs.set_verdict(board, id, Verdict::Gone { why: "refunded on the board before our transaction".into() }, now);
                "reverted: the submitter took the bond back first; nothing is owed".into()
            }
            _ => "reverted: read again next tick (an estimate guards the next attempt)".into(),
        },
        Err(e) => format!("reverted, and the board is not read ({e}): next tick"),
    }
}
