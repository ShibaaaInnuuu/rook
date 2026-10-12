//! A Solana NFT purchase on the books (docs/v9/solana-nfts-canister.md §1 step 2, §3; docs/v9/canister-only-solana.md §8): the advance
//! `M[i] → K_main` and the purchase in ONE transaction on the money line, signed by `K_main` (payer, buyer, custody) and `K_coll[i]`
//! (the advance's source). The advance is a chain link (`OpKind::Advance`, `Scope::Module`, `Effects { lamports_out }`): booked
//! (`Books::advance`) before the signature, gated by `pre_sign` against the chain at finalized, undone if the transaction never lands
//! or fails (nothing moved: one transaction). The transaction is judged whole ([`judge_tx`]): the venue's judge on the purchase
//! (`solnftbuy::judge` / `solnftm2::judge`: J1–J4, only `SetComputeUnitLimit` beside it, audit B2-01), and beside it exactly the
//! advance's transfer and the link's memo. Landed: the item's custody at finalized ([`arrived`]), then `mint_item` + `settle_advance`
//! through `desk::book_outcome` (audit B1-06). An M2 purchase is the same transaction compiled as a v0 message that loads M2's fixed
//! accounts from the canister's lookup table ([`table`], read at finalized before each use): a pNFT's does not fit otherwise (C1-03).
use super::*;
use crate::chainlink::driver::SignError;
use crate::chainlink::ledger::{Effects, LinkError, OpKind, OwedWrite, Scope, Spec};
use crate::desk::state as desk;
use crate::solhome::{LineOwner, LineStep};
use crate::solix2::spl;
use crate::solkey::KeyPath;
use crate::solnft::Member;
use crate::solnftbuy::{self as nbuy, Ask, AskDetail, Expect};
use crate::soltx::{self, Instruction};

/// The limit of a buy: the books' floor plus the collection's premium, never above the module's `max_item_cost` (the most an advance
/// may be). 0 while either says no.
pub fn limit_of(accepted: u64, premium_bps: u16, max_item_cost: u64) -> u64 {
    let p = (accepted as u128 * (10_000 + premium_bps as u128) / 10_000).min(u64::MAX as u128) as u64;
    p.min(max_item_cost)
}

/// Audit C1-05: the hand-in chooses which listings are read and so where the floor goes (inside the books' bounds, up to 20 % a day);
/// a buy's limit is never above the LOWEST floor the canister accepted over [`FLOOR_WINDOW_DAYS`] days (`window_floor`) plus the
/// premium, so a floor walked up on chosen listings carries no buy until it has held for the whole window.
pub fn window_limit(limit: u64, window_floor: u64, premium_bps: u16) -> u64 {
    limit.min(limit_of(window_floor, premium_bps, u64::MAX))
}

/// The transaction of a purchase as it is signed: the venue's first instruction (its compute limit), the advance `module → me`, the
/// venue's other instructions, the link's memo.
pub fn compose(ixs: &[Instruction], module: &Pubkey, me: &Pubkey, amount: u64, memo: &str) -> Vec<Instruction> {
    let mut tx = ixs.first().cloned().into_iter().collect::<Vec<_>>();
    tx.push(soltx::transfer(*module, *me, amount));
    tx.extend(ixs.iter().skip(1).cloned());
    tx.push(spl::memo(memo, &[]));
    tx
}

/// Audit C1-03: the wire size of the transaction the flow would sign for this purchase, an upper bound: the durable nonce's advance
/// (counted whatever the line's lifetime), [`compose`] with the longest memo a link of `coll` can carry, the two signatures; compiled as
/// it will be (`solsend::compile_with`: legacy, or v0 with `tables`). Above `soltx::PACKET_DATA_SIZE` the ask is never picked (an M2
/// pNFT purchase in a legacy message: 1,395–1,461 bytes; with the canister's lookup table, [`table`]: 1,028–1,094).
pub fn composed_size(me: &Pubkey, module: &Pubkey, coll: u16, ixs: &[Instruction], amount: u64, tables: &[soltx::LookupTable])
    -> Result<usize, String> {
    let memo = crate::chainlink::memo::memo(Some(coll), OpKind::Advance, u64::MAX, None);
    let mut tx = vec![soltx::advance_nonce([0xEE; 32], *me)];
    tx.extend(compose(ixs, module, me, amount, &memo));
    Ok(soltx::wire_size(&crate::solsend::compile_with(&tx, *me, [0; 32], tables)?))
}

/// The ask to buy: `solnftbuy::best_ask` (keyless; M2 never) and, when the operator switched M2 on, the cheapest buyer-only M2 ask
/// whose auction house was read; the cheaper of the two all in. `held`: assets the books hold already (never bought twice).
pub fn pick<'a>(asks: &'a [(Ask, Member)], limit: u64, m2: bool, held: &[Pubkey]) -> Option<&'a (Ask, Member)> {
    let fresh: Vec<&(Ask, Member)> = asks.iter().filter(|(a, _)| !held.contains(&a.asset)).collect();
    let own: Vec<(Ask, Member)> = fresh.iter().map(|x| (*x).clone()).collect();
    let k = nbuy::best_ask(&own, limit, &[]).map(|(a, _)| a.listing);
    let m = if m2 {
        fresh.iter().filter(|(a, m)| a.venue == Venue::M2 && matches!(a.detail, AskDetail::M2 { house: Some(_), .. })
            && a.total.saturating_add(a.rent(m)) <= limit).min_by(|(x, _), (y, _)| (x.total, x.price, x.listing).cmp(&(y.total, y.price, y.listing)))
            .map(|(a, _)| a.listing)
    } else {
        None
    };
    let by = |l: Pubkey| fresh.iter().copied().find(|(a, _)| a.listing == l);
    match (k.and_then(by), m.and_then(by)) {
        (Some(x), Some(y)) => Some(if (y.0.total, y.0.price, y.0.listing) < (x.0.total, x.0.price, x.0.listing) { y } else { x }),
        (x, y) => x.or(y),
    }
}

/// The purchase's instructions for `ask` (the venue's builder).
pub fn purchase(me: &Pubkey, ask: &Ask, m: &Member) -> Result<Vec<Instruction>, String> {
    match ask.venue {
        Venue::M2 => crate::solnftm2::buy_instructions(me, ask, m),
        _ => nbuy::buy_instructions(me, ask, m),
    }
}

fn venue_judge(ixs: &[Instruction], e: &Expect) -> Result<(), String> {
    match e.ask.venue {
        Venue::M2 => crate::solnftm2::judge(ixs, e),
        _ => nbuy::judge(ixs, e),
    }
}

/// The whole transaction judged before it is signed: exactly one System instruction, the advance `module → me` of `amount`; exactly
/// one Memo, `memo`; everything else passes the venue's judge (which admits `SetComputeUnitLimit` and nothing else of Compute Budget);
/// and the advance pays the purchase all in (the ask's total and rent) and no more.
pub fn judge_tx(ixs: &[Instruction], module: &Pubkey, amount: u64, memo: &str, e: &Expect) -> Result<(), String> {
    let (sys, memo_p) = (soltx::SYSTEM_PROGRAM, crate::solix2::id(crate::solix2::MEMO_PROGRAM));
    let transfers: Vec<&Instruction> = ixs.iter().filter(|i| i.program == sys).collect();
    if transfers.len() != 1 || *transfers[0] != soltx::transfer(*module, e.payer, amount) {
        return Err("the transaction's System instructions are not exactly the advance M[i] → K_main".into());
    }
    let memos: Vec<&Instruction> = ixs.iter().filter(|i| i.program == memo_p).collect();
    if memos.len() != 1 || *memos[0] != spl::memo(memo, &[]) {
        return Err("the transaction does not carry exactly the link's memo".into());
    }
    let all_in = e.ask.total.saturating_add(e.ask.rent(e.member));
    if amount != all_in || amount > e.limit {
        return Err(format!("the advance {amount} is not the purchase's all-in {all_in} within the limit {}", e.limit));
    }
    let rest: Vec<Instruction> = ixs.iter().filter(|i| i.program != sys && i.program != memo_p).cloned().collect();
    venue_judge(&rest, e)
}

/// The buy of collection `coll`, if its signal fires: picked, judged, booked, signed. `Ok(None)`: nothing to buy now (the reason is
/// the collection's note).
pub async fn plan_and_sign<I: SolNftIo>(io: &mut I, coll: u16, now: u64) -> Result<Option<String>, String> {
    let (collection, custodian) = listing_of(coll)?;
    let (cs, held_keys) = (with(|s| s.cols.get(&coll).cloned()).unwrap_or_default(), desk::with(|s| {
        s.books.get(coll).map(|c| c.ledger.held_markers.keys().copied().collect::<Vec<_>>()).unwrap_or_default()
    }));
    if cs.arrival.is_some() {
        return Ok(None);
    }
    let (asks, refused) = cache(coll, |c| (c.asks.clone(), c.refused.clone()));
    if now.saturating_sub(cs.read_at) > ASKS_TTL_SECS || asks.is_empty() {
        return Ok(None);
    }
    if desk::collection_busy(coll) {
        return Ok(None);
    }
    // the module's signal (home_books::gate's): open, fresh, wanting an item beyond its buffer; the books' cap
    let (_, slot) = io.sol_clock().await?;
    let now_i = now as i64;
    let (col, kd, max_cost) = desk::with(|s| {
        let c = s.books.get(coll).map_err(|e| format!("{e}"))?;
        let kd = c.kd(s.books.keeper_paused);
        Ok::<_, String>((c.clone(), kd, s.books.buy_signal(coll, now_i).map(|x| x.1).unwrap_or(0)))
    })?;
    if col.paused {
        return Ok(None);
    }
    let view = crate::solhome::books_sink_view(&col, kd, now_i, slot, u128::MAX);
    if let Err(e) = crate::itemsink::may_start(&view, 0) {
        set_note(coll, format!("no buy: {e}"));
        return Ok(None);
    }
    // audit C1-05: never above the lowest floor of the window plus the premium
    let window = cs.window_floor(col.module.accepted, now);
    let limit = window_limit(limit_of(col.module.accepted, cs.premium_bps, max_cost), window, cs.premium_bps);
    let me = io.key_of(&KeyPath::Main).await?;
    if me != custodian {
        return Err(format!("the listing's custodian {} is not K_main {}: nothing is bought", b58_encode(&custodian), b58_encode(&me)));
    }
    let module = io.key_of(&KeyPath::Coll(coll as u32)).await?;
    // 2026-10-12: an M2 purchase is a v0 message with the canister's lookup table, read at finalized now (none usable: an M2 ask waits
    // for it, skipped this pass and not remembered as refused; [`table::pass`] makes it when nothing else is signed)
    let (lookup, unusable) = if cs.m2 && asks.iter().any(|(a, _)| a.venue == Venue::M2) {
        match table::usable(io, &me).await {
            Ok(t) => (t, String::new()),
            Err(e) => (None, format!(" (it is not usable: {e})")),
        }
    } else {
        (None, String::new())
    };
    // audit C1-03: the cheapest ask whose purchase is built, JUDGED and fits in one packet as it will be signed (before anything is
    // booked); one that does not is skipped, remembered for the rest of this read, and the next is tried
    let mut skip: Vec<Pubkey> = held_keys.iter().copied().chain(refused).collect();
    let mut skipped = vec![];
    let chosen = loop {
        let Some((ask, member)) = pick(&asks, limit, cs.m2, &skip).cloned() else { break None };
        skip.push(ask.asset);
        let tables: Vec<soltx::LookupTable> = match (ask.venue, &lookup, &ask.detail) {
            (Venue::M2, Some(t), _) => vec![t.clone()],
            (Venue::M2, None, AskDetail::M2 { house: Some(h), .. }) => {
                table::want(coll, h);
                skipped.push(format!("{} on M2 waits for the lookup table{unusable}", b58_encode(&ask.asset)));
                continue;
            }
            _ => vec![],
        };
        let amount = ask.total.saturating_add(ask.rent(&member));
        let built = purchase(&me, &ask, &member).and_then(|ixs| {
            venue_judge(&ixs, &Expect { payer: me, ask: &ask, member: &member, collection, limit }).map_err(|e| format!("the judge refuses it: {e}"))?;
            soltx::check_size(composed_size(&me, &module, coll, &ixs, amount, &tables)?)?;
            Ok(ixs)
        });
        match built {
            Ok(ixs) => break Some((ask, member, ixs, tables)),
            Err(e) => {
                cache_mut(coll, |c| c.refused.push(ask.asset));
                skipped.push(format!("{} on {:?} skipped: {e}", b58_encode(&ask.asset), ask.venue));
            }
        }
    };
    let Some((ask, member, ixs, tables)) = chosen else {
        let also = if skipped.is_empty() { String::new() } else { format!(" ({})", skipped.join("; ")) };
        set_note(coll, format!("no buy: no verified ask within {limit} lamports (floor {} + {} bps, the {FLOOR_WINDOW_DAYS}-day floor {window}, item cap \
            {max_cost}){also}", col.module.accepted, cs.premium_bps));
        return Ok(None);
    };
    let amount = ask.total.saturating_add(ask.rent(&member));
    // audit C1-02: the install's policy (a collection it does not name is refused, as `check_acquire` refuses any) and ONE book for the
    // whole canister: every other flow's items and every Solana NFT listing's, held at their cost, this one at its all-in
    let rate = io.lamports_per_eth().await?;
    with_mut(|s| s.rate = Some(rate));
    let key = cap_key(&collection);
    let policy = io.caps().unwrap_or_else(|| crate::custodycap::stage0(&live_cap_keys()));
    let mut book = io.cap_book();
    book.entries.retain(|e| !e.id.starts_with(CAP_ID_PREFIX));
    book.entries.extend(cap_entries(Some(rate)));
    if let Err(r) = crate::custodycap::check_acquire(&policy, &book, &key, crate::rates::lamports_to_wei(amount as u128, rate)) {
        set_note(coll, format!("no buy: the custody cap refuses it: {r:?}"));
        return Ok(None);
    }
    let expect = Expect { payer: me, ask: &ask, member: &member, collection, limit };
    // the advance, booked and recorded (memo, effects, undo) before the signature
    let op = with_mut(|s| {
        s.ops += 1;
        s.ops
    });
    let spec = Spec { key: format!("solnft:{coll}:buy:{op}"), kind: OpKind::Advance, coll: Some(coll), intent: None, scope: Scope::Module };
    let begun = desk::with_mut(|s| s.link.begin(&mut s.books, spec, now, |b| {
        b.advance(coll, amount, now_i)?;
        Ok((Effects { lamports_out: amount, ..Default::default() }, amount))
    }));
    let link = match begun {
        Ok((id, _)) => id,
        Err(LinkError::Books(e)) => {
            set_note(coll, format!("no buy: the books refuse the advance of {amount}: {e}"));
            return Ok(None);
        }
        Err(e) => return Err(format!("{e}")),
    };
    let memo = desk::with(|s| s.link.get(link).map(|l| l.memo.clone())).unwrap_or_default();
    let tx = compose(&ixs, &module, &me, amount, &memo);
    if let Err(e) = judge_tx(&tx, &module, amount, &memo, &expect) {
        desk::with_mut(|s| {
            let crate::chainlink::driver::DeskStateRef { books, link: l } = crate::chainlink::driver::DeskStateRef::of(s);
            l.undo(books, link, crate::chainlink::ledger::Phase::Undone, &format!("the judge refused the transaction: {e}"), now);
        });
        return Err(format!("the judge refuses the transaction: {e}"));
    }
    let step = SolNftStep::Buy { coll, asset: ask.asset, venue: ask.venue, cost: amount, link };
    let label = step.label();
    match crate::coins::link::sign_linked_tables(io, LineOwner::SolNft, LineStep::SolNft(step), link, tx, vec![KeyPath::Coll(coll as u32)],
        tables, now).await {
        Ok(sig) => {
            let p = io.sol_line_of(crate::solhome::LineId::Money).pending.clone();
            with_mut(|s| {
                s.out = p;
                s.signed_today += 1;
            });
            // an ask bought from is never picked again from this read (its listing closes; the item may be delivered within it)
            cache_mut(coll, |c| c.asks.retain(|(x, _)| x.asset != ask.asset));
            set_note(coll, format!("buying {} ({sig})", b58_encode(&ask.asset)));
            Ok(Some(format!("{label}: signed and sent ({sig})")))
        }
        Err(SignError::Refused(e)) => Err(format!("{label}: REFUSED by pre_sign: {e}")),
        Err(SignError::Deferred(e)) => {
            // audit C1-03: an ask whose purchase every provider refuses by simulation is not picked again before the next read
            if e.contains(crate::solsend::SIMULATION_REFUSED) && !e.contains("insufficient_funds") {
                cache_mut(coll, |c| c.refused.push(ask.asset));
            }
            Ok(Some(format!("{label}: waits: {e}")))
        }
    }
}

fn set_note(coll: u16, note: String) {
    with_mut(|s| s.cols.entry(coll).or_default().note = note);
}

/// Whether the books hold `asset` (its held marker), and whether its `SolNftArrived` is owed to them (a link of `coll` open).
fn booked(coll: u16, asset: &Pubkey) -> (bool, bool) {
    desk::with(|s| (s.books.get(coll).is_ok_and(|c| c.ledger.held_markers.contains_key(asset)),
        s.link.owed_of(coll).iter().any(|w| matches!(w, OwedWrite::SolNftArrived { key, .. } if key == asset))))
}

/// The arrival is booked: the flow holds the item.
fn hold(coll: u16, a: &Arrival) {
    with_mut(|s| {
        let c = s.cols.entry(coll).or_default();
        c.arrival = None;
        c.held.insert(a.asset, Held { cost: a.cost, signature: a.signature.clone() });
        c.note = format!("{} in custody ({})", b58_encode(&a.asset), a.signature);
    });
}

/// A landed purchase's item in custody: checked at finalized for `K_main`, then booked (`mint_item` + `settle_advance` through the
/// deferred writer). Kept and checked again next pass until the read agrees, and (audit C1-04) kept until the books HOLD the item: an
/// owed write is waited for, and one refused when it was applied is made again (the custody read first), so the advance settles.
pub async fn arrived<I: SolNftIo>(io: &mut I, coll: u16, now: u64) -> Option<String> {
    let a = with(|s| s.cols.get(&coll).and_then(|c| c.arrival.clone()))?;
    match booked(coll, &a.asset) {
        (true, _) => {
            hold(coll, &a);
            return Some(format!("solnft {coll}: {} is in the books (mint_item + settle_advance({}) applied, {})", b58_encode(&a.asset), a.cost,
                a.signature));
        }
        (false, true) => return None,
        (false, false) => {}
    }
    let (collection, _) = listing_of(coll).ok()?;
    let standard = with(|s| s.cols.get(&coll).map(|c| c.standard)).unwrap_or(Standard::Programmable);
    let me = match io.key_of(&KeyPath::Main).await {
        Ok(m) => m,
        Err(e) => return Some(format!("solnft {coll}: the custody check waits: {e}")),
    };
    let checked = match read(io, &crate::solnft::check_keys(&a.asset, &collection, standard, &me)).await {
        Ok(acc) => crate::solnft::check_on(&acc, &a.asset, &collection, standard, &me).map_err(|e| e.to_string()),
        Err(e) => Err(e),
    };
    if let Err(e) = checked {
        let tries = with_mut(|s| {
            let x = s.cols.entry(coll).or_default().arrival.as_mut().map(|x| {
                x.tries += 1;
                x.tries
            });
            x.unwrap_or(0)
        });
        if tries == 5 {
            desk::with_mut(|s| s.link.incident(crate::chainlink::ledger::Incident { coll: Some(coll), link: None, at: now,
                what: format!("solnft: the purchase {} of {} landed but K_main's custody is not confirmed after {tries} reads: {e} (OPERATOR)",
                    a.signature, b58_encode(&a.asset)) }));
        }
        return Some(format!("solnft {coll}: {} landed; its custody is not confirmed yet: {e}", b58_encode(&a.asset)));
    }
    let w = OwedWrite::SolNftArrived { key: a.asset, tx: tx32(&a.signature), cost: a.cost, now: now as i64 };
    match desk::book_outcome(coll, w) {
        Ok(()) if booked(coll, &a.asset).0 => {
            hold(coll, &a);
            Some(format!("solnft {coll}: {} is in K_main's custody: mint_item + settle_advance({}) in the books ({})", b58_encode(&a.asset), a.cost,
                a.signature))
        }
        Ok(()) => Some(format!("solnft {coll}: {} is in K_main's custody: mint_item + settle_advance({}) owed to the books (a link of the \
            collection is open; the arrival is kept until they hold it) ({})", b58_encode(&a.asset), a.cost, a.signature)),
        Err(e) => Some(format!("solnft {coll}: {} is in custody but the books refuse it now: {e} (tried again next pass)", b58_encode(&a.asset))),
    }
}
