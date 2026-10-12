//! rook_core's `math.rs`, `clock.rs`, `units.rs` and `hashing.rs`, ported for the canister's books (docs/v9/canister-only-solana.md
//! §4.1). Checked `u128` arithmetic with explicit rounding, the clock constants, the 9-decimal units, keccak Merkle membership and
//! the taproot check. Line for line with `solana/programs/rook_core/src/{math,clock,units,hashing}.rs`; only the error type changes
//! (`RookError` → [`BooksError`]).

use sha3::{Digest, Keccak256};

use super::BooksError;

pub type Res<T> = Result<T, BooksError>;

// ---- units (rook_core units.rs) ----------------------------------------------------------------------------------------------

/// One whole wrapped item, in wX base units (9 decimals; the SPL mint the canister issues).
pub const ONE: u64 = 1_000_000_000;
/// The SPL decimals of every wX mint.
pub const WX_DECIMALS: u8 = 9;
pub const LAMPORTS_PER_SOL: u64 = 1_000_000_000;
pub const BPS: u64 = 10_000;

// ---- clock (rook_core clock.rs) ----------------------------------------------------------------------------------------------

pub const MINUTE: i64 = 60;
pub const HOUR: i64 = 3_600;
pub const DAY: i64 = 86_400;

/// Seconds since `at`, 0 if `at` is in the future.
pub fn elapsed(now: i64, at: i64) -> i64 {
    if now > at {
        now - at
    } else {
        0
    }
}

/// The drand evmnet chain: genesis and period (as `drand.rs`).
pub const DRAND_GENESIS: i64 = 1_727_521_075;
pub const DRAND_PERIOD: i64 = 3;

/// `DrandEvmnet.roundAt`: the round live at `t` (0 before genesis).
pub fn round_at(t: i64) -> u64 {
    if t < DRAND_GENESIS {
        0
    } else {
        ((t - DRAND_GENESIS) / DRAND_PERIOD) as u64 + 1
    }
}

// ---- math (rook_core math.rs) ------------------------------------------------------------------------------------------------

/// `⌊a·b/d⌋`, refused on `d == 0` or a result above `u64::MAX`.
pub fn mul_div(a: u64, b: u64, d: u64) -> Res<u64> {
    if d == 0 {
        return Err(BooksError::MathOverflow);
    }
    let r = (a as u128) * (b as u128) / (d as u128);
    u64::try_from(r).map_err(|_| BooksError::MathOverflow)
}

/// `⌈a·b/d⌉`.
pub fn mul_div_ceil(a: u64, b: u64, d: u64) -> Res<u64> {
    if d == 0 {
        return Err(BooksError::MathOverflow);
    }
    let n = (a as u128) * (b as u128);
    let d = d as u128;
    let r = n / d + if n % d == 0 { 0 } else { 1 };
    u64::try_from(r).map_err(|_| BooksError::MathOverflow)
}

pub fn add(a: u64, b: u64) -> Res<u64> {
    a.checked_add(b).ok_or(BooksError::MathOverflow)
}

pub fn sub(a: u64, b: u64) -> Res<u64> {
    a.checked_sub(b).ok_or(BooksError::MathOverflow)
}

// ---- hashing (rook_core hashing.rs) ------------------------------------------------------------------------------------------

pub fn keccak(parts: &[&[u8]]) -> [u8; 32] {
    let mut h = Keccak256::new();
    for p in parts {
        h.update(p);
    }
    h.finalize().into()
}

/// `ListingRegistry.leafOf`: `keccak(keccak(inscription_id))`.
pub fn leaf_of(inscription_id: &str) -> [u8; 32] {
    let inner = keccak(&[inscription_id.as_bytes()]);
    keccak(&[&inner])
}

/// OpenZeppelin `MerkleProof.verify` with sorted pairs.
pub fn verify_proof(proof: &[[u8; 32]], root: &[u8; 32], leaf: [u8; 32]) -> bool {
    let mut h = leaf;
    for p in proof {
        h = if h <= *p { keccak(&[&h, p]) } else { keccak(&[p, &h]) };
    }
    h == *root
}

const BECH32_ALPHABET: &[u8; 32] = b"qpzry9x8gf2tvdw0s3jn54khce6mua7l";
const BECH32M: u32 = 0x2bc8_30a3;

fn value(c: u8) -> Option<u32> {
    BECH32_ALPHABET.iter().position(|&a| a == c).map(|p| p as u32)
}

fn step(chk: u32, v: u32) -> u32 {
    let b = chk >> 25;
    let mut c = ((chk & 0x1ff_ffff) << 5) ^ v;
    for (i, g) in [0x3b6a_57b2u32, 0x2650_8e6d, 0x1ea1_19fa, 0x3d42_33dd, 0x2a14_62b3].iter().enumerate() {
        if b & (1 << i) != 0 {
            c ^= g;
        }
    }
    c
}

/// `BtcAddress.isTaproot` for the network whose bech32 prefix is `hrp` (`"bc"` or `"tb"`): 62 characters, `…1p…`, a valid
/// bech32m checksum and zero padding bits.
pub fn is_taproot_on(addr: &str, hrp: &str) -> bool {
    let a = addr.as_bytes();
    let h = hrp.as_bytes();
    if !(hrp == "bc" || hrp == "tb") || a.len() != 62 || &a[..2] != h || &a[2..4] != b"1p" {
        return false;
    }
    let mut chk: u32 = 1;
    for &c in h {
        chk = step(chk, (c >> 5) as u32);
    }
    chk = step(chk, 0);
    for &c in h {
        chk = step(chk, (c & 31) as u32);
    }
    for &c in &a[3..] {
        match value(c) {
            Some(v) => chk = step(chk, v),
            None => return false,
        }
    }
    if value(a[55]).unwrap_or(1) & 0x0f != 0 {
        return false;
    }
    chk == BECH32M
}

/// The bech32 prefix of a taproot address (`"bc"` or `"tb"`), if it is one.
pub fn taproot_hrp(addr: &str) -> Option<&'static str> {
    ["bc", "tb"].into_iter().find(|h| is_taproot_on(addr, h))
}

/// `Wrapper.BURN_ADDRESS`: BIP341's NUMS point as a key-path-only taproot output.
pub const BTC_BURN_ADDRESS: &str = "bc1p2zffkaxp5py4fdutfdsrt6t6tcrc5ks09rkfd428hlhf4n5q8tqqwv7ahk";

#[cfg(test)]
mod tests {
    use super::*;

    // rook_core math.rs: rounding_is_explicit
    #[test]
    fn rounding_is_explicit() {
        assert_eq!(mul_div(10, 3, 4).unwrap(), 7);
        assert_eq!(mul_div_ceil(10, 3, 4).unwrap(), 8);
        assert_eq!(mul_div_ceil(8, 3, 4).unwrap(), 6);
        assert!(mul_div(1, 1, 0).is_err());
        assert!(mul_div(u64::MAX, 2, 1).is_err());
        assert_eq!(mul_div(u64::MAX, u64::MAX, u64::MAX).unwrap(), u64::MAX);
    }

    #[test]
    fn the_burn_address_is_a_mainnet_taproot_and_the_hashing_matches_the_canisters() {
        assert_eq!(taproot_hrp(BTC_BURN_ADDRESS), Some("bc"));
        assert!(!is_taproot_on(BTC_BURN_ADDRESS, "tb"));
        assert_eq!(leaf_of("a"), crate::frogbuy::leaf_of("a"));
        assert_eq!(round_at(DRAND_GENESIS + 30) as u64, crate::drand::round_at((DRAND_GENESIS + 30) as u64));
    }
}
