use anchor_lang::prelude::*;

/// rook_core's program id (the floor module's `convert`, the treasury). Local development key; the mainnet id is set at the
/// ceremony together with `rook_core`'s own `declare_id!`.
pub const ROOK_CORE_ID: Pubkey = pubkey!("GepazuDZfS6zjwQ3U2zHCgk93xpRkp34AkmEFZKrAd7T");
/// Meteora Dynamic Bonding Curve (mainnet, release 0.2.1, deployed 2026-09-09 slot 445503633).
pub const DBC_ID: Pubkey = pubkey!("dbcij3LWUppWqq96dh6gJWwBifmcGfLSB5D4DuSMaqN");
/// DBC's `const_pda::pool_authority` (PDA ["pool_authority"] of DBC).
pub const DBC_POOL_AUTHORITY: Pubkey = pubkey!("FhVo3mqL8PW5pH5U2CN4XE33DokiyZnUwuGpH2hmHLuM");
/// Meteora DAMM v2 (cp_amm, release 0.2.4, deployed 2026-09-08 slot 445230614).
pub const DAMM_V2_ID: Pubkey = pubkey!("cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG");
/// DAMM v2's `const_pda::pool_authority`.
pub const DAMM_V2_POOL_AUTHORITY: Pubkey = pubkey!("HLnpSz9h2S4hiLQ43rnSD9XkcUThA7B8hQMKmDaiTLcC");
/// Wrapped SOL: the only quote mint a Rook config may use.
pub const NATIVE_MINT: Pubkey = pubkey!("So11111111111111111111111111111111111111112");

#[constant]
pub const SETTINGS_SEED: &[u8] = b"settings";
#[constant]
pub const PARTNER_SEED: &[u8] = b"partner";
/// The DBC `fee_claimer` of every config of a collection: a data-less, system-owned PDA ["claimer", listing].
#[constant]
pub const CLAIMER_SEED: &[u8] = b"claimer";
/// The ONE program-wide signer of rook_core `convert`: a data-less PDA ["converter"] of this program, set in rook_core's
/// `Registry.converters` at `init_registry` (no `CHANGE_DELAY`). `back` signs with it; the collection's claimer pays.
#[constant]
pub const CONVERTER_SEED: &[u8] = b"converter";
/// A transient wSOL account ["claimer_wsol", listing] (authority: the claimer), created and closed inside one instruction.
#[constant]
pub const CLAIMER_WSOL_SEED: &[u8] = b"claimer_wsol";
#[constant]
pub const VAULT_SEED: &[u8] = b"vault";
#[constant]
pub const VAULT_WX_SEED: &[u8] = b"vault_wx";
/// rook_core's treasury PDA ["treasury"]: the only destination of swept partner fees.
pub const ROOK_CORE_TREASURY_SEED: &[u8] = b"treasury";

/// DBC configs a collection may register (one per curve shape / fee schedule).
pub const MAX_CONFIGS: usize = 8;

/// `back`'s price bound (audit A3-06, A1-08): the wX units a conversion must yield are at least `(1 − TOL)` of what the
/// lamports buy at the module's ACCEPTED floor plus the conversion allowance, `accepted × (1 + conv_bps)`, without the
/// target premium rook_core's `convert` may add (`max(price, upTarget)`). A cranker's lower `min_units` is ignored.
///
/// 10 %: `fill_band_bps` is at most 10 % (rook_core `validate_params`), so the fill TWAP alone (clamped to `accepted × (1 ±
/// band)`) never trips the bound; it trips only when the keeper's raw target runs more than about 11 % ahead of the accepted
/// floor (the day band allows 20 %), the window a cranker could otherwise pick to mint the coin's holders up to ~24 % fewer
/// units. A refused `back` leaves the fee in DBC and succeeds after the next posts bring `accepted` up.
pub const BACK_PRICE_TOLERANCE_BPS: u64 = 1_000;
pub const BPS: u64 = 10_000;
/// One whole wX unit (9 decimals).
pub const WX_ONE: u64 = 1_000_000_000;

/// rook_core `convert` discriminator: sha256("global:convert")[..8].
pub const CONVERT_DISCRIMINATOR: [u8; 8] = [122, 80, 212, 208, 92, 200, 34, 161];
