//! Relay from a **Solana** origin (docs/v9/spikes/q5-relay-solana-legs.md, ported from `ops/rehearsal/relay-sol/src/lib.rs`): a
//! quote is judged and bound the way [`crate::relayconv::judge`] binds one from 4663. The Solana deposit is one instruction to
//! Relay's depository program, `deposit_native(amount: u64, id: [u8; 32])`, whose `id` is **the same EIP-712 `hashStruct(Order)`**
//! of `protocol.v2.orderData` that the EVM `depositNative(address, bytes32 id)` carries, with Solana addresses as their 32 raw
//! bytes ([`crate::relayconv::order_id`], `"solana"` in its `vm_of`). Recomputing it binds the instruction the canister signs to
//! the order it inspected; the canister writes its own message from [`deposit_instruction`] (the quote's lookup table is never
//! used) — "sign only what was recomputed".
//!
//! The rules S1–S12 are the Solana equivalents of `relayconv::judge`'s 1–12 (Q5 §3). New on Solana (S3): the depository program
//! is UPGRADEABLE by the key that owns its state, so every judgement reads its `ProgramData` (deploy slot and upgrade authority)
//! and stops on any change, as it stops on a rotated allocator.
//!
//! Also here: the readers of finalized transactions the conversion flow needs (our deposit landed: [`deposit_in`]; a refund:
//! [`refund_of`], a solver transfer memoed `0x<order id>`), and the decoders of the depository's state, events and
//! `execute_transfer`. Pure: no network, no keys, no clock.
use base64::Engine;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::relayconv::{bytes_of, encode_address, get, list, order_id, text, uint_of};
use crate::soltx::{self, anchor_disc, b58_decode, b58_encode, meta, AccountMeta, Instruction, Pubkey};

// ---- pinned values (read 2026-10-09 from mainnet; tests/fixtures/relay-sol/chain/*) --------------------------------------------

/// Relay's Solana depository program (every Solana-origin quote's `programId` and `paymentDetails.depository`).
pub const DEPOSITORY_PROGRAM: &str = "99vQwtBwYtrqqD9YSXbdum3KBdxPAVxYTaQ3cfnJSrN2";
/// Its `RelayDepository` state account: PDA `["relay_depository"]`.
pub const RELAY_DEPOSITORY: &str = "Dodg2HifwU8rmaVVyMyUZDGTRbqAJTyVYxXPwcbNpBKc";
/// Its SOL vault: PDA `["vault"]`, bump 255 (= the state's `vault_bump`).
pub const VAULT: &str = "7uTT8Xi5RWXzy7h9XL244GRgEycDYDhLjr3ZyNdXi8pZ";
/// The state's `allocator` on 2026-10-09 (the only key whose ed25519 signature releases vault funds).
pub const ALLOCATOR: &str = "HuAjVESMnrR2wuthKHeAsJeDsfN2jKUEi7TCcZC7eW6Z";
/// The state's `owner`, and the program's **upgrade authority** (BPFLoaderUpgradeable).
pub const OWNER_AND_UPGRADE_AUTHORITY: &str = "6Bz8AWWipCDf7XCSqNuRXrEtKZM5M4Y56uP9ZRjSaMWD";
/// The program's `ProgramData` account and the slot of the deployment reviewed on 2026-10-09 (rule S3).
pub const PROGRAMDATA: &str = "6y7C7Lfh1WRRbKohE2FQmFBD2asw3yMi17kStwEcAWWF";
pub const PROGRAM_DEPLOY_SLOT: u64 = 386_211_280;
/// The state's `domain_separator` (every `execute_transfer` request carries it).
pub const DOMAIN_SEPARATOR: &str = "01c7a22107be8f4973259032638387b984782556d68bd9b8e3fa23c48f473eb4";
/// The one lookup table every quote names. The canister does not need it (5 static keys fit) and never uses a table it did not pin.
pub const QUOTE_LUT: &str = "Hm9fUgcn7qwDaiNTFiGh6pNtVATgnaRcmK6Bbx6EMZfP";
/// Relay's Solana solver hot wallet seen on 2026-10-09: it signs fills, refunds and the depository withdrawals.
pub const SOLVER_2026_10: &str = "F7p3dFrjRTbtRp8FRF6qHLomXbKRBzpvBLjtQcfcgmNe";
pub const BPF_LOADER_UPGRADEABLE: &str = "BPFLoaderUpgradeab1e11111111111111111111111";

pub const SYSTEM_PROGRAM: &str = "11111111111111111111111111111111";
pub const SOL_NATIVE: &str = SYSTEM_PROGRAM;
pub const MEMO_V2: &str = "MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr";
pub const MEMO_V1: &str = "Memo1UhkJRfHyvLMcVucJwxXeuD728EqVDDwQDxFMNo";
/// Relay's chain id for Solana.
pub const RELAY_SOLANA: u64 = 792_703_809;
pub const BTC_NATIVE: &str = crate::relayconv::BTC_NATIVE;
pub const EVM_NATIVE: &str = crate::relayconv::EVM_NATIVE;

/// Anchor discriminators of the depository (recomputed by the tests from `sha256("<ns>:<name>")`).
pub const IX_DEPOSIT_NATIVE: [u8; 8] = [0x0d, 0x9e, 0x0d, 0xdf, 0x5f, 0xd5, 0x1c, 0x06];
pub const IX_DEPOSIT_TOKEN: [u8; 8] = [0x0b, 0x9c, 0x60, 0xda, 0x27, 0xa3, 0xb4, 0x13];
pub const IX_EXECUTE_TRANSFER: [u8; 8] = [0xe9, 0x7e, 0xa0, 0xb8, 0xeb, 0xce, 0x1f, 0x77];
pub const EV_DEPOSIT: [u8; 8] = [0x78, 0xf8, 0x3d, 0x53, 0x1f, 0x8e, 0x6b, 0x90];
pub const EV_TRANSFER_EXECUTED: [u8; 8] = [0x5c, 0x0a, 0xb2, 0xb8, 0x12, 0x2c, 0x78, 0x7c];
pub const ACC_RELAY_DEPOSITORY: [u8; 8] = [0x80, 0x35, 0x9c, 0xbd, 0xf3, 0x53, 0xf5, 0x6b];

pub const QUOTE_TTL_SECS: u64 = crate::relayconv::QUOTE_TTL_SECS;
pub const MIN_DEADLINE_SECS: u64 = crate::relayconv::MIN_DEADLINE_SECS;
pub const MAX_DEADLINE_SECS: u64 = crate::relayconv::MAX_DEADLINE_SECS;
pub const MAX_QUOTE_BYTES: usize = 64 * 1024;

// ---- the deposit instruction ---------------------------------------------------------------------------------------------------

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
pub fn deposit_native_metas(ours: &Pubkey) -> Vec<AccountMeta> {
    vec![
        meta(soltx::key(RELAY_DEPOSITORY), false, false),
        meta(*ours, true, true),
        meta(*ours, false, false),
        meta(soltx::key(VAULT), false, true),
        meta(soltx::SYSTEM_PROGRAM, false, false),
    ]
}

/// The instruction the canister signs for a judged deposit: built from the judged `amount` and `id` alone, never copied from the quote.
pub fn deposit_instruction(ours: &Pubkey, d: &SolDeposit) -> Instruction {
    Instruction { program: soltx::key(DEPOSITORY_PROGRAM), accounts: deposit_native_metas(ours), data: encode_deposit_native(d.lamports, &d.order_id) }
}

/// The quote's instruction: program, metas, data.
pub fn quote_instruction(ix: &Value, at: &str) -> Result<(Pubkey, Vec<AccountMeta>, Vec<u8>), String> {
    let program = soltx::pubkey(text(ix, "programId", at)?)?;
    let mut metas = Vec::new();
    for (i, k) in list(ix, "keys", at)?.iter().enumerate() {
        let kat = format!("{at}.keys[{i}]");
        let flag = |f: &str| get(k, f, &kat).and_then(|x| x.as_bool().ok_or_else(|| format!("{kat}.{f} is not a bool")));
        metas.push(meta(soltx::pubkey(text(k, "pubkey", &kat)?)?, flag("isSigner")?, flag("isWritable")?));
    }
    let d = text(ix, "data", at)?;
    let data = hex::decode(d).map_err(|_| format!("{at}.data is not hex"))?;
    Ok((program, metas, data))
}

// ---- the judge (the Solana rules S1..S12; Q5 §3) -------------------------------------------------------------------------------

/// What a Solana-origin quote is judged against: the canister's own values and reads.
pub struct SolWant<'a> {
    /// The canister's Solana address (threshold Ed25519): the Solana refund recipient.
    pub our_sol: &'a str,
    /// Audit A5-03: the deposit's sender and depositor, the dedicated Relay key (`solkey::RELAY_DERIVATION`), funded with exactly the
    /// deposit and its fee: the only key ever handed to Relay's upgradeable depository as a signer. Never a refund recipient.
    pub depositor: &'a str,
    /// The destination as the order names it (`"bitcoin"`, `"ethereum"`), our address there and its native currency id.
    pub dest_chain: &'a str,
    pub dest_recipient: &'a str,
    pub dest_currency: &'a str,
    /// The lamports asked.
    pub lamports: u64,
    /// The least the order must guarantee, from the canister's own rate less its fee bound (rule S10).
    pub min_out_floor: u128,
    /// The canister's finalized reads of the depository state and its `ProgramData` (rule S3).
    pub allocator_read: &'a str,
    pub upgrade_authority_read: Option<&'a str>,
    pub deploy_slot_read: u64,
    pub now: u64,
    pub handed_in_at: u64,
    pub used: &'a [[u8; 32]],
}

/// The deposit the canister writes into its own message, from a quote that passed every rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SolDeposit {
    pub program: Pubkey,
    pub metas: Vec<AccountMeta>,
    pub data: Vec<u8>,
    pub order_id: [u8; 32],
    pub lamports: u64,
    pub min_out: u128,
    pub deadline: u64,
}

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
    soltx::pubkey(w.our_sol).map_err(|e| format!("our address: {e}"))?;
    let ours = soltx::pubkey(w.depositor).map_err(|e| format!("our Relay deposit key: {e}"))?;
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
    // S3: the pinned program, the allocator and the code we accepted
    if program != soltx::key(DEPOSITORY_PROGRAM) {
        return Err(format!("rule S3: the instruction goes to {}, not Relay's pinned depository program", b58_encode(&program)));
    }
    if w.allocator_read != ALLOCATOR {
        return Err(format!("rule S3: the depository's allocator is {}, not the pinned {ALLOCATOR}: review before converting", w.allocator_read));
    }
    if w.upgrade_authority_read != Some(OWNER_AND_UPGRADE_AUTHORITY) || w.deploy_slot_read != PROGRAM_DEPLOY_SLOT {
        return Err("rule S3: the depository program was upgraded (or its authority changed) since it was reviewed".into());
    }
    // S5: exactly the program's accounts for our Relay deposit key as sender and depositor, flags included
    if metas != deposit_native_metas(&ours) {
        return Err("rule S5: the accounts are not (relay_depository, our Relay key signer, our Relay key depositor, vault, system program)".into());
    }
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
    pub depositor: Pubkey,
    pub token: Option<Pubkey>,
    pub amount: u64,
    pub id: [u8; 32],
}

pub fn decode_deposit_event(b: &[u8]) -> Option<DepositEvent> {
    if b.get(..8)? != EV_DEPOSIT {
        return None;
    }
    let depositor: Pubkey = b.get(8..40)?.try_into().ok()?;
    let (token, at) = match *b.get(40)? {
        0 => (None, 41),
        1 => (Some(b.get(41..73)?.try_into().ok()?), 73),
        _ => return None,
    };
    let amount = u64::from_le_bytes(b.get(at..at + 8)?.try_into().ok()?);
    let id: [u8; 32] = b.get(at + 8..at + 40)?.try_into().ok()?;
    (b.len() == at + 40).then_some(DepositEvent { depositor, token, amount, id })
}

/// `TransferRequest { domain, recipient, token: Option<Pubkey>, amount, nonce, expiration, vault_address }` (Borsh).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransferRequest {
    pub domain: [u8; 32],
    pub recipient: Pubkey,
    pub token: Option<Pubkey>,
    pub amount: u64,
    pub nonce: u64,
    pub expiration: i64,
    pub vault_address: Pubkey,
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

/// `sha256(borsh(request))`: what the allocator signs, and the seed of the `used_request` PDA.
pub fn transfer_request_hash(r: &TransferRequest) -> [u8; 32] {
    Sha256::digest(encode_transfer_request(r)).into()
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
pub fn decode_transfer_executed(b: &[u8]) -> Option<(TransferRequest, Pubkey, [u8; 32])> {
    if b.get(..8)? != EV_TRANSFER_EXECUTED {
        return None;
    }
    let (r, n) = decode_transfer_request(&b[8..])?;
    let at = 8 + n;
    (b.len() == at + 64).then(|| (r, b[at..at + 32].try_into().expect("32"), b[at + 32..at + 64].try_into().expect("32")))
}

/// The deposit a finalized transaction made into the depository: the top-level `deposit_native` instruction and its
/// `DepositEvent`, which must agree. Returns the sender. `None` when the transaction failed or holds no such deposit.
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
/// `ours` and whose memo is `0x<order id>`. Returns the lamports credited. The memo is an unauthenticated LABEL: it decides which
/// conversion a credit belongs to, never whether we were paid (that is the balance rule, `relayconv::refunded`; Q5 §6.3).
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
pub fn decode_relay_depository(b: &[u8]) -> Option<(Pubkey, Pubkey, u8, Option<[u8; 32]>)> {
    if b.get(..8)? != ACC_RELAY_DEPOSITORY {
        return None;
    }
    let domain = match *b.get(73)? {
        0 => None,
        1 => Some(b.get(74..106)?.try_into().ok()?),
        _ => return None,
    };
    Some((b.get(8..40)?.try_into().ok()?, b.get(40..72)?.try_into().ok()?, b[72], domain))
}

/// A BPFLoaderUpgradeable `ProgramData` header: `(deploy slot, upgrade authority)` (rule S3; read with a 45-byte data slice).
pub fn decode_programdata_head(b: &[u8]) -> Option<(u64, Option<Pubkey>)> {
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

/// The S3 reads as [`SolWant`] takes them, from the two finalized accounts: the depository state (owned by the program) and the
/// program's `ProgramData` (owned by the upgradeable loader). Refuses accounts with the wrong owner.
pub fn s3_reads(state: &crate::solrpc::Account, programdata: &crate::solrpc::Account) -> Result<(String, Option<String>, u64), String> {
    if state.owner != soltx::key(DEPOSITORY_PROGRAM) {
        return Err("rule S3: the depository state is not owned by the depository program".into());
    }
    if programdata.owner != soltx::key(BPF_LOADER_UPGRADEABLE) {
        return Err("rule S3: the ProgramData is not owned by the upgradeable loader".into());
    }
    let (_, allocator, _, _) = decode_relay_depository(&state.data).ok_or("rule S3: the depository state does not decode")?;
    let (slot, auth) = decode_programdata_head(&programdata.data).ok_or("rule S3: the ProgramData header does not decode")?;
    Ok((b58_encode(&allocator), auth.map(|a| b58_encode(&a)), slot))
}

/// Anchor's discriminator, re-exported for the tests' recomputation.
pub fn disc(namespace: &str, name: &str) -> [u8; 8] {
    anchor_disc(namespace, name)
}
