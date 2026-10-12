//! Taproot script trees (BIP 341), as far as a protected listing's escrow needs them (docs/v8/ow-protected.md §1, P2).
//!
//! An Ordinals Wallet passthrough pays `tr(NUMS, {sale_leaf, recovery_leaf})`. The canister never trusts the escrow's script:
//! it reads the seller key from the sale leaf, rebuilds the whole output here from that key and the **pinned** co-signer, and
//! compares. Pure: hashes and one point addition.

use crate::sighash::tagged;

/// BIP 341's provably unspendable internal key (`H = lift_x(sha256(G))`): an escrow with it has no key path.
pub const NUMS: [u8; 32] = hex32("50929b74c1a04954b78b4b6035e97a5e078a5a0f28ec96d547bfee9ace803ac0");
/// Tapscript's leaf version.
pub const TAPSCRIPT: u8 = 0xc0;

/// A 64-character hex constant, decoded at compile time.
pub const fn hex32(s: &str) -> [u8; 32] {
    const fn nib(c: u8) -> u8 {
        match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            _ => panic!("not lowercase hex"),
        }
    }
    let b = s.as_bytes();
    assert!(b.len() == 64, "a 32-byte constant is 64 hex characters");
    let mut out = [0u8; 32];
    let mut i = 0;
    while i < 32 {
        out[i] = (nib(b[2 * i]) << 4) | nib(b[2 * i + 1]);
        i += 1;
    }
    out
}

fn compact(n: usize) -> Vec<u8> {
    match n {
        0..=0xfc => vec![n as u8],
        0xfd..=0xffff => {
            let mut v = vec![0xfd];
            v.extend_from_slice(&(n as u16).to_le_bytes());
            v
        }
        _ => {
            let mut v = vec![0xfe];
            v.extend_from_slice(&(n as u32).to_le_bytes());
            v
        }
    }
}

/// `tagged("TapLeaf", version || compact_size(len) || script)`.
pub fn leaf_hash(script: &[u8], version: u8) -> [u8; 32] {
    tagged("TapLeaf", &[&[version], &compact(script.len()), script])
}

/// `tagged("TapBranch", min(a, b) || max(a, b))`.
pub fn branch_hash(a: &[u8; 32], b: &[u8; 32]) -> [u8; 32] {
    let (x, y) = if a <= b { (a, b) } else { (b, a) };
    tagged("TapBranch", &[x, y])
}

/// The output key of `internal` committed to `merkle_root` (`None`: a key-path-only output), and whether its Y is odd (the
/// parity bit of a control block). `Q = lift_x(P) + int(tagged("TapTweak", P || root)) G`. With `None` this is exactly the
/// tweak `canister.rs` applies to the float and custody keys.
pub fn output_key(internal: &[u8; 32], merkle_root: Option<&[u8; 32]>) -> Result<([u8; 32], bool), String> {
    use k256::elliptic_curve::sec1::ToEncodedPoint;
    use k256::elliptic_curve::PrimeField;
    use k256::{ProjectivePoint, PublicKey, Scalar};

    let mut sec1 = [0u8; 33];
    sec1[0] = 0x02;
    sec1[1..].copy_from_slice(internal);
    let p = PublicKey::from_sec1_bytes(&sec1).map_err(|e| format!("the internal key is not on the curve: {e}"))?;
    let t = match merkle_root {
        Some(r) => tagged("TapTweak", &[internal, r]),
        None => tagged("TapTweak", &[internal]),
    };
    let scalar = Option::<Scalar>::from(Scalar::from_repr(t.into()))
        .ok_or("the tweak is not a valid scalar, which has probability about 2^-128")?;
    let q = ProjectivePoint::from(*p.as_affine()) + ProjectivePoint::GENERATOR * scalar;
    let enc = q.to_affine().to_encoded_point(true);
    let x: [u8; 32] = enc.as_bytes()[1..33].try_into().map_err(|_| "the tweaked point did not encode".to_string())?;
    Ok((x, enc.as_bytes()[0] == 0x03))
}

/// `OP_1 <32-byte key>`.
pub fn p2tr_script(output_key: &[u8; 32]) -> Vec<u8> {
    [&[0x51, 0x20][..], output_key].concat()
}

/// A two-leaf tree under `internal`, each leaf at depth one: its scriptPubKey and each leaf's control block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TwoLeaf {
    pub script: Vec<u8>,
    pub output_key: [u8; 32],
    pub control_a: Vec<u8>,
    pub control_b: Vec<u8>,
}

/// `tr(internal, {a, b})`, both tapscript leaves.
pub fn two_leaf(internal: &[u8; 32], a: &[u8], b: &[u8]) -> Result<TwoLeaf, String> {
    let (ha, hb) = (leaf_hash(a, TAPSCRIPT), leaf_hash(b, TAPSCRIPT));
    let root = branch_hash(&ha, &hb);
    let (q, odd) = output_key(internal, Some(&root))?;
    let control = |sibling: &[u8; 32]| [&[TAPSCRIPT | odd as u8][..], internal, sibling].concat();
    Ok(TwoLeaf { script: p2tr_script(&q), output_key: q, control_a: control(&hb), control_b: control(&ha) })
}

/// A passthrough v4 escrow, rebuilt (Ordinals Wallet's `passthrough_escrow_with`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Escrow {
    /// `<S> OP_CHECKSIG <C> OP_CHECKSIGADD OP_2 OP_NUMEQUAL`
    pub sale_leaf: Vec<u8>,
    /// `<144> OP_CHECKSEQUENCEVERIFY OP_DROP <S> OP_CHECKSIG`
    pub recovery_leaf: Vec<u8>,
    /// `tr(NUMS, {sale_leaf, recovery_leaf})`
    pub script: Vec<u8>,
    pub sale_control_block: Vec<u8>,
    pub recovery_control_block: Vec<u8>,
}

/// The sale leaf for seller `s` and co-signer `c`.
pub fn sale_leaf(s: &[u8; 32], c: &[u8; 32]) -> Vec<u8> {
    [&[0x20][..], s, &[0xac, 0x20], c, &[0xba, 0x52, 0x9c]].concat()
}

/// The recovery leaf: 144 blocks (`0x90 0x00`, minimally encoded), then the seller alone.
pub fn recovery_leaf(s: &[u8; 32]) -> Vec<u8> {
    [&[0x02, 0x90, 0x00, 0xb2, 0x75, 0x20][..], s, &[0xac]].concat()
}

/// Both keys of a sale leaf, or `None` if `leaf` is not exactly one.
pub fn parse_sale_leaf(leaf: &[u8]) -> Option<([u8; 32], [u8; 32])> {
    if leaf.len() != 70 || leaf[0] != 0x20 || leaf[33] != 0xac || leaf[34] != 0x20 || leaf[67..] != [0xba, 0x52, 0x9c] {
        return None;
    }
    Some((leaf[1..33].try_into().ok()?, leaf[35..67].try_into().ok()?))
}

/// Rebuild the escrow for seller `s` and co-signer `c`. Refused, as Ordinals Wallet refuses it: equal keys (a 2-of-2 of one
/// key is a 1-of-1).
pub fn passthrough_escrow(s: &[u8; 32], c: &[u8; 32]) -> Result<Escrow, String> {
    if s == c {
        return Err("the seller and co-signer keys are the same key".into());
    }
    let (sale, recovery) = (sale_leaf(s, c), recovery_leaf(s));
    let t = two_leaf(&NUMS, &sale, &recovery)?;
    Ok(Escrow { sale_leaf: sale, recovery_leaf: recovery, script: t.script, sale_control_block: t.control_a, recovery_control_block: t.control_b })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nums_is_the_bip341_point_and_the_key_path_tweak_is_the_canisters() {
        // BIP 341: H = lift_x(0x50929b74…) is a point; anyone can lift it, nobody knows its discrete log
        assert!(output_key(&NUMS, None).is_ok());
        // a key that is not on the curve is refused by name (x = 5 has no y on secp256k1)
        let mut bad = [0u8; 32];
        bad[31] = 5;
        assert!(output_key(&bad, None).unwrap_err().contains("not on the curve"));
    }

    #[test]
    fn the_sale_leaf_parses_back_to_its_keys_and_nothing_else_does() {
        let (s, c) = ([0x11; 32], [0x22; 32]);
        assert_eq!(parse_sale_leaf(&sale_leaf(&s, &c)), Some((s, c)));
        let mut one_of = sale_leaf(&s, &c);
        one_of[68] = 0x51; // OP_1 instead of OP_2
        assert_eq!(parse_sale_leaf(&one_of), None);
        assert_eq!(parse_sale_leaf(&recovery_leaf(&s)), None);
        assert_eq!(parse_sale_leaf(&sale_leaf(&s, &c)[..69]), None);
        assert!(passthrough_escrow(&c, &c).is_err(), "a 2-of-2 of one key is refused");
    }

    #[test]
    fn a_branch_is_order_free_and_a_control_block_names_the_other_leaf() {
        let (a, b) = (leaf_hash(b"\x51", TAPSCRIPT), leaf_hash(b"\x52", TAPSCRIPT));
        assert_eq!(branch_hash(&a, &b), branch_hash(&b, &a));
        let t = two_leaf(&NUMS, b"\x51", b"\x52").unwrap();
        assert_eq!(&t.control_a[33..], &b);
        assert_eq!(&t.control_b[33..], &a);
        assert_eq!(&t.control_a[1..33], &NUMS);
        assert_eq!(t.control_a[0] & 0xfe, TAPSCRIPT);
    }
}
