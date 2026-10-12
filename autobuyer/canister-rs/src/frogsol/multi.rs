//! Ordinals multi-collection on the books home (docs/v9/ordinals-collections.md §4, "Built (2026-10-12)"): every Inscriptions listing
//! in the books is a frog collection of its own. Its frog state (`frogs`, `proofs`, `convs`, `protected`, `ow_creators`, the floor
//! reading, `values`, its floor sink's view) is an entry of the top-level `FrogState.colls`, keyed by its books id; its life is a
//! `dormancy::Life` (`FrogState.dorm`); its redemptions are an entry of `Custody.colls`. What custody is stays single and
//! custody-wide: the one taproot address, `seen`, `unmarked`, the deposits, the fee pool and the float, `next_frog` (frog ids unique
//! canister-wide), the one Solana line and the one Relay leg. Pure but for the books' read (`desk::with`).
use crate::books::{self, ListingKind};
use crate::custody::{Contents, Outpoint};
use crate::desk::state as desk;
use crate::dormancy::{Dorm, Life, Phase, WakeBy};
use crate::frogbuy;
use crate::frogflow::FrogState;
use crate::redeem::Custody;
use crate::soltx::Pubkey;

/// One Inscriptions listing as the frog side needs it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listing {
    pub id: u16,
    pub wx_mint: Pubkey,
    pub root: [u8; 32],
    pub custody: String,
}

/// Every Inscriptions listing in the books, by id.
pub fn listings() -> Vec<Listing> {
    desk::with(|s| listings_of(&s.books))
}

pub fn listings_of(b: &books::Books) -> Vec<Listing> {
    b.collections.iter().filter_map(|(id, c)| match &c.kind {
        ListingKind::Inscriptions { membership_root, custody } => Some(Listing { id: *id, wx_mint: c.wx_mint, root: *membership_root,
            custody: custody.clone() }),
        _ => None,
    }).collect()
}

pub fn listing(id: u16) -> Result<Listing, String> {
    listings().into_iter().find(|l| l.id == id).ok_or_else(|| format!("refused: collection {id} is not an Inscriptions listing in the books"))
}

/// A listing's life key: the first 20 bytes of its wX mint (`dormancy::Life.collection`).
pub fn life_key(wx: &Pubkey) -> [u8; 20] {
    wx[..20].try_into().expect("20")
}

/// Whether a listing is live. Default-live (operator decision 2026-10-12, audit C2 §4): an Inscriptions listing with NO life is live
/// from install (a collection simply cannot mint until a floor reading is accepted: `books_sink_view` wants nothing of a module never
/// posted); only the operator's brake, `frog_sleep`, gives it a DORMANT life, and `frog_wake` makes it ACTIVE again.
pub fn is_live(top: &FrogState, wx: &Pubkey) -> bool {
    top.dorm.as_ref().and_then(|d| d.life(&life_key(wx))).is_none_or(|l| l.is_active())
}

/// Audit C2-14: a listing whose members root repeats a lower id's is never live (its members would be one item in two ledgers): the
/// first listing of a root is the one served.
pub fn repeats_root(all: &[Listing], l: &Listing) -> bool {
    all.iter().any(|x| x.id < l.id && x.root == l.root)
}

/// The live listings, by id (every listing the operator has not put to sleep, a repeated root excepted).
pub fn live(top: &FrogState, all: &[Listing]) -> Vec<Listing> {
    all.iter().filter(|l| is_live(top, &l.wx_mint) && !repeats_root(all, l)).cloned().collect()
}

pub fn entry(top: &FrogState, id: u16) -> Option<&FrogState> {
    top.colls.as_ref()?.iter().find(|(c, _)| *c == id).map(|(_, s)| s)
}

/// The entry of `id`, made empty if it has none.
pub fn entry_mut(top: &mut FrogState, id: u16) -> &mut FrogState {
    let v = top.colls.get_or_insert_with(Vec::new);
    let i = match v.iter().position(|(c, _)| *c == id) {
        Some(i) => i,
        None => {
            v.push((id, FrogState::default()));
            v.len() - 1
        }
    };
    &mut v[i].1
}

/// Write a collection's state back: its entry, and `next_frog` canister-wide (the larger).
pub fn put(top: &mut FrogState, id: u16, st: &FrogState) {
    top.next_frog = top.next_frog.max(st.next_frog);
    let mut st = st.clone();
    st.colls = None;
    st.dorm = None;
    *entry_mut(top, id) = st;
}

/// A collection's state to tick: its entry with `next_frog` canister-wide.
pub fn take(top: &FrogState, id: u16) -> FrogState {
    let mut st = entry(top, id).cloned().unwrap_or_default();
    st.next_frog = st.next_frog.max(top.next_frog);
    st
}

/// The top-level state and every collection's: what canister-wide rules read (the one Relay leg, the reimbursements owed).
pub fn states(top: &FrogState) -> Vec<&FrogState> {
    std::iter::once(top).chain(top.colls.iter().flatten().map(|(_, s)| s)).collect()
}

/// B-L-6 across collections: the one Relay leg is held by any collection's conversion (or the top-level state's).
pub fn holds_relay(top: &FrogState) -> bool {
    states(top).into_iter().any(crate::convflow::holds_relay)
}

/// B-L-6 across collections, for one collection: another collection's conversion holds the leg (or waits for it first).
pub fn held_elsewhere(top: &FrogState, me: u16) -> bool {
    crate::convflow::holds_relay(top) || top.colls.iter().flatten().any(|(c, s)| *c != me && crate::convflow::holds_relay(s))
}

/// B-L-6's fairness across chains: a Bitcoin-side conversion waits for the leg, so the Ethereum side starts no new buy. Audit C2-07: not
/// while the Ethereum side's turn is due (an Inscriptions collection took the last leg, [`took_leg`]): the Ethereum side yields to the
/// Bitcoin side once per leg, never to every collection in turn.
pub fn wants_relay(top: &FrogState) -> bool {
    top.eth_turn.is_none() && states(top).into_iter().any(crate::convflow::wants_relay)
}

/// Audit C2-07: how long the Ethereum side's turn holds a free leg for it once it is due: two runner cycles. Past it (the Ethereum side
/// wanted nothing) the Bitcoin side's collections take the leg again.
pub const ETH_TURN_SECS: u64 = 120;

/// Audit C2-07: an Inscriptions collection took the Relay leg (or a step on the line): the rotation's cursor moves past it, and for the
/// leg the Ethereum side's turn is due next.
pub fn took(top: &mut FrogState, id: u16, leg: bool) {
    top.turn = Some(id);
    if leg {
        top.eth_turn = Some(0);
    }
}

/// [`took`] for the leg.
pub fn took_leg(top: &mut FrogState, id: u16) {
    took(top, id, true)
}

/// Audit C2-07, once a tick before the collections' passes: whether the Bitcoin side yields the free leg to the Ethereum side now.
/// `eth_holds`: a chain side holds the leg (its turn is had: nothing is due); `held`: a collection still holds it (the turn's clock
/// starts once it is free). The turn lapses after [`ETH_TURN_SECS`] of a free leg the Ethereum side did not take.
pub fn eth_turn_now(top: &mut FrogState, eth_holds: bool, now: u64) -> bool {
    if eth_holds {
        top.eth_turn = None;
        return false;
    }
    let held = holds_relay(top);
    match top.eth_turn {
        None => false,
        Some(_) if held => false,
        Some(0) => {
            top.eth_turn = Some(now.max(1));
            true
        }
        Some(t) if now.saturating_sub(t) < ETH_TURN_SECS => true,
        Some(_) => {
            top.eth_turn = None;
            false
        }
    }
}

/// Audit C2-07: the live listings in the rotation's order: those after the last that took the leg or the line (`turn`), then the rest,
/// so the first collection wanting the leg or the frog turn is the next in a round, never the lowest id each time.
pub fn in_turn(top: &FrogState, live: Vec<Listing>) -> Vec<Listing> {
    let Some(t) = top.turn else { return live };
    let (after, upto): (Vec<Listing>, Vec<Listing>) = live.into_iter().partition(|l| l.id > t);
    after.into_iter().chain(upto).collect()
}

/// What is still open in a collection's state: a frog not finished, a conversion not finished, a protected buy handed back, a step out
/// on the line. A collection is put to sleep only with nothing open.
pub fn open_work(st: &FrogState) -> Option<String> {
    if let Some(f) = st.frogs.iter().find(|f| !f.finished()) {
        return Some(format!("frog {} is {:?}", f.id, f.phase));
    }
    if let Some(c) = st.convs().iter().find(|c| !c.finished()) {
        return Some(format!("frog {}'s conversion is {:?}", c.frog, c.phase));
    }
    // audit C2-02: a finished conversion whose written-off order is still watched (C-03), or anything that holds the one Relay leg or
    // watches our balance: a dormant collection is never ticked, so its watch would never be read and the leg never freed
    if let Some(c) = st.convs().iter().find(|c| c.late_open()) {
        return Some(format!("frog {}'s written-off Relay order is still watched and holds the one Relay leg (relay_late_release, or wait \
            for its refund or fill)", c.frog));
    }
    if crate::convflow::holds_relay(st) || crate::convflow::watching(st) {
        return Some("a conversion holds the one Relay leg".into());
    }
    if st.sol_step.is_some() || st.pending.is_some() {
        return Some("a step is out".into());
    }
    None
}

/// The operator's wake (`eth_wake`'s form: the wake desk is dropped on the books home, O-11): DORMANT to ACTIVE at once (no backfill:
/// the runner hands in the floor reading). Default-live (2026-10-12): it lifts the operator's own brake; a listing with no life is live
/// already and gets none.
pub fn wake(top: &mut FrogState, l: &Listing, now: u64) -> Result<String, String> {
    let k = life_key(&l.wx_mint);
    let Some(life) = top.dorm.as_mut().and_then(|d| d.life_mut(&k)) else { return Ok(format!("collection {} is already live", l.id)) };
    if life.is_active() {
        return Ok(format!("collection {} is already live", l.id));
    }
    life.phase = Phase::Active { since: now };
    life.since = now;
    life.by = Some(WakeBy::Operator);
    life.queued = None;
    life.ever_active = true;
    life.note = "woken by the operator".into();
    Ok(format!("collection {} is live: floor readings, book reads and buys from the next tick", l.id))
}

/// The operator's sleep: ACTIVE to DORMANT, only with nothing of the collection open. Its redemptions go on (custody's exit is every
/// listing's, live or not).
pub fn sleep(top: &mut FrogState, l: &Listing, now: u64) -> Result<String, String> {
    if let Some(w) = entry(top, l.id).and_then(open_work) {
        return Err(format!("refused: collection {} has work open ({w}); it sleeps once it is finished", l.id));
    }
    let k = life_key(&l.wx_mint);
    let d = top.dorm.get_or_insert_with(Dorm::default);
    // default-live: a listing with no life is live, and the sleep gives it a DORMANT one
    let first = d.life(&k).is_none();
    if first {
        d.lives.push(Life::new(k, Phase::Dormant, now));
    }
    let life = d.life_mut(&k).expect("pushed");
    if life.is_dormant() && !first {
        return Ok(format!("collection {} is already dormant", l.id));
    }
    life.phase = Phase::Dormant;
    life.since = now;
    life.note = "put to sleep by the operator".into();
    if let Some(e) = top.colls.as_mut().and_then(|v| v.iter_mut().find(|(c, _)| *c == l.id)) {
        e.1.floor_reading = None;
    }
    Ok(format!("collection {} is dormant: no floor reading, no book read, no buy", l.id))
}

/// Default-live (operator decision 2026-10-12; audit C2 §4, must-hold 5): what the wake check waited for, set before a listing's first
/// floor reading is posted (the tick runs this before the listing's pass, and the reading is posted only inside it), from the launch
/// files (`frogsol::launch`) and the canister's own derivation, each only where nothing is set yet (a controller's `set_floor_sink`,
/// `set_purchase_limit`, `set_ow_creators` always win): the floor sink (the listing's module, `mk`, buying Bitcoin inscriptions),
/// the purchase limit and minimum, the Ordinals Wallet creators. A listing whose members root no launch file names gets the floor sink
/// only (its first floor is refused anyway, audit C2-12). Returns what was set.
pub fn launch_defaults(top: &mut FrogState, fl: &mut crate::floortrigger::FloorState, l: &Listing, mk: [u8; 20]) -> Vec<String> {
    use crate::itemsink::{SinkKind, SinkRec};
    let mut set = Vec::new();
    if fl.item_of(&mk).is_none() && fl.add_sink(mk, crate::floortrigger::FloorItem::Bitcoin).is_ok() {
        set.push("its floor sink (its module)".to_string());
    }
    if !top.sinks.iter().any(|s| s.sink == mk) {
        top.sinks.push(SinkRec { kind: SinkKind::Floor, sink: mk, coin: [0; 20], view: None, refused: None, drove_refunds_at: None, drove_settle: false });
    }
    let Some(d) = super::launch::of_root(&l.root) else { return set };
    if fl.limit_for(&mk).is_none() {
        if let Some(max) = d.max_purchase_lamports {
            match fl.set_limit_min_in(mk, max, d.min_purchase_lamports, super::ABS_MIN_PURCHASE_LAMPORTS, super::MIN_PURCHASE_LAMPORTS,
                super::RELAY_SOL_BTC_MAX_LAMPORTS, "lamports") {
                Ok(()) => set.push(format!("its purchase limit ({max} lamports at most, {} at least)", d.min_purchase_lamports
                    .unwrap_or(super::MIN_PURCHASE_LAMPORTS))),
                Err(e) => set.push(format!("NOT its purchase limit: {e}")),
            }
        }
    }
    let e = entry_mut(top, l.id);
    if e.ow_creators.is_none() {
        e.ow_creators = Some(d.ow_creators.iter().map(|s| s.to_string()).collect());
        set.push(format!("its Ordinals Wallet creators ({} pinned)", d.ow_creators.len()));
    }
    set
}

/// Whether the single state of before carries a collection's work (anything but what stays custody-wide). Audit C2-10: the floor sinks
/// as configured are custody-wide configuration, not work (moot under default-live, where every listing is live anyway).
fn has_collection_work(s: &FrogState) -> bool {
    !s.frogs.is_empty() || !s.proofs.is_empty() || !s.convs().is_empty() || s.protected.as_ref().is_some_and(|v| !v.is_empty())
        || s.ow_creators.as_ref().is_some_and(|v| !v.is_empty()) || s.floor_reading.is_some() || s.sol_step.is_some()
}

/// The upgrade from the single state (`post_upgrade`, and lazily at the first tick that finds an Inscriptions listing): what is one
/// collection's moves into the entry of `l` (the one Inscriptions listing), which stays live as it was; what is custody-wide (`seen`,
/// `unmarked`, the deposits and their clocks, `next_frog`, the floor sinks as configured) stays at the top. Once only: `false` when
/// the state is already per collection.
pub fn migrate(top: &mut FrogState, l: &Listing, now: u64) -> bool {
    if top.colls.is_some() {
        return false;
    }
    let work = has_collection_work(top);
    let e = FrogState {
        sinks: top.sinks.clone(),
        frogs: std::mem::take(&mut top.frogs),
        next_frog: top.next_frog,
        proofs: std::mem::take(&mut top.proofs),
        buy_clock: top.buy_clock.take(),
        convs: top.convs.take(),
        protected: top.protected.take(),
        ow_creators: top.ow_creators.take(),
        sol_step: top.sol_step.take(),
        floor_reading: top.floor_reading.take(),
        values: top.values.take(),
        ..Default::default()
    };
    top.colls = Some(vec![(l.id, e)]);
    if work {
        let _ = wake(top, l, now);
    }
    true
}

/// The custodian's upgrade likewise: the one cursor, the redemptions and the scanner of the single state move into the entry of `id`.
pub fn migrate_custody(c: &mut Custody, id: u16) -> bool {
    if c.colls.is_some() {
        return false;
    }
    let e = Custody { cursor: std::mem::take(&mut c.cursor), redemptions: std::mem::take(&mut c.redemptions), logs: c.logs.take(), colls: None };
    c.colls = Some(vec![(id, e)]);
    true
}

pub fn custody_entry(c: &Custody, id: u16) -> Custody {
    c.colls.as_ref().and_then(|v| v.iter().find(|(x, _)| *x == id)).map(|(_, e)| e.clone()).unwrap_or_default()
}

pub fn custody_entry_mut(c: &mut Custody, id: u16) -> &mut Custody {
    let v = c.colls.get_or_insert_with(Vec::new);
    let i = match v.iter().position(|(x, _)| *x == id) {
        Some(i) => i,
        None => {
            v.push((id, Custody::default()));
            v.len() - 1
        }
    };
    &mut v[i].1
}

pub fn put_custody(c: &mut Custody, id: u16, e: Custody) {
    let v = c.colls.get_or_insert_with(Vec::new);
    match v.iter_mut().find(|(x, _)| *x == id) {
        Some(x) => x.1 = e,
        None => v.push((id, e)),
    }
}

/// Every collection's custodian state and the top-level one's (the views; fee inputs reserved across collections).
pub fn custodies(c: &Custody) -> Vec<&Custody> {
    std::iter::once(c).chain(c.colls.iter().flatten().map(|(_, e)| e)).collect()
}

// ---- custody: one address, classified once ---------------------------------------------------------------------------------------

/// Where a custody output goes, decided once for the whole custody.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Route {
    /// A purchase of this collection (its frog's bought output).
    Bought(u16),
    /// Plain sats (a fee input, change).
    Plain,
    /// One inscription, a member of exactly this collection's root: that collection's (`unmarked` there, for the reconciliation; a
    /// holder deposit's wrap waits for the EWD2 claim, Q7).
    Member(u16),
    /// Nothing a live collection may hold (no root its inscription proves against, or several, or not exactly one inscription, or a
    /// rune): refused once, custody-wide (`unmarked` at the top), never once per collection.
    NotMember(String),
}

/// One custody output's route. `contents`: ord's, `None` while ord has not indexed it (decided later). The proof is the operator's
/// hand-in (`frog_proof`, kept in the collection it names), and every proof handed in for the inscription is tried against every live
/// root: a member of exactly one is that collection's.
pub fn route(o: &Outpoint, contents: Option<&Contents>, top: &FrogState, roots: &[(u16, [u8; 32])]) -> Option<Route> {
    if let Some((id, _)) = top.colls.iter().flatten().find(|(_, s)| s.frogs.iter().any(|f| f.bought().is_some_and(|b| b.outpoint == *o))) {
        return Some(Route::Bought(*id));
    }
    let c = contents?;
    if c.inscriptions.is_empty() && c.runes.is_empty() {
        return Some(Route::Plain);
    }
    if c.inscriptions.len() != 1 || !c.runes.is_empty() {
        return Some(Route::NotMember(format!("it carries {:?} and runes {:?}, not exactly one inscription", c.inscriptions, c.runes)));
    }
    let id = &c.inscriptions[0];
    let leaf = frogbuy::leaf_of(id);
    let proofs: Vec<Vec<[u8; 32]>> = states(top).into_iter().filter_map(|s| s.proof(id)).collect();
    let hits: Vec<u16> = roots.iter().filter(|(_, r)| proofs.iter().any(|p| frogbuy::verify_proof(p, r, &leaf))).map(|(c, _)| *c).collect();
    Some(match hits.as_slice() {
        [c] => Route::Member(*c),
        [] => Route::NotMember(format!("{id} proves against none of the {} live roots", roots.len())),
        _ => Route::NotMember(format!("{id} proves against {} roots ({hits:?}): ambiguous", hits.len())),
    })
}

/// The routes applied: each output is `seen` once, custody-wide; a member is its collection's, a non-member the top's `unmarked`
/// (once). `unspent`: custody's listing now (what was seen stays bounded by it). Returns the log lines.
pub fn apply(top: &mut FrogState, unspent: &[Outpoint], routes: Vec<(Outpoint, Route)>) -> Vec<String> {
    top.seen.retain(|o| unspent.contains(o));
    top.unmarked.retain(|o| unspent.contains(o));
    if let Some(u) = top.unread.as_mut() {
        u.retain(|o| unspent.contains(o));
    }
    if let Some(f) = top.read_failures.as_mut() {
        f.retain(|(o, _)| unspent.contains(o) && !routes.iter().any(|(r, _)| r == o));
    }
    for (_, e) in top.colls.iter_mut().flatten() {
        e.unmarked.retain(|o| unspent.contains(o));
    }
    let mut lines = Vec::new();
    for (o, r) in routes {
        if top.seen.contains(&o) {
            continue;
        }
        top.seen.push(o);
        match r {
            Route::Bought(_) | Route::Plain => {}
            Route::Member(c) => {
                // audit C2-06: an output refused before (no live root, or its proof not handed in yet) and routed again is its
                // collection's now
                top.unmarked.retain(|x| *x != o);
                let e = entry_mut(top, c);
                if !e.unmarked.contains(&o) {
                    e.unmarked.push(o);
                }
                lines.push(format!("custody: {} holds an item of collection {c} we did not buy: kept for its reconciliation", o.text()));
            }
            Route::NotMember(why) => {
                if !top.unmarked.contains(&o) {
                    top.unmarked.push(o);
                    lines.push(format!("custody: {} is no live collection's ({why}): refused once, kept for the reconciliation", o.text()));
                }
            }
        }
    }
    lines
}

/// Audit C2-06: the classification is no longer decided once for good: when the live roots change (`frog_wake`, `frog_sleep`) or a
/// proof is handed in (`frog_proof`), every output refused custody-wide (`unmarked` at the top) is routed again at the next ticks
/// (it stays in `unmarked` until a route makes it a collection's; at most `MAX_NEW_OUTPUTS_PER_TICK` a tick, as new outputs).
/// Returns how many are looked at again.
pub fn reopen(top: &mut FrogState) -> usize {
    let again: Vec<Outpoint> = top.unmarked.clone();
    top.seen.retain(|o| !again.contains(o));
    again.len()
}

/// Audit C2-05: outputs whose contents could not be read this tick: each counted, and set aside (`unread`, for the reconciliation)
/// after `deposits::MAX_READ_FAILURES`, as `deposits_pass` does (audit D, H-3). The rest of the batch was routed regardless.
pub fn set_aside(top: &mut FrogState, failed: Vec<(Outpoint, String)>) -> Vec<String> {
    let mut lines = Vec::new();
    for (o, e) in failed {
        let fails = top.read_failures.get_or_insert_with(Vec::new);
        let k = match fails.iter_mut().find(|(x, _)| *x == o) {
            Some(x) => {
                x.1 += 1;
                x.1
            }
            None => {
                fails.push((o, 1));
                1
            }
        };
        if k >= crate::deposits::MAX_READ_FAILURES {
            fails.retain(|(x, _)| *x != o);
            let u = top.unread.get_or_insert_with(Vec::new);
            if !u.contains(&o) {
                u.push(o);
            }
            lines.push(format!("custody: {} not read {k} times ({e}): set aside for the reconciliation, the classification goes on", o.text()));
        } else {
            lines.push(format!("custody: {} not read ({k} of {}): {e}", o.text(), crate::deposits::MAX_READ_FAILURES));
        }
    }
    lines
}

/// Custody's classification once a tick (the books home): custody's confirmed outputs not seen yet, at most
/// `frogflow::MAX_NEW_OUTPUTS_PER_TICK`, each routed once ([`route`]), then applied by the caller ([`apply`]). Returns custody's
/// unspent listing, the routes, and (audit C2-05) the outputs whose contents could not be read: skipped, never the batch's end
/// ([`set_aside`] counts them).
pub async fn classify<I: crate::frogflow::FrogIo>(io: &mut I, top: &FrogState, roots: &[(u16, [u8; 32])])
    -> Result<(Vec<Outpoint>, Vec<(Outpoint, Route)>, Vec<(Outpoint, String)>), String> {
    let utxos = io.custody_utxos().await?;
    let unspent: Vec<Outpoint> = utxos.iter().map(|u| u.outpoint).collect();
    let unread = top.unread.clone().unwrap_or_default();
    let (mut routes, mut failed) = (Vec::new(), Vec::new());
    for u in utxos.iter().filter(|u| !top.seen.contains(&u.outpoint) && !unread.contains(&u.outpoint)) {
        if routes.len() + failed.len() >= crate::frogflow::MAX_NEW_OUTPUTS_PER_TICK {
            break;
        }
        let pre = route(&u.outpoint, None, top, roots);
        let r = match pre {
            Some(r) => Some(r),
            None => match io.contents(&u.outpoint).await {
                Ok(c) => route(&u.outpoint, c.as_ref(), top, roots),
                Err(e) => {
                    failed.push((u.outpoint, e));
                    None
                }
            },
        };
        if let Some(r) = r {
            routes.push((u.outpoint, r));
        }
    }
    Ok((unspent, routes, failed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frogflow::FrogState;

    /// The launch file's members roots and one real member and proof each (tests/fixtures/ordinals/multi.json, made from
    /// deployments/launch/btc/*-members.json; every root is nft-pilot.json's).
    const FIX: &str = include_str!("../../tests/fixtures/ordinals/multi.json");
    fn fix(slug: &str) -> ([u8; 32], String, Vec<[u8; 32]>) {
        let v: serde_json::Value = serde_json::from_str(FIX).unwrap();
        let h = |s: &str| -> [u8; 32] { hex::decode(s).unwrap().try_into().unwrap() };
        let e = &v[slug];
        (h(e["root"].as_str().unwrap()), e["inscription"].as_str().unwrap().to_string(),
            e["proof"].as_array().unwrap().iter().map(|p| h(p.as_str().unwrap())).collect())
    }
    const FROGS: u16 = 11;
    const MONKES: u16 = 12;
    const OMB: u16 = 13;
    fn listing(id: u16, slug: &str) -> Listing {
        Listing { id, wx_mint: [id as u8; 32], root: fix(slug).0, custody: "bc1pcustody".into() }
    }
    fn o(n: u8) -> Outpoint {
        Outpoint { txid: [n; 32], vout: 0 }
    }
    fn holding(id: &str) -> Contents {
        Contents { inscriptions: vec![id.into()], runes: vec![], value: 546 }
    }
    fn plain() -> Contents {
        Contents { inscriptions: vec![], runes: vec![], value: 9_000 }
    }
    /// Frogs and NodeMonkes live, OMB listed and put to sleep (default-live: the operator's brake); each proof handed in to its own
    /// collection.
    fn two_live() -> FrogState {
        let mut top = FrogState::default();
        migrate(&mut top, &listing(FROGS, "bitcoin-frogs"), 1);
        sleep(&mut top, &listing(OMB, "omb"), 1).unwrap();
        for (c, slug) in [(FROGS, "bitcoin-frogs"), (MONKES, "nodemonkes"), (OMB, "omb")] {
            let (_, id, proof) = fix(slug);
            entry_mut(&mut top, c).put_proof(id, proof);
        }
        top
    }
    fn live_roots(top: &FrogState) -> Vec<(u16, [u8; 32])> {
        let all = [listing(FROGS, "bitcoin-frogs"), listing(MONKES, "nodemonkes"), listing(OMB, "omb")];
        live(top, &all).iter().map(|l| (l.id, l.root)).collect()
    }

    /// Part 2: one custody address, each output classified once: a Frog and a NodeMonke each routed to its own collection by the root its
    /// inscription proves against (the real roots and proofs), an OMB (its collection put to sleep) and an output of two inscriptions refused
    /// ONCE custody-wide, plain sats and a purchase of ours seen and nothing more.
    #[test]
    fn custody_routes_each_output_once_to_the_collection_whose_root_it_proves_against() {
        let mut top = two_live();
        let roots = live_roots(&top);
        assert_eq!(roots.iter().map(|r| r.0).collect::<Vec<_>>(), vec![FROGS, MONKES], "OMB is asleep: its root is not live");
        let (frog, monke, omb) = (fix("bitcoin-frogs").1, fix("nodemonkes").1, fix("omb").1);
        assert_eq!(route(&o(1), Some(&holding(&frog)), &top, &roots), Some(Route::Member(FROGS)));
        assert_eq!(route(&o(2), Some(&holding(&monke)), &top, &roots), Some(Route::Member(MONKES)));
        assert!(matches!(route(&o(3), Some(&holding(&omb)), &top, &roots), Some(Route::NotMember(w)) if w.contains("none of the 2 live roots")));
        let two = Contents { inscriptions: vec![frog.clone(), monke.clone()], runes: vec![], value: 546 };
        assert!(matches!(route(&o(4), Some(&two), &top, &roots), Some(Route::NotMember(_))));
        assert_eq!(route(&o(5), Some(&plain()), &top, &roots), Some(Route::Plain));
        assert_eq!(route(&o(6), None, &top, &roots), None, "not indexed yet: decided later");
        // a proof handed in under the wrong collection still proves against its own root only: the route follows the root
        let mut wrong = two_live();
        wrong.colls.as_mut().unwrap().iter_mut().for_each(|(_, s)| s.proofs.clear());
        let (_, id, proof) = fix("nodemonkes");
        entry_mut(&mut wrong, FROGS).put_proof(id, proof);
        assert_eq!(route(&o(2), Some(&holding(&monke)), &wrong, &roots), Some(Route::Member(MONKES)));
        // a purchase of ours is its collection's, read from no provider
        let b = crate::frogbuy::Bought { inscription: monke.clone(), outpoint: o(7), seller: o(8), spend_sats: 1, fee_sats: 1, rate: 1, cost_wei: 1,
            raw: vec![], setup_txid: None };
        entry_mut(&mut top, MONKES).frogs.push(crate::frogbuy::FrogRec { id: 0, target: None, phase: crate::frogbuy::FPhase::Held { buy: b } });
        assert_eq!(route(&o(7), None, &top, &roots), Some(Route::Bought(MONKES)));
        // applied: every output seen once; the members in their collections, the non-members once at the top; applied again, nothing
        let contents = [holding(&frog), holding(&monke), holding(&omb), two.clone(), plain()];
        let routes: Vec<(Outpoint, Route)> = (1..=5u8).map(|n| (o(n), route(&o(n), Some(&contents[n as usize - 1]), &top, &roots).unwrap())).collect();
        let unspent: Vec<Outpoint> = (1..=7).map(o).collect();
        let lines = apply(&mut top, &unspent, routes.clone());
        assert_eq!(lines.iter().filter(|l| l.contains("refused once")).count(), 2, "{lines:?}");
        assert_eq!(apply(&mut top, &unspent, routes), Vec::<String>::new(), "seen: never classified twice");
        assert_eq!((top.seen.len(), top.unmarked.clone()), (5, vec![o(3), o(4)]));
        assert_eq!((entry(&top, FROGS).unwrap().unmarked.clone(), entry(&top, MONKES).unwrap().unmarked.clone()), (vec![o(1)], vec![o(2)]));
        // spent outputs leave every list (only o(2) is unspent now)
        apply(&mut top, &[o(2)], vec![]);
        assert_eq!((top.seen.clone(), top.unmarked.len(), entry(&top, FROGS).unwrap().unmarked.len(), entry(&top, MONKES).unwrap().unmarked.clone()),
            (vec![o(2)], 0, 0, vec![o(2)]));
    }

    /// Part 5, default-live (operator decision 2026-10-12, audit C2 §4): an Inscriptions listing with no life is LIVE; the operator's
    /// brake (`frog_sleep`) makes one dormant, only with nothing open; `frog_wake` lifts it; the one that trades is unaffected by the
    /// other's sleep.
    #[test]
    fn default_live_a_listing_is_live_with_no_life_and_sleeps_only_as_the_operators_brake() {
        let mut top = FrogState::default();
        let (f, m) = (listing(FROGS, "bitcoin-frogs"), listing(MONKES, "nodemonkes"));
        assert!(is_live(&top, &f.wx_mint) && is_live(&top, &m.wx_mint), "no life: live (default-live), nothing to wake");
        assert!(wake(&mut top, &m, 6).unwrap().contains("already live"));
        // NodeMonkes has a frog wanted: no sleep; Frogs has none: it sleeps, its reading dropped, NodeMonkes stays live
        entry_mut(&mut top, MONKES).frogs.push(crate::frogbuy::FrogRec { id: 3, target: None, phase: crate::frogbuy::FPhase::Wanted { cap_wei: 1 } });
        assert!(sleep(&mut top, &m, 7).unwrap_err().contains("frog 3"));
        entry_mut(&mut top, FROGS).floor_reading = Some(crate::frogsol::FrogReading { sats: 1, confidence_bps: 1, depth_items: 1, at: 1 });
        sleep(&mut top, &f, 7).unwrap();
        assert!(top.dorm.as_ref().unwrap().life(&life_key(&f.wx_mint)).unwrap().is_dormant(), "the sleep is a DORMANT life");
        assert!(!is_live(&top, &f.wx_mint) && is_live(&top, &m.wx_mint));
        assert_eq!(entry(&top, FROGS).unwrap().floor_reading, None);
        assert_eq!(live(&top, &[f.clone(), m.clone()]), vec![m.clone()], "the tick reads and buys for NodeMonkes only");
        wake(&mut top, &f, 8).unwrap();
        assert!(matches!(top.dorm.as_ref().unwrap().life(&life_key(&f.wx_mint)).unwrap().by, Some(WakeBy::Operator)));
        assert_eq!(live(&top, &[f, m]).len(), 2);
    }

    /// Audit C2-02: `frog_sleep` is refused while the collection holds the one Relay leg: a conversion finished by its give-back whose
    /// written-off order is still watched, a stuck one still watched.
    #[test]
    fn c2_02_no_sleep_while_a_conversion_holds_the_relay_leg() {
        use crate::relayconv::{CPhase, ConvRec, LateConv, Watch};
        let mut top = FrogState::default();
        let f = listing(FROGS, "bitcoin-frogs");
        let w = Watch { order_id: [9; 32], value: 5, min_sats: 1, deadline: 1, balance_after: 1 };
        let mut c = ConvRec::new(0, [7; 20]);
        c.phase = CPhase::Returned { wei: 5 };
        c.late = Some(LateConv { watch: w.clone(), written_off_at: 1, balance_after: 1, refund_wei: None, credited: None, filled: None, released_at: None });
        entry_mut(&mut top, FROGS).convs = Some(vec![c.clone()]);
        assert!(sleep(&mut top, &f, 2).unwrap_err().contains("written-off"));
        assert!(is_live(&top, &f.wx_mint));
        // released by the operator (relay_late_release): it may sleep
        c.late.as_mut().unwrap().released_at = Some(3);
        entry_mut(&mut top, FROGS).convs = Some(vec![c]);
        sleep(&mut top, &f, 4).unwrap();
        assert!(!is_live(&top, &f.wx_mint) && !held_elsewhere(&top, MONKES));
    }

    /// Audit C2-05: an output whose contents cannot be read is counted, set aside after `MAX_READ_FAILURES`, and never stops the others'
    /// classification; once set aside it is not read again, and a spent one leaves every list.
    #[test]
    fn c2_05_an_unreadable_output_is_set_aside_and_never_the_batchs_end() {
        let mut top = two_live();
        let roots = live_roots(&top);
        let frog = fix("bitcoin-frogs").1;
        for k in 1..crate::deposits::MAX_READ_FAILURES {
            let l = set_aside(&mut top, vec![(o(9), "the providers disagree".into())]);
            assert!(l[0].contains(&format!("({k} of")), "{l:?}");
            // the rest of the batch is routed all the same
            let r = route(&o(1), Some(&holding(&frog)), &top, &roots).unwrap();
            apply(&mut top, &[o(1), o(9)], vec![(o(1), r)]);
        }
        assert_eq!(entry(&top, FROGS).unwrap().unmarked, vec![o(1)]);
        let l = set_aside(&mut top, vec![(o(9), "the providers disagree".into())]);
        assert!(l[0].contains("set aside"), "{l:?}");
        assert_eq!((top.unread.clone(), top.read_failures.clone()), (Some(vec![o(9)]), Some(vec![])));
        apply(&mut top, &[o(1)], vec![]);
        assert_eq!(top.unread, Some(vec![]), "spent: it leaves the set-aside list");
    }

    /// Audit C2-06: a member refused because no live root was its (its collection asleep) or its proof was not handed in yet is routed
    /// again once the roots or the proofs change (`reopen`, at `frog_wake` / `frog_sleep` / `frog_proof`), and is then its collection's.
    #[test]
    fn c2_06_a_member_refused_before_its_proof_or_its_wake_is_routed_again() {
        let mut top = two_live();
        let omb = fix("omb").1;
        // OMB asleep: refused custody-wide
        let roots = live_roots(&top);
        let r = route(&o(3), Some(&holding(&omb)), &top, &roots).unwrap();
        apply(&mut top, &[o(3)], vec![(o(3), r)]);
        assert_eq!(top.unmarked, vec![o(3)]);
        // OMB woken: the output is looked at again and is OMB's now, no longer custody-wide
        wake(&mut top, &listing(OMB, "omb"), 9).unwrap();
        assert_eq!(reopen(&mut top), 1);
        let roots = live_roots(&top);
        let r = route(&o(3), Some(&holding(&omb)), &top, &roots).unwrap();
        assert_eq!(r, Route::Member(OMB));
        let lines = apply(&mut top, &[o(3)], vec![(o(3), r)]);
        assert_eq!((top.unmarked.clone(), entry(&top, OMB).unwrap().unmarked.clone()), (vec![], vec![o(3)]), "{lines:?}");
        // a non-member routed again is refused again, once (no second line, no duplicate)
        let mut top = two_live();
        let roots = live_roots(&top);
        let r = route(&o(4), Some(&holding(&omb)), &top, &roots).unwrap();
        apply(&mut top, &[o(4)], vec![(o(4), r.clone())]);
        reopen(&mut top);
        assert!(apply(&mut top, &[o(4)], vec![(o(4), r)]).is_empty());
        assert_eq!(top.unmarked, vec![o(4)]);
    }

    /// Audit C2-07: the collections take their passes in a rotation (after the last that took the leg or the line), and the Ethereum side's
    /// turn on a free leg is kept once per leg: the Bitcoin side yields it, and the Ethereum side no longer waits on `wants_relay`.
    #[test]
    fn c2_07_the_leg_and_the_frog_turn_rotate_and_the_ethereum_side_gets_its_turn() {
        let ls: Vec<Listing> = [(FROGS, "bitcoin-frogs"), (MONKES, "nodemonkes"), (OMB, "omb")].iter().map(|(i, s)| listing(*i, s)).collect();
        let ids = |v: Vec<Listing>| v.iter().map(|l| l.id).collect::<Vec<_>>();
        let mut top = FrogState::default();
        assert_eq!(ids(in_turn(&top, ls.clone())), vec![FROGS, MONKES, OMB]);
        took(&mut top, FROGS, false);
        assert_eq!(ids(in_turn(&top, ls.clone())), vec![MONKES, OMB, FROGS], "the line's last taker goes last");
        assert_eq!(top.eth_turn, None, "a line step is not the leg");
        // NodeMonkes takes the leg (an advance) while OMB and Frogs wait: next round starts at OMB; the Ethereum side's turn is due
        let mut a = take(&top, MONKES);
        let mut c = crate::relayconv::ConvRec::new(0, [1; 20]);
        c.phase = crate::relayconv::CPhase::Advancing;
        a.convs = Some(vec![c]);
        put(&mut top, MONKES, &a);
        let mut b = take(&top, OMB);
        let mut w = crate::relayconv::ConvRec::new(1, [2; 20]);
        w.phase = crate::relayconv::CPhase::NeedAdvance;
        b.convs = Some(vec![w]);
        put(&mut top, OMB, &b);
        took_leg(&mut top, MONKES);
        assert_eq!(ids(in_turn(&top, ls.clone())), vec![OMB, FROGS, MONKES]);
        assert!(!wants_relay(&top), "the Ethereum side's turn is due: OMB's wait no longer holds its buys back");
        // while NodeMonkes holds the leg nothing is yielded; once it is free the Bitcoin side yields it for ETH_TURN_SECS
        assert!(!eth_turn_now(&mut top, false, 100));
        put(&mut top, MONKES, &FrogState::default());
        assert!(eth_turn_now(&mut top, false, 200) && eth_turn_now(&mut top, false, 200 + ETH_TURN_SECS - 1));
        assert!(!eth_turn_now(&mut top, false, 200 + ETH_TURN_SECS), "the Ethereum side wanted nothing: the turn lapses");
        assert!(wants_relay(&top), "OMB waits for the leg again: the Ethereum side starts no buy");
        // the Ethereum side took it: its turn is had
        took_leg(&mut top, OMB);
        assert!(!eth_turn_now(&mut top, true, 300) && top.eth_turn.is_none());
    }

    /// Audit C2-14: a listing whose members root repeats a lower id's is never live and gets no cap; the first of the root is served.
    #[test]
    fn c2_14_a_repeated_members_root_is_never_live() {
        let top = FrogState::default();
        let a = listing(FROGS, "bitcoin-frogs");
        let b = Listing { id: 20, wx_mint: [20; 32], ..a.clone() };
        assert!(repeats_root(&[a.clone(), b.clone()], &b) && !repeats_root(&[a.clone(), b.clone()], &a));
        assert_eq!(live(&top, &[a.clone(), b]), vec![a]);
    }

    /// Default-live, must-hold 5 (audit C2 §4): before a listing's first reading its floor sink, purchase limit and minimum and OW creators
    /// are set from the launch files where nothing is set; a controller's own settings are never overwritten.
    #[test]
    fn default_live_the_launch_defaults_are_set_where_nothing_is() {
        let mut top = FrogState::default();
        let mut fl = crate::floortrigger::FloorState::default();
        let runestone = Listing { id: 18, wx_mint: [18; 32], root: hex::decode(crate::frogsol::launch::LAUNCH[6].root).unwrap().try_into().unwrap(),
            custody: "bc1p".into() };
        let set = launch_defaults(&mut top, &mut fl, &runestone, [0x18; 20]);
        assert_eq!(set.len(), 3, "{set:?}");
        assert_eq!((fl.item_of(&[0x18; 20]), fl.limit_for(&[0x18; 20]), fl.min_for(&[0x18; 20])),
            (Some(&crate::floortrigger::FloorItem::Bitcoin), Some(10_000_000_000), Some(250_000_000)));
        assert!(top.sinks.iter().any(|s| s.sink == [0x18; 20]));
        assert_eq!(entry(&top, 18).unwrap().ow_creators, Some(vec![]), "no creator pinned: a royalty-carrying protected sale is refused");
        assert!(launch_defaults(&mut top, &mut fl, &runestone, [0x18; 20]).is_empty(), "once");
        // a controller's own limit stands
        let mut fl = crate::floortrigger::FloorState::default();
        fl.set_limit_min_in([0x18; 20], 3_000_000_000, None, crate::frogsol::ABS_MIN_PURCHASE_LAMPORTS, crate::frogsol::MIN_PURCHASE_LAMPORTS,
            crate::frogsol::RELAY_SOL_BTC_MAX_LAMPORTS, "lamports").unwrap();
        launch_defaults(&mut FrogState::default(), &mut fl, &runestone, [0x18; 20]);
        assert_eq!(fl.limit_for(&[0x18; 20]), Some(3_000_000_000));
        // Bitcoin Frogs: no limit in its launch entry (the home's 10 SOL and 1 SOL)
        let frogs = Listing { root: hex::decode(crate::frogsol::launch::LAUNCH[0].root).unwrap().try_into().unwrap(), ..listing(FROGS, "bitcoin-frogs") };
        let mut fl = crate::floortrigger::FloorState::default();
        launch_defaults(&mut FrogState::default(), &mut fl, &frogs, [0x11; 20]);
        assert_eq!((fl.item_of(&[0x11; 20]).is_some(), fl.limit_for(&[0x11; 20])), (true, None));
    }

    /// Part 1: frog ids stay unique canister-wide across collections (`next_frog` at the top), and one collection's Relay conversion holds
    /// the one leg for every other (B-L-6).
    #[test]
    fn frog_ids_are_canister_wide_and_one_relay_leg_is_shared() {
        let mut top = FrogState::default();
        let mut a = take(&top, FROGS);
        a.next_frog += 2;
        put(&mut top, FROGS, &a);
        assert_eq!(take(&top, MONKES).next_frog, 2, "the next collection's frogs start after the first's");
        let mut a = take(&top, FROGS);
        let mut c = crate::relayconv::ConvRec::new(0, [1; 20]);
        c.phase = crate::relayconv::CPhase::Advancing;
        a.convs = Some(vec![c]);
        put(&mut top, FROGS, &a);
        assert!(holds_relay(&top) && held_elsewhere(&top, MONKES) && !held_elsewhere(&top, FROGS));
    }

    /// The upgrade: a frog state saved before the change (no `colls`, no `dorm`) restores in the upgrade tuple, and its single state moves
    /// into the entry of the one Inscriptions listing, which stays live; what is custody-wide stays at the top; the custodian's cursor
    /// likewise. Once.
    #[test]
    fn the_single_state_of_before_restores_and_moves_into_its_listing() {
        #[derive(candid::CandidType, Default)]
        struct Old { factory: Option<[u8; 20]>, desk: Option<[u8; 20]>, coin_cursor: u64, sinks: Vec<crate::itemsink::SinkRec>, poll: u64,
            request_cursor: u64, requests: Vec<crate::mintdesk::ReqRec>, frogs: Vec<crate::frogbuy::FrogRec>, next_frog: u64,
            deposits: Vec<crate::deposits::DepositRec>, seen: Vec<Outpoint>, unmarked: Vec<Outpoint>, proofs: Vec<(String, Vec<[u8; 32]>)>,
            mint_times: Vec<u64>, return_times: Vec<u64>, pending: Option<crate::frogflow::FrogEvm>, unread: Option<Vec<Outpoint>>,
            read_failures: Option<Vec<(Outpoint, u32)>>, buy_clock: Option<Vec<(u64, u64)>>, convs: Option<Vec<crate::relayconv::ConvRec>>,
            protected: Option<Vec<crate::frogflow::ProtectedRec>>, ow_creators: Option<Vec<String>>, sol_step: Option<crate::solhome::LinePending>,
            floor_reading: Option<crate::frogsol::FrogReading>, values: Option<Vec<(u64, u128)>> }
        let frog = crate::frogbuy::FrogRec { id: 4, target: None, phase: crate::frogbuy::FPhase::Wanted { cap_wei: 7 } };
        let old = Old { frogs: vec![frog.clone()], next_frog: 5, seen: vec![o(1)], unmarked: vec![o(2)], proofs: vec![("x".into(), vec![[3; 32]])],
            ow_creators: Some(vec!["bc1q".into()]), values: Some(vec![(4, 9)]), ..Default::default() };
        // the upgrade tuple as an old canister saved it: its seventh element the old frog state
        type OldSaved = (Option<crate::canister::Init>, Vec<crate::state::Purchase>, bool, Vec<String>, Option<Custody>,
            Option<crate::chains::NftState>, Option<Old>);
        let saved: OldSaved = (None, vec![], false, vec![], None, None, Some(old));
        let (back,): (crate::canister::Saved,) = candid::decode_args(&candid::encode_args((saved,)).unwrap()).expect("the old encoding decodes");
        let mut top = back.6.expect("the frog state restores");
        assert_eq!((top.colls.clone(), top.dorm.clone(), top.next_frog), (None, None, 5));
        let l = listing(FROGS, "bitcoin-frogs");
        assert!(migrate(&mut top, &l, 9));
        assert!(!migrate(&mut top, &l, 10), "once");
        let e = entry(&top, FROGS).unwrap();
        assert_eq!((e.frogs.clone(), e.proofs.len(), e.ow_creators.clone(), e.values.clone()), (vec![frog], 1, Some(vec!["bc1q".into()]), Some(vec![(4, 9)])));
        assert_eq!((top.frogs.len(), top.seen.clone(), top.unmarked.clone(), top.next_frog), (0, vec![o(1)], vec![o(2)], 5), "custody-wide stays");
        assert!(is_live(&top, &l.wx_mint), "the collection that was trading stays live");
        // with its entries, the whole state round-trips the upgrade encoding (the recursive map included)
        let again: FrogState = candid::decode_one(&candid::encode_one(&top).unwrap()).unwrap();
        assert_eq!(again, top);
        // a fresh install's empty state: nothing to migrate, and (default-live) the listing is live all the same
        let mut fresh = FrogState::default();
        assert!(migrate(&mut fresh, &l, 9) && is_live(&fresh, &l.wx_mint) && fresh.dorm.is_none());
        // the custodian's cursor and redemptions
        let mut c = Custody { cursor: 3, redemptions: vec![], logs: None, colls: None };
        assert!(migrate_custody(&mut c, FROGS) && !migrate_custody(&mut c, FROGS));
        assert_eq!((c.cursor, custody_entry(&c, FROGS).cursor, custody_entry(&c, MONKES).cursor), (0, 3, 0));
        let again: Custody = candid::decode_one(&candid::encode_one(&c).unwrap()).unwrap();
        assert_eq!(again, c);
    }

    /// Part 6, audit C2-08: the default caps name EVERY Inscriptions listing in the books at stage 0 (`sol:<wX mint>`), one the operator
    /// put to sleep too (the brake never changes the caps); the global is re-sized by the exit rule (A4-L-1: what the outflow limits let
    /// out within the delay less a day).
    #[test]
    fn the_default_caps_name_every_listing_and_the_exit_rule_sizes_the_global() {
        crate::desk::state::reset();
        // Frogs, NodeMonkes, OMB, Puppets, Bitcoin Punks, Natcats (the roots stand in: the caps read no root)
        crate::desk::state::with_mut(|s| {
            for i in 0..6u16 {
                let id = FROGS + i;
                s.books.add_collection(id, ListingKind::Inscriptions { membership_root: [id as u8; 32], custody: "bc1p49upe77txy03ysp4m3r4m992tf27d2xgce7yk6w53qg5jtc85exqtkpg3t".into() },
                    [id as u8; 32], books::default_params(5_000_000_000)).unwrap();
            }
        });
        let all = listings();
        assert_eq!(all.len(), 6);
        crate::frognet::FROGS.with(|f| {
            let mut top = f.borrow_mut();
            *top = FrogState::default();
            sleep(&mut top, &all[5], 1).unwrap();
        });
        let keys = crate::frognet::listing_cap_keys();
        assert_eq!(keys.len(), 6, "Natcats asleep keeps its cap");
        let home = crate::canister::Home::Solana { cluster: "mainnet-beta".into(), rpc_urls: vec!["https://a".into(), "https://b".into()],
            rook_core: "books".into(), nonce_seed: "rook-home-0".into(), blockhash_fallback: None, rook_backing: None, drand_urls: None };
        let i = crate::canister::Init { home: Some(home), ..crate::canister::tests::good() };
        let caps = crate::canister::caps_of(&i);
        for k in &keys {
            let c = caps.cap(k).unwrap_or_else(|| panic!("{k} has no cap"));
            assert_eq!(c.max_items, crate::custodycap::FIRST_MAX_ITEMS);
        }
        // the brake changes nothing: woken, the caps are the same
        crate::frognet::FROGS.with(|f| wake(&mut f.borrow_mut(), &all[5], 2).unwrap());
        assert_eq!(crate::canister::caps_of(&i), caps);
        let room = crate::canister::exit_room(&crate::canister::limits_of(&i));
        assert_eq!((room, caps.global_max_items), (12, 12), "6 × 3 items named, 12 can leave within the delay: the global is 12");
        crate::frognet::FROGS.with(|f| *f.borrow_mut() = FrogState::default());
        crate::desk::state::reset();
    }

    /// Default-live (audit C2 §4): a fresh install with the 15 Ordinals listings of the launch files: every one live with no wake, none
    /// mints without a floor (a module never posted wants nothing), a first floor outside the band of its launch floor refused, the caps
    /// naming all 15 at stage 0 and the global held to the exit rule's 12.
    #[test]
    fn default_live_a_fresh_install_with_15_listings() {
        crate::desk::state::reset();
        let launch = crate::frogsol::launch::LAUNCH;
        crate::desk::state::with_mut(|s| {
            for (k, d) in launch.iter().enumerate() {
                let id = 11 + k as u16;
                let root: [u8; 32] = hex::decode(d.root).unwrap().try_into().unwrap();
                s.books.add_collection(id, ListingKind::Inscriptions { membership_root: root, custody: "bc1p49upe77txy03ysp4m3r4m992tf27d2xgce7yk6w53qg5jtc85exqtkpg3t".into() },
                    [id as u8; 32], books::value_params(d.floor_lamports, 30, 24, 5_000_000_000).unwrap()).unwrap();
            }
        });
        let all = listings();
        assert_eq!(all.len(), 15);
        crate::frognet::FROGS.with(|f| *f.borrow_mut() = FrogState::default());
        let top = FrogState::default();
        assert_eq!(live(&top, &all).len(), 15, "all live, no frog_wake");
        // none mints (nor buys) without a floor: the module was never posted
        crate::desk::state::with(|s| {
            for l in &all {
                let c = s.books.get(l.id).unwrap();
                let v = crate::solhome::books_sink_view(c, false, 1_800_000_000, 1, crate::frogsol::MAX_PURCHASE_LAMPORTS);
                assert!(!v.open && v.want == 0 && crate::itemsink::may_start(&v, 0).is_err(), "{}: {}", l.id, v.status);
            }
        });
        // a first floor: inside the band of its launch floor posted, outside refused
        for (l, d) in all.iter().zip(launch.iter()) {
            let f = d.floor_lamports as u128;
            assert!(crate::frogsol::launch::first_floor_ok(&l.root, f * 11 / 10).is_ok(), "{}", d.slug);
            assert!(crate::frogsol::launch::first_floor_ok(&l.root, f * 2).is_err(), "{}", d.slug);
            assert!(crate::frogsol::launch::first_floor_ok(&l.root, f / 3).is_err(), "{}", d.slug);
        }
        // the caps: 15 at stage 0, the global by the exit rule
        let home = crate::canister::Home::Solana { cluster: "mainnet-beta".into(), rpc_urls: vec!["https://a".into(), "https://b".into()],
            rook_core: "books".into(), nonce_seed: "rook-home-0".into(), blockhash_fallback: None, rook_backing: None, drand_urls: None };
        let i = crate::canister::Init { home: Some(home), ..crate::canister::tests::good() };
        let caps = crate::canister::caps_of(&i);
        for l in &all {
            assert_eq!(caps.cap(&crate::custodycap::sol_key(&l.wx_mint)).map(|c| c.max_items), Some(crate::custodycap::FIRST_MAX_ITEMS));
        }
        let room = crate::canister::exit_room(&crate::canister::limits_of(&i));
        assert_eq!((room, caps.global_max_items), (12, 12), "45 items named; 12 can leave within the delay: the whole canister holds 12");
        crate::desk::state::reset();
    }

    /// Part 7 (docs/v9/ordinals-collections.md §7): a collection's own minimum purchase under 1 SOL, never under what Relay was measured
    /// filling; the EVM home keeps its policy.
    #[test]
    fn a_collection_minimum_purchase_under_one_sol_and_never_under_what_relay_fills() {
        use crate::floortrigger::FloorState;
        use crate::frogsol::{check_limits_min, ABS_MIN_PURCHASE_LAMPORTS, MIN_PURCHASE_LAMPORTS, RELAY_SOL_BTC_MAX_LAMPORTS};
        let sol = |x: f64| (x * 1e9) as u128;
        let mut fl = FloorState::default();
        let set = |fl: &mut FloorState, max: u128, min: Option<u128>|
            fl.set_limit_min_in([1; 20], max, min, ABS_MIN_PURCHASE_LAMPORTS, MIN_PURCHASE_LAMPORTS, RELAY_SOL_BTC_MAX_LAMPORTS, "lamports");
        assert!(set(&mut fl, sol(10.0), Some(sol(0.1))).unwrap_err().contains("outside"), "under 0.15 SOL: under what Relay fills");
        assert!(set(&mut fl, sol(10.0), Some(sol(1.5))).unwrap_err().contains("outside"), "over the home's 1 SOL");
        assert!(set(&mut fl, sol(0.2), Some(sol(0.25))).unwrap_err().contains("under"), "the maximum under the minimum");
        set(&mut fl, sol(10.0), Some(sol(0.25))).unwrap(); // Runestone, RSIC
        assert_eq!((fl.limit_for(&[1; 20]), fl.min_for(&[1; 20])), (Some(sol(10.0)), Some(sol(0.25))));
        assert!(check_limits_min(sol(0.3), Some(sol(10.0)), fl.min_for(&[1; 20]), true).is_ok(), "0.3 SOL for a sink with min 0.25");
        assert!(check_limits_min(sol(0.3), Some(sol(10.0)), None, true).unwrap_err().contains("minimum"), "and not for one without");
        assert!(check_limits_min(sol(0.2), Some(sol(10.0)), Some(sol(0.25)), true).is_err());
        set(&mut fl, sol(10.0), None).unwrap();
        assert_eq!(fl.min_for(&[1; 20]), None, "back to the home's 1 SOL");
        assert_eq!(check_limits_min(1, None, Some(sol(0.25)), false), crate::frogbuy::check_limits_for(1, None), "the EVM home's policy");
        // a limit saved before the minimum existed restores without one
        #[derive(candid::CandidType)]
        struct OldLimit { sink: [u8; 20], max_wei: u128 }
        let l: crate::floortrigger::PurchaseLimit = candid::decode_one(&candid::encode_one(OldLimit { sink: [2; 20], max_wei: 5 }).unwrap()).unwrap();
        assert_eq!((l.max_wei, l.min_wei), (5, None));
    }
}
