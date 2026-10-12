//! The canister's books (workstream W1 of docs/v9/canister-only-solana.md): rook_core's floor module, wrapper ledger, treasury
//! and raffle, ported from `solana/programs/rook_core/src/{state,instructions}` into pure Rust over the canister's own state.
//!
//! **The books decide, the chain executes.** Every operation here validates and updates the books and returns what Solana must
//! then do (lamports in or out of a collection's module address `M[i]` or the treasury key `K_tre`, a `MintTo` / `Burn` /
//! `TransferChecked` of wX). Nothing here signs, sends or reads the network. Every operation is all-or-nothing: a refused
//! operation leaves the books exactly as they were (a Solana transaction reverted on any error; here each operation checks
//! before it writes, or works on a copy).
//!
//! **Invariants** ([`books_balance`], [`treasury_balance`], [`supply_matches`], [`no_free_profit`], and the pre-signature
//! guards [`supply_guard`] and [`payout_guard`]) are plain functions: the flows call them before every mint or payout signature
//! (§4.4), and the tests call them after every step.
//!
//! **Storage.** [`Books`] is `CandidType + Serialize`; [`encode`] / [`decode`] wrap it in the versioned [`StableBooks`] enum for
//! stable memory (a later layout is a new variant; old bytes keep decoding).
//!
//! Units: lamports (10⁹ per SOL) and wX base units (10⁹ per whole item, [`math::ONE`]); all arithmetic is checked, products in
//! `u128`. Time: unix seconds (`i64`).
//!
//! Not wired into the flows yet (W4 / W5 / W8 / W9 do that).

pub mod floor;
pub mod ledger;
pub mod math;
pub mod params;
pub mod raffle;
pub mod treasury;

use std::collections::BTreeMap;

use candid::CandidType;
use serde::{Deserialize, Serialize};

pub use floor::Module;
pub use ledger::{Destination, Ledger, ListingKind, MintItemArgs, MintTo, Resolution, Stage, Ticket};
pub use math::{Res, BPS, LAMPORTS_PER_SOL, ONE};
pub use params::{default_params, value_params, Params};
pub use raffle::{Raffle, RafflePhase};
pub use treasury::{Allocation, Distribution, Treasury, MAX_COLLECTIONS};

/// Every refusal of the books. Names follow rook_core's `RookError` (and so the Solidity) so a test maps to its source.
#[derive(CandidType, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum BooksError {
    // ---- common ----
    MathOverflow,
    BadInput,
    BadParams,
    NotKeeper,
    NothingPending,
    TooEarly,
    Full,
    WrongCollection,
    UnknownCollection,
    // ---- floor ----
    Stale,
    Paused,
    Slippage,
    OverCap,
    OverBacked,
    NotEnoughBacked,
    OverBasis,
    CapStale,
    OverItemCap,
    NoWrapped,
    ArrivalRateLimited,
    AdvanceOpen,
    NoAdvance,
    NothingToSettle,
    // ---- wrap ----
    ListingPaused,
    AlreadyHeld,
    AlreadyMinted,
    NotInCollection,
    MintRateLimited,
    NotFresh,
    BadDestination,
    NoItem,
    WrongStage,
    OutOfOrder,
    NothingToDraw,
    NotUndeliverable,
    DeliverySentNotRecorded,
    RoundNotProven,
    CustodyFull,
    RoundInFuture,
    CustodyDestination,
    RoundProven,
    WrongHeldMarker,
    // ---- treasury ----
    BadShares,
    AlreadyCollection,
    NotCollection,
    NothingToDo,
    RoundOpen,
    BadReserveCap,
    BadRaffleBps,
    // ---- raffle ----
    NoRaffle,
    PrizeHeld,
    PotTooSmall,
    RafflePhase,
    RaffleTooEarly,
    NoPrize,
    BadSnapshot,
    ClaimWindowOver,
    NotTheWinner,
    BadProof,
    RaffleNotExpired,
    // ---- invariants (a refusal here is an incident: stop the collection, page) ----
    /// `balance(M[i]) − rent < backing + reserve + surplus + revenue`.
    BooksUnbacked,
    /// `balance(K_tre) − rent < unallocated + reserves_pot + Σ raffle pots`.
    TreasuryUnbacked,
    /// The wX supply on chain is not the ledger's.
    SupplyMismatch,
    /// A unit could be minted and burned at once for more than it cost.
    FreeProfit,
    /// Stable bytes of an unknown layout.
    BadVersion,
}

impl std::fmt::Display for BooksError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "books: {self:?}")
    }
}

impl std::error::Error for BooksError {}

use BooksError as E;

fn ensure(ok: bool, e: E) -> Res<()> {
    if ok {
        Ok(())
    } else {
        Err(e)
    }
}

/// One collection: rook_core's `Listing` (the parts the books read), `Module`, `Wrapper` + `ItemSet` + items + tickets, and
/// `Raffle`.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Collection {
    pub id: u16,
    pub kind: ListingKind,
    /// The wX SPL mint (its mint authority is the collection key `K_coll[i]`); the item draw is keyed by it.
    pub wx_mint: [u8; 32],
    /// The guardian's `pause_collection` (§10.4): item mints and public mints closed; burns, redemptions, deliveries go on.
    pub paused: bool,
    /// This collection's poster switch (rook_core `poster_paused`, D-M-1). With the global keeper switch it is `kd`.
    pub poster_paused: bool,
    pub module: Module,
    pub ledger: Ledger,
    pub raffle: Option<Raffle>,
}

/// Every book of the canister-only home.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
pub struct Books {
    /// The registry's keeper switch (rook_core `pause_keeper`): every collection's poster down.
    pub keeper_paused: bool,
    pub collections: BTreeMap<u16, Collection>,
    pub treasury: Treasury,
}

// ---- what the chain must do -------------------------------------------------------------------------------------------------

/// `mint`: `lamports` move into `M[i]`; `units` wX are minted to the buyer.
#[derive(CandidType, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Minted {
    pub lamports: u64,
    pub units: u64,
}

/// `burn`: `units` wX are burned from the holder; `paid` lamports leave `M[i]` for the recipient.
#[derive(CandidType, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Burned {
    pub units: u64,
    pub paid: u64,
    pub released: u64,
}

/// `fund_prize`: `price` lamports move `K_tre → M[i]`; ONE wX is minted to the raffle's prize account at `K_tre`.
#[derive(CandidType, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct PrizeFunded {
    pub price: u64,
    pub units: u64,
}

/// The stable layout. A new layout is a new variant; [`decode`] keeps reading the old ones.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum StableBooks {
    V1(Books),
}

pub const BOOKS_VERSION: u32 = 1;

/// The books as stable bytes (candid, versioned).
pub fn encode(b: &Books) -> Vec<u8> {
    candid::encode_one(StableBooks::V1(b.clone())).expect("the books encode")
}

/// The books from stable bytes of any known version.
pub fn decode(bytes: &[u8]) -> Res<Books> {
    match candid::decode_one::<StableBooks>(bytes).map_err(|_| E::BadVersion)? {
        StableBooks::V1(b) => Ok(b),
    }
}

impl Collection {
    /// rook_core `Listing::keeper_down` (with the A1-01 fix: the poster switch counts for every listing kind).
    pub fn kd(&self, keeper_paused: bool) -> bool {
        keeper_paused || self.poster_paused
    }

    /// The wX supply the chain must show for this collection.
    pub fn expected_supply(&self) -> Option<u64> {
        self.ledger.expected_supply()
    }
}

impl Books {
    pub fn new() -> Books {
        Books::default()
    }

    pub fn get(&self, id: u16) -> Res<&Collection> {
        self.collections.get(&id).ok_or(E::UnknownCollection)
    }

    fn get_mut(&mut self, id: u16) -> Res<&mut Collection> {
        self.collections.get_mut(&id).ok_or(E::UnknownCollection)
    }

    /// `kd` of collection `id`.
    pub fn kd(&self, id: u16) -> Res<bool> {
        Ok(self.get(id)?.kd(self.keeper_paused))
    }

    // ---- setup and switches --------------------------------------------------------------------------------------------

    /// `list_collection` + `init_wrapper` + `init_module` + `add_collection`: a collection with its parameters fixed for good.
    pub fn add_collection(&mut self, id: u16, kind: ListingKind, wx_mint: [u8; 32], params: Params) -> Res<()> {
        ensure(!self.collections.contains_key(&id), E::AlreadyCollection)?;
        if let ListingKind::Inscriptions { membership_root, custody } = &kind {
            ensure(*membership_root != [0u8; 32], E::BadInput)?;
            ensure(math::taproot_hrp(custody).is_some(), E::BadInput)?;
        }
        let module = Module::new(params)?;
        self.treasury.add_collection(id)?;
        self.collections.insert(
            id,
            Collection { id, kind, wx_mint, paused: false, poster_paused: false, module, ledger: Ledger::default(), raffle: None },
        );
        Ok(())
    }

    /// `init_raffle`: the collection's raffle (until it exists, `distribute` keeps its raffle part in the reserves pot).
    pub fn init_raffle(&mut self, id: u16, key: [u8; 32]) -> Res<()> {
        let c = self.get_mut(id)?;
        ensure(c.raffle.is_none(), E::AlreadyCollection)?;
        c.raffle = Some(Raffle::new(key));
        Ok(())
    }

    pub fn set_paused(&mut self, id: u16, paused: bool) -> Res<()> {
        self.get_mut(id)?.paused = paused;
        Ok(())
    }

    pub fn set_poster_paused(&mut self, id: u16, paused: bool) -> Res<()> {
        self.get_mut(id)?.poster_paused = paused;
        Ok(())
    }

    pub fn set_keeper_paused(&mut self, paused: bool) {
        self.keeper_paused = paused;
    }

    // ---- the floor -----------------------------------------------------------------------------------------------------

    /// `post_floor`: the canister's own reading (Ethereum) or the runner's (Ordinals). Refused while the poster is down.
    pub fn post_floor(&mut self, id: u16, price: u64, confidence_bps: u16, depth_items: u32, now: i64) -> Res<(u64, u64)> {
        let kp = self.keeper_paused;
        let c = self.get_mut(id)?;
        ensure(!c.kd(kp), E::NotKeeper)?;
        c.module.post_floor(price, confidence_bps, depth_items, now)
    }

    /// The public mint: pay `lamports`, receive wX at `max(price, target) × (1 + spread)`, SOL-backed. Refused while the
    /// collection is paused (§10.4).
    pub fn mint(&mut self, id: u16, lamports: u64, min_units: u64, now: i64) -> Res<Minted> {
        let kp = self.keeper_paused;
        let c = self.get_mut(id)?;
        ensure(!c.paused, E::ListingPaused)?;
        let kd = c.kd(kp);
        let e = c.ledger.sol_backed;
        let units = c.module.mint(lamports, min_units, now, e, kd)?;
        c.ledger.sol_backed = e + units; // units ≤ max_eth_backed − e: no overflow
        Ok(Minted { lamports, units })
    }

    /// The public burn: `units` wX for lamports (reserve tier first, never above the backing per unit).
    pub fn burn(&mut self, id: u16, units: u64, min_lamports: u64, now: i64) -> Res<Burned> {
        let kp = self.keeper_paused;
        let c = self.get_mut(id)?;
        let kd = c.kd(kp);
        let e = c.ledger.sol_backed;
        let (paid, released) = c.module.burn(units, min_lamports, now, e, kd)?;
        c.ledger.sol_backed = e - units; // units ≤ live ≤ e
        Ok(Burned { units, paid, released })
    }

    /// What `mint` would give now, without changing anything.
    pub fn quote_mint(&self, id: u16, lamports: u64, now: i64) -> Res<u64> {
        let mut b = self.get(id)?.clone();
        let kd = b.kd(self.keeper_paused);
        ensure(!b.paused, E::ListingPaused)?;
        b.module.mint(lamports, 0, now, b.ledger.sol_backed, kd)
    }

    /// What `burn` would pay now, without changing anything.
    pub fn quote_burn(&self, id: u16, units: u64, now: i64) -> Res<u64> {
        let mut b = self.get(id)?.clone();
        let kd = b.kd(self.keeper_paused);
        Ok(b.module.burn(units, 0, now, b.ledger.sol_backed, kd)?.0)
    }

    pub fn mint_price(&self, id: u16, now: i64) -> Res<u64> {
        let c = self.get(id)?;
        c.module.mint_price(now, c.ledger.sol_backed)
    }

    /// The canister's buy signal and cap: (`net_units`, `max_item_cost`).
    pub fn buy_signal(&self, id: u16, now: i64) -> Res<(u64, u64)> {
        let c = self.get(id)?;
        let e = c.ledger.sol_backed;
        Ok((c.module.net_units(e), c.module.max_item_cost(now, e, c.kd(self.keeper_paused))?))
    }

    /// `advance(amount)`: one convertible unit into transit; `amount` leaves `M[i]` for the purchase leg now.
    pub fn advance(&mut self, id: u16, amount: u64, now: i64) -> Res<u64> {
        let kp = self.keeper_paused;
        let c = self.get_mut(id)?;
        let kd = c.kd(kp);
        c.module.advance(amount, now, c.ledger.sol_backed, kd)
    }

    /// `settle_advance(cost)`: the advanced item is in custody and wrapped to the module ([`MintTo::Module`]); the transit unit
    /// becomes item-backed. No lamports and no wX move.
    pub fn settle_advance(&mut self, id: u16, cost: u64, now: i64) -> Res<u64> {
        let kp = self.keeper_paused;
        let c = self.get_mut(id)?;
        let kd = c.kd(kp);
        let mut m = c.module.clone();
        let adv = m.settle_advance(cost, now, kd, c.ledger.module_units)?;
        c.ledger.take_module_unit()?; // the module's own checks first (rook_core's order), then the ledger's
        c.module = m;
        Ok(adv)
    }

    /// `item_arrived(cost)`: the pay-on-arrival path; `cost` leaves `M[i]` for the purchase leg.
    pub fn item_arrived(&mut self, id: u16, cost: u64, now: i64) -> Res<u64> {
        let kp = self.keeper_paused;
        let c = self.get_mut(id)?;
        let kd = c.kd(kp);
        let mut m = c.module.clone();
        let released = m.item_arrived(cost, now, c.ledger.sol_backed, kd, c.ledger.module_units)?;
        c.ledger.take_module_unit()?;
        c.module = m;
        Ok(released)
    }

    /// `return_advance`: `lamports` came back to `M[i]`. Returns (basis, loss).
    pub fn return_advance(&mut self, id: u16, lamports: u64) -> Res<(u64, u64)> {
        self.get_mut(id)?.module.return_advance(lamports)
    }

    /// `snapshot_cap`.
    pub fn snapshot_cap(&mut self, id: u16, now: i64) -> Res<u64> {
        let kp = self.keeper_paused;
        let c = self.get_mut(id)?;
        let kd = c.kd(kp);
        c.module.snapshot_cap(now, kd)
    }

    /// `fund_reserve`: `lamports` arrived at `M[i]` and join the surplus (a gift, an unbooked sweep).
    pub fn fund_reserve(&mut self, id: u16, lamports: u64) -> Res<()> {
        self.get_mut(id)?.module.fund_reserve(lamports)
    }

    /// `skim`: revenue above the target (all of it and the surplus once escaped) moves `M[i] → K_tre` and is booked as
    /// treasury income.
    pub fn skim(&mut self, id: u16, now: i64) -> Res<u64> {
        let room = u64::MAX - self.treasury.unallocated;
        let c = self.get_mut(id)?;
        let mut m = c.module.clone();
        let amount = m.skim(now)?;
        ensure(amount <= room, E::MathOverflow)?;
        c.module = m;
        self.treasury.receive(amount)?;
        Ok(amount)
    }

    /// Audit B1-01: wX burned on chain by their holders (an owner-signed `Burn`), booked by the reconciliation: the supply the chain
    /// must show drops by `units`; the module keeps their backing.
    pub fn book_holder_burn(&mut self, id: u16, units: u64) -> Res<()> {
        let l = &mut self.get_mut(id)?.ledger;
        l.holder_burned = l.holder_burned.checked_add(units).ok_or(E::MathOverflow)?;
        ensure(l.expected_supply().is_some(), E::SupplyMismatch)
    }

    // ---- items and tickets ---------------------------------------------------------------------------------------------

    pub fn mint_item(&mut self, id: u16, args: MintItemArgs, to: MintTo, now: i64) -> Res<u32> {
        let c = self.get_mut(id)?;
        let (kind, paused) = (c.kind.clone(), c.paused);
        if to == MintTo::Module {
            // the module's unit is taken against a SOL-backed unit: never more module units than SOL-backed units
            ensure(c.ledger.module_units + ONE <= c.ledger.sol_backed, E::NotEnoughBacked)?;
        }
        c.ledger.mint_item(&kind, paused, args, to, now)
    }

    /// `redeem`: the redeemer's ONE wX is burned on chain; a ticket is pinned. Returns the ticket id.
    pub fn redeem(&mut self, id: u16, owner: [u8; 32], destination: Destination, now: i64) -> Res<u64> {
        let c = self.get_mut(id)?;
        let kind = c.kind.clone();
        c.ledger.redeem(&kind, owner, destination, now)
    }

    pub fn resolve(&mut self, id: u16, ticket: u64, randomness: Option<[u8; 32]>, now: i64) -> Res<Resolution> {
        let c = self.get_mut(id)?;
        let key = c.wx_mint;
        c.ledger.resolve(ticket, randomness, &key, now)
    }

    pub fn note_sent(&mut self, id: u16, ticket: u64, tx: [u8; 32]) -> Res<()> {
        self.get_mut(id)?.ledger.note_sent(ticket, tx)
    }

    pub fn mark_undeliverable(&mut self, id: u16, ticket: u64) -> Res<()> {
        self.get_mut(id)?.ledger.mark_undeliverable(ticket)
    }

    pub fn record_delivery(&mut self, id: u16, ticket: u64, tx: [u8; 32]) -> Res<()> {
        self.get_mut(id)?.ledger.record_delivery(ticket, tx)
    }

    /// `refund`: the chain re-mints ONE wX to the returned owner.
    pub fn refund(&mut self, id: u16, ticket: u64, round_proven: bool, now: i64) -> Res<[u8; 32]> {
        self.get_mut(id)?.ledger.refund(ticket, round_proven, now)
    }

    /// `initMintLimit`; `supply` = the wX supply on chain.
    pub fn init_mint_limit(&mut self, id: u16, limit: u32, supply: u64) -> Res<()> {
        self.get_mut(id)?.ledger.init_mint_limit(limit, supply)
    }

    pub fn propose_mint_limit(&mut self, id: u16, limit: u32, now: i64) -> Res<i64> {
        self.get_mut(id)?.ledger.propose_mint_limit(limit, now)
    }

    pub fn accept_mint_limit(&mut self, id: u16, now: i64) -> Res<u32> {
        self.get_mut(id)?.ledger.accept_mint_limit(now)
    }

    // ---- the treasury --------------------------------------------------------------------------------------------------

    /// Booked income at `K_tre`.
    pub fn receive(&mut self, lamports: u64) -> Res<()> {
        self.treasury.receive(lamports)
    }

    pub fn allocate(&mut self) -> Res<Allocation> {
        self.treasury.allocate()
    }

    /// Weights: each registered collection's seven-day volume.
    pub fn weights(&self, now: i64) -> Res<Vec<u64>> {
        self.treasury.collections.iter().map(|id| Ok(self.get(*id)?.module.volume_7d(now))).collect()
    }

    pub fn begin_distribution(&mut self, now: i64) -> Res<()> {
        let w = self.weights(now)?;
        self.treasury.begin_distribution(&w, now)
    }

    /// `distribute(index)`: `to_surplus` moves `K_tre → M[i]` and joins the module's surplus; `to_raffle` joins the raffle pot
    /// (a book move at `K_tre`); `to_ops` moves `K_tre → ops`. (The books-only form, rook_core's; the canister-only cranks use
    /// [`Books::distribute_owed`].)
    pub fn distribute(&mut self, index: u8) -> Res<Distribution> {
        self.distribute_with(index, true)
    }

    /// Review F2 (canister-only): as [`Books::distribute`], but `to_surplus` is only OWED to `M[i]`: it joins the module's surplus
    /// (`fund_reserve`) when the treasury's transfer lands (a chain link, `cranks::TreasuryStep::Surplus`), so the books never say a
    /// module holds SOL it does not.
    pub fn distribute_owed(&mut self, index: u8) -> Res<Distribution> {
        self.distribute_with(index, false)
    }

    fn distribute_with(&mut self, index: u8, fund: bool) -> Res<Distribution> {
        let id = *self.treasury.collections.get(index as usize).ok_or(E::NotCollection)?;
        let c = self.collections.get(&id).ok_or(E::UnknownCollection)?;
        let mut t = self.treasury.clone();
        let d = t.distribute(index, c.module.accepted, c.module.surplus, c.raffle.is_some())?;
        ensure(c.module.surplus.checked_add(d.to_surplus).is_some(), E::MathOverflow)?;
        self.treasury = t;
        let c = self.collections.get_mut(&id).expect("checked above");
        if fund && d.to_surplus != 0 {
            c.module.fund_reserve(d.to_surplus)?;
        }
        if let (Some(r), true) = (c.raffle.as_mut(), d.to_raffle != 0) {
            r.credit(d.to_raffle);
        }
        Ok(d)
    }

    /// One call distributes to every collection (§9.1: pagination goes): a round over all registered collections, then each
    /// index owed something. Refused as `begin_distribution` is (an open round younger than `ROUND_TIMEOUT`, no volume).
    pub fn distribute_all(&mut self, now: i64) -> Res<Vec<Distribution>> {
        self.distribute_all_with(now, true)
    }

    /// [`Books::distribute_all`] with the surplus only owed ([`Books::distribute_owed`]).
    pub fn distribute_all_owed(&mut self, now: i64) -> Res<Vec<Distribution>> {
        self.distribute_all_with(now, false)
    }

    fn distribute_all_with(&mut self, now: i64, fund: bool) -> Res<Vec<Distribution>> {
        self.begin_distribution(now)?;
        let mut out = Vec::new();
        for i in 0..self.treasury.collections.len() {
            if self.treasury.round_owed(i)? != 0 {
                out.push(self.distribute_with(i as u8, fund)?);
            }
        }
        Ok(out)
    }

    // ---- the raffle ----------------------------------------------------------------------------------------------------

    /// Lamports booked at `K_tre` straight into collection `id`'s raffle pot (rook_core: a plain transfer to the raffle account
    /// joined its pot at the next `fund_prize`; here the canister attributes the deposit explicitly, §4.4).
    pub fn fund_raffle(&mut self, id: u16, lamports: u64) -> Res<()> {
        ensure(lamports != 0, E::BadInput)?;
        let r = self.get_mut(id)?.raffle.as_mut().ok_or(E::NoRaffle)?;
        ensure(r.pot.checked_add(lamports).is_some(), E::MathOverflow)?;
        r.credit(lamports);
        Ok(())
    }

    /// `fund_prize`: the pot buys exactly one whole wX through the module's public mint at `mint_price`.
    pub fn fund_prize(&mut self, id: u16, now: i64) -> Res<PrizeFunded> {
        let kp = self.keeper_paused;
        let c = self.get_mut(id)?;
        let kd = c.kd(kp);
        let r = c.raffle.as_ref().ok_or(E::NoRaffle)?;
        ensure(r.prize < ONE, E::PrizeHeld)?;
        ensure(!c.paused, E::ListingPaused)?;
        ensure(c.module.fresh(now), E::Stale)?;
        ensure(!kd, E::Paused)?;
        let e = c.ledger.sol_backed;
        // `mint` buys ⌊lamports · ONE / unit_price⌋ with unit_price = mint_price: paying exactly mint_price buys exactly ONE
        let price = c.module.mint_price(now, e)?;
        r.check_fund(price)?;
        let mut m = c.module.clone();
        let units = m.mint(price, ONE, now, e, kd)?;
        ensure(units == ONE, E::MathOverflow)?;
        c.module = m;
        c.ledger.sol_backed = e + units;
        c.raffle.as_mut().expect("checked above").funded(price, units);
        Ok(PrizeFunded { price, units })
    }

    pub fn commit_raffle(&mut self, id: u16, root: [u8; 32], total_weight: u64, snapshot_slot: u64, entries: u32, slot: u64, now: i64) -> Res<u64> {
        let r = self.get_mut(id)?.raffle.as_mut().ok_or(E::NoRaffle)?;
        r.commit(root, total_weight, snapshot_slot, entries, slot, now)
    }

    pub fn draw_raffle(&mut self, id: u16, round: u64, randomness: &[u8; 32], now: i64) -> Res<u64> {
        let r = self.get_mut(id)?.raffle.as_mut().ok_or(E::NoRaffle)?;
        r.draw(round, randomness, now)
    }

    /// `claim_raffle`: the chain transfers ONE wX from the prize account to `owner`'s associated token account.
    pub fn claim_raffle(&mut self, id: u16, owner: [u8; 32], cum_start: u64, weight: u64, proof: &[[u8; 32]], now: i64) -> Res<u64> {
        let r = self.get_mut(id)?.raffle.as_mut().ok_or(E::NoRaffle)?;
        r.claim(owner, cum_start, weight, proof, now)
    }

    pub fn expire_raffle(&mut self, id: u16, now: i64) -> Res<bool> {
        let r = self.get_mut(id)?.raffle.as_mut().ok_or(E::NoRaffle)?;
        r.expire(now)
    }

    /// Lamports the raffle pots claim at `K_tre`.
    pub fn raffle_pots(&self) -> u128 {
        self.collections.values().filter_map(|c| c.raffle.as_ref()).map(|r| r.pot as u128).sum()
    }
}

// ---- invariants -------------------------------------------------------------------------------------------------------------

/// `balance(M[i]) − rent ≥ backing + reserve + surplus + revenue`. `free` = the module address's lamports above its rent (read
/// at finalized), net of any in-flight payout already booked.
pub fn books_balance(m: &Module, free: u64) -> Res<()> {
    ensure(free as u128 >= m.books(), E::BooksUnbacked)
}

/// `balance(K_tre) − rent ≥ unallocated + reserves_pot + Σ raffle pots`.
pub fn treasury_balance(b: &Books, free: u64) -> Res<()> {
    ensure(free as u128 >= b.treasury.books() + b.raffle_pots(), E::TreasuryUnbacked)
}

/// The ledger's supply: `(held − open_tickets)·ONE + sol_backed − module_units` (`None` if inconsistent).
pub fn expected_supply(held: u64, open_tickets: u64, sol_backed: u64, module_units: u64) -> Option<u64> {
    held.checked_sub(open_tickets)?.checked_mul(ONE)?.checked_add(sol_backed)?.checked_sub(module_units)
}

/// The wX supply on chain is the ledger's, or UNDER it (rook_core `check_ledger` was exact; audit B1-01: any holder may burn their own
/// wX on chain, which the reconciliation books as `holder_burned`; a supply over the ledger's is a mint the books never made).
pub fn supply_matches(l: &Ledger, supply: u64) -> Res<()> {
    let want = l.expected_supply().ok_or(E::SupplyMismatch)?;
    ensure(supply <= want, E::SupplyMismatch)
}

/// No free profit: while the public mint is priced (fresh, poster up), a whole unit bought now and burned at once returns at most
/// what it cost. Holds by construction (`mint_price ≥ max(price, up_target) ≥ the burn tier prices`); checked before signing.
pub fn no_free_profit(m: &Module, now: i64, e: u64, kd: bool) -> Res<()> {
    if !m.fresh(now) || kd {
        return Ok(());
    }
    let buy = m.mint_price(now, e)?;
    let (pr, pc) = m.tier_prices(now, e, kd)?;
    ensure(pr <= buy && pc <= buy, E::FreeProfit)
}

/// §4.4 rule 2, before signing a `MintTo` of `amount`: the finalized supply, plus mints signed and not finalized, plus this one,
/// may not exceed the ledger's supply (the books already include `amount` and the pending mints: book first, then sign).
pub fn supply_guard(l: &Ledger, finalized_supply: u64, pending_mints: u64, amount: u64) -> Res<()> {
    let want = l.expected_supply().ok_or(E::SupplyMismatch)?;
    let total = (finalized_supply as u128) + (pending_mints as u128) + (amount as u128);
    ensure(total <= want as u128, E::SupplyMismatch)
}

/// §4.4 rule 3, before signing a payout of `amount` from `M[i]`: what stays (free balance at finalized, less the payouts signed
/// and not finalized, less this one) covers the books, which already exclude this payout.
pub fn payout_guard(m: &Module, free: u64, pending_out: u64, amount: u64) -> Res<()> {
    let left = (free as i128) - (pending_out as i128) - (amount as i128);
    ensure(left >= 0 && left as u128 >= m.books(), E::BooksUnbacked)
}

/// Every invariant of one collection at once: its module address's free lamports and its wX supply on chain.
pub fn check_collection(b: &Books, id: u16, free: u64, supply: u64, now: i64) -> Res<()> {
    let c = b.get(id)?;
    books_balance(&c.module, free)?;
    supply_matches(&c.ledger, supply)?;
    no_free_profit(&c.module, now, c.ledger.sol_backed, c.kd(b.keeper_paused))?;
    ensure(c.module.reserve_units <= c.ledger.sol_backed && c.module.transit_units <= ONE, E::SupplyMismatch)?;
    ensure(c.ledger.held >= c.ledger.open_tickets, E::SupplyMismatch)
}
