//! Solana transaction bytes, by hand (no solana-sdk in the wasm): base58, the legacy message, the System Program instructions the
//! spike needs (transfer, create-with-seed, nonce initialize/advance), the durable-nonce prefix, the 1,232-byte check and the nonce
//! account decoder. Pure: every function here is tested natively (`tests.rs`), and the bytes are held to the Solana CLI's own
//! `--sign-only` output there.
use sha2::{Digest, Sha256};

pub type Pubkey = [u8; 32];
pub type Hash = [u8; 32];

/// The largest serialized transaction a Solana node accepts (IPv6 MTU 1,280 − 48 bytes of headers).
pub const PACKET_DATA_SIZE: usize = 1232;
pub const SYSTEM_PROGRAM: Pubkey = [0; 32];
/// `SysvarRecentB1ockHashes11111111111111111111`
pub const SYSVAR_RECENT_BLOCKHASHES: &str = "SysvarRecentB1ockHashes11111111111111111111";
/// `SysvarRent111111111111111111111111111111111`
pub const SYSVAR_RENT: &str = "SysvarRent111111111111111111111111111111111";
/// The size of a nonce account (`nonce::State::size()`).
pub const NONCE_ACCOUNT_SPACE: u64 = 80;

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

pub fn pubkey(s: &str) -> Result<Pubkey, String> {
    b58_decode(s)?.try_into().map_err(|v: Vec<u8>| format!("{s}: {} bytes, not 32", v.len()))
}

// ---- instructions -----------------------------------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountMeta {
    pub key: Pubkey,
    pub signer: bool,
    pub writable: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Instruction {
    pub program: Pubkey,
    pub accounts: Vec<AccountMeta>,
    pub data: Vec<u8>,
}

fn meta(key: Pubkey, signer: bool, writable: bool) -> AccountMeta {
    AccountMeta { key, signer, writable }
}

fn sysvar(s: &str) -> Pubkey {
    pubkey(s).expect("a constant sysvar id")
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
        accounts: vec![meta(nonce, false, true), meta(sysvar(SYSVAR_RECENT_BLOCKHASHES), false, false), meta(authority, true, false)],
        data: 4u32.to_le_bytes().to_vec(),
    }
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

/// `SystemInstruction::InitializeNonceAccount(authority)` (index 6).
pub fn initialize_nonce(nonce: Pubkey, authority: Pubkey) -> Instruction {
    let mut data = 6u32.to_le_bytes().to_vec();
    data.extend(authority);
    Instruction {
        program: SYSTEM_PROGRAM,
        accounts: vec![meta(nonce, false, true), meta(sysvar(SYSVAR_RECENT_BLOCKHASHES), false, false), meta(sysvar(SYSVAR_RENT), false, false)],
        data,
    }
}

/// `Pubkey::create_with_seed`: sha256(base ‖ seed ‖ owner).
pub fn address_with_seed(base: &Pubkey, seed: &str, owner: &Pubkey) -> Result<Pubkey, String> {
    if seed.len() > 32 {
        return Err("seed longer than 32 bytes".into());
    }
    let mut h = Sha256::new();
    h.update(base);
    h.update(seed.as_bytes());
    h.update(owner);
    Ok(h.finalize().into())
}

// ---- the legacy message -----------------------------------------------------------------------------------------------------------

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

/// A compiled legacy message: the bytes that are signed, and how many signatures the transaction carries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub bytes: Vec<u8>,
    pub signers: Vec<Pubkey>,
}

/// `Message::new_with_blockhash(instructions, Some(payer), blockhash)`: keys ordered payer, writable signers, readonly signers,
/// writable non-signers, readonly non-signers, sorted by key inside each class (the SDK's `CompiledKeys` is a BTreeMap).
pub fn compile(instructions: &[Instruction], payer: Pubkey, blockhash: Hash) -> Message {
    let mut keys: Vec<AccountMeta> = vec![meta(payer, true, true)];
    let mut add = |m: AccountMeta| {
        if let Some(k) = keys.iter_mut().find(|k| k.key == m.key) {
            k.signer |= m.signer;
            k.writable |= m.writable;
        } else {
            keys.push(m);
        }
    };
    for ix in instructions {
        for a in &ix.accounts {
            add(a.clone());
        }
        add(meta(ix.program, false, false));
    }
    let class = |k: &AccountMeta| match (k.signer, k.writable) {
        (true, true) => 0,
        (true, false) => 1,
        (false, true) => 2,
        (false, false) => 3,
    };
    let payer_meta = keys.remove(0);
    let mut ordered = vec![payer_meta];
    for c in 0..4 {
        let mut cls: Vec<AccountMeta> = keys.iter().filter(|k| class(k) == c).cloned().collect();
        cls.sort_by(|a, b| a.key.cmp(&b.key));
        ordered.extend(cls);
    }
    let n_sig = ordered.iter().filter(|k| k.signer).count();
    let n_ro_signed = ordered.iter().filter(|k| k.signer && !k.writable).count();
    let n_ro_unsigned = ordered.iter().filter(|k| !k.signer && !k.writable).count();
    let index = |key: &Pubkey| ordered.iter().position(|k| &k.key == key).expect("every key was added") as u8;

    let mut b = vec![n_sig as u8, n_ro_signed as u8, n_ro_unsigned as u8];
    compact_u16(ordered.len(), &mut b);
    for k in &ordered {
        b.extend(k.key);
    }
    b.extend(blockhash);
    compact_u16(instructions.len(), &mut b);
    for ix in instructions {
        b.push(index(&ix.program));
        compact_u16(ix.accounts.len(), &mut b);
        for a in &ix.accounts {
            b.push(index(&a.key));
        }
        compact_u16(ix.data.len(), &mut b);
        b.extend(&ix.data);
    }
    Message { bytes: b, signers: ordered.iter().filter(|k| k.signer).map(|k| k.key).collect() }
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
    let state = u32::from_le_bytes(data[4..8].try_into().unwrap());
    if state != 1 {
        return Err(format!("nonce account not initialized (state {state})"));
    }
    Ok(NonceState {
        authority: data[8..40].try_into().unwrap(),
        durable_nonce: data[40..72].try_into().unwrap(),
        lamports_per_signature: u64::from_le_bytes(data[72..80].try_into().unwrap()),
    })
}
