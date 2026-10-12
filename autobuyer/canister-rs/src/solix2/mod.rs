//! W2 (docs/v9/canister-only-solana.md §2, §3.3, §5, §13.1): the canister's own Solana instruction builders and account decoders
//! for canister-only Rook. No SDK: every instruction is laid out by hand on [`soltx`]'s primitives (compact-u16 and the message
//! compiler, `find_program_address`, the associated-token address, Anchor discriminators and Borsh).
//!
//! - [`spl`]: System `CreateAccount`; SPL Token `InitializeMint2`, `MintTo(Checked)`, `Burn(Checked)`, `Transfer(Checked)`,
//!   `CloseAccount`, `SyncNative`, `SetAuthority` (built for the ceremony; the canister's [`guard`] refuses to sign it); the
//!   Associated Token Account program's `Create` / `CreateIdempotent`; Memo v2.
//! - [`mpl`]: Metaplex Token Metadata `CreateMetadataAccountV3` and `UpdateMetadataAccountV2` (rename only).
//! - [`dbc`]: Meteora Dynamic Bonding Curve 0.2.1: `create_config` (Rook's pinned parameters, [`dbc::rook_wx_config`]),
//!   `initialize_virtual_pool_with_spl_token`, `swap`, `swap2`, `claim_trading_fee`, `withdraw_migration_fee`,
//!   `partner_withdraw_surplus`, `withdraw_leftover`, `migration_damm_v2`.
//! - [`damm`]: Meteora DAMM v2 (cp_amm) 0.2.4: `initialize_customizable_pool` (a concentrated range), `create_position`,
//!   `add_liquidity`, `remove_liquidity`, `remove_all_liquidity`, `claim_position_fee`, `close_position`, `swap`, `swap2`.
//! - [`decode`]: the accounts the canister reads: SPL `Mint` and token `Account`, DBC `PoolConfig` and `VirtualPool`, DAMM v2
//!   `Pool` and `Position`.
//! - [`guard`]: what DBC does not check and Rook must (spikes S1–S3 §2.2): the wX mint (freeze authority none, mint authority =
//!   the canister key), the config (quote, claimer, threshold bound to the curve), and the instructions the canister never signs.
//! - [`route`]: the transactions as the spikes sized them: the desk's pool opening trimmed to fit a durable nonce, and the site's
//!   own two-hop buy SOL → wX → coin (required: it works with or without Jupiter).
//!
//! Every builder is checked byte for byte against instructions that landed in LiteSVM on the mainnet program bytes and were
//! built by spl-token and by the Anchor clients generated from the official IDLs (`tests/fixtures/solix2/flow.json`, written by
//! `solana/tests/solix2_fixtures.rs`; checked by `tests/solix2_builders.rs`). Pure: no network, no keys, no clock.
use crate::soltx::{self, find_program_address, AccountMeta, Instruction, Pubkey};

pub mod damm;
pub mod dbc;
pub mod dlmm;
pub mod decode;
pub mod guard;
pub mod mpl;
pub mod route;
pub mod spl;

pub const TOKEN_2022_PROGRAM: &str = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb";
pub const MEMO_PROGRAM: &str = "MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr";
pub const METADATA_PROGRAM: &str = "metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s";
pub const DBC_PROGRAM: &str = "dbcij3LWUppWqq96dh6gJWwBifmcGfLSB5D4DuSMaqN";
pub const DBC_POOL_AUTHORITY: &str = "FhVo3mqL8PW5pH5U2CN4XE33DokiyZnUwuGpH2hmHLuM";
pub const DAMM_PROGRAM: &str = "cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG";
pub const DAMM_POOL_AUTHORITY: &str = "HLnpSz9h2S4hiLQ43rnSD9XkcUThA7B8hQMKmDaiTLcC";
/// The DAMM v2 config DBC migrates into with `migration_fee_option` 6 (Customizable).
pub const DAMM_CONFIG_CUSTOMIZABLE: &str = "A8gMrEPJkacWkcb3DGwtJwTe16HktSEfvwtuDh2MCtck";
pub const NATIVE_MINT: &str = "So11111111111111111111111111111111111111112";

pub fn id(s: &str) -> Pubkey {
    soltx::key(s)
}
pub fn token_program() -> Pubkey {
    id(soltx::TOKEN_PROGRAM)
}
pub fn native_mint() -> Pubkey {
    id(NATIVE_MINT)
}

pub(crate) fn rw(k: Pubkey) -> AccountMeta {
    soltx::meta(k, false, true)
}
pub(crate) fn ro(k: Pubkey) -> AccountMeta {
    soltx::meta(k, false, false)
}
pub(crate) fn signer(k: Pubkey, writable: bool) -> AccountMeta {
    soltx::meta(k, true, writable)
}
pub(crate) fn ix(program: Pubkey, accounts: Vec<AccountMeta>, data: Vec<u8>) -> Instruction {
    Instruction { program, accounts, data }
}

fn pda(seeds: &[&[u8]], program: &Pubkey) -> Pubkey {
    find_program_address(seeds, program).expect("a program address exists").0
}
/// Anchor's `["__event_authority"]` of a program (every Meteora instruction that emits a CPI event names it).
pub fn event_authority(program: &Pubkey) -> Pubkey {
    pda(&[b"__event_authority"], program)
}
/// The larger key first: the order DBC and DAMM v2 seed a pair with (`max_key`, `min_key`), byte-wise as `Pubkey`'s `Ord`.
pub fn ordered(a: &Pubkey, b: &Pubkey) -> (Pubkey, Pubkey) {
    if a > b {
        (*a, *b)
    } else {
        (*b, *a)
    }
}
/// Token Metadata's `["metadata", program, mint]`.
pub fn metadata_address(mint: &Pubkey) -> Pubkey {
    let p = id(METADATA_PROGRAM);
    pda(&[b"metadata", &p, mint], &p)
}
/// DBC `["pool", config, max(base, quote), min(base, quote)]`.
pub fn dbc_pool_address(config: &Pubkey, base: &Pubkey, quote: &Pubkey) -> Pubkey {
    let (hi, lo) = ordered(base, quote);
    pda(&[b"pool", config, &hi, &lo], &id(DBC_PROGRAM))
}
/// DBC `["token_vault", mint, pool]`.
pub fn dbc_vault_address(mint: &Pubkey, pool: &Pubkey) -> Pubkey {
    pda(&[b"token_vault", mint, pool], &id(DBC_PROGRAM))
}
/// DAMM v2 `["cpool", max, min]`: the one customizable pool of a pair (anyone holding both tokens can create it first, §5.1).
pub fn damm_customizable_pool_address(a: &Pubkey, b: &Pubkey) -> Pubkey {
    let (hi, lo) = ordered(a, b);
    pda(&[b"cpool", &hi, &lo], &id(DAMM_PROGRAM))
}
/// DAMM v2 `["pool", config, max, min]`: a pool on a config (the migrated COIN/wX pool, on [`DAMM_CONFIG_CUSTOMIZABLE`]).
pub fn damm_config_pool_address(config: &Pubkey, a: &Pubkey, b: &Pubkey) -> Pubkey {
    let (hi, lo) = ordered(a, b);
    pda(&[b"pool", config, &hi, &lo], &id(DAMM_PROGRAM))
}
pub fn damm_vault_address(mint: &Pubkey, pool: &Pubkey) -> Pubkey {
    pda(&[b"token_vault", mint, pool], &id(DAMM_PROGRAM))
}
pub fn damm_position_address(nft_mint: &Pubkey) -> Pubkey {
    pda(&[b"position", nft_mint], &id(DAMM_PROGRAM))
}
pub fn damm_position_nft_account(nft_mint: &Pubkey) -> Pubkey {
    pda(&[b"position_nft_account", nft_mint], &id(DAMM_PROGRAM))
}
