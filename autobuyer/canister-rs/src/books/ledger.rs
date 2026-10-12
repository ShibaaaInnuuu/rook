//! The wrapper ledger (rook_core `state/wrap.rs` + the book-keeping of `instructions/wrap.rs`; EthNftWrapper.sol, Wrapper.sol):
//! items in custody, held markers, sources used once ever, the draw sets, tickets and their resolution order, deliveries,
//! refunds, and the item-mint rate limit (H11).
//!
//! wX is an SPL token the canister mints (docs/v9/canister-only-solana.md §2, §8): the ledger DECIDES every supply change and
//! the chain executes it. On chain the supply must equal [`Ledger::expected_supply`]:
//!
//! `supply = (held − open_tickets) · ONE + sol_backed − module_units`
//!
//! which is rook_core's `(held − open_tickets) · ONE + eth_backed` with one change: an item bought with an advance is credited
//! to the module in the books only (`module_units`) and taken by `settle_advance` / `item_arrived`, so no wX moves for it (§8:
//! "`mint_item` + `settle_advance` … book only … no wX moves"). In rook_core that unit was minted to the module's token account
//! and burned at settlement: the net supply is the same.
//!
//! Account checks become map lookups (§4.1). What each operation asks the chain to do is in its return value.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use candid::CandidType;
use serde::{Deserialize, Serialize};

use super::math::{is_taproot_on, keccak, leaf_of, round_at, taproot_hrp, verify_proof, Res, DAY, ONE};
use super::BooksError as E;

/// A ticket pins the drand round 20 ahead of the one live at its burn.
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
/// Items one collection may hold at once (rook_core's account-size bound; the custody caps are far below it).
pub const MAX_HELD: usize = 256;
pub const MAX_INSCRIPTION_ID: usize = 80;
pub const MAX_TAPROOT: usize = 62;

fn ensure(ok: bool, e: E) -> Res<()> {
    if ok {
        Ok(())
    } else {
        Err(e)
    }
}

/// What a collection's items are (rook_core `ListingKind` with the custody it checks destinations against).
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum ListingKind {
    /// ERC-721 on an EVM chain; `custody` = the canister's custody address there.
    EthNft { chain_id: u64, collection: [u8; 20], custody: [u8; 20] },
    /// Ordinals; membership by Merkle root; `custody` = the canister's taproot custody address (its prefix decides the network).
    Inscriptions { membership_root: [u8; 32], custody: String },
    /// Solana NFTs (paused design, docs/v9/solana-nfts-design.md); `custodian` = the custody wallet.
    SolNft { collection: [u8; 32], custodian: [u8; 32] },
}

#[derive(CandidType, Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pending {
    pub item_id: u32,
    /// How many tickets existed when the item became free: ticket `n` may draw it only if `stamp <= n`.
    pub stamp: u64,
}

/// One custody stay of one NFT or inscription.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub item_id: u32,
    /// EthNft: the token id (32 bytes big-endian). Inscriptions: the membership leaf. SolNft: the mint or asset.
    pub key: [u8; 32],
    /// The transaction on the collection's chain that brought it into custody.
    pub source_tx: [u8; 32],
    pub vout: u32,
    pub deposit: bool,
    pub inscription_id: String,
}

#[derive(CandidType, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    None,
    Pinned,
    Resolved,
    Delivered,
    Refunded,
}

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum Destination {
    Eth([u8; 20]),
    Taproot(String),
    /// `redeemToBurn`: `0x…dEaD` or the NUMS taproot. Not on a SolNft listing.
    Burn,
    Sol([u8; 32]),
}

/// One redemption.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Ticket {
    pub id: u64,
    pub stage: Stage,
    /// The Solana wallet the refund re-mints to.
    pub owner: [u8; 32],
    pub round: u64,
    pub pinned_at: i64,
    pub deliver_by: i64,
    pub item_id: u32,
    pub delivery_tx: [u8; 32],
    pub sent_tx: [u8; 32],
    pub undeliverable: bool,
    pub destination: Destination,
}

/// rook_core `MintItemArgs`.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct MintItemArgs {
    pub key: [u8; 32],
    /// EthNft / SolNft: `keccak(key ‖ tx)`. Inscriptions: `keccak(tx ‖ vout be)`.
    pub source: [u8; 32],
    pub tx: [u8; 32],
    pub vout: u32,
    pub deposit: bool,
    pub inscription_id: String,
    pub proof: Vec<[u8; 32]>,
}

/// Where a newly wrapped item's unit goes.
#[derive(CandidType, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum MintTo {
    /// A purchase an advance (or `item_arrived`) pays for: credited to the module in the books; nothing is minted on chain.
    Module,
    /// A deposit: `MintTo(owner's wX ATA, ONE)` on chain.
    Holder([u8; 32]),
}

/// The wrapper ledger of one collection.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Ledger {
    /// Items minted and not delivered: free, pending, or on a resolved ticket.
    pub held: u64,
    /// Redemptions neither delivered nor refunded.
    pub open_tickets: u64,
    /// Items delivered to the burn address.
    pub burned: u64,
    pub next_to_resolve: u64,
    pub ticket_count: u64,
    pub item_count: u32,
    /// Units in supply backed by the module's lamports, not by an item (rook_core `eth_backed`).
    pub sol_backed: u64,
    /// Item units credited to the module and not yet taken (rook_core: the module's wX token account balance).
    pub module_units: u64,
    /// Audit B1-01: wX burned on chain by their holders themselves (an owner-signed SPL `Burn`, no desk): booked by the reconciliation
    /// when the chain's supply is under the ledger's; their backing stays at the module (the holder's gift), nothing stops.
    #[serde(default)]
    pub holder_burned: u64,
    // ---- H11 ----
    pub max_mints_per_day: u32,
    pub item_mints: u64,
    pub minted_at: Vec<i64>,
    pub pending_mint_limit: u32,
    pub pending_limit_at: i64,
    // ---- maps (rook_core's PDAs) ----
    pub items: BTreeMap<u32, Item>,
    /// `["held", key]`: which item holds this key now.
    pub held_markers: BTreeMap<[u8; 32], u32>,
    /// `["source", source]`: once, ever.
    pub sources_used: BTreeSet<[u8; 32]>,
    pub free: Vec<u32>,
    pub pending: VecDeque<Pending>,
    pub tickets: BTreeMap<u64, Ticket>,
}

impl Default for Ledger {
    fn default() -> Self {
        Ledger {
            held: 0,
            open_tickets: 0,
            burned: 0,
            next_to_resolve: 0,
            ticket_count: 0,
            item_count: 0,
            sol_backed: 0,
            module_units: 0,
            holder_burned: 0,
            max_mints_per_day: DEFAULT_MINTS_PER_DAY,
            item_mints: 0,
            minted_at: vec![0; MAX_MINTS_PER_DAY],
            pending_mint_limit: 0,
            pending_limit_at: 0,
            items: BTreeMap::new(),
            held_markers: BTreeMap::new(),
            sources_used: BTreeSet::new(),
            free: Vec::new(),
            pending: VecDeque::new(),
            tickets: BTreeMap::new(),
        }
    }
}

/// What a resolution drew.
#[derive(CandidType, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resolution {
    /// A ticket refunded while pinned, stepped over.
    Skipped,
    Drawn { item_id: u32, deliver_by: i64 },
}

impl Ledger {
    /// The wX supply the chain must show: `(held − open_tickets)·ONE + sol_backed − module_units`. `None` if the books are
    /// inconsistent (more open tickets than items, or more module units than SOL-backed units).
    pub fn expected_supply(&self) -> Option<u64> {
        let items = self.held.checked_sub(self.open_tickets)?.checked_mul(ONE)?;
        items.checked_add(self.sol_backed)?.checked_sub(self.module_units)?.checked_sub(self.holder_burned)
    }

    /// Items free for a new redemption (`held − open_tickets`).
    pub fn redeemable(&self) -> u64 {
        self.held.saturating_sub(self.open_tickets)
    }

    // ---- H11 -----------------------------------------------------------------------------------------------------------

    pub fn mint_room(&self, now: i64) -> bool {
        let n = self.item_mints;
        let m = self.max_mints_per_day as u64;
        if n < m {
            return true;
        }
        now >= self.minted_at[((n - m) as usize) % MAX_MINTS_PER_DAY] + MINT_WINDOW
    }

    fn count_mint(&mut self, now: i64) -> Res<()> {
        ensure(self.mint_room(now), E::MintRateLimited)?;
        let i = (self.item_mints as usize) % MAX_MINTS_PER_DAY;
        self.minted_at[i] = now;
        self.item_mints += 1;
        Ok(())
    }

    /// `initMintLimit`: once, before anything was ever minted (`supply` = the wX supply the chain shows).
    pub fn init_mint_limit(&mut self, limit: u32, supply: u64) -> Res<()> {
        ensure(limit != 0 && (limit as usize) <= MAX_MINTS_PER_DAY, E::BadInput)?;
        ensure(self.item_mints == 0 && supply == 0 && self.held == 0, E::NotFresh)?;
        self.max_mints_per_day = limit;
        Ok(())
    }

    /// `proposeMintLimit`: a lower limit applies at once; a raise waits `LIMIT_DELAY`. Returns when it may be accepted (0: now).
    pub fn propose_mint_limit(&mut self, limit: u32, now: i64) -> Res<i64> {
        ensure(limit != 0 && (limit as usize) <= MAX_MINTS_PER_DAY, E::BadInput)?;
        if limit <= self.max_mints_per_day {
            self.max_mints_per_day = limit;
            self.pending_mint_limit = 0;
            self.pending_limit_at = 0;
            return Ok(0);
        }
        self.pending_mint_limit = limit;
        self.pending_limit_at = now + LIMIT_DELAY;
        Ok(self.pending_limit_at)
    }

    /// `acceptMintLimit`: after the delay.
    pub fn accept_mint_limit(&mut self, now: i64) -> Res<u32> {
        let limit = self.pending_mint_limit;
        ensure(limit != 0, E::WrongStage)?;
        ensure(now >= self.pending_limit_at, E::TooEarly)?;
        self.pending_mint_limit = 0;
        self.pending_limit_at = 0;
        self.max_mints_per_day = limit;
        Ok(limit)
    }

    // ---- wrap ----------------------------------------------------------------------------------------------------------

    /// `mint` / `mintDeposit` / `wrapDeposit`, once the item is in custody (the canister's own reading: an Ethereum `ownerOf`, the
    /// inscription at the custody output, a Solana token account). Refused while the listing is paused, once per item held, once
    /// per (item, transaction) ever, and under H11. Returns the new item's id; with [`MintTo::Holder`] the chain mints ONE to
    /// that owner, with [`MintTo::Module`] nothing moves.
    pub fn mint_item(&mut self, kind: &ListingKind, paused: bool, args: MintItemArgs, to: MintTo, now: i64) -> Res<u32> {
        ensure(!paused, E::ListingPaused)?;
        ensure(args.tx != [0u8; 32], E::BadInput)?;
        match kind {
            ListingKind::EthNft { .. } | ListingKind::SolNft { .. } => {
                ensure(args.source == keccak(&[&args.key, &args.tx]), E::BadInput)?;
                ensure(args.inscription_id.is_empty(), E::BadInput)?;
            }
            ListingKind::Inscriptions { membership_root, .. } => {
                ensure(!args.inscription_id.is_empty() && args.inscription_id.len() <= MAX_INSCRIPTION_ID, E::BadInput)?;
                ensure(args.key == leaf_of(&args.inscription_id), E::BadInput)?;
                ensure(verify_proof(&args.proof, membership_root, args.key), E::NotInCollection)?;
                ensure(args.source == keccak(&[&args.tx, &args.vout.to_be_bytes()]), E::BadInput)?;
            }
        }
        ensure(!self.held_markers.contains_key(&args.key), E::AlreadyHeld)?;
        ensure(!self.sources_used.contains(&args.source), E::AlreadyMinted)?;
        ensure((self.held as usize) < MAX_HELD, E::CustodyFull)?;
        let item_id = self.item_count;
        let next = item_id.checked_add(1).ok_or(E::MathOverflow)?;
        if to == MintTo::Module {
            self.module_units.checked_add(ONE).ok_or(E::MathOverflow)?;
        }
        // every check passed; H11 is the last (it writes only when it passes), then nothing refuses
        self.count_mint(now)?;
        self.sources_used.insert(args.source);
        self.items.insert(
            item_id,
            Item {
                item_id,
                key: args.key,
                source_tx: args.tx,
                vout: args.vout,
                deposit: args.deposit,
                inscription_id: args.inscription_id,
            },
        );
        self.held_markers.insert(args.key, item_id);
        self.item_count = next;
        self.held += 1;
        self.pending.push_back(Pending { item_id, stamp: self.ticket_count });
        if to == MintTo::Module {
            self.module_units += ONE;
        }
        Ok(item_id)
    }

    /// The module takes its item unit (`settle_advance` / `item_arrived`): `sol_backed −= ONE`, `module_units −= ONE`.
    pub fn take_module_unit(&mut self) -> Res<()> {
        ensure(self.module_units >= ONE, E::NoWrapped)?;
        ensure(self.sol_backed >= ONE, E::BadInput)?;
        self.module_units -= ONE;
        self.sol_backed -= ONE;
        Ok(())
    }

    // ---- redeem --------------------------------------------------------------------------------------------------------

    /// rook_core `handle_redeem`'s destination rules.
    pub fn check_destination(kind: &ListingKind, destination: &Destination) -> Res<()> {
        match (kind, destination) {
            (ListingKind::EthNft { custody, .. }, Destination::Eth(a)) => {
                ensure(*a != [0u8; 20] && a != custody, E::BadDestination)
            }
            (ListingKind::Inscriptions { custody, .. }, Destination::Taproot(s)) => {
                let hrp = taproot_hrp(custody).ok_or(E::BadDestination)?;
                ensure(is_taproot_on(s, hrp), E::BadDestination)?;
                // never the custody address itself (audit A2-03)
                ensure(s != custody, E::CustodyDestination)
            }
            (ListingKind::SolNft { custodian, .. }, Destination::Sol(a)) => {
                ensure(*a != [0u8; 32], E::BadDestination)?;
                ensure(a != custodian, E::CustodyDestination)
            }
            (ListingKind::SolNft { .. }, _) => Err(E::BadDestination),
            (_, Destination::Burn) => Ok(()),
            _ => Err(E::BadDestination),
        }
    }

    /// `redeem` / `redeemToBurn`: one whole unit (burned on chain from the redeemer's intent address) for one random item,
    /// delivered to `destination`. The round that decides is pinned now (`round_at(now) + 20`). Pausing stops no redemption.
    pub fn redeem(&mut self, kind: &ListingKind, owner: [u8; 32], destination: Destination, now: i64) -> Res<u64> {
        Self::check_destination(kind, &destination)?;
        ensure(self.held >= self.open_tickets + 1, E::NoItem)?;
        let id = self.ticket_count;
        self.tickets.insert(
            id,
            Ticket {
                id,
                stage: Stage::Pinned,
                owner,
                round: round_at(now) + PIN_AHEAD,
                pinned_at: now,
                deliver_by: 0,
                item_id: 0,
                delivery_tx: [0; 32],
                sent_tx: [0; 32],
                undeliverable: false,
                destination,
            },
        );
        self.ticket_count = id + 1;
        self.open_tickets += 1;
        Ok(id)
    }

    pub fn ticket(&self, id: u64) -> Res<&Ticket> {
        self.tickets.get(&id).ok_or(E::WrongStage)
    }

    /// `resolve`, strictly in ticket order. `randomness` = the canister's VERIFIED drand randomness of the ticket's pinned round
    /// (`None` while it is not proven); `draw_key` = the collection's 32-byte key the draw is keyed by (rook_core: the wrapper
    /// PDA; here the wX mint). A ticket refunded while pinned is stepped over.
    pub fn resolve(&mut self, id: u64, randomness: Option<[u8; 32]>, draw_key: &[u8; 32], now: i64) -> Res<Resolution> {
        ensure(id == self.next_to_resolve, E::OutOfOrder)?;
        let tk = self.tickets.get(&id).ok_or(E::OutOfOrder)?;
        if tk.stage == Stage::Refunded {
            self.next_to_resolve = id + 1;
            return Ok(Resolution::Skipped);
        }
        ensure(tk.stage == Stage::Pinned, E::WrongStage)?;
        let rnd = randomness.ok_or(E::RoundNotProven)?;
        // never a round that is not live yet (audit A2-02): no randomness of a future round is accepted
        ensure(tk.round <= round_at(now), E::RoundInFuture)?;
        let k = self.pending.iter().take_while(|p| p.stamp <= id).count();
        let size = (self.free.len() + k) as u64;
        ensure(size != 0, E::NothingToDraw)?;
        let promoted: Vec<u32> = self.pending.drain(..k).map(|p| p.item_id).collect();
        self.free.extend(promoted);
        let h = keccak(&[&rnd, draw_key, &id.to_le_bytes()]);
        let mut i: u64 = 0;
        for b in h {
            i = ((i << 8) | b as u64) % size;
        }
        let item_id = self.free.swap_remove(i as usize);
        let deliver_by = now + DELIVERY_DEADLINE;
        let tk = self.tickets.get_mut(&id).expect("checked above");
        tk.item_id = item_id;
        tk.stage = Stage::Resolved;
        tk.deliver_by = deliver_by;
        self.next_to_resolve = id + 1;
        Ok(Resolution::Drawn { item_id, deliver_by })
    }

    /// `noteSent`: a transaction delivering this resolved ticket mined; the late window never refunds it any more.
    pub fn note_sent(&mut self, id: u64, tx: [u8; 32]) -> Res<()> {
        let tk = self.tickets.get_mut(&id).ok_or(E::WrongStage)?;
        ensure(tk.stage == Stage::Resolved, E::WrongStage)?;
        ensure(tx != [0u8; 32], E::BadInput)?;
        tk.sent_tx = tx;
        Ok(())
    }

    /// `markUndeliverable`.
    pub fn mark_undeliverable(&mut self, id: u64) -> Res<()> {
        let tk = self.tickets.get_mut(&id).ok_or(E::WrongStage)?;
        ensure(tk.stage == Stage::Resolved, E::WrongStage)?;
        tk.undeliverable = true;
        Ok(())
    }

    /// `recordDelivery`: the item left custody to the ticket's destination; allowed past the deadline while nobody refunded.
    /// The held marker must still point to the ticket's item (audit A2-09).
    pub fn record_delivery(&mut self, id: u64, tx: [u8; 32]) -> Res<()> {
        let tk = self.tickets.get(&id).ok_or(E::WrongStage)?;
        let item = self.items.get(&tk.item_id).ok_or(E::WrongCollection)?;
        ensure(self.held_markers.get(&item.key) == Some(&tk.item_id), E::WrongHeldMarker)?;
        ensure(tk.stage == Stage::Resolved, E::WrongStage)?;
        ensure(tx != [0u8; 32], E::BadInput)?;
        ensure(self.held >= 1 && self.open_tickets >= 1, E::MathOverflow)?;
        let key = item.key;
        let burn = tk.destination == Destination::Burn;
        let tk = self.tickets.get_mut(&id).expect("checked above");
        tk.stage = Stage::Delivered;
        tk.delivery_tx = tx;
        self.held_markers.remove(&key);
        self.held -= 1;
        self.open_tickets -= 1;
        if burn {
            self.burned += 1;
        }
        Ok(())
    }

    /// `refund` (anyone): the unit goes back to the ticket's owner (the chain re-mints ONE to them). A pinned ticket after
    /// `PIN_TIMEOUT` whose round was never proven; a resolved one after `deliver_by + REFUND_GRACE` if marked undeliverable,
    /// or after `deliver_by + LATE_DELIVERY_WINDOW` without the mark and without `note_sent` (D3-H-1). The item goes back.
    ///
    /// `round_proven`: whether the canister holds verified randomness for the ticket's round. A pinned ticket whose round IS
    /// proven is refused (audit A2-04: its outcome is public, a refund would be a free re-roll). Returns the owner to re-mint to.
    pub fn refund(&mut self, id: u64, round_proven: bool, now: i64) -> Res<[u8; 32]> {
        let tk = self.tickets.get(&id).ok_or(E::WrongStage)?;
        match tk.stage {
            Stage::Pinned => {
                ensure(now >= tk.pinned_at + PIN_TIMEOUT, E::TooEarly)?;
                ensure(!round_proven, E::RoundProven)?;
            }
            Stage::Resolved => {
                ensure(now >= tk.deliver_by + REFUND_GRACE, E::TooEarly)?;
                if !tk.undeliverable {
                    ensure(tk.sent_tx == [0u8; 32], E::DeliverySentNotRecorded)?;
                    ensure(now >= tk.deliver_by + LATE_DELIVERY_WINDOW, E::NotUndeliverable)?;
                }
            }
            _ => return Err(E::WrongStage),
        }
        ensure(self.open_tickets >= 1, E::MathOverflow)?;
        let (stage, item_id, owner) = (tk.stage, tk.item_id, tk.owner);
        self.tickets.get_mut(&id).expect("checked above").stage = Stage::Refunded;
        self.open_tickets -= 1;
        if stage == Stage::Pinned {
            if id == self.next_to_resolve {
                self.next_to_resolve += 1;
            }
        } else {
            self.pending.push_back(Pending { item_id, stamp: self.ticket_count });
        }
        Ok(owner)
    }
}
