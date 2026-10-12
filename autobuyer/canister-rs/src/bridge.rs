//! Getting BTC for the vault's ETH, behind an interface, because this is the least trustworthy hop in the design.
//!
//! Whatever the bridge says, the buyer believes **Bitcoin**: the quote is checked against our own rate bound before we sign
//! anything, and arrival is only ever established by reading the chain. A bridge that takes the ETH and never pays leaves the
//! purchase `Stuck`, which freezes the vault and is written off by the owner in public. See `docs/v2/autonomous-buyer.md`.

/// What a bridge offered, before we agreed to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeQuote {
    /// The wei we would send.
    pub send_wei: u128,
    /// The sats we are told to expect.
    pub expect_sats: u64,
    /// The contract the transaction calls, which must be on our allowlist.
    pub to: [u8; 20],
    /// Its calldata, simulated against 4663 before signing.
    pub calldata: Vec<u8>,
    /// Where the BTC is to be sent: must equal our own taproot address.
    pub recipient_script: Vec<u8>,
}

pub trait Bridge {
    fn quote(&self, send_wei: u128, our_script: &[u8]) -> Result<BridgeQuote, BridgeError>;
}

#[derive(Debug, PartialEq, Eq)]
pub enum BridgeError {
    NoRoute,
    Unreadable,
    Refused(String),
}

/// Why a quote was refused before we signed for it.
#[derive(Debug, PartialEq, Eq)]
pub enum QuoteRefusal {
    /// The contract it wants to call is not one we allow.
    UnknownContract([u8; 20]),
    /// It would send the BTC somewhere that is not ours.
    NotOurRecipient,
    /// It would move a different amount of ETH than the vault released.
    WrongAmount { asked: u128, quoted: u128 },
    /// The rate is outside the tolerance we allow against our own reference.
    RateOutOfBounds { expect_sats: u64, floor_sats: u64 },
}

/// Judge a bridge's offer before signing for it. `floor_sats` is what our own reference rate says the ETH is worth, less the
/// tolerance we accept — computed by the caller from an independent source, never from the bridge.
pub fn check(q: &BridgeQuote, asked_wei: u128, our_script: &[u8], allowlist: &[[u8; 20]], floor_sats: u64) -> Result<(), QuoteRefusal> {
    if !allowlist.contains(&q.to) {
        return Err(QuoteRefusal::UnknownContract(q.to));
    }
    if q.recipient_script != our_script {
        return Err(QuoteRefusal::NotOurRecipient);
    }
    if q.send_wei != asked_wei {
        return Err(QuoteRefusal::WrongAmount { asked: asked_wei, quoted: q.send_wei });
    }
    if q.expect_sats < floor_sats {
        return Err(QuoteRefusal::RateOutOfBounds { expect_sats: q.expect_sats, floor_sats });
    }
    Ok(())
}

/// A recorded bridge, for the local end-to-end run.
pub struct Fixture {
    pub quote: BridgeQuote,
}

impl Bridge for Fixture {
    fn quote(&self, _send_wei: u128, _our_script: &[u8]) -> Result<BridgeQuote, BridgeError> {
        Ok(self.quote.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OURS: &[u8] = &[0x51, 0x20, 1];
    const GOOD: [u8; 20] = [0xb4; 20];
    const BAD: [u8; 20] = [0xff; 20];

    fn q() -> BridgeQuote {
        BridgeQuote {
            send_wei: 1_000_000_000_000_000_000,
            expect_sats: 100_000,
            to: GOOD,
            calldata: vec![1, 2, 3],
            recipient_script: OURS.to_vec(),
        }
    }

    #[test]
    fn a_good_quote_passes() {
        assert_eq!(check(&q(), q().send_wei, OURS, &[GOOD], 95_000), Ok(()));
    }

    #[test]
    fn a_bridge_contract_we_do_not_know_is_refused() {
        let mut x = q();
        x.to = BAD;
        assert_eq!(check(&x, x.send_wei, OURS, &[GOOD], 95_000), Err(QuoteRefusal::UnknownContract(BAD)));
    }

    #[test]
    fn a_bridge_that_would_pay_somebody_else_is_refused() {
        let mut x = q();
        x.recipient_script = vec![0x51, 0x20, 9];
        assert_eq!(check(&x, x.send_wei, OURS, &[GOOD], 95_000), Err(QuoteRefusal::NotOurRecipient));
    }

    #[test]
    fn a_bridge_that_would_move_a_different_amount_is_refused() {
        let x = q();
        assert_eq!(
            check(&x, 2_000_000_000_000_000_000, OURS, &[GOOD], 95_000),
            Err(QuoteRefusal::WrongAmount { asked: 2_000_000_000_000_000_000, quoted: x.send_wei })
        );
    }

    #[test]
    fn a_rate_worse_than_our_own_reference_allows_is_refused() {
        let x = q();
        assert_eq!(
            check(&x, x.send_wei, OURS, &[GOOD], 120_000),
            Err(QuoteRefusal::RateOutOfBounds { expect_sats: 100_000, floor_sats: 120_000 }),
            "the bridge's own number is never the check"
        );
    }
}
