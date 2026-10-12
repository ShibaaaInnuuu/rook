//! Signing for the coins (crank line) and the pool desk (pool line): `chainlink::driver::sign_recorded`'s gate, on a line of the caller's
//! choosing. A step that moves a collection's money is a [`chainlink`](crate::chainlink) link first (booked, recorded, memo), then passes
//! `pre_sign` against the chain at finalized, then is signed; a step that moves none (a graduation's wX moving from the curve to the
//! vault, a fee claim, a pool's opening) only checks the instructions and signs.
//!
//! Neither coins nor the pool desk ever hold a place in the line's queue (`solhome::release` after any refusal): they sign only on a free
//! line, and a flow of the money line or the desk never waits for them.
use crate::chainlink::driver::{self, DeskIo, DeskStateRef, SignError};
use crate::chainlink::gate::{self, ChainView};
use crate::chainlink::ledger::Phase;
use crate::desk::state::{with, with_mut};
use crate::solhome::{self, LineId, LineOwner, LineStep};
use crate::solix2::guard;
use crate::solkey::KeyPath;
use crate::soltx::Instruction;

/// The saved signature of the transaction on `owner`'s line (the one just signed).
fn saved_signature<I: DeskIo>(io: &mut I, owner: LineOwner) -> String {
    io.sol_line_of(LineId::of(owner)).line.saved.as_ref().map(|s| s.signature.clone()).unwrap_or_default()
}

/// Checks the instructions and signs a step that changes no collection's money. `Err`: nothing was signed (the text says why; a
/// simulation refusal carries `solsend::SIMULATION_REFUSED`).
pub async fn sign_plain<I: DeskIo>(io: &mut I, owner: LineOwner, step: LineStep, ixs: Vec<Instruction>, keys: Vec<KeyPath>, now: u64) -> Result<String, String> {
    guard::refuse_forbidden(&ixs)?;
    let label = step.label();
    if let Err(e) = solhome::ready(io, owner, now, &label).await {
        solhome::release(io, owner);
        return Err(e);
    }
    match solhome::sign_keys(io, step, now, ixs, keys).await {
        Ok(_) => Ok(saved_signature(io, owner)),
        Err(e) => {
            solhome::release(io, owner);
            Err(e)
        }
    }
}

/// Signs link `id` (already begun: booked and journalled) on `owner`'s line: the chain read at finalized, `pre_sign`, the line, the
/// signature; the link becomes Signed. Anything short of a signature undoes the link (the books are as before); a gate refusal also
/// stops the collection (an incident).
pub async fn sign_linked<I: DeskIo>(io: &mut I, owner: LineOwner, step: LineStep, id: u64, ixs: Vec<Instruction>, keys: Vec<KeyPath>, now: u64)
    -> Result<String, SignError> {
    sign_linked_tables(io, owner, step, id, ixs, keys, vec![], now).await
}

/// [`sign_linked`] for a step sent as a v0 message loading keys from `tables` (read by the canister at finalized, owned by `K_main`;
/// 2026-10-12: a Magic Eden M2 pNFT purchase, `solnftflow::table`).
#[allow(clippy::too_many_arguments)]
pub async fn sign_linked_tables<I: DeskIo>(io: &mut I, owner: LineOwner, step: LineStep, id: u64, ixs: Vec<Instruction>, keys: Vec<KeyPath>,
    tables: Vec<crate::soltx::LookupTable>, now: u64)
    -> Result<String, SignError> {
    let Some((coll, memo, has_effect)) = with(|s| s.link.get(id).map(|l| (l.coll, l.memo.clone(), !l.effects.is_none()))) else {
        return Err(SignError::Deferred(format!("no link {id}")));
    };
    let undo = |why: String, phase: Phase| {
        with_mut(|s| {
            let DeskStateRef { books, link } = DeskStateRef::of(s);
            link.undo(books, id, phase, &why, now);
        })
    };
    if let Err(e) = guard::refuse_forbidden(&ixs) {
        undo(format!("not signed: {e}"), Phase::Undone);
        return Err(SignError::Deferred(e));
    }
    // 1. the chain at finalized, only for a step that moves money at a collection
    let view = match coll {
        Some(c) if has_effect => match driver::read_view(io, c).await {
            Ok(v) => Some(v),
            Err(e) => {
                undo(format!("the chain was not readable: {e}"), Phase::Undone);
                return Err(SignError::Deferred(format!("{memo}: the chain is not readable: {e}")));
            }
        },
        _ => None,
    };
    // 2. the gate: book first, then sign, and only if every guard holds against the chain
    let gated = match view {
        Some(v) => with(|s| gate::pre_sign(&s.books, &s.link, id, &v, now as i64)).map_err(|e| (e, Some(v))),
        None => with(|s| gate::pre_sign(&s.books, &s.link, id, &ChainView { coll: coll.unwrap_or(0), free_lamports: 0, supply: 0 }, now as i64)).map_err(|e| (e, None)),
    };
    if let Err((e, v)) = gated {
        let chain = v.map(|v| format!(" (chain: free {} lamports, supply {})", v.free_lamports, v.supply)).unwrap_or_default();
        let why = format!("pre_sign refused {memo}: {e}{chain}");
        with_mut(|s| {
            let DeskStateRef { books, link } = DeskStateRef::of(s);
            link.undo(books, id, Phase::Undone, &why, now);
            if let (Some(c), true) = (coll, v.is_some()) {
                link.halt(books, c, &why, now);
            }
        });
        return Err(SignError::Refused(e));
    }
    // 3. the line's turn and the signature
    let label = step.label();
    if let Err(e) = solhome::ready(io, owner, now, &label).await {
        solhome::release(io, owner);
        undo(format!("the {} line was not ready: {e}", LineId::of(owner).tag()), Phase::Undone);
        return Err(SignError::Deferred(e));
    }
    match solhome::sign_tables(io, step, now, ixs, keys, tables).await {
        Ok(_) => {
            let sig = saved_signature(io, owner);
            with_mut(|s| s.link.signed(id, &sig, now));
            Ok(sig)
        }
        Err(e) => {
            solhome::release(io, owner);
            undo(format!("not signed: {e}"), Phase::Undone);
            Err(SignError::Deferred(e))
        }
    }
}

/// Where a link stands now (`None`: no such link, e.g. pruned).
pub fn phase_of(id: u64) -> Option<Phase> {
    with(|s| s.link.get(id).map(|l| l.phase))
}
