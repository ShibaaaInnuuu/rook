//! The vault's calls, and the EIP-1559 envelope they travel in. Pure: no network, no keys (`docs/v2/buyer-loop.md`).
//!
//! Every selector and calldata here is checked against Foundry's `cast`, and the envelope against the transaction the
//! rehearsal got mined on chain 46630, so none of it is checked only against a second copy of my own reading of the ABI
//! or of EIP-1559.

use sha3::{Digest, Keccak256};

pub fn keccak(b: &[u8]) -> [u8; 32] {
    Keccak256::digest(b).into()
}

// ---- the vault's ABI -------------------------------------------------------------------------------------------------------

/// `announce(uint256,uint256,string)`
pub const ANNOUNCE: [u8; 4] = [0xe7, 0x2b, 0xfb, 0x75];
/// `pull(uint256)`
pub const PULL: [u8; 4] = [0x4d, 0x03, 0x92, 0xa8];
/// `settleRune(uint256,uint256,bytes32,uint64)`
pub const SETTLE_RUNE: [u8; 4] = [0x78, 0x8d, 0xbc, 0x70];
/// `returnFunds(uint256)`
pub const RETURN_FUNDS: [u8; 4] = [0x45, 0x75, 0x5d, 0xd6];
/// `purchase(uint256)` (view)
pub const PURCHASE: [u8; 4] = [0xef, 0xef, 0x39, 0xa1];
/// `purchaseCount()` (view)
pub const PURCHASE_COUNT: [u8; 4] = [0x2b, 0x85, 0xed, 0x9c];

fn word(v: u128) -> [u8; 32] {
    let mut w = [0u8; 32];
    w[16..].copy_from_slice(&v.to_be_bytes());
    w
}

fn call(sel: [u8; 4], words: &[[u8; 32]]) -> Vec<u8> {
    let mut out = sel.to_vec();
    for w in words {
        out.extend_from_slice(w);
    }
    out
}

pub fn announce(max_eth_wei: u128, quantity: u128, cap_ref: &str) -> Vec<u8> {
    let mut out = call(ANNOUNCE, &[word(max_eth_wei), word(quantity), word(0x60)]);
    out.extend_from_slice(&word(cap_ref.len() as u128));
    let mut data = cap_ref.as_bytes().to_vec();
    data.resize(cap_ref.len().div_ceil(32) * 32, 0);
    out.extend(data);
    out
}

pub fn pull(id: u64) -> Vec<u8> {
    call(PULL, &[word(id as u128)])
}

/// `btc_txid` is in Bitcoin's display order (the one an explorer shows and a person searches for), which is the order
/// `psbt_txid` returns.
pub fn settle_rune(id: u64, amount_received: u128, btc_txid: [u8; 32], sats_paid: u64) -> Vec<u8> {
    call(SETTLE_RUNE, &[word(id as u128), word(amount_received), btc_txid, word(sats_paid as u128)])
}

pub fn return_funds(id: u64) -> Vec<u8> {
    call(RETURN_FUNDS, &[word(id as u128)])
}

pub fn purchase(id: u64) -> Vec<u8> {
    call(PURCHASE, &[word(id as u128)])
}

pub fn purchase_count() -> Vec<u8> {
    PURCHASE_COUNT.to_vec()
}

/// `AutoBuyerVault.Purchase`, as `purchase(id)` returns it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub struct VaultPurchase {
    /// 0 None, 1 Announced, 2 Pulled, 3 Settled, 4 Returned, 5 WrittenOff
    pub status: u8,
    pub ready_at: u64,
    pub ready_at_block: u64,
    pub deadline: u64,
    pub late: bool,
    pub max_eth: u128,
    pub quantity: u128,
    pub settled_quantity: u128,
    pub returned: u128,
    pub cap_ref: String,
}

fn read_word(b: &[u8], at: usize) -> Result<&[u8], String> {
    b.get(at..at + 32).ok_or_else(|| format!("return data ends before byte {}", at + 32))
}

/// A word that must fit in `bits` bits; anything wider is refused rather than truncated (the out-of-range rune id lesson).
fn uint(b: &[u8], at: usize, bits: u32) -> Result<u128, String> {
    let w = read_word(b, at)?;
    let lead = 32 - (bits as usize).div_ceil(8);
    if w[..lead].iter().any(|&x| x != 0) {
        return Err(format!("a value at byte {at} is wider than {bits} bits"));
    }
    let v = u128::from_be_bytes(w[16..].try_into().expect("16 bytes"));
    if bits < 128 && v >> bits != 0 {
        return Err(format!("a value at byte {at} is wider than {bits} bits"));
    }
    Ok(v)
}

pub fn decode_count(ret: &[u8]) -> Result<u64, String> {
    Ok(uint(ret, 0, 64)? as u64)
}

pub fn decode_purchase(ret: &[u8]) -> Result<VaultPurchase, String> {
    let base = uint(ret, 0, 32)? as usize; // the tuple is dynamic (it holds a string), so it is behind an offset
    let f = |i: usize, bits: u32| uint(ret, base + 32 * i, bits);
    let status = f(0, 8)? as u8;
    if status > 5 {
        return Err(format!("status {status} is not one the vault has"));
    }
    let late = f(4, 8)?;
    if late > 1 {
        return Err("late is not a bool".into());
    }
    let s_off = base + f(9, 32)? as usize;
    let len = uint(ret, s_off, 32)? as usize;
    let bytes = ret.get(s_off + 32..s_off + 32 + len).ok_or("capRef runs past the return data")?;
    Ok(VaultPurchase {
        status,
        ready_at: f(1, 64)? as u64,
        ready_at_block: f(2, 64)? as u64,
        deadline: f(3, 64)? as u64,
        late: late == 1,
        max_eth: f(5, 128)?,
        quantity: f(6, 128)?,
        settled_quantity: f(7, 128)?,
        returned: f(8, 128)?,
        cap_ref: String::from_utf8(bytes.to_vec()).map_err(|_| "capRef is not UTF-8")?,
    })
}

// ---- RLP and the EIP-1559 envelope -----------------------------------------------------------------------------------------

fn rlp_len(len: usize, short: u8, long: u8) -> Vec<u8> {
    if len < 56 {
        vec![short + len as u8]
    } else {
        let b = (len as u64).to_be_bytes();
        let b: Vec<u8> = b.iter().copied().skip_while(|&x| x == 0).collect();
        let mut out = vec![long + b.len() as u8];
        out.extend(b);
        out
    }
}

fn rlp_bytes(b: &[u8]) -> Vec<u8> {
    if b.len() == 1 && b[0] < 0x80 {
        return b.to_vec();
    }
    let mut out = rlp_len(b.len(), 0x80, 0xb7);
    out.extend_from_slice(b);
    out
}

/// An RLP integer: big-endian, minimal width, zero as the empty string.
fn rlp_uint(v: u128) -> Vec<u8> {
    let b: Vec<u8> = v.to_be_bytes().iter().copied().skip_while(|&x| x == 0).collect();
    rlp_bytes(&b)
}

fn rlp_list(items: &[Vec<u8>]) -> Vec<u8> {
    let body: Vec<u8> = items.concat();
    let mut out = rlp_len(body.len(), 0xc0, 0xf7);
    out.extend(body);
    out
}

/// An EIP-1559 transaction to the vault (or anywhere). No access list: the buyer never sends one.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub struct Tx1559 {
    pub chain_id: u64,
    pub nonce: u64,
    pub max_priority_fee: u128,
    pub max_fee: u128,
    pub gas: u64,
    pub to: [u8; 20],
    pub value: u128,
    pub data: Vec<u8>,
}

impl Tx1559 {
    fn fields(&self) -> Vec<Vec<u8>> {
        vec![
            rlp_uint(self.chain_id as u128),
            rlp_uint(self.nonce as u128),
            rlp_uint(self.max_priority_fee),
            rlp_uint(self.max_fee),
            rlp_uint(self.gas as u128),
            rlp_bytes(&self.to),
            rlp_uint(self.value),
            rlp_bytes(&self.data),
            rlp_list(&[]),
        ]
    }

    /// What is signed: keccak(0x02 ‖ rlp([chainId, nonce, tip, maxFee, gas, to, value, data, accessList])).
    pub fn signing_hash(&self) -> [u8; 32] {
        let mut b = vec![0x02];
        b.extend(rlp_list(&self.fields()));
        keccak(&b)
    }

    /// The bytes a node takes: the same list plus yParity, r, s (r and s as minimal-width integers).
    pub fn signed(&self, y_parity: u8, r: &[u8; 32], s: &[u8; 32]) -> Vec<u8> {
        let mut f = self.fields();
        f.push(rlp_uint(y_parity as u128));
        f.push(rlp_uint_bytes(r));
        f.push(rlp_uint_bytes(s));
        let mut b = vec![0x02];
        b.extend(rlp_list(&f));
        b
    }
}

fn rlp_uint_bytes(b: &[u8; 32]) -> Vec<u8> {
    let t: Vec<u8> = b.iter().copied().skip_while(|&x| x == 0).collect();
    rlp_bytes(&t)
}

pub fn parse_address(s: &str) -> Result<[u8; 20], String> {
    let h = s.trim().trim_start_matches("0x");
    let b = hex::decode(h).map_err(|e| format!("not an address: {e}"))?;
    b.try_into().map_err(|_| format!("an address is 20 bytes: {s}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(s: &str) -> Vec<u8> {
        hex::decode(s.trim_start_matches("0x")).unwrap()
    }

    #[test]
    fn selectors_are_the_keccak_of_their_signatures() {
        for (sig, sel) in [
            ("announce(uint256,uint256,string)", ANNOUNCE),
            ("pull(uint256)", PULL),
            ("settleRune(uint256,uint256,bytes32,uint64)", SETTLE_RUNE),
            ("returnFunds(uint256)", RETURN_FUNDS),
            ("purchase(uint256)", PURCHASE),
            ("purchaseCount()", PURCHASE_COUNT),
        ] {
            assert_eq!(keccak(sig.as_bytes())[..4], sel, "{sig}");
        }
    }

    #[test]
    fn calldata_matches_foundrys_cast_byte_for_byte() {
        // `cast calldata ...`, 2026-09-23
        assert_eq!(announce(1_000_000_000_000_000_000, 1000, "cap:https://x/y#3"), h("0xe72bfb750000000000000000000000000000000000000000000000000de0b6b3a764000000000000000000000000000000000000000000000000000000000000000003e8000000000000000000000000000000000000000000000000000000000000006000000000000000000000000000000000000000000000000000000000000000116361703a68747470733a2f2f782f792333000000000000000000000000000000"));
        assert_eq!(pull(7), h("0x4d0392a80000000000000000000000000000000000000000000000000000000000000007"));
        assert_eq!(settle_rune(7, 1000, [0xab; 32], 5000), h("0x788dbc70000000000000000000000000000000000000000000000000000000000000000700000000000000000000000000000000000000000000000000000000000003e8abababababababababababababababababababababababababababababababab0000000000000000000000000000000000000000000000000000000000001388"));
        assert_eq!(return_funds(7), h("0x45755dd60000000000000000000000000000000000000000000000000000000000000007"));
        assert_eq!(purchase(7), h("0xefef39a10000000000000000000000000000000000000000000000000000000000000007"));
        assert_eq!(purchase_count(), h("0x2b85ed9c"));
        // a cap reference of exactly 32 bytes takes one data word, and an empty one none
        assert_eq!(announce(1, 1, &"a".repeat(32)).len(), 4 + 32 * 5);
        assert_eq!(announce(1, 1, "").len(), 4 + 32 * 4);
    }

    #[test]
    fn a_purchase_decodes_from_what_cast_encodes() {
        // `cast abi-encode "f((uint8,uint64,uint64,uint64,bool,uint256,uint256,uint256,uint256,string))" "(2,...)"`
        let ret = h("0x00000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000006ab13b800000000000000000000000000000000000000000000000000000000007459280000000000000000000000000000000000000000000000000000000006ab5300000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000de0b6b3a764000000000000000000000000000000000000000000000000000000000000000003e8000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000003782dace9d90000000000000000000000000000000000000000000000000000000000000000014000000000000000000000000000000000000000000000000000000000000000116361703a68747470733a2f2f782f792333000000000000000000000000000000");
        assert_eq!(decode_purchase(&ret), Ok(VaultPurchase {
            status: 2, ready_at: 1_790_000_000, ready_at_block: 122_000_000, deadline: 1_790_259_200, late: false,
            max_eth: 1_000_000_000_000_000_000, quantity: 1000, settled_quantity: 0, returned: 250_000_000_000_000_000,
            cap_ref: "cap:https://x/y#3".into(),
        }));
        // refused, not truncated: a status the vault does not have, a word too wide, data that stops short
        let mut bad = ret.clone();
        bad[32 + 31] = 9;
        assert!(decode_purchase(&bad).unwrap_err().contains("status 9"));
        let mut wide = ret.clone();
        wide[32 + 32 + 20] = 1; // readyAt is a uint64
        assert!(decode_purchase(&wide).unwrap_err().contains("wider than 64 bits"));
        assert!(decode_purchase(&ret[..200]).is_err());
        assert_eq!(decode_count(&word(3)), Ok(3));
    }

    #[test]
    fn the_envelope_rebuilds_the_transaction_the_rehearsal_got_mined() {
        // chain 46630, nonce 1, mined 2026-09-22 as 0xb0de664d...; its signing hash, r, s and parity from .evm-rehearsal.json
        let tx = Tx1559 { chain_id: 46630, nonce: 1, max_priority_fee: 0, max_fee: 40_000_000, gas: 41_392,
            to: parse_address("0x5b0b0649f0306d08b4c0ad37406a7082fc25ebc2").unwrap(), value: 0, data: vec![] };
        assert_eq!(hex::encode(tx.signing_hash()), "66dca7bb78f0741da06e4d1a092798806c1e08753bb0450b11350bb39a585fb2");
        let r: [u8; 32] = h("ed058429f5854747ef2599377fa0a8c13d72df31bbc556113f315b556286d774").try_into().unwrap();
        let s: [u8; 32] = h("36676166f86adb87da22ff1a6c1ff288ba3de6752966b75fbf4b46463be90e86").try_into().unwrap();
        let raw = tx.signed(0, &r, &s);
        assert_eq!(hex::encode(&raw), "02f86882b62601808402625a0082a1b0945b0b0649f0306d08b4c0ad37406a7082fc25ebc28080c080a0ed058429f5854747ef2599377fa0a8c13d72df31bbc556113f315b556286d774a036676166f86adb87da22ff1a6c1ff288ba3de6752966b75fbf4b46463be90e86");
        assert_eq!(hex::encode(keccak(&raw)), "b0de664d35e3df8523466d20f2e5c02121bd762b69bcbfcb417c2b6dbd8f95ff");
        assert_eq!(crate::evmsend::chain_id_of(&raw), Ok(46630));
        // a long payload takes the long-form list header, and the chain id still reads back
        let big = Tx1559 { data: vec![0xaa; 300], ..tx.clone() };
        assert_eq!(crate::evmsend::chain_id_of(&big.signed(1, &r, &s)), Ok(46630));
    }
}
