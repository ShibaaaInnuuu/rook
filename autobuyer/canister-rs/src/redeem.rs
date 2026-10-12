//! The custodian's delivery of redeemed frogs (docs/v3/wrapper.md, "Step 3, the flow").
//!
//! Every tick: find the resolved tickets, and for each one owed a frog, find the frog, build and sign the transaction, send it
//! only when `delivery::decide` says Send **at that moment**, believe only Bitcoin, and get the delivery recorded on chain.
//! Everything that touches the world goes through [`CustodyIo`]; this module decides and sequences, and is tested against a
//! fake world that lags, refunds, dies between calls and disagrees with itself (`tests/redeem.rs`).

use crate::abi::{self, Arg};
use crate::custody::{self, Contents, Frog, Location, NotSendable, Outpoint, Output, Utxo};
use crate::delivery::{self, Decision, Intent, TicketRead};
use crate::evmtx::{self, Tx1559};
use crate::looper::{BtcView, EvmView, FEE_MULTIPLE, GAS_MARGIN_PCT};
use crate::psbt;
use crate::sighash;

// ---- the wrapper's and the vault's ABI (selectors checked against `cast sig` in the tests) ----------------------------------

/// `ticketCount()`
pub const TICKET_COUNT: [u8; 4] = [0xcf, 0xbd, 0x90, 0x0f];
/// `ticket(uint256)`
pub const TICKET: [u8; 4] = [0xc1, 0x29, 0xff, 0x32];
/// `recordDelivery(uint256,bytes32)`
pub const RECORD_DELIVERY: [u8; 4] = [0x19, 0xa8, 0x35, 0xd9];
/// `registry()`
pub const REGISTRY: [u8; 4] = [0x7b, 0x10, 0x39, 0x99];
/// `listingId()`
pub const LISTING_ID: [u8; 4] = [0x94, 0x05, 0xdf, 0xdd];
/// `vaultOf(uint256)` (registry)
pub const VAULT_OF: [u8; 4] = [0xda, 0xa0, 0xbf, 0xba];
/// `custodian()` (wrapper)
pub const CUSTODIAN: [u8; 4] = [0x37, 0x5b, 0x74, 0xc3];
/// `acquisition(uint256)` (vault)
pub const ACQUISITION: [u8; 4] = [0xc9, 0xc3, 0xfc, 0x3e];

/// `markUndeliverable(uint256)` (wrapper; r3-contracts, for canister review2 N-2): the custodian attests that no version of a
/// resolved ticket's delivery can confirm, so the ticket may be refunded at `deliverBy + REFUND_GRACE` rather than only after
/// `LATE_DELIVERY_WINDOW`.
pub fn mark_undeliverable(id: u64) -> Vec<u8> {
    let k = evmtx::keccak(b"markUndeliverable(uint256)");
    abi::encode_call([k[0], k[1], k[2], k[3]], &[Arg::Word(abi::uint_word(id as u128))])
}

/// A conflict-out (or the third spend that killed a delivery) this deep is final enough to attest the delivery undeliverable.
pub const ATTEST_DEPTH: u64 = 6;

pub fn call_uint(sel: [u8; 4], v: u128) -> Vec<u8> {
    abi::encode_call(sel, &[Arg::Word(abi::uint_word(v))])
}

/// `recordDelivery(id, txid)`, the txid in display order (as `keeper/live/cli.ts` takes it and an explorer shows it).
pub fn record_delivery(id: u64, txid_display: [u8; 32]) -> Vec<u8> {
    abi::encode_call(RECORD_DELIVERY, &[Arg::Word(abi::uint_word(id as u128)), Arg::Word(txid_display)])
}

/// A ticket's fields as `ticket(uint256)` returns them (a struct with one trailing string, so the return starts with its
/// offset). The clocks and `read_at` are the reader's, filled in by the `Io`.
pub struct TicketFields {
    pub stage: delivery::Stage,
    pub deliver_by: u64,
    pub deliver_by_block: u64,
    pub acquisition_id: u128,
    pub destination: String,
}

pub fn decode_ticket(ret: &[u8]) -> Result<TicketFields, String> {
    let base = abi::uint_at(ret, 0, 0, 32)? as usize;
    let stage = match abi::uint_at(ret, base, 0, 8)? {
        0 => delivery::Stage::None,
        1 => delivery::Stage::Pinned,
        2 => delivery::Stage::Resolved,
        3 => delivery::Stage::Delivered,
        4 => delivery::Stage::Refunded,
        s => return Err(format!("a ticket stage of {s} is not the contract's")),
    };
    let destination = String::from_utf8(abi::bytes_at(ret, base, 9)?).map_err(|_| "a destination that is not text")?;
    Ok(TicketFields {
        stage,
        deliver_by: abi::uint_at(ret, base, 5, 64)? as u64,
        deliver_by_block: abi::uint_at(ret, base, 6, 64)? as u64,
        // an acquisition id, or a deposit's `DEPOSIT_FLAG | n` carried as bit 127 (docs/v3/curve.md, "Canister: …")
        acquisition_id: crate::deposits::frog_id_of_word(abi::word_at(ret, base, 7)?)?,
        destination,
    })
}

/// The inscription id (`assetRef`) of `acquisition(uint256)`'s returned struct, and its amount (1 for an inscription).
pub fn decode_acquisition_ref(ret: &[u8]) -> Result<(u128, String), String> {
    let base = abi::uint_at(ret, 0, 0, 32)? as usize;
    let amount = abi::uint_at(ret, base, 2, 128)?;
    let r = String::from_utf8(abi::bytes_at(ret, base, 5)?).map_err(|_| "an assetRef that is not text")?;
    Ok((amount, r))
}

// ---- the ticket's facts: its state at the fast pin, its events at finality (docs/v5/custody-hardening.md, H4 and H9) -------

/// `Redeemed(uint256 indexed ticketId, address indexed owner, uint64 round, string destination)` (`cast keccak`).
pub const REDEEMED_TOPIC: [u8; 32] = [0x29, 0xdd, 0xec, 0xfe, 0xe6, 0x71, 0x48, 0x43, 0x44, 0xbb, 0xe3, 0xd9, 0xa6, 0x88, 0x4f, 0x71,
    0x78, 0xe2, 0x74, 0x73, 0x5c, 0x93, 0x7b, 0x0a, 0x30, 0xae, 0x43, 0xea, 0xdc, 0x2e, 0x00, 0x0f];

/// The facts `outbound::verify_btc` checks a delivery against (audit D, H-1). H9 asked for the ticket read at the finalized
/// block; the Phase-1 providers cannot serve state that deep (Robinhood keeps ≈ 6,200 blocks, finality is 8,096–12,023
/// behind), so finality comes from the wrapper's own events instead (`ticketlog.rs`):
///
/// - `resolved`: the ticket reads `Resolved` at the fast pin just now (`t`) AND its `Resolved` event is final;
/// - `destination_script`: the ticket's destination at the fast pin; `event_destination_script`: the final `Redeemed` event's;
/// - `names_this_item`: the ticket's acquisition at the fast pin AND the final `Resolved` event's `acquisitionId` are both the
///   one this redemption delivers (two separate reads: the state and the log);
/// - `finalized`: true, meaning the events these rest on are at or below the finalized block.
///
/// After `Resolved` the ticket can only become `Delivered` or `Refunded`, which the rule reads at a fresh fast pin immediately
/// before sending; so no reorganisation can un-redeem it, or change its item or its destination, under a signed delivery. More
/// than one `Redeemed` or `Resolved` event for one ticket is something the contract cannot produce: refused, not guessed at.
pub fn facts_of(logs: &crate::ticketlog::TicketLogs, t: &TicketRead, ticket: u64, acquisition_id: u128, hrp: &str)
    -> Result<crate::outbound::TicketFacts, String> {
    let dests = logs.redeemed_of(ticket);
    let acqs = logs.resolved_of(ticket);
    if dests.len() > 1 {
        return Err(format!("ticket {ticket} has more than one Redeemed event: the contract cannot produce that"));
    }
    if acqs.len() > 1 {
        return Err(format!("ticket {ticket} has more than one Resolved event: the contract cannot produce that"));
    }
    // a ticket not resolved yet may carry no destination the network accepts yet either; the caller waits on `resolved`
    let destination_script = crate::policy::script_of(&t.destination, hrp).unwrap_or_default();
    let event_destination_script = dests.first().map(|d| crate::policy::script_of(d, hrp).unwrap_or_default());
    Ok(crate::outbound::TicketFacts { ticket, resolved: t.stage == delivery::Stage::Resolved && acqs.len() == 1, finalized: true,
        destination_script, event_destination_script,
        names_this_item: t.acquisition_id == acquisition_id && acqs.first() == Some(&acquisition_id) })
}

/// Phase B2: the facts H4 checks a delivery against from a ticket read from the books (`books::ledger::Ticket`): resolved, its
/// destination and its item, all of the one final read (the Solana form of "its state AND its final events": there the account is
/// the final state, and the logs are not load-bearing).
pub fn facts_final(t: &TicketRead, ticket: u64, acquisition_id: u128, hrp: &str) -> crate::outbound::TicketFacts {
    let destination_script = crate::policy::script_of(&t.destination, hrp).unwrap_or_default();
    crate::outbound::TicketFacts { ticket, resolved: t.stage == delivery::Stage::Resolved, finalized: true,
        event_destination_script: Some(destination_script.clone()), destination_script, names_this_item: t.acquisition_id == acquisition_id }
}

// ---- the record -------------------------------------------------------------------------------------------------------------

/// A delivery's re-pricing (audit D, M-1): when it was first signed, when its current bytes were, what they pay, and every
/// earlier version (display txids), each the same frog, fee input and destination with a lower fee. Any version confirming is
/// the delivery.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub struct Bump {
    pub first_at: u64,
    pub last_at: u64,
    /// What the current bytes pay the miner; 0 when not known (a record from before bumps existed).
    pub fee: u64,
    pub earlier: Vec<[u8; 32]>,
}

/// A delivery not confirmed this long after its current bytes were signed is looked at again: the rule on a fresh read, then a
/// fee bump (or, refused, the conflict-out).
pub const BUMP_AFTER_SECS: u64 = 6 * 3600;
/// A bump never pays more than this: the ceiling less room for the conflict-out that may still have to replace it (BIP125 asks
/// the replacement for the replaced fee plus its own size, and a conflict is under the same 25,000-sat ceiling).
pub const MAX_BUMP_FEE_SATS: u64 = custody::MAX_DELIVERY_FEE_SATS - 500;

/// Where a redemption is. Every variant carries what a restart needs.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub enum RPhase {
    /// A resolved ticket, owed a frog. Nothing built or sent.
    Owed,
    /// Signed and saved; being broadcast. The same bytes, and only these, go out until Bitcoin shows them. `bump`: `None` in a
    /// record saved before M-1's fix (its clock starts at the next look).
    Signed {
        txid: [u8; 32],
        raw: Vec<u8>,
        fee_input: Outpoint,
        #[serde(default)]
        bump: Option<Bump>,
    },
    /// Bitcoin has it (a mempool at least).
    Sent {
        txid: [u8; 32],
        raw: Vec<u8>,
        fee_input: Outpoint,
        #[serde(default)]
        bump: Option<Bump>,
    },
    /// Confirmed on Bitcoin; the delivery is not yet recorded on chain.
    Confirmed { txid: [u8; 32] },
    /// Confirmed, and not ours to record: waiting for the keeper (`pending_delivery_records`).
    AwaitingRecord { txid: [u8; 32] },
    /// Our own `recordDelivery` is out.
    Recording { txid: [u8; 32], raw: Vec<u8>, hash: [u8; 32], nonce: u64 },
    /// The ticket is `Delivered` on chain. Finished.
    Done { txid: [u8; 32] },
    /// Refused by the rule. Final. Either nothing of ours was ever saved, or saved bytes were refused and then killed by a
    /// conflict (H8), which the reason names.
    Refused { reason: String },
    /// The frog was delivered on Bitcoin and the ticket refunded on chain: the redeemer has both. Recorded, never retried.
    LostRace { txid: [u8; 32] },
    /// The frog is not where it can safely be sent from, or something the contract could not have produced. Loud.
    Stuck { why: String },
    /// A relist ticket (docs/v6/canister-floor-followups.md §4): its destination is custody itself, so nothing is delivered;
    /// the sale side lists the item. Final here.
    Relist,
    /// H8: the rule refused saved bytes that were sent (evicted, or still in a mempool as the deadline nears). Until they can
    /// never be mined, their fee input is being spent back to custody (`conflict.rs`). Ends `Refused` (the conflict, or a third
    /// spend, confirmed) or `Confirmed` (a version of the delivery confirmed first, and the record path decides `Done` or
    /// `LostRace`).
    Conflicting { reason: String, conflict: crate::conflict::Conflict },
    /// Phase B2, the Solana home: our `record_delivery` is out on the canister-wide Solana line as `intent`.
    SolRecording { txid: [u8; 32], intent: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub struct Redemption {
    pub ticket: u64,
    pub acquisition_id: u128,
    pub destination: String,
    pub phase: RPhase,
    /// The inscription delivered, once located and signed for: what the custody cap book releases when it leaves (H2).
    #[serde(default)]
    pub inscription: Option<String>,
    /// A refused redemption's attestation to the wrapper that its delivery can no longer confirm (`markUndeliverable`; canister
    /// review2 N-2 on the EVM side): until it lands, the ticket is refundable only after `LATE_DELIVERY_WINDOW`. `None`: nothing
    /// to attest (or a record from before it existed).
    #[serde(default)]
    pub attest: Option<Attest>,
    /// Phase B2, the Solana home: the ticket's `sent_tx` names our delivery (`note_sent`): from then on the late window never refunds it.
    #[serde(default)]
    pub noted: Option<bool>,
}

/// Where a refused redemption's `markUndeliverable` is.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub enum Attest {
    /// To be sent once `deep` (our conflict, or the third spend that killed the delivery; display txid) is `ATTEST_DEPTH` deep;
    /// at once when `None` (nothing of ours was ever broadcast).
    Due { deep: Option<[u8; 32]> },
    /// Signed, saved and sent.
    Sent { raw: Vec<u8>, hash: [u8; 32], nonce: u64 },
    /// Mined, or not needed (the ticket no longer reads `Resolved`, or this canister is not the custodian).
    Done { note: String },
    /// Phase B2, the Solana home: `mark_undeliverable` is out on the canister-wide Solana line as `intent`.
    SolSent { intent: u64 },
}

impl Redemption {
    pub fn finished(&self) -> bool {
        matches!(self.phase, RPhase::Done { .. } | RPhase::Refused { .. } | RPhase::LostRace { .. } | RPhase::Stuck { .. } | RPhase::Relist)
    }
    /// An EVM transaction of ours is out for this redemption.
    pub fn evm_pending(&self) -> bool {
        matches!(self.phase, RPhase::Recording { .. }) || matches!(self.attest, Some(Attest::Sent { .. }))
    }
    /// Refused, and its `markUndeliverable` not yet done.
    pub fn attest_pending(&self) -> bool {
        matches!(self.phase, RPhase::Refused { .. }) && matches!(self.attest, Some(Attest::Due { .. } | Attest::Sent { .. } | Attest::SolSent { .. }))
    }
    /// The custody outputs a delivery in flight, or its conflict-out, has claimed: no other send may choose them.
    pub fn reserved(&self) -> Vec<Outpoint> {
        match &self.phase {
            RPhase::Signed { fee_input, .. } | RPhase::Sent { fee_input, .. } => vec![*fee_input],
            RPhase::Conflicting { conflict, .. } => conflict.reserved(),
            _ => vec![],
        }
    }
    /// A confirmed delivery whose record has not landed (for monitoring).
    pub fn record_owed(&self) -> Option<[u8; 32]> {
        match &self.phase {
            RPhase::Confirmed { txid } | RPhase::AwaitingRecord { txid } => Some(*txid),
            _ => None,
        }
    }
    /// Every txid this delivery has been signed as (the current bytes and the earlier, lower-fee versions): the canary matches
    /// custody's outbound transactions against these.
    pub fn txids(&self) -> Vec<[u8; 32]> {
        match &self.phase {
            RPhase::Signed { txid, bump, .. } | RPhase::Sent { txid, bump, .. } => {
                let mut v = bump.as_ref().map(|b| b.earlier.clone()).unwrap_or_default();
                v.push(*txid);
                v
            }
            RPhase::Conflicting { conflict, .. } => {
                let mut v = conflict.earlier.clone().unwrap_or_default();
                v.push(conflict.leaked_txid);
                v.extend(conflict.tx.iter().map(|t| t.txid));
                v
            }
            RPhase::Confirmed { txid } | RPhase::AwaitingRecord { txid } | RPhase::Recording { txid, .. } | RPhase::Done { txid }
            | RPhase::LostRace { txid } | RPhase::SolRecording { txid, .. } => vec![*txid],
            _ => vec![],
        }
    }
    /// Signed, sent or owed: its ticket's final events are still needed (to sign, or to re-sign a bump).
    fn needs_events(&self) -> bool {
        matches!(self.phase, RPhase::Owed | RPhase::Signed { .. } | RPhase::Sent { .. })
    }
}

/// What the custodian keeps between ticks: how far it has read the wrapper's tickets, every redemption, and the ticket-log
/// scanner (audit D, H-1; `None` in a state saved before it existed).
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub struct Custody {
    pub cursor: u64,
    pub redemptions: Vec<Redemption>,
    #[serde(default)]
    pub logs: Option<crate::ticketlog::TicketLogs>,
    /// Ordinals multi-collection (the books home): each Inscriptions listing's own cursor and redemptions (its tickets are numbered
    /// per listing), keyed by its books id (`frogsol::multi`). `None` in a state saved before it, and on the EVM home.
    #[serde(default)]
    pub colls: Option<Vec<(u16, Custody)>>,
}

// ---- the world ---------------------------------------------------------------------------------------------------------------

/// Everything the flow may do to the world. `save` must be durable before the next `await` returns (as `looper::Io`).
#[allow(async_fn_in_trait)]
pub trait CustodyIo: crate::solhome::SolIo {
    /// Phase B2: the frog wrapper's Solana address on the Solana home (its tickets are the books' `Ticket`s, its records and
    /// marks steps on the canister-wide line); `None` on the EVM home.
    fn sol_wrapper(&self) -> Option<crate::soltx::Pubkey> {
        None
    }
    fn now(&self) -> u64;
    fn chain_id(&self) -> u64;
    /// `ticketCount()` at the tick's fast pin, and that pin's block number.
    async fn ticket_count(&mut self) -> Result<(u64, u64), String>;
    /// The ticket at one agreed block, with that block's clocks; `read_at` is `now()`. `fresh`: a new pin now (the rule's
    /// reads, "immediately before sending"); otherwise the tick's pin (the discovery walk).
    async fn read_ticket(&mut self, id: u64, fresh: bool) -> Result<TicketRead, String>;
    /// The inscription id the vault recorded for `acquisition_id`, refused unless it is one inscription.
    async fn inscription_of(&mut self, acquisition_id: u128) -> Result<String, String>;
    async fn locate(&mut self, inscription: &str) -> Result<Option<Location>, String>;
    async fn contents(&mut self, o: &Outpoint) -> Result<Option<Contents>, String>;
    /// The output as every explorer serves it, agreeing, and whether it is spent.
    async fn output(&mut self, o: &Outpoint) -> Result<(Output, bool), String>;
    /// Candidates for a fee input: the canister's fee pool (audit D, H-4), or, when it has none, custody's listed UTXOs. Each is
    /// verified with `output` and `contents` before it is used; nothing here is believed on its own.
    async fn our_utxos(&mut self) -> Result<Vec<Utxo>, String>;
    async fn fee_rate(&mut self) -> Result<u64, String>;
    async fn our_script(&mut self) -> Result<Vec<u8>, String>;
    fn btc_hrp(&self) -> Result<&'static str, String>;
    async fn sign_taproot(&mut self, sighash: [u8; 32]) -> Result<Vec<u8>, String>;
    async fn btc_send(&mut self, raw: &[u8]) -> Result<(), String>;
    /// `txid` in display order.
    async fn btc_seen(&mut self, txid: [u8; 32]) -> BtcView;
    /// Whether our EVM address is the wrapper's custodian, the one party `recordDelivery` accepts.
    async fn may_record(&mut self) -> Result<bool, String>;
    fn wrapper(&self) -> [u8; 20];
    async fn evm_nonce(&mut self) -> Result<u64, String>;
    async fn base_fee(&mut self) -> Result<u128, String>;
    async fn estimate_gas(&mut self, to: [u8; 20], value: u128, data: &[u8]) -> Result<u64, String>;
    async fn sign_evm(&mut self, hash: [u8; 32]) -> Result<(u8, [u8; 32], [u8; 32]), String>;
    async fn send_evm(&mut self, raw: &[u8]) -> Result<(), String>;
    async fn evm_seen(&mut self, hash: [u8; 32]) -> EvmView;
    fn save(&mut self, r: &Redemption);
    fn save_cursor(&mut self, cursor: u64);
    /// The outbound limiter's key of a ticket's delivery: `ticket:<n>`; a world serving one of several Inscriptions listings names its
    /// collection (tickets are numbered per listing), so two collections' ticket `n` are two acts.
    fn ticket_key(&self, ticket: u64) -> String {
        format!("ticket:{ticket}")
    }
    /// Fee inputs another collection's deliveries in flight have claimed (one custody address for every Inscriptions listing): no
    /// delivery of this one may choose them. A world of one collection has none.
    fn reserved_elsewhere(&self) -> Vec<Outpoint> {
        vec![]
    }
    /// Run `f` on the custody caps in force and the book (docs/v5/custody-hardening.md, H2).
    fn with_caps<R>(&mut self, f: impl FnOnce(&crate::custodycap::CapPolicy, &mut crate::custodycap::Book) -> R) -> R;
    /// Ask the outbound limiter (H3); a trip freezes custody (the canister pauses). `Err` is a wait or the freeze.
    fn outflow_check(&mut self, key: &str, collection: &str, flow: crate::outflow::Flow, amount: u128) -> Result<(), String>;
    /// Count an act that went out, once its signed bytes are saved (H3). Idempotent per key.
    fn outflow_record(&mut self, key: &str, collection: &str, flow: crate::outflow::Flow, amount: u128);
    /// Read the wrapper's `Redeemed` and `Resolved` events forward to finality (audit D, H-1; `ticketlog::advance` over the
    /// installed providers). The outcome is recorded in `logs` either way. `wanted`: the tickets a block too big to read whole
    /// is read for (audit R2, N-1); `None` when the tickets were not walked this tick.
    async fn scan_ticket_logs(&mut self, logs: &mut crate::ticketlog::TicketLogs, wanted: Option<&[u64]>) -> Result<String, String>;
    /// The rules the outbound verifier applies to custody's spends (H4).
    fn outbound_policy(&self, custody_script: &[u8]) -> crate::outbound::BtcPolicy;
    /// The buyer's pause (H1; an H3 trip sets it). The conflict-out (H8) signs nothing while it is on.
    fn paused(&self) -> bool;
    /// Who spent an output (display txid) and whether that is confirmed, every explorer agreeing; `None` unspent (H8).
    async fn outspend(&mut self, o: &Outpoint) -> Result<Option<([u8; 32], bool)>, String>;
    /// A custody output of ours, plain sats, that a transaction of ours made and Bitcoin confirmed: a fee input for later (H-4).
    fn fee_pool_add(&mut self, _o: Outpoint, _value: u64) {}
    /// A fee-pool entry found spent.
    fn fee_pool_drop(&mut self, _o: &Outpoint) {}
    /// A fee candidate failed to read while another candidate's read worked (audit R2, N-4): counted against a pool entry, which
    /// is quarantined at `custody::QUARANTINE_AFTER`.
    fn fee_pool_failed(&mut self, _o: &Outpoint, _why: &str) {}
    /// A fee candidate read fine.
    fn fee_pool_ok(&mut self, _o: &Outpoint) {}
    /// Custody's listed UTXOs, for when the pool has unreserved entries and none of them is usable (audit R2, N-4). Empty when
    /// `our_utxos` already listed them (an empty pool).
    async fn listed_utxos(&mut self) -> Result<Vec<Utxo>, String> {
        Ok(vec![])
    }
    /// How deep a transaction is (display txid); `None` not in a block. The default counts a confirmed one as deep enough (the
    /// tests' worlds); the canister reads the explorers' tip and checks the block (`frogflow::depth`).
    async fn btc_depth(&mut self, txid: [u8; 32]) -> Result<Option<u64>, String> {
        match self.btc_seen(txid).await {
            BtcView::Confirmed => Ok(Some(ATTEST_DEPTH)),
            BtcView::Unsure(e) => Err(e),
            _ => Ok(None),
        }
    }
    /// This canister's own scripts that are not custody (the float, the buyer): never a delivery's destination (audit D, I-6).
    async fn not_destinations(&mut self) -> Result<Vec<Vec<u8>>, String> {
        Ok(vec![])
    }
}

// ---- finding the tickets -----------------------------------------------------------------------------------------------------

/// What one discovery walk found: the new redemptions (already saved), the new cursor (saved), and the ticket count with the
/// block it was read at.
pub struct Discovered {
    pub found: Vec<Redemption>,
    pub cursor: u64,
    pub count: u64,
    pub at: u64,
}

/// Walk the wrapper's tickets from `cursor`. Tickets resolve strictly in order, so the walk stops at the first `Pinned` one.
/// A resolved ticket whose destination is custody itself is a relist (`RPhase::Relist`): never owed a delivery, never asked of
/// the limiter.
pub async fn discover<I: CustodyIo>(io: &mut I, cursor: u64, known: &[u64]) -> Result<Discovered, String> {
    let (count, at) = io.ticket_count().await?;
    let mut c = cursor;
    let mut found = Vec::new();
    let mut ours: Option<Vec<u8>> = None;
    while c < count {
        let t = io.read_ticket(c, false).await?;
        match t.stage {
            delivery::Stage::Pinned => break,
            delivery::Stage::Resolved if !known.contains(&c) => {
                if ours.is_none() {
                    ours = Some(io.our_script().await?);
                }
                // audit A5-06: on the Solana home (sales are off) nothing relists: a ticket to custody is owed, and refused there
                let to_custody = io.sol_wrapper().is_none() && crate::policy::script_of(&t.destination, io.btc_hrp()?).ok() == ours;
                let r = Redemption { ticket: c, acquisition_id: t.acquisition_id, destination: t.destination.clone(),
                    phase: if to_custody { RPhase::Relist } else { RPhase::Owed }, inscription: None, attest: None, noted: None };
                io.save(&r);
                found.push(r);
            }
            // delivered or refunded before we first looked (or ours already): nothing owed by this walk
            _ => {}
        }
        c += 1;
        io.save_cursor(c);
    }
    Ok(Discovered { found, cursor: c, count, at })
}

// ---- one redemption, one step --------------------------------------------------------------------------------------------------

/// One pass over one redemption. Never throws: a failure is the redemption waiting this tick, with the reason. `logs`: the
/// ticket events read at finality (the facts H4 checks a signature against).
pub async fn step<I: CustodyIo>(io: &mut I, r: &mut Redemption, other_evm_pending: bool, reserved: &[Outpoint],
    logs: &crate::ticketlog::TicketLogs) -> String {
    match step_inner(io, r, other_evm_pending, reserved, logs).await {
        Ok(s) => format!("ticket {}: {s}", r.ticket),
        Err(e) => format!("ticket {}: waits: {e}", r.ticket),
    }
}

fn intent(r: &Redemption) -> Intent {
    Intent { acquisition_id: r.acquisition_id, destination: r.destination.clone() }
}

/// The rule, on a fresh read, and the read. `Ok((t, None))`: send; `Ok((t, Some(why)))`: refused; `Err`: wait.
async fn ruling<I: CustodyIo>(io: &mut I, r: &Redemption) -> Result<(TicketRead, Option<String>), String> {
    let t = io.read_ticket(r.ticket, true).await?;
    match delivery::decide(&t, &intent(r), io.now()) {
        Decision::Send => Ok((t, None)),
        Decision::Wait { why } => Err(why.to_string()),
        Decision::Refuse(why) => Ok((t, Some(format!("{why:?}")))),
    }
}

/// The rule before anything is saved. `Ok(Some(read))`: send; a refusal is written into the record, final.
async fn rule<I: CustodyIo>(io: &mut I, r: &mut Redemption) -> Result<Option<TicketRead>, String> {
    match ruling(io, r).await? {
        (t, None) => Ok(Some(t)),
        (_, Some(reason)) => {
            // nothing of ours was ever broadcast for it: attestable at once
            r.phase = RPhase::Refused { reason };
            r.attest = Some(Attest::Due { deep: None });
            io.save(r);
            Ok(None)
        }
    }
}

/// The facts for `r` from a fresh read, or a wait that says what the scanner is doing.
fn facts_for<I: CustodyIo>(io: &I, logs: &crate::ticketlog::TicketLogs, t: &TicketRead, r: &Redemption)
    -> Result<crate::outbound::TicketFacts, String> {
    // phase B2: on the Solana home the ticket IS final state (every read is at `finalized`; no event is load-bearing, §2.4)
    if io.sol_home().is_some() {
        return Ok(facts_final(t, r.ticket, r.acquisition_id, io.btc_hrp()?));
    }
    let facts = facts_of(logs, t, r.ticket, r.acquisition_id, io.btc_hrp()?)?;
    if !facts.resolved {
        return Err(format!("the ticket's Resolved event is not final yet; {}", logs.status()));
    }
    if facts.event_destination_script.is_none() {
        return Err(format!("the ticket's Redeemed event is not final yet; {}", logs.status()));
    }
    Ok(facts)
}

async fn step_inner<I: CustodyIo>(io: &mut I, r: &mut Redemption, other_evm_pending: bool, reserved: &[Outpoint],
    logs: &crate::ticketlog::TicketLogs) -> Result<String, String> {
    match r.phase.clone() {
        RPhase::Owed => owed(io, r, reserved, logs).await,
        RPhase::Signed { txid, raw, fee_input, bump } => {
            if let Some(t) = version_confirmed(io, txid, fee_input, bump.as_ref()).await? {
                return confirmed(io, r, t, (t == txid).then_some(raw.as_slice()));
            }
            match io.btc_seen(txid).await {
                BtcView::Confirmed => confirmed(io, r, txid, Some(&raw)),
                BtcView::Mempool => {
                    r.phase = RPhase::Sent { txid, raw, fee_input, bump };
                    io.save(r);
                    let n = sol_note(io, r, txid).await;
                    Ok(format!("Bitcoin has it{n}"))
                }
                BtcView::Unsure(e) => Err(e),
                BtcView::NotYet => {
                    // saved but not seen: the rule again, before the same bytes go again
                    let (t, refusal) = ruling(io, r).await?;
                    if let Some(why) = refusal {
                        return conflict_out(io, r, txid, &raw, fee_input, bump, why, reserved).await;
                    }
                    // audit R2, N-2: a bump no mempool took while the version it replaces sits in one is a pin (or a floor)
                    let pinned = match bump.as_ref().and_then(|b| b.earlier.last().copied()) {
                        Some(prev) if matches!(io.btc_seen(prev).await, BtcView::Mempool) => Some(prev),
                        _ => None,
                    };
                    // M-1: bytes no mempool keeps (evicted below a rising mempool floor) are re-priced like ones that sit there
                    if let Some(b) = bump.filter(|b| io.now().saturating_sub(b.last_at) >= BUMP_AFTER_SECS) {
                        return bump_delivery(io, r, logs, &t, txid, &raw, fee_input, b, reserved).await;
                    }
                    let _ = io.btc_send(&raw).await;
                    match pinned {
                        Some(prev) => Ok(format!("PIN WATCH: the re-priced bytes {} are not taken while the version they replace, {}, \
                            sits in a mempool (a child on output 0 may be pinning it); the same saved bytes sent again", hex::encode(txid),
                            hex::encode(prev))),
                        None => Ok("the same saved bytes sent again; Bitcoin does not show them yet".into()),
                    }
                }
            }
        }
        RPhase::Sent { txid, raw, fee_input, bump } => {
            if let Some(t) = version_confirmed(io, txid, fee_input, bump.as_ref()).await? {
                return confirmed(io, r, t, (t == txid).then_some(raw.as_slice()));
            }
            let view = io.btc_seen(txid).await;
            match view {
                BtcView::Confirmed => confirmed(io, r, txid, Some(&raw)),
                // seen once and gone (evicted): back to Signed, so the rule runs before the same bytes go again
                BtcView::NotYet => {
                    r.phase = RPhase::Signed { txid, raw, fee_input, bump };
                    io.save(r);
                    Ok("Bitcoin no longer shows it: the rule runs again before the same bytes are re-sent".into())
                }
                BtcView::Mempool | BtcView::Unsure(_) => {
                    // M-1: a delivery that sits in a mempool is looked at again every BUMP_AFTER_SECS: refused (the deadline
                    // near, a refund) is a conflict-out even while it is in a mempool; otherwise it is re-priced
                    let now = io.now();
                    let Some(b) = bump else {
                        r.phase = RPhase::Sent { txid, raw, fee_input, bump: Some(Bump { first_at: now, last_at: now, fee: 0, earlier: vec![] }) };
                        io.save(r);
                        return match view { BtcView::Unsure(e) => Err(e), _ => Ok("in a mempool; waiting for a block".into()) };
                    };
                    if now.saturating_sub(b.last_at) < BUMP_AFTER_SECS {
                        if let BtcView::Unsure(e) = view {
                            return Err(e);
                        }
                        let n = sol_note(io, r, txid).await;
                        return Ok(format!("in a mempool; waiting for a block{n}"));
                    }
                    let (t, refusal) = ruling(io, r).await?;
                    if let Some(why) = refusal {
                        return conflict_out(io, r, txid, &raw, fee_input, Some(b), why, reserved).await;
                    }
                    if let BtcView::Unsure(e) = view {
                        return Err(e);
                    }
                    // audit R2, N-2: who spends output 0, the redeemer's, while the delivery waits: a child there can pin it
                    let watch = match io.outspend(&Outpoint { txid, vout: 0 }).await {
                        Ok(Some((child, false))) => format!("PIN WATCH: output 0 of {} has an unconfirmed child {}; ", hex::encode(txid),
                            hex::encode(child)),
                        _ => String::new(),
                    };
                    bump_delivery(io, r, logs, &t, txid, &raw, fee_input, b, reserved).await.map(|s| format!("{watch}{s}"))
                }
            }
        }
        RPhase::Confirmed { txid } | RPhase::AwaitingRecord { txid } => record(io, r, txid, other_evm_pending).await,
        RPhase::Recording { txid, raw, hash, nonce } => recording(io, r, txid, raw, hash, nonce).await,
        RPhase::SolRecording { txid, intent } => sol_recording(io, r, txid, intent).await,
        RPhase::Conflicting { .. } => conflicting(io, r, reserved).await,
        RPhase::Refused { .. } if r.attest_pending() => attest(io, r, other_evm_pending).await,
        _ => Ok(format!("{:?}: nothing further", r.phase)),
    }
}

/// The wrapper is told that a refused redemption's delivery can no longer confirm (`markUndeliverable`), once whatever killed it
/// is `ATTEST_DEPTH` deep, so the ticket's refund is not held for `LATE_DELIVERY_WINDOW` (canister review2 N-2, the wrapper side
/// in r3-contracts). One EVM transaction of ours at a time; saved, then sent; only the chain is believed.
async fn attest<I: CustodyIo>(io: &mut I, r: &mut Redemption, other_evm_pending: bool) -> Result<String, String> {
    match r.attest.clone() {
        Some(Attest::Due { deep }) => {
            if let Some(t) = deep {
                match io.btc_depth(t).await? {
                    Some(d) if d >= ATTEST_DEPTH => {}
                    d => return Ok(format!("markUndeliverable waits: {} is {} of {ATTEST_DEPTH} deep", hex::encode(t), d.unwrap_or(0))),
                }
            }
            let t = io.read_ticket(r.ticket, true).await?;
            if t.stage != delivery::Stage::Resolved {
                r.attest = Some(Attest::Done { note: format!("not needed: the ticket reads {:?}", t.stage) });
                io.save(r);
                return Ok(format!("no markUndeliverable: the ticket reads {:?}", t.stage));
            }
            if !io.may_record().await? {
                r.attest = Some(Attest::Done { note: "not the wrapper's custodian: the ticket refunds after LATE_DELIVERY_WINDOW".into() });
                io.save(r);
                return Ok("no markUndeliverable: this canister is not the wrapper's custodian".into());
            }
            // phase B2: on the Solana home, `mark_undeliverable` on the ONE line
            if let Some(w) = io.sol_wrapper() {
                let now = io.now();
                let (s, p) = crate::frogsol::custody_send(io, w, crate::solhome::CustodyStep::Mark { ticket: r.ticket }, now).await?;
                r.attest = Some(Attest::SolSent { intent: p.intent });
                io.save(r);
                return Ok(s);
            }
            if other_evm_pending {
                return Ok("markUndeliverable waits: another EVM transaction of ours is out (one at a time)".into());
            }
            let data = mark_undeliverable(r.ticket);
            let to = io.wrapper();
            let nonce = io.evm_nonce().await?;
            let base = io.base_fee().await?;
            let est = io.estimate_gas(to, 0, &data).await.map_err(|e| format!("markUndeliverable would not succeed now: {e}"))?;
            let tx = Tx1559 { chain_id: io.chain_id(), nonce, max_priority_fee: 0, max_fee: base.saturating_mul(FEE_MULTIPLE),
                gas: est + est * GAS_MARGIN_PCT / 100, to, value: 0, data };
            let (y, rr, s) = io.sign_evm(tx.signing_hash()).await?;
            let raw = tx.signed(y, &rr, &s);
            let hash = evmtx::keccak(&raw);
            r.attest = Some(Attest::Sent { raw: raw.clone(), hash, nonce });
            io.save(r);
            let _ = io.send_evm(&raw).await;
            Ok(format!("markUndeliverable({}) signed, saved and sent (nonce {nonce})", r.ticket))
        }
        Some(Attest::Sent { raw, hash, nonce }) => {
            let _ = io.send_evm(&raw).await; // the same bytes again; the provider's answer is not believed
            match io.evm_seen(hash).await {
                EvmView::Mined { success } => {
                    let note = format!("markUndeliverable mined ({})", if success { "success" } else { "reverted: the ticket moved on" });
                    r.attest = Some(Attest::Done { note: note.clone() });
                    io.save(r);
                    Ok(note)
                }
                EvmView::NotYet if io.evm_nonce().await? > nonce => {
                    Err(format!("nonce {nonce} is used but markUndeliverable is not visible yet: waiting rather than guessing"))
                }
                EvmView::NotYet => Ok(format!("markUndeliverable sent again (nonce {nonce}), not seen yet")),
                EvmView::Pending => Ok(format!("markUndeliverable pending (nonce {nonce})")),
                EvmView::Unsure(e) => Err(e),
            }
        }
        Some(Attest::SolSent { intent }) => {
            // the ticket says what happened (an account at finalized); while our intent is out it waits; landed failed or never landed,
            // it is due again
            let w = io.sol_wrapper().ok_or("a Solana attestation on a home that is not Solana")?;
            let t = crate::frogsol::ticket(io, w, r.ticket).await?;
            if t.undeliverable || t.stage != crate::books::ledger::Stage::Resolved {
                let note = format!("mark_undeliverable landed (the ticket reads {:?}, undeliverable {})", t.stage, t.undeliverable);
                r.attest = Some(Attest::Done { note: note.clone() });
                io.save(r);
                return Ok(note);
            }
            if crate::frogsol::custody_pending(io, r.ticket) == Some(intent) {
                return Ok(format!("mark_undeliverable is out on the Solana line (intent {intent})"));
            }
            r.attest = Some(Attest::Due { deep: None });
            io.save(r);
            Ok(format!("mark_undeliverable (intent {intent}) did not take: due again"))
        }
        _ => Ok("nothing to attest".into()),
    }
}

/// Phase B2, the Solana home: the delivery is on Bitcoin (a mempool at least): `note_sent(txid)` on the ticket once, so the late window
/// never refunds a ticket whose frog is on its way. Best effort: the line busy, or not the custodian, it waits (the record follows at
/// confirmation either way). Returns a note for the log.
async fn sol_note<I: CustodyIo>(io: &mut I, r: &mut Redemption, txid: [u8; 32]) -> String {
    let Some(w) = io.sol_wrapper() else { return String::new() };
    if r.noted == Some(true) || crate::frogsol::custody_pending(io, r.ticket).is_some() {
        return String::new();
    }
    match crate::frogsol::ticket(io, w, r.ticket).await {
        Ok(t) if t.sent_tx != [0; 32] => {
            r.noted = Some(true);
            io.save(r);
            return "; noted sent on the ticket".into();
        }
        Ok(t) if t.stage != crate::books::ledger::Stage::Resolved => return String::new(),
        Ok(_) => {}
        Err(e) => return format!("; note_sent waits: {e}"),
    }
    let now = io.now();
    match crate::frogsol::custody_send(io, w, crate::solhome::CustodyStep::Sent { ticket: r.ticket, txid }, now).await {
        Ok((s, _)) => format!("; {s}"),
        Err(e) => format!("; note_sent waits: {e}"),
    }
}

/// Phase B2, the Solana home: our `record_delivery` out on the line. The ticket at finalized says what happened (`Delivered`: done;
/// `Refunded`: a lost race); while our intent is out it waits; landed failed or never landed, the record is due again.
async fn sol_recording<I: CustodyIo>(io: &mut I, r: &mut Redemption, txid: [u8; 32], intent: u64) -> Result<String, String> {
    let w = io.sol_wrapper().ok_or("a Solana record on a home that is not Solana")?;
    let t = crate::frogsol::ticket(io, w, r.ticket).await?;
    match t.stage {
        crate::books::ledger::Stage::Delivered => {
            r.phase = RPhase::Done { txid };
            io.save(r);
            release(io, r);
            Ok("recorded on its ticket account: done".into())
        }
        crate::books::ledger::Stage::Refunded => {
            r.phase = RPhase::LostRace { txid };
            io.save(r);
            release(io, r);
            Ok(format!("LOST RACE: {} delivered the frog and the ticket was refunded", hex::encode(txid)))
        }
        _ if crate::frogsol::custody_pending(io, r.ticket) == Some(intent) => Ok(format!("record_delivery is out on the Solana line (intent {intent})")),
        _ => {
            r.phase = RPhase::Confirmed { txid };
            io.save(r);
            Ok(format!("record_delivery (intent {intent}) did not take: recorded again next"))
        }
    }
}

/// With earlier versions out, which version (if any) spent the fee input in a block: that is the one that confirmed.
async fn version_confirmed<I: CustodyIo>(io: &mut I, txid: [u8; 32], fee_input: Outpoint, bump: Option<&Bump>)
    -> Result<Option<[u8; 32]>, String> {
    let Some(b) = bump.filter(|b| !b.earlier.is_empty()) else { return Ok(None) };
    match io.outspend(&fee_input).await? {
        Some((t, true)) if t == txid || b.earlier.contains(&t) => Ok(Some(t)),
        _ => Ok(None),
    }
}

/// A version of the delivery confirmed. Its change, if we know the bytes, joins the fee pool (H-4).
fn confirmed<I: CustodyIo>(io: &mut I, r: &mut Redemption, txid: [u8; 32], raw: Option<&[u8]>) -> Result<String, String> {
    r.phase = RPhase::Confirmed { txid };
    io.save(r);
    if let Some(raw) = raw {
        pool_outputs(io, txid, raw, 1);
    }
    Ok(format!("confirmed on Bitcoin: {}", hex::encode(txid)))
}

/// Our transaction's outputs from `first` on (change or a conflict's output; H4 checked they pay only custody) join the fee pool.
fn pool_outputs<I: CustodyIo>(io: &mut I, txid: [u8; 32], raw: &[u8], first: usize) {
    if let Ok(t) = crate::outbound::parse_tx(raw) {
        for (i, o) in t.outputs.iter().enumerate().skip(first) {
            io.fee_pool_add(Outpoint { txid, vout: i as u32 }, o.value);
        }
    }
}

/// H8 for saved bytes the rule now refuses: they are not sent again, and their fee input is spent back to custody (replacing
/// them if they are still in a mempool), so that no version of them can ever be mined.
#[allow(clippy::too_many_arguments)]
async fn conflict_out<I: CustodyIo>(io: &mut I, r: &mut Redemption, txid: [u8; 32], raw: &[u8], fee_input: Outpoint,
    bump: Option<Bump>, why: String, reserved: &[Outpoint]) -> Result<String, String> {
    let mut conflict = match crate::conflict::Conflict::of(txid, raw, fee_input) {
        Ok(c) => c,
        Err(e) => {
            r.phase = RPhase::Stuck { why: format!("{why}: refused after signing, and the saved bytes cannot be conflicted out: {e}") };
            io.save(r);
            return Ok(format!("STUCK: {:?}", r.phase));
        }
    };
    conflict.earlier = bump.map(|b| b.earlier).filter(|e| !e.is_empty());
    let reason = format!("{why}; signed bytes were saved and sent, then refused: they are not sent again, and their fee input \
        {} is spent back to custody (H8)", fee_input.text());
    r.phase = RPhase::Conflicting { reason, conflict };
    io.save(r);
    conflicting(io, r, reserved).await.map(|s| format!("refused after sending; conflict-out: {s}"))
}

/// M-1: the same delivery at a higher fee. Same frog, same fee input, same destination, output 0 unchanged; the fee at least
/// half as much again, at least BIP125's increment, at least today's rate, at most [`MAX_BUMP_FEE_SATS`], paid from the
/// change. Checked by H4 against the final facts, then the rule again, then saved, then sent. With no change to take it
/// from, or at the ceiling, nothing is re-signed: the margin's conflict-out is the answer.
#[allow(clippy::too_many_arguments)]
async fn bump_delivery<I: CustodyIo>(io: &mut I, r: &mut Redemption, logs: &crate::ticketlog::TicketLogs, t: &TicketRead,
    txid: [u8; 32], raw: &[u8], fee_input: Outpoint, b: Bump, reserved: &[Outpoint]) -> Result<String, String> {
    // H1: a custody signature, so nothing is re-priced while the buyer is paused (a guardian, or an H3 trip)
    if io.paused() {
        return Err("in a mempool; the buyer is paused: not re-priced".into());
    }
    let old = crate::outbound::parse_tx(raw)?;
    let (Some(i0), Some(o0)) = (old.inputs.first(), old.outputs.first()) else { return Err("saved bytes with no frog".into()) };
    let mut frog_txid = i0.txid;
    frog_txid.reverse();
    let frog = Frog { output: Outpoint { txid: frog_txid, vout: i0.vout }, value: o0.value };
    let ours = io.our_script().await?;
    let (fee_out, _) = io.output(&fee_input).await?;
    if fee_out.script != ours {
        return Err(format!("the explorers say fee input {} is not custody's", fee_input.text()));
    }
    let out_sum: u64 = old.outputs.iter().map(|o| o.value).sum();
    let old_fee = (frog.value + fee_out.value).checked_sub(out_sum).ok_or("the saved bytes pay out more than they spend")?;
    let dest = crate::policy::script_of(&r.destination, io.btc_hrp()?)?;
    let vs = custody::vsize(2, &[dest.len(), ours.len()]);
    let rate = io.fee_rate().await?;
    let want = (old_fee * 3 / 2).max(old_fee + vs * crate::conflict::INCREMENTAL_RELAY_SATS_PER_VB + 1).max(vs * rate)
        .min(MAX_BUMP_FEE_SATS);
    if want <= old_fee {
        return Ok(format!("in a mempool at the fee ceiling ({old_fee} sats): not bumped; the margin's conflict-out is next"));
    }
    let fee = Utxo { outpoint: fee_input, value: fee_out.value, confirmed: true };
    let p = match custody::build_with_fee(&frog, &fee, &dest, &ours, want) {
        Ok(p) => p,
        Err(e) => return Ok(format!("in a mempool; not bumped ({e}); the margin's conflict-out is next")),
    };
    let facts = facts_for(io, logs, t, r)?;
    let mut sigs = Vec::new();
    for i in 0..p.tx.inputs.len() {
        let h = sighash::taproot_key_path(&p, i).map_err(|e| format!("digest: {e:?}"))?;
        sigs.push((i, io.sign_taproot(h).await?));
    }
    let new_raw = psbt::finalize(&p, &vec![None; p.tx.inputs.len()], &sigs)?;
    let new_txid = p.txid();
    let pre = vec![
        crate::outbound::Prevout { txid: frog.output.internal_txid(), vout: frog.output.vout, value: frog.value, script: ours.clone() },
        crate::outbound::Prevout { txid: fee.outpoint.internal_txid(), vout: fee.outpoint.vout, value: fee.value, script: ours.clone() },
    ];
    let purpose = crate::outbound::Purpose::Delivery { ticket: facts, item: pre[0].clone() };
    if let Err(e) = crate::outbound::verify_btc(&new_raw, &pre, &purpose, &io.outbound_policy(&ours)) {
        return Ok(format!("the outbound check refused the bump ({e:?}); nothing new saved or sent, the old bytes stand"));
    }
    // the rule AGAIN: signing awaited the subnet, and a guardian may have paused meanwhile
    let (_, refusal) = ruling(io, r).await?;
    if let Some(why) = refusal {
        return conflict_out(io, r, txid, raw, fee_input, Some(b), why, reserved).await;
    }
    if io.paused() {
        return Err("paused while the bump was being signed: nothing new saved or sent".into());
    }
    let now = io.now();
    let mut earlier = b.earlier.clone();
    earlier.push(txid);
    r.phase = RPhase::Signed { txid: new_txid, raw: new_raw.clone(), fee_input,
        bump: Some(Bump { first_at: b.first_at, last_at: now, fee: want, earlier }) };
    io.save(r);
    let _ = io.btc_send(&new_raw).await;
    Ok(format!("re-priced: {} replaces {} at {want} sats (was {old_fee})", hex::encode(new_txid), hex::encode(txid)))
}

async fn owed<I: CustodyIo>(io: &mut I, r: &mut Redemption, reserved: &[Outpoint], logs: &crate::ticketlog::TicketLogs)
    -> Result<String, String> {
    // 1. the rule, before anything is built: a ticket inside the margin is refused here, final
    let Some(t) = rule(io, r).await? else {
        return Ok(format!("refused: {:?}", r.phase));
    };
    // 1b. H4 and H9: the ticket's Redeemed and Resolved events, final. Not there yet is a wait, never a refusal
    let facts = facts_for(io, logs, &t, r)?;
    // 1c. H3: one item out, against the day's limit; a flood of asks trips the freeze. Asked only once the events are in (audit
    // R2, N-12): a ticket waiting behind a failing scan could never be signed, so it is no demand the tripwire should count
    // phase B2: a Solana wrapper's inscriptions are keyed `sol:<wrapper>`
    let collection = match io.sol_wrapper() {
        Some(w) => crate::custodycap::sol_key(&w),
        None => crate::custodycap::btc_key(&io.wrapper()),
    };
    // audit A5-I2: a destination that does not parse for our network never will: refused, final (then marked undeliverable, so the
    // holder is refunded), never an error read again every tick
    let hrp = io.btc_hrp()?;
    let dest = match crate::policy::script_of(&r.destination, hrp) {
        Ok(d) => d,
        Err(e) => {
            r.phase = RPhase::Refused { reason: format!("{:?}: the destination {:?} is not an address of this network ({e})",
                delivery::Refusal::Malformed, r.destination) };
            r.attest = Some(Attest::Due { deep: None });
            io.save(r);
            return Ok(format!("refused: {:?}", r.phase));
        }
    };
    // audit A5-06: on the Solana home (no sales, so nothing relists) a ticket to custody itself is refused like the float's or the
    // buyer's address (it is never a `Relist` there), and marked undeliverable so the holder is refunded
    if io.sol_wrapper().is_some() && dest == io.our_script().await? {
        r.phase = RPhase::Refused { reason: "the destination is this canister's own custody address: nothing relists on the Solana home, \
            so the frog is not sent to itself".into() };
        r.attest = Some(Attest::Due { deep: None });
        io.save(r);
        return Ok(format!("refused: {:?}", r.phase));
    }
    let key = io.ticket_key(r.ticket);
    io.outflow_check(&key, &collection, crate::outflow::Flow::Item, 1)?;
    // 1d. I-6: never to the canister's own float or buyer address, where the frog could never be moved again
    if io.not_destinations().await?.contains(&dest) {
        r.phase = RPhase::Refused { reason: "the destination is this canister's own float or buyer address: a frog sent there could \
            never be moved again".into() };
        r.attest = Some(Attest::Due { deep: None });
        io.save(r);
        return Ok(format!("refused: {:?}", r.phase));
    }
    // 2. the frog: where ord says it is, and what every explorer says about that output
    let id = io.inscription_of(r.acquisition_id).await?;
    custody::check_inscription_id(&id)?;
    let Some(loc) = io.locate(&id).await? else {
        return Err(format!("ord does not know {id} yet"));
    };
    let contents = io.contents(&loc.output).await?;
    let (out, spent) = io.output(&loc.output).await?;
    let ours = io.our_script().await?;
    let frog = match custody::frog_at(&id, &loc, contents.as_ref(), &out, spent, &ours) {
        Ok(f) => f,
        Err(NotSendable::Wait(e)) => return Err(e),
        Err(NotSendable::Stuck(why)) => {
            r.phase = RPhase::Stuck { why: why.clone() };
            io.save(r);
            return Ok(format!("STUCK: {why}"));
        }
    };
    // 3. the fee: one clean, confirmed UTXO of ours, not the frog, not another delivery's
    let rate = io.fee_rate().await?;
    let fee = choose_fee(io, &frog, reserved, rate).await?;
    // 4. the transaction, built and signed here, from the prevouts just verified
    let p = custody::build(&frog, &fee, &dest, &ours, rate)?;
    let paid = (frog.value + fee.value).saturating_sub(p.tx.outputs.iter().map(|o| o.value).sum::<u64>());
    let mut sigs = Vec::new();
    for i in 0..p.tx.inputs.len() {
        let h = sighash::taproot_key_path(&p, i).map_err(|e| format!("digest: {e:?}"))?;
        sigs.push((i, io.sign_taproot(h).await?));
    }
    let raw = psbt::finalize(&p, &vec![None; p.tx.inputs.len()], &sigs)?;
    let txid = p.txid();
    // 4b. H4: the finalized bytes against the finalized ticket, independently of the builder. A refusal is loud and final: it
    // is a bug or a forgery, and nothing is saved or sent.
    let pre = vec![
        crate::outbound::Prevout { txid: frog.output.internal_txid(), vout: frog.output.vout, value: frog.value, script: ours.clone() },
        crate::outbound::Prevout { txid: fee.outpoint.internal_txid(), vout: fee.outpoint.vout, value: fee.value, script: ours.clone() },
    ];
    let purpose = crate::outbound::Purpose::Delivery { ticket: facts, item: pre[0].clone() };
    if let Err(e) = crate::outbound::verify_btc(&raw, &pre, &purpose, &io.outbound_policy(&ours)) {
        r.phase = RPhase::Stuck { why: format!("the outbound check refused the signed delivery: {e:?}; nothing was saved or sent") };
        io.save(r);
        return Ok(format!("STUCK: outbound check: {e:?}"));
    }
    // 5. the rule AGAIN, now: signing awaited the subnet, and "immediately before sending" is a number
    if rule(io, r).await?.is_none() {
        return Ok(format!("refused after signing, nothing saved or sent: {:?}", r.phase));
    }
    // 6. save, then send; only Bitcoin is believed
    let now = io.now();
    r.phase = RPhase::Signed { txid, raw: raw.clone(), fee_input: fee.outpoint,
        bump: Some(Bump { first_at: now, last_at: now, fee: paid, earlier: vec![] }) };
    r.inscription = Some(id.clone());
    io.save(r);
    io.outflow_record(&key, &collection, crate::outflow::Flow::Item, 1);
    let _ = io.btc_send(&raw).await;
    Ok(format!("{id} signed, saved and sent in {} (fee from {})", hex::encode(txid), fee.outpoint.text()))
}

/// One clean fee input: a candidate (the pool, or a listing) that every explorer shows at our script, confirmed, unspent and
/// worth what it claims, and that ord shows carrying nothing (audit D, H-4). A pool entry found spent is dropped. A candidate that
/// cannot be read is skipped, never the end of the choice (audit R2, N-4); when the pool had candidates and none was usable, the
/// address's listing is tried too.
async fn choose_fee<I: CustodyIo>(io: &mut I, frog: &Frog, reserved: &[Outpoint], rate: u64) -> Result<Utxo, String> {
    let utxos = io.our_utxos().await?;
    let ours = io.our_script().await?;
    let cands = custody::fee_candidates(&utxos, &frog.output, reserved, rate);
    let (found, mut why) = first_clean(io, &cands, frog, reserved, &ours).await;
    if let Some(u) = found {
        return Ok(u);
    }
    if !cands.is_empty() {
        match io.listed_utxos().await {
            Ok(listed) => {
                let more: Vec<Utxo> = custody::fee_candidates(&listed, &frog.output, reserved, rate).into_iter()
                    .filter(|u| !cands.iter().any(|c| c.outpoint == u.outpoint)).collect();
                let (found, w) = first_clean(io, &more, frog, reserved, &ours).await;
                if let Some(u) = found {
                    return Ok(u);
                }
                why.extend(w);
            }
            Err(e) => why.push(format!("the listing: {e}")),
        }
    }
    let why = if why.is_empty() { String::new() } else { format!(" [{}]", why.join("; ")) };
    Err(format!("no clean confirmed UTXO of ours can pay the fee (none in the fee pool or listed without inscriptions or runes, \
        unreserved, large enough): the operator registers one with add_custody_fee_input{why}"))
}

/// The first of `cands` that verifies as a clean fee input, and why the others did not read. A read that fails is skipped; it is
/// counted against its pool entry only when another read in the same choice worked (so the explorers were up).
async fn first_clean<I: CustodyIo>(io: &mut I, cands: &[Utxo], frog: &Frog, reserved: &[Outpoint], ours: &[u8])
    -> (Option<Utxo>, Vec<String>) {
    let (mut failed, mut ok) = (Vec::<(Outpoint, String)>::new(), Vec::new());
    let mut found = None;
    for u in cands {
        let (out, spent) = match io.output(&u.outpoint).await {
            Ok(x) => x,
            Err(e) => {
                failed.push((u.outpoint, e));
                continue;
            }
        };
        ok.push(u.outpoint);
        if spent {
            io.fee_pool_drop(&u.outpoint);
            continue;
        }
        if !out.confirmed || out.script != ours || out.value != u.value {
            continue;
        }
        match io.contents(&u.outpoint).await {
            Ok(c) if custody::fee_eligible(u, c.as_ref(), &frog.output, reserved) => {
                found = Some(u.clone());
                break;
            }
            Ok(_) => {}
            Err(e) => failed.push((u.outpoint, format!("ord: {e}"))),
        }
    }
    for o in &ok {
        io.fee_pool_ok(o);
    }
    if !ok.is_empty() {
        for (o, e) in &failed {
            io.fee_pool_failed(o, e);
        }
    }
    (found, failed.into_iter().map(|(o, e)| format!("{}: {e}", o.text())).collect())
}

async fn record<I: CustodyIo>(io: &mut I, r: &mut Redemption, txid: [u8; 32], other_evm_pending: bool) -> Result<String, String> {
    let t = io.read_ticket(r.ticket, true).await?;
    match t.stage {
        delivery::Stage::Delivered => {
            r.phase = RPhase::Done { txid };
            io.save(r);
            release(io, r);
            Ok("recorded on chain: done".into())
        }
        delivery::Stage::Refunded => {
            r.phase = RPhase::LostRace { txid };
            io.save(r);
            release(io, r);
            Ok(format!("LOST RACE: {} delivered the frog and the ticket was refunded", hex::encode(txid)))
        }
        delivery::Stage::Resolved => {
            if !io.may_record().await? {
                if !matches!(r.phase, RPhase::AwaitingRecord { .. }) {
                    r.phase = RPhase::AwaitingRecord { txid };
                    io.save(r);
                }
                return Ok(format!("confirmed; NOT recorded: this canister is not the wrapper's custodian, the only party recordDelivery({}, {}) accepts", r.ticket, hex::encode(txid)));
            }
            // phase B2: on the Solana home, `record_delivery` on the ONE line (allowed while paused: it moves nothing)
            if let Some(w) = io.sol_wrapper() {
                let now = io.now();
                let (s, p) = crate::frogsol::custody_send(io, w, crate::solhome::CustodyStep::Record { ticket: r.ticket, txid }, now).await?;
                r.phase = RPhase::SolRecording { txid, intent: p.intent };
                io.save(r);
                return Ok(s);
            }
            if other_evm_pending {
                return Ok("recordDelivery waits: another EVM transaction of ours is out (one at a time)".into());
            }
            let data = record_delivery(r.ticket, txid);
            let to = io.wrapper();
            let nonce = io.evm_nonce().await?;
            let base = io.base_fee().await?;
            let est = io.estimate_gas(to, 0, &data).await.map_err(|e| format!("recordDelivery would not succeed now: {e}"))?;
            let tx = Tx1559 { chain_id: io.chain_id(), nonce, max_priority_fee: 0, max_fee: base.saturating_mul(FEE_MULTIPLE),
                gas: est + est * GAS_MARGIN_PCT / 100, to, value: 0, data };
            let (y, rr, s) = io.sign_evm(tx.signing_hash()).await?;
            let raw = tx.signed(y, &rr, &s);
            let hash = evmtx::keccak(&raw);
            r.phase = RPhase::Recording { txid, raw: raw.clone(), hash, nonce };
            io.save(r);
            let _ = io.send_evm(&raw).await;
            Ok(format!("recordDelivery signed, saved and sent (nonce {nonce})"))
        }
        s => {
            r.phase = RPhase::Stuck { why: format!("a delivered frog's ticket reads {s:?}, which the contract cannot produce") };
            io.save(r);
            Ok(format!("STUCK: {:?}", r.phase))
        }
    }
}

// ---- H8: the conflict-out ------------------------------------------------------------------------------------------------------

/// One pass of a `Conflicting` redemption through `conflict::step`, and where each ending leaves the record.
async fn conflicting<I: CustodyIo>(io: &mut I, r: &mut Redemption, reserved: &[Outpoint]) -> Result<String, String> {
    let RPhase::Conflicting { reason, mut conflict } = r.phase.clone() else { return Err("not in conflict".into()) };
    let stepped = {
        let mut via = Via { io: &mut *io, r: &mut *r, reason: &reason };
        crate::conflict::step(&mut via, &mut conflict, reserved).await?
    };
    use crate::conflict::{Settled, Stepped};
    match stepped {
        Stepped::Waits(s) => Ok(s),
        Stepped::Settled(Settled::LeakedConfirmed) => {
            // it won the race: a confirmed delivery like any other, so the record path decides Done or LOST RACE
            r.phase = RPhase::Confirmed { txid: conflict.leaked_txid };
            io.save(r);
            pool_outputs(io, conflict.leaked_txid, &conflict.raw, 1);
            Ok(format!("the refused delivery {} confirmed before its conflict: recorded as delivered if the ticket still allows, \
                else a LOST RACE", hex::encode(conflict.leaked_txid)))
        }
        Stepped::Settled(Settled::EarlierConfirmed(t)) => {
            r.phase = RPhase::Confirmed { txid: t };
            io.save(r);
            Ok(format!("an earlier version {} of the refused delivery confirmed before its conflict: recorded as delivered if the \
                ticket still allows, else a LOST RACE", hex::encode(t)))
        }
        Stepped::Settled(Settled::ConflictConfirmed(t)) => {
            r.phase = RPhase::Refused { reason: format!("{reason}; the leaked delivery was conflicted out in {}", hex::encode(t)) };
            r.attest = Some(Attest::Due { deep: Some(t) });
            io.save(r);
            if let Some(tx) = conflict.tx.as_ref().filter(|x| x.txid == t) {
                pool_outputs(io, t, &tx.raw, 0);
            }
            Ok(format!("conflicted out in {}: the refused delivery can never be mined", hex::encode(t)))
        }
        Stepped::Settled(Settled::SpentElsewhere(t)) => {
            r.phase = RPhase::Refused { reason: format!("{reason}; its fee input was spent by {} (confirmed), so the leaked delivery \
                can never be mined", hex::encode(t)) };
            r.attest = Some(Attest::Due { deep: Some(t) });
            io.save(r);
            Ok(format!("the fee input was spent by {}: the refused delivery can never be mined", hex::encode(t)))
        }
        Stepped::Stuck(why) => {
            r.phase = RPhase::Stuck { why: format!("conflict-out: {why}; {reason}") };
            io.save(r);
            Ok(format!("STUCK: conflict-out: {why}"))
        }
    }
}

/// The custodian's world, as the conflict needs it. `save` writes the conflict into the redemption and saves that.
struct Via<'a, I: CustodyIo> {
    io: &'a mut I,
    r: &'a mut Redemption,
    reason: &'a str,
}

impl<I: CustodyIo> crate::conflict::ConflictIo for Via<'_, I> {
    /// A delivery's conflict-out runs while the buyer is paused (audit R2, M-1 (c)): it pays nobody but custody (H4's
    /// `ConflictOut`), with a bounded fee, so all it can do is stop an item leaving. A pause that spans a delivery's margin ends
    /// in the conflict-out, not in a lost race.
    fn paused(&self) -> bool {
        false
    }
    fn now(&self) -> u64 {
        self.io.now()
    }
    async fn seen(&mut self, txid: [u8; 32]) -> BtcView {
        self.io.btc_seen(txid).await
    }
    async fn output(&mut self, o: &Outpoint) -> Result<(Output, bool), String> {
        self.io.output(o).await
    }
    async fn outspend(&mut self, o: &Outpoint) -> Result<Option<([u8; 32], bool)>, String> {
        self.io.outspend(o).await
    }
    async fn utxos(&mut self) -> Result<Vec<Utxo>, String> {
        self.io.our_utxos().await
    }
    async fn contents(&mut self, o: &Outpoint) -> Result<Option<Contents>, String> {
        self.io.contents(o).await
    }
    async fn fee_rate(&mut self) -> Result<u64, String> {
        self.io.fee_rate().await
    }
    async fn custody_script(&mut self) -> Result<Vec<u8>, String> {
        self.io.our_script().await
    }
    async fn sign_custody(&mut self, sighash: [u8; 32]) -> Result<Vec<u8>, String> {
        self.io.sign_taproot(sighash).await // `CustodyIo::sign_taproot` is the custody key
    }
    async fn send(&mut self, raw: &[u8]) {
        let _ = self.io.btc_send(raw).await;
    }
    fn policy(&self, custody_script: &[u8]) -> crate::outbound::BtcPolicy {
        self.io.outbound_policy(custody_script)
    }
    fn save(&mut self, c: &crate::conflict::Conflict) {
        self.r.phase = RPhase::Conflicting { reason: self.reason.to_string(), conflict: c.clone() };
        self.io.save(self.r);
    }
}

/// The delivered frog left custody: its room under the caps is free again (H2).
fn release<I: CustodyIo>(io: &mut I, r: &Redemption) {
    if let Some(i) = &r.inscription {
        let _ = io.with_caps(|_, b| crate::custodycap::release(b, i));
    }
}

async fn recording<I: CustodyIo>(io: &mut I, r: &mut Redemption, txid: [u8; 32], raw: Vec<u8>, hash: [u8; 32], nonce: u64)
    -> Result<String, String> {
    let _ = io.send_evm(&raw).await; // the same bytes again; the provider's answer is not believed
    match io.evm_seen(hash).await {
        EvmView::Mined { success } => {
            // mined either way: the ticket says what happened (a revert may be the keeper recording it first)
            r.phase = RPhase::Confirmed { txid };
            io.save(r);
            Ok(format!("recordDelivery mined ({}); the ticket is read next", if success { "success" } else { "reverted" }))
        }
        EvmView::NotYet => {
            if io.evm_nonce().await? > nonce {
                Err(format!("nonce {nonce} is used but recordDelivery is not visible yet: waiting rather than guessing"))
            } else {
                Ok(format!("recordDelivery sent again (nonce {nonce}), not seen yet"))
            }
        }
        EvmView::Pending => Ok(format!("recordDelivery pending (nonce {nonce})")),
        EvmView::Unsure(e) => Err(e),
    }
}

/// The ticket logs, once a tick (audit D, H-1): not read at all when every ticket is accounted for (the idle skip), else read
/// forward to finality. A failure is loud in the log on the first try and every sixth; in between it is a wait line.
async fn logs_pass<I: CustodyIo>(io: &mut I, c: &Custody, logs: &mut crate::ticketlog::TicketLogs, walked_at: Option<u64>,
    count: Option<u64>) -> Option<String> {
    let lacking = c.redemptions.iter().any(|r| r.needs_events() && !logs.has_both(r.ticket));
    if let (Some(at), false) = (walked_at, lacking) {
        logs.idle_through = Some(logs.idle_through.map_or(at, |x| x.max(at)));
        return None;
    }
    // what a block too big to read whole is read for (audit R2, N-1): every redemption lacking its events, and every ticket not
    // yet walked (its Redeemed event may be in that block), up to the count read at the tick's pin
    let wanted: Option<Vec<u64>> = count.map(|n| c.redemptions.iter().filter(|r| r.needs_events() && !logs.has_both(r.ticket))
        .map(|r| r.ticket).chain(c.cursor..n).collect());
    let r = io.scan_ticket_logs(logs, wanted.as_deref()).await;
    if let Some(d) = logs.dense_status().filter(|_| r.is_ok()) {
        return Some(format!("custody: ticket logs: {d}"));
    }
    match r {
        Ok(_) => None,
        Err(e) if logs.failures == 1 || logs.failures % 6 == 0 => Some(format!("custody: TICKET LOGS NOT READ ({} tries): {e}", logs.failures)),
        Err(e) => Some(format!("custody: ticket logs: waits: {e}")),
    }
}

/// The whole custodian pass: find new resolved tickets, read their final events, then step every unfinished redemption, one
/// EVM transaction of ours at a time (`purchases_evm_pending` is the buyer's side of that rule).
pub async fn tick<I: CustodyIo>(io: &mut I, c: &mut Custody, purchases_evm_pending: bool) -> Vec<String> {
    if io.paused() {
        return paused_tick(io, c, purchases_evm_pending).await;
    }
    let mut lines = Vec::new();
    let known: Vec<u64> = c.redemptions.iter().map(|r| r.ticket).collect();
    let (mut walked_at, mut count) = (None, None);
    match discover(io, c.cursor, &known).await {
        Ok(d) => {
            for r in &d.found {
                lines.push(match r.phase {
                    RPhase::Relist => format!("ticket {}: resolved to custody itself: a relist of frog {} (the sale side lists it)", r.ticket, r.acquisition_id),
                    _ => format!("ticket {}: resolved, owed frog {} to {}", r.ticket, r.acquisition_id, r.destination),
                });
            }
            c.redemptions.extend(d.found);
            c.cursor = d.cursor;
            count = Some(d.count);
            if d.cursor == d.count {
                walked_at = Some(d.at);
            }
        }
        Err(e) => lines.push(format!("custody: tickets not read: {e}")),
    }
    let mut logs = c.logs.take().unwrap_or_default();
    // phase B2: on the Solana home the tickets are final state (accounts at finalized): no event scan; the line once
    let sol = io.sol_home().is_some();
    if sol {
        lines.extend(sol_line_pass(io).await);
    } else if let Some(l) = logs_pass(io, c, &mut logs, walked_at, count).await {
        lines.push(l);
    }
    for i in 0..c.redemptions.len() {
        if c.redemptions[i].finished() && !c.redemptions[i].attest_pending() {
            continue;
        }
        let other = purchases_evm_pending || c.redemptions.iter().enumerate().any(|(j, x)| j != i && x.evm_pending());
        let mut reserved: Vec<Outpoint> = c.redemptions.iter().enumerate().filter(|(j, _)| *j != i).flat_map(|(_, x)| x.reserved()).collect();
        reserved.extend(io.reserved_elsewhere());
        let mut r = c.redemptions[i].clone();
        lines.push(step(io, &mut r, other, &reserved, &logs).await);
        io.save(&r);
        c.redemptions[i] = r;
    }
    // phase B2: no redemption wants the Solana line now (a record, a mark, a note): the custodian's turn, if it held one, is given up
    if sol && !c.redemptions.iter().any(|r| matches!(r.phase, RPhase::Confirmed { .. } | RPhase::AwaitingRecord { .. })
        || matches!(r.attest, Some(Attest::Due { .. })) && matches!(r.phase, RPhase::Refused { .. })
        || matches!(r.phase, RPhase::Sent { .. }) && r.noted != Some(true)) {
        crate::solhome::release(io, crate::solhome::LineOwner::Custody);
    } else if sol {
        // audit A5-08: every redemption that wants the line was stepped: one that reached it took the line; a turn left at the head
        // of a free line is one none of them used (each failed before `ready`), given up
        crate::ethflow::release_untaken_turn(io, crate::solhome::LineOwner::Custody);
    }
    // what the scanner keeps stays bounded: only tickets still to be walked, or whose redemption still needs them
    let keep = |t: u64| t >= c.cursor || c.redemptions.iter().any(|r| r.ticket == t && r.needs_events());
    logs.redeemed.retain(|(t, _)| keep(*t));
    logs.resolved.retain(|(t, _)| keep(*t));
    c.logs = Some(logs);
    lines
}

// ---- while paused (audit R2, M-1 (c)) ----------------------------------------------------------------------------------------

/// The custodian's pass while the buyer is paused (a guardian, the operator, or an H3 trip). Nothing new leaves custody: no
/// delivery is signed, bumped or re-sent, nothing is walked or scanned. What already left is still protected: a delivery in flight
/// is watched, the rule is read again as often as a bump would look, and when it refuses (inside the margin, refunded) the
/// conflict-out is signed and sent (it pays only custody: H4's `ConflictOut`); a conflict is stepped and re-priced; a delivery
/// Bitcoin confirmed is recorded (`recordDelivery` moves nothing, and it is what stops a refund of a delivered ticket). So a
/// pause that spans a delivery's margin ends in the conflict-out or in the record, not in a lost race.
async fn paused_tick<I: CustodyIo>(io: &mut I, c: &mut Custody, purchases_evm_pending: bool) -> Vec<String> {
    let mut lines = Vec::new();
    if io.sol_home().is_some() {
        lines.extend(sol_line_pass(io).await);
    }
    for i in 0..c.redemptions.len() {
        if c.redemptions[i].finished() {
            continue;
        }
        let other = purchases_evm_pending || c.redemptions.iter().enumerate().any(|(j, x)| j != i && x.evm_pending());
        let reserved: Vec<Outpoint> = c.redemptions.iter().enumerate().filter(|(j, _)| *j != i).flat_map(|(_, x)| x.reserved()).collect();
        let mut r = c.redemptions[i].clone();
        lines.push(match paused_step(io, &mut r, other, &reserved).await {
            Ok(s) => format!("ticket {}: {s}", r.ticket),
            Err(e) => format!("ticket {}: waits: {e}", r.ticket),
        });
        io.save(&r);
        c.redemptions[i] = r;
    }
    lines
}

async fn paused_step<I: CustodyIo>(io: &mut I, r: &mut Redemption, other_evm_pending: bool, reserved: &[Outpoint])
    -> Result<String, String> {
    match r.phase.clone() {
        RPhase::Owed => Err("the buyer is paused: nothing is signed for it".into()),
        RPhase::Signed { txid, raw, fee_input, bump } | RPhase::Sent { txid, raw, fee_input, bump } => {
            let was_sent = matches!(r.phase, RPhase::Sent { .. });
            if let Some(t) = version_confirmed(io, txid, fee_input, bump.as_ref()).await? {
                return confirmed(io, r, t, (t == txid).then_some(raw.as_slice()));
            }
            let view = io.btc_seen(txid).await;
            match view {
                BtcView::Confirmed => return confirmed(io, r, txid, Some(&raw)),
                BtcView::Unsure(e) => return Err(e),
                BtcView::Mempool | BtcView::NotYet => {}
            }
            let now = io.now();
            let b = bump.clone().unwrap_or(Bump { first_at: now, last_at: now, fee: 0, earlier: vec![] });
            // where Bitcoin shows it, as the running pass would record it; nothing is sent
            let seen = matches!(view, BtcView::Mempool);
            if seen != was_sent || bump.is_none() {
                r.phase = if seen { RPhase::Sent { txid, raw: raw.clone(), fee_input, bump: Some(b.clone()) } }
                    else { RPhase::Signed { txid, raw: raw.clone(), fee_input, bump: Some(b.clone()) } };
                io.save(r);
            }
            if now.saturating_sub(b.last_at) < BUMP_AFTER_SECS {
                return Err("the buyer is paused: in flight, neither re-sent nor re-priced".into());
            }
            let (_, refusal) = ruling(io, r).await?;
            match refusal {
                Some(why) => conflict_out(io, r, txid, &raw, fee_input, Some(b), why, reserved).await
                    .map(|s| format!("while paused: {s}")),
                None => Err("the buyer is paused: in flight and still allowed by the rule; neither re-sent nor re-priced".into()),
            }
        }
        RPhase::Confirmed { txid } | RPhase::AwaitingRecord { txid } => record(io, r, txid, other_evm_pending).await,
        RPhase::Recording { txid, raw, hash, nonce } => recording(io, r, txid, raw, hash, nonce).await,
        RPhase::SolRecording { txid, intent } => sol_recording(io, r, txid, intent).await,
        RPhase::Conflicting { .. } => conflicting(io, r, reserved).await,
        _ => Ok(format!("{:?}: nothing further", r.phase)),
    }
}

/// Phase B2: the canister-wide Solana line once the custodian's pass (`solhome::tick_line`); the custodian's own outcomes are taken and
/// dropped (its records, marks and notes are read back from the ticket accounts, never from a landing).
async fn sol_line_pass<I: CustodyIo>(io: &mut I) -> Vec<String> {
    let mut out = Vec::new();
    let now = io.now();
    if let Some(t) = crate::solhome::tick_line(io, now).await {
        out.push(format!("custody: {t}"));
    }
    let mut l = io.sol_line();
    if !l.take(crate::solhome::LineOwner::Custody).is_empty() {
        io.sol_line_save(&l);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIX: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/delivery/");
    fn hexfx(name: &str) -> Vec<u8> {
        let s = std::fs::read_to_string(format!("{FIX}{name}")).expect("fixture");
        hex::decode(s.trim().trim_start_matches("0x")).expect("hex")
    }

    #[test]
    fn the_calls_and_returns_match_foundrys_encoding() {
        // selectors from `cast sig`
        for (sel, want) in [(TICKET_COUNT, "cfbd900f"), (TICKET, "c129ff32"), (RECORD_DELIVERY, "19a835d9"), (REGISTRY, "7b103999"),
            (LISTING_ID, "9405dfdd"), (VAULT_OF, "daa0bfba"), (ACQUISITION, "c9c3fc3e"), (CUSTODIAN, "375b74c3")] {
            assert_eq!(hex::encode(sel), want);
        }
        // `cast calldata "recordDelivery(uint256,bytes32)" 7 0x717d…416a`
        let txid: [u8; 32] = hex::decode("717d25914a1191c30f4c1d709e3ec0e1fab79cc7bf92cd12a93bed68db63416a").unwrap().try_into().unwrap();
        assert_eq!(record_delivery(7, txid), hexfx("record-delivery.hex"));
        // a Resolved ticket, as `cast abi-encode` builds the struct the contract returns
        let t = decode_ticket(&hexfx("ticket-resolved.hex")).unwrap();
        assert_eq!((t.stage, t.deliver_by, t.deliver_by_block, t.acquisition_id), (delivery::Stage::Resolved, 1_790_604_800, 23_050_400, 7));
        assert_eq!(t.destination, "bc1p5cyxnuxmeuwuvkwfem96lqzszd02n6xdcjrs20cac6yqjjwudpxqkedrcr");
        let (amount, r) = decode_acquisition_ref(&hexfx("acquisition.hex")).unwrap();
        assert_eq!((amount, r.as_str()), (1, "6fb976ab49dcec017f1e201e84395983204ae1a7c2abf7ced0a85d692e442799i0"));
        // a frog minted by wrapDeposit resolves to `DEPOSIT_FLAG | n`: carried through, where it was refused as too wide
        let mut dep = hexfx("ticket-resolved.hex");
        let w7 = 32 + 7 * 32;
        dep[w7] = 0x80;
        assert_eq!(decode_ticket(&dep).unwrap().acquisition_id, crate::deposits::DEPOSIT_BIT | 7);
        // a stage the contract cannot return is refused
        let mut bad = hexfx("ticket-resolved.hex");
        bad[63] = 9;
        bad[32 + 31] = 9;
        assert!(decode_ticket(&bad).is_err());
    }

    const REDEEMED_DATA: &str = "0x00000000000000000000000000000000000000000000000000000000000000030000000000000000000000000000000000000000000000000000000000000040000000000000000000000000000000000000000000000000000000000000003e62633170356379786e75786d65757775766b7766656d39366c717a737a6430326e367864636a727332306361633679716a6a7775647078716b65647263720000";
    const W: [u8; 20] = [0xcc; 20];
    const DEST: &str = "bc1p5cyxnuxmeuwuvkwfem96lqzszd02n6xdcjrs20cac6yqjjwudpxqkedrcr";

    fn word(v: u128) -> String {
        format!("0x{}", hex::encode(abi::uint_word(v)))
    }
    fn redeemed(ticket: u64, block: u64) -> crate::evmread::LogView {
        crate::evmread::LogView { address: format!("0x{}", hex::encode(W)), topics: vec![format!("0x{}", hex::encode(REDEEMED_TOPIC)),
            word(ticket as u128), word(0)], data: REDEEMED_DATA.into(), block, removed: false }
    }
    fn resolved(ticket: u64, acq: u128, block: u64) -> crate::evmread::LogView {
        crate::evmread::LogView { address: format!("0x{}", hex::encode(W)), topics: vec![format!("0x{}", hex::encode(crate::ticketlog::resolved_topic())),
            word(ticket as u128), word(acq)], data: "0x".into(), block, removed: false }
    }
    fn read(stage: delivery::Stage, acq: u128) -> TicketRead {
        TicketRead { stage, acquisition_id: acq, destination: DEST.into(), deliver_by: 2, deliver_by_block: 2, chain_time: 1,
            parent_block: 1, read_at: 1 }
    }

    #[test]
    fn the_redeemed_topic_is_the_events_keccak() {
        assert_eq!(crate::evmtx::keccak(b"Redeemed(uint256,address,uint64,string)"), REDEEMED_TOPIC);
    }

    #[test]
    fn the_facts_are_the_tickets_state_at_the_fast_pin_and_its_events_at_finality() {
        let dest = crate::policy::script_of(DEST, "bc").unwrap();
        let mut logs = crate::ticketlog::TicketLogs::default();
        // no events yet: not resolved (the flow waits), no event destination
        let f = facts_of(&logs, &read(delivery::Stage::Resolved, 7), 0, 7, "bc").unwrap();
        assert!(!f.resolved && f.event_destination_script.is_none() && !f.names_this_item);
        // both final: resolved, the event's destination, the item named by the state AND the Resolved log
        logs.ingest(&[redeemed(0, 10), resolved(0, 7, 11)]).unwrap();
        let f = facts_of(&logs, &read(delivery::Stage::Resolved, 7), 0, 7, "bc").unwrap();
        assert!(f.resolved && f.finalized && f.names_this_item);
        assert_eq!((f.destination_script.clone(), f.event_destination_script.clone()), (dest.clone(), Some(dest)));
        // the state and the log disagree about the item: not this item (H4 refuses)
        assert!(!facts_of(&logs, &read(delivery::Stage::Resolved, 8), 0, 8, "bc").unwrap().names_this_item, "the log says 7");
        assert!(!facts_of(&logs, &read(delivery::Stage::Resolved, 7), 0, 8, "bc").unwrap().names_this_item);
        // refunded or delivered at the fast pin: not resolved, whatever the events say
        assert!(!facts_of(&logs, &read(delivery::Stage::Refunded, 7), 0, 7, "bc").unwrap().resolved);
        // two different Redeemed or Resolved events for one ticket: refused, not guessed at
        let mut two = logs.clone();
        two.redeemed.push((0, "bc1pother".into()));
        assert!(facts_of(&two, &read(delivery::Stage::Resolved, 7), 0, 7, "bc").unwrap_err().contains("more than one Redeemed"));
        let mut two = logs.clone();
        two.ingest(&[resolved(0, 9, 12)]).unwrap();
        assert!(facts_of(&two, &read(delivery::Stage::Resolved, 7), 0, 7, "bc").unwrap_err().contains("more than one Resolved"));
    }
}
