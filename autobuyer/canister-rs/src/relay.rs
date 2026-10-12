//! The venue, handed in (`docs/v2/venue-bridge-policy.md` §3). No venue is both keyless and reachable over IPv6 from the
//! canister, and a key placed here would be public. So a relayer off the canister -- the operator today -- fetches the venue's
//! quote and PSBT with whatever key the venue needs and hands them in. The loop's venue is then [`Relayed`]: it returns what was
//! handed in for the current purchase and nothing else, and says `NoListing` (the purchase waits) until something is.
//!
//! The relayer's power is bounded by what the canister already checks: a quote must match the purchase's order and our
//! holdings (and the recorded-quote checks in `state.rs` / `record_quote`); a PSBT is judged by `psbt.rs` against the RECORDED
//! quote. A relayer can waste time. It cannot redirect the asset or spend past the recorded quote.
//!
//! Held in heap memory, not stable: after an upgrade the relayer hands the same things in again, which costs nothing.

use crate::psbt::Quote;
use crate::venue::{Venue, VenueError};
use std::cell::RefCell;

/// What has been handed in, for one purchase.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Handoff {
    pub purchase: u64,
    pub quote: Option<Quote>,
    pub psbt: Option<(Vec<u8>, Vec<usize>)>,
}

thread_local! {
    static HANDOFF: RefCell<Option<Handoff>> = const { RefCell::new(None) };
}

/// Hand in a quote for `purchase`. A handoff for any other purchase is dropped: one purchase at a time.
pub fn put_quote(purchase: u64, q: Quote) {
    HANDOFF.with(|h| {
        let mut h = h.borrow_mut();
        match h.as_mut() {
            Some(x) if x.purchase == purchase => x.quote = Some(q),
            _ => *h = Some(Handoff { purchase, quote: Some(q), psbt: None }),
        }
    });
}

/// Hand in a PSBT for `purchase`, with the input indexes the venue asks us to sign.
pub fn put_psbt(purchase: u64, psbt: Vec<u8>, to_sign: Vec<usize>) {
    HANDOFF.with(|h| {
        let mut h = h.borrow_mut();
        match h.as_mut() {
            Some(x) if x.purchase == purchase => x.psbt = Some((psbt, to_sign)),
            _ => *h = Some(Handoff { purchase, quote: None, psbt: Some((psbt, to_sign)) }),
        }
    });
}

pub fn current() -> Option<Handoff> {
    HANDOFF.with(|h| h.borrow().clone())
}

pub fn clear() {
    HANDOFF.with(|h| *h.borrow_mut() = None);
}

/// The venue the loop uses on a real install: whatever was handed in for `purchase`, and nothing for any other.
#[derive(Debug, Clone, Default)]
pub struct Relayed {
    pub handoff: Option<Handoff>,
}

impl Relayed {
    pub fn for_purchase(purchase: Option<u64>) -> Self {
        Relayed { handoff: current().filter(|h| Some(h.purchase) == purchase) }
    }
}

impl Venue for Relayed {
    fn quote(&self, _rune: &str, _units: u128, _recipient_script: &[u8]) -> Result<Quote, VenueError> {
        self.handoff.as_ref().and_then(|h| h.quote.clone()).ok_or(VenueError::NoListing)
    }
    /// The PSBT handed in, whatever quote it was fetched for: `psbt.rs` judges it against the RECORDED quote, so a PSBT made
    /// for anything else is refused there, not here.
    fn psbt(&self, _quote: &Quote) -> Result<(Vec<u8>, Vec<usize>), VenueError> {
        self.handoff.as_ref().and_then(|h| h.psbt.clone()).ok_or(VenueError::NoListing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runestone::RuneId;

    fn q(units: u128) -> Quote {
        Quote { rune: RuneId { block: 840_000, tx: 3 }, units, max_spend_sats: 10_000, max_fee_sats: 1_000,
            recipient_script: vec![0x51, 0x20, 2], our_script: vec![0x51, 0x20, 1] }
    }

    #[test]
    fn nothing_handed_in_means_the_purchase_waits() {
        clear();
        let v = Relayed::for_purchase(Some(7));
        assert_eq!(v.quote("840000:3", 1, &[]), Err(VenueError::NoListing));
        assert_eq!(v.psbt(&q(1)), Err(VenueError::NoListing));
    }

    #[test]
    fn a_handoff_serves_its_own_purchase_only_and_a_new_purchase_replaces_it() {
        clear();
        put_quote(7, q(100));
        put_psbt(7, vec![1, 2, 3], vec![0]);
        assert_eq!(Relayed::for_purchase(Some(7)).quote("x", 0, &[]), Ok(q(100)));
        assert_eq!(Relayed::for_purchase(Some(7)).psbt(&q(100)), Ok((vec![1, 2, 3], vec![0])));
        assert_eq!(Relayed::for_purchase(Some(8)).quote("x", 0, &[]), Err(VenueError::NoListing), "not purchase 8's");
        assert_eq!(Relayed::for_purchase(None).psbt(&q(100)), Err(VenueError::NoListing));
        put_quote(8, q(5));
        assert_eq!(current(), Some(Handoff { purchase: 8, quote: Some(q(5)), psbt: None }), "purchase 7's PSBT is gone");
        put_quote(8, q(6));
        assert_eq!(current().unwrap().quote, Some(q(6)), "a newer quote for the same purchase replaces the older");
    }
}
