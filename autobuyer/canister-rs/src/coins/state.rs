//! The coins' state (W6): what the launch file pins per collection, how far each collection is open, every Rook coin the canister found
//! under a pinned config with its vault book, the fee books, the coin redemptions, and the waits.
//!
//! **Persistence.** One versioned candid blob ([`StableCoins`], like `desk::state`), carried across an upgrade in the lines' `coins_state`
//! slot (`before_upgrade` / `after_upgrade`). A blob that does not decode refuses the upgrade; it is never a fresh start (that would forget
//! every vault book and every redemption in flight). Read and written only in synchronous sections ([`with`] / [`with_mut`]): never held
//! across an await.
use std::cell::RefCell;
use std::collections::BTreeMap;

use candid::CandidType;
use serde::{Deserialize, Serialize};

use crate::books::{ListingKind, Params};
use crate::soltx::Pubkey;

/// What the launch file pins for one collection (§3.1, §3.2): the curve for its `T_wX` and everything the books need to list it. The
/// canister refuses any DBC config that differs from this (`solix2::guard::check_config`).
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct CollectionPin {
    /// `K_coll[index]`, the books' collection id.
    pub index: u32,
    pub slug: String,
    pub name: String,
    pub symbol: String,
    pub uri: String,
    /// Graduation threshold, whole wX, and the curve `design_curve.mjs` produced for it.
    pub t_wx: u64,
    pub sqrt_start_price: u128,
    pub curve: Vec<(u128, u128)>,
    /// The books' listing and parameters (value-based caps from the floor at listing, §4.3).
    pub kind: ListingKind,
    pub params: Params,
    /// The wX mint and the DBC config, when the ceremony already made them (pinned); `None`: the canister makes them at opening and these
    /// are derived (`open::wx_mint_of`, `KeyPath::Config`).
    pub wx_mint: Option<Pubkey>,
    pub config: Option<Pubkey>,
}

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum ColStage {
    /// Pinned, not opened yet (waiting its turn and the float).
    Pinned,
    /// Some of the chain side exists (the mint, or the mint and the config); the rest is made at the next pass.
    Opening,
    /// Chain side verified, listed in the books: its coins are Rook's.
    Open,
    /// The chain side differs from the pin (the reason): nothing under this config is treated as Rook's until the operator looks.
    Refused(String),
}

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ColRec {
    pub index: u32,
    pub stage: ColStage,
    pub wx_mint: Pubkey,
    pub config: Pubkey,
    pub opened_at: u64,
    /// The pass that last read its chain side.
    pub checked_at: u64,
    /// `M[i] = K_coll[i]`: the module's address (the reconciler reads its balance against the books). Zero in a record saved before it.
    #[serde(default)]
    pub module: Pubkey,
}

#[derive(CandidType, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoinStage {
    /// On the DBC curve.
    Curve,
    /// Migrated; the migration fee, the leftovers or the burn are still to do.
    Migrated,
    /// Backed: the migration fee is in the vault and the leftovers are burned.
    Backed,
}

/// A graduated coin's backing book (§7): the vault is `K_coin[c]`'s wX token account.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct Vault {
    pub address: Pubkey,
    pub owner: Pubkey,
    /// wX that arrived from the migration fee.
    pub wx_in: u64,
    /// Coins burned (the leftovers, then every redeemed coin).
    pub coins_burned: u64,
    /// wX paid out to redeemers.
    pub wx_paid: u64,
    pub backed_at: u64,
}

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct CoinRec {
    /// The collection index.
    pub coll: u32,
    pub coin: Pubkey,
    pub pool: Pubkey,
    pub creator: Pubkey,
    pub launched_at: u64,
    /// Found in this order: the migration's two position NFT mints are `PositionNft(2·seq)` and `PositionNft(2·seq + 1)`.
    pub seq: u64,
    pub stage: CoinStage,
    pub vault: Option<Vault>,
    /// The migrated DAMM v2 pool and the position NFT `K_fc` holds, once read.
    pub damm_pool: Option<Pubkey>,
    pub partner_nft: Option<Pubkey>,
    /// Attempts at the partner's surplus (a refusal means there is none).
    pub surplus_tries: u8,
}

/// What a collection's fee wX became (§9.1): claimed from the curve and from the migrated pools' positions, cashed at the module for
/// SOL to the treasury, sent to the pool desk.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct FeeBook {
    pub curve_wx: u64,
    pub lp_wx: u64,
    pub cashed_wx: u64,
    pub cashed_lamports: u64,
    pub to_desk_wx: u64,
    pub to_desk_burned_wx: u64,
    pub to_desk_lamports: u64,
}

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum RedeemState {
    Requested,
    /// The transaction is out (its link).
    Signed { link: u64 },
    Landed,
    /// Refused or failed (the reason): the coins are still in the intent's account, the desk returns them.
    Failed(String),
}

/// A coin redemption (the desk's `Kind::RedeemCoin` intent `n`): the coins in `K_int[n]`'s coin account burned for their share of the vault.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Redeem {
    pub n: u64,
    pub coin: Pubkey,
    pub coll: u32,
    pub recipient: Pubkey,
    pub amount: u64,
    pub payout: u64,
    pub state: RedeemState,
    pub signature: String,
    pub at: u64,
    /// Attempts signed (a transaction that was undone or failed is asked again, up to `redeem::MAX_TRIES`).
    pub tries: u32,
}

/// A linked step in flight whose books effect is applied when its link lands (the treasury's income, the fee book): kept until the link
/// is decided.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Receipt {
    pub link: u64,
    pub step: super::step::CoinStep,
    pub coll: u32,
    /// Lamports that reach the treasury, and the desk's share (lamports and wX units) of a fee cash.
    pub to_treasury: u64,
    pub desk_lamports: u64,
    pub desk_units: u64,
    /// Units burned in all, and the part of them burned for the desk's SOL.
    pub burned: u64,
    pub desk_burned: u64,
    pub at: u64,
}

/// A step that could not be made, and why: skipped by scans until `retry_at`. The monitor pages one older than its threshold.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Wait {
    pub key: String,
    pub since: u64,
    pub why: String,
    pub retry_at: u64,
    pub tries: u32,
}

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct CoinsState {
    pub pins: BTreeMap<u32, CollectionPin>,
    pub cols: BTreeMap<u32, ColRec>,
    /// By coin mint.
    pub coins: BTreeMap<Pubkey, CoinRec>,
    pub next_seq: u64,
    pub fees: BTreeMap<u32, FeeBook>,
    pub redeems: BTreeMap<u64, Redeem>,
    pub receipts: Vec<Receipt>,
    /// A counter for the idempotence keys of the links this module begins (a key is used once, whatever became of its link).
    pub ops: u64,
    pub waits: Vec<Wait>,
    /// When each kind of scan is next due (`discover`, `graduate`, `fees`).
    pub next: BTreeMap<String, u64>,
    /// The day (unix / 86,400) the counters below count.
    pub day: u64,
    pub signed_today: u32,
    pub opened_today: u32,
    /// Accounts whose coins are left out of the supply a redemption divides by (§7: none by default).
    pub exclude_from_supply: Vec<Pubkey>,
    /// Audit B3-11: changes to that list the operator asked, applied by the pass once their day is over: `(account, excluded, at)`.
    #[serde(default)]
    pub pending_exclusions: Vec<(Pubkey, bool, u64)>,
    /// Each step kind's last landing: `(kind, when, what)`.
    pub last: Vec<(String, u64, String)>,
    /// The operator's switch for the whole module.
    pub off: bool,
    /// Operator decision 2026-10-12 ("the people choose"): the collections a creator asked to open (`coins_request_open`), FIFO:
    /// `(index, asked at)`. The opening pass takes the oldest first; a collection is dropped once Open or Refused. One entry per
    /// collection at most, so the queue is bounded by the pins.
    #[serde(default)]
    pub requested: Vec<(u32, u64)>,
    /// The operator's switch (`coins_set_auto_open`, default off): with no request pending, open the pinned collections in index order
    /// anyway (the float left over spent as the earlier design did). Off: nothing opens unasked.
    #[serde(default)]
    pub auto_open: bool,
    /// Audit C4-02: who asked for each pending request (the caller's principal bytes; empty for the operator's retry, C4-01).
    #[serde(default)]
    pub requesters: BTreeMap<u32, Vec<u8>>,
    /// Audit C4-02: the requests accepted in the last hour, `(caller, at)`: the per-caller, anonymous and overall hourly limits.
    #[serde(default)]
    pub request_log: Vec<(Vec<u8>, u64)>,
    /// Audit C4-04: the requested collections the opening pass is waiting on (by index): skipped after `open::HEAD_PATIENCE` when the
    /// wait is the collection's own, keeping their place; shown to the site (`coins_open_queue`, C4-12).
    #[serde(default)]
    pub stalls: BTreeMap<u32, Stall>,
}

/// Audit C4-04, C4-12: the opening pass waits on a requested collection.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Stall {
    /// When the wait began (unix seconds), and the pass's last look at it.
    pub since: u64,
    pub last_look: u64,
    /// The pass's last wait reason.
    pub why: String,
    /// The wait is the collection's own (a pinned mint the chain lacks, a step refused by simulation), not the float's or the
    /// pace's, which every request shares: only an own wait is skipped.
    pub own: bool,
}

pub const WAITS_KEPT: usize = 64;
pub const WAIT_RETRY_SECS: u64 = 6 * 3_600;
/// Steps signed a day, at most (each is one threshold signature set, ≈ 26 B cycles a signature).
pub const STEPS_PER_DAY: u32 = 96;
/// Collections opened a day, at most (the unlock pace; §16 opens them one by one as fees cover each).
pub const OPENS_PER_DAY: u32 = 8;

impl CoinsState {
    pub fn due(&self, k: &str, now: u64) -> bool {
        self.next.get(k).is_none_or(|t| now >= *t)
    }
    pub fn set_next(&mut self, k: &str, at: u64) {
        self.next.insert(k.to_string(), at);
    }
    pub fn waiting(&self, key: &str, now: u64) -> bool {
        self.waits.iter().any(|w| w.key == key && now < w.retry_at)
    }
    pub fn wait(&mut self, key: String, why: String, now: u64) {
        match self.waits.iter_mut().find(|w| w.key == key) {
            Some(w) => {
                w.why = why;
                w.retry_at = now + WAIT_RETRY_SECS;
                w.tries += 1;
            }
            None => {
                self.waits.push(Wait { key, since: now, why, retry_at: now + WAIT_RETRY_SECS, tries: 1 });
                if self.waits.len() > WAITS_KEPT {
                    self.waits.remove(0);
                }
            }
        }
    }
    pub fn clear_wait(&mut self, key: &str) {
        self.waits.retain(|w| w.key != key);
    }
    /// Audit B3-11: the exclusions whose day is over are applied (as the treasury's setters apply after `TREASURY_DELAY`). Returns them.
    pub fn apply_exclusions(&mut self, now: u64) -> Vec<(Pubkey, bool)> {
        let (due, later): (Vec<_>, Vec<_>) = self.pending_exclusions.drain(..).partition(|(_, _, at)| now >= *at);
        self.pending_exclusions = later;
        for (k, excluded, _) in &due {
            self.exclude_from_supply.retain(|x| x != k);
            if *excluded {
                self.exclude_from_supply.push(*k);
            }
        }
        due.into_iter().map(|(k, e, _)| (k, e)).collect()
    }
    pub fn roll_day(&mut self, now: u64) {
        let d = now / 86_400;
        if d != self.day {
            self.day = d;
            self.signed_today = 0;
            self.opened_today = 0;
        }
    }
    pub fn landed(&mut self, kind: &str, now: u64, what: String) {
        self.last.retain(|(k, ..)| k != kind);
        self.last.push((kind.to_string(), now, what));
    }
}

/// The stable layout. A new layout is a new variant.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum StableCoins {
    V1(CoinsState),
}

pub fn encode_state(s: &CoinsState) -> Vec<u8> {
    candid::encode_one(StableCoins::V1(s.clone())).expect("the coins state encodes")
}

pub fn decode_state(bytes: &[u8]) -> Result<CoinsState, String> {
    match candid::decode_one::<StableCoins>(bytes).map_err(|e| format!("the coins state does not decode: {e}"))? {
        StableCoins::V1(s) => Ok(s),
    }
}

thread_local! {
    static STATE: RefCell<CoinsState> = RefCell::new(CoinsState::default());
}

/// Read the state. Never across an await.
pub fn with<R>(f: impl FnOnce(&CoinsState) -> R) -> R {
    STATE.with(|s| f(&s.borrow()))
}

/// Write the state. Never across an await.
pub fn with_mut<R>(f: impl FnOnce(&mut CoinsState) -> R) -> R {
    STATE.with(|s| f(&mut s.borrow_mut()))
}

pub fn encode() -> Vec<u8> {
    with(encode_state)
}

pub fn restore(bytes: &[u8]) -> Result<(), String> {
    let s = decode_state(bytes)?;
    STATE.with(|c| *c.borrow_mut() = s);
    Ok(())
}

/// A fresh, empty state (tests).
pub fn reset() {
    STATE.with(|c| *c.borrow_mut() = CoinsState::default());
}

/// `pre_upgrade`: the coins go into the saved lines' blob (skipped when nothing was ever pinned).
pub fn before_upgrade() {
    let blob = with(|s| (*s != CoinsState::default()).then(|| encode_state(s)));
    crate::solhome::LINES.with(|l| l.borrow_mut().coins_state = blob);
}

/// `post_upgrade`, after the lines are restored: the coins come out of the blob (and the blob is cleared from the live lines).
pub fn after_upgrade() -> Result<(), String> {
    let blob = crate::solhome::LINES.with(|l| l.borrow_mut().coins_state.take());
    match blob {
        Some(b) => restore(&b),
        None => {
            reset();
            Ok(())
        }
    }
}
