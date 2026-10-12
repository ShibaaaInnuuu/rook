//! The Ethereum NFT side's sequencer (docs/v7/phase1-nft.md §2, §4): buys (advance → Relay to our Ethereum address → a judged
//! Seaport fill → finality → mint to the module → settle), redemptions (a resolved ticket → a judged `transferFrom` → finality →
//! `recordDelivery`, or `markUndeliverable` once no version can confirm), the floor (verified evidence → `postFloor`) and holder
//! deposits (behind their switch).
//!
//! Two transaction lines: the home chain's (4663), one transaction of ours canister-wide as everywhere, and Ethereum's, one in
//! flight, its bytes saved before any send. Everything that touches the world goes through [`EthIo`]; the rules are `ethnft`'s,
//! `relayeth`'s and `seaport.rs`'s. Tested against a fake world in `tests/ethnft.rs`.
//!
//! docs/v9/solana-migration.md §4 (phase B1), canister-only since decision D-l (docs/v9/canister-only-solana.md §8): when the world's
//! home is Solana (`EthIo::sol_home`), every home read comes from the books and every home step is a book update, but the advance and
//! the way back (SOL moved on the money line) and Relay's `deposit_native` (`home_books`, `home_sol`, at the end of this file); the
//! home's amounts are lamports, the Ethereum side's wei, met at this tick's SOL/ETH rate. With no Solana home every path is the EVM
//! one, unchanged. Tested against a fake Solana chain in `tests/ethnft_sol.rs`.

use candid::CandidType;
use serde::{Deserialize, Serialize};

use crate::custodycap;
use crate::dormancy::{self as dm, Kind};
use crate::ethnft::{self as en, Col, EthNftConfig, FloorPost, ReceiptV, Seen, Stage, TicketV};
use crate::evmtx::Tx1559;
use crate::itemsink::{self, Clock, SinkView};
use crate::looper::EvmView;
use crate::outflow::Flow;
use crate::relayconv::ConvAsk;
use crate::relayeth as re;
use crate::seaport::Word;


/// Quotes per advance before the ETH goes back (as `relayconv::MAX_ATTEMPTS`).
pub const MAX_ATTEMPTS: u32 = 3;
/// Kept at our 4663 address from each advance for that purchase's own gas there.
pub const GAS_KEEP_WEI: u128 = crate::relayconv::GAS_KEEP_WEI;
/// Tickets read per tick, per wrapper.
pub const TICKETS_PER_TICK: u64 = 8;
/// A cancel pays at least this much more per gas than the transaction it replaces (nodes want +10 %).
pub const CANCEL_BUMP_PCT: u128 = 25;
/// Past the order's deadline by this with no arrival and no refund: stuck.
pub const DEADLINE_GRACE_SECS: u64 = crate::relayconv::DEADLINE_GRACE_SECS;
/// A transaction on the Ethereum line unmined this long is re-priced at the same nonce (docs/v7/phase1-nft-pooled.md §4, J6).
pub const LINE_BUMP_SECS: u64 = 1_800;
/// A handed-in listing or piece of evidence is used within this of its hand-in, or not at all.
pub const HANDIN_TTL_SECS: u64 = 600;
/// Deposit claims read per tick (each is two agreed reads on every Ethereum provider).
pub const CLAIMS_PER_TICK: usize = 4;
/// A claim whose transaction cannot be read yet (not mined, above the pin, a provider error) is tried again for this long, then
/// dropped unrecorded: it can be claimed again. Only a judged claim is recorded.
pub const CLAIM_RETRY_SECS: u64 = 6 * 3_600;
/// A stray's return is signed only while our Ethereum balance, less the ETH of an arrived buy, stays this far above the return's
/// worst-case gas: the gas pot is for deliveries first (docs §9 D3, D5).
pub const RETURN_GAS_KEEP_WEI: u128 = 5_000_000_000_000_000;
/// A transfer's simulation that every provider agrees REVERTS is believed only once it has said so for this long, over several
/// ticks (audit 2026-09-27-nft, B-M-2 / C-M-1): a delivery is marked undeliverable, or a stray held, on a repeated agreed revert,
/// never on one answer. A read that fails or disagrees is never a revert.
pub const SIM_REFUSAL_SECS: u64 = 3_600;
/// Review round 5 (A5-L-2): a delivery this close to its deadline goes before any backlog (`delivery_order`).
pub const DELIVERY_URGENT_SECS: u64 = 2 * 86_400;
/// A fill unmined this long after it was first signed (re-priced every `LINE_BUMP_SECS` meanwhile) is cancelled at its nonce: a
/// 0-value self-send; if the fill mines instead it is a buy as ever (audit 2026-09-27-nft, B-L-3 / C-M-2; the 2026-09-26 L-5 rule).
pub const FILL_CANCEL_SECS: u64 = 6 * 3_600;
/// A due floor post whose module was last posted this long ago goes before every other home-chain act (audit 2026-09-27-nft,
/// B-L-9 / B-H-2): the module is stale at `FLOOR_TTL` (1 h), and a stale module refuses mints and burns for every holder.
pub const URGENT_POST_SECS: u64 = 2_400;
/// Floor evidence: once this many more than `min_listings` distinct offerers are verified, the rest of the hand-in is not read
/// (audit 2026-09-27-nft, B-M-7: every listing is six agreed reads on every provider).
pub const EXTRA_LISTINGS: usize = 2;
/// A sale receipt this many blocks under the pin is final for our purposes (two epochs and a margin): its sales are kept by hash
/// and it is never read again (B-M-7). At most `SALE_CACHE_PER_COLLECTION` per collection, the newest (round 2: every judged
/// transaction of the canister's own scan, a sale or not, over the 7-day window).
pub const FINAL_DEPTH_BLOCKS: u64 = en::SALE_FINAL_DEPTH_BLOCKS;
pub const SALE_CACHE_PER_COLLECTION: usize = 2_048;
/// Round 3 (B3-M-1): judged transactions without a sale, kept per collection (8-byte hash prefix and block) over the window.
pub const SALE_JUDGED_PER_COLLECTION: usize = 8_192;
/// Under this many cycles the floor evidence is not verified (the deliveries and the rest of the tick go on): the evidence pass is
/// the canister's largest outcall bill (B-M-7), and deposits use the same floor (`frognet::DEPOSITS_CYCLES_FLOOR`).
pub const EVIDENCE_CYCLES_FLOOR: u128 = 8_000_000_000_000;
/// `EthNftWrapper.DELIVERY_DEADLINE`: a ticket's `deliverBy` is the block time it resolved at plus this.
pub const DELIVERY_DEADLINE_SECS: u64 = 7 * 86_400;
/// The advance gate (audit 2026-09-27-nft round 2, A2-M-1): a buy starts, and its advance is signed, only while what would reach
/// Ethereum covers the module's floor with a margin: `item_cap − GAS_KEEP − item_cap × ADVANCE_FEE_EST_BPS` must be at least
/// `accepted × (1 + ADVANCE_MARGIN_BPS) + ETH_GAS_RESERVE`. The fee estimate is Relay's measured ETH → ETH cost (0.36 %) with room;
/// the judged quote is still bounded by `relay_max_fee_bps`. A pooled module whose blended backing is short of the floor waits on
/// 4663 instead of stranding its ETH on Ethereum.
pub const ADVANCE_FEE_EST_BPS: u128 = 50;
pub const ADVANCE_MARGIN_BPS: u128 = 100;
/// An `Arrived` buy holds its collection's deposit mints (C-H-1 / A-L-1) for this long at most (round 2, C2-M-2): past it the
/// deposits mint (the buy's own mint then waits for the day's room, if it ever fills).
pub const ARRIVED_HOLD_SECS: u64 = 6 * 3_600;
/// An `Arrived` buy that has not filled for this long goes back (round 2, A2-M-1): its ETH is bridged Ethereum → 4663 through a
/// Relay order judged like the forward one (`relayeth::FROM_ETHEREUM`), then returned to its module (`returnAdvance`).
pub const ARRIVED_BACK_SECS: u64 = 48 * 3_600;
/// A2-L-1, the gas-pot sweep (the operator, 2026-09-27): a sweep smaller than this is not worth a Relay round trip (it waits for more
/// leftover); one with no quote for `SWEEP_QUOTE_SECS` is abandoned, its leftover credited back, and none is tried again for
/// `SWEEP_COOLDOWN_SECS` (while a sweep waits for its quote no new buy starts, so the wait is bounded).
pub const SWEEP_MIN_WEI: u128 = 20_000_000_000_000_000;
/// Round 3 (C3-M-1): the way back's bounded exits. A buy going back (`BackFunded`) holds other collections' new buys (it is next on the
/// Relay leg) for `BACK_QUOTE_SECS` at most; with no deposit signed by then it waits at `Arrived` again (it may take a listing) and
/// tries again `BACK_RETRY_SECS` later. A way back past its order's deadline and the grace, with neither an arrival on 4663 nor a
/// refund on Ethereum for `BACK_LATE_SECS` more, stops being watched (`Stuck`, off the Relay leg: other collections buy again); the
/// operator closes it once Relay says the order is dead (`eth_give_back`).
pub const BACK_QUOTE_SECS: u64 = 2 * 3_600;
/// Audit C3-03: a `Funded` buy (its advance at our home address, waiting for the forward Relay quote) that no quote reached for this
/// long goes back to its module (`Returning`): one chain's dead Relay leg (a runner that cannot quote it) never holds the one leg every
/// chain's buys, ways back and the frog side's conversions share. The collection then waits `RETURN_COOLDOWN_SECS` before advancing again.
pub const FUNDED_QUOTE_SECS: u64 = 2 * 3_600;
pub const BACK_RETRY_SECS: u64 = 24 * 3_600;
pub const BACK_LATE_SECS: u64 = 24 * 3_600;
/// Round 3 (C3-L-3): the way back's arrival is a rise of our 4663 balance of at least the order's minimum less this (our own 4663
/// gas meanwhile), no longer half of it.
pub const BACK_ARRIVAL_SLACK_BPS: u128 = 100;
/// Round 3 (A3-L-1 / C3-L-2): a collection whose advance came back (`Returned`) is not advanced again for this long, so a market with
/// no listing in the bound, or a runner that hands in none, cannot churn its ETH through Relay round trips at the holders' cost.
pub const RETURN_COOLDOWN_SECS: u64 = 24 * 3_600;
pub const SWEEP_QUOTE_SECS: u64 = 2 * 3_600;
pub const SWEEP_COOLDOWN_SECS: u64 = 24 * 3_600;
/// Review round 4 (C4-H-1), round 5 (F5-M-1): our home transaction's nonce is used on chain while the transaction is seen nowhere
/// (not mined at the pin, in no provider's pool) on every read in a row for this long. Since round 4's floors that is almost always
/// OUR transaction mined while every provider's index misses it, so nothing is taken back on that absence alone: what used the nonce
/// is identified (`EthIo::home_nonce_tx`), or the step's own effect is read back (`home_effect`); only a step whose effect is
/// readable and absent (or a floor post, harmless to do again) is taken back as a revert after this long. A Relay deposit or a
/// sweep's `fundReserve` waits, paged, until the chain shows it or the operator says (`eth_home_clear`).
pub const HOME_NONCE_TAKEN_SECS: u64 = 3_600;
/// Review round 5 (F5-M-1): how far back the search for what used a nonce starts when the pending transaction does not say its pin
/// (one saved by an earlier build).
pub const HOME_NONCE_SEARCH_BLOCKS: u64 = 100_000;
/// Review round 6 (F6-L-1): how far under the pin 4663's providers are taken to keep STATE (`rpc.mainnet.chain.robinhood.com` keeps
/// ≈ 6,200 blocks under its head, ≈ 10 minutes at 10 blocks a second, measured 2026-09-28; the pin is 1,024–2,048 under the head). The
/// search for what used a nonce starts no deeper than this under the pin when the transaction's own pin is out of that reach and the
/// nonce is still unused there; the runner ticks sooner while a home transaction is out (`RUNNER_PENDING_EVERY_S`), so a transaction
/// mined seconds after it was sent is inside it at the next tick.
pub const HOME_STATE_REACH_BLOCKS: u64 = 4_000;
/// Review round 7 (F7-L-1): the state `rpc.mainnet.chain.robinhood.com` keeps under its OWN head (6,198–6,234 blocks, measured
/// 2026-09-28, the least taken), and 4663's rate (9.95 blocks a second, measured). A read at block `b` is served while the provider's
/// head is at most this far above `b`.
pub const HOME_PROVIDER_STATE_BLOCKS: u64 = 6_198;

/// Review round 7 (F7-L-1): where the search for what used our 4663 nonce starts, nearest the head first. `signed_pin` is the pin the
/// transaction was signed at (`HomeTx.pin`: the fast pin, one `bucket` under the lowest provider's rounded head), `pin` this tick's.
/// Candidates, highest first, each a block where the nonce is expected unused:
/// 1. `pin − HOME_STATE_REACH_BLOCKS` when it is above the next one (a transaction mined late, a resend);
/// 2. `signed_pin + bucket`: the lowest provider's rounded head at signing. Every provider's head was at or past it before the
///    transaction was broadcast, so it cannot have mined at or under it, and nothing else of ours signs while it is out. It is
///    0–1,023 blocks under the head at signing, where round 6 started at the fast pin itself, 1,024–2,047 under it: a bucket (≈ 103 s)
///    more of the provider's state reach is left for the next tick;
/// 3. `signed_pin` itself (the round-6 start), in case 2 is refused;
///
/// then, with no `signed_pin` (a transaction saved by an earlier build), `pin − HOME_NONCE_SEARCH_BLOCKS`. Each is at most `pin`.
pub fn home_search_starts(signed_pin: Option<u64>, pin: u64, bucket: u64) -> Vec<u64> {
    let mut v = Vec::new();
    match signed_pin {
        Some(s) => {
            let head = s.saturating_add(bucket).min(pin);
            let near = pin.saturating_sub(HOME_STATE_REACH_BLOCKS);
            if near > head {
                v.push(near);
            }
            v.push(head);
            v.push(s.min(pin));
        }
        None => v.push(pin.saturating_sub(HOME_NONCE_SEARCH_BLOCKS)),
    }
    v.dedup();
    v
}

/// Review round 7 (F7-L-1): the longest wait, in seconds, from signing to the first read of the search that still lands inside the
/// provider's state, for a transaction signed with the lowest provider's head `lag` blocks past its rounded head and the provider
/// keeping state `HOME_PROVIDER_STATE_BLOCKS` under its own head, `spread` blocks ahead of the lowest (4663 makes ≈ 9.95 blocks a
/// second: 10 is taken, the conservative side). The runbook's `RUNNER_PENDING_EVERY_S` bound is set from it.
pub fn home_search_reach_secs(lag: u64, spread: u64) -> u64 {
    HOME_PROVIDER_STATE_BLOCKS.saturating_sub(lag).saturating_sub(spread) / 10
}

// ---- records -------------------------------------------------------------------------------------------------------------------

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum BPhase {
    /// `advance(amount)` is the next home-chain act.
    NeedAdvance,
    Advancing,
    /// The ETH is at our home-chain address; the runner is asked for a Relay quote (`ask`).
    Funded { wei: u128, since: u64 },
    Depositing { order_id: [u8; 32], value: u128, min_out: u128, deadline: u64 },
    /// The deposit mined; watching Ethereum for the arrival and the home chain for a refund.
    Bridging { order_id: [u8; 32], value: u128, min_out: u128, deadline: u64, since: u64 },
    /// `wei` arrived at our Ethereum address for this buy; the runner is asked for a listing.
    Arrived { wei: u128, since: u64 },
    /// A fill is signed on the Ethereum line.
    Buying { arrived: u128, token: Word, total: u128, tx: [u8; 32] },
    /// Mined, ownerOf is us; waiting for Ethereum finality.
    Bought { arrived: u128, token: Word, total: u128, tx: [u8; 32], block: u64 },
    /// Final on Ethereum: the mint to the module is the next home-chain act (then `Minting`).
    Final { arrived: u128, token: Word, total: u128, tx: [u8; 32] },
    Minting { arrived: u128, token: Word, total: u128, tx: [u8; 32] },
    /// The unit is at the module: `settleAdvance(cost)` is next.
    Minted { arrived: u128, token: Word, total: u128, tx: [u8; 32] },
    Settling { token: Word, total: u128, tx: [u8; 32], cost: u128 },
    Settled { token: Word, total: u128, tx: [u8; 32], cost: u128 },
    /// `returnAdvance{value: wei}` is the next (or current) home-chain act.
    Returning { wei: u128 },
    Returned { wei: u128 },
    Stuck { why: String },
    /// Round 2 (A2-M-1): an arrived buy's ETH goes back. `wei` is at our Ethereum address; the runner is asked for a Relay quote
    /// Ethereum → 4663 (`ask`).
    BackFunded { wei: u128, since: u64 },
    /// The Relay deposit on Ethereum is on the Ethereum line (`EthKind::BackDeposit`).
    BackSending { wei: u128, order_id: [u8; 32], value: u128, min_out: u128, deadline: u64, tx: [u8; 32] },
    /// It mined; watching 4663 for the arrival (then `Returning`) and Ethereum for a refund (then `Arrived` again).
    BackBridging { order_id: [u8; 32], value: u128, min_out: u128, deadline: u64, since: u64 },
}

/// A mined Relay deposit being watched.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Watch {
    pub order_id: [u8; 32],
    pub value: u128,
    pub min_out: u128,
    pub deadline: u64,
    /// Our home-chain balance as the deposit left it: a refund raises it again.
    pub balance_after: u128,
    /// Our Ethereum balance, and what our Ethereum transactions had spent in all, just before the deposit.
    pub eth_before: u128,
    pub eth_out_before: u128,
    /// Round 3 (E3-M-1): the Ethereum pin `eth_before` was read at. The watch is read again only at a pin at or above it (and above
    /// every spend `eth_out` counts): a pin that moved back sees neither a counted spend nor this watch's own start.
    #[serde(default)]
    pub pin: Option<u64>,
    /// Review round 4 (C4-M-1): the order's solver (`orderData.solver`, bound by the order id). Review round 5 (C5-L-1): Relay's one
    /// solver for every order, so it no longer names this order on its own; kept for the record.
    #[serde(default)]
    pub solver: Option<[u8; 20]>,
    /// Review round 5 (C5-L-1): Relay's request id for this order (the quote's `steps[0].requestId`), which Relay's own fill carries in
    /// its calldata: with the order id, what names a payout as this order's.
    #[serde(default)]
    pub request_id: Option<[u8; 32]>,
    /// Review round 5 (C5-L-1): the 4663 pin when the order was deposited: a payout on 4663 mined at or before it is not this order's.
    #[serde(default)]
    pub home_pin: Option<u64>,
}

/// Review round 4 (C4-M-1): a Relay order let go unpaid (the operator wrote it off once Relay said nothing more would come). Its
/// watch is kept, so a payout that lands after all is credited to THIS order and its collection (`eth_credit_payout`, judged against
/// the order by `judge_payout`), never read as another order's.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct LateOrder {
    pub watch: Watch,
    /// The way back (Ethereum → 4663), or the forward leg (4663 → Ethereum).
    pub back: bool,
    pub let_go_at: u64,
    /// Set once a payout was credited to it: `(the payout's transaction, its wei, on 4663)`.
    pub credited: Option<([u8; 32], u128, bool)>,
    /// Review round 5 (C5-M-1): while a written-off order is neither credited nor released, it holds the Relay leg's attribution: no
    /// new Relay order (a buy, a way back, a sweep) starts, so no other order's watch can read its late payout as its own. The operator
    /// releases it (`eth_late_release`) once Relay's own records show where its ETH went.
    #[serde(default)]
    pub released_at: Option<u64>,
}

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct EthBuy {
    pub id: u64,
    pub collection: [u8; 20],
    pub module: [u8; 20],
    pub wrapper: [u8; 20],
    pub advanced: u128,
    pub attempts: u32,
    pub phase: BPhase,
    pub orders: Vec<[u8; 32]>,
    pub deposits: Vec<[u8; 32]>,
    pub ask: Option<ConvAsk>,
    pub last: Option<Watch>,
    pub note: String,
    /// A2-L-1 (2026-09-27): not a buy but a gas-pot sweep: leftover of this collection's purchases going back to its module through
    /// the way back (`BackFunded` → `BackSending` → `BackBridging` → `Returning` → `Returned`), paid in by `fundReserve`, never
    /// `returnAdvance` (no advance is open for it). It never reaches `Arrived`: a refund ends it and credits the leftover back.
    #[serde(default)]
    pub sweep: Option<bool>,
    /// Round 3 (E3-L-1): when its ETH first arrived on Ethereum. The deposit hold (`ARRIVED_HOLD_SECS`) is measured from it, so a
    /// fill that reverts (or is cancelled) does not restart it.
    #[serde(default)]
    pub arrived_at: Option<u64>,
    /// Round 3 (E3-L-1 / C3-M-1): when its way back is due: the first arrival + `ARRIVED_BACK_SECS` (D4's 48 h), or
    /// `BACK_RETRY_SECS` after a way back that did not go (no quote in its window, or refunded on Ethereum). A reverted fill does not
    /// move it.
    #[serde(default)]
    pub back_at: Option<u64>,
    /// Round 3 (A3-L-1): when it was returned to its module (`Returned`), for `RETURN_COOLDOWN_SECS`.
    #[serde(default)]
    pub ended_at: Option<u64>,
    /// Review round 4 (C4-M-1): its Relay order, written off unpaid, kept for a late payout.
    #[serde(default)]
    pub late: Option<LateOrder>,
}

impl EthBuy {
    pub fn is_sweep(&self) -> bool {
        self.sweep == Some(true)
    }
    pub fn finished(&self) -> bool {
        matches!(self.phase, BPhase::Settled { .. } | BPhase::Returned { .. })
    }
    /// The module paid: never released for "no longer needed".
    pub fn committed(&self) -> bool {
        !matches!(self.phase, BPhase::NeedAdvance)
    }
    /// On the Relay leg (docs/v7/phase1-nft-pooled.md §4): its arrival or refund is read as a rise in one of our two balances, so
    /// at most one buy canister-wide may be here. A stuck buy still watched for a late fill counts.
    pub fn in_relay(&self) -> bool {
        match self.phase {
            // audit A5-02: a way back waiting for its quote (`BackFunded`) holds it too: its deposit is signed as soon as a quote is
            // handed in, so nothing else (the frog side's conversion: `eth_relay_busy`) may open a watch on our balances meanwhile
            BPhase::NeedAdvance | BPhase::Advancing | BPhase::Funded { .. } | BPhase::Depositing { .. } | BPhase::Bridging { .. }
                | BPhase::Returning { .. } | BPhase::BackFunded { .. } | BPhase::BackSending { .. } | BPhase::BackBridging { .. } => true,
            BPhase::Stuck { .. } => self.last.is_some(),
            _ => false,
        }
    }
    /// The ETH at our Ethereum address that is this buy's and no one else's: what arrived, until its fill is mined.
    pub fn earmark(&self) -> u128 {
        match self.phase {
            BPhase::Arrived { wei, .. } | BPhase::BackFunded { wei, .. } | BPhase::BackSending { wei, .. } => wei,
            BPhase::Buying { arrived, .. } => arrived,
            _ => 0,
        }
    }
}

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum DPhase {
    /// Resolved, not yet signed.
    Waiting,
    /// A transfer is on the Ethereum line.
    Sent { tx: [u8; 32], nonce: u64 },
    /// Mined (the transfer to the destination succeeded in `block`); waiting for finality. Round 3 (D3-H-1): what the destination
    /// does with the token afterwards is not the delivery's question.
    Moved { tx: [u8; 32], block: u64 },
    /// Final: `recordDelivery` is the next home-chain act.
    Record { tx: [u8; 32] },
    Recording { tx: [u8; 32] },
    Recorded { tx: [u8; 32] },
    /// Our cancel (a 0-value self-send at the transfer's nonce) is on the line.
    Cancelling { of: [u8; 32], tx: [u8; 32], nonce: u64 },
    /// Our cancel, or the reverted transfer, mined in `block`: once that is final, no version can confirm.
    MarkAfter { why: String, block: u64 },
    /// No version of the delivery can confirm any more: `markUndeliverable` is next.
    Mark { why: String },
    Marking { why: String },
    Marked { why: String },
    /// The ticket left `Resolved` by another path (refunded after the late window): nothing more is done.
    Gone { why: String },
}

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Delivery {
    pub wrapper: [u8; 20],
    pub collection: [u8; 20],
    pub ticket: u128,
    pub token: Word,
    pub destination: [u8; 20],
    pub deliver_by: u64,
    pub phase: DPhase,
    pub note: String,
    /// Since when every provider agrees the transfer's simulation reverts (B-M-2); cleared by a simulation that passes.
    #[serde(default)]
    pub refused_since: Option<u64>,
    /// Round 3 (D3-H-1): `noteSent` mined on the wrapper (the transfer mined on Ethereum): the wrapper's late window can no longer
    /// refund this ticket, whatever happens to the record.
    #[serde(default)]
    pub sent: Option<bool>,
}

impl Delivery {
    pub fn finished(&self) -> bool {
        matches!(self.phase, DPhase::Recorded { .. } | DPhase::Marked { .. } | DPhase::Gone { .. })
    }
}

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum XPhase {
    /// A valid deposit, verified at a pin; waiting for Ethereum finality.
    Seen,
    /// Final and ours: `mintDeposit` is the next home-chain act (then `Minting`).
    Mint,
    Minting,
    Minted,
    /// Not mintable (no claim data, garbled data, another wrapper, moved by an operator or a contract, or no custody room): it
    /// goes back to its own `Transfer`'s sender once its block is final (docs/v7/phase1-nft.md §2.4, D5).
    Return { why: String },
    /// The return is on the Ethereum line.
    Returning { tx: [u8; 32] },
    Returned { tx: [u8; 32] },
    /// It stays at our address and the operator looks: a return that cannot be judged or signed (the token is held or in use, the
    /// simulation refuses, the collection is no longer configured). Nothing mints for it.
    Held { why: String },
    /// Nothing of a configured collection reached us in this transaction, or it is no longer ours.
    Refused { why: String },
}

impl XPhase {
    /// Still being worked on: its token is in use by this claim.
    pub fn open(&self) -> bool {
        matches!(self, XPhase::Seen | XPhase::Mint | XPhase::Minting | XPhase::Return { .. } | XPhase::Returning { .. })
    }
}

/// One NFT that reached our Ethereum address in a claimed transaction (docs §2.4): a holder's deposit, or a stray to return.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct DepositRec {
    pub tx: [u8; 32],
    pub collection: [u8; 20],
    pub token: Word,
    /// The 4663 recipient a valid deposit names (zero for a stray).
    pub recipient: [u8; 20],
    /// Its `Transfer`'s `from`: where a return goes.
    pub from: [u8; 20],
    /// The block the transaction mined in, as last read.
    pub block: u64,
    pub phase: XPhase,
    /// Since when every provider agrees the return's simulation reverts (C-M-1); cleared by a simulation that passes.
    #[serde(default)]
    pub refused_since: Option<u64>,
}

/// Our last floor reading per collection.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Reading {
    pub collection: [u8; 20],
    pub post: FloorPost,
    pub at: u64,
    pub pin: u64,
    /// Due: a `postFloor` is the next home-chain act of its kind.
    pub due: bool,
    /// The module's last post when this reading was made (an old one makes the post urgent, B-L-9).
    #[serde(default)]
    pub module_posted_at: Option<u64>,
}

/// The sales one final receipt showed for one collection, kept by hash so it is never read again (audit 2026-09-27-nft, B-M-7).
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct CachedSales {
    pub collection: [u8; 20],
    pub hash: [u8; 32],
    pub block: u64,
    /// `(token, price, seller, buyer)`
    pub sales: Vec<(Word, u128, [u8; 20], [u8; 20])>,
}

impl CachedSales {
    fn of(collection: [u8; 20], hash: [u8; 32], block: u64, v: &[en::Sale]) -> Self {
        CachedSales { collection, hash, block, sales: v.iter().map(|x| (x.token, x.price, x.seller, x.buyer)).collect() }
    }
    fn sales(&self) -> Vec<en::Sale> {
        self.sales.iter().map(|(token, price, seller, buyer)| en::Sale { token: *token, price: *price, seller: *seller, buyer: *buyer,
            block: self.block }).collect()
    }
}

/// What a transaction on the Ethereum line is for.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum EthKind {
    Buy { buy: u64 },
    Deliver { wrapper: [u8; 20], ticket: u128 },
    Cancel { wrapper: [u8; 20], ticket: u128 },
    /// A stray going back to its sender (the claimed transaction and the token).
    Return { tx: [u8; 32], token: Word },
    /// Our 0-value self-send at an unmined fill's nonce (audit 2026-09-27-nft, B-L-3 / C-M-2): the fill's versions are its rivals.
    CancelFill { buy: u64 },
    /// Round 2 (A2-M-1): an arrived buy's Relay deposit Ethereum → 4663.
    BackDeposit { buy: u64 },
}

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct EthTx {
    pub kind: EthKind,
    pub nonce: u64,
    pub hash: [u8; 32],
    pub raw: Vec<u8>,
    /// What it pays out besides gas when it succeeds (a fill's price).
    pub value: u128,
    pub max_fee: u128,
    pub priority_fee: u128,
    pub signed_at: u64,
    /// What was signed, so it can be re-signed at the same nonce with higher fees (docs/v7/phase1-nft-pooled.md §4).
    #[serde(default)]
    pub to: [u8; 20],
    #[serde(default)]
    pub data: Vec<u8>,
    #[serde(default)]
    pub gas: u64,
    /// Every earlier version of this transaction (the same nonce, lower fees): any of them mining settles it.
    #[serde(default)]
    pub earlier: Vec<[u8; 32]>,
    /// A cancel's rival: every version of the delivery it replaces.
    #[serde(default)]
    pub rivals: Vec<[u8; 32]>,
    /// When its first version was signed (a re-price keeps it): a fill this old is cancelled (`FILL_CANCEL_SECS`).
    #[serde(default)]
    pub first_signed_at: Option<u64>,
}

impl EthTx {
    /// This transaction's hash and every earlier version's, newest first.
    pub fn versions(&self) -> Vec<[u8; 32]> {
        std::iter::once(self.hash).chain(self.earlier.iter().rev().copied()).collect()
    }
}

impl OurReceipt {
    /// What this receipt took from our Ethereum balance, for a transaction sending `value`.
    pub fn spent(&self, value: u128) -> u128 {
        self.fee.saturating_add(if self.success { value } else { 0 })
    }
}

/// Which of our home-chain calls a saved transaction is.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum Step {
    Advance { buy: u64 },
    Deposit { buy: u64 },
    GiveBack { buy: u64 },
    Mint { buy: u64 },
    Settle { buy: u64 },
    Record { wrapper: [u8; 20], ticket: u128 },
    Mark { wrapper: [u8; 20], ticket: u128 },
    Post { collection: [u8; 20] },
    MintDeposit { tx: [u8; 32] },
    /// Round 3 (D3-H-1): `noteSent(ticket, tx)`: the delivery mined on Ethereum.
    Sent { wrapper: [u8; 20], ticket: u128 },
    /// docs/v7/dormant-collections.md §5: the permissionless `FloorModule.fill(coin)` of a graduation queued while the module was stale.
    Fill { collection: [u8; 20], coin: [u8; 20] },
    /// Independent review DI-M-1: `WakeDesk.markServed(id)`: the post that served the collection's desk request, recorded inside its window.
    Served { collection: [u8; 20], id: u64 },
}

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct HomeTx {
    pub step: Step,
    pub raw: Vec<u8>,
    pub hash: [u8; 32],
    pub nonce: u64,
    /// Review round 4 (C4-H-1): since when its nonce is used on chain while it is seen nowhere (not mined at the pin, in no
    /// provider's pool): past `HOME_NONCE_TAKEN_SECS` it is dropped (another transaction of ours holds its nonce, so it can never
    /// mine) and its step is taken back, never waited on for ever.
    #[serde(default)]
    pub used_since: Option<u64>,
    /// Review round 5 (F5-M-1): the 4663 pin it was signed at (its nonce was unused there): where the search for what used the nonce
    /// starts.
    #[serde(default)]
    pub pin: Option<u64>,
    /// Review round 6 (D6-L-2): when it was signed: the monitor pages a home transaction out this long whatever the providers show
    /// (a stale pool entry, providers that disagree), not only one seen nowhere.
    #[serde(default)]
    pub sent_at: Option<u64>,
}

/// A payout transaction the operator hands in for a written-off order (`eth_credit_payout`, C4-M-1): `(buy, on 4663, tx, the wei it
/// paid when the transaction does not say, handed in at)`.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PayoutClaim {
    pub buy: u64,
    pub home: bool,
    pub tx: [u8; 32],
    pub wei: u128,
    pub at: u64,
}

/// A transaction as the payout judge reads it (either chain, at its pin): who sent it where, with what, and whether it succeeded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PayoutTx {
    pub from: [u8; 20],
    pub to: [u8; 20],
    pub value: u128,
    pub input: Vec<u8>,
    pub block: u64,
    pub success: bool,
}

impl PayoutTx {
    /// A transaction's projection (`evmsend::payout_view`: `from`, `to`, `value`, `input`, hex) and its receipt's block and status.
    pub fn of(t: &serde_json::Value, block: u64, success: bool) -> Result<PayoutTx, String> {
        let s = |k: &str| t.get(k).and_then(|x| x.as_str()).map(|x| x.trim_start_matches("0x").to_string()).ok_or_else(|| format!("no {k}"));
        let addr = |k: &str| -> Result<[u8; 20], String> {
            hex::decode(s(k)?).map_err(|_| format!("{k} is not hex"))?.try_into().map_err(|_| format!("{k} is not an address"))
        };
        let value = u128::from_str_radix(&s("value")?, 16).map_err(|_| "value is not a quantity".to_string())?;
        let input = hex::decode(s("input")?).map_err(|_| "input is not hex".to_string())?;
        Ok(PayoutTx { from: addr("from")?, to: addr("to")?, value, input, block, success })
    }
}

/// What the runner handed in, kept until used or stale.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct Inbox {
    pub quotes: Vec<(u64, Vec<u8>, u64)>,
    pub listings: Vec<(u64, String, u64)>,
    pub evidence: Vec<([u8; 20], Vec<String>, Vec<[u8; 32]>, u64)>,
    pub deposits: Vec<([u8; 32], u64)>,
    /// Round 4 (C4-M-1): payouts handed in for written-off orders.
    #[serde(default)]
    pub payouts: Option<Vec<PayoutClaim>>,
    /// Review round 5 (F5-M-1): the operator's word on a pending home transaction whose nonce is used and which no provider shows and
    /// the canister could not identify (`eth_home_clear`): `(its nonce, it mined, handed in at)`. Used only for that transaction.
    #[serde(default)]
    pub home_clear: Option<(u64, bool, u64)>,
}

/// The canister's own read of one collection's sales (audit 2026-09-27-nft round 2, A-M-1 / B2-M-1 / D2-M-2): the final blocks
/// `[down, up]` whose `Transfer` logs were read (none while `up` is 0), the chunk size in use, and the transactions found there whose
/// receipts are not judged yet, newest first. Each judged receipt's sales (or none) go to `sale_cache`.
///
/// Round 3 (B3-M-1): the transactions to judge are `pend`, each with the token and sender of its move, read one per token and per
/// sender at a time, oldest first (`sale_order`), so a flood of transfers (gas only) can no longer starve a real sale; `todo` is
/// round 2's queue, moved into `pend` on the first pass. Past `SALE_TODO_MAX` the last in that order are dropped and their blocks
/// are read again (`rescan`) once the queue has room. Judged transactions without a sale are kept by hash in `judged` (the first 8
/// bytes and the block), so they never take a sale's place in `sale_cache` and are never read again.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SaleScan {
    pub collection: [u8; 20],
    pub down: u64,
    pub up: u64,
    pub chunk: u64,
    pub todo: Vec<([u8; 32], u64)>,
    #[serde(default)]
    pub pend: Option<Vec<en::Move>>,
    #[serde(default)]
    pub rescan: Option<(u64, u64)>,
    #[serde(default)]
    pub judged: Option<Vec<(u64, u64)>>,
    /// Review round 4 (B4-H-1): block ranges the IC refused for their size (over 2 MB of `Transfer` logs): read by halves, never
    /// whole again, down to one block.
    #[serde(default)]
    pub bad: Option<Vec<(u64, u64)>>,
    /// Round 4 (B4-H-1): single blocks whose `Transfer` logs are over 2 MB and whose marketplace sale logs could not be read either:
    /// `(block, read again after, tries)`. The walks go past them; they are tried again later, never every pass.
    #[serde(default)]
    pub skipped: Option<Vec<(u64, u64, u32)>>,
    /// Review round 5 (B5-M-1 / B4-L-2): single oversized blocks already read through their marketplace sale logs: never bisected or
    /// asked again.
    #[serde(default)]
    pub done_big: Option<Vec<u64>>,
    /// Review round 5 (B5-M-2): `(range start, failed reads in a row there)`: a start that keeps failing is deferred like an oversized
    /// range.
    #[serde(default)]
    pub stuck: Option<(u64, u32)>,
    /// Review round 5 (B5-L-1): `(the day's start, response bytes the scan reserved since)`: the scan's bound per day.
    #[serde(default)]
    pub spent: Option<(u64, u64)>,
    /// Review round 6 (B6-M-1): blocks a re-anchored forward walk passed over (`SALE_REANCHOR_LAG_BLOCKS`), read newest first by a
    /// walk of their own until done or out of the window.
    #[serde(default)]
    pub hole: Option<(u64, u64)>,
    /// Review round 6 (B6-M-1): the last span the providers refused as too deep to serve (`ScanErr::Deep`): its sales are not
    /// counted. Paged (the window's sample is short); kept until it leaves the window.
    #[serde(default)]
    pub lost: Option<(u64, u64)>,
    /// Review round 6 (B6-M-1 / D6-I-1): after each pass, how many final blocks the newest blocks' walk is behind the final head: the
    /// monitor pages a long lag.
    #[serde(default)]
    pub behind: Option<u64>,
    /// Review round 6 (B6-L-1): receipts that failed (`(transaction, failed reads, not before)`): retried with a bound, behind newer
    /// moves once past `SALE_RECEIPT_FAST_TRIES`.
    #[serde(default)]
    pub rfail: Option<Vec<([u8; 32], u32, u64)>>,
    /// Review round 6 (B6-L-2): deferred ranges whose marketplace logs were NOT read whole (a rate limit, Seaport's half over the cap,
    /// the pass's budget): never the ones evicted while another can be, since their sales are read nowhere else yet.
    #[serde(default)]
    pub raw: Option<Vec<(u64, u64)>>,
    /// Review round 7 (B7-L-3): the final head of the last pass whose forward walk reached it (or where the scan started): how far
    /// the scan is behind is the final head now less this, however the forward walk was re-anchored meanwhile, and whether or not
    /// passes still run (the monitor reads it against its own Ethereum head). `behind` is that lag at the pass's end.
    #[serde(default)]
    pub current: Option<u64>,
    /// Review round 7 (B7-L-1): the hole's top refused as deeper than the providers keep logs, BOTH providers saying so (`nftnet`):
    /// `(first such answer at, such answers, not before)`. The hole is kept and read again (at once `SALE_DEEP_FAST_TRIES` times,
    /// then every `SALE_DEEP_RETRY_SECS`) until it is read or leaves the window; never given up on one answer. Paged.
    #[serde(default)]
    pub hole_deep: Option<(u64, u32, u64)>,
    /// Review round 7 (B7-L-2): receipts answered as a rate limit (`(transaction, busy answers in a row, not before)`): backed off
    /// exponentially up to `SALE_BUSY_RETRY_MAX_SECS`, read only with the retries' share of a pass, so they never hold a newer sale.
    #[serde(default)]
    pub rbusy: Option<Vec<([u8; 32], u32, u64)>>,
    /// Review round 7 (B7-L-2): `(since, passes in a row)` in which a provider answered a read of the scan as a rate limit: paged
    /// past `SALE_BUSY_PAGE_PASSES`.
    #[serde(default)]
    pub busy: Option<(u64, u32)>,
}

/// Everything this side keeps in stable memory (the upgrade tuple's thirteenth element).
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct EthState {
    pub buys: Vec<EthBuy>,
    pub next_buy: u64,
    pub deliveries: Vec<Delivery>,
    pub cursors: Vec<([u8; 20], u128)>,
    pub deposits: Vec<DepositRec>,
    pub readings: Vec<Reading>,
    pub line: Option<EthTx>,
    /// What our mined Ethereum transactions spent in all (value and fees): the arrival watch subtracts it.
    pub eth_out: u128,
    pub pending: Option<HomeTx>,
    /// Every Relay order id ever accepted (rule 12). Never pruned.
    pub used_orders: Vec<[u8; 32]>,
    /// Every token our own transactions moved out of our Ethereum address, and the block (deliveries and returns). A stray's
    /// return is judged against it: a token that left us after the claimed transfer came in was brought back by another one.
    pub outbound: Vec<([u8; 20], Word, u64)>,
    pub inbox: Inbox,
    /// The last collection a buy was started for (round robin).
    pub rr: u64,
    /// Our last mined Ethereum transaction: `(nonce, block)` (audit 2026-09-27-nft, B-M-4). Nothing is signed below `nonce + 1`,
    /// whatever a pin says, and nothing while the pin is below `block` (a pin that moved backwards).
    #[serde(default)]
    pub last_mined: Option<(u64, u64)>,
    /// Set when a transaction that is not ours took the line's nonce (audit 2026-09-27-nft, B-M-3 / C-M-3): nothing more is signed on
    /// Ethereum until the operator has looked (`eth_line_clear`). The line is kept, and read again every tick.
    #[serde(default)]
    pub halted: Option<String>,
    /// Final sale receipts' sales, by hash (B-M-7); since round 2 every judged transaction of the canister's own scan, a sale or not.
    #[serde(default)]
    pub sale_cache: Option<Vec<CachedSales>>,
    /// The canister's own sale scan per collection (round 2).
    #[serde(default)]
    pub sale_scan: Option<Vec<SaleScan>>,
    /// A2-L-1 (2026-09-27): per collection, the wei its purchases left at our Ethereum address (what arrived less the price and the
    /// fill's gas; what a way back's deposit kept less its gas) and not yet swept back to its module. One collection's leftover is
    /// never credited to another's module.
    #[serde(default)]
    pub leftover: Option<Vec<([u8; 20], u128)>>,
    /// No sweep starts before this (a sweep abandoned or refunded waits `SWEEP_COOLDOWN_SECS`).
    #[serde(default)]
    pub sweep_after: Option<u64>,
    /// Review round 4 (C4-H-1): the home line's own floor: `(the next nonce: past our last mined home transaction, the highest home
    /// pin this side has read)`. Nothing is signed at a nonce under the first; nothing of 4663 is read at a pin under the second.
    #[serde(default)]
    pub home_floor: Option<(u64, u64)>,
    /// Round 4 (C4-H-1): the canister-wide pin floors of both chains (`canister::PIN_FLOORS`), written here only when the state is
    /// saved for an upgrade (and shown by `eth_nft_state`).
    #[serde(default)]
    pub pin_floors: Option<crate::pin::PinFloors>,
    /// Round 4 (C4-M-1): every payout transaction ever credited to a written-off order: never twice. Never pruned.
    #[serde(default)]
    pub credited: Option<Vec<[u8; 32]>>,
    /// docs/v7/dormant-collections.md: each collection's life (DORMANT, WAKING, ACTIVE), the wake queue and the desk cursor. `None`
    /// until the first tick with `eth_nft.dormancy` set.
    #[serde(default)]
    pub dorm: Option<dm::Dorm>,
    /// docs/v9/solana-migration.md §4, phase B1: the Solana home's line as B1 kept it. Phase B2 moved the line canister-wide
    /// (`solhome::SolLine`, the upgrade tuple's fourteenth element; `solhome::migrate` at `post_upgrade`): `None` from then on.
    #[serde(default)]
    pub sol: Option<crate::solhome::SolHome>,
    /// Phase B2: this side's step out on the canister-wide Solana line (a copy of the line's pending record), until its outcome is taken.
    #[serde(default)]
    pub sol_step: Option<crate::solhome::LinePending>,
    /// Phase B2: the Solana signatures of payouts handed in (`put_sol_payout`), by their keccak (the key the payout and the credited list
    /// carry). At most 8.
    #[serde(default)]
    pub sol_payouts: Option<Vec<([u8; 32], String)>>,
    /// 2026-10-12 (Robinhood Chain NFTs): the other chains' sides (`ethnet`: one per chain the collections live on beside the config's
    /// own, each its own `EthState`, candid-encoded, with its own Ethereum pin floor in `pin_floors`), here ONLY across an upgrade, as
    /// `pin_floors`: `pre_upgrade` puts them in, `post_upgrade` takes them out. `None` in a state saved before them and in the live state.
    #[serde(default)]
    pub sides: Option<Vec<(u64, Vec<u8>)>>,
}

impl EthState {
    pub fn buy(&self, id: u64) -> Option<&EthBuy> {
        self.buys.iter().find(|b| b.id == id)
    }
    pub fn buy_mut(&mut self, id: u64) -> Option<&mut EthBuy> {
        self.buys.iter_mut().find(|b| b.id == id)
    }
    /// The first open buy (the oldest).
    pub fn open_buy(&self) -> Option<&EthBuy> {
        self.buys.iter().find(|b| !b.finished())
    }
    /// A collection's open buy: at most one per collection (its module allows one advance at a time).
    pub fn open_buy_for(&self, collection: &[u8; 20]) -> Option<&EthBuy> {
        self.buys.iter().find(|b| !b.finished() && b.collection == *collection)
    }
    /// Whether a buy holds the Relay leg (one canister-wide). Review round 5 (C5-M-1): a written-off order that is neither credited
    /// nor released holds it too (`late_hold`): its payout may still land, and no other order's watch may be open to read it.
    pub fn relay_busy(&self) -> bool {
        self.buys.iter().any(|b| !b.finished() && b.in_relay()) || self.late_hold().is_some()
    }
    /// Review round 5 (C5-M-1): the first written-off Relay order still holding the leg's attribution (not credited, not released by
    /// the operator), whatever its buy's phase (closing it as a loss does not stop its payout from landing).
    pub fn late_hold(&self) -> Option<u64> {
        self.buys.iter().find(|b| b.late.as_ref().is_some_and(|l| l.credited.is_none() && l.released_at.is_none())).map(|b| b.id)
    }
    /// The ETH at our Ethereum address earmarked to open buys other than `id` (docs/v7/phase1-nft-pooled.md §3.6).
    pub fn earmarked_except(&self, id: u64) -> u128 {
        self.buys.iter().filter(|b| !b.finished() && b.id != id).map(|b| b.earmark()).fold(0u128, u128::saturating_add)
    }
    /// A2-L-1: a collection's leftover not yet swept back.
    pub fn leftover_of(&self, c: &[u8; 20]) -> u128 {
        self.leftover.as_ref().and_then(|v| v.iter().find(|(x, _)| x == c)).map_or(0, |(_, w)| *w)
    }
    pub fn credit_leftover(&mut self, c: &[u8; 20], wei: u128) {
        if wei == 0 {
            return;
        }
        let v = self.leftover.get_or_insert_with(Vec::new);
        match v.iter_mut().find(|(x, _)| x == c) {
            Some((_, w)) => *w = w.saturating_add(wei),
            None => v.push((*c, wei)),
        }
    }
    fn debit_leftover(&mut self, c: &[u8; 20], wei: u128) {
        if let Some((_, w)) = self.leftover.as_mut().and_then(|v| v.iter_mut().find(|(x, _)| x == c)) {
            *w = w.saturating_sub(wei);
        }
    }
    pub fn delivery_mut(&mut self, w: &[u8; 20], t: u128) -> Option<&mut Delivery> {
        self.deliveries.iter_mut().find(|d| d.wrapper == *w && d.ticket == t)
    }
    pub fn cursor(&self, w: &[u8; 20]) -> u128 {
        self.cursors.iter().find(|(x, _)| x == w).map_or(0, |(_, c)| *c)
    }
    fn set_cursor(&mut self, w: [u8; 20], c: u128) {
        match self.cursors.iter_mut().find(|(x, _)| *x == w) {
            Some(x) => x.1 = c,
            None => self.cursors.push((w, c)),
        }
    }
    pub fn reading(&self, c: &[u8; 20]) -> Option<&Reading> {
        self.readings.iter().find(|r| r.collection == *c)
    }
    pub fn evm_pending(&self) -> bool {
        self.pending.is_some()
    }
    /// A home-chain transaction of this side is out: the EVM home's pending one, or the Solana home line's.
    pub fn home_busy(&self) -> bool {
        self.pending.is_some() || self.sol_step.is_some()
    }
    /// The last block in which a transaction of ours moved this token out.
    pub fn last_out(&self, collection: &[u8; 20], token: &Word) -> Option<u64> {
        self.outbound.iter().filter(|(c, t, _)| c == collection && t == token).map(|(_, _, b)| *b).max()
    }
    /// The token is part of an open buy, or of an open claim other than `(tx)`'s.
    pub fn token_busy(&self, collection: &[u8; 20], token: &Word, except_tx: &[u8; 32]) -> bool {
        let buy = self.buys.iter().any(|b| b.collection == *collection && match &b.phase {
            BPhase::Buying { token: t, .. } | BPhase::Bought { token: t, .. } | BPhase::Final { token: t, .. } | BPhase::Minting { token: t, .. } => t == token,
            _ => false,
        });
        buy || self.deposits.iter().any(|d| d.tx != *except_tx && d.collection == *collection && d.token == *token && d.phase.open())
    }
    fn deposit_mut(&mut self, tx: &[u8; 32], token: &Word) -> Option<&mut DepositRec> {
        self.deposits.iter_mut().find(|d| d.tx == *tx && d.token == *token)
    }
    /// The next nonce the canister may sign at: past every transaction of ours seen mined (B-M-4).
    pub fn nonce_floor(&self) -> u64 {
        self.last_mined.map_or(0, |(n, _)| n + 1)
    }
    fn note_mined(&mut self, nonce: u64, block: u64) {
        let (n0, b0) = self.last_mined.unwrap_or((0, 0));
        self.last_mined = Some((n0.max(nonce), b0.max(block)));
    }
    /// Round 4 (C4-H-1): the least nonce the home line may sign at: past every home transaction of ours seen mined (or seen taken).
    pub fn home_nonce_floor(&self) -> u64 {
        self.home_floor.map_or(0, |(n, _)| n)
    }
    fn note_home_nonce(&mut self, next: u64) {
        let (n, p) = self.home_floor.unwrap_or((0, 0));
        self.home_floor = Some((n.max(next), p));
    }
    /// A buy of this collection holds the day's mint room from its advance until its own mint (audit 2026-09-27-nft, C-H-1 / A-L-1):
    /// no deposit of the collection mints meanwhile, so deposits cannot use up the H3 and H11 room the purchase's mint needs. Round
    /// 2 (C2-M-2): the hold is bounded. A buy `Arrived` for `ARRIVED_HOLD_SECS`, a `Stuck` one, and one going back (`Back*`,
    /// `Returning`) hold nothing: none of them mints soon, and if one ever does, its mint waits for the day's room like any other.
    pub fn buy_holds_mint(&self, collection: &[u8; 20], now: u64) -> bool {
        self.buys.iter().any(|b| !b.finished() && b.collection == *collection && match b.phase {
            BPhase::NeedAdvance | BPhase::Minted { .. } | BPhase::Settling { .. } | BPhase::Returning { .. } | BPhase::Stuck { .. }
                | BPhase::BackFunded { .. } | BPhase::BackSending { .. } | BPhase::BackBridging { .. } => false,
            BPhase::Arrived { since, .. } => now.saturating_sub(b.arrived_at.unwrap_or(since).min(since)) < ARRIVED_HOLD_SECS,
            _ => true,
        })
    }
    fn scan_mut(&mut self, c: &[u8; 20]) -> &mut SaleScan {
        let v = self.sale_scan.get_or_insert_with(Vec::new);
        if let Some(i) = v.iter().position(|x| x.collection == *c) {
            return &mut v[i];
        }
        v.push(SaleScan { collection: *c, down: 0, up: 0, chunk: en::SALE_SCAN_CHUNK_BLOCKS, todo: vec![], pend: None, rescan: None, judged: None,
            bad: None, skipped: None, done_big: None, stuck: None, spent: None, hole: None, lost: None, behind: None, rfail: None, raw: None, current: None, hole_deep: None, rbusy: None, busy: None });
        v.last_mut().expect("pushed")
    }
}

// ---- the world ----------------------------------------------------------------------------------------------------------------------

/// The Ethereum pin: the agreed block and its clock and base fee.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EthPin {
    pub number: u64,
    pub timestamp: u64,
    pub base_fee: u128,
}

/// Our own mined transaction on Ethereum.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OurReceipt {
    pub block: u64,
    pub success: bool,
    /// The gas it cost us (`gasUsed × effectiveGasPrice`).
    pub fee: u128,
}

/// A transaction ready to sign on Ethereum, from a judge that passed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EthPlan {
    pub to: [u8; 20],
    pub value: u128,
    pub data: Vec<u8>,
    pub gas: u64,
    pub max_fee: u128,
    pub priority_fee: u128,
    pub nonce: u64,
    pub token: Word,
    pub total: u128,
}

#[allow(async_fn_in_trait)]
pub trait EthIo: crate::solhome::SolIo {
    /// B-L-6: whether a Bitcoin-side conversion holds the one Relay leg (`convflow::holds_relay`). A side that cannot tell says busy.
    fn frog_relay_busy(&self) -> bool {
        false
    }
    /// B-L-6, fairness: whether a Bitcoin-side conversion waits for the leg (`convflow::wants_relay`): no new Ethereum buy starts then.
    fn frog_relay_waiting(&self) -> bool {
        false
    }
    /// Audit A5-02: whether a reimbursement of the Bitcoin side is owed soon (`convflow::reimbursement_owed`: a frog report or an
    /// item claim that pays our address). A side that cannot tell says owed.
    fn frog_reimbursement_owed(&self) -> bool {
        false
    }
    /// 2026-10-12 (Robinhood Chain NFTs): whether a step on the canister-wide Solana line is another chain's side's (one of its buys,
    /// its collections: `ethnet` runs one side per chain the collections live on, each with its own state, all as `LineOwner::Eth`).
    /// This side leaves such an outcome on the line for its own side to take. A world with one side says no.
    fn other_side_step(&self, _step: &Step) -> bool {
        false
    }
    fn now(&self) -> u64;
    fn home_chain(&self) -> u64;
    /// The chain this side buys on: every read, fill, transfer and Relay leg of the flow is on it (`ethnet`: the side's chain).
    fn eth_chain(&self) -> u64;
    fn config(&self) -> Option<EthNftConfig>;
    fn paused(&self) -> bool;
    /// The Ethereum chain's fee cap, from `evm_chains`.
    fn eth_max_fee(&self) -> u128;
    fn save(&mut self, st: &EthState);
    fn with_caps<R>(&mut self, f: impl FnOnce(&custodycap::CapPolicy, &mut custodycap::Book) -> R) -> R;
    fn outflow_check(&mut self, key: &str, collection: &str, flow: Flow, amount: u128) -> Result<(), String>;
    fn outflow_record(&mut self, key: &str, collection: &str, flow: Flow, amount: u128);
    /// Whether the canister has cycles for the floor evidence pass (`EVIDENCE_CYCLES_FLOOR`).
    fn cycles_ok(&self) -> bool {
        true
    }
    /// docs/v7/dormant-collections.md §11: the canister's liquid cycles balance (`canister_liquid_cycle_balance128`: what it can spend,
    /// its freezing reserve excluded), which the dormancy's limits scale with. `None`: not known (with scaling set, no wake starts).
    fn cycles_balance(&self) -> Option<u128> {
        None
    }

    // the home chain, at this tick's pin
    async fn our_evm(&mut self) -> Result<[u8; 20], String>;
    async fn home_read(&mut self, to: [u8; 20], data: &[u8]) -> Result<Vec<u8>, String>;
    async fn home_clock(&mut self) -> Result<Clock, String>;
    async fn home_balance(&mut self) -> Result<u128, String>;
    /// Estimate, sign at the pinned nonce and fees; nothing is sent. `(raw, hash, nonce)`.
    async fn home_sign(&mut self, to: [u8; 20], value: u128, data: &[u8]) -> Result<(Vec<u8>, [u8; 32], u64), String>;
    async fn home_broadcast(&mut self, raw: &[u8]);
    async fn home_seen(&mut self, hash: [u8; 32]) -> EvmView;
    async fn home_nonce(&mut self) -> Result<u64, String>;
    /// The home chain's finalized block's timestamp (the lower over every provider, rounded down): a ticket resolved strictly
    /// before it is final (the H9 rule, audit 2026-09-27-nft B-L-2 / C-L-6).
    async fn home_finalized_time(&mut self) -> Result<u64, String>;

    // Ethereum, at its own pin, every provider agreeing
    async fn eth_pin(&mut self) -> Result<EthPin, String>;
    /// The lower of every provider's finalized block, rounded down to the bucket.
    async fn eth_finalized(&mut self) -> Result<u64, String>;
    async fn eth_read(&mut self, to: [u8; 20], data: &[u8]) -> Result<Vec<u8>, String>;
    async fn eth_balance(&mut self) -> Result<u128, String>;
    async fn eth_nonce(&mut self) -> Result<u64, String>;
    async fn eth_receipt(&mut self, hash: [u8; 32]) -> Result<Option<OurReceipt>, String>;
    /// `nftnet::check` of a handed-in order for `collection` at most `cap`: every Seaport rule, the simulation, fees.
    async fn eth_plan_buy(&mut self, order_hex: &str, collection: [u8; 20], cap: u128) -> Result<EthPlan, String>;
    /// `transferFrom(us, dest, token)`: ownerOf is us, it simulates, fees within the caps.
    async fn eth_plan_transfer(&mut self, collection: [u8; 20], token: Word, dest: [u8; 20]) -> Result<EthPlan, String>;
    async fn eth_sign(&mut self, tx: &Tx1559) -> Result<Vec<u8>, String>;
    async fn eth_broadcast(&mut self, raw: &[u8]);
    /// A listing verified as floor evidence (`ethnft::judge_listing` at the pin).
    async fn eth_listing(&mut self, order_hex: &str, collection: [u8; 20]) -> Result<Seen, String>;
    /// A sale's receipt at the pin (projected), or `None`.
    async fn eth_sale_receipt(&mut self, hash: [u8; 32]) -> Result<Option<ReceiptV>, String>;
    /// A claimed deposit's transaction and receipt at the pin: `(from, to, input, receipt)`.
    async fn eth_deposit_tx(&mut self, hash: [u8; 32]) -> Result<Option<([u8; 20], [u8; 20], Vec<u8>, ReceiptV)>, String>;
    /// Round 2 (A-M-1 / B2-M-1): the transactions that moved a token of `collection` between two wallets in the blocks
    /// `[from, to]` (`to` final, at or under the pin), every provider agreeing: `(hash, block)`, by block (`ethnft::transfer_logs_body`).
    async fn eth_transfer_txs(&mut self, collection: [u8; 20], from: u64, to: u64) -> Result<Vec<([u8; 32], u64)>, String>;
    /// Round 3 (B3-M-1): the same transactions, each with its token and sender (`ethnft::Move`), so the scan reads one transaction
    /// per token and per sender before a second of either. A world that cannot tell gives each transaction a token and sender of its own.
    async fn eth_transfer_moves(&mut self, collection: [u8; 20], from: u64, to: u64) -> Result<Vec<en::Move>, String> {
        Ok(self.eth_transfer_txs(collection, from, to).await?.into_iter()
            .map(|(hash, block)| en::Move { hash, block, token: hash, from: hash[..20].try_into().expect("20") }).collect())
    }
    /// Round 2 (A2-M-1): a plain call from our Ethereum address (the Relay deposit back to 4663): it simulates at the pin, fees within
    /// the chain's caps, and our balance covers `value` and its gas.
    async fn eth_plan_call(&mut self, to: [u8; 20], value: u128, data: &[u8]) -> Result<EthPlan, String>;
    /// Review round 4 (C4-H-1): this tick's home (4663) pin, when the world can tell (the canister's is never below one already
    /// read: `canister::pin`). `None`: the world does not say; only the nonce floor guards the home line then.
    async fn home_pin(&mut self) -> Result<Option<u64>, String> {
        Ok(None)
    }
    /// Round 4 (B4-H-1): the marketplace sale logs (Seaport 1.5 / 1.6 `OrderFulfilled`, Blur v2 `Execution721*`) of ONE block that name
    /// `collection`, as `Move`s: the route to a block whose `Transfer` logs are over the largest response class. Review round 5
    /// (B5-L-2): a fill padded with thousands of consideration items can make one marketplace's logs of a block that big too; each
    /// marketplace is read on its own, so that hides nothing on the other, and a block unreadable both ways is skipped with a note.
    async fn eth_sale_moves(&mut self, _collection: [u8; 20], _block: u64) -> Result<Vec<en::Move>, String> {
        Err("this world reads no marketplace logs".into())
    }
    /// Review round 5 (B5-M-1): the marketplace sale logs of the blocks `[from, to]` that name `collection` (the route around a range
    /// whose `Transfer` logs are too big to read: one read, filtered by address and topic). A world that reads them one block at a time
    /// says so for a longer range.
    async fn eth_sale_moves_range(&mut self, collection: [u8; 20], from: u64, to: u64) -> Result<Vec<en::Move>, String> {
        if from == to {
            return self.eth_sale_moves(collection, from).await;
        }
        Err("this world reads the marketplace logs of one block at a time".into())
    }
    /// Review round 6 (B6-L-2): `eth_sale_moves_range`, and whether every marketplace answered (a block is remembered as read, never
    /// asked again, only then: a Seaport half refused by a rate limit or its size while Blur's answered is read again later). A world
    /// that cannot tell says whole.
    async fn eth_sale_moves_whole(&mut self, collection: [u8; 20], from: u64, to: u64) -> Result<(Vec<en::Move>, bool), String> {
        Ok((self.eth_sale_moves_range(collection, from, to).await?, true))
    }
    /// Round 4 (B4-H-1): the response bytes the log reads of this tick have reserved so far (every size class tried). The sale scan
    /// spends at most `SALE_SCAN_BYTES_PER_PASS` of it per collection and pass. A world that cannot tell says 0.
    fn logs_bytes(&self) -> u64 {
        0
    }
    /// Review round 5 (B5-L-1): the most one log read can reserve (every size class, on every provider): a read is started only when
    /// this still fits the pass's budget.
    fn logs_read_worst(&self) -> u64 {
        2 * (64_000 + 400_000 + 2_000_000)
    }
    /// Review round 5 (B5-L-1): the response bytes the sale receipts of this tick have reserved so far, and the most one can.
    fn receipt_bytes(&self) -> u64 {
        0
    }
    fn receipt_read_worst(&self) -> u64 {
        2 * (32_000 + 200_000)
    }
    /// Review round 5 (F5-M-1): which transaction of ours used our 4663 nonce `nonce`, found without the transaction index (our nonce
    /// count at past blocks, from `from`, a pin at which it was unused, and that block's transactions from us), every provider agreeing:
    /// its hash, or `None` when the nonce is not used at the pin. An error: it could not be told.
    async fn home_nonce_tx(&mut self, _nonce: u64, _from: Option<u64>) -> Result<Option<[u8; 32]>, String> {
        Err("this world cannot tell which transaction used a nonce".into())
    }
    /// Review round 6 (C6-L-1): a payout transaction's receipt logs (either chain, at its pin, every provider agreeing). A world that
    /// cannot read them says none, and no call to Relay's depository is then a payout.
    async fn payout_logs(&mut self, _home: bool, _hash: [u8; 32]) -> Result<Vec<en::LogV>, String> {
        Ok(vec![])
    }
    /// Round 4 (C4-M-1): a transaction on 4663 (`home`) or Ethereum, read at that chain's pin, every provider agreeing: `None` when it
    /// is not mined at the pin.
    async fn payout_tx(&mut self, _home: bool, _hash: [u8; 32]) -> Result<Option<PayoutTx>, String> {
        Err("this world reads no payout transaction".into())
    }

    // ---- the Solana home (docs/v9/solana-migration.md §4, phases B1 and B2) ------------------------------------------------------
    // A world whose home is Solana says so through `SolIo::sol_home` and answers `SolIo`'s methods; it answers the generic home
    // methods in that home's terms: `home_balance` in lamports (our Solana address at finalized), `home_clock` as `{unix time, slot}`
    // of a finalized block, `home_finalized_time` that block's unix time, `home_chain` Relay's id for Solana, `home_pin` none. It is
    // never asked an EVM read (`home_read`), an EVM signature or a nonce.
}

/// This side's step out on the Solana line (phase B2: `EthState.sol_step`).
fn eth_sol_step(st: &EthState) -> Option<Step> {
    match st.sol_step.as_ref().map(|p| &p.step) {
        Some(crate::solhome::LineStep::Eth(s)) => Some(s.clone()),
        _ => None,
    }
}

/// The home currency's unit, for the log: wei on the EVM home, lamports on Solana.
fn unit_of<I: EthIo>(io: &I) -> &'static str {
    if io.sol_home().is_some() { "lamports" } else { "wei" }
}

/// The home currency's gas keep per advance: wei on the EVM home, lamports on Solana.
fn keep_of<I: EthIo>(io: &I) -> u128 {
    if io.sol_home().is_some() { crate::solhome::SOL_GAS_KEEP_LAMPORTS } else { GAS_KEEP_WEI }
}

/// An amount of the home currency as at most this much ETH could be worth: wei on the EVM home as is; lamports on Solana at this
/// tick's rate plus its tolerance (a payout or a refund is never taken as more than its order sent, in either currency).
async fn wei_cap_of<I: EthIo>(io: &mut I, home: u128) -> Result<u128, String> {
    if io.sol_home().is_none() {
        return Ok(home);
    }
    let r = io.sol_rate().await?;
    Ok(crate::rates::lamports_to_wei(home, r).saturating_mul(10_000 + crate::solhome::FX_TOLERANCE_BPS) / 10_000)
}

/// The same the other way: wei as at most this much of the home currency.
async fn home_cap_of<I: EthIo>(io: &mut I, wei: u128) -> Result<u128, String> {
    if io.sol_home().is_none() {
        return Ok(wei);
    }
    let r = io.sol_rate().await?;
    Ok(crate::rates::wei_to_lamports(wei, r).saturating_mul(10_000 + crate::solhome::FX_TOLERANCE_BPS) / 10_000)
}

/// What the custody caps and the outflow limits count an advance of `home` units at: its ETH worth (wei as is; lamports at the rate).
async fn value_wei_of<I: EthIo>(io: &mut I, home: u128) -> Result<u128, String> {
    if io.sol_home().is_none() {
        return Ok(home);
    }
    let r = io.sol_rate().await?;
    Ok(crate::rates::lamports_to_wei(home, r))
}

fn h(b: &[u8]) -> String {
    format!("0x{}", hex::encode(b))
}
fn key_of(c: &Col, chain: u64) -> String {
    custodycap::evm_key(chain, &c.collection)
}
fn held_id(chain: u64, collection: &[u8; 20], token: &Word) -> String {
    format!("{}:{}", custodycap::evm_key(chain, collection), hex::encode(token))
}

// ---- the tick ------------------------------------------------------------------------------------------------------------------------

/// The whole pass. `others_pending`: a home-chain transaction of another part of the canister is out (one at a time).
pub async fn tick<I: EthIo>(io: &mut I, st: &mut EthState, others_pending: bool) -> Vec<String> {
    let Some(cfg) = io.config() else { return vec![] };
    // docs/v9 §4: the canister's other EVM transactions are on another chain than the Solana home's line: they hold nothing here
    let others_pending = others_pending && io.sol_home().is_none();
    let cols = match cfg.cols() {
        Ok(c) => c,
        Err(e) => return vec![format!("eth: the configuration does not parse: {e}")],
    };
    let mut lines = Vec::new();
    let mut push = |l: String| lines.push(l);
    // 0. round 4 (C4-H-1): the 4663 pin never goes back. A pin below one already read (a provider's head fell back across a bucket)
    // shows our own mined home transactions undone: a nonce we used, the balance before our Relay deposit. Nothing of this tick reads
    // 4663 at it or signs there; the next tick's pin is past it
    if let Err(e) = home_floor_ok(io, st).await {
        push(format!("eth: waits: {e}"));
        io.save(st);
        return lines;
    }
    // 1. our home-chain transaction, if one is out (docs/v9 §4: on the Solana home, the line: what landed, what is out, an incident)
    if let Some(p) = st.pending.clone() {
        push(format!("eth: {}", resolve_home(io, st, &cfg, p).await.unwrap_or_else(|e| format!("waits: {e}"))));
        io.save(st);
    }
    if io.sol_home().is_some() {
        for l in home_sol::resolve(io, st, &cfg).await {
            push(format!("eth: {l}"));
        }
        io.save(st);
    }
    // 2. the Ethereum line. Round 2 (C2-M-1): `unsettled` is set when the line's outcome could not be read at this pin (a receipt or
    // an ownerOf read failed): a transaction of ours may be mined here and not yet counted in `eth_out`, so no arrival watch starts
    let mut unsettled = false;
    if let Some(t) = st.line.clone() {
        push(format!("eth line: {}", resolve_eth(io, st, t, &mut unsettled).await.unwrap_or_else(|e| format!("waits: {e}"))));
        io.save(st);
    }
    if io.paused() {
        // audit 2026-09-27-nft B-L-4 / C-L-1, R3 M-1(c) on the Ethereum side: nothing new leaves while paused, but what already
        // left is still accounted for. The line was read above (a mined transfer is Moved; a delivery inside its last day is
        // cancelled, a 0-value self-send that moves nothing), and a Moved delivery that is final is recorded on the home chain.
        for l in delivery_pass(io, st, &cfg).await {
            push(l);
        }
        io.save(st);
        if !st.home_busy() && !others_pending {
            match home_act_paused(io, st).await {
                Ok(Some(l)) => push(l),
                Ok(None) => {}
                Err(e) => push(format!("eth: the paused pass's record waits: {e}")),
            }
            io.save(st);
        }
        push("eth: paused: nothing new is signed (the line is read, final deliveries are recorded, one in its last day is cancelled)".into());
        return lines;
    }
    // 2b. docs/v7/dormant-collections.md: each collection's life: wake requests (the desk, deposits, the watchdog), the caps, timeouts,
    // and the checks that send an idle collection back to DORMANT
    if cfg.dormancy.is_some() {
        for l in dorm_pass(io, st, &cfg, &cols).await {
            push(l);
        }
        io.save(st);
    }
    // 3. each collection: its tickets, its floor evidence. A DORMANT (or queued) collection is not read at all: its evidence is
    // dropped without an outcall; a WAKING one holds no ticket (it holds nothing)
    for c in &cols {
        let k = kind_of(st, &cfg, &c.collection);
        if k.asleep() {
            if let Some(i) = st.inbox.evidence.iter().position(|(x, ..)| *x == c.collection) {
                st.inbox.evidence.remove(i);
                push(format!("eth floor 0x{}: {}: its evidence is not read (no outcall; a wake request wakes it)", hex::encode(c.collection), k.word()));
            }
            continue;
        }
        if k != Kind::Waking {
            for l in scan_tickets(io, st, c).await {
                push(l);
            }
        }
        if let Some(l) = floor_pass(io, st, c, &cfg, k).await {
            push(l);
        }
        io.save(st);
    }
    // 4. round 4 (C4-M-1): a late payout handed in for a written-off order, judged against that order; then every open buy, one
    // step each; a new one when the Relay leg is free
    for l in payout_pass(io, st, &cfg).await {
        push(l);
    }
    for l in buy_pass(io, st, &cfg, &cols).await {
        push(l);
    }
    if let Some(l) = sweep_pass(io, st, &cfg, &cols).await {
        push(l);
    }
    io.save(st);
    // 5. deliveries (finality, cancels, marks that need no transaction: C-M-2, whatever holds the line), then deposits
    for l in delivery_pass(io, st, &cfg).await {
        push(l);
    }
    for l in mark_pass(io, st).await {
        push(l);
    }
    io.save(st);
    if cfg.deposits {
        for l in deposit_pass(io, st, &cfg, &cols).await {
            push(l);
        }
        io.save(st);
    }
    // 6. at most one Ethereum transaction: a delivery first (a holder is waiting), then a buy
    if st.line.is_none() {
        if let Some(h) = st.halted.clone() {
            push(format!("eth line: HALTED, nothing is signed on Ethereum until the operator looks (eth_line_clear): {h}"));
        } else {
            match eth_act(io, st, &cfg, &cols).await {
                Ok(Some(l)) => push(l),
                Ok(None) => {}
                Err(e) => push(format!("eth line: waits: {e}")),
            }
        }
        io.save(st);
    }
    // 7. at most one home-chain transaction
    if !st.home_busy() {
        if others_pending {
            push("eth: the home-chain act waits: another transaction of ours is out (one at a time)".into());
        } else {
            match home_act(io, st, &cfg, &cols, unsettled).await {
                Ok(Some(l)) => push(l),
                Ok(None) => {
                    // phase B2: nothing to sign: this side's turn on the canister-wide Solana line, if it held one, is given up
                    if io.sol_home().is_some() {
                        crate::solhome::release(io, crate::solhome::LineOwner::Eth);
                    }
                }
                Err(e) => {
                    // audit A5-08: an act that failed before it reached the line gives up a turn it no longer takes
                    if io.sol_home().is_some() {
                        release_untaken_turn(io, crate::solhome::LineOwner::Eth);
                    }
                    push(format!("eth: the home-chain act waits: {e}"))
                }
            }
        }
        io.save(st);
    }
    lines
}

/// Audit A5-08: after a flow's act on the Solana home ended without signing (an error), its turn in the line's queue is given up when
/// that turn is at the head of a FREE line: an owner whose act reached [`crate::solhome::ready`] there would have taken the line
/// (`claim` removes it from the queue), so a head entry left on a free line is one its act did not use this tick (it failed before
/// `ready`: a refused quote, a read). Kept while the line is busy or another owner is ahead: that is the queued wait itself, and the
/// owner keeps its place.
pub(crate) fn release_untaken_turn<I: crate::solhome::SolIo>(io: &mut I, owner: crate::solhome::LineOwner) {
    let l = io.sol_line();
    if !l.busy() && l.queue.first().is_some_and(|(o, _)| *o == owner) {
        crate::solhome::release(io, owner);
    }
}

// ---- reading the wrappers --------------------------------------------------------------------------------------------------------

async fn read_ticket<I: EthIo>(io: &mut I, w: [u8; 20], id: u128) -> Result<TicketV, String> {
    if io.sol_home().is_some() {
        return home_books::ticket_v(io, w, id).await;
    }
    en::decode_ticket(&io.home_read(w, &en::call_word(en::TICKET, crate::abi::uint_word(id))).await?)
}

async fn token_of<I: EthIo>(io: &mut I, w: [u8; 20], item: u128) -> Result<Word, String> {
    if io.sol_home().is_some() {
        return home_books::token_of(io, w, item).await;
    }
    Ok(en::decode_item(&io.home_read(w, &en::call_word(en::ITEM, crate::abi::uint_word(item))).await?)?.0)
}

/// New resolved tickets become deliveries, in order; a ticket still pinned stops the walk (it resolves in order).
async fn scan_tickets<I: EthIo>(io: &mut I, st: &mut EthState, c: &Col) -> Vec<String> {
    let mut lines = Vec::new();
    let count = match if io.sol_home().is_some() { home_books::ticket_count(io, c).await } else {
        io.home_read(c.wrapper, &en::TICKET_COUNT).await.and_then(|r| crate::abi::uint_at(&r, 0, 0, 128)) } {
        Ok(n) => n,
        Err(e) => return vec![format!("eth: wrapper 0x{} not read: {e}", hex::encode(c.wrapper))],
    };
    let mut cur = st.cursor(&c.wrapper);
    let end = count.min(cur + TICKETS_PER_TICK as u128);
    while cur < end {
        let t = match read_ticket(io, c.wrapper, cur).await {
            Ok(t) => t,
            Err(e) => {
                lines.push(format!("eth: ticket {cur} not read: {e}"));
                break;
            }
        };
        match t.stage {
            Stage::Pinned | Stage::None => break,
            Stage::Resolved => {
                let token = match token_of(io, c.wrapper, t.item_id).await {
                    Ok(x) => x,
                    Err(e) => {
                        lines.push(format!("eth: ticket {cur}'s item not read: {e}"));
                        break;
                    }
                };
                st.deliveries.push(Delivery { wrapper: c.wrapper, collection: c.collection, ticket: cur, token, destination: t.destination,
                    deliver_by: t.deliver_by, phase: DPhase::Waiting, note: String::new(), refused_since: None, sent: None });
                lines.push(format!("eth: ticket {cur}: resolved to token {}, delivery to {} due by {}", h(&token), h(&t.destination), t.deliver_by));
            }
            // delivered or refunded before we saw it (another canister's record cannot exist; a refund after the late window can)
            Stage::Delivered | Stage::Refunded => {}
        }
        cur += 1;
        st.set_cursor(c.wrapper, cur);
    }
    lines
}

// ---- the floor -----------------------------------------------------------------------------------------------------------------------

async fn floor_pass<I: EthIo>(io: &mut I, st: &mut EthState, c: &Col, cfg: &EthNftConfig, k: Kind) -> Option<String> {
    let i = st.inbox.evidence.iter().position(|(x, ..)| *x == c.collection)?;
    // round 2 (A-M-1 / B2-M-1 / D2-M-2): the runner hands in LISTINGS only; any sale hashes it sends are not used, the canister
    // reads the collection's sales itself (`sale_scan`)
    let (_, listings, _, at) = st.inbox.evidence.remove(i);
    let now = io.now();
    if now.saturating_sub(at) > HANDIN_TTL_SECS {
        return Some(format!("eth floor 0x{}: evidence handed in {} s ago: stale, not used", hex::encode(c.collection), now - at));
    }
    // docs/v7/dormant-collections.md §4: an ACTIVE collection's pass and scan run on the slow cadence, every hand-in when hot (a
    // buy, a deposit waiting for its mint, a module that wanted an item); a WAKING one's every hand-in, with the scan
    let hot = is_hot(st, &c.collection, now);
    let life = st.dorm.as_ref().and_then(|d| d.life(&c.collection)).cloned();
    if let (Kind::Active, Some(l)) = (k, life.as_ref()) {
        if !dm::pass_due(l, now, hot) {
            // DR-M-3: the hand-in is kept (until its TTL, or a newer one) for the pass that comes due
            st.inbox.evidence.push((c.collection, listings, vec![], at));
            let next = (l.pass_at + dm::PASS_EVERY_SECS).min((l.posted_at + dm::FLOOR_TTL_SECS).saturating_sub(dm::TTL_MARGIN_SECS));
            return Some(format!("eth floor 0x{}: waits: not due (slow cadence): the next pass from {next}", hex::encode(c.collection)));
        }
    }
    let scan_due = match (k, life.as_ref()) {
        (Kind::Active, Some(l)) => dm::scan_due(l, now, hot),
        _ => true,
    };
    let receipts = if k == Kind::Waking { dm::WAKE_RECEIPTS_PER_PASS } else { en::SALE_RECEIPTS_PER_PASS };
    let sales_before = sales_cached(st, &c.collection);
    // round 4 (B4-H-1): low cycles stop the canister's own sale scan (its largest outcall bill), never the post: the listings are
    // still verified and the floor posted from them and the sales already read, within the module's bounds
    let scan = io.cycles_ok() && scan_due;
    // the module's floor first: a post that is not confirmed by the sales is held against it (round 2)
    let (accepted, target, posted_at) = match module_floor(io, c.module).await {
        Ok(x) => x,
        Err(e) => return Some(format!("eth floor 0x{}: the module's floor not read: {e}", hex::encode(c.collection))),
    };
    let got = reading_of(io, st, c, cfg, &listings, accepted, scan, receipts).await;
    let behind = st.sale_scan.iter().flatten().find(|x| x.collection == c.collection).and_then(|x| x.behind).unwrap_or(u64::MAX);
    let new_sales = sales_cached(st, &c.collection) > sales_before;
    let mut wake_note = String::new();
    let mut backfilled = false;
    if let Some(l) = st.dorm.as_mut().and_then(|d| d.life_mut(&c.collection)) {
        l.pass_at = now;
        l.posted_at = l.posted_at.max(posted_at);
        if scan {
            dm::note_scan(l, now, new_sales);
        }
        l.read_scan = got.is_ok().then_some((now, scan, behind));
    }
    if k == Kind::Waking {
        // §2.3: no post before the backfill is complete and the reading confirmed; the estimate at every pass
        let top = got.as_ref().map(|(_, pin, _)| pin.saturating_sub(en::sale_blocks(cfg.chain_id).final_depth)).ok();
        let sc = st.sale_scan.iter().flatten().find(|x| x.collection == c.collection).cloned();
        let gaps = top.map_or_else(|| vec!["no reading at this pass".to_string()], |t| dm::backfill_gaps(sc.as_ref(), t));
        let confirmed = got.as_ref().is_ok_and(|(p, ..)| p.held.is_none() && p.sales >= cfg.floor_params().min_sales as u32);
        backfilled = gaps.is_empty() && confirmed;
        let reads = (en::sale_blocks(cfg.chain_id).chunks_per_pass as u64 * en::SALE_SCAN_MAX_CHUNK_BLOCKS).div_ceil(cfg.logs_max().max(1));
        let seen = sc.as_ref().map_or(0, |x| x.judged.as_ref().map_or(0, |j| j.len() as u64))
            + st.sale_cache.iter().flatten().filter(|x| x.collection == c.collection).count() as u64;
        let eta = if backfilled { dm::WAKE_PASS_SECS } else { dm::wake_eta_secs(sc.as_ref(), top.unwrap_or(0), reads, receipts as u64, seen) };
        if let Some(l) = st.dorm.as_mut().and_then(|d| d.life_mut(&c.collection)) {
            l.eta = Some((eta, now));
            if let dm::Phase::Waking { backfilled_at, .. } = &mut l.phase {
                if backfilled && backfilled_at.is_none() {
                    *backfilled_at = Some(now);
                }
            }
            l.note = if backfilled { "warming up: the 7-day backfill is read and the reading confirmed: the first post is next".into() }
                else { format!("warming up (~{} min): {}{}", eta.div_ceil(60), gaps.join(", "), if confirmed { "" } else { "; the reading is not confirmed yet" }) };
            wake_note = format!("; WAKING: {}", l.note);
        }
    }
    let text = match got {
        Ok((post, pin, note)) => {
            // §2.3: while WAKING, a post is due only once the backfill is complete and the reading confirmed
            let due = (k != Kind::Waking || backfilled) && en::should_post(post.price, accepted, target, posted_at, now);
            let text = format!("eth floor 0x{}: {} wei (listing {}, counted sales' median {} and p25 {}, confidence {}, {} listings, {} sales \
                counted{}{}){}", hex::encode(c.collection), post.price, post.listing, post.sale_median, post.sale_p25, post.confidence,
                post.listings, post.sales, post.held.as_ref().map(|w| format!("; HELD: {w}")).unwrap_or_default(), note,
                if due { ": a post is due" } else { "" }) + &wake_note;
            st.readings.retain(|x| x.collection != c.collection);
            st.readings.push(Reading { collection: c.collection, post, at: now, pin, due,
                module_posted_at: (posted_at != 0).then_some(posted_at) });
            text
        }
        Err(e) => {
            // B2-I-1: no reading now: the last one is neither posted nor used for a buy or a deposit any more
            st.readings.retain(|x| x.collection != c.collection);
            format!("eth floor 0x{}: {e}{wake_note}", hex::encode(c.collection))
        }
    };
    Some(text)
}

/// Sales in the canister's cache for `c` (a count, for the scan's back-off: a pass that added none backs it off).
fn sales_cached(st: &EthState, c: &[u8; 20]) -> usize {
    st.sale_cache.iter().flatten().filter(|x| x.collection == *c).map(|x| x.sales.len()).sum()
}

/// The canister's own read of `c`'s sales (audit 2026-09-27-nft round 2, A-M-1 / B2-M-1 / D2-M-2): the collection's `Transfer` logs
/// over the final blocks of the window, newest first (a few chunks a pass, a chunk halved when a read fails and grown back after),
/// then up to `SALE_RECEIPTS_PER_PASS` receipts of the transactions found, in `sale_order` (round 3, B3-M-1: one per token and per
/// sender at a time, oldest first). Every judged receipt is kept (a sale in `sale_cache`, none by hash in `judged`), so each
/// transaction is read once; a receipt the IC refuses for its size at the largest class is final and kept as no sale (B2-L-2).
/// Nobody hands anything in here: which sales count is the canister's own reading. Returns a note on what was not read this pass.
///
/// Review round 5 (B5-M-1 / B5-M-2 / B5-L-1): a range too big to read (the IC's size reject, a provider's own "too many results" or
/// limit error, or any error repeated `SALE_SCAN_STUCK_FAILS` times at one start) never holds a walk: its sales are read at once
/// through the marketplaces' own sale logs (one read, filtered by address and topic), and the range is deferred (`SaleScan.bad`) to be
/// bisected later through its `Transfer` logs for any other sale, on what is left of the pass. So a walk's progress costs the
/// attacker one oversized block per chunk it holds for one pass, never a week; the first read anchors the scan even when it is too
/// big, so a collection's first post needs only the newest blocks' sales. The pass's reserved bytes are a real bound: logs and
/// receipts each have their own, a read is started only when its worst case fits, and the day's scan has a bound too.
async fn sale_scan<I: EthIo>(io: &mut I, st: &mut EthState, c: &Col, pin: u64, max_chunk: u64, receipts_per_pass: usize) -> String {
    // audit C3-02: every count of blocks in this chain's blocks (the window is 7 days of them, not 50,400 on any chain)
    let sb = en::sale_blocks(io.eth_chain());
    let top = pin.saturating_sub(sb.final_depth);
    let bottom = top.saturating_sub(sb.window);
    let now = io.now();
    let mut sc = st.scan_mut(&c.collection).clone();
    // round 2's queue joins round 3's (each its own token and sender: it groups with nothing)
    let mut pend: Vec<en::Move> = sc.pend.take().unwrap_or_default();
    for (hash, block) in std::mem::take(&mut sc.todo) {
        pend.push(en::Move { hash, block, token: hash, from: hash[..20].try_into().expect("20") });
    }
    if sc.up != 0 && sc.up < bottom {
        // the window moved past what was read (a long outage): start again from the newest blocks. Round 4 (B4-I-3): a pin that went
        // back (up over top) no longer resets the scan: the newest blocks simply wait until the pin is past them again
        sc.up = 0;
        sc.down = 0;
        pend.clear();
        sc.rescan = None;
    }
    // round 4 (B4-H-1), round 5: what is known about oversized blocks and deferred ranges, kept to the window
    if let Some(b) = sc.bad.as_mut() {
        b.retain(|(_, y)| *y >= bottom);
    }
    if let Some(s) = sc.skipped.as_mut() {
        s.retain(|(b, ..)| *b >= bottom);
    }
    if let Some(d) = sc.done_big.as_mut() {
        d.retain(|b| *b >= bottom);
    }
    // review round 6 (B6-M-1 / B6-L-2): the hole, what was lost, the raw ranges and the failed receipts, kept to the window
    sc.hole = sc.hole.filter(|(_, y)| *y >= bottom).map(|(x, y)| (x.max(bottom), y));
    sc.lost = sc.lost.filter(|(_, y)| *y >= bottom);
    if let Some(r) = sc.raw.as_mut() {
        r.retain(|(_, y)| *y >= bottom);
    }
    // review round 7 (B7-L-1): a too-deep record lives only as long as the hole it is about
    if sc.hole.is_none() {
        sc.hole_deep = None;
    }
    // review round 7 (B7-L-3): where the scan was last current (a state saved before this round: where its forward walk is, else here)
    if sc.current.is_none() {
        sc.current = Some(if sc.up != 0 { sc.up.min(top) } else { top });
    }
    let mut notes = Vec::new();
    let mut found: Vec<en::Move> = Vec::new();
    // review round 5 (B5-L-1): the day's bound, then the pass's (receipts first: what was found is judged)
    let (day, spent_day) = match sc.spent {
        Some((d, b)) if now < d.saturating_add(86_400) => (d, b),
        _ => (now, 0),
    };
    let left_day = en::SALE_SCAN_BYTES_PER_DAY.saturating_sub(spent_day);
    let receipts_cap = en::SALE_RECEIPT_BYTES_PER_PASS.min(left_day);
    let logs_cap = en::SALE_SCAN_BYTES_PER_PASS.min(left_day - receipts_cap);
    if left_day == 0 {
        notes.push(format!("the day's scan budget ({} bytes) is spent: the scan waits until {}", en::SALE_SCAN_BYTES_PER_DAY, day + 86_400));
    }
    // never wider than the provider serves (eth_nft.logs_max_blocks; a chunk saved at the old 2,000 is cut to it). A pass covers as
    // many blocks as 13 chunks of 2,000 while the window is being read; once it is read, a pass asks for the new blocks alone
    let max_chunk = max_chunk.clamp(en::SALE_SCAN_MIN_CHUNK_BLOCKS, en::SALE_SCAN_MAX_CHUNK_BLOCKS);
    let mut p = ScanPass { reads: (sb.chunks_per_pass as u64 * en::SALE_SCAN_MAX_CHUNK_BLOCKS).div_ceil(max_chunk) as usize,
        logs_from: io.logs_bytes(), receipts_from: io.receipt_bytes(), logs_cap, receipts_cap, busy: false };
    sc.chunk = sc.chunk.clamp(en::SALE_SCAN_MIN_CHUNK_BLOCKS, max_chunk);
    let grow = |x: u64| x.saturating_mul(2).min(max_chunk);
    let shrink = |x: u64| (x / 2).max(en::SALE_SCAN_MIN_CHUNK_BLOCKS);
    // review round 6 (B6-M-1): a forward walk a day or more behind the final head is re-anchored at the newest blocks; what it passes
    // over is read newest first by the hole's own walk (below), never waited on until the window resets
    if sc.up != 0 && top > sc.up.saturating_add(sb.reanchor_lag) {
        notes.push(reanchor(&mut sc, top, bottom, "a day or more behind the final head"));
    }
    // the newest blocks first: the first read walks down from the newest final block and sets both ends, even when it is too big
    let mut fails = 0;
    while p.can_read(io) && fails < en::SALE_SCAN_FAILS_PER_WALK && (sc.up == 0 || sc.up < top) && top > 0 {
        let first = sc.up == 0;
        let (lo, hi) = if first { (top.saturating_sub(sc.chunk - 1).max(bottom), top) } else { (sc.up + 1, top.min(sc.up + sc.chunk)) };
        match scan_read(io, c, &mut sc, &mut p, lo, hi, !first, now, &mut notes).await {
            Got::Read(a, b, v) => {
                if first {
                    sc.down = a;
                }
                sc.up = b;
                found.extend(v);
                sc.chunk = grow(sc.chunk);
            }
            Got::Failed(a, b, e, k) => {
                fails += 1;
                notes.push(format!("the transfer logs of blocks {a}..{b} not read ({e})"));
                match k {
                    // review round 6 (B6-L-2): a rate limit: the walk backs off for the pass (read again next pass), nothing deferred
                    ScanErr::Busy => break,
                    // review round 6 (B6-M-1): the providers keep no logs this deep (a gap longer than their history): re-anchor at the
                    // newest blocks instead of asking the same refused start until the window resets (≈ 5 days)
                    ScanErr::Deep if !first && top > sc.up.saturating_add(en::SALE_SCAN_MAX_CHUNK_BLOCKS) => {
                        notes.push(reanchor(&mut sc, top, bottom, "refused as deeper than the providers keep logs"));
                    }
                    ScanErr::Deep => {}
                    _ => sc.chunk = shrink(sc.chunk),
                }
            }
        }
    }
    // review round 7 (B7-L-3): the forward walk is at the final head: the scan is current here
    if sc.up != 0 && sc.up >= top {
        sc.current = Some(top);
    }
    // review round 6 (B6-M-1): the blocks a re-anchor passed over, newest first. Review round 7 (B7-L-1): a span refused as too deep
    // (both providers saying so: `nftnet`) is never given up on that answer: the hole is kept, recorded (`hole_deep`, paged) and read
    // again, at once `SALE_DEEP_FAST_TRIES` times, then every `SALE_DEEP_RETRY_SECS`, until it is read or leaves the window
    let mut fails = 0;
    let hole_due = sc.hole_deep.is_none_or(|(_, _, at)| at <= now);
    while hole_due && p.can_read(io) && fails < en::SALE_SCAN_FAILS_PER_WALK {
        let Some((x, y)) = sc.hole else { break };
        let from = y.saturating_sub(sc.chunk - 1).max(x);
        match scan_read(io, c, &mut sc, &mut p, from, y, false, now, &mut notes).await {
            Got::Read(a, _, v) => {
                found.extend(v);
                sc.hole = if a <= x { None } else { Some((x, a - 1)) };
                sc.hole_deep = None;
                sc.chunk = grow(sc.chunk);
            }
            Got::Failed(a, b, e, k) => {
                fails += 1;
                notes.push(format!("the transfer logs of blocks {a}..{b} (passed over by a re-anchor) not read ({e})"));
                match k {
                    ScanErr::Busy => break,
                    ScanErr::Deep => {
                        let (first, n) = sc.hole_deep.map_or((now, 0), |(f, n, _)| (f, n));
                        let n = n.saturating_add(1);
                        let again = if n < en::SALE_DEEP_FAST_TRIES { now } else { now.saturating_add(en::SALE_DEEP_RETRY_SECS) };
                        sc.hole_deep = Some((first, n, again));
                        notes.push(format!("blocks {x}..{b} (passed over by a re-anchor) are refused as deeper than the providers keep logs \
                            ({n} time(s) since {first}): kept and read again {}; their sales are not counted until then (B7-L-1)",
                            if again == now { "next pass".to_string() } else { format!("after {again}") }));
                        break;
                    }
                    _ => sc.chunk = shrink(sc.chunk),
                }
            }
        }
    }
    if sc.hole.is_none() {
        sc.hole_deep = None;
    }
    // then back to the window's start
    let mut fails = 0;
    while p.can_read(io) && fails < en::SALE_SCAN_FAILS_PER_WALK && sc.up != 0 && sc.down > bottom {
        let to = sc.down - 1;
        let from = to.saturating_sub(sc.chunk - 1).max(bottom);
        match scan_read(io, c, &mut sc, &mut p, from, to, false, now, &mut notes).await {
            Got::Read(a, _, v) => {
                sc.down = a;
                found.extend(v);
                sc.chunk = grow(sc.chunk);
                // review round 6: blocks read after all (a provider back to depth) are no longer lost
                if let Some((p0, q0)) = sc.lost.filter(|(_, q0)| *q0 >= a) {
                    sc.lost = (p0 < a).then(|| (p0, a - 1).min((p0, q0)));
                }
            }
            Got::Failed(a, b, e, k) => {
                fails += 1;
                notes.push(format!("the transfer logs of blocks {a}..{b} not read ({e})"));
                match k {
                    ScanErr::Busy => break,
                    ScanErr::Deep => {
                        // review round 5: the providers keep no logs that deep (a keyless endpoint's history): the walk back stops here
                        // this pass, and the newer blocks' walk keeps its chunk. Review round 6 (B6-M-1): recorded and paged: the window's
                        // sample is short of its 7 days
                        sc.lost = Some(sc.lost.map_or((bottom, b), |(p0, q0)| (p0.min(bottom), q0.max(b))));
                        break;
                    }
                    _ => sc.chunk = shrink(sc.chunk),
                }
            }
        }
    }
    sc.down = sc.down.max(bottom);
    // round 3 (B3-M-1): blocks whose transactions were dropped from a full queue are read again once it has room, so no real sale
    // is lost for good (a flood that keeps the queue full keeps being read in `sale_order`, one per token and sender first)
    // round 4 (B4-L-1): one chunk a pass is re-read even while the queue is full, so a flood that keeps it full (its repeats rank last
    // and are the ones dropped) cannot hold the re-read of an honest overflow for as long as it lasts
    let mut fails = 0;
    let mut share = true;
    while p.can_read(io) && fails < en::SALE_SCAN_FAILS_PER_WALK && (share || pend.len() + found.len() < en::SALE_TODO_MAX / 2) {
        share = false;
        let Some((a, b)) = sc.rescan else { break };
        let a = a.max(bottom);
        if a > b {
            sc.rescan = None;
            break;
        }
        let to = b.min(a.saturating_add(sc.chunk - 1));
        match scan_read(io, c, &mut sc, &mut p, a, to, true, now, &mut notes).await {
            Got::Read(_, y, v) => {
                found.extend(v);
                sc.rescan = if y >= b { None } else { Some((y + 1, b)) };
                sc.chunk = grow(sc.chunk);
            }
            Got::Failed(x, y, e, k) => {
                fails += 1;
                notes.push(format!("the transfer logs of blocks {x}..{y} (read again) not read ({e})"));
                match k {
                    ScanErr::Busy | ScanErr::Deep => break,
                    _ => sc.chunk = shrink(sc.chunk),
                }
            }
        }
    }
    // review round 5 (B5-M-1): the deferred ranges, newest first, on what is left of the pass: each is bisected through its Transfer
    // logs (the halves that read are its other marketplaces' sales), down to single oversized blocks, read through their own
    // marketplace logs once and remembered. Never a walk's budget, never ahead of a walk
    let mut rfails = 0;
    while p.can_read(io) && rfails < en::SALE_REFINE_FAILS_PER_PASS {
        let Some(i) = sc.bad.as_ref().and_then(|b| (0..b.len()).max_by_key(|&i| b[i].1)) else { break };
        let (x, y) = sc.bad.as_ref().expect("some")[i];
        // an oversized block inside it that is already read (or skipped) is never asked again: the range is split around it
        if let Some(k) = sc.done_big.iter().flatten().copied().chain(sc.skipped.iter().flatten().map(|(b, ..)| *b)).find(|k| x <= *k && *k <= y) {
            let bad = sc.bad.as_mut().expect("some");
            bad.remove(i);
            if k > x {
                bad.push((x, k - 1));
            }
            if k < y {
                bad.push((k + 1, y));
            }
            continue;
        }
        // its upper half (the one block itself when it is one): read, or split again; one block too big goes through its own
        // marketplace logs
        let mid = x + (y - x + 1) / 2;
        let mid = if x == y { x } else { mid };
        p.reads = p.reads.saturating_sub(1);
        match io.eth_transfer_moves(c.collection, mid, y).await {
            Ok(v) => {
                found.extend(v);
                let bad = sc.bad.as_mut().expect("some");
                if mid == x { bad.remove(i); } else { bad[i] = (x, mid - 1); }
            }
            Err(e) => {
                rfails += 1;
                p.busy |= scan_error(&e) == ScanErr::Busy;
                if scan_error(&e) != ScanErr::TooBig {
                    notes.push(format!("the deferred blocks {mid}..{y} not read ({e})"));
                    break;
                }
                let raw = is_raw(&sc, x, y);
                let bad = sc.bad.as_mut().expect("some");
                if mid == x { bad.remove(i); } else { bad[i] = (x, mid - 1); }
                if mid == y {
                    let Got::Read(_, _, v) = block_sales(io, c, &mut sc, &mut p, y, now, &mut notes).await else { break };
                    found.extend(v);
                } else {
                    // review round 6 (B6-I-2): the split is kept to the bound too (was pushed past it: 529 ranges seen)
                    defer(&mut sc, mid, y, raw);
                }
            }
        }
    }
    if let Some(b) = sc.bad.as_ref().filter(|b| !b.is_empty()) {
        notes.push(format!("{} deferred range(s) ({}) read through the marketplaces' sale logs only, so far: their other sales are read \
            later", b.len(), b.iter().map(|(x, y)| format!("{x}..{y}")).collect::<Vec<_>>().join(", ")));
    }
    // round 4 (B4-H-1): at most one skipped block a pass is tried again, and only once it is due
    // review round 6 (B5-L-2 / B6-L-2): a skipped block is never given up while it is in the window (its retries back off to
    // `SALE_SKIP_TRIES` × `SALE_SKIP_RETRY_SECS`); a rate limit is no try; it is remembered as read only when both marketplaces answered
    if p.can_read(io) {
        if let Some(i) = sc.skipped.as_ref().and_then(|s| s.iter().position(|(_, at, _)| *at <= now)) {
            let (blk, _, tries) = sc.skipped.as_ref().expect("some")[i];
            p.reads = p.reads.saturating_sub(1);
            match io.eth_sale_moves_whole(c.collection, blk, blk).await {
                Ok((v, true)) => {
                    notes.push(format!("block {blk} (skipped: its transfer logs are over the largest class) read through its marketplace sale logs"));
                    found.extend(v);
                    sc.skipped.as_mut().expect("some").remove(i);
                    remember_big(&mut sc, blk);
                }
                Ok((v, false)) => {
                    found.extend(v);
                    let t = tries + 1;
                    sc.skipped.as_mut().expect("some")[i] = (blk, skip_again(now, t), t);
                    notes.push(format!("block {blk} (skipped): one marketplace's sale logs read, the other's not yet: read again later"));
                }
                Err(e) => {
                    p.busy |= scan_error(&e) == ScanErr::Busy;
                    let t = if scan_error(&e) == ScanErr::Busy { tries } else { tries + 1 };
                    sc.skipped.as_mut().expect("some")[i] = (blk, if t == tries { now } else { skip_again(now, t) }, t);
                    notes.push(format!("block {blk} is still unreadable ({e}): read again later"));
                }
            }
        }
    }
    if let Some(s) = sc.skipped.as_ref().filter(|s| !s.is_empty()) {
        notes.push(format!("{} block(s) over the largest response class not read yet ({}): their sales are not counted until they are", s.len(),
            s.iter().map(|(b, ..)| b.to_string()).collect::<Vec<_>>().join(", ")));
    }
    // the transactions to judge: each once, none judged already, in the window
    let mut cache = st.sale_cache.take().unwrap_or_default();
    let mut judged = sc.judged.take().unwrap_or_default();
    let prefix = |h: &[u8; 32]| u64::from_be_bytes(h[..8].try_into().expect("8"));
    let done: std::collections::BTreeSet<u64> = judged.iter().map(|(p, _)| *p).collect();
    let cached: std::collections::BTreeSet<[u8; 32]> = cache.iter().filter(|y| y.collection == c.collection).map(|y| y.hash).collect();
    let mut seen: std::collections::BTreeSet<[u8; 32]> = std::collections::BTreeSet::new();
    let mut todo: Vec<en::Move> = Vec::new();
    let mut later: Vec<en::Move> = Vec::new();
    for x in pend.into_iter().chain(found) {
        if x.block < bottom || cached.contains(&x.hash) || done.contains(&prefix(&x.hash)) || !seen.insert(x.hash) {
            continue;
        }
        // round 4 (B4-I-3): above this pin's final blocks (the pin went back): kept for a later pass, never dropped
        if x.block > top {
            later.push(x);
            continue;
        }
        todo.push(x);
    }
    let mut todo = sale_order(todo);
    if todo.len() > en::SALE_TODO_MAX {
        // the last in the order go (repeats of a token or a sender, the newest first); their blocks are read again later
        let dropped = todo.split_off(en::SALE_TODO_MAX);
        let lo = dropped.iter().map(|m| m.block).min().unwrap_or(bottom);
        let hi = dropped.iter().map(|m| m.block).max().unwrap_or(bottom);
        sc.rescan = Some(sc.rescan.map_or((lo, hi), |(a, b)| (a.min(lo), b.max(hi))));
        notes.push(format!("{} transaction(s) over the queue's {} dropped: blocks {lo}..{hi} are read again once it has room", dropped.len(),
            en::SALE_TODO_MAX));
    }
    let (mut reads, mut unread, mut keep, mut again) = (0usize, 0usize, Vec::new(), Vec::new());
    // review round 6 (B6-L-1): a receipt that keeps failing (a provider that answers `null` for it, the providers disagreeing) is
    // retried at once `SALE_RECEIPT_FAST_TRIES` times, then only when due, and never with more than `SALE_RECEIPT_RETRY_READS` of the
    // pass's reads: it can no longer hold every newer sale behind it for the window
    let mut rfail = sc.rfail.take().unwrap_or_default();
    // review round 7 (B7-L-2): a receipt answered as a rate limit backs off exponentially (capped) and is read only with the retries'
    // share: busy answers for old receipts can no longer hold every newer sale behind them
    let mut rbusy = sc.rbusy.take().unwrap_or_default();
    let mut retries = 0usize;
    for m in todo {
        let pre = prefix(&m.hash);
        let prior = rfail.iter().position(|x| x.0 == m.hash);
        let bprior = rbusy.iter().position(|x| x.0 == m.hash);
        let hash = m.hash;
        if prior.is_some_and(|i| rfail[i].2 > now) || bprior.is_some_and(|i| rbusy[i].2 > now) {
            keep.push(m);
            continue;
        }
        let retry = prior.is_some() || bprior.is_some();
        // review round 5 (B5-L-1): the receipts' own bound, checked before each read against its worst case
        if reads >= receipts_per_pass || !p.can_receipt(io) || (retry && retries >= en::SALE_RECEIPT_RETRY_READS) {
            keep.push(m);
            continue;
        }
        reads += 1;
        if retry {
            retries += 1;
        }
        let mut busy = false;
        let failed = match io.eth_sale_receipt(m.hash).await {
            Ok(Some(r)) if r.block <= top => {
                let sales = en::sales_in(&r, &c.collection);
                if sales.is_empty() {
                    judged.push((pre, r.block));
                } else {
                    cache.push(CachedSales::of(c.collection, m.hash, r.block, &sales));
                }
                false
            }
            // not seen at a final block at this pin (a lagging provider): again, after the others
            Ok(Some(_)) => {
                again.push(m);
                false
            }
            Err(e) if e.contains(crate::evmread::SIZE_REJECT) => {
                // B2-L-2: over the largest response class; it is final, so it is kept as no sale and never paid for again
                unread += 1;
                judged.push((pre, m.block));
                false
            }
            // review round 6: a rate limit is no failure of the receipt's; review round 7 (B7-L-2): but it is counted and backed off
            Err(e) if scan_error(&e) == ScanErr::Busy => {
                again.push(m);
                busy = true;
                p.busy = true;
                false
            }
            // no receipt for a transaction whose transfer is final (a provider that no longer serves it), or a failed read
            Ok(None) | Err(_) => {
                unread += 1;
                again.push(m);
                true
            }
        };
        match (failed, prior) {
            (true, Some(i)) => {
                let n = rfail[i].1.saturating_add(1);
                rfail[i] = (hash, n, receipt_again(now, n));
            }
            (true, None) => rfail.push((hash, 1, receipt_again(now, 1))),
            (false, Some(i)) if !again.last().is_some_and(|x| x.hash == hash) => {
                rfail.remove(i);
            }
            _ => {}
        }
        match (busy, bprior) {
            (true, Some(i)) => {
                let n = rbusy[i].1.saturating_add(1);
                rbusy[i] = (hash, n, busy_again(now, n));
            }
            (true, None) => rbusy.push((hash, 1, busy_again(now, 1))),
            (false, Some(i)) => {
                rbusy.remove(i);
            }
            (false, None) => {}
        }
    }
    keep.extend(again);
    keep.extend(later);
    // the failed receipts' records, kept for what is still queued
    let queued: std::collections::BTreeSet<[u8; 32]> = keep.iter().map(|m| m.hash).collect();
    rfail.retain(|x| queued.contains(&x.0));
    rbusy.retain(|x| queued.contains(&x.0));
    let slow = rfail.iter().filter(|x| x.1 >= en::SALE_RECEIPT_FAST_TRIES).count();
    if slow > 0 {
        notes.push(format!("{slow} receipt(s) unreadable after {} tries (a provider that no longer serves them, or providers that disagree): \
            retried every {} h at most, behind newer transactions (B6-L-1)", en::SALE_RECEIPT_FAST_TRIES, en::SALE_RECEIPT_RETRY_SECS / 3_600));
    }
    let limited = rbusy.iter().filter(|x| x.1 >= en::SALE_BUSY_PAGE_TRIES).count();
    if limited > 0 {
        notes.push(format!("{limited} receipt(s) answered as a rate limit {} times in a row or more: backed off (at most every {} h), behind \
            newer transactions (B7-L-2)", en::SALE_BUSY_PAGE_TRIES, en::SALE_BUSY_RETRY_MAX_SECS / 3_600));
    }
    sc.rfail = (!rfail.is_empty()).then_some(rfail);
    sc.rbusy = (!rbusy.is_empty()).then_some(rbusy);
    // keep the judgments to the window; a sale is never pushed out of the cache by a transaction without one (B3-M-1)
    judged.retain(|(_, b)| *b >= bottom);
    judged.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    judged.truncate(SALE_JUDGED_PER_COLLECTION);
    sc.judged = Some(judged);
    cache.retain(|x| x.block >= bottom);
    cache.sort_by(|a, b| a.sales.is_empty().cmp(&b.sales.is_empty()).then(b.block.cmp(&a.block)).then(a.hash.cmp(&b.hash)));
    let mut kept: Vec<CachedSales> = Vec::new();
    for x in cache {
        if kept.iter().filter(|y| y.collection == x.collection).count() < SALE_CACHE_PER_COLLECTION {
            kept.push(x);
        }
    }
    st.sale_cache = Some(kept);
    if unread > 0 {
        notes.push(format!("{unread} receipt(s) unread and not counted"));
    }
    if !keep.is_empty() || sc.down > bottom || sc.rescan.is_some() {
        notes.push(format!("{} transaction(s) and blocks {}..{} still to read{}", keep.len(), bottom, sc.down.max(bottom),
            sc.rescan.map(|(a, b)| format!(", blocks {a}..{b} to read again")).unwrap_or_default()));
    }
    // review round 5 (B5-M-1): how far the newest blocks' walk is behind the final head (the monitor pages a long lag)
    if sc.up != 0 && top.saturating_sub(sc.up) > en::SALE_SCAN_CHUNK_BLOCKS {
        notes.push(format!("the scan is {} blocks behind the final head", top - sc.up));
    }
    // review round 6 (B6-M-1 / D6-I-1): what the monitor pages on: the lag, the hole still to read, what the providers could not serve.
    // Review round 7 (B7-L-3): the lag is measured from where the scan was last current, not from the re-anchored forward walk (which
    // a re-anchor keeps within a day of the head whether or not anything is read)
    sc.behind = sc.current.map(|c| top.saturating_sub(c));
    if let Some(b) = sc.behind.filter(|b| *b >= sb.lag_sev2) {
        notes.push(format!("the scan was last current {b} final blocks ago (block {}): no newer sale is counted meanwhile (B7-L-3)",
            sc.current.unwrap_or_default()));
    }
    // review round 7 (B7-L-2): passes in a row that met a rate limit
    sc.busy = p.busy.then(|| sc.busy.map_or((now, 1), |(s, n)| (s, n.saturating_add(1))));
    if let Some((s, n)) = sc.busy.filter(|(_, n)| *n >= en::SALE_BUSY_PAGE_PASSES) {
        notes.push(format!("a provider has answered the scan as a rate limit in {n} passes in a row (since {s}) (B7-L-2)"));
    }
    if let Some((x, y)) = sc.hole {
        notes.push(format!("blocks {x}..{y} passed over by a re-anchor are still to read (newest first)"));
    }
    if let Some((x, y)) = sc.lost {
        notes.push(format!("blocks {x}..{y} are deeper than the providers keep logs: their sales are not counted (the window's sample is short)"));
    }
    // the raw marks follow the deferred ranges they cover
    let bad_now = sc.bad.clone().unwrap_or_default();
    if let Some(r) = sc.raw.as_mut() {
        r.retain(|(x, y)| bad_now.iter().any(|(a, b)| a <= y && x <= b));
    }
    if sc.raw.as_ref().is_some_and(|r| r.is_empty()) {
        sc.raw = None;
    }
    let used = io.logs_bytes().saturating_sub(p.logs_from).saturating_add(io.receipt_bytes().saturating_sub(p.receipts_from));
    sc.spent = Some((day, spent_day.saturating_add(used)));
    sc.pend = Some(keep);
    *st.scan_mut(&c.collection) = sc;
    if notes.is_empty() { String::new() } else { format!("; {}", notes.join("; ")) }
}

/// Round 4 (B4-H-1), review round 5 (B5-L-1): one pass's allowance for one collection's scan: the chunk budget, and the response bytes
/// its log reads and its receipts may reserve (each read started only when its worst case still fits).
struct ScanPass {
    reads: usize,
    logs_from: u64,
    receipts_from: u64,
    logs_cap: u64,
    receipts_cap: u64,
    /// Review round 7 (B7-L-2): a provider answered a read of this pass as a rate limit.
    busy: bool,
}

impl ScanPass {
    fn can_read<I: EthIo>(&self, io: &I) -> bool {
        self.reads > 0 && io.logs_bytes().saturating_sub(self.logs_from).saturating_add(io.logs_read_worst()) <= self.logs_cap
    }
    fn can_receipt<I: EthIo>(&self, io: &I) -> bool {
        io.receipt_bytes().saturating_sub(self.receipts_from).saturating_add(io.receipt_read_worst()) <= self.receipts_cap
    }
}

/// What one read of a walk did.
enum Got {
    /// The blocks `[a, b]` are done for the walk: read, read through the marketplaces' logs, or deferred (`SaleScan.bad`).
    Read(u64, u64, Vec<en::Move>),
    /// Any other failure (a provider, a disagreement): the chunk halves, as before; with what it was (review round 6: a rate limit
    /// backs off, a history the providers do not keep re-anchors or is recorded as lost).
    Failed(u64, u64, String, ScanErr),
}

/// Review round 5 (B5-M-2): what a failed log read says, whoever said it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScanErr {
    /// Too big to answer whole: the IC's own size reject, or a provider's result, range, size or time limit (measured 2026-09-28 on
    /// the canister's keyless pair: publicnode answers 74 MB for 1,000 dense blocks, which the IC refuses; xrpc.cl answers `-32005`
    /// "capacity temporarily busy" for a dense or wide range; others say "query returned more than 10000 results").
    TooBig,
    /// The providers keep no logs that deep (publicnode's keyless endpoint: "Archive requests require a personal token" from about
    /// 10,000 blocks back).
    Deep,
    /// Review round 6 (B6-L-2): a rate limit (xrpc.cl's and Tenderly's `-32005`, an HTTP 429): read again later, never taken as too
    /// big (deferred, split, a block given up) and never counted toward "stuck".
    Busy,
    Other,
}

/// Review round 5 (B5-M-2), round 6 (B6-M-1 / B6-L-2 / B6-I-1): what a failed log read says. The transforms drop a provider's words
/// but keep their CLASS (`pin::error_class`: `"class":"busy" | "deep" | "limit"`), so what reaches the canister is classified here as
/// the provider meant it; a bare code (`-32005` is both a rate limit and a result cap) names nothing and is `Other` (deferred only
/// when it persists). Words reaching here unchanged (a world that keeps them) are classified by the same lists.
pub fn scan_error(e: &str) -> ScanErr {
    if crate::evmread::too_big(e) || e.contains(crate::evmread::SIZE_REJECT) {
        return ScanErr::TooBig;
    }
    // review round 7 (B7-I-1): the IC's reject of an outcall whose replicas got different answers (some a rate limit, some a result)
    // is a transient disagreement, read again (bounded: B7-L-2), never a range too big (its details say `timeout:`)
    if e.contains(IC_DIVERGENCE) {
        return ScanErr::Busy;
    }
    let class = if e.contains(r#""class":"busy""#) {
        Some("busy")
    } else if e.contains(r#""class":"deep""#) {
        Some("deep")
    } else if e.contains(r#""class":"limit""#) {
        Some("limit")
    } else {
        crate::pin::error_class(e)
    };
    match class {
        Some("busy") => ScanErr::Busy,
        Some("deep") => ScanErr::Deep,
        Some("limit") => ScanErr::TooBig,
        _ => ScanErr::Other,
    }
}

/// Review round 7 (B7-I-1): the words the IC's reject starts with when an outcall's replicas returned different transformed answers.
pub const IC_DIVERGENCE: &str = "No consensus could be reached. Replicas had different responses.";

/// Review round 6 (B6-M-1): the forward walk moves to the newest final blocks; the blocks it passes over join the hole, read newest
/// first by a walk of their own. Returns the note.
fn reanchor(sc: &mut SaleScan, top: u64, bottom: u64, why: &str) -> String {
    let lo = top.saturating_sub(sc.chunk.saturating_sub(1)).max(bottom).max(sc.up + 1);
    let (x, y) = (sc.up + 1, lo.saturating_sub(1));
    if y >= x {
        sc.hole = Some(sc.hole.map_or((x, y), |(a, b)| (a.min(x), b.max(y))));
    }
    sc.up = lo.saturating_sub(1);
    sc.stuck = None;
    format!("the newest blocks' walk was {why} (at block {x}): re-anchored at block {lo}; blocks {x}..{y} are read newest first (B6-M-1)")
}

/// Review round 6 (B6-L-2): whether the deferred range `[x, y]` lies in one whose marketplace logs were not read whole.
fn is_raw(sc: &SaleScan, x: u64, y: u64) -> bool {
    sc.raw.iter().flatten().any(|(a, b)| *a <= x && y <= *b)
}

/// Review round 6 (B5-L-2): when a skipped block is tried again: backing off with its tries, never given up.
fn skip_again(now: u64, tries: u32) -> u64 {
    now.saturating_add(en::SALE_SKIP_RETRY_SECS.saturating_mul(tries.clamp(1, en::SALE_SKIP_TRIES) as u64))
}

/// Review round 7 (B7-L-2): when a receipt answered as a rate limit `n` times in a row is read again: `SALE_BUSY_RETRY_SECS` doubled
/// each time, at most `SALE_BUSY_RETRY_MAX_SECS`.
fn busy_again(now: u64, n: u32) -> u64 {
    let d = en::SALE_BUSY_RETRY_SECS.saturating_mul(1u64 << n.saturating_sub(1).min(20)).min(en::SALE_BUSY_RETRY_MAX_SECS);
    now.saturating_add(d)
}

/// Review round 6 (B6-L-1): when a failed receipt is read again.
fn receipt_again(now: u64, fails: u32) -> u64 {
    if fails < en::SALE_RECEIPT_FAST_TRIES {
        now
    } else {
        now.saturating_add(en::SALE_RECEIPT_RETRY_SECS.saturating_mul((fails + 1 - en::SALE_RECEIPT_FAST_TRIES).min(4) as u64))
    }
}

fn remember_big(sc: &mut SaleScan, blk: u64) {
    let d = sc.done_big.get_or_insert_with(Vec::new);
    if !d.contains(&blk) {
        d.push(blk);
        if d.len() > en::SALE_DONE_BIG_MAX {
            let lo = *d.iter().min().expect("some");
            d.retain(|b| *b != lo);
        }
    }
}

/// Defers `[a, b]` (`raw`: its marketplace logs were not read whole). Past `SALE_BAD_MAX` the oldest range goes (its sales leave the
/// window first); review round 6 (B6-L-2): the oldest whose marketplace sales WERE read goes first, a raw one only when every kept
/// range is raw and there are twice the bound.
fn defer(sc: &mut SaleScan, a: u64, b: u64, raw: bool) {
    if raw && !is_raw(sc, a, b) {
        sc.raw.get_or_insert_with(Vec::new).push((a, b));
    }
    let raws = sc.raw.clone().unwrap_or_default();
    let v = sc.bad.get_or_insert_with(Vec::new);
    v.push((a, b));
    let is_r = |x: &(u64, u64)| raws.iter().any(|(p, q)| *p <= x.0 && x.1 <= *q);
    if v.len() > en::SALE_BAD_MAX {
        let pick = (0..v.len()).filter(|&i| !is_r(&v[i])).min_by_key(|&i| v[i].1)
            .or_else(|| (v.len() > 2 * en::SALE_BAD_MAX).then(|| (0..v.len()).min_by_key(|&i| v[i].1)).flatten());
        if let Some(i) = pick {
            v.remove(i);
        }
    }
}

/// One oversized block, through its marketplaces' own sale logs: read (and remembered, never asked again), or skipped (tried again
/// later, never every pass). The walk goes past it either way.
#[allow(clippy::too_many_arguments)]
async fn block_sales<I: EthIo>(io: &mut I, c: &Col, sc: &mut SaleScan, p: &mut ScanPass, blk: u64, now: u64, notes: &mut Vec<String>) -> Got {
    if !p.can_read(io) {
        let s = sc.skipped.get_or_insert_with(Vec::new);
        if !s.iter().any(|(x, ..)| *x == blk) {
            s.push((blk, now, 0));
        }
        notes.push(format!("block {blk} is over the largest response class: its marketplace sale logs are read next pass (this pass's budget)"));
        return Got::Read(blk, blk, vec![]);
    }
    p.reads = p.reads.saturating_sub(1);
    match io.eth_sale_moves_whole(c.collection, blk, blk).await {
        Ok((v, true)) => {
            notes.push(format!("block {blk}'s transfer logs are over the largest response class: its {} marketplace sale(s) read from the \
                marketplaces' own logs", v.len()));
            remember_big(sc, blk);
            Got::Read(blk, blk, v)
        }
        // review round 6 (B6-L-2): one marketplace answered, the other did not (a rate limit, its size): what was read counts, and the
        // block stays skipped for the other, never remembered as read
        Ok((v, false)) => {
            let s = sc.skipped.get_or_insert_with(Vec::new);
            if !s.iter().any(|(x, ..)| *x == blk) {
                s.push((blk, now, 0));
            }
            notes.push(format!("block {blk}'s transfer logs are over the largest response class: {} marketplace sale(s) read, the other \
                marketplace's logs not yet: skipped for now, read again later", v.len()));
            Got::Read(blk, blk, v)
        }
        Err(e2) => {
            let busy = scan_error(&e2) == ScanErr::Busy;
            p.busy |= busy;
            let s = sc.skipped.get_or_insert_with(Vec::new);
            if !s.iter().any(|(x, ..)| *x == blk) {
                s.push(if busy { (blk, now, 0) } else { (blk, now.saturating_add(en::SALE_SKIP_RETRY_SECS), 1) });
            }
            notes.push(format!("block {blk}'s transfer logs are over the largest response class and its marketplace logs were not read \
                ({e2}): skipped for now, read again later (B4-H-1)"));
            Got::Read(blk, blk, vec![])
        }
    }
}

/// One `eth_getLogs` of the scan inside `[lo, hi]`, walking up (from `lo`) or down (from `hi`). Review round 5 (B5-M-1 / B5-M-2 /
/// B4-L-2): ranges already deferred, oversized blocks already read and skipped blocks are stepped over (never asked again here); a
/// range too big to read, however it says so, is read through the marketplaces' sale logs of the whole range in one read (the route
/// B4-H-1 took for one block) and deferred for its other sales, or deferred outright when that fails too or the pass cannot afford it:
/// either way the walk goes past it. One oversized block alone goes through its own marketplace logs. Nothing here is a walk's
/// failure but a read that failed.
#[allow(clippy::too_many_arguments)]
async fn scan_read<I: EthIo>(io: &mut I, c: &Col, sc: &mut SaleScan, p: &mut ScanPass, lo: u64, hi: u64, up: bool, now: u64,
    notes: &mut Vec<String>) -> Got {
    let (mut a, mut b) = (lo, hi);
    let known: Vec<(u64, u64)> = sc.bad.iter().flatten().copied()
        .chain(sc.done_big.iter().flatten().map(|x| (*x, *x)))
        .chain(sc.skipped.iter().flatten().map(|(x, ..)| (*x, *x)))
        .collect();
    for (x, y) in known {
        if up {
            if x <= a && a <= y {
                return Got::Read(a, y.min(hi), vec![]);
            }
            if a < x && x <= b {
                b = x - 1;
            }
        } else {
            if x <= b && b <= y {
                return Got::Read(x.max(lo), b, vec![]);
            }
            if a <= y && y < b {
                a = y + 1;
            }
        }
    }
    p.reads = p.reads.saturating_sub(1);
    let e = match io.eth_transfer_moves(c.collection, a, b).await {
        Ok(v) => {
            if sc.stuck.is_some_and(|(s, _)| s == a) {
                sc.stuck = None;
            }
            return Got::Read(a, b, v);
        }
        Err(e) => e,
    };
    let class = scan_error(&e);
    p.busy |= class == ScanErr::Busy;
    let stuck = class == ScanErr::Other && {
        let n = match sc.stuck {
            Some((s, n)) if s == a => n + 1,
            _ => 1,
        };
        sc.stuck = Some((a, n));
        n >= en::SALE_SCAN_STUCK_FAILS
    };
    if class != ScanErr::TooBig && !stuck {
        return Got::Failed(a, b, e, class);
    }
    sc.stuck = None;
    if a == b {
        return block_sales(io, c, sc, p, a, now, notes).await;
    }
    let why = if stuck { format!("fail {} times in a row ({e})", en::SALE_SCAN_STUCK_FAILS) } else { format!("are too big to read ({e})") };
    if !p.can_read(io) {
        defer(sc, a, b, true);
        notes.push(format!("the transfer logs of blocks {a}..{b} {why}: deferred (this pass's budget), the walk goes on"));
        return Got::Read(a, b, vec![]);
    }
    p.reads = p.reads.saturating_sub(1);
    match io.eth_sale_moves_whole(c.collection, a, b).await {
        Ok((v, whole)) => {
            // review round 6 (B6-L-2): read whole or not, the range is refined later through its Transfer logs; a range whose
            // marketplace logs were not read whole is kept first when the deferred ranges are over their bound
            defer(sc, a, b, !whole);
            notes.push(format!("the transfer logs of blocks {a}..{b} {why}: their {} marketplace sale(s) read from the marketplaces' own logs{}, \
                the rest deferred", v.len(), if whole { "" } else { " (one marketplace's only: the other's read later)" }));
            Got::Read(a, b, v)
        }
        Err(e2) => {
            defer(sc, a, b, true);
            notes.push(format!("the transfer logs of blocks {a}..{b} {why} and their marketplace logs were not read ({e2}): deferred, the walk \
                goes on"));
            Got::Read(a, b, vec![])
        }
    }
}

/// Round 3 (B3-M-1): the order the scan reads receipts in. A transaction's rank is one more than the number of older pending
/// transactions that moved the same token, or came from the same sender: the oldest move of each token by each sender comes first,
/// oldest first, then the second of each, and so on. A flood (one token ping-ponged, many tokens from one wallet, a chain of fresh
/// wallets passing one token on) ranks behind every real sale of a token and seller that moved once in the queue; to be read before
/// a real sale it needs a fresh token AND a fresh sender for every transaction, and even then only its transactions OLDER than that
/// sale are: a sale waits at most for what was queued before it, never for what comes after.
pub fn sale_order(mut v: Vec<en::Move>) -> Vec<en::Move> {
    v.sort_by(|a, b| a.block.cmp(&b.block).then(a.hash.cmp(&b.hash)));
    let mut tokens: std::collections::BTreeMap<Word, u32> = std::collections::BTreeMap::new();
    let mut froms: std::collections::BTreeMap<[u8; 20], u32> = std::collections::BTreeMap::new();
    let mut ranked: Vec<(u32, en::Move)> = Vec::with_capacity(v.len());
    for m in v {
        let t = tokens.entry(m.token).or_insert(0);
        *t += 1;
        let f = froms.entry(m.from).or_insert(0);
        *f += 1;
        ranked.push(((*t).max(*f), m));
    }
    ranked.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.block.cmp(&b.1.block)).then(a.1.hash.cmp(&b.1.hash)));
    ranked.into_iter().map(|(_, m)| m).collect()
}

/// One evidence pass (§4.2), at one Ethereum pin. Listings: verified until `min_listings + EXTRA_LISTINGS` distinct offerers are
/// (B-M-7). Sales: the canister's own (`sale_scan`), the window's final blocks, never a sale of ours. `(post, pin, note)`.
async fn reading_of<I: EthIo>(io: &mut I, st: &mut EthState, c: &Col, cfg: &EthNftConfig, listings: &[String], accepted: u128,
    scan: bool, receipts: usize) -> Result<(FloorPost, u64, String), String> {
    let pin = io.eth_pin().await?;
    let p = cfg.floor_params();
    let want = p.min_listings as usize + EXTRA_LISTINGS;
    let mut seen: Vec<Seen> = Vec::new();
    let mut refused = Vec::new();
    for l in listings.iter().take(en::MAX_LISTINGS) {
        let mut offerers: Vec<[u8; 20]> = seen.iter().map(|x| x.offerer).collect();
        offerers.sort();
        offerers.dedup();
        if offerers.len() >= want {
            break;
        }
        match io.eth_listing(l, c.collection).await {
            Ok(x) => seen.push(x),
            Err(e) => refused.push(e),
        }
    }
    let note = if scan { sale_scan(io, st, c, pin.number, cfg.logs_max(), receipts).await } else if !io.cycles_ok() {
        format!("; the canister's own sale scan did not run: its cycles are under {EVIDENCE_CYCLES_FLOOR} (top it up); the sales it already \
            read still count")
    } else {
        "; the sale scan is not due this pass (slow cadence): the sales already read count".to_string()
    };
    let us = io.our_evm().await.ok();
    // audit C3-02: the window and the final depth in this side's chain's blocks
    let sb = en::sale_blocks(cfg.chain_id);
    let top = pin.number.saturating_sub(sb.final_depth);
    let bottom = top.saturating_sub(sb.window);
    let mut window: Vec<en::Sale> = st.sale_cache.iter().flatten().filter(|x| x.collection == c.collection && x.block >= bottom && x.block <= top)
        .flat_map(|x| x.sales()).filter(|x| Some(x.seller) != us && Some(x.buyer) != us).collect();
    window.sort_by(|a, b| b.block.cmp(&a.block).then(a.token.cmp(&b.token)));
    // audit C3-01: a token-bound collection's prices at the share that is the NFT alone (what its accounts hold is never valued: a
    // seller can drain one before the fill), so the post, the module's value and the buy bound cut from it are the NFT's
    en::nft_only(&mut seen, &mut window, c.nft_bps);
    let post = en::floor_from_on(&seen, &window, &p, accepted, &sb)
        .map_err(|e| format!("no post: {e} ({} listings refused: {}{note})", refused.len(), refused.join("; ")))?;
    Ok((post, pin.number, note))
}

/// The module's `(accepted, target, postedAt)`.
async fn module_floor<I: EthIo>(io: &mut I, module: [u8; 20]) -> Result<(u128, u128, u64), String> {
    // docs/v9 §2.3: on the Solana home the module's lamports are read as wei at this tick's rate (the readings are Ethereum's)
    if io.sol_home().is_some() {
        return home_books::module_floor(io, module).await;
    }
    let a = crate::abi::uint_at(&io.home_read(module, &en::ACCEPTED).await?, 0, 0, 128)?;
    let f = io.home_read(module, &crate::floorsink::FLOOR).await?;
    Ok((a, crate::abi::uint_at(&f, 0, 0, 128)?, crate::abi::uint_at(&f, 0, 1, 64)? as u64))
}

/// The module as the item rules see it, and its accepted floor (`floor()`'s first word, read with it).
async fn module_view<I: EthIo>(io: &mut I, c: &Col) -> Result<(SinkView, u128), String> {
    let mut r = Vec::new();
    for call in crate::floorsink::reads() {
        r.push(io.home_read(c.module, &call).await?);
    }
    let clock = io.home_clock().await?;
    let accepted = crate::abi::uint_at(&r[3], 0, 0, 128)?;
    Ok((crate::floorsink::view(&r, clock, Some(c.max_purchase_wei))?, accepted))
}

/// Round 3 (E3-M-1): the Ethereum pin a balance read of the flow may be believed at: at or above our last mined transaction's block
/// (every spend `eth_out` counts is in the balance there) and at or above where the watch `w` was taken. A pin that moved back (a
/// provider's head fell back across a bucket) is a wait: at it a counted spend would read as unspent, and a real arrival could hide.
async fn eth_floor<I: EthIo>(io: &mut I, st: &EthState, w: Option<&Watch>) -> Result<u64, String> {
    let pin = io.eth_pin().await?.number;
    let floor = st.last_mined.map_or(0, |(_, b)| b).max(w.and_then(|w| w.pin).unwrap_or(0));
    if pin < floor {
        return Err(format!("the Ethereum pin {pin} is below block {floor} (our last mined transaction, or where the watch was taken): no \
            balance is read there (E3-M-1)"));
    }
    Ok(pin)
}

/// The advance gate (audit 2026-09-27-nft round 2, A2-M-1): an advance of `item_cap` reaches Ethereum, after Relay's estimated fee
/// and the 4663 gas keep, as at least the module's accepted floor plus `ADVANCE_MARGIN_BPS` and the Ethereum gas reserve. A pooled
/// module whose blended backing fell behind a rising floor (or any module short of its floor) does not advance: its ETH stays on
/// 4663, where it is burnable, and the monitor's pool page says so after a day.
pub fn advance_covers(item_cap: u128, accepted: u128) -> Result<(), String> {
    if accepted == 0 {
        return Err("the module has no accepted floor".into());
    }
    let arrive = item_cap.saturating_sub(GAS_KEEP_WEI).saturating_sub(item_cap.saturating_mul(ADVANCE_FEE_EST_BPS) / 10_000);
    let need = accepted.saturating_mul(10_000 + ADVANCE_MARGIN_BPS) / 10_000 + en::ETH_GAS_RESERVE_WEI;
    if arrive < need {
        return Err(format!("an advance of {item_cap} wei would reach Ethereum as about {arrive} wei, under the module's floor {accepted} plus \
            {} % and the gas reserve ({need}): the pool waits on 4663 (A2-M-1)", ADVANCE_MARGIN_BPS / 100));
    }
    Ok(())
}

/// A2-L-1, part 1 (the operator, 2026-09-27): what a buy's advance is. The buy on Ethereum pays at most `ethnft::buy_cap`'s floor
/// bound: the floor (our fresh reading, never above the module's accepted floor; the accepted floor alone without a fresh reading)
/// plus the premium, within `max_purchase_wei`. The advance is that plus the Ethereum gas reserve, grossed up for the Relay fee the
/// judge accepts (`relay_max_fee_bps`, not the 0.5 % estimate: whatever fee a judged quote charges, what arrives still pays the
/// buy's most; review round 3 C3-L-1), plus the 4663 gas keep, and never more than the item cap. What the item cap has beyond it
/// never leaves the module: `advance` releases the unit's backing and books the rest as that module's revenue, which tops up its
/// later conversions. What Relay does not charge of the fee allowance is left over on Ethereum, and swept back (`sweep_pass`).
/// Returns the amount and the floor that bounds the buy.
pub fn advance_bound(item_cap: u128, accepted: u128, own: Option<(u128, u64)>, now: u64, premium_bps: u16, max_purchase_wei: u128,
    fee_bps: u16) -> (u128, u128) {
    let floor = match own {
        Some((r, at)) if r > 0 && now <= at.saturating_add(en::READING_TTL_SECS) => r.min(accepted),
        _ => accepted,
    };
    let buy = (floor.saturating_mul(10_000 + premium_bps as u128) / 10_000).min(max_purchase_wei);
    let need = buy.saturating_add(en::ETH_GAS_RESERVE_WEI);
    let fee = (fee_bps as u128).min(9_999);
    let gross = need.saturating_mul(10_000).div_ceil(10_000 - fee).saturating_add(GAS_KEEP_WEI);
    (gross.min(item_cap), floor)
}

// ---- buys ------------------------------------------------------------------------------------------------------------------------------

async fn buy_pass<I: EthIo>(io: &mut I, st: &mut EthState, cfg: &EthNftConfig, cols: &[Col]) -> Vec<String> {
    let mut lines = Vec::new();
    // every open buy (at most one per collection) takes its own step: one waiting never holds another up
    let open: Vec<EthBuy> = st.buys.iter().filter(|b| !b.finished()).cloned().collect();
    for b in open {
        let r = step_buy(io, st, cfg, &b).await;
        let text = match &r {
            Ok(Some(s)) => s.clone(),
            Ok(None) => continue,
            Err(e) => format!("waits: {e}"),
        };
        if let Some(x) = st.buy_mut(b.id) {
            x.note = text.clone();
        }
        lines.push(format!("eth buy {}: {text}", b.id));
    }
    // audit 2026-09-27-nft (the 2026-09-26 L-2 sweep, B-M-4): a custody reservation `eth:{id}` whose buy is gone, finished, or
    // stuck with nothing on the way (no watch, no token of ours) is settled, so a buy that can never land frees its room
    let stale: Vec<String> = io.with_caps(|_, bk| bk.entries.iter().filter(|e| e.in_flight && e.id.starts_with("eth:")).map(|e| e.id.clone())
        .collect::<Vec<_>>()).into_iter().filter(|id| {
            let n: Option<u64> = id.strip_prefix("eth:").and_then(|x| x.parse().ok());
            match n.and_then(|n| st.buy(n)) {
                None => true,
                // review round 6 (C6-L-3): a written-off forward order still open (neither credited nor released) keeps its room: a late
                // fill credited to it mints the item it paid for, which the custody book must count
                Some(b) => (b.finished() || matches!(b.phase, BPhase::Stuck { .. }) && b.last.is_none())
                    && b.late.as_ref().is_none_or(|l| l.back || l.credited.is_some() || l.released_at.is_some()),
            }
        }).collect();
    for id in stale {
        let _ = io.with_caps(|_, bk| custodycap::settle(bk, &id, None));
        lines.push(format!("eth: the custody reservation {id} of a buy that can no longer land is released"));
    }
    // a new buy: only while no buy holds the Relay leg; round robin over the collections with no open buy whose module wants one. A
    // buy on its way back (round 2) is next on the leg: no new buy starts before its deposit is signed, for its quote window at most
    // (round 3, C3-M-1: one collection's way back that Relay cannot quote no longer stops every collection's buys)
    let now = io.now();
    if let Some(l) = st.late_hold().filter(|_| !st.buys.iter().any(|b| !b.finished() && b.in_relay())) {
        lines.push(format!("eth: no new Relay order: buy {l}'s written-off order is neither credited nor released (its payout may still \
            land, and would be read as a new order's: C5-M-1); eth_credit_payout credits it, eth_late_release releases it"));
        return lines;
    }
    if st.relay_busy() || io.frog_relay_busy() || io.frog_relay_waiting() || st.buys.iter().any(|b| matches!(b.phase, BPhase::BackFunded { since, .. } if since == 0
        || now.saturating_sub(since) < BACK_QUOTE_SECS)) {
        return lines;
    }
    // audit A5-02: on the Solana home every flow is paid into the one address this buy's refund watch will read: no new buy opens a
    // watch while a reimbursement of the Bitcoin side (a frog's `item_arrived`, which pays us) is owed (the symmetric of C-06)
    if io.sol_home().is_some() && io.frog_reimbursement_owed() {
        lines.push("eth: no new Relay order: a reimbursement of the Bitcoin side (paying our Solana address) is owed first (A5-02)".into());
        return lines;
    }
    let n = cols.len() as u64;
    for k in 0..n {
        let c = cols[((st.rr + k) % n) as usize];
        if st.open_buy_for(&c.collection).is_some() {
            continue;
        }
        // docs/v7/dormant-collections.md §2.3–§4: buys only for a live collection (never while DORMANT or WAKING), and an ACTIVE one's
        // module is read on the slow cadence unless hot
        let kind = kind_of(st, cfg, &c.collection);
        if !kind.live() {
            continue;
        }
        if kind == Kind::Active {
            let hot = is_hot(st, &c.collection, now);
            if st.dorm.as_ref().and_then(|d| d.life(&c.collection)).is_some_and(|l| !dm::view_due(l, now, hot)) {
                continue;
            }
            if let Some(l) = st.dorm.as_mut().and_then(|d| d.life_mut(&c.collection)) {
                l.view_at = now;
            }
        }
        // A3-L-1 / C3-L-2: a collection whose advance just came back waits a day before the next one
        if let Some(t) = st.buys.iter().filter(|x| x.collection == c.collection && !x.is_sweep() && matches!(x.phase, BPhase::Returned { .. }))
            .filter_map(|x| x.ended_at).max().filter(|t| now < t.saturating_add(RETURN_COOLDOWN_SECS)) {
            lines.push(format!("eth: 0x{}: no buy until {}: its last advance came back at {t} (a day between round trips)",
                hex::encode(c.collection), t + RETURN_COOLDOWN_SECS));
            continue;
        }
        // docs/v9 §2.3: on the Solana home the module is in lamports and its accepted floor read in wei at this tick's rate
        let got = if io.sol_home().is_some() {
            home_books::gate(io, &c).await.map(|g| (g.view, g.accepted_wei, Some(g.rate)))
        } else {
            module_view(io, &c).await.map(|(v, a)| (v, a, None))
        };
        let (v, accepted, rate) = match got {
            Ok(v) => v,
            Err(e) => {
                lines.push(format!("eth: module 0x{} not read: {e}", hex::encode(c.module)));
                continue;
            }
        };
        // docs/v7/dormant-collections.md §4: the module's net demand wants an item: the collection is hot (its module read every tick,
        // its evidence and scan every hand-in) until the buy is under way, whatever holds the start now (a price near its expiry)
        if v.want > 0 {
            if let Some(l) = st.dorm.as_mut().and_then(|d| d.life_mut(&c.collection)) {
                l.hot_until = now.saturating_add(dm::HOT_SECS);
            }
        }
        if itemsink::may_start(&v, 0).is_err() {
            continue;
        }
        // dormant-review DR-M-1: under the dormancy, no buy while the module's accepted floor has not caught up with our reading
        if cfg.dormancy.is_some() {
            match st.reading(&c.collection).map(|r| r.post.price) {
                Some(p) if dm::converged(p, accepted) => {}
                got => {
                    lines.push(format!("eth: 0x{}: no buy yet: the module's accepted floor {accepted} has not caught up with our reading ({})",
                        hex::encode(c.collection), got.map_or("none".into(), |p| p.to_string())));
                    continue;
                }
            }
        }
        // A2-M-1: never a buy whose advance could not pay the floor on Ethereum
        let covers = match rate {
            Some(r) => crate::solhome::advance_covers(v.item_cap.unwrap_or(0), accepted, r),
            None => advance_covers(v.item_cap.unwrap_or(0), accepted),
        };
        if let Err(e) = covers {
            lines.push(format!("eth: 0x{}: no buy: {e}", hex::encode(c.collection)));
            continue;
        }
        // H2 at the item's value (audit 2026-09-27-nft, B-H-2): a buy that could never reserve its room is not started, so it never
        // holds the one Relay leg while it waits. The value is what it would advance (A2-L-1: the buy's bound), in ETH
        let own = st.reading(&c.collection).map(|r| (r.post.price, r.at));
        let amount = match rate {
            Some(r) => crate::rates::lamports_to_wei(crate::solhome::advance_bound(v.item_cap.unwrap_or(0), accepted, own, io.now(), c.max_premium_bps,
                c.max_purchase_wei, cfg.relay_max_fee_bps, r).0, r),
            None => advance_bound(v.item_cap.unwrap_or(0), accepted, own, io.now(), c.max_premium_bps, c.max_purchase_wei, cfg.relay_max_fee_bps).0,
        };
        if let Err(e) = io.with_caps(|p, b| custodycap::check_acquire(p, b, &key_of(&c, io_chain(cfg)), amount)) {
            lines.push(format!("eth: 0x{}: no buy: custody cap: {e:?}", hex::encode(c.collection)));
            continue;
        }
        let id = st.next_buy;
        st.next_buy += 1;
        st.rr = st.rr.wrapping_add(k + 1);
        st.buys.push(EthBuy { id, collection: c.collection, module: c.module, wrapper: c.wrapper, advanced: 0, attempts: 0,
            phase: BPhase::NeedAdvance, orders: vec![], deposits: vec![], ask: None, last: None, note: String::new(), sweep: None,
            arrived_at: None, back_at: None, ended_at: None, late: None });
        lines.push(format!("eth buy {id}: wanted for 0x{} (the module wants {} item(s)): the advance is next", hex::encode(c.collection), v.want));
        break;
    }
    lines
}

fn io_chain(cfg: &EthNftConfig) -> u64 {
    cfg.chain_id
}

async fn ask_for<I: EthIo>(io: &mut I, wei: u128) -> Result<ConvAsk, String> {
    // docs/v9 §2.8: on the Solana home the forward leg is SOL → ETH: lamports deposited by the dedicated Relay key (audit A5-03: the
    // only key handed to Relay's depository), refunded to our Solana address
    if io.sol_home().is_some() {
        let sol = crate::soltx::b58_encode(&io.sol_me().await?);
        let relay = crate::soltx::b58_encode(&io.sol_relay().await?);
        let me = h(&io.our_evm().await?);
        return Ok(ConvAsk { amount_wei: wei, user: relay, recipient: me, refund_to: sol, origin_chain_id: io.home_chain(),
            destination_chain_id: io.eth_chain(), origin_currency: crate::relaysol::SOL_NATIVE.into(),
            destination_currency: crate::relayconv::EVM_NATIVE.into() });
    }
    let me = h(&io.our_evm().await?);
    Ok(ConvAsk { amount_wei: wei, user: me.clone(), recipient: me.clone(), refund_to: me, origin_chain_id: io.home_chain(),
        destination_chain_id: io.eth_chain(), origin_currency: crate::relayconv::EVM_NATIVE.into(),
        destination_currency: crate::relayconv::EVM_NATIVE.into() })
}

async fn step_buy<I: EthIo>(io: &mut I, st: &mut EthState, cfg: &EthNftConfig, b: &EthBuy) -> Result<Option<String>, String> {
    match b.phase.clone() {
        BPhase::Bridging { order_id, value, min_out, deadline, since } => {
            let w = b.last.clone().ok_or("nothing is watched")?;
            // the arrival: our Ethereum balance, plus what our own Ethereum transactions spent since, up by the minimum (E3-M-1: at a
            // pin at or above every spend counted and the watch's own)
            eth_floor(io, st, Some(&w)).await?;
            let bal = io.eth_balance().await?;
            let got = bal.saturating_add(st.eth_out.saturating_sub(w.eth_out_before)).saturating_sub(w.eth_before);
            if got >= min_out {
                // docs/v9 §2.3: on the Solana home the order sent lamports: what arrived is bounded by their worth in ETH
                let cap = wei_cap_of(io, value).await?;
                return Ok(Some(arrive(st, b.id, got, cap, bal, io.now(), &format!("ARRIVED (order 0x{})", hex::encode(order_id)))?));
            }
            // the refund on the home chain (round 4, C4-H-1: read only at a 4663 pin at or above every one already read, so our own
            // deposit's spend is in the balance; C4-M-1: what comes back is the rise over the watch's start, never the raw balance)
            let home = io.home_balance().await?;
            if crate::relayconv::refunded(home, w.balance_after, value) {
                // docs/v9 §2.8 / Q5 §5.2: on Solana the balance rule decides; the memo naming the order is a label only
                let label = if io.sol_home().is_some() { home_sol::refund_label(io, st, &order_id).await } else { String::new() };
                let back = home.saturating_sub(w.balance_after).min(value);
                let x = st.buy_mut(b.id).expect("open");
                x.last = None;
                if b.attempts < MAX_ATTEMPTS && back > 0 {
                    x.phase = BPhase::Funded { wei: back, since: io.now() };
                    let ask = ask_for(io, back).await?;
                    st.buy_mut(b.id).expect("open").ask = Some(ask);
                    return Ok(Some(format!("REFUNDED: {back} {} back{label}; attempt {} of {MAX_ATTEMPTS} next", unit_of(io), b.attempts + 1)));
                }
                let wei = back.min(b.advanced);
                x.phase = BPhase::Returning { wei };
                return Ok(Some(format!("REFUNDED {} times: {wei} {} goes back to the module{label}", b.attempts, unit_of(io))));
            }
            if io.now() > deadline.saturating_add(DEADLINE_GRACE_SECS) {
                st.buy_mut(b.id).expect("open").phase = BPhase::Stuck { why: format!("order 0x{} is past its deadline with no arrival \
                    of {min_out} wei on Ethereum and no refund", hex::encode(order_id)) };
                return Ok(Some("STUCK: past the order's deadline".into()));
            }
            Ok(Some(format!("bridging for {} s; no arrival or refund yet", io.now().saturating_sub(since))))
        }
        BPhase::Stuck { .. } => {
            // still watched: a late fill or refund moves it on
            let Some(w) = b.last.clone() else { return Ok(Some("STUCK".into())) };
            eth_floor(io, st, Some(&w)).await?;
            let bal = io.eth_balance().await?;
            let got = bal.saturating_add(st.eth_out.saturating_sub(w.eth_out_before)).saturating_sub(w.eth_before);
            // docs/v9 §2.3: the order's value in ETH (lamports at the rate on the Solana home; as is on the EVM home)
            let vcap = wei_cap_of(io, w.value).await?;
            if got >= w.min_out {
                return Ok(Some(arrive(st, b.id, got, vcap, bal, io.now(), "ARRIVED late")?));
            }
            // audit 2026-09-27-nft B-L-7: Relay may refund on Ethereum (the quote names an Ethereum refund, its minimum unbounded):
            // past the deadline, at least the refund share of the deposit at our Ethereum address is this buy's ETH there. It waits
            // for a listing as an arrival would (D4), and no longer holds the one Relay leg for every collection
            if got.saturating_mul(crate::relayconv::BPS) >= vcap.saturating_mul(crate::relayconv::REFUND_SHARE_BPS) {
                return Ok(Some(arrive(st, b.id, got, vcap, bal, io.now(), "REFUNDED ON ETHEREUM (under the minimum out); it waits for a listing")?));
            }
            // round 4 (C4-M-1): Relay refunds after the deadline, so a refund on 4663 lands here: read by this order's own watch (the rise
            // over its start, at a 4663 pin never below one read), never left for another order's watch to read as its own
            let home = io.home_balance().await?;
            if crate::relayconv::refunded(home, w.balance_after, w.value) {
                let wei = home.saturating_sub(w.balance_after).min(w.value).min(b.advanced);
                let x = st.buy_mut(b.id).expect("open");
                x.phase = BPhase::Returning { wei };
                x.last = None;
                // audit A5-I3: the home's chain and unit
                return Ok(Some(if io.sol_home().is_some() {
                    format!("REFUNDED LATE on Solana: {wei} lamports go back to the module")
                } else {
                    format!("REFUNDED LATE on 4663: {wei} wei goes back to the module")
                }));
            }
            Ok(Some("STUCK: still watching for a late fill or refund".into()))
        }
        BPhase::Arrived { wei, since } => {
            // round 2 (A2-M-1 / C2-M-2): an arrived buy that has not filled for ARRIVED_BACK_SECS goes back to its module, through a
            // Relay order Ethereum → 4663 judged like the forward one (its deposit takes the one Relay leg when it is signed). Round 3
            // (E3-L-1): counted from the first arrival (a fill that reverts does not restart it), or BACK_RETRY_SECS after a way back
            // that did not go
            if io.now() < b.back_at.unwrap_or(since.saturating_add(ARRIVED_BACK_SECS)) {
                return Ok(None);
            }
            if st.line.as_ref().is_some_and(|l| matches!(l.kind, EthKind::Buy { buy } | EthKind::CancelFill { buy } if buy == b.id)) {
                return Ok(None);
            }
            // B-L-6: the way back takes the Relay leg; it waits while a Bitcoin-side conversion holds it
            if io.frog_relay_busy() {
                return Ok(None);
            }
            back_now(io, st, b.id, wei).await
        }
        BPhase::BackFunded { wei, since } => {
            // round 3 (C3-M-1): no deposit signed in its quote window (Relay quotes nothing we accept, the runner hands in none, or
            // the Relay leg stayed busy): it waits at Arrived again, earmarked as before (nothing was signed), may take a listing, and
            // tries again BACK_RETRY_SECS later; meanwhile it no longer holds any collection's new buys
            let now = io.now();
            if !b.is_sweep() && since != 0 && now.saturating_sub(since) >= BACK_QUOTE_SECS {
                let x = st.buy_mut(b.id).expect("open");
                x.phase = BPhase::Arrived { wei, since: now };
                x.ask = None;
                x.back_at = Some(now.saturating_add(BACK_RETRY_SECS));
                st.inbox.quotes.retain(|(q, ..)| *q != b.id);
                return Ok(Some(format!("the way back had no deposit signed within {BACK_QUOTE_SECS} s: its {wei} wei waits at our Ethereum \
                    address for a listing again, and goes back again in {BACK_RETRY_SECS} s")));
            }
            if b.ask.is_none() && !io.frog_relay_busy() {
                return back_now(io, st, b.id, wei).await;
            }
            Ok(None)
        }
        BPhase::BackBridging { order_id, value, min_out, deadline, since } => {
            let w = b.last.clone().ok_or("nothing is watched")?;
            // the arrival on 4663: our home balance up by the order's minimum at least (less our own 4663 gas meanwhile: round 3,
            // C3-L-3, no longer half of it), and never more than the order sent (only this leg moves it: one Relay leg)
            let home = io.home_balance().await?;
            if home >= w.balance_after.saturating_add(min_out.saturating_mul(10_000 - BACK_ARRIVAL_SLACK_BPS) / 10_000) {
                // docs/v9 §2.3: on the Solana home the order sent wei and pays lamports: bounded by its worth in lamports
                let vcap = home_cap_of(io, value).await?;
                let wei = home.saturating_sub(w.balance_after).min(vcap);
                let x = st.buy_mut(b.id).expect("open");
                x.phase = BPhase::Returning { wei };
                x.last = None;
                let on = if io.sol_home().is_some() { "SOLANA" } else { "4663" };
                return Ok(Some(format!("BACK ON {on}: {wei} {} (order 0x{}): returnAdvance to the module is next", unit_of(io), hex::encode(order_id))));
            }
            // a refund on Ethereum (the order could not be filled): the ETH is this buy's again there (E3-M-1: read only at a pin at or
            // above the watch's and every spend counted; lower, the refund is not read this tick)
            let floor = eth_floor(io, st, Some(&w)).await;
            let got = match floor {
                Ok(_) => {
                    let bal = io.eth_balance().await?;
                    Some((bal, bal.saturating_add(st.eth_out.saturating_sub(w.eth_out_before)).saturating_sub(w.eth_before)))
                }
                Err(_) => None,
            };
            if let Some((bal, got)) = got.filter(|(_, g)| g.saturating_mul(crate::relayconv::BPS) >= value.saturating_mul(crate::relayconv::REFUND_SHARE_BPS)) {
                if b.is_sweep() {
                    // A2-L-1: a refunded sweep is never an arrival (nothing is bought with it): it ends, its leftover credited back
                    let back = got.min(value);
                    let now = io.now();
                    let x = st.buy_mut(b.id).expect("open");
                    x.phase = BPhase::Returned { wei: 0 };
                    x.last = None;
                    let c = x.collection;
                    st.credit_leftover(&c, back);
                    st.sweep_after = Some(now.saturating_add(SWEEP_COOLDOWN_SECS));
                    return Ok(Some(format!("the sweep was REFUNDED on Ethereum: {back} wei is 0x{}'s leftover again", hex::encode(c))));
                }
                return Ok(Some(arrive(st, b.id, got, value, bal, io.now(), "the way back was REFUNDED on Ethereum; it waits again")?));
            }
            let now = io.now();
            let late = deadline.saturating_add(DEADLINE_GRACE_SECS);
            if now > late.saturating_add(BACK_LATE_SECS) {
                // round 4 (C4-M-1): it stays watched, holding the Relay leg, until its payout is read here or the operator writes it off
                // (eth_give_back, once Relay says nothing more will come): a payout later than the watch was read as ANOTHER order's
                // refund or arrival (round 3 let it go at this point), or was nobody's. Written off, its watch is kept (LateOrder) and a
                // payout that comes after all is credited to this order by its transaction (eth_credit_payout)
                return Ok(Some(format!("LATE: the way back (order 0x{}) is {} s past its deadline with no arrival of {min_out} wei on 4663 and \
                    no refund on Ethereum{}; still watched (it holds the Relay leg): once Relay says no payout will come, eth_give_back({}) \
                    writes it off", hex::encode(order_id), now - deadline, floor.err().map(|e| format!("; {e}")).unwrap_or_default(), b.id)));
            }
            if now > late {
                return Ok(Some(format!("going back for {} s, past the order's deadline: no arrival on 4663 and no refund on Ethereum yet{} (still \
                    watched until {}, then Stuck)", now.saturating_sub(since), floor.err().map(|e| format!("; {e}")).unwrap_or_default(),
                    late.saturating_add(BACK_LATE_SECS))));
            }
            Ok(Some(format!("going back to 4663 for {} s; no arrival or refund yet", io.now().saturating_sub(since))))
        }
        BPhase::Bought { arrived, token, total, tx, block } => {
            let fin = io.eth_finalized().await?;
            if block > fin {
                return Ok(Some(format!("bought in block {block}; finalized is {fin}: waiting for finality")));
            }
            // still ours at the pin, past finality
            let owner = crate::abi::addr_at(&io.eth_read(b.collection, &en::call_word(crate::seaport::OWNER_OF, token)).await?, 0, 0)?;
            if owner != io.our_evm().await? {
                st.buy_mut(b.id).expect("open").phase = BPhase::Stuck { why: format!("token {} was bought in {} but its owner is 0x{} now",
                    h(&token), h(&tx), hex::encode(owner)) };
                return Ok(Some("STUCK: the bought token is not ours after finality".into()));
            }
            st.buy_mut(b.id).expect("open").phase = BPhase::Final { arrived, token, total, tx };
            let _ = cfg;
            Ok(Some(format!("final (block {block} ≤ {fin}): the mint to the module is next")))
        }
        _ => Ok(None),
    }
}

/// An arrival on Ethereum for buy `id` (audit 2026-09-27-nft round 2, C2-M-1 / C2-L-1): what is earmarked is at most what the order
/// sent (`value`: Relay never pays out more than it took in) and at most what our balance holds beside every other buy's earmark, so
/// the earmarks never exceed the balance; a shortfall there is a wait, never an arrival. The Relay watch ends with the arrival: a
/// later `Stuck` can never read it again.
fn arrive(st: &mut EthState, id: u64, got: u128, value: u128, bal: u128, now: u64, what: &str) -> Result<String, String> {
    let room = bal.saturating_sub(st.earmarked_except(id));
    let wei = got.min(value).min(room);
    if wei < got.min(value) {
        return Err(format!("{got} wei would be this buy's, but our Ethereum balance {bal} holds only {room} beside the other buys' earmarks: \
            nothing is earmarked on ETH that is not there (C2-M-1); read again next tick"));
    }
    let x = st.buy_mut(id).expect("open");
    x.phase = BPhase::Arrived { wei, since: now };
    x.last = None;
    // round 3 (E3-L-1): the first arrival starts the deposit hold and D4's 48 h; a way back refunded on Ethereum tries again later
    if x.arrived_at.is_none() {
        x.arrived_at = Some(now);
        x.back_at = Some(now.saturating_add(ARRIVED_BACK_SECS));
    } else {
        x.back_at = Some(now.saturating_add(BACK_RETRY_SECS));
    }
    Ok(format!("{what}: {wei} wei at our Ethereum address: the runner is asked for a listing"))
}

/// An arrived buy starts its way back (round 2, A2-M-1): what arrived, less the gas reserve kept for the deposit itself, is asked for
/// in a Relay quote Ethereum → 4663, to our own address.
async fn back_now<I: EthIo>(io: &mut I, st: &mut EthState, id: u64, wei: u128) -> Result<Option<String>, String> {
    let Some(amount) = wei.checked_sub(en::ETH_GAS_RESERVE_WEI).filter(|a| *a > 0) else {
        // round 3 (C3-I-3): what is left does not pay the way back's own gas: it can buy nothing and go nowhere. The dust stays in the
        // gas pot as the collection's leftover, and the advance is returned as the loss it is (returnAdvance of 0), so the buy ends
        let x = st.buy_mut(id).expect("open");
        let c = x.collection;
        if x.is_sweep() {
            x.phase = BPhase::Returned { wei: 0 };
        } else {
            x.phase = BPhase::Returning { wei: 0 };
        }
        x.ask = None;
        st.credit_leftover(&c, wei);
        return Ok(Some(format!("only {wei} wei is left, under the {} wei the way back's gas needs: kept as leftover; the advance is \
            returned as a loss", en::ETH_GAS_RESERVE_WEI)));
    };
    let me = h(&io.our_evm().await?);
    // docs/v9 §2.8: on the Solana home the way back is ETH → SOL, paid to our Solana address
    let (recipient, dest_currency) = if io.sol_home().is_some() {
        (crate::soltx::b58_encode(&io.sol_me().await?), crate::relaysol::SOL_NATIVE)
    } else {
        (me.clone(), crate::relayconv::EVM_NATIVE)
    };
    let ask = ConvAsk { amount_wei: amount, user: me.clone(), recipient, refund_to: me, origin_chain_id: io.eth_chain(),
        destination_chain_id: io.home_chain(), origin_currency: crate::relayconv::EVM_NATIVE.into(), destination_currency: dest_currency.into() };
    let now = io.now();
    let x = st.buy_mut(id).expect("open");
    let since = match x.phase { BPhase::BackFunded { since, .. } if since != 0 => since, _ => now };
    x.phase = BPhase::BackFunded { wei, since };
    x.ask = Some(ask);
    Ok(Some(format!("GOING BACK: {wei} wei has waited at our Ethereum address for a listing; {amount} wei goes back to 4663 through Relay \
        (the runner is asked for a quote), then to the module")))
}

// ---- deliveries ---------------------------------------------------------------------------------------------------------------------

async fn delivery_pass<I: EthIo>(io: &mut I, st: &mut EthState, cfg: &EthNftConfig) -> Vec<String> {
    let mut lines = Vec::new();
    let open: Vec<(usize, Delivery)> = st.deliveries.iter().cloned().enumerate().filter(|(_, d)| !d.finished()).collect();
    for (i, d) in open {
        let r = async {
            match d.phase.clone() {
                DPhase::Moved { tx, block } => {
                    let fin = io.eth_finalized().await?;
                    if block > fin {
                        return Ok(Some(format!("moved in block {block}; finalized {fin}: waiting")));
                    }
                    // round 3 (D3-H-1): judged by our delivery's OWN receipt at the pin (past finality), never by who owns the token now:
                    // a redeemer who sold the NFT into a bid or moved it to another wallet was still delivered to
                    let us = io.our_evm().await?;
                    match io.eth_sale_receipt(tx).await? {
                        // round 4 (A4-L-2): a transfer a reorg dropped for good is never noted sent (the note waits for this receipt), so
                        // the wrapper's late window refunds its ticket; once it has, the delivery is gone
                        None if !still_resolved(io, d.wrapper, d.ticket).await? => {
                            st.deliveries[i].phase = DPhase::Gone { why: format!("our delivery {} is not seen at the pin after finality and the \
                                ticket left Resolved (refunded after the late window)", h(&tx)) };
                            Ok(Some("the transfer is gone (a reorg) and the ticket was refunded: nothing more to do".into()))
                        }
                        None => Err(format!("final by block {block}, but our delivery {} is not seen at the pin now: read again next tick (never \
                            noted sent: the wrapper's late window still refunds it if it never comes back)", h(&tx))),
                        Some(r) if r.block != block => {
                            st.deliveries[i].phase = DPhase::Moved { tx, block: r.block };
                            Ok(Some(format!("our delivery is now in block {} (a reorg): waiting for its finality", r.block)))
                        }
                        Some(r) if !r.success => {
                            st.deliveries[i].phase = DPhase::MarkAfter { why: format!("the transfer reverted in block {} (read at finality)", r.block),
                                block: r.block };
                            Ok(Some("the transfer REVERTED (read at finality): markUndeliverable once final".into()))
                        }
                        Some(r) if !en::delivered_in(&r, &d.collection, &us, &d.destination, &d.token) => Err(format!("final, but the receipt of {} \
                            shows no Transfer of token {} from us to the destination: nothing is recorded; look", h(&tx), h(&d.token))),
                        Some(_) => {
                            st.deliveries[i].phase = DPhase::Record { tx };
                            Ok(Some("final, and its receipt shows the token delivered to the destination: recordDelivery is next".into()))
                        }
                    }
                }
                DPhase::MarkAfter { why, block } => {
                    let fin = io.eth_finalized().await?;
                    if block > fin {
                        return Ok(Some(format!("{why}; block {block}, finalized {fin}: the mark waits for finality")));
                    }
                    st.deliveries[i].phase = DPhase::Mark { why };
                    Ok(Some("final: markUndeliverable is next".into()))
                }
                DPhase::Waiting | DPhase::Sent { .. } | DPhase::Cancelling { .. } => {
                    // the ticket may have left Resolved by another path (a refund after the late window)
                    let t = read_ticket(io, d.wrapper, d.ticket).await?;
                    if t.stage != Stage::Resolved && matches!(d.phase, DPhase::Waiting) {
                        st.deliveries[i].phase = DPhase::Gone { why: format!("the ticket is {:?}", t.stage) };
                        return Ok(Some(format!("the ticket is {:?}: nothing to deliver", t.stage)));
                    }
                    Ok(None)
                }
                _ => Ok(None),
            }
        }.await;
        let _ = cfg;
        match r {
            Ok(Some(s)) => lines.push(format!("eth delivery {}#{}: {s}", h(&d.wrapper), d.ticket)),
            Ok(None) => {}
            Err(e) => {
                st.deliveries[i].note = e.clone();
                lines.push(format!("eth delivery {}#{}: waits: {e}", h(&d.wrapper), d.ticket));
            }
        }
    }
    lines
}

/// Deliveries that were never signed and never will be (a relist or empty destination, or inside the deadline's last day) are
/// marked undeliverable, whatever holds the Ethereum line (audit 2026-09-27-nft, C-M-2): the mark signs nothing on Ethereum, so an
/// unmined fill or return can no longer keep a holder from the 72 h refund.
async fn mark_pass<I: EthIo>(io: &mut I, st: &mut EthState) -> Vec<String> {
    let mut lines = Vec::new();
    let now = io.now();
    let Ok(us) = io.our_evm().await else { return lines };
    for i in 0..st.deliveries.len() {
        let d = st.deliveries[i].clone();
        if d.phase != DPhase::Waiting || !(d.destination == us || d.destination == [0; 20] || now.saturating_add(en::DELIVERY_STOP_SECS) >= d.deliver_by) {
            continue;
        }
        match read_ticket(io, d.wrapper, d.ticket).await {
            Ok(t) if t.stage == Stage::Resolved => {
                let why = if d.destination == us || d.destination == [0; 20] { "the ticket's destination is nobody or us (a relist ticket): never delivered".to_string() }
                    else { format!("within a day of its deadline {} and never signed", d.deliver_by) };
                st.deliveries[i].phase = DPhase::Mark { why: format!("never sent: {why}") };
                lines.push(format!("eth delivery {}#{}: never sent ({why}): markUndeliverable is next", h(&d.wrapper), d.ticket));
            }
            Ok(_) => {}
            Err(e) => lines.push(format!("eth delivery {}#{}: waits: the ticket is not read: {e}", h(&d.wrapper), d.ticket)),
        }
    }
    lines
}

// ---- deposits -------------------------------------------------------------------------------------------------------------------------

/// Claims (a holder's deposit, or anything the runner saw arrive): judged into deposits to mint and strays to return. Then the
/// valid deposits past finality: re-read, still ours, room in custody → `Mint`; no room → returned.
async fn deposit_pass<I: EthIo>(io: &mut I, st: &mut EthState, cfg: &EthNftConfig, cols: &[Col]) -> Vec<String> {
    let mut lines = Vec::new();
    let us = match io.our_evm().await {
        Ok(u) => u,
        Err(e) => return vec![format!("eth deposits: {e}")],
    };
    let now = io.now();
    let home = io.home_chain();
    // 1. claims, a few per tick; one that cannot be read yet waits in the inbox, then is dropped unrecorded
    let claims: Vec<([u8; 32], u64)> = std::mem::take(&mut st.inbox.deposits);
    let mut keep = Vec::new();
    let mut read = 0;
    for (tx, at) in claims {
        if st.deposits.iter().any(|d| d.tx == tx) {
            continue;
        }
        if read >= CLAIMS_PER_TICK {
            keep.push((tx, at));
            continue;
        }
        read += 1;
        let got = io.eth_deposit_tx(tx).await;
        let (from, to, input, receipt) = match got {
            Ok(Some(x)) => x,
            Ok(None) | Err(_) => {
                let why = match got { Err(e) => e, _ => "not mined at the pin yet".into() };
                if now.saturating_sub(at) < CLAIM_RETRY_SECS {
                    lines.push(format!("eth claim {}: {why}: tried again next tick", h(&tx)));
                    keep.push((tx, at));
                } else {
                    lines.push(format!("eth claim {}: {why} for {} s: dropped, not recorded (it may be claimed again)", h(&tx), now - at));
                }
                continue;
            }
        };
        if from == us {
            // our own transaction (a Seaport fill, a delivery, a return): what it brought in is a purchase's, never a claim's
            lines.push(format!("eth claim {}: refused: our own transaction", h(&tx)));
            st.deposits.push(DepositRec { tx, collection: [0; 20], token: [0; 32], recipient: [0; 20], from: [0; 20], block: receipt.block,
                phase: XPhase::Refused { why: "our own transaction".into() }, refused_since: None });
            continue;
        }
        let mut recs = Vec::new();
        for c in cols {
            let t = en::DepositTarget { home_chain: home, wrapper: c.wrapper, us };
            match en::judge_claim(&from, &to, &input, &receipt, &c.collection, &t) {
                en::Claim::Deposit { token, recipient, from } if cols.iter().any(|x| x.wrapper == recipient || x.module == recipient) => {
                    // audit 2026-09-27-nft A-I-3: a unit minted to a wrapper or a Floor Module of ours would be locked (and a module's
                    // wX balance is what settles an advance): such a claim is a stray, back to its sender
                    let why = "the claim names a wrapper or Floor Module of ours as its recipient: the unit would be locked".to_string();
                    lines.push(format!("eth claim {}: token {} is a stray ({why}): it goes back to 0x{} once final", h(&tx), h(&token), hex::encode(from)));
                    recs.push(DepositRec { tx, collection: c.collection, token, recipient: [0; 20], from, block: receipt.block,
                        phase: XPhase::Return { why }, refused_since: None });
                }
                en::Claim::Deposit { token, recipient, from } => {
                    lines.push(format!("eth deposit {}: token {} for 0x{}: waiting for finality", h(&tx), h(&token), hex::encode(recipient)));
                    recs.push(DepositRec { tx, collection: c.collection, token, recipient, from, block: receipt.block, phase: XPhase::Seen,
                        refused_since: None });
                }
                en::Claim::Strays { transfers, why } => {
                    for (k, (from, token)) in transfers.into_iter().enumerate() {
                        let phase = if k < en::MAX_TRANSFERS_PER_CLAIM { XPhase::Return { why: why.clone() } }
                            else { XPhase::Held { why: format!("more than {} transfers to us in one transaction: the operator looks", en::MAX_TRANSFERS_PER_CLAIM) } };
                        lines.push(format!("eth claim {}: token {} of 0x{} is a stray ({why}): {}", h(&tx), h(&token), hex::encode(c.collection),
                            if matches!(phase, XPhase::Return { .. }) { format!("it goes back to 0x{} once final", hex::encode(from)) } else { "held".into() }));
                        recs.push(DepositRec { tx, collection: c.collection, token, recipient: [0; 20], from, block: receipt.block, phase,
                            refused_since: None });
                    }
                }
                en::Claim::Nothing => {}
            }
        }
        if recs.is_empty() {
            lines.push(format!("eth claim {}: refused: no NFT of a configured collection reached us in it", h(&tx)));
            recs.push(DepositRec { tx, collection: [0; 20], token: [0; 32], recipient: [0; 20], from: [0; 20], block: receipt.block,
                phase: XPhase::Refused { why: "no NFT of a configured collection reached us in this transaction".into() }, refused_since: None });
        }
        st.deposits.extend(recs);
    }
    st.inbox.deposits = keep;
    // 2. valid deposits past finality
    if !st.deposits.iter().any(|d| d.phase == XPhase::Seen) {
        return lines;
    }
    let fin = match io.eth_finalized().await {
        Ok(f) => f,
        Err(e) => {
            lines.push(format!("eth deposits: finality not read: {e}"));
            return lines;
        }
    };
    for i in 0..st.deposits.len() {
        if st.deposits[i].phase != XPhase::Seen {
            continue;
        }
        let d = st.deposits[i].clone();
        let r = async {
            if d.block > fin {
                return Ok(format!("block {}, finalized {fin}: waiting", d.block));
            }
            // the receipt again, at a pin past finality: the block it is final in
            match io.eth_sale_receipt(d.tx).await? {
                None => return Ok("the receipt is not seen at the pin now: waiting".into()),
                Some(r) if r.block != d.block || !r.success => {
                    st.deposits[i].block = r.block;
                    return Ok(format!("it moved to block {} (a reorg before finality): waiting for that one", r.block));
                }
                Some(_) => {}
            }
            let owner = crate::abi::addr_at(&io.eth_read(d.collection, &en::call_word(crate::seaport::OWNER_OF, d.token)).await?, 0, 0)?;
            if owner != us {
                st.deposits[i].phase = XPhase::Refused { why: format!("not ours at the pin after finality (owner 0x{})", hex::encode(owner)) };
                return Ok("refused: not ours after finality".into());
            }
            let Some(c) = cols.iter().find(|c| c.collection == d.collection) else {
                st.deposits[i].phase = XPhase::Held { why: "its collection is no longer configured".into() };
                return Ok("held: the collection is no longer configured".into());
            };
            let item_of = crate::abi::uint_at(&io.home_read(c.wrapper, &en::call_word(en::ITEM_OF, d.token)).await?, 0, 0, 128)?;
            if item_of != 0 {
                st.deposits[i].phase = XPhase::Held { why: "the wrapper already holds this token".into() };
                return Ok("held: the wrapper already holds this token".into());
            }
            // H2: room for one more item in custody; none → the holder gets the NFT back rather than waiting unminted
            // J7 / review round 3 A3-L-4: a deposit counts against the value cap at the module's accepted floor, the value its unit is
            // minted against (it counted 0 wei before, so deposits were bounded by item counts alone)
            let mark = module_floor(io, c.module).await?.0;
            if let Err(e) = io.with_caps(|p, b| custodycap::check_acquire(p, b, &key_of(c, cfg.chain_id), mark)) {
                st.deposits[i].phase = XPhase::Return { why: format!("no custody room ({e:?})") };
                return Ok(format!("no custody room ({e:?}): it goes back to 0x{}", hex::encode(d.from)));
            }
            st.deposits[i].phase = XPhase::Mint;
            Ok::<_, String>("final: mintDeposit is next".into())
        }.await;
        lines.push(format!("eth deposit {}: {}", h(&d.tx), r.unwrap_or_else(|e| format!("waits: {e}"))));
    }
    lines
}

/// One stray back to its sender, if one is due: final, judged, within the day's returns, and never under the gas kept for
/// deliveries. `Ok(None)` when none is due.
async fn return_act<I: EthIo>(io: &mut I, st: &mut EthState, cfg: &EthNftConfig, cols: &[Col], us: [u8; 20]) -> Result<Option<String>, String> {
    if !st.deposits.iter().any(|d| matches!(d.phase, XPhase::Return { .. })) {
        return Ok(None);
    }
    let fin = io.eth_finalized().await?;
    // the oldest stray whose block is final
    let Some(i) = st.deposits.iter().position(|d| matches!(d.phase, XPhase::Return { .. }) && d.block <= fin) else {
        return Err(format!("a stray's return waits for Ethereum finality (finalized {fin})"));
    };
    let d = st.deposits[i].clone();
    let Some(c) = cols.iter().find(|c| c.collection == d.collection).copied() else {
        st.deposits[i].phase = XPhase::Held { why: "its collection is no longer configured: the canister touches configured collections only".into() };
        return Ok(Some(format!("eth claim {}: held (its collection is no longer configured)", h(&d.tx))));
    };
    let owner = crate::abi::addr_at(&io.eth_read(d.collection, &en::call_word(crate::seaport::OWNER_OF, d.token)).await?, 0, 0)?;
    let item_of = crate::abi::uint_at(&io.home_read(c.wrapper, &en::call_word(en::ITEM_OF, d.token)).await?, 0, 0, 128)?;
    let facts = en::ReturnFacts { owner, item_of, busy: st.token_busy(&d.collection, &d.token, &d.tx), last_out: st.last_out(&d.collection, &d.token) };
    if let Err(e) = en::judge_return(&d.from, d.block, &facts, &us) {
        st.deposits[i].phase = if owner != us { XPhase::Refused { why: format!("not returned: {e}") } } else { XPhase::Held { why: format!("not returned: {e}") } };
        return Ok(Some(format!("eth claim {}: token {} is not returned: {e}", h(&d.tx), h(&d.token))));
    }
    let key = format!("return:eth:{}:{}", h(&d.tx), h(&d.token));
    let col = key_of(&c, cfg.chain_id);
    io.outflow_check(&key, &col, Flow::Return, 1)?;
    let plan = match io.eth_plan_transfer(d.collection, d.token, d.from).await {
        Ok(p) => {
            st.deposits[i].refused_since = None;
            p
        }
        // audit 2026-09-27-nft C-M-1: held only once every provider has agreed it reverts, for `SIM_REFUSAL_SECS` over several ticks;
        // a read that fails or disagrees is a wait, never a decision
        Err(e) if e.contains("refused (simulation)") => {
            let now = io.now();
            let since = *st.deposits[i].refused_since.get_or_insert(now);
            if now.saturating_sub(since) < SIM_REFUSAL_SECS {
                return Err(format!("return of {}: its simulation reverts on every provider since {since}; held only if it still does \
                    {SIM_REFUSAL_SECS} s on: {e}", h(&d.token)));
            }
            st.deposits[i].phase = XPhase::Held { why: format!("the return cannot succeed (it reverts on every provider since {since}): {e}") };
            return Ok(Some(format!("eth claim {}: the return of {} cannot succeed ({e}): held", h(&d.tx), h(&d.token))));
        }
        Err(e) => return Err(format!("return of {}: {e}", h(&d.token))),
    };
    // the gas pot is for deliveries first: an arrived buy's ETH and a keep stay untouched
    let bal = io.eth_balance().await?;
    let arrived = st.earmarked_except(u64::MAX);
    let need = arrived.saturating_add(plan.max_fee.saturating_mul(plan.gas as u128)).saturating_add(RETURN_GAS_KEEP_WEI);
    if bal < need {
        return Err(format!("return of {}: our Ethereum balance {bal} is under {need} (an arrived buy's ETH, the return's gas and the \
            {RETURN_GAS_KEEP_WEI} wei kept for deliveries): it waits", h(&d.token)));
    }
    let hash = sign_eth(io, st, EthKind::Return { tx: d.tx, token: d.token }, &plan).await?;
    io.outflow_record(&key, &col, Flow::Return, 1);
    st.deposits[i].phase = XPhase::Returning { tx: hash };
    Ok(Some(format!("eth claim {}: transferFrom of {} back to 0x{} signed, saved and sent ({}, nonce {})", h(&d.tx), h(&d.token),
        hex::encode(d.from), h(&hash), plan.nonce)))
}

// ---- the Ethereum line -----------------------------------------------------------------------------------------------------------------

async fn sign_eth<I: EthIo>(io: &mut I, st: &mut EthState, kind: EthKind, p: &EthPlan) -> Result<[u8; 32], String> {
    // C-I-5: the pause is read again at the signature (a guardian's pause may have landed during this tick's awaits)
    if io.paused() {
        return Err("paused: nothing new is signed".into());
    }
    // B-M-4: never below a nonce of ours already seen mined, whatever the pin said
    let nonce = p.nonce.max(st.nonce_floor());
    let tx = Tx1559 { chain_id: io.eth_chain(), nonce, max_priority_fee: p.priority_fee, max_fee: p.max_fee, gas: p.gas, to: p.to,
        value: p.value, data: p.data.clone() };
    let raw = io.eth_sign(&tx).await?;
    let hash = crate::evmtx::keccak(&raw);
    if st.line.is_some() {
        return Err("refused (in flight): the Ethereum line was taken while signing".into());
    }
    // saved BEFORE anything is sent
    let now = io.now();
    st.line = Some(EthTx { kind, nonce, hash, raw: raw.clone(), value: p.value, max_fee: p.max_fee, priority_fee: p.priority_fee,
        signed_at: now, to: p.to, data: p.data.clone(), gas: p.gas, earlier: vec![], rivals: vec![], first_signed_at: Some(now) });
    io.save(st);
    io.eth_broadcast(&raw).await;
    Ok(hash)
}

async fn eth_act<I: EthIo>(io: &mut I, st: &mut EthState, cfg: &EthNftConfig, cols: &[Col]) -> Result<Option<String>, String> {
    let now = io.now();
    let us = io.our_evm().await?;
    // B-M-4: a pin below our last mined transaction's block (a provider's head moved back) signs nothing
    if let Some((_, b)) = st.last_mined {
        let pin = io.eth_pin().await?;
        if pin.number < b {
            return Err(format!("the Ethereum pin {} is below the block {b} of our last mined transaction: waiting", pin.number));
        }
    }
    let mut errs = Vec::new();
    // 1. deliveries waiting: each one's wait is its own (B-L-8 / C-L-2: one collection's limit or read never holds up another's)
    // review round 5 (A5-L-2): the collection with the most deliveries waiting first (deadline order within it), a delivery within two
    // days of its deadline before all: the order in which the exit rule's two tests are enough for every item to leave in time
    let waiting = delivery_order(&st.deliveries, now);
    let mut home_final: Option<u64> = None;
    for i in waiting {
        let d = st.deliveries[i].clone();
        let r: Result<Option<String>, String> = async {
            let t = read_ticket(io, d.wrapper, d.ticket).await?;
            let owner = crate::abi::addr_at(&io.eth_read(d.collection, &en::call_word(crate::seaport::OWNER_OF, d.token)).await?, 0, 0)?;
            match en::judge_transfer(&t, &owner, &us, now) {
                Ok(()) => {}
                Err(e) if t.stage == Stage::Resolved && (t.destination == us || now.saturating_add(en::DELIVERY_STOP_SECS) >= t.deliver_by) => {
                    // nothing was ever signed for it, and nothing will be: it can never confirm
                    st.deliveries[i].phase = DPhase::Mark { why: format!("never sent: {e}") };
                    return Ok(Some(format!("eth delivery {}#{}: never sent ({e}): markUndeliverable is next", h(&d.wrapper), d.ticket)));
                }
                Err(e) => return Err(e),
            }
            // H9 on this side (audit 2026-09-27-nft B-L-2 / C-L-6): the ticket's resolution is final on the home chain before its
            // delivery is signed (it resolved at deliverBy − 7 days: strictly before the finalized block's time), so a reorg can
            // no longer un-redeem it or change its item or destination under a signed delivery
            let fin = match home_final {
                Some(x) => x,
                None => {
                    let x = io.home_finalized_time().await?;
                    home_final = Some(x);
                    x
                }
            };
            let resolved_at = t.deliver_by.saturating_sub(DELIVERY_DEADLINE_SECS);
            if resolved_at >= fin {
                return Err(format!("it resolved at {resolved_at}; the home chain is final to {fin}: waiting for its finality"));
            }
            let key = format!("deliver:{}:{}", h(&d.wrapper), d.ticket);
            let col = custodycap::evm_key(cfg.chain_id, &d.collection);
            let plan = match io.eth_plan_transfer(d.collection, d.token, d.destination).await {
                Ok(p) => {
                    st.deliveries[i].refused_since = None;
                    p
                }
                // audit 2026-09-27-nft B-M-2 / C-M-1: only an agreed revert on every provider, repeated over `SIM_REFUSAL_SECS`, is
                // "never sent, never will be"; a transport failure or a disagreement is a wait
                Err(e) if e.contains("refused (simulation)") => {
                    let since = *st.deliveries[i].refused_since.get_or_insert(now);
                    if now.saturating_sub(since) >= SIM_REFUSAL_SECS {
                        st.deliveries[i].phase = DPhase::Mark { why: format!("never sent: the transfer reverts on every provider since {since}: {e}") };
                        return Ok(Some(format!("eth delivery #{}: the transfer cannot succeed ({e}): markUndeliverable is next", d.ticket)));
                    }
                    return Err(format!("its transfer reverts on every provider since {since}; marked only if it still does {SIM_REFUSAL_SECS} s on"));
                }
                Err(e) => return Err(e),
            };
            io.outflow_check(&key, &col, Flow::Item, 1)?;
            let hash = sign_eth(io, st, EthKind::Deliver { wrapper: d.wrapper, ticket: d.ticket }, &plan).await?;
            io.outflow_record(&key, &col, Flow::Item, 1);
            let nonce = st.line.as_ref().map_or(plan.nonce, |l| l.nonce);
            st.deliveries[i].phase = DPhase::Sent { tx: hash, nonce };
            Ok(Some(format!("eth delivery {}#{}: transferFrom of {} to {} signed, saved and sent ({}, nonce {nonce})", h(&d.wrapper), d.ticket,
                h(&d.token), h(&d.destination), h(&hash))))
        }.await;
        match r {
            Ok(Some(l)) => return Ok(Some(l)),
            Ok(None) => {}
            Err(e) => {
                st.deliveries[i].note = e.clone();
                errs.push(format!("delivery {}#{}: {e}", h(&d.wrapper), d.ticket));
                if st.line.is_some() {
                    return Err(errs.join("; "));
                }
            }
        }
    }
    // 2. an arrived buy with a listing handed in, round robin across collections (docs/v7/phase1-nft-pooled.md §4)
    let arrived: Vec<EthBuy> = st.buys.iter().filter(|b| matches!(b.phase, BPhase::Arrived { .. })).cloned().collect();
    let start = if arrived.is_empty() { 0 } else { (st.rr as usize) % arrived.len() };
    for k in 0..arrived.len() {
        let b = arrived[(start + k) % arrived.len()].clone();
        let BPhase::Arrived { wei, .. } = b.phase else { continue };
        let Some(i) = st.inbox.listings.iter().position(|(id, ..)| *id == b.id) else { continue };
        let (_, order, at) = st.inbox.listings.remove(i);
        // one collection's refused listing never holds up another's: its error is noted and the next arrived buy is tried
        let r: Result<String, String> = async {
            if now.saturating_sub(at) > HANDIN_TTL_SECS {
                return Err(format!("buy {}: the listing was handed in {} s ago: stale", b.id, now - at));
            }
            let c = cols.iter().find(|c| c.collection == b.collection).copied().ok_or("the buy's collection is no longer configured")?;
            let own = st.reading(&c.collection).map(|r| (r.post.price, r.at));
            let (accepted, _, _) = module_floor(io, c.module).await?;
            // docs/v7/dormant-collections.md §4.3: one fresh floor and sale read immediately before the fill, checked against the
            // module's accepted floor and the floor the advance was sized on; a stale read keeps the listing for the next pass
            if cfg.dormancy.is_some() {
                if let Err((why, keep)) = fresh_for_fill(st, cfg, &c, b.id, accepted, now) {
                    if let Some(l) = st.dorm.as_mut().and_then(|d| d.life_mut(&c.collection)) {
                        l.hot_until = now.saturating_add(dm::HOT_SECS);
                    }
                    if keep {
                        st.inbox.listings.push((b.id, order.clone(), at));
                    }
                    return Err(format!("buy {}: {why}{}", b.id, if keep { ": the listing is kept for the next pass" } else { ": the listing is dropped" }));
                }
            }
            // strictly its own: at most what arrived for it, and never another open buy's earmarked ETH
            let bal = io.eth_balance().await?;
            let available = bal.saturating_sub(st.earmarked_except(b.id));
            let cap = en::buy_cap(own, accepted, now, c.max_premium_bps, wei.min(available), c.max_purchase_wei)
                .map_err(|e| format!("buy {}: {e}", b.id))?;
            let plan = io.eth_plan_buy(&order, c.collection, cap).await.map_err(|e| format!("buy {}: the listing is refused: {e}", b.id))?;
            if plan.total > cap || plan.value != plan.total {
                return Err(format!("buy {}: the plan pays {} (value {}), over the cap {cap}", b.id, plan.total, plan.value));
            }
            let hash = sign_eth(io, st, EthKind::Buy { buy: b.id }, &plan).await?;
            st.buy_mut(b.id).expect("open").phase = BPhase::Buying { arrived: wei, token: plan.token, total: plan.total, tx: hash };
            st.rr = st.rr.wrapping_add(1);
            Ok(format!("eth buy {}: the fill of {} for {} wei (cap {cap}) signed, saved and sent ({})", b.id, h(&plan.token), plan.total, h(&hash)))
        }.await;
        match r {
            Ok(l) => return Ok(Some(l)),
            Err(e) => {
                if let Some(x) = st.buy_mut(b.id) {
                    x.note = e.clone();
                }
                errs.push(e);
            }
        }
    }
    // 3. an arrived buy going back (round 2, A2-M-1): its Relay deposit Ethereum → 4663, judged like the forward one
    let backs: Vec<EthBuy> = st.buys.iter().filter(|b| matches!(b.phase, BPhase::BackFunded { .. })).cloned().collect();
    for b in backs {
        let BPhase::BackFunded { wei, .. } = b.phase else { continue };
        let Some(i) = st.inbox.quotes.iter().position(|(id, ..)| *id == b.id) else { continue };
        let (_, body, at) = st.inbox.quotes.remove(i);
        let r: Result<String, String> = async {
            // its deposit takes the one Relay leg (the arrival on 4663 is read as a balance rise): only while no other buy holds it
            // (audit A5-02: another way back still waiting for its own quote holds no watch yet: the first one signed goes first)
            if st.buys.iter().any(|x| x.id != b.id && !x.finished() && x.in_relay() && !matches!(x.phase, BPhase::BackFunded { .. })) {
                return Err(format!("buy {}: the way back waits: another buy holds the Relay leg", b.id));
            }
            // audit A5-02 (B-L-6): nor while a Bitcoin-side conversion holds it (its watch reads the same Solana balance the way back's
            // payout lands in)
            if io.frog_relay_busy() {
                return Err(format!("buy {}: the way back waits: a Bitcoin-side conversion holds the Relay leg (B-L-6)", b.id));
            }
            // review round 5 (C5-M-1): nor while a written-off order may still be paid (its payout would be read as this one's)
            if let Some(l) = st.late_hold() {
                return Err(format!("buy {}: the way back waits: buy {l}'s written-off Relay order is not credited or released yet (C5-M-1)", b.id));
            }
            // 2026-10-12: the way back is the leg from this side's chain: Ethereum → 4663 on the EVM home; Ethereum or Robinhood Chain →
            // Solana on the Solana home
            let leg = if io.sol_home().is_some() { re::to_solana(cfg.chain_id) } else { Some(re::FROM_ETHEREUM).filter(|l| l.origin_chain == cfg.chain_id) }
                .ok_or_else(|| format!("buy {}: no Relay leg back from chain {} in this build", b.id, cfg.chain_id))?;
            let amount = b.ask.as_ref().map(|a| a.amount_wei).ok_or("no ask")?;
            let allocator = crate::abi::addr_at(&io.eth_read(leg.depository, &crate::relayconv::ALLOCATOR).await?, 0, 0)?;
            let d = if io.sol_home().is_some() {
                // docs/v9 §2.8: Ethereum (or Robinhood Chain) → Solana, the minimum against our own SOL/ETH rate less the fee bound
                let rate = io.sol_rate().await?;
                let sol = crate::soltx::b58_encode(&io.sol_me().await?);
                let floor = crate::rates::wei_to_lamports(amount, rate).saturating_mul(10_000 - cfg.relay_max_fee_bps as u128) / 10_000;
                let w = re::Want { leg, ours: us, amount_wei: amount, max_fee_bps: cfg.relay_max_fee_bps, allocator, now,
                    handed_in_at: at, used: &st.used_orders };
                re::judge_to_solana(&body, &w, &re::SolDest { recipient: &sol, min_out_floor: floor })
            } else {
                let w = re::Want { leg, ours: us, amount_wei: amount, max_fee_bps: cfg.relay_max_fee_bps, allocator, now,
                    handed_in_at: at, used: &st.used_orders };
                re::judge(&body, &w)
            }.map_err(|e| format!("buy {}: the way back's quote is refused, nothing signed: {e}", b.id))?;
            let plan = io.eth_plan_call(d.to, d.value, &d.data).await.map_err(|e| format!("buy {}: the way back's deposit: {e}", b.id))?;
            // strictly its own: what arrived for it pays the deposit and its gas, never another buy's earmark
            let bal = io.eth_balance().await?;
            let own = wei.min(bal.saturating_sub(st.earmarked_except(b.id)));
            let cost = d.value.saturating_add(plan.max_fee.saturating_mul(plan.gas as u128));
            if plan.value != d.value || cost > own {
                return Err(format!("buy {}: the way back needs {cost} wei (value and gas), over the {own} that is this buy's", b.id));
            }
            let home = io.home_balance().await?;
            // review round 6 (C6-I-1): the 4663 pin is read, never skipped on a failure (the payout judge's "mined after the deposit")
            let home_pin = io.home_pin().await?;
            let pin = io.eth_pin().await?.number;      // at or above our last mined block: eth_act refuses to sign below it (B-M-4)
            // the order is used from here on, sent or not (rule 12)
            st.used_orders.push(d.order_id);
            let hash = sign_eth(io, st, EthKind::BackDeposit { buy: b.id }, &plan).await?;
            let out = st.eth_out;
            let x = st.buy_mut(b.id).expect("open");
            x.orders.push(d.order_id);
            x.phase = BPhase::BackSending { wei, order_id: d.order_id, value: d.value, min_out: d.min_out, deadline: d.deadline, tx: hash };
            // the watch is taken BEFORE the deposit can mine: 4663 as it is now (Relay's arrival raises it), Ethereum with the value
            // still here (the deposit's spend is counted in eth_out when it mines, so a refund reads as a rise)
            x.last = Some(Watch { order_id: d.order_id, value: d.value, min_out: d.min_out, deadline: d.deadline, balance_after: home,
                eth_before: bal, eth_out_before: out, pin: Some(pin), solver: d.solver, request_id: d.request_id, home_pin });
            Ok(format!("eth buy {}: the way back: Relay deposit of {} wei on Ethereum (order 0x{}, at least {} wei on 4663) signed, saved and \
                sent ({})", b.id, d.value, hex::encode(d.order_id), d.min_out, h(&hash)))
        }.await;
        match r {
            Ok(l) => return Ok(Some(l)),
            Err(e) => {
                if let Some(x) = st.buy_mut(b.id) {
                    x.note = e.clone();
                }
                errs.push(e);
                if st.line.is_some() {
                    return Err(errs.join("; "));
                }
            }
        }
    }
    // 4. a stray back to its sender (deposits on): after deliveries and the buy, one at a time
    if cfg.deposits {
        match return_act(io, st, cfg, cols, us).await {
            Ok(Some(l)) => return Ok(Some(l)),
            Ok(None) => {}
            Err(e) => errs.push(e),
        }
    }
    if errs.is_empty() { Ok(None) } else { Err(errs.join("; ")) }
}

/// Review round 5 (A5-L-2): the order waiting deliveries are tried in, one signed a tick (the first the limiter allows). A delivery
/// within `DELIVERY_URGENT_SECS` of its deadline first, nearest deadline first; then the collection with the most deliveries still
/// waiting, its nearest deadline first. Deadline order alone let the collections whose tickets resolved first take a day's global
/// room while the biggest collection's items waited for the margin day (5 Azukis at the launch split, 9 at J7's 61 at 31 a day);
/// backlog first drains every collection within the exit rule's two usable days whenever the rule's per-collection and total tests
/// hold (`outflow::exit_fits`).
pub fn delivery_order(deliveries: &[Delivery], now: u64) -> Vec<usize> {
    let mut waiting: Vec<usize> = (0..deliveries.len()).filter(|&i| deliveries[i].phase == DPhase::Waiting).collect();
    let backlog = |c: &[u8; 20]| waiting.iter().filter(|&&j| deliveries[j].collection == *c).count();
    let mut keyed: Vec<(bool, usize, u64, usize)> = waiting.iter().map(|&i| {
        let d = &deliveries[i];
        let urgent = now.saturating_add(DELIVERY_URGENT_SECS) >= d.deliver_by;
        (!urgent, if urgent { 0 } else { usize::MAX - backlog(&d.collection) }, d.deliver_by, i)
    }).collect();
    keyed.sort();
    waiting = keyed.into_iter().map(|(.., i)| i).collect();
    waiting
}

/// The line's transaction: mined (any version of it, and what that means for its kind), superseded, a delivery pending too long
/// (cancel), a fill pending too long (cancel, B-L-3 / C-M-2), or any transaction unmined for `LINE_BUMP_SECS` (re-priced at the
/// same nonce). While paused only what protects holders is done: the line is read, and a delivery in its last day is cancelled.
async fn resolve_eth<I: EthIo>(io: &mut I, st: &mut EthState, t: EthTx, unsettled: &mut bool) -> Result<String, String> {
    let paused = io.paused();
    // the same bytes again (the answer is not believed); a delivery's only while its ticket is still Resolved and outside its last
    // day (C-L-1 b: a refunded ticket's transfer is never pushed again), and nothing new-going while paused
    let resend = match &t.kind {
        EthKind::Deliver { wrapper, ticket } => !paused && match read_ticket(io, *wrapper, *ticket).await {
            Ok(tk) => tk.stage == Stage::Resolved && io.now().saturating_add(en::DELIVERY_STOP_SECS) < tk.deliver_by,
            Err(_) => false,
        },
        EthKind::Cancel { .. } | EthKind::CancelFill { .. } => true,
        _ => !paused,
    };
    if resend {
        io.eth_broadcast(&t.raw).await;
    }
    for v in t.versions() {
        let got = io.eth_receipt(v).await.inspect_err(|_| *unsettled = true)?;
        if let Some(r) = got {
            st.eth_out = st.eth_out.saturating_add(r.spent(t.value));
            st.line = None;
            st.note_mined(t.nonce, r.block);
            let mut t = t.clone();
            if v != t.hash {
                retag(st, &t.kind, v);
                t.hash = v;
            }
            let out = mined_eth(io, st, &t, r).await;
            if st.line.is_some() {
                // mined, but what it means was not read (mined_eth put it back, its spend taken back out of eth_out): not settled
                *unsettled = true;
            }
            return Ok(out);
        }
    }
    let nonce = io.eth_nonce().await.inspect_err(|_| *unsettled = true)?;
    if nonce > t.nonce {
        // another transaction took the nonce: our cancel against a delivery or a fill, or one of those against its cancel. Until
        // the outcome is read, whatever mined at that nonce may have spent from our balance unseen
        let r = superseded(io, st, &t).await;
        if st.line.is_some() {
            *unsettled = true;
        }
        return r;
    }
    let now = io.now();
    // a delivery still pending a day before its deadline: cancel it at the same nonce (a self-send: also while paused)
    if let EthKind::Deliver { wrapper, ticket } = t.kind.clone() {
        let d = st.deliveries.iter().find(|d| d.wrapper == wrapper && d.ticket == ticket).cloned();
        if let Some(d) = d {
            if now.saturating_add(en::DELIVERY_STOP_SECS) >= d.deliver_by {
                return cancel(io, st, &t, EthKind::Cancel { wrapper, ticket }).await;
            }
        }
    }
    // a fill unmined for FILL_CANCEL_SECS (re-priced all along): cancelled at its nonce; if it mines instead, it is the buy
    if let EthKind::Buy { buy } = t.kind {
        if now.saturating_sub(t.first_signed_at.unwrap_or(t.signed_at)) >= FILL_CANCEL_SECS {
            return cancel(io, st, &t, EthKind::CancelFill { buy }).await;
        }
    }
    if now.saturating_sub(t.signed_at) >= LINE_BUMP_SECS && !paused && t.gas != 0 {
        return reprice(io, st, &t).await;
    }
    Ok(format!("{} (nonce {}) not mined yet{}", h(&t.hash), t.nonce, if resend { "; sent again" } else { "" }))
}

fn bumped(x: u128) -> u128 {
    x.saturating_mul(100 + CANCEL_BUMP_PCT) / 100 + 1
}

/// The records that name the line's transaction by hash follow its newest version.
fn retag(st: &mut EthState, kind: &EthKind, hash: [u8; 32]) {
    match kind.clone() {
        EthKind::Buy { buy } => {
            if let Some(b) = st.buy_mut(buy) {
                if let BPhase::Buying { tx, .. } = &mut b.phase {
                    *tx = hash;
                }
            }
        }
        EthKind::Deliver { wrapper, ticket } => {
            if let Some(d) = st.delivery_mut(&wrapper, ticket) {
                if let DPhase::Sent { tx, .. } = &mut d.phase {
                    *tx = hash;
                }
            }
        }
        EthKind::Cancel { wrapper, ticket } => {
            if let Some(d) = st.delivery_mut(&wrapper, ticket) {
                if let DPhase::Cancelling { tx, .. } = &mut d.phase {
                    *tx = hash;
                }
            }
        }
        EthKind::Return { tx: claim, token } => {
            if let Some(d) = st.deposit_mut(&claim, &token) {
                if let XPhase::Returning { tx } = &mut d.phase {
                    *tx = hash;
                }
            }
        }
        EthKind::CancelFill { .. } => {}
        EthKind::BackDeposit { buy } => {
            if let Some(b) = st.buy_mut(buy) {
                if let BPhase::BackSending { tx, .. } = &mut b.phase {
                    *tx = hash;
                }
            }
        }
    }
}

/// The same transaction (nonce, destination, value, calldata, gas) with its tip and max fee raised by `CANCEL_BUMP_PCT` (and at
/// least the pin's base fee × 4 + tip), never over the chain's cap. Every earlier version stays watched.
async fn reprice<I: EthIo>(io: &mut I, st: &mut EthState, t: &EthTx) -> Result<String, String> {
    let pin = io.eth_pin().await?;
    let prio = bumped(t.priority_fee);
    let max_fee = bumped(t.max_fee).max(pin.base_fee.saturating_mul(crate::looper::FEE_MULTIPLE).saturating_add(prio));
    if max_fee > io.eth_max_fee() {
        return Err(format!("{} (nonce {}) is unmined and a re-price would need {max_fee} wei per gas, over the chain's cap: it waits",
            h(&t.hash), t.nonce));
    }
    let tx = Tx1559 { chain_id: io.eth_chain(), nonce: t.nonce, max_priority_fee: prio, max_fee, gas: t.gas, to: t.to, value: t.value,
        data: t.data.clone() };
    let raw = io.eth_sign(&tx).await?;
    let hash = crate::evmtx::keccak(&raw);
    let mut earlier = t.earlier.clone();
    earlier.push(t.hash);
    st.line = Some(EthTx { hash, raw: raw.clone(), max_fee, priority_fee: prio, signed_at: io.now(), earlier, ..t.clone() });
    retag(st, &t.kind, hash);
    io.save(st);
    io.eth_broadcast(&raw).await;
    Ok(format!("{} (nonce {}) unmined for {} s: re-priced as {} (max fee {max_fee}, tip {prio}); every version is watched", h(&t.hash),
        t.nonce, io.now().saturating_sub(t.signed_at), h(&hash)))
}

/// A 0-value self-send at `t`'s nonce, fees bumped over its newest version: `kind` is `Cancel` (a delivery) or `CancelFill`. Every
/// version of `t` is the cancel's rival; whichever mines settles it.
async fn cancel<I: EthIo>(io: &mut I, st: &mut EthState, t: &EthTx, kind: EthKind) -> Result<String, String> {
    let us = io.our_evm().await?;
    let pin = io.eth_pin().await?;
    let prio = bumped(t.priority_fee);
    let max_fee = bumped(t.max_fee).max(pin.base_fee.saturating_mul(crate::looper::FEE_MULTIPLE).saturating_add(prio));
    if max_fee > io.eth_max_fee() {
        return Err(format!("a cancel needs a max fee of {max_fee}, over the chain's cap: {} waits (a delivery's refund window does not \
            open until we mark it)", h(&t.hash)));
    }
    let tx = Tx1559 { chain_id: io.eth_chain(), nonce: t.nonce, max_priority_fee: prio, max_fee, gas: 21_000, to: us, value: 0, data: vec![] };
    let raw = io.eth_sign(&tx).await?;
    let hash = crate::evmtx::keccak(&raw);
    let now = io.now();
    st.line = Some(EthTx { kind: kind.clone(), nonce: t.nonce, hash, raw: raw.clone(), value: 0, max_fee, priority_fee: prio, signed_at: now,
        to: us, data: vec![], gas: 21_000, earlier: vec![], rivals: t.versions(), first_signed_at: Some(now) });
    let what = match kind {
        EthKind::Cancel { wrapper, ticket } => {
            if let Some(d) = st.delivery_mut(&wrapper, ticket) {
                d.phase = DPhase::Cancelling { of: t.hash, tx: hash, nonce: t.nonce };
            }
            format!("delivery #{ticket} still pending a day before its deadline")
        }
        EthKind::CancelFill { buy } => {
            if let Some(b) = st.buy_mut(buy) {
                b.note = format!("its fill {} was unmined for {FILL_CANCEL_SECS} s: cancelled at nonce {}", h(&t.hash), t.nonce);
            }
            format!("buy {buy}'s fill unmined for {} s", now.saturating_sub(t.first_signed_at.unwrap_or(t.signed_at)))
        }
        _ => "a line transaction".into(),
    };
    io.save(st);
    io.eth_broadcast(&raw).await;
    Ok(format!("{what}: cancel {} at nonce {} signed, saved and sent", h(&hash), t.nonce))
}

/// The nonce moved past ours with no receipt for any version of the line's transaction. A cancel and what it cancels share a
/// nonce: which one mined is read from the chain. The line is cleared only once the outcome is read (audit 2026-09-27-nft,
/// B-M-3 / C-M-3): a read that fails, or nothing visible yet, keeps it and the next tick reads again. A nonce taken by a transaction
/// that is not ours halts the line (`EthState::halted`) until the operator has looked.
async fn superseded<I: EthIo>(io: &mut I, st: &mut EthState, t: &EthTx) -> Result<String, String> {
    match t.kind.clone() {
        EthKind::Cancel { wrapper, ticket } => {
            // our cancel did not mine (every version was read just above): a version of the delivery did
            for v in t.rivals.iter().copied() {
                if let Some(r) = io.eth_receipt(v).await? {
                    st.line = None;
                    st.eth_out = st.eth_out.saturating_add(r.spent(0));
                    st.note_mined(t.nonce, r.block);
                    let Some(d) = st.delivery_mut(&wrapper, ticket) else { return Ok("an unknown delivery".into()) };
                    d.phase = if r.success { DPhase::Moved { tx: v, block: r.block } }
                        else { DPhase::MarkAfter { why: "the transfer reverted".into(), block: r.block } };
                    if r.success {
                        let (c, tk) = (d.collection, d.token);
                        st.outbound.push((c, tk, r.block));
                    }
                    return Ok(format!("delivery #{ticket}: the transfer mined before its cancel"));
                }
            }
            Err(format!("nonce {} is used but neither our cancel nor any version of the transfer is visible: the line is kept, read again \
                next tick", t.nonce))
        }
        EthKind::CancelFill { buy } => {
            for v in t.rivals.iter().copied() {
                if let Some(r) = io.eth_receipt(v).await? {
                    let value = match st.buy(buy).map(|b| b.phase.clone()) {
                        Some(BPhase::Buying { total, .. }) => total,
                        _ => 0,
                    };
                    st.line = None;
                    st.eth_out = st.eth_out.saturating_add(r.spent(value));
                    st.note_mined(t.nonce, r.block);
                    retag(st, &EthKind::Buy { buy }, v);
                    let fill = EthTx { kind: EthKind::Buy { buy }, hash: v, value, ..t.clone() };
                    return Ok(format!("buy {buy}: the fill mined before its cancel: {}", mined_eth(io, st, &fill, r).await));
                }
            }
            Err(format!("nonce {} is used but neither our cancel nor any version of the fill is visible: the line is kept, read again \
                next tick", t.nonce))
        }
        EthKind::Deliver { .. } | EthKind::Buy { .. } | EthKind::Return { .. } | EthKind::BackDeposit { .. } => {
            // no version of ours mined, and no cancel of ours holds this nonce: a transaction that is not ours took it
            let why = format!("nonce {} of our Ethereum address was used by a transaction that is not ours ({:?}, {}): look before \
                anything else is signed on Ethereum", t.nonce, t.kind, h(&t.hash));
            st.halted = Some(why.clone());
            Err(why)
        }
    }
}

async fn mined_eth<I: EthIo>(io: &mut I, st: &mut EthState, t: &EthTx, r: OurReceipt) -> String {
    match t.kind.clone() {
        EthKind::Buy { buy } => {
            let Some(b) = st.buy(buy).cloned() else { return "an unknown buy".into() };
            let BPhase::Buying { arrived, token, total, tx } = b.phase else { return "not buying".into() };
            if !r.success {
                st.buy_mut(buy).expect("found").phase = BPhase::Arrived { wei: arrived.saturating_sub(r.fee), since: io.now() };
                return format!("buy {buy}: the fill reverted in block {}: the ETH (less gas) waits for another listing", r.block);
            }
            let owner = match io.eth_read(b.collection, &en::call_word(crate::seaport::OWNER_OF, token)).await.and_then(|x| crate::abi::addr_at(&x, 0, 0)) {
                Ok(o) => o,
                Err(e) => {
                    st.line = Some(t.clone()); // read again next tick
                    st.eth_out = st.eth_out.saturating_sub(r.spent(t.value));
                    return format!("buy {buy}: mined; ownerOf not read ({e}): again next tick");
                }
            };
            if Some(owner) != io.our_evm().await.ok() {
                st.buy_mut(buy).expect("found").phase = BPhase::Stuck { why: format!("the fill mined but ownerOf is 0x{}", hex::encode(owner)) };
                return format!("buy {buy}: STUCK: mined but the token is not ours");
            }
            st.buy_mut(buy).expect("found").phase = BPhase::Bought { arrived, token, total, tx, block: r.block };
            // A2-L-1: what the item did not cost stays at our Ethereum address as this collection's leftover, swept back to its module
            let left = arrived.saturating_sub(total).saturating_sub(r.fee);
            st.credit_leftover(&b.collection, left);
            format!("buy {buy}: {} is ours (block {}): waiting for finality; {left} wei left over", h(&token), r.block)
        }
        EthKind::Deliver { wrapper, ticket } => {
            let Some(d) = st.delivery_mut(&wrapper, ticket) else { return "an unknown delivery".into() };
            if r.success {
                d.phase = DPhase::Moved { tx: t.hash, block: r.block };
                let (c, tk) = (d.collection, d.token);
                st.outbound.push((c, tk, r.block));
                format!("delivery #{ticket}: mined in block {}: waiting for finality", r.block)
            } else {
                d.phase = DPhase::MarkAfter { why: format!("the transfer reverted in block {}", r.block), block: r.block };
                format!("delivery #{ticket}: the transfer REVERTED: markUndeliverable once final")
            }
        }
        EthKind::Cancel { wrapper, ticket } => {
            let Some(d) = st.delivery_mut(&wrapper, ticket) else { return "an unknown delivery".into() };
            d.phase = DPhase::MarkAfter { why: format!("cancelled by {} in block {}", h(&t.hash), r.block), block: r.block };
            format!("delivery #{ticket}: our cancel mined: the transfer can never confirm once that is final")
        }
        EthKind::CancelFill { buy } => {
            // the fill never mined: its ETH (less the cancel's gas) waits for another listing
            let Some(b) = st.buy_mut(buy) else { return "an unknown buy".into() };
            let BPhase::Buying { arrived, .. } = b.phase.clone() else { return "not buying".into() };
            b.phase = BPhase::Arrived { wei: arrived.saturating_sub(r.fee), since: io.now() };
            format!("buy {buy}: our cancel of its fill mined (block {}): the ETH waits for another listing", r.block)
        }
        EthKind::BackDeposit { buy } => {
            let Some(b) = st.buy_mut(buy) else { return "an unknown buy".into() };
            let BPhase::BackSending { wei, order_id, value, min_out, deadline, .. } = b.phase.clone() else { return "not going back".into() };
            if !r.success {
                b.phase = BPhase::BackFunded { wei: wei.saturating_sub(r.fee), since: io.now() };
                b.last = None;
                b.ask = None;          // C3-I-3: asked again for what is left, not the old amount
                return format!("buy {buy}: the way back's deposit REVERTED in block {}: the ETH (less gas) waits for another quote", r.block);
            }
            b.phase = BPhase::BackBridging { order_id, value, min_out, deadline, since: io.now() };
            // A2-L-1: the gas reserve the deposit kept, less its gas, is this collection's leftover
            let (c, left) = (b.collection, wei.saturating_sub(value).saturating_sub(r.fee));
            st.credit_leftover(&c, left);
            format!("buy {buy}: the way back's deposit mined (block {}): watching 4663 for {min_out} wei at least", r.block)
        }
        EthKind::Return { tx, token } => {
            let Some(d) = st.deposit_mut(&tx, &token) else { return "an unknown return".into() };
            if r.success {
                d.phase = XPhase::Returned { tx: t.hash };
                let c = d.collection;
                st.outbound.push((c, token, r.block));
                format!("the stray {} went back to its sender in block {}", h(&token), r.block)
            } else {
                d.phase = XPhase::Held { why: format!("the return reverted in block {}", r.block) };
                format!("the return of {} REVERTED: held; look", h(&token))
            }
        }
    }
}

// ---- the home chain's one transaction ----------------------------------------------------------------------------------------------

/// Round 4 (C4-H-1): this tick's home pin against the highest this side has read. A pin below it is a wait for the whole tick (the
/// canister's own pin never goes below one read anywhere in it: `canister::pin`); a world that cannot say its pin is not held here.
async fn home_floor_ok<I: EthIo>(io: &mut I, st: &mut EthState) -> Result<(), String> {
    match io.home_pin().await {
        Ok(Some(p)) => {
            let (n, seen) = st.home_floor.unwrap_or((0, 0));
            if p < seen {
                return Err(format!("the 4663 pin {p} is below block {seen} (a 4663 pin already read: a provider's head fell back): nothing \
                    of 4663 is read or signed at it (C4-H-1)"));
            }
            st.home_floor = Some((n, p));
        }
        // a world that cannot say its pin: the nonce at it must be past our last mined home transaction, or the pin is behind our own
        // transactions (their spends unseen: our own Relay deposit would read as refunded)
        Ok(None) if st.home_nonce_floor() > 0 => {
            if let Ok(n) = io.home_nonce().await {
                if n < st.home_nonce_floor() {
                    return Err(format!("the 4663 nonce at this pin is {n}, under {} (past our last mined home transaction): the pin is behind \
                        our own transactions; nothing of 4663 is read or signed at it (C4-H-1)", st.home_nonce_floor()));
                }
            }
        }
        _ => {}
    }
    Ok(())
}

/// A home step: to the EVM home's sender as before, or, on the Solana home (docs/v9 §4, §8), as `call` on the books (or the money
/// line, for what moves SOL; `col`: the step's collection).
#[allow(clippy::too_many_arguments)]
async fn send_step<I: EthIo>(io: &mut I, st: &mut EthState, step: Step, col: Option<Col>, to: [u8; 20], value: u128, data: Vec<u8>,
    call: home_sol::Call) -> Result<String, String> {
    if io.sol_home().is_some() {
        let c = col.ok_or_else(|| format!("{step:?}: its collection is no longer configured"))?;
        return home_sol::send(io, st, step, &c, call).await;
    }
    send_home(io, st, step, to, value, data).await
}

/// The configured collection whose wrapper is `w`.
fn col_of_wrapper<I: EthIo>(io: &I, w: &[u8; 20]) -> Option<Col> {
    io.config()?.cols().ok()?.into_iter().find(|c| c.wrapper == *w)
}

async fn send_home<I: EthIo>(io: &mut I, st: &mut EthState, step: Step, to: [u8; 20], value: u128, data: Vec<u8>) -> Result<String, String> {
    // C-I-5: the pause is read again at the signature; while paused only what already left (a record, a note, a mark) goes out
    // (R3 M-1(c); round 4 A4-L-2: a mark too, it signs nothing on Ethereum and only lets a holder be refunded)
    if io.paused() && !matches!(step, Step::Record { .. } | Step::Sent { .. } | Step::Mark { .. }) {
        return Err("paused: nothing new is signed".into());
    }
    // round 4 (C4-H-1): never at a nonce one of our home transactions already used: the nonce at this pin must be past the last one
    // seen mined (a pin that moved back shows it unused), else nothing is signed this tick
    let floor = st.home_nonce_floor();
    let at_pin = io.home_nonce().await?;
    if at_pin < floor {
        return Err(format!("the 4663 nonce at this pin is {at_pin}, under {floor} (past our last mined home transaction): the pin is behind \
            our own transactions; nothing is signed (C4-H-1)"));
    }
    let (raw, hash, nonce) = io.home_sign(to, value, &data).await?;
    if nonce < floor {
        return Err(format!("the home transaction was signed at nonce {nonce}, under {floor}: not sent (C4-H-1)"));
    }
    let pin = io.home_pin().await.ok().flatten();
    st.pending = Some(HomeTx { step: step.clone(), raw: raw.clone(), hash, nonce, used_since: None, pin, sent_at: Some(io.now()) });
    io.save(st);
    io.home_broadcast(&raw).await;
    Ok(format!("{step:?} signed, saved and sent (nonce {nonce})"))
}

/// Round 3 (D3-H-1): `noteSent` for a delivery whose transfer mined: from then on the wrapper's late window cannot refund the ticket
/// (only our `markUndeliverable` can), whatever becomes of the record. Nothing to note once the ticket left `Resolved`.
async fn note_sent<I: EthIo>(io: &mut I, st: &mut EthState, i: usize, tx: [u8; 32]) -> Result<Option<String>, String> {
    let d = st.deliveries[i].clone();
    if !still_resolved(io, d.wrapper, d.ticket).await? {
        st.deliveries[i].sent = Some(true);
        return Ok(None);
    }
    // review round 5 (F5-M-1): a noteSent of ours that mined unseen is on the wrapper already: never signed again
    let noted: Word = if io.sol_home().is_some() { home_books::sent_tx(io, d.wrapper, d.ticket).await? } else {
        crate::abi::word_at(&io.home_read(d.wrapper, &en::call_word(en::SENT_TX, crate::abi::uint_word(d.ticket))).await?, 0, 0)?
            .try_into().expect("32")
    };
    if noted == tx {
        st.deliveries[i].sent = Some(true);
        return Ok(None);
    }
    let col = col_of_wrapper(io, &d.wrapper);
    let s = send_step(io, st, Step::Sent { wrapper: d.wrapper, ticket: d.ticket }, col, d.wrapper, 0, en::note_sent_call(d.ticket, tx),
        home_sol::Call::Sent { ticket: d.ticket, tx }).await?;
    Ok(Some(format!("eth delivery #{}: noteSent (its transfer mined; the late window can no longer refund it): {s}", d.ticket)))
}

/// A delivery or a mark marked as final: the ticket is still `Resolved` at the pin (else nothing to record).
async fn still_resolved<I: EthIo>(io: &mut I, w: [u8; 20], t: u128) -> Result<bool, String> {
    Ok(read_ticket(io, w, t).await?.stage == Stage::Resolved)
}

/// A final delivery whose record is next: `None` while its ticket is still `Resolved` (the record is signed). Review round 5
/// (F5-M-1): a ticket already `Delivered` was recorded by a transaction of ours (one taken back unseen): it is recorded, and its item
/// leaves the custody book (it went `Gone` before, holding the item's room for good). Any other stage: gone.
async fn record_done<I: EthIo>(io: &mut I, st: &mut EthState, i: usize) -> Result<Option<String>, String> {
    let d = st.deliveries[i].clone();
    let DPhase::Record { tx } = d.phase else { return Ok(None) };
    match read_ticket(io, d.wrapper, d.ticket).await?.stage {
        Stage::Resolved => Ok(None),
        Stage::Delivered => {
            st.deliveries[i].phase = DPhase::Recorded { tx };
            let chain = io.config().map_or(1, |c| c.chain_id);
            let _ = io.with_caps(|_, bk| custodycap::release(bk, &held_id(chain, &d.collection, &d.token)));
            Ok(Some(format!("eth delivery #{}: its ticket is already Delivered (a record of ours mined unseen): recorded", d.ticket)))
        }
        s => {
            st.deliveries[i].phase = DPhase::Gone { why: format!("the ticket left Resolved before the record ({s:?})") };
            Ok(Some(format!("eth delivery #{}: the ticket is {s:?}: nothing to record", d.ticket)))
        }
    }
}

/// A mark that is next: true when there is nothing to sign. Review round 5 (F5-M-1): a ticket the wrapper already holds undeliverable
/// (a mark of ours taken back unseen) is marked; one no longer `Resolved` is gone.
async fn mark_done<I: EthIo>(io: &mut I, st: &mut EthState, i: usize) -> Result<bool, String> {
    let d = st.deliveries[i].clone();
    let DPhase::Mark { why } = d.phase else { return Ok(true) };
    if !still_resolved(io, d.wrapper, d.ticket).await? {
        st.deliveries[i].phase = DPhase::Gone { why: "the ticket left Resolved before the mark".into() };
        return Ok(true);
    }
    let marked = if io.sol_home().is_some() { home_books::undeliverable(io, d.wrapper, d.ticket).await? } else {
        let r = io.home_read(d.wrapper, &en::call_word(en::UNDELIVERABLE, crate::abi::uint_word(d.ticket))).await?;
        crate::abi::uint_at(&r, 0, 0, 8)? != 0
    };
    if marked {
        st.deliveries[i].phase = DPhase::Marked { why };
        return Ok(true);
    }
    Ok(false)
}

/// `FloorModule.floorBounds()`: what the accepted floor may become if we post now.
pub const FLOOR_BOUNDS: [u8; 4] = [0xd6, 0x4d, 0xcf, 0x23];

/// One due reading posted as the module's keeper, clamped into the module's `floorBounds()` read at the pin (audit 2026-09-27-nft,
/// D-H-2): what the canister posts is what the module accepts, as the keeper's plan was, so the monitor's clamp rule keeps its
/// premise. Between the read and the transaction the bounds only WIDEN (they grow with time, and only our own post moves
/// `accepted`), so the post stays inside them (B2-I-2).
async fn post_reading<I: EthIo>(io: &mut I, st: &mut EthState, i: usize, c: &Col) -> Result<String, String> {
    if io.sol_home().is_some() {
        return home_books::post(io, st, i, c).await;
    }
    let r = st.readings[i].clone();
    let keeper = crate::abi::addr_at(&io.home_read(c.module, &en::KEEPER).await?, 0, 0)?;
    if keeper != io.our_evm().await? {
        st.readings[i].due = false;
        return Err(format!("the module's keeper is 0x{}, not us (paused, or not our module): nothing is posted", hex::encode(keeper)));
    }
    let b = io.home_read(c.module, &FLOOR_BOUNDS).await?;
    let lo = crate::abi::uint_at(&b, 0, 0, 128)?;
    // before the module's first post its bounds are (0, 2^256 − 1): no bound
    let hi = match crate::abi::uint_at(&b, 0, 1, 128) {
        Ok(v) => v,
        Err(_) if crate::abi::word_at(&b, 0, 1).is_ok() => u128::MAX,
        Err(e) => return Err(format!("floorBounds() not read: {e}")),
    };
    let price = r.post.price.max(lo).min(hi.max(lo));
    let s = send_home(io, st, Step::Post { collection: c.collection }, c.module, 0, en::post_floor_call(price, r.post.confidence, r.post.depth)).await?;
    st.readings[i].due = false;
    Ok(format!("eth floor 0x{}: postFloor({price}, {}, {}){}: {s}", hex::encode(c.collection), r.post.confidence, r.post.depth,
        if price != r.post.price { format!(" (our reading {} clamped into the module's bounds [{lo}, {hi}])", r.post.price) } else { String::new() }))
}

/// A due reading, fresh enough to post: `(index, collection, urgent)`. Urgent: the module was last posted `URGENT_POST_SECS` ago.
fn due_readings(st: &EthState, cols: &[Col], now: u64) -> Vec<(usize, Col, bool)> {
    st.readings.iter().enumerate().filter(|(_, r)| r.due && now <= r.at.saturating_add(en::READING_TTL_SECS))
        .filter_map(|(i, r)| cols.iter().find(|c| c.collection == r.collection).map(|c| (i, *c,
            r.module_posted_at.is_none_or(|p| now.saturating_sub(p) >= URGENT_POST_SECS))))
        .collect()
}

/// The paused pass's home-chain act: only what records what already happened (audit 2026-09-27-nft C-L-1, R3 M-1(c)): a final
/// delivery's note (round 4, A4-L-2: at finality) and record, and a mark of one that can never confirm (round 4, A4-L-2: the mark
/// signs nothing on Ethereum and only lets its holder be refunded after the grace, so a pause no longer holds that refund back).
async fn home_act_paused<I: EthIo>(io: &mut I, st: &mut EthState) -> Result<Option<String>, String> {
    for i in 0..st.deliveries.len() {
        let d = st.deliveries[i].clone();
        if let DPhase::Record { tx } = d.phase {
            if let Some(l) = record_done(io, st, i).await? {
                let _ = l;
                continue;
            }
            if d.sent != Some(true) {
                if let Some(l) = note_sent(io, st, i, tx).await? {
                    return Ok(Some(format!("{l} (paused pass)")));
                }
            }
            let col = col_of_wrapper(io, &d.wrapper);
            let s = send_step(io, st, Step::Record { wrapper: d.wrapper, ticket: d.ticket }, col, d.wrapper, 0, en::record_delivery_call(d.ticket, tx),
                home_sol::Call::Record { ticket: d.ticket, tx }).await?;
            st.deliveries[i].phase = DPhase::Recording { tx };
            return Ok(Some(format!("eth delivery #{} (paused pass): recordDelivery: {s}", d.ticket)));
        }
    }
    for i in 0..st.deliveries.len() {
        let d = st.deliveries[i].clone();
        if let DPhase::Mark { why } = d.phase {
            if mark_done(io, st, i).await? {
                continue;
            }
            let col = col_of_wrapper(io, &d.wrapper);
            let s = send_step(io, st, Step::Mark { wrapper: d.wrapper, ticket: d.ticket }, col, d.wrapper, 0, en::mark_call(d.ticket),
                home_sol::Call::Mark { ticket: d.ticket }).await?;
            st.deliveries[i].phase = DPhase::Marking { why };
            return Ok(Some(format!("eth delivery #{} (paused pass): markUndeliverable: {s}", d.ticket)));
        }
    }
    Ok(None)
}

/// A deposit of `c` may be minted now (audit 2026-09-27-nft A-M-1; round 2): our last reading is fresh, its sales are a quorum of
/// the canister's OWN counted sales (`min_sales`: one per token and per wallet, no round trip, over the window), and the module's
/// accepted floor is at most `DEPOSIT_FLOOR_BAND_BPS` over the LOWER of the listing floor and those sales' 25th percentile.
/// Otherwise a deposit then a burn could be paid a floor the market has not traded at: the deposit waits (never refused, never
/// returned). The burn side is the post itself: it rises only as far as the same counted sales support (`floor_from`), and never
/// above a live listing anyone can fill.
async fn deposit_floor_ok<I: EthIo>(io: &mut I, st: &EthState, c: &Col, min_sales: u8) -> Result<(), String> {
    let now = io.now();
    let r = st.reading(&c.collection).ok_or("no floor reading of ours yet: a deposit mints against a verified floor only")?;
    if now > r.at.saturating_add(en::READING_TTL_SECS) {
        return Err(format!("our floor reading is {} s old: a deposit mints against a fresh one only", now - r.at));
    }
    if r.post.sales < min_sales as u32 || r.post.sale_p25 == 0 {
        return Err(format!("{} counted sales: a deposit mints only against a floor the market's own sales confirm ({min_sales} needed)", r.post.sales));
    }
    let accepted = crate::abi::uint_at(&io.home_read(c.module, &en::ACCEPTED).await?, 0, 0, 128)?;
    // review round 4 (E4-I-2 / A4-I-6): never before the module's first post: its unit would be admitted to custody at 0 wei
    if accepted == 0 {
        return Err("the module has no accepted floor yet (never posted): a deposit mints against a posted floor only".into());
    }
    let base = r.post.listing.min(r.post.sale_p25);
    let top = base.saturating_mul(10_000 + en::DEPOSIT_FLOOR_BAND_BPS) / 10_000;
    if accepted > top {
        return Err(format!("the module's accepted floor {accepted} is more than {} % over the lower of the listing floor {} and the counted \
            sales' 25th percentile {}: a deposit then a burn would be paid a floor the market has not traded at; it waits",
            en::DEPOSIT_FLOOR_BAND_BPS / 100, r.post.listing, r.post.sale_p25));
    }
    Ok(())
}

/// Review round 4 (E4-H-1): the H3 key of a collection's NEXT mint, a purchase's or a deposit's: the collection and how many of its
/// mints went out before (buys minting or past it, deposits minting or minted). While one waits, nothing moves the count, so every
/// tick's ask of it (whichever buy or deposit asks) is the same key: one ask in the tripwire's demand, as C3-H-1's advances. A mint
/// that reverts (or is dropped) takes its count back and asks again under its own key, which counted it once. A collection's demand is
/// therefore at most what went out in the window plus one: 3 + 1 against the 9 that trip at stage 0's 3 a day.
fn mint_key(st: &EthState, col: &str, c: &[u8; 20]) -> String {
    let buys = st.buys.iter().filter(|b| b.collection == *c && !b.is_sweep() && matches!(b.phase, BPhase::Minting { .. } | BPhase::Minted { .. }
        | BPhase::Settling { .. } | BPhase::Settled { .. })).count();
    let deposits = st.deposits.iter().filter(|d| d.collection == *c && matches!(d.phase, XPhase::Minting | XPhase::Minted)).count();
    format!("mint:eth:{col}:{}", buys + deposits)
}

/// The most urgent home-chain act: a floor post whose module is near stale (every holder's mints and burns wait on it), records and
/// marks (holders wait on them), then the buys' steps, then deposits' mints, then the other floor posts. Every step's wait is its
/// own (audit 2026-09-27-nft, B-H-2 / C-H-1): none returns before the floor step.
async fn home_act<I: EthIo>(io: &mut I, st: &mut EthState, cfg: &EthNftConfig, cols: &[Col], unsettled: bool) -> Result<Option<String>, String> {
    let chain = cfg.chain_id;
    let now = io.now();
    let mut waits: Vec<String> = Vec::new();
    // 0. an urgent floor post
    for (i, c, urgent) in due_readings(st, cols, now) {
        if !urgent {
            continue;
        }
        match post_reading(io, st, i, &c).await {
            Ok(l) => return Ok(Some(l)),
            Err(e) => waits.push(format!("eth floor 0x{}: {e}", hex::encode(c.collection))),
        }
    }
    // 1. records and marks
    for i in 0..st.deliveries.len() {
        let d = st.deliveries[i].clone();
        let r: Result<Option<String>, String> = async {
            match d.phase {
                DPhase::Record { tx } => {
                    if let Some(l) = record_done(io, st, i).await? {
                        return Ok(Some(l));
                    }
                    // round 4 (A4-L-2): noted sent only now, at finality, from the same receipt as the record: a transfer a reorg
                    // drops before finality is never noted, so the wrapper's late window still refunds its ticket
                    if d.sent != Some(true) {
                        if let Some(l) = note_sent(io, st, i, tx).await? {
                            return Ok(Some(l));
                        }
                    }
                    let col = col_of_wrapper(io, &d.wrapper);
                    let s = send_step(io, st, Step::Record { wrapper: d.wrapper, ticket: d.ticket }, col, d.wrapper, 0,
                        en::record_delivery_call(d.ticket, tx), home_sol::Call::Record { ticket: d.ticket, tx }).await?;
                    st.deliveries[i].phase = DPhase::Recording { tx };
                    Ok(Some(format!("eth delivery #{}: recordDelivery: {s}", d.ticket)))
                }
                DPhase::Mark { why } => {
                    if mark_done(io, st, i).await? {
                        return Ok(None);
                    }
                    let col = col_of_wrapper(io, &d.wrapper);
                    let s = send_step(io, st, Step::Mark { wrapper: d.wrapper, ticket: d.ticket }, col, d.wrapper, 0, en::mark_call(d.ticket),
                        home_sol::Call::Mark { ticket: d.ticket }).await?;
                    st.deliveries[i].phase = DPhase::Marking { why };
                    Ok(Some(format!("eth delivery #{}: markUndeliverable: {s}", d.ticket)))
                }
                _ => Ok(None),
            }
        }.await;
        match r {
            Ok(Some(s)) => return Ok(Some(s)),
            Ok(None) => {}
            Err(e) => waits.push(format!("eth delivery {}#{}: {e}", h(&d.wrapper), d.ticket)),
        }
    }
    // 2. the buys, oldest first: each collection's step is its own, and one that cannot act now never holds up another
    let open: Vec<EthBuy> = st.buys.iter().filter(|b| !b.finished()).cloned().collect();
    for b in open {
        let r: Result<Option<String>, String> = async {
                let c = cols.iter().find(|c| c.collection == b.collection).copied();
                match b.phase.clone() {
                    BPhase::Final { arrived, token, total, tx } => {
                        let c = c.ok_or("the buy's collection is no longer configured")?;
                        // review round 5 (E5-L-1 / F5-M-1): a mint of ours that mined but was read as not (a failed read, an earlier build's
                        // drop): the wrapper already holds the token, so it is minted, never asked again
                        if wrapper_holds(io, c.wrapper, &token, None).await? {
                            st.buy_mut(b.id).expect("open").phase = BPhase::Minting { arrived, token, total, tx };
                            let out = mined_home(io, st, cfg, &Step::Mint { buy: b.id }, true).await;
                            return Ok(Some(format!("eth buy {}: the wrapper already holds its token: {out}", b.id)));
                        }
                        let col = key_of(&c, chain);
                        // round 4 (E4-H-1): the collection's next mint, the same key however long it waits (as C3-H-1's advances)
                        let key = mint_key(st, &col, &c.collection);
                        io.outflow_check(&key, &col, Flow::Mint, 1)?;
                        let s = send_step(io, st, Step::Mint { buy: b.id }, Some(c), c.wrapper, 0, en::mint_call(false, token, tx, &c.module),
                            home_sol::Call::Mint { token, tx }).await?;
                        io.outflow_record(&key, &col, Flow::Mint, 1);
                        st.buy_mut(b.id).expect("open").phase = BPhase::Minting { arrived, token, total, tx };
                        return Ok(Some(format!("eth buy {}: mint to the module: {s}", b.id)));
                    }
                    BPhase::Minted { arrived, token, total, tx } => {
                        let cost = en::cost_of(b.advanced, total, arrived)?;
                        let s = send_step(io, st, Step::Settle { buy: b.id }, c, b.module, 0, crate::relayconv::settle_call(cost),
                            home_sol::Call::Settle(cost)).await?;
                        st.buy_mut(b.id).expect("open").phase = BPhase::Settling { token, total, tx, cost };
                        return Ok(Some(format!("eth buy {}: settleAdvance({cost}): {s}", b.id)));
                    }
                    // round 4: a settle that reverted or was dropped (no home transaction is out, or home_act would not run)
                    BPhase::Settling { cost, .. } => {
                        let s = send_step(io, st, Step::Settle { buy: b.id }, c, b.module, 0, crate::relayconv::settle_call(cost),
                            home_sol::Call::Settle(cost)).await?;
                        return Ok(Some(format!("eth buy {}: settleAdvance({cost}) again: {s}", b.id)));
                    }
                    BPhase::Returning { wei } => {
                        // review round 5 (F5-M-1): a returnAdvance of ours that mined unseen (an earlier build took it back) already closed the
                        // module's advance: nothing is paid again, the buy is returned
                        if !b.is_sweep() && !transit_open(io, b.module).await? {
                            let out = mined_home(io, st, cfg, &Step::GiveBack { buy: b.id }, true).await;
                            return Ok(Some(format!("eth buy {}: the module's advance is already closed (our returnAdvance mined): {out}", b.id)));
                        }
                        let bal = io.home_balance().await?;
                        if bal < wei {
                            return Err(format!("our balance {bal} is under the {wei} to return"));
                        }
                        // A2-L-1: a sweep has no advance open: its ETH joins the module's surplus (`fundReserve`)
                        let (call, what) = if b.is_sweep() { (fund_reserve_call(), "fundReserve") } else { (crate::relayconv::return_call(), "returnAdvance") };
                        let s = send_step(io, st, Step::GiveBack { buy: b.id }, c, b.module, wei, call,
                            home_sol::Call::GiveBack { lamports: wei, sweep: b.is_sweep() }).await?;
                        return Ok(Some(format!("eth buy {}: {what} of {wei}: {s}", b.id)));
                    }
                    BPhase::NeedAdvance => {
                        // C2-L-2: a buy whose collection an upgrade dropped never holds the Relay leg: nothing was paid yet
                        let Some(c) = c else {
                            st.buys.retain(|x| x.id != b.id);
                            return Ok(Some(format!("eth buy {}: its collection is no longer configured: dropped before its advance, nothing spent", b.id)));
                        };
                        // review round 5 (F5-M-1): an advance of ours already open at the module (one an earlier build took back while it had
                        // mined unseen) is this buy's: adopted at what the module paid, never paid again
                        if transit_open(io, c.module).await? {
                            return adopt_advance(io, st, &c, b.id, chain).await.map(Some);
                        }
                        // docs/v9 §2.3: on the Solana home the module is in lamports, its floor read in wei at this tick's rate
                        let (v, accepted, rate) = if io.sol_home().is_some() {
                            let g = home_books::gate(io, &c).await?;
                            (g.view, g.accepted_wei, Some(g.rate))
                        } else {
                            let (v, a) = module_view(io, &c).await?;
                            (v, a, None)
                        };
                        let covers = |cap: u128, floor: u128| match rate {
                            Some(r) => crate::solhome::advance_covers(cap, floor, r),
                            None => advance_covers(cap, floor),
                        };
                        if let Err(e) = itemsink::may_start(&v, 0) {
                            st.buys.retain(|x| x.id != b.id);
                            return Ok(Some(format!("eth buy {}: the module no longer wants it ({e}): dropped, nothing spent", b.id)));
                        }
                        let item_cap = v.item_cap.ok_or("the module states no item cap")?;
                        // A2-M-1: read again right before the module pays: the pool covers the module's floor
                        if let Err(e) = covers(item_cap, accepted) {
                            st.buys.retain(|x| x.id != b.id);
                            return Ok(Some(format!("eth buy {}: dropped before its advance, nothing spent: {e}", b.id)));
                        }
                        // A2-L-1: only the buy's bound is advanced; it still reaches Ethereum covering the floor that bounds the buy + 1 %
                        let own = st.reading(&c.collection).map(|r| (r.post.price, r.at));
                        let (amount, floor) = match rate {
                            Some(r) => crate::solhome::advance_bound(item_cap, accepted, own, io.now(), c.max_premium_bps, c.max_purchase_wei,
                                cfg.relay_max_fee_bps, r),
                            None => advance_bound(item_cap, accepted, own, io.now(), c.max_premium_bps, c.max_purchase_wei, cfg.relay_max_fee_bps),
                        };
                        if let Err(e) = covers(amount, floor) {
                            st.buys.retain(|x| x.id != b.id);
                            return Ok(Some(format!("eth buy {}: dropped before its advance, nothing spent: {e}", b.id)));
                        }
                        if amount <= keep_of(io) {
                            return Err(format!("{amount} wei would leave nothing to bridge after the gas keep"));
                        }
                        // the custody caps and the outflow limits count the advance at its ETH worth
                        let value = match rate { Some(r) => crate::rates::lamports_to_wei(amount, r), None => amount };
                        // H3 and H2, before the module pays. Round 3 (C3-H-1): the advance's H3 key names the collection and how many of its
                        // advances went out before, not the buy: a buy whose advance waits on the day's value is dropped (it never holds
                        // the Relay leg) and opened again next tick under a new id, and the same wait must be asked (and counted toward the
                        // tripwire) once, not once a tick. An advance that reverted asks again under its own key (it released nothing)
                        let n = st.buys.iter().filter(|x| x.collection == b.collection && !x.is_sweep() && x.committed()).count();
                        let (vkey, flight, col) = (format!("advance:eth:{}:{n}", key_of(&c, chain)), format!("eth:{}", b.id), key_of(&c, chain));
                        // audit 2026-09-27-nft B-H-2 / B-M-5: a buy that cannot take its value or room now is dropped (nothing was paid),
                        // so it never holds the one Relay leg while it waits; the round robin tries the next collection
                        let room = io.outflow_check(&vkey, &col, Flow::Value, value).and_then(|_| io.with_caps(|p, bk|
                            custodycap::reserve(p, bk, &flight, &col, value)).map_err(|e| format!("custody cap: {e:?}")));
                        if let Err(e) = room {
                            st.buys.retain(|x| x.id != b.id);
                            return Ok(Some(format!("eth buy {}: dropped before its advance, nothing spent ({e}): the Relay leg is free", b.id)));
                        }
                        match send_step(io, st, Step::Advance { buy: b.id }, Some(c), c.module, 0, crate::relayconv::advance_call(amount),
                            home_sol::Call::Advance(amount)).await {
                            Ok(s) => {
                                io.outflow_record(&vkey, &col, Flow::Value, value);
                                // docs/v7/dormant-collections.md §4.3: the floor this buy's advance was sized on, for the fill's check
                                if let Some(l) = st.dorm.as_mut().and_then(|d| d.life_mut(&c.collection)) {
                                    l.buy_floor = Some((b.id, floor));
                                }
                                let x = st.buy_mut(b.id).expect("open");
                                x.advanced = amount;
                                x.phase = BPhase::Advancing;
                                return Ok(Some(format!("eth buy {}: advance of {amount} {}: {s}", b.id, unit_of(io))));
                            }
                            Err(e) => {
                                let _ = io.with_caps(|_, bk| custodycap::settle(bk, &flight, None));
                                return Err(e);
                            }
                        }
                    }
                    BPhase::Funded { wei, since } => {
                        // C2-L-2: the collection was dropped by an upgrade: the advance goes back to the module it came from. Audit C3-03:
                        // so does a funded buy that no quote reached for `FUNDED_QUOTE_SECS` (a runner that cannot quote its chain's leg):
                        // `Funded` holds the one Relay leg canister-wide (every chain's buys, ways back and the frog side's conversions)
                        let quoteless = !st.inbox.quotes.iter().any(|(id, ..)| *id == b.id) && since != 0
                            && now.saturating_sub(since) >= FUNDED_QUOTE_SECS;
                        if c.is_none() || quoteless {
                            let bal = io.home_balance().await?;
                            let back = wei.min(bal.saturating_sub(keep_of(io)));
                            let x = st.buy_mut(b.id).expect("open");
                            x.phase = BPhase::Returning { wei: back };
                            x.ask = None;
                            let unit = if io.sol_home().is_some() { "lamports" } else { "wei" };
                            if c.is_none() {
                                return Ok(Some(format!("eth buy {}: its collection is no longer configured: {back} {unit} goes back to its module", b.id)));
                            }
                            return Ok(Some(format!("eth buy {}: no Relay quote was judged within {FUNDED_QUOTE_SECS} s of its funding: {back} {unit} \
                                goes back to its module and the Relay leg is free (audit C3-03)", b.id)));
                        }
                        // C2-M-1: the arrival watch reads our Ethereum balance now; while a transaction of ours may be mined at this
                        // pin and not yet counted (its receipt or ownerOf unread), the quote waits a tick
                        if unsettled && st.inbox.quotes.iter().any(|(id, ..)| *id == b.id) {
                            return Err("the Ethereum line's outcome is not read at this pin: the arrival watch starts once every spend \
                                of ours is counted (C2-M-1)".into());
                        }
                        // docs/v9 §2.8: on the Solana home the quote has a Solana origin (rules S1–S12) and our own deposit_native
                        if let (true, Some(c)) = (io.sol_home().is_some(), c) {
                            return home_sol::fund(io, st, cfg, &b, &c, wei).await;
                        }
                        if let Some(i) = st.inbox.quotes.iter().position(|(id, ..)| *id == b.id) {
                            // E3-M-1: the watch starts only at a pin at or above every spend of ours counted (a pin that moved back
                            // would take `eth_before` without a fill `eth_out` counts); the quote waits in the inbox meanwhile
                            let pin = eth_floor(io, st, None).await?;
                            let (_, body, at) = st.inbox.quotes.remove(i);
                            let allocator = crate::abi::addr_at(&io.home_read(re::TO_ETHEREUM.depository, &crate::relayconv::ALLOCATOR).await?, 0, 0)?;
                            let ours = io.our_evm().await?;
                            let w = re::Want { leg: re::TO_ETHEREUM, ours, amount_wei: wei, max_fee_bps: cfg.relay_max_fee_bps, allocator,
                                now: io.now(), handed_in_at: at, used: &st.used_orders };
                            let d = re::judge(&body, &w).map_err(|e| format!("eth buy {}: quote refused, nothing signed: {e}", b.id))?;
                            let bal = io.home_balance().await?;
                            if bal < d.value {
                                return Err(format!("our balance {bal} is under the deposit's {}", d.value));
                            }
                            let eth_before = io.eth_balance().await?;
                            // review round 6 (C6-I-1): read, never skipped on a failure
                            let home_pin = io.home_pin().await?;
                            // the order is used from here on, sent or not (rule 12)
                            st.used_orders.push(d.order_id);
                            let out_before = st.eth_out;
                            {
                                let x = st.buy_mut(b.id).expect("open");
                                x.orders.push(d.order_id);
                                x.phase = BPhase::Depositing { order_id: d.order_id, value: d.value, min_out: d.min_out, deadline: d.deadline };
                                x.ask = None;
                                x.last = Some(Watch { order_id: d.order_id, value: d.value, min_out: d.min_out, deadline: d.deadline,
                                    balance_after: bal.saturating_sub(d.value), eth_before, eth_out_before: out_before, pin: Some(pin), solver: d.solver,
                                    request_id: d.request_id, home_pin });
                            }
                            io.save(st);
                            match send_home(io, st, Step::Deposit { buy: b.id }, d.to, d.value, d.data.clone()).await {
                                Ok(s) => {
                                    let hash = st.pending.as_ref().map(|p| p.hash).unwrap_or_default();
                                    let x = st.buy_mut(b.id).expect("open");
                                    x.deposits.push(hash);
                                    x.attempts += 1;
                                    return Ok(Some(format!("eth buy {}: Relay deposit of {} wei (order 0x{}, at least {} wei on Ethereum): {s}", b.id,
                                        d.value, hex::encode(d.order_id), d.min_out)));
                                }
                                Err(e) => {
                                    let ask = ask_for(io, wei).await.ok();
                                    let x = st.buy_mut(b.id).expect("open");
                                    x.phase = BPhase::Funded { wei, since: io.now() };
                                    x.ask = ask;
                                    return Err(e);
                                }
                            }
                        }
                    }
                    _ => {}
                }
            Ok(None)
        }.await;
        match r {
            Ok(Some(s)) => return Ok(Some(s)),
            Ok(None) => {}
            Err(e) => waits.push(format!("eth buy {}: {e}", b.id)),
        }
    }
    // 3. deposits' mints: the first that may mint now (C-L-2: one collection's wait never holds up another's), never while a buy of
    // its collection holds the mint room from its advance to its own mint (C-H-1 / A-L-1), and only against a floor the market has
    // traded at (A-M-1). Round 4 (E4-H-1): one ask per collection, under the collection's next-mint key: a collection whose first
    // candidate waits is not asked again this pass, and its waiting mint is ONE ask in the tripwire's demand however many deposits
    // wait behind it and however long (before, each waiting deposit was its own ask, and nine in a day froze custody)
    // Review round 5 (E5-L-1): only a wait that is the COLLECTION's (H3, the floor, a purchase holding the mint room, a module read)
    // skips its other deposits this pass; a mint of one deposit that would not succeed (its estimate reverts) moves on to the next, and
    // a deposit whose token the wrapper already holds from this very transaction (a mint of ours read as not minted) is minted
    let candidates: Vec<usize> = (0..st.deposits.len()).filter(|&i| st.deposits[i].phase == XPhase::Mint).collect();
    let mut waited: Vec<[u8; 20]> = Vec::new();
    for i in candidates {
        let d = st.deposits[i].clone();
        if waited.contains(&d.collection) {
            continue;
        }
        let mut own = false;
        let r: Result<Option<String>, String> = async {
            // docs/v7/dormant-collections.md §2.3: a deposit mints only once its collection is ACTIVE (a floor confirmed by the backfill)
            let kind = kind_of(st, cfg, &d.collection);
            if !kind.live() {
                return Err(format!("its collection is {}: the mint waits until it is active (the deposit asked for its wake)", kind.word()));
            }
            if st.buy_holds_mint(&d.collection, now) {
                return Err("a purchase of its collection holds the day's mint room until its own mint".into());
            }
            let Some(c) = cols.iter().find(|c| c.collection == d.collection).copied() else {
                st.deposits[i].phase = XPhase::Held { why: "its collection is no longer configured".into() };
                return Ok(None);
            };
            deposit_floor_ok(io, st, &c, cfg.floor_params().min_sales).await?;
            let held = held_id(chain, &d.collection, &d.token);
            let col = key_of(&c, chain);
            let key = mint_key(st, &col, &d.collection);
            io.outflow_check(&key, &col, Flow::Mint, 1)?;
            let mark = module_floor(io, c.module).await?.0;          // A3-L-4: at the floor its unit is minted against
            if let Err(e) = io.with_caps(|p, bk| if bk.contains(&held) { Ok(()) } else { custodycap::admit(p, bk, &held, &col, mark) }) {
                // the room went to a purchase since finality: the holder gets the NFT back
                st.deposits[i].phase = XPhase::Return { why: format!("no custody room at the mint ({e:?})") };
                return Ok(None);
            }
            let s = match send_home(io, st, Step::MintDeposit { tx: d.tx }, c.wrapper, 0, en::mint_call(true, d.token, d.tx, &d.recipient)).await {
                Ok(s) => s,
                Err(e) => {
                    own = true;
                    if wrapper_holds(io, c.wrapper, &d.token, Some(&d.tx)).await.unwrap_or(false) {
                        io.outflow_record(&key, &col, Flow::Mint, 1);
                        st.deposits[i].phase = XPhase::Minted;
                        return Ok(Some(format!("eth deposit {}: the wrapper already holds its token from this transaction (a mint of ours read as \
                            not minted): minted", h(&d.tx))));
                    }
                    return Err(e);
                }
            };
            io.outflow_record(&key, &col, Flow::Mint, 1);
            st.deposits[i].phase = XPhase::Minting;
            Ok(Some(format!("eth deposit {}: mintDeposit: {s}", h(&d.tx))))
        }.await;
        match r {
            Ok(Some(s)) => return Ok(Some(s)),
            Ok(None) => {}
            Err(e) => {
                if !own {
                    waited.push(d.collection);
                }
                waits.push(format!("eth deposit {}: waits: {e}", h(&d.tx)));
            }
        }
    }
    // 4. the floor, when due
    for (i, c, _) in due_readings(st, cols, now) {
        match post_reading(io, st, i, &c).await {
            Ok(l) => return Ok(Some(l)),
            Err(e) => waits.push(format!("eth floor 0x{}: {e}", hex::encode(c.collection))),
        }
    }
    // 5. docs/v7/dormant-collections.md §5: a graduation queued while its module was stale, filled once the floor is fresh
    // docs/v9 §2.1: the books have no fill (DBC owns the graduation queue): no fill hunt on the Solana home
    if cfg.dormancy.is_some() && io.sol_home().is_none() {
        match fill_act(io, st, cfg, cols).await {
            Ok(Some(l)) => return Ok(Some(l)),
            Ok(None) => {}
            Err(e) => waits.push(e),
        }
    }
    if cfg.dormancy.is_some() {
        // 6. independent review DI-M-1: the post that served a desk request, recorded on the desk inside its window
        match mark_act(io, st, cfg, cols).await {
            Ok(Some(l)) => return Ok(Some(l)),
            Ok(None) => {}
            Err(e) => waits.push(e),
        }
    }
    if waits.is_empty() { Ok(None) } else { Err(waits.join("; ")) }
}

async fn resolve_home<I: EthIo>(io: &mut I, st: &mut EthState, cfg: &EthNftConfig, p: HomeTx) -> Result<String, String> {
    io.home_broadcast(&p.raw).await;
    let view = io.home_seen(p.hash).await;
    // review round 6 (F6-L-2): the providers disagreeing on its receipt (one misses it, or answers it differently) is the likelier
    // index failure; with its nonce used it is decided as "seen nowhere" is: identified, or read back by its effect, or (a Relay deposit,
    // a fundReserve) paged for the operator after the hour. Its clock is not restarted by the disagreement. With the nonce unused it
    // waits, as before
    let unsure = match &view {
        EvmView::Unsure(e) => Some(e.clone()),
        _ => None,
    };
    match view {
        EvmView::Mined { success } => {
            st.pending = None;
            // round 4 (C4-H-1): nothing is signed at this nonce or under it again, whatever a later pin says
            st.note_home_nonce(p.nonce + 1);
            Ok(format!("{:?} mined ({}): {}", p.step, if success { "success" } else { "REVERTED" }, mined_home(io, st, cfg, &p.step, success).await))
        }
        EvmView::NotYet | EvmView::Unsure(_) => {
            let chain = match io.home_nonce().await {
                Ok(n) => n,
                Err(e) => return Err(unsure.map_or(e.clone(), |u| format!("{u}; and the nonce not read: {e}"))),
            };
            if chain <= p.nonce {
                if let Some(x) = st.pending.as_mut() {
                    x.used_since = None;
                }
                if let Some(u) = unsure {
                    return Err(u);
                }
                return Ok(format!("{:?} sent again (nonce {}), not seen yet", p.step, p.nonce));
            }
            // its nonce is used, and no provider shows it (no receipt at the pin, no transaction by hash). Review round 5 (F5-M-1): since
            // round 4's floors nothing of ours signs at a used nonce, so this is almost always OUR transaction mined while every
            // provider's index misses it. A step is never taken back on that absence alone: first what used the nonce is identified
            // (our nonce count at past blocks and that block's transactions, not the index), and the step's own effect is read back
            let now = io.now();
            let since = st.pending.as_mut().map_or(now, |x| *x.used_since.get_or_insert(now));
            let unseen = now.saturating_sub(since);
            let who = io.home_nonce_tx(p.nonce, p.pin).await;
            let effect = home_effect(io, st, cfg, &p.step).await;
            match who {
                Ok(Some(h)) if h == p.hash => {
                    // ours, mined: its status is its effect where one is readable (a Relay deposit, a fundReserve and a post leave none;
                    // they are taken as succeeded: a Relay deposit that reverted is read by its watch as refunded, the ETH being there)
                    let ok = match effect {
                        Ok(Some(x)) => x,
                        Ok(None) => true,
                        Err(e) => return Err(format!("nonce {} was used by {:?} itself ({}; found by our nonce count, every index misses it), \
                            but its effect is not read yet: {e}", p.nonce, p.step, h32(&h))),
                    };
                    st.pending = None;
                    st.note_home_nonce(p.nonce + 1);
                    let out = mined_home(io, st, cfg, &p.step, ok).await;
                    Ok(format!("{:?} (nonce {}) MINED ({}), found by our nonce count although no provider shows its receipt: {out}", p.step,
                        p.nonce, if ok { "its effect is on chain" } else { "its effect is not: reverted" }))
                }
                Ok(Some(h)) => {
                    // another transaction of ours took the nonce (one an earlier build signed at a pin that had moved back): ours can
                    // never mine. What it did is its own effect: read back, never assumed
                    let done = match effect {
                        Ok(Some(x)) => x,
                        Ok(None) => false,
                        Err(e) => return Err(format!("nonce {} was used by {} (not {:?}), but the step's effect is not read yet: {e}", p.nonce,
                            h32(&h), p.step)),
                    };
                    st.pending = None;
                    st.note_home_nonce(chain);
                    let out = mined_home(io, st, cfg, &p.step, done).await;
                    Ok(format!("{:?} (nonce {}) DROPPED: its nonce was used by {} (another transaction of ours); {}: {out}", p.step, p.nonce, h32(&h),
                        if done { "its effect is on chain all the same" } else { "taken back as a revert" }))
                }
                Ok(None) => {
                    if let Some(x) = st.pending.as_mut() {
                        x.used_since = None;
                    }
                    Ok(format!("{:?} (nonce {}): the nonce is not used at the search's pin: sent again", p.step, p.nonce))
                }
                Err(why) => match effect {
                    // what used the nonce cannot be told (a provider that keeps no state that deep, a disagreement): the step's own
                    // effect decides, and only a readable one
                    Ok(Some(true)) => {
                        st.pending = None;
                        st.note_home_nonce(p.nonce + 1);
                        let out = mined_home(io, st, cfg, &p.step, true).await;
                        Ok(format!("{:?} (nonce {}): no provider shows it, but its effect is on chain: MINED: {out}", p.step, p.nonce))
                    }
                    Ok(Some(false)) | Ok(None) if unseen < HOME_NONCE_TAKEN_SECS => Err(format!("nonce {} is used but {:?} is not visible \
                        ({why}); read again (its effect is read back {} s after {since} at the earliest, F5-M-1)", p.nonce, p.step,
                        HOME_NONCE_TAKEN_SECS)),
                    Ok(Some(false)) => {
                        // its effect is absent at a pin past its nonce, seen so on every read for an hour: it did not happen
                        st.pending = None;
                        st.note_home_nonce(chain);
                        let out = mined_home(io, st, cfg, &p.step, false).await;
                        Ok(format!("{:?} (nonce {}) DROPPED: its nonce is used and its effect is absent at the pin, seen nowhere for {unseen} s: \
                            taken back as a revert: {out}", p.step, p.nonce))
                    }
                    Ok(None) if matches!(p.step, Step::Post { .. }) => {
                        // a floor post: doing it again is harmless (the next reading is simply due)
                        st.pending = None;
                        st.note_home_nonce(chain);
                        let out = mined_home(io, st, cfg, &p.step, false).await;
                        Ok(format!("{:?} (nonce {}) DROPPED after {unseen} s seen nowhere (a post: done again harmlessly): {out}", p.step, p.nonce))
                    }
                    Ok(None) => {
                        // a Relay deposit or a sweep's fundReserve: nothing on chain says whether it happened, and doing it again would pay
                        // twice. It waits, paged, until what used the nonce is identified or the operator has looked (eth_home_clear)
                        if let Some((_, mined, _)) = st.inbox.home_clear.filter(|(n, ..)| *n == p.nonce) {
                            st.inbox.home_clear = None;
                            st.pending = None;
                            st.note_home_nonce(if mined { p.nonce + 1 } else { chain });
                            let out = mined_home(io, st, cfg, &p.step, mined).await;
                            return Ok(format!("{:?} (nonce {}): the operator says it {} (eth_home_clear): {out}", p.step, p.nonce,
                                if mined { "MINED" } else { "did NOT mine" }));
                        }
                        Err(format!("UNIDENTIFIED: nonce {} is used, {:?} is seen nowhere for {unseen} s and what used the nonce cannot be told \
                            ({why}); it moves value, so it is never done again on the absence of evidence: it waits until the chain shows it, or \
                            the operator looks and says (eth_home_clear) (F5-M-1)", p.nonce, p.step))
                    }
                    Err(e) => Err(format!("nonce {} is used but {:?} is not visible ({why}), and its effect is not read: {e}", p.nonce, p.step)),
                },
            }
        }
        // review round 5 (F5-M-1): a provider that shows it (in a pool, or the providers disagree) restarts the clock: "seen nowhere"
        // is every read in a row, never two samples an hour apart
        EvmView::Pending => {
            if let Some(x) = st.pending.as_mut() {
                x.used_since = None;
            }
            Ok(format!("{:?} pending (nonce {})", p.step, p.nonce))
        }
    }
}

fn h32(x: &[u8; 32]) -> String {
    h(x)
}

/// Review round 5 (F5-M-1): the module's open advance, which no buy tracks (an earlier build took a mined advance back as a revert), is
/// buy `id`'s: what the module paid (`advanced()`, at the pin) is recorded under the collection's advance key (the key it was asked
/// under: recording it again is idempotent), its custody room is reserved again, and the buy is funded with it as if its advance had
/// just been read mined. Nothing is paid again.
async fn adopt_advance<I: EthIo>(io: &mut I, st: &mut EthState, c: &Col, id: u64, chain: u64) -> Result<String, String> {
    let wei = if io.sol_home().is_some() { home_books::advanced(io, c.module).await? } else {
        crate::abi::uint_at(&io.home_read(c.module, &en::ADVANCED).await?, 0, 0, 128)?
    };
    let keep = keep_of(io);
    if wei <= keep {
        return Err(format!("the module's advance is open, paying {wei} wei: no buy of ours tracks it; the operator looks"));
    }
    let n = st.buys.iter().filter(|x| x.collection == c.collection && !x.is_sweep() && x.committed()).count();
    let (vkey, flight, col) = (format!("advance:eth:{}:{n}", key_of(c, chain)), format!("eth:{id}"), key_of(c, chain));
    // the caps count it at its ETH worth (lamports at the rate on the Solana home)
    let value = value_wei_of(io, wei).await?;
    io.outflow_record(&vkey, &col, Flow::Value, value);
    let room = io.with_caps(|p, bk| custodycap::reserve(p, bk, &flight, &col, value)).map_err(|e| format!("{e:?}"));
    let funded = wei - keep;
    let ask = ask_for(io, funded).await.ok();
    let now = io.now();
    let x = st.buy_mut(id).expect("open");
    x.advanced = wei;
    x.phase = BPhase::Funded { wei: funded, since: now };
    x.ask = ask;
    Ok(format!("eth buy {id}: the module's advance of {wei} wei is already open (an advance of ours that mined unseen): adopted, nothing \
        paid again; the runner is asked for a Relay quote{}", room.err().map(|e| format!(" (its custody room: {e})")).unwrap_or_default()))
}

/// Review round 5 (F5-M-1): whether the module's advance is open (`transitUnits`), at this tick's pin.
async fn transit_open<I: EthIo>(io: &mut I, module: [u8; 20]) -> Result<bool, String> {
    if io.sol_home().is_some() {
        return home_books::transit_open(io, module).await;
    }
    Ok(crate::abi::uint_at(&io.home_read(module, &crate::relayconv::TRANSIT_UNITS).await?, 0, 0, 128)? != 0)
}

/// Review round 5 (F5-M-1): whether the wrapper holds `token` from the transaction `tx` (a mint of ours): `itemOf`, then the item.
async fn wrapper_holds<I: EthIo>(io: &mut I, wrapper: [u8; 20], token: &Word, tx: Option<&[u8; 32]>) -> Result<bool, String> {
    if io.sol_home().is_some() {
        return home_books::wrapper_holds(io, wrapper, token, tx).await;
    }
    let n = crate::abi::uint_at(&io.home_read(wrapper, &en::call_word(en::ITEM_OF, *token)).await?, 0, 0, 128)?;
    if n == 0 {
        return Ok(false);
    }
    let Some(tx) = tx else { return Ok(true) };
    let (tok, src, _) = en::decode_item(&io.home_read(wrapper, &en::call_word(en::ITEM, crate::abi::uint_word(n - 1))).await?)?;
    Ok(tok == *token && src == *tx)
}

/// Review round 5 (F5-M-1): what the chain shows, at this tick's pin, of a home step's own effect: `Some(true)` it happened, `Some(false)`
/// it did not, `None` when it leaves nothing readable (a Relay deposit, a sweep's `fundReserve`, a floor post). A step is taken back
/// on the canister's own reading only when its effect is readable and absent.
async fn home_effect<I: EthIo>(io: &mut I, st: &EthState, cfg: &EthNftConfig, step: &Step) -> Result<Option<bool>, String> {
    let buy = |id: &u64| st.buy(*id).cloned().ok_or_else(|| format!("an unknown buy {id}"));
    match step {
        Step::Advance { buy: id } => Ok(Some(transit_open(io, buy(id)?.module).await?)),
        Step::Settle { buy: id } => Ok(Some(!transit_open(io, buy(id)?.module).await?)),
        Step::GiveBack { buy: id } => {
            let b = buy(id)?;
            if b.is_sweep() { Ok(None) } else { Ok(Some(!transit_open(io, b.module).await?)) }
        }
        Step::Mint { buy: id } => {
            let b = buy(id)?;
            let BPhase::Minting { token, .. } = b.phase else { return Ok(Some(false)) };
            Ok(Some(wrapper_holds(io, b.wrapper, &token, None).await?))
        }
        Step::MintDeposit { tx } => {
            let Some(d) = st.deposits.iter().find(|d| d.tx == *tx && d.phase == XPhase::Minting).cloned() else { return Ok(Some(false)) };
            let c = cfg.col(&d.collection).ok_or("the deposit's collection is no longer configured")?;
            Ok(Some(wrapper_holds(io, c.wrapper, &d.token, Some(tx)).await?))
        }
        Step::Record { wrapper, ticket } => {
            let r = io.home_read(*wrapper, &en::call_word(en::TICKET, crate::abi::uint_word(*ticket))).await?;
            Ok(Some(en::decode_ticket(&r)?.stage == Stage::Delivered))
        }
        Step::Sent { wrapper, ticket } => {
            let tx = st.deliveries.iter().find(|d| d.wrapper == *wrapper && d.ticket == *ticket).and_then(|d| match d.phase {
                DPhase::Record { tx } | DPhase::Recording { tx } | DPhase::Recorded { tx } => Some(tx),
                _ => None,
            });
            let got: Word = crate::abi::word_at(&io.home_read(*wrapper, &en::call_word(en::SENT_TX, crate::abi::uint_word(*ticket))).await?, 0, 0)?
                .try_into().expect("32");
            Ok(Some(got != [0; 32] && tx.is_none_or(|t| t == got)))
        }
        Step::Mark { wrapper, ticket } => {
            let r = io.home_read(*wrapper, &en::call_word(en::UNDELIVERABLE, crate::abi::uint_word(*ticket))).await?;
            Ok(Some(crate::abi::uint_at(&r, 0, 0, 8)? != 0))
        }
        Step::Fill { collection, coin } => {
            let c = cfg.col(collection).ok_or("the fill's collection is no longer configured")?;
            let r = io.home_read(c.module, &dm::call_addr(dm::IS_FILLED, coin)).await?;
            Ok(Some(crate::abi::uint_at(&r, 0, 0, 8)? != 0))
        }
        Step::Served { id, .. } => {
            let desk = cfg.dormancy.as_ref().and_then(|p| p.desk().ok().flatten()).ok_or("no wake desk is configured")?;
            let r = io.home_read(desk, &en::call_word(dm::SERVED_AT, crate::abi::uint_word(*id as u128))).await?;
            Ok(Some(crate::abi::uint_at(&r, 0, 0, 64)? != 0))
        }
        Step::Deposit { .. } | Step::Post { .. } => Ok(None),
    }
}

async fn mined_home<I: EthIo>(io: &mut I, st: &mut EthState, cfg: &EthNftConfig, step: &Step, ok: bool) -> String {
    let chain = cfg.chain_id;
    match step.clone() {
        Step::Advance { buy } => {
            let Some(b) = st.buy(buy).cloned() else { return "an unknown buy".into() };
            if !ok {
                let _ = io.with_caps(|_, bk| custodycap::settle(bk, &format!("eth:{buy}"), None));
                let x = st.buy_mut(buy).expect("found");
                x.phase = BPhase::NeedAdvance;
                x.advanced = 0;
                return "the advance reverted: nothing was paid".into();
            }
            let wei = b.advanced.saturating_sub(keep_of(io));
            let ask = ask_for(io, wei).await.ok();
            let x = st.buy_mut(buy).expect("found");
            x.phase = BPhase::Funded { wei, since: io.now() };
            x.ask = ask;
            format!("the module advanced {} {}: the runner is asked for a Relay quote to Ethereum", b.advanced, unit_of(io))
        }
        Step::Deposit { buy } => {
            let Some(b) = st.buy(buy).cloned() else { return "an unknown buy".into() };
            let BPhase::Depositing { order_id, value, min_out, deadline } = b.phase else { return "not depositing".into() };
            if !ok {
                let ask = ask_for(io, value).await.ok();
                let x = st.buy_mut(buy).expect("found");
                x.phase = BPhase::Funded { wei: value, since: io.now() };
                x.ask = ask;
                x.last = None;
                return "the deposit reverted: the ETH is still ours; a new quote is asked".into();
            }
            st.buy_mut(buy).expect("found").phase = BPhase::Bridging { order_id, value, min_out, deadline, since: io.now() };
            format!("deposited with Relay: order 0x{}, at least {min_out} wei to our Ethereum address", hex::encode(order_id))
        }
        Step::GiveBack { buy } => {
            if !ok {
                return "returnAdvance reverted: sent again next tick".into();
            }
            let _ = io.with_caps(|_, bk| custodycap::settle(bk, &format!("eth:{buy}"), None));
            let now = io.now();
            let Some(x) = st.buy_mut(buy) else { return "an unknown buy".into() };
            let BPhase::Returning { wei } = x.phase else { return "not returning".into() };
            x.phase = BPhase::Returned { wei };
            x.ended_at = Some(now);
            if x.is_sweep() {
                return format!("{wei} wei of leftover paid into the module's surplus (fundReserve)");
            }
            format!("{wei} wei returned to the module: nothing bought")
        }
        Step::Mint { buy } => {
            let Some(b) = st.buy(buy).cloned() else { return "an unknown buy".into() };
            let BPhase::Minting { arrived, token, total, tx } = b.phase else { return "not minting".into() };
            // minted, or already minted (the wrapper's itemOf says so; on the Solana home its HeldMarker)
            let minted = ok || if io.sol_home().is_some() { wrapper_holds(io, b.wrapper, &token, None).await.unwrap_or(false) } else {
                io.home_read(b.wrapper, &en::call_word(en::ITEM_OF, token)).await
                    .and_then(|r| crate::abi::uint_at(&r, 0, 0, 128)).is_ok_and(|x| x != 0)
            };
            let x = st.buy_mut(buy).expect("found");
            if minted {
                x.phase = BPhase::Minted { arrived, token, total, tx };
                let held = held_id(chain, &b.collection, &token);
                if let Err(e) = io.with_caps(|_, bk| custodycap::settle(bk, &format!("eth:{buy}"), Some(&held))) {
                    return format!("minted to the module; the custody book: {e}");
                }
                "minted to the module: settleAdvance is next".into()
            } else {
                x.phase = BPhase::Final { arrived, token, total, tx };
                "the mint reverted: tried again (an estimate guards it)".into()
            }
        }
        Step::Settle { buy } => {
            let Some(x) = st.buy_mut(buy) else { return "an unknown buy".into() };
            let BPhase::Settling { token, total, tx, cost } = x.phase.clone() else { return "not settling".into() };
            if ok {
                x.phase = BPhase::Settled { token, total, tx, cost };
                format!("settled at {cost} wei: the unit is item-backed")
            } else {
                // round 4: a Settling buy with no home transaction out is signed again by home_act (an estimate guards it)
                "settleAdvance reverted (or was dropped): signed again next tick (an estimate guards it)".into()
            }
        }
        Step::Record { wrapper, ticket } => {
            let Some(d) = st.delivery_mut(&wrapper, ticket) else { return "an unknown delivery".into() };
            let DPhase::Recording { tx } = d.phase.clone() else { return "not recording".into() };
            if ok {
                d.phase = DPhase::Recorded { tx };
                let (c, t) = (d.collection, d.token);
                let _ = io.with_caps(|_, bk| custodycap::release(bk, &held_id(chain, &c, &t)));
                "recorded: the delivery is final on both chains".into()
            } else {
                d.phase = DPhase::Record { tx };
                "recordDelivery reverted: read again next tick".into()
            }
        }
        Step::Mark { wrapper, ticket } => {
            let Some(d) = st.delivery_mut(&wrapper, ticket) else { return "an unknown delivery".into() };
            let DPhase::Marking { why } = d.phase.clone() else { return "not marking".into() };
            d.phase = if ok { DPhase::Marked { why } } else { DPhase::Mark { why } };
            if ok { "marked undeliverable: the ticket refunds after the grace".into() } else { "the mark reverted: again next tick".into() }
        }
        Step::Post { collection } => {
            if !ok {
                return "postFloor reverted (the module's keeper, a pause?)".into();
            }
            // docs/v7/dormant-collections.md §2.3: a WAKING collection's first confirmed post mined: it is ACTIVE
            let now = io.now();
            // dormant-review DR-L-3: any mined post makes the module one a launch can use (the watchdog reads it from then on), even
            // one that mined after its wake timed out; and it restarts the TTL margin's clock (DR-M-3)
            if let Some(l) = st.dorm.as_mut().and_then(|d| d.life_mut(&collection)) {
                l.ever_active = true;
                l.posted_at = l.posted_at.max(now);
                // DI-M-1: a post after the collection's desk request, inside its window, serves it: recorded on the desk next
                if let Some(a) = l.desk.as_mut().filter(|a| a.served.is_none() && now >= a.at && now < a.until) {
                    a.served = Some(now);
                }
            }
            let woke = st.dorm.as_mut().is_some_and(|d| d.life(&collection).is_some_and(|l| matches!(l.phase, dm::Phase::Waking { backfilled_at: Some(_), .. }))
                && d.activate(&collection, now));
            if woke { "floor posted: the collection is ACTIVE (its first confirmed post after the 7-day backfill)".into() } else { "floor posted".into() }
        }
        Step::Fill { collection, coin } => {
            if let Some(l) = st.dorm.as_mut().and_then(|d| d.life_mut(&collection)) {
                l.note = if ok { format!("filled the graduation of 0x{} queued while the module was stale", hex::encode(coin)) }
                    else { format!("fill(0x{}) reverted: the hunt reads it again", hex::encode(coin)) };
            }
            if ok { format!("fill(0x{}): the queued graduation is converted", hex::encode(coin)) } else { format!("fill(0x{}) reverted", hex::encode(coin)) }
        }
        Step::Served { collection, id } => {
            // a revert (or a step taken back) is tried again by mark_act while the window lasts: the desk's servedAt, requestAt and the
            // module's latest post say whether it still can be (D2-L-2: an estimate alone is never taken as the answer)
            if ok {
                if let Some(a) = desk_ask_mut(st, &collection, id) {
                    a.marked = true;
                }
                format!("markServed({id}): the desk holds the post that served the request inside its window")
            } else {
                format!("markServed({id}) did not take: read again at the next home act while the request's window lasts")
            }
        }
        Step::Sent { wrapper, ticket } => {
            let moved_on = !ok && !still_resolved(io, wrapper, ticket).await.unwrap_or(true);
            let Some(d) = st.delivery_mut(&wrapper, ticket) else { return "an unknown delivery".into() };
            if ok || moved_on {
                d.sent = Some(true);
            }
            if ok { "noted sent: the wrapper's late window no longer refunds it".into() }
            else if moved_on { "noteSent reverted: the ticket is no longer Resolved (recorded first): nothing to note".into() }
            else { "noteSent reverted: sent again next tick".into() }
        }
        Step::MintDeposit { tx } => {
            let Some(d) = st.deposits.iter().find(|d| d.tx == tx && d.phase == XPhase::Minting).cloned() else { return "an unknown deposit".into() };
            // minted, or already minted from this very transaction (the wrapper's item says so)
            let minted = ok || match cfg.col(&d.collection) {
                Some(c) => match io.home_read(c.wrapper, &en::call_word(en::ITEM_OF, d.token)).await.and_then(|r| crate::abi::uint_at(&r, 0, 0, 128)) {
                    Ok(n) if n != 0 => io.home_read(c.wrapper, &en::call_word(en::ITEM, crate::abi::uint_word(n - 1))).await
                        .and_then(|r| en::decode_item(&r)).is_ok_and(|(tok, src, dep)| tok == d.token && src == d.tx && dep),
                    _ => false,
                },
                None => false,
            };
            let x = st.deposits.iter_mut().find(|x| x.tx == tx && x.phase == XPhase::Minting).expect("found");
            x.phase = if minted { XPhase::Minted } else { XPhase::Mint };
            if minted { "the deposit is minted to its recipient".into() } else { "mintDeposit reverted: again next tick (an estimate guards it)".into() }
        }
    }
}

// ---- endpoints' pure parts -----------------------------------------------------------------------------------------------------

/// `eth_give_back`: a funded, arrived-less or stuck buy's ETH on the home chain goes back to the module. Round 2 (A2-M-1 / C2-M-2):
/// an ARRIVED buy starts its way back at once (Relay Ethereum → 4663, then the module; its deposit waits for the Relay leg).
/// `FloorModule.fundReserve()`: ETH paid in joins the module's reserve surplus (after any bootstrap loan, none in Phase 1), which
/// tops up its conversions and never pays a burn.
pub const FUND_RESERVE: [u8; 4] = [0x6f, 0x9c, 0x3c, 0x8f];

pub fn fund_reserve_call() -> Vec<u8> {
    FUND_RESERVE.to_vec()
}

/// A2-L-1, part 2 (the operator, 2026-09-27): the gas-pot sweep. While nothing at our Ethereum address is anyone's (no open buy holds
/// or awaits ETH there, nothing is on the Ethereum line, no delivery is open, no stray is going back) and the balance is over the
/// target (`eth_nft.gas_pot_target_wei`, 0.05 ETH by default), the excess, up to the leftover of the collection with the most,
/// goes back to that collection's module: a Relay order Ethereum → 4663 judged like the way back (`relayeth::judge`,
/// `FROM_ETHEREUM`), then `fundReserve`. A sweep waiting too long for its quote is abandoned and its leftover credited back.
async fn sweep_pass<I: EthIo>(io: &mut I, st: &mut EthState, cfg: &EthNftConfig, cols: &[Col]) -> Option<String> {
    let now = io.now();
    // a sweep with no quote for too long ends (it blocks new buys while it waits)
    let late: Vec<(u64, [u8; 20], u128)> = st.buys.iter().filter(|b| b.is_sweep())
        .filter_map(|b| match b.phase { BPhase::BackFunded { wei, since } if now.saturating_sub(since) >= SWEEP_QUOTE_SECS => Some((b.id, b.collection, wei)), _ => None })
        .collect();
    if let Some((id, c, wei)) = late.first().copied() {
        let x = st.buy_mut(id).expect("open");
        x.phase = BPhase::Returned { wei: 0 };
        x.ask = None;
        st.inbox.quotes.retain(|(q, ..)| *q != id);
        st.credit_leftover(&c, wei);
        st.sweep_after = Some(now.saturating_add(SWEEP_COOLDOWN_SECS));
        return Some(format!("eth sweep {id}: no quote for {SWEEP_QUOTE_SECS} s: abandoned, {wei} wei is 0x{}'s leftover again", hex::encode(c)));
    }
    let leg_back = if io.sol_home().is_some() { re::to_solana(cfg.chain_id).is_some() } else { cfg.chain_id == re::FROM_ETHEREUM.origin_chain };
    if now < st.sweep_after.unwrap_or(0) || st.line.is_some() || st.halted.is_some() || !leg_back {
        return None;
    }
    if st.late_hold().is_some() || st.buys.iter().any(|b| !b.finished() && (b.is_sweep() || b.in_relay() || b.earmark() > 0
        || matches!(b.phase, BPhase::Arrived { .. })))
        || st.deliveries.iter().any(|d| !d.finished())
        || st.deposits.iter().any(|d| matches!(d.phase, XPhase::Return { .. } | XPhase::Returning { .. })) {
        return None;
    }
    let (c, left) = cols.iter().map(|c| (*c, st.leftover_of(&c.collection))).max_by_key(|(_, l)| *l)?;
    if left < SWEEP_MIN_WEI {
        return None;
    }
    if let Err(e) = eth_floor(io, st, None).await {
        return Some(format!("eth sweep: waits: {e}"));
    }
    let bal = match io.eth_balance().await {
        Ok(b) => b,
        Err(e) => return Some(format!("eth sweep: our Ethereum balance not read ({e})")),
    };
    let wei = left.min(bal.saturating_sub(cfg.gas_pot_target()));
    if wei < SWEEP_MIN_WEI {
        return None;
    }
    let id = st.next_buy;
    st.next_buy += 1;
    st.buys.push(EthBuy { id, collection: c.collection, module: c.module, wrapper: c.wrapper, advanced: 0, attempts: 0,
        phase: BPhase::BackFunded { wei, since: 0 }, orders: vec![], deposits: vec![], ask: None, last: None, note: String::new(), sweep: Some(true),
        arrived_at: None, back_at: None, ended_at: None, late: None });
    st.debit_leftover(&c.collection, wei);
    match back_now(io, st, id, wei).await {
        Ok(_) => Some(format!("eth sweep {id}: {wei} wei of 0x{}'s leftover (our Ethereum balance {bal}, target {}) goes back to its module             through Relay, then fundReserve (the runner is asked for a quote)", hex::encode(c.collection), cfg.gas_pot_target())),
        Err(e) => {
            st.buys.retain(|b| b.id != id);
            st.credit_leftover(&c.collection, wei);
            Some(format!("eth sweep: not started: {e}"))
        }
    }
}

/// `eth_give_back`, the operator's exits, each of which spends nothing that is not the buy's own (round 3, C3-M-1 / E3-L-3 / C3-L-2):
///
/// - `Funded` (its ETH on 4663): back to the module now (`returnAdvance`), at most what the home balance holds beside the gas keep;
/// - a `Stuck` buy still watched (the forward leg past its deadline): the same;
/// - `Arrived` (its ETH on Ethereum), once it has been there `ARRIVED_HOLD_SECS`: its way back starts now (C3-L-2: never at once,
///   so a runner holding the operator key cannot churn a module's ETH through Relay);
/// - `BackFunded` (a way back with no deposit signed): it stops and waits at `Arrived` again (earmarked as before; nothing was
///   signed), trying again `BACK_RETRY_SECS` later; other collections buy meanwhile;
/// - `BackBridging` a day past its order's deadline and the grace: written off (`Stuck`, no longer watched, its order kept as
///   `late`); review round 5 (C5-M-1): it still holds the Relay leg for new orders until a payout of it is credited or the operator
///   releases it (`late_release`);
/// - a `Stuck` buy no longer watched (a fill whose token is not ours, or a way back no longer watched: the ETH is not ours to find):
///   closed as a loss: `returnAdvance` of 0 ends the module's transit and the module books the loss (a sweep ends with nothing).
///   The operator does this once Relay (or the chain) says nothing more will come.
pub fn give_back_now(st: &mut EthState, buy: u64, balance: u128, now: u64) -> Result<String, String> {
    give_back_now_keeping(st, buy, balance, now, GAS_KEEP_WEI)
}

/// [`give_back_now`] with the home's own gas keep (`keep`: `GAS_KEEP_WEI` on the EVM home, `solhome::SOL_GAS_KEEP_LAMPORTS` on Solana,
/// whose `balance` is lamports).
pub fn give_back_now_keeping(st: &mut EthState, buy: u64, balance: u128, now: u64, keep: u128) -> Result<String, String> {
    // audit A5-I3: the home's unit for the home-chain amounts below (the keep is the home's own: lamports on the Solana home)
    let unit = if keep == crate::solhome::SOL_GAS_KEEP_LAMPORTS { "lamports" } else { "wei" };
    let b = st.buy_mut(buy).ok_or_else(|| format!("no buy {buy}"))?;
    let sweep = b.is_sweep();
    match b.phase.clone() {
        BPhase::Arrived { wei, since } if !sweep => {
            let first = b.arrived_at.unwrap_or(since).min(since);
            if now.saturating_sub(first) < ARRIVED_HOLD_SECS {
                return Err(format!("refused: buy {buy} arrived {} s ago; it may be sent back once it has waited {ARRIVED_HOLD_SECS} s for a \
                    listing (C3-L-2)", now.saturating_sub(first)));
            }
            if wei <= en::ETH_GAS_RESERVE_WEI {
                return Err(format!("refused: buy {buy} holds {wei} wei, under the way back's gas reserve: nothing to send back"));
            }
            b.phase = BPhase::BackFunded { wei, since: now };
            b.ask = None;
            Ok(format!("buy {buy}: its {wei} wei goes back: a Relay quote Ethereum → 4663 is asked at the next tick, then the module"))
        }
        BPhase::BackFunded { wei, .. } if !sweep => {
            b.phase = BPhase::Arrived { wei, since: now };
            b.ask = None;
            b.back_at = Some(now.saturating_add(BACK_RETRY_SECS));
            st.inbox.quotes.retain(|(q, ..)| *q != buy);
            Ok(format!("buy {buy}: its way back is stopped before any deposit: its {wei} wei waits at our Ethereum address for a listing \
                again, and goes back again in {BACK_RETRY_SECS} s"))
        }
        BPhase::BackBridging { order_id, deadline, .. } => {
            // round 4 (C4-M-1): written off only once the canister itself has watched it a day past the deadline and the grace (Relay
            // refunds after the deadline): a payout that lands after the write-off is credited to it by its transaction. Review round 5
            // (C5-M-1): it keeps the Relay leg's attribution (`late_hold`) until credited or released, so no other order's watch is
            // open to read that payout as its own
            let at = deadline.saturating_add(DEADLINE_GRACE_SECS).saturating_add(BACK_LATE_SECS);
            if now <= at {
                return Err(format!("refused: buy {buy}'s way back (order 0x{}) is watched until {at} (its deadline {deadline}, the grace and a \
                    day): a payout until then is read as its own", hex::encode(order_id)));
            }
            let w = b.last.take().ok_or_else(|| format!("refused: buy {buy}'s way back has no watch"))?;
            b.phase = BPhase::Stuck { why: format!("its way back (order 0x{}) is past its deadline with no arrival on 4663 and no refund on \
                Ethereum: written off by the operator; eth_give_back({buy}) closes it as a loss, eth_credit_payout credits a late payout of it", hex::encode(order_id)) };
            b.late = Some(LateOrder { watch: w, back: true, let_go_at: now, credited: None, released_at: None });
            Ok(format!("buy {buy}: its way back is written off, no longer watched; it still holds the Relay leg for new orders (C5-M-1) until a \
                payout of it is credited (eth_credit_payout) or the operator releases it (eth_late_release); eth_give_back({buy}) again closes \
                it as a loss"))
        }
        BPhase::Stuck { .. } if b.last.is_none() => {
            if sweep {
                b.phase = BPhase::Returned { wei: 0 };
                st.sweep_after = Some(now.saturating_add(SWEEP_COOLDOWN_SECS));
                return Ok(format!("sweep {buy}: closed; its leftover is written off"));
            }
            b.phase = BPhase::Returning { wei: 0 };
            b.ask = None;
            Ok(format!("buy {buy}: closed as a loss: returnAdvance of 0 at the next tick ends its module's transit (the module books the loss)"))
        }
        _ if sweep => Err(format!("refused: {buy} is a gas-pot sweep; only a sweep stuck and no longer watched is closed by hand")),
        BPhase::Funded { wei, .. } => {
            let wei = wei.min(balance.saturating_sub(keep));
            if wei == 0 {
                return Err("refused: nothing to give back".into());
            }
            b.phase = BPhase::Returning { wei };
            b.ask = None;
            Ok(format!("buy {buy}: {wei} {unit} goes back to the module at the next tick"))
        }
        BPhase::Stuck { .. } => {
            // a forward order still watched past its deadline (round 4, C4-M-1): only what the watch measures as this order's refund on
            // 4663 goes back (the rise over the watch's start at a pin never below one read), never the raw balance (other ETH at our
            // address: a late payout of another order, the frog side's, the gas pot)
            let w = b.last.clone().ok_or("refused: nothing is watched")?;
            if crate::relayconv::refunded(balance, w.balance_after, w.value) {
                let wei = balance.saturating_sub(w.balance_after).min(w.value).min(b.advanced);
                b.phase = BPhase::Returning { wei };
                b.ask = None;
                b.last = None;
                return Ok(format!("buy {buy}: its order's refund, {wei} {unit}, goes back to the module at the next tick"));
            }
            let at = w.deadline.saturating_add(DEADLINE_GRACE_SECS).saturating_add(BACK_LATE_SECS);
            if now <= at {
                return Err(format!("refused: buy {buy}'s order 0x{} shows no refund at our 4663 address and is watched until {at}: a late fill \
                    or refund until then is read as its own", hex::encode(w.order_id)));
            }
            b.last = None;
            b.late = Some(LateOrder { watch: w.clone(), back: false, let_go_at: now, credited: None, released_at: None });
            b.phase = BPhase::Stuck { why: format!("its order 0x{} is past its deadline with no arrival on Ethereum and no refund on 4663: \
                written off by the operator; eth_give_back({buy}) closes it as a loss, eth_credit_payout credits a late payout of it",
                hex::encode(w.order_id)) };
            Ok(format!("buy {buy}: its order shows no payout: written off, no longer watched; it still holds the Relay leg for new orders \
                (C5-M-1) until a payout of it is credited (eth_credit_payout) or the operator releases it (eth_late_release); eth_give_back({buy}) \
                again closes it as a loss"))
        }
        p => Err(format!("refused: buy {buy} is {p:?}; only a funded, arrived, going-back or stuck buy has an exit here")),
    }
}

/// Review round 4 (C4-M-1): the operator hands in the transaction that paid out a written-off order late (Relay's status names it:
/// the fill on the destination, or the refund), on 4663 (`home`) or Ethereum. `wei`: what it paid, only for a payout the transaction
/// itself does not show (a refund paid from inside the depository's call); 0 otherwise. Judged at the next tick against THAT order
/// (`judge_payout`), and credited to its own buy and collection; never to another.
/// Phase B2: `eth_credit_payout` on the Solana home, for a payout ON Solana (a written-off order's late refund of the forward leg, or its
/// late fill of the way back): the transaction's base58 signature, keyed by its keccak in the credited list; judged at the next tick by
/// `judge_sol_payout`. `lamports`: what Relay's records give (0: the transaction's own credit to us).
pub fn put_sol_payout(st: &mut EthState, buy: u64, signature: &str, lamports: u128, now: u64) -> Result<String, String> {
    let sig = crate::soltx::signature(signature.trim()).map_err(|e| format!("refused: {signature:?} is not a Solana signature: {e}"))?;
    let key = crate::evmtx::keccak(&sig);
    let r = put_payout(st, buy, true, key, lamports, now)?;
    let v = st.sol_payouts.get_or_insert_with(Vec::new);
    v.retain(|(k, _)| *k != key);
    v.push((key, signature.trim().to_string()));
    if v.len() > 8 {
        v.remove(0);
    }
    Ok(r)
}

/// Phase B2: whether a finalized Solana transaction is a payout of the written-off order `o` (the Solana form of [`judge_payout`]; spike
/// Q5 §5.2–5.3: Relay's solver pays fills and refunds from its own wallet as plain transfers, memoed `0x<order id>`, a fill also its
/// request id): successful, never credited before (`key`, the signature's keccak); a System transfer to our address from another
/// account; a memo naming the order (or Relay's request id for it); the amount the lamports it credited us (the operator's statement, when
/// given, must be that); at most what the order sent and at least half of it. The memo is a label anyone can write: the half-the-order
/// floor makes a forged one cost at least that much, and paying us cannot hurt us (Q5 §6.3).
pub fn judge_sol_payout(o: &LateOrder, tx: &serde_json::Value, key: &[u8; 32], ours: &str, named: u128, credited: &[[u8; 32]]) -> Result<u128, String> {
    if credited.contains(key) {
        return Err("this transaction was credited already".into());
    }
    if !tx["meta"]["err"].is_null() {
        return Err("the transaction failed".into());
    }
    let w = &o.watch;
    let ids: Vec<String> = std::iter::once(w.order_id).chain(w.request_id.filter(|r| r[..12] != [0u8; 12]))
        .map(|i| format!("0x{}", hex::encode(i))).collect();
    let memos = crate::relaysol::memos(tx)?;
    if !memos.iter().any(|m| ids.iter().any(|i| m.eq_ignore_ascii_case(i))) {
        return Err(format!("the transaction's memos {memos:?} do not name order 0x{} (nor its request id)", hex::encode(w.order_id)));
    }
    let to_us: u64 = crate::relaysol::system_transfers_to(tx, ours)?.iter().filter(|(from, _)| from != ours).map(|(_, l)| *l).sum();
    let delta = crate::relaysol::lamport_delta(tx, ours).unwrap_or(0);
    if to_us == 0 || delta <= 0 {
        return Err("the transaction pays our Solana address nothing (from another account)".into());
    }
    let paid = (to_us as u128).min(delta as u128);
    if named != 0 && named != paid {
        return Err(format!("the transaction credited us {paid} lamports, not the {named} stated"));
    }
    // audit A5-01: a payout on Solana is in lamports: the way back's (ETH → SOL) is in its destination's unit, judged against its
    // min out; the forward order's (SOL → ETH) refund is in its own, judged against what it sent
    payout_in_bounds(w, paid, o.back, "lamports")?;
    Ok(paid)
}

/// Audit A5-01: the bounds a written-off order's payout is held to, in the payout's own unit (`unit`, for the texts).
///
/// - On the order's ORIGIN chain (a refund, or any payout of the EVM home, whose legs are all wei) the bound is the order's `value`:
///   at most what it sent and at least `REFUND_SHARE_BPS` (half) of it, as before.
/// - On the Solana home, on the order's DESTINATION chain (`dest`: a forward SOL → ETH order paid in wei on Ethereum, a way back
///   ETH → SOL paid in lamports on Solana), `value` is in the other chain's unit, so the bounds are the order's own `min_out`, which is
///   in the destination's unit already: at least half of `min_out`, and at most `min_out × BPS / (BPS − relayeth::MAX_FEE_BPS)`,
///   grossed up by `FX_TOLERANCE_BPS`. Every order was judged with `min_out ≥ value at our rate × (1 − relay_max_fee_bps)` and
///   `relay_max_fee_bps ≤ relayeth::MAX_FEE_BPS`, so that cap is at least the order's value in the destination's unit (the most "the
///   order sent" can be there), plus the rate tolerance between the judgement and the payout. The floor is at most half the order's
///   value there (`min_out` is never more than the value it was cut from).
fn payout_in_bounds(w: &Watch, paid: u128, dest: bool, unit: &str) -> Result<(), String> {
    use crate::relayconv::{BPS, REFUND_SHARE_BPS};
    if dest {
        let fee = crate::relayeth::MAX_FEE_BPS as u128;
        let tol = crate::solhome::FX_TOLERANCE_BPS;
        let cap = w.min_out.saturating_mul(BPS) / (BPS - fee) * (BPS + tol) / BPS;
        if paid > cap {
            return Err(format!("{paid} {unit} is more than the order sent (at most {cap} {unit} for its minimum out of {}, at {fee} bps \
                fee and {tol} bps rate tolerance)", w.min_out));
        }
        if paid.saturating_mul(BPS) < w.min_out.saturating_mul(REFUND_SHARE_BPS) {
            return Err(format!("{paid} {unit} is under half the order's minimum out of {} (no fill or refund pays that little)", w.min_out));
        }
        return Ok(());
    }
    if paid > w.value {
        return Err(format!("{paid} {unit} is more than the order sent ({})", w.value));
    }
    if paid.saturating_mul(BPS) < w.value.saturating_mul(REFUND_SHARE_BPS) {
        return Err(format!("{paid} {unit} is under half the order's {} (no fill or refund pays that little)", w.value));
    }
    Ok(())
}

pub fn put_payout(st: &mut EthState, buy: u64, home: bool, tx: [u8; 32], wei: u128, now: u64) -> Result<String, String> {
    let b = st.buy(buy).ok_or_else(|| format!("no buy {buy}"))?;
    let o = b.late.as_ref().ok_or_else(|| format!("refused: buy {buy} has no written-off order (a watched order's payout is read by its watch)"))?;
    if o.credited.is_some() {
        return Err(format!("refused: buy {buy}'s written-off order was credited already"));
    }
    if st.credited.iter().flatten().any(|x| *x == tx) {
        return Err("refused: this transaction was credited already".into());
    }
    let v = st.inbox.payouts.get_or_insert_with(Vec::new);
    v.retain(|p| p.buy != buy);
    if v.len() >= 8 {
        return Err("refused: 8 payouts wait already".into());
    }
    v.push(PayoutClaim { buy, home, tx, wei, at: now });
    Ok(format!("payout held for buy {buy}; judged at the next tick against its order"))
}

/// Round 4 (C4-M-1): whether `t` (read at its chain's pin) is a payout of the written-off order `o`, and what it paid us. Rules
/// (review round 5, C5-L-1: tied to THIS order, never by Relay's solver alone, which is every order's):
/// 1. mined and successful, never credited before, and not ours (our own `depositNative` of the order names it too);
/// 2. it names the order: its input carries the order id or Relay's request id for it (Relay's fill carries it in its calldata);
/// 3. it mined after the order was deposited (above the pin the watch was taken at, on that chain);
/// 4. it pays us: a plain transfer to our address (the value is what it paid), or a call to Relay's depository naming the order whose
///    payment the operator states (`named`: a refund paid from inside the depository), never a deposit into it, never more than the
///    order sent;
/// 5. at least half the order's value (Relay's refund share, the least either payout can be; a fill or a refund may land on either
///    chain, so no per-chain minimum is held: C5-I-2).
///
/// Review round 6 (C6-L-1): a call to Relay's depository is credited only by Relay's own execution record in its receipt: the
/// depository's `RelayCallExecuted` (emitted only by its allocator-signed `execute`, never by a view function called with trailing
/// bytes, a deposit, or anyone else's contract), paying OUR address; the amount is that record's, never the operator's statement (a
/// statement that differs is refused). A request id that looks like an address or a small number (twelve leading zero bytes) names
/// nothing.
///
/// Review round 7 (C7-L-1): the record's id is the EIP-712 struct hash of the `CallRequest` its `execute` ran, never an order id. A
/// call to the depository is credited only when its input decodes as that `execute`, and only the records whose id is the recomputed
/// hash of THAT request, paying us, count (C7-I-1). A real `execute` carries nothing of the order, so it is named by the operator from
/// Relay's records: the stated amount must be exactly one of those records (pending the C5-I-1 measurement of a real refund).
#[allow(clippy::too_many_arguments)]
///
/// Audit A5-01: `sol_home`, the home is Solana: a forward order's `value` is lamports while its fill here is wei, so that payout is
/// judged against the order's `min_out` instead ([`payout_in_bounds`]).
pub fn judge_payout(o: &LateOrder, t: &PayoutTx, logs: &[en::LogV], hash: &[u8; 32], home: bool, us: &[u8; 20], named: u128,
    credited: &[[u8; 32]], sol_home: bool) -> Result<u128, String> {
    if credited.contains(hash) {
        return Err("this transaction was credited already".into());
    }
    if !t.success {
        return Err("the transaction reverted".into());
    }
    if t.from == *us {
        return Err("the transaction is ours (our own deposit names the order too): never a payout".into());
    }
    let w = &o.watch;
    let ids: Vec<[u8; 32]> = std::iter::once(w.order_id).chain(w.request_id.filter(|r| r[..12] != [0u8; 12])).collect();
    // review round 7 (C7-L-1): a depository payout is its allocator-signed `execute`: the records credited are the depository's own
    // `RelayCallExecuted` whose id is the EIP-712 struct hash of THIS transaction's decoded request (the id is never an order id), so
    // only this request's calls count (C7-I-1)
    let depository = t.to == crate::relayconv::RELAY_DEPOSITORY_4663 && t.input.get(..4) != Some(&crate::relayconv::DEPOSIT_NATIVE[..]);
    let request = if depository { crate::relayconv::decode_execute(&t.input) } else { None };
    let execs: Vec<([u8; 32], [u8; 20], u128)> = match &request {
        Some(r) => {
            let sh = crate::relayconv::call_request_hash(r);
            logs.iter().filter(|l| l.address == crate::relayconv::RELAY_DEPOSITORY_4663).filter_map(crate::relayconv::call_executed)
                .filter(|(id, ..)| *id == sh).collect()
        }
        None => vec![],
    };
    let in_input = t.input.windows(32).any(|x| ids.iter().any(|i| x == i));
    // review round 7 (C7-L-1): a real depository execute carries only calls, the allocator's nonce, the expiry and the signature, so it
    // cannot name the order on chain: it is named by the operator from Relay's own records for the order (its status names this
    // transaction), stating the amount, which must be exactly one of this request's records paying us. PENDING the operator's C5-I-1
    // measurement of a real Relay refund
    let names = in_input || (request.is_some() && named != 0);
    if !names {
        return Err(format!("the transaction does not name order 0x{} (neither the order id nor Relay's request id is in its input; Relay's \
            solver alone is every order's sender; a depository execute is named by the operator stating the amount Relay's records give for \
            this order)", hex::encode(w.order_id)));
    }
    if let Some(start) = if home { w.home_pin } else { w.pin } {
        if t.block <= start {
            return Err(format!("the transaction mined in block {}, not after the order was deposited (block {start})", t.block));
        }
    }
    let paid = if t.to == *us && t.value > 0 {
        if named != 0 && named != t.value {
            return Err(format!("the transaction paid us {} wei, not the {named} stated", t.value));
        }
        t.value
    } else if depository {
        if request.is_none() {
            return Err("a call to Relay's depository that is not its execute(CallRequest, signature): not a payout (C6-L-1 / C7-L-1)".into());
        }
        let to_us: Vec<u128> = execs.iter().filter(|(_, to, v)| to == us && *v > 0).map(|(.., v)| *v).collect();
        if to_us.is_empty() {
            return Err("a call to Relay's depository carries no RelayCallExecuted of this request (its id the request's struct hash) paying \
                our address: a call that only names the order is not a payout (C6-L-1 / C7-L-1)".into());
        }
        if named != 0 {
            if !to_us.contains(&named) {
                return Err(format!("none of this request's execution records paying us is the {named} wei stated ({to_us:?})"));
            }
            named
        } else if to_us.len() == 1 {
            to_us[0]
        } else {
            return Err(format!("this request pays us {} times ({to_us:?}): state which amount is this order's (C7-I-1)", to_us.len()));
        }
    } else {
        return Err("the transaction is neither a payment to our address nor a call to Relay's depository (not a deposit) with its amount \
            stated".into());
    };
    // audit A5-01: on the Solana home a payout here (on Ethereum, in wei) is the forward order's fill, in its destination's unit
    // (its `value` is lamports), or the way back's refund, in its own; on the EVM home every leg is in wei
    payout_in_bounds(w, paid, sol_home && home == o.back, "wei")?;
    Ok(paid)
}

/// Round 4 (C4-M-1): the payouts handed in, one a tick. A credited payout is its own order's: on 4663 it goes back to its module
/// (`returnAdvance` while the buy is open; `fundReserve` once the buy was closed as a loss), on Ethereum it is its buy's again (the
/// buy waits for a listing or goes back) or, once the buy was closed, its collection's leftover (swept back to its module). The ETH must
/// be at our address beside every other claim on it there. Review round 5 (C5-L-3): every read comes first; nothing is written unless
/// the whole credit applies.
async fn payout_pass<I: EthIo>(io: &mut I, st: &mut EthState, cfg: &EthNftConfig) -> Vec<String> {
    let Some(p) = st.inbox.payouts.as_mut().and_then(|v| (!v.is_empty()).then(|| v.remove(0))) else { return vec![] };
    let r: Result<String, String> = async {
        if io.now().saturating_sub(p.at) > HANDIN_TTL_SECS {
            return Err("handed in too long ago: stale".into());
        }
        let b = st.buy(p.buy).cloned().ok_or("no such buy")?;
        let o = b.late.clone().ok_or("the buy has no written-off order")?;
        if o.credited.is_some() {
            return Err("its order was credited already".into());
        }
        // review round 6 (C6-L-2): never while the buy has a 4663 transaction out (the loss-closing returnAdvance(0), a settle, a mint):
        // its phase is what that transaction resolves from, and a credit rewriting it would book a payout nobody is paid. The claim waits
        if let Some(step) = st.pending.as_ref().map(|x| x.step.clone()).or_else(|| eth_sol_step(st))
            .filter(|s| buy_of_step(s) == Some(p.buy)) {
            return Err(format!("WAIT: buy {}'s {step:?} is out on 4663: the payout is judged once it resolves (the claim is kept)", p.buy));
        }
        // phase B2: on the Solana home a payout there is a finalized transaction credited by its memo (`judge_sol_payout`)
        let paid = if p.home && io.sol_home().is_some() {
            let sig = st.sol_payouts.iter().flatten().find(|(k, _)| *k == p.tx).map(|(_, s)| s.clone())
                .ok_or("the Solana payout's signature was not handed in with it")?;
            let tx = io.sol_transaction(&sig).await?.ok_or("the transaction is not finalized on Solana")?;
            let ours = crate::soltx::b58_encode(&io.sol_me().await?);
            judge_sol_payout(&o, &tx, &p.tx, &ours, p.wei, st.credited.as_deref().unwrap_or_default())?
        } else {
            let t = io.payout_tx(p.home, p.tx).await?.ok_or("the transaction is not mined at the pin")?;
            if !p.home && t.block > io.eth_finalized().await? {
                return Err(format!("the payout mined in Ethereum block {}, not final yet", t.block));
            }
            let us = io.our_evm().await?;
            // review round 6 (C6-L-1): a call to Relay's depository is credited only by Relay's own execution record in its receipt
            let logs = if t.to == crate::relayconv::RELAY_DEPOSITORY_4663 { io.payout_logs(p.home, p.tx).await? } else { vec![] };
            judge_payout(&o, &t, &logs, &p.tx, p.home, &us, p.wei, st.credited.as_deref().unwrap_or_default(), io.sol_home().is_some())?
        };
        // the ETH is still at our address, beside everything else there that is someone's
        let eth_bal = if p.home {
            let bal = io.home_balance().await?;
            let others = st.buys.iter().filter(|x| !x.finished()).map(|x| match x.phase {
                BPhase::Funded { wei, .. } | BPhase::Returning { wei } => wei,
                _ => 0,
            }).fold(0u128, u128::saturating_add);
            if bal.saturating_sub(others) < paid {
                return Err(format!("our 4663 balance {bal} holds {} beside the open buys' ETH there, under the {paid} to credit", bal.saturating_sub(others)));
            }
            None
        } else {
            eth_floor(io, st, None).await?;
            let bal = io.eth_balance().await?;
            let left = st.leftover.iter().flatten().map(|(_, w)| *w).fold(0u128, u128::saturating_add);
            let free = bal.saturating_sub(st.earmarked_except(u64::MAX)).saturating_sub(left);
            if free < paid {
                return Err(format!("our Ethereum balance {bal} holds {free} beside the earmarks and the booked leftover, under the {paid} to credit"));
            }
            Some(bal)
        };
        let now = io.now();
        let open = !b.finished();
        // audit A5-I3: a payout on the home is in the home's unit (lamports on the Solana home)
        let on_home = if io.sol_home().is_some() { "lamports on Solana" } else { "wei on 4663" };
        // the one step that can still refuse (an arrival beside the other earmarks) goes first; only then is anything written
        let what = if b.is_sweep() && open {
            // a sweep's written-off way back: on 4663 its ETH goes on to the module's surplus; on Ethereum it is leftover again
            let c = b.collection;
            let x = st.buy_mut(p.buy).expect("found");
            if p.home {
                x.phase = BPhase::Returning { wei: paid };
                format!("{paid} {on_home} goes on to the module's surplus (fundReserve)")
            } else {
                x.phase = BPhase::Returned { wei: 0 };
                st.credit_leftover(&c, paid);
                format!("{paid} wei on Ethereum is 0x{}'s leftover again", hex::encode(c))
            }
        } else {
            match (p.home, open) {
                (true, true) => {
                    let x = st.buy_mut(p.buy).expect("found");
                    x.phase = BPhase::Returning { wei: paid.min(x.advanced) };
                    format!("{paid} {on_home} goes back to the module (returnAdvance)")
                }
                (false, true) => {
                    // review round 6 (C6-L-3): the item this ETH buys must be in the custody book. The written-off order kept its room
                    // while it was open; one released first (the operator's call) takes it again, and with no room the ETH is the
                    // collection's leftover (swept back to its module), never an item outside the book
                    let flight = format!("eth:{}", p.buy);
                    let col = custodycap::evm_key(cfg.chain_id, &b.collection);
                    let had = io.with_caps(|_, bk| bk.entries.iter().any(|e| e.id == flight && e.in_flight));
                    let room = had || io.with_caps(|cp, bk| custodycap::reserve(cp, bk, &flight, &col, b.advanced)).is_ok();
                    if room {
                        match arrive(st, p.buy, paid, paid, eth_bal.unwrap_or(0), now, "a late payout on Ethereum, credited by its transaction") {
                            Ok(x) => x,
                            Err(e) => {
                                if !had {
                                    let _ = io.with_caps(|_, bk| custodycap::settle(bk, &flight, None));
                                }
                                return Err(e);
                            }
                        }
                    } else {
                        // the buy stays as it is (its advance is closed as a loss by the operator's eth_give_back, as any written-off one)
                        let c = b.collection;
                        st.credit_leftover(&c, paid);
                        format!("{paid} wei on Ethereum is 0x{}'s leftover (swept back to its module): no custody room for the item it would buy; \
                            buy {} stays written off (eth_give_back closes it as a loss)", hex::encode(c), p.buy)
                    }
                }
                (true, false) => {
                    // the buy was closed as a loss: its module booked it; the ETH now joins that module's surplus
                    let (c, m, w) = (b.collection, b.module, b.wrapper);
                    let id = st.next_buy;
                    st.next_buy += 1;
                    st.buys.push(EthBuy { id, collection: c, module: m, wrapper: w, advanced: 0, attempts: 0, phase: BPhase::Returning { wei: paid },
                        orders: vec![], deposits: vec![], ask: None, last: None, note: format!("the late payout of buy {}", p.buy), sweep: Some(true),
                        arrived_at: None, back_at: None, ended_at: None, late: None });
                    format!("{paid} {on_home} goes to the module's surplus (fundReserve, as sweep {id}): buy {} was closed as a loss", p.buy)
                }
                (false, false) => {
                    let c = b.collection;
                    st.credit_leftover(&c, paid);
                    format!("{paid} wei on Ethereum is 0x{}'s leftover (swept back to its module): buy {} was closed as a loss", hex::encode(c), p.buy)
                }
            }
        };
        st.credited.get_or_insert_with(Vec::new).push(p.tx);
        if let Some(l) = st.buy_mut(p.buy).and_then(|x| x.late.as_mut()) {
            l.credited = Some((p.tx, paid, p.home));
        }
        Ok(what)
    }.await;
    vec![match r {
        Ok(s) => format!("eth payout {} for buy {}: CREDITED: {s}", h(&p.tx), p.buy),
        Err(e) if e.starts_with("WAIT: ") => {
            // review round 6 (C6-L-2): kept, first in line, with its hand-in time (a claim past its TTL is still refused as stale)
            let why = e.trim_start_matches("WAIT: ").to_string();
            st.inbox.payouts.get_or_insert_with(Vec::new).insert(0, p.clone());
            format!("eth payout {} for buy {}: waits, nothing credited yet: {why}", h(&p.tx), p.buy)
        }
        Err(e) => format!("eth payout {} for buy {}: refused, nothing credited: {e}", h(&p.tx), p.buy),
    }]
}

/// The buy a home step is for, if any.
fn buy_of_step(s: &Step) -> Option<u64> {
    match s {
        Step::Advance { buy } | Step::Deposit { buy } | Step::GiveBack { buy } | Step::Mint { buy } | Step::Settle { buy } => Some(*buy),
        _ => None,
    }
}

/// Review round 5 (C5-M-1): `eth_late_release`: the operator releases a written-off order's hold on the Relay leg, once Relay's own
/// records show where its ETH went (paid elsewhere, or definitively nothing). New Relay orders start again; a payout of it that lands
/// afterwards can still be credited by its transaction, but while another order is watched it may be read as that order's: the
/// operator's call, so it is never automatic.
pub fn late_release(st: &mut EthState, buy: u64, now: u64) -> Result<String, String> {
    let b = st.buy_mut(buy).ok_or_else(|| format!("no buy {buy}"))?;
    let l = b.late.as_mut().ok_or_else(|| format!("refused: buy {buy} has no written-off order"))?;
    if l.credited.is_some() {
        return Err(format!("refused: buy {buy}'s written-off order was credited: it holds nothing"));
    }
    if l.released_at.is_some() {
        return Err(format!("refused: buy {buy}'s written-off order was released already"));
    }
    l.released_at = Some(now);
    Ok(format!("buy {buy}: its written-off order no longer holds the Relay leg; new orders start at the next tick (a payout of it that lands \
        later can still be credited with eth_credit_payout)"))
}

/// Review round 5 (F5-M-1): `eth_home_clear`: the operator's word, after looking, on the pending home transaction at `nonce` that no
/// provider shows, whose nonce is used, and that the canister could not identify (a Relay deposit or a sweep's `fundReserve`, which
/// leave nothing to read back): `mined` true, it is taken as mined; false, it is taken back as a revert and done again. Only for that
/// transaction, and only once it has been seen nowhere for `HOME_NONCE_TAKEN_SECS`.
pub fn home_clear(st: &mut EthState, nonce: u64, mined: bool, now: u64) -> Result<String, String> {
    let p = st.pending.as_ref().ok_or("refused: no home transaction is out")?;
    if p.nonce != nonce {
        return Err(format!("refused: the home transaction out is at nonce {}, not {nonce}", p.nonce));
    }
    // review round 6 (D6-L-1 / F6-I-2): only a step that leaves nothing to read back takes the operator's word; every other step is
    // decided by its own effect on chain (a read that keeps failing is the providers', not the operator's to settle)
    if !home_clear_applies(st, &p.step) {
        return Err(format!("refused: {:?} is decided by its own effect on chain (read back at each tick), never by the operator's word: \
            eth_home_clear applies only to a Relay deposit or a sweep's fundReserve; if its read keeps failing, look at the 4663 providers",
            p.step));
    }
    match p.used_since {
        Some(t) if now.saturating_sub(t) >= HOME_NONCE_TAKEN_SECS => {}
        _ => return Err(format!("refused: {:?} is not yet seen nowhere with its nonce used for {HOME_NONCE_TAKEN_SECS} s", p.step)),
    }
    st.inbox.home_clear = Some((nonce, mined, now));
    Ok(format!("{:?} (nonce {nonce}) is taken as {} at the next tick, unless the chain shows it first", p.step,
        if mined { "MINED" } else { "NOT mined (it is done again)" }))
}

/// Review round 6 (D6-L-1): the steps `eth_home_clear` is ever applied to: those that leave nothing on chain to read back (a Relay
/// deposit, a sweep's `fundReserve`). A post drops itself after the hour.
pub fn home_clear_applies(st: &EthState, step: &Step) -> bool {
    match step {
        Step::Deposit { .. } => true,
        Step::GiveBack { buy } => st.buy(*buy).is_some_and(|b| b.is_sweep()),
        _ => false,
    }
}

/// Review round 5 (F5-L-1): an upgrade that moves the NFT side to another chain (`from` → `to`; `post_upgrade` allows it only while
/// nothing is open there) resets what belongs to the old chain: its pin floor and nonce floor (`PinFloors.eth`) and our last mined
/// transaction there (`last_mined`). Kept, they would refuse every pin or nonce of the new chain for good. True when it moved.
pub fn chain_moved(st: &mut EthState, floors: &mut crate::pin::PinFloors, from: Option<u64>, to: Option<u64>) -> bool {
    match (from, to) {
        (Some(a), Some(b)) if a != b => {
            floors.eth = Default::default();
            st.last_mined = None;
            true
        }
        _ => false,
    }
}

/// What an upgrade to `next` would strand (audit 2026-09-27-nft round 2, C2-L-2): an open buy whose collection, wrapper and module
/// `next` no longer names together, an unfinished delivery of a wrapper it no longer names, a deposit or stray still being worked on
/// (or held) of a collection it no longer names, a transaction on the Ethereum line. `post_upgrade` refuses such an upgrade.
pub fn stranded_by(st: &EthState, next: Option<&EthNftConfig>) -> Vec<String> {
    let cols = next.and_then(|c| c.cols().ok()).unwrap_or_default();
    let mut out = Vec::new();
    for b in st.buys.iter().filter(|b| !b.finished()) {
        if !cols.iter().any(|c| c.collection == b.collection && c.wrapper == b.wrapper && c.module == b.module) {
            out.push(format!("buy {} of 0x{} is {:?}", b.id, hex::encode(b.collection), b.phase));
        }
    }
    for d in st.deliveries.iter().filter(|d| !d.finished()) {
        if !cols.iter().any(|c| c.collection == d.collection && c.wrapper == d.wrapper) {
            out.push(format!("delivery of ticket {} of 0x{} is {:?}", d.ticket, hex::encode(d.wrapper), d.phase));
        }
    }
    for x in st.deposits.iter().filter(|x| x.phase.open() || matches!(x.phase, XPhase::Held { .. })) {
        if x.collection != [0; 20] && !cols.iter().any(|c| c.collection == x.collection) {
            out.push(format!("claim {} of 0x{} is {:?}", h(&x.tx), hex::encode(x.collection), x.phase));
        }
    }
    if next.is_none() && st.line.is_some() {
        out.push("a transaction is on the Ethereum line".into());
    }
    out
}

/// 2026-10-12 (Robinhood Chain NFTs): the first buy id of the side of `chain` (not the config's own chain, whose side counts from 0):
/// `chain << 40`, so no two sides' buy ids meet (2^40 buys a side). The operator's hand-ins name a buy by its id alone and reach its side.
pub fn side_buy_base(chain: u64) -> u64 {
    (chain & 0xFF_FFFF) << 40
}

/// A new side of `chain`: an empty state whose buys count from [`side_buy_base`].
pub fn new_side(chain: u64) -> EthState {
    EthState { next_buy: side_buy_base(chain), ..EthState::default() }
}

/// 2026-10-12 (Robinhood Chain NFTs): whether a home step is this side's: one of its buys, or a collection (wrapper) of its config.
/// A side's buy ids never meet another's (`ethnet::side_buy_base`), nor do its collections (an address is configured once).
pub fn owns_step(st: &EthState, cfg: &EthNftConfig, step: &Step) -> bool {
    let cols = cfg.cols().unwrap_or_default();
    match step {
        Step::Advance { buy } | Step::Deposit { buy } | Step::GiveBack { buy } | Step::Mint { buy } | Step::Settle { buy } => st.buy(*buy).is_some(),
        Step::Record { wrapper, .. } | Step::Mark { wrapper, .. } | Step::Sent { wrapper, .. } => cols.iter().any(|c| c.wrapper == *wrapper),
        Step::Post { collection } | Step::Fill { collection, .. } | Step::Served { collection, .. } => cols.iter().any(|c| c.collection == *collection),
        Step::MintDeposit { tx } => st.deposits.iter().any(|d| d.tx == *tx),
    }
}

/// `eth_line_clear` (audit 2026-09-27-nft, B-M-3): the operator's exit from a halted line, after looking. Only while halted, and
/// only a line none of whose versions the chain shows mined (the operator re-reads it first: `eth_nft_state().line`). The nonce is
/// taken (by the transaction that was not ours), so nothing is signed below it again; the record goes back to before its
/// signature: a delivery waits to be signed again, a fill's buy waits for a listing again with what arrived, a stray waits for its
/// return again. The canister judges each one again from the start.
pub fn line_clear(st: &mut EthState) -> Result<String, String> {
    let why = st.halted.clone().ok_or("refused: the Ethereum line is not halted")?;
    let t = st.line.clone().ok_or("refused: nothing is on the line")?;
    match t.kind.clone() {
        EthKind::Deliver { wrapper, ticket } => {
            if let Some(d) = st.delivery_mut(&wrapper, ticket) {
                d.phase = DPhase::Waiting;
            }
        }
        EthKind::Buy { buy } => {
            if let Some(b) = st.buy_mut(buy) {
                if let BPhase::Buying { arrived, .. } = b.phase {
                    b.phase = BPhase::Arrived { wei: arrived, since: t.signed_at };
                }
            }
        }
        EthKind::Return { tx, token } => {
            if let Some(d) = st.deposit_mut(&tx, &token) {
                d.phase = XPhase::Return { why: "its return's nonce was taken by another transaction; judged again".into() };
            }
        }
        EthKind::BackDeposit { buy } => {
            if let Some(b) = st.buy_mut(buy) {
                if let BPhase::BackSending { wei, .. } = b.phase {
                    b.phase = BPhase::BackFunded { wei, since: t.signed_at };
                    b.last = None;
                }
            }
        }
        EthKind::Cancel { .. } | EthKind::CancelFill { .. } => return Err("refused: a cancel's nonce is read from its rivals, never cleared by hand".into()),
    }
    let (n0, b0) = st.last_mined.unwrap_or((0, 0));
    st.last_mined = Some((n0.max(t.nonce), b0));
    st.line = None;
    st.halted = None;
    Ok(format!("the line (nonce {}, {:?}) is cleared after: {why}", t.nonce, t.kind))
}

/// `eth_rearm_stray` (audit 2026-09-27-nft, C-L-5): a stray held for the operator goes back to its return path, where every check of
/// `judge_return` (ours, not held by the wrapper, not in use, not moved out since it came in) and the simulation run again. Nothing
/// is signed here, and nothing the canister would refuse is returned.
pub fn rearm_stray(st: &mut EthState, cfg: &EthNftConfig, tx: [u8; 32], token: Word) -> Result<String, String> {
    let d = st.deposit_mut(&tx, &token).ok_or("refused: no such claim")?;
    if !matches!(d.phase, XPhase::Held { .. }) {
        return Err(format!("refused: the claim is {:?}, not held", d.phase));
    }
    if cfg.col(&d.collection).is_none() {
        return Err("refused: its collection is not configured: the canister touches configured collections only".into());
    }
    if d.from == [0; 20] {
        return Err("refused: no sender to return it to".into());
    }
    d.phase = XPhase::Return { why: "re-armed by the operator".into() };
    d.refused_since = None;
    Ok(format!("claim {} token {}: back to its return path; judged again at the next tick", h(&tx), h(&token)))
}

/// Keep one hand-in per key, newest wins.
pub fn put_quote(st: &mut EthState, buy: u64, body: Vec<u8>, now: u64) -> Result<String, String> {
    let b = st.buy(buy).ok_or_else(|| format!("no buy {buy}"))?;
    if !matches!(b.phase, BPhase::Funded { .. } | BPhase::BackFunded { .. }) {
        return Err(format!("refused: buy {buy} is {:?}; a quote is handed in only while it is funded (or going back)", b.phase));
    }
    if body.len() > crate::relayconv::MAX_QUOTE_BYTES {
        return Err("refused: the quote is too large".into());
    }
    st.inbox.quotes.retain(|(x, ..)| *x != buy);
    st.inbox.quotes.push((buy, body, now));
    Ok(format!("quote held for buy {buy}; judged at the next tick"))
}

pub fn put_listing(st: &mut EthState, buy: u64, order_hex: String, now: u64) -> Result<String, String> {
    let b = st.buy(buy).ok_or_else(|| format!("no buy {buy}"))?;
    if !matches!(b.phase, BPhase::Arrived { .. }) {
        return Err(format!("refused: buy {buy} is {:?}; a listing is handed in only once its ETH arrived", b.phase));
    }
    if order_hex.len() > 32 * 1024 {
        return Err("refused: the order is too large".into());
    }
    en::Listing::parse_hex(&order_hex)?;
    st.inbox.listings.retain(|(x, ..)| *x != buy);
    st.inbox.listings.push((buy, order_hex, now));
    Ok(format!("listing held for buy {buy}; judged at the next tick"))
}

/// Listings as floor evidence (the sale hashes of a hand-in are kept for the interface and not used: round 2, the canister reads the
/// collection's sales itself, `sale_scan`).
pub fn put_evidence(st: &mut EthState, cfg: &EthNftConfig, collection: [u8; 20], listings: Vec<String>, sales: Vec<[u8; 32]>, now: u64)
    -> Result<String, String> {
    if cfg.col(&collection).is_none() {
        return Err("refused: not a configured collection".into());
    }
    // docs/v7/dormant-collections.md §2.2: a DORMANT collection's evidence is never read
    let k = kind_of(st, cfg, &collection);
    if k.asleep() {
        return Err(format!("refused: 0x{} is {}: its evidence is not read (a wake request wakes it)", hex::encode(collection), k.word()));
    }
    if listings.len() > en::MAX_LISTINGS || sales.len() > en::MAX_SALES || listings.iter().any(|l| l.len() > 32 * 1024) {
        return Err(format!("refused: at most {} listings and {} sales", en::MAX_LISTINGS, en::MAX_SALES));
    }
    st.inbox.evidence.retain(|(c, ..)| *c != collection);
    st.inbox.evidence.push((collection, listings, sales, now));
    Ok("evidence held; verified at the next tick".into())
}

pub fn put_deposit(st: &mut EthState, cfg: &EthNftConfig, tx: [u8; 32], now: u64) -> Result<String, String> {
    if !cfg.deposits {
        return Err("refused: holder deposits are off (eth_nft.deposits = false)".into());
    }
    if st.deposits.iter().any(|d| d.tx == tx) || st.inbox.deposits.iter().any(|(x, _)| *x == tx) {
        return Err("refused: this transaction was already claimed".into());
    }
    if st.inbox.deposits.len() >= 16 {
        return Err("refused: 16 claims wait already".into());
    }
    st.inbox.deposits.push((tx, now));
    Ok("claim held; verified at the next tick".into())
}

// ---- dormant, wake-on-demand collections (docs/v7/dormant-collections.md) ---------------------------------------------------------

/// The collection as the flow treats it: `Always` without `eth_nft.dormancy`, and before its life is decided (never watched less
/// than before this change).
pub fn kind_of(st: &EthState, cfg: &EthNftConfig, c: &[u8; 20]) -> Kind {
    if cfg.dormancy.is_none() {
        return Kind::Always;
    }
    st.dorm.as_ref().map_or(Kind::Always, |d| d.kind(c))
}

/// §4: hot (every hand-in is a pass with a scan, the module read every tick): an open buy of the collection, a deposit of it waiting
/// for its mint, or the window a module that wanted an item, or a fill that waited for a fresh read, opened.
pub fn is_hot(st: &EthState, c: &[u8; 20], now: u64) -> bool {
    st.buys.iter().any(|b| !b.finished() && b.collection == *c)
        || st.deposits.iter().any(|d| d.collection == *c && matches!(d.phase, XPhase::Seen | XPhase::Mint))
        || st.dorm.as_ref().and_then(|d| d.life(c)).is_some_and(|l| l.hot_until > now)
}

/// §4.3: the fill is signed only against a fresh, scanned, confirmed reading that agrees with the module's accepted floor and with the
/// floor the buy's advance was sized on. `Err((why, keep the listing))`: a read that is only not fresh keeps the listing for the next
/// pass; a mismatch drops it.
fn fresh_for_fill(st: &EthState, cfg: &EthNftConfig, c: &Col, buy: u64, accepted: u128, now: u64) -> Result<(), (String, bool)> {
    let k = kind_of(st, cfg, &c.collection);
    if !k.live() {
        return Err((format!("its collection is {}: no fill until it is active", k.word()), false));
    }
    let r = st.reading(&c.collection).ok_or_else(|| ("no floor reading of ours: the fill waits for a fresh floor and sale read".to_string(), true))?;
    if now > r.at.saturating_add(dm::BUY_FRESH_SECS) {
        return Err((format!("our floor reading is {} s old: the fill needs one at most {} s old (a fresh floor and sale read)", now - r.at,
            dm::BUY_FRESH_SECS), true));
    }
    let life = st.dorm.as_ref().and_then(|d| d.life(&c.collection));
    match life.and_then(|l| l.read_scan) {
        Some((at, true, behind)) if at == r.at && behind <= dm::BUY_SCAN_LAG_BLOCKS => {}
        Some((at, true, behind)) if at == r.at => {
            return Err((format!("the sale scan of our reading ended {behind} final blocks behind (at most {})", dm::BUY_SCAN_LAG_BLOCKS), true));
        }
        _ => return Err(("our reading's pass did not run the sale scan: the fill waits for a pass that does".into(), true)),
    }
    if r.post.held.is_some() || r.post.sales < cfg.floor_params().min_sales as u32 {
        return Err((format!("our reading is not confirmed by the counted sales ({} counted{})", r.post.sales,
            r.post.held.as_ref().map(|h| format!("; held: {h}")).unwrap_or_default()), true));
    }
    if accepted == 0 || !dm::within_bps(r.post.price, accepted, dm::BUY_MISMATCH_BPS) {
        return Err((format!("MISMATCH: our fresh reading {} is more than {} bps from the module's accepted floor {accepted}", r.post.price,
            dm::BUY_MISMATCH_BPS), false));
    }
    // dormant-review DR-L-4: no record of the floor this buy's advance was sized on is a refusal, never a pass
    let Some((_, f)) = life.and_then(|l| l.buy_floor).filter(|(id, _)| *id == buy) else {
        return Err((format!("no record of the floor buy {buy}'s advance was sized on: no fill against an unchecked drift"), false));
    };
    if !dm::within_bps(r.post.price, f, dm::BUY_DRIFT_BPS) {
        return Err((format!("MISMATCH: our fresh reading {} is more than {} bps from the floor {f} this buy's advance was sized on",
            r.post.price, dm::BUY_DRIFT_BPS), false));
    }
    Ok(())
}

/// A collection that goes to sleep (or starts a wake) keeps no reading, no scan and no cached sale: the next wake reads its 7-day
/// window afresh.
fn forget_scan(st: &mut EthState, c: &[u8; 20]) {
    st.readings.retain(|r| r.collection != *c);
    if let Some(v) = st.sale_scan.as_mut() {
        v.retain(|x| x.collection != *c);
    }
    if let Some(v) = st.sale_cache.as_mut() {
        v.retain(|x| x.collection != *c);
    }
    st.inbox.evidence.retain(|(x, ..)| x != c);
}

/// §2.1: the canister already has something of `c` (it was watched before the dormancy was turned on).
fn has_work(st: &EthState, c: &Col) -> bool {
    st.readings.iter().any(|r| r.collection == c.collection)
        || st.buys.iter().any(|b| b.collection == c.collection)
        || st.deliveries.iter().any(|d| d.collection == c.collection)
        || st.deposits.iter().any(|d| d.collection == c.collection && d.phase.open())
        || st.sale_scan.iter().flatten().any(|s| s.collection == c.collection)
        || st.cursor(&c.wrapper) > 0
}

/// Whether a home step belongs to collection `c`.
fn step_of(step: &Step, st: &EthState, c: &Col) -> bool {
    match step {
        Step::Post { collection } | Step::Fill { collection, .. } | Step::Served { collection, .. } => *collection == c.collection,
        Step::Record { wrapper, .. } | Step::Mark { wrapper, .. } | Step::Sent { wrapper, .. } => *wrapper == c.wrapper,
        Step::MintDeposit { tx } => st.deposits.iter().any(|d| d.tx == *tx && d.collection == c.collection),
        s => buy_of_step(s).and_then(|id| st.buy(id)).is_some_and(|b| b.collection == c.collection),
    }
}

/// §5: what the canister itself still has open for `c` (a buy, a written-off order holding the leg, a delivery, a deposit waiting for
/// its mint, a home or Ethereum transaction, a leftover to sweep), or `None`. A stray's return needs no floor and holds nothing.
fn canister_open(st: &EthState, c: &Col) -> Option<String> {
    if let Some(b) = st.buys.iter().find(|b| b.collection == c.collection
        && (!b.finished() || b.late.as_ref().is_some_and(|l| l.credited.is_none() && l.released_at.is_none()))) {
        return Some(format!("buy {} is open", b.id));
    }
    if let Some(d) = st.deliveries.iter().find(|d| d.wrapper == c.wrapper && !d.finished()) {
        return Some(format!("ticket {} is being delivered", d.ticket));
    }
    if st.deposits.iter().any(|d| d.collection == c.collection && matches!(d.phase, XPhase::Seen | XPhase::Mint | XPhase::Minting)) {
        return Some("a deposit waits for its mint".into());
    }
    if let Some(p) = st.pending.as_ref().filter(|p| step_of(&p.step, st, c)) {
        return Some(format!("{:?} is out on 4663", p.step));
    }
    if let Some(p) = eth_sol_step(st).filter(|p| step_of(p, st, c)) {
        return Some(format!("{p:?} is out on Solana"));
    }
    if let Some(t) = &st.line {
        let ours = match &t.kind {
            EthKind::Buy { buy } | EthKind::CancelFill { buy } | EthKind::BackDeposit { buy } => st.buy(*buy).is_some_and(|b| b.collection == c.collection),
            EthKind::Deliver { wrapper, .. } | EthKind::Cancel { wrapper, .. } => *wrapper == c.wrapper,
            EthKind::Return { tx, .. } => st.deposits.iter().any(|d| d.tx == *tx && d.collection == c.collection),
        };
        if ours {
            return Some("a transaction of it is on the Ethereum line".into());
        }
    }
    if st.leftover_of(&c.collection) >= SWEEP_MIN_WEI {
        return Some(format!("its leftover {} wei waits for a sweep", st.leftover_of(&c.collection)));
    }
    None
}

async fn read_u(io: &mut impl EthIo, to: [u8; 20], data: &[u8]) -> Result<u128, String> {
    crate::abi::uint_at(&io.home_read(to, data).await?, 0, 0, 128)
}

async fn read_a(io: &mut impl EthIo, to: [u8; 20], data: &[u8]) -> Result<[u8; 20], String> {
    crate::abi::addr_at(&io.home_read(to, data).await?, 0, 0)
}

// ---- the wake desk and the module, on either home (docs/v9 §4; the books home has no desk: O-11) ----------------------------------

async fn desk_window<I: EthIo>(io: &mut I, desk: [u8; 20]) -> Result<u128, String> {
    if io.sol_home().is_some() {
        return Ok(home_sol::desk(io).await?.0 as u128);
    }
    read_u(io, desk, &dm::WINDOW).await
}

async fn desk_count<I: EthIo>(io: &mut I, desk: [u8; 20]) -> Result<u128, String> {
    if io.sol_home().is_some() {
        return Ok(home_sol::desk(io).await?.1 as u128);
    }
    read_u(io, desk, &dm::REQUEST_COUNT).await
}

/// A desk request as `dormancy::decode_request` gives it: `(wrapper, requester, at, bond, closed)` (the bond is not read on Solana).
async fn desk_request<I: EthIo>(io: &mut I, desk: [u8; 20], id: u64) -> Result<([u8; 20], [u8; 20], u64, u128, bool), String> {
    if io.sol_home().is_some() {
        let (w, r, at, _, closed) = home_sol::request(io, id).await?;
        return Ok((w, r, at, 0, closed));
    }
    io.home_read(desk, &en::call_word(dm::REQUEST_AT, crate::abi::uint_word(id as u128))).await.and_then(|r| dm::decode_request(&r))
}

async fn desk_served_at<I: EthIo>(io: &mut I, desk: [u8; 20], id: u64) -> Result<u128, String> {
    if io.sol_home().is_some() {
        return Ok(home_sol::request(io, id).await?.3 as u128);
    }
    io.home_read(desk, &en::call_word(dm::SERVED_AT, crate::abi::uint_word(id as u128))).await.and_then(|r| crate::abi::uint_at(&r, 0, 0, 64))
}

async fn module_accepted<I: EthIo>(io: &mut I, c: &Col) -> Result<u128, String> {
    if io.sol_home().is_some() {
        return home_books::accepted(io, c.module).await;
    }
    read_u(io, c.module, &en::ACCEPTED).await
}

/// The module's factory, read once and kept.
async fn factory_of<I: EthIo>(io: &mut I, st: &mut EthState, c: &Col) -> Result<[u8; 20], String> {
    if let Some(f) = st.dorm.as_ref().and_then(|d| d.life(&c.collection)).and_then(|l| l.factory) {
        return Ok(f);
    }
    let f = read_a(io, c.module, &dm::FACTORY).await?;
    if f == [0; 20] {
        return Err("the module has no factory wired".into());
    }
    if let Some(l) = st.dorm.as_mut().and_then(|d| d.life_mut(&c.collection)) {
        l.factory = Some(f);
    }
    Ok(f)
}

/// §3.3: the desk's new bonded requests, a few a tick. Independent review DI-M-1 / DI-M-2: each one read is held on its collection's
/// life with its window's end (`at + window()`, the window read once): queued while DORMANT and still servable inside the window, kept
/// while WAKING or ACTIVE (a post serves it). A request read after its window ended asks for nothing.
async fn desk_read<I: EthIo>(io: &mut I, st: &mut EthState, p: &dm::Dormancy, cols: &[Col], desk: [u8; 20], now: u64, lines: &mut Vec<String>) {
    let d = st.dorm.as_mut().expect("set");
    if d.desk != Some(desk) {
        d.desk = Some(desk);
        d.desk_cursor = 0;
        d.desk_window = None;
    }
    let from = d.desk_cursor;
    let window = match d.desk_window {
        Some(w) => w,
        None => match desk_window(io, desk).await {
            Ok(w) => {
                let w = w.min(u64::MAX as u128) as u64;
                st.dorm.as_mut().expect("set").desk_window = Some(w);
                let need = p.wake_timeout_secs.saturating_add(dm::DESK_START_MARGIN_SECS);
                lines.push(format!("eth dormancy: the wake desk's window is {w} s{}", if w <= need {
                    format!(": NOT above wake_timeout_secs + {} s = {need} s, so no desk request can ever be served inside it", dm::DESK_START_MARGIN_SECS)
                } else { String::new() }));
                w
            }
            Err(e) => {
                lines.push(format!("eth dormancy: waits: the desk's window() not read: {e}"));
                return;
            }
        },
    };
    let n = match desk_count(io, desk).await {
        Ok(n) => n as u64,
        Err(e) => {
            lines.push(format!("eth dormancy: waits: the desk's requestCount() not read: {e}"));
            return;
        }
    };
    let mut id = from;
    while id < n && id < from.saturating_add(dm::DESK_READS_PER_TICK) {
        let r = desk_request(io, desk, id).await;
        let (wrapper, requester, at, _, closed) = match r {
            Ok(x) => x,
            Err(e) => {
                lines.push(format!("eth dormancy: waits: the desk's request {id} not read: {e}"));
                break;
            }
        };
        id += 1;
        if closed {
            continue;
        }
        let Some(c) = cols.iter().find(|c| c.wrapper == wrapper) else {
            lines.push(format!("eth dormancy: the desk's request {} names 0x{}, which this canister does not serve: ignored", id - 1, hex::encode(wrapper)));
            continue;
        };
        let until = at.saturating_add(window);
        if now >= until {
            lines.push(format!("eth dormancy: the desk's request {} for 0x{} was read after its window ended: nothing to serve", id - 1,
                hex::encode(c.collection)));
            continue;
        }
        let ask = dm::DeskAsk { id: id - 1, requester, at, until, tried: false, served: None, marked: false };
        match st.dorm.as_mut().expect("set").desk_ask(&c.collection, ask, p, now) {
            Ok(s) => lines.push(format!("eth dormancy: {s}")),
            Err(e) => {
                // dormant-review DR-L-1: the request is read again once its collection's life is decided
                id -= 1;
                lines.push(format!("eth dormancy: waits: the desk's request {id}: {e}"));
                break;
            }
        }
    }
    st.dorm.as_mut().expect("set").desk_cursor = id;
}

/// The dormancy's pass, each tick (§2, §3, §5): lives decided, the desk's new requests, owed deposits, timeouts, one or two
/// collections' checks (the DORMANT watchdog, the ACTIVE launch count and sleep), the fill hunt's trigger, and the wakes that fit.
async fn dorm_pass<I: EthIo>(io: &mut I, st: &mut EthState, cfg: &EthNftConfig, cols: &[Col]) -> Vec<String> {
    let Some(p) = cfg.dormancy.clone() else { return vec![] };
    let now = io.now();
    let mut lines = Vec::new();
    let keys: Vec<[u8; 20]> = cols.iter().map(|c| c.collection).collect();
    st.dorm.get_or_insert_with(Default::default).retain(&keys);
    // 1. §2.1: each collection's life, decided once: ACTIVE if the canister has anything of it or its module was ever posted
    for c in cols {
        if st.dorm.as_ref().is_some_and(|d| d.life(&c.collection).is_some()) {
            continue;
        }
        let phase = if has_work(st, c) {
            Some(dm::Phase::Active { since: now })
        } else {
            match module_accepted(io, c).await {
                Ok(0) => Some(dm::Phase::Dormant),
                Ok(_) => Some(dm::Phase::Active { since: now }),
                Err(e) => {
                    lines.push(format!("eth dormancy 0x{}: waits: its life is not decided yet (the module's accepted() not read: {e}); watched \
                        as before meanwhile", hex::encode(c.collection)));
                    None
                }
            }
        };
        if let Some(ph) = phase {
            let mut l = dm::Life::new(c.collection, ph, now);
            if l.is_active() {
                // an existing collection's positions before today are not known: its idle clock starts now (idle_days, not a day)
                l.position_at = Some(now);
                // §11: counted as holding a position until its first check says otherwise (the safe side for the capacity page)
                l.pinned = true;
                l.check_at = now.saturating_add(dm::SLEEP_CHECK_SECS);
                l.note = "active: it was watched before the dormancy was configured (the canister holds something of it, or its module \
                    was posted)".into();
            } else {
                l.note = "dormant: nothing is read until a wake is asked".into();
            }
            lines.push(format!("eth dormancy 0x{}: {}", hex::encode(c.collection), l.note));
            st.dorm.as_mut().expect("set").lives.push(l);
        }
    }
    // 2. §3.3: the desk's new bonded requests, a few a tick
    match p.desk() {
        Ok(Some(desk)) => desk_read(io, st, &p, cols, desk, now, &mut lines).await,
        Ok(None) => {}
        Err(e) => lines.push(format!("eth dormancy: the configured desk does not parse: {e}")),
    }
    // 3. §3.1: a valid deposit of a DORMANT collection asks for its wake (owed: an NFT is at our address)
    let owed: Vec<([u8; 20], [u8; 32])> = st.deposits.iter().filter(|d| matches!(d.phase, XPhase::Seen | XPhase::Mint))
        .map(|d| (d.collection, d.tx)).collect();
    for (c, tx) in owed {
        let d = st.dorm.as_mut().expect("set");
        // dormant-review DR-H-2: after OWED_FAILS_MAX timed-out owed wakes, the deposit goes back to its sender instead of asking again
        if d.life(&c).is_some_and(|l| l.is_dormant() && l.owed_fails >= dm::OWED_FAILS_MAX) {
            if let Some(l) = d.life_mut(&c) {
                if matches!(l.queued, Some((dm::WakeBy::Deposit { tx: t }, _)) if t == tx) {
                    l.queued = None;
                }
            }
            if let Some(x) = st.deposits.iter_mut().find(|x| x.tx == tx && x.collection == c && matches!(x.phase, XPhase::Seen | XPhase::Mint)) {
                x.phase = XPhase::Return { why: format!("its collection's owed wake timed out {} times: the NFT goes back to its sender",
                    dm::OWED_FAILS_MAX) };
                lines.push(format!("eth dormancy 0x{}: deposit {}: {} owed wakes timed out: RETURNED to its sender (DR-H-2)", hex::encode(c),
                    h(&tx), dm::OWED_FAILS_MAX));
            }
            continue;
        }
        // independent review DI-H-1: a DORMANT collection's deposit asks for its owed wake whatever is queued already: a queued desk or
        // operator wake is upgraded to owed (keeping its place), so the owed reserve and the owed path hold for it
        if d.life(&c).is_some_and(|l| l.is_dormant() && !l.queued.as_ref().is_some_and(|(b, _)| b.owed())) {
            if let Ok(s) = d.request(&c, dm::WakeBy::Deposit { tx }, now) {
                lines.push(format!("eth dormancy: {s}"));
            }
        }
    }
    // 4. §2.3: a wake past its timeout fails: DORMANT, the cooldown, paged
    for c in st.dorm.as_mut().expect("set").timeouts(&p, now) {
        forget_scan(st, &c);
        lines.push(format!("eth dormancy 0x{}: WAKE TIMED OUT after {} s: dormant again, cooldown {} s (the monitor pages it)", hex::encode(c),
            p.wake_timeout_secs, p.cooldown_secs));
    }
    // 5. the checks that are due, a couple a tick: the DORMANT watchdog (a once-active module only) and the ACTIVE sleep check.
    // §11: the balance is read once a tick; an ACTIVE collection found with nothing open while more are awake than the balance keeps is
    // put to sleep at its check without waiting out its idle period (scaling down never touches one with a position)
    let balance = io.cycles_balance();
    let due: Vec<Col> = cols.iter().filter(|c| st.dorm.as_ref().and_then(|d| d.life(&c.collection)).is_some_and(|l| now >= l.check_at
        // DI-H-1: the watchdog reads a DORMANT once-active module whose queued wake, if any, is not owed (a graduation upgrades it)
        && (l.is_active() || (l.is_dormant() && l.ever_active && l.queued.as_ref().is_none_or(|(b, _)| !b.owed())))))
        .take(dm::CHECKS_PER_TICK).copied().collect();
    for c in due {
        if let Some(l) = st.dorm.as_mut().and_then(|d| d.life_mut(&c.collection)) {
            l.check_at = now.saturating_add(dm::SLEEP_CHECK_SECS);
        }
        // review S-3: only on a balance actually read (an unread one starts no wake, and puts nothing to sleep either)
        let over = balance.is_some() && st.dorm.as_ref().is_some_and(|d| {
            let cap = dm::capacity(&p, d, balance, cols.len(), pinned_count(st, cols), now);
            cap.scaling && cap.awake > cap.effective_max_active
        });
        match check_life(io, st, &p, &c, now, over).await {
            Ok(s) => lines.push(format!("eth dormancy 0x{}: {s}", hex::encode(c.collection))),
            Err(e) => lines.push(format!("eth dormancy 0x{}: waits: the check's reads failed: {e} (again at the next period)", hex::encode(c.collection))),
        }
    }
    // 6. §5: an ACTIVE collection's queued graduation (gradEth() > 0) starts the fill hunt, read on the view's cadence. docs/v9 §2.1:
    // none on the Solana home (the books have no graduation queue and no fill)
    let grads: Vec<Col> = cols.iter().filter(|_| io.sol_home().is_none()).filter(|c| st.dorm.as_ref().and_then(|d| d.life(&c.collection)).is_some_and(|l| l.is_active()
        && now >= l.grad_at.saturating_add(dm::VIEW_EVERY_SECS))).copied().collect();
    for c in grads {
        let got = read_u(io, c.module, &dm::GRAD_ETH).await;
        if let Some(l) = st.dorm.as_mut().and_then(|d| d.life_mut(&c.collection)) {
            l.grad_at = now;
            match got {
                Ok(0) => l.fill_hunt = None,
                Ok(x) => {
                    if l.fill_hunt.is_none() {
                        l.fill_hunt = Some(now);
                        lines.push(format!("eth dormancy 0x{}: {x} wei of graduation queued at its module: the fill hunt starts", hex::encode(c.collection)));
                    }
                }
                Err(e) => lines.push(format!("eth dormancy 0x{}: waits: gradEth() not read: {e}", hex::encode(c.collection))),
            }
        }
    }
    // 7. §3.2: the wakes that fit the caps start; each reads its 7-day window afresh. DI-M-1: first the desk requests' windows (a desk
    // wake that could no longer post inside its window is dropped, never started late), and each desk wake about to start is read again
    // on the desk: one closed there (reclaimed) is dropped, one not read waits for the next tick
    for n in st.dorm.as_mut().expect("set").desk_upkeep(&p, now) {
        lines.push(format!("eth dormancy: {n}"));
    }
    let held = desk_recheck(io, st, &p, now, balance, &mut lines).await;
    let (started, notes) = st.dorm.as_mut().expect("set").start_wakes(&p, now, balance);
    for (c, q) in held {
        if let Some(l) = st.dorm.as_mut().and_then(|d| d.life_mut(&c)).filter(|l| l.is_dormant() && l.queued.is_none()) {
            l.queued = Some(q);
        }
    }
    for c in started {
        forget_scan(st, &c);
        let by = st.dorm.as_ref().and_then(|d| d.life(&c)).and_then(|l| l.by.clone()).map(|b| b.word()).unwrap_or_default();
        lines.push(format!("eth dormancy 0x{}: WAKING (woken by {by}): the sale scan reads the 7-day window; no post, buy or deposit mint \
            until the backfill is complete and the reading confirmed", hex::encode(c)));
    }
    for n in notes {
        lines.push(format!("eth dormancy: waits: {n}"));
    }
    // §11: the capacity as of this tick, for the monitor (it pages when fewer can be awake than hold positions) and the site
    let pinned = pinned_count(st, cols);
    let d = st.dorm.as_mut().expect("set");
    let cap = dm::capacity(&p, d, balance, cols.len(), pinned, now);
    if cap.scaling && d.cap.as_ref().is_none_or(|x| x.effective_max_active != cap.effective_max_active || x.pinned != cap.pinned) {
        lines.push(format!("eth dormancy: capacity: {} (liquid balance {} B, ceiling {})", cap.status, cap.balance / dm::B, cap.ceiling));
    }
    d.cap = Some(cap);
    lines
}

/// DI-M-1: the queued desk wakes that would start this tick are read again on the desk before they do. A request closed there (its bond
/// already reclaimed) is dropped from the queue; one whose read failed is held out of this tick's start (returned, to be put back after
/// it). Repeated until the wakes that would start are all read (at most one read per queued desk wake a tick).
async fn desk_recheck<I: EthIo>(io: &mut I, st: &mut EthState, p: &dm::Dormancy, now: u64, balance: Option<u128>, lines: &mut Vec<String>)
    -> Vec<([u8; 20], (dm::WakeBy, u64))> {
    let mut held = Vec::new();
    let Ok(Some(desk)) = p.desk() else { return held };
    let mut checked: Vec<u64> = Vec::new();
    loop {
        let Some(d) = st.dorm.as_ref() else { return held };
        let (would, _) = d.clone().start_wakes(p, now, balance);
        let next: Vec<([u8; 20], u64)> = would.iter().filter_map(|c| match &d.life(c)?.queued {
            Some((dm::WakeBy::Desk { id, .. }, _)) if !checked.contains(id) => Some((*c, *id)),
            _ => None,
        }).collect();
        if next.is_empty() {
            return held;
        }
        for (c, id) in next {
            checked.push(id);
            let r = desk_request(io, desk, id).await;
            let Some(l) = st.dorm.as_mut().and_then(|d| d.life_mut(&c)) else { continue };
            match r {
                Ok((.., closed)) if closed => {
                    l.queued = None;
                    if l.desk.as_ref().is_some_and(|a| a.id == id) {
                        l.desk = None;
                    }
                    lines.push(format!("eth dormancy 0x{}: the wake desk's request #{id} was closed on the desk before its wake started: \
                        dropped from the queue", hex::encode(c)));
                }
                Ok(_) => {}
                Err(e) => {
                    if let Some(q) = l.queued.take() {
                        held.push((c, q));
                    }
                    lines.push(format!("eth dormancy 0x{}: waits: the wake desk's request #{id} not read again before its wake: {e}", hex::encode(c)));
                }
            }
        }
    }
}

/// §11: the awake collections that hold a position: on 4663 at their last check (`Life.pinned`), or open in the canister now.
fn pinned_count(st: &EthState, cols: &[Col]) -> usize {
    let Some(d) = st.dorm.as_ref() else { return 0 };
    cols.iter().filter(|c| d.life(&c.collection).is_some_and(|l| (l.is_active() || l.is_waking()) && (l.pinned || canister_open(st, c).is_some())))
        .count()
}

/// One collection's check (§2.2, §5): the DORMANT watchdog (a graduation queued at a once-active module: an owed wake), or, ACTIVE, the
/// positions on 4663 and in the canister, and, once idle, the move back to DORMANT. Dormant-review DR-H-1: a bare launch is no position
/// (it costs gas alone) and is never read here: only what holds value, or waits on the floor, keeps a collection awake.
async fn check_life<I: EthIo>(io: &mut I, st: &mut EthState, p: &dm::Dormancy, c: &Col, now: u64, over: bool) -> Result<String, String> {
    let l = st.dorm.as_ref().and_then(|d| d.life(&c.collection)).cloned().ok_or("no life")?;
    match l.phase {
        dm::Phase::Dormant if io.sol_home().is_some() => {
            // docs/v9 §2.1: a coin of the collection graduated (its DBC pool migrated) and is not backed yet: its backing waits for a
            // fresh floor: an owed wake
            let n = home_books::graduations(io, c).await?;
            if n > 0 {
                let d = st.dorm.as_mut().expect("set");
                return d.request(&c.collection, dm::WakeBy::Graduation, now).map(|s| format!("watchdog: {n} graduated coin(s) wait for \
                    their backing at its stale module: {s}"));
            }
            Ok("watchdog: no graduated coin waits for its backing".into())
        }
        dm::Phase::Dormant => {
            let grad = read_u(io, c.module, &dm::GRAD_ETH).await?;
            if grad > 0 {
                let d = st.dorm.as_mut().expect("set");
                return d.request(&c.collection, dm::WakeBy::Graduation, now).map(|s| format!("watchdog: {grad} wei of graduation queued at \
                    its stale module: {s}"));
            }
            Ok("watchdog: no graduation queued".into())
        }
        dm::Phase::Active { .. } => {
            // the positions on 4663 first (read every check: they are the idle clock), then what the canister holds open. docs/v9 §4:
            // on the Solana home, the books (a coin not yet backed counts as open)
            let open = if io.sol_home().is_some() { home_books::positions(io, c).await? } else { dm::OnChain {
                total_supply: read_u(io, c.wrapper, &dm::TOTAL_SUPPLY).await?,
                held: read_u(io, c.wrapper, &dm::HELD).await?,
                rented_held: read_u(io, c.wrapper, &dm::RENTED_HELD).await?,
                grad_eth: read_u(io, c.module, &dm::GRAD_ETH).await?,
                grad_units: read_u(io, c.module, &dm::GRAD_UNITS).await?,
                transit_units: read_u(io, c.module, &crate::relayconv::TRANSIT_UNITS).await?,
                float_units: read_u(io, c.module, &dm::FLOAT_UNITS).await?,
            }.open() };
            let why = if !open.is_empty() { Some(format!("on {} it has {}", if io.sol_home().is_some() { "Solana" } else { "4663" }, open.join(", "))) }
                else { canister_open(st, c) };
            if let Some(why) = why {
                if let Some(x) = st.dorm.as_mut().and_then(|d| d.life_mut(&c.collection)) {
                    x.position_at = Some(now);
                    x.pinned = true;
                }
                return Ok(format!("stays active: {why}"));
            }
            if let Some(x) = st.dorm.as_mut().and_then(|d| d.life_mut(&c.collection)) {
                x.pinned = false;
            }
            // §11: more awake than the cycles balance keeps, and this one holds nothing: it sleeps now (scaling down), not after its idle period
            if over {
                st.dorm.as_mut().expect("set").sleep(&c.collection, p, now, "scaled down: more collections were awake than the cycles balance \
                    keeps, and this one held no position");
                forget_scan(st, &c.collection);
                return Ok(format!("DORMANT: scaled down (more awake than the cycles balance keeps; nothing open here): its reading, scan and \
                    cached sales are dropped, cooldown {} s", p.cooldown_secs));
            }
            if let Err(e) = dm::idle_long_enough(&l, p, now) {
                return Ok(format!("stays active: nothing open now; {e}"));
            }
            st.dorm.as_mut().expect("set").sleep(&c.collection, p, now, "idle: no position for the idle period (no unit, item, rental, \
                graduation, advance or float on 4663; nothing open in the canister)");
            forget_scan(st, &c.collection);
            Ok(format!("DORMANT: idle, and nothing open: its reading, scan and cached sales are dropped, cooldown {} s (a coin still trading \
                on it wakes it again when it graduates)", p.cooldown_secs))
        }
        dm::Phase::Waking { .. } => Ok(String::new()),
    }
}

/// §5: the fill hunt (dormant-review DR-M-2). An ACTIVE collection whose module holds a queued graduation (`gradEth() > 0`, read on the
/// view's cadence) and whose floor is fresh: the operator's hand-ins for it (`eth_fill`) first, then the factory's newest coins, a few a
/// try and a try at most every `FILL_RETRY_SECS`. The first `Graduating` curve whose graduation is not filled gets the permissionless
/// `FloorModule.fill(coin)`; one that would not succeed now (a sweep-only coin, no room) is passed over. A flood of launched coins can
/// never make the hunt read them all: an older graduation is found by the operator's hand-in (the module's `GradQueued` event names the
/// coin), and the monitor pages a graduation left queued.
async fn fill_act<I: EthIo>(io: &mut I, st: &mut EthState, cfg: &EthNftConfig, cols: &[Col]) -> Result<Option<String>, String> {
    let now = io.now();
    // independent review DI-M-3: one collection's wait (its floor not fresh, its accepted floor still being walked to our reading, a read
    // that failed) never stops another collection's fill: each wait is collected, that collection tried again after FILL_RETRY_SECS, and
    // the hunt goes on to the next
    let mut waits = Vec::new();
    for c in cols {
        let Some(at) = st.dorm.as_ref().and_then(|d| d.life(&c.collection)).filter(|l| l.is_active()).and_then(|l| l.fill_hunt) else {
            continue;
        };
        if now < at || kind_of(st, cfg, &c.collection) != Kind::Active {
            continue;
        }
        match fill_one(io, st, c, now).await {
            Ok(Some(l)) => return Ok(Some(l)),
            Ok(None) => {}
            Err((e, global)) => {
                if global {
                    // a signature that failed for a reason of the home line (the nonce, the pause): nothing else is signed this tick
                    return Err(e);
                }
                if let Some(l) = st.dorm.as_mut().and_then(|d| d.life_mut(&c.collection)) {
                    l.fill_hunt = Some(now.saturating_add(dm::FILL_RETRY_SECS));
                }
                waits.push(e);
            }
        }
    }
    if waits.is_empty() { Ok(None) } else { Err(waits.join("; ")) }
}

/// One collection's fill hunt (`fill_act`). `Err((why, global))`: `global` when the home line refused the signature (nothing else is
/// signed this tick), else this collection's own wait.
async fn fill_one<I: EthIo>(io: &mut I, st: &mut EthState, c: &Col, now: u64) -> Result<Option<String>, (String, bool)> {
    let x = hex::encode(c.collection);
    let local = |e: String| (format!("eth fill 0x{x}: waits: {e}"), false);
    let (accepted, _, posted_at) = module_floor(io, c.module).await.map_err(local)?;
    if posted_at == 0 || now.saturating_add(60) >= posted_at.saturating_add(3_600) {
        return Err((format!("eth fill 0x{x}: waits: the module's floor is not fresh (posted at {posted_at}): the fill follows the next post"), false));
    }
    // dormant-review DR-M-1: a graduation converts at the module's price: not while its accepted floor is still being walked to
    // our reading (the clamped posts after a long dormancy)
    match st.reading(&c.collection).map(|r| r.post.price) {
        Some(p) if dm::converged(p, accepted) => {}
        got => return Err((format!("eth fill 0x{x}: waits: the module's accepted floor {accepted} has not caught up with our reading ({}): \
            the fill follows the post that brings it within {} bps", got.map_or("none".into(), |p| p.to_string()), dm::BUY_MISMATCH_BPS), false)),
    }
    let factory = factory_of(io, st, c).await.map_err(local)?;
    let n = read_u(io, factory, &dm::COIN_COUNT).await.map_err(local)? as u64;
    let handed: Vec<[u8; 20]> = st.dorm.as_ref().map_or(vec![], |d| d.fills.iter().filter(|(x, ..)| *x == c.collection).map(|(_, k, _)| *k).collect());
    let mut cands: Vec<[u8; 20]> = handed.clone();
    for i in (n.saturating_sub(dm::COINS_PER_FILL_TICK)..n).rev() {
        let coin = read_a(io, factory, &en::call_word(dm::COINS, crate::abi::uint_word(i as u128))).await.map_err(local)?;
        if !cands.contains(&coin) {
            cands.push(coin);
        }
    }
    let mut waits_fill: Vec<String> = Vec::new();
    for coin in cands {
        // a coin of this factory (a hand-in is the operator's word only for WHICH coin; the factory says whether it is one)
        let curve = read_a(io, factory, &dm::call_addr(dm::ROUTER_OF, &coin)).await.map_err(local)?;
        let filled = curve == [0; 20] || read_u(io, c.module, &dm::call_addr(dm::IS_FILLED, &coin)).await.map_err(local)? != 0;
        if !filled && read_u(io, curve, &dm::STATE).await.map_err(local)? == dm::CURVE_GRADUATING {
            match send_home(io, st, Step::Fill { collection: c.collection, coin }, c.module, 0, dm::call_addr(dm::FILL, &coin)).await {
                Ok(s) => {
                    if let Some(d) = st.dorm.as_mut() {
                        d.fills.retain(|(x, k, _)| !(*x == c.collection && *k == coin));
                    }
                    return Ok(Some(format!("eth fill 0x{x}: fill(0x{}) of a graduation queued while the module was stale: {s}", hex::encode(coin))));
                }
                Err(e) if e.contains("would not succeed") => {
                    // independent review 2 (D2-L-2): an estimate that failed is not the module's word (a provider that timed out reads
                    // the same as a revert). The chain was just read at this pin: the coin's graduation is queued and not filled, its
                    // curve Graduating. A hand-in is kept, and tried again at the next hunt; it is dropped only once the chain shows it
                    // filled or its curve no longer Graduating (the loop's end, below)
                    if handed.contains(&coin) {
                        waits_fill.push(format!("fill(0x{}) was not estimated ({e}); the operator's hand-in is kept and tried again at the next \
                            hunt", hex::encode(coin)));
                        continue;
                    }
                }
                Err(e) => return Err((format!("eth fill 0x{x}: {e}"), true)),
            }
        }
        if let Some(d) = st.dorm.as_mut() {
            d.fills.retain(|(x, k, _)| !(*x == c.collection && *k == coin));
        }
    }
    if !waits_fill.is_empty() {
        // the collection's wait: fill_act tries it again after FILL_RETRY_SECS, with the hand-in still held
        return Err((format!("eth fill 0x{x}: waits: {}", waits_fill.join("; ")), false));
    }
    if let Some(l) = st.dorm.as_mut().and_then(|d| d.life_mut(&c.collection)) {
        l.fill_hunt = Some(now.saturating_add(dm::FILL_RETRY_SECS));
        l.note = "a graduation is queued at its module and none of the newest coins nor a hand-in is it: hand in eth_fill(<collection>, \
            <the coin of its GradQueued event>)".into();
    }
    Ok(None)
}

fn desk_ask_mut<'a>(st: &'a mut EthState, c: &[u8; 20], id: u64) -> Option<&'a mut dm::DeskAsk> {
    st.dorm.as_mut().and_then(|d| d.life_mut(c)).and_then(|l| l.desk.as_mut()).filter(|x| x.id == id)
}

/// Independent review DI-M-1: the post that served a collection's desk request is recorded on the desk (`markServed(id)`) inside the
/// request's window, so the bond is decided by the post that served it and never by a later one. Read first: the desk already holding it
/// (anyone may record it), or the module's latest post not after the request (the next post serves it).
async fn mark_act<I: EthIo>(io: &mut I, st: &mut EthState, cfg: &EthNftConfig, cols: &[Col]) -> Result<Option<String>, String> {
    let Some(desk) = cfg.dormancy.as_ref().and_then(|p| p.desk().ok().flatten()) else { return Ok(None) };
    let now = io.now();
    let mut waits = Vec::new();
    for c in cols {
        let Some(a) = st.dorm.as_ref().and_then(|d| d.life(&c.collection)).and_then(|l| l.desk.clone())
            .filter(|a| a.served.is_some() && !a.marked && now < a.until) else { continue };
        let x = hex::encode(c.collection);
        let got = desk_served_at(io, desk, a.id).await;
        match got {
            Ok(0) => {}
            Ok(_) => {
                if let Some(z) = desk_ask_mut(st, &c.collection, a.id) {
                    z.marked = true;
                }
                continue;
            }
            Err(e) => {
                waits.push(format!("eth desk 0x{x}: waits: servedAt({}) not read: {e}", a.id));
                continue;
            }
        }
        let posted = match module_floor(io, c.module).await {
            Ok((_, _, posted)) if posted < a.at => {
                // our post mined before the request after all: the next post serves it
                if let Some(z) = desk_ask_mut(st, &c.collection, a.id) {
                    z.served = None;
                }
                continue;
            }
            Ok((_, _, posted)) => posted,
            Err(e) => {
                waits.push(format!("eth desk 0x{x}: waits: the module's postedAt not read: {e}"));
                continue;
            }
        };
        if posted > a.until {
            // the desk takes only a latest post inside [at, at + window] (D2-L-1), and a module's latest post never moves back
            if let Some(z) = desk_ask_mut(st, &c.collection, a.id) {
                z.marked = true;
            }
            waits.push(format!("eth desk 0x{x}: markServed({}) can no longer be taken: the module's latest post ({posted}) is past the \
                request's window ({}): the desk decides from the module's latest post", a.id, a.until));
            continue;
        }
        match send_step(io, st, Step::Served { collection: c.collection, id: a.id }, Some(*c), desk, 0,
            en::call_word(dm::MARK_SERVED, crate::abi::uint_word(a.id as u128)), home_sol::Call::Served { id: a.id }).await {
            Ok(s) => return Ok(Some(format!("eth desk 0x{x}: markServed({}): the post that served the wake desk's request, recorded in its \
                window: {s}", a.id))),
            Err(e) if e.contains("would not succeed") => {
                // independent review 2 (D2-L-2): an estimate that failed is not taken as the desk's word. A provider that timed out,
                // rate-limited or disagreed reads the same as a revert, so the ask is given up only once the chain confirms markServed
                // cannot be taken: the request closed (its servedAt was read 0 above, and its latest post is inside the window)
                let closed = desk_request(io, desk, a.id).await.map(|(.., closed)| closed);
                match closed {
                    Ok(true) => {
                        if let Some(z) = desk_ask_mut(st, &c.collection, a.id) {
                            z.marked = true;
                        }
                        waits.push(format!("eth desk 0x{x}: markServed({}) would not succeed: the request is closed on the desk", a.id));
                    }
                    Ok(false) => waits.push(format!("eth desk 0x{x}: waits: markServed({}) was not estimated ({e}), though the desk shows it can \
                        be taken (the request open, not recorded, the module's latest post {posted} inside its window): tried again next tick", a.id)),
                    Err(r) => waits.push(format!("eth desk 0x{x}: waits: markServed({}) was not estimated ({e}), and the request was not read \
                        again ({r}): tried again next tick", a.id)),
                }
            }
            Err(e) => return Err(format!("eth desk 0x{x}: {e}")),
        }
    }
    if waits.is_empty() { Ok(None) } else { Err(waits.join("; ")) }
}

/// `eth_fill`: the operator names the coin of a graduation queued at an ACTIVE collection's module (its `GradQueued` event), for the fill
/// hunt to try first. Nothing is trusted but which coin: the factory, the module and the curve are read before anything is signed.
pub fn put_fill(st: &mut EthState, cfg: &EthNftConfig, collection: [u8; 20], coin: [u8; 20], now: u64) -> Result<String, String> {
    if cfg.dormancy.is_none() {
        return Err("refused: eth_nft.dormancy is not configured (a graduation is filled at once while every floor is kept fresh)".into());
    }
    if cfg.col(&collection).is_none() {
        return Err("refused: not a configured collection".into());
    }
    let d = st.dorm.as_mut().ok_or("refused: the collections' lives are decided at the next tick: ask again after it")?;
    d.fills.retain(|(x, k, _)| !(*x == collection && *k == coin));
    if d.fills.len() >= dm::FILLS_MAX {
        return Err(format!("refused: {} fill hand-ins wait already", dm::FILLS_MAX));
    }
    d.fills.push((collection, coin, now));
    if let Some(l) = d.life_mut(&collection) {
        if l.is_active() {
            l.fill_hunt = Some(now);
        }
    }
    Ok(format!("fill hand-in held for 0x{}; tried at the next home act while its module holds a queued graduation", hex::encode(coin)))
}

/// `eth_wake`: the operator asks for a collection's wake (queued; the caps decide when it runs).
pub fn put_wake(st: &mut EthState, cfg: &EthNftConfig, c: [u8; 20], now: u64) -> Result<String, String> {
    if cfg.dormancy.is_none() {
        return Err("refused: eth_nft.dormancy is not configured: every collection is always on".into());
    }
    if cfg.col(&c).is_none() {
        return Err("refused: not a configured collection".into());
    }
    let d = st.dorm.as_mut().ok_or("refused: the collections' lives are decided at the next tick: ask again after it")?;
    d.request(&c, dm::WakeBy::Operator, now)
}

/// §11: the capacity on `balance` (the query reads the canister's balance now; `Dorm.cap` is the last tick's).
pub fn capacity_view(st: &EthState, cfg: &EthNftConfig, p: &dm::Dormancy, balance: Option<u128>, now: u64) -> Option<dm::Capacity> {
    let cols = cfg.cols().ok()?;
    let d = st.dorm.clone().unwrap_or_default();
    Some(dm::capacity(p, &d, balance, cols.len(), pinned_count(st, &cols), now))
}

/// `eth_nft_collections()`: every configured collection's life, with the status the site shows ("warming up (~N min)").
pub fn collections_view(st: &EthState, cfg: &EthNftConfig, now: u64) -> Vec<dm::CollectionLife> {
    let d = st.dorm.as_ref();
    cfg.collections.iter().filter_map(|x| {
        let c = crate::evmtx::parse_address(&x.collection).ok()?;
        let k = kind_of(st, cfg, &c);
        let l = if cfg.dormancy.is_some() { d.and_then(|d| d.life(&c)) } else { None };
        // a Solana wrapper or module is base58: case is meaning there
        let low = |a: &str| if crate::evmtx::parse_address(a).is_ok() { a.to_lowercase() } else { a.to_string() };
        Some(dm::view(k, l, d, now, (x.collection.to_lowercase(), low(&x.wrapper), low(&x.module))))
    }).collect()
}

// ---- the Solana home ------------------------------------------------------------------------------------------------------------

/// `ethflow`'s home on Solana (docs/v9/solana-migration.md §4, phase B1; canister-only since decision D-l, docs/v9/canister-only-solana.md
/// §8): the books keep every read and every step but what moves SOL (`home_books`); here what is left on the money line, Relay's
/// `deposit_native` from our Solana address (the Relay deposit, judged with a Solana origin), and the line's outcomes. A child of
/// `ethflow`, so the flow's own records and rules are used as they are; only what touches the home is here. Nothing here runs on the
/// EVM home.
///
/// Units: on this home every home-chain amount of the flow's records (`EthBuy.advanced`, `Funded.wei`, `Depositing.value`,
/// `Watch.balance_after`, `Returning.wei`, a settle's cost) is in LAMPORTS; every Ethereum amount (`Arrived.wei`, a listing's price,
/// a reading, `min_out` of the forward leg) stays in wei. The two meet only through this tick's SOL/ETH rate (`EthIo::sol_rate`).

mod home_books;

mod home_sol {
    use super::*;
    use crate::relaysol;
    use crate::solhome::{self as sh, SolDepositRec};
    use crate::soltx;

    /// What a module gives the buy rules on this home: the module as the item rules see it (lamports), its accepted floor in wei, and
    /// the rate both were read at.
    pub(super) struct Gate {
        pub view: crate::itemsink::SinkView,
        pub accepted_wei: u128,
        pub rate: u64,
    }

    // ---- the line --------------------------------------------------------------------------------------------------------------------

    /// A home step as the home takes it.
    pub(super) enum Call {
        Advance(u128),
        Deposit(relaysol::SolDeposit),
        Mint { token: Word, tx: [u8; 32] },
        Settle(u128),
        GiveBack { lamports: u128, sweep: bool },
        Record { ticket: u128, tx: [u8; 32] },
        Mark { ticket: u128 },
        Sent { ticket: u128, tx: [u8; 32] },
        Post { price: u64, confidence: u16, depth: u32 },
        Served { id: u64 },
    }

    pub(super) fn u64_of(x: u128, what: &str) -> Result<u64, String> {
        u64::try_from(x).map_err(|_| format!("{what} {x} is over a u64 of lamports"))
    }

    /// A home step: a Relay deposit is signed on the money line (`solhome::ready` / `solhome::sign`: simulated first, so a step that
    /// would fail costs no signature; audit A5-03: the deposit is the dedicated Relay key's alone, the line funds it first) and this side
    /// keeps a copy of what it put out (`EthState.sol_step`); every other step is the books' (`home_books::send`).
    pub(super) async fn send<I: EthIo>(io: &mut I, st: &mut EthState, step: Step, col: &Col, call: Call) -> Result<String, String> {
        let Call::Deposit(d) = call else { return home_books::send(io, st, step, col, call).await };
        // C-I-5: the pause is read again at the signature
        if io.paused() {
            return Err("paused: nothing new is signed".into());
        }
        if st.sol_step.is_some() {
            return Err("this side's step is still out on the Solana line (one at a time)".into());
        }
        let what = sh::LineStep::Eth(step.clone()).label();
        sh::ready(io, sh::LineOwner::Eth, io.now(), &what).await?;
        let ixs = vec![relaysol::deposit_instruction(&io.sol_relay().await?, &d)];
        let now = io.now();
        let (text, p) = sh::sign(io, sh::LineStep::Eth(step), now, ixs).await?;
        st.sol_step = Some(p);
        io.save(st);
        Ok(text)
    }

    /// The line, each tick (`solhome::tick_line`: what landed, what is out, an incident, a preflight refusal, the nonce account's
    /// opening), then this side's outcomes: the books follow a step that moved SOL (`home_books::landed`), then each is resolved as on
    /// the EVM home (`mined_home`); a Relay deposit that landed is remembered for its refund's label.
    pub(super) async fn resolve<I: EthIo>(io: &mut I, st: &mut EthState, cfg: &EthNftConfig) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(t) = sh::tick_line(io, io.now()).await {
            out.push(t);
        }
        let mut l = io.sol_line();
        // 2026-10-12: every chain's side is `LineOwner::Eth`; another side's outcomes stay on the line for it
        let (mine, theirs): (Vec<_>, Vec<_>) = l.take(sh::LineOwner::Eth).into_iter()
            .partition(|o| !matches!(&o.step, sh::LineStep::Eth(s) if io.other_side_step(s)));
        l.outcomes.extend(theirs);
        if mine.is_empty() {
            // this side's step is no longer on the line and no outcome names it (a line restored without it): taken back
            if let Some(p) = st.sol_step.clone().filter(|p| l.pending.as_ref().is_none_or(|x| x.intent != p.intent)) {
                st.sol_step = None;
                if let sh::LineStep::Eth(step) = &p.step {
                    out.push(format!("sol line: {} is no longer on the line and left no outcome: taken back, {}", p.step.label(),
                        { home_books::landed(st, cfg, step, false); mined_home(io, st, cfg, step, false).await }));
                }
            }
            return out;
        }
        for o in mine {
            let sh::LineStep::Eth(step) = o.step.clone() else { continue };
            if st.sol_step.as_ref().is_some_and(|p| p.intent == o.intent) {
                st.sol_step = None;
            }
            let ok = o.ok == Some(true);
            if let (Step::Deposit { buy }, true) = (&step, ok) {
                if let Some(BPhase::Depositing { order_id, .. }) = st.buy(*buy).map(|b| b.phase.clone()) {
                    l.deposits.push(SolDepositRec { buy: *buy, order_id, signature: o.signature.clone(), frog: None });
                }
            }
            io.sol_line_save(&l);
            home_books::landed(st, cfg, &step, ok);
            let r = mined_home(io, st, cfg, &step, ok).await;
            out.push(match o.ok {
                Some(true) => format!("sol line: {} landed ({}): {r}", o.step.label(), o.signature),
                Some(false) => format!("sol line: {} FAILED on chain ({}): {r}", o.step.label(), o.signature),
                None => format!("sol line: {} never landed (abandoned): {r}", o.step.label()),
            });
            l = io.sol_line();
        }
        io.sol_line_save(&l);
        out
    }

    // ---- Relay from Solana (docs/v9/spikes/q5-relay-solana-legs.md) ----------------------------------------------------------------------

    /// A funded buy's quote, judged with a Solana origin (rules S1–S12): the depository's state and its `ProgramData` read at finalized
    /// (S3), our own deposit instruction built from the judged amount and order id alone, signed on the home line.
    pub(super) async fn fund<I: EthIo>(io: &mut I, st: &mut EthState, cfg: &EthNftConfig, b: &EthBuy, c: &Col, wei: u128) -> Result<Option<String>, String> {
        let Some(i) = st.inbox.quotes.iter().position(|(id, ..)| *id == b.id) else { return Ok(None) };
        // audit A5-07: the line's turn first, before the quote is consumed or any read is paid for (a busy line keeps the quote)
        if st.sol_step.is_some() {
            return Err(format!("eth buy {}: the quote waits (kept): this side's step is still out on the Solana line", b.id));
        }
        sh::turn(io, sh::LineOwner::Eth, io.now()).map_err(|e| format!("eth buy {}: the quote waits (kept): {e}", b.id))?;
        let pin = eth_floor(io, st, None).await?;
        let (_, body, at) = st.inbox.quotes.remove(i);
        let state = io.sol_accounts(&[soltx::key(relaysol::RELAY_DEPOSITORY)], 128).await?.pop().flatten()
            .ok_or("Relay's depository state does not exist")?;
        let pd = io.sol_account_head(&soltx::key(relaysol::PROGRAMDATA), 45).await?.ok_or("Relay's ProgramData does not exist")?;
        let (allocator, authority, slot) = relaysol::s3_reads(&state, &pd)?;
        let me = soltx::b58_encode(&io.sol_me().await?);
        let relay = soltx::b58_encode(&io.sol_relay().await?);
        let us = h(&io.our_evm().await?);
        let rate = io.sol_rate().await?;
        let floor = crate::rates::lamports_to_wei(wei, rate).saturating_mul(10_000 - cfg.relay_max_fee_bps as u128) / 10_000;
        let lamports = u64_of(wei, "the deposit")?;
        // 2026-10-12: the order pays out on this side's chain, as Relay names it ("ethereum", "robinhood")
        let dest_chain = crate::relayeth::relay_name(io.eth_chain())
            .ok_or_else(|| format!("eth buy {}: chain {} has no Relay leg in this build", b.id, io.eth_chain()))?;
        let w = relaysol::SolWant { our_sol: &me, depositor: &relay, dest_chain, dest_recipient: &us, dest_currency: crate::relayconv::EVM_NATIVE,
            lamports, min_out_floor: floor, allocator_read: &allocator, upgrade_authority_read: authority.as_deref(), deploy_slot_read: slot,
            now: io.now(), handed_in_at: at, used: &st.used_orders };
        let d = relaysol::judge_sol(&body, &w).map_err(|e| format!("eth buy {}: quote refused, nothing signed: {e}", b.id))?;
        let bal = io.home_balance().await?;
        // the deposit, the Relay key's fee for it, our fee for its funding (audit A5-03)
        let need = d.lamports as u128 + 2 * crate::solsend::SIGNATURE_FEE_LAMPORTS as u128;
        if bal < need {
            return Err(format!("our Solana balance {bal} is under the deposit's {} lamports and its fees ({need})", d.lamports));
        }
        let eth_before = io.eth_balance().await?;
        let request_id = serde_json::from_slice::<serde_json::Value>(&body).ok().and_then(|q| q["steps"][0]["requestId"].as_str().map(String::from))
            .and_then(|x| hex::decode(x.trim_start_matches("0x")).ok()).and_then(|b| <[u8; 32]>::try_from(b).ok()).filter(|r| *r != [0; 32]);
        st.used_orders.push(d.order_id);
        let out_before = st.eth_out;
        {
            let x = st.buy_mut(b.id).expect("open");
            x.orders.push(d.order_id);
            x.phase = BPhase::Depositing { order_id: d.order_id, value: d.lamports as u128, min_out: d.min_out, deadline: d.deadline };
            x.ask = None;
            x.last = Some(Watch { order_id: d.order_id, value: d.lamports as u128, min_out: d.min_out, deadline: d.deadline,
                balance_after: bal.saturating_sub(d.lamports as u128), eth_before, eth_out_before: out_before, pin: Some(pin), solver: None,
                request_id, home_pin: None });
        }
        io.save(st);
        match send(io, st, Step::Deposit { buy: b.id }, c, Call::Deposit(d.clone())).await {
            Ok(s) => {
                st.buy_mut(b.id).expect("open").attempts += 1;
                Ok(Some(format!("eth buy {}: Relay deposit of {} lamports from Solana (order 0x{}, at least {} wei on Ethereum): {s}", b.id, d.lamports,
                    hex::encode(d.order_id), d.min_out)))
            }
            Err(e) => {
                let ask = ask_for(io, wei).await.ok();
                let x = st.buy_mut(b.id).expect("open");
                x.phase = BPhase::Funded { wei, since: io.now() };
                x.ask = ask;
                x.last = None;
                Err(e)
            }
        }
    }

    /// The refund's label (Q5 §5.2): a finalized credit to our Solana address after the deposit, memoed `0x<order id>`. The balance rule
    /// decided the refund; this only says which order the memo names (an unauthenticated label: it decides nothing). Empty when unread.
    pub(super) async fn refund_label<I: EthIo>(io: &mut I, _st: &EthState, order_id: &[u8; 32]) -> String {
        sh::refund_label(io, order_id).await
    }

    // ---- the dormancy's reads (docs/v7/dormant-collections.md on Solana) -------------------------------------------------------------

    /// The books home has no wake desk (docs/v9/canister-only-solana.md O-11: a configured `wake_desk` is refused at install;
    /// `eth_wake` only): a read of one is an error, never a guess.
    const NO_DESK: &str = "the books home has no wake desk (O-11): a dormant collection wakes by eth_wake";

    /// The wake desk's `(window, request_count)`: none on this home.
    pub(super) async fn desk<I: EthIo>(_io: &mut I) -> Result<(u64, u64), String> {
        Err(NO_DESK.into())
    }

    /// A desk request: none on this home.
    pub(super) async fn request<I: EthIo>(_io: &mut I, id: u64) -> Result<([u8; 20], [u8; 20], u64, u64, bool), String> {
        Err(format!("desk request {id}: {NO_DESK}"))
    }
}
