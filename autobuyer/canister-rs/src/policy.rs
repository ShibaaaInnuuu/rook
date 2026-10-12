//! When the buyer opens a purchase: an on-chain trigger with no operator discretion (`docs/v2/venue-bridge-policy.md` §1).
//!
//! Everything here is pure. The loop reads a [`Snapshot`] of the vault and its listing (replicated, providers agreeing),
//! asks [`candidates`] for the amounts worth trying, asks the vault's `payersFor` about each until one fits, and asks
//! [`order_for`] what to open. Every input is on chain; the constants are code, so changing them passes the timelock.

use crate::abi::{addr_at, bool_at, bytes_at, encode_call, uint_at, uint_word, Arg};

/// Below this, the route's fixed costs are too large a share of a purchase to be worth it.
pub const MIN_PURCHASE_WEI: u128 = 50_000_000_000_000_000; // 0.05 ETH
/// The most one purchase takes, whatever the vault could fund: a bound on what one bad venue or bridge costs.
pub const MAX_PURCHASE_WEI: u128 = 500_000_000_000_000_000; // 0.5 ETH
/// The keeper's cap must stay valid at least this long: `announce` lands a few blocks after the decision.
pub const CAP_MARGIN_SECS: u64 = 15 * 60;
/// The route's fixed costs may be at most this share of the amount, in basis points.
pub const MAX_COST_BPS: u128 = 500; // 5%
/// The vault's own limit on how many coins one announcement may charge.
pub const MAX_PAYERS: u64 = 64;

/// A listing as the registry holds it (`ListingRegistry.Listing`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing {
    pub is_rune: bool,
    pub paused: bool,
    pub autonomous: bool,
    pub divisibility: u8,
    pub vault: [u8; 20],
    /// A rune's id, "block:tx".
    pub reference: String,
    /// The Bitcoin address the asset must land on.
    pub btc_vault: String,
}

/// What the policy decides from: the listing and the vault, read at one moment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub now: u64,
    pub listing: Listing,
    pub attributed: u128,
    pub cap_wei: u128,
    pub cap_expiry: u64,
    pub max_in_flight: u128,
    pub in_flight: u128,
    pub open: u64,
    pub blocked: bool,
}

/// What the policy opens: the same fields `open_purchase` takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Opening {
    pub amount_wei: u128,
    pub rune: String,
    pub units: u128,
    pub recipient_script: Vec<u8>,
    pub cap_ref: String,
}

/// The amounts worth asking `payersFor` about, largest first (halving down to the minimum), or why nothing is. `cost_wei` is
/// the route's fixed cost for a purchase (bridge fee, Bitcoin fee, gas), from our own estimate.
pub fn candidates(s: &Snapshot, cost_wei: u128) -> Result<Vec<u128>, String> {
    let l = &s.listing;
    if !l.is_rune { return Err("the listing is not a rune: the loop settles runes only".into()); }
    if l.paused { return Err("the listing is paused".into()); }
    if s.blocked { return Err("the vault is blocked by an overdue purchase".into()); }
    if s.open != 0 { return Err(format!("the vault has {} open purchase(s)", s.open)); }
    if s.cap_wei == 0 { return Err("no cap has ever been posted".into()); }
    if s.cap_expiry < s.now.saturating_add(CAP_MARGIN_SECS) {
        return Err(format!("the cap expires at {}, inside the {CAP_MARGIN_SECS} s margin", s.cap_expiry));
    }
    let top = s.attributed.min(s.max_in_flight.saturating_sub(s.in_flight)).min(MAX_PURCHASE_WEI);
    if top < MIN_PURCHASE_WEI {
        return Err(format!("{top} wei is available, under the {MIN_PURCHASE_WEI} minimum"));
    }
    let mut out = Vec::new();
    let mut a = top;
    while a >= MIN_PURCHASE_WEI {
        // the costs are fixed per purchase, so a smaller amount only makes their share worse: stop at the first that fails
        if cost_wei.saturating_mul(10_000) > a.saturating_mul(MAX_COST_BPS) {
            break;
        }
        out.push(a);
        a /= 2;
    }
    if out.is_empty() {
        return Err(format!("the route costs {cost_wei} wei, over {}% of {top}", MAX_COST_BPS as f64 / 100.0));
    }
    Ok(out)
}

/// The first candidate the vault can charge within its payer limit (`payers[i]` is `payersFor(candidates[i])`; 0 means the
/// vault cannot fund it).
pub fn first_fundable(candidates: &[u128], payers: &[u64]) -> Option<u128> {
    candidates.iter().zip(payers).find(|(_, p)| (1..=MAX_PAYERS).contains(*p)).map(|(a, _)| *a)
}

/// What to open for `amount`: the fewest smallest units the vault's bound allows for that ETH (so the buyer never promises
/// more than the cap price buys), the listing's rune, its Bitcoin address as a script, and where the cap came from.
pub fn order_for(s: &Snapshot, amount: u128, hrp: &str) -> Result<Opening, String> {
    let scale = 10u128.checked_pow(s.listing.divisibility as u32).ok_or("divisibility too large")?;
    let num = amount.checked_mul(scale).ok_or("amount × 10^divisibility overflows")?;
    let units = num.div_ceil(s.cap_wei);
    // the vault's own check, restated: boundFor(units) = cap × units / 10^div must reach the amount
    let bound = s.cap_wei.checked_mul(units).ok_or("cap × units overflows")? / scale;
    if bound < amount || units == 0 {
        return Err(format!("{units} units bound only {bound} wei, under {amount}"));
    }
    crate::looper::parse_rune(&s.listing.reference)?;
    Ok(Opening {
        amount_wei: amount,
        rune: s.listing.reference.clone(),
        units,
        recipient_script: script_of(&s.listing.btc_vault, hrp)?,
        cap_ref: format!("cap:{}@{}", s.cap_wei, s.cap_expiry),
    })
}

/// The address prefix for the installed Bitcoin network.
pub fn hrp_for(network: &str) -> Result<&'static str, String> {
    match network {
        "mainnet" | "bitcoin" => Ok("bc"),
        "signet" | "testnet" | "testnet4" => Ok("tb"),
        "regtest" => Ok("bcrt"),
        other => Err(format!("no address prefix for {other:?}")),
    }
}

// ---- the reads, as calls ------------------------------------------------------------------------------------------------------

pub const REGISTRY: [u8; 4] = [0x7b, 0x10, 0x39, 0x99];
pub const LISTING_ID: [u8; 4] = [0x94, 0x05, 0xdf, 0xdd];
pub const ATTRIBUTED: [u8; 4] = [0xe2, 0xb9, 0x63, 0x35];
pub const CAP_WEI: [u8; 4] = [0x32, 0xe7, 0x67, 0x41];
pub const CAP_EXPIRY: [u8; 4] = [0xa6, 0x5c, 0xf7, 0xa9];
pub const MAX_IN_FLIGHT: [u8; 4] = [0x15, 0xc0, 0x40, 0xd4];
pub const IN_FLIGHT: [u8; 4] = [0x38, 0x8b, 0x17, 0x0b];
pub const OPEN_PURCHASES: [u8; 4] = [0x31, 0x47, 0x6f, 0xb5];
pub const IS_BLOCKED: [u8; 4] = [0xd7, 0x5f, 0x0d, 0xa7];
pub const PAYERS_FOR: [u8; 4] = [0x78, 0x2a, 0xf7, 0x57];
pub const LISTING: [u8; 4] = [0x84, 0xa1, 0x84, 0xde];

pub fn call0(sel: [u8; 4]) -> Vec<u8> { sel.to_vec() }
pub fn call_uint(sel: [u8; 4], v: u128) -> Vec<u8> { encode_call(sel, &[Arg::Word(uint_word(v))]) }

pub fn ret_uint(b: &[u8], bits: u32) -> Result<u128, String> { uint_at(b, 0, 0, bits) }
pub fn ret_address(b: &[u8]) -> Result<[u8; 20], String> { addr_at(b, 0, 0) }
pub fn ret_bool_first(b: &[u8]) -> Result<bool, String> { bool_at(b, 0, 0) }
/// `openPurchases()` returns `uint256[]`: the count is what the policy needs.
pub fn ret_array_len(b: &[u8]) -> Result<u64, String> {
    let off = uint_at(b, 0, 0, 32)? as usize;
    Ok(uint_at(b, off, 0, 32)? as u64)
}

/// `registry.listing(id)`: a struct holding strings, returned as one dynamic tuple.
pub fn ret_listing(b: &[u8]) -> Result<Listing, String> {
    let base = uint_at(b, 0, 0, 32)? as usize;
    let kind = uint_at(b, base, 0, 8)?;
    if kind > 1 {
        return Err(format!("listing kind {kind} is not a known kind"));
    }
    let s = |i: usize| -> Result<String, String> { String::from_utf8(bytes_at(b, base, i)?).map_err(|_| format!("field {i} is not UTF-8")) };
    Ok(Listing {
        is_rune: kind == 1,
        paused: bool_at(b, base, 1)?,
        autonomous: bool_at(b, base, 2)?,
        divisibility: uint_at(b, base, 3, 8)? as u8,
        vault: addr_at(b, base, 4)?,
        reference: s(7)?,
        btc_vault: s(8)?,
    })
}

// ---- a segwit address to its script (BIP 173 / BIP 350) ---------------------------------------------------------------------

const CHARSET: &[u8; 32] = b"qpzry9x8gf2tvdw0s3jn54khce6mua7l";

fn polymod(values: &[u8]) -> u32 {
    const G: [u32; 5] = [0x3b6a57b2, 0x26508e6d, 0x1ea119fa, 0x3d4233dd, 0x2a1462b3];
    let mut chk: u32 = 1;
    for v in values {
        let b = chk >> 25;
        chk = ((chk & 0x1ff_ffff) << 5) ^ (*v as u32);
        for (i, g) in G.iter().enumerate() {
            if (b >> i) & 1 == 1 { chk ^= g; }
        }
    }
    chk
}

/// The taproot address (`bc1p…`/`tb1p…`, bech32m) of a 32-byte x-only output key: the inverse of [`script_of`] for version 1.
pub fn taproot_address(output_key: &[u8; 32], hrp: &str) -> String {
    let mut data = vec![1u8]; // witness version 1
    let (mut acc, mut bits) = (0u32, 0u32);
    for b in output_key {
        acc = (acc << 8) | *b as u32;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            data.push(((acc >> bits) & 31) as u8);
        }
    }
    if bits > 0 {
        data.push(((acc << (5 - bits)) & 31) as u8);
    }
    let mut exp: Vec<u8> = hrp.bytes().map(|c| c >> 5).collect();
    exp.push(0);
    exp.extend(hrp.bytes().map(|c| c & 31));
    exp.extend_from_slice(&data);
    exp.extend([0u8; 6]);
    let chk = polymod(&exp) ^ 0x2bc8_30a3;
    let mut out = format!("{hrp}1");
    for d in data.iter().copied().chain((0..6).map(|i| ((chk >> (5 * (5 - i))) & 31) as u8)) {
        out.push(CHARSET[d as usize] as char);
    }
    out
}

/// The scriptPubKey an address pays: `OP_n <program>`. Refuses a wrong network prefix, a bad checksum, the wrong checksum
/// variant for the witness version (bech32 for v0, bech32m for v1+), mixed case and a malformed program.
pub fn script_of(addr: &str, hrp: &str) -> Result<Vec<u8>, String> {
    if addr.chars().any(|c| c.is_ascii_lowercase()) && addr.chars().any(|c| c.is_ascii_uppercase()) {
        return Err("mixed-case address".into());
    }
    let a = addr.to_ascii_lowercase();
    let pos = a.rfind('1').ok_or("no separator")?;
    let (h, rest) = (&a[..pos], &a[pos + 1..]);
    if h != hrp {
        return Err(format!("address is for {h:?}, the installed network is {hrp:?}"));
    }
    if rest.len() < 7 || a.len() > 90 {
        return Err("address length out of range".into());
    }
    let data: Vec<u8> = rest.bytes().map(|c| CHARSET.iter().position(|x| *x == c).map(|p| p as u8))
        .collect::<Option<_>>().ok_or("not a bech32 character")?;
    let mut exp: Vec<u8> = h.bytes().map(|c| c >> 5).collect();
    exp.push(0);
    exp.extend(h.bytes().map(|c| c & 31));
    exp.extend_from_slice(&data);
    let check = polymod(&exp);
    let version = data[0];
    let want = if version == 0 { 1 } else { 0x2bc8_30a3 };
    if check != want {
        return Err("bad checksum (or the wrong checksum for its witness version)".into());
    }
    // 5-bit groups to bytes, no padding allowed beyond 4 zero bits
    let mut prog = Vec::new();
    let (mut acc, mut bits) = (0u32, 0u32);
    for v in &data[1..data.len() - 6] {
        acc = (acc << 5) | *v as u32;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            prog.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    if bits >= 5 || acc != 0 {
        return Err("bad padding".into());
    }
    if version > 16 || prog.len() < 2 || prog.len() > 40 || (version == 0 && prog.len() != 20 && prog.len() != 32) {
        return Err("malformed witness program".into());
    }
    let op = if version == 0 { 0 } else { 0x50 + version };
    Ok([vec![op, prog.len() as u8], prog].concat())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap() -> Snapshot {
        Snapshot {
            now: 1_000_000,
            listing: Listing { is_rune: true, paused: false, autonomous: true, divisibility: 2, vault: [1; 20],
                reference: "840000:3".into(),
                btc_vault: "bc1p5cyxnuxmeuwuvkwfem96lqzszd02n6xdcjrs20cac6yqjjwudpxqkedrcr".into() },
            attributed: 300_000_000_000_000_000,
            cap_wei: 1_000_000_000_000_000, // 0.001 ETH per whole unit
            cap_expiry: 1_000_000 + 3_600,
            max_in_flight: 1_000_000_000_000_000_000,
            in_flight: 0,
            open: 0,
            blocked: false,
        }
    }

    #[test]
    fn it_opens_the_most_the_vault_can_fund_within_the_limits_and_halves_from_there() {
        let s = snap();
        assert_eq!(candidates(&s, 1_000_000_000_000_000).unwrap(), vec![300_000_000_000_000_000, 150_000_000_000_000_000,
            75_000_000_000_000_000], "0.3 ETH, halved while at least 0.05 ETH");
        let mut big = snap();
        big.attributed = 10_000_000_000_000_000_000;
        assert_eq!(candidates(&big, 0).unwrap()[0], MAX_PURCHASE_WEI, "never more than the per-purchase ceiling");
        let mut tight = snap();
        tight.in_flight = 900_000_000_000_000_000;
        assert_eq!(candidates(&tight, 0).unwrap()[0], 100_000_000_000_000_000, "never past maxInFlight's headroom");
    }

    #[test]
    fn it_waits_rather_than_open_anything_the_vault_would_refuse() {
        let cases: Vec<(fn(&mut Snapshot), &str)> = vec![
            (|s| s.listing.is_rune = false, "not a rune"),
            (|s| s.listing.paused = true, "paused"),
            (|s| s.blocked = true, "blocked"),
            (|s| s.open = 1, "open purchase"),
            (|s| s.cap_expiry = s.now + 600, "margin"),
            (|s| s.cap_wei = 0, "no cap"),
            (|s| s.attributed = 40_000_000_000_000_000, "under the"),
        ];
        for (f, why) in cases {
            let mut s = snap();
            f(&mut s);
            let e = candidates(&s, 0).unwrap_err();
            assert!(e.contains(why), "{why}: {e}");
        }
        // fixed costs: 0.02 ETH is over 5% of 0.3 ETH, so nothing is worth opening
        assert!(candidates(&snap(), 20_000_000_000_000_000).unwrap_err().contains("costs"));
        // …while 0.01 ETH allows 0.3 but not 0.15 (6.7%)
        assert_eq!(candidates(&snap(), 10_000_000_000_000_000).unwrap(), vec![300_000_000_000_000_000]);
    }

    #[test]
    fn the_first_amount_the_vault_can_charge_within_64_payers_is_taken() {
        let c = [300u128, 150, 75];
        assert_eq!(first_fundable(&c, &[0, 70, 12]), Some(75), "0: cannot fund; 70: too many payers; 12: yes");
        assert_eq!(first_fundable(&c, &[3, 1, 1]), Some(300));
        assert_eq!(first_fundable(&c, &[0, 0, 65]), None);
    }

    #[test]
    fn the_order_promises_no_more_units_than_the_cap_buys_and_satisfies_the_vaults_bound() {
        let s = snap();
        let o = order_for(&s, 300_000_000_000_000_000, "bc").unwrap();
        // 0.3 ETH at 0.001 ETH a whole unit (2 decimals) = 300 whole = 30,000 smallest units
        assert_eq!(o.units, 30_000);
        assert_eq!(s.cap_wei * o.units / 100, o.amount_wei, "boundFor(units) is exactly the amount");
        assert_eq!(o.rune, "840000:3");
        assert_eq!(o.cap_ref, "cap:1000000000000000@1003600");
        assert_eq!(hex::encode(&o.recipient_script), "5120a60869f0dbcf1dc659c9cecbaf8050135ea9e8cdc487053f1dc6880949dc684c");
        // an amount that does not divide evenly rounds the units UP, so the bound still covers it
        let odd = order_for(&s, 300_000_000_000_000_001, "bc").unwrap();
        assert_eq!(odd.units, 30_001);
        assert!(s.cap_wei * odd.units / 100 >= odd.amount_wei);
        // a listing whose address is for another network is refused
        assert!(order_for(&s, 300_000_000_000_000_000, "tb").unwrap_err().contains("installed network"));
    }

    #[test]
    fn segwit_addresses_decode_per_bip_173_and_350_and_the_wrong_checksum_variant_is_refused() {
        // BIP 350 valid vectors
        assert_eq!(hex::encode(script_of("BC1QW508D6QEJXTDG4Y5R3ZARVARY0C5XW7KV8F3T4", "bc").unwrap()),
            "0014751e76e8199196d454941c45d1b3a323f1433bd6");
        assert_eq!(hex::encode(script_of("tb1qqqqqp399et2xygdj5xreqhjjvcmzhxw4aywxecjdzew6hylgvsesrxh6hy", "tb").unwrap()),
            "0020000000c4a5cad46221b2a187905e5266362b99d5e91c6ce24d165dab93e86433");
        assert_eq!(hex::encode(script_of("bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqzk5jj0", "bc").unwrap()),
            "512079be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798");
        // BIP 350 invalid: v1 with a bech32 (not bech32m) checksum; v0 with bech32m
        assert!(script_of("bc1p38j9r5y49hruaue7wxjce0updqjuyyx0kh56v8s25huc6995vvpql3jow4", "bc").is_err());
        assert!(script_of("BC1S0XLXVLHEMJA6C4DQV22UAPCTQUPFHLXM9H8Z3K2E72Q4K9HCZ7VQ54WELL", "bc").is_err());
        assert!(script_of("bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kemeawh", "bc").is_err());
        // mixed case, and a wrong prefix
        assert!(script_of("bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqzk5jJ0", "bc").is_err());
        assert!(script_of("bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqzk5jj0", "tb").is_err());
    }

    #[test]
    fn the_listing_struct_decodes_from_what_the_registry_returns() {
        // `cast abi-encode "f((uint8,bool,bool,uint8,address,bytes32,string,string,string))"` of a rune listing
        let enc = hex::decode(concat!(
            "0000000000000000000000000000000000000000000000000000000000000020",
            "0000000000000000000000000000000000000000000000000000000000000001",
            "0000000000000000000000000000000000000000000000000000000000000000",
            "0000000000000000000000000000000000000000000000000000000000000001",
            "0000000000000000000000000000000000000000000000000000000000000002",
            "0000000000000000000000000101010101010101010101010101010101010101",
            "0000000000000000000000000000000000000000000000000000000000000000",
            "0000000000000000000000000000000000000000000000000000000000000120",
            "0000000000000000000000000000000000000000000000000000000000000160",
            "00000000000000000000000000000000000000000000000000000000000001a0",
            "0000000000000000000000000000000000000000000000000000000000000004",
            "4d4f4f4e00000000000000000000000000000000000000000000000000000000",
            "0000000000000000000000000000000000000000000000000000000000000008",
            "3834303030303a33000000000000000000000000000000000000000000000000",
            "000000000000000000000000000000000000000000000000000000000000003e",
            "62633170356379786e75786d65757775766b7766656d39366c717a737a643032",
            "6e367864636a727332306361633679716a6a7775647078716b65647263720000",
        )).unwrap();
        let l = ret_listing(&enc).unwrap();
        assert_eq!(l, snap().listing);
    }

    #[test]
    fn the_calls_are_the_selectors_cast_computes() {
        assert_eq!(hex::encode(call_uint(PAYERS_FOR, 5)), "782af7570000000000000000000000000000000000000000000000000000000000000005");
        assert_eq!(hex::encode(call0(LISTING_ID)), "9405dfdd");
        let arr = hex::decode(concat!("0000000000000000000000000000000000000000000000000000000000000020",
            "0000000000000000000000000000000000000000000000000000000000000002",
            "0000000000000000000000000000000000000000000000000000000000000003",
            "0000000000000000000000000000000000000000000000000000000000000004")).unwrap();
        assert_eq!(ret_array_len(&arr), Ok(2));
    }
    #[test]
    fn a_taproot_output_key_encodes_to_its_address_and_back() {
        let key: [u8; 32] = hex::decode("a60869f0dbcf1dc659c9cecbaf8050135ea9e8cdc487053f1dc6880949dc684c").unwrap().try_into().unwrap();
        let a = taproot_address(&key, "bc");
        assert_eq!(a, "bc1p5cyxnuxmeuwuvkwfem96lqzszd02n6xdcjrs20cac6yqjjwudpxqkedrcr", "BIP-86's first test vector");
        assert_eq!(script_of(&a, "bc").unwrap()[2..], key);
        assert!(taproot_address(&key, "tb").starts_with("tb1p"));
    }
}
