//! The desk's public surface (§6.1, §10.1): opening an intent, its status, the quotes, the operator's switches, and the canister's own
//! tick. The logic is [`open`] and `run::tick`, generic over `DeskIo`; the IC wrappers at the bottom only supply the world.
use candid::{CandidType, Principal};
use serde::{Deserialize, Serialize};

use super::intent::*;
use super::quote::{self, BurnQuote, MintQuote};
use super::state::{with, with_mut};
use crate::books::{Destination, ListingKind, Params};
use crate::chainlink::driver::{self, DeskIo};
use crate::chainlink::ledger::{Effects, Incident, Phase};
use crate::chainlink::recon::Report;
use crate::solkey::KeyPath;
use crate::soltx::{associated_token_address, b58_encode, pubkey, Pubkey};

/// What the site asks for.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum OpenKind {
    /// Mint wX of `coll` to `recipient` (base58), at least `min_out` base units.
    Mint { coll: u16, recipient: String, min_out: Option<u64> },
    /// Burn wX of `coll`: SOL to `recipient`, at least `min_out` lamports net of the fee; a refusal returns the wX to `refund_to`
    /// (default `recipient`).
    Burn { coll: u16, recipient: String, refund_to: Option<String>, min_out: Option<u64> },
    /// Redeem one wX of `coll` for an item to `destination` (`0x` + 40 hex for Ethereum, a taproot address for Bitcoin, a base58
    /// wallet for a Solana listing, or `burn`); `owner` (base58) owns the ticket and gets back an unused deposit.
    RedeemNft { coll: u16, destination: String, owner: String },
    /// Redeem a coin (`coin` = its mint, base58) of collection `coll` for its backing wX to `recipient`.
    RedeemCoin { coin: String, coll: u16, recipient: String, refund_to: Option<String> },
}

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum Quote {
    Mint(MintQuote),
    Burn(BurnQuote),
}

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Opened {
    pub id: u64,
    /// Where the user sends SOL (a mint) or whose token account receives the wX / coins (the others).
    pub address: String,
    /// `address`'s associated token account of the deposited mint (a burn, a redemption): the one the transfer goes to.
    pub token_account: Option<String>,
    pub expires_at: u64,
    pub min_deposit: u64,
    pub fee: u64,
    /// At the minimum deposit (a mint) or the minimum wX (a burn); none while the floor is stale or for a redemption.
    pub quote: Option<Quote>,
}

fn parse_key(s: &str, what: &str) -> Result<[u8; 32], String> {
    pubkey(s).map_err(|e| format!("{what}: {e}"))
}

/// `0x…` (20 bytes) → Ethereum; `burn` → the burn destination; a taproot address (`bc1p…`, `tb1p…`, `bcrt1p…`); else a base58 Solana wallet.
pub fn parse_destination(s: &str) -> Result<Destination, String> {
    let s = s.trim();
    if s.eq_ignore_ascii_case("burn") {
        return Ok(Destination::Burn);
    }
    if let Some(h) = s.strip_prefix("0x") {
        let b = hex::decode(h).map_err(|e| format!("destination: {e}"))?;
        return Ok(Destination::Eth(b.try_into().map_err(|_| "destination: an Ethereum address is 20 bytes".to_string())?));
    }
    let l = s.to_ascii_lowercase();
    if l.starts_with("bc1p") || l.starts_with("tb1p") || l.starts_with("bcrt1p") {
        return Ok(Destination::Taproot(s.to_string()));
    }
    Ok(Destination::Sol(parse_key(s, "destination")?))
}

fn anonymous(caller: &[u8]) -> bool {
    caller == Principal::anonymous().as_slice()
}

/// Anonymous callers share this many openings an hour (a per-principal limit means nothing for them; the per-recipient and total limits
/// still bind).
pub const ANON_PER_HOUR: usize = 120;

/// Open an intent: validate, reserve its number, derive its address, answer with the deposit address, a quote and the expiry.
pub async fn open<I: DeskIo>(io: &mut I, caller: Vec<u8>, args: OpenKind, now: u64) -> Result<Opened, String> {
    if with(|s| s.desk_paused) {
        return Err("the desk is paused".into());
    }
    let (coll, kind) = match args {
        OpenKind::Mint { coll, recipient, min_out } => (coll, Kind::Mint { coll, recipient: parse_key(&recipient, "recipient")?, min_units: min_out.unwrap_or(0) }),
        OpenKind::Burn { coll, recipient, refund_to, min_out } => {
            let r = parse_key(&recipient, "recipient")?;
            let f = match refund_to {
                Some(f) => parse_key(&f, "refund_to")?,
                None => r,
            };
            (coll, Kind::Burn { coll, recipient: r, refund_to: f, min_lamports: min_out.unwrap_or(0) })
        }
        OpenKind::RedeemNft { coll, destination, owner } => {
            let d = parse_destination(&destination)?;
            let kind = with(|s| s.books.get(coll).map(|c| c.kind.clone())).map_err(|e| format!("{e}"))?;
            crate::books::Ledger::check_destination(&kind, &d).map_err(|e| format!("destination: {e}"))?;
            (coll, Kind::RedeemNft { coll, destination: d, owner: parse_key(&owner, "owner")? })
        }
        OpenKind::RedeemCoin { coin, coll, recipient, refund_to } => {
            if !with(|s| s.coin_redeem) {
                return Err("coin redemption is not enabled yet".into());
            }
            let r = parse_key(&recipient, "recipient")?;
            let f = match refund_to {
                Some(f) => parse_key(&f, "refund_to")?,
                None => r,
            };
            (coll, Kind::RedeemCoin { coin: parse_key(&coin, "coin")?, coll, recipient: r, refund_to: f })
        }
    };
    // the collection, and the limits: all checked and the number reserved BEFORE the first await
    let id = with_mut(|s| {
        let c = s.books.get(coll).map_err(|e| format!("{e}"))?;
        if s.link.halted.contains_key(&coll) {
            return Err(format!("collection {coll} is stopped by an incident"));
        }
        if matches!(kind, Kind::Mint { .. }) && c.paused {
            return Err(format!("collection {coll} is paused: no mint"));
        }
        // audit B1-05: an opening is free (and may be anonymous), so intents that never saw a deposit have a cap of their own and
        // never take one of the MAX_OPEN slots of intents with a deposit
        let live = |i: &&Intent| matches!(i.state, State::Open | State::Funded | State::Executing) && !i.expired(now);
        let (unfunded, funded): (Vec<&Intent>, Vec<&Intent>) = s.intents.values().filter(live).partition(|i| i.unfunded());
        if unfunded.len() >= MAX_UNFUNDED_OPEN {
            return Err(format!("{MAX_UNFUNDED_OPEN} intents are waiting for a deposit: try again later"));
        }
        if funded.len() >= MAX_OPEN {
            return Err(format!("{MAX_OPEN} intents are open: try again later"));
        }
        // only the last hour counts (the tick prunes the log, but an open may come before the next tick)
        let mine = s.opens.iter().filter(|o| o.caller == caller && o.at + 3_600 > now).count();
        let cap = if anonymous(&caller) { ANON_PER_HOUR } else { MAX_PER_CALLER_HOUR };
        if mine >= cap {
            return Err(format!("{cap} intents an hour per caller: try again later"));
        }
        // audit B1-09: the per-recipient bound counts this caller's own intents for the recipient (a stranger's free openings naming a
        // wallet never lock that wallet out)
        let to = kind.refund_to();
        let same = s.intents.values().filter(|i| i.caller == caller && i.kind.refund_to() == to && matches!(i.state, State::Open | State::Funded | State::Executing) && !i.expired(now)).count();
        if same >= MAX_PER_RECIPIENT {
            return Err(format!("{MAX_PER_RECIPIENT} intents are already open for this recipient"));
        }
        let id = s.next_intent;
        s.next_intent += 1;
        s.opens.push(super::state::OpenLog { caller: caller.clone(), at: now });
        Ok(id)
    })?;
    let address = io.key_of(&KeyPath::Intent(id)).await?;
    let mint = match &kind {
        Kind::Mint { .. } => None,
        Kind::RedeemCoin { coin, .. } => Some(*coin),
        k => with(|s| s.books.get(k.coll()).ok().map(|c| c.wx_mint)),
    };
    let token_account = mint.map(|m| associated_token_address(&address, &m));
    let intent = Intent::new(id, kind.clone(), address, token_account, caller, now);
    let expires_at = intent.expires_at;
    with_mut(|s| s.intents.insert(id, intent));
    let now_i = now as i64;
    let q = with(|s| match &kind {
        Kind::Mint { .. } => quote::quote_mint(&s.books, coll, MIN_MINT_LAMPORTS, quote::ATA_RENT_LAMPORTS, now_i).ok().map(Quote::Mint),
        Kind::Burn { .. } => quote::quote_burn(&s.books, coll, MIN_BURN_UNITS, now_i).ok().map(Quote::Burn),
        _ => None,
    });
    Ok(Opened { id, address: b58_encode(&address), token_account: token_account.map(|t| b58_encode(&t)), expires_at, min_deposit: kind.min_deposit(),
        fee: FEE_LAMPORTS, quote: q })
}

// ---- views ----------------------------------------------------------------------------------------------------------------------

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct TxView {
    pub link: u64,
    pub op: String,
    pub memo: String,
    pub signature: String,
    pub result: String,
    pub at: u64,
}

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct IntentView {
    pub id: u64,
    pub kind: String,
    pub coll: u16,
    pub state: State,
    pub address: String,
    pub token_account: Option<String>,
    pub recipient: String,
    pub refund_to: String,
    pub deposit: u64,
    pub min_deposit: u64,
    pub created_at: u64,
    pub expires_at: u64,
    pub funded_at: Option<u64>,
    pub minted_units: u64,
    pub paid_lamports: u64,
    pub ticket: Option<u64>,
    pub refunding: Option<String>,
    /// The transaction out now, as its memo.
    pub in_flight: Option<String>,
    pub note: String,
    pub attention: bool,
    pub txs: Vec<TxView>,
}

pub fn view_of(i: &Intent) -> IntentView {
    let recipient = match &i.kind {
        Kind::Mint { recipient, .. } | Kind::Burn { recipient, .. } | Kind::RedeemCoin { recipient, .. } => *recipient,
        Kind::RedeemNft { owner, .. } => *owner,
    };
    IntentView {
        id: i.id, kind: i.kind.tag().into(), coll: i.kind.coll(), state: i.state, address: b58_encode(&i.address),
        token_account: i.token_account.map(|t| b58_encode(&t)), recipient: b58_encode(&recipient), refund_to: b58_encode(&i.kind.refund_to()),
        deposit: i.deposit, min_deposit: i.kind.min_deposit(), created_at: i.created_at, expires_at: i.expires_at, funded_at: i.funded_at,
        minted_units: i.minted_units, paid_lamports: i.paid_lamports, ticket: i.ticket,
        refunding: i.must_refund.clone().or_else(|| i.refund_only.then(|| "a deposit that cannot be acted on".to_string())),
        in_flight: i.op.and_then(|o| with(|s| s.link.get(o.link).map(|l| l.memo.clone()))), note: i.note.clone(), attention: i.attention,
        txs: i.history.iter().map(|t| TxView { link: t.link, op: t.op.clone(), memo: t.memo.clone(), signature: t.signature.clone(), result: t.result.clone(), at: t.at }).collect(),
    }
}

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct LinkView {
    pub id: u64,
    pub key: String,
    pub kind: String,
    pub coll: Option<u16>,
    pub intent: Option<u64>,
    pub memo: String,
    pub phase: Phase,
    pub signature: String,
    pub reason: String,
    pub effects: Effects,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct DeskStatus {
    pub paused: bool,
    pub collections: u32,
    pub open: u32,
    pub funded: u32,
    pub executing: u32,
    pub needing_attention: u32,
    pub open_links: u32,
    pub halted: Vec<(u16, String)>,
    pub incidents: Vec<Incident>,
    pub last_reconcile: u64,
    pub report: Option<Report>,
    pub coin_redeem: bool,
}

pub fn status() -> DeskStatus {
    with(|s| DeskStatus {
        paused: s.desk_paused,
        collections: s.books.collections.len() as u32,
        open: s.intents.values().filter(|i| i.state == State::Open).count() as u32,
        funded: s.intents.values().filter(|i| i.state == State::Funded).count() as u32,
        executing: s.intents.values().filter(|i| i.state == State::Executing).count() as u32,
        needing_attention: s.intents.values().filter(|i| i.attention).count() as u32,
        open_links: s.link.links.values().filter(|l| l.phase.open()).count() as u32,
        halted: s.link.halted.iter().map(|(c, i)| (*c, i.what.clone())).collect(),
        incidents: s.link.incidents.clone(),
        last_reconcile: s.last_reconcile,
        report: s.link.last_report.clone(),
        coin_redeem: s.coin_redeem,
    })
}

pub fn log(from: u64, n: u64) -> Vec<LinkView> {
    with(|s| {
        s.link.links.range(from..).take(n.min(200) as usize).map(|(_, l)| LinkView { id: l.id, key: l.key.clone(), kind: l.kind.tag().into(), coll: l.coll,
            intent: l.intent, memo: l.memo.clone(), phase: l.phase, signature: l.signature.clone(), reason: l.reason.clone(), effects: l.effects,
            created_at: l.created_at, updated_at: l.updated_at }).collect()
    })
}

/// The public methods a stranger may call as ingress (`inspect_message`).
pub fn is_public(method: &str) -> bool {
    method == "open_intent"
}

// ---- the IC world ---------------------------------------------------------------------------------------------------------------

fn now_secs() -> u64 {
    ic_cdk::api::time() / 1_000_000_000
}

pub(crate) struct DeskNet {
    sol: Option<crate::solhome::SolNet>,
}

crate::sol_io_via!(DeskNet, sol);

impl DeskIo for DeskNet {
    async fn key_of(&mut self, p: &KeyPath) -> Result<Pubkey, String> {
        crate::solkey::derive_key(p).await
    }
}

fn net() -> Result<DeskNet, String> {
    Ok(DeskNet { sol: Some(crate::solhome::SolNet::of_install()?.ok_or("the Solana home is not configured")?) })
}

/// The desk's pass for the canister's timer and the runner's tick (the tick's flag held by the caller). Nothing without a Solana home.
pub(crate) async fn tick() -> Vec<String> {
    if crate::canister::cfg().home.is_none() {
        return vec![];
    }
    match net() {
        Ok(mut n) => super::run::tick(&mut n, now_secs()).await,
        Err(e) => vec![format!("desk: waits: {e}")],
    }
}

/// Open an intent. Anyone (anonymous callers too); rate-limited per caller and per recipient, at most 200 awaiting a deposit and 300
/// with one (audit B1-05).
#[ic_cdk::update]
pub async fn open_intent(args: OpenKind) -> Result<Opened, String> {
    crate::canister::not_paused()?;
    let caller = ic_cdk::api::msg_caller().as_slice().to_vec();
    let mut n = net()?;
    open(&mut n, caller, args, now_secs()).await
}

/// An intent's state, its deposit, its transactions.
#[ic_cdk::query]
pub fn intent_status(id: u64) -> Option<IntentView> {
    with(|s| s.intents.get(&id).map(view_of))
}

/// What depositing `lamports` would mint of collection `coll` now (the recipient's token account assumed to need creating).
#[ic_cdk::query]
pub fn quote_mint(coll: u16, lamports: u64) -> Result<MintQuote, String> {
    with(|s| quote::quote_mint(&s.books, coll, lamports, quote::ATA_RENT_LAMPORTS, (now_secs()) as i64)).map_err(|e| format!("{e}"))
}

/// What burning `units` wX base units of collection `coll` would pay now.
#[ic_cdk::query]
pub fn quote_burn(coll: u16, units: u64) -> Result<BurnQuote, String> {
    with(|s| quote::quote_burn(&s.books, coll, units, now_secs() as i64)).map_err(|e| format!("{e}"))
}

/// The desk, the pending-ledger and the last reconciliation.
#[ic_cdk::query]
pub fn desk_status() -> DeskStatus {
    status()
}

/// The pending-ledger from link `from`: every operation with its memo, its phase and its signature (anyone can replay it against Solana).
#[ic_cdk::query]
pub fn ledger_log(from: u64, n: u64) -> Vec<LinkView> {
    log(from, n)
}

/// The operator (or the guardian, through the timelock) pauses a collection's public mint at once, or asks for its resume, which the
/// cranks apply a day later (§10.4; audit B3-01: no method lifts a pause at once). Burns, redemptions and refunds go on.
#[ic_cdk::update]
pub fn desk_pause_collection(coll: u16, paused: bool) -> Result<String, String> {
    crate::canister::only_operator();
    crate::canister::not_paused()?;
    crate::cranks::collection_pause(coll, paused, ic_cdk::api::time() / 1_000_000_000)
}

/// The operator stops (or resumes) the whole desk: no new intent, no new mint, burn or redemption; open deposits are still returned.
#[ic_cdk::update]
pub fn desk_set_paused(paused: bool) -> Result<String, String> {
    crate::canister::only_operator();
    crate::canister::not_paused()?;
    with_mut(|s| s.desk_paused = paused);
    Ok(format!("the desk is {}", if paused { "paused" } else { "running" }))
}

/// The operator, having looked: collection `coll` runs again after an incident (its public mint stays paused until lifted).
#[ic_cdk::update]
pub fn desk_clear_halt(coll: u16) -> Result<String, String> {
    crate::canister::only_operator();
    crate::canister::not_paused()?;
    let cleared = with_mut(|s| s.link.clear_halt(coll));
    Ok(if cleared { format!("collection {coll}: the incident is cleared") } else { format!("collection {coll} was not halted") })
}

/// The operator turns on coin redemption (once the coin desk's executor is wired).
#[ic_cdk::update]
pub fn desk_set_coin_redeem(on: bool) -> Result<String, String> {
    crate::canister::only_operator();
    crate::canister::not_paused()?;
    with_mut(|s| s.coin_redeem = on);
    Ok(format!("coin redemption intents are {}", if on { "accepted" } else { "refused" }))
}

/// The operator registers a collection in the books (parameters fixed for good). `wx_mint` is the base58 wX mint.
#[ic_cdk::update]
pub fn desk_add_collection(id: u16, wx_mint: String, kind: ListingKind, params: Params) -> Result<String, String> {
    crate::canister::only_operator();
    crate::canister::not_paused()?;
    let m = parse_key(&wx_mint, "wx_mint")?;
    with_mut(|s| s.books.add_collection(id, kind, m, params)).map_err(|e| format!("{e}"))?;
    Ok(format!("collection {id} registered"))
}

/// The operator runs a reconciliation now (settling first): the books against the chain at finalized, and the memo scan.
#[ic_cdk::update]
pub async fn desk_reconcile() -> Result<Report, String> {
    crate::canister::only_operator();
    crate::canister::not_paused()?;
    let _held = crate::canister::hold_tick()?;
    let mut n = net()?;
    let now = now_secs();
    driver::settle(&mut n, now).await;
    Ok(driver::reconcile(&mut n, now).await)
}

/// Audit B1-11: the operator decides a STUCK link (signed, absent from its line, unknown to the chain for `ORPHAN_SECS`): the chain is
/// asked first by the signature and its finalized verdict wins; unknown to the chain, only `landed = false` is taken (undone: the
/// books restored), on the operator's word that the transaction can never land.
#[ic_cdk::update]
pub async fn desk_resolve_link(id: u64, landed: bool) -> Result<String, String> {
    crate::canister::only_operator();
    crate::canister::not_paused()?;
    let _held = crate::canister::hold_tick()?;
    let mut n = net()?;
    driver::resolve_stuck(&mut n, id, landed, now_secs()).await
}

/// The operator books the lamports found at `M[coll]` above the books (an unsolicited transfer, a gift) as surplus.
#[ic_cdk::update]
pub async fn desk_sweep_unbooked(coll: u16) -> Result<u64, String> {
    crate::canister::only_operator();
    crate::canister::not_paused()?;
    let _held = crate::canister::hold_tick()?;
    let mut n = net()?;
    driver::sweep_unbooked(&mut n, coll, now_secs()).await
}

// ---- the books of one collection (§10.1 `books(coll)` + `ledger(coll)`): what the reconciler and the site read -------------------------

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ItemView {
    pub item_id: u32,
    pub key: String,
    pub source_tx: String,
    pub vout: u32,
    pub deposit: bool,
    pub inscription_id: String,
}

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct TicketSummary {
    pub id: u64,
    pub stage: String,
    pub item_id: u32,
    pub deliver_by: i64,
    pub destination: String,
    pub undeliverable: bool,
}

/// One collection's books, as the proof-of-reserves reader and the site read them (lamports and wX base units).
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct BooksView {
    pub coll: u16,
    pub kind: ListingKind,
    pub wx_mint: String,
    pub paused: bool,
    pub poster_paused: bool,
    // the module
    pub accepted: u64,
    pub target: u64,
    pub posted_at: i64,
    pub fresh: bool,
    /// `escaped()`: stale, with no good post for long: burns reopen at the last good price.
    pub escaped: bool,
    pub last_good_at: i64,
    pub backing: u64,
    pub reserve: u64,
    pub surplus: u64,
    pub revenue: u64,
    /// `backing + reserve + surplus + revenue`: what `M[i]` must hold above its rent.
    pub owed: u64,
    pub reserve_units: u64,
    pub transit_units: u64,
    pub advanced: u64,
    pub mint_price: Option<u64>,
    pub burn_pays_one: Option<u64>,
    pub volume_7d: u64,
    pub items_arrived: u64,
    // the ledger
    pub held: u64,
    pub open_tickets: u64,
    pub burned: u64,
    pub ticket_count: u64,
    pub item_count: u32,
    pub sol_backed: u64,
    pub module_units: u64,
    /// `(held − open_tickets)·10⁹ + sol_backed − module_units`: the wX supply the chain must show (`None`: the books are inconsistent).
    pub expected_supply: Option<u64>,
    pub items: Vec<ItemView>,
    pub tickets_open: Vec<TicketSummary>,
    /// Whether the collection is stopped by an incident.
    pub halted: Option<String>,
    pub raffle_pot: u64,
    pub raffle_prize: u64,
}

pub fn books_of(coll: u16, now: i64) -> Option<BooksView> {
    with(|s| {
        let c = s.books.get(coll).ok()?;
        let (m, l) = (&c.module, &c.ledger);
        let b58 = |k: &[u8; 32]| crate::soltx::b58_encode(k);
        let hexs = |k: &[u8; 32]| hex::encode(k);
        // W8c: a Solana NFT's custody proof is its mint (base58) held by the listing's custodian; the others' keys stay hex
        let sol = matches!(c.kind, ListingKind::SolNft { .. });
        let key_of = |k: &[u8; 32]| if sol { b58(k) } else { hexs(k) };
        Some(BooksView {
            coll,
            kind: c.kind.clone(),
            wx_mint: b58(&c.wx_mint),
            paused: c.paused,
            poster_paused: c.poster_paused,
            accepted: m.accepted,
            target: m.target,
            posted_at: m.posted_at,
            fresh: m.fresh(now),
            escaped: m.escaped(now),
            last_good_at: m.last_good_at,
            backing: m.backing,
            reserve: m.reserve,
            surplus: m.surplus,
            revenue: m.revenue,
            owed: u64::try_from(m.books()).unwrap_or(u64::MAX),
            reserve_units: m.reserve_units,
            transit_units: m.transit_units,
            advanced: m.advanced,
            mint_price: m.mint_price(now, l.sol_backed).ok(),
            burn_pays_one: s.books.quote_burn(coll, crate::books::ONE, now).ok(),
            volume_7d: m.volume_7d(now),
            items_arrived: m.items_arrived,
            held: l.held,
            open_tickets: l.open_tickets,
            burned: l.burned,
            ticket_count: l.ticket_count,
            item_count: l.item_count,
            sol_backed: l.sol_backed,
            module_units: l.module_units,
            expected_supply: l.expected_supply(),
            items: l.items.values().map(|x| ItemView { item_id: x.item_id, key: key_of(&x.key), source_tx: hexs(&x.source_tx), vout: x.vout, deposit: x.deposit,
                inscription_id: x.inscription_id.clone() }).collect(),
            tickets_open: l.tickets.values().filter(|t| !matches!(t.stage, crate::books::Stage::Delivered | crate::books::Stage::Refunded)).map(|t| TicketSummary {
                id: t.id, stage: format!("{:?}", t.stage), item_id: t.item_id, deliver_by: t.deliver_by, undeliverable: t.undeliverable,
                destination: match &t.destination {
                    crate::books::Destination::Eth(a) => format!("0x{}", hex::encode(a)),
                    crate::books::Destination::Taproot(a) => a.clone(),
                    crate::books::Destination::Burn => "burn".into(),
                    crate::books::Destination::Sol(k) => b58(k),
                } }).collect(),
            halted: s.link.halted.get(&coll).cloned().map(|h| format!("{h:?}")),
            raffle_pot: c.raffle.as_ref().map(|r| r.pot).unwrap_or(0),
            raffle_prize: c.raffle.as_ref().map(|r| r.prize).unwrap_or(0),
        })
    })
}

/// One collection's books (§10.1): the module's numbers, the ledger, every held item with its custody proof, the open tickets.
#[ic_cdk::query]
pub fn books_view(coll: u16) -> Option<crate::desk::api::BooksView> {
    books_of(coll, now_secs() as i64)
}

/// Every listed collection's id and wX mint.
#[ic_cdk::query]
pub fn books_collections() -> Vec<(u16, String)> {
    with(|s| s.books.collections.iter().map(|(id, c)| (*id, crate::soltx::b58_encode(&c.wx_mint))).collect())
}
