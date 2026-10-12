//! Solana transaction bytes, by hand (no solana-sdk in the wasm; docs/v9/solana-migration.md §4, spike Q3
//! `ops/rehearsal/sol-sign/canister/src/soltx.rs`, whose legacy compiler is byte-for-byte the Solana CLI 4.3.0's): base58; the
//! legacy and v0 messages (address lookup tables included); the System Program instructions the send line needs (transfer,
//! create-with-seed, nonce initialize/advance); the durable-nonce prefix; compute-budget instructions; the 1,232-byte check;
//! the nonce account decoder; Anchor instruction encoding (`sha256("global:<name>")[..8]` ‖ Borsh) and a Borsh reader for
//! account data; program-derived addresses (`find_program_address`, sha256 + the off-curve check).
//!
//! Pure: no network, no keys, no clock. Every function is tested natively (`tests/solana.rs`).
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub type Pubkey = [u8; 32];
pub type Hash = [u8; 32];

/// The largest serialized transaction a Solana node accepts (IPv6 MTU 1,280 − 48 bytes of headers).
pub const PACKET_DATA_SIZE: usize = 1232;
pub const SYSTEM_PROGRAM: Pubkey = [0; 32];
pub const SYSVAR_RECENT_BLOCKHASHES: &str = "SysvarRecentB1ockHashes11111111111111111111";
pub const SYSVAR_RENT: &str = "SysvarRent111111111111111111111111111111111";
pub const COMPUTE_BUDGET_PROGRAM: &str = "ComputeBudget111111111111111111111111111111";
pub const TOKEN_PROGRAM: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
pub const ASSOCIATED_TOKEN_PROGRAM: &str = "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL";
/// The size of a nonce account (`nonce::State::size()`).
pub const NONCE_ACCOUNT_SPACE: u64 = 80;
/// `MAX_SEED_LEN` and `MAX_SEEDS` of `create_program_address`.
pub const MAX_SEED_LEN: usize = 32;
pub const MAX_SEEDS: usize = 16;

// ---- base58 ---------------------------------------------------------------------------------------------------------------------

const B58: &[u8; 58] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

pub fn b58_encode(input: &[u8]) -> String {
    let zeros = input.iter().take_while(|&&b| b == 0).count();
    let mut digits: Vec<u8> = Vec::with_capacity(input.len() * 138 / 100 + 1);
    for &byte in &input[zeros..] {
        let mut carry = byte as u32;
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
    let mut s = String::with_capacity(zeros + digits.len());
    s.extend(std::iter::repeat('1').take(zeros));
    s.extend(digits.iter().rev().map(|&d| B58[d as usize] as char));
    s
}

pub fn b58_decode(s: &str) -> Result<Vec<u8>, String> {
    let zeros = s.bytes().take_while(|&c| c == b'1').count();
    let mut bytes: Vec<u8> = Vec::with_capacity(s.len());
    for c in s.bytes().skip(zeros) {
        let v = B58.iter().position(|&x| x == c).ok_or_else(|| format!("not base58: {:?}", c as char))? as u32;
        let mut carry = v;
        for b in bytes.iter_mut() {
            carry += (*b as u32) * 58;
            *b = (carry & 0xff) as u8;
            carry >>= 8;
        }
        while carry > 0 {
            bytes.push((carry & 0xff) as u8);
            carry >>= 8;
        }
    }
    let mut out = vec![0u8; zeros];
    out.extend(bytes.iter().rev());
    Ok(out)
}

/// A Solana address: base58 of exactly 32 bytes.
pub fn pubkey(s: &str) -> Result<Pubkey, String> {
    b58_decode(s)?.try_into().map_err(|v: Vec<u8>| format!("{s}: {} bytes, not 32", v.len()))
}

/// A pinned constant (a sysvar, a program id): base58 that is known to decode.
pub fn key(s: &str) -> Pubkey {
    pubkey(s).expect("a constant Solana address")
}

/// A transaction signature: base58 of exactly 64 bytes.
pub fn signature(s: &str) -> Result<[u8; 64], String> {
    b58_decode(s)?.try_into().map_err(|v: Vec<u8>| format!("{s}: {} bytes, not 64", v.len()))
}

// ---- program-derived addresses ----------------------------------------------------------------------------------------------------

/// Whether 32 bytes decompress to a point of the ed25519 curve (`Pubkey::is_on_curve`). A program-derived address must NOT.
pub fn is_on_curve(k: &Pubkey) -> bool {
    curve25519_dalek::edwards::CompressedEdwardsY(*k).decompress().is_some()
}

/// `create_program_address`: `sha256(seeds ‖ program ‖ "ProgramDerivedAddress")`, refused when the point is on the curve (or
/// a seed is longer than 32 bytes, or there are more than 16 seeds).
pub fn create_program_address(seeds: &[&[u8]], program: &Pubkey) -> Option<Pubkey> {
    if seeds.len() > MAX_SEEDS || seeds.iter().any(|s| s.len() > MAX_SEED_LEN) {
        return None;
    }
    let mut h = Sha256::new();
    for s in seeds {
        h.update(s);
    }
    h.update(program);
    h.update(b"ProgramDerivedAddress");
    let k: Pubkey = h.finalize().into();
    (!is_on_curve(&k)).then_some(k)
}

/// `find_program_address`: the first off-curve address from bump 255 down, and its bump.
pub fn find_program_address(seeds: &[&[u8]], program: &Pubkey) -> Option<(Pubkey, u8)> {
    if seeds.len() >= MAX_SEEDS {
        return None;
    }
    (0..=255u8).rev().find_map(|bump| {
        let b = [bump];
        let mut s = seeds.to_vec();
        s.push(&b);
        create_program_address(&s, program).map(|k| (k, bump))
    })
}

/// The associated token account of `owner` for `mint` (SPL Token, not Token-2022).
pub fn associated_token_address(owner: &Pubkey, mint: &Pubkey) -> Pubkey {
    find_program_address(&[owner, &key(TOKEN_PROGRAM), mint], &key(ASSOCIATED_TOKEN_PROGRAM)).expect("an ATA always exists").0
}

// ---- instructions -----------------------------------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq, candid::CandidType, serde::Deserialize, serde::Serialize)]
pub struct AccountMeta {
    pub key: Pubkey,
    pub signer: bool,
    pub writable: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, candid::CandidType, serde::Deserialize, serde::Serialize)]
pub struct Instruction {
    pub program: Pubkey,
    pub accounts: Vec<AccountMeta>,
    pub data: Vec<u8>,
}

pub fn meta(key: Pubkey, signer: bool, writable: bool) -> AccountMeta {
    AccountMeta { key, signer, writable }
}

/// `SystemInstruction::Transfer { lamports }` (index 2).
pub fn transfer(from: Pubkey, to: Pubkey, lamports: u64) -> Instruction {
    let mut data = 2u32.to_le_bytes().to_vec();
    data.extend(lamports.to_le_bytes());
    Instruction { program: SYSTEM_PROGRAM, accounts: vec![meta(from, true, true), meta(to, false, true)], data }
}

/// `SystemInstruction::AdvanceNonceAccount` (index 4): the durable-nonce prefix, always the transaction's FIRST instruction.
pub fn advance_nonce(nonce: Pubkey, authority: Pubkey) -> Instruction {
    Instruction {
        program: SYSTEM_PROGRAM,
        accounts: vec![meta(nonce, false, true), meta(key(SYSVAR_RECENT_BLOCKHASHES), false, false), meta(authority, true, false)],
        data: 4u32.to_le_bytes().to_vec(),
    }
}

/// The durable-nonce form of a transaction's instructions: `AdvanceNonceAccount` first (a node treats a transaction as nonced only
/// when its first instruction is that advance), then the rest (compute-budget instructions come after it).
pub fn with_nonce(nonce: Pubkey, authority: Pubkey, rest: &[Instruction]) -> Vec<Instruction> {
    let mut v = vec![advance_nonce(nonce, authority)];
    v.extend_from_slice(rest);
    v
}

/// `SystemInstruction::CreateAccountWithSeed` (index 3), the base paying (one signer: the canister's own key).
pub fn create_with_seed(payer: Pubkey, created: Pubkey, base: Pubkey, seed: &str, lamports: u64, space: u64, owner: Pubkey) -> Instruction {
    let mut data = 3u32.to_le_bytes().to_vec();
    data.extend(base);
    data.extend((seed.len() as u64).to_le_bytes());
    data.extend(seed.as_bytes());
    data.extend(lamports.to_le_bytes());
    data.extend(space.to_le_bytes());
    data.extend(owner);
    let mut accounts = vec![meta(payer, true, true), meta(created, false, true)];
    if base != payer {
        accounts.push(meta(base, true, false));
    }
    Instruction { program: SYSTEM_PROGRAM, accounts, data }
}

/// `SystemInstruction::AllocateWithSeed { base, seed, space, owner }` (index 9): gives a seed-derived, System-owned account that holds
/// lamports but no data its `space` (and `owner`), signed by `base` alone (audit A4-02: a squatted nonce address is opened in place;
/// an outsider can transfer to the address but never allocate or assign it).
pub fn allocate_with_seed(account: Pubkey, base: Pubkey, seed: &str, space: u64, owner: Pubkey) -> Instruction {
    let mut data = 9u32.to_le_bytes().to_vec();
    data.extend(base);
    data.extend((seed.len() as u64).to_le_bytes());
    data.extend(seed.as_bytes());
    data.extend(space.to_le_bytes());
    data.extend(owner);
    Instruction { program: SYSTEM_PROGRAM, accounts: vec![meta(account, false, true), meta(base, true, false)], data }
}

/// `SystemInstruction::InitializeNonceAccount(authority)` (index 6).
pub fn initialize_nonce(nonce: Pubkey, authority: Pubkey) -> Instruction {
    let mut data = 6u32.to_le_bytes().to_vec();
    data.extend(authority);
    Instruction {
        program: SYSTEM_PROGRAM,
        accounts: vec![meta(nonce, false, true), meta(key(SYSVAR_RECENT_BLOCKHASHES), false, false), meta(key(SYSVAR_RENT), false, false)],
        data,
    }
}

/// `ComputeBudgetInstruction::SetComputeUnitLimit(units)` (tag 2).
pub fn compute_unit_limit(units: u32) -> Instruction {
    let mut data = vec![2u8];
    data.extend(units.to_le_bytes());
    Instruction { program: key(COMPUTE_BUDGET_PROGRAM), accounts: vec![], data }
}

/// `ComputeBudgetInstruction::SetComputeUnitPrice(micro_lamports)` (tag 3).
/// Audit B2-01: the one Compute Budget instruction a judged transaction may carry is `SetComputeUnitLimit` (tag 2): a unit PRICE is a
/// fee on the payer no limit covers, a heap frame or a loaded-accounts bound has no place in the canister's transactions.
pub fn compute_budget_ok(ix: &Instruction) -> Result<(), String> {
    if ix.program != key(COMPUTE_BUDGET_PROGRAM) {
        return Ok(());
    }
    match ix.data.first() {
        Some(2) if ix.data.len() == 5 => Ok(()),
        Some(3) => Err("a Compute Budget SetComputeUnitPrice: a priority fee on the payer the limit never covers".into()),
        other => Err(format!("a Compute Budget instruction other than SetComputeUnitLimit (tag {other:?})")),
    }
}

pub fn compute_unit_price(micro_lamports: u64) -> Instruction {
    let mut data = vec![3u8];
    data.extend(micro_lamports.to_le_bytes());
    Instruction { program: key(COMPUTE_BUDGET_PROGRAM), accounts: vec![], data }
}

/// `Pubkey::create_with_seed`: sha256(base ‖ seed ‖ owner).
pub fn address_with_seed(base: &Pubkey, seed: &str, owner: &Pubkey) -> Result<Pubkey, String> {
    if seed.len() > MAX_SEED_LEN {
        return Err("seed longer than 32 bytes".into());
    }
    let mut h = Sha256::new();
    h.update(base);
    h.update(seed.as_bytes());
    h.update(owner);
    Ok(h.finalize().into())
}

// ---- Anchor and Borsh -------------------------------------------------------------------------------------------------------------

/// An Anchor discriminator: `sha256("<namespace>:<name>")[..8]` (`global` for instructions, `account` for accounts, `event`).
pub fn anchor_disc(namespace: &str, name: &str) -> [u8; 8] {
    Sha256::digest(format!("{namespace}:{name}").as_bytes())[..8].try_into().expect("8 bytes")
}

/// Borsh, written field by field (little-endian integers, `u32` lengths, `Option` as 0/1, an enum as its `u8` variant index).
#[derive(Default, Clone, Debug)]
pub struct Borsh(pub Vec<u8>);

impl Borsh {
    /// An Anchor instruction's data: the discriminator of `global:<name>`, then the arguments.
    pub fn ix(name: &str) -> Self {
        Borsh(anchor_disc("global", name).to_vec())
    }
    pub fn u8(mut self, v: u8) -> Self {
        self.0.push(v);
        self
    }
    pub fn bool(self, v: bool) -> Self {
        self.u8(v as u8)
    }
    pub fn u16(mut self, v: u16) -> Self {
        self.0.extend(v.to_le_bytes());
        self
    }
    pub fn u32(mut self, v: u32) -> Self {
        self.0.extend(v.to_le_bytes());
        self
    }
    pub fn u64(mut self, v: u64) -> Self {
        self.0.extend(v.to_le_bytes());
        self
    }
    pub fn i64(mut self, v: i64) -> Self {
        self.0.extend(v.to_le_bytes());
        self
    }
    pub fn bytes(mut self, v: &[u8]) -> Self {
        self.0.extend_from_slice(v);
        self
    }
    pub fn string(self, s: &str) -> Self {
        self.u32(s.len() as u32).bytes(s.as_bytes())
    }
    pub fn vec32(mut self, v: &[[u8; 32]]) -> Self {
        self = self.u32(v.len() as u32);
        for x in v {
            self.0.extend_from_slice(x);
        }
        self
    }
    pub fn done(self) -> Vec<u8> {
        self.0
    }
}

/// Borsh, read field by field over account data. Every read is bounded; a short or malformed account is an `Err`, never a panic.
pub struct Reader<'a> {
    pub data: &'a [u8],
    pub at: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Reader { data, at: 0 }
    }
    /// After an Anchor account's discriminator, which must be `account:<name>`'s.
    pub fn account(data: &'a [u8], name: &str) -> Result<Self, String> {
        let want = anchor_disc("account", name);
        match data.get(..8) {
            Some(d) if d == want => Ok(Reader { data, at: 8 }),
            Some(_) => Err(format!("not a {name} account (discriminator)")),
            None => Err(format!("{} bytes: not a {name} account", data.len())),
        }
    }
    pub fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.at.checked_add(n).filter(|e| *e <= self.data.len()).ok_or_else(|| format!("short account: {} bytes", self.data.len()))?;
        let s = &self.data[self.at..end];
        self.at = end;
        Ok(s)
    }
    pub fn arr<const N: usize>(&mut self) -> Result<[u8; N], String> {
        Ok(self.take(N)?.try_into().expect("N bytes"))
    }
    pub fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    pub fn bool(&mut self) -> Result<bool, String> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            b => Err(format!("a bool of {b}")),
        }
    }
    pub fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_le_bytes(self.arr()?))
    }
    pub fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.arr()?))
    }
    pub fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_le_bytes(self.arr()?))
    }
    pub fn i64(&mut self) -> Result<i64, String> {
        Ok(i64::from_le_bytes(self.arr()?))
    }
    pub fn key(&mut self) -> Result<Pubkey, String> {
        self.arr()
    }
    /// A Borsh string no longer than the program's `max_len`.
    pub fn string(&mut self, max: usize) -> Result<String, String> {
        let n = self.u32()? as usize;
        if n > max {
            return Err(format!("a string of {n} bytes, over its {max}"));
        }
        String::from_utf8(self.take(n)?.to_vec()).map_err(|_| "a string that is not UTF-8".to_string())
    }
    /// A Borsh vector's length, no longer than the program's `max_len`.
    pub fn len(&mut self, max: usize) -> Result<usize, String> {
        let n = self.u32()? as usize;
        if n > max {
            return Err(format!("a vector of {n}, over its {max}"));
        }
        Ok(n)
    }
}

// ---- messages: legacy and v0 ------------------------------------------------------------------------------------------------------

pub fn compact_u16(n: usize, out: &mut Vec<u8>) {
    let mut v = n as u16;
    loop {
        let mut b = (v & 0x7f) as u8;
        v >>= 7;
        if v != 0 {
            b |= 0x80;
        }
        out.push(b);
        if v == 0 {
            break;
        }
    }
}

fn read_compact(b: &[u8], at: &mut usize) -> Result<usize, String> {
    let mut n = 0usize;
    for i in 0..3 {
        let x = *b.get(*at).ok_or("short message")?;
        *at += 1;
        n |= ((x & 0x7f) as usize) << (7 * i);
        if x & 0x80 == 0 {
            return Ok(n);
        }
    }
    Err("a compact-u16 over 3 bytes".into())
}

/// A message's header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub required_signatures: u8,
    pub readonly_signed: u8,
    pub readonly_unsigned: u8,
}

/// One compiled instruction: indexes into the message's keys (static, then the loaded writable, then the loaded read-only).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompiledIx {
    pub program: u8,
    pub accounts: Vec<u8>,
    pub data: Vec<u8>,
}

/// One address lookup table a v0 message loads keys from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableUse {
    pub table: Pubkey,
    pub writable: Vec<u8>,
    pub readonly: Vec<u8>,
}

/// A message in its parts. `version: None` is legacy; `Some(0)` is v0 (the only versioned form a node accepts today).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Parts {
    pub version: Option<u8>,
    pub header: Header,
    pub keys: Vec<Pubkey>,
    pub blockhash: Hash,
    pub instructions: Vec<CompiledIx>,
    pub tables: Vec<TableUse>,
}

impl Parts {
    pub fn serialize(&self) -> Vec<u8> {
        let mut b = Vec::new();
        if let Some(v) = self.version {
            b.push(0x80 | v);
        }
        b.extend([self.header.required_signatures, self.header.readonly_signed, self.header.readonly_unsigned]);
        compact_u16(self.keys.len(), &mut b);
        for k in &self.keys {
            b.extend(k);
        }
        b.extend(self.blockhash);
        compact_u16(self.instructions.len(), &mut b);
        for ix in &self.instructions {
            b.push(ix.program);
            compact_u16(ix.accounts.len(), &mut b);
            b.extend(&ix.accounts);
            compact_u16(ix.data.len(), &mut b);
            b.extend(&ix.data);
        }
        if self.version.is_some() {
            compact_u16(self.tables.len(), &mut b);
            for t in &self.tables {
                b.extend(t.table);
                compact_u16(t.writable.len(), &mut b);
                b.extend(&t.writable);
                compact_u16(t.readonly.len(), &mut b);
                b.extend(&t.readonly);
            }
        }
        b
    }

    /// The inverse of [`Parts::serialize`]; refuses trailing bytes.
    pub fn parse(b: &[u8]) -> Result<Parts, String> {
        let mut at = 0usize;
        let take = |at: &mut usize, n: usize| -> Result<Vec<u8>, String> {
            let s = b.get(*at..*at + n).ok_or("short message")?.to_vec();
            *at += n;
            Ok(s)
        };
        let first = *b.first().ok_or("empty message")?;
        let version = if first & 0x80 != 0 {
            at = 1;
            let v = first & 0x7f;
            if v != 0 {
                return Err(format!("message version {v}"));
            }
            Some(v)
        } else {
            None
        };
        let h = take(&mut at, 3)?;
        let header = Header { required_signatures: h[0], readonly_signed: h[1], readonly_unsigned: h[2] };
        let n = read_compact(b, &mut at)?;
        let keys = (0..n).map(|_| take(&mut at, 32).map(|k| k.try_into().expect("32"))).collect::<Result<Vec<Pubkey>, _>>()?;
        let blockhash: Hash = take(&mut at, 32)?.try_into().expect("32");
        let n = read_compact(b, &mut at)?;
        let mut instructions = Vec::with_capacity(n);
        for _ in 0..n {
            let program = take(&mut at, 1)?[0];
            let na = read_compact(b, &mut at)?;
            let accounts = take(&mut at, na)?;
            let nd = read_compact(b, &mut at)?;
            let data = take(&mut at, nd)?;
            instructions.push(CompiledIx { program, accounts, data });
        }
        let mut tables = Vec::new();
        if version.is_some() {
            let n = read_compact(b, &mut at)?;
            for _ in 0..n {
                let table: Pubkey = take(&mut at, 32)?.try_into().expect("32");
                let nw = read_compact(b, &mut at)?;
                let writable = take(&mut at, nw)?;
                let nr = read_compact(b, &mut at)?;
                let readonly = take(&mut at, nr)?;
                tables.push(TableUse { table, writable, readonly });
            }
        }
        if at != b.len() {
            return Err(format!("{} trailing bytes after the message", b.len() - at));
        }
        Ok(Parts { version, header, keys, blockhash, instructions, tables })
    }

    /// The signers, in signature order: the first `required_signatures` static keys.
    pub fn signers(&self) -> Vec<Pubkey> {
        self.keys.iter().take(self.header.required_signatures as usize).copied().collect()
    }

    /// The instructions back, given the keys loaded from the tables (writable then read-only, table by table, as a node loads them).
    pub fn decompile(&self, loaded_writable: &[Pubkey], loaded_readonly: &[Pubkey]) -> Result<Vec<Instruction>, String> {
        let all: Vec<Pubkey> = self.keys.iter().chain(loaded_writable).chain(loaded_readonly).copied().collect();
        let (n, h) = (self.keys.len(), self.header);
        let signed = h.required_signatures as usize;
        let flags = |i: usize| -> (bool, bool) {
            if i < n {
                let signer = i < signed;
                let writable = if signer { i < signed - h.readonly_signed as usize } else { i < n - h.readonly_unsigned as usize };
                (signer, writable)
            } else {
                (false, i < n + loaded_writable.len())
            }
        };
        self.instructions.iter().map(|ix| {
            let k = |i: u8| all.get(i as usize).copied().ok_or_else(|| format!("key index {i} out of {}", all.len()));
            Ok(Instruction {
                program: k(ix.program)?,
                accounts: ix.accounts.iter().map(|&i| { let (s, w) = flags(i as usize); k(i).map(|key| meta(key, s, w)) }).collect::<Result<_, String>>()?,
                data: ix.data.clone(),
            })
        }).collect()
    }
}

/// A compiled message: the bytes that are signed, and the keys that must sign them (in signature order).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub bytes: Vec<u8>,
    pub signers: Vec<Pubkey>,
}

#[derive(Clone, Copy, Default, Debug)]
struct KeyFlags {
    signer: bool,
    writable: bool,
    invoked: bool,
}

/// `CompiledKeys::compile`: every key once, its flags OR-ed across instructions, the payer a writable signer. A BTreeMap, so
/// keys come out sorted inside each class, as the SDK's do.
fn compiled_keys(instructions: &[Instruction], payer: Pubkey) -> BTreeMap<Pubkey, KeyFlags> {
    let mut m: BTreeMap<Pubkey, KeyFlags> = BTreeMap::new();
    for ix in instructions {
        m.entry(ix.program).or_default().invoked = true;
        for a in &ix.accounts {
            let f = m.entry(a.key).or_default();
            f.signer |= a.signer;
            f.writable |= a.writable;
        }
    }
    let f = m.entry(payer).or_default();
    f.signer = true;
    f.writable = true;
    m
}

/// `try_into_message_components`: payer, writable signers, read-only signers, writable non-signers, read-only non-signers.
fn static_keys(mut m: BTreeMap<Pubkey, KeyFlags>, payer: Pubkey) -> Result<(Header, Vec<Pubkey>), String> {
    m.remove(&payer);
    let class = |s: bool, w: bool| m.iter().filter(move |(_, f)| f.signer == s && f.writable == w).map(|(k, _)| *k);
    let ws: Vec<Pubkey> = std::iter::once(payer).chain(class(true, true)).collect();
    let rs: Vec<Pubkey> = class(true, false).collect();
    let wn: Vec<Pubkey> = class(false, true).collect();
    let rn: Vec<Pubkey> = class(false, false).collect();
    let u8_of = |n: usize| u8::try_from(n).map_err(|_| format!("{n} keys: a message holds at most 255"));
    let header = Header { required_signatures: u8_of(ws.len() + rs.len())?, readonly_signed: u8_of(rs.len())?, readonly_unsigned: u8_of(rn.len())? };
    Ok((header, ws.into_iter().chain(rs).chain(wn).chain(rn).collect()))
}

fn compile_instructions(instructions: &[Instruction], all: &[Pubkey]) -> Result<Vec<CompiledIx>, String> {
    let index = |k: &Pubkey| -> Result<u8, String> {
        let i = all.iter().position(|x| x == k).ok_or_else(|| format!("{} is in no key list", b58_encode(k)))?;
        u8::try_from(i).map_err(|_| "a key index over 255".to_string())
    };
    instructions.iter().map(|ix| Ok(CompiledIx {
        program: index(&ix.program)?,
        accounts: ix.accounts.iter().map(|a| index(&a.key)).collect::<Result<_, String>>()?,
        data: ix.data.clone(),
    })).collect()
}

/// `Message::new_with_blockhash(instructions, Some(payer), blockhash)`: the legacy message.
pub fn compile(instructions: &[Instruction], payer: Pubkey, blockhash: Hash) -> Message {
    compile_parts(instructions, payer, blockhash).expect("a legacy message of the send line's size").message()
}

pub fn compile_parts(instructions: &[Instruction], payer: Pubkey, blockhash: Hash) -> Result<Parts, String> {
    let (header, keys) = static_keys(compiled_keys(instructions, payer), payer)?;
    let instructions = compile_instructions(instructions, &keys)?;
    Ok(Parts { version: None, header, keys, blockhash, instructions, tables: vec![] })
}

/// An address lookup table's contents as the canister read (and pinned) them: never one the canister did not read itself.
#[derive(Clone, Debug, PartialEq, Eq, candid::CandidType, serde::Deserialize, serde::Serialize)]
pub struct LookupTable {
    pub key: Pubkey,
    pub addresses: Vec<Pubkey>,
}

/// `v0::Message::try_compile`: the legacy key set, less every non-signer, non-program key found in a table (writable first, then
/// read-only; each in key order; a table that supplies nothing is not named).
pub fn compile_v0(instructions: &[Instruction], payer: Pubkey, blockhash: Hash, tables: &[LookupTable]) -> Result<Message, String> {
    Ok(compile_v0_parts(instructions, payer, blockhash, tables)?.message())
}

pub fn compile_v0_parts(instructions: &[Instruction], payer: Pubkey, blockhash: Hash, tables: &[LookupTable]) -> Result<Parts, String> {
    let mut m = compiled_keys(instructions, payer);
    let (mut uses, mut loaded_w, mut loaded_r) = (Vec::new(), Vec::new(), Vec::new());
    for t in tables {
        let mut drain = |writable: bool| -> Result<(Vec<u8>, Vec<Pubkey>), String> {
            let found: Vec<(u8, Pubkey)> = m.iter()
                .filter(|(_, f)| !f.signer && !f.invoked && f.writable == writable)
                .filter_map(|(k, _)| t.addresses.iter().position(|a| a == k).map(|i| (i, *k)))
                .map(|(i, k)| u8::try_from(i).map(|i| (i, k)).map_err(|_| "a table index over 255".to_string()))
                .collect::<Result<_, _>>()?;
            for (_, k) in &found {
                m.remove(k);
            }
            Ok(found.into_iter().unzip())
        };
        let (wi, wk) = drain(true)?;
        let (ri, rk) = drain(false)?;
        if wi.is_empty() && ri.is_empty() {
            continue;
        }
        uses.push(TableUse { table: t.key, writable: wi, readonly: ri });
        loaded_w.extend(wk);
        loaded_r.extend(rk);
    }
    let (header, keys) = static_keys(m, payer)?;
    let all: Vec<Pubkey> = keys.iter().chain(&loaded_w).chain(&loaded_r).copied().collect();
    let instructions = compile_instructions(instructions, &all)?;
    Ok(Parts { version: Some(0), header, keys, blockhash, instructions, tables: uses })
}

impl Parts {
    pub fn message(&self) -> Message {
        Message { bytes: self.serialize(), signers: self.signers() }
    }
}

/// The wire transaction: compact(n) ‖ signatures ‖ message. Refuses what a node would refuse for size.
pub fn assemble(msg: &Message, signatures: &[[u8; 64]]) -> Result<Vec<u8>, String> {
    if signatures.len() != msg.signers.len() {
        return Err(format!("{} signatures for {} signers", signatures.len(), msg.signers.len()));
    }
    let mut tx = Vec::with_capacity(1 + 64 * signatures.len() + msg.bytes.len());
    compact_u16(signatures.len(), &mut tx);
    for s in signatures {
        tx.extend(s);
    }
    tx.extend(&msg.bytes);
    check_size(tx.len())?;
    Ok(tx)
}

/// A wire transaction's signatures and message bytes.
pub fn split_tx(tx: &[u8]) -> Result<(Vec<[u8; 64]>, &[u8]), String> {
    let mut at = 0;
    let n = read_compact(tx, &mut at)?;
    let mut sigs = Vec::with_capacity(n);
    for _ in 0..n {
        sigs.push(tx.get(at..at + 64).ok_or("short transaction")?.try_into().expect("64"));
        at += 64;
    }
    Ok((sigs, &tx[at..]))
}

/// The 1,232-byte check, made BEFORE signing (the size is known from the message and the signer count) and again on the bytes.
pub fn check_size(len: usize) -> Result<(), String> {
    if len > PACKET_DATA_SIZE {
        Err(format!("transaction is {len} bytes; Solana's limit is {PACKET_DATA_SIZE}"))
    } else {
        Ok(())
    }
}

pub fn wire_size(msg: &Message) -> usize {
    let mut n = Vec::new();
    compact_u16(msg.signers.len(), &mut n);
    n.len() + 64 * msg.signers.len() + msg.bytes.len()
}

// ---- address lookup tables ------------------------------------------------------------------------------------------------------

/// The Address Lookup Table program.
pub const ADDRESS_LOOKUP_TABLE_PROGRAM: &str = "AddressLookupTab1e1111111111111111111111111";
/// `LOOKUP_TABLE_META_SIZE`: the table's header before its addresses.
pub const LOOKUP_TABLE_META_SIZE: usize = 56;

/// A table's address: `find_program_address([authority, recent_slot (LE)], ALT program)`.
pub fn lookup_table_address(authority: &Pubkey, recent_slot: u64) -> (Pubkey, u8) {
    find_program_address(&[authority, &recent_slot.to_le_bytes()], &key(ADDRESS_LOOKUP_TABLE_PROGRAM)).expect("a PDA")
}

/// `ProgramInstruction::CreateLookupTable { recent_slot, bump_seed }` (index 0), `authority` signing (`create_lookup_table_signed`):
/// the table at [`lookup_table_address`], paid by `payer`. `recent_slot` must be in the SlotHashes sysvar when it executes.
pub fn create_lookup_table(authority: Pubkey, payer: Pubkey, recent_slot: u64) -> (Instruction, Pubkey) {
    let (table, bump) = lookup_table_address(&authority, recent_slot);
    let mut data = 0u32.to_le_bytes().to_vec();
    data.extend(recent_slot.to_le_bytes());
    data.push(bump);
    (Instruction { program: key(ADDRESS_LOOKUP_TABLE_PROGRAM), accounts: vec![meta(table, false, true), meta(authority, true, false),
        meta(payer, true, true), meta(SYSTEM_PROGRAM, false, false)], data }, table)
}

/// `ProgramInstruction::ExtendLookupTable { new_addresses }` (index 2): appends; `payer` funds the larger account's rent.
pub fn extend_lookup_table(table: Pubkey, authority: Pubkey, payer: Pubkey, addresses: &[Pubkey]) -> Instruction {
    let mut data = 2u32.to_le_bytes().to_vec();
    data.extend((addresses.len() as u64).to_le_bytes());
    for a in addresses {
        data.extend(a);
    }
    Instruction { program: key(ADDRESS_LOOKUP_TABLE_PROGRAM), accounts: vec![meta(table, false, true), meta(authority, true, false),
        meta(payer, true, true), meta(SYSTEM_PROGRAM, false, false)], data }
}

/// A lookup table account as read: `ProgramState::LookupTable(meta)` (bincode: u32 tag 1, deactivation slot, last extended slot, its
/// start index, `Option<authority>`, u16 padding: 56 bytes), then the addresses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LookupTableState {
    pub deactivation_slot: u64,
    pub last_extended_slot: u64,
    pub last_extended_start: u8,
    pub authority: Option<Pubkey>,
    pub addresses: Vec<Pubkey>,
}

pub fn decode_lookup_table(data: &[u8]) -> Result<LookupTableState, String> {
    if data.len() < LOOKUP_TABLE_META_SIZE || (data.len() - LOOKUP_TABLE_META_SIZE) % 32 != 0 {
        return Err(format!("a lookup table of {} bytes", data.len()));
    }
    if data[0..4] != [1, 0, 0, 0] {
        return Err("not an initialized lookup table".into());
    }
    let u = |a: usize| u64::from_le_bytes(data[a..a + 8].try_into().expect("8"));
    let authority = match data[21] {
        0 => None,
        1 => Some(data[22..54].try_into().expect("32")),
        x => return Err(format!("a lookup table's authority tag {x}")),
    };
    Ok(LookupTableState { deactivation_slot: u(4), last_extended_slot: u(12), last_extended_start: data[20], authority,
        addresses: data[LOOKUP_TABLE_META_SIZE..].chunks(32).map(|c| c.try_into().expect("32")).collect() })
}

/// The bytes of a lookup table account (the fakes and the tests).
pub fn encode_lookup_table(t: &LookupTableState) -> Vec<u8> {
    let mut d = 1u32.to_le_bytes().to_vec();
    d.extend(t.deactivation_slot.to_le_bytes());
    d.extend(t.last_extended_slot.to_le_bytes());
    d.push(t.last_extended_start);
    match t.authority {
        Some(a) => {
            d.push(1);
            d.extend(a);
        }
        None => d.extend([0u8; 33]),
    }
    d.extend([0, 0]);
    for a in &t.addresses {
        d.extend(a);
    }
    d
}

/// The keys a v0 message loads from `tables` (as read), writable then read-only, table by table in the message's order: what
/// [`Parts::decompile`] takes. Refuses a table the message names that is not among them, and an index past a table's end.
pub fn loaded_keys(p: &Parts, tables: &[LookupTable]) -> Result<(Vec<Pubkey>, Vec<Pubkey>), String> {
    let (mut w, mut r) = (vec![], vec![]);
    for u in &p.tables {
        let t = tables.iter().find(|t| t.key == u.table).ok_or_else(|| format!("the message loads table {}, not one read", b58_encode(&u.table)))?;
        let at = |i: &u8| t.addresses.get(*i as usize).copied().ok_or_else(|| format!("index {i} past table {}'s end", b58_encode(&t.key)));
        w.extend(u.writable.iter().map(at).collect::<Result<Vec<_>, _>>()?);
        r.extend(u.readonly.iter().map(at).collect::<Result<Vec<_>, _>>()?);
    }
    Ok((w, r))
}

// ---- the nonce account ------------------------------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NonceState {
    pub authority: Pubkey,
    pub durable_nonce: Hash,
    pub lamports_per_signature: u64,
}

/// `nonce::state::Versions` (bincode): version u32 ‖ state u32 (1 = Initialized) ‖ authority ‖ durable nonce ‖ fee calculator u64.
pub fn decode_nonce(data: &[u8]) -> Result<NonceState, String> {
    if data.len() != 80 {
        return Err(format!("a nonce account holds 80 bytes, not {}", data.len()));
    }
    let state = u32::from_le_bytes(data[4..8].try_into().expect("4"));
    if state != 1 {
        return Err(format!("nonce account not initialized (state {state})"));
    }
    Ok(NonceState {
        authority: data[8..40].try_into().expect("32"),
        durable_nonce: data[40..72].try_into().expect("32"),
        lamports_per_signature: u64::from_le_bytes(data[72..80].try_into().expect("8")),
    })
}

/// The bytes of an initialized nonce account (the fakes and the tests).
pub fn encode_nonce(n: &NonceState) -> Vec<u8> {
    let mut d = vec![1, 0, 0, 0, 1, 0, 0, 0];
    d.extend(n.authority);
    d.extend(n.durable_nonce);
    d.extend(n.lamports_per_signature.to_le_bytes());
    d
}
