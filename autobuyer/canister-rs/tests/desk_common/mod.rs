//! The desk's test world (W4/W5): the fake Solana provider pair (`tests/fakerpc/sol.rs`), a signer holding REAL Ed25519 keys per
//! `KeyPath`, the four lines, and a MINI RUNTIME that executes what lands (System transfers with the rent rule, the Associated Token
//! Account `CreateIdempotent`, SPL `MintTo` / `Burn` / `Transfer` / `CloseAccount` with their authority checks, Memo) on the fake
//! chain's accounts, so supplies and balances are the chain's own and a wrong amount, owner or signer makes a transaction fail the way
//! it would on Solana (the simulation before signing runs the same code).
#![allow(dead_code)]

use super::fakerpc;

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::future::Future;
use std::pin::pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

use ed25519_dalek::{Signer, SigningKey};
use etch_autobuyer::books::{self, Books, ListingKind};
use etch_autobuyer::chainlink::DeskIo;
use etch_autobuyer::desk::state;
use etch_autobuyer::solhome::{self, LineId, SolHomeCfg, SolIo, SolLine};
use etch_autobuyer::solkey::{KeyPath, SolSigner};
use etch_autobuyer::solrpc::{self, Account, Level, SigInfo};
use etch_autobuyer::solsend::{self, LifetimeKind, Line, Step};
use etch_autobuyer::soltx::{self, Instruction, Pubkey, SYSTEM_PROGRAM};
use fakerpc::sol::{pair, FakeSol, SolChain};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub fn block_on<F: Future>(f: F) -> F::Output {
    let mut f = pin!(f);
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
    }
}

pub const SEED: &str = "rook-home-0";
pub const SOL: u64 = 1_000_000_000;
/// A floor of 200 SOL a whole unit (10 ETH).
pub const FLOOR: u64 = 200 * SOL;
pub const RENT0: u64 = 890_880;
pub const RENT165: u64 = 2_039_280;
/// The collection under test.
pub const COLL: u16 = 0;
/// The wX mint of collection 0.
pub const WX: Pubkey = [0xC0; 32];

/// A signer with a real Ed25519 key per path.
#[derive(Clone)]
pub struct RealSigner {
    pub calls: Rc<RefCell<Vec<(KeyPath, Vec<u8>)>>>,
}

pub fn sk(p: &KeyPath) -> SigningKey {
    let seed: [u8; 32] = Sha256::digest([b"rook-desk-test".to_vec(), p.derivation().concat()].concat()).into();
    SigningKey::from_bytes(&seed)
}

pub fn pk(p: KeyPath) -> Pubkey {
    sk(&p).verifying_key().to_bytes()
}

pub fn me() -> Pubkey {
    pk(KeyPath::Main)
}

pub fn module() -> Pubkey {
    pk(KeyPath::Coll(COLL as u32))
}

pub fn intent_key(n: u64) -> Pubkey {
    pk(KeyPath::Intent(n))
}

/// A wallet of the users.
pub fn wallet(n: u8) -> Pubkey {
    [0x70 + n; 32]
}

impl RealSigner {
    pub fn new() -> RealSigner {
        RealSigner { calls: Rc::new(RefCell::new(vec![])) }
    }
    fn sign(&mut self, p: &KeyPath, m: &[u8]) -> Result<[u8; 64], String> {
        self.calls.borrow_mut().push((*p, m.to_vec()));
        Ok(sk(p).sign(m).to_bytes())
    }
    pub fn signed(&self) -> usize {
        self.calls.borrow().len()
    }
}

impl SolSigner for RealSigner {
    fn me(&self) -> Pubkey {
        pk(KeyPath::Main)
    }
    async fn sign_bytes(&mut self, message: &[u8]) -> Result<[u8; 64], String> {
        self.sign(&KeyPath::Main, message)
    }
    fn relay(&self) -> Option<Pubkey> {
        Some(pk(KeyPath::Relay))
    }
    async fn sign_relay_bytes(&mut self, message: &[u8]) -> Result<[u8; 64], String> {
        self.sign(&KeyPath::Relay, message)
    }
    async fn key(&mut self, path: &KeyPath) -> Result<Pubkey, String> {
        Ok(pk(*path))
    }
    async fn sign_paths(&mut self, paths: &[KeyPath], message: &[u8]) -> Vec<Result<[u8; 64], String>> {
        paths.iter().map(|p| self.sign(p, message)).collect()
    }
}

// ---- the mini runtime ------------------------------------------------------------------------------------------------------------

fn token_program() -> Pubkey {
    soltx::key(soltx::TOKEN_PROGRAM)
}

pub fn mint_data(authority: Pubkey, supply: u64) -> Vec<u8> {
    let mut d = vec![0u8; 82];
    d[0..4].copy_from_slice(&1u32.to_le_bytes());
    d[4..36].copy_from_slice(&authority);
    d[36..44].copy_from_slice(&supply.to_le_bytes());
    d[44] = 9;
    d[45] = 1;
    d
}

pub fn token_data(mint: Pubkey, owner: Pubkey, amount: u64) -> Vec<u8> {
    let mut d = vec![0u8; 165];
    d[0..32].copy_from_slice(&mint);
    d[32..64].copy_from_slice(&owner);
    d[64..72].copy_from_slice(&amount.to_le_bytes());
    d[108] = 1;
    d
}

fn amount_of(a: &Account) -> u64 {
    u64::from_le_bytes(a.data[64..72].try_into().unwrap())
}
fn set_amount(a: &mut Account, v: u64) {
    a.data[64..72].copy_from_slice(&v.to_le_bytes());
}
fn supply_of(a: &Account) -> u64 {
    u64::from_le_bytes(a.data[36..44].try_into().unwrap())
}

fn err(msg: &str) -> Value {
    json!({ "InstructionError": [0, { "Custom": msg }] })
}

/// What a transaction's instructions do to the accounts (Solana's rules that matter here: signers, owners, balances, and rent-exempt
/// minimums at the end of the transaction).
pub fn run_ixs(ixs: &[Instruction], acc: &mut BTreeMap<Pubkey, Account>) -> Result<(), Value> {
    let tok = token_program();
    let ata_prog = soltx::key(soltx::ASSOCIATED_TOKEN_PROGRAM);
    let memo_prog = soltx::key(etch_autobuyer::relaysol::MEMO_V2);
    let mut touched: Vec<Pubkey> = vec![];
    for ix in ixs {
        for m in &ix.accounts {
            touched.push(m.key);
        }
        let k = |i: usize| ix.accounts.get(i).map(|m| m.key).ok_or_else(|| err("missing account"));
        let signer = |i: usize| ix.accounts.get(i).map(|m| m.signer).unwrap_or(false);
        if ix.program == SYSTEM_PROGRAM {
            if ix.data.len() == 12 && ix.data[..4] == [2, 0, 0, 0] {
                let amount = u64::from_le_bytes(ix.data[4..12].try_into().unwrap());
                let (from, to) = (k(0)?, k(1)?);
                if !signer(0) {
                    return Err(err("system transfer: from did not sign"));
                }
                let f = acc.get_mut(&from).ok_or_else(|| err("transfer from a missing account"))?;
                if f.owner != SYSTEM_PROGRAM || !f.data.is_empty() {
                    return Err(err("transfer from an account with data"));
                }
                f.lamports = f.lamports.checked_sub(amount).ok_or_else(|| json!("InsufficientFundsForFee"))?;
                let t = acc.entry(to).or_insert(Account { lamports: 0, owner: SYSTEM_PROGRAM, executable: false, data: vec![] });
                t.lamports += amount;
            } else if ix.data.len() >= 92 && ix.data[..4] == [3, 0, 0, 0] {
                // CreateAccountWithSeed { base, seed, lamports, space, owner }: the created address is checked against the seed, the
                // payer funds it, and it comes up owned by `owner` with `space` zero bytes (the wX mint of a collection's opening)
                let (payer, created) = (k(0)?, k(1)?);
                if !signer(0) {
                    return Err(err("create with seed: the payer did not sign"));
                }
                let base: Pubkey = ix.data[4..36].try_into().unwrap();
                let n = u64::from_le_bytes(ix.data[36..44].try_into().unwrap()) as usize;
                let seed = std::str::from_utf8(&ix.data[44..44 + n]).map_err(|_| err("create with seed: a non-utf8 seed"))?;
                let rest = &ix.data[44 + n..];
                let lamports = u64::from_le_bytes(rest[0..8].try_into().unwrap());
                let space = u64::from_le_bytes(rest[8..16].try_into().unwrap()) as usize;
                let owner: Pubkey = rest[16..48].try_into().unwrap();
                if base != payer && !ix.accounts.iter().any(|m| m.key == base && m.signer) {
                    return Err(err("create with seed: the base did not sign"));
                }
                if soltx::address_with_seed(&base, seed, &owner).ok() != Some(created) {
                    return Err(err("create with seed: the address is not the seed's"));
                }
                // audit C4-13: as Solana's `create_account`, an address that holds lamports already (a plain transfer to it is enough)
                // is refused `AccountAlreadyInUse`, whatever its owner and data: the opening allocates over it instead (C4-01)
                if acc.get(&created).is_some_and(|a| a.lamports > 0 || !a.data.is_empty() || a.owner != SYSTEM_PROGRAM) {
                    return Err(err("create with seed: AccountAlreadyInUse"));
                }
                let p = acc.get_mut(&payer).ok_or_else(|| err("create with seed: the payer is missing"))?;
                p.lamports = p.lamports.checked_sub(lamports).ok_or_else(|| json!("InsufficientFundsForFee"))?;
                acc.insert(created, Account { lamports, owner, executable: false, data: vec![0; space] });
            } else if ix.data.len() >= 60 && ix.data[..4] == [9, 0, 0, 0] {
                // AllocateWithSeed { base, seed, space, owner } (audit C4-01, C4-13): a System-owned account with no data at the seed's
                // address gets its space and its owner, signed by the base alone (an outsider can transfer to the address, never this)
                let (account, base_acc) = (k(0)?, k(1)?);
                let base: Pubkey = ix.data[4..36].try_into().unwrap();
                let n = u64::from_le_bytes(ix.data[36..44].try_into().unwrap()) as usize;
                let seed = std::str::from_utf8(&ix.data[44..44 + n]).map_err(|_| err("allocate with seed: a non-utf8 seed"))?;
                let rest = &ix.data[44 + n..];
                let space = u64::from_le_bytes(rest[0..8].try_into().unwrap()) as usize;
                let owner: Pubkey = rest[8..40].try_into().unwrap();
                if base_acc != base || !signer(1) {
                    return Err(err("allocate with seed: the base did not sign"));
                }
                if soltx::address_with_seed(&base, seed, &owner).ok() != Some(account) {
                    return Err(err("allocate with seed: the address is not the seed's"));
                }
                let a = acc.get_mut(&account).ok_or_else(|| err("allocate with seed: no account there"))?;
                if !a.data.is_empty() || a.owner != SYSTEM_PROGRAM {
                    return Err(err("allocate with seed: AccountAlreadyInUse"));
                }
                a.owner = owner;
                a.data = vec![0; space];
            } else {
                return Err(err("unsupported system instruction"));
            }
        } else if ix.program == memo_prog {
            // a label
        } else if ix.program == soltx::key(soltx::ADDRESS_LOOKUP_TABLE_PROGRAM) {
            // the Address Lookup Table program (2026-10-12, `solnftflow::table`): create at the authority's derived address, extend by
            // its authority (LiteSVM runs the real program: autobuyer/solnft-svm/tests/m2_table.rs)
            let (table, authority) = (k(0)?, k(1)?);
            if !signer(1) || !signer(2) {
                return Err(err("lookup table: the authority or the payer did not sign"));
            }
            match ix.data.get(..4) {
                Some([0, 0, 0, 0]) => {
                    let slot = u64::from_le_bytes(ix.data[4..12].try_into().unwrap());
                    if soltx::lookup_table_address(&authority, slot).0 != table || acc.contains_key(&table) {
                        return Err(err("create lookup table: not the derived address, or in use"));
                    }
                    let st = soltx::LookupTableState { deactivation_slot: u64::MAX, last_extended_slot: 0, last_extended_start: 0,
                        authority: Some(authority), addresses: vec![] };
                    acc.insert(table, Account { lamports: 1_000_000, owner: soltx::key(soltx::ADDRESS_LOOKUP_TABLE_PROGRAM), executable: false,
                        data: soltx::encode_lookup_table(&st) });
                }
                Some([2, 0, 0, 0]) => {
                    let a = acc.get_mut(&table).ok_or_else(|| err("extend lookup table: no table"))?;
                    let mut st = soltx::decode_lookup_table(&a.data).map_err(|e| err(&e))?;
                    if st.authority != Some(authority) {
                        return Err(err("extend lookup table: not its authority"));
                    }
                    let n = u64::from_le_bytes(ix.data[4..12].try_into().unwrap()) as usize;
                    st.last_extended_start = st.addresses.len() as u8;
                    st.addresses.extend(ix.data[12..12 + 32 * n].chunks(32).map(|c| <Pubkey>::try_from(c).unwrap()));
                    a.data = soltx::encode_lookup_table(&st);
                    a.lamports += 32 * n as u64 * 6_960;
                }
                _ => return Err(err("unsupported lookup table instruction")),
            }
        } else if ix.program == ata_prog {
            let (payer, ata, owner, mint) = (k(0)?, k(1)?, k(2)?, k(3)?);
            if !signer(0) {
                return Err(err("ATA: payer did not sign"));
            }
            // as the program: an existing token account is left alone (idempotent); a pre-funded system account at the address is
            // allocated over (the payer tops its rent up; audit B1-08); anything else is an error
            let held = match acc.get(&ata) {
                Some(a) if a.owner == tok => continue,
                Some(a) if a.owner == SYSTEM_PROGRAM && a.data.is_empty() => a.lamports,
                Some(_) => return Err(err("ATA: the address holds another account")),
                None => 0,
            };
            if ata != soltx::associated_token_address(&owner, &mint) {
                return Err(err("ATA: wrong address"));
            }
            if !acc.contains_key(&mint) {
                return Err(err("ATA: no such mint"));
            }
            let p = acc.get_mut(&payer).ok_or_else(|| err("ATA: payer missing"))?;
            p.lamports = p.lamports.checked_sub(RENT165.saturating_sub(held)).ok_or_else(|| err("ATA: the payer cannot pay the rent"))?;
            acc.insert(ata, Account { lamports: RENT165.max(held), owner: tok, executable: false, data: token_data(mint, owner, 0) });
        } else if ix.program == tok {
            let tag = *ix.data.first().ok_or_else(|| err("empty token instruction"))?;
            let amount = || -> Result<u64, Value> { Ok(u64::from_le_bytes(ix.data.get(1..9).ok_or_else(|| err("short"))?.try_into().unwrap())) };
            match tag {
                20 => {
                    // InitializeMint2 { decimals, mint_authority, freeze_authority }: the account must be the Token program's, 82 bytes, blank
                    let mint = k(0)?;
                    let decimals = *ix.data.get(1).ok_or_else(|| err("InitializeMint2: short"))?;
                    let auth = ix.data.get(2..34).ok_or_else(|| err("InitializeMint2: short"))?;
                    let m = acc.get_mut(&mint).ok_or_else(|| err("InitializeMint2: no account"))?;
                    if m.owner != tok || m.data.len() != 82 || m.data[45] != 0 {
                        return Err(err("InitializeMint2: not a blank mint account of the Token program"));
                    }
                    let mut d = mint_data(auth.try_into().unwrap(), 0);
                    d[44] = decimals;
                    match ix.data.get(34) {
                        Some(0) => {}
                        Some(1) => {
                            d[46..50].copy_from_slice(&1u32.to_le_bytes());
                            d[50..82].copy_from_slice(ix.data.get(35..67).ok_or_else(|| err("InitializeMint2: short"))?);
                        }
                        _ => return Err(err("InitializeMint2: a bad freeze authority option")),
                    }
                    m.data = d;
                }
                7 | 14 => {
                    let (mint, dest, auth) = (k(0)?, k(1)?, k(2)?);
                    let n = amount()?;
                    if !signer(2) {
                        return Err(err("MintTo: the authority did not sign"));
                    }
                    let m = acc.get_mut(&mint).ok_or_else(|| err("MintTo: no mint"))?;
                    if m.data[4..36] != auth || m.data[0] != 1 {
                        return Err(err("MintTo: not the mint authority"));
                    }
                    let s = supply_of(m).checked_add(n).ok_or_else(|| err("supply overflow"))?;
                    m.data[36..44].copy_from_slice(&s.to_le_bytes());
                    let d = acc.get_mut(&dest).ok_or_else(|| err("MintTo: no destination"))?;
                    if d.data[0..32] != mint {
                        return Err(err("MintTo: the account is of another mint"));
                    }
                    let a = amount_of(d) + n;
                    set_amount(d, a);
                }
                8 | 15 => {
                    let (account, mint, owner) = (k(0)?, k(1)?, k(2)?);
                    let n = amount()?;
                    if !signer(2) {
                        return Err(err("Burn: the owner did not sign"));
                    }
                    let a = acc.get_mut(&account).ok_or_else(|| err("Burn: no account"))?;
                    if a.data[32..64] != owner || a.data[0..32] != mint {
                        return Err(err("Burn: wrong owner or mint"));
                    }
                    let left = amount_of(a).checked_sub(n).ok_or_else(|| err("Burn: insufficient funds"))?;
                    set_amount(a, left);
                    let m = acc.get_mut(&mint).ok_or_else(|| err("Burn: no mint"))?;
                    let s = supply_of(m).checked_sub(n).ok_or_else(|| err("Burn: more than the supply"))?;
                    m.data[36..44].copy_from_slice(&s.to_le_bytes());
                }
                3 | 12 => {
                    // Transfer (source, destination, owner) and TransferChecked (source, mint, destination, owner)
                    let (src, dst, owner, checked_mint) = if tag == 3 { (k(0)?, k(1)?, k(2)?, None) } else { (k(0)?, k(2)?, k(3)?, Some(k(1)?)) };
                    let n = amount()?;
                    if !signer(if tag == 3 { 2 } else { 3 }) {
                        return Err(err("Transfer: the owner did not sign"));
                    }
                    if let Some(m) = checked_mint {
                        if acc.get(&src).is_some_and(|s| s.data[0..32] != m) {
                            return Err(err("TransferChecked: another mint"));
                        }
                    }
                    let s = acc.get_mut(&src).ok_or_else(|| err("Transfer: no source"))?;
                    if s.data[32..64] != owner {
                        return Err(err("Transfer: not the owner"));
                    }
                    let mint: [u8; 32] = s.data[0..32].try_into().unwrap();
                    let left = amount_of(s).checked_sub(n).ok_or_else(|| err("Transfer: insufficient funds"))?;
                    set_amount(s, left);
                    let d = acc.get_mut(&dst).ok_or_else(|| err("Transfer: no destination"))?;
                    if d.data[0..32] != mint {
                        return Err(err("Transfer: another mint"));
                    }
                    let a = amount_of(d) + n;
                    set_amount(d, a);
                }
                9 => {
                    let (account, dest, owner) = (k(0)?, k(1)?, k(2)?);
                    if !signer(2) {
                        return Err(err("CloseAccount: the owner did not sign"));
                    }
                    let a = acc.get(&account).ok_or_else(|| err("Close: no account"))?;
                    // SPL: a native (wSOL) account closes with its balance (all its lamports go to `dest`); any other only empty
                    let native = a.data[0..32] == etch_autobuyer::solix2::native_mint();
                    if a.data[32..64] != owner || (amount_of(a) != 0 && !native) {
                        return Err(err("Close: not the owner or not empty"));
                    }
                    let lamports = a.lamports;
                    acc.remove(&account);
                    let d = acc.entry(dest).or_insert(Account { lamports: 0, owner: SYSTEM_PROGRAM, executable: false, data: vec![] });
                    d.lamports += lamports;
                }
                17 => {
                    // SyncNative: the account's lamports above rent become its wSOL balance
                    let k0 = k(0)?;
                    let a = acc.get_mut(&k0).ok_or_else(|| err("SyncNative: no account"))?;
                    let n = a.lamports.saturating_sub(RENT165);
                    set_amount(a, n);
                }
                _ => return Err(err("unsupported token instruction")),
            }
        } else if ix.program == soltx::key(etch_autobuyer::solix2::dlmm::DLMM_PROGRAM) || ix.program == soltx::key(soltx::COMPUTE_BUDGET_PROGRAM)
            || ix.program == etch_autobuyer::solix2::dbc::program() || ix.program == etch_autobuyer::solix2::damm::program()
            || ix.program == etch_autobuyer::solix2::id(etch_autobuyer::solix2::METADATA_PROGRAM) {
            // DLMM's, DBC's, DAMM v2's and Token Metadata's effects are LiteSVM's to check (solana/tests/dlmm_fixtures.rs,
            // solana/tests/solix2_fixtures.rs); here the instruction lands as sent and the test plants what the program would have written
        } else {
            return Err(err("unsupported program"));
        }
    }
    // the rent rule, at the end of the transaction
    for key in touched {
        if let Some(a) = acc.get(&key) {
            let min = if a.data.is_empty() { RENT0 } else if a.data.len() == 165 { RENT165 } else { 0 };
            if a.lamports > 0 && a.lamports < min {
                return Err(json!("InsufficientFundsForRent"));
            }
            if a.lamports == 0 && a.owner == SYSTEM_PROGRAM {
                acc.remove(&key);
            }
        }
    }
    Ok(())
}

// ---- the world -------------------------------------------------------------------------------------------------------------------

pub struct World {
    pub cfg: SolHomeCfg,
    pub ps: Vec<FakeSol>,
    pub signer: RealSigner,
    pub lines: BTreeMap<LineId, SolLine>,
    pub chain: Rc<RefCell<SolChain>>,
    pub now: u64,
}

impl SolIo for World {
    fn sol_home(&self) -> Option<SolHomeCfg> {
        Some(self.cfg.clone())
    }
    async fn sol_me(&mut self) -> Result<Pubkey, String> {
        Ok(me())
    }
    async fn sol_accounts(&mut self, keys: &[Pubkey], max_data: usize) -> Result<Vec<Option<Account>>, String> {
        Ok(solrpc::accounts(&mut self.ps, keys, max_data).await?)
    }
    async fn sol_accounts_slice(&mut self, keys: &[Pubkey], offset: usize, len: usize) -> Result<Vec<Option<Account>>, String> {
        Ok(solrpc::accounts_slice(&mut self.ps, keys, offset, len).await?)
    }
    async fn sol_program_slices(&mut self, program: &Pubkey, size: usize, memcmps: &[(usize, Vec<u8>)], slice: (usize, usize), max: usize)
        -> Result<Vec<(Pubkey, Pubkey, Vec<u8>)>, String> {
        Ok(solrpc::program_slices(&mut self.ps, program, size, memcmps, slice, max).await?)
    }
    async fn sol_clock(&mut self) -> Result<(u64, u64), String> {
        Ok(solrpc::finalized_clock(&mut self.ps).await?)
    }
    async fn sol_key_of(&mut self, p: &KeyPath) -> Result<Pubkey, String> {
        Ok(pk(*p))
    }
    async fn sol_rent(&mut self, space: u64) -> Result<u64, String> {
        Ok(match space {
            0 => RENT0,
            165 => RENT165,
            n => solrpc::rent_exempt(&mut self.ps, n).await?,
        })
    }
    async fn sol_relay(&mut self) -> Result<Pubkey, String> {
        Ok(pk(KeyPath::Relay))
    }
    async fn sol_balance_of(&mut self, key: &Pubkey) -> Result<u128, String> {
        Ok(solrpc::balance(&mut self.ps, key).await? as u128)
    }
    async fn sol_balance(&mut self) -> Result<u128, String> {
        Ok(solrpc::balance(&mut self.ps, &me()).await? as u128)
    }
    /// W6: the coins' discovery (`getProgramAccounts` on DBC by config, sliced).
    /// W6: `K_fc`'s position NFTs (`getTokenAccountsByOwner`, sliced).
    async fn sol_token_accounts(&mut self, owner: &Pubkey, program: &Pubkey, slice: (usize, usize), max: usize) -> Result<Vec<(Pubkey, Account)>, String> {
        Ok(solrpc::token_accounts(&mut self.ps, owner, program, slice, max).await?)
    }
    async fn sol_prepare(&mut self, line: &mut Line, kind: LifetimeKind, intent: u64, what: &str, ixs: Vec<Instruction>) -> Result<(), String> {
        solsend::prepare(line, kind, intent, what, ixs, &mut self.ps, &mut self.signer).await
    }
    async fn sol_prepare_keys(&mut self, line: &mut Line, kind: LifetimeKind, intent: u64, what: &str, ixs: Vec<Instruction>, keys: Vec<KeyPath>)
        -> Result<(), String> {
        solsend::prepare_keys(line, kind, intent, what, ixs, keys, &mut self.ps, &mut self.signer).await
    }
    async fn sol_prepare_tables(&mut self, line: &mut Line, kind: LifetimeKind, intent: u64, what: &str, ixs: Vec<Instruction>,
        keys: Vec<KeyPath>, tables: Vec<soltx::LookupTable>) -> Result<(), String> {
        solsend::prepare_tables(line, kind, intent, what, ixs, keys, tables, &mut self.ps, &mut self.signer).await
    }
    async fn sol_tick(&mut self, line: &mut Line) -> Result<Step, String> {
        Ok(solsend::tick(line, &mut self.ps, &mut self.signer).await?)
    }
    /// The finalized signatures of `K_main` newer than `until`, newest first, each with the memo it carries (`[len] text`, as the RPC
    /// reports it).
    async fn sol_signatures(&mut self, until: &str, limit: usize) -> Result<Vec<SigInfo>, String> {
        let c = self.chain.borrow();
        let mut out = vec![];
        let mut seen = vec![];
        for tx in &c.sent {
            let (sigs, msg) = soltx::split_tx(tx).unwrap();
            let sig = soltx::b58_encode(&sigs[0]);
            if seen.contains(&sig) {
                continue;
            }
            seen.push(sig.clone());
            let Some(l) = c.landed.get(&sig) else { continue };
            if l.level != Level::Finalized {
                continue;
            }
            let p = soltx::Parts::parse(msg).unwrap();
            let memos: Vec<String> = c.instructions_of(&p).iter().filter(|i| i.program == soltx::key(etch_autobuyer::relaysol::MEMO_V2))
                .map(|i| { let t = String::from_utf8(i.data.clone()).unwrap(); format!("[{}] {t}", t.len()) }).collect();
            out.push(SigInfo { signature: sig, slot: l.slot, failed: l.err.is_some(), memo: (!memos.is_empty()).then(|| memos.join("; ")) });
        }
        out.reverse();
        let mut v = vec![];
        for s in out {
            if s.signature == until {
                break;
            }
            v.push(s);
        }
        v.truncate(limit);
        Ok(v)
    }
    async fn sol_transaction(&mut self, sig: &str) -> Result<Option<Value>, String> {
        let c = self.chain.borrow();
        Ok(c.landed.get(sig).filter(|l| l.level == Level::Finalized).map(|l| json!({ "meta": { "err": l.err } })))
    }
    fn sol_line(&mut self) -> SolLine {
        self.sol_line_of(LineId::Money)
    }
    fn sol_line_save(&mut self, l: &SolLine) {
        self.sol_line_save_of(LineId::Money, l)
    }
    fn sol_line_of(&mut self, id: LineId) -> SolLine {
        self.lines.get(&id).cloned().unwrap_or_default()
    }
    fn sol_line_save_of(&mut self, id: LineId, l: &SolLine) {
        self.lines.insert(id, l.clone());
    }
}

impl DeskIo for World {
    async fn key_of(&mut self, p: &KeyPath) -> Result<Pubkey, String> {
        Ok(pk(*p))
    }
}

pub fn nonce_line(c: &Rc<RefCell<SolChain>>, id: LineId) -> Line {
    let n = solsend::nonce_account_of(&me(), &solhome::line_seed(SEED, id)).unwrap();
    c.borrow_mut().set_nonce(n, me());
    Line { nonce_account: Some(n), ..Default::default() }
}

impl World {
    /// A world with one collection (id 0, wX mint [`WX`], floor [`FLOOR`] posted now), all four lines open, a funded `K_main`, an empty
    /// desk state. The state is the thread's own (each test runs on its own thread).
    pub fn new() -> World {
        World::with_floor(FLOOR)
    }

    /// A world whose collection's accepted floor is `floor` (lamports a whole wX) from the start.
    pub fn with_floor(floor: u64) -> World {
        state::reset();
        etch_autobuyer::pooldesk::state::reset();
        etch_autobuyer::coins::state::reset();
        let chain = SolChain::new();
        {
            let mut c = chain.borrow_mut();
            c.set_account(me(), SYSTEM_PROGRAM, 10 * SOL, vec![]);
            c.set_account(module(), SYSTEM_PROGRAM, RENT0, vec![]);
            c.set_account(WX, token_program(), 1_461_600, mint_data(module(), 0));
            c.advance(25, &[]);
            c.exec = Some(Box::new(|ixs, payer, acc, _| {
                let _ = payer;
                run_ixs(ixs, acc)
            }));
            c.sim = Some(Box::new(|ixs, _, acc, _| run_ixs(ixs, &mut acc.clone())));
        }
        let cfg = SolHomeCfg { lifetime: LifetimeKind::DurableNonce, nonce_seed: SEED.into() };
        let now = chain.borrow().unix;
        let mut w = World { cfg, ps: pair(&chain), signer: RealSigner::new(), lines: BTreeMap::new(), chain, now };
        for id in LineId::ALL {
            let l = nonce_line(&w.chain, id);
            w.lines.insert(id, SolLine { line: l, ..Default::default() });
        }
        state::with_mut(|s| {
            s.books.add_collection(COLL, ListingKind::EthNft { chain_id: 1, collection: [0x5a; 20], custody: [0x11; 20] }, WX,
                books::default_params(20 * SOL)).unwrap();
            s.books.post_floor(COLL, floor, 9_000, 20, now as i64).unwrap();
        });
        w
    }

    /// One pass of the coins (W6).
    pub fn coins_tick(&mut self) -> Vec<String> {
        let now = self.now;
        block_on(etch_autobuyer::coins::run::tick(self, now))
    }

    /// One pass of the pool desk (W7).
    pub fn pool_tick(&mut self) -> Vec<String> {
        let now = self.now;
        block_on(etch_autobuyer::pooldesk::run::tick(self, now))
    }

    /// The pool desk acts, the chain lands it, the lines and the desk settle it.
    pub fn pool_round(&mut self) -> Vec<String> {
        let mut log = self.pool_tick();
        self.settle();
        log.extend(self.pool_tick());
        log
    }

    /// Plants an account as the chain would show it.
    pub fn plant(&mut self, key: Pubkey, owner: Pubkey, lamports: u64, data: Vec<u8>) {
        self.chain.borrow_mut().set_account(key, owner, lamports, data);
    }

    pub fn tick(&mut self) -> Vec<String> {
        let now = self.now;
        block_on(etch_autobuyer::desk::run::tick(self, now))
    }

    /// The chain lands and finalizes what was sent; the four lines see it (one pass).
    pub fn settle(&mut self) {
        self.chain.borrow_mut().advance(40, &[]);
        self.chain.borrow_mut().process(Level::Finalized);
        let now = self.now;
        let _ = block_on(solhome::tick_lines(self, None, now));
    }

    /// One full round: the desk acts, the chain lands it, the lines and the desk settle it.
    pub fn round(&mut self) -> Vec<String> {
        let mut log = self.tick();
        self.settle();
        log.extend(self.tick());
        log
    }

    /// `n` rounds.
    pub fn rounds(&mut self, n: usize) {
        for _ in 0..n {
            self.round();
        }
    }

    pub fn advance_time(&mut self, secs: u64) {
        self.now += secs;
        self.chain.borrow_mut().unix += secs;
        self.chain.borrow_mut().advance(secs.min(5), &[]);
    }

    /// Re-post the floor now (the poster).
    pub fn post_floor(&mut self) {
        let now = self.now as i64;
        state::with_mut(|s| s.books.post_floor(COLL, FLOOR, 9_000, 20, now).unwrap());
    }

    pub fn open(&mut self, caller: u8, args: etch_autobuyer::desk::OpenKind) -> Result<etch_autobuyer::desk::api::Opened, String> {
        let now = self.now;
        block_on(etch_autobuyer::desk::api::open(self, vec![caller], args, now))
    }

    pub fn lamports(&self, k: &Pubkey) -> u64 {
        self.chain.borrow().accounts.get(k).map(|a| a.lamports).unwrap_or(0)
    }
    pub fn token(&self, owner: &Pubkey) -> u64 {
        let ata = soltx::associated_token_address(owner, &WX);
        self.chain.borrow().accounts.get(&ata).map(amount_of).unwrap_or(0)
    }
    pub fn supply(&self) -> u64 {
        supply_of(&self.chain.borrow().accounts[&WX])
    }
    pub fn deposit_sol(&mut self, to: &Pubkey, lamports: u64) {
        let mut c = self.chain.borrow_mut();
        let a = c.accounts.entry(*to).or_insert(Account { lamports: 0, owner: SYSTEM_PROGRAM, executable: false, data: vec![] });
        a.lamports += lamports;
    }
    /// A user sends wX to an intent's token account (creating it, as the site does): the wX come from nowhere in the fake (a holder's).
    pub fn deposit_wx(&mut self, owner: &Pubkey, units: u64) {
        let ata = soltx::associated_token_address(owner, &WX);
        let mut c = self.chain.borrow_mut();
        if let Some(a) = c.accounts.get_mut(&ata) {
            let n = amount_of(a) + units;
            set_amount(a, n);
        } else {
            c.accounts.insert(ata, Account { lamports: RENT165, owner: token_program(), executable: false, data: token_data(WX, *owner, units) });
        }
    }
    /// The supply the holder's wX came from (the books' mints that happened outside the desk): added to the chain's supply.
    pub fn inject_supply(&mut self, units: i64) {
        let mut c = self.chain.borrow_mut();
        let m = c.accounts.get_mut(&WX).unwrap();
        let s = (supply_of(m) as i64 + units) as u64;
        m.data[36..44].copy_from_slice(&s.to_le_bytes());
    }
    pub fn books(&self) -> Books {
        state::with(|s| s.books.clone())
    }
    pub fn intent(&self, id: u64) -> etch_autobuyer::desk::intent::Intent {
        state::with(|s| s.intents[&id].clone())
    }
    pub fn line(&self, id: LineId) -> SolLine {
        self.lines.get(&id).cloned().unwrap_or_default()
    }
}
