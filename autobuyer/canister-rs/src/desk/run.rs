//! The desk's tick (§6.1): settle what the desk line reported, find finalized deposits (ONE batched `getMultipleAccounts` for up to
//! 100 open intents), act on at most one per tick per line (the desk line carries one transaction at a time), expire, clean up, and
//! reconcile every [`RECONCILE_EVERY`] seconds.
//!
//! Every money-moving step is a [`chainlink`](crate::chainlink) link: booked and recorded before it is signed, gated by `pre_sign`,
//! final only when the line reports it finalized. The intent machine below only decides WHICH operation a deposit gets.
//!
//! Which refusals wait and which refund (decision): a refusal that may pass on its own (a stale floor, a paused poster, an hourly
//! bucket, a cap, no item yet to redeem) waits, retried every tick, until [`RETRY_WINDOW_SECS`] after the deposit was seen; the
//! user's own bound (slippage), a paused collection, a halted collection, a bad destination, a deposit too small for its fees go back
//! at once. Three failed attempts (landed failed, or proven never to land) also send the deposit back.
use super::build::{self, Addrs};
use super::intent::*;
use super::state::{with, with_mut};
use crate::books::{BooksError, ONE};
use crate::chainlink::driver::{self, DeskIo, SignError};
use crate::chainlink::ledger::{Effects, LinkError, OpKind, Phase, Scope, Spec};
use crate::solhome::{self, LineId, LineOwner};
use crate::solix2::decode::TokenAccount;
use crate::solrpc::{Account, MAX_KEYS};
use crate::soltx::{associated_token_address, Pubkey, SYSTEM_PROGRAM};

/// Seconds between reconciliations of the books with the chain.
pub const RECONCILE_EVERY: u64 = 600;
/// Candidates tried in one tick before it gives up (each costs reads).
const TRIES_PER_TICK: usize = 5;

/// Whether a books refusal may pass by itself.
pub fn transient(e: &BooksError) -> bool {
    matches!(e, BooksError::Stale | BooksError::Paused | BooksError::OverCap | BooksError::CapStale | BooksError::OverBacked | BooksError::NotKeeper
        | BooksError::NoItem | BooksError::ArrivalRateLimited | BooksError::NotEnoughBacked)
}

enum Exec {
    /// A transaction is out.
    Started,
    /// Not this intent now: try the next.
    Skip,
    /// The desk line cannot take anything now.
    Stop,
}

fn get(id: u64) -> Option<Intent> {
    with(|s| s.intents.get(&id).cloned())
}

fn edit<R>(id: u64, f: impl FnOnce(&mut Intent) -> R) -> Option<R> {
    with_mut(|s| s.intents.get_mut(&id).map(f))
}

/// One pass of the desk. `now` in unix seconds.
pub async fn tick<I: DeskIo>(io: &mut I, now: u64) -> Vec<String> {
    let mut log = vec![];
    let idle = with(|s| s.intents.is_empty() && !s.link.any_open() && s.books.collections.is_empty());
    if idle {
        return log;
    }
    log.extend(driver::settle(io, now).await);
    // audit B1-06: what line outcomes owed the books while a link was open, for every collection now free of one (the ledger
    // applies it at each decision too; this pass is the backstop), before any new link is begun
    with_mut(|s| {
        let colls: Vec<u16> = s.link.owed.as_ref().map(|o| o.keys().copied().collect()).unwrap_or_default();
        for c in colls {
            s.link.apply_owed(&mut s.books, c, now);
        }
    });
    apply_outcomes(now, &mut log);
    detect(io, now, &mut log).await;
    housekeeping(now, &mut log);
    execute_next(io, now, &mut log).await;
    if with(|s| !s.books.collections.is_empty() && now.saturating_sub(s.last_reconcile) >= RECONCILE_EVERY) {
        let r = driver::reconcile(io, now).await;
        if !r.findings.is_empty() || !r.halted.is_empty() {
            log.push(format!("desk: reconciliation found {} divergence(s), stopped {:?}", r.findings.len(), r.halted));
        }
    }
    log
}

// ---- links back to intents ----------------------------------------------------------------------------------------------------------

/// Every intent with a transaction out is looked at against its link: landed → its next state; undone or failed → back to Funded.
pub fn apply_outcomes(now: u64, log: &mut Vec<String>) {
    // a close (audit B1-04) is out while the intent keeps its state (Done, Refunded)
    let out: Vec<(u64, Op)> = with(|s| s.intents.values().filter(|i| i.state == State::Executing || i.op.is_some_and(|o| o.purpose == Purpose::Close))
        .filter_map(|i| i.op.map(|o| (i.id, o))).collect());
    for (id, op) in out {
        let Some(link) = with(|s| s.link.get(op.link).cloned()) else {
            if op.purpose == Purpose::Close {
                edit(id, |i| close_failed(i, "its link is gone"));
                continue;
            }
            edit(id, |i| {
                i.state = State::Funded;
                i.op = None;
                i.note = "its link is gone".into();
            });
            continue;
        };
        match link.phase {
            Phase::Recorded | Phase::Signed => {}
            Phase::Stuck => {
                edit(id, |i| {
                    i.attention = true;
                    i.note = format!("transaction {} is stuck: {}", link.memo, link.reason);
                });
            }
            Phase::Landed if op.purpose == Purpose::Close => {
                edit(id, |i| {
                    i.push_tx(TxRec { link: link.id, op: "Close".into(), memo: link.memo.clone(), signature: link.signature.clone(), result: "landed".into(), at: now });
                    i.op = None;
                    i.close_due = false;
                    i.close_tries = 0;
                    i.note = "the deposit account is closed".into();
                });
                log.push(format!("desk: intent {id}: its token account is closed ({})", link.signature));
            }
            Phase::Landed => {
                edit(id, |i| {
                    i.push_tx(TxRec { link: link.id, op: format!("{:?}", op.purpose), memo: link.memo.clone(), signature: link.signature.clone(),
                        result: "landed".into(), at: now });
                    i.op = None;
                    i.skipped_at = None;
                    i.deposit = 0;
                    i.attempts = 0;
                    i.funded_at = None;
                    i.updated_at = now;
                    match op.purpose {
                        Purpose::Refund => {
                            i.state = State::Refunded;
                            i.refund_only = false;
                            i.must_refund = None;
                            i.refund_attempts = 0;
                            i.note = "the deposit was returned".into();
                            // audit B1-04: a token refund no longer closes `A`'s account; the close is the next, separate step
                            i.close_due |= !i.kind.is_sol();
                        }
                        Purpose::Close => {}
                        Purpose::Execute => {
                            i.state = State::Done;
                            i.done_ops += 1;
                            i.expires_at = i.expires_at.max(now + TTL_SECS);
                            match i.kind {
                                Kind::Mint { .. } => i.minted_units += link.aux,
                                Kind::Burn { .. } => {
                                    i.paid_lamports += link.aux;
                                    // audit B1-04: the burn left `A`'s emptied account open; the close is the next, separate step
                                    i.close_due = true;
                                }
                                Kind::RedeemNft { .. } => i.ticket = Some(link.aux),
                                Kind::RedeemCoin { .. } => {}
                            }
                            i.note = "done".into();
                        }
                    }
                });
                log.push(format!("desk: intent {id}: {:?} landed ({})", op.purpose, link.signature));
            }
            Phase::Failed | Phase::Undone if op.purpose == Purpose::Close => {
                edit(id, |i| {
                    i.push_tx(TxRec { link: link.id, op: "Close".into(), memo: link.memo.clone(), signature: link.signature.clone(),
                        result: format!("{:?}: {}", link.phase, link.reason), at: now });
                    close_failed(i, "the close did not land");
                });
                log.push(format!("desk: intent {id}: the close did not land ({:?})", link.phase));
            }
            Phase::Failed | Phase::Undone => {
                edit(id, |i| {
                    i.push_tx(TxRec { link: link.id, op: format!("{:?}", op.purpose), memo: link.memo.clone(), signature: link.signature.clone(),
                        result: format!("{:?}: {}", link.phase, link.reason), at: now });
                    i.op = None;
                    i.state = State::Funded;
                    i.updated_at = now;
                    match op.purpose {
                        Purpose::Execute => {
                            i.attempts += 1;
                            if i.attempts >= MAX_ATTEMPTS && i.must_refund.is_none() {
                                i.must_refund = Some(format!("{} attempts did not land", i.attempts));
                            }
                        }
                        Purpose::Refund => {
                            i.refund_attempts += 1;
                            if i.refund_attempts >= MAX_REFUND_ATTEMPTS {
                                i.attention = true;
                                i.note = "the refund did not land: an operator must look".into();
                            }
                        }
                        Purpose::Close => {}
                    }
                });
                log.push(format!("desk: intent {id}: {:?} did not land ({:?}): back to funded", op.purpose, link.phase));
            }
        }
    }
}

/// Audit B1-04: a close that found `A`'s account not empty, or did not land. At [`MAX_CLOSE_TRIES`] the account is left open: its rent
/// stays at `A` (the dust in it is a stranger's), and `K_main` no longer creates a refund's destination for this intent (`refund_op`).
fn close_failed(i: &mut Intent, why: &str) {
    i.op = None;
    i.close_tries += 1;
    if i.close_tries >= MAX_CLOSE_TRIES {
        i.close_due = false;
        i.note = format!("{why}: the deposit account is left open, its rent stays there");
    } else {
        i.note = format!("{why}: the close is tried again");
    }
}

// ---- deposits -----------------------------------------------------------------------------------------------------------------------

fn amount_of(i: &Intent, a: &Option<Account>, mint: &Pubkey) -> u64 {
    let Some(a) = a else { return 0 };
    if i.kind.is_sol() {
        return if a.owner == SYSTEM_PROGRAM && a.data.is_empty() { a.lamports } else { 0 };
    }
    match TokenAccount::decode(&a.owner, &a.data) {
        Ok(t) if t.owner == i.address && t.mint == *mint => t.amount,
        _ => 0,
    }
}

/// The mint an intent's token deposit is of (the wX of its collection, or the coin).
fn deposit_mint(i: &Intent) -> Option<Pubkey> {
    match &i.kind {
        Kind::Mint { .. } => None,
        Kind::RedeemCoin { coin, .. } => Some(*coin),
        k => with(|s| s.books.get(k.coll()).ok().map(|c| c.wx_mint)),
    }
}

/// One batched read of every watched address, then each intent's state from what finalized shows. Audit B1-05: intents that never saw a
/// deposit and expired more than [`UNFUNDED_GRACE_SECS`] ago are not in the batch: ONE further read per tick covers up to
/// [`MAX_KEYS`] of them in turn (`DeskState::rotation`), so free openings cannot grow the reads, and a late deposit is still found.
async fn detect<I: DeskIo>(io: &mut I, now: u64, log: &mut Vec<String>) {
    let key_of = |i: &Intent| -> Option<(u64, Pubkey, Option<Pubkey>)> {
        let key = if i.kind.is_sol() { i.address } else { i.token_account? };
        Some((i.id, key, deposit_mint(i)))
    };
    let (watch, rotation) = with(|s| {
        let watch: Vec<_> = s.intents.values().filter(|i| i.watched(now) && !i.rotated(now)).filter_map(key_of).collect();
        let rotated: Vec<&Intent> = s.intents.values().filter(|i| i.rotated(now)).collect();
        let rotation: Vec<_> = rotated.iter().filter(|i| i.id > s.rotation).chain(rotated.iter().filter(|i| i.id <= s.rotation))
            .filter_map(|i| key_of(i)).take(MAX_KEYS).collect();
        (watch, rotation)
    });
    for chunk in watch.chunks(MAX_KEYS) {
        if !read_deposits(io, chunk, now, log).await {
            return;
        }
    }
    if let Some((last, _, _)) = rotation.last() {
        if read_deposits(io, &rotation, now, log).await {
            with_mut(|s| s.rotation = *last);
        }
    }
}

/// One `getMultipleAccounts` of up to [`MAX_KEYS`] deposit addresses, and what it shows. `false`: not readable.
async fn read_deposits<I: DeskIo>(io: &mut I, chunk: &[(u64, Pubkey, Option<Pubkey>)], now: u64, log: &mut Vec<String>) -> bool {
    let keys: Vec<Pubkey> = chunk.iter().map(|(_, k, _)| *k).collect();
    let accs = match io.sol_accounts(&keys, 165).await {
        Ok(a) => a,
        Err(e) => {
            log.push(format!("desk: waits: the deposits are not readable: {e}"));
            return false;
        }
    };
    for ((id, _, mint), acc) in chunk.iter().zip(accs) {
        edit(*id, |i| {
            let amount = amount_of(i, &acc, &mint.unwrap_or_default());
            if observe(i, amount, now) {
                log.push(format!("desk: intent {}: a deposit of {amount} is there ({})", i.id, i.note));
            }
        });
    }
    true
}

/// The intent's state from a finalized deposit of `amount` (no transaction out). `true`: it became funded now.
pub fn observe(i: &mut Intent, amount: u64, now: u64) -> bool {
    if i.state == State::Executing || i.op.is_some() {
        return false;
    }
    i.deposit = amount;
    if amount == 0 {
        if i.state == State::Funded {
            i.state = State::Open;
            i.funded_at = None;
            i.note = "the deposit is gone".into();
        }
        return false;
    }
    let served = matches!(i.kind, Kind::RedeemNft { .. }) && i.done_ops > 0;
    let was_funded = i.state == State::Funded;
    if i.expired(now) || served || i.must_refund.is_some() {
        if amount >= i.kind.dust() {
            i.refund_only = true;
            i.state = State::Funded;
            i.funded_at.get_or_insert(now);
            i.note = if served { "a deposit beyond the redemption goes back".into() } else if i.expired(now) { "a deposit after expiry goes back".into() } else { "the deposit goes back".into() };
            return !was_funded;
        }
        return false;
    }
    if amount >= i.kind.min_deposit() {
        i.state = State::Funded;
        i.funded_at.get_or_insert(now);
        i.expires_at = i.expires_at.max(now + TTL_SECS);
        i.note = format!("funded: {amount}");
        return !was_funded;
    }
    i.note = format!("waiting: {amount} is below the minimum {}", i.kind.min_deposit());
    false
}

/// Untouched intents expire; long-finished ones leave the record.
fn housekeeping(now: u64, log: &mut Vec<String>) {
    with_mut(|s| {
        for i in s.intents.values_mut() {
            if i.state == State::Open && i.expired(now) && i.deposit < i.kind.dust() && i.done_ops == 0 {
                i.state = State::Expired;
                i.note = "no deposit in 24 h".into();
                log.push(format!("desk: intent {} expired", i.id));
            }
        }
        s.intents.retain(|_, i| !i.droppable(now));
        let hour_ago = now.saturating_sub(3_600);
        s.opens.retain(|o| o.at > hour_ago);
    });
}

// ---- acting on a funded intent -----------------------------------------------------------------------------------------------------

async fn execute_next<I: DeskIo>(io: &mut I, now: u64, log: &mut Vec<String>) {
    let mut tried: Vec<u64> = vec![];
    for _ in 0..TRIES_PER_TICK {
        let line = io.sol_line_of(LineId::Desk);
        if line.busy() {
            return;
        }
        // Audit B1-03: the never-skipped first, in (funded_at, id) order, then the least recently skipped. Before, the order was
        // (funded_at, id) alone: five intents waiting on a refusal that may pass (one collection's stale floor, its bucket, its cap)
        // were the five tried every tick, and every other collection's intents were never reached. A skipped intent now goes to the
        // back, so each tick reaches new ones; among intents that can go, the FIFO is kept. A close due (audit B1-04) comes after the
        // funded intents.
        let next = with(|s| {
            let mut c: Vec<&Intent> = s.intents.values().filter(|i| !tried.contains(&i.id) && !i.attention && i.op.is_none()
                && (i.state == State::Funded || (i.close_due && i.state != State::Executing))).collect();
            c.sort_by_key(|i| (i.skipped_at.unwrap_or(0), i.state != State::Funded, i.funded_at.unwrap_or(u64::MAX), i.id));
            c.first().map(|i| i.id)
        });
        let Some(id) = next else {
            solhome::release(io, LineOwner::Desk);
            return;
        };
        tried.push(id);
        match execute(io, id, now, log).await {
            Exec::Started => return,
            Exec::Skip => {
                edit(id, |i| i.skipped_at = Some(now));
            }
            Exec::Stop => return,
        }
    }
}

fn set_refund(id: u64, why: String) {
    edit(id, |i| {
        if i.must_refund.is_none() {
            i.note = format!("the deposit goes back: {why}");
            i.must_refund = Some(why);
        }
    });
}

fn set_note(id: u64, why: String) {
    edit(id, |i| i.note = why);
}

async fn execute<I: DeskIo>(io: &mut I, id: u64, now: u64, log: &mut Vec<String>) -> Exec {
    let Some(it) = get(id) else { return Exec::Skip };
    if it.state != State::Funded {
        return exec_close(io, &it, now, log).await;
    }
    let refund = it.refund_only || it.must_refund.is_some();
    if !refund && with(|s| s.desk_paused) {
        set_note(id, "the desk is paused".into());
        return Exec::Skip;
    }
    if !refund {
        if let Some(f) = it.funded_at {
            if now >= f.saturating_add(RETRY_WINDOW_SECS) {
                let why = format!("waited {} s: {}", RETRY_WINDOW_SECS, it.note);
                set_refund(id, why);
                return Box::pin(execute(io, id, now, log)).await;
            }
        }
    }
    let r = if refund {
        refund_op(io, &it, now, log).await
    } else {
        match &it.kind {
            Kind::Mint { .. } => exec_mint(io, &it, now, log).await,
            Kind::Burn { .. } => exec_burn(io, &it, now, log).await,
            Kind::RedeemNft { .. } => exec_redeem(io, &it, now, log).await,
            Kind::RedeemCoin { .. } => {
                if with(|s| s.coin_redeem) {
                    set_note(id, "coin redemption waits for its executor".into());
                } else {
                    set_note(id, "coin redemption is not enabled".into());
                }
                Exec::Skip
            }
        }
    };
    r
}

/// Begin a link for `it` (booked and recorded, nothing signed yet).
fn begin_link(it: &Intent, kind: OpKind, scope: Scope, now: u64, f: impl FnOnce(&mut crate::books::Books) -> Result<(Effects, u64), BooksError>)
    -> Result<u64, LinkError> {
    let seq = it.seq;
    edit(it.id, |i| i.seq += 1);
    let coll = (!matches!(kind, OpKind::Refund | OpKind::Close)).then(|| it.kind.coll());
    let spec = Spec { key: format!("i{}:{}:{seq}", it.id, kind.tag()), kind, coll, intent: Some(it.id), scope };
    with_mut(|s| s.link.begin(&mut s.books, spec, now, f)).map(|(id, _)| id)
}

/// A refusal of `begin_link`: wait, or send the deposit back.
fn refused(it: &Intent, e: LinkError) -> Exec {
    match e {
        LinkError::Busy(_) => {
            set_note(it.id, "the collection has an unfinished operation".into());
        }
        LinkError::Halted(c) => set_refund(it.id, format!("collection {c} is halted by an incident")),
        LinkError::Books(b) if transient(&b) => set_note(it.id, format!("waiting: {b}")),
        LinkError::Books(b) => set_refund(it.id, format!("{b}")),
    }
    Exec::Skip
}

/// Sign the recorded link and put the intent in Executing.
async fn send<I: DeskIo>(io: &mut I, it: &Intent, link: u64, purpose: Purpose, deposit: u64, ixs: Vec<crate::soltx::Instruction>,
    keys: Vec<crate::solkey::KeyPath>, now: u64, log: &mut Vec<String>) -> Exec {
    match driver::sign_recorded(io, link, ixs, keys, now).await {
        Ok(sig) => {
            let memo = with(|s| s.link.get(link).map(|l| l.memo.clone())).unwrap_or_default();
            edit(it.id, |i| {
                // a close is out while the intent keeps its state (audit B1-04)
                if purpose != Purpose::Close {
                    i.state = State::Executing;
                }
                i.skipped_at = None;
                i.op = Some(Op { link, purpose, deposit });
                i.updated_at = now;
                i.note = format!("{purpose:?} signed ({memo})");
            });
            log.push(format!("desk: intent {}: {:?} signed as {memo} ({sig})", it.id, purpose));
            Exec::Started
        }
        Err(SignError::Refused(e)) if purpose == Purpose::Close => {
            edit(it.id, |i| close_failed(i, &format!("the close was refused: {e}")));
            Exec::Skip
        }
        Err(SignError::Refused(e)) => {
            log.push(format!("desk: intent {}: the gate refused ({e}): collection {} is stopped", it.id, it.kind.coll()));
            set_refund(it.id, format!("the pre-sign gate refused: {e}"));
            Exec::Skip
        }
        Err(SignError::Deferred(e)) => {
            let sim = e.starts_with(crate::solsend::SIMULATION_REFUSED);
            edit(it.id, |i| {
                i.note = format!("waits: {e}");
                if sim && purpose == Purpose::Close {
                    close_failed(i, &format!("the close is refused by simulation: {e}"));
                } else if sim {
                    i.attempts += 1;
                    if i.attempts >= MAX_ATTEMPTS && i.must_refund.is_none() {
                        i.must_refund = Some(format!("the transaction is refused by simulation: {e}"));
                    }
                }
            });
            log.push(format!("desk: intent {}: waits: {e}", it.id));
            if sim { Exec::Skip } else { Exec::Stop }
        }
    }
}

async fn addrs<I: DeskIo>(io: &mut I, it: &Intent, mint: Pubkey) -> Result<Addrs, String> {
    let main = io.sol_me().await?;
    let module = driver::module_address(io, it.kind.coll()).await?;
    Ok(Addrs { main, intent: it.address, module, mint })
}

fn memo_of(link: u64) -> String {
    with(|s| s.link.get(link).map(|l| l.memo.clone())).unwrap_or_default()
}

async fn exec_mint<I: DeskIo>(io: &mut I, it: &Intent, now: u64, log: &mut Vec<String>) -> Exec {
    let Kind::Mint { coll, recipient, min_units } = it.kind else { return Exec::Skip };
    let Ok(mint) = with(|s| s.books.get(coll).map(|c| c.wx_mint)) else {
        set_refund(it.id, "unknown collection".into());
        return Exec::Skip;
    };
    let ata = associated_token_address(&recipient, &mint);
    let a = match addrs(io, it, mint).await {
        Ok(a) => a,
        Err(e) => {
            log.push(format!("desk: waits: {e}"));
            return Exec::Stop;
        }
    };
    let accs = match io.sol_accounts(&[it.address, ata], 165).await {
        Ok(v) => v,
        Err(e) => {
            log.push(format!("desk: waits: {e}"));
            return Exec::Stop;
        }
    };
    let deposit = amount_of(it, &accs[0], &Pubkey::default());
    if deposit < MIN_MINT_LAMPORTS {
        edit(it.id, |i| {
            i.state = State::Open;
            i.deposit = deposit;
        });
        return Exec::Skip;
    }
    // audit B1-08: the recipient's token account is created unless one of this mint is there (a system account anyone planted at the
    // address is no token account: the ATA program creates over a pre-funded address)
    let create = !accs[1].as_ref().is_some_and(|a| TokenAccount::decode(&a.owner, &a.data).is_ok_and(|t| t.mint == mint));
    let rent = if create { driver::rent_of(io, crate::solix2::spl::ACCOUNT_LEN).await.unwrap_or(super::quote::ATA_RENT_LAMPORTS) } else { 0 };
    let Some(to_module) = deposit.checked_sub(FEE_LAMPORTS).and_then(|d| d.checked_sub(rent)).filter(|d| *d > 0) else {
        set_refund(it.id, "the deposit does not cover the fee".into());
        return Exec::Skip;
    };
    let now_i = now as i64;
    let (to_module, min_units) = fit_bucket(coll, to_module, min_units, now_i);
    let link = match begin_link(it, OpKind::Mint, Scope::Module, now, |b| {
        let m = b.mint(coll, to_module, min_units, now_i)?;
        Ok((Effects { mint_units: m.units, lamports_in: to_module, ..Default::default() }, m.units))
    }) {
        Ok(l) => l,
        Err(e) => return refused(it, e),
    };
    let units = with(|s| s.link.get(link).map(|l| l.effects.mint_units)).unwrap_or(0);
    let ixs = build::mint_tx(&a, recipient, units, to_module, FEE_LAMPORTS, create, &memo_of(link));
    send(io, it, link, Purpose::Execute, deposit, ixs, build::mint_keys(it.id, coll), now, log).await
}

/// Audit B1-07 (§6.1 partial fills): a deposit whose wX would overflow the hourly bucket mints what fits now (its bound scaled alike);
/// the rest stays at `A` as the next deposit (the next hour's bucket, or the refund after the wait). A fit under the least mint, or
/// under a tenth of the deposit, is no fit: the whole deposit goes to the books' own refusal (a transient wait).
fn fit_bucket(coll: u16, lamports: u64, min_units: u64, now: i64) -> (u64, u64) {
    with(|s| {
        let Ok(c) = s.books.get(coll) else { return (lamports, min_units) };
        let Ok(room) = c.module.mint_room(now) else { return (lamports, min_units) };
        // the whole deposit's wX: the quote, or (over the bucket, which the quote refuses) the least mint's quote scaled
        let units = match s.books.quote_mint(coll, lamports, now) {
            Ok(u) => u,
            Err(_) => match s.books.quote_mint(coll, MIN_MINT_LAMPORTS, now) {
                Ok(u) => (u as u128 * lamports as u128 / MIN_MINT_LAMPORTS as u128) as u64,
                Err(_) => return (lamports, min_units),
            },
        };
        if units <= room || room == 0 {
            return (lamports, min_units);
        }
        let mut fit = (lamports as u128 * room as u128 / units as u128) as u64;
        for _ in 0..8 {
            match s.books.quote_mint(coll, fit, now) {
                Ok(u) if u <= room => break,
                _ => fit = fit / 100 * 99,
            }
        }
        // a sliver is not worth its fee and its transaction: under a tenth of the deposit, the whole waits for the bucket
        if fit < MIN_MINT_LAMPORTS || fit < lamports / 10 || s.books.quote_mint(coll, fit, now).is_err() {
            return (lamports, min_units);
        }
        (fit, (min_units as u128 * fit as u128 / lamports as u128) as u64)
    })
}

async fn token_amount<I: DeskIo>(io: &mut I, it: &Intent, extra: &[Pubkey], mint: &Pubkey) -> Result<(u64, Vec<Option<Account>>), String> {
    let ata = it.token_account.ok_or("no token account")?;
    let mut keys = vec![ata];
    keys.extend_from_slice(extra);
    let mut accs = io.sol_accounts(&keys, 165).await?;
    let first = accs.remove(0);
    Ok((amount_of(it, &first, mint), accs))
}

async fn exec_burn<I: DeskIo>(io: &mut I, it: &Intent, now: u64, log: &mut Vec<String>) -> Exec {
    let Kind::Burn { coll, recipient, min_lamports, .. } = it.kind else { return Exec::Skip };
    let Ok(mint) = with(|s| s.books.get(coll).map(|c| c.wx_mint)) else {
        set_refund(it.id, "unknown collection".into());
        return Exec::Skip;
    };
    let a = match addrs(io, it, mint).await {
        Ok(a) => a,
        Err(e) => {
            log.push(format!("desk: waits: {e}"));
            return Exec::Stop;
        }
    };
    let (units, _) = match token_amount(io, it, &[], &mint).await {
        Ok(v) => v,
        Err(e) => {
            log.push(format!("desk: waits: {e}"));
            return Exec::Stop;
        }
    };
    if units < MIN_BURN_UNITS {
        edit(it.id, |i| {
            i.state = State::Open;
            i.deposit = units;
        });
        return Exec::Skip;
    }
    let now_i = now as i64;
    let quote = with(|s| super::quote::quote_burn(&s.books, coll, units, now_i));
    let q = match quote {
        Ok(q) if q.paid > FEE_LAMPORTS => q,
        Ok(_) => {
            set_refund(it.id, "the payout does not cover the fee".into());
            return Exec::Skip;
        }
        Err(e) => return refused(it, LinkError::Books(e)),
    };
    let link = match begin_link(it, OpKind::Burn, Scope::Module, now, |b| {
        let r = b.burn(coll, units, min_lamports.saturating_add(FEE_LAMPORTS), now_i)?;
        Ok((Effects { burn_units: units, lamports_out: r.paid, ..Default::default() }, r.paid))
    }) {
        Ok(l) => l,
        Err(e) => return refused(it, e),
    };
    let paid = with(|s| s.link.get(link).map(|l| l.effects.lamports_out)).unwrap_or(q.paid);
    let ixs = build::burn_tx(&a, recipient, units, paid.saturating_sub(FEE_LAMPORTS), FEE_LAMPORTS, &memo_of(link));
    send(io, it, link, Purpose::Execute, units, ixs, build::burn_keys(it.id, coll), now, log).await
}

async fn exec_redeem<I: DeskIo>(io: &mut I, it: &Intent, now: u64, log: &mut Vec<String>) -> Exec {
    let Kind::RedeemNft { coll, ref destination, owner } = it.kind else { return Exec::Skip };
    let Ok(mint) = with(|s| s.books.get(coll).map(|c| c.wx_mint)) else {
        set_refund(it.id, "unknown collection".into());
        return Exec::Skip;
    };
    let a = match addrs(io, it, mint).await {
        Ok(a) => a,
        Err(e) => {
            log.push(format!("desk: waits: {e}"));
            return Exec::Stop;
        }
    };
    let (units, _) = match token_amount(io, it, &[], &mint).await {
        Ok(v) => v,
        Err(e) => {
            log.push(format!("desk: waits: {e}"));
            return Exec::Stop;
        }
    };
    if units < ONE {
        edit(it.id, |i| {
            i.state = State::Open;
            i.deposit = units;
        });
        return Exec::Skip;
    }
    let dest = destination.clone();
    let link = match begin_link(it, OpKind::Redeem, Scope::Collection, now, |b| {
        let t = b.redeem(coll, owner, dest, now as i64)?;
        Ok((Effects { burn_units: ONE, ..Default::default() }, t))
    }) {
        Ok(l) => l,
        Err(e) => return refused(it, e),
    };
    let ixs = build::redeem_tx(&a, ONE, &memo_of(link));
    send(io, it, link, Purpose::Execute, units, ixs, build::redeem_keys(it.id), now, log).await
}

/// The deposit goes back to the intent's refund wallet (no books change: a refund is not an operation of the collection).
async fn refund_op<I: DeskIo>(io: &mut I, it: &Intent, now: u64, log: &mut Vec<String>) -> Exec {
    let to = it.kind.refund_to();
    let mint = deposit_mint(it).unwrap_or_default();
    let main = match io.sol_me().await {
        Ok(m) => m,
        Err(e) => {
            log.push(format!("desk: waits: {e}"));
            return Exec::Stop;
        }
    };
    let a = Addrs { main, intent: it.address, module: Pubkey::default(), mint };
    if it.kind.is_sol() {
        let deposit = match io.sol_accounts(&[it.address], 165).await {
            Ok(v) => amount_of(it, &v[0], &mint),
            Err(e) => {
                log.push(format!("desk: waits: {e}"));
                return Exec::Stop;
            }
        };
        let Some(net) = deposit.checked_sub(FEE_LAMPORTS).filter(|_| deposit >= DUST_REFUND_LAMPORTS) else {
            return gave_up(it, deposit, "too small to return");
        };
        let link = match begin_link(it, OpKind::Refund, Scope::None, now, |_| Ok((Effects::default(), deposit))) {
            Ok(l) => l,
            Err(e) => return refused(it, e),
        };
        let ixs = build::refund_sol_tx(&a, to, net, FEE_LAMPORTS, &memo_of(link));
        send(io, it, link, Purpose::Refund, deposit, ixs, build::refund_keys(it.id), now, log).await
    } else {
        let dest = associated_token_address(&to, &mint);
        let (units, others) = match token_amount(io, it, &[dest], &mint).await {
            Ok(v) => v,
            Err(e) => {
                log.push(format!("desk: waits: {e}"));
                return Exec::Stop;
            }
        };
        if units < DUST_REFUND_UNITS {
            return gave_up(it, units, "too small to return");
        }
        // Audit B1-02 with B1-04: `K_main` creates an absent destination only when `A`'s account will be closed to it afterwards (no
        // close is pending or given up). Otherwise a depositor could keep `A` open (redeposit before the close, or dust it) and have
        // `K_main` create a destination they then close for its rent, again and again.
        if others.first().is_some_and(|d| d.is_none()) && (it.close_due || it.close_tries >= MAX_CLOSE_TRIES) {
            edit(it.id, |i| {
                i.attention = true;
                i.note = "the refund's destination has no token account and the deposit account was not closed: an operator must look".into();
            });
            return Exec::Skip;
        }
        let link = match begin_link(it, OpKind::Refund, Scope::None, now, |_| Ok((Effects::default(), units))) {
            Ok(l) => l,
            Err(e) => return refused(it, e),
        };
        let ixs = build::refund_token_tx(&a, to, units, &memo_of(link));
        send(io, it, link, Purpose::Refund, units, ixs, build::refund_keys(it.id), now, log).await
    }
}

/// Audit B1-04: `A`'s token account, emptied by a burn or a token refund that landed, closed by a transaction of its own (to `K_main`),
/// only when finalized shows it empty; dust in it counts as a try, and after [`MAX_CLOSE_TRIES`] it is left open.
async fn exec_close<I: DeskIo>(io: &mut I, it: &Intent, now: u64, log: &mut Vec<String>) -> Exec {
    let Some(ata) = it.token_account.filter(|_| it.close_due) else {
        edit(it.id, |i| i.close_due = false);
        return Exec::Skip;
    };
    let mint = deposit_mint(it).unwrap_or_default();
    let main = match io.sol_me().await {
        Ok(m) => m,
        Err(e) => {
            log.push(format!("desk: waits: {e}"));
            return Exec::Stop;
        }
    };
    let acc = match io.sol_accounts(&[ata], 165).await {
        Ok(mut v) => v.remove(0),
        Err(e) => {
            log.push(format!("desk: waits: {e}"));
            return Exec::Stop;
        }
    };
    if acc.is_none() {
        edit(it.id, |i| {
            i.close_due = false;
            i.close_tries = 0;
        });
        return Exec::Skip;
    }
    let left = amount_of(it, &acc, &mint);
    if left > 0 {
        edit(it.id, |i| close_failed(i, &format!("the deposit account holds {left}")));
        return Exec::Skip;
    }
    let link = match begin_link(it, OpKind::Close, Scope::None, now, |_| Ok((Effects::default(), 0))) {
        Ok(l) => l,
        Err(e) => return refused(it, e),
    };
    let a = Addrs { main, intent: it.address, module: Pubkey::default(), mint };
    let ixs = build::close_tx(&a, &memo_of(link));
    send(io, it, link, Purpose::Close, 0, ixs, build::refund_keys(it.id), now, log).await
}

/// A deposit not worth a transaction stays where it is; the intent stops asking.
fn gave_up(it: &Intent, amount: u64, why: &str) -> Exec {
    edit(it.id, |i| {
        i.refund_only = false;
        i.must_refund = None;
        i.funded_at = None;
        i.state = if i.done_ops > 0 { State::Done } else { State::Open };
        i.note = format!("{amount} left at the deposit address: {why}");
    });
    Exec::Skip
}
