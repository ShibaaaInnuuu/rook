//! The wake desk (src/floor/WakeDesk.sol). Bonds are held by each `Request` account itself (on top of its rent), so the
//! desk holds only bonds, each request its own.

use anchor_lang::prelude::*;

use crate::clock::DAY;

pub const MIN_WINDOW: i64 = DAY;
pub const MAX_WINDOW: i64 = 30 * DAY;
/// A reclaim may land this long after the window, before anyone may forfeit.
pub const RECLAIM_GRACE: i64 = DAY;

/// PDA `["desk"]`. Bond, window and the bond's bounds are fixed at `init_desk` (WakeDesk's immutables).
#[account]
#[derive(InitSpace)]
pub struct Desk {
    pub bond: u64,
    pub window: i64,
    pub request_count: u64,
    /// `WakeDesk.MIN_BOND` / `MAX_BOND` (0.0005 / 1 ETH in the EVM): lamports from the launch file (D-c), kept so the go-live
    /// check reads the bounds the bond was checked against.
    pub min_bond: u64,
    pub max_bond: u64,
    pub bump: u8,
}

/// `init_desk`'s parameters, all lamports or seconds; the ETH-denominated three come from the launch file, converted at the
/// live SOL/ETH rate (decision D-c, solana-migration.md §2.9; `ops/solana/launch-params.mjs`).
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeskParams {
    /// The bond a wake request pays (`ETCH_WAKE_BOND`, 0.002 ETH at the Robinhood launch).
    pub bond: u64,
    /// `WakeDesk.MIN_BOND` (0.0005 ETH): the least a bond may be.
    pub min_bond: u64,
    /// `WakeDesk.MAX_BOND` (1 ETH): the most a bond may be.
    pub max_bond: u64,
    /// The request window, seconds (`ETCH_WAKE_WINDOW`), within [`MIN_WINDOW`, `MAX_WINDOW`].
    pub window: i64,
}

/// PDA `["request", id u64 le]`.
#[account]
#[derive(InitSpace)]
pub struct Request {
    pub id: u64,
    pub listing: Pubkey,
    pub requester: Pubkey,
    pub at: i64,
    pub bond: u64,
    pub closed: bool,
    /// DI-M-1: the module's post inside the window that served it (`mark_served`); 0: none recorded.
    pub served_at: i64,
}

impl Request {
    pub fn open(&self, now: i64, window: i64) -> bool {
        !self.closed && now < self.at + window
    }
}

/// The last request per collection (`lastOf[wrapper]`, `id + 1`; 0: none). PDA `["wake_slot", listing]`.
#[account]
#[derive(InitSpace)]
pub struct WakeSlot {
    pub last_plus_one: u64,
}

/// One coin pays back one bond (`spent[coin]`). PDA `["spent", backing vault]`.
#[account]
#[derive(InitSpace)]
pub struct Spent {
    pub used: bool,
}

/// The rook_backing program (Anchor.toml `programs.localnet.rook_backing`). PLACEHOLDER: must equal the deployed id.
pub const ROOK_BACKING_ID: Pubkey = pubkey!("8VqSJrKwQjFAXY5RXFxb69wYXmcCzTTuct4chu7FxbGf");

/// What `reclaim` reads of a rook_backing per-coin `Vault` (PDA `["vault", coin_mint]` of [`ROOK_BACKING_ID`]). The offsets
/// are not kept here: they are the shared layout `rook_layouts::backing_vault`, which rook_backing tests against its own
/// `Vault` struct (and rook_core's wake suite builds its vaults from `rook_backing::Vault`), so a field reorder fails CI.
pub struct BackingVaultView {
    pub partner: Pubkey,
    pub coin_mint: Pubkey,
    pub creator: Pubkey,
    pub graduated: bool,
    pub backed_at: i64,
    /// The coin's launch: its DBC pool's creation time (rook_backing copies the pool's `activation_point` at `open_vault`).
    pub launched_at: i64,
}

pub const BACKING_PARTNER_SEED: &[u8] = b"partner";

impl BackingVaultView {
    /// `None` unless `data` is exactly a rook_backing `Vault` (length and discriminator); the caller checks the owner.
    pub fn parse(data: &[u8]) -> Option<Self> {
        let v = rook_layouts::backing_vault::parse(data)?;
        Some(Self {
            partner: Pubkey::new_from_array(v.partner),
            coin_mint: Pubkey::new_from_array(v.coin_mint),
            creator: Pubkey::new_from_array(v.creator),
            graduated: v.graduated,
            backed_at: v.backed_at,
            launched_at: v.launched_at,
        })
    }

    /// The rook_backing partner PDA of a rook_core listing.
    pub fn partner_of(listing: &Pubkey) -> Pubkey {
        Pubkey::find_program_address(&[BACKING_PARTNER_SEED, listing.as_ref()], &ROOK_BACKING_ID).0
    }
}
