//! Anchor events, for the indexer only (§2.4: no event is load-bearing; the canister reads accounts).

use anchor_lang::prelude::*;

#[event]
pub struct Listed {
    pub listing: Pubkey,
    pub id: u32,
}
#[event]
pub struct ListingPausedSet {
    pub listing: Pubkey,
    pub paused: bool,
}
#[event]
pub struct KeeperPausedSet {
    pub by: Pubkey,
    pub paused: bool,
}
#[event]
pub struct PosterPausedSet {
    pub listing: Pubkey,
    pub by: Pubkey,
    pub paused: bool,
}
#[event]
pub struct RolesSet {
    pub keeper: Pubkey,
    pub ready_at: i64,
}
/// A converter proposed (`allowed == false`, acceptable from `ready_at`) or accepted (`allowed == true`).
#[event]
pub struct ConverterSet {
    pub converter: Pubkey,
    pub ready_at: i64,
    pub allowed: bool,
}
/// A converter removed, or a pending one cancelled, by the owner or the guardian.
#[event]
pub struct ConverterRemoved {
    pub converter: Pubkey,
    pub by: Pubkey,
}
#[event]
pub struct RootSet {
    pub listing: Pubkey,
    pub root: [u8; 32],
    pub ready_at: i64,
}
#[event]
pub struct FloorPosted {
    pub module: Pubkey,
    pub target: u64,
    pub accepted: u64,
    pub moved_bps: u64,
    pub confidence_bps: u16,
    pub depth_items: u32,
}
#[event]
pub struct Minted {
    pub module: Pubkey,
    pub payer: Pubkey,
    pub lamports: u64,
    pub units: u64,
}
#[event]
pub struct Burned {
    pub module: Pubkey,
    pub owner: Pubkey,
    pub units: u64,
    pub lamports: u64,
    pub released: u64,
}
#[event]
pub struct Converted {
    pub module: Pubkey,
    pub converter: Pubkey,
    pub dest: Pubkey,
    pub lamports: u64,
    pub units: u64,
    pub unit_price: u64,
}
#[event]
pub struct Advanced {
    pub module: Pubkey,
    pub amount: u64,
    pub released: u64,
}
#[event]
pub struct AdvanceSettled {
    pub module: Pubkey,
    pub advanced: u64,
    pub cost: u64,
}
#[event]
pub struct AdvanceReturned {
    pub module: Pubkey,
    pub returned: u64,
    pub basis: u64,
    pub loss: u64,
}
#[event]
pub struct ItemArrived {
    pub module: Pubkey,
    pub cost: u64,
    pub released: u64,
}
#[event]
pub struct ReserveFunded {
    pub module: Pubkey,
    pub from: Pubkey,
    pub lamports: u64,
}
#[event]
pub struct Skimmed {
    pub module: Pubkey,
    pub lamports: u64,
}
#[event]
pub struct Wrapped {
    pub wrapper: Pubkey,
    pub item_id: u32,
    pub key: [u8; 32],
    pub source_tx: [u8; 32],
    pub to: Pubkey,
    pub deposit: bool,
}
#[event]
pub struct Redeemed {
    pub wrapper: Pubkey,
    pub ticket: u64,
    pub owner: Pubkey,
    pub round: u64,
    /// Where the item goes (as the EVM's `Redeemed` emitted it; audit A2-09).
    pub destination: crate::state::Destination,
}
#[event]
pub struct Resolved {
    pub wrapper: Pubkey,
    pub ticket: u64,
    pub item_id: u32,
    pub deliver_by: i64,
}
#[event]
pub struct DeliverySent {
    pub wrapper: Pubkey,
    pub ticket: u64,
    pub tx: [u8; 32],
}
#[event]
pub struct Delivered {
    pub wrapper: Pubkey,
    pub ticket: u64,
    pub tx: [u8; 32],
}
#[event]
pub struct Undeliverable {
    pub wrapper: Pubkey,
    pub ticket: u64,
}
#[event]
pub struct Refunded {
    pub wrapper: Pubkey,
    pub ticket: u64,
    pub owner: Pubkey,
    /// The item that goes back to the set (a resolved ticket); none for a pinned one (audit A2-09).
    pub item: Option<u32>,
    pub destination: crate::state::Destination,
}
#[event]
pub struct MintLimitSet {
    pub wrapper: Pubkey,
    pub limit: u32,
    pub ready_at: i64,
}
#[event]
pub struct Allocated {
    pub amount: u64,
    pub to_reserves: u64,
    pub to_ops: u64,
}
#[event]
pub struct ReserveDelivered {
    pub module: Pubkey,
    /// Paid into the module's surplus (up to its reserve cap).
    pub amount: u64,
    /// The rest of its share above the cap, paid to operations.
    pub to_ops: u64,
    /// The rest of its share above the cap kept in the reserves pot (no operations address set).
    pub kept: u64,
}
#[event]
pub struct WakeRequested {
    pub id: u64,
    pub listing: Pubkey,
    pub requester: Pubkey,
    pub bond: u64,
}
#[event]
pub struct WakeClosed {
    pub id: u64,
    /// 0 reclaimed, 1 forfeited to the treasury, 2 refunded (not served).
    pub outcome: u8,
    pub bond: u64,
}
#[event]
pub struct WakeServed {
    pub id: u64,
    pub posted_at: i64,
}
#[event]
pub struct WxMetadataSet {
    pub wrapper: Pubkey,
    pub name: String,
    pub symbol: String,
    pub uri: String,
}

#[event]
pub struct TreasurySharesProposed {
    pub reserves_bps: u16,
    pub ops_bps: u16,
    pub ready_at: i64,
}

#[event]
pub struct TreasurySharesSet {
    pub reserves_bps: u16,
    pub ops_bps: u16,
}

#[event]
pub struct ReserveCapProposed {
    pub bps: u32,
    pub ready_at: i64,
}

#[event]
pub struct ReserveCapSet {
    pub bps: u32,
}

#[event]
pub struct TreasuryOpsProposed {
    pub ops: Pubkey,
    pub ready_at: i64,
}

#[event]
pub struct TreasuryOpsSet {
    pub ops: Pubkey,
}

// ---- the holder raffle (docs/v9/raffle-design.md) ----

#[event]
pub struct RaffleBpsProposed {
    pub bps: u16,
    pub ready_at: i64,
}

#[event]
pub struct RaffleBpsSet {
    pub bps: u16,
}

#[event]
pub struct RaffleInitialized {
    pub raffle: Pubkey,
    pub listing: Pubkey,
    pub wx_account: Pubkey,
}

/// `distribute` paid `lamports` of a collection's reserve overflow into its raffle pot (now `pot`).
#[event]
pub struct RafflePotFunded {
    pub raffle: Pubkey,
    pub listing: Pubkey,
    pub lamports: u64,
    pub pot: u64,
}

/// `fund_prize`: `lamports` of the pot bought `units` (one whole wX) through the floor module's mint.
#[event]
pub struct RafflePrizeFunded {
    pub raffle: Pubkey,
    pub round: u64,
    pub lamports: u64,
    pub units: u64,
    pub pot: u64,
}

#[event]
pub struct RaffleCommitted {
    pub raffle: Pubkey,
    pub round: u64,
    pub root: [u8; 32],
    pub total_weight: u64,
    pub entries: u32,
    pub snapshot_slot: u64,
    pub drand_round: u64,
}

#[event]
pub struct RaffleDrawn {
    pub raffle: Pubkey,
    pub round: u64,
    pub drand_round: u64,
    pub ticket: u64,
}

#[event]
pub struct RaffleClaimed {
    pub raffle: Pubkey,
    pub round: u64,
    pub winner: Pubkey,
    pub ticket: u64,
    pub cum_start: u64,
    pub weight: u64,
}

/// `drawn`: an unclaimed prize returned for the next round; otherwise an undrawn round cancelled.
#[event]
pub struct RaffleExpired {
    pub raffle: Pubkey,
    pub round: u64,
    pub drawn: bool,
}

