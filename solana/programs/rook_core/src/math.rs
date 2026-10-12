//! Checked `u128` arithmetic: OpenZeppelin's `Math.mulDiv` for `u64` amounts. Every call names its rounding direction, as
//! the Solidity did (`Math.Rounding.Ceil` where the source rounds up, floor otherwise).

use crate::error::RookError;
use anchor_lang::prelude::*;

/// `⌊a·b/d⌋`, refused on `d == 0` or a result above `u64::MAX`.
pub fn mul_div(a: u64, b: u64, d: u64) -> Result<u64> {
    require!(d != 0, RookError::MathOverflow);
    let r = (a as u128) * (b as u128) / (d as u128);
    u64::try_from(r).map_err(|_| error!(RookError::MathOverflow))
}

/// `⌈a·b/d⌉`.
pub fn mul_div_ceil(a: u64, b: u64, d: u64) -> Result<u64> {
    require!(d != 0, RookError::MathOverflow);
    let n = (a as u128) * (b as u128);
    let d = d as u128;
    let r = n / d + if n % d == 0 { 0 } else { 1 };
    u64::try_from(r).map_err(|_| error!(RookError::MathOverflow))
}

pub fn add(a: u64, b: u64) -> Result<u64> {
    a.checked_add(b).ok_or_else(|| error!(RookError::MathOverflow))
}

pub fn sub(a: u64, b: u64) -> Result<u64> {
    a.checked_sub(b).ok_or_else(|| error!(RookError::MathOverflow))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounding_is_explicit() {
        assert_eq!(mul_div(10, 3, 4).unwrap(), 7);
        assert_eq!(mul_div_ceil(10, 3, 4).unwrap(), 8);
        assert_eq!(mul_div_ceil(8, 3, 4).unwrap(), 6);
        assert!(mul_div(1, 1, 0).is_err());
        assert!(mul_div(u64::MAX, 2, 1).is_err());
        assert_eq!(mul_div(u64::MAX, u64::MAX, u64::MAX).unwrap(), u64::MAX);
    }
}
