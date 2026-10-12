//! Spike Q5: a Relay order from a **Solana** origin, judged and bound the way `relayconv::judge` binds one from 4663
//! (docs/v9/spikes/q5-relay-solana-legs.md).
//!
//! The finding this crate proves on every saved quote: the Solana deposit is one instruction to Relay's depository program,
//! `deposit_native(amount: u64, id: [u8; 32])`, whose `id` is **the same EIP-712 `hashStruct(Order)`** of
//! `protocol.v2.orderData` that the EVM `depositNative(address, bytes32 id)` carries; Solana addresses enter the order as their
//! 32 raw bytes (Relay's settlement SDK `encodeAddress(.., "solana-vm")`: base58-decode, exactly 32 bytes).
//!
//! Pure: no network, no keys, no clock. Nothing here signs or sends.

use base64::Engine;
use serde_json::Value;
use sha2::{Digest, Sha256};
use sha3::Keccak256;

// ---- pinned values (read 2026-10-09 from mainnet; fixtures/chain/*) -------------------------------------------------------------

/// Relay's Solana depository program (every Solana-origin quote's `programId` and `paymentDetails.depository`).
pub const DEPOSITORY_PROGRAM: &str = "99vQwtBwYtrqqD9YSXbdum3KBdxPAVxYTaQ3cfnJSrN2";
/// Its `RelayDepository` state account: PDA `["relay_depository"]`.
pub const RELAY_DEPOSITORY: &str = "Dodg2HifwU8rmaVVyMyUZDGTRbqAJTyVYxXPwcbNpBKc";
/// Its SOL vault: PDA `["vault"]`, bump 255 (= the state's `vault_bump`).
pub const VAULT: &str = "7uTT8Xi5RWXzy7h9XL244GRgEycDYDhLjr3ZyNdXi8pZ";
/// The state's `allocator` on 2026-10-09 (the only key whose ed25519 signature releases vault funds).
pub const ALLOCATOR: &str = "HuAjVESMnrR2wuthKHeAsJeDsfN2jKUEi7TCcZC7eW6Z";
/// The state's `owner`, and the program's **upgrade authority** (the program is upgradeable: BPFLoaderUpgradeable).
pub const OWNER_AND_UPGRADE_AUTHORITY: &str = "6Bz8AWWipCDf7XCSqNuRXrEtKZM5M4Y56uP9ZRjSaMWD";
/// The programdata account and the slot of the deployment read on 2026-10-09.
pub const PROGRAMDATA: &str = "6y7C7Lfh1WRRbKohE2FQmFBD2asw3yMi17kStwEcAWWF";
pub const PROGRAM_DEPLOY_SLOT: u64 = 386_211_280;
/// The state's `domain_separator` (every `execute_transfer` request carries it).
pub const DOMAIN_SEPARATOR: &str = "01c7a22107be8f4973259032638387b984782556d68bd9b8e3fa23c48f473eb4";
/// The one lookup table every quote names. The canister does not need it (5 static keys fit) and never uses one it did not pin.
pub const QUOTE_LUT: &str = "Hm9fUgcn7qwDaiNTFiGh6pNtVATgnaRcmK6Bbx6EMZfP";
/// Relay's Solana solver hot wallet seen on 2026-10-09: it signs fills, refunds and the depository withdrawals.
pub const SOLVER_2026_10: &str = "F7p3dFrjRTbtRp8FRF6qHLomXbKRBzpvBLjtQcfcgmNe";

pub const SYSTEM_PROGRAM: &str = "11111111111111111111111111111111";
pub const SOL_NATIVE: &str = SYSTEM_PROGRAM;
pub const MEMO_V2: &str = "MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr";
pub const MEMO_V1: &str = "Memo1UhkJRfHyvLMcVucJwxXeuD728EqVDDwQDxFMNo";
pub const RELAY_SOLANA: u64 = 792_703_809;
pub const BTC_NATIVE: &str = "bc1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqmql8k8";
pub const EVM_NATIVE: &str = "0x0000000000000000000000000000000000000000";

/// Anchor discriminators: `sha256("global:<name>")[..8]` / `sha256("event:<Name>")[..8]` / `sha256("account:<Name>")[..8]`.
pub const IX_DEPOSIT_NATIVE: [u8; 8] = [0x0d, 0x9e, 0x0d, 0xdf, 0x5f, 0xd5, 0x1c, 0x06];
pub const IX_DEPOSIT_TOKEN: [u8; 8] = [0x0b, 0x9c, 0x60, 0xda, 0x27, 0xa3, 0xb4, 0x13];
pub const IX_EXECUTE_TRANSFER: [u8; 8] = [0xe9, 0x7e, 0xa0, 0xb8, 0xeb, 0xce, 0x1f, 0x77];
pub const EV_DEPOSIT: [u8; 8] = [0x78, 0xf8, 0x3d, 0x53, 0x1f, 0x8e, 0x6b, 0x90];
pub const EV_TRANSFER_EXECUTED: [u8; 8] = [0x5c, 0x0a, 0xb2, 0xb8, 0x12, 0x2c, 0x78, 0x7c];
pub const ACC_RELAY_DEPOSITORY: [u8; 8] = [0x80, 0x35, 0x9c, 0xbd, 0xf3, 0x53, 0xf5, 0x6b];

pub fn anchor_disc(namespace: &str, name: &str) -> [u8; 8] {
    Sha256::digest(format!("{namespace}:{name}").as_bytes())[..8].try_into().expect("8")
}

// ---- base58 ------------------------------------------------------------------------------------------------------------------

const B58: &[u8; 58] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

pub fn b58_decode(s: &str) -> Result<Vec<u8>, String> {
    let mut out: Vec<u8> = Vec::new(); // little-endian base-256
    for c in s.bytes() {
        let mut carry = B58.iter().position(|x| *x == c).ok_or_else(|| format!("{s:?} is not base58"))? as u32;
        for b in out.iter_mut() {
            carry += (*b as u32) * 58;
            *b = carry as u8;
            carry >>= 8;
        }
        while carry > 0 {
            out.push(carry as u8);
            carry >>= 8;
        }
    }
    let zeros = s.bytes().take_while(|c| *c == b'1').count();
    out.extend(std::iter::repeat(0).take(zeros));
    out.reverse();
    Ok(out)
}

pub fn b58_encode(b: &[u8]) -> String {
    let mut digits: Vec<u8> = Vec::new(); // little-endian base-58
    for byte in b {
        let mut carry = *byte as u32;
        for d in digits.iter_mut() {
            carry += (*d as u32) << 8;
            *d = (carry % 58) as u8;
            carry /= 58;
        }
        while carry > 0 {
            digits.push((carry % 58) as u8);
            carry /= 58;
        }
    }
    let zeros = b.iter().take_while(|x| **x == 0).count();
    let mut s = "1".repeat(zeros);
    s.extend(digits.iter().rev().map(|d| B58[*d as usize] as char));
    s
}

/// A Solana address: base58 of exactly 32 bytes.
pub fn pubkey(s: &str) -> Result<[u8; 32], String> {
    b58_decode(s)?.try_into().map_err(|v: Vec<u8>| format!("{s:?} decodes to {} bytes, not 32", v.len()))
}

// ---- PDAs ----------------------------------------------------------------------------------------------------------------------

/// `create_program_address`: `sha256(seeds ‖ program ‖ "ProgramDerivedAddress")`, refused when the point is on the curve.
pub fn create_program_address(seeds: &[&[u8]], program: &[u8; 32]) -> Option<[u8; 32]> {
    let mut h = Sha256::new();
    for s in seeds {
        h.update(s);
    }
    h.update(program);
    h.update(b"ProgramDerivedAddress");
    let k: [u8; 32] = h.finalize().into();
    curve25519_dalek::edwards::CompressedEdwardsY(k).decompress().is_none().then_some(k)
}

/// `find_program_address`: the first off-curve bump from 255 down.
pub fn find_program_address(seeds: &[&[u8]], program: &[u8; 32]) -> Option<([u8; 32], u8)> {
    (0..=255u8).rev().find_map(|bump| {
        let mut s = seeds.to_vec();
        let b = [bump];
        s.push(&b);
        create_program_address(&s, program).map(|k| (k, bump))
    })
}

// ---- the order hash (Relay's settlement SDK `getOrderId`), with `solana-vm` addresses ------------------------------------------

const T_ORDER: &str = "Order(string version,string solverChainId,address solver,uint256 salt,Input[] inputs,Output output,Fee[] fees)";
const T_FEE: &str = "Fee(string recipientChainId,bytes recipient,string currencyChainId,bytes currency,uint256 amount)";
const T_INPUT: &str = "Input(InputPayment payment,InputRefund[] refunds)";
const T_INPUT_PAYMENT: &str = "InputPayment(string chainId,bytes currency,uint256 amount,uint256 weight)";
const T_INPUT_REFUND: &str = "InputRefund(string chainId,bytes recipient,bytes currency,uint256 minimumAmount,uint32 deadline,bytes extraData)";
const T_OUTPUT: &str = "Output(string chainId,OutputPayment[] payments,uint32 deadline,bytes[] calls,bytes extraData)";
const T_OUTPUT_PAYMENT: &str = "OutputPayment(bytes recipient,bytes currency,uint256 minimumAmount,uint256 expectedAmount)";

pub fn keccak(b: &[u8]) -> [u8; 32] {
    Keccak256::digest(b).into()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Vm {
    Evm,
    Bitcoin,
    Solana,
}

/// Relay's protocol chain names this judge encodes; nothing guessed.
pub fn vm_of(chain: &str) -> Result<Vm, String> {
    match chain {
        "robinhood" | "base" | "ethereum" | "arbitrum" | "optimism" => Ok(Vm::Evm),
        "bitcoin" => Ok(Vm::Bitcoin),
        "solana" => Ok(Vm::Solana),
        c => Err(format!("the order names chain {c:?}, which this judge does not encode")),
    }
}

/// Relay's `encodeAddress`: EVM 20 bytes; Bitcoin witness version ‖ program (bech32/bech32m, no program-length rule); Solana the
/// 32 raw bytes of the base58 address.
pub fn encode_address(addr: &str, chain: &str) -> Result<Vec<u8>, String> {
    match vm_of(chain)? {
        Vm::Evm => evm_address(addr).map(|a| a.to_vec()),
        Vm::Bitcoin => bech32_words(addr),
        Vm::Solana => pubkey(addr).map(|a| a.to_vec()),
    }
}

pub fn evm_address(a: &str) -> Result<[u8; 20], String> {
    let h = a.strip_prefix("0x").ok_or_else(|| format!("{a:?} is not 0x-hex"))?;
    hex::decode(h).map_err(|_| format!("{a:?} is not hex"))?.try_into().map_err(|_| format!("{a:?} is not 20 bytes"))
}

const CHARSET: &[u8; 32] = b"qpzry9x8gf2tvdw0s3jn54khce6mua7l";

fn polymod(values: &[u8]) -> u32 {
    const GEN: [u32; 5] = [0x3b6a_57b2, 0x2650_8e6d, 0x1ea1_19fa, 0x3d42_33dd, 0x2a14_62b3];
    let mut chk: u32 = 1;
    for v in values {
        let b = chk >> 25;
        chk = ((chk & 0x01ff_ffff) << 5) ^ *v as u32;
        for (i, g) in GEN.iter().enumerate() {
            if (b >> i) & 1 == 1 {
                chk ^= g;
            }
        }
    }
    chk
}

/// Copied from `relayconv::bech32_words` (2026-10-09), unchanged.
fn bech32_words(addr: &str) -> Result<Vec<u8>, String> {
    if addr.chars().any(|c| c.is_ascii_uppercase()) {
        return Err(format!("{addr:?}: only lower-case addresses are encoded"));
    }
    let pos = addr.rfind('1').ok_or_else(|| format!("{addr:?} is not bech32"))?;
    let (hrp, rest) = (&addr[..pos], &addr[pos + 1..]);
    if hrp != "bc" || rest.len() < 7 || addr.len() > 90 {
        return Err(format!("{addr:?} is not a Bitcoin mainnet segwit address"));
    }
    let data: Vec<u8> = rest.bytes().map(|c| CHARSET.iter().position(|x| *x == c).map(|p| p as u8)).collect::<Option<_>>()
        .ok_or_else(|| format!("{addr:?} has a non-bech32 character"))?;
    let mut exp: Vec<u8> = hrp.bytes().map(|c| c >> 5).collect();
    exp.push(0);
    exp.extend(hrp.bytes().map(|c| c & 31));
    exp.extend_from_slice(&data);
    let want = if data[0] == 0 { 1 } else { 0x2bc8_30a3 };
    if polymod(&exp) != want || data[0] > 16 {
        return Err(format!("{addr:?}: bad checksum for its witness version"));
    }
    let mut out = vec![data[0]];
    let (mut acc, mut bits) = (0u32, 0u32);
    for v in &data[1..data.len() - 6] {
        acc = (acc << 5) | *v as u32;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    if bits >= 5 || acc != 0 {
        return Err(format!("{addr:?}: bad padding"));
    }
    Ok(out)
}

pub fn get<'a>(v: &'a Value, k: &str, at: &str) -> Result<&'a Value, String> {
    v.get(k).ok_or_else(|| format!("{at} has no {k}"))
}
pub fn text<'a>(v: &'a Value, k: &str, at: &str) -> Result<&'a str, String> {
    get(v, k, at)?.as_str().ok_or_else(|| format!("{at}.{k} is not a string"))
}
pub fn list<'a>(v: &'a Value, k: &str, at: &str) -> Result<&'a Vec<Value>, String> {
    get(v, k, at)?.as_array().ok_or_else(|| format!("{at}.{k} is not a list"))
}
fn uint_word(v: &Value, k: &str, at: &str) -> Result<[u8; 32], String> {
    let x = get(v, k, at)?;
    let s = x.as_str().map(|s| s.to_string()).or_else(|| x.as_u64().map(|n| n.to_string()))
        .ok_or_else(|| format!("{at}.{k} is not a number"))?;
    let mut w = [0u8; 32];
    if let Some(h) = s.strip_prefix("0x") {
        if h.is_empty() || h.len() > 64 {
            return Err(format!("{at}.{k} is not a 256-bit hex number"));
        }
        w.copy_from_slice(&hex::decode(format!("{h:0>64}")).map_err(|_| format!("{at}.{k} is not hex"))?);
    } else {
        let n: u128 = s.parse().map_err(|_| format!("{at}.{k} = {s:?} is not a whole number (or over 128 bits)"))?;
        w[16..].copy_from_slice(&n.to_be_bytes());
    }
    Ok(w)
}
pub fn uint_of(v: &Value, k: &str, at: &str) -> Result<u128, String> {
    let w = uint_word(v, k, at)?;
    if w[..16].iter().any(|b| *b != 0) {
        return Err(format!("{at}.{k} is over 128 bits"));
    }
    Ok(u128::from_be_bytes(w[16..].try_into().expect("16")))
}
fn bytes_of(v: &Value, k: &str, at: &str) -> Result<Vec<u8>, String> {
    let s = text(v, k, at)?;
    hex::decode(s.strip_prefix("0x").ok_or_else(|| format!("{at}.{k} is not 0x-hex"))?).map_err(|_| format!("{at}.{k} is not hex"))
}
fn struct_hash(th: [u8; 32], fields: &[[u8; 32]]) -> [u8; 32] {
    let mut enc = th.to_vec();
    for f in fields {
        enc.extend_from_slice(f);
    }
    keccak(&enc)
}
fn array_hash(items: &[[u8; 32]]) -> [u8; 32] {
    keccak(&items.concat())
}
fn h_str(s: &str) -> [u8; 32] {
    keccak(s.as_bytes())
}

/// `hashStruct(Order)` of `protocol.v2.orderData`: Relay's order id. `relayconv::order_id` with `solana` added to `vm_of`.
pub fn order_id(o: &Value) -> Result<[u8; 32], String> {
    let at = "orderData";
    let th = |parts: &[&str]| keccak(parts.concat().as_bytes());
    let t_input = th(&[T_INPUT, T_INPUT_PAYMENT, T_INPUT_REFUND]);
    let t_payment = th(&[T_INPUT_PAYMENT]);
    let t_refund = th(&[T_INPUT_REFUND]);
    let t_output = th(&[T_OUTPUT, T_OUTPUT_PAYMENT]);
    let t_out_payment = th(&[T_OUTPUT_PAYMENT]);
    let t_fee = th(&[T_FEE]);
    let t_order = th(&[T_ORDER, T_FEE, T_INPUT, T_INPUT_PAYMENT, T_INPUT_REFUND, T_OUTPUT, T_OUTPUT_PAYMENT]);
    let mut inputs = Vec::new();
    for (i, input) in list(o, "inputs", at)?.iter().enumerate() {
        let at = format!("orderData.inputs[{i}]");
        let p = get(input, "payment", &at)?;
        let pat = format!("{at}.payment");
        let pc = text(p, "chainId", &pat)?;
        let payment = struct_hash(t_payment, &[h_str(pc), keccak(&encode_address(text(p, "currency", &pat)?, pc)?),
            uint_word(p, "amount", &pat)?, uint_word(p, "weight", &pat)?]);
        let mut refunds = Vec::new();
        for (j, r) in list(input, "refunds", &at)?.iter().enumerate() {
            let rat = format!("{at}.refunds[{j}]");
            let rc = text(r, "chainId", &rat)?;
            refunds.push(struct_hash(t_refund, &[h_str(rc), keccak(&encode_address(text(r, "recipient", &rat)?, rc)?),
                keccak(&encode_address(text(r, "currency", &rat)?, rc)?), uint_word(r, "minimumAmount", &rat)?,
                uint_word(r, "deadline", &rat)?, keccak(&bytes_of(r, "extraData", &rat)?)]));
        }
        inputs.push(struct_hash(t_input, &[payment, array_hash(&refunds)]));
    }
    let out = get(o, "output", at)?;
    let oat = "orderData.output";
    let oc = text(out, "chainId", oat)?;
    let mut payments = Vec::new();
    for (i, p) in list(out, "payments", oat)?.iter().enumerate() {
        let pat = format!("{oat}.payments[{i}]");
        payments.push(struct_hash(t_out_payment, &[keccak(&encode_address(text(p, "recipient", &pat)?, oc)?),
            keccak(&encode_address(text(p, "currency", &pat)?, oc)?), uint_word(p, "minimumAmount", &pat)?,
            uint_word(p, "expectedAmount", &pat)?]));
    }
    let mut calls = Vec::new();
    for (i, c) in list(out, "calls", oat)?.iter().enumerate() {
        let s = c.as_str().ok_or_else(|| format!("{oat}.calls[{i}] is not a string"))?;
        calls.push(keccak(&hex::decode(s.trim_start_matches("0x")).map_err(|_| format!("{oat}.calls[{i}] is not hex"))?));
    }
    let output = struct_hash(t_output, &[h_str(oc), array_hash(&payments), uint_word(out, "deadline", oat)?, array_hash(&calls),
        keccak(&bytes_of(out, "extraData", oat)?)]);
    let mut fees = Vec::new();
    for (i, f) in list(o, "fees", at)?.iter().enumerate() {
        let fat = format!("orderData.fees[{i}]");
        let (rc, cc) = (text(f, "recipientChainId", &fat)?, text(f, "currencyChainId", &fat)?);
        fees.push(struct_hash(t_fee, &[h_str(rc), keccak(&encode_address(text(f, "recipient", &fat)?, rc)?), h_str(cc),
            keccak(&encode_address(text(f, "currency", &fat)?, cc)?), uint_word(f, "amount", &fat)?]));
    }
    let mut solver = [0u8; 32];
    solver[12..].copy_from_slice(&evm_address(text(o, "solver", at)?)?);
    Ok(struct_hash(t_order, &[h_str(text(o, "version", at)?), h_str(text(o, "solverChainId", at)?), solver, uint_word(o, "salt", at)?,
        array_hash(&inputs), output, array_hash(&fees)]))
}

// ---- the deposit instruction -----------------------------------------------------------------------------------------------------

/// One account meta as the quote (and the canister's message) lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Meta {
    pub pubkey: [u8; 32],
    pub signer: bool,
    pub writable: bool,
}

/// `deposit_native`'s instruction data, exactly 48 bytes: `disc[8] ‖ amount u64 LE ‖ id[32]`. `None` for anything else.
pub fn decode_deposit_native(data: &[u8]) -> Option<(u64, [u8; 32])> {
    if data.len() != 48 || data[..8] != IX_DEPOSIT_NATIVE {
        return None;
    }
    Some((u64::from_le_bytes(data[8..16].try_into().ok()?), data[16..48].try_into().ok()?))
}

/// The inverse (what the canister writes into its own message).
pub fn encode_deposit_native(amount: u64, id: &[u8; 32]) -> Vec<u8> {
    [IX_DEPOSIT_NATIVE.as_slice(), &amount.to_le_bytes(), id].concat()
}

/// The five account metas of `deposit_native` for `ours` as sender and depositor, in the program's order
/// (`relay_depository, sender, depositor, vault, system_program`).
pub fn deposit_native_metas(ours: &[u8; 32]) -> Vec<Meta> {
    let k = |s: &str| pubkey(s).expect("pinned");
    vec![
        Meta { pubkey: k(RELAY_DEPOSITORY), signer: false, writable: false },
        Meta { pubkey: *ours, signer: true, writable: true },
        Meta { pubkey: *ours, signer: false, writable: false },
        Meta { pubkey: k(VAULT), signer: false, writable: true },
        Meta { pubkey: k(SYSTEM_PROGRAM), signer: false, writable: false },
    ]
}

/// The quote's instruction: program, metas, data.
pub fn quote_instruction(ix: &Value, at: &str) -> Result<([u8; 32], Vec<Meta>, Vec<u8>), String> {
    let program = pubkey(text(ix, "programId", at)?)?;
    let mut metas = Vec::new();
    for (i, k) in list(ix, "keys", at)?.iter().enumerate() {
        let kat = format!("{at}.keys[{i}]");
        let flag = |f: &str| get(k, f, &kat).and_then(|x| x.as_bool().ok_or_else(|| format!("{kat}.{f} is not a bool")));
        metas.push(Meta { pubkey: pubkey(text(k, "pubkey", &kat)?)?, signer: flag("isSigner")?, writable: flag("isWritable")? });
    }
    let d = text(ix, "data", at)?;
    let data = hex::decode(d).map_err(|_| format!("{at}.data is not hex"))?;
    Ok((program, metas, data))
}

// ---- the judge (the Solana rules S1..S12; docs §4) -------------------------------------------------------------------------------

/// What a Solana-origin quote is judged against: the canister's own values and reads.
pub struct SolWant<'a> {
    /// The canister's Solana address (threshold Ed25519): sender, depositor and the Solana refund recipient.
    pub our_sol: &'a str,
    /// The destination as the order names it (`"bitcoin"`, `"ethereum"`), our address there and the native currency id.
    pub dest_chain: &'a str,
    pub dest_recipient: &'a str,
    pub dest_currency: &'a str,
    /// The lamports asked.
    pub lamports: u64,
    /// The least the order must guarantee, from the canister's own rate less its fee bound (rule S10).
    pub min_out_floor: u128,
    /// The canister's reads of the depository state and the program account (rule S3).
    pub allocator_read: &'a str,
    pub upgrade_authority_read: Option<&'a str>,
    pub deploy_slot_read: u64,
    pub now: u64,
    pub handed_in_at: u64,
    pub used: &'a [[u8; 32]],
}

/// The deposit the canister would write into its own message, from a quote that passed every rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SolDeposit {
    pub program: [u8; 32],
    pub metas: Vec<Meta>,
    pub data: Vec<u8>,
    pub order_id: [u8; 32],
    pub lamports: u64,
    pub min_out: u128,
    pub deadline: u64,
}

pub const QUOTE_TTL_SECS: u64 = 300;
pub const MIN_DEADLINE_SECS: u64 = 86_400;
pub const MAX_DEADLINE_SECS: u64 = 8 * 86_400;
pub const MAX_QUOTE_BYTES: usize = 64 * 1024;

fn in_band(t: u64, now: u64) -> bool {
    t >= now.saturating_add(MIN_DEADLINE_SECS) && t <= now.saturating_add(MAX_DEADLINE_SECS)
}
fn deadline_of(v: &Value, k: &str, at: &str) -> Result<u64, String> {
    let d = uint_of(v, k, at)?;
    u32::try_from(d).map(u64::from).map_err(|_| format!("rule S11: {at}.{k} = {d} is over a uint32"))
}
/// Same address on `chain`, compared as Relay encodes it (case, checksum and base58 spelling cannot differ in meaning).
pub fn same(a: &str, b: &str, chain: &str) -> bool {
    matches!((encode_address(a, chain), encode_address(b, chain)), (Ok(x), Ok(y)) if x == y)
}

/// Judge a Solana-origin quote. Every refusal names the rule and the field.
pub fn judge_sol(body: &[u8], w: &SolWant) -> Result<SolDeposit, String> {
    if body.len() > MAX_QUOTE_BYTES {
        return Err(format!("the quote is {} bytes, over {MAX_QUOTE_BYTES}", body.len()));
    }
    // S11 (first): a stale hand-in
    if w.now.saturating_sub(w.handed_in_at) > QUOTE_TTL_SECS {
        return Err("rule S11: stale hand-in".into());
    }
    let q: Value = serde_json::from_slice(body).map_err(|e| format!("not JSON: {e}"))?;
    let ours = pubkey(w.our_sol).map_err(|e| format!("our address: {e}"))?;
    // S1: one deposit step, one item, one instruction; the lookup tables are never used
    let steps = list(&q, "steps", "the quote")?;
    if steps.len() != 1 {
        return Err(format!("rule S1: {} steps; only a single deposit step is signed", steps.len()));
    }
    let step = &steps[0];
    if text(step, "id", "steps[0]")? != "deposit" || text(step, "kind", "steps[0]")? != "transaction" {
        return Err("rule S1: the one step is not a deposit transaction".into());
    }
    let items = list(step, "items", "steps[0]")?;
    if items.len() != 1 {
        return Err(format!("rule S1: {} items; exactly one transaction is signed", items.len()));
    }
    let d = get(&items[0], "data", "steps[0].items[0]")?;
    let ixs = list(d, "instructions", "the deposit").map_err(|e| format!("rule S1/S2: not a Solana deposit: {e}"))?;
    if ixs.len() != 1 {
        return Err(format!("rule S1: {} instructions; only the one deposit_native is signed", ixs.len()));
    }
    let (program, metas, data) = quote_instruction(&ixs[0], "instructions[0]").map_err(|e| format!("rule S1: {e}"))?;
    // S3: the pinned program, its state and vault PDAs, the allocator and the code we accepted
    if program != pubkey(DEPOSITORY_PROGRAM)? {
        return Err(format!("rule S3: the instruction goes to {}, not Relay's pinned depository program", b58_encode(&program)));
    }
    if w.allocator_read != ALLOCATOR {
        return Err(format!("rule S3: the depository's allocator is {}, not the pinned {ALLOCATOR}: review before converting", w.allocator_read));
    }
    if w.upgrade_authority_read != Some(OWNER_AND_UPGRADE_AUTHORITY) || w.deploy_slot_read != PROGRAM_DEPLOY_SLOT {
        return Err("rule S3: the depository program was upgraded (or its authority changed) since it was reviewed".into());
    }
    // S5: exactly the program's accounts for us as sender and depositor, flags included
    if metas != deposit_native_metas(&ours) {
        return Err("rule S5: the accounts are not (relay_depository, us signer, us depositor, vault, system program)".into());
    }
    // S5: the data
    let (amount, id) = decode_deposit_native(&data).ok_or("rule S5: the data is not deposit_native(u64, [u8; 32])")?;
    // S4: the amount
    if amount != w.lamports {
        return Err(format!("rule S4: the deposit sends {amount} lamports, not the {} asked", w.lamports));
    }
    // S6: the order is the order we inspect
    let v2 = get(get(&q, "protocol", "the quote")?, "v2", "protocol")?;
    let o = get(v2, "orderData", "protocol.v2")?;
    let computed = order_id(o).map_err(|e| format!("rule S6: {e}"))?;
    if computed != id {
        return Err(format!("rule S6: the instruction commits to 0x{}, but the order hashes to 0x{}", hex::encode(id), hex::encode(computed)));
    }
    if bytes_of(v2, "orderId", "protocol.v2")? != id {
        return Err("rule S6: protocol.v2.orderId is not the instruction's id".into());
    }
    let pd = get(v2, "paymentDetails", "protocol.v2")?;
    // S2: the chain
    if text(pd, "chainId", "paymentDetails")? != "solana" || text(pd, "depository", "paymentDetails")? != DEPOSITORY_PROGRAM
        || text(pd, "currency", "paymentDetails")? != SOL_NATIVE || uint_of(pd, "amount", "paymentDetails")? != amount as u128 {
        return Err("rule S2/S3/S4: paymentDetails names another chain, program, currency or amount".into());
    }
    // S7: the input
    let inputs = list(o, "inputs", "orderData")?;
    if inputs.len() != 1 {
        return Err(format!("rule S7: {} inputs, not one", inputs.len()));
    }
    let p = get(&inputs[0], "payment", "orderData.inputs[0]")?;
    if text(p, "chainId", "payment")? != "solana" || text(p, "currency", "payment")? != SOL_NATIVE
        || uint_of(p, "amount", "payment")? != amount as u128 {
        return Err("rule S7: the order's payment is not native SOL on solana for the deposit's amount".into());
    }
    // S8: every refund is ours
    let refunds = list(&inputs[0], "refunds", "orderData.inputs[0]")?;
    let mut origin_refund = false;
    for (i, r) in refunds.iter().enumerate() {
        let rat = format!("refunds[{i}]");
        let (chain, to, cur) = (text(r, "chainId", &rat)?, text(r, "recipient", &rat)?, text(r, "currency", &rat)?);
        let ok = match chain {
            "solana" => same(to, w.our_sol, "solana") && cur == SOL_NATIVE,
            c if c == w.dest_chain => same(to, w.dest_recipient, c) && same(cur, w.dest_currency, c),
            _ => false,
        };
        if !ok {
            return Err(format!("rule S8: {rat} pays {to} on {chain}, not our own address in its native currency"));
        }
        origin_refund |= chain == "solana";
        let dl = deadline_of(r, "deadline", &rat)?;
        if !in_band(dl, w.now) {
            return Err(format!("rule S11: {rat}'s deadline {dl} is outside [now + 1 day, now + 8 days]"));
        }
    }
    if !origin_refund {
        return Err("rule S8: no refund on solana to our address".into());
    }
    // S9: the output
    let out = get(o, "output", "orderData")?;
    if text(out, "chainId", "output")? != w.dest_chain {
        return Err("rule S9: the order pays out on another chain".into());
    }
    let pays = list(out, "payments", "output")?;
    if pays.len() != 1 {
        return Err(format!("rule S9: {} payments, not one", pays.len()));
    }
    let pay = &pays[0];
    if !same(text(pay, "recipient", "payment")?, w.dest_recipient, w.dest_chain) {
        return Err(format!("rule S9: the order pays {}, not {}", text(pay, "recipient", "payment")?, w.dest_recipient));
    }
    if !same(text(pay, "currency", "payment")?, w.dest_currency, w.dest_chain) {
        return Err("rule S9: the order does not pay the destination's native currency".into());
    }
    if !list(out, "calls", "output")?.is_empty() {
        return Err("rule S9: the order carries calls".into());
    }
    if !list(o, "fees", "orderData")?.is_empty() {
        return Err("rule S9: the order carries fees".into());
    }
    // S10: the minimum out against our own rate
    let min = uint_of(pay, "minimumAmount", "payment")?;
    if min < w.min_out_floor {
        return Err(format!("rule S10: the order guarantees {min}, under our floor of {}", w.min_out_floor));
    }
    // S11: the deadline band
    let deadline = deadline_of(out, "deadline", "output")?;
    if !in_band(deadline, w.now) {
        return Err(format!("rule S11: the order's deadline {deadline} is outside [now + 1 day, now + 8 days]"));
    }
    // S12: replay
    if w.used.contains(&id) {
        return Err(format!("rule S12: order 0x{} was accepted before: never twice", hex::encode(id)));
    }
    Ok(SolDeposit { program, metas, data, order_id: id, lamports: amount, min_out: min, deadline })
}

// ---- reading finalized transactions (getTransaction, encoding "json") -----------------------------------------------------------

/// All account keys of a v0 transaction: static, then the loaded writable, then the loaded read-only.
pub fn tx_keys(tx: &Value) -> Vec<String> {
    let mut keys: Vec<String> = tx["transaction"]["message"]["accountKeys"].as_array().into_iter().flatten()
        .filter_map(|k| k.as_str().map(String::from)).collect();
    for part in ["writable", "readonly"] {
        keys.extend(tx["meta"]["loadedAddresses"][part].as_array().into_iter().flatten().filter_map(|k| k.as_str().map(String::from)));
    }
    keys
}

/// One instruction, top-level or inner, resolved.
#[derive(Clone, Debug)]
pub struct Ix {
    pub program: String,
    pub accounts: Vec<String>,
    pub data: Vec<u8>,
    /// `None` for a top-level instruction; `Some(i)` for one invoked by top-level instruction `i`.
    pub parent: Option<usize>,
}

pub fn tx_instructions(tx: &Value) -> Result<Vec<Ix>, String> {
    let keys = tx_keys(tx);
    let res = |ix: &Value, parent: Option<usize>| -> Result<Ix, String> {
        let k = |i: &Value| i.as_u64().and_then(|i| keys.get(i as usize).cloned()).ok_or("bad key index".to_string());
        Ok(Ix { program: k(&ix["programIdIndex"])?,
            accounts: ix["accounts"].as_array().ok_or("no accounts")?.iter().map(k).collect::<Result<_, _>>()?,
            data: b58_decode(ix["data"].as_str().ok_or("no data")?)?, parent })
    };
    let mut out = Vec::new();
    for ix in tx["transaction"]["message"]["instructions"].as_array().ok_or("no instructions")? {
        out.push(res(ix, None)?);
    }
    for group in tx["meta"]["innerInstructions"].as_array().into_iter().flatten() {
        let idx = group["index"].as_u64().ok_or("no index")? as usize;
        for ix in group["instructions"].as_array().ok_or("no inner instructions")? {
            out.push(res(ix, Some(idx))?);
        }
    }
    Ok(out)
}

/// `Program data:` payloads attributed to the program that logged them (the invoke stack), so a CPI'd program cannot forge an
/// event of another.
pub fn program_data(tx: &Value, program: &str) -> Vec<Vec<u8>> {
    let mut stack: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for l in tx["meta"]["logMessages"].as_array().into_iter().flatten().filter_map(|l| l.as_str()) {
        if let Some(rest) = l.strip_prefix("Program data: ") {
            if stack.last().map(String::as_str) == Some(program) {
                let mut b = Vec::new();
                for chunk in rest.split(' ') {
                    if let Ok(x) = base64::engine::general_purpose::STANDARD.decode(chunk) {
                        b.extend(x);
                    }
                }
                out.push(b);
            }
        } else if let Some(rest) = l.strip_prefix("Program ") {
            let mut it = rest.split(' ');
            let (p, verb) = (it.next().unwrap_or(""), it.next().unwrap_or(""));
            match verb {
                "invoke" => stack.push(p.to_string()),
                "success" | "failed:" => {
                    stack.pop();
                }
                _ => {}
            }
        }
    }
    out
}

/// `DepositEvent { depositor, token: Option<Pubkey>, amount, id }`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DepositEvent {
    pub depositor: [u8; 32],
    pub token: Option<[u8; 32]>,
    pub amount: u64,
    pub id: [u8; 32],
}

pub fn decode_deposit_event(b: &[u8]) -> Option<DepositEvent> {
    if b.get(..8)? != EV_DEPOSIT {
        return None;
    }
    let depositor: [u8; 32] = b.get(8..40)?.try_into().ok()?;
    let (token, at) = match *b.get(40)? {
        0 => (None, 41),
        1 => (Some(b.get(41..73)?.try_into().ok()?), 73),
        _ => return None,
    };
    let amount = u64::from_le_bytes(b.get(at..at + 8)?.try_into().ok()?);
    let id: [u8; 32] = b.get(at + 8..at + 40)?.try_into().ok()?;
    (b.len() == at + 40).then_some(DepositEvent { depositor, token, amount, id })
}

/// `TransferRequest { domain, recipient, token: Option<Pubkey>, amount, nonce, expiration, vault_address }` (borsh).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransferRequest {
    pub domain: [u8; 32],
    pub recipient: [u8; 32],
    pub token: Option<[u8; 32]>,
    pub amount: u64,
    pub nonce: u64,
    pub expiration: i64,
    pub vault_address: [u8; 32],
}

/// The request and the bytes it occupied.
pub fn decode_transfer_request(b: &[u8]) -> Option<(TransferRequest, usize)> {
    let domain = b.get(..32)?.try_into().ok()?;
    let recipient = b.get(32..64)?.try_into().ok()?;
    let (token, at) = match *b.get(64)? {
        0 => (None, 65),
        1 => (Some(b.get(65..97)?.try_into().ok()?), 97),
        _ => return None,
    };
    let n = |i: usize| -> Option<[u8; 8]> { b.get(i..i + 8)?.try_into().ok() };
    let r = TransferRequest { domain, recipient, token, amount: u64::from_le_bytes(n(at)?), nonce: u64::from_le_bytes(n(at + 8)?),
        expiration: i64::from_le_bytes(n(at + 16)?), vault_address: b.get(at + 24..at + 56)?.try_into().ok()? };
    Some((r, at + 56))
}

pub fn encode_transfer_request(r: &TransferRequest) -> Vec<u8> {
    let mut v = [r.domain, r.recipient].concat();
    match r.token {
        None => v.push(0),
        Some(t) => {
            v.push(1);
            v.extend(t);
        }
    }
    v.extend(r.amount.to_le_bytes());
    v.extend(r.nonce.to_le_bytes());
    v.extend(r.expiration.to_le_bytes());
    v.extend(r.vault_address);
    v
}

/// `execute_transfer(request)` instruction data.
pub fn decode_execute_transfer(data: &[u8]) -> Option<TransferRequest> {
    if data.get(..8)? != IX_EXECUTE_TRANSFER {
        return None;
    }
    let (r, n) = decode_transfer_request(&data[8..])?;
    (data.len() == 8 + n).then_some(r)
}

/// `TransferExecutedEvent { request, executor, id }`.
pub fn decode_transfer_executed(b: &[u8]) -> Option<(TransferRequest, [u8; 32], [u8; 32])> {
    if b.get(..8)? != EV_TRANSFER_EXECUTED {
        return None;
    }
    let (r, n) = decode_transfer_request(&b[8..])?;
    let at = 8 + n;
    (b.len() == at + 64).then(|| (r, b[at..at + 32].try_into().expect("32"), b[at + 32..at + 64].try_into().expect("32")))
}

/// The deposit a finalized transaction made into the depository: the top-level `deposit_native` instruction (program, metas,
/// data) and its `DepositEvent`, which must agree. `None` when the transaction failed or holds no such deposit.
pub fn deposit_in(tx: &Value) -> Result<Option<(String, DepositEvent)>, String> {
    if !tx["meta"]["err"].is_null() {
        return Ok(None);
    }
    let ixs = tx_instructions(tx)?;
    let Some(ix) = ixs.iter().find(|i| i.program == DEPOSITORY_PROGRAM && i.parent.is_none() && i.data.get(..8) == Some(&IX_DEPOSIT_NATIVE)) else {
        return Ok(None);
    };
    let (amount, id) = decode_deposit_native(&ix.data).ok_or("malformed deposit_native")?;
    if ix.accounts.len() != 5 || ix.accounts[0] != RELAY_DEPOSITORY || ix.accounts[3] != VAULT || ix.accounts[4] != SYSTEM_PROGRAM {
        return Err("deposit_native with foreign accounts".into());
    }
    let evs: Vec<DepositEvent> = program_data(tx, DEPOSITORY_PROGRAM).iter().filter_map(|b| decode_deposit_event(b)).collect();
    let [ev] = evs.as_slice() else { return Err(format!("{} DepositEvents", evs.len())) };
    if ev.amount != amount || ev.id != id || ev.token.is_some() || b58_encode(&ev.depositor) != ix.accounts[2] {
        return Err("the DepositEvent disagrees with the instruction".into());
    }
    Ok(Some((ix.accounts[1].clone(), ev.clone())))
}

/// The lamports `addr` gained in a finalized transaction (post − pre; negative is a loss).
pub fn lamport_delta(tx: &Value, addr: &str) -> Option<i128> {
    let i = tx_keys(tx).iter().position(|k| k == addr)?;
    let pre = tx["meta"]["preBalances"][i].as_u64()? as i128;
    let post = tx["meta"]["postBalances"][i].as_u64()? as i128;
    Some(post - pre)
}

/// System Program `Transfer { lamports }` instructions to `to`, top-level or inner: `(from, lamports)`.
pub fn system_transfers_to(tx: &Value, to: &str) -> Result<Vec<(String, u64)>, String> {
    Ok(tx_instructions(tx)?.into_iter().filter(|i| i.program == SYSTEM_PROGRAM && i.data.len() == 12 && i.data[..4] == [2, 0, 0, 0]
        && i.accounts.len() == 2 && i.accounts[1] == to)
        .map(|i| (i.accounts[0].clone(), u64::from_le_bytes(i.data[4..12].try_into().expect("8")))).collect())
}

/// The memos a transaction carries (SPL Memo v1/v2), as text.
pub fn memos(tx: &Value) -> Result<Vec<String>, String> {
    Ok(tx_instructions(tx)?.into_iter().filter(|i| i.program == MEMO_V2 || i.program == MEMO_V1)
        .filter_map(|i| String::from_utf8(i.data).ok()).collect())
}

/// A refund of order `id` to `ours`, as Relay made the ones on 2026-10-09: a finalized transaction whose System transfer pays
/// `ours` and whose memo is `0x<order id>`. Returns the lamports credited (the balance delta, which is what the canister
/// trusts; the memo binds the credit to the order so it is not mistaken for any other inflow).
pub fn refund_of(tx: &Value, ours: &str, id: &[u8; 32]) -> Result<Option<u64>, String> {
    if !tx["meta"]["err"].is_null() {
        return Ok(None);
    }
    let want = format!("0x{}", hex::encode(id));
    if !memos(tx)?.iter().any(|m| m.eq_ignore_ascii_case(&want)) {
        return Ok(None);
    }
    let paid: u64 = system_transfers_to(tx, ours)?.iter().map(|(_, l)| *l).sum();
    let delta = lamport_delta(tx, ours).unwrap_or(0);
    if paid == 0 || delta <= 0 {
        return Ok(None);
    }
    Ok(Some(paid.min(delta as u64)))
}

/// The `RelayDepository` state account (106 bytes since the domain-separator migration): `(owner, allocator, vault_bump, domain)`.
pub fn decode_relay_depository(b: &[u8]) -> Option<([u8; 32], [u8; 32], u8, Option<[u8; 32]>)> {
    if b.get(..8)? != ACC_RELAY_DEPOSITORY {
        return None;
    }
    let domain = match *b.get(73)? {
        0 => None,
        1 => Some(b.get(74..106)?.try_into().ok()?),
        _ => return None,
    };
    Some((b[8..40].try_into().ok()?, b[40..72].try_into().ok()?, b[72], domain))
}

/// A BPFLoaderUpgradeable `ProgramData` header: `(deploy slot, upgrade authority)`.
pub fn decode_programdata_head(b: &[u8]) -> Option<(u64, Option<[u8; 32]>)> {
    if b.get(..4)? != [3, 0, 0, 0] {
        return None;
    }
    let slot = u64::from_le_bytes(b.get(4..12)?.try_into().ok()?);
    let auth = match *b.get(12)? {
        0 => None,
        1 => Some(b.get(13..45)?.try_into().ok()?),
        _ => return None,
    };
    Some((slot, auth))
}
