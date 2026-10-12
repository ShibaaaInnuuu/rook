//! W8 (docs/v9/canister-only-solana.md §8): `ethflow`'s home on Solana: the canister keeps the books itself (decision D-l). Every
//! home read comes from the books; every former rook_core instruction is a book update, applied at once and reported to the flow as a
//! landed step (`mined_home`), except the two that move SOL: the advance (`M[i] → K_main`, signed by `K_coll[i]` on the money line,
//! the books advanced BEFORE the signature and taken back if it never lands: §4.4) and the way back (`K_main → M[i]`, the books
//! credited when it lands). The Relay deposit from `K_main` is `home_sol`'s. A child of `ethflow`, like `home_sol`.
use super::*;
use crate::books::ledger::{Destination as BDest, MintItemArgs as BMintArgs, MintTo, Stage as BStage};
use crate::books::{self, ListingKind};
use crate::chainlink::ledger::OwedWrite;
use crate::desk::state as desk;
use crate::solhome as sh;
use crate::solkey::KeyPath;
use crate::soltx;

/// The configured collection whose wrapper (or module) record key is `k`.
fn col_by<I: EthIo>(io: &I, k: &[u8; 20], wrapper: bool) -> Result<Col, String> {
    let cfg = io.config().ok_or("eth_nft is not configured")?;
    cfg.cols()?.into_iter().find(|c| if wrapper { c.wrapper == *k } else { c.module == *k })
        .ok_or_else(|| format!("no configured collection's {} is {}", if wrapper { "wrapper" } else { "module" }, h(k)))
}

/// The books' collection id of a configured collection: the EthNft listing of its ERC-721 address ON ITS CHAIN (audit C3-07: a listing
/// naming the address on another chain, a pin's typo or a twin collection at the same address there, is never this collection's: the
/// reconciler and the site would locate the item on the listing's chain).
pub(super) fn coll_of(col: &Col) -> Result<u16, String> {
    desk::with(|s| books_id_of(&s.books, col)).ok_or_else(|| format!("collection 0x{} is not listed in the books on chain {}", hex::encode(col.collection),
        col.chain_id))
}

/// [`coll_of`] on `books` (audit C3-07): the chain and the address both match.
pub fn books_id_of(books: &books::Books, col: &Col) -> Option<u16> {
    books.collections.iter().find(|(_, c)| matches!(c.kind, ListingKind::EthNft { chain_id, collection, .. }
        if collection == col.collection && chain_id == col.chain_id)).map(|(id, _)| *id)
}

fn with_col<R>(col: &Col, f: impl FnOnce(&books::Collection, &books::Books) -> R) -> Result<R, String> {
    let id = coll_of(col)?;
    desk::with(|s| s.books.get(id).map(|c| f(c, &s.books)).map_err(|e| format!("{e}")))
}

fn ticket_of(col: &Col, id: u128) -> Result<books::ledger::Ticket, String> {
    let id = u64::try_from(id).map_err(|_| format!("ticket {id} is over a u64"))?;
    with_col(col, |c, _| c.ledger.ticket(id).cloned().map_err(|e| format!("ticket {id}: {e}")))?
}

pub(super) async fn ticket_v<I: EthIo>(io: &mut I, wrapper: [u8; 20], id: u128) -> Result<TicketV, String> {
    let col = col_by(io, &wrapper, true)?;
    let t = ticket_of(&col, id)?;
    Ok(TicketV {
        stage: match t.stage {
            BStage::None => Stage::None,
            BStage::Pinned => Stage::Pinned,
            BStage::Resolved => Stage::Resolved,
            BStage::Delivered => Stage::Delivered,
            BStage::Refunded => Stage::Refunded,
        },
        owner: t.owner[..20].try_into().expect("20"),
        deliver_by: t.deliver_by.max(0) as u64,
        item_id: t.item_id as u128,
        destination: match t.destination {
            BDest::Eth(a) => a,
            BDest::Burn => sh::BURN_DESTINATION,
            BDest::Taproot(_) | BDest::Sol(_) => [0; 20],
        },
    })
}

pub(super) async fn token_of<I: EthIo>(io: &mut I, wrapper: [u8; 20], item_id: u128) -> Result<Word, String> {
    let col = col_by(io, &wrapper, true)?;
    let id = u32::try_from(item_id).map_err(|_| format!("item {item_id} is over a u32"))?;
    with_col(&col, |c, _| c.ledger.items.get(&id).map(|x| x.key).ok_or_else(|| format!("item {id} does not exist")))?
}

pub(super) async fn ticket_count<I: EthIo>(_io: &mut I, col: &Col) -> Result<u128, String> {
    with_col(col, |c, _| c.ledger.ticket_count as u128)
}

pub(super) async fn sent_tx<I: EthIo>(io: &mut I, wrapper: [u8; 20], t: u128) -> Result<Word, String> {
    Ok(ticket_of(&col_by(io, &wrapper, true)?, t)?.sent_tx)
}

pub(super) async fn undeliverable<I: EthIo>(io: &mut I, wrapper: [u8; 20], t: u128) -> Result<bool, String> {
    Ok(ticket_of(&col_by(io, &wrapper, true)?, t)?.undeliverable)
}

pub(super) async fn module_floor<I: EthIo>(io: &mut I, module: [u8; 20]) -> Result<(u128, u128, u64), String> {
    let col = col_by(io, &module, false)?;
    let (accepted, target, at) = with_col(&col, |c, _| (c.module.accepted, c.module.target, c.module.posted_at))?;
    let rate = io.sol_rate().await?;
    Ok((crate::rates::lamports_to_wei(accepted as u128, rate), crate::rates::lamports_to_wei(target as u128, rate), at.max(0) as u64))
}

pub(super) async fn accepted<I: EthIo>(io: &mut I, module: [u8; 20]) -> Result<u128, String> {
    with_col(&col_by(io, &module, false)?, |c, _| c.module.accepted as u128)
}

pub(super) async fn transit_open<I: EthIo>(io: &mut I, module: [u8; 20]) -> Result<bool, String> {
    with_col(&col_by(io, &module, false)?, |c, _| c.module.transit_units != 0)
}

pub(super) async fn advanced<I: EthIo>(io: &mut I, module: [u8; 20]) -> Result<u128, String> {
    with_col(&col_by(io, &module, false)?, |c, _| c.module.advanced as u128)
}

/// Whether the ledger holds `token` (its held marker names a live item), and, with `tx`, from that very transaction.
pub(super) async fn wrapper_holds<I: EthIo>(io: &mut I, wrapper: [u8; 20], token: &Word, tx: Option<&[u8; 32]>) -> Result<bool, String> {
    let col = col_by(io, &wrapper, true)?;
    with_col(&col, |c, _| {
        let Some(id) = c.ledger.held_markers.get(token) else { return false };
        match (tx, c.ledger.items.get(id)) {
            (None, Some(_)) => true,
            (Some(tx), Some(x)) => x.key == *token && x.source_tx == *tx,
            _ => false,
        }
    })
}

pub(super) async fn gate<I: EthIo>(io: &mut I, c: &Col) -> Result<home_sol::Gate, String> {
    let id = coll_of(c)?;
    let clock = io.home_clock().await?;
    let (now, slot) = (clock.time as i64, clock.parent_block);
    let (col, kd) = desk::with(|s| s.books.get(id).map(|c| (c.clone(), c.kd(s.books.keeper_paused))).map_err(|e| format!("{e}")))?;
    let bare = sh::books_sink_view(&col, kd, now, slot, u128::MAX);
    if crate::itemsink::may_start(&bare, 0).is_err() {
        return Ok(home_sol::Gate { view: bare, accepted_wei: 0, rate: 0 });
    }
    let rate = io.sol_rate().await?;
    let limit = crate::rates::wei_to_lamports(c.max_purchase_wei, rate);
    Ok(home_sol::Gate { view: sh::books_sink_view(&col, kd, now, slot, limit), accepted_wei: crate::rates::lamports_to_wei(col.module.accepted as u128, rate), rate })
}

use home_sol::u64_of;

fn books_err(e: books::BooksError) -> String {
    format!("the books refused: {e}")
}

/// A home step on this home. A step that moves SOL (the advance, the way back) is signed on the money line and resolves through the
/// line's outcome ([`landed`]); every other step is a book update, final at once, reported to the flow as landed.
pub(super) async fn send<I: EthIo>(io: &mut I, st: &mut EthState, step: Step, col: &Col, call: home_sol::Call) -> Result<String, String> {
    use home_sol::Call;
    if io.paused() && !matches!(step, Step::Record { .. } | Step::Sent { .. } | Step::Mark { .. }) {
        return Err("paused: nothing new is signed".into());
    }
    let id = coll_of(col)?;
    if desk::collection_busy(id) && !matches!(call, Call::Deposit(_)) {
        return Err(format!("collection {id} has an unfinished chain link: the step waits for it (review F1)"));
    }
    let now = io.now() as i64;
    let cfg = ();
    let book = |f: &dyn Fn(&mut books::Books) -> Result<String, books::BooksError>| -> Result<String, String> {
        desk::with_mut(|s| f(&mut s.books)).map_err(books_err)
    };
    let text = match call {
        Call::Deposit(_) => return Err(format!("{step:?}: not a book step of this home")),
        Call::Served { id } => return Err(format!("{step:?} (desk request {id}): the books home has no wake desk (O-11)")),
        Call::Advance(a) => {
            // the advance: booked first, then `M[i] → K_main` signed by the collection's key; taken back if it never lands
            let lamports = u64_of(a, "the advance")?;
            if st.sol_step.is_some() {
                return Err("this side's step is still out on the Solana line (one at a time)".into());
            }
            let what = sh::LineStep::Eth(step.clone()).label();
            let me = sh::ready(io, sh::LineOwner::Eth, io.now(), &what).await?;
            let module = io.sol_key_of(&KeyPath::Coll(id as u32)).await?;
            desk::with_mut(|s| s.books.advance(id, lamports, now)).map_err(books_err)?;
            let ixs = vec![soltx::transfer(module, me, lamports)];
            match sh::sign_keys(io, sh::LineStep::Eth(step), io.now(), ixs, vec![KeyPath::Coll(id as u32)]).await {
                Ok((text, p)) => {
                    st.sol_step = Some(p);
                    io.save(st);
                    return Ok(text);
                }
                Err(e) => {
                    desk::with_mut(|s| s.books.return_advance(id, lamports)).ok();
                    return Err(e);
                }
            }
        }
        Call::GiveBack { lamports, sweep } => {
            // the way back: `K_main → M[i]`, booked when it lands (`fund_reserve` for a sweep, `return_advance` otherwise)
            let lamports = u64_of(lamports, if sweep { "the sweep" } else { "the return" })?;
            if st.sol_step.is_some() {
                return Err("this side's step is still out on the Solana line (one at a time)".into());
            }
            let what = sh::LineStep::Eth(step.clone()).label();
            let me = sh::ready(io, sh::LineOwner::Eth, io.now(), &what).await?;
            let module = io.sol_key_of(&KeyPath::Coll(id as u32)).await?;
            let (text, p) = sh::sign(io, sh::LineStep::Eth(step), io.now(), vec![soltx::transfer(me, module, lamports)]).await?;
            st.sol_step = Some(p);
            io.save(st);
            return Ok(text);
        }
        Call::Mint { token, tx } => book(&|b| {
            let args = BMintArgs { key: token, source: books::math::keccak(&[&token, &tx]), tx, vout: 0, deposit: false, inscription_id: String::new(), proof: vec![] };
            let item = b.mint_item(id, args, MintTo::Module, now)?;
            Ok(format!("item {item} minted to the module in the books"))
        })?,
        Call::Settle(cost) => {
            let cost = u64_of(cost, "the cost")?;
            book(&|b| b.settle_advance(id, cost, now).map(|_| format!("settled at {cost} lamports in the books")))?
        }
        Call::Record { ticket, tx } => {
            let t = u64_of(ticket, "the ticket")?;
            book(&|b| b.record_delivery(id, t, tx).map(|_| format!("ticket {t} recorded delivered in the books")))?
        }
        Call::Mark { ticket } => {
            let t = u64_of(ticket, "the ticket")?;
            book(&|b| b.mark_undeliverable(id, t).map(|_| format!("ticket {t} marked undeliverable in the books")))?
        }
        Call::Sent { ticket, tx } => {
            let t = u64_of(ticket, "the ticket")?;
            book(&|b| b.note_sent(id, t, tx).map(|_| format!("ticket {t} noted sent in the books")))?
        }
        Call::Post { price, confidence, depth } => {
            book(&|b| b.post_floor(id, price, confidence, depth, now).map(|(a, t)| format!("floor posted in the books: accepted {a}, target {t}")))?
        }
    };
    // final at once: an outcome on the line, landed, which the next tick's `resolve` reports to the flow like any other step's
    // (the caller marks its record "out" after this returns; the flow's own landing logic then moves it on)
    let mut l = io.sol_line();
    l.outcomes.push(sh::LineOutcome { step: sh::LineStep::Eth(step), intent: 0, ok: Some(true), signature: "books".into(), at: io.now() });
    io.sol_line_save(&l);
    let _ = cfg;
    Ok(format!("{text} (final; the flow reads it next tick)"))
}

/// The line's outcome of a step that moved SOL: the books follow (an advance that never landed is taken back; a way back that landed
/// is credited). Before the flow's own `mined_home`. Audit B1-06: through `desk::book_outcome`, never under a desk link's snapshot.
pub(super) fn landed(st: &EthState, cfg: &EthNftConfig, step: &Step, ok: bool) {
    match step {
        Step::Advance { buy } if !ok => {
            if let Some(b) = st.buy(*buy) {
                if let (Some(col), Ok(lamports)) = (col_by_buy(cfg, b), u64::try_from(b.advanced)) {
                    if let Ok(id) = coll_of(&col) {
                        let _ = desk::book_outcome(id, OwedWrite::ReturnAdvance { lamports });
                    }
                }
            }
        }
        Step::GiveBack { buy } if ok => {
            if let Some(b) = st.buy(*buy) {
                if let (BPhase::Returning { wei }, Some(col)) = (&b.phase, col_by_buy(cfg, b)) {
                    if let (Ok(id), Ok(lamports)) = (coll_of(&col), u64::try_from(*wei)) {
                        let w = if b.is_sweep() { OwedWrite::FundReserve { lamports } } else { OwedWrite::ReturnAdvance { lamports } };
                        let _ = desk::book_outcome(id, w);
                    }
                }
            }
        }
        _ => {}
    }
}

fn col_by_buy(cfg: &EthNftConfig, b: &EthBuy) -> Option<Col> {
    // the buy's collection address is the books' key; the record keys are the configured ones. Audit C3-07: on this side's chain (the
    // collection's own when it is still configured)
    let chain_id = cfg.chain_of_addr(&b.collection).unwrap_or(cfg.chain_id);
    Some(Col { collection: b.collection, wrapper: b.wrapper, module: b.module, max_purchase_wei: 0, max_premium_bps: 0, books: true, chain_id,
        nft_bps: 10_000 })
}

/// The positions a collection holds (the dormancy's "a position is open"): its wX supply, items held, open tickets, an advance in
/// transit, and coins found under its config and not yet backed.
pub(super) async fn positions<I: EthIo>(_io: &mut I, c: &Col) -> Result<Vec<String>, String> {
    let id = coll_of(c)?;
    let (supply, held, tickets, transit) = with_col(c, |c, _| {
        let l = &c.ledger;
        ((l.held.saturating_sub(l.open_tickets) as u128) * books::ONE as u128 + l.sol_backed as u128, l.held as u128, l.open_tickets as u128, c.module.transit_units as u128)
    })?;
    let unbacked = crate::coins::state::with(|s| s.coins.values().filter(|x| x.coll == id as u32 && x.stage != crate::coins::state::CoinStage::Backed).count()) as u128;
    let mut open = Vec::new();
    for (n, x) in [("wX supply", supply), ("items held", held), ("open tickets", tickets), ("an advance in transit", transit), ("coins not yet backed", unbacked)] {
        if x != 0 {
            open.push(format!("{n} {x}"));
        }
    }
    Ok(open)
}

/// Coins of the collection that graduated and are not backed yet (the coins module backs them; the watchdog only counts).
pub(super) async fn graduations<I: EthIo>(_io: &mut I, c: &Col) -> Result<usize, String> {
    let id = coll_of(c)?;
    Ok(crate::coins::state::with(|s| s.coins.values().filter(|x| x.coll == id as u32 && x.stage == crate::coins::state::CoinStage::Migrated).count()))
}

/// One due reading posted (`post_floor` in the books): our reading (wei per item) in lamports per whole unit at this tick's rate,
/// clamped into the module's bounds. The canister is the poster.
pub(super) async fn post<I: EthIo>(io: &mut I, st: &mut EthState, i: usize, c: &Col) -> Result<String, String> {
    let r = st.readings[i].clone();
    let id = coll_of(c)?;
    let now = io.now() as i64;
    let (bounds, kd) = desk::with(|s| s.books.get(id).map(|c| (c.module.floor_bounds(now).unwrap_or((0, u64::MAX)), c.kd(s.books.keeper_paused))).map_err(|e| format!("{e}")))?;
    if kd {
        st.readings[i].due = false;
        return Err("the poster is paused: nothing is posted".into());
    }
    let rate = io.sol_rate().await?;
    let price = sh::post_price(r.post.price, rate, bounds);
    if price == 0 {
        return Err(format!("our reading of {} wei is 0 lamports at {rate} lamports per ETH: nothing is posted", r.post.price));
    }
    let raw = crate::rates::wei_to_lamports(r.post.price, rate);
    let s = send(io, st, Step::Post { collection: c.collection }, c, home_sol::Call::Post { price, confidence: r.post.confidence, depth: r.post.depth }).await?;
    st.readings[i].due = false;
    Ok(format!("eth floor 0x{}: post_floor({price} lamports per unit, {}, {}) from {} wei at {rate} lamports per ETH{}: {s}", hex::encode(c.collection),
        r.post.confidence, r.post.depth, r.post.price, if raw != price as u128 { format!(" (clamped from {raw} into the module's bounds [{}, {}])",
            bounds.0, bounds.1) } else { String::new() }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Audit C3-07: a configured collection is the books' listing naming its chain AND its address; a listing of the same address on another
    /// chain (a pin's typo, a twin collection there) is never it.
    #[test]
    fn fixed_c3_07_the_books_listing_is_matched_by_chain_and_address() {
        let coll = [0xe3; 20];
        let mut b = books::Books::default();
        b.add_collection(1, ListingKind::EthNft { chain_id: 1, collection: coll, custody: [9; 20] }, [0xC0; 32], books::default_params(1)).unwrap();
        let col = |chain_id| Col { collection: coll, wrapper: [1; 20], module: [2; 20], max_purchase_wei: 1, max_premium_bps: 0, books: true, chain_id,
            nft_bps: 10_000 };
        assert_eq!(books_id_of(&b, &col(1)), Some(1));
        assert_eq!(books_id_of(&b, &col(4663)), None, "the address on 4663 is not the Ethereum listing");
        b.add_collection(2, ListingKind::EthNft { chain_id: 4663, collection: coll, custody: [9; 20] }, [0xC1; 32], books::default_params(1)).unwrap();
        assert_eq!((books_id_of(&b, &col(1)), books_id_of(&b, &col(4663))), (Some(1), Some(2)));
    }
}

