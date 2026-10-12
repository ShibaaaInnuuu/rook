//! The frog side's home on SOLANA (docs/v9/solana-migration.md §2.3, §2.4, §4; phase B2; canister-only since decision D-l,
//! docs/v9/canister-only-solana.md §8): `frogflow`, `convflow` and the custodian's reads and steps against the books' one Inscriptions
//! listing ([`books`]). With `home = Solana` the frog collection is that listing; every read is a book read, every step a book update
//! final at once, except what moves SOL: the advance (`M → K_main`, signed by the collection's key), the way back and a late credit
//! (`K_main → M`), and Relay's `deposit_native` (`relaysol`, SOL → BTC), signed on the money line (`solhome`), simulated before they
//! are signed. The Bitcoin side (offers, judges, custody, the float) is unchanged. With no home nothing here runs.
//!
//! Units: on this home every home-chain amount of the frog records (`FPhase::Wanted.cap_wei`, `Bought.cost_wei`, `ConvRec.advanced`,
//! a purchase limit) is in LAMPORTS, and the frog flow's reference rate (`FrogIo::rate`) is satoshis per 10^18 lamports
//! (`rates::sats_per_home_unit`), so `frogbuy`'s arithmetic reads the same in both homes. Custody caps and the outbound limiter stay in
//! wei: a lamport amount is converted at this tick's SOL/ETH rate before it is counted ([`value_wei`]).
//!
//! What maps to what (the EVM home's adapters): `curvebuy` (a graduating `FrogCurve` buying frogs from its raise) has no Solana form —
//! DBC owns the curve and the coins module backs the raise's share as SOL-backed units of the floor module, which the module's buy
//! signal (`net_units`) then buys like any other: so a coin's frogs come through [`read_sink`]; `floorsink` / `floortrigger` read the
//! module (`solhome::books_sink_view`); `itemArrived` / `settleAdvance` / `returnAdvance` / `fundReserve` / `advance` are the books'
//! `item_arrived` / `settle_advance` / `return_advance` / `fund_reserve` / `advance`; `wrapDeposit` is `mint_item` with the inscription
//! id and its Merkle proof against the listing's own root; the floor post is `post_floor` in the books, the canister the poster
//! ([`post_act`]).
pub mod books;
pub mod launch;
pub mod multi;

use crate::custody::Outpoint;
use crate::frogbuy::Bought;
use crate::frogflow::{FStep, FrogIo, FrogState};
use crate::itemsink::{SinkKind, SinkView};
use crate::relayconv as rc;
use crate::relaysol;
use crate::solhome::{self as sh, LineOwner, LineStep, SolDepositRec, SolIo};
use crate::soltx::{self, Pubkey};

/// Relay's SOL → BTC capacity per order (spike Q5 §1: 250 SOL quoted, 300 refused `INSUFFICIENT_FUNDS`, the solver's BTC inventory):
/// an advance over it is refused, as a purchase over a collection's limit is. The purchase limit's ceiling on this home is the same.
pub const RELAY_SOL_BTC_MAX_LAMPORTS: u128 = 250_000_000_000;
/// The least and the most one frog purchase may cost on this home when no collection limit is set: 1 SOL and 10 SOL, the operator's
/// decision of 2026-10-09 (D-b, docs/v9/solana-migration.md §2.9; SOL amounts, not conversions of the EVM home's 0.05 / 0.5 ETH).
pub const MIN_PURCHASE_LAMPORTS: u128 = 1_000_000_000;
pub const MAX_PURCHASE_LAMPORTS: u128 = 10_000_000_000;
/// docs/v9/ordinals-collections.md §7: the least a collection's own minimum purchase may be (`set_purchase_limit`'s `min`): 0.15 SOL,
/// the smallest SOL to BTC order Relay was measured filling (2026-10-10, 0.15 SOL for 0.00019438 BTC). No collection goes under it.
pub const ABS_MIN_PURCHASE_LAMPORTS: u128 = 150_000_000;
/// The least SOL to BTC deposit judged (§7, "refuse under what Relay fills"): the smallest purchase less the advance's gas keep, so a
/// deposit is never smaller than what Relay was measured filling, rounded down by the keep.
pub const RELAY_SOL_BTC_MIN_LAMPORTS: u128 = ABS_MIN_PURCHASE_LAMPORTS - sh::SOL_GAS_KEEP_LAMPORTS;
/// A frog floor reading is posted within this of its hand-in, or dropped (the module's `FLOOR_TTL` is 3 h; a reading an hour old is
/// not what the reader saw now).
pub const READING_TTL_SECS: u64 = 3_600;

/// A frog purchase's bounds in the home's unit: `(least, most without a collection limit, the ceiling of any limit, the unit)`.
pub fn bounds(solana: bool) -> (u128, u128, u128, &'static str) {
    if solana {
        (MIN_PURCHASE_LAMPORTS, MAX_PURCHASE_LAMPORTS, RELAY_SOL_BTC_MAX_LAMPORTS, "lamports")
    } else {
        (crate::policy::MIN_PURCHASE_WEI, crate::policy::MAX_PURCHASE_WEI, crate::floortrigger::PURCHASE_LIMIT_CEILING_WEI, "wei")
    }
}

/// [`bounds`] with a collection's own minimum purchase (§7; the Solana home only: the EVM home's minimum is the policy's).
pub fn bounds_min(solana: bool, min: Option<u128>) -> (u128, u128, u128, &'static str) {
    let (least, max, ceiling, unit) = bounds(solana);
    (if solana { min.unwrap_or(least) } else { least }, max, ceiling, unit)
}

/// `frogbuy::check_limits_for` in the home's unit.
pub fn check_limits(cost: u128, limit: Option<u128>, solana: bool) -> Result<(), String> {
    check_limits_min(cost, limit, None, solana)
}

/// [`check_limits`] with the collection's own minimum purchase (`FloorState::min_for`, docs/v9/ordinals-collections.md §7).
pub fn check_limits_min(cost: u128, limit: Option<u128>, min: Option<u128>, solana: bool) -> Result<(), String> {
    if !solana {
        return crate::frogbuy::check_limits_for(cost, limit);
    }
    let (min, max, ceiling, unit) = bounds_min(true, min);
    if cost < min {
        return Err(format!("a purchase costing {cost} {unit} is under the {min} minimum"));
    }
    let top = limit.unwrap_or(max).min(ceiling);
    if cost > top {
        return Err(format!("a purchase costing {cost} {unit} is over this collection's {top} maximum"));
    }
    Ok(())
}

/// What an advance keeps at our home address for that purchase's own fees (the Relay deposit, the line's signatures).
pub fn gas_keep(solana: bool) -> u128 {
    if solana { sh::SOL_GAS_KEEP_LAMPORTS } else { rc::GAS_KEEP_WEI }
}

/// [`rc::convertible`] with the home's keep.
pub fn convertible(advanced: u128, solana: bool) -> Option<u128> {
    advanced.checked_sub(gas_keep(solana)).filter(|w| *w > 0)
}

pub fn solana<I: FrogIo>(io: &I) -> bool {
    io.sol_home().is_some()
}

/// A home amount as custody caps and the outbound limiter count it: wei (lamports at this tick's SOL/ETH rate on this home).
pub async fn value_wei<I: FrogIo>(io: &mut I, home: u128) -> Result<u128, String> {
    if !solana(io) {
        return Ok(home);
    }
    let r = io.sol_rate().await?;
    Ok(crate::rates::lamports_to_wei(home, r))
}

/// The custody-cap key of the frog collection: `btc:0x<wrapper>` on the EVM home, `sol:<the books' wX mint>` on this one (or, with no
/// listing in the books yet, `sol:<the configured wrapper>`).
pub fn collection_key<I: FrogIo>(io: &I) -> String {
    if solana(io) {
        if let Some(k) = books::collection_key(io.sol_wrapper().as_ref()) {
            return k;
        }
    }
    match io.sol_wrapper() {
        Some(w) if solana(io) => crate::custodycap::sol_key(&w),
        _ => crate::custodycap::btc_key(&io.wrapper()),
    }
}

// ---- reads ---------------------------------------------------------------------------------------------------------------------

/// The frog module's record key (the first 20 bytes of its address): the only Bitcoin floor sink the frog collection has.
pub fn module_key(module: &Pubkey) -> [u8; 20] {
    module[..20].try_into().expect("20")
}

/// Who we are to the listing: its custodian (the only minter, the taker of advances) is the canister's Solana key. Returns the key's
/// first 20 bytes (the record key the flow passes around as "me").
pub async fn custodian_ok<I: FrogIo>(io: &mut I) -> Result<[u8; 20], String> {
    books::custodian_ok(io).await
}

/// The listing's membership root (`ListingKind::Inscriptions`): what a proof is checked against, here and by the books' `mint_item`.
pub async fn root<I: FrogIo>(io: &mut I) -> Result<[u8; 32], String> {
    books::root(io).await
}

/// Whether `key` (a floor sink's record key) names the frog collection's own module, and why not.
pub async fn identity<I: FrogIo>(io: &mut I, key: [u8; 20]) -> Result<Option<String>, String> {
    books::identity(io, key).await
}

/// The frog module as the item rules see it (`floorsink::view`'s Solana form, `solhome::books_sink_view`): lamports throughout; its
/// per-item maximum within the collection's limit (lamports; [`MAX_PURCHASE_LAMPORTS`] without one).
pub async fn read_sink<I: FrogIo>(io: &mut I, kind: SinkKind, key: [u8; 20], limit: Option<u128>) -> Result<SinkView, String> {
    books::read_sink(io, kind, key, limit).await
}

/// The module's unit in transit (an advance open).
pub async fn transit_open<I: FrogIo>(io: &mut I) -> Result<bool, String> {
    books::transit_open(io).await
}

/// The source `mint_item` checks for an inscription bought in `txid` (display order) at `vout`: `keccak(tx ‖ vout big-endian)`.
pub fn source_of(txid: [u8; 32], vout: u32) -> [u8; 32] {
    crate::evmtx::keccak(&[txid.as_slice(), &vout.to_be_bytes()].concat())
}

/// Whether the ledger holds this frog, minted from this very outpoint (its held marker names an item whose source is the outpoint):
/// the Solana form of `depositMinted(outpointKey)`.
pub async fn minted<I: FrogIo>(io: &mut I, buy: &Bought) -> Result<bool, String> {
    books::minted(io, buy).await
}

// ---- the line ------------------------------------------------------------------------------------------------------------------

/// A frog step as the home takes it.
pub enum Call {
    Advance(u128),
    Deposit(relaysol::SolDeposit),
    /// `settle_advance(cost)`: the advanced frog backs the module's unit.
    Settle(u128),
    /// `item_arrived(cost)`: a frog bought from the operator's float, reimbursed.
    Arrived(u128),
    /// `return_advance(lamports)`.
    Return(u128),
    /// `fund_reserve(lamports)`: a written-off order's late refund.
    Reserve(u128),
    Mint { inscription: String, proof: Vec<[u8; 32]>, outpoint: Outpoint },
    Post { price: u64, confidence: u16, depth: u32 },
}

fn u64_of(x: u128, what: &str) -> Result<u64, String> {
    u64::try_from(x).map_err(|_| format!("{what} {x} is over a u64 of lamports"))
}

/// A frog step: a Relay deposit is signed on the money line (simulated first: a step that would fail costs no signature; audit A5-03:
/// the deposit is the dedicated Relay key's alone, the line funds it first) and a copy of what went out is kept (`FrogState.sol_step`);
/// every other step is the books' ([`books::send`]). Nothing new while paused (the frog pass does not run then).
pub async fn send<I: FrogIo>(io: &mut I, st: &mut FrogState, step: FStep, call: Call) -> Result<String, String> {
    let Call::Deposit(d) = call else { return books::send(io, st, step, call).await };
    if io.paused() {
        return Err("paused: nothing new is signed".into());
    }
    if st.sol_step.is_some() {
        return Err("the frog side's step is still out on the Solana line (one at a time)".into());
    }
    let what = LineStep::Frog(step.clone()).label();
    sh::ready(io, LineOwner::Frog, io.now(), &what).await?;
    let ixs = vec![relaysol::deposit_instruction(&io.sol_relay().await?, &d)];
    let now = io.now();
    let (text, p) = sh::sign(io, LineStep::Frog(step), now, ixs).await?;
    st.sol_step = Some(p);
    io.save(st);
    Ok(text)
}

/// The line once a frog pass (`solhome::tick_line`), then the frog side's outcomes: the books follow a step that moved SOL
/// ([`books::landed`]), then each is resolved as on the EVM home (`frogflow::mined`, `convflow::mined`); a Relay deposit that landed is
/// remembered for its refund's label.
pub async fn resolve<I: FrogIo>(io: &mut I, st: &mut FrogState, fl: &mut crate::floortrigger::FloorState) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(t) = sh::tick_line(io, io.now()).await {
        out.push(format!("items: {t}"));
    }
    let w = io.sol_wrapper();
    let me = books::coll_for(w.as_ref()).ok().map(|(id, _)| id);
    let mut l = io.sol_line();
    let mine = l.take_where(LineOwner::Frog, |o| owns(st, me, o));
    if mine.is_empty() {
        if let Some(p) = st.sol_step.clone().filter(|p| l.pending.as_ref().is_none_or(|x| x.intent != p.intent)) {
            // the step is no longer on the line and left no outcome (a line restored without it): taken back as not done
            st.sol_step = None;
            io.save(st);
            if let LineStep::Frog(step) = &p.step {
                books::landed(w.as_ref(), st, step, false);
                let r = mined_any(io, st, fl, step, false).await;
                out.push(format!("items: sol line: {} is no longer on the line and left no outcome: taken back, {r}", p.step.label()));
            }
        }
        return out;
    }
    io.sol_line_save(&l);
    for o in mine {
        let LineStep::Frog(step) = o.step.clone() else { continue };
        if st.sol_step.as_ref().is_some_and(|p| p.intent == o.intent) {
            st.sol_step = None;
        }
        let ok = o.ok == Some(true);
        if let (FStep::Convert { frog }, true) = (&step, ok) {
            if let Some(crate::relayconv::CPhase::Depositing { order_id, .. }) = st.conv(*frog).map(|c| c.phase.clone()) {
                let mut l = io.sol_line();
                l.deposits.push(SolDepositRec { buy: *frog, order_id, signature: o.signature.clone(), frog: Some(true) });
                io.sol_line_save(&l);
            }
        }
        io.save(st);
        books::landed(w.as_ref(), st, &step, ok);
        let r = mined_any(io, st, fl, &step, ok).await;
        out.push(match o.ok {
            Some(true) => format!("items: sol line: {} landed ({}): {r}", o.step.label(), o.signature),
            Some(false) => format!("items: sol line: {} FAILED on chain ({}): {r}", o.step.label(), o.signature),
            None => format!("items: sol line: {} never landed (abandoned): {r}", o.step.label()),
        });
        io.save(st);
    }
    out
}

/// Whether a frog outcome on the line is this collection's (`me`, its books id): a book update names its collection (`books:<id>`);
/// a step that moved SOL is the one this state sent (its `sol_step`), or names a frog this state holds (frog ids are unique
/// canister-wide); a step that names no frog and no collection (none moves SOL) is anyone's, as before.
pub fn owns(st: &FrogState, me: Option<u16>, o: &sh::LineOutcome) -> bool {
    let LineStep::Frog(step) = &o.step else { return false };
    if let Some(id) = o.signature.strip_prefix("books:") {
        return me.is_some_and(|m| id == m.to_string());
    }
    if st.sol_step.as_ref().is_some_and(|p| p.intent == o.intent) {
        return true;
    }
    match step.frog_id() {
        Some(f) => st.frogs.iter().any(|x| x.id == f) || st.conv(f).is_some(),
        None => true,
    }
}

async fn mined_any<I: FrogIo>(io: &mut I, st: &mut FrogState, fl: &mut crate::floortrigger::FloorState, step: &FStep, ok: bool) -> String {
    match step {
        FStep::Advance { .. } | FStep::Convert { .. } | FStep::GiveBack { .. } | FStep::LateCredit { .. } => crate::convflow::mined(io, st, step, ok).await,
        FStep::Post => {
            st.floor_reading = None;
            if ok { "the frog listing's floor is posted".into() } else { "the floor post did not land: the reading is dropped".into() }
        }
        _ => crate::frogflow::mined(io, st, fl, step, ok).await,
    }
}

// ---- the floor post -------------------------------------------------------------------------------------------------------------

/// A BTC floor reading for the frog listing, handed in (the runner's sources: the Ordinals Wallet book), posted in lamports.
#[derive(candid::CandidType, serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct FrogReading {
    /// Satoshis per frog.
    pub sats: u64,
    pub confidence_bps: u16,
    pub depth_items: u32,
    pub at: u64,
}

/// A reading in sats as the module's floor: lamports per whole unit at SOL/BTC (rounded down), clamped into the module's `floor_bounds`
/// (D-H-2: what is posted is what the module accepts).
pub fn post_price(sats: u64, sats_per_sol: u64, bounds: (u64, u64)) -> u64 {
    let l = u64::try_from(crate::rates::sats_to_lamports(sats as u128, sats_per_sol)).unwrap_or(u64::MAX);
    l.max(bounds.0).min(bounds.1.max(bounds.0))
}

/// Every fresh reading the runner hands in (`frog_floor_reading`: the Ordinals Wallet book) is posted in the books, the canister the
/// poster ([`books::post_act`]); while the poster is paused a reading is dropped, loud.
pub async fn post_act<I: FrogIo>(io: &mut I, st: &mut FrogState) -> Result<Option<String>, String> {
    books::post_act(io, st).await
}

// ---- Relay SOL → BTC (docs/v9/spikes/q5-relay-solana-legs.md) ---------------------------------------------------------------------

/// What the runner must ask Relay for: deposited by the dedicated Relay key (audit A5-03), to the float, refunded to our Solana address.
pub async fn ask<I: FrogIo>(io: &mut I, lamports: u128) -> Result<rc::ConvAsk, String> {
    let me = soltx::b58_encode(&io.sol_me().await?);
    let relay = soltx::b58_encode(&io.sol_relay().await?);
    let float = io.float_address().await?;
    Ok(rc::ConvAsk { amount_wei: lamports, user: relay, recipient: float, refund_to: me, origin_chain_id: relaysol::RELAY_SOLANA,
        destination_chain_id: rc::RELAY_BITCOIN, origin_currency: relaysol::SOL_NATIVE.into(), destination_currency: rc::BTC_NATIVE.into() })
}

/// A funded conversion's quote, judged with a Solana origin and a Bitcoin destination (rules S1–S12): the depository's state and its
/// `ProgramData` read at finalized (S3), the float as the only recipient, our Solana address as every Solana refund, the minimum against
/// our own SOL/BTC rate less the fee bound (S10). Returns the deposit to sign (our own instruction, built from the judged amount and id).
pub async fn judge<I: FrogIo>(io: &mut I, body: &[u8], at: u64, lamports: u128, max_fee_bps: u16, used: &[[u8; 32]])
    -> Result<relaysol::SolDeposit, String> {
    if lamports > RELAY_SOL_BTC_MAX_LAMPORTS {
        return Err(format!("{lamports} lamports is over Relay's SOL to BTC capacity of {RELAY_SOL_BTC_MAX_LAMPORTS} an order: refused"));
    }
    // §7: under the smallest order Relay was measured filling, a deposit is refused rather than risked
    if lamports < RELAY_SOL_BTC_MIN_LAMPORTS {
        return Err(format!("{lamports} lamports is under the {RELAY_SOL_BTC_MIN_LAMPORTS} least SOL to BTC order Relay was measured filling: refused"));
    }
    let state = io.sol_accounts(&[soltx::key(relaysol::RELAY_DEPOSITORY)], 128).await?.pop().flatten()
        .ok_or("Relay's depository state does not exist")?;
    let pd = io.sol_account_head(&soltx::key(relaysol::PROGRAMDATA), 45).await?.ok_or("Relay's ProgramData does not exist")?;
    let (allocator, authority, slot) = relaysol::s3_reads(&state, &pd)?;
    let me = soltx::b58_encode(&io.sol_me().await?);
    let relay = soltx::b58_encode(&io.sol_relay().await?);
    let float = io.float_address().await?;
    let rate = io.rate().await?;
    let floor = rc::min_sats_floor(lamports, rate, max_fee_bps)? as u128;
    let w = relaysol::SolWant { our_sol: &me, depositor: &relay, dest_chain: "bitcoin", dest_recipient: &float, dest_currency: rc::BTC_NATIVE,
        lamports: u64_of(lamports, "the deposit")?, min_out_floor: floor, allocator_read: &allocator, upgrade_authority_read: authority.as_deref(),
        deploy_slot_read: slot, now: io.now(), handed_in_at: at, used };
    relaysol::judge_sol(body, &w)
}

/// The refund's label (Q5 §5.2), as the Ethereum side reads it (`solhome::refund_label`).
pub async fn refund_label<I: FrogIo>(io: &mut I, order_id: &[u8; 32]) -> String {
    sh::refund_label(io, order_id).await
}

// ---- the custodian's side: frog redemptions from the books' tickets (redeem.rs on the Solana home) ----------------------------------

/// `ticketCount()` and the finalized slot. Refused for a listing of another Bitcoin network than ours (`hrp`): its tickets are never
/// scanned.
pub async fn ticket_count<I: SolIo>(io: &mut I, w: Pubkey, hrp: &str) -> Result<(u64, u64), String> {
    books::ticket_count(io, &w, hrp).await
}

/// One frog ticket, from the books.
pub async fn ticket<I: SolIo>(_io: &mut I, w: Pubkey, id: u64) -> Result<crate::books::ledger::Ticket, String> {
    books::ticket(&w, id)
}

/// A ticket's taproot destination as text: an Inscriptions ticket names a taproot address of the listing's network, or the burn (BIP341's
/// NUMS key, out of custody for good, as the EVM wrapper's `BURN_ADDRESS`, on the network whose prefix is `hrp`); an Ethereum one is no
/// frog's (empty: refused by the rule's script check).
pub fn destination_text(d: &crate::books::ledger::Destination, hrp: &str) -> String {
    use crate::books::ledger::Destination;
    match d {
        Destination::Taproot(s) => s.clone(),
        Destination::Burn => burn_address(hrp).into(),
        Destination::Eth(_) | Destination::Sol(_) => String::new(),
    }
}

/// rook_core's `Wrapper.BURN_ADDRESS` (`hashing.rs`): a key-path-only taproot output nobody holds the key of.
pub const BTC_BURN_ADDRESS: &str = "bc1p2zffkaxp5py4fdutfdsrt6t6tcrc5ks09rkfd428hlhf4n5q8tqqwv7ahk";
/// The same output on signet and testnet4 (the `tb` prefix; rook_core's `hashing` tests hold the pair).
pub const BTC_BURN_ADDRESS_TB: &str = "tb1p2zffkaxp5py4fdutfdsrt6t6tcrc5ks09rkfd428hlhf4n5q8tqqeygjde";

/// The burn address on the network whose prefix is `hrp` (mainnet's for any prefix but `tb`: never a spendable guess).
pub fn burn_address(hrp: &str) -> &'static str {
    if hrp == "tb" { BTC_BURN_ADDRESS_TB } else { BTC_BURN_ADDRESS }
}

/// The ticket as the delivery rule reads it (`delivery::TicketRead`), on unix clocks (§2.4: the deadlines are unix time, no
/// parent-chain clock): the block clock is the same time in 12-second steps (`deliver_by / 12` against `chain_time / 12`, the canister's
/// time since the read taken off in whole steps, rounded up), so it never binds later than the time clock. `chain_time` is the
/// finalized block's time, at least the canister's own: the deadline only nearer, the safe direction for a rule that stops.
pub async fn ticket_read<I: SolIo>(io: &mut I, w: Pubkey, id: u64, now: u64, hrp: &str) -> Result<crate::delivery::TicketRead, String> {
    books::ticket_read(io, &w, id, now, hrp).await
}

/// The inscription a ticket's item is (`Item.inscription_id`, its key checked to be that id's leaf).
pub async fn inscription_of<I: SolIo>(_io: &mut I, w: Pubkey, item_id: u128) -> Result<String, String> {
    books::inscription_of(&w, item_id)
}

/// Whether the canister is the listing's custodian, the one party `record_delivery` and `mark_undeliverable` accept: it is, once the
/// books list the frog collection.
pub async fn may_record<I: SolIo>(_io: &mut I, w: Pubkey) -> Result<bool, String> {
    Ok(books::coll_for(Some(&w)).is_ok())
}

/// A custodian's step (`record_delivery`, `mark_undeliverable`, `note_sent` of a frog ticket): a book update, final at once, posted as a
/// landed outcome on the line. A record, a mark and a note are what already left, so they go out while paused too (C-I-5, as on the
/// EVM home).
pub async fn custody_send<I: SolIo>(_io: &mut I, w: Pubkey, step: sh::CustodyStep, now: u64) -> Result<(String, sh::LinePending), String> {
    books::custody_send(&w, step, now)
}

/// The custodian's step still out on the line, if any (its intent).
pub fn custody_pending<I: SolIo>(io: &mut I, ticket: u64) -> Option<u64> {
    let l = io.sol_line();
    match l.pending.as_ref().map(|p| (&p.step, p.intent)) {
        Some((LineStep::Custody(sh::CustodyStep::Record { ticket: t, .. } | sh::CustodyStep::Mark { ticket: t }
            | sh::CustodyStep::Sent { ticket: t, .. }), i)) if *t == ticket => Some(i),
        _ => None,
    }
}
