//! W8c (docs/v9/solana-nfts-canister.md §1 "the flow a later solnftflow should run", §9; docs/v9/canister-only-solana.md §8): the
//! Solana NFT flow on the books home. A collection listed in the books as `ListingKind::SolNft { collection, custodian }` is bought,
//! held and delivered by the canister itself, the custody wallet being `K_main` (`KeyPath::Main`) and the collection's money `M[i]`
//! (`KeyPath::Coll(i)`), as the Ethereum (`ethflow::home_books`) and Bitcoin Frogs (`frogsol::books`) collections are:
//!
//! - [`floor`]: the runner hands in Magic Eden's listings and stats (`sol_nft_evidence`, operator-only, bounded, idempotent); the
//!   canister turns them into candidates (`solnftfloor::candidates_from_me`, `reference_from_me_stats`), verifies every one on chain
//!   at `finalized` through the agreed providers (`solnftbuy::verify_all` on one read), and posts the floor (`solnftfloor::floor_from`)
//!   to the books (`post_floor`, under the books' own bounds: 20 % a day, stale after 3 h). No fresh floor, no mint and no buy.
//! - [`buy`]: when the module's buy signal fires (`books_sink_view` + `itemsink::may_start`, as `home_books::gate`), the cheapest
//!   verified ask under the limit `min(floor × (1 + premium), max_item_cost)` (`solnftbuy::best_ask`; Magic Eden M2 buyer-only, on by
//!   default since 2026-10-12, off at once by the operator's switch), built and JUDGED, then ONE transaction on the money line signed
//!   by `K_main` and `K_coll[i]`: the advance `M[i] → K_main` (a chain link, booked before its signature, `Effects { lamports_out }`, undone if it
//!   never lands or fails), the purchase, and the link's memo. Landed: `solnft::check` for `K_main` at finalized, then `mint_item` (the
//!   mint as the custody proof, the landed signature as the source) and `settle_advance` through the deferred writer
//!   (`desk::book_outcome`, audit B1-06). An M2 purchase is sent as a v0 message loading M2's fixed accounts from the canister's own
//!   lookup table ([`table`]: created once on the money line, read at finalized before each use), which is what makes a pNFT's fit.
//! - [`deliver`]: a resolved ticket to a Solana wallet: `solnft::check` for `K_main`, `send_instructions` + `judge_send`, signed on the
//!   money line by `K_main` alone; landed (the line's verdict on the judged, canonical transfer): `record_delivery` (audit C1-01: the
//!   transaction decides, not what the destination did with the item since). A destination that refuses (not a wallet; a pNFT rule
//!   set veto, B2-09: refused by simulation AT the transfer, audit C1-07, or failed on chain [`DELIVERY_TRIES`] times) is
//!   `mark_undeliverable`. `Destination::Burn` never reaches here: the ledger refuses it on a SolNft listing.
//! - [`api`]: the IC surface (`sol_nft_evidence`, `sol_nft_status`, `sol_nft_configure`, `sol_nft_set_off`) and the tick's world.
//!
//! One step of this flow at a time on the money line (`out`), at most [`STEPS_PER_DAY`] signed a day, never while the collection has
//! an open chain link (`desk::collection_busy`), within the custody caps (`custodycap`, key `sol:<collection>`, a collection of the
//! install's policy and counted in its global caps, audit C1-02) and the outflow limits (deliveries, H3).
pub mod api;
pub mod buy;
pub mod deliver;
pub mod floor;
pub mod run;
pub mod table;

use std::cell::RefCell;
use std::collections::BTreeMap;

use candid::CandidType;
use serde::{Deserialize, Serialize};

use crate::chainlink::driver::DeskIo;
use crate::solhome::LinePending;
use crate::solnft::{Accounts, Standard};
use crate::solnftbuy::{Candidate, Venue};
use crate::solnftfloor::SolFloor;
use crate::soltx::{b58_encode, Pubkey};

pub use run::tick;

/// O-T1 (docs/v9/solana-nfts-canister.md §8, §9): the premium a keyless buy may pay over the books' floor, by default (the Ethereum
/// side's `max_premium_bps`-style 10 %). Over the CURRENT floor the books' own item cap (`max_item_cost`: the floor plus `conv_bps`,
/// 3 % by default) binds below it (audit C1-I1); the premium is what binds over the lowest floor of the last [`FLOOR_WINDOW_DAYS`]
/// days (audit C1-05, [`buy::window_limit`]): how far a walked-up floor can carry a buy.
pub const DEFAULT_PREMIUM_BPS: u16 = 1_000;
/// The most the operator may set it to.
pub const MAX_PREMIUM_BPS: u16 = 3_000;
/// Magic Eden M2 bought buyer-only by default (the operator's decision of 2026-10-12: Magic Eden's cheap asks of pNFT collections are
/// mostly M2; a pNFT purchase fits as a v0 message with the canister's lookup table, [`table`]).
pub const DEFAULT_M2: bool = true;
/// A widening change (a premium raise, M2 switched on) waits this long, as the treasury's knobs do.
pub const CHANGE_DELAY_SECS: u64 = 24 * 3_600;
/// Evidence older than this is not read.
pub const EVIDENCE_TTL_SECS: u64 = 1_800;
/// Verified asks are bought from for this long after their read, at most (else the next evidence is awaited).
pub const ASKS_TTL_SECS: u64 = 120;
/// Candidates kept from one hand-in (the cheapest by Magic Eden's price), and the hand-in's size bounds.
pub const MAX_CANDIDATES: usize = 40;
pub const MAX_LISTINGS_BYTES: usize = 64 * 1024;
pub const MAX_STATS_BYTES: usize = 4 * 1024;
/// Steps signed a day, at most (purchases and deliveries).
pub const STEPS_PER_DAY: u32 = 24;
/// A delivery refused this many times (simulation or on chain) marks its ticket undeliverable.
pub const DELIVERY_TRIES: u32 = 3;
/// Asks kept for the status view.
pub const ASKS_SHOWN: usize = 10;
/// Audit C1-05: a buy's limit is at most the LOWEST floor the canister accepted over this many days (and today), plus the premium.
/// The hand-in chooses which listings the canister reads; a floor walked up on its own listings (the books let it climb 20 % a day)
/// carries the buy limit only once it has held this long.
pub const FLOOR_WINDOW_DAYS: u64 = 7;
/// Audit C1-08: no delivery is signed this close to (or past) its ticket's late refund (`deliver_by + LATE_DELIVERY_WINDOW`): there
/// the refund rule decides, and a delivery never races the public refund.
pub const LATE_MARGIN_SECS: i64 = 86_400;
/// Audit C1-01: tickets of one collection looked at in one delivery pass, at most (one that cannot progress never stops the others).
pub const TICKETS_PER_PASS: usize = 8;
/// Audit C1-02: the custody book's ids of this flow's items (`solnft:<mint>`), added to the canister's book whenever it is checked and
/// never kept in it (the books' `held_markers` are the record).
pub const CAP_ID_PREFIX: &str = "solnft:";

/// A step of this flow on the money line.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum SolNftStep {
    /// The advance `M[i] → K_main` and the purchase of `asset`, in one transaction (`link` the advance's chain link, `cost` its lamports).
    Buy { coll: u16, asset: Pubkey, venue: Venue, cost: u64, link: u64 },
    /// The delivery of ticket `ticket`'s item to `to`.
    Deliver { coll: u16, ticket: u64, to: Pubkey },
    /// 2026-10-12: the canister's address lookup table `key` for M2 purchases created and filled ([`table`]; `coll` the collection whose
    /// M2 ask waited for it). Moves no collection's money (`K_main` pays its rent).
    Table { coll: u16, key: Pubkey },
}

impl SolNftStep {
    pub fn link(&self) -> Option<u64> {
        match self {
            SolNftStep::Buy { link, .. } => Some(*link),
            SolNftStep::Deliver { .. } | SolNftStep::Table { .. } => None,
        }
    }
    pub fn coll(&self) -> u16 {
        match self {
            SolNftStep::Buy { coll, .. } | SolNftStep::Deliver { coll, .. } | SolNftStep::Table { coll, .. } => *coll,
        }
    }
    pub fn label(&self) -> String {
        match self {
            SolNftStep::Buy { coll, asset, venue, cost, link } => format!("solnft: advance + purchase of {} on {venue:?} for collection {coll}, \
                {cost} lamports (link {link})", b58_encode(asset)),
            SolNftStep::Deliver { coll, ticket, to } => format!("solnft: delivery of collection {coll}'s ticket {ticket} to {}", b58_encode(to)),
            SolNftStep::Table { coll, key } => format!("solnft: the M2 lookup table {} (for collection {coll})", b58_encode(key)),
        }
    }
}

/// Magic Eden's hand-in, kept as what the canister uses of it: the candidates (addresses only) and the reference floor.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Evidence {
    pub cands: Vec<Candidate>,
    pub reference: u64,
    pub at: u64,
}

/// One verified ask as the status shows it.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct AskView {
    pub venue: Venue,
    pub listing: String,
    pub asset: String,
    pub price: u64,
    pub total: u64,
    pub rent: u64,
    pub keyless: bool,
}

/// A widening change waiting its delay.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PendingChange {
    pub premium_bps: u16,
    pub m2: bool,
    pub at: u64,
}

/// A purchase that landed: the item's custody is checked at finalized before it is booked.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Arrival {
    pub asset: Pubkey,
    pub signature: String,
    pub cost: u64,
    pub tries: u32,
}

/// An item in custody: its cost (the custody caps' value) and the signature that brought it (the full custody proof).
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Held {
    pub cost: u64,
    pub signature: String,
}

/// One collection's flow.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ColState {
    /// What the collection's items are (a Token Metadata listing, `Programmable`, accepts legacy members too; `Core` only Core).
    pub standard: Standard,
    pub premium_bps: u16,
    /// Buy Magic Eden M2 listings buyer-only (`solnftm2`; spike S5). On by default for a collection the flow has no state for yet (the
    /// operator's decision of 2026-10-12; a pNFT purchase is a v0 message with the canister's lookup table, [`table`]); a state saved
    /// before keeps its value. `sol_nft_configure(.., m2 = Some(false))` switches it off at once; on again waits [`CHANGE_DELAY_SECS`].
    pub m2: bool,
    pub pending: Option<PendingChange>,
    /// When the evidence was last read on chain (the read happens once per hand-in). The hand-in itself and the read's asks are not
    /// kept here (audit C1-I2: [`Cache`], never persisted).
    pub read_at: u64,
    pub floor: Option<SolFloor>,
    pub floor_at: u64,
    pub floor_note: String,
    /// A landed purchase, kept until the books HOLD its item (audit C1-04: not until `book_outcome` merely accepted or owed it).
    pub arrival: Option<Arrival>,
    /// Delivery refusals by ticket.
    pub tries: BTreeMap<u64, u32>,
    /// Items in custody, by mint.
    pub held: BTreeMap<Pubkey, Held>,
    pub note: String,
    /// Audit C1-05: the lowest floor the books accepted from this flow's posts, by day (`(day, lamports)`, the last
    /// [`FLOOR_WINDOW_DAYS`] days and today). Optional: a state saved before it decodes.
    pub floor_days: Option<Vec<(u64, u64)>>,
}

impl Default for ColState {
    fn default() -> Self {
        ColState { standard: Standard::Programmable, premium_bps: DEFAULT_PREMIUM_BPS, m2: DEFAULT_M2, pending: None, read_at: 0, floor: None,
            floor_at: 0, floor_note: String::new(), arrival: None, tries: BTreeMap::new(), held: BTreeMap::new(), note: String::new(),
            floor_days: None }
    }
}

impl ColState {
    /// Audit C1-05: a floor the books accepted at `now`, kept as its day's lowest.
    pub fn note_floor(&mut self, accepted: u64, now: u64) {
        let day = now / 86_400;
        let v = self.floor_days.get_or_insert_with(Vec::new);
        match v.iter_mut().find(|(d, _)| *d == day) {
            Some((_, f)) => *f = (*f).min(accepted),
            None => v.push((day, accepted)),
        }
        v.retain(|(d, _)| d.saturating_add(FLOOR_WINDOW_DAYS) >= day);
    }

    /// Audit C1-05: the lowest accepted floor of the last [`FLOOR_WINDOW_DAYS`] days and today, `current` included.
    pub fn window_floor(&self, current: u64, now: u64) -> u64 {
        let day = now / 86_400;
        self.floor_days.iter().flatten().filter(|(d, _)| d.saturating_add(FLOOR_WINDOW_DAYS) >= day).map(|(_, f)| *f).fold(current, u64::min)
    }
}

/// Audit C1-I2: what a hand-in leaves for the next passes, NOT persisted (the stable blob holds no type of `solnftbuy` / `solnft`, so a
/// change there never makes it undecodable; after an upgrade the flow only waits for the next hand-in): the hand-in, the read's
/// verified asks (bought from while [`ASKS_TTL_SECS`] old at most) and the asks this read could not buy (audit C1-03: never picked
/// again before the next read).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cache {
    pub evidence: Option<Evidence>,
    pub asks: Vec<(crate::solnftbuy::Ask, crate::solnft::Member)>,
    pub refused: Vec<Pubkey>,
}

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct SolNftState {
    pub cols: BTreeMap<u16, ColState>,
    /// This flow's step out on the money line (a copy of the line's pending record), until its outcome is taken.
    pub out: Option<LinePending>,
    pub ops: u64,
    pub day: u64,
    pub signed_today: u32,
    /// The operator's switch (stop-only: nothing new is signed; outcomes are still booked).
    pub off: bool,
    /// Audit C1-02: the last lamports-per-ETH rate the flow read, for its items' values in the canister's custody book (optional: a
    /// state saved before it decodes; none yet counts the items at no value, their number still counts).
    pub rate: Option<u64>,
    /// 2026-10-12: the canister's lookup table for M2 purchases ([`table`]), once created (optional: a state saved before decodes).
    pub table: Option<table::Table>,
}

impl SolNftState {
    pub fn roll_day(&mut self, now: u64) {
        let d = now / 86_400;
        if d != self.day {
            self.day = d;
            self.signed_today = 0;
        }
    }
}

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum StableSolNft {
    V1(SolNftState),
}

pub fn encode_state(s: &SolNftState) -> Vec<u8> {
    candid::encode_one(StableSolNft::V1(s.clone())).expect("the solnft state encodes")
}

pub fn decode_state(bytes: &[u8]) -> Result<SolNftState, String> {
    match candid::decode_one::<StableSolNft>(bytes).map_err(|e| format!("the solnft state does not decode: {e}"))? {
        StableSolNft::V1(s) => Ok(s),
    }
}

thread_local! {
    static STATE: RefCell<SolNftState> = RefCell::new(SolNftState::default());
    static CACHE: RefCell<BTreeMap<u16, Cache>> = const { RefCell::new(BTreeMap::new()) };
}

/// Read collection `coll`'s transient cache (audit C1-I2).
pub fn cache<R>(coll: u16, f: impl FnOnce(&Cache) -> R) -> R {
    CACHE.with(|c| f(c.borrow().get(&coll).unwrap_or(&Cache::default())))
}

pub fn cache_mut<R>(coll: u16, f: impl FnOnce(&mut Cache) -> R) -> R {
    CACHE.with(|c| f(c.borrow_mut().entry(coll).or_default()))
}

pub fn with<R>(f: impl FnOnce(&SolNftState) -> R) -> R {
    STATE.with(|s| f(&s.borrow()))
}

pub fn with_mut<R>(f: impl FnOnce(&mut SolNftState) -> R) -> R {
    STATE.with(|s| f(&mut s.borrow_mut()))
}

pub fn reset() {
    STATE.with(|c| *c.borrow_mut() = SolNftState::default());
    CACHE.with(|c| c.borrow_mut().clear());
    table::unwant();
}

/// `pre_upgrade`: the state rides in the lines' `solnft_state` blob (absent while empty).
pub fn before_upgrade() {
    let blob = with(|s| (*s != SolNftState::default()).then(|| encode_state(s)));
    crate::solhome::LINES.with(|l| l.borrow_mut().solnft_state = blob);
}

/// `post_upgrade`: a blob that does not decode refuses the upgrade.
pub fn after_upgrade() -> Result<(), String> {
    match crate::solhome::LINES.with(|l| l.borrow_mut().solnft_state.take()) {
        Some(b) => {
            let s = decode_state(&b)?;
            STATE.with(|c| *c.borrow_mut() = s);
            Ok(())
        }
        None => {
            reset();
            Ok(())
        }
    }
}

/// The world of this flow: the desk's (Solana reads, the lines, the canister's keys), the clock, the pause, the custody caps and the
/// SOL/ETH rate the caps' values are in.
#[allow(async_fn_in_trait)]
pub trait SolNftIo: DeskIo {
    fn now(&self) -> u64;
    fn paused(&self) -> bool;
    /// The install's custody caps (`None`: none named; a SolNft collection then starts at stage 0: `custodycap::stage0`).
    fn caps(&self) -> Option<crate::custodycap::CapPolicy>;
    /// Lamports per ETH (the caps are in wei).
    async fn lamports_per_eth(&mut self) -> Result<u64, String> {
        self.sol_rate().await
    }
    /// Audit C1-02: the canister's custody book (every other flow's items, held and in flight); this flow adds its own
    /// ([`cap_entries`]) to check a buy against the global caps.
    fn cap_book(&self) -> crate::custodycap::Book;
    /// Audit C1-02: the outflow limits (H3), asked before a delivery is signed as every other delivery asks them (`Err`: it waits,
    /// or custody is frozen), and the delivery counted once signed.
    fn outflow_check(&mut self, key: &str, collection: &str, flow: crate::outflow::Flow, amount: u128) -> Result<(), String>;
    fn outflow_record(&mut self, key: &str, collection: &str, flow: crate::outflow::Flow, amount: u128);
}

/// The books' SolNft listings: `(id, collection, custodian)`.
pub fn listings() -> Vec<(u16, Pubkey, Pubkey)> {
    crate::desk::state::with(|s| s.books.collections.iter().filter_map(|(id, c)| match c.kind {
        crate::books::ListingKind::SolNft { collection, custodian } => Some((*id, collection, custodian)),
        _ => None,
    }).collect())
}

/// The listing of collection `coll`: `(collection, custodian)`.
pub fn listing_of(coll: u16) -> Result<(Pubkey, Pubkey), String> {
    listings().into_iter().find(|(id, ..)| *id == coll).map(|(_, c, k)| (c, k)).ok_or_else(|| format!("collection {coll} is not a Solana NFT listing in the books"))
}

/// The custody caps' key of a Solana NFT collection: `sol:<the collection's address>`.
pub fn cap_key(collection: &Pubkey) -> String {
    crate::custodycap::sol_key(collection)
}

/// Audit C1-02: the caps' key of every Solana NFT listing of the books, for the install's default policy (`canister::caps_of`: stage 0
/// each, the global cap re-sized by the exit rule A4-L-1 with them).
pub fn live_cap_keys() -> Vec<String> {
    listings().iter().map(|(_, c, _)| cap_key(c)).collect()
}

/// Audit C1-02: this flow's items as custody-book entries (`solnft:<mint>`): every item the books hold of every Solana NFT listing (at
/// its cost, else the floor), and a landed purchase not booked yet (in flight, at its cost), valued at `rate` lamports per ETH (else
/// the last rate read; none: no value, the count still counts).
pub fn cap_entries(rate: Option<u64>) -> Vec<crate::custodycap::Entry> {
    let (rate, cols) = with(|s| (rate.or(s.rate), s.cols.clone()));
    let wei = |lamports: u64| rate.map_or(0, |r| crate::rates::lamports_to_wei(lamports as u128, r));
    let mut out = vec![];
    for (id, collection, _) in listings() {
        let key = cap_key(&collection);
        let cs = cols.get(&id).cloned().unwrap_or_default();
        let (held, accepted) = crate::desk::state::with(|s| s.books.get(id).map(|c| (c.ledger.held_markers.keys().copied().collect::<Vec<_>>(),
            c.module.accepted)).unwrap_or_default());
        for k in &held {
            let cost = cs.held.get(k).map(|h| h.cost).unwrap_or(accepted);
            out.push(crate::custodycap::Entry { id: format!("{CAP_ID_PREFIX}{}", b58_encode(k)), collection: key.clone(), value_wei: wei(cost),
                in_flight: false });
        }
        if let Some(a) = cs.arrival.as_ref().filter(|a| !held.contains(&a.asset)) {
            out.push(crate::custodycap::Entry { id: format!("{CAP_ID_PREFIX}{}", b58_encode(&a.asset)), collection: key.clone(),
                value_wei: wei(a.cost), in_flight: true });
        }
    }
    out
}

/// Agreed accounts at finalized (`SolIo::sol_accounts`: every provider must agree), in pieces of `solrpc::MAX_KEYS`.
pub async fn read<I: DeskIo>(io: &mut I, keys: &[Pubkey]) -> Result<Accounts, String> {
    let mut keys = keys.to_vec();
    keys.sort_unstable();
    keys.dedup();
    let mut out = Accounts::new();
    for chunk in keys.chunks(crate::solrpc::MAX_KEYS) {
        let got = io.sol_accounts(chunk, crate::solnft::MAX_NFT_ACCOUNT).await?;
        if got.len() != chunk.len() {
            return Err(format!("{} accounts for {} keys", got.len(), chunk.len()));
        }
        out.extend(chunk.iter().copied().zip(got));
    }
    Ok(out)
}

/// The first 32 bytes of a base58 signature: the books' `tx` of a Solana transaction.
pub fn tx32(signature: &str) -> [u8; 32] {
    let b = crate::soltx::b58_decode(signature).unwrap_or_default();
    let mut t = [0u8; 32];
    for (i, x) in b.iter().take(32).enumerate() {
        t[i] = *x;
    }
    t
}
