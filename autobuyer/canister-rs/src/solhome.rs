//! The Solana home of the Ethereum NFT side (docs/v9/solana-migration.md §2.3, §2.4, §4; phase B1; canister-only since decision D-l,
//! docs/v9/canister-only-solana.md §0). When `Init.home` is `Home::Solana`, the canister keeps the books itself (`rook_core = "books"`,
//! the only home an install accepts): every home step of `ethflow` that moves SOL (the advance, the way back) and Relay's
//! `deposit_native` (`relaysol`) is signed by the canister's threshold Ed25519 keys and sent on a `solsend` line (one durable-nonce
//! account, opened by the line itself with one signature when it is absent); every other home step is a book update, and every home
//! read comes from the books (`ethflow::home_books`, `frogsol::books`). With no `home`, nothing here runs and the EVM home is exactly
//! as before.
//!
//! What this module holds: the money line canister-wide ([`SolLine`], phase B2: shared by `ethflow`, `frogflow` and the custodian, its
//! steps [`LineStep`] queued in order; saved as the upgrade tuple's FOURTEENTH element, moved there at `post_upgrade` from B1's
//! `EthState.sol` by [`migrate`]) and the three lines beside it (W3), the Solana world every flow reads through ([`SolIo`]), the home's
//! configuration ([`SolHomeCfg`]), the money rules in two currencies (the advance in lamports, the NFT in ETH: [`advance_covers`],
//! [`advance_bound`], [`post_price`]), the module as the item rules see it ([`books_sink_view`]), the IC's world ([`SolNet`]: `IcSol`
//! providers, the `IcSigner`, the SOL/ETH and SOL/BTC rates) and the operator's `sol_clear_incident`.
//!
//! Holder deposits stay OFF on this home (docs §2.7: they come back with the EWD2 claim): `ethnft::validate_home` refuses the
//! switch and a `MintDeposit` step is refused here. rook_core has no `fill` (the graduation queue is DBC's, §2.1), so the fill
//! hunt never runs on this home.

use candid::CandidType;
use serde::{Deserialize, Serialize};

use crate::itemsink::{Clock, SinkKind, SinkView};
use crate::rates;
use crate::solrpc::{self, Account, IcSol, SigInfo};
use crate::solsend::{self, LifetimeKind, Line};
use crate::soltx::{Instruction, Pubkey};

/// Kept at our Solana address from each advance for that purchase's own fees there (the Relay deposit, the nonce advance): the
/// Solana counterpart of `GAS_KEEP_WEI` (0.002 SOL ≈ 400 signatures).
pub const SOL_GAS_KEEP_LAMPORTS: u128 = 2_000_000;
/// The advance gate's Relay fee estimate for SOL → ETH (measured 2026-10-09: −0.58 % to −0.69 % with every fee, spike Q5 §1) and its
/// FX margin: the SOL/ETH rate the canister reads (tickers + XRC) against the one Relay fills at, within the quote's TTL. Together
/// 100 bps where ETH → ETH took 50: a floor needs ≈ 0.31 ETH for an advance to cover it on Ethereum (the EVM home's ≈ 0.22).
pub const SOL_ADVANCE_FEE_EST_BPS: u128 = 75;
pub const FX_MARGIN_BPS: u128 = 25;
/// When an amount in one currency bounds one in the other (a payout is never more than its order sent), the rate's own tolerance.
pub const FX_TOLERANCE_BPS: u128 = rates::TOLERANCE_BPS as u128;
/// A ticket redeemed to burn (`Destination::Burn`) is delivered to Ethereum's customary dead address: out of custody for good.
pub const BURN_DESTINATION: [u8; 20] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xde, 0xad];
/// A finalized transaction's projection, at most (the refund memo's attribution).
pub const TX_BYTES: u64 = 24_000;
/// Signatures read per refund attribution, at most.
pub const SIGS_READ: usize = 16;

/// The home's configuration, from `Init.home` (`Home::Solana`): the books home (decision D-l: the canister keeps the books; no
/// program id is accepted), its lines' lifetime and the money line's nonce seed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SolHomeCfg {
    pub lifetime: LifetimeKind,
    pub nonce_seed: String,
}

impl SolHomeCfg {
    /// W8 (canister-only, docs/v9/canister-only-solana.md): `Home::Solana.rook_core` names no program but `"books"`: the canister keeps
    /// the books itself. The only value an install accepts (D-l; `canister::validate_home`).
    pub const BOOKS: &'static str = "books";

    pub fn of(h: &crate::canister::Home) -> Result<SolHomeCfg, String> {
        let crate::canister::Home::Solana { rook_core, nonce_seed, blockhash_fallback, .. } = h;
        if rook_core != Self::BOOKS {
            return Err(format!("home rook_core {rook_core:?}: Rook deploys no Solana programs (decision D-l): the canister keeps the books; only \
                \"books\" is accepted"));
        }
        Ok(SolHomeCfg {
            lifetime: if *blockhash_fallback == Some(true) { LifetimeKind::RecentBlockhash } else { LifetimeKind::DurableNonce },
            nonce_seed: nonce_seed.clone(),
        })
    }
}

/// B1's home step on the line (`EthState.sol.pending`): kept to decode a state saved by B1; [`migrate`] moves it to [`LinePending`].
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SolPending {
    pub step: crate::ethflow::Step,
    pub intent: u64,
    pub at: u64,
}

/// A Relay deposit that landed on the line: its buy (or frog), its order, and its signature (a refund is a credit after it, memoed with
/// the order id: `relaysol::refund_of`).
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SolDepositRec {
    pub buy: u64,
    pub order_id: [u8; 32],
    pub signature: String,
    /// Phase B2: `Some(true)` when `buy` is a frog id (a SOL → BTC conversion); `None` an Ethereum buy (B1's records).
    #[serde(default)]
    pub frog: Option<bool>,
}

/// B1's line, the `sol` field of `ethflow::EthState`: kept only so a state saved by B1 decodes; [`migrate`] moves it to the canister-wide
/// [`SolLine`] at `post_upgrade` and leaves `EthState.sol` empty.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct SolHome {
    pub line: Line,
    pub pending: Option<SolPending>,
    pub next_intent: u64,
    pub opening: Option<u64>,
    pub deposits: Vec<SolDepositRec>,
}

/// The custodian's steps on the line (frog deliveries: `redeem.rs`).
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum CustodyStep {
    /// `record_delivery(txid)` of a frog ticket.
    Record { ticket: u64, txid: [u8; 32] },
    /// `mark_undeliverable` of a refused frog ticket.
    Mark { ticket: u64 },
    /// `note_sent(txid)`: the delivery is on Bitcoin (a mempool at least).
    Sent { ticket: u64, txid: [u8; 32] },
}

/// Who put a step on the line (the queue's turns).
#[derive(CandidType, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineOwner {
    Eth,
    Frog,
    Custody,
    /// W3: the mint/burn desk (intents: §6 of the canister-only design), on its own line ([`LineId::Desk`]).
    Desk,
    /// W3: the pool desk (the wX/SOL pools: §5), on its own line ([`LineId::Pool`]).
    Pool,
    /// W6: the coins' steps (collection opening, graduation backing, leftovers, fee claims and cashing, coin redemption: `coins`), on the
    /// crank line. Never queued: a coin step signs only on a free line (`crankline::line_free`).
    Coin,
    /// W9: the treasury's steps (`cranks`: distributed surplus, the ops share, raffle prizes and claims), on the crank line.
    Treasury,
    /// W8c: the Solana NFT flow's steps (`solnftflow`: the advance + purchase in one transaction, the deliveries), on the money line.
    SolNft,
}

/// W3 (canister-only design §13.1): the FOUR send lines. Each has its own durable-nonce account (seed [`line_seed`]), at most one
/// transaction in flight, and the A4-hardened state machine (`solsend`); they never wait for each other. `K_main` pays the fees and is
/// the nonce authority of all four.
#[derive(CandidType, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LineId {
    /// The money flows: the Ethereum NFT side, the frogs, the custodian (the line of phase B2; the upgrade tuple's 14th element).
    Money,
    /// The cranks: the coins' steps and the treasury's (`coins`, `cranks`; once the registry keeper's, retired with rook_core).
    Crank,
    /// The mint/burn desk (intents).
    Desk,
    /// The pool desk.
    Pool,
}

impl LineId {
    pub const ALL: [LineId; 4] = [LineId::Money, LineId::Crank, LineId::Desk, LineId::Pool];
    pub fn tag(&self) -> &'static str {
        match self {
            LineId::Money => "money",
            LineId::Crank => "crank",
            LineId::Desk => "desk",
            LineId::Pool => "pool",
        }
    }
    /// The line a flow's steps go out on.
    pub fn of(owner: LineOwner) -> LineId {
        match owner {
            LineOwner::Eth | LineOwner::Frog | LineOwner::Custody | LineOwner::SolNft => LineId::Money,
            LineOwner::Coin | LineOwner::Treasury => LineId::Crank,
            LineOwner::Desk => LineId::Desk,
            LineOwner::Pool => LineId::Pool,
        }
    }
    fn index(&self) -> usize {
        LineId::ALL.iter().position(|x| x == self).expect("listed")
    }
}

/// A line's durable-nonce seed (PERMANENT: it is the nonce account's address): the money line keeps the install's `nonce_seed` (the
/// account phase B2 opened); every other line is `"<tag>:<nonce_seed>"`, cut to `create_with_seed`'s 32 bytes.
pub fn line_seed(base: &str, id: LineId) -> String {
    match id {
        LineId::Money => base.to_string(),
        _ => {
            let mut s = format!("{}:{base}", id.tag());
            s.truncate(crate::soltx::MAX_SEED_LEN); // printable ASCII (canister::validate): a byte cut is a char cut
            s
        }
    }
}

/// W3: a step of the desks' lines before the desks name their own typed steps (W5, W7): who, what, and the flow's own id for it.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct LineOp {
    pub owner: LineOwner,
    pub what: String,
    pub id: u64,
}

/// A step on the ONE line, of any flow.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum LineStep {
    Eth(crate::ethflow::Step),
    Frog(crate::frogflow::FStep),
    Custody(CustodyStep),
    /// W3: a step of the mint/burn desk or the pool desk ([`LineOp`]).
    Op(LineOp),
    /// W6: a step of the coins (`coins::CoinStep`), on the crank line.
    Coin(crate::coins::CoinStep),
    /// W7: a step of the pool desk (`pooldesk::PoolStep`), on the pool line.
    Pool(crate::pooldesk::PoolStep),
    /// W9: a step of the treasury (`cranks::TreasuryStep`), on the crank line.
    Treasury(crate::cranks::TreasuryStep),
    /// W8c: a step of the Solana NFT flow (`solnftflow::SolNftStep`), on the money line.
    SolNft(crate::solnftflow::SolNftStep),
}

impl LineStep {
    pub fn owner(&self) -> LineOwner {
        match self {
            LineStep::Eth(_) => LineOwner::Eth,
            LineStep::Frog(_) => LineOwner::Frog,
            LineStep::Custody(_) => LineOwner::Custody,
            LineStep::Op(o) => o.owner,
            LineStep::Coin(_) => LineOwner::Coin,
            LineStep::Pool(_) => LineOwner::Pool,
            LineStep::Treasury(_) => LineOwner::Treasury,
            LineStep::SolNft(_) => LineOwner::SolNft,
        }
    }
    /// For the line's log (`Step`'s Debug spells keys as byte lists).
    pub fn label(&self) -> String {
        let h = |a: &[u8; 20]| format!("0x{}", hex::encode(a));
        use crate::ethflow::Step;
        match self {
            LineStep::Eth(s) => match s {
                Step::Advance { buy } => format!("advance (buy {buy})"),
                Step::Deposit { buy } => format!("Relay deposit_native (buy {buy})"),
                Step::GiveBack { buy } => format!("return_advance / fund_reserve (buy {buy})"),
                Step::Mint { buy } => format!("mint_item (buy {buy})"),
                Step::Settle { buy } => format!("settle_advance (buy {buy})"),
                Step::Record { wrapper, ticket } => format!("record_delivery (wrapper {}, ticket {ticket})", h(wrapper)),
                Step::Mark { wrapper, ticket } => format!("mark_undeliverable (wrapper {}, ticket {ticket})", h(wrapper)),
                Step::Sent { wrapper, ticket } => format!("note_sent (wrapper {}, ticket {ticket})", h(wrapper)),
                Step::Post { collection } => format!("post_floor (collection {})", h(collection)),
                Step::Served { collection, id } => format!("mark_served (collection {}, request {id})", h(collection)),
                s => format!("{s:?}"),
            },
            LineStep::Frog(f) => format!("frogs: {}", f.sol_text()),
            LineStep::Custody(c) => match c {
                CustodyStep::Record { ticket, txid } => format!("frogs: record_delivery (ticket {ticket}, {})", hex::encode(txid)),
                CustodyStep::Mark { ticket } => format!("frogs: mark_undeliverable (ticket {ticket})"),
                CustodyStep::Sent { ticket, txid } => format!("frogs: note_sent (ticket {ticket}, {})", hex::encode(txid)),
            },
            LineStep::Op(o) => format!("{}: {} ({})", LineId::of(o.owner).tag(), o.what, o.id),
            LineStep::Coin(c) => format!("coins: {}", c.label()),
            LineStep::Pool(p) => p.label(),
            LineStep::Treasury(t) => t.label(),
            LineStep::SolNft(s) => s.label(),
        }
    }
}

/// A step out on the line: what it is and the line's intent id (the flow that sent it keeps a copy: `EthState.sol_step`,
/// `FrogState.sol_step`).
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct LinePending {
    pub step: LineStep,
    pub intent: u64,
    pub at: u64,
}

/// Audit A5-03: a Relay `deposit_native` is never signed by the canister's key. The step is three legs on the line: our key funds the
/// dedicated Relay key with exactly the deposit plus its one-signature fee; the Relay key alone signs and pays for the deposit (on a
/// recent blockhash: it is no nonce authority); after anything but a successful deposit (it failed, it was given up, the key held
/// something before) the Relay key's balance is swept back to ours BEFORE the step's outcome is reported, so no balance watch reads it
/// as a refund. The depository, upgradeable by a third party, is thus handed signer privilege over the one deposit only.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct RelayLeg {
    /// The dedicated Relay key.
    pub key: Pubkey,
    /// The deposit, signed by `key` alone.
    pub deposit: Vec<Instruction>,
    /// Lamports the funding moves to `key`: the deposit plus its fee.
    pub fund: u64,
    /// The line intent of the leg's transaction out now (0 while none is signed).
    pub intent: u64,
    pub phase: RelayPhase,
}

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum RelayPhase {
    /// Our key's transfer to the Relay key is out (`RelayLeg.intent`).
    Funding,
    /// The deposit is out, or is signed at the next pass.
    Depositing,
    /// The Relay key's balance goes back to ours; then the step's outcome is `ok` (with `signature`).
    Sweeping { ok: Option<bool>, signature: String },
}

/// What became of a step: landed (`ok`: its on-chain result) or abandoned unlanded (`ok = None`: after an incident, or a simulation
/// that failed at its first send). Kept until its flow takes it.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct LineOutcome {
    pub step: LineStep,
    pub intent: u64,
    pub ok: Option<bool>,
    pub signature: String,
    pub at: u64,
}

/// Outcomes kept for their flows, at most (a flow that never runs again cannot fill the state). Audit C2-03: counted over the outcomes
/// of steps that were SIGNED (they moved SOL, or were abandoned): a book update's outcome ([`is_book_outcome`]) never counts here, so
/// a landed advance is never trimmed to make room for book outcomes.
pub const OUTCOMES_KEPT: usize = 16;
/// Audit C2-03: book updates' outcomes (`books`, `books:<id>`: final at once, re-readable from the books) kept at most, on their own
/// count: one per Inscriptions listing and per chain side outstanding at once, with room to spare.
pub const BOOK_OUTCOMES_KEPT: usize = 64;

/// Audit C2-03: a book update's outcome (the Ethereum side's `books`, an Inscriptions listing's `books:<id>`), not a signed step's.
pub fn is_book_outcome(o: &LineOutcome) -> bool {
    o.signature == "books" || o.signature.starts_with("books:")
}
/// A flow waiting for the line holds its turn this long at most (a turn not taken in two ticks of the runner's cadence is dropped).
pub const TURN_SECS: u64 = 3_600;

/// Audit C2-03: the oldest outcomes of one kind (book updates, or signed steps) past `keep` of that kind dropped; the other kind untouched.
fn trim_outcomes(v: &mut Vec<LineOutcome>, books: bool, keep: usize) {
    let mut extra = v.iter().filter(|o| is_book_outcome(o) == books).count().saturating_sub(keep);
    v.retain(|o| {
        if extra > 0 && is_book_outcome(o) == books {
            extra -= 1;
            return false;
        }
        true
    });
}

/// The ONE Solana line canister-wide (phase B2; docs/v9/solana-migration.md §2.4 "the home line is one line canister-wide"): one
/// durable-nonce account, one transaction in flight, every flow's steps queued on it in the order they asked.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct SolLine {
    pub line: Line,
    pub pending: Option<LinePending>,
    pub next_intent: u64,
    /// The intent that opens the line's durable-nonce account (signed on a recent blockhash: the nonce does not exist yet).
    pub opening: Option<u64>,
    pub deposits: Vec<SolDepositRec>,
    pub outcomes: Vec<LineOutcome>,
    /// The flows waiting for the line, in the order they first asked, with when.
    pub queue: Vec<(LineOwner, u64)>,
    /// Audit A5-03: the pending step's Relay deposit legs, when it is one (the funding from our key, the deposit by the dedicated Relay
    /// key alone, the sweep back). `pending.intent` stays the step's; the leg's own transaction is `RelayLeg.intent`.
    #[serde(default)]
    pub relay: Option<RelayLeg>,
    /// The raffle's published snapshots (`raffle::BOOK`), here ONLY across an upgrade: `pre_upgrade` puts them in, `post_upgrade` takes
    /// them out (the live line never carries them, so its copies stay small). `None` in a state saved before the raffle.
    #[serde(default)]
    pub raffle_book: Option<crate::raffle::RaffleBook>,
}

impl SolLine {
    /// Something of ours is out on the line (one transaction at a time).
    pub fn busy(&self) -> bool {
        self.pending.is_some() || self.line.saved.is_some() || self.opening.is_some()
    }
    /// `owner`'s outcomes, taken.
    pub fn take(&mut self, owner: LineOwner) -> Vec<LineOutcome> {
        let (mine, rest): (Vec<_>, Vec<_>) = self.outcomes.drain(..).partition(|o| o.step.owner() == owner);
        self.outcomes = rest;
        mine
    }
    /// `owner`'s outcomes that `mine` claims, taken (the Ordinals multi-collection change: each Inscriptions collection takes its own
    /// frog outcomes, `frogsol::resolve`); the rest stay for their flows.
    pub fn take_where(&mut self, owner: LineOwner, mine: impl Fn(&LineOutcome) -> bool) -> Vec<LineOutcome> {
        let (mine, rest): (Vec<_>, Vec<_>) = self.outcomes.drain(..).partition(|o| o.step.owner() == owner && mine(o));
        self.outcomes = rest;
        mine
    }
    /// A signed step's outcome. Audit C2-03: the trim drops the oldest SIGNED steps' outcomes past [`OUTCOMES_KEPT`] and never a book
    /// update's, nor counts one (before, every collection's book outcomes counted, so a landed advance could be dropped for them and
    /// then taken back as never landed).
    fn outcome(&mut self, o: LineOutcome) {
        self.outcomes.push(o);
        trim_outcomes(&mut self.outcomes, false, OUTCOMES_KEPT);
    }
    /// Audit C2-03: a book update's outcome (final at once), on its own count ([`BOOK_OUTCOMES_KEPT`]): past it the oldest BOOK outcome
    /// goes, never a signed step's.
    pub fn book_outcome(&mut self, o: LineOutcome) {
        debug_assert!(is_book_outcome(&o));
        self.outcomes.push(o);
        trim_outcomes(&mut self.outcomes, true, BOOK_OUTCOMES_KEPT);
    }
    /// `owner` may sign now: the line is free and no other flow's turn comes first (a turn older than [`TURN_SECS`] is dropped).
    /// Otherwise `owner` takes its place in the queue (once) and the answer says what it waits for.
    pub fn claim(&mut self, owner: LineOwner, now: u64) -> Result<(), String> {
        self.queue.retain(|(o, at)| *o == owner || now.saturating_sub(*at) <= TURN_SECS);
        let busy = self.busy();
        let head = self.queue.first().map(|(o, _)| *o);
        if !busy && head.is_none_or(|h| h == owner) {
            self.queue.retain(|(o, _)| *o != owner);
            return Ok(());
        }
        if !self.queue.iter().any(|(o, _)| *o == owner) {
            self.queue.push((owner, now));
        }
        if busy {
            let what = self.pending.as_ref().map(|p| p.step.label()).or_else(|| self.opening.map(|_| "the nonce account's opening".into()))
                .unwrap_or_else(|| "a transaction".into());
            Err(format!("another transaction of ours is out on the Solana line ({what}; one at a time): {owner:?} waits its turn"))
        } else {
            Err(format!("the Solana line is free and {:?} asked first: {owner:?} waits its turn", head.expect("not none")))
        }
    }
    /// `owner` passed with nothing to sign: its turn is given up.
    pub fn release(&mut self, owner: LineOwner) {
        self.queue.retain(|(o, _)| *o != owner);
    }
}

/// Phase B2's upgrade move (pure): B1's line, kept in `EthState.sol`, becomes the canister-wide one (`saved`: the fourteenth element,
/// `None` in a state saved before it). B1's pending step stays the Ethereum side's (`EthState.sol_step`). Refused when both carry a
/// line (nothing guesses which is real).
pub fn migrate(saved: Option<SolLine>, eth: &mut crate::ethflow::EthState) -> Result<Option<SolLine>, String> {
    let Some(old) = eth.sol.take() else { return Ok(saved) };
    if old == SolHome::default() {
        return Ok(saved);
    }
    if saved.as_ref().is_some_and(|l| *l != SolLine::default()) {
        eth.sol = Some(old);
        return Err("refused: the saved state carries B1's Solana line in EthState.sol AND a canister-wide line: nothing guesses which is \
            real".into());
    }
    let pending = old.pending.map(|p| LinePending { step: LineStep::Eth(p.step), intent: p.intent, at: p.at });
    eth.sol_step = pending.clone();
    Ok(Some(SolLine { line: old.line, pending, next_intent: old.next_intent, opening: old.opening, deposits: old.deposits, outcomes: vec![],
        queue: vec![], relay: None, raffle_book: None }))
}

/// W3: the three lines beside the money line, and what schedules the four: the upgrade tuple's FIFTEENTH element (`None` in a state
/// saved before it; [`migrate_lines`] at `post_upgrade`).
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct SolLines {
    pub crank: Option<SolLine>,
    pub desk: Option<SolLine>,
    pub pool: Option<SolLine>,
    /// Fair scheduling ([`pass_order`]): the line the next pass of the four starts with (`LineId::ALL` index).
    #[serde(default)]
    pub next: Option<u8>,
    /// The canister timer (W14b): `Some(true)` when the operator switched it off (the runner's `tick` still drives everything).
    #[serde(default)]
    pub timer_off: Option<bool>,
    /// The timer as it last ran: what the monitor pages a silent timer on.
    #[serde(default)]
    pub timer: Option<TimerSeen>,
    /// W4/W5: the desk's state (books, pending-ledger, intents) as one versioned blob, here ONLY across an upgrade (`desk::state`:
    /// `pre_upgrade` puts it in, `post_upgrade` takes it out; the live lines never carry it).
    #[serde(default)]
    pub desk_state: Option<Vec<u8>>,
    /// W6: the coins' state (`coins::state`) as one versioned blob, here ONLY across an upgrade, as `desk_state`.
    #[serde(default)]
    pub coins_state: Option<Vec<u8>>,
    /// W7: the pool desk's state (`pooldesk::state`) as one versioned blob, here ONLY across an upgrade, as `desk_state`.
    #[serde(default)]
    pub pool_state: Option<Vec<u8>>,
    /// W9: the cranks' state (`cranks`) as one versioned blob, here ONLY across an upgrade, as `desk_state`.
    #[serde(default)]
    pub cranks_state: Option<Vec<u8>>,
    /// W8c: the Solana NFT flow's state (`solnftflow`) as one versioned blob, here ONLY across an upgrade, as `desk_state`.
    #[serde(default)]
    pub solnft_state: Option<Vec<u8>>,
}

/// The canister timer's last run (W14b).
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct TimerSeen {
    /// When it last ran (unix seconds), and when it runs next.
    pub last_run: u64,
    pub next_at: u64,
    /// Runs since install or upgrade, and runs that found a tick already running (they did nothing).
    pub runs: u64,
    pub skipped: u64,
    /// Whether the last run found work (busy cadence) or none (idle cadence).
    pub busy: bool,
}

impl SolLines {
    /// Line `id` (the money line is not here: [`LINE`]).
    pub fn get(&self, id: LineId) -> Option<&SolLine> {
        match id {
            LineId::Money => None,
            LineId::Crank => self.crank.as_ref(),
            LineId::Desk => self.desk.as_ref(),
            LineId::Pool => self.pool.as_ref(),
        }
    }
    pub fn slot(&mut self, id: LineId) -> Option<&mut Option<SolLine>> {
        match id {
            LineId::Money => None,
            LineId::Crank => Some(&mut self.crank),
            LineId::Desk => Some(&mut self.desk),
            LineId::Pool => Some(&mut self.pool),
        }
    }
}

/// W3's upgrade move (pure): the single line of phase B2 (the 14th element) stays the MONEY line; a turn of another line's owner in its
/// queue leaves it. The registry keeper's crank state, which a pre-W3 line carried (`crank`), is retired with rook_core (D-l): an old
/// encoding still decodes, the field dropped (`tests/sol_lines.rs`). Once: a state saved by this build moves nothing.
pub fn migrate_lines(money: Option<SolLine>, lines: Option<SolLines>) -> Result<(Option<SolLine>, SolLines), String> {
    let lines = lines.unwrap_or_default();
    let Some(mut money) = money else { return Ok((None, lines)) };
    money.queue.retain(|(o, _)| LineId::of(*o) == LineId::Money);
    Ok((Some(money), lines))
}

thread_local! {
    /// The money line (phase B2's canister-wide line). Saved as the upgrade tuple's FOURTEENTH element.
    pub(crate) static LINE: std::cell::RefCell<Option<SolLine>> = const { std::cell::RefCell::new(None) };
    /// W3: the crank, desk and pool lines, the scheduling cursor and the timer. Saved as the upgrade tuple's FIFTEENTH element.
    pub(crate) static LINES: std::cell::RefCell<SolLines> = std::cell::RefCell::new(SolLines::default());
}

/// The money line now (a copy: every flow saves it back at once, before its next await).
pub(crate) fn line_now() -> SolLine {
    LINE.with(|l| l.borrow().clone().unwrap_or_default())
}

pub(crate) fn line_put(l: &SolLine) {
    LINE.with(|x| *x.borrow_mut() = Some(l.clone()));
}

/// Line `id` now (a copy).
pub(crate) fn line_now_of(id: LineId) -> SolLine {
    match id {
        LineId::Money => line_now(),
        _ => LINES.with(|l| l.borrow().get(id).cloned().unwrap_or_default()),
    }
}

pub(crate) fn line_put_of(id: LineId, l: &SolLine) {
    match id {
        LineId::Money => line_put(l),
        _ => LINES.with(|x| {
            if let Some(slot) = x.borrow_mut().slot(id) {
                *slot = Some(l.clone());
            }
        }),
    }
}

/// W3: the four lines for the monitor and the operator, each as [`public_view`] shows it (`None`: never used), with the timer.
#[ic_cdk::query]
pub fn sol_lines() -> (Vec<(LineId, Option<SolLine>)>, Option<TimerSeen>, bool) {
    let all = LineId::ALL.iter().map(|id| {
        let l = match id {
            LineId::Money => LINE.with(|l| l.borrow().clone()),
            _ => LINES.with(|l| l.borrow().get(*id).cloned()),
        };
        (*id, l.map(public_view))
    }).collect();
    let (timer, off) = LINES.with(|l| (l.borrow().timer.clone(), l.borrow().timer_off == Some(true)));
    (all, timer, off)
}

/// W3, fair scheduling (pure): the order the four lines are served in by a pass that starts at `cursor` (round robin), so a line the
/// last pass could not reach (the signing queue full, S3) goes first next time.
pub fn pass_order(cursor: Option<u8>) -> [LineId; 4] {
    let c = cursor.unwrap_or(0) as usize % 4;
    [LineId::ALL[c], LineId::ALL[(c + 1) % 4], LineId::ALL[(c + 2) % 4], LineId::ALL[(c + 3) % 4]]
}

/// The cursor after a pass in `order`: the first line it did not serve (`stopped_at`: its index in `order`), else the line after the
/// first one served (every line moves one place forward each full pass).
pub fn next_cursor(order: &[LineId; 4], stopped_at: Option<usize>) -> u8 {
    match stopped_at {
        Some(i) if i < 4 => order[i].index() as u8,
        _ => ((order[0].index() + 1) % 4) as u8,
    }
}

/// The canister-wide Solana line, for the monitor and the operator (`None`: never used). Audit A4-01: the signed bytes of the transaction
/// in flight are NOT exposed (`saved.tx` is empty here): nobody but the providers needs them before they land, and public bytes let anyone
/// land an intent the line is giving up at a time of their choosing. The signature (its id) is kept.
#[ic_cdk::query]
pub fn sol_line() -> Option<SolLine> {
    LINE.with(|l| l.borrow().clone()).map(public_view)
}

/// What [`sol_line`] shows: the line without the signed bytes in flight.
pub fn public_view(mut l: SolLine) -> SolLine {
    if let Some(s) = l.line.saved.as_mut() {
        s.tx = vec![];
    }
    l.raffle_book = None;
    l
}

// ---- the Solana world every flow reads through --------------------------------------------------------------------------------------

const NOT_SOL: &str = "the Solana home is not configured";

/// Everything a flow may ask of Solana (phase B2: one trait, `EthIo`, `FrogIo` and `CustodyIo` extend it; a world without a Solana
/// home answers "not configured" to all of it). The line is loaded and saved whole ([`SolIo::sol_line`] / [`SolIo::sol_line_save`]):
/// on the IC one thread-local, in the tests one shared cell, so every flow sees the same line.
#[allow(async_fn_in_trait)]
pub trait SolIo {
    fn sol_home(&self) -> Option<SolHomeCfg> {
        None
    }
    async fn sol_me(&mut self) -> Result<Pubkey, String> {
        Err(NOT_SOL.into())
    }
    async fn sol_accounts(&mut self, _keys: &[Pubkey], _max_data: usize) -> Result<Vec<Option<Account>>, String> {
        Err(NOT_SOL.into())
    }
    async fn sol_account_head(&mut self, _key: &Pubkey, _len: usize) -> Result<Option<Account>, String> {
        Err(NOT_SOL.into())
    }
    async fn sol_program_accounts(&mut self, _program: &Pubkey, _size: usize, _offset: usize, _bytes: &[u8], _max: usize)
        -> Result<Vec<(Pubkey, Account)>, String> {
        Err(NOT_SOL.into())
    }
    /// `getProgramAccounts` returning `(address, owner, slice)` of every match (`solrpc::program_slices`).
    async fn sol_program_slices(&mut self, _program: &Pubkey, _size: usize, _memcmps: &[(usize, Vec<u8>)], _slice: (usize, usize), _max: usize)
        -> Result<Vec<(Pubkey, Pubkey, Vec<u8>)>, String> {
        Err(NOT_SOL.into())
    }
    async fn sol_rent(&mut self, _space: u64) -> Result<u64, String> {
        Err(NOT_SOL.into())
    }
    /// `len` bytes from `offset` of several accounts (`solrpc::accounts_slice`).
    async fn sol_accounts_slice(&mut self, _keys: &[Pubkey], _offset: usize, _len: usize) -> Result<Vec<Option<Account>>, String> {
        Err(NOT_SOL.into())
    }
    /// `owner`'s token accounts of `program`, `slice` of each (`solrpc::token_accounts`).
    async fn sol_token_accounts(&mut self, _owner: &Pubkey, _program: &Pubkey, _slice: (usize, usize), _max: usize)
        -> Result<Vec<(Pubkey, Account)>, String> {
        Err(NOT_SOL.into())
    }
    /// Audit A5-03: the dedicated Relay deposit key (a second threshold Ed25519 derivation path, `solkey::RELAY_DERIVATION`).
    async fn sol_relay(&mut self) -> Result<Pubkey, String> {
        Err(NOT_SOL.into())
    }
    /// W8: the address of one of the canister's key paths (`solkey`): the threshold key's by default; a fake's in tests.
    async fn sol_key_of(&mut self, p: &crate::solkey::KeyPath) -> Result<Pubkey, String> {
        crate::solkey::derive_key(p).await
    }
    /// Any address's lamports at finalized (the Relay key's, for its sweep back).
    async fn sol_balance_of(&mut self, _key: &Pubkey) -> Result<u128, String> {
        Err(NOT_SOL.into())
    }
    /// SOL/ETH: lamports per ETH (`oracle::read_sol_rate`), read once a tick.
    async fn sol_rate(&mut self) -> Result<u64, String> {
        Err(NOT_SOL.into())
    }
    /// SOL/BTC: satoshis per SOL (`oracle::read_sol_btc_rate`), read once a tick.
    async fn sol_btc_rate(&mut self) -> Result<u64, String> {
        Err(NOT_SOL.into())
    }
    /// Our Solana address's lamports at finalized.
    async fn sol_balance(&mut self) -> Result<u128, String> {
        Err(NOT_SOL.into())
    }
    /// The finalized clock `(unix seconds, slot)`.
    async fn sol_clock(&mut self) -> Result<(u64, u64), String> {
        Err(NOT_SOL.into())
    }
    async fn sol_prepare(&mut self, _line: &mut Line, _kind: LifetimeKind, _intent: u64, _what: &str, _ixs: Vec<Instruction>) -> Result<(), String> {
        Err(NOT_SOL.into())
    }
    async fn sol_tick(&mut self, _line: &mut Line) -> Result<solsend::Step, String> {
        Err(NOT_SOL.into())
    }
    async fn sol_signatures(&mut self, _until: &str, _limit: usize) -> Result<Vec<SigInfo>, String> {
        Err(NOT_SOL.into())
    }
    /// Audit B1-12: the page of signatures older than `before` and newer than `until` (`getSignaturesForAddress` with both bounds).
    async fn sol_signatures_before(&mut self, _until: &str, _before: &str, _limit: usize) -> Result<Vec<SigInfo>, String> {
        Err(NOT_SOL.into())
    }
    async fn sol_transaction(&mut self, _sig: &str) -> Result<Option<serde_json::Value>, String> {
        Err(NOT_SOL.into())
    }
    /// The cluster's genesis hash, agreed (the `assert_chain` of the Solana home).
    async fn sol_genesis(&mut self) -> Result<String, String> {
        Err(NOT_SOL.into())
    }
    /// The canister-wide line (a copy).
    fn sol_line(&mut self) -> SolLine {
        SolLine::default()
    }
    /// Save the line, durably, before the next await.
    fn sol_line_save(&mut self, _l: &SolLine) {}
    /// W3: line `id` (a copy). The money line is [`SolIo::sol_line`]; a world that holds no other line answers an empty one.
    fn sol_line_of(&mut self, id: LineId) -> SolLine {
        if id == LineId::Money { self.sol_line() } else { SolLine::default() }
    }
    /// W3: save line `id`, durably, before the next await (a world that holds no other line keeps nothing of it).
    fn sol_line_save_of(&mut self, id: LineId, l: &SolLine) {
        if id == LineId::Money {
            self.sol_line_save(l)
        }
    }
    /// W3: [`SolIo::sol_prepare`] for an intent signed by `K_main` and the canister keys `keys` (`solsend::prepare_keys`).
    async fn sol_prepare_keys(&mut self, line: &mut Line, kind: LifetimeKind, intent: u64, what: &str, ixs: Vec<Instruction>,
        keys: Vec<crate::solkey::KeyPath>) -> Result<(), String> {
        if keys.is_empty() {
            return self.sol_prepare(line, kind, intent, what, ixs).await;
        }
        Err("this world signs with K_main alone".into())
    }
    /// 2026-10-12: [`SolIo::sol_prepare_keys`] for an intent sent as a v0 message loading keys from `tables`, read by the canister
    /// (`solsend::prepare_tables`). No tables: a legacy message.
    #[allow(clippy::too_many_arguments)]
    async fn sol_prepare_tables(&mut self, line: &mut Line, kind: LifetimeKind, intent: u64, what: &str, ixs: Vec<Instruction>,
        keys: Vec<crate::solkey::KeyPath>, tables: Vec<crate::soltx::LookupTable>) -> Result<(), String> {
        if tables.is_empty() {
            return self.sol_prepare_keys(line, kind, intent, what, ixs, keys).await;
        }
        Err("this world signs no v0 message".into())
    }
}

/// The line's durable-nonce account: adopted when it exists and is ours (an opening that landed while its record did not survive),
/// else opened by the line itself (`create_with_seed` + `initialize_nonce`, one signature, on a recent blockhash). Audit A4-02: an
/// address that already holds a System-owned account with NO data (a transfer anyone can make to it) is opened in place
/// (`AllocateWithSeed` + a top-up + `initialize_nonce`, still signed by us alone); a nonce account there with another authority, or any
/// other account, is refused for the operator. `Ok(true)`: the line has its nonce account now.
pub async fn open_line<I: SolIo>(io: &mut I, cfg: &SolHomeCfg, me: Pubkey) -> Result<bool, String> {
    open_line_on(io, LineId::Money, cfg, me).await
}

/// [`open_line`] for line `id` (its own seed: [`line_seed`]).
pub async fn open_line_on<I: SolIo>(io: &mut I, id: LineId, cfg: &SolHomeCfg, me: Pubkey) -> Result<bool, String> {
    let seed = line_seed(&cfg.nonce_seed, id);
    let addr = solsend::nonce_account_of(&me, &seed)?;
    let a = io.sol_accounts(&[addr], crate::soltx::NONCE_ACCOUNT_SPACE as usize).await?;
    let (ixs, what) = match a[0].as_ref() {
        Some(acc) if acc.owner == crate::soltx::SYSTEM_PROGRAM && acc.data.is_empty() && !acc.executable => {
            let r = io.sol_rent(crate::soltx::NONCE_ACCOUNT_SPACE).await?;
            (solsend::open_squatted_nonce_ixs(&me, &seed, r, acc.lamports)?,
                format!("open the {} line's durable-nonce account in place (its address already held {} lamports, no data)", id.tag(), acc.lamports))
        }
        Some(acc) => {
            let ours = acc.owner == crate::soltx::SYSTEM_PROGRAM && crate::soltx::decode_nonce(&acc.data).is_ok_and(|n| n.authority == me);
            if !ours {
                return Err(format!("the {} line's nonce account {} exists and is not an initialized nonce account of ours: an operator looks",
                    id.tag(), crate::soltx::b58_encode(&addr)));
            }
            let mut l = io.sol_line_of(id);
            l.line.nonce_account = Some(addr);
            io.sol_line_save_of(id, &l);
            return Ok(true);
        }
        None => {
            let r = io.sol_rent(crate::soltx::NONCE_ACCOUNT_SPACE).await?;
            (solsend::open_nonce_ixs(&me, &seed, r)?, format!("open the {} line's durable-nonce account", id.tag()))
        }
    };
    let mut l = io.sol_line_of(id);
    let intent = l.next_intent;
    let r = io.sol_prepare(&mut l.line, LifetimeKind::RecentBlockhash, intent, &what, ixs).await;
    if let Err(e) = r {
        io.sol_line_save_of(id, &l);
        return Err(e);
    }
    l.next_intent += 1;
    l.opening = Some(intent);
    io.sol_line_save_of(id, &l);
    let mut l = io.sol_line_of(id);
    let _ = io.sol_tick(&mut l.line).await;
    io.sol_line_save_of(id, &l);
    Ok(false)
}

/// May `owner` sign on the ONE line now (no outcall)? The incident and the turn ([`SolLine::claim`]: one step in flight canister-wide,
/// the flows in the order they asked; a refusal queues `owner`). Audit A5-07: a flow asks this BEFORE it consumes a quote or pays for
/// the reads that judge it.
pub fn turn<I: SolIo>(io: &mut I, owner: LineOwner, now: u64) -> Result<(), String> {
    let id = LineId::of(owner);
    let mut l = io.sol_line_of(id);
    if let Some(i) = &l.line.incident {
        return Err(format!("the Solana {} line has an open INCIDENT, nothing is signed on it until the operator clears it (sol_clear_incident): {i}",
            if id == LineId::Money { "home" } else { id.tag() }));
    }
    let c = l.claim(owner, now);
    io.sol_line_save_of(id, &l);
    c
}

/// May `owner` sign on the ONE line now? The incident, the turn ([`turn`]) and the durable-nonce account (opened by the line itself
/// when absent: then not this tick). `Ok(me)`: build the instructions and [`sign`] them. The pause is the caller's to check.
pub async fn ready<I: SolIo>(io: &mut I, owner: LineOwner, now: u64, what: &str) -> Result<Pubkey, String> {
    let cfg = io.sol_home().ok_or(NOT_SOL)?;
    let id = LineId::of(owner);
    turn(io, owner, now)?;
    let me = io.sol_me().await?;
    if cfg.lifetime == LifetimeKind::DurableNonce && io.sol_line_of(id).line.nonce_account.is_none() && !open_line_on(io, id, &cfg, me).await? {
        return Err(format!("the {} line is opening its durable-nonce account (one signature, sent now): {what} waits for it",
            if id == LineId::Money { "home" } else { id.tag() }));
    }
    Ok(me)
}

/// Audit A5-03: `ixs` is a Relay deposit of the dedicated Relay key (`relaysol::deposit_instruction(&relay, ..)`): its amount.
fn relay_deposit(ixs: &[Instruction], relay: &Pubkey) -> Option<u64> {
    let [ix] = ixs else { return None };
    if ix.program != crate::soltx::key(crate::relaysol::DEPOSITORY_PROGRAM) || !ix.accounts.iter().any(|a| a.signer && a.key == *relay) {
        return None;
    }
    crate::relaysol::decode_deposit_native(&ix.data).map(|(amount, _)| amount)
}

/// Sign `step` on the line and save it before its first send (`solsend::prepare`, which simulates first: a step that would fail is
/// refused unsigned), then send it. Called after [`ready`]. Returns the log text and the pending record (the flow keeps a copy).
/// A Relay deposit (audit A5-03) goes out as its first leg: our key's funding of the Relay key ([`RelayLeg`]); or, when the Relay key
/// holds anything already, its sweep back first (the step then ends unlanded, after the sweep).
pub async fn sign<I: SolIo>(io: &mut I, step: LineStep, now: u64, ixs: Vec<Instruction>) -> Result<(String, LinePending), String> {
    sign_keys(io, step, now, ixs, vec![]).await
}

/// W3: [`sign`] for a step signed by `K_main` and the canister keys `keys` (the owner of the money it moves: `K_coll[i]`, `K_int[n]`, …),
/// on the step's own line ([`LineId::of`] its owner). Every signer of the compiled message must be one of them, or nothing is signed.
pub async fn sign_keys<I: SolIo>(io: &mut I, step: LineStep, now: u64, ixs: Vec<Instruction>, keys: Vec<crate::solkey::KeyPath>)
    -> Result<(String, LinePending), String> {
    sign_tables(io, step, now, ixs, keys, vec![]).await
}

/// 2026-10-12: [`sign_keys`] for a step sent as a v0 message loading keys from `tables` (read by the canister at finalized, owned by
/// `K_main`: `solnftflow::table`); no tables: a legacy message.
pub async fn sign_tables<I: SolIo>(io: &mut I, step: LineStep, now: u64, ixs: Vec<Instruction>, keys: Vec<crate::solkey::KeyPath>,
    tables: Vec<crate::soltx::LookupTable>) -> Result<(String, LinePending), String> {
    let cfg = io.sol_home().ok_or(NOT_SOL)?;
    let what = step.label();
    let id = LineId::of(step.owner());
    if let LineStep::Op(o) = &step {
        if !matches!(o.owner, LineOwner::Desk | LineOwner::Pool) {
            return Err(format!("{what}: refused: an Op step is the desks' (Desk or Pool), not {:?}'s", o.owner));
        }
    }
    if io.sol_line_of(id).busy() {
        return Err(format!("{what}: the Solana {} line is busy (one at a time)", id.tag()));
    }
    let mut leg = None;
    let mut ixs = ixs;
    let mut kind = cfg.lifetime;
    let mut note = String::new();
    if ixs.first().is_some_and(|i| i.program == crate::soltx::key(crate::relaysol::DEPOSITORY_PROGRAM)) {
        let relay = io.sol_relay().await?;
        let amount = relay_deposit(&ixs, &relay)
            .ok_or_else(|| format!("{what}: refused: a Relay deposit is signed by the dedicated Relay key alone (audit A5-03)"))?;
        let me = io.sol_me().await?;
        let held = io.sol_balance_of(&relay).await?;
        let fund = amount.checked_add(solsend::SIGNATURE_FEE_LAMPORTS).ok_or("the deposit overflows")?;
        if held > solsend::SIGNATURE_FEE_LAMPORTS as u128 {
            let back = u64::try_from(held).unwrap_or(u64::MAX) - solsend::SIGNATURE_FEE_LAMPORTS;
            note = format!(" (the Relay key held {held} lamports: they are swept back first and this deposit is not made)");
            leg = Some(RelayLeg { key: relay, deposit: ixs.clone(), fund, intent: 0, phase: RelayPhase::Sweeping { ok: None, signature: String::new() } });
            ixs = vec![crate::soltx::transfer(relay, me, back)];
            kind = LifetimeKind::RecentBlockhash;
        } else {
            leg = Some(RelayLeg { key: relay, deposit: ixs.clone(), fund, intent: 0, phase: RelayPhase::Funding });
            ixs = vec![crate::soltx::transfer(me, relay, fund)];
        }
    }
    if leg.is_some() && (!keys.is_empty() || !tables.is_empty()) {
        return Err(format!("{what}: refused: a Relay deposit is signed by the dedicated Relay key alone (audit A5-03)"));
    }
    let mut l = io.sol_line_of(id);
    let intent = l.next_intent;
    let r = io.sol_prepare_tables(&mut l.line, kind, intent, &what, ixs, keys, tables).await;
    if let Err(e) = r {
        io.sol_line_save_of(id, &l);
        return Err(e);
    }
    l.next_intent += 1;
    if let Some(x) = leg.as_mut() {
        x.intent = intent;
    }
    let relayed = leg.is_some();
    let p = LinePending { step, intent, at: now };
    l.pending = Some(p.clone());
    l.relay = leg;
    io.sol_line_save_of(id, &l);
    let mut l = io.sol_line_of(id);
    let first = io.sol_tick(&mut l.line).await;
    let sig = l.line.saved.as_ref().map(|x| x.signature.clone()).unwrap_or_default();
    io.sol_line_save_of(id, &l);
    let what = if relayed { format!("{what}: its funding of the Relay key{note}") } else { what };
    Ok((match first {
        Ok(solsend::Step::Preflight(e)) => format!("{what} signed and saved ({sig}); its first send was refused: {e}"),
        Ok(_) => format!("{what} signed, saved and sent on the Solana {} line (intent {intent}, {sig})", id.tag()),
        Err(e) => format!("{what} signed and saved (intent {intent}, {sig}); its first send waits: {e}"),
    }, p))
}

/// The refund's label (Q5 §5.2): a finalized credit to our Solana address after the deposit, memoed `0x<order id>`. The balance rule
/// decided the refund; this only says which order the memo names (an unauthenticated label: it decides nothing). Empty when unread.
pub async fn refund_label<I: SolIo>(io: &mut I, order_id: &[u8; 32]) -> String {
    let Some(dep) = io.sol_line().deposits.iter().rev().find(|d| d.order_id == *order_id).cloned() else { return String::new() };
    let Ok(me) = io.sol_me().await else { return String::new() };
    let ours = crate::soltx::b58_encode(&me);
    let Ok(sigs) = io.sol_signatures(&dep.signature, SIGS_READ).await else { return String::new() };
    for s in sigs.iter().filter(|s| !s.failed) {
        if let Ok(Some(tx)) = io.sol_transaction(&s.signature).await {
            if let Ok(Some(l)) = crate::relaysol::refund_of(&tx, &ours, order_id) {
                return format!(" (Relay's refund memoed with this order: {l} lamports in {})", s.signature);
            }
        }
    }
    " (no transfer memoed with this order found yet: the balance rule decides)".into()
}

/// `owner` passed with nothing to sign: its turn in the queue is given up.
pub fn release<I: SolIo>(io: &mut I, owner: LineOwner) {
    let id = LineId::of(owner);
    let mut l = io.sol_line_of(id);
    if l.queue.iter().any(|(o, _)| *o == owner) {
        l.release(owner);
        io.sol_line_save_of(id, &l);
    }
}

/// The line, once a flow's pass: what landed (kept as an outcome for the flow that sent it), what is still out, an incident, a
/// preflight refusal, a given-up intent's nonce burn, the nonce account's opening, a Relay deposit's next leg. `None` when nothing is out.
pub async fn tick_line<I: SolIo>(io: &mut I, now: u64) -> Option<String> {
    tick_line_on(io, LineId::Money, now).await
}

/// [`tick_line`] for line `id`. The text of a line other than the money line names it (`sol line [crank]: …`).
pub async fn tick_line_on<I: SolIo>(io: &mut I, id: LineId, now: u64) -> Option<String> {
    let t = tick_line_inner(io, id, now).await?;
    Some(if id == LineId::Money { t } else { t.replacen("sol line", &format!("sol line [{}]", id.tag()), 1) })
}

async fn tick_line_inner<I: SolIo>(io: &mut I, id: LineId, now: u64) -> Option<String> {
    let l = io.sol_line_of(id);
    if !l.busy() {
        return None;
    }
    // audit A5-03: a Relay leg whose next transaction is not signed yet (the deposit after its funding, the sweep back)
    if l.line.saved.is_none() && l.line.incident.is_none() && l.pending.is_some() && l.relay.is_some() {
        return Some(relay_next(io, id, now).await);
    }
    let mut l = l;
    let r = io.sol_tick(&mut l.line).await;
    io.sol_line_save_of(id, &l);
    let me = io.sol_me().await.ok();
    let seed = io.sol_home().map(|c| line_seed(&c.nonce_seed, id)).unwrap_or_default();
    let mut l = io.sol_line_of(id);
    let opening = l.opening;
    let pending = l.pending.clone();
    // the leg of a Relay deposit this landing is (audit A5-03)
    let relay = l.relay.clone();
    let leg_of = |intent: u64| relay.clone().filter(|r| r.intent == intent && pending.is_some());
    let mut relay_again = false;
    let text = match r {
        Err(e) => format!("sol line: waits: {e}"),
        Ok(solsend::Step::Idle) => {
            // nothing on the line for a pending step: its intent was abandoned by the operator after an incident (none of its
            // signatures known on chain, re-read at the clear: NEW-A4-01), so the step did not happen and is taken back by its flow
            l.opening = None;
            l.relay = None;
            match l.pending.take() {
                Some(p) => {
                    let t = format!("sol line: {} was abandoned after an incident (it never landed): taken back by its flow", p.step.label());
                    l.outcome(LineOutcome { step: p.step, intent: p.intent, ok: None, signature: String::new(), at: now });
                    t
                }
                None => "sol line: idle".into(),
            }
        }
        Ok(solsend::Step::Landed(d)) | Ok(solsend::Step::Abandoned(d)) => {
            let abandoned = d.abandoned == Some(true);
            let ok = !abandoned && d.err.is_none();
            if opening == Some(d.intent) {
                l.opening = None;
                if ok {
                    let addr = me.and_then(|m| solsend::nonce_account_of(&m, &seed).ok());
                    l.line.nonce_account = addr;
                    format!("sol line: the durable-nonce account {} is open ({})", addr.map(|a| crate::soltx::b58_encode(&a)).unwrap_or_default(),
                        d.signature)
                } else if abandoned {
                    "sol line: opening the durable-nonce account was given up (proven never to land): tried again at the next step".into()
                } else {
                    format!("sol line: opening the durable-nonce account FAILED ({}): tried again at the next step", d.err.unwrap_or_default())
                }
            } else if let (Some(leg), Some(p)) = (leg_of(d.intent), pending.clone()) {
                // audit A5-03: a leg of a Relay deposit
                let label = p.step.label();
                let t = match leg.phase {
                    RelayPhase::Funding if ok => {
                        l.relay = Some(RelayLeg { phase: RelayPhase::Depositing, intent: 0, ..leg });
                        relay_again = true;
                        format!("sol line: {label}: the Relay key is funded ({}): the deposit is signed by it next", d.signature)
                    }
                    RelayPhase::Funding => {
                        // nothing moved: the step did not happen
                        let o = if abandoned { None } else { Some(false) };
                        l.outcome(LineOutcome { step: p.step.clone(), intent: p.intent, ok: o, signature: d.signature.clone(), at: now });
                        l.pending = None;
                        l.relay = None;
                        format!("sol line: {label}: its funding of the Relay key {}: taken back by its flow",
                            if abandoned { "never landed (given up)".to_string() } else { format!("FAILED on chain ({})", d.signature) })
                    }
                    RelayPhase::Depositing if ok => {
                        l.outcome(LineOutcome { step: p.step.clone(), intent: p.intent, ok: Some(true), signature: d.signature.clone(), at: now });
                        l.pending = None;
                        l.relay = None;
                        format!("sol line: {label} landed (success, {}), signed by the Relay key alone", d.signature)
                    }
                    RelayPhase::Depositing => {
                        let o = if abandoned { None } else { Some(false) };
                        l.relay = Some(RelayLeg { phase: RelayPhase::Sweeping { ok: o, signature: d.signature.clone() }, intent: 0, ..leg });
                        relay_again = true;
                        format!("sol line: {label}: the deposit {}: the Relay key is swept back before its flow is told",
                            if abandoned { "never landed (given up)".to_string() } else { format!("FAILED on chain ({})", d.signature) })
                    }
                    RelayPhase::Sweeping { .. } => {
                        l.relay = Some(RelayLeg { intent: 0, ..leg });
                        relay_again = true;
                        format!("sol line: {label}: the sweep back of the Relay key {} ({})", if ok { "landed" } else { "did not land" }, d.signature)
                    }
                };
                t
            } else if let Some(p) = pending.filter(|p| p.intent == d.intent && relay.is_none()) {
                l.pending = None;
                let o = if abandoned { None } else { Some(ok) };
                let t = format!("sol line: {} {}", p.step.label(), match o {
                    Some(true) => format!("landed (success, {})", d.signature),
                    Some(false) => format!("landed (FAILED on chain, {})", d.signature),
                    None => format!("was given up and is PROVEN never to land ({}): taken back by its flow",
                        if d.signature.is_empty() { "expired".into() } else { format!("its nonce burned by {}", d.signature) }),
                });
                l.outcome(LineOutcome { step: p.step, intent: p.intent, ok: o, signature: d.signature.clone(), at: now });
                t
            } else {
                format!("sol line: intent {} ({}) {} and is no step of ours now", d.intent, d.what, if abandoned { "was given up" } else { "landed" })
            }
        }
        Ok(solsend::Step::Pending { intent, sends }) => format!("sol line: intent {intent} sent {sends} time(s), not finalized yet{}",
            pending.map(|p| format!(" ({})", p.step.label())).unwrap_or_default()),
        Ok(solsend::Step::Resigned { intent, signature }) => format!("sol line: intent {intent}'s blockhash expired unlanded: re-signed as {signature}"),
        Ok(solsend::Step::Incident(why)) => format!("sol line: INCIDENT (nothing more is signed; the operator looks and clears it: sol_clear_incident): {why}"),
        Ok(solsend::Step::Burning { intent, signature }) => format!("sol line: intent {intent} is given up: its nonce burn {signature} is signed \
            (whichever of the two lands is its outcome)"),
        // audit A4-01: a refusal by every provider's preflight decides nothing; it is the line's error (and the intent is given up)
        Ok(solsend::Step::Preflight(e)) => format!("sol line: ERROR: {e}"),
    };
    io.sol_line_save_of(id, &l);
    if relay_again {
        let t = relay_next(io, id, now).await;
        return Some(format!("{text}; {t}"));
    }
    Some(text)
}

/// W3: one pass over the four lines in fair order ([`pass_order`] from `cursor`): every line with something out is ticked once (land,
/// resend, expiry, incident, a Relay leg, the nonce account's opening: [`tick_line_on`]). A line whose tick met a full signing queue
/// ([`crate::solkey::QUEUE_FULL`]) ends the pass: the lines after it wait, and go FIRST next pass. Lines are independent: one line's
/// incident, error or busy transaction never holds another. Returns the log, the next cursor, and whether anything is still out or
/// queued on any line (the timer's busy cadence).
pub async fn tick_lines<I: SolIo>(io: &mut I, cursor: Option<u8>, now: u64) -> (Vec<String>, u8, bool) {
    let order = pass_order(cursor);
    let mut out = Vec::new();
    let mut stopped = None;
    for (i, id) in order.iter().enumerate() {
        if !io.sol_line_of(*id).busy() {
            continue;
        }
        if let Some(t) = tick_line_on(io, *id, now).await {
            let full = crate::solkey::is_queue_full(&t);
            out.push(t);
            if full {
                stopped = Some(i + 1);
                if i + 1 < order.len() {
                    out.push(format!("sol lines: the signing queue is full: {} wait for the next pass (they go first)",
                        order[i + 1..].iter().map(|x| x.tag()).collect::<Vec<_>>().join(", ")));
                }
                break;
            }
        }
    }
    // an open incident is not work the timer can do (only the operator's clear is): it does not hold the busy cadence; a turn older
    // than TURN_SECS is dropped here as `claim` drops it (audit B2-03: a stale turn held the busy cadence for ever)
    let busy = LineId::ALL.iter().any(|id| {
        let mut l = io.sol_line_of(*id);
        let n = l.queue.len();
        l.queue.retain(|(_, at)| now.saturating_sub(*at) <= TURN_SECS);
        if l.queue.len() != n {
            io.sol_line_save_of(*id, &l);
        }
        (l.busy() && l.line.incident.is_none()) || !l.queue.is_empty()
    });
    (out, next_cursor(&order, stopped), busy)
}

/// Audit A5-03: sign a Relay deposit's next leg (the line is free of saved bytes): the deposit by the Relay key alone; or, when sweeping,
/// the Relay key's balance back to ours, and once it holds nothing the step's outcome for its flow.
async fn relay_next<I: SolIo>(io: &mut I, id: LineId, now: u64) -> String {
    let l = io.sol_line_of(id);
    let Some(p) = l.pending.clone() else { return "sol line: idle".into() };
    let Some(leg) = l.relay.clone() else { return "sol line: no Relay leg".into() };
    let label = p.step.label();
    let (ixs, sweeping) = match &leg.phase {
        RelayPhase::Funding => {
            // its funding left the line unlanded (the operator's clear after an incident, every signature proven unknown): nothing moved
            let mut l = io.sol_line_of(id);
            l.pending = None;
            l.relay = None;
            l.outcome(LineOutcome { step: p.step.clone(), intent: p.intent, ok: None, signature: String::new(), at: now });
            io.sol_line_save_of(id, &l);
            return format!("sol line: {label}: its funding of the Relay key was abandoned after an incident (it never landed): taken back by its flow");
        }
        RelayPhase::Depositing => (leg.deposit.clone(), false),
        RelayPhase::Sweeping { ok, signature } => {
            let held = match io.sol_balance_of(&leg.key).await {
                Ok(h) => h,
                Err(e) => return format!("sol line: {label}: the Relay key's balance is not read ({e}): its sweep waits"),
            };
            if held <= solsend::SIGNATURE_FEE_LAMPORTS as u128 {
                let mut l = io.sol_line_of(id);
                l.pending = None;
                l.relay = None;
                l.outcome(LineOutcome { step: p.step.clone(), intent: p.intent, ok: *ok, signature: signature.clone(), at: now });
                io.sol_line_save_of(id, &l);
                return format!("sol line: {label}: the Relay key is empty ({held} lamports): {} for its flow", match ok {
                    Some(true) => "landed",
                    Some(false) => "FAILED on chain",
                    None => "never landed",
                });
            }
            let me = match io.sol_me().await {
                Ok(m) => m,
                Err(e) => return format!("sol line: {label}: {e}: the sweep waits"),
            };
            let back = u64::try_from(held).unwrap_or(u64::MAX) - solsend::SIGNATURE_FEE_LAMPORTS;
            (vec![crate::soltx::transfer(leg.key, me, back)], true)
        }
    };
    let mut l = io.sol_line_of(id);
    let intent = l.next_intent;
    let what = format!("{label}: {}", if sweeping { "the Relay key's sweep back" } else { "the deposit by the Relay key" });
    let r = io.sol_prepare(&mut l.line, LifetimeKind::RecentBlockhash, intent, &what, ixs).await;
    if let Err(e) = r {
        if !sweeping && e.starts_with(solsend::SIMULATION_REFUSED) {
            // the deposit would fail now (the depository refuses it): nothing is signed; the Relay key is swept back and the step taken back
            if let Some(x) = l.relay.as_mut() {
                x.phase = RelayPhase::Sweeping { ok: None, signature: String::new() };
            }
            io.sol_line_save_of(id, &l);
            return format!("sol line: {label}: the deposit is refused by simulation ({e}): the Relay key is swept back, the step taken back after");
        }
        io.sol_line_save_of(id, &l);
        return format!("sol line: {label}: {what} waits: {e}");
    }
    l.next_intent += 1;
    if let Some(x) = l.relay.as_mut() {
        x.intent = intent;
    }
    io.sol_line_save_of(id, &l);
    let mut l = io.sol_line_of(id);
    let first = io.sol_tick(&mut l.line).await;
    let sig = l.line.saved.as_ref().map(|x| x.signature.clone()).unwrap_or_default();
    io.sol_line_save_of(id, &l);
    match first {
        Ok(solsend::Step::Preflight(e)) => format!("sol line: {what} signed and saved ({sig}); its first send was refused: {e}"),
        Ok(_) => format!("sol line: {what} signed, saved and sent (intent {intent}, {sig})"),
        Err(e) => format!("sol line: {what} signed and saved (intent {intent}, {sig}); its first send waits: {e}"),
    }
}

// ---- money in two currencies ---------------------------------------------------------------------------------------------------

/// The advance gate on the Solana home (A2-M-1 in two currencies): an advance of `cap` lamports reaches Ethereum, after the Solana
/// keep, Relay's SOL → ETH fee estimate and the FX margin, as at least the module's accepted floor (`accepted_wei`, in ETH) plus
/// `ADVANCE_MARGIN_BPS` and the Ethereum gas reserve.
pub fn advance_covers(cap: u128, accepted_wei: u128, lamports_per_eth: u64) -> Result<(), String> {
    if accepted_wei == 0 {
        return Err("the module has no accepted floor".into());
    }
    let cut = cap.saturating_mul(SOL_ADVANCE_FEE_EST_BPS + FX_MARGIN_BPS) / 10_000;
    let arrive = rates::lamports_to_wei(cap.saturating_sub(SOL_GAS_KEEP_LAMPORTS).saturating_sub(cut), lamports_per_eth);
    let need = accepted_wei.saturating_mul(10_000 + crate::ethflow::ADVANCE_MARGIN_BPS) / 10_000 + crate::ethnft::ETH_GAS_RESERVE_WEI;
    if arrive < need {
        return Err(format!("an advance of {cap} lamports would reach Ethereum as about {arrive} wei (SOL/ETH {lamports_per_eth} lamports \
            per ETH, less the Solana keep, Relay's fee and the FX margin), under the module's floor {accepted_wei} wei plus {} % and the gas \
            reserve ({need}): the pool waits on Solana (A2-M-1)", crate::ethflow::ADVANCE_MARGIN_BPS / 100));
    }
    Ok(())
}

/// The buy's advance on the Solana home (A2-L-1 in two currencies): the buy's bound in ETH (our fresh reading, never above the
/// module's accepted floor in ETH, plus the premium, within `max_purchase_wei`) plus the Ethereum gas reserve, grossed up for the
/// judge's Relay fee bound, in lamports at the rate (rounded up) plus the FX margin and the Solana keep, never above the item cap.
/// Returns `(lamports, the floor in wei that bounds the buy)`.
#[allow(clippy::too_many_arguments)]
pub fn advance_bound(cap: u128, accepted_wei: u128, own: Option<(u128, u64)>, now: u64, premium_bps: u16, max_purchase_wei: u128,
    fee_bps: u16, lamports_per_eth: u64) -> (u128, u128) {
    let floor = match own {
        Some((r, at)) if r > 0 && now <= at.saturating_add(crate::ethnft::READING_TTL_SECS) => r.min(accepted_wei),
        _ => accepted_wei,
    };
    let buy = (floor.saturating_mul(10_000 + premium_bps as u128) / 10_000).min(max_purchase_wei);
    let need = buy.saturating_add(crate::ethnft::ETH_GAS_RESERVE_WEI);
    let fee = (fee_bps as u128).min(9_999);
    let gross_wei = need.saturating_mul(10_000).div_ceil(10_000 - fee);
    let gross = rates::wei_to_lamports_ceil(gross_wei, lamports_per_eth).saturating_mul(10_000 + FX_MARGIN_BPS).div_ceil(10_000)
        .saturating_add(SOL_GAS_KEEP_LAMPORTS);
    (gross.min(cap), floor)
}

/// A reading (wei per item) as the module's floor: lamports per whole unit at the rate, rounded down, clamped into the module's
/// `floor_bounds` (D-H-2: what is posted is what the module accepts).
pub fn post_price(price_wei: u128, lamports_per_eth: u64, bounds: (u64, u64)) -> u64 {
    let l = u64::try_from(rates::wei_to_lamports(price_wei, lamports_per_eth)).unwrap_or(u64::MAX);
    l.max(bounds.0).min(bounds.1.max(bounds.0))
}

/// The module as the item rules see it (`floorsink::view`'s Solana form), from the books (W8, the canister-only home): open once
/// posted; wanting one item per whole net unit (plus the one in transit) beyond the buffer, in 1e18-unit scale as `floorsink::wanted`
/// counts; its per-item maximum the module's `max_item_cost` within the collection's limit (`limit` lamports); fresh until
/// `posted_at + FLOOR_TTL`. Lamports throughout.
pub fn books_sink_view(c: &crate::books::Collection, kd: bool, now: i64, slot: u64, limit: u128) -> SinkView {
    let (m, e) = (&c.module, c.ledger.sol_backed);
    let scale = |u: u64| u as u128 * 1_000_000_000;
    let cap = (m.max_item_cost(now, e, kd).unwrap_or(0) as u128).min(limit);
    let open = m.accepted > 0;
    let buffer = scale(crate::books::ONE.saturating_mul(m.params.buffer_bps as u64) / crate::books::math::BPS);
    let net = scale(m.net_units(e));
    SinkView {
        kind: SinkKind::Floor,
        status: if !open { "no floor posted".into() } else {
            format!("net {} units (buffer {} bps), backing {} lamports, floor {} lamports posted at {}{}", m.net_units(e), m.params.buffer_bps,
                m.backing, m.accepted, m.posted_at, if kd { "; the poster is paused" } else { "" })
        },
        open,
        done: false,
        recorded: m.items_arrived,
        target: None,
        want: if open { crate::floorsink::wanted(net.saturating_add(scale(m.transit_units)), buffer) } else { 0 },
        budget_left: m.backing as u128,
        item_cap: Some(cap),
        deadline: None,
        clock: Clock { time: now.max(0) as u64, parent_block: slot },
        drive_refunds: false,
        drive_settle: false,
        price_expiry: Some(m.posted_at.saturating_add(crate::books::floor::FLOOR_TTL).max(0) as u64),
    }
}

// ---- the operator --------------------------------------------------------------------------------------------------------------

/// `sol_clear_incident`'s pure part: the line's nonce moved under a pending transaction none of whose signatures is known (proven
/// by `solsend::tick`, the nonce first and the statuses second, and proven AGAIN by the caller's fresh status read just before this:
/// `known` is whether any signature of the intent is known on chain now; NEW-A4-01), so it can never land: the operator, having looked
/// (a key used elsewhere? a bug?), frees the line. The intent's signatures move to `done` (abandoned), so `sol_seen` keeps them. The
/// pending step is taken back by its flow at the next tick (the line reads idle with the step still pending: an outcome "abandoned") and
/// is signed again under a fresh read of the nonce.
pub fn clear_incident(sh: &mut SolLine, known: bool) -> Result<String, String> {
    let why = sh.line.incident.clone().ok_or("refused: the Solana home line has no open incident")?;
    if known {
        return Err(format!("refused: a signature of the pending intent is known on chain now ({why}): it may have landed; nothing is \
            abandoned (the line settles it at its next tick)"));
    }
    let saved = solsend::clear_incident(&mut sh.line);
    Ok(format!("the home line's incident is cleared ({why}); intent {} is abandoned (none of its signatures {} is known, re-read now), its \
        step taken back at the next tick", saved.as_ref().map_or("?".into(), |s| s.intent.to_string()),
        saved.as_ref().map_or(String::new(), |s| s.history.join(", "))))
}

/// The operator clears the Solana home line's incident (`solsend::clear_incident`), after looking. Operator-only, refused while
/// paused and mid-tick, as every act on ticked state. NEW-A4-01: every signature of the pending intent is read again on chain (agreed)
/// first, holding the tick's flag (no tick starts meanwhile); any known one refuses the clear, and so does a line changed meanwhile.
#[ic_cdk::update]
pub async fn sol_clear_incident() -> Result<String, String> {
    crate::canister::only_operator();
    crate::canister::not_paused()?;
    clear_incident_on(LineId::Money).await
}

/// W3: [`sol_clear_incident`] for any of the four lines (an incident on one line stops that line only).
#[ic_cdk::update]
pub async fn sol_clear_line_incident(line: LineId) -> Result<String, String> {
    crate::canister::only_operator();
    crate::canister::not_paused()?;
    clear_incident_on(line).await
}

async fn clear_incident_on(id: LineId) -> Result<String, String> {
    // no tick starts while the statuses are read (the tick holds a copy of the line)
    let _held = crate::canister::hold_tick()?;
    let current = || match id {
        LineId::Money => LINE.with(|l| l.borrow().clone()),
        _ => LINES.with(|l| l.borrow().get(id).cloned()),
    };
    let before = current().ok_or("refused: the Solana home has no line yet")?;
    before.line.incident.as_ref().ok_or("refused: the Solana home line has no open incident")?;
    let known = match &before.line.saved {
        Some(s) => {
            let sigs: Vec<String> = s.expired.iter().flatten().chain(s.history.iter()).cloned().collect();
            let mut net = SolNet::of_install()?.ok_or(NOT_SOL)?;
            solrpc::statuses(&mut net.ps, &sigs).await?.iter().any(|x| x.is_some())
        }
        None => false,
    };
    crate::canister::not_paused()?;
    let mut sh = current().ok_or("refused: the Solana home has no line yet")?;
    if sh != before {
        return Err("refused: the line changed while its signatures were read; ask again".to_string());
    }
    let r = clear_incident(&mut sh, known)?;
    line_put_of(id, &sh);
    Ok(r)
}

// ---- the IC's world ------------------------------------------------------------------------------------------------------------

/// The Solana home on the IC: the configured providers (replicated outcalls through `sol_tf`), the threshold Ed25519 key, and this
/// tick's SOL/ETH rate and clock (read once a tick, never cached across ticks).
pub(crate) struct SolNet {
    pub cfg: SolHomeCfg,
    ps: Vec<IcSol>,
    signer: Option<crate::solkey::IcSigner>,
    rate: Option<u64>,
    btc_rate: Option<u64>,
    clock: Option<(u64, u64)>,
}

impl SolNet {
    pub fn new(h: &crate::canister::Home) -> Result<SolNet, String> {
        let crate::canister::Home::Solana { rpc_urls, .. } = h;
        Ok(SolNet { cfg: SolHomeCfg::of(h)?, ps: solrpc::ic_providers(rpc_urls), signer: None, rate: None, btc_rate: None, clock: None })
    }
    /// The installed home's world, if the install names one.
    pub fn of_install() -> Result<Option<SolNet>, String> {
        crate::canister::cfg().home.as_ref().map(SolNet::new).transpose()
    }
    pub async fn btc_rate(&mut self) -> Result<u64, String> {
        if let Some(r) = self.btc_rate {
            return Ok(r);
        }
        let r = crate::oracle::read_sol_btc_rate().await?.sats_per_sol;
        self.btc_rate = Some(r);
        Ok(r)
    }
    pub async fn program_slices(&mut self, program: &Pubkey, size: usize, memcmps: &[(usize, Vec<u8>)], slice: (usize, usize), max: usize)
        -> Result<Vec<(Pubkey, Pubkey, Vec<u8>)>, String> {
        Ok(solrpc::program_slices(&mut self.ps, program, size, memcmps, slice, max).await?)
    }
    pub async fn genesis(&mut self) -> Result<String, String> {
        Ok(solrpc::genesis_hash(&mut self.ps).await?)
    }
    async fn signer(&mut self) -> Result<&mut crate::solkey::IcSigner, String> {
        if self.signer.is_none() {
            self.signer = Some(crate::solkey::IcSigner::new().await?);
        }
        Ok(self.signer.as_mut().expect("set"))
    }
    pub async fn me(&mut self) -> Result<Pubkey, String> {
        crate::solkey::sol_pubkey().await
    }
    pub async fn accounts(&mut self, keys: &[Pubkey], max_data: usize) -> Result<Vec<Option<Account>>, String> {
        Ok(solrpc::accounts(&mut self.ps, keys, max_data).await?)
    }
    pub async fn account_head(&mut self, key: &Pubkey, len: usize) -> Result<Option<Account>, String> {
        Ok(solrpc::account_slice(&mut self.ps, key, 0, len).await?)
    }
    pub async fn program_accounts(&mut self, program: &Pubkey, size: usize, offset: usize, bytes: &[u8], max: usize)
        -> Result<Vec<(Pubkey, Account)>, String> {
        Ok(solrpc::program_accounts(&mut self.ps, program, size, offset, bytes, max, size).await?)
    }
    pub async fn balance(&mut self) -> Result<u128, String> {
        let me = self.me().await?;
        Ok(solrpc::balance(&mut self.ps, &me).await? as u128)
    }
    pub async fn relay(&mut self) -> Result<Pubkey, String> {
        crate::solkey::sol_relay_pubkey().await
    }
    pub async fn balance_of(&mut self, key: &Pubkey) -> Result<u128, String> {
        Ok(solrpc::balance(&mut self.ps, key).await? as u128)
    }
    pub async fn rent(&mut self, space: u64) -> Result<u64, String> {
        Ok(solrpc::rent_exempt(&mut self.ps, space).await?)
    }
    pub async fn accounts_slice(&mut self, keys: &[Pubkey], offset: usize, len: usize) -> Result<Vec<Option<Account>>, String> {
        Ok(solrpc::accounts_slice(&mut self.ps, keys, offset, len).await?)
    }
    pub async fn token_accounts(&mut self, owner: &Pubkey, program: &Pubkey, slice: (usize, usize), max: usize) -> Result<Vec<(Pubkey, Account)>, String> {
        Ok(solrpc::token_accounts(&mut self.ps, owner, program, slice, max).await?)
    }
    pub async fn clock(&mut self) -> Result<(u64, u64), String> {
        if let Some(c) = self.clock {
            return Ok(c);
        }
        let c = solrpc::finalized_clock(&mut self.ps).await?;
        self.clock = Some(c);
        Ok(c)
    }
    pub async fn rate(&mut self) -> Result<u64, String> {
        if let Some(r) = self.rate {
            return Ok(r);
        }
        let r = crate::oracle::read_sol_rate().await?.lamports_per_eth;
        self.rate = Some(r);
        Ok(r)
    }
    pub async fn prepare(&mut self, line: &mut Line, kind: LifetimeKind, intent: u64, what: &str, ixs: Vec<Instruction>) -> Result<(), String> {
        self.prepare_keys(line, kind, intent, what, ixs, vec![]).await
    }
    pub async fn prepare_keys(&mut self, line: &mut Line, kind: LifetimeKind, intent: u64, what: &str, ixs: Vec<Instruction>,
        keys: Vec<crate::solkey::KeyPath>) -> Result<(), String> {
        self.prepare_tables(line, kind, intent, what, ixs, keys, vec![]).await
    }
    #[allow(clippy::too_many_arguments)]
    pub async fn prepare_tables(&mut self, line: &mut Line, kind: LifetimeKind, intent: u64, what: &str, ixs: Vec<Instruction>,
        keys: Vec<crate::solkey::KeyPath>, tables: Vec<crate::soltx::LookupTable>) -> Result<(), String> {
        self.signer().await?;
        let SolNet { ps, signer, .. } = self;
        solsend::prepare_tables(line, kind, intent, what, ixs, keys, tables, ps, signer.as_mut().expect("set")).await
    }
    pub async fn tick(&mut self, line: &mut Line) -> Result<solsend::Step, String> {
        self.signer().await?;
        let SolNet { ps, signer, .. } = self;
        Ok(solsend::tick(line, ps, signer.as_mut().expect("set")).await?)
    }
    pub async fn signatures(&mut self, until: &str, limit: usize) -> Result<Vec<SigInfo>, String> {
        let me = self.me().await?;
        Ok(solrpc::signatures_since(&mut self.ps, &me, until, limit).await?)
    }
    pub async fn signatures_before(&mut self, until: &str, before: &str, limit: usize) -> Result<Vec<SigInfo>, String> {
        let me = self.me().await?;
        Ok(solrpc::signatures_between(&mut self.ps, &me, until, before, limit).await?)
    }
    pub async fn transaction(&mut self, sig: &str) -> Result<Option<serde_json::Value>, String> {
        Ok(solrpc::transaction(&mut self.ps, sig, TX_BYTES).await?)
    }
}

/// `SolIo` for an IC world holding an `Option<SolNet>` in field `$f` (the home configured: forwarded; else "not configured"); the line
/// is the canister-wide [`LINE`].
#[macro_export]
macro_rules! sol_io_via {
    ($t:ty, $f:ident) => {
        impl $crate::solhome::SolIo for $t {
            fn sol_home(&self) -> Option<$crate::solhome::SolHomeCfg> {
                self.$f.as_ref().map(|s| s.cfg.clone())
            }
            async fn sol_me(&mut self) -> Result<$crate::soltx::Pubkey, String> {
                self.$f.as_mut().ok_or("the Solana home is not configured")?.me().await
            }
            async fn sol_accounts(&mut self, keys: &[$crate::soltx::Pubkey], max_data: usize) -> Result<Vec<Option<$crate::solrpc::Account>>, String> {
                self.$f.as_mut().ok_or("the Solana home is not configured")?.accounts(keys, max_data).await
            }
            async fn sol_account_head(&mut self, key: &$crate::soltx::Pubkey, len: usize) -> Result<Option<$crate::solrpc::Account>, String> {
                self.$f.as_mut().ok_or("the Solana home is not configured")?.account_head(key, len).await
            }
            async fn sol_program_accounts(&mut self, program: &$crate::soltx::Pubkey, size: usize, offset: usize, bytes: &[u8], max: usize)
                -> Result<Vec<($crate::soltx::Pubkey, $crate::solrpc::Account)>, String> {
                self.$f.as_mut().ok_or("the Solana home is not configured")?.program_accounts(program, size, offset, bytes, max).await
            }
            async fn sol_program_slices(&mut self, program: &$crate::soltx::Pubkey, size: usize, memcmps: &[(usize, Vec<u8>)], slice: (usize, usize),
                max: usize) -> Result<Vec<($crate::soltx::Pubkey, $crate::soltx::Pubkey, Vec<u8>)>, String> {
                self.$f.as_mut().ok_or("the Solana home is not configured")?.program_slices(program, size, memcmps, slice, max).await
            }
            async fn sol_rent(&mut self, space: u64) -> Result<u64, String> {
                self.$f.as_mut().ok_or("the Solana home is not configured")?.rent(space).await
            }
            async fn sol_accounts_slice(&mut self, keys: &[$crate::soltx::Pubkey], offset: usize, len: usize)
                -> Result<Vec<Option<$crate::solrpc::Account>>, String> {
                self.$f.as_mut().ok_or("the Solana home is not configured")?.accounts_slice(keys, offset, len).await
            }
            async fn sol_token_accounts(&mut self, owner: &$crate::soltx::Pubkey, program: &$crate::soltx::Pubkey, slice: (usize, usize), max: usize)
                -> Result<Vec<($crate::soltx::Pubkey, $crate::solrpc::Account)>, String> {
                self.$f.as_mut().ok_or("the Solana home is not configured")?.token_accounts(owner, program, slice, max).await
            }
            async fn sol_relay(&mut self) -> Result<$crate::soltx::Pubkey, String> {
                self.$f.as_mut().ok_or("the Solana home is not configured")?.relay().await
            }
            async fn sol_balance_of(&mut self, key: &$crate::soltx::Pubkey) -> Result<u128, String> {
                self.$f.as_mut().ok_or("the Solana home is not configured")?.balance_of(key).await
            }
            async fn sol_rate(&mut self) -> Result<u64, String> {
                self.$f.as_mut().ok_or("the Solana home is not configured")?.rate().await
            }
            async fn sol_btc_rate(&mut self) -> Result<u64, String> {
                self.$f.as_mut().ok_or("the Solana home is not configured")?.btc_rate().await
            }
            async fn sol_balance(&mut self) -> Result<u128, String> {
                self.$f.as_mut().ok_or("the Solana home is not configured")?.balance().await
            }
            async fn sol_clock(&mut self) -> Result<(u64, u64), String> {
                self.$f.as_mut().ok_or("the Solana home is not configured")?.clock().await
            }
            async fn sol_prepare(&mut self, line: &mut $crate::solsend::Line, kind: $crate::solsend::LifetimeKind, intent: u64, what: &str,
                ixs: Vec<$crate::soltx::Instruction>) -> Result<(), String> {
                self.$f.as_mut().ok_or("the Solana home is not configured")?.prepare(line, kind, intent, what, ixs).await
            }
            async fn sol_tick(&mut self, line: &mut $crate::solsend::Line) -> Result<$crate::solsend::Step, String> {
                self.$f.as_mut().ok_or("the Solana home is not configured")?.tick(line).await
            }
            async fn sol_signatures(&mut self, until: &str, limit: usize) -> Result<Vec<$crate::solrpc::SigInfo>, String> {
                self.$f.as_mut().ok_or("the Solana home is not configured")?.signatures(until, limit).await
            }
            async fn sol_signatures_before(&mut self, until: &str, before: &str, limit: usize) -> Result<Vec<$crate::solrpc::SigInfo>, String> {
                self.$f.as_mut().ok_or("the Solana home is not configured")?.signatures_before(until, before, limit).await
            }
            async fn sol_transaction(&mut self, sig: &str) -> Result<Option<serde_json::Value>, String> {
                self.$f.as_mut().ok_or("the Solana home is not configured")?.transaction(sig).await
            }
            async fn sol_genesis(&mut self) -> Result<String, String> {
                self.$f.as_mut().ok_or("the Solana home is not configured")?.genesis().await
            }
            fn sol_line(&mut self) -> $crate::solhome::SolLine {
                $crate::solhome::line_now()
            }
            fn sol_line_save(&mut self, l: &$crate::solhome::SolLine) {
                $crate::solhome::line_put(l)
            }
            fn sol_line_of(&mut self, id: $crate::solhome::LineId) -> $crate::solhome::SolLine {
                $crate::solhome::line_now_of(id)
            }
            fn sol_line_save_of(&mut self, id: $crate::solhome::LineId, l: &$crate::solhome::SolLine) {
                $crate::solhome::line_put_of(id, l)
            }
            async fn sol_prepare_keys(&mut self, line: &mut $crate::solsend::Line, kind: $crate::solsend::LifetimeKind, intent: u64, what: &str,
                ixs: Vec<$crate::soltx::Instruction>, keys: Vec<$crate::solkey::KeyPath>) -> Result<(), String> {
                self.$f.as_mut().ok_or("the Solana home is not configured")?.prepare_keys(line, kind, intent, what, ixs, keys).await
            }
            async fn sol_prepare_tables(&mut self, line: &mut $crate::solsend::Line, kind: $crate::solsend::LifetimeKind, intent: u64,
                what: &str, ixs: Vec<$crate::soltx::Instruction>, keys: Vec<$crate::solkey::KeyPath>, tables: Vec<$crate::soltx::LookupTable>)
                -> Result<(), String> {
                self.$f.as_mut().ok_or("the Solana home is not configured")?.prepare_tables(line, kind, intent, what, ixs, keys, tables).await
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    const E: u128 = 1_000_000_000_000_000_000;
    /// 22.84 SOL an ETH (2026-10-09).
    const RATE: u64 = 22_840_000_000;

    /// Audit C2-03: a landed advance's outcome is never trimmed for book outcomes: with 15 collections posting (and twice as many book
    /// outcomes, the Ethereum side's raw `books` among them) and 15 more signed steps landing after it, the advance's outcome is still on
    /// the line for its collection to read; book outcomes are trimmed on their own count, oldest first.
    #[test]
    fn c2_03_a_landed_advance_is_never_trimmed_for_book_outcomes() {
        use crate::frogflow::FStep;
        let mut l = SolLine::default();
        let at = |n: u64| n;
        l.outcome(LineOutcome { step: LineStep::Frog(FStep::Advance { frog: 0 }), intent: 1, ok: Some(true), signature: "advance-sig".into(), at: at(1) });
        for c in 0..30u16 {
            l.book_outcome(LineOutcome { step: LineStep::Frog(FStep::Post), intent: 0, ok: Some(true), signature: format!("books:{}", 11 + c % 15), at: at(2) });
        }
        l.outcomes.push(LineOutcome { step: LineStep::Frog(FStep::Post), intent: 0, ok: Some(true), signature: "books".into(), at: at(2) });
        for i in 0..(OUTCOMES_KEPT as u64 - 1) {
            l.outcome(LineOutcome { step: LineStep::Frog(FStep::Convert { frog: 100 + i }), intent: 10 + i, ok: Some(true), signature: format!("s{i}"),
                at: at(3) });
        }
        assert!(l.outcomes.iter().any(|o| o.signature == "advance-sig"), "the landed advance is still on the line");
        assert_eq!(l.outcomes.iter().filter(|o| is_book_outcome(o)).count(), 31);
        // one more signed step: now the oldest SIGNED one goes (16 kept), never a book outcome in its place
        l.outcome(LineOutcome { step: LineStep::Frog(FStep::Convert { frog: 999 }), intent: 99, ok: Some(true), signature: "last".into(), at: at(4) });
        assert!(!l.outcomes.iter().any(|o| o.signature == "advance-sig"));
        assert_eq!((l.outcomes.iter().filter(|o| !is_book_outcome(o)).count(), l.outcomes.iter().filter(|o| is_book_outcome(o)).count()),
            (OUTCOMES_KEPT, 31));
        // book outcomes past their own cap: the oldest book ones go, no signed one
        for _ in 0..BOOK_OUTCOMES_KEPT {
            l.book_outcome(LineOutcome { step: LineStep::Frog(FStep::Post), intent: 0, ok: Some(true), signature: "books:12".into(), at: at(5) });
        }
        assert_eq!((l.outcomes.iter().filter(|o| !is_book_outcome(o)).count(), l.outcomes.iter().filter(|o| is_book_outcome(o)).count()),
            (OUTCOMES_KEPT, BOOK_OUTCOMES_KEPT));
        assert!(l.outcomes.iter().any(|o| o.signature == "last"));
    }

    #[test]
    fn the_gate_needs_a_floor_its_advance_covers_in_eth() {
        // a 1 ETH floor in lamports, and the module's item cap at it + 3 %
        let accepted_wei = E;
        let cap = rates::wei_to_lamports(E, RATE) * 103 / 100;
        assert!(advance_covers(cap, accepted_wei, RATE).is_ok());
        // a 0.2 ETH floor does not cover the gas reserve on top of the margins
        let cap = rates::wei_to_lamports(E / 5, RATE) * 103 / 100;
        assert!(advance_covers(cap, E / 5, RATE).unwrap_err().contains("waits on Solana"));
        assert!(advance_covers(cap, 0, RATE).is_err());
    }

    #[test]
    fn the_advance_is_the_buys_bound_in_lamports_never_over_the_cap() {
        let cap = rates::wei_to_lamports(E, RATE) * 110 / 100;
        let (l, floor) = advance_bound(cap, E, Some((95 * E / 100, 1_000)), 1_000, 500, 2 * E, 300, RATE);
        assert_eq!(floor, 95 * E / 100, "our fresher, lower reading bounds the buy");
        // what reaches Ethereum after the judge's worst fee still pays the buy's most and its gas
        let buy = floor * 105 / 100 + crate::ethnft::ETH_GAS_RESERVE_WEI;
        assert!(rates::lamports_to_wei(l - SOL_GAS_KEEP_LAMPORTS, RATE) * 97 / 100 >= buy, "{l}");
        assert!(l <= cap);
        // a stale reading: the module's floor
        assert_eq!(advance_bound(cap, E, Some((E / 2, 1_000)), 1_000 + 3_601, 500, 2 * E, 300, RATE).1, E);
        // capped by the item cap
        assert_eq!(advance_bound(1_000, E, None, 0, 500, 2 * E, 300, RATE).0, 1_000);
    }

    #[test]
    fn a_post_is_lamports_at_the_rate_inside_the_bounds() {
        assert_eq!(post_price(E, RATE, (0, u64::MAX)), RATE);
        assert_eq!(post_price(E, RATE, (0, 1_000)), 1_000);
        assert_eq!(post_price(E, RATE, (RATE as u64 + 5, u64::MAX)), RATE + 5);
    }
}
