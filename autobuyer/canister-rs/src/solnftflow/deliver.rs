//! A resolved ticket of a Solana NFT listing delivered (docs/v9/solana-nfts-canister.md §1 step 3, §4; docs/v9/canister-only-solana.md
//! §8): `K_main`'s custody checked at finalized, the canonical transfer (`solnftsend::send_instructions`) judged (`judge_send`), within
//! the outflow limits (H3, audit C1-02), and signed on the money line by `K_main` alone; once it lands, `record_delivery` (audit C1-01:
//! the line's verdict on the judged transfer decides, not what the destination did with the item since). No books change before the
//! landing, so no link and no memo (§4.4 rule 4: a delivery changes no supply and no backing balance). A destination that refuses: not
//! a wallet (`send_instructions` refuses it: a PDA, the custodian) at once; a rule set veto (B2-09) once the delivery was refused
//! [`DELIVERY_TRIES`] times (by simulation AT the transfer, audit C1-07, or on chain): `mark_undeliverable`, then the cranks' refund
//! rules. A ticket that cannot progress (its item not in `K_main`'s custody, a judge refusal) never stops the collection's other
//! tickets (C1-01), and no delivery is signed within [`LATE_MARGIN_SECS`] of its ticket's late refund (C1-08).
use super::*;
use crate::books::ledger::{Destination, Stage, LATE_DELIVERY_WINDOW};
use crate::chainlink::ledger::OwedWrite;
use crate::desk::state as desk;
use crate::solhome::{self as sh, LineOwner, LineStep};
use crate::solkey::KeyPath;
use crate::soltx::{self, Instruction};

/// A ticket of the collection to act on.
struct Due {
    ticket: u64,
    asset: Pubkey,
    to: Pubkey,
    sent: [u8; 32],
    deliver_by: i64,
}

/// The tickets of `coll` to act on, in ticket order: resolved, not undeliverable, to a Solana wallet. Audit C1-01: all of them, not
/// the first only (one that cannot progress never stops the others).
fn due(coll: u16) -> Vec<Due> {
    desk::with(|s| {
        let Ok(c) = s.books.get(coll) else { return vec![] };
        c.ledger.tickets.values().filter(|t| t.stage == Stage::Resolved && !t.undeliverable).filter_map(|t| match t.destination {
            Destination::Sol(d) => c.ledger.items.get(&t.item_id).map(|x| Due { ticket: t.id, asset: x.key, to: d, sent: t.sent_tx,
                deliver_by: t.deliver_by }),
            _ => None,
        }).collect()
    })
}

fn tries_of(coll: u16, ticket: u64) -> u32 {
    with(|s| s.cols.get(&coll).and_then(|c| c.tries.get(&ticket).copied()).unwrap_or(0))
}

/// One more refusal of `ticket`'s delivery; at [`DELIVERY_TRIES`] the ticket is marked undeliverable. Returns the log text.
pub fn refused(coll: u16, ticket: u64, why: &str) -> String {
    let n = with_mut(|s| {
        let t = s.cols.entry(coll).or_default().tries.entry(ticket).or_insert(0);
        *t += 1;
        *t
    });
    if n >= DELIVERY_TRIES {
        return mark(coll, ticket, &format!("refused {n} times, the last: {why}"));
    }
    format!("solnft {coll}: ticket {ticket}'s delivery refused ({n} of {DELIVERY_TRIES}): {why}")
}

fn mark(coll: u16, ticket: u64, why: &str) -> String {
    match desk::book_outcome(coll, OwedWrite::MarkUndeliverable { ticket }) {
        Ok(()) => format!("solnft {coll}: ticket {ticket} marked undeliverable in the books: {why}"),
        Err(e) => format!("solnft {coll}: ticket {ticket} is undeliverable ({why}) but the books refuse the mark: {e}"),
    }
}

/// The index of the instruction a simulation error names (`{"InstructionError":[i, …]}`), if any.
pub fn failed_instruction(err: &str) -> Option<usize> {
    let at = err.find("\"InstructionError\":[")? + "\"InstructionError\":[".len();
    let digits: String = err[at..].chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

/// Audit C1-07: whether a refusal by simulation is the DESTINATION's: the error names one of the delivery's own instructions (the
/// transfer, and for a legacy item the destination's token account and the source's close; never the compute limit), `prefix`
/// instructions (the durable nonce's advance) before them. Anything else (the nonce, a provider's fault, a stale read) is a wait.
pub fn names_destination(err: &str, ixs: &[Instruction], prefix: usize) -> bool {
    let cb = soltx::key(soltx::COMPUTE_BUDGET_PROGRAM);
    failed_instruction(err).and_then(|i| i.checked_sub(prefix)).and_then(|i| ixs.get(i)).is_some_and(|ix| ix.program != cb)
}

/// The delivery pass of `coll`: tickets noted sent recorded; then (`sign`) the first ticket that can be delivered, signed. Nothing
/// while the collection has an open link.
pub async fn pass<I: SolNftIo>(io: &mut I, coll: u16, now: u64, sign: bool) -> Vec<String> {
    let mut log = vec![];
    let tickets = due(coll);
    if tickets.is_empty() || desk::collection_busy(coll) {
        return log;
    }
    // audit C1-01: a ticket noted sent had its delivery LAND (the only `note_sent` of a Solana NFT listing, by a build before this
    // fix): the landing decided it, whatever the destination did with the item since
    for t in tickets.iter().filter(|t| t.sent != [0u8; 32]) {
        log.push(match desk::book_outcome(coll, OwedWrite::RecordDelivery { ticket: t.ticket, tx: t.sent }) {
            Ok(()) => format!("solnft {coll}: ticket {} recorded delivered (its delivery of {} landed)", t.ticket, b58_encode(&t.asset)),
            Err(e) => format!("solnft {coll}: ticket {}: its delivery landed but the books refuse the record: {e}", t.ticket),
        });
    }
    if !sign {
        return log;
    }
    let Ok((collection, _)) = listing_of(coll) else { return log };
    let standard = with(|s| s.cols.get(&coll).map(|c| c.standard)).unwrap_or(Standard::Programmable);
    let me = match io.key_of(&KeyPath::Main).await {
        Ok(m) => m,
        Err(e) => {
            log.push(format!("solnft {coll}: a delivery waits: {e}"));
            return log;
        }
    };
    let prefix = usize::from(io.sol_home().is_some_and(|h| h.lifetime == crate::solsend::LifetimeKind::DurableNonce));
    let ckey = cap_key(&collection);
    for t in tickets.iter().filter(|t| t.sent == [0u8; 32]).take(TICKETS_PER_PASS) {
        let (ticket, asset, to) = (t.ticket, t.asset, t.to);
        // audit C1-08: within a day of the late refund (or past it) the refund rule decides; a delivery would race it
        if (now as i64).saturating_add(LATE_MARGIN_SECS) >= t.deliver_by.saturating_add(LATE_DELIVERY_WINDOW) {
            log.push(format!("solnft {coll}: ticket {ticket} is within {LATE_MARGIN_SECS} s of its late refund: no delivery is signed"));
            continue;
        }
        let acc = match read(io, &crate::solnft::check_keys(&asset, &collection, standard, &me)).await {
            Ok(a) => a,
            Err(e) => {
                log.push(format!("solnft {coll}: ticket {ticket}'s delivery waits: {e}"));
                return log;
            }
        };
        let (member, _) = match crate::solnft::check_on(&acc, &asset, &collection, standard, &me) {
            Ok(x) => x,
            Err(e) => {
                // audit C1-01: this ticket waits; the next is tried
                log.push(format!("solnft {coll}: ticket {ticket}: {} is not in K_main's custody: {e} (OPERATOR: nothing is sent for it)",
                    b58_encode(&asset)));
                continue;
            }
        };
        let ixs = match crate::solnftsend::send_instructions(&me, &to, &member) {
            Ok(x) => x,
            // a destination that refuses by construction: never a wallet (a PDA) or the custodian
            Err(e) => {
                log.push(mark(coll, ticket, &e));
                continue;
            }
        };
        if let Err(e) = crate::solnftsend::judge_send(&ixs, &me, &to, &member) {
            log.push(format!("solnft {coll}: ticket {ticket}: the delivery judge refuses: {e}"));
            continue;
        }
        // audit C1-02: the outflow limits (H3), as every other delivery out of custody
        let okey = format!("solnft:{coll}:deliver:{ticket}");
        if let Err(e) = io.outflow_check(&okey, &ckey, crate::outflow::Flow::Item, 1) {
            log.push(format!("solnft {coll}: ticket {ticket}'s delivery {e}"));
            return log;
        }
        let step = SolNftStep::Deliver { coll, ticket, to };
        let label = step.label();
        if let Err(e) = sh::ready(io, LineOwner::SolNft, now, &label).await {
            log.push(format!("{label}: waits: {e}"));
            return log;
        }
        match sh::sign(io, LineStep::SolNft(step), now, ixs.clone()).await {
            Ok((text, p)) => {
                io.outflow_record(&okey, &ckey, crate::outflow::Flow::Item, 1);
                with_mut(|s| {
                    s.out = Some(p);
                    s.signed_today += 1;
                });
                log.push(text);
            }
            Err(e) => {
                sh::release(io, LineOwner::SolNft);
                // audit C1-07: only a refusal AT the delivery's own instructions is the destination's (a rule set veto)
                if e.contains(crate::solsend::SIMULATION_REFUSED) && !e.contains("insufficient_funds") && names_destination(&e, &ixs, prefix) {
                    log.push(refused(coll, ticket, &e));
                } else {
                    log.push(format!("{label}: waits: {e}"));
                }
            }
        }
        return log;
    }
    log
}

/// A delivery's outcome on the line: landed → `record_delivery` (audit C1-01: the judged, canonical transfer landed without error, so
/// the item left `K_main` for the ticket's destination; audit C1-I6: no read, no await between the outcome taken and its booking);
/// failed on chain → a refusal ([`refused`]); never landed → tried again.
pub fn landed(coll: u16, ticket: u64, ok: Option<bool>, signature: &str) -> String {
    match ok {
        Some(true) => {
            let tx = tx32(signature);
            let asset = desk::with(|s| s.books.get(coll).ok().and_then(|c| c.ledger.tickets.get(&ticket).and_then(|t| c.ledger.items.get(&t.item_id)))
                .map(|x| x.key));
            let Some(asset) = asset else { return format!("solnft {coll}: ticket {ticket} is gone") };
            with_mut(|s| {
                let c = s.cols.entry(coll).or_default();
                c.tries.remove(&ticket);
                c.held.remove(&asset);
            });
            match desk::book_outcome(coll, OwedWrite::RecordDelivery { ticket, tx }) {
                Ok(()) => format!("solnft {coll}: ticket {ticket}'s delivery landed ({signature}): recorded delivered"),
                Err(e) => format!("solnft {coll}: ticket {ticket}'s delivery landed ({signature}) but the books refuse the record: {e} (OPERATOR)"),
            }
        }
        Some(false) => refused(coll, ticket, &format!("failed on chain ({signature})")),
        None => format!("solnft {coll}: ticket {ticket}'s delivery never landed: tried again"),
    }
}

/// The ticket's refusals so far (the status).
pub fn tries(coll: u16, ticket: u64) -> u32 {
    tries_of(coll, ticket)
}
