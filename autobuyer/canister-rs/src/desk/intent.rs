//! Intents (§6.1): a user's request to mint, burn or redeem, with its own deposit address `A = K_int[n]`. One record per intent,
//! versioned, persisted with the books (`desk::state`).
use candid::CandidType;
use serde::{Deserialize, Serialize};

use crate::books::{Destination, ONE};

/// The record layout. A later layout bumps this and `state` migrates.
pub const INTENT_VERSION: u32 = 1;

/// The fee per operation: 0.0015 SOL (O-8). It covers the intent's extra signature, the Solana fee and a priority fee.
pub const FEE_LAMPORTS: u64 = 1_500_000;
/// The smallest SOL deposit a mint acts on (O-8).
pub const MIN_MINT_LAMPORTS: u64 = 50_000_000;
/// The smallest wX deposit a burn acts on (O-8), in base units (0.01 wX).
pub const MIN_BURN_UNITS: u64 = ONE / 100;
/// A redemption takes exactly one whole wX per intent.
pub const MIN_REDEEM_UNITS: u64 = ONE;
/// A deposit returned after expiry (or left below the minimum at expiry) must cover the fee and a new account's rent to be worth a
/// transaction: lamports.
pub const DUST_REFUND_LAMPORTS: u64 = 2_500_000;
/// The same in wX base units (0.001 wX).
pub const DUST_REFUND_UNITS: u64 = ONE / 1_000;
/// 24 h of no activity.
pub const TTL_SECS: u64 = 86_400;
/// A refusal that may pass (a stale floor, a paused poster, an hourly bucket) is waited out this long, then the deposit goes back.
pub const RETRY_WINDOW_SECS: u64 = 6 * 3_600;
/// After expiry the address is still watched this long, and a deposit found is returned.
pub const REFUND_GRACE_SECS: u64 = 7 * 86_400;
/// Audit B1-05: an intent that never saw a deposit is read in the tick's batch only this long after it expires; for the rest of its
/// [`REFUND_GRACE_SECS`] it is read in the rotation (one batched read of [`crate::solrpc::MAX_KEYS`] such addresses per tick), so free
/// openings never grow the per-tick reads.
pub const UNFUNDED_GRACE_SECS: u64 = 3_600;
/// An intent is dropped from the record this long after its grace ends.
pub const KEEP_SECS: u64 = 30 * 86_400;
/// Failed attempts (a transaction that landed failed or never landed) before the deposit goes back.
pub const MAX_ATTEMPTS: u32 = 3;
/// A refund that fails this many times is left for the operator.
pub const MAX_REFUND_ATTEMPTS: u32 = 5;
/// Failed or refused closes of `A`'s emptied token account (audit B1-04) before it is left open, its rent at `A`.
pub const MAX_CLOSE_TRIES: u32 = 2;
/// Open intents that have seen a deposit, in all (audit B1-05: free openings count apart, [`MAX_UNFUNDED_OPEN`]); per caller principal
/// per hour; open per recipient.
pub const MAX_OPEN: usize = 300;
/// Audit B1-05: open intents that never saw a deposit (an opening is free and may be anonymous), in all. They never take one of the
/// [`MAX_OPEN`] slots, so free openings cannot refuse the work of funded intents.
pub const MAX_UNFUNDED_OPEN: usize = 200;
pub const MAX_PER_CALLER_HOUR: usize = 5;
pub const MAX_PER_RECIPIENT: usize = 3;
/// Transactions kept per intent.
pub const HISTORY_KEPT: usize = 8;

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// SOL to `A`; wX of `coll` to `recipient`'s token account, at least `min_units`.
    Mint { coll: u16, recipient: [u8; 32], min_units: u64 },
    /// wX to `A`'s token account; SOL to `recipient` (at least `min_lamports` net of the fee); a refusal returns the wX to `refund_to`.
    Burn { coll: u16, recipient: [u8; 32], refund_to: [u8; 32], min_lamports: u64 },
    /// One wX to `A`'s token account: burned, a ticket for one item to `destination`; `owner` is the ticket's owner (the refund
    /// re-mints to it) and where an unused deposit goes back.
    RedeemNft { coll: u16, destination: Destination, owner: [u8; 32] },
    /// Coins to `A`'s coin token account: redeemed for their share of the vault (W6).
    RedeemCoin { coin: [u8; 32], coll: u16, recipient: [u8; 32], refund_to: [u8; 32] },
}

impl Kind {
    pub fn coll(&self) -> u16 {
        match self {
            Kind::Mint { coll, .. } | Kind::Burn { coll, .. } | Kind::RedeemNft { coll, .. } | Kind::RedeemCoin { coll, .. } => *coll,
        }
    }
    pub fn tag(&self) -> &'static str {
        match self {
            Kind::Mint { .. } => "mint",
            Kind::Burn { .. } => "burn",
            Kind::RedeemNft { .. } => "redeem_nft",
            Kind::RedeemCoin { .. } => "redeem_coin",
        }
    }
    /// A SOL deposit (a system account) rather than a token deposit.
    pub fn is_sol(&self) -> bool {
        matches!(self, Kind::Mint { .. })
    }
    /// The smallest deposit that is acted on (lamports for a mint, base units otherwise).
    pub fn min_deposit(&self) -> u64 {
        match self {
            Kind::Mint { .. } => MIN_MINT_LAMPORTS,
            Kind::Burn { .. } => MIN_BURN_UNITS,
            Kind::RedeemNft { .. } => MIN_REDEEM_UNITS,
            Kind::RedeemCoin { .. } => 1,
        }
    }
    /// The smallest deposit worth returning when it cannot be acted on.
    pub fn dust(&self) -> u64 {
        match self {
            Kind::Mint { .. } => DUST_REFUND_LAMPORTS,
            _ => DUST_REFUND_UNITS,
        }
    }
    /// Where a returned deposit goes.
    pub fn refund_to(&self) -> [u8; 32] {
        match self {
            Kind::Mint { recipient, .. } => *recipient,
            Kind::Burn { refund_to, .. } | Kind::RedeemCoin { refund_to, .. } => *refund_to,
            Kind::RedeemNft { owner, .. } => *owner,
        }
    }
}

#[derive(CandidType, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// Waiting for a deposit (or for one that reaches the minimum).
    Open,
    /// A finalized deposit is there; the next tick acts on it (retrying a refusal that may pass).
    Funded,
    /// A transaction is out.
    Executing,
    /// An operation of the intent landed and nothing is pending (a later deposit is a new operation until it expires).
    Done,
    /// The deposit went back.
    Refunded,
    /// Nothing was ever deposited and 24 h passed.
    Expired,
}

#[derive(CandidType, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    Execute,
    Refund,
    /// Audit B1-04: `A`'s emptied token account closed, a transaction of its own after a burn or a token refund landed.
    Close,
}

/// The transaction out for an intent.
#[derive(CandidType, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Op {
    pub link: u64,
    pub purpose: Purpose,
    /// The deposit it acts on (lamports or base units).
    pub deposit: u64,
}

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct TxRec {
    pub link: u64,
    pub op: String,
    pub memo: String,
    pub signature: String,
    pub result: String,
    pub at: u64,
}

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Intent {
    pub version: u32,
    pub id: u64,
    pub kind: Kind,
    pub state: State,
    /// `K_int[id]`: where SOL goes (a mint), and the owner of the token account below.
    pub address: [u8; 32],
    /// `A`'s associated token account of the deposited mint (a burn, a redemption).
    pub token_account: Option<[u8; 32]>,
    /// The caller's principal as bytes (the rate limit).
    pub caller: Vec<u8>,
    pub created_at: u64,
    pub expires_at: u64,
    pub funded_at: Option<u64>,
    pub updated_at: u64,
    /// The last finalized deposit seen while no transaction was out (lamports or base units).
    pub deposit: u64,
    /// The deposit goes back whatever it is (it came after expiry, or the intent's purpose is served).
    pub refund_only: bool,
    /// The deposit goes back, and why.
    pub must_refund: Option<String>,
    /// Failed attempts at the current deposit (a transaction that landed failed or never landed).
    pub attempts: u32,
    pub refund_attempts: u32,
    pub op: Option<Op>,
    pub done_ops: u32,
    /// Links begun for this intent (the idempotence key's counter).
    pub seq: u32,
    pub minted_units: u64,
    pub paid_lamports: u64,
    pub ticket: Option<u64>,
    pub history: Vec<TxRec>,
    pub note: String,
    /// The operator must look (a refund that cannot be made, a stuck transaction).
    pub attention: bool,
    /// Audit B1-03: when the desk last tried this intent and it could not go (a refusal that may pass): the pass tries never-skipped
    /// intents first, then the least recently skipped, so a few waiting intents never hold the rest.
    #[serde(default)]
    pub skipped_at: Option<u64>,
    /// Audit B1-04: `A`'s token account is to be closed (a burn or a token refund landed; the close is a later transaction of its own,
    /// so a stranger's dust at `A` can fail only the close, never the operation).
    #[serde(default)]
    pub close_due: bool,
    /// Closes that found `A`'s account not empty or did not land (reset when one lands). At [`MAX_CLOSE_TRIES`] the close is given up.
    #[serde(default)]
    pub close_tries: u32,
}

impl Intent {
    pub fn new(id: u64, kind: Kind, address: [u8; 32], token_account: Option<[u8; 32]>, caller: Vec<u8>, now: u64) -> Intent {
        Intent { version: INTENT_VERSION, id, kind, state: State::Open, address, token_account, caller, created_at: now, expires_at: now + TTL_SECS,
            funded_at: None, updated_at: now, deposit: 0, refund_only: false, must_refund: None, attempts: 0, refund_attempts: 0, op: None, done_ops: 0, seq: 0,
            minted_units: 0, paid_lamports: 0, ticket: None, history: vec![], note: String::new(), attention: false, skipped_at: None,
            close_due: false, close_tries: 0 }
    }

    pub fn expired(&self, now: u64) -> bool {
        now > self.expires_at
    }

    /// The address is still read for deposits: until its grace after expiry ends, and never while a transaction is out.
    pub fn watched(&self, now: u64) -> bool {
        self.state != State::Executing && now <= self.expires_at.saturating_add(REFUND_GRACE_SECS) && !self.attention
    }

    /// Audit B1-05: no deposit was ever seen at the intent (a free opening, live or expired).
    pub fn unfunded(&self) -> bool {
        matches!(self.state, State::Open | State::Expired) && self.deposit == 0 && self.done_ops == 0 && self.funded_at.is_none() && self.history.is_empty()
    }

    /// Audit B1-05: watched, but through the rotation rather than every tick's batch (never funded, and an hour past its expiry).
    pub fn rotated(&self, now: u64) -> bool {
        self.watched(now) && self.unfunded() && now > self.expires_at.saturating_add(UNFUNDED_GRACE_SECS)
    }

    pub fn push_tx(&mut self, rec: TxRec) {
        self.history.push(rec);
        let extra = self.history.len().saturating_sub(HISTORY_KEPT);
        self.history.drain(..extra);
    }

    /// The record is dropped from the state: it is long over.
    pub fn droppable(&self, now: u64) -> bool {
        self.state != State::Executing && now > self.expires_at.saturating_add(REFUND_GRACE_SECS).saturating_add(KEEP_SECS)
    }
}
