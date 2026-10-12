//! Keccak Merkle membership (ListingRegistry.isMember, OpenZeppelin `MerkleProof` sorted pairs) and the Bitcoin taproot check
//! (src/lib/BtcAddress.sol).

pub fn keccak(parts: &[&[u8]]) -> [u8; 32] {
    solana_keccak_hasher::hashv(parts).to_bytes()
}

/// `ListingRegistry.leafOf`: `keccak256(bytes.concat(keccak256(bytes(inscriptionId))))`, hashed twice so an inner node is never
/// a leaf.
pub fn leaf_of(inscription_id: &str) -> [u8; 32] {
    let inner = keccak(&[inscription_id.as_bytes()]);
    keccak(&[&inner])
}

/// OpenZeppelin `MerkleProof.verify` with sorted pairs (`_hashPair(a, b)` = keccak of the smaller first).
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
    if b & 1 != 0 {
        c ^= 0x3b6a_57b2;
    }
    if b & 2 != 0 {
        c ^= 0x2650_8e6d;
    }
    if b & 4 != 0 {
        c ^= 0x1ea1_19fa;
    }
    if b & 8 != 0 {
        c ^= 0x3d42_33dd;
    }
    if b & 16 != 0 {
        c ^= 0x2a14_62b3;
    }
    c
}

/// `BtcAddress.isTaproot`: a mainnet `bc1p…` address of 62 characters with a valid bech32m checksum and zero padding bits.
pub fn is_taproot(addr: &str) -> bool {
    is_taproot_on(addr, "bc")
}

/// [`is_taproot`] for the network whose bech32 prefix is `hrp` (`"bc"` mainnet, `"tb"` testnet4 and signet: both are
/// 62-character `…1p…` addresses with a witness v1 32-byte program). Any other prefix is refused.
pub fn is_taproot_on(addr: &str, hrp: &str) -> bool {
    let a = addr.as_bytes();
    let h = hrp.as_bytes();
    if !(hrp == "bc" || hrp == "tb") || a.len() != 62 || &a[..2] != h || &a[2..4] != b"1p" {
        return false;
    }
    // the checksum's expansion of the prefix: high bits of each character, a zero, then the low five bits
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

/// `Wrapper.BURN_ADDRESS`: BIP341's NUMS point as a key-path-only taproot output; nobody holds its key.
pub const BTC_BURN_ADDRESS: &str = "bc1p2zffkaxp5py4fdutfdsrt6t6tcrc5ks09rkfd428hlhf4n5q8tqqwv7ahk";
/// `EthNftWrapper.DEAD`.
pub const ETH_DEAD: [u8; 20] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xde, 0xad,
];

/// The bech32 prefix of a taproot address this module accepts (`"bc"` or `"tb"`), if it is one.
pub fn taproot_hrp(addr: &str) -> Option<&'static str> {
    ["bc", "tb"].into_iter().find(|h| is_taproot_on(addr, h))
}

/// Parse `0x` + 40 hex characters.
pub fn parse_evm_address(s: &str) -> Option<[u8; 20]> {
    let b = s.as_bytes();
    if b.len() != 42 || b[0] != b'0' || (b[1] != b'x' && b[1] != b'X') {
        return None;
    }
    let mut out = [0u8; 20];
    for i in 0..20 {
        let hi = (b[2 + 2 * i] as char).to_digit(16)?;
        let lo = (b[3 + 2 * i] as char).to_digit(16)?;
        out[i] = (hi * 16 + lo) as u8;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_burn_address_is_taproot() {
        assert!(is_taproot(BTC_BURN_ADDRESS));
        let mut bad = BTC_BURN_ADDRESS.to_string();
        bad.replace_range(10..11, "q");
        assert!(!is_taproot(&bad));
        assert!(!is_taproot("bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq"));
    }

    /// BIP341's NUMS key-path output on signet/testnet4 (same witness program as the mainnet burn address).
    const TB_BURN: &str = "tb1p2zffkaxp5py4fdutfdsrt6t6tcrc5ks09rkfd428hlhf4n5q8tqqeygjde";

    #[test]
    fn taproot_follows_the_network() {
        assert!(is_taproot_on(BTC_BURN_ADDRESS, "bc"));
        assert!(!is_taproot_on(BTC_BURN_ADDRESS, "tb"), "a mainnet address is not a signet one");
        assert!(is_taproot_on(TB_BURN, "tb"));
        assert!(!is_taproot_on(TB_BURN, "bc"));
        assert!(!is_taproot(TB_BURN), "is_taproot stays mainnet-only");
        assert_eq!(taproot_hrp(TB_BURN), Some("tb"));
        assert_eq!(taproot_hrp(BTC_BURN_ADDRESS), Some("bc"));
        assert_eq!(taproot_hrp("bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq"), None);
        // the prefix swapped without recomputing the checksum is refused
        let swapped = BTC_BURN_ADDRESS.replacen("bc", "tb", 1);
        assert!(!is_taproot_on(&swapped, "tb"));
        assert!(!is_taproot_on(TB_BURN, "bcrt"), "regtest is not a network the registry serves");
    }

    #[test]
    fn proofs_use_sorted_pairs() {
        let a = leaf_of("a");
        let b = leaf_of("b");
        let root = if a <= b { keccak(&[&a, &b]) } else { keccak(&[&b, &a]) };
        assert!(verify_proof(&[b], &root, a));
        assert!(verify_proof(&[a], &root, b));
        assert!(!verify_proof(&[a], &root, a));
    }

    #[test]
    fn evm_addresses_parse() {
        assert_eq!(parse_evm_address("0x000000000000000000000000000000000000dEaD"), Some(ETH_DEAD));
        assert_eq!(parse_evm_address("0x00"), None);
    }
}
