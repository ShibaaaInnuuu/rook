//! The sequencer for item sinks, the mint desk and holder deposits (docs/v3/curve.md, "Canister: …").
//!
//! Every tick: settle our one EVM transaction if one is out; read the sinks and the desk at one pin; move each frog on
//! Bitcoin (offer → signed → sent → six deep and ord-verified → held); give held frogs to whatever can take them, and start at
//! most one purchase; scan custody for marked deposits and move each; then send at most ONE EVM transaction of ours, the most
//! urgent. Everything that touches the world goes through [`FrogIo`]; the decisions are `frogbuy`, `itemsink`, `mintdesk` and
//! `deposits`'s. Tested against a fake world in `tests/frogs.rs`.

use crate::custody::{self, Outpoint};
use crate::deposits::{self, DPhase, DepositRec, Marked, Reason};
use crate::evmtx::{self, Tx1559};
use crate::floortrigger::{self, FloorItem, FloorState, WPhase};
use crate::frogbuy::{self, Bought, FPhase, FrogRec, Target, CONFIRMATIONS};
use crate::itemsink::{self, Clock, SinkKind, SinkRec, SinkView};
use crate::looper::{EvmView, FEE_MULTIPLE, GAS_MARGIN_PCT};
use crate::mintdesk::{self, DStatus, ReqRec};
use crate::psbt::{self, Psbt};
use crate::btcnet::BtcSeen;
use crate::custodycap;
use crate::outflow::Flow;
use crate::handin::{HandinState, Verdict};
use crate::sale::SaleState;

/// Bounds per tick, so one tick's outcalls stay bounded however many coins and requests exist.
pub const MAX_NEW_COINS_PER_TICK: u64 = 8;
pub const MAX_SINKS_READ_PER_TICK: usize = 8;
pub const MAX_NEW_REQUESTS_PER_TICK: u64 = 8;
pub const MAX_REQUESTS_READ_PER_TICK: usize = 8;
pub const MAX_NEW_OUTPUTS_PER_TICK: usize = 8;
/// A frog buy signed this long ago and not confirmed is abandoned (audit D, L-5): a seller who keeps a conflicting spend alive
/// in mempools would otherwise hold the float, and every purchase behind it, for ever.
pub const BUY_STALE_SECS: u64 = 48 * 3600;
/// A protected sale handed back this long ago and not seen by Bitcoin is abandoned (docs/v8/ow-protected.md §3.3): Ordinals
/// Wallet relays at submit, so a sale it has not relayed in six hours is not coming.
pub const PROTECTED_STALE_SECS: u64 = 6 * 3600;

/// Which of our EVM calls a saved transaction is.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub enum FStep {
    /// `wrapDeposit` of a bought frog to its target.
    Mint { frog: u64 },
    /// `recordItem` / `recordFrog` on the frog's sink.
    Report { frog: u64 },
    /// `fulfill(request, cost)` on the desk.
    Fulfill { frog: u64, request: u64 },
    /// The sink's refund trigger (`refundExpired` / `openRefunds`).
    Refunds { sink: [u8; 20] },
    /// The sink's settlement (`graduate`).
    Settle { sink: [u8; 20] },
    /// `wrapDeposit` of a holder's deposit to its recipient.
    Deposit { outpoint: Outpoint },
    /// `itemArrived(cost)` on a Floor Module for an EVM item that arrived (docs/v5/floor-feed.md §6.2).
    FloorClaim { want: u64 },
    /// `claim(id)` on an OfferBoard: the hand-in's purchase is held (docs/v6/offer-handins.md).
    Claim { board: [u8; 20], id: u64 },
    /// `slash(id)` on an OfferBoard: the hand-in was invalid and the board's delay has passed.
    Slash { board: [u8; 20], id: u64 },
    /// `redeemToRelist()` on the wrapper for a protocol sale (docs/v6/canister-floor-followups.md §3.2); our ticket is the
    /// first at or after `count_before` whose owner is us.
    Relist { sale: u64, count_before: u64 },
    /// `recordDelivery(ticket, keccak(listing))`: the delivery-by-listing (§3.3).
    SaleRecord { sale: u64 },
    /// `itemSold{value}` on the module or `recordSale{value}` on the vault (§3.4, §4).
    SaleReport { sale: u64 },
    /// `wrapDeposit` of a delisted item back to the vault or to the canister (§3.3).
    SaleDeposit { sale: u64 },
    /// `reclaimListing(ticket)` on the vault after the re-deposit (§4).
    SaleReclaim { sale: u64 },
    /// `advance(amount)` on a Floor Module: its own ETH for a frog, before the frog (docs/v6/relay-conversion.md).
    Advance { frog: u64 },
    /// `depositNative` to Relay's depository: the advance converted to BTC at the float.
    Convert { frog: u64 },
    /// `returnAdvance{value}` on the module: the conversion failed; the ETH goes back.
    GiveBack { frog: u64 },
    /// `fundReserve{value}` on the module: a written-off order's late refund (conversion review C-03).
    LateCredit { frog: u64 },
    /// Phase B2, the Solana home only: `post_floor` on the frog listing's module in the books, from a BTC floor reading in sats
    /// converted at SOL/BTC.
    Post,
}

impl FStep {
    /// The frog a step is for, when it names one (frog ids are unique canister-wide: the Solana line's outcome of such a step belongs to
    /// the collection whose state holds the frog).
    pub fn frog_id(&self) -> Option<u64> {
        match self {
            FStep::Mint { frog } | FStep::Report { frog } | FStep::Fulfill { frog, .. } | FStep::Advance { frog } | FStep::Convert { frog }
            | FStep::GiveBack { frog } | FStep::LateCredit { frog } => Some(*frog),
            _ => None,
        }
    }
    /// For the log: addresses in hex.
    pub fn text(&self) -> String {
        let h = |a: &[u8; 20]| format!("0x{}", hex::encode(a));
        match self {
            FStep::Mint { frog } => format!("wrapDeposit of frog {frog}"),
            FStep::Report { frog } => format!("the report of frog {frog}"),
            FStep::Fulfill { frog, request } => format!("fulfill of request {request} with frog {frog}"),
            FStep::Refunds { sink } => format!("the refund trigger of {}", h(sink)),
            FStep::Settle { sink } => format!("the settlement of {}", h(sink)),
            FStep::Deposit { outpoint } => format!("wrapDeposit of deposit {}", outpoint.text()),
            FStep::FloorClaim { want } => format!("the claim of NFT want {want}"),
            FStep::Claim { board, id } => format!("the claim of hand-in {id} on {}", h(board)),
            FStep::Slash { board, id } => format!("the slash of hand-in {id} on {}", h(board)),
            FStep::Relist { sale, .. } => format!("redeemToRelist for sale {sale}"),
            FStep::SaleRecord { sale } => format!("the delivery-by-listing record of sale {sale}"),
            FStep::SaleReport { sale } => format!("the report of sale {sale}"),
            FStep::SaleDeposit { sale } => format!("the re-deposit of sale {sale}"),
            FStep::SaleReclaim { sale } => format!("the reclaim of sale {sale}"),
            FStep::Advance { frog } => format!("the advance for frog {frog}"),
            FStep::Convert { frog } => format!("the Relay deposit for frog {frog}"),
            FStep::GiveBack { frog } => format!("the return of frog {frog}'s advance"),
            FStep::LateCredit { frog } => format!("the late refund of frog {frog}'s written-off order"),
            FStep::Post => "the frog listing's floor post".into(),
        }
    }
    /// For the Solana line's log: the rook_core instruction each step is on the Solana home (phase B2).
    pub fn sol_text(&self) -> String {
        match self {
            FStep::Mint { frog } => format!("mint_item (frog {frog})"),
            FStep::Report { frog } => format!("settle_advance / item_arrived (frog {frog})"),
            FStep::Advance { frog } => format!("advance (frog {frog})"),
            FStep::Convert { frog } => format!("Relay deposit_native, SOL to BTC (frog {frog})"),
            FStep::GiveBack { frog } => format!("return_advance (frog {frog})"),
            FStep::LateCredit { frog } => format!("fund_reserve (frog {frog}'s written-off order)"),
            FStep::Post => "post_floor (the frog listing, as the registry's keeper)".into(),
            s => s.text(),
        }
    }
}

/// Our one EVM transaction, saved before it was sent.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub struct FrogEvm {
    pub step: FStep,
    pub raw: Vec<u8>,
    pub hash: [u8; 32],
    pub nonce: u64,
}

/// Everything this side keeps in stable memory.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub struct FrogState {
    /// Set once each, from nothing, by a controller (`set_item_config`).
    pub factory: Option<[u8; 20]>,
    pub desk: Option<[u8; 20]>,
    pub coin_cursor: u64,
    pub sinks: Vec<SinkRec>,
    pub poll: u64,
    pub request_cursor: u64,
    pub requests: Vec<ReqRec>,
    pub frogs: Vec<FrogRec>,
    pub next_frog: u64,
    pub deposits: Vec<DepositRec>,
    /// Custody outputs already classified (a deposit, or not).
    pub seen: Vec<Outpoint>,
    /// Inscription-bearing custody outputs with no `EWD1` marker: never minted or returned here; for the reconciliation.
    pub unmarked: Vec<Outpoint>,
    /// Membership proofs handed in, by inscription id; verified against the root at every use.
    pub proofs: Vec<(String, Vec<[u8; 32]>)>,
    /// When each holder deposit was minted (the rate limit).
    pub mint_times: Vec<u64>,
    /// When each refused deposit's return was sent: returns pay custody's fee float, so they are limited too.
    pub return_times: Vec<u64>,
    pub pending: Option<FrogEvm>,
    /// Custody outputs whose transaction could not be read `deposits::MAX_READ_FAILURES` times (audit D, H-3): set aside, not
    /// retried every tick; for the reconciliation. `None` in a state saved before it existed.
    #[serde(default)]
    pub unread: Option<Vec<Outpoint>>,
    /// Failed reads so far per custody output not yet classified.
    #[serde(default)]
    pub read_failures: Option<Vec<(Outpoint, u32)>>,
    /// When each frog buy was first seen signed and unconfirmed (frog id, time): the L-5 clock. Cleared as it leaves.
    #[serde(default)]
    pub buy_clock: Option<Vec<(u64, u64)>>,
    /// Relay conversions (docs/v6/relay-conversion.md §3.2), one unfinished at a time. `None` in a state saved before them.
    #[serde(default)]
    pub convs: Option<Vec<crate::relayconv::ConvRec>>,
    /// Protected Ordinals Wallet buys signed and handed back (docs/v8/ow-protected.md §3.2–3.3), one per frog, kept from signing
    /// until the frog is held or ends. `None` in a state saved before them.
    #[serde(default)]
    pub protected: Option<Vec<ProtectedRec>>,
    /// Protected-listing review P-01 (2026-10-09): the frog collection's creator addresses (mainnet segwit), pinned by a controller
    /// (`set_ow_creators`): the only recipients a protected sale may pay a royalty to; the listing's own `creator_address` is never
    /// believed. `None` or empty: a sale carrying a royalty is refused.
    #[serde(default)]
    pub ow_creators: Option<Vec<String>>,
    /// Phase B2, the Solana home: this side's step out on the canister-wide Solana line (a copy of the line's pending record).
    #[serde(default)]
    pub sol_step: Option<crate::solhome::LinePending>,
    /// Phase B2: a BTC floor reading for the frog listing, posted by the canister when the registry names it the keeper.
    #[serde(default)]
    pub floor_reading: Option<crate::frogsol::FrogReading>,
    /// Phase B2: what each frog's purchase was counted at in wei (custody caps, the outbound limiter), when its cost is in lamports.
    #[serde(default)]
    pub values: Option<Vec<(u64, u128)>>,
    /// Ordinals multi-collection (docs/v9/ordinals-collections.md §4, on the books home): each Inscriptions listing's own frog state,
    /// keyed by its books collection id (`frogsol::multi`). The top-level state then keeps only what is custody-wide (`seen`, `unmarked`,
    /// `deposits`, the rate-limit clocks, `next_frog`, the floor sinks as configured). `None` in a state saved before it (and on the
    /// EVM home, which keeps the single state): `post_upgrade` moves the single state into the entry of the one Inscriptions listing.
    #[serde(default)]
    pub colls: Option<Vec<(u16, FrogState)>>,
    /// The Inscriptions listings' life (`dormancy.rs`'s `Life`, keyed by the first 20 bytes of the listing's wX mint): a dormant one
    /// gets no floor reading, no book read and no buy; the operator wakes it (`frog_wake`). `None`: none decided yet.
    /// Default-live (operator decision 2026-10-12): a listing with no life is LIVE; `frog_sleep` gives it a dormant one.
    #[serde(default)]
    pub dorm: Option<crate::dormancy::Dorm>,
    /// Audit C2-07 (top-level state only): the Inscriptions collection that last took the Relay leg or the line (the rotation's cursor,
    /// `frogsol::multi::in_turn`). `None` in a state saved before it.
    #[serde(default)]
    pub turn: Option<u16>,
    /// Audit C2-07 (top-level state only): the Ethereum side's turn on the Relay leg is due (`Some(0)`: an Inscriptions collection took
    /// the last leg; `Some(t)`: the leg free for it since `t`), `frogsol::multi::eth_turn_now`. `None`: not due.
    #[serde(default)]
    pub eth_turn: Option<u64>,
}

/// Review P-01: the most creator addresses a controller may pin.
pub const MAX_OW_CREATORS: usize = 8;

impl FrogState {
    /// Review P-01: the pinned creators' scripts (each was checked when it was pinned).
    pub fn ow_creator_scripts(&self) -> Vec<Vec<u8>> {
        self.ow_creators.as_deref().unwrap_or_default().iter().filter_map(|a| crate::policy::script_of(a, "bc").ok()).collect()
    }
    /// Review P-01: `set_ow_creators`'s pure part: every address a mainnet segwit one (protected listings are mainnet's), at most
    /// `MAX_OW_CREATORS`; replaces the list.
    pub fn set_ow_creators(&mut self, v: Vec<String>) -> Result<String, String> {
        if v.len() > MAX_OW_CREATORS {
            return Err(format!("refused: {} creator addresses, over {MAX_OW_CREATORS}", v.len()));
        }
        for a in &v {
            crate::policy::script_of(a, "bc").map_err(|_| format!("refused: {a:?} is not a mainnet segwit address"))?;
        }
        let n = v.len();
        self.ow_creators = Some(v);
        Ok(format!("{n} creator address(es) pinned for protected-listing royalties"))
    }
}

/// A protected buy as signed (docs/v8/ow-protected.md §3.2): the PSBTs handed back to the runner, and what the stale path (§3.3)
/// needs to kill them without ever holding the final transaction.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub struct ProtectedRec {
    pub frog: u64,
    /// Display order: the transaction watched for.
    pub sale_txid: [u8; 32],
    /// The sale PSBT with the float's `0x13`/`0x17` added to its float inputs, otherwise as Ordinals Wallet built it.
    pub sale_psbt: Vec<u8>,
    /// The setup, likewise, when the build had one; its txid (display order).
    pub setup_psbt: Option<Vec<u8>>,
    pub setup_txid: Option<[u8; 32]>,
    /// The sale's float inputs and the setup's, with their values (from their `witness_utxo`): what a conflict may spend.
    pub sale_inputs: Vec<(Outpoint, u64)>,
    pub setup_inputs: Vec<(Outpoint, u64)>,
    /// What each pays the miner (the judge's figures).
    pub sale_fee: u64,
    pub setup_fee: u64,
    /// The escrow input (the passthrough's output 0): the item, never a conflict's input.
    pub escrow: Outpoint,
    /// When it was handed back (seconds): the stale clock, kept if the sale leaves a mempool after `Sent`.
    pub at: u64,
    /// Protected-listing review P-03: the listed outpoint (the parent's one input): spent by anything but the parent, the listing
    /// is gone. `None` in a record saved before it.
    #[serde(default)]
    pub listing: Option<Outpoint>,
    /// Review P-02: Bitcoin has shown the sale (a mempool or a block) at least once. Never seen, its conflict outbids nothing
    /// (it is built at the market rate) and its value was never counted (H3 counts a protected buy once its sale is seen).
    #[serde(default)]
    pub seen: Option<bool>,
}

impl FrogState {
    pub fn protected_rec(&self, frog: u64) -> Option<&ProtectedRec> {
        self.protected.as_ref()?.iter().find(|r| r.frog == frog)
    }
    /// The signed PSBTs of a frog that is `Handed` (for the runner to submit); `None` in any other phase.
    pub fn protected_signed(&self, frog: u64) -> Option<&ProtectedRec> {
        let handed = self.frogs.iter().any(|f| f.id == frog && matches!(f.phase, FPhase::Handed { .. }));
        if handed { self.protected_rec(frog) } else { None }
    }
    fn put_protected(&mut self, r: ProtectedRec) {
        let v = self.protected.get_or_insert_with(Vec::new);
        v.retain(|x| x.frog != r.frog);
        v.push(r);
    }
}

impl FrogState {
    pub fn evm_pending(&self) -> bool {
        self.pending.is_some()
    }
    /// Phase B2: what frog `id`'s purchase was counted at in wei (the caps, the limiter), `None` on the EVM home (its cost is wei).
    pub fn value_of(&self, id: u64) -> Option<u128> {
        self.values.as_ref()?.iter().find(|(f, _)| *f == id).map(|(_, v)| *v)
    }
    pub(crate) fn put_value_pub(&mut self, id: u64, wei: u128) {
        self.put_value(id, wei)
    }
    fn put_value(&mut self, id: u64, wei: u128) {
        let v = self.values.get_or_insert_with(Vec::new);
        v.retain(|(f, _)| *f != id);
        v.push((id, wei));
    }
    /// Custody outputs a return in flight has claimed as its fee: no delivery may choose them.
    pub fn reserved(&self) -> Vec<Outpoint> {
        self.deposits.iter().flat_map(|d| d.reserved()).collect()
    }
    pub fn proof(&self, id: &str) -> Option<Vec<[u8; 32]>> {
        self.proofs.iter().find(|(i, _)| i == id).map(|(_, p)| p.clone())
    }
    pub fn put_proof(&mut self, id: String, proof: Vec<[u8; 32]>) {
        match self.proofs.iter_mut().find(|(i, _)| *i == id) {
            Some(x) => x.1 = proof,
            None => self.proofs.push((id, proof)),
        }
    }
    fn frog(&mut self, id: u64) -> Option<&mut FrogRec> {
        self.frogs.iter_mut().find(|f| f.id == id)
    }
    fn sink(&self, a: &[u8; 20]) -> Option<&SinkRec> {
        self.sinks.iter().find(|s| s.sink == *a)
    }
    fn sink_mut(&mut self, a: &[u8; 20]) -> Option<&mut SinkRec> {
        self.sinks.iter_mut().find(|s| s.sink == *a)
    }
    /// The inscription of a frog this canister minted, by its registry leaf: how the custodian finds a deposit id's frog.
    pub fn inscription_for_leaf(&self, leaf: &[u8; 32]) -> Option<String> {
        let frogs = self.frogs.iter().filter(|f| f.minted()).filter_map(|f| f.bought()).map(|b| b.inscription.clone());
        let deps = self.deposits.iter().filter_map(|d| match &d.phase {
            DPhase::Minted { inscription, .. } => Some(inscription.clone()),
            _ => None,
        });
        frogs.chain(deps).find(|i| frogbuy::leaf_of(i) == *leaf)
    }
}

/// A marketplace offer for a frog record, handed in by the relayer (as `relay.rs` for the vault buyer).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    pub inscription: String,
    pub psbt: Vec<u8>,
    pub to_sign: Vec<usize>,
    pub proof: Vec<[u8; 32]>,
}

/// A protected Ordinals Wallet sale for a frog record (docs/v8/ow-protected.md §3.4), handed in by the runner. Nothing in it is
/// believed: it is judged at the next tick against the canister's own reads (`owbuy::judge_protected`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtectedOffer {
    pub inscription: String,
    pub sale_psbt: Vec<u8>,
    /// The passthrough's raw bytes.
    pub parent: Vec<u8>,
    pub listing: crate::owbuy::Listing,
    pub setup_psbt: Option<Vec<u8>>,
    pub proof: Vec<[u8; 32]>,
}

/// Everything the flow may do to the world. `save` must be durable before the next `await` returns (as `looper::Io`). Phase B2: a
/// world whose home is Solana answers `SolIo` too (`sol_home`) and names its frog wrapper (`sol_wrapper`); its `rate` is then
/// satoshis per 10^18 lamports and `our_balance` our Solana address's lamports (`frogsol`).
#[allow(async_fn_in_trait)]
pub trait FrogIo: crate::solhome::SolIo {
    /// The frog wrapper's Solana address (rook_core's Inscriptions `Wrapper` PDA) on the Solana home; `None` on the EVM home.
    fn sol_wrapper(&self) -> Option<crate::soltx::Pubkey> {
        None
    }
    /// B-L-6: whether the Ethereum NFT side holds the one Relay leg (its `relay_busy`). A side that cannot tell says busy.
    fn eth_relay_busy(&self) -> bool {
        false
    }
    fn now(&self) -> u64;
    fn chain_id(&self) -> u64;
    fn wrapper(&self) -> [u8; 20];
    /// The next read takes a new pin: "immediately before" a send.
    fn fresh(&mut self);
    async fn eth_read(&mut self, to: [u8; 20], data: &[u8]) -> Result<Vec<u8>, String>;
    /// The pinned block's two clocks (see `itemsink::Clock`).
    async fn clock(&mut self) -> Result<Clock, String>;
    async fn our_evm(&mut self) -> Result<[u8; 20], String>;
    async fn evm_nonce(&mut self) -> Result<u64, String>;
    async fn base_fee(&mut self) -> Result<u128, String>;
    async fn estimate_gas(&mut self, to: [u8; 20], value: u128, data: &[u8]) -> Result<u64, String>;
    async fn sign_evm(&mut self, hash: [u8; 32]) -> Result<(u8, [u8; 32], [u8; 32]), String>;
    async fn send_evm(&mut self, raw: &[u8]) -> Result<(), String>;
    async fn evm_seen(&mut self, hash: [u8; 32]) -> EvmView;
    async fn float_script(&mut self) -> Result<Vec<u8>, String>;
    async fn custody_script(&mut self) -> Result<Vec<u8>, String>;
    async fn locate(&mut self, inscription: &str) -> Result<Option<custody::Location>, String>;
    async fn contents(&mut self, o: &Outpoint) -> Result<Option<custody::Contents>, String>;
    /// The output as every explorer serves it, agreeing, and whether it is spent.
    async fn output(&mut self, o: &Outpoint) -> Result<(custody::Output, bool), String>;
    /// Who spent an output (display txid) and whether that is confirmed; `None` unspent.
    async fn outspend(&mut self, o: &Outpoint) -> Result<Option<([u8; 32], bool)>, String>;
    /// `txid` in display order.
    async fn tx_status(&mut self, txid: [u8; 32]) -> BtcSeen;
    async fn tip(&mut self) -> Result<u64, String>;
    async fn block_hash(&mut self, height: u64) -> Result<String, String>;
    async fn btc_send(&mut self, raw: &[u8]) -> Result<(), String>;
    async fn custody_utxos(&mut self) -> Result<Vec<custody::Utxo>, String>;
    async fn tx_view(&mut self, txid: [u8; 32]) -> Result<deposits::TxView, String>;
    async fn fee_rate(&mut self) -> Result<u64, String>;
    /// The reference rate, sats per ETH.
    async fn rate(&mut self) -> Result<u64, String>;
    /// ord shows the output carrying no inscription and no rune (the vault buyer's belt and braces).
    async fn input_clean(&mut self, o: Outpoint) -> Result<(), String>;
    async fn sign_float(&mut self, sighash: [u8; 32]) -> Result<Vec<u8>, String>;
    async fn sign_custody(&mut self, sighash: [u8; 32]) -> Result<Vec<u8>, String>;
    fn take_offer(&mut self, frog: u64) -> Option<Offer>;
    fn save(&mut self, st: &FrogState);
    /// The floor side's state (docs/v5/floor-feed.md §6.4), saved as `save` is. A world without floor sinks keeps nothing.
    fn save_floor(&mut self, _fl: &FloorState) {}
    /// The hand-ins' state (docs/v6/offer-handins.md §6), saved as `save` is. A world without boards keeps nothing.
    fn save_handins(&mut self, _hs: &HandinState) {}
    /// The sales' state (docs/v6/canister-floor-followups.md §6), saved as `save` is. A world without sales keeps nothing.
    fn save_sales(&mut self, _ss: &SaleState) {}
    /// The Bitcoin network's bech32 prefix, to decode a ticket's destination. A world that never reads tickets has none.
    fn btc_hrp(&self) -> Result<&'static str, String> {
        Err("the network's address prefix is not known in this world".into())
    }
    /// Our 4663 balance at the pin: what a sale's report may be sent with. A world without sales does not read it.
    async fn our_balance(&mut self) -> Result<u128, String> {
        Err("the balance is not read in this world".into())
    }
    /// Run `f` on the custody caps in force and the book (docs/v5/custody-hardening.md, H2).
    fn with_caps<R>(&mut self, f: impl FnOnce(&custodycap::CapPolicy, &mut custodycap::Book) -> R) -> R;
    /// Ask the outbound limiter (H3); a trip freezes custody (the canister pauses). `Err` is a wait or the freeze.
    fn outflow_check(&mut self, key: &str, collection: &str, flow: Flow, amount: u128) -> Result<(), String>;
    /// Count an act that went out, once its signed bytes are saved (H3). Idempotent per key.
    fn outflow_record(&mut self, key: &str, collection: &str, flow: Flow, amount: u128);
    /// The rules the outbound verifier applies to custody's spends (H4).
    fn outbound_policy(&self, custody_script: &[u8]) -> crate::outbound::BtcPolicy;
    /// The buyer's pause (H1; an H3 trip sets it). A return's conflict-out (H8) signs nothing while it is on.
    fn paused(&self) -> bool;
    /// Holder deposits are switched on (audit D, H-2): off, the deposits pass reads nothing and returns nothing.
    fn deposits_on(&self) -> bool {
        false
    }
    /// Candidates for a return's fee input: the canister's fee pool, else custody's listing (audit D, H-4). Each is verified
    /// with `output` and `contents` before use. A world without a pool lists custody.
    async fn fee_utxos(&mut self) -> Result<Vec<custody::Utxo>, String> {
        self.custody_utxos().await
    }
    /// A custody output of ours (a return's change, or its conflict's output), confirmed: a fee input for later.
    fn fee_pool_add(&mut self, _o: Outpoint, _value: u64) {}
    /// The buyer's own script: never a return's destination (audit D, I-6). A world without one returns nothing.
    async fn buyer_script(&mut self) -> Result<Vec<u8>, String> {
        Ok(vec![])
    }
    /// The float's confirmed UTXOs, every explorer agreeing: a second input when a stale buy's conflict-out needs one (audit
    /// D, L-5).
    async fn float_utxos(&mut self) -> Result<Vec<custody::Utxo>, String> {
        Err("the float's UTXOs are not read in this world".into())
    }
    /// The float's address, for Relay's order (docs/v6/relay-conversion.md).
    async fn float_address(&mut self) -> Result<String, String> {
        Err("the float's address is not known in this world".into())
    }
    /// How floor frogs are paid for: `Some` = Relay conversions with these bounds (the install's `frog_funding`, default on),
    /// `None` = the operator's float only (a world without conversions).
    fn funding(&self) -> Option<crate::relayconv::FrogFunding> {
        None
    }
    /// Relay orders another Inscriptions collection's conversions have used (the Ordinals multi-collection change: rule 12 is
    /// canister-wide, so an order one collection judged is never judged again for another). A world of one collection has none.
    fn used_elsewhere(&self) -> Vec<[u8; 32]> {
        vec![]
    }
    /// The runner's quote for `frog`, with when it was handed in; taken once.
    fn take_quote(&mut self, _frog: u64) -> Option<(Vec<u8>, u64)> {
        None
    }
    /// A quote taken but not used (audit A5-07: the line was busy): handed back, as it was.
    fn return_quote(&mut self, _frog: u64, _body: Vec<u8>, _at: u64) {}
    /// The runner's protected Ordinals Wallet offer for `frog` (docs/v8/ow-protected.md §3.4); taken once.
    fn take_protected(&mut self, _frog: u64) -> Option<ProtectedOffer> {
        None
    }
    /// The float's untweaked x-only internal key (`PSBT_IN_TAP_INTERNAL_KEY` of the inputs it signs).
    async fn float_internal_key(&mut self) -> Result<[u8; 32], String> {
        Err("the float's internal key is not known in this world".into())
    }
}

/// H2: room under the caps for one more item of `collection`, before a purchase is started (its cost is checked and reserved
/// when it is signed). Refused is "no purchase", never an error.
fn room<I: FrogIo>(io: &mut I, collection: &str) -> Result<(), String> {
    io.with_caps(|p, b| custodycap::check_acquire(p, b, collection, 0)).map_err(|e| format!("custody cap: {e:?}"))
}

// ---- reads -----------------------------------------------------------------------------------------------------------------------

pub(crate) async fn word<I: FrogIo>(io: &mut I, to: [u8; 20], data: &[u8]) -> Result<Vec<u8>, String> {
    io.eth_read(to, data).await
}

pub(crate) async fn addr<I: FrogIo>(io: &mut I, to: [u8; 20], sel: [u8; 4]) -> Result<[u8; 20], String> {
    crate::abi::addr_at(&word(io, to, &sel).await?, 0, 0)
}

pub(crate) async fn flag<I: FrogIo>(io: &mut I, to: [u8; 20], sel: [u8; 4], w: [u8; 32]) -> Result<bool, String> {
    crate::abi::bool_at(&word(io, to, &deposits::call_word(sel, w)).await?, 0, 0)
}

/// The listing's membership root, through the wrapper's own registry and listing id.
async fn root<I: FrogIo>(io: &mut I) -> Result<[u8; 32], String> {
    // phase B2: the Inscriptions listing's own root (rook_core's `mint_item` checks the proof against the same)
    if crate::frogsol::solana(io) {
        return crate::frogsol::root(io).await;
    }
    let w = io.wrapper();
    let registry = addr(io, w, deposits::REGISTRY).await?;
    let id = crate::abi::uint_at(&word(io, w, &deposits::LISTING_ID).await?, 0, 0, 128)?;
    deposits::ret_membership_root(&word(io, registry, &crate::abi::encode_call(deposits::LISTING,
        &[crate::abi::Arg::Word(crate::abi::uint_word(id))])).await?)
}

async fn proof_ok<I: FrogIo>(io: &mut I, inscription: &str, proof: &[[u8; 32]]) -> Result<(), String> {
    if frogbuy::verify_proof(proof, &root(io).await?, &frogbuy::leaf_of(inscription)) {
        Ok(())
    } else {
        Err(format!("the proof for {inscription} does not verify against the listing's root"))
    }
}

/// A sink read at the current pin, as the rules see it. `limit`: the collection's purchase limit (a Floor Module's cap).
pub(crate) async fn read_sink<I: FrogIo>(io: &mut I, kind: SinkKind, a: [u8; 20], limit: Option<u128>) -> Result<SinkView, String> {
    // phase B2: the frog listing's rook_core module at finalized, in lamports
    if crate::frogsol::solana(io) {
        return crate::frogsol::read_sink(io, kind, a, limit).await;
    }
    let calls = match kind {
        SinkKind::Curve => crate::curvebuy::reads(),
        SinkKind::Queue => crate::buyqueue::reads(),
        SinkKind::Floor => crate::floorsink::reads(),
    };
    let mut r = Vec::with_capacity(calls.len());
    for c in calls {
        r.push(word(io, a, &c).await?);
    }
    let clock = io.clock().await?;
    match kind {
        SinkKind::Curve => crate::curvebuy::view(&r, clock),
        SinkKind::Queue => crate::buyqueue::view(&r, clock),
        SinkKind::Floor => crate::floorsink::view(&r, clock, limit),
    }
}

/// Whether a sink is ours to serve: it reimburses our EVM address, for our wrapper (and a curve is the factory's).
async fn identity<I: FrogIo>(io: &mut I, kind: SinkKind, a: [u8; 20], me: [u8; 20], factory: Option<[u8; 20]>, wrapper: [u8; 20])
    -> Result<Option<String>, String> {
    // phase B2: on the Solana home a wrapper has one module (its listing's PDA), and the listing's custodian is checked by `custodian_ok`
    if crate::frogsol::solana(io) {
        let _ = (kind, me, factory, wrapper);
        return crate::frogsol::identity(io, a).await;
    }
    if addr(io, a, crate::curvebuy::PAYEE).await? != me {
        return Ok(Some("its payee is not this canister: it would reimburse someone else".into()));
    }
    if addr(io, a, crate::curvebuy::WRAPPER).await? != wrapper {
        return Ok(Some("its wrapper is not ours".into()));
    }
    if kind == SinkKind::Curve && Some(addr(io, a, crate::curvebuy::FACTORY).await?) != factory {
        return Ok(Some("it was not made by the configured factory".into()));
    }
    Ok(None)
}

async fn read_request<I: FrogIo>(io: &mut I, desk: [u8; 20], id: u64) -> Result<mintdesk::Request, String> {
    mintdesk::decode_request(&word(io, desk, &crate::curvebuy::call_uint(mintdesk::REQUEST_OF, id as u128)).await?)
}

async fn desk_ttl<I: FrogIo>(io: &mut I, desk: [u8; 20]) -> Result<u64, String> {
    Ok(crate::abi::uint_at(&word(io, desk, &mintdesk::TTL).await?, 0, 0, 64)? as u64)
}

/// How deep `txid` is, with the block the explorers named still the block at its height. `Ok(None)`: not in a block.
pub(crate) async fn depth<I: FrogIo>(io: &mut I, txid: [u8; 32]) -> Result<Option<u64>, String> {
    match io.tx_status(txid).await {
        BtcSeen::Confirmed { height, block_hash } => {
            let at = io.block_hash(height).await?;
            if at != block_hash {
                return Err(format!("block {height} is now {at}, not {block_hash}: reorganised, read again"));
            }
            Ok(Some(frogbuy::depth(io.tip().await?, height)))
        }
        BtcSeen::Mempool | BtcSeen::NotYet => Ok(None),
        BtcSeen::Unsure(e) => Err(e),
    }
}

// ---- the tick ----------------------------------------------------------------------------------------------------------------------

/// The whole pass. `others_pending`: an EVM transaction of the vault loop's, the custodian's or the NFT line's is out (one at
/// a time, canister-wide). `delivery_reserved`: fee inputs redemption deliveries in flight have claimed.
pub async fn tick<I: FrogIo>(io: &mut I, st: &mut FrogState, others_pending: bool, delivery_reserved: &[Outpoint]) -> Vec<String> {
    tick_with(io, st, &mut FloorState::default(), others_pending, delivery_reserved).await
}

/// [`tick`], with the floor side's state: which floor sinks buy EVM items, the per-collection purchase limits, the NFT wants
/// (docs/v5/floor-feed.md §6). Without a floor sink or a limit it is exactly `tick`.
pub async fn tick_with<I: FrogIo>(io: &mut I, st: &mut FrogState, fl: &mut FloorState, others_pending: bool, delivery_reserved: &[Outpoint])
    -> Vec<String> {
    tick_full(io, st, fl, &mut HandinState::default(), others_pending, delivery_reserved).await
}

/// [`tick_with`], with the offer boards' state (docs/v6/offer-handins.md): the boards are read at the pin, a wanted frog
/// with no operator offer tries the cheapest hand-in, and claims and slashes are the fifth EVM group. Without a board it is
/// exactly `tick_with`.
pub async fn tick_full<I: FrogIo>(io: &mut I, st: &mut FrogState, fl: &mut FloorState, hs: &mut HandinState, others_pending: bool,
    delivery_reserved: &[Outpoint]) -> Vec<String> {
    tick_all(io, st, fl, hs, &mut SaleState::default(), others_pending, delivery_reserved).await
}

/// [`tick_full`], with the sales' state (docs/v6/canister-floor-followups.md §5): the modules' sale side and the wrapper's
/// relist tickets are read at the pin, listings are signed and watched on Bitcoin, and the sale steps are the sixth EVM
/// group. Without a sale policy or a sale it is exactly `tick_full`.
pub async fn tick_all<I: FrogIo>(io: &mut I, st: &mut FrogState, fl: &mut FloorState, hs: &mut HandinState, ss: &mut SaleState,
    others_pending: bool, delivery_reserved: &[Outpoint]) -> Vec<String> {
    let mut lines = Vec::new();
    // 1. our own EVM transaction, if one is out, before anything is decided
    if let Some(p) = st.pending.clone() {
        lines.push(match resolve(io, st, fl, hs, ss, p).await {
            Ok(s) => format!("items: {s}"),
            Err(e) => format!("items: waits: {e}"),
        });
        io.save(st);
        io.save_floor(fl);
        io.save_handins(hs);
        io.save_sales(ss);
    }
    // 1b. phase B2: on the Solana home, the canister-wide line: what landed for this side, what is still out
    let sol = crate::frogsol::solana(io);
    if sol {
        for l in crate::frogsol::resolve(io, st, fl).await {
            lines.push(l);
        }
        io.save(st);
        io.save_floor(fl);
    }
    // 2. who we are to the wrapper: without the custodian's role nothing is minted, so nothing new is bought
    let role = custodian_ok(io).await;
    if let Err(e) = &role {
        lines.push(format!("items: waits: {e}"));
    }
    let me = role.as_ref().ok().copied();
    // 3. the sinks and the desk, read at this tick's pin
    if let Some(me) = me {
        for l in discover_curves(io, st, me).await {
            lines.push(l);
        }
        for l in read_sinks(io, st, fl, me).await {
            lines.push(l);
        }
        for l in read_desk(io, st).await {
            lines.push(l);
        }
        // the offer boards (docs/v6/offer-handins.md §1.3), at the same pin
        for l in crate::handinflow::read_boards(io, hs).await {
            lines.push(l);
        }
        io.save_handins(hs);
        // the modules' sale side and the wrapper's relist tickets (docs/v6/canister-floor-followups.md §3.1, §4)
        for l in crate::saleflow::read(io, st, fl, ss, me).await {
            lines.push(l);
        }
        io.save_sales(ss);
    }
    io.save(st);
    // 4. each frog on Bitcoin
    let ids: Vec<u64> = st.frogs.iter()
        .filter(|f| matches!(f.phase, FPhase::Wanted { .. } | FPhase::Signed { .. } | FPhase::Sent { .. } | FPhase::Abandoning { .. }
            | FPhase::Handed { .. }))
        .map(|f| f.id).collect();
    for id in ids {
        let line = match step_frog(io, st, fl, hs, id).await {
            Ok(s) => format!("frog {id}: {s}"),
            Err(e) => format!("frog {id}: waits: {e}"),
        };
        io.save(st);
        io.save_handins(hs);
        lines.push(line);
    }
    // 4-bis. conversions: a mined Relay deposit watched for its arrival, a refund or its deadline (docs/v6/relay-conversion.md)
    for l in crate::convflow::pass(io, st).await {
        lines.push(l);
    }
    // 4a. L-2 (audit D): a reservation whose purchase is gone (a continuation lost to an upgrade mid-signing, which leaves the frog
    //     wanted with its room reserved for ever) is released; the L-5 clocks of frogs no longer signed or sent are forgotten
    for l in sweep_stale(io, st) {
        lines.push(l);
    }
    // 4b. each sale on Bitcoin: listings signed, listed outpoints watched, moves confirmed; a move's fee input is reserved
    //     from the deliveries and returns, as theirs are from it
    let all_reserved: Vec<Outpoint> = delivery_reserved.iter().copied().chain(st.reserved()).collect();
    for l in crate::saleflow::btc_pass(io, ss, &all_reserved).await {
        lines.push(l);
    }
    let delivery_reserved: Vec<Outpoint> = delivery_reserved.iter().copied().chain(ss.reserved()).collect();
    let delivery_reserved = delivery_reserved.as_slice();
    // 5. give held frogs to what can take them; start at most one purchase
    if me.is_some() {
        for l in assign(io, st, fl).await {
            lines.push(l);
        }
        io.save(st);
        io.save_floor(fl);
    }
    // 6. holder deposits
    for l in deposits_pass(io, st, fl, delivery_reserved).await {
        lines.push(l);
    }
    io.save(st);
    // 7. at most one EVM transaction of ours (phase B2: on the Solana home, one step on the canister-wide line, which the line itself
    //    serialises across every flow; the EVM's other transactions are on another chain)
    if me.is_some() && st.pending.is_none() && st.sol_step.is_none() {
        if others_pending && !sol {
            lines.push("items: EVM waits: another transaction of ours is out (one at a time)".into());
        } else {
            match evm_act(io, st, fl, hs, ss).await {
                Ok(Some(s)) => lines.push(format!("items: {s}")),
                Ok(None) => {
                    if sol {
                        crate::solhome::release(io, crate::solhome::LineOwner::Frog);
                    }
                }
                Err(e) => {
                    // audit A5-08: an act that failed before it reached the line gives up a turn it no longer takes
                    if sol {
                        crate::ethflow::release_untaken_turn(io, crate::solhome::LineOwner::Frog);
                    }
                    lines.push(format!("items: {} waits: {e}", if sol { "the Solana line" } else { "EVM" }))
                }
            }
        }
        io.save(st);
        io.save_floor(fl);
        io.save_handins(hs);
        io.save_sales(ss);
    }
    lines
}

async fn custodian_ok<I: FrogIo>(io: &mut I) -> Result<[u8; 20], String> {
    // phase B2: the listing's custodian is our Ed25519 key
    if crate::frogsol::solana(io) {
        return crate::frogsol::custodian_ok(io).await;
    }
    let me = io.our_evm().await?;
    let w = io.wrapper();
    let c = addr(io, w, deposits::CUSTODIAN).await?;
    if c != me {
        return Err(format!("the wrapper's custodian is 0x{}, not us: nothing is bought or minted", hex::encode(c)));
    }
    Ok(me)
}

async fn discover_curves<I: FrogIo>(io: &mut I, st: &mut FrogState, me: [u8; 20]) -> Vec<String> {
    let Some(f) = st.factory else { return vec![] };
    let mut lines = Vec::new();
    let count = match word(io, f, &crate::curvebuy::COIN_COUNT).await.and_then(|b| crate::curvebuy::ret_uint(&b)) {
        Ok(c) => c as u64,
        Err(e) => return vec![format!("items: the factory is not read: {e}")],
    };
    let end = count.min(st.coin_cursor + MAX_NEW_COINS_PER_TICK);
    while st.coin_cursor < end {
        let i = st.coin_cursor;
        let r = async {
            let coin = crate::curvebuy::ret_addr(&word(io, f, &crate::curvebuy::call_uint(crate::curvebuy::COINS, i as u128)).await?)?;
            let curve = crate::curvebuy::ret_addr(&word(io, f, &crate::curvebuy::call_addr(crate::curvebuy::ROUTER_OF, &coin)).await?)?;
            let w = io.wrapper();
            let refused = identity(io, SinkKind::Curve, curve, me, Some(f), w).await?;
            Ok::<_, String>((coin, curve, refused))
        }.await;
        match r {
            Ok((coin, curve, refused)) => {
                if let Some(why) = &refused {
                    lines.push(format!("curve 0x{}: never served: {why}", hex::encode(curve)));
                }
                st.sinks.push(SinkRec { kind: SinkKind::Curve, sink: curve, coin, view: None, refused, drove_refunds_at: None,
                    drove_settle: false });
                st.coin_cursor += 1;
            }
            Err(e) => {
                lines.push(format!("items: coin {i} not read: {e}"));
                break;
            }
        }
    }
    lines
}

/// Read up to `MAX_SINKS_READ_PER_TICK` live sinks, round robin, and those with frogs of ours first.
async fn read_sinks<I: FrogIo>(io: &mut I, st: &mut FrogState, fl: &FloorState, me: [u8; 20]) -> Vec<String> {
    let mut lines = Vec::new();
    // a configured queue's identity is checked once, when first read
    let live: Vec<usize> = (0..st.sinks.len()).filter(|&i| !st.sinks[i].done()).collect();
    if live.is_empty() {
        return lines;
    }
    let busy: Vec<usize> = live.iter().copied().filter(|&i| itemsink::assigned(&st.frogs, &st.sinks[i].sink).0 > 0).collect();
    let start = (st.poll as usize) % live.len();
    let mut order: Vec<usize> = busy.clone();
    for k in 0..live.len() {
        let i = live[(start + k) % live.len()];
        if !order.contains(&i) {
            order.push(i);
        }
    }
    order.truncate(MAX_SINKS_READ_PER_TICK.max(busy.len()));
    st.poll = st.poll.wrapping_add(MAX_SINKS_READ_PER_TICK as u64);
    for i in order {
        let (kind, a) = (st.sinks[i].kind, st.sinks[i].sink);
        let unchecked = st.sinks[i].view.is_none() && matches!(kind, SinkKind::Queue | SinkKind::Floor);
        if unchecked {
            // a floor sink for EVM items is wrapped by that collection's NftWrapper; everything else by ours
            let wrapper = match fl.item_of(&a) {
                Some(FloorItem::Evm { wrapper, .. }) => *wrapper,
                _ => io.wrapper(),
            };
            match identity(io, kind, a, me, None, wrapper).await {
                Ok(Some(why)) => {
                    lines.push(format!("{kind:?} 0x{}: never served: {why}", hex::encode(a)));
                    st.sinks[i].refused = Some(why);
                    continue;
                }
                Ok(None) => {}
                Err(e) => {
                    lines.push(format!("{kind:?} 0x{}: not read: {e}", hex::encode(a)));
                    continue;
                }
            }
        }
        match read_sink(io, kind, a, fl.limit_for(&a)).await {
            Ok(v) => st.sinks[i].view = Some(v),
            Err(e) => lines.push(format!("{kind:?} 0x{}: not read: {e}", hex::encode(a))),
        }
    }
    lines
}

async fn read_desk<I: FrogIo>(io: &mut I, st: &mut FrogState) -> Vec<String> {
    let Some(d) = st.desk else { return vec![] };
    let mut lines = Vec::new();
    let count = match word(io, d, &mintdesk::REQUEST_COUNT).await.and_then(|b| crate::curvebuy::ret_uint(&b)) {
        Ok(c) => c as u64,
        Err(e) => return vec![format!("desk: not read: {e}")],
    };
    let end = count.min(st.request_cursor + MAX_NEW_REQUESTS_PER_TICK);
    while st.request_cursor < end {
        let id = st.request_cursor;
        match read_request(io, d, id).await {
            Ok(r) => {
                let open = r.status == DStatus::Open;
                st.requests.push(ReqRec { id, request: r, done: !open, note: String::new() });
                st.request_cursor += 1;
            }
            Err(e) => {
                lines.push(format!("desk: request {id} not read: {e}"));
                break;
            }
        }
    }
    // refresh the open ones we still care about, ours first
    let mut open: Vec<usize> = (0..st.requests.len()).filter(|&i| !st.requests[i].done).collect();
    open.sort_by_key(|&i| !st.frogs.iter().any(|f| f.target == Some(Target::Desk(st.requests[i].id)) && !f.finished()));
    open.truncate(MAX_REQUESTS_READ_PER_TICK);
    let clock = io.clock().await.ok();
    let ttl = desk_ttl(io, d).await.ok();
    for i in open {
        let id = st.requests[i].id;
        match read_request(io, d, id).await {
            Ok(r) => {
                let ours = st.frogs.iter().any(|f| f.target == Some(Target::Desk(id)) && !f.finished());
                let expired_by_clock = matches!((ttl, clock), (Some(t), Some(c)) if mintdesk::secs_left(&r, t, &c) == 0);
                if r.status != DStatus::Open || (expired_by_clock && !ours) {
                    st.requests[i].done = true;
                }
                st.requests[i].request = r;
            }
            Err(e) => lines.push(format!("desk: request {id} not read: {e}")),
        }
    }
    lines
}

// ---- one frog on Bitcoin ------------------------------------------------------------------------------------------------------

async fn step_frog<I: FrogIo>(io: &mut I, st: &mut FrogState, fl: &FloorState, hs: &mut HandinState, id: u64) -> Result<String, String> {
    let f = st.frogs.iter().find(|f| f.id == id).cloned().ok_or("no such frog")?;
    match f.phase {
        FPhase::Wanted { cap_wei } => {
            if let Some(w) = crate::convflow::waits_in(st, id, if crate::frogsol::solana(io) { "lamports" } else { "wei" }) {
                let _ = io.take_offer(id); // an offer before the sats arrived is not kept: hand it in again
                let _ = io.take_protected(id);
                return Ok(w);
            }
            let (limit, min) = match f.target {
                Some(Target::Sink(s)) => (fl.limit_for(&s), fl.min_for(&s)),
                _ => (None, None),
            };
            // the operator's offer first; then the cheapest permissionless hand-in (docs/v6/offer-handins.md §1.3)
            if let Some(offer) = io.take_offer(id) {
                return Ok(match accept(io, st, id, cap_wei, limit, min, offer, None).await {
                    Ok(s) => s,
                    Err(e) => format!("offer refused, nothing signed: {}", e.text()),
                });
            }
            // a protected Ordinals Wallet sale (docs/v8/ow-protected.md): signed on the float's inputs and handed back, never sent
            if let Some(offer) = io.take_protected(id) {
                return Ok(match accept_protected(io, st, id, cap_wei, limit, min, offer).await {
                    Ok(s) => s,
                    Err(e) => format!("protected offer refused, nothing signed: {}", e.text()),
                });
            }
            let Some(c) = crate::handinflow::candidate(io, hs, cap_wei).await? else {
                return Ok(format!("wanted, for at most {cap_wei} wei: no offer yet"));
            };
            let offer = Offer { inscription: c.inscription.clone(), psbt: c.psbt.clone(), to_sign: c.to_sign.clone(), proof: c.proof.clone() };
            let now = io.now();
            match accept(io, st, id, cap_wei, limit, min, offer, Some(c.ask_sats)).await {
                Ok(s) => {
                    hs.set_verdict(c.board, c.id, Verdict::Bought { frog: id }, now);
                    Ok(format!("hand-in {}: {s}", c.id))
                }
                Err(Why::Invalid(e)) => {
                    hs.set_verdict(c.board, c.id, Verdict::Invalid { why: e.clone() }, now);
                    Ok(format!("hand-in {} INVALID, nothing signed (slashed once the board allows): {e}", c.id))
                }
                Err(Why::Gone(e)) => {
                    hs.set_verdict(c.board, c.id, Verdict::Gone { why: e.clone() }, now);
                    Ok(format!("hand-in {} not fillable, nothing signed: {e}", c.id))
                }
                Err(Why::Later(e)) => Ok(format!("hand-in {}: waits: {e}", c.id)),
            }
        }
        FPhase::Signed { buy } | FPhase::Sent { buy } if buy_stale(io, st, id) => {
            // review P-04: a protected buy (no bytes of ours: `raw` empty) is never taken for a classic one
            if st.protected_rec(id).is_some() || buy.raw.is_empty() {
                abandon_protected(io, st, id, buy).await
            } else {
                abandon(io, st, id, buy).await
            }
        }
        FPhase::Abandoning { .. } => abandoning(io, st, id).await,
        FPhase::Handed { buy, at, .. } => handed(io, st, id, buy, at).await,
        FPhase::Signed { buy } => match io.tx_status(buy.outpoint.txid).await {
            BtcSeen::Confirmed { .. } | BtcSeen::Mempool => {
                set_phase(st, id, FPhase::Sent { buy });
                Ok("Bitcoin has it".into())
            }
            BtcSeen::Unsure(e) => Err(e),
            BtcSeen::NotYet if buy.raw.is_empty() => {
                // review P-04: a protected buy has no bytes of ours to send: it is watched as handed
                let at = st.protected_rec(id).map_or(io.now(), |r| r.at);
                let sale_txid = hex::encode(buy.outpoint.txid);
                set_phase(st, id, FPhase::Handed { buy, sale_txid, at });
                Ok("a protected sale (no bytes of ours): watched as handed, nothing sent".into())
            }
            BtcSeen::NotYet => {
                // the listing vanished: its outpoint spent, in a block, by a transaction that is not ours
                if let Some((spender, true)) = io.outspend(&buy.seller).await? {
                    if spender != buy.outpoint.txid {
                        return Ok(fail_buy(io, st, id, format!("the listing vanished: {} was spent by {}", buy.seller.text(),
                            hex::encode(spender)), "the listing vanished before ours confirmed: nothing of ours was spent".into()));
                    }
                }
                let _ = io.btc_send(&buy.raw).await;
                Ok("the same saved bytes sent again; Bitcoin does not show them yet".into())
            }
        },
        FPhase::Sent { buy } => match depth(io, buy.outpoint.txid).await {
            Ok(Some(d)) if d >= CONFIRMATIONS => {
                let cust = io.custody_script().await?;
                match custody_holds(io, &buy, &cust).await? {
                    Ok(()) => {
                        // H2: the reservation made at signing is now the held inscription. Audit C2-01: an arrival always counts: with
                        // no reservation in flight (released by a pass that should not have) the item custody now holds is booked
                        // anyway, loud, so the caps and the exit rule keep bounding what custody holds
                        let (collection, value) = (crate::frogsol::collection_key(io), st.value_of(id).unwrap_or(buy.cost_wei));
                        let book = io.with_caps(|_, b| custodycap::hold_arrived(b, &format!("frog:{id}"), &buy.inscription, &collection, value));
                        let note = match book {
                            Ok(None) => String::new(),
                            Ok(Some(w)) => format!(" (CUSTODY BOOK, audit C2-01: {w})"),
                            Err(e) => format!(" (custody book: {e})"),
                        };
                        set_phase(st, id, FPhase::Held { buy });
                        Ok(format!("{d} deep and in custody: held{note}"))
                    }
                    Err(why) => {
                        // bought, and custody holds something other than exactly this frog: the money is spent, the frog is
                        // not mintable. Loud; a person looks.
                        set_phase(st, id, FPhase::Stranded { buy, why: why.clone() });
                        Ok(format!("STRANDED: {why}"))
                    }
                }
            }
            Ok(Some(d)) => Ok(format!("{d} of {CONFIRMATIONS} confirmations")),
            Ok(None) => match io.tx_status(buy.outpoint.txid).await {
                BtcSeen::NotYet if st.protected_rec(id).is_some() || buy.raw.is_empty() => {
                    // a protected sale: the canister holds no bytes to send; back to watching it, on its first clock (review P-04: a
                    // record lost to an older build is recognised by its empty bytes, and watched from now)
                    let at = st.protected_rec(id).map_or(io.now(), |r| r.at);
                    let sale_txid = hex::encode(buy.outpoint.txid);
                    set_phase(st, id, FPhase::Handed { buy, sale_txid, at });
                    Ok("Bitcoin no longer shows the protected sale (evicted or reorganised out): handed again, nothing sent".into())
                }
                BtcSeen::NotYet => {
                    set_phase(st, id, FPhase::Signed { buy });
                    Ok("Bitcoin no longer shows it (evicted or reorganised out): back to sending the same bytes".into())
                }
                _ => Ok("in a mempool".into()),
            },
            Err(e) => Err(e),
        },
        other => Ok(format!("{}", phase_name(&other))),
    }
}

/// L-5: the buy's clock, started the first time it is seen signed and unconfirmed; true once it has run `BUY_STALE_SECS`.
fn buy_stale<I: FrogIo>(io: &I, st: &mut FrogState, id: u64) -> bool {
    let now = io.now();
    let clock = st.buy_clock.get_or_insert_with(Vec::new);
    let since = match clock.iter().find(|(f, _)| *f == id) {
        Some((_, t)) => *t,
        None => {
            clock.push((id, now));
            now
        }
    };
    now.saturating_sub(since) >= BUY_STALE_SECS
}

/// L-5: a buy that never confirmed. Unless it has confirmed after all, one of the float's inputs is spent back to the float,
/// which kills the signed bytes (the conflict pays more than the buy, so it replaces it in any mempool that has it).
async fn abandon<I: FrogIo>(io: &mut I, st: &mut FrogState, id: u64, buy: Bought) -> Result<String, String> {
    if let BtcSeen::Confirmed { .. } = io.tx_status(buy.outpoint.txid).await {
        if let Some(c) = st.buy_clock.as_mut() {
            c.retain(|(f, _)| *f != id); // confirmed: the clock starts again only if it ever leaves a block
        }
        set_phase(st, id, FPhase::Sent { buy });
        return Ok("the buy confirmed: waiting for six blocks and ord".into());
    }
    // the float's largest input of the buy: the one that can pay the conflict's fee on its own
    let t = crate::outbound::parse_tx(&buy.raw)?;
    let mut ours: Option<(Outpoint, u64)> = None;
    for o in t.inputs.iter().map(|i| custody::outpoint_of(&crate::psbt::TxIn { txid: i.txid, vout: i.vout, sequence: 0 }))
        .filter(|o| *o != buy.seller) {
        let (out, _) = io.output(&o).await?;
        if ours.is_none_or(|(_, v)| out.value > v) {
            ours = Some((o, out.value));
        }
    }
    let (ours, _) = ours.ok_or("the saved buy spends no input of ours")?;
    let conflict = match crate::conflict::Conflict::of_input(buy.outpoint.txid, &buy.raw, ours, buy.seller, buy.fee_sats) {
        Ok(c) => c,
        Err(e) => {
            set_phase(st, id, FPhase::Stranded { buy, why: format!("a stale buy whose saved bytes cannot be conflicted out: {e}") });
            return Ok(format!("STRANDED: {e}"));
        }
    };
    set_phase(st, id, FPhase::Abandoning { buy, conflict });
    io.save(st);
    abandoning(io, st, id).await.map(|s| format!("not confirmed in {} h: abandoned; the float's input goes back to the float: {s}",
        BUY_STALE_SECS / 3600))
}

/// A protected sale handed back (docs/v8/ow-protected.md §3.3): seen, `Sent`; not seen within `PROTECTED_STALE_SECS`, abandoned.
/// Nothing is ever sent from here: Ordinals Wallet broadcasts, the canister cannot.
async fn handed<I: FrogIo>(io: &mut I, st: &mut FrogState, id: u64, buy: Bought, at: u64) -> Result<String, String> {
    match io.tx_status(buy.outpoint.txid).await {
        BtcSeen::Confirmed { .. } | BtcSeen::Mempool => {
            protected_seen(io, st, id, &buy);
            set_phase(st, id, FPhase::Sent { buy });
            Ok("Bitcoin has the protected sale".into())
        }
        BtcSeen::Unsure(e) => Err(e),
        BtcSeen::NotYet => {
            // review P-03: the listing taken by someone else (the listed output or the escrow spent, in a block, by another
            // transaction): the sale can never be mined and nothing of ours is spent; no conflict
            if let Some(s) = protected_vanished(io, st, id).await? {
                return Ok(s);
            }
            let waited = io.now().saturating_sub(at);
            if waited < PROTECTED_STALE_SECS {
                return Ok(format!("handed to Ordinals Wallet {} min ago; Bitcoin does not show sale {} yet (nothing is sent here)",
                    waited / 60, hex::encode(buy.outpoint.txid)));
            }
            abandon_protected(io, st, id, buy).await
        }
    }
}

/// A protected sale that never confirmed: one float input it spends is spent back to the float. With a setup not yet confirmed,
/// the setup's largest input (a confirmed float coin: killing it kills the setup and the sale); else the sale's largest float
/// input. The conflict must outbid everything it replaces: the sale's fee, plus the setup's when it replaces that too.
async fn abandon_protected<I: FrogIo>(io: &mut I, st: &mut FrogState, id: u64, buy: Bought) -> Result<String, String> {
    let sale = io.tx_status(buy.outpoint.txid).await;
    if let BtcSeen::Confirmed { .. } = sale {
        protected_seen(io, st, id, &buy);
        set_phase(st, id, FPhase::Sent { buy });
        return Ok("the protected sale confirmed: waiting for six blocks and ord".into());
    }
    let Some(rec) = st.protected_rec(id).cloned() else {
        // review P-04: a protected buy whose record an older build dropped: its float inputs are not known here. Loud.
        set_phase(st, id, FPhase::Stranded { buy, why: "a stale protected buy whose record is gone: its float inputs are not known \
            here, so it cannot be conflicted out; a person looks".into() });
        return Ok("STRANDED: a stale protected buy with no record".into());
    };
    if let Some(s) = protected_vanished(io, st, id).await? {
        return Ok(s);
    }
    if matches!(sale, BtcSeen::Mempool) {
        protected_seen(io, st, id, &buy);
    }
    let sale_seen = matches!(sale, BtcSeen::Mempool) || rec.seen == Some(true);
    let (setup_open, setup_seen) = match rec.setup_txid {
        Some(t) => match io.tx_status(t).await {
            BtcSeen::Confirmed { .. } => (None, true),
            BtcSeen::Unsure(e) => return Err(e),
            BtcSeen::Mempool => (Some(t), true),
            BtcSeen::NotYet => (Some(t), false),
        },
        None => (None, false),
    };
    let (o, value, leaked, earlier) = protected_target(&rec, setup_open.is_some(), sale_seen, setup_seen)?;
    abandon_named(io, st, id, buy, o, value, leaked, rec.escrow, earlier).await
        .map(|s| format!("not seen within {} h of the hand-back: abandoned; the float's input goes back to the float: {s}",
            PROTECTED_STALE_SECS / 3600))
}

/// Review P-02: a protected sale Bitcoin has shown: marked seen, and its value counted under the day's limit (H3) now, once (an
/// advanced frog was counted at its advance). A sale never seen is never counted, so a conflict that kills it leaves no count.
fn protected_seen<I: FrogIo>(io: &mut I, st: &mut FrogState, id: u64, buy: &Bought) {
    let Some(r) = st.protected.as_mut().and_then(|v| v.iter_mut().find(|r| r.frog == id)) else { return };
    if r.seen == Some(true) {
        return;
    }
    r.seen = Some(true);
    if st.advanced_for(id).is_none() {
        // the key `buy_keys` names (written out: the H3 review reads each site's key from its own `let`)
        let value_key = format!("buy:frog:{id}");
        let collection = crate::frogsol::collection_key(io);
        io.outflow_record(&value_key, &collection, Flow::Value, st.value_of(id).unwrap_or(buy.cost_wei));
    }
}

/// Review P-03: a handed protected sale whose listing was taken: the listed outpoint spent in a block by a transaction other than
/// the parent, or the escrow by one other than our sale. Nothing of ours was spent: the buy fails without a conflict (or, its
/// conversion arrived, the frog is wanted again: C-01).
async fn protected_vanished<I: FrogIo>(io: &mut I, st: &mut FrogState, id: u64) -> Result<Option<String>, String> {
    let Some(rec) = st.protected_rec(id).cloned() else { return Ok(None) };
    let mut why = None;
    if let Some(l) = rec.listing {
        if let Some((spender, true)) = io.outspend(&l).await? {
            if spender != rec.escrow.txid {
                why = Some(format!("the listing vanished: {} was spent by {}, not the passthrough", l.text(), hex::encode(spender)));
            }
        }
    }
    if why.is_none() {
        if let Some((spender, true)) = io.outspend(&rec.escrow).await? {
            if spender != rec.sale_txid {
                why = Some(format!("the listing was sold to someone else: the escrow {} was spent by {}", rec.escrow.text(), hex::encode(spender)));
            }
        }
    }
    let Some(why) = why else { return Ok(None) };
    Ok(Some(fail_buy(io, st, id, why, "the protected listing vanished before our sale: nothing of ours was spent, no conflict".into())))
}

/// Which float input a protected sale's conflict spends, its value, what the conflict must outbid, and the transactions it may
/// replace unconfirmed: the setup's largest input while the setup is open (`setup_open`), else the sale's largest float input.
/// Review P-02: a fee is outbid only for a transaction Bitcoin has shown (`sale_seen`, `setup_seen`): a sale the API never relayed
/// costs the float a conflict at the market rate, not the fee the API chose.
fn protected_target(rec: &ProtectedRec, setup_open: bool, sale_seen: bool, setup_seen: bool)
    -> Result<(Outpoint, u64, u64, Option<Vec<[u8; 32]>>), String> {
    let largest = |v: &[(Outpoint, u64)]| v.iter().copied().max_by_key(|(o, value)| (*value, *o));
    let sale_fee = if sale_seen { rec.sale_fee } else { 0 };
    let (input, leaked, earlier) = match rec.setup_txid {
        Some(t) if setup_open => (largest(&rec.setup_inputs), sale_fee.saturating_add(if setup_seen { rec.setup_fee } else { 0 }),
            Some(vec![t])),
        _ => (largest(&rec.sale_inputs), sale_fee, None),
    };
    let (o, value) = input.ok_or("the protected record names no float input")?;
    Ok((o, value, leaked, earlier))
}

/// L-5 for a buy whose final bytes the canister never held: the conflict is told which float input to spend (`o`, worth
/// `value`) and what it must outbid (`leaked_fee`), and the explorers must show that input the float's, at that value.
#[allow(clippy::too_many_arguments)]
async fn abandon_named<I: FrogIo>(io: &mut I, st: &mut FrogState, id: u64, buy: Bought, o: Outpoint, value: u64, leaked_fee: u64,
    item: Outpoint, earlier: Option<Vec<[u8; 32]>>) -> Result<String, String> {
    let float = io.float_script().await?;
    let (out, _) = io.output(&o).await?;
    if out.script != float || out.value != value {
        set_phase(st, id, FPhase::Stranded { buy, why: format!("a stale buy's named input {} is not the float's {value} sats on the \
            explorers: it cannot be conflicted out", o.text()) });
        return Ok(format!("STRANDED: {} is not the float's {value} sats", o.text()));
    }
    let conflict = match crate::conflict::Conflict::of_named(buy.outpoint.txid, o, item, leaked_fee, earlier) {
        Ok(c) => c,
        Err(e) => {
            set_phase(st, id, FPhase::Stranded { buy, why: format!("a stale buy that cannot be conflicted out: {e}") });
            return Ok(format!("STRANDED: {e}"));
        }
    };
    set_phase(st, id, FPhase::Abandoning { buy, conflict });
    io.save(st);
    abandoning(io, st, id).await
}

/// One pass of an abandoning buy through `conflict::step` with the float's key, and where each ending leaves it.
async fn abandoning<I: FrogIo>(io: &mut I, st: &mut FrogState, id: u64) -> Result<String, String> {
    let Some(FPhase::Abandoning { buy, mut conflict }) = st.frogs.iter().find(|f| f.id == id).map(|f| f.phase.clone()) else {
        return Err("not abandoning".into());
    };
    let stepped = {
        let mut via = FloatVia { io: &mut *io, st: &mut *st, id, buy: &buy };
        crate::conflict::step(&mut via, &mut conflict, &[]).await?
    };
    use crate::conflict::{Settled, Stepped};
    // a protected sale's setup confirmed under a conflict of its input: the sale may still confirm on the setup's outputs, so
    // the conflict moves to the sale's own input (docs/v8/ow-protected.md §3.3)
    let setup = st.protected_rec(id).and_then(|r| r.setup_txid);
    if let Stepped::Settled(Settled::EarlierConfirmed(t) | Settled::SpentElsewhere(t)) = &stepped {
        if Some(*t) == setup {
            // the sale's inputs are the setup's outputs, at the values and script the judge held them to (P10), now confirmed
            let rec = st.protected_rec(id).cloned().ok_or("no protected record for this frog")?;
            let next = protected_target(&rec, false, rec.seen == Some(true), true).and_then(|(o, _, fee, earlier)| {
                crate::conflict::Conflict::of_named(buy.outpoint.txid, o, rec.escrow, fee, earlier)
            });
            let t = *t;
            return Ok(match next {
                Ok(c) => {
                    set_phase(st, id, FPhase::Abandoning { buy, conflict: c });
                    format!("the protected sale's setup {} confirmed first: the sale's own float input goes back to the float next",
                        hex::encode(t))
                }
                Err(e) => {
                    set_phase(st, id, FPhase::Stranded { buy, why: format!("a stale protected sale that cannot be conflicted out: {e}") });
                    format!("STRANDED: {e}")
                }
            });
        }
    }
    let hours = if st.protected_rec(id).is_some() { PROTECTED_STALE_SECS } else { BUY_STALE_SECS } / 3600;
    match stepped {
        Stepped::Waits(s) => Ok(s),
        Stepped::Settled(Settled::LeakedConfirmed | Settled::EarlierConfirmed(_)) => {
            protected_seen(io, st, id, &buy);
            set_phase(st, id, FPhase::Sent { buy });
            Ok("the buy confirmed before its conflict: bought after all".into())
        }
        Stepped::Settled(Settled::ConflictConfirmed(t) | Settled::SpentElsewhere(t)) => {
            Ok(fail_buy(io, st, id, format!("abandoned after {hours} h unconfirmed; the float's input was spent in {} so the buy can never \
                be mined", hex::encode(t)), format!("abandoned: {} confirmed; nothing was bought", hex::encode(t))))
        }
        Stepped::Stuck(why) => {
            set_phase(st, id, FPhase::Stranded { buy, why: format!("abandoning: conflict-out: {why}") });
            Ok(format!("STRANDED: abandoning: {why}"))
        }
    }
}

/// A buy that can never be mined (the listing vanished, a stale buy abandoned). Conversion review C-01: a frog whose conversion has
/// arrived is wanted again, its room still reserved (the module paid; the sats are still in the float); any other fails and its room
/// is freed (H2).
fn fail_buy<I: FrogIo>(io: &mut I, st: &mut FrogState, id: u64, why: String, line: String) -> String {
    if let Some(s) = crate::convflow::rewant(st, id, io.now(), &why) {
        return s;
    }
    set_phase(st, id, FPhase::Failed { why });
    let _ = io.with_caps(|_, b| custodycap::settle(b, &format!("frog:{id}"), None)); // H2: its room is free
    line
}

/// The frog flow's world as a stale buy's conflict needs it: the FLOAT's script and key, never custody's.
struct FloatVia<'a, I: FrogIo> {
    io: &'a mut I,
    st: &'a mut FrogState,
    id: u64,
    buy: &'a Bought,
}

impl<I: FrogIo> crate::conflict::ConflictIo for FloatVia<'_, I> {
    fn paused(&self) -> bool {
        self.io.paused()
    }
    fn now(&self) -> u64 {
        self.io.now()
    }
    async fn seen(&mut self, txid: [u8; 32]) -> crate::looper::BtcView {
        match self.io.tx_status(txid).await {
            BtcSeen::Confirmed { .. } => crate::looper::BtcView::Confirmed,
            BtcSeen::Mempool => crate::looper::BtcView::Mempool,
            BtcSeen::NotYet => crate::looper::BtcView::NotYet,
            BtcSeen::Unsure(e) => crate::looper::BtcView::Unsure(e),
        }
    }
    async fn output(&mut self, o: &Outpoint) -> Result<(custody::Output, bool), String> {
        self.io.output(o).await
    }
    async fn outspend(&mut self, o: &Outpoint) -> Result<Option<([u8; 32], bool)>, String> {
        self.io.outspend(o).await
    }
    async fn utxos(&mut self) -> Result<Vec<custody::Utxo>, String> {
        self.io.float_utxos().await
    }
    async fn contents(&mut self, o: &Outpoint) -> Result<Option<custody::Contents>, String> {
        self.io.contents(o).await
    }
    async fn fee_rate(&mut self) -> Result<u64, String> {
        self.io.fee_rate().await
    }
    async fn custody_script(&mut self) -> Result<Vec<u8>, String> {
        self.io.float_script().await // the conflict pays the float back, never custody
    }
    async fn sign_custody(&mut self, sighash: [u8; 32]) -> Result<Vec<u8>, String> {
        self.io.sign_float(sighash).await // the float's key: the input is the float's
    }
    async fn send(&mut self, raw: &[u8]) {
        let _ = self.io.btc_send(raw).await;
    }
    fn policy(&self, float_script: &[u8]) -> crate::outbound::BtcPolicy {
        self.io.outbound_policy(float_script)
    }
    fn save(&mut self, c: &crate::conflict::Conflict) {
        set_phase(self.st, self.id, FPhase::Abandoning { buy: self.buy.clone(), conflict: c.clone() });
        self.io.save(self.st);
    }
}

/// L-2: `frog:{id}` reservations whose frog is missing, still wanted (its signing never finished), or failed. Audit C2-01: only this
/// collection's (the reservation's collection key is `collection_key(io)`): on the books home `st` is one Inscriptions collection's
/// entry and the custody book is one book, so another collection's frog is always "missing" here, and its purchase in flight is not
/// this pass's to release.
fn sweep_stale<I: FrogIo>(io: &mut I, st: &mut FrogState) -> Vec<String> {
    let mine = crate::frogsol::collection_key(io);
    let stale: Vec<String> = io.with_caps(|_, b| b.entries.iter().filter(|e| e.in_flight && e.collection == mine).filter_map(|e| {
        let id: u64 = e.id.strip_prefix("frog:")?.parse().ok()?;
        let gone = match st.frogs.iter().find(|f| f.id == id) {
            None => true,
            // a wanted frog whose module has advanced keeps the room reserved at the advance
            Some(f) => matches!(f.phase, FPhase::Wanted { .. } | FPhase::Failed { .. }) && !st.conv_committed(id),
        };
        gone.then(|| e.id.clone())
    }).collect());
    for k in &stale {
        let _ = io.with_caps(|_, b| custodycap::settle(b, k, None));
    }
    if let Some(c) = st.buy_clock.as_mut() {
        let live: Vec<u64> = st.frogs.iter().filter(|f| matches!(f.phase, FPhase::Signed { .. } | FPhase::Sent { .. })).map(|f| f.id).collect();
        c.retain(|(f, _)| live.contains(f));
    }
    // a protected buy's record is kept while its sale may still be killed or watched: handed, sent, abandoning, and (review P-04)
    // held, from which a reorganisation puts it back to sent
    if let Some(p) = st.protected.as_mut() {
        let live: Vec<u64> = st.frogs.iter().filter(|f| matches!(f.phase, FPhase::Handed { .. } | FPhase::Signed { .. }
            | FPhase::Sent { .. } | FPhase::Abandoning { .. } | FPhase::Held { .. })).map(|f| f.id).collect();
        p.retain(|r| live.contains(&r.frog));
    }
    stale.into_iter().map(|k| format!("custody book: {k} was reserved for a purchase that is gone: released")).collect()
}

fn phase_name(p: &FPhase) -> &'static str {
    match p {
        FPhase::Wanted { .. } => "wanted",
        FPhase::Signed { .. } => "signed",
        FPhase::Sent { .. } => "sent",
        FPhase::Held { .. } => "held",
        FPhase::Minted { .. } => "minted",
        FPhase::Settled { .. } => "settled",
        FPhase::Stray { .. } => "stray at the desk",
        FPhase::Stranded { .. } => "stranded",
        FPhase::Failed { .. } => "failed",
        FPhase::Abandoning { .. } => "abandoning",
        FPhase::Handed { .. } => "handed to Ordinals Wallet",
    }
}

fn set_phase(st: &mut FrogState, id: u64, p: FPhase) {
    if let Some(f) = st.frog(id) {
        f.phase = p;
    }
}

/// Custody holds exactly this frog at the bought outpoint: ord's location and contents, the explorers' output. `Ok(Err)`:
/// something is wrong that waiting will not fix; `Err`: read again.
async fn custody_holds<I: FrogIo>(io: &mut I, buy: &Bought, cust: &[u8]) -> Result<Result<(), String>, String> {
    let loc = io.locate(&buy.inscription).await?.ok_or_else(|| format!("ord does not locate {} yet", buy.inscription))?;
    if loc.output != buy.outpoint {
        return Err(format!("ord places {} at {}, not {} yet", buy.inscription, loc.output.text(), buy.outpoint.text()));
    }
    let c = io.contents(&buy.outpoint).await?.ok_or("ord has not indexed the custody output yet")?;
    let (out, spent) = io.output(&buy.outpoint).await?;
    if spent || !out.confirmed {
        return Err("the explorers do not show the custody output confirmed and unspent".into());
    }
    if out.script != cust || out.value != loc.value || c.value != loc.value {
        return Ok(Err(format!("{} is not ours at {} sats on every source", buy.outpoint.text(), loc.value)));
    }
    if c.inscriptions != [buy.inscription.clone()] {
        return Ok(Err(format!("custody's output carries {:?}, not exactly {}", c.inscriptions, buy.inscription)));
    }
    if !c.runes.is_empty() {
        return Ok(Err(format!("custody's output also carries runes {:?}", c.runes)));
    }
    Ok(Ok(()))
}

/// Why an offer was not signed: a fault in the offer itself (a hand-in is slashed for it), a listing that is no longer
/// fillable through no fault of the offer, or something to read again next tick. The operator's path prints all three alike.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Why {
    Invalid(String),
    Gone(String),
    Later(String),
}

impl Why {
    pub fn text(&self) -> &str {
        match self {
            Why::Invalid(s) | Why::Gone(s) | Why::Later(s) => s,
        }
    }
}

/// What every offer, classic or protected, is held to before its own judge speaks: the membership proof, ord's location and
/// contents (exactly the inscription, no runes), the explorers' output (unspent, confirmed, values agreeing), and the cap in sats.
struct Located {
    loc: custody::Location,
    rate: u64,
    /// A frog the module advanced for (docs/v6/relay-conversion.md): the advance and the sats it bought.
    funded: Option<(u128, u64)>,
    cap_sats: u64,
}

async fn located<I: FrogIo>(io: &mut I, st: &FrogState, id: u64, cap_wei: u128, inscription: &str, proof: &[[u8; 32]])
    -> Result<Located, Why> {
    let later = |e: String| Why::Later(e);
    custody::check_inscription_id(inscription).map_err(Why::Invalid)?;
    let root = root(io).await.map_err(later)?;
    if !frogbuy::verify_proof(proof, &root, &frogbuy::leaf_of(inscription)) {
        return Err(Why::Invalid(format!("the proof for {inscription} does not verify against the listing's root")));
    }
    let loc = io.locate(inscription).await.map_err(later)?.ok_or_else(|| Why::Invalid(format!("ord does not know {inscription}")))?;
    let c = io.contents(&loc.output).await.map_err(later)?.ok_or_else(|| Why::Later("ord has not indexed the seller's output".into()))?;
    if c.inscriptions != [inscription.to_string()] || !c.runes.is_empty() {
        return Err(Why::Invalid(format!("the seller's output carries {:?} and runes {:?}, not exactly {inscription}", c.inscriptions, c.runes)));
    }
    let (out, spent) = io.output(&loc.output).await.map_err(later)?;
    if spent {
        return Err(Why::Gone("the seller's output is spent: the listing is gone".into()));
    }
    if !out.confirmed || out.value != loc.value || c.value != loc.value {
        return Err(Why::Later("the seller's output is not confirmed at ord's value on every explorer".into()));
    }
    let rate = io.rate().await.map_err(later)?;
    // a frog the module advanced for may spend what arrived from Relay, and costs its share of the advance
    let funded = crate::convflow::funded(st, id);
    let cap_sats = match funded {
        Some((_, sats)) => sats,
        None => frogbuy::sats_for_wei(cap_wei, rate).map_err(later)?,
    };
    Ok(Located { loc, rate, funded, cap_sats })
}

/// The judged spend's ETH cost, held to the policy's limits (or the collection's own maximum when a controller set one,
/// docs/v5/floor-feed.md §6.3) and to the frog's cap.
fn cost_checked(l: &Located, spend: u64, cap_wei: u128, limit: Option<u128>, min: Option<u128>, sol: bool) -> Result<u128, Why> {
    let later = |e: String| Why::Later(e);
    let cost_wei = match l.funded {
        Some((advanced, sats)) => crate::relayconv::cost_of(advanced, spend, sats).map_err(later)?,
        None => frogbuy::wei_for_sats(spend, l.rate).map_err(later)?,
    };
    // phase B2: in the home's unit (lamports on Solana)
    // docs/v9/ordinals-collections.md §7: the collection's own minimum when a controller set one (Solana home only)
    let (least, .., unit) = crate::frogsol::bounds_min(sol, min);
    if cost_wei < least {
        return Err(Why::Gone(crate::frogsol::check_limits_min(cost_wei, limit, min, sol).err().unwrap_or_default()));
    }
    crate::frogsol::check_limits_min(cost_wei, limit, min, sol).map_err(later)?;
    if cost_wei > cap_wei {
        return Err(Why::Later(format!("it would cost {cost_wei} {unit}, over the {cap_wei} cap")));
    }
    Ok(cost_wei)
}

/// The keys a frog buy is counted under: the collection, H3's value key, H2's flight key.
fn buy_keys<I: FrogIo>(io: &I, id: u64) -> (String, String, String) {
    let value_key = format!("buy:frog:{id}");
    (crate::frogsol::collection_key(io), value_key, format!("frog:{id}"))
}

/// H3 and H2, before anything is signed: the price against the day's value limit, then room under the caps, reserved. An
/// advanced frog was counted (H3) and its room reserved (H2) when the module paid.
/// `value_wei`: the cost as the caps and the limiter count it (the cost itself on the EVM home; lamports in wei on Solana).
fn count_and_reserve<I: FrogIo>(io: &mut I, id: u64, funded: bool, value_wei: u128) -> Result<(), Why> {
    if funded {
        return Ok(());
    }
    let (collection, value_key, flight) = buy_keys(io, id);
    io.outflow_check(&value_key, &collection, Flow::Value, value_wei).map_err(Why::Later)?;
    io.with_caps(|p, b| custodycap::reserve(p, b, &flight, &collection, value_wei)).map_err(|e| Why::Later(format!("custody cap: {e:?}")))
}

/// Judge an offer and, if it passes, sign it with the float key, save the bytes, then broadcast. `ask`: what a hand-in said
/// would leave the float; a spend over it is the submitter's lie (docs/v6/offer-handins.md §2.2).
async fn accept<I: FrogIo>(io: &mut I, st: &mut FrogState, id: u64, cap_wei: u128, limit: Option<u128>, min: Option<u128>, o: Offer, ask: Option<u64>)
    -> Result<String, Why> {
    let later = |e: String| Why::Later(e);
    let l = located(io, st, id, cap_wei, &o.inscription, &o.proof).await?;
    let (loc, rate, funded, cap_sats) = (l.loc.clone(), l.rate, l.funded, l.cap_sats);
    let psbt = Psbt::parse(&o.psbt).map_err(|e| Why::Invalid(format!("the PSBT does not parse: {e:?}")))?;
    let witnesses = psbt::final_witnesses(&o.psbt).map_err(|e| Why::Invalid(format!("{e:?}")))?;
    let (float, cust) = (io.float_script().await.map_err(later)?, io.custody_script().await.map_err(later)?);
    let seller = frogbuy::Seller { outpoint: loc.output, offset: loc.offset, value: loc.value };
    let j = frogbuy::judge(&psbt, &witnesses, &o.to_sign, &frogbuy::Facts { float: &float, custody: &cust, seller, cap_sats })
        .map_err(|e| match e {
            // over the cap is the offer's fault only when it spends more than it said; otherwise the cap moved
            frogbuy::Refused::OverCap { spend, cap } if ask.is_none_or(|a| spend <= a) =>
                Why::Later(format!("OverCap {{ spend: {spend}, cap: {cap} }}")),
            frogbuy::Refused::OverCap { spend, .. } => Why::Invalid(format!("it spends {spend} sats, more than the {} it said", ask.unwrap_or(0))),
            e => Why::Invalid(format!("{e:?}")),
        })?;
    if ask.is_some_and(|a| j.spend > a) {
        return Err(Why::Invalid(format!("it spends {} sats, more than the {} it said", j.spend, ask.unwrap_or(0))));
    }
    let sol = crate::frogsol::solana(io);
    let cost_wei = cost_checked(&l, j.spend, cap_wei, limit, min, sol)?;
    for &i in &j.ours {
        io.input_clean(custody::outpoint_of(&psbt.tx.inputs[i])).await.map_err(later)?;
    }
    // phase B2: the caps and the limiter count wei; a cost in lamports is converted at this tick's SOL/ETH rate, and kept
    let value = crate::frogsol::value_wei(io, cost_wei).await.map_err(later)?;
    count_and_reserve(io, id, funded.is_some(), value)?;
    if sol {
        st.put_value(id, value);
    }
    let (collection, value_key, flight) = buy_keys(io, id);
    let signed = async {
        let mut sigs = Vec::new();
        for &i in &j.ours {
            let h = crate::sighash::taproot_key_path(&psbt, i).map_err(|e| format!("digest: {e:?}"))?;
            sigs.push((i, io.sign_float(h).await?));
        }
        psbt::finalize(&psbt, &witnesses, &sigs)
    }.await;
    let raw = match signed {
        Ok(r) => r,
        Err(e) => {
            if funded.is_none() {
                let _ = io.with_caps(|_, b| custodycap::settle(b, &flight, None));
            }
            return Err(Why::Later(e));
        }
    };
    let txid = psbt.txid();
    let buy = Bought { inscription: o.inscription.clone(), outpoint: Outpoint { txid, vout: j.custody_vout }, seller: loc.output,
        spend_sats: j.spend, fee_sats: j.fee, rate, cost_wei, raw: raw.clone(), setup_txid: None };
    st.put_proof(o.inscription.clone(), o.proof.clone());
    set_phase(st, id, FPhase::Signed { buy });
    io.save(st);
    if funded.is_none() {
        io.outflow_record(&value_key, &collection, Flow::Value, value);
    }
    let _ = io.btc_send(&raw).await;
    Ok(format!("{} bought for {} sats ({} {}), signed, saved and sent in {}", o.inscription, j.spend, cost_wei, if sol { "lamports" } else { "wei" },
        hex::encode(txid)))
}

/// The float's key-path signatures (`SIGHASH_DEFAULT`) on inputs `ours` of a PSBT, written as `0x13` with the internal key as
/// `0x17`; every other byte as it came. Not finalized.
async fn sign_float_inputs<I: FrogIo>(io: &mut I, bytes: &[u8], ours: &[usize], internal_key: [u8; 32]) -> Result<Vec<u8>, String> {
    let p = Psbt::parse(bytes).map_err(|e| format!("the PSBT does not parse: {e:?}"))?;
    let mut sigs = Vec::with_capacity(ours.len());
    for &i in ours {
        let h = crate::sighash::taproot_key_path(&p, i).map_err(|e| format!("digest: {e:?}"))?;
        let sig: [u8; 64] = io.sign_float(h).await?.try_into()
            .map_err(|s: Vec<u8>| format!("a float signature of {} bytes, not 64", s.len()))?;
        sigs.push((i, sig));
    }
    psbt::with_tap_key_sigs(bytes, &sigs, internal_key)
}

/// Judge a protected Ordinals Wallet sale (docs/v8/ow-protected.md §3.1) after the checks every offer passes, and if it passes,
/// sign the float's inputs of the setup and the sale, save the signed PSBTs and hand them back (§3.2). Never finalized, never
/// broadcast: Ordinals Wallet co-signs the escrow input and relays the package.
async fn accept_protected<I: FrogIo>(io: &mut I, st: &mut FrogState, id: u64, cap_wei: u128, limit: Option<u128>, min: Option<u128>, o: ProtectedOffer)
    -> Result<String, Why> {
    let later = |e: String| Why::Later(e);
    let l = located(io, st, id, cap_wei, &o.inscription, &o.proof).await?;
    let (loc, rate, funded, cap_sats) = (l.loc.clone(), l.rate, l.funded, l.cap_sats);
    let (float, cust) = (io.float_script().await.map_err(later)?, io.custody_script().await.map_err(later)?);
    // review P-01: royalties only to the creators pinned in the config; P-02: the miner fee against our own fee rate
    let creators = st.ow_creator_scripts();
    let fee_rate = io.fee_rate().await.map_err(later)?;
    let facts = crate::owbuy::ProtectedFacts { float: &float, custody: &cust,
        inscription: frogbuy::Seller { outpoint: loc.output, offset: loc.offset, value: loc.value }, cap_sats, creators: &creators,
        fee_rate: Some(fee_rate) };
    let j = crate::owbuy::judge_protected(&o.sale_psbt, &o.parent, &o.listing, &facts, o.setup_psbt.as_deref())
        .map_err(|e| match e {
            // as for a classic offer: over the cap is the cap having moved (the runner built for the cap it read)
            crate::owbuy::Refused::OverCap { .. } => Why::Later(format!("{e}")),
            e => Why::Invalid(format!("{e}")),
        })?;
    let sol = crate::frogsol::solana(io);
    let cost_wei = cost_checked(&l, j.spend, cap_wei, limit, min, sol)?;
    let sale = Psbt::parse(&o.sale_psbt).map_err(|e| Why::Invalid(format!("the sale PSBT does not parse: {e:?}")))?;
    let setup = match &o.setup_psbt {
        Some(b) => Some(Psbt::parse(b).map_err(|e| Why::Invalid(format!("the setup PSBT does not parse: {e:?}")))?),
        None => None,
    };
    let with_values = |p: &Psbt, ours: &[usize]| -> Vec<(Outpoint, u64)> {
        ours.iter().map(|&i| (custody::outpoint_of(&p.tx.inputs[i]), p.witness_utxos[i].as_ref().map_or(0, |u| u.value))).collect()
    };
    let sale_inputs = with_values(&sale, &j.float_inputs);
    let setup_inputs = match (&setup, &j.setup) {
        (Some(p), Some(s)) => with_values(p, &s.float_inputs),
        _ => vec![],
    };
    // ord shows every float coin this buy spends carrying nothing. With a setup, the sale's float inputs are the setup's own
    // outputs (P10), which ord cannot know before the setup confirms; their sats are the setup inputs', checked here.
    let checked = if j.setup.is_some() { &setup_inputs } else { &sale_inputs };
    for (op, _) in checked {
        io.input_clean(*op).await.map_err(later)?;
    }
    let value = crate::frogsol::value_wei(io, cost_wei).await.map_err(later)?;
    count_and_reserve(io, id, funded.is_some(), value)?;
    if sol {
        st.put_value(id, value);
    }
    let (_, _, flight) = buy_keys(io, id);
    let signed = async {
        let key = io.float_internal_key().await?;
        let setup_signed = match (&o.setup_psbt, &j.setup) {
            (Some(b), Some(s)) => Some(sign_float_inputs(io, b, &s.float_inputs, key).await?),
            _ => None,
        };
        let sale_signed = sign_float_inputs(io, &o.sale_psbt, &j.float_inputs, key).await?;
        Ok::<_, String>((sale_signed, setup_signed))
    }.await;
    let (sale_signed, setup_signed) = match signed {
        Ok(s) => s,
        Err(e) => {
            if funded.is_none() {
                let _ = io.with_caps(|_, b| custodycap::settle(b, &flight, None));
            }
            return Err(Why::Later(e));
        }
    };
    let escrow = Outpoint { txid: j.parent_txid, vout: 0 };
    let now = io.now();
    let buy = Bought { inscription: o.inscription.clone(), outpoint: Outpoint { txid: j.sale_txid, vout: j.custody_vout }, seller: escrow,
        spend_sats: j.spend, fee_sats: j.fee, rate, cost_wei, raw: vec![], setup_txid: j.setup.as_ref().map(|s| s.txid) };
    st.put_proof(o.inscription.clone(), o.proof.clone());
    st.put_protected(ProtectedRec { frog: id, sale_txid: j.sale_txid, sale_psbt: sale_signed, setup_psbt: setup_signed,
        setup_txid: j.setup.as_ref().map(|s| s.txid), sale_inputs, setup_inputs, sale_fee: j.sale_fee,
        setup_fee: j.setup.as_ref().map_or(0, |s| s.fee), escrow, at: now, listing: Some(o.listing.outpoint), seen: None });
    set_phase(st, id, FPhase::Handed { buy, sale_txid: hex::encode(j.sale_txid), at: now });
    io.save(st);
    // review P-02: the value is counted (H3) once Bitcoin shows the sale (`protected_seen`), not at the hand-back
    Ok(format!("{} bought for {} sats ({} wei) through a protected listing: signed, saved and handed back in {} (Ordinals Wallet \
        broadcasts; nothing sent here)", o.inscription, j.spend, cost_wei, hex::encode(j.sale_txid)))
}

// ---- who gets which frog -------------------------------------------------------------------------------------------------------

fn new_frog(st: &mut FrogState, target: Target, cap_wei: u128) -> u64 {
    let id = st.next_frog;
    st.next_frog += 1;
    st.frogs.push(FrogRec { id, target: Some(target), phase: FPhase::Wanted { cap_wei } });
    id
}

async fn assign<I: FrogIo>(io: &mut I, st: &mut FrogState, fl: &mut FloorState) -> Vec<String> {
    let mut lines = Vec::new();
    let desk = st.desk;
    let ttl = match desk {
        Some(d) => desk_ttl(io, d).await.ok(),
        None => None,
    };
    let clock = io.clock().await.ok();
    // 1. release: held frogs whose target cannot take them become inventory; wanted ones whose target is gone fail
    for i in 0..st.frogs.len() {
        let f = st.frogs[i].clone();
        let Some(t) = f.target else { continue };
        let keep = match (&f.phase, t) {
            // the module advanced for this frog (docs/v6/relay-conversion.md): it is that module's, whatever it reads now
            (_, Target::Sink(_)) if st.conv_committed(f.id) || st.advanced_for(f.id).is_some_and(|_| f.bought().is_some()) => true,
            (FPhase::Held { buy }, Target::Sink(s)) => st.sink(&s).and_then(|r| r.view.clone()).is_some_and(|v| itemsink::may_mint(&v, buy.cost_wei).is_ok()),
            (FPhase::Held { buy }, Target::Desk(r)) => request_takes(st, r, ttl, clock, true).is_ok() && req_amount(st, r) >= buy.cost_wei,
            (FPhase::Wanted { .. }, Target::Sink(s)) => st.sink(&s).and_then(|r| r.view.clone())
                .is_some_and(|v| itemsink::may_start(&v, itemsink::assigned(&st.frogs, &s).0 - 1).is_ok()),
            (FPhase::Wanted { .. }, Target::Desk(r)) => request_takes(st, r, ttl, clock, false).is_ok(),
            _ => true,
        };
        if keep {
            continue;
        }
        match f.phase {
            FPhase::Held { .. } => {
                st.frogs[i].target = None;
                lines.push(format!("frog {}: its target can no longer take it: inventory", f.id));
            }
            FPhase::Wanted { .. } => {
                crate::convflow::drop_uncommitted(st, f.id);
                st.frogs[i].phase = FPhase::Failed { why: "no acceptable offer before its target stopped needing it".into() };
                lines.push(format!("frog {}: no offer within the cap in time: failed, nothing spent", f.id));
            }
            _ => {}
        }
    }
    // 2. sinks first (a deadline shared by every funder), then desk requests: inventory before a new purchase
    let acquiring = st.frogs.iter().any(|f| f.acquiring()) || st.conv_open();
    let sinks: Vec<[u8; 20]> = st.sinks.iter().filter(|s| !s.done() && s.view.is_some()).map(|s| s.sink).collect();
    let mut started = acquiring;
    for s in sinks {
        let v = st.sink(&s).and_then(|r| r.view.clone()).expect("filtered");
        // an EVM floor sink: the same trigger starts an NFT want, not a frog (docs/v5/floor-feed.md §6.2)
        if fl.is_evm(&s) {
            for id in fl.release(&s, &v) {
                lines.push(format!("nft want {id}: its module no longer wants it: failed, nothing spent"));
            }
            // H2: no want starts for a collection custody has no room for
            if let Some(FloorItem::Evm { chain_id, collection, .. }) = fl.item_of(&s).cloned() {
                if !fl.wants.iter().any(|x| x.module == s && x.phase == WPhase::Wanted) {
                    if let Err(e) = room(io, &custodycap::evm_key(chain_id, &collection)) {
                        lines.push(format!("Floor 0x{}: no purchase: {e}", hex::encode(s)));
                        continue;
                    }
                }
            }
            match fl.plan_evm(s, &v) {
                Ok(Some(id)) => lines.push(format!("nft want {id}: wanted for Floor 0x{} at most {} wei; waiting for an order", hex::encode(s),
                    fl.wants.iter().find(|w| w.id == id).map_or(0, |w| w.cap_wei))),
                Ok(None) => {}
                Err(e) => lines.push(format!("Floor 0x{}: no purchase: {e}", hex::encode(s))),
            }
            continue;
        }
        let (n, w) = itemsink::assigned(&st.frogs, &s);
        let Some(cap) = itemsink::per_item_cap(&v, n, w) else { continue };
        if let Some(i) = inventory(st, |b| b.cost_wei <= cap && itemsink::may_mint(&v, b.cost_wei).is_ok()) {
            st.frogs[i].target = Some(Target::Sink(s));
            lines.push(format!("frog {}: from inventory to {:?} 0x{}", st.frogs[i].id, v.kind, hex::encode(s)));
            continue;
        }
        if started {
            continue;
        }
        match itemsink::may_start(&v, n).and_then(|_| room(io, &crate::frogsol::collection_key(io))) {
            Ok(()) => {
                // docs/v6/relay-conversion.md: with Relay funding, the module's own ETH pays, before the frog; an operator
                // float is used first only when the install says so and it covers the cap
                let relay = v.kind == SinkKind::Floor && use_relay(io, cap).await;
                // conversion review C-08: a day between a module's returned advance and its next one (the Ethereum side's rule)
                if let Some(t) = relay.then(|| crate::convflow::cooling(st, &s, io.now())).flatten() {
                    lines.push(format!("Floor 0x{}: no purchase until {t}: its last advance came back less than a day ago (C-08)", hex::encode(s)));
                    continue;
                }
                let id = new_frog(st, Target::Sink(s), cap);
                started = true;
                if relay {
                    crate::convflow::start(st, id, s);
                }
                lines.push(format!("frog {id}: wanted for {:?} 0x{} at most {cap} {}; waiting for an offer", v.kind, hex::encode(s),
                    if crate::frogsol::solana(io) { "lamports" } else { "wei" }));
            }
            Err(e) => lines.push(format!("{:?} 0x{}: no purchase: {e}", v.kind, hex::encode(s))),
        }
    }
    if desk.is_some() {
        let open: Vec<u64> = st.requests.iter().filter(|r| !r.done).map(|r| r.id).collect();
        for r in open {
            if st.frogs.iter().any(|f| f.target == Some(Target::Desk(r)) && !f.finished()) {
                continue;
            }
            let amount = req_amount(st, r);
            // a stray unit of ours at the desk first: it fills with `fulfill` alone
            if let Some(i) = st.frogs.iter().position(|f| f.target.is_none() && matches!(&f.phase, FPhase::Stray { buy } if buy.cost_wei <= amount)) {
                if request_takes(st, r, ttl, clock, true).is_ok() {
                    let FPhase::Stray { buy } = st.frogs[i].phase.clone() else { unreachable!() };
                    st.frogs[i].phase = FPhase::Minted { buy };
                    st.frogs[i].target = Some(Target::Desk(r));
                    lines.push(format!("frog {}: its stray unit at the desk goes to request {r}", st.frogs[i].id));
                    continue;
                }
            }
            if request_takes(st, r, ttl, clock, true).is_ok() {
                if let Some(i) = inventory(st, |b| b.cost_wei <= amount) {
                    st.frogs[i].target = Some(Target::Desk(r));
                    lines.push(format!("frog {}: from inventory to request {r}", st.frogs[i].id));
                    continue;
                }
            }
            if started {
                continue;
            }
            match request_takes(st, r, ttl, clock, false).and_then(|_| room(io, &crate::frogsol::collection_key(io))) {
                Ok(()) => {
                    let id = new_frog(st, Target::Desk(r), amount);
                    started = true;
                    lines.push(format!("frog {id}: wanted for request {r} at most {amount} wei; waiting for an offer"));
                }
                Err(e) => {
                    if let Some(x) = st.requests.iter_mut().find(|x| x.id == r) {
                        x.note = e;
                    }
                }
            }
        }
    }
    lines
}

/// A new floor frog is paid for through Relay: funding is Relay, unless `float_first` and the float already covers the cap.
async fn use_relay<I: FrogIo>(io: &mut I, cap_wei: u128) -> bool {
    let Some(f) = io.funding() else { return false };
    if !f.float_first {
        return true;
    }
    let (Ok(rate), Ok(utxos)) = (io.rate().await, io.float_utxos().await) else { return true };
    let have: u64 = utxos.iter().filter(|u| u.confirmed).map(|u| u.value).sum();
    frogbuy::sats_for_wei(cap_wei, rate).map_or(true, |need| have < need)
}

/// A held frog with no target that `ok` accepts. Conversion review C-01: never one the module advanced for (it is that module's).
fn inventory(st: &FrogState, ok: impl Fn(&Bought) -> bool) -> Option<usize> {
    st.frogs.iter().position(|f| f.target.is_none() && st.advanced_for(f.id).is_none() && matches!(&f.phase, FPhase::Held { buy } if ok(buy)))
}

fn req_amount(st: &FrogState, r: u64) -> u128 {
    st.requests.iter().find(|x| x.id == r).map_or(0, |x| x.request.amount)
}

fn request_takes(st: &FrogState, r: u64, ttl: Option<u64>, clock: Option<Clock>, have_frog: bool) -> Result<(), String> {
    let x = st.requests.iter().find(|x| x.id == r).ok_or("no such request")?;
    let (Some(ttl), Some(clock)) = (ttl, clock) else { return Err("the desk's clock is not read".into()) };
    mintdesk::may_take(&x.request, ttl, &clock, have_frog)
}

// ---- holder deposits ---------------------------------------------------------------------------------------------------------------

async fn deposits_pass<I: FrogIo>(io: &mut I, st: &mut FrogState, fl: &FloorState, delivery_reserved: &[Outpoint]) -> Vec<String> {
    // H-2 (audit D): switched off, nothing is read, classified or returned
    if !io.deposits_on() {
        return vec![];
    }
    let mut lines = Vec::new();
    // 1. new custody outputs, classified once; at most MAX_NEW_OUTPUTS_PER_TICK a tick, and none past MAX_OPEN_DEPOSITS
    match io.custody_utxos().await {
        Ok(utxos) => {
            // H-3: what was seen stays bounded by what is unspent
            st.seen.retain(|o| utxos.iter().any(|u| u.outpoint == *o));
            st.unmarked.retain(|o| utxos.iter().any(|u| u.outpoint == *o));
            let ours: Vec<Outpoint> = st.frogs.iter().filter_map(|f| f.bought().map(|b| b.outpoint)).collect();
            let unread = st.unread.clone().unwrap_or_default();
            let mut open = st.deposits.iter().filter(|d| !d.finished()).count();
            let mut n = 0;
            for u in utxos {
                if n >= MAX_NEW_OUTPUTS_PER_TICK || st.seen.contains(&u.outpoint) || unread.contains(&u.outpoint) {
                    continue;
                }
                if ours.contains(&u.outpoint) {
                    st.seen.push(u.outpoint);
                    continue;
                }
                if open >= deposits::MAX_OPEN_DEPOSITS {
                    lines.push(format!("deposits: {open} open, the most tracked at once: new custody outputs wait"));
                    break;
                }
                n += 1;
                match io.tx_view(u.outpoint.txid).await {
                    Ok(tx) => match deposits::marker(&tx.outputs) {
                        Marked::No => {
                            if let Ok(Some(c)) = io.contents(&u.outpoint).await {
                                if !c.inscriptions.is_empty() || !c.runes.is_empty() {
                                    st.unmarked.push(u.outpoint);
                                }
                            }
                            st.seen.push(u.outpoint);
                        }
                        // audit R2, N-7: `seen` is pruned to the confirmed listing, so a deposit reorganised out and back
                        // is seen again; its record already exists and is never pushed twice
                        Marked::Yes(_) if st.deposits.iter().any(|d| d.outpoint == u.outpoint) => {
                            st.seen.push(u.outpoint);
                        }
                        Marked::Yes(_) => {
                            st.deposits.push(DepositRec { outpoint: u.outpoint, first_seen: io.now(),
                                phase: DPhase::Pending { why: "new".into() }, admitted: None, next_check: None, tries: None });
                            st.seen.push(u.outpoint);
                            open += 1;
                            lines.push(format!("deposit {}: marked; checks begin", u.outpoint.text()));
                        }
                    },
                    Err(e) => {
                        // H-3: a transaction that cannot be read (padded past the cap, say) is set aside after a few tries
                        let fails = st.read_failures.get_or_insert_with(Vec::new);
                        let k = match fails.iter_mut().find(|(o, _)| *o == u.outpoint) {
                            Some(x) => { x.1 += 1; x.1 }
                            None => { fails.push((u.outpoint, 1)); 1 }
                        };
                        if k >= deposits::MAX_READ_FAILURES {
                            fails.retain(|(o, _)| *o != u.outpoint);
                            st.unread.get_or_insert_with(Vec::new).push(u.outpoint);
                            lines.push(format!("custody output {}: its transaction could not be read {k} times: set aside for a person ({e})",
                                u.outpoint.text()));
                        } else {
                            lines.push(format!("custody output {} not read ({k} of {}): {e}", u.outpoint.text(), deposits::MAX_READ_FAILURES));
                        }
                    }
                }
            }
        }
        Err(e) => lines.push(format!("deposits: custody not read: {e}")),
    }
    io.save(st);
    // 2. the deposits due, earliest first, at most MAX_DEPOSITS_STEPPED_PER_TICK: a stranger's many deposits cost what four do
    let now = io.now();
    let mut due: Vec<usize> = (0..st.deposits.len()).filter(|&i| {
        let d = &st.deposits[i];
        if d.finished() || d.next_check.is_some_and(|t| t > now) {
            return false;
        }
        // one waiting for a proof is read again only once a proof for its inscription is here (or to park it)
        match &d.phase {
            DPhase::AwaitingProof { inscription } => st.proof(inscription).is_some() || now.saturating_sub(d.first_seen) >= deposits::PARK_AFTER_SECS,
            _ => true,
        }
    }).collect();
    due.sort_by_key(|&i| (st.deposits[i].next_check.unwrap_or(0), i));
    due.truncate(deposits::MAX_DEPOSITS_STEPPED_PER_TICK);
    for i in due {
        let mut reserved: Vec<Outpoint> = delivery_reserved.to_vec();
        reserved.extend(st.deposits.iter().enumerate().filter(|(j, _)| *j != i).flat_map(|(_, d)| d.reserved()));
        let mut d = st.deposits[i].clone();
        let before = d.phase.clone();
        let mark = deposit_mark(io, st, fl);
        let r = step_deposit(io, st, &mut d, &reserved, mark).await;
        // H-3: a look that changed nothing backs off; one that moved it resets
        if d.phase == before {
            let tries = d.tries.unwrap_or(0).saturating_add(1);
            d.tries = Some(tries);
            d.next_check = Some(deposits::next_check(io.now(), tries));
        } else {
            d.tries = None;
            d.next_check = None;
        }
        let line = match r {
            Ok(s) => format!("deposit {}: {s}", d.outpoint.text()),
            Err(e) => format!("deposit {}: waits: {e}", d.outpoint.text()),
        };
        st.deposits[i] = d;
        io.save(st);
        lines.push(line);
    }
    lines
}

/// The checks, in order; the first that fails decides. `Ok(Ok(id, recipient))`: ready to mint.
async fn check_deposit<I: FrogIo>(io: &mut I, st: &FrogState, o: &Outpoint)
    -> Result<Result<(String, [u8; 20]), DPhase>, String> {
    let tx = io.tx_view(o.txid).await?;
    let (Some(height), Some(hash)) = (tx.height, tx.block_hash.clone()) else {
        return Ok(Err(DPhase::Pending { why: "not in a block".into() }));
    };
    if io.block_hash(height).await? != hash {
        return Ok(Err(DPhase::Pending { why: format!("block {height} was reorganised: read again") }));
    }
    let d = frogbuy::depth(io.tip().await?, height);
    if d < CONFIRMATIONS {
        return Ok(Err(DPhase::Pending { why: format!("{d} of {CONFIRMATIONS} confirmations") }));
    }
    // H-2 (audit D): plain sats are never returned, whatever the marker says, so a junk "deposit" costs its sender an
    // inscription at least. What the output carries is read before the marker is judged.
    let contents = io.contents(o).await?;
    match &contents {
        // ord behind: wait, and judge nothing (a plain output must never be refused, and so returned, for want of a read)
        None => return Ok(Err(DPhase::Pending { why: format!("ord has not indexed {} yet", o.text()) })),
        Some(c) if c.inscriptions.is_empty() && c.runes.is_empty() => return Ok(Err(DPhase::NotAFrog)),
        Some(_) => {}
    }
    let m = match deposits::marker(&tx.outputs) {
        Marked::No => return Err("the marker is gone from a transaction that had one: explorers disagree with themselves".into()),
        Marked::Yes(Err(e)) => return Ok(Err(DPhase::Refused { reason: Reason::Malformed(e) })),
        Marked::Yes(Ok(m)) => m,
    };
    let cust = io.custody_script().await?;
    if let Err(r) = deposits::check_tx(&tx, &m, io.chain_id(), &io.wrapper(), &cust) {
        return Ok(Err(DPhase::Refused { reason: r }));
    }
    let located = match contents.as_ref().and_then(|c| c.inscriptions.first().cloned()) {
        Some(id) if contents.as_ref().is_some_and(|c| c.inscriptions.len() == 1) => io.locate(&id).await?,
        _ => None,
    };
    let (out, spent) = io.output(o).await?;
    let id = match deposits::check_output(o, contents.as_ref(), located.as_ref(), &out, spent, &cust) {
        Ok(Some(id)) => id,
        Ok(None) => return Ok(Err(DPhase::NotAFrog)),
        Err(Ok(r)) => return Ok(Err(DPhase::Refused { reason: r })),
        Err(Err(w)) => return Ok(Err(DPhase::Pending { why: w })),
    };
    let Some(proof) = st.proof(&id) else {
        // H-3: not read again until a proof for it is handed in
        return Ok(Err(DPhase::AwaitingProof { inscription: id }));
    };
    if let Err(e) = proof_ok(io, &id, &proof).await {
        return Ok(Err(DPhase::Pending { why: e }));
    }
    let w = io.wrapper();
    if flag(io, w, deposits::DEPOSIT_MINTED, deposits::outpoint_key(o.txid, o.vout)).await? {
        return Ok(Err(DPhase::Minted { inscription: id, recipient: m.recipient }));
    }
    if flag(io, w, deposits::INSCRIPTION_HELD, frogbuy::leaf_of(&id)).await? {
        return Ok(Err(DPhase::Pending { why: format!("the wrapper already counts {id} held: waiting for its delivery record") }));
    }
    if !deposits::under_limit(&st.mint_times, io.now()) {
        return Ok(Err(DPhase::Pending { why: format!("{} deposits minted in the last day: the limit", deposits::DEPOSITS_PER_DAY) }));
    }
    Ok(Ok((id, m.recipient)))
}

/// What a holder deposit counts for under the caps (H2): the highest fresh per-item cap among our wrapper's sinks (what the
/// item would be bought for), or the collection's per-item maximum when no sink says.
fn deposit_mark<I: FrogIo>(io: &mut I, st: &FrogState, fl: &FloorState) -> u128 {
    let fresh = st.sinks.iter().filter(|s| !fl.is_evm(&s.sink) && !s.done()).filter_map(|s| s.view.as_ref().and_then(|v| v.item_cap)).max();
    let key = custodycap::btc_key(&io.wrapper());
    fresh.unwrap_or_else(|| io.with_caps(|p, _| p.cap(&key).map_or(0, |c| c.max_item_wei)))
}

/// Count a deposit under the caps once (H2): it is admitted before it is minted, and one over the cap is refused, to be
/// returned to its depositor.
fn admit_deposit<I: FrogIo>(io: &mut I, d: &mut DepositRec, inscription: &str, mark: u128) -> Result<(), custodycap::Refusal> {
    if d.admitted.as_deref() == Some(inscription) {
        return Ok(());
    }
    let key = custodycap::btc_key(&io.wrapper());
    io.with_caps(|p, b| if b.contains(inscription) { Ok(()) } else { custodycap::admit(p, b, inscription, &key, mark) })?;
    d.admitted = Some(inscription.to_string());
    Ok(())
}

async fn step_deposit<I: FrogIo>(io: &mut I, st: &mut FrogState, d: &mut DepositRec, reserved: &[Outpoint], mark: u128) -> Result<String, String> {
    // H-3: waited PARK_AFTER_SECS for a proof: parked, and read no more
    if let DPhase::AwaitingProof { inscription } = d.phase.clone() {
        if st.proof(&inscription).is_none() && io.now().saturating_sub(d.first_seen) >= deposits::PARK_AFTER_SECS {
            d.phase = DPhase::Parked { inscription: inscription.clone() };
            return Ok(format!("PARKED: no membership proof for {inscription} in {} days (refuse_deposit returns it; frog_proof wakes it)",
                deposits::PARK_AFTER_SECS / deposits::DAY_SECS));
        }
    }
    match d.phase.clone() {
        DPhase::Pending { .. } | DPhase::Ready { .. } | DPhase::AwaitingProof { .. } => match check_deposit(io, st, &d.outpoint).await? {
            Ok((inscription, recipient)) => {
                if let Err(e) = admit_deposit(io, d, &inscription, mark) {
                    d.phase = DPhase::Refused { reason: Reason::OverCustodyCap(format!("{e:?}")) };
                    return Ok(format!("REFUSED: {inscription} does not fit under the custody caps ({e:?}): it is returned"));
                }
                d.phase = DPhase::Ready { inscription: inscription.clone(), recipient };
                Ok(format!("{inscription} for 0x{}: ready to mint", hex::encode(recipient)))
            }
            Err(p) => {
                let s = match &p {
                    DPhase::Pending { why } => format!("waits: {why}"),
                    DPhase::Refused { reason } => format!("REFUSED: {reason:?}"),
                    DPhase::Minted { inscription, .. } => {
                        // minted already: in custody whatever the caps say, so it is counted (never refused here)
                        let _ = admit_deposit(io, d, inscription, mark);
                        "already minted (the wrapper says so): adopted".into()
                    }
                    DPhase::NotAFrog => "plain sats with a marker: nothing to mint or return".into(),
                    DPhase::AwaitingProof { inscription } => format!("waits for a membership proof for {inscription} (frog_proof), or the \
                        operator's refusal: not read again until one"),
                    _ => String::new(),
                };
                d.phase = p;
                Ok(s)
            }
        },
        DPhase::Refused { reason } => {
            if !deposits::returns_under_limit(&st.return_times, io.now()) {
                return Ok(format!("waits: {} returns sent in the last day: the limit", deposits::RETURNS_PER_DAY));
            }
            give_back(io, st, d, reason, reserved).await
        }
        DPhase::Returning { reason, txid, raw, fee_input, unseen_since } => match io.tx_status(txid).await {
            BtcSeen::Confirmed { .. } => {
                d.phase = DPhase::Returned { reason, txid };
                release_deposit(io, d);
                pool_outputs(io, txid, &raw, 1);
                Ok(format!("returned: {} confirmed", hex::encode(txid)))
            }
            BtcSeen::Mempool => {
                if unseen_since.is_some() {
                    d.phase = DPhase::Returning { reason, txid, raw, fee_input, unseen_since: None };
                }
                Ok("the return is in a mempool".into())
            }
            BtcSeen::NotYet => {
                let now = io.now();
                let since = unseen_since.unwrap_or(now);
                if now.saturating_sub(since) < crate::conflict::RETURN_STALE_SECS {
                    d.phase = DPhase::Returning { reason, txid, raw: raw.clone(), fee_input, unseen_since: Some(since) };
                    let _ = io.btc_send(&raw).await;
                    return Ok("the same saved return sent again".into());
                }
                // H8: stranded (evicted, or never relayed) for RETURN_STALE_SECS: abandoned. Its fee input is spent back to custody
                // first, so the old bytes can never be mined; then a fresh return is built.
                let conflict = match crate::conflict::Conflict::of(txid, &raw, fee_input) {
                    Ok(c) => c,
                    Err(e) => {
                        d.phase = DPhase::Held { reason, why: format!("an evicted return whose saved bytes cannot be conflicted out: {e}") };
                        return Ok(format!("HELD: {e}"));
                    }
                };
                d.phase = DPhase::ReturnConflicting { reason, conflict };
                return_conflicting(io, st, d, reserved).await
                    .map(|s| format!("the return was not seen for {} h: abandoned; conflict-out: {s}", crate::conflict::RETURN_STALE_SECS / 3600))
            }
            BtcSeen::Unsure(e) => Err(e),
        },
        DPhase::ReturnConflicting { .. } => return_conflicting(io, st, d, reserved).await,
        other => Ok(format!("{other:?}")),
    }
}

/// H2: a deposit that left custody frees its room under the caps, if it was ever counted.
fn release_deposit<I: FrogIo>(io: &mut I, d: &DepositRec) {
    if let Some(i) = d.admitted.clone() {
        let _ = io.with_caps(|_, b| custodycap::release(b, &i));
    }
}

/// One pass of a `ReturnConflicting` deposit through `conflict::step` (H8), and where each ending leaves it.
async fn return_conflicting<I: FrogIo>(io: &mut I, st: &mut FrogState, d: &mut DepositRec, reserved: &[Outpoint]) -> Result<String, String> {
    let DPhase::ReturnConflicting { reason, mut conflict } = d.phase.clone() else { return Err("not in conflict".into()) };
    let stepped = {
        let mut via = FrogVia { io: &mut *io, st: &mut *st, d: &mut *d, reason: &reason };
        crate::conflict::step(&mut via, &mut conflict, reserved).await?
    };
    use crate::conflict::{Settled, Stepped};
    match stepped {
        Stepped::Waits(s) => Ok(s),
        Stepped::Settled(Settled::LeakedConfirmed) => {
            d.phase = DPhase::Returned { reason, txid: conflict.leaked_txid };
            release_deposit(io, d);
            Ok(format!("the abandoned return {} confirmed after all: returned", hex::encode(conflict.leaked_txid)))
        }
        Stepped::Settled(Settled::EarlierConfirmed(t)) => {
            d.phase = DPhase::Returned { reason, txid: t };
            release_deposit(io, d);
            Ok(format!("an earlier version {} of the abandoned return confirmed: returned", hex::encode(t)))
        }
        Stepped::Settled(Settled::ConflictConfirmed(t) | Settled::SpentElsewhere(t)) => {
            if let Some(tx) = conflict.tx.as_ref().filter(|x| x.txid == t) {
                pool_outputs(io, t, &tx.raw, 0);
            }
            // the old return can never be mined: refused again, so the next pass builds a fresh one at today's rate
            d.phase = DPhase::Refused { reason };
            Ok(format!("the abandoned return's fee input is spent in {} (confirmed): a fresh return is built next", hex::encode(t)))
        }
        Stepped::Stuck(why) => {
            d.phase = DPhase::Held { reason, why: format!("conflict-out: {why}") };
            Ok(format!("HELD: conflict-out: {why}"))
        }
    }
}

/// The frog flow's world, as the conflict needs it. `save` writes the conflict into the deposit, the deposit into the state, and
/// saves the state.
struct FrogVia<'a, I: FrogIo> {
    io: &'a mut I,
    st: &'a mut FrogState,
    d: &'a mut DepositRec,
    reason: &'a Reason,
}

impl<I: FrogIo> crate::conflict::ConflictIo for FrogVia<'_, I> {
    fn paused(&self) -> bool {
        self.io.paused()
    }
    fn now(&self) -> u64 {
        self.io.now()
    }
    async fn seen(&mut self, txid: [u8; 32]) -> crate::looper::BtcView {
        match self.io.tx_status(txid).await {
            BtcSeen::Confirmed { .. } => crate::looper::BtcView::Confirmed,
            BtcSeen::Mempool => crate::looper::BtcView::Mempool,
            BtcSeen::NotYet => crate::looper::BtcView::NotYet,
            BtcSeen::Unsure(e) => crate::looper::BtcView::Unsure(e),
        }
    }
    async fn output(&mut self, o: &Outpoint) -> Result<(custody::Output, bool), String> {
        self.io.output(o).await
    }
    async fn outspend(&mut self, o: &Outpoint) -> Result<Option<([u8; 32], bool)>, String> {
        self.io.outspend(o).await
    }
    async fn utxos(&mut self) -> Result<Vec<custody::Utxo>, String> {
        self.io.fee_utxos().await
    }
    async fn contents(&mut self, o: &Outpoint) -> Result<Option<custody::Contents>, String> {
        self.io.contents(o).await
    }
    async fn fee_rate(&mut self) -> Result<u64, String> {
        self.io.fee_rate().await
    }
    async fn custody_script(&mut self) -> Result<Vec<u8>, String> {
        self.io.custody_script().await
    }
    async fn sign_custody(&mut self, sighash: [u8; 32]) -> Result<Vec<u8>, String> {
        self.io.sign_custody(sighash).await
    }
    async fn send(&mut self, raw: &[u8]) {
        let _ = self.io.btc_send(raw).await;
    }
    fn policy(&self, custody_script: &[u8]) -> crate::outbound::BtcPolicy {
        self.io.outbound_policy(custody_script)
    }
    fn save(&mut self, c: &crate::conflict::Conflict) {
        self.d.phase = DPhase::ReturnConflicting { reason: self.reason.clone(), conflict: c.clone() };
        if let Some(x) = self.st.deposits.iter_mut().find(|x| x.outpoint == self.d.outpoint) {
            *x = self.d.clone();
        }
        self.io.save(self.st);
    }
}

/// Our transaction's outputs from `first` on (a return's change, a conflict's output: H4 checked they pay only custody) join the
/// fee pool once it confirms (audit D, H-4).
fn pool_outputs<I: FrogIo>(io: &mut I, txid: [u8; 32], raw: &[u8], first: usize) {
    if let Ok(t) = crate::outbound::parse_tx(raw) {
        for (i, o) in t.outputs.iter().enumerate().skip(first) {
            io.fee_pool_add(Outpoint { txid, vout: i as u32 }, o.value);
        }
    }
}

/// The return of a refused frog: whole, to the taproot script of its transaction's input 0, with custody's delivery builder.
/// The cheap checks first (audit D, H-2 and H-3): the limiter is asked before anything is read. The record is saved with its
/// bytes before they are sent (audit D, L-1).
async fn give_back<I: FrogIo>(io: &mut I, st: &mut FrogState, d: &mut DepositRec, reason: Reason, reserved: &[Outpoint])
    -> Result<String, String> {
    if !reason.returnable() {
        d.phase = DPhase::Held { reason, why: "custody's own coin: never returned".into() };
        return Ok("held: not returnable".into());
    }
    // H3, its own flow (H-2): one return out, against the day's return limit; a return never trips the freeze
    let collection = custodycap::btc_key(&io.wrapper());
    let key = format!("return:{}", d.outpoint.text());
    io.outflow_check(&key, &collection, Flow::Return, 1)?;
    let tx = io.tx_view(d.outpoint.txid).await?;
    let (cust, float, buyer) = (io.custody_script().await?, io.float_script().await?, io.buyer_script().await?);
    let Some(dest) = deposits::return_dest(&tx, &[&cust, &float, &buyer]) else {
        d.phase = DPhase::Held { reason, why: "its sender is not a taproot address that is not ours: left to a person".into() };
        return Ok("held for a person: no taproot sender to return it to".into());
    };
    let (out, spent) = io.output(&d.outpoint).await?;
    if spent {
        d.phase = DPhase::Held { reason, why: "the output is spent already".into() };
        return Ok("held: its output is already spent".into());
    }
    if !out.confirmed || out.script != cust {
        return Err("the explorers do not show it confirmed at custody".into());
    }
    let frog = custody::Frog { output: d.outpoint, value: out.value };
    let rate = io.fee_rate().await?;
    let utxos = io.fee_utxos().await?;
    let mut fee = None;
    for u in custody::fee_candidates(&utxos, &frog.output, reserved, rate) {
        // a candidate that cannot be read is skipped, never the end of the choice (audit R2, N-4)
        let Ok((o, sp)) = io.output(&u.outpoint).await else { continue };
        if sp || !o.confirmed || o.script != cust || o.value != u.value {
            continue;
        }
        let Ok(c) = io.contents(&u.outpoint).await else { continue };
        if custody::fee_eligible(&u, c.as_ref(), &frog.output, reserved) {
            fee = Some(u);
            break;
        }
    }
    let fee = fee.ok_or("no clean custody UTXO can pay the return's fee (the operator registers one with add_custody_fee_input)")?;
    let p = custody::build(&frog, &fee, &dest, &cust, rate)?;
    let mut sigs = Vec::new();
    for i in 0..p.tx.inputs.len() {
        let h = crate::sighash::taproot_key_path(&p, i).map_err(|e| format!("digest: {e:?}"))?;
        sigs.push((i, io.sign_custody(h).await?));
    }
    let raw = psbt::finalize(&p, &vec![None; p.tx.inputs.len()], &sigs)?;
    let txid = p.txid();
    // H4: the finalized bytes return the deposit whole to its sender and pay nobody else, independently of the builder. A
    // refusal is a bug: nothing is saved or sent, and a person looks.
    let pre = vec![
        crate::outbound::Prevout { txid: d.outpoint.internal_txid(), vout: d.outpoint.vout, value: out.value, script: cust.clone() },
        crate::outbound::Prevout { txid: fee.outpoint.internal_txid(), vout: fee.outpoint.vout, value: fee.value, script: cust.clone() },
    ];
    let purpose = crate::outbound::Purpose::DepositReturn { deposit: pre[0].clone(), sender_script: dest.clone() };
    if let Err(e) = crate::outbound::verify_btc(&raw, &pre, &purpose, &io.outbound_policy(&cust)) {
        d.phase = DPhase::Held { reason, why: format!("the outbound check refused the signed return: {e:?}; nothing was saved or sent") };
        return Ok(format!("HELD: outbound check: {e:?}"));
    }
    // L-1: save, then send. The record goes into the state and the state is saved before the send's await
    d.phase = DPhase::Returning { reason, txid, raw: raw.clone(), fee_input: fee.outpoint, unseen_since: None };
    st.return_times.push(io.now());
    if let Some(x) = st.deposits.iter_mut().find(|x| x.outpoint == d.outpoint) {
        *x = d.clone();
    }
    io.save(st);
    io.outflow_record(&key, &collection, Flow::Return, 1);
    let _ = io.btc_send(&raw).await; // the send's answer is not believed
    Ok(format!("return signed, saved and sent in {}", hex::encode(txid)))
}

// ---- the one EVM transaction ----------------------------------------------------------------------------------------------------

pub(crate) async fn send<I: FrogIo>(io: &mut I, st: &mut FrogState, step: FStep, to: [u8; 20], data: Vec<u8>) -> Result<String, String> {
    send_value(io, st, step, to, 0, data).await
}

/// [`send`], carrying `value` wei: a sale's report pays the module or the vault the proceeds it reports.
pub(crate) async fn send_value<I: FrogIo>(io: &mut I, st: &mut FrogState, step: FStep, to: [u8; 20], value: u128, data: Vec<u8>)
    -> Result<String, String> {
    let nonce = io.evm_nonce().await?;
    let base = io.base_fee().await?;
    let est = io.estimate_gas(to, value, &data).await.map_err(|e| format!("{} would not succeed now: {e}", step.text()))?;
    let tx = Tx1559 { chain_id: io.chain_id(), nonce, max_priority_fee: 0, max_fee: base.saturating_mul(FEE_MULTIPLE),
        gas: est + est * GAS_MARGIN_PCT / 100, to, value, data };
    let (y, r, s) = io.sign_evm(tx.signing_hash()).await?;
    let raw = tx.signed(y, &r, &s);
    let hash = evmtx::keccak(&raw);
    st.pending = Some(FrogEvm { step: step.clone(), raw: raw.clone(), hash, nonce });
    io.save(st);
    let _ = io.send_evm(&raw).await;
    Ok(format!("{} signed, saved and sent (nonce {nonce})", step.text()))
}

fn report_call(kind: SinkKind, cost: u128) -> Vec<u8> {
    match kind {
        SinkKind::Curve => crate::curvebuy::record_frog(cost),
        SinkKind::Queue => crate::buyqueue::record_item(cost),
        SinkKind::Floor => crate::floorsink::item_arrived(cost),
    }
}

/// The most urgent EVM act, if any: reports (the unit is already at its target), then mints, then deposits, then triggers.
/// One that cannot go now (its estimate fails) does not hold up the others: the reasons are reported together.
async fn evm_act<I: FrogIo>(io: &mut I, st: &mut FrogState, fl: &mut FloorState, hs: &mut HandinState, ss: &mut SaleState)
    -> Result<Option<String>, String> {
    io.fresh(); // immediately before sending: a new pin
    let mut errs: Vec<String> = Vec::new();
    // conversion review C-06: while a refund watch reads our balance, no reimbursement of ours is triggered (it would read as the
    // refund); an advanced frog's settleAdvance pays nothing and goes on
    let watching = crate::convflow::watching(st);
    // 1. reports owed
    for i in 0..st.frogs.len() {
        if !matches!(st.frogs[i].phase, FPhase::Minted { .. }) {
            continue;
        }
        if watching && st.advanced_for(st.frogs[i].id).is_none() {
            errs.push(format!("frog {}: the report waits (C-06): a Relay refund watch reads our balance", st.frogs[i].id));
            continue;
        }
        match report_one(io, st, fl, i).await {
            Ok(Some(s)) => return Ok(Some(s)),
            Ok(None) => {}
            Err(e) => errs.push(format!("frog {}: {e}", st.frogs[i].id)),
        }
    }
    // 2. mints: a held frog with a target, re-checked now; one unreported unit per sink at a time
    for i in 0..st.frogs.len() {
        let f = st.frogs[i].clone();
        let (FPhase::Held { buy }, Some(t)) = (&f.phase, f.target) else { continue };
        if matches!(t, Target::Sink(_)) && st.frogs.iter().any(|g| g.target == Some(t) && matches!(g.phase, FPhase::Minted { .. })) {
            continue;
        }
        match mint_frog(io, st, fl, i, buy.clone(), t).await {
            Ok(Some(s)) => return Ok(Some(s)),
            Ok(None) => {}
            Err(e) => errs.push(format!("frog {}: {e}", f.id)),
        }
    }
    // 2a. phase B2, the Solana home: the frog listing's floor, posted as the registry's keeper when it names our key
    if crate::frogsol::solana(io) {
        match crate::frogsol::post_act(io, st).await {
            Ok(Some(s)) => return Ok(Some(s)),
            Ok(None) => {}
            Err(e) => errs.push(e),
        }
    }
    // 2b. claims for EVM items that arrived and whose unit is at the module
    for i in 0..fl.wants.len() {
        if !matches!(fl.wants[i].phase, WPhase::Arrived { .. }) {
            continue;
        }
        if watching {
            errs.push(format!("nft want {}: the claim waits (C-06): a Relay refund watch reads our balance", fl.wants[i].id));
            continue;
        }
        let w = fl.wants[i].clone();
        let r = async {
            let v = read_sink(io, SinkKind::Floor, w.module, fl.limit_for(&w.module)).await?;
            let wrapper = addr(io, w.module, crate::floorsink::WRAPPER).await?;
            let bal = crate::abi::uint_at(&word(io, wrapper, &crate::curvebuy::call_addr(crate::curvebuy::BALANCE_OF, &w.module)).await?, 0, 0, 128)?;
            floortrigger::claim_due(&w, bal, &v)
        }.await;
        // H2 and H3: the claim is the canister's word that the item is in custody; it is counted under the caps (the NFT
        // line's settlement put it there) and it is a mint on our word, against the day's limit
        let r = r.and_then(|cost| {
            let WPhase::Arrived { token_id, .. } = &w.phase else { return Err("not arrived".to_string()) };
            let held = crate::nftnet::nft_item_id(w.chain_id, &w.collection, token_id)?;
            let key = custodycap::evm_key(w.chain_id, &w.collection);
            io.with_caps(|p, b| if b.contains(&held) { Ok(()) } else { custodycap::admit(p, b, &held, &key, cost) })
                .map_err(|e| format!("custody cap: {e:?}"))?;
            io.outflow_check(&format!("claim:{}", w.id), &key, Flow::Mint, 1)?;
            Ok((cost, key))
        });
        match r {
            Ok((cost, key)) => match send(io, st, FStep::FloorClaim { want: w.id }, w.module, crate::floorsink::item_arrived(cost)).await {
                Ok(s) => {
                    io.outflow_record(&format!("claim:{}", w.id), &key, Flow::Mint, 1);
                    return Ok(Some(s));
                }
                Err(e) => errs.push(format!("nft want {}: {e}", w.id)),
            },
            Err(e) => errs.push(format!("nft want {}: {e}", w.id)),
        }
    }
    // 2c. a Relay conversion's act: the advance, a judged deposit, or the return (docs/v6/relay-conversion.md)
    if let Some(s) = crate::convflow::evm_act(io, st, fl, &mut errs).await {
        return Ok(Some(s));
    }
    // 3. holder deposits
    for i in 0..st.deposits.len() {
        if !matches!(st.deposits[i].phase, DPhase::Ready { .. }) {
            continue;
        }
        let o = st.deposits[i].outpoint;
        match deposit_one(io, st, i).await {
            Ok(Some(s)) => return Ok(Some(s)),
            Ok(None) => {}
            Err(e) => errs.push(format!("deposit {}: {e}", o.text())),
        }
    }
    // 4. the sinks' own triggers, driven once each
    for i in 0..st.sinks.len() {
        let r = st.sinks[i].clone();
        if r.done() {
            continue;
        }
        if itemsink::drive_settle(&r) && r.kind == SinkKind::Curve {
            match send(io, st, FStep::Settle { sink: r.sink }, r.sink, crate::curvebuy::GRADUATE.to_vec()).await {
                Ok(s) => {
                    st.sinks[i].drove_settle = true;
                    return Ok(Some(s));
                }
                Err(e) => errs.push(e),
            }
        }
        if itemsink::drive_refunds(&r) {
            let data = match r.kind {
                SinkKind::Curve => crate::curvebuy::OPEN_REFUNDS.to_vec(),
                SinkKind::Queue => crate::buyqueue::REFUND_EXPIRED.to_vec(),
                SinkKind::Floor => continue, // a module has no refund trigger (its view never asks for one)
            };
            match send(io, st, FStep::Refunds { sink: r.sink }, r.sink, data).await {
                Ok(s) => {
                    st.sinks[i].drove_refunds_at = r.view.as_ref().map(|v| v.recorded);
                    return Ok(Some(s));
                }
                Err(e) => errs.push(e),
            }
        }
    }
    // 5. hand-ins: the claims of held frogs, then the slashes the boards allow (docs/v6/offer-handins.md §6)
    if let Some(s) = crate::handinflow::evm_act(io, st, hs, &mut errs).await {
        return Ok(Some(s));
    }
    // 6. sales: redeemToRelist, the delivery-by-listing records, the reports with their ETH, re-deposits and reclaims
    //    (docs/v6/canister-floor-followups.md §5)
    if let Some(s) = crate::saleflow::evm_act(io, st, ss, &mut errs).await {
        return Ok(Some(s));
    }
    if errs.is_empty() { Ok(None) } else { Err(errs.join("; ")) }
}

/// The report of one minted frog: `recordItem` / `recordFrog` / `itemArrived` on its sink, or `fulfill` on the desk.
async fn report_one<I: FrogIo>(io: &mut I, st: &mut FrogState, fl: &FloorState, i: usize) -> Result<Option<String>, String> {
    let f = st.frogs[i].clone();
    let FPhase::Minted { buy } = &f.phase else { return Ok(None) };
    match f.target {
        Some(Target::Sink(s)) => {
            let kind = st.sink(&s).map(|r| r.kind).ok_or("a frog minted to a sink we do not know")?;
            let v = read_sink(io, kind, s, fl.limit_for(&s)).await?;
            if v.done {
                st.frogs[i].phase = FPhase::Stranded { buy: buy.clone(), why: format!("the {kind:?} is {} before the report", v.status) };
                return Ok(Some(format!("frog {}: STRANDED: its sink closed before the report", f.id)));
            }
            if !v.open {
                return Err(format!("the {kind:?} is {}: the report waits", v.status));
            }
            // phase B2: `settle_advance(cost)` for a frog its module advanced for, `item_arrived(cost)` otherwise, on the Solana line
            if crate::frogsol::solana(io) {
                let advanced = st.advanced_for(f.id).is_some();
                // audit A5-02: `item_arrived(cost)` pays `cost` lamports to our Solana address, the one an Ethereum buy's Relay watch
                // reads (a rise of half its deposit is its refund): it waits while the Ethereum side holds the Relay leg
                if !advanced && io.eth_relay_busy() {
                    return Err(format!("frog {}: the report (item_arrived, paying our Solana address) waits: the Ethereum side holds the \
                        Relay leg, whose watch reads that balance (A5-02)", f.id));
                }
                let call = if advanced { crate::frogsol::Call::Settle(buy.cost_wei) } else { crate::frogsol::Call::Arrived(buy.cost_wei) };
                return crate::frogsol::send(io, st, FStep::Report { frog: f.id }, call).await.map(Some);
            }
            let data = crate::convflow::report_call(st, f.id, buy.cost_wei).unwrap_or_else(|| report_call(kind, buy.cost_wei));
            send(io, st, FStep::Report { frog: f.id }, s, data).await.map(Some)
        }
        Some(Target::Desk(r)) => {
            let d = st.desk.ok_or("a frog minted to a desk that is not configured")?;
            let req = read_request(io, d, r).await?;
            let expired = flag_uint(io, d, mintdesk::IS_EXPIRED, r).await?;
            match mintdesk::may_fulfill(&req, expired, buy.cost_wei) {
                Ok(()) => send(io, st, FStep::Fulfill { frog: f.id, request: r }, d, mintdesk::fulfill(r, buy.cost_wei)).await.map(Some),
                Err(e) => {
                    st.frogs[i].phase = FPhase::Stray { buy: buy.clone() };
                    st.frogs[i].target = None;
                    Ok(Some(format!("frog {}: request {r} cannot be filled ({e}): its unit is a stray of ours at the desk", f.id)))
                }
            }
        }
        None => Ok(None),
    }
}

/// The mint of one ready deposit, its checks run again now.
async fn deposit_one<I: FrogIo>(io: &mut I, st: &mut FrogState, i: usize) -> Result<Option<String>, String> {
    let o = st.deposits[i].outpoint;
    match check_deposit(io, st, &o).await? {
        Ok((inscription, recipient)) => {
            let proof = st.proof(&inscription).ok_or("the proof went missing")?;
            let data = deposits::wrap_deposit(&inscription, &proof, o.txid, o.vout, &recipient);
            let w = io.wrapper();
            // H3: a unit minted on our word, against the day's limit
            let (key, collection) = (format!("mint:{}", o.text()), custodycap::btc_key(&w));
            io.outflow_check(&key, &collection, Flow::Mint, 1)?;
            let r = send(io, st, FStep::Deposit { outpoint: o }, w, data).await?;
            io.outflow_record(&key, &collection, Flow::Mint, 1);
            st.mint_times.push(io.now());
            Ok(Some(format!("deposit {}: {r}", o.text())))
        }
        Err(p) => {
            st.deposits[i].phase = p;
            Ok(None)
        }
    }
}

async fn flag_uint<I: FrogIo>(io: &mut I, to: [u8; 20], sel: [u8; 4], v: u64) -> Result<bool, String> {
    crate::abi::bool_at(&word(io, to, &crate::curvebuy::call_uint(sel, v as u128)).await?, 0, 0)
}

/// Mint one held frog to its target, if everything still holds now. `Ok(None)`: not now (the frog may have been released).
async fn mint_frog<I: FrogIo>(io: &mut I, st: &mut FrogState, fl: &FloorState, i: usize, buy: Bought, t: Target) -> Result<Option<String>, String> {
    let id = st.frogs[i].id;
    // six deep, now, with the block still the block
    match depth(io, buy.outpoint.txid).await? {
        Some(d) if d >= CONFIRMATIONS => {}
        Some(d) => {
            st.frogs[i].phase = FPhase::Sent { buy };
            return Ok(Some(format!("frog {id}: only {d} deep now (a reorganisation): back to waiting for six")));
        }
        None => {
            st.frogs[i].phase = FPhase::Sent { buy };
            return Ok(Some(format!("frog {id}: no longer in a block (a reorganisation): back to waiting")));
        }
    }
    let w = io.wrapper();
    if minted_frog(io, &buy).await? {
        st.frogs[i].phase = FPhase::Minted { buy };
        return Ok(Some(format!("frog {id}: the wrapper says its outpoint has minted: adopted as minted")));
    }
    let proof = st.proof(&buy.inscription).ok_or("no membership proof kept")?;
    proof_ok(io, &buy.inscription, &proof).await?;
    let to = match t {
        Target::Sink(s) => {
            let kind = st.sink(&s).map(|r| r.kind).ok_or("an unknown sink")?;
            let v = read_sink(io, kind, s, fl.limit_for(&s)).await?;
            let advanced = st.advanced_for(id).is_some();
            if advanced && !v.open {
                return Err(format!("the module is {}: the advanced frog's mint waits", v.status));
            }
            if let Err(e) = if advanced { Ok(()) } else { itemsink::may_mint(&v, buy.cost_wei) } {
                st.frogs[i].target = None;
                if let Some(r) = st.sink_mut(&s) {
                    r.view = Some(v);
                }
                return Ok(Some(format!("frog {id}: not minted ({e}): inventory")));
            }
            s
        }
        Target::Desk(r) => {
            let d = st.desk.ok_or("no desk")?;
            let req = read_request(io, d, r).await?;
            let expired = flag_uint(io, d, mintdesk::IS_EXPIRED, r).await?;
            let ttl = desk_ttl(io, d).await?;
            let clock = io.clock().await?;
            if let Err(e) = mintdesk::may_mint(&req, expired, ttl, &clock, buy.cost_wei) {
                st.frogs[i].target = None;
                return Ok(Some(format!("frog {id}: not minted for request {r} ({e}): inventory")));
            }
            d
        }
    };
    // H3: a unit minted on our word, against the day's limit
    let (key, collection) = (format!("mint:frog:{id}"), crate::frogsol::collection_key(io));
    io.outflow_check(&key, &collection, Flow::Mint, 1)?;
    // phase B2: `mint_item` with the inscription id and its proof, to the module's own wX account, on the Solana line
    let s = if crate::frogsol::solana(io) {
        let _ = to;
        crate::frogsol::send(io, st, FStep::Mint { frog: id }, crate::frogsol::Call::Mint { inscription: buy.inscription.clone(), proof,
            outpoint: buy.outpoint }).await?
    } else {
        let data = deposits::wrap_deposit(&buy.inscription, &proof, buy.outpoint.txid, buy.outpoint.vout, &to);
        send(io, st, FStep::Mint { frog: id }, w, data).await?
    };
    io.outflow_record(&key, &collection, Flow::Mint, 1);
    Ok(Some(format!("frog {id}: {s}")))
}

// ---- settling our transaction -------------------------------------------------------------------------------------------------------

async fn resolve<I: FrogIo>(io: &mut I, st: &mut FrogState, fl: &mut FloorState, hs: &mut HandinState, ss: &mut SaleState, p: FrogEvm)
    -> Result<String, String> {
    let _ = io.send_evm(&p.raw).await; // the same bytes again; the provider's answer is not believed
    match io.evm_seen(p.hash).await {
        EvmView::Mined { success } => {
            st.pending = None;
            let s = match p.step {
                FStep::Claim { .. } | FStep::Slash { .. } => {
                    io.fresh();
                    crate::handinflow::mined(io, hs, &p.step, success).await
                }
                FStep::Relist { .. } | FStep::SaleRecord { .. } | FStep::SaleReport { .. } | FStep::SaleDeposit { .. } | FStep::SaleReclaim { .. } => {
                    io.fresh();
                    crate::saleflow::mined(io, ss, &p.step, success).await
                }
                FStep::Advance { .. } | FStep::Convert { .. } | FStep::GiveBack { .. } | FStep::LateCredit { .. } => {
                    io.fresh();
                    crate::convflow::mined(io, st, &p.step, success).await
                }
                _ => mined(io, st, fl, &p.step, success).await,
            };
            Ok(format!("{} mined ({}): {s}", p.step.text(), if success { "success" } else { "REVERTED" }))
        }
        EvmView::NotYet => {
            if io.evm_nonce().await? > p.nonce {
                Err(format!("nonce {} is used but {} is not visible yet: waiting rather than guessing", p.nonce, p.step.text()))
            } else {
                Ok(format!("{} sent again (nonce {}), not seen yet", p.step.text(), p.nonce))
            }
        }
        EvmView::Pending => Ok(format!("{} pending (nonce {})", p.step.text(), p.nonce)),
        EvmView::Unsure(e) => Err(e),
    }
}

/// Whether the wrapper minted this frog from its outpoint: `depositMinted(outpointKey)` on the EVM home, the `HeldMarker` of its leaf
/// naming an item from this outpoint on Solana (phase B2).
async fn minted_frog<I: FrogIo>(io: &mut I, buy: &Bought) -> Result<bool, String> {
    if crate::frogsol::solana(io) {
        return crate::frogsol::minted(io, buy).await;
    }
    let w = io.wrapper();
    flag(io, w, deposits::DEPOSIT_MINTED, deposits::outpoint_key(buy.outpoint.txid, buy.outpoint.vout)).await
}

pub(crate) async fn mined<I: FrogIo>(io: &mut I, st: &mut FrogState, fl: &mut FloorState, step: &FStep, success: bool) -> String {
    io.fresh();
    match step.clone() {
        FStep::Mint { frog } => {
            let Some(f) = st.frogs.iter().find(|f| f.id == frog).cloned() else { return "an unknown frog".into() };
            let Some(buy) = f.bought().cloned() else { return "a frog with nothing bought".into() };
            let minted = success || minted_frog(io, &buy).await.unwrap_or(false);
            if minted {
                set_phase(st, frog, FPhase::Minted { buy });
                "minted to its target; the report is next".into()
            } else {
                set_phase(st, frog, FPhase::Held { buy });
                // conversion review C-01: a frog its module advanced for stays that module's; any other is inventory again
                if let Some(m) = st.advanced_for(frog).map(|c| c.module) {
                    if let Some(x) = st.frog(frog) {
                        x.target = Some(Target::Sink(m));
                    }
                    return "not minted: the advanced frog stays its module's; the mint is tried again".into();
                }
                if let Some(x) = st.frog(frog) {
                    x.target = None;
                }
                "not minted: the frog is inventory again".into()
            }
        }
        FStep::Report { frog } => {
            let Some(buy) = st.frogs.iter().find(|f| f.id == frog).and_then(|f| f.bought().cloned()) else { return "an unknown frog".into() };
            if success {
                let advanced = st.advanced_for(frog).is_some();
                if advanced {
                    crate::convflow::settled(st, frog, buy.spend_sats, buy.cost_wei);
                }
                set_phase(st, frog, FPhase::Settled { buy });
                if advanced { "settled against its advance: the unit is item-backed".into() } else { "reported and reimbursed".into() }
            } else {
                "the report reverted: read again next tick (an estimate guards the next attempt)".into()
            }
        }
        FStep::Fulfill { frog, request } => {
            let Some(buy) = st.frogs.iter().find(|f| f.id == frog).and_then(|f| f.bought().cloned()) else { return "an unknown frog".into() };
            let filled = success || match st.desk {
                Some(d) => read_request(io, d, request).await.is_ok_and(|r| r.status != DStatus::Open && r.cost == buy.cost_wei),
                None => false,
            };
            if filled {
                set_phase(st, frog, FPhase::Settled { buy });
                if let Some(r) = st.requests.iter_mut().find(|r| r.id == request) {
                    r.done = true;
                }
                "filled and paid".into()
            } else {
                set_phase(st, frog, FPhase::Stray { buy });
                if let Some(x) = st.frog(frog) {
                    x.target = None;
                }
                "not filled: its unit is a stray of ours at the desk".into()
            }
        }
        FStep::Refunds { .. } | FStep::Settle { .. } => "the contract's state is read next tick".into(),
        FStep::Post => if success { "the frog listing's floor is posted".into() } else { "the floor post failed: the reading is dropped".into() },
        FStep::Claim { .. } | FStep::Slash { .. } => "a hand-in step: handinflow settles it".into(),
        FStep::Relist { .. } | FStep::SaleRecord { .. } | FStep::SaleReport { .. } | FStep::SaleDeposit { .. } | FStep::SaleReclaim { .. } =>
            "a sale step: saleflow settles it".into(),
        FStep::Advance { .. } | FStep::Convert { .. } | FStep::GiveBack { .. } | FStep::LateCredit { .. } =>
            "a conversion step: convflow settles it".into(),
        FStep::FloorClaim { want } => {
            let Some(w) = fl.want_mut(want) else { return "an unknown want".into() };
            match (&w.phase, success) {
                (WPhase::Arrived { token_id, tx_hash, cost_wei }, true) => {
                    w.phase = WPhase::Claimed { token_id: token_id.clone(), tx_hash: tx_hash.clone(), cost_wei: *cost_wei };
                    "claimed: the module paid the item's cost".into()
                }
                _ => "the claim reverted: read again next tick (an estimate guards the next attempt)".into(),
            }
        }
        FStep::Deposit { outpoint } => {
            let Some(d) = st.deposits.iter_mut().find(|d| d.outpoint == outpoint) else { return "an unknown deposit".into() };
            let ready = d.phase.clone();
            let minted = success || {
                let w = io.wrapper();
                flag(io, w, deposits::DEPOSIT_MINTED, deposits::outpoint_key(outpoint.txid, outpoint.vout)).await.unwrap_or(false)
            };
            let Some(d) = st.deposits.iter_mut().find(|d| d.outpoint == outpoint) else { return "an unknown deposit".into() };
            match (minted, ready) {
                (true, DPhase::Ready { inscription, recipient }) => {
                    d.phase = DPhase::Minted { inscription, recipient };
                    "minted to its recipient".into()
                }
                (true, _) => "minted".into(),
                (false, _) => {
                    d.phase = DPhase::Pending { why: "wrapDeposit reverted: checked again".into() };
                    "wrapDeposit reverted: the checks run again".into()
                }
            }
        }
    }
}
