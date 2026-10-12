//! `pre_sign`: the ONE gate every signature of a money-moving transaction passes (§4.4 rules 2 and 3), with the chain read at
//! finalized immediately before. Pure: the caller reads [`ChainView`] and acts on the refusal (an incident, the link undone).
//!
//! The books already include the operation being signed and every other undecided one (book first), the chain includes none of them
//! yet, so each guard compares the chain's finalized numbers adjusted by what is still to land ("eventual" numbers):
//!
//! - [`books::supply_guard`]: `supply (finalized, less the burns still to land) + the other undecided mints + this mint ≤ the ledger's supply`;
//! - [`books::payout_guard`]: `free lamports − the other undecided payouts − this payout ≥ books`;
//! - [`books::check_collection`] on the eventual numbers: `free + Σ in − Σ out ≥ books` (`books_balance`), `supply + Σ mint − Σ burn =
//!   the ledger's supply` (`supply_matches`, exact: a chain that drifted from the books in either direction stops here), no free
//!   profit, the module's own sanity.
//!
//! A refusal here is an incident, not a retry: the collection is stopped (`Ledger::halt`).
use super::ledger::{Ledger, Phase};
use crate::books::{self, Books, BooksError};

/// What the chain shows for collection `coll` at finalized.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChainView {
    pub coll: u16,
    /// `M[i]`'s lamports above its rent-exempt minimum.
    pub free_lamports: u64,
    /// The wX mint's supply.
    pub supply: u64,
}

/// The eventual `(free lamports, supply)` of a collection if every undecided link lands (`None`: the arithmetic does not hold).
pub fn eventual(ledger: &Ledger, view: &ChainView) -> Option<(u64, u64)> {
    let t = ledger.totals(view.coll, None);
    let free = (view.free_lamports as i128) + (t.lamports_in as i128) - (t.lamports_out as i128);
    let supply = (view.supply as i128) + (t.mint_units as i128) - (t.burn_units as i128);
    Some((u64::try_from(free).ok()?, u64::try_from(supply).ok()?))
}

/// May link `id` be signed now? `Ok` for a link that moves nothing at a collection.
pub fn pre_sign(books: &Books, ledger: &Ledger, id: u64, view: &ChainView, now: i64) -> Result<(), BooksError> {
    let l = ledger.get(id).ok_or(BooksError::NothingPending)?;
    if l.phase != Phase::Recorded {
        return Err(BooksError::WrongStage);
    }
    let Some(coll) = l.coll else { return Ok(()) };
    if view.coll != coll {
        return Err(BooksError::WrongCollection);
    }
    if l.effects.is_none() {
        return Ok(());
    }
    let c = books.get(coll)?;
    let others = ledger.totals(coll, Some(id));
    let me = l.effects;
    // the supply, as it will stand when every burn still to land has landed
    let burns = others.burn_units.checked_add(me.burn_units).ok_or(BooksError::MathOverflow)?;
    let supply_after_burns = view.supply.checked_sub(burns).ok_or(BooksError::SupplyMismatch)?;
    if me.mint_units > 0 {
        books::supply_guard(&c.ledger, supply_after_burns, others.mint_units, me.mint_units)?;
    }
    if me.lamports_out > 0 {
        books::payout_guard(&c.module, view.free_lamports, others.lamports_out, me.lamports_out)?;
    }
    let (free, supply) = eventual(ledger, view).ok_or(BooksError::BooksUnbacked)?;
    books::check_collection(books, coll, free, supply, now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chainlink::ledger::{Effects, OpKind, Scope, Spec};

    fn books_with_floor() -> Books {
        let mut b = Books::new();
        b.add_collection(0, books::ListingKind::EthNft { chain_id: 1, collection: [1; 20], custody: [2; 20] }, [9; 32], books::default_params(20 * books::LAMPORTS_PER_SOL))
            .unwrap();
        b.post_floor(0, 200 * books::LAMPORTS_PER_SOL, 9_000, 20, 1_000).unwrap();
        b
    }

    #[test]
    fn a_mint_passes_when_the_chain_matches_and_is_refused_when_it_drifted() {
        let mut b = books_with_floor();
        let mut l = Ledger::default();
        let spec = Spec { key: "k".into(), kind: OpKind::Mint, coll: Some(0), intent: None, scope: Scope::Module };
        let lamports = 5 * books::LAMPORTS_PER_SOL;
        let (id, _) = l.begin(&mut b, spec, 1_000, |b| {
            let m = b.mint(0, lamports, 0, 1_000)?;
            Ok((Effects { mint_units: m.units, lamports_in: lamports, ..Default::default() }, 0))
        }).unwrap();
        let ok = ChainView { coll: 0, free_lamports: 0, supply: 0 };
        assert_eq!(pre_sign(&b, &l, id, &ok, 1_000), Ok(()));
        // the chain's supply is above the ledger's: stopped, whatever else holds
        assert_eq!(pre_sign(&b, &l, id, &ChainView { supply: 1, ..ok }, 1_000), Err(BooksError::SupplyMismatch));
        // the module address holds less than the books before this mint
        let mut b2 = b.clone();
        b2.collections.get_mut(&0).unwrap().module.surplus += 1;
        assert_eq!(pre_sign(&b2, &l, id, &ok, 1_000), Err(BooksError::BooksUnbacked));
    }
}
