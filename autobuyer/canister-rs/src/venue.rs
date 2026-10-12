//! The venue: where a rune is bought, behind an interface, so the買 path can be tested without one.
//!
//! Two implementations are intended. [`Fixture`] replays a recorded exchange and is what the tests and the local end-to-end
//! run use. A UniSat implementation talks to the marketplace over a **non-replicated** HTTPS outcall — safe only because
//! [`crate::psbt::Psbt::verify`] judges whatever comes back. It is not built here: buying at UniSat needs their written
//! permission for commercial use (§3.1), which the operator has asked for and not received.

use crate::psbt::Quote;

/// What a venue is asked for, and what it must answer with.
pub trait Venue {
    /// What the asset costs right now. Recorded **before** any PSBT is requested, so the PSBT can be judged against it.
    fn quote(&self, rune: &str, units: u128, recipient_script: &[u8]) -> Result<Quote, VenueError>;
    /// The unsigned transaction that would do it, plus which inputs the venue wants signed.
    fn psbt(&self, quote: &Quote) -> Result<(Vec<u8>, Vec<usize>), VenueError>;
}

#[derive(Debug, PartialEq, Eq)]
pub enum VenueError {
    /// Nothing listed at or under our cap.
    NoListing,
    /// The venue answered, but not with something we can read.
    Unreadable,
    /// The venue refused us: rate limit, missing key, or terms.
    Refused(String),
}

/// A recorded exchange, replayed. The local end-to-end run uses this so the whole loop can be exercised without a key, a
/// listing, or anybody's permission.
pub struct Fixture {
    pub quote: Quote,
    pub psbt: Vec<u8>,
    pub to_sign: Vec<usize>,
}

impl Venue for Fixture {
    fn quote(&self, _rune: &str, _units: u128, _recipient_script: &[u8]) -> Result<Quote, VenueError> {
        Ok(self.quote.clone())
    }
    fn psbt(&self, _quote: &Quote) -> Result<(Vec<u8>, Vec<usize>), VenueError> {
        Ok((self.psbt.clone(), self.to_sign.clone()))
    }
}
