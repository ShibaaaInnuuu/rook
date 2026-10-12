//! System `CreateAccount`, SPL Token (classic `Tokenkeg…`), the Associated Token Account program and Memo v2, laid out as the
//! programs unpack them (`spl_token::instruction::TokenInstruction::pack`: a one-byte tag, little-endian integers, a
//! `COption<Pubkey>` as one byte 0, or 1 + the key).
use super::{id, ix, ro, rw, signer, MEMO_PROGRAM};
use crate::soltx::{self, Instruction, Pubkey, SYSTEM_PROGRAM};

/// The size of a classic SPL mint and token account.
pub const MINT_LEN: u64 = 82;
pub const ACCOUNT_LEN: u64 = 165;

/// SPL Token instruction tags.
pub mod tag {
    pub const TRANSFER: u8 = 3;
    pub const APPROVE: u8 = 4;
    pub const SET_AUTHORITY: u8 = 6;
    pub const MINT_TO: u8 = 7;
    pub const BURN: u8 = 8;
    pub const CLOSE_ACCOUNT: u8 = 9;
    pub const FREEZE_ACCOUNT: u8 = 10;
    pub const TRANSFER_CHECKED: u8 = 12;
    pub const APPROVE_CHECKED: u8 = 13;
    pub const MINT_TO_CHECKED: u8 = 14;
    pub const BURN_CHECKED: u8 = 15;
    pub const SYNC_NATIVE: u8 = 17;
    pub const INITIALIZE_MINT2: u8 = 20;
}

/// `spl_token::instruction::AuthorityType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthorityType {
    MintTokens = 0,
    FreezeAccount = 1,
    AccountOwner = 2,
    CloseAccount = 3,
}

fn token(accounts: Vec<crate::soltx::AccountMeta>, data: Vec<u8>) -> Instruction {
    ix(super::token_program(), accounts, data)
}
fn with_amount(tag: u8, amount: u64) -> Vec<u8> {
    let mut d = vec![tag];
    d.extend(amount.to_le_bytes());
    d
}
fn coption_key(d: &mut Vec<u8>, k: Option<Pubkey>) {
    match k {
        Some(k) => {
            d.push(1);
            d.extend(k);
        }
        None => d.push(0),
    }
}

/// `SystemInstruction::CreateAccount` (index 0): `from` pays, `new` signs.
pub fn create_account(from: Pubkey, new: Pubkey, lamports: u64, space: u64, owner: Pubkey) -> Instruction {
    let mut d = 0u32.to_le_bytes().to_vec();
    d.extend(lamports.to_le_bytes());
    d.extend(space.to_le_bytes());
    d.extend(owner);
    ix(SYSTEM_PROGRAM, vec![signer(from, true), signer(new, true)], d)
}

/// `InitializeMint2 { decimals, mint_authority, freeze_authority }` (no rent sysvar).
pub fn initialize_mint2(mint: Pubkey, mint_authority: Pubkey, freeze_authority: Option<Pubkey>, decimals: u8) -> Instruction {
    let mut d = vec![tag::INITIALIZE_MINT2, decimals];
    d.extend(mint_authority);
    coption_key(&mut d, freeze_authority);
    token(vec![rw(mint)], d)
}

pub fn mint_to(mint: Pubkey, dest: Pubkey, authority: Pubkey, amount: u64) -> Instruction {
    token(vec![rw(mint), rw(dest), signer(authority, false)], with_amount(tag::MINT_TO, amount))
}
pub fn mint_to_checked(mint: Pubkey, dest: Pubkey, authority: Pubkey, amount: u64, decimals: u8) -> Instruction {
    let mut d = with_amount(tag::MINT_TO_CHECKED, amount);
    d.push(decimals);
    token(vec![rw(mint), rw(dest), signer(authority, false)], d)
}
pub fn burn(account: Pubkey, mint: Pubkey, owner: Pubkey, amount: u64) -> Instruction {
    token(vec![rw(account), rw(mint), signer(owner, false)], with_amount(tag::BURN, amount))
}
pub fn burn_checked(account: Pubkey, mint: Pubkey, owner: Pubkey, amount: u64, decimals: u8) -> Instruction {
    let mut d = with_amount(tag::BURN_CHECKED, amount);
    d.push(decimals);
    token(vec![rw(account), rw(mint), signer(owner, false)], d)
}
pub fn transfer(source: Pubkey, dest: Pubkey, owner: Pubkey, amount: u64) -> Instruction {
    token(vec![rw(source), rw(dest), signer(owner, false)], with_amount(tag::TRANSFER, amount))
}
pub fn transfer_checked(source: Pubkey, mint: Pubkey, dest: Pubkey, owner: Pubkey, amount: u64, decimals: u8) -> Instruction {
    let mut d = with_amount(tag::TRANSFER_CHECKED, amount);
    d.push(decimals);
    token(vec![rw(source), ro(mint), rw(dest), signer(owner, false)], d)
}
/// `CloseAccount`: the account's lamports (its rent; for wSOL also the wrapped SOL) go to `dest`.
pub fn close_account(account: Pubkey, dest: Pubkey, owner: Pubkey) -> Instruction {
    token(vec![rw(account), rw(dest), signer(owner, false)], vec![tag::CLOSE_ACCOUNT])
}
/// `SyncNative`: a wSOL account's token amount catches up with its lamports.
pub fn sync_native(account: Pubkey) -> Instruction {
    token(vec![rw(account)], vec![tag::SYNC_NATIVE])
}
/// `SetAuthority`. Built for the ceremony (the hot key hands `MintTokens` to `K_coll[i]`, §2) and for tests: the canister's
/// [`super::guard::refuse_forbidden`] refuses to sign any transaction that carries it.
pub fn set_authority(account: Pubkey, kind: AuthorityType, new_authority: Option<Pubkey>, current: Pubkey) -> Instruction {
    let mut d = vec![tag::SET_AUTHORITY, kind as u8];
    coption_key(&mut d, new_authority);
    token(vec![rw(account), signer(current, false)], d)
}

fn ata(payer: Pubkey, owner: Pubkey, mint: Pubkey, data: Vec<u8>) -> Instruction {
    ix(id(soltx::ASSOCIATED_TOKEN_PROGRAM), vec![signer(payer, true), rw(soltx::associated_token_address(&owner, &mint)), ro(owner), ro(mint),
        ro(SYSTEM_PROGRAM), ro(super::token_program())], data)
}
/// The Associated Token Account program's `Create` (fails if the account exists).
pub fn create_ata(payer: Pubkey, owner: Pubkey, mint: Pubkey) -> Instruction {
    ata(payer, owner, mint, vec![0])
}
/// `CreateIdempotent`: a no-op when the account already exists with that owner.
pub fn create_ata_idempotent(payer: Pubkey, owner: Pubkey, mint: Pubkey) -> Instruction {
    ata(payer, owner, mint, vec![1])
}

/// Memo v2: the text, and each of `signers` must have signed the transaction (the memo program checks it).
pub fn memo(text: &str, signers: &[Pubkey]) -> Instruction {
    ix(id(MEMO_PROGRAM), signers.iter().map(|k| signer(*k, false)).collect(), text.as_bytes().to_vec())
}
