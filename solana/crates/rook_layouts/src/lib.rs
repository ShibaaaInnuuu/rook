//! The layout contract between rook_core and rook_backing: the bytes one program reads of an account the other owns.
//!
//! Both programs depend on this crate, and each OWNING program has a test that serializes its real Anchor struct and asserts
//! these readers return every field (rook_backing: `state::tests`, rook_core: `state::registry::tests` for the registry,
//! `layouts_tests` for the listing, module, module vault and wrapper; rook_core's wake suite also builds its test vaults from
//! `rook_backing::Vault` itself). A field added, removed or reordered in either struct
//! fails CI instead of silently shifting an offset.
//!
//! Anchor accounts are an 8-byte discriminator (sha256("account:<Name>")[..8]) followed by the Borsh encoding of the struct:
//! `Pubkey` 32 bytes, `bool`/`u8` 1, `u64`/`i64` 8 little-endian, no padding. Keys are plain `[u8; 32]` here so the crate has
//! no dependencies.
#![no_std]

fn key(d: &[u8], at: usize) -> [u8; 32] {
    let mut k = [0u8; 32];
    k.copy_from_slice(&d[at..at + 32]);
    k
}

fn u16_at(d: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([d[at], d[at + 1]])
}

fn u64_at(d: &[u8], at: usize) -> u64 {
    let mut b = [0u8; 8];
    b.copy_from_slice(&d[at..at + 8]);
    u64::from_le_bytes(b)
}

fn i64_at(d: &[u8], at: usize) -> i64 {
    let mut b = [0u8; 8];
    b.copy_from_slice(&d[at..at + 8]);
    i64::from_le_bytes(b)
}

/// rook_backing's per-coin `Vault` (PDA `["vault", coin_mint]` of rook_backing), read by rook_core's wake `reclaim`.
pub mod backing_vault {
    use super::*;

    /// sha256("account:Vault")[..8].
    pub const DISCRIMINATOR: [u8; 8] = [211, 8, 232, 43, 2, 152, 117, 119];
    pub const PARTNER: usize = 8;
    pub const COIN_MINT: usize = 40;
    pub const DBC_POOL: usize = 72;
    pub const DBC_CONFIG: usize = 104;
    pub const CREATOR: usize = 136;
    pub const WX_ACCOUNT: usize = 168;
    pub const GRADUATED: usize = 200;
    pub const LAMPORTS_BACKED: usize = 201;
    pub const WX_UNITS: usize = 209;
    pub const COIN_SUPPLY_AT_BACKING: usize = 217;
    pub const BACKED_AT: usize = 225;
    pub const LAUNCHED_AT: usize = 233;
    pub const COINS_BURNED: usize = 241;
    pub const WX_PAID: usize = 249;
    pub const BUMP: usize = 257;
    pub const WX_BUMP: usize = 258;
    /// The whole account: discriminator + struct.
    pub const LEN: usize = 259;

    /// What rook_core reads of a vault.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct View {
        /// rook_backing's `Partner` PDA `["partner", listing]`: names the collection.
        pub partner: [u8; 32],
        pub coin_mint: [u8; 32],
        /// The holder of the DBC pool's (transferable) creator role when the coin was backed (audit A3-01), not necessarily
        /// who launched it.
        pub creator: [u8; 32],
        /// Set by `back` once the coin migrated and its migration fee was converted.
        pub graduated: bool,
        pub backed_at: i64,
        /// The DBC pool's own activation point (unix seconds) = its creation time, copied at `open_vault`.
        pub launched_at: i64,
    }

    /// `None` unless the data is exactly a vault (length and discriminator). The caller checks the account's owner.
    pub fn parse(d: &[u8]) -> Option<View> {
        if d.len() != LEN || d[..8] != DISCRIMINATOR {
            return None;
        }
        Some(View {
            partner: key(d, PARTNER),
            coin_mint: key(d, COIN_MINT),
            creator: key(d, CREATOR),
            graduated: d[GRADUATED] != 0,
            backed_at: i64_at(d, BACKED_AT),
            launched_at: i64_at(d, LAUNCHED_AT),
        })
    }
}

/// rook_core's `Registry` (PDA `["registry"]` of rook_core), read by rook_backing's admin instructions.
pub mod registry {
    use super::*;

    pub const SEED: &[u8] = b"registry";
    /// sha256("account:Registry")[..8].
    pub const DISCRIMINATOR: [u8; 8] = [47, 174, 110, 246, 184, 182, 252, 218];
    /// `owner: Pubkey`, the struct's first field.
    pub const OWNER: usize = 8;

    /// The registry owner, `None` unless the data starts with the discriminator and holds the field. The caller checks the
    /// account's owner program and address.
    pub fn owner(d: &[u8]) -> Option<[u8; 32]> {
        if d.len() < OWNER + 32 || d[..8] != DISCRIMINATOR {
            return None;
        }
        Some(key(d, OWNER))
    }
}

/// rook_core's PDA seeds rook_backing derives (`rook_core::seeds`; rook_core's `layouts_tests` checks them).
pub mod core_seeds {
    pub const LISTING: &[u8] = b"listing";
    pub const MODULE: &[u8] = b"module";
    pub const MODULE_VAULT: &[u8] = b"module_vault";
    pub const WRAPPER: &[u8] = b"wrapper";
    pub const WX_MINT: &[u8] = b"wx_mint";
}

/// rook_core's `Listing` (PDA `["listing", id]`), read by rook_backing's `register_partner` (audit A3-03).
pub mod listing {
    /// sha256("account:Listing")[..8].
    pub const DISCRIMINATOR: [u8; 8] = [218, 32, 50, 73, 43, 134, 26, 58];
    /// `id: u32`, the struct's first field.
    pub const ID: usize = 8;

    /// The listing id, `None` unless the data starts with the discriminator and holds the field. The caller checks the
    /// account's owner program.
    pub fn id(d: &[u8]) -> Option<u32> {
        if d.len() < ID + 4 || d[..8] != DISCRIMINATOR {
            return None;
        }
        Some(u32::from_le_bytes([d[ID], d[ID + 1], d[ID + 2], d[ID + 3]]))
    }
}

/// rook_core's `ModuleVault` (PDA `["module_vault", module]`, no fields).
pub mod module_vault {
    /// sha256("account:ModuleVault")[..8].
    pub const DISCRIMINATOR: [u8; 8] = [30, 11, 184, 217, 138, 194, 117, 119];

    pub fn is(d: &[u8]) -> bool {
        d.len() >= 8 && d[..8] == DISCRIMINATOR
    }
}

/// rook_core's `Wrapper` (PDA `["wrapper", listing]`), read by rook_backing's `register_partner` (audit A3-03).
pub mod wrapper {
    use super::*;

    /// sha256("account:Wrapper")[..8].
    pub const DISCRIMINATOR: [u8; 8] = [161, 11, 109, 119, 86, 61, 163, 136];
    pub const LISTING: usize = 8;
    pub const WX_MINT: usize = 40;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct View {
        pub listing: [u8; 32],
        pub wx_mint: [u8; 32],
    }

    /// `None` unless the data starts with the discriminator and holds the fields. The caller checks the owner program.
    pub fn parse(d: &[u8]) -> Option<View> {
        if d.len() < WX_MINT + 32 || d[..8] != DISCRIMINATOR {
            return None;
        }
        Some(View { listing: key(d, LISTING), wx_mint: key(d, WX_MINT) })
    }
}

/// rook_core's floor `Module` (PDA `["module", listing]`), read by rook_backing: `register_partner` checks the accounts it
/// pins (audit A3-03) and `back` bounds the conversion price by the accepted floor (audit A3-06).
pub mod module {
    use super::*;

    /// sha256("account:Module")[..8].
    pub const DISCRIMINATOR: [u8; 8] = [234, 149, 112, 29, 65, 203, 69, 160];
    pub const LISTING: usize = 8;
    pub const WRAPPER: usize = 40;
    pub const VAULT: usize = 72;
    /// `params` starts at 139 (after four keys and three bumps); `conv_bps` is its seventh `u16`.
    pub const CONV_BPS: usize = 151;
    /// `accepted: u64`, the first field after `params` (14 × u16 + 5 × u64 = 68 bytes).
    pub const ACCEPTED: usize = 207;
    /// The whole account: discriminator + struct.
    pub const LEN: usize = 957;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct View {
        pub listing: [u8; 32],
        pub wrapper: [u8; 32],
        pub vault: [u8; 32],
        pub conv_bps: u16,
        /// The accepted floor: lamports per whole unit, after the move limit.
        pub accepted: u64,
    }

    /// `None` unless the data is exactly a module (length and discriminator). The caller checks the owner program.
    pub fn parse(d: &[u8]) -> Option<View> {
        if d.len() != LEN || d[..8] != DISCRIMINATOR {
            return None;
        }
        Some(View {
            listing: key(d, LISTING),
            wrapper: key(d, WRAPPER),
            vault: key(d, VAULT),
            conv_bps: u16_at(d, CONV_BPS),
            accepted: u64_at(d, ACCEPTED),
        })
    }
}
