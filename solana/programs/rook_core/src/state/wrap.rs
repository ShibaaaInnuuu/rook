//! The wrapper (src/nft/EthNftWrapper.sol, keyed by inscription as src/Wrapper.sol for Bitcoin listings). The `wX` SPL mint
//! (9 decimals; mint authority = the wrapper PDA; no freeze authority) is the ERC-20; this account keeps the ledger
//! `supply == (held − open_tickets) · ONE + eth_backed`.

use anchor_lang::prelude::*;

use crate::clock::DAY;

/// `EthNftWrapper.PIN_AHEAD`: a ticket pins the drand round 20 ahead of the one live at its burn.
pub const PIN_AHEAD: u64 = 20;
pub const PIN_TIMEOUT: i64 = 7 * DAY;
pub const DELIVERY_DEADLINE: i64 = 7 * DAY;
pub const REFUND_GRACE: i64 = 3 * DAY;
pub const LATE_DELIVERY_WINDOW: i64 = 30 * DAY;
pub const MINT_WINDOW: i64 = DAY;
pub const DEFAULT_MINTS_PER_DAY: u32 = 20;
pub const LIMIT_DELAY: i64 = 7 * DAY;
/// The H11 ring: a mint limit is at most this.
pub const MAX_MINTS_PER_DAY: usize = 64;
/// Items one wrapper may hold at once (the free set and the pending queue are bounded by it). Custody caps live in the
/// canister (48 items across all collections at step 1); this is an account-size bound far above them.
pub const MAX_HELD: usize = 256;
pub const MAX_INSCRIPTION_ID: usize = 80;
pub const MAX_TAPROOT: usize = 62;

/// PDA `["wrapper", listing]`.
#[account]
#[derive(InitSpace)]
pub struct Wrapper {
    pub listing: Pubkey,
    pub wx_mint: Pubkey,
    pub bump: u8,
    pub mint_bump: u8,
    /// Items minted and not delivered: free, pending, or on a resolved ticket.
    pub held: u64,
    /// Redemptions neither delivered nor refunded.
    pub open_tickets: u64,
    /// Items delivered to the burn address.
    pub burned: u64,
    pub next_to_resolve: u64,
    pub ticket_count: u64,
    pub item_count: u32,
    /// Units in supply backed by the module's lamports, not by an item.
    pub eth_backed: u64,
    // ---- H11 ----
    pub max_mints_per_day: u32,
    pub item_mints: u64,
    pub minted_at: [i64; MAX_MINTS_PER_DAY],
    pub pending_mint_limit: u32,
    pub pending_limit_at: i64,
}

impl Wrapper {
    /// `mintRoom()`.
    pub fn mint_room(&self, now: i64) -> bool {
        let n = self.item_mints;
        let m = self.max_mints_per_day as u64;
        if n < m {
            return true;
        }
        now >= self.minted_at[((n - m) as usize) % MAX_MINTS_PER_DAY] + MINT_WINDOW
    }

    pub fn count_mint(&mut self, now: i64) -> Result<()> {
        require!(self.mint_room(now), crate::error::RookError::MintRateLimited);
        self.minted_at[(self.item_mints as usize) % MAX_MINTS_PER_DAY] = now;
        self.item_mints += 1;
        Ok(())
    }
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, Default, PartialEq, Eq, InitSpace)]
pub struct Pending {
    pub item_id: u32,
    /// How many tickets existed when the item became free: ticket `n` may draw it only if `stamp <= n`.
    pub stamp: u64,
}

/// The draw sets of one wrapper (EthNftWrapper `_free` / `_pending`), kept apart from `Wrapper` so mints, burns and
/// conversions do not deserialize them. PDA `["items", wrapper]`.
#[account]
#[derive(InitSpace)]
pub struct ItemSet {
    pub wrapper: Pubkey,
    #[max_len(MAX_HELD)]
    pub free: Vec<u32>,
    /// A queue: stamps only grow.
    #[max_len(MAX_HELD)]
    pub pending: Vec<Pending>,
}

/// One custody stay of one NFT or inscription. PDA `["item", wrapper, item_id u32 le]`.
#[account]
#[derive(InitSpace)]
pub struct Item {
    pub wrapper: Pubkey,
    pub item_id: u32,
    /// EthNft: the ERC-721 token id, 32 bytes big-endian. Inscriptions: the membership leaf of the inscription id. SolNft:
    /// the mint (Token Metadata) or the Core asset.
    pub key: [u8; 32],
    /// The transaction on the collection's chain that brought it into custody.
    pub source_tx: [u8; 32],
    pub vout: u32,
    pub deposit: bool,
    /// Inscriptions only.
    #[max_len(MAX_INSCRIPTION_ID)]
    pub inscription_id: String,
}

/// `itemOf[tokenId]` / `inscriptionHeld[leaf]`: which item holds this key now (`item_id + 1`; 0 when none).
/// PDA `["held", wrapper, key]`.
#[account]
#[derive(InitSpace)]
pub struct HeldMarker {
    pub item_plus_one: u32,
}

/// `sourceUsed[keccak(tokenId, tx)]` / `depositMinted[keccak(txid, vout)]`: once, ever. PDA `["source", wrapper, source]`.
#[account]
#[derive(InitSpace)]
pub struct SourceMarker {
    pub used: bool,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, PartialEq, Eq, InitSpace)]
pub enum Stage {
    None,
    Pinned,
    Resolved,
    Delivered,
    Refunded,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, PartialEq, Eq, InitSpace)]
pub enum Destination {
    /// An address on the EthNft listing's chain.
    Eth([u8; 20]),
    /// A taproot address on the listing's Bitcoin network (the registry's: `bc1p…` mainnet, `tb1p…` signet/testnet4), for an
    /// Inscriptions listing.
    Taproot(#[max_len(MAX_TAPROOT)] String),
    /// `redeemToBurn`: `0x…dEaD` or the NUMS taproot; nobody holds the key. Not on a `SolNft` listing.
    Burn,
    /// A Solana wallet, for a `SolNft` listing (appended 2026-10-10; docs/v9/solana-nfts-design.md): the canister transfers
    /// the NFT wallet to wallet and `record_delivery` checks on chain that this wallet holds it.
    Sol(Pubkey),
}

/// One redemption. PDA `["ticket", wrapper, id u64 le]`.
#[account]
#[derive(InitSpace)]
pub struct Ticket {
    pub wrapper: Pubkey,
    pub id: u64,
    pub stage: Stage,
    pub owner: Pubkey,
    pub round: u64,
    pub pinned_at: i64,
    pub deliver_by: i64,
    pub item_id: u32,
    pub delivery_tx: [u8; 32],
    /// `noteSent`: the last delivering transaction noted mined (zero: none).
    pub sent_tx: [u8; 32],
    pub undeliverable: bool,
    pub destination: Destination,
}

/// A drand evmnet round's randomness. PDA `["drand", round u64 le]`.
/// TODO(drand on alt_bn128, Q4): written by `prove_round`, which is keeper-only until the BLS pairing check is ported
/// (DrandEvmnet + BN254HashToCurve over the alt_bn128 syscalls); then anyone proves with the signature and this account
/// stores the keccak of the verified signature as the EVM did.
#[account]
#[derive(InitSpace)]
pub struct DrandRound {
    pub round: u64,
    pub randomness: [u8; 32],
}
