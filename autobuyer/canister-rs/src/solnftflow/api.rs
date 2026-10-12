//! The Solana NFT flow on the canister's surface: the runner's hand-in (`sol_nft_evidence`), the operator's knobs (`sol_nft_configure`:
//! a widening change waits [`CHANGE_DELAY_SECS`], as the treasury's do; `sol_nft_set_off`, stop-only), the status (`sol_nft_status`),
//! and the IC world of the tick.
use candid::CandidType;
use serde::{Deserialize, Serialize};

use super::*;
use crate::solkey::KeyPath;

/// What the operator sees of one collection: its knobs, the last read (asks and floor) and what is in flight.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SolNftView {
    pub coll: u16,
    pub collection: String,
    pub standard: Standard,
    pub premium_bps: u16,
    pub m2: bool,
    pub pending: Option<PendingChange>,
    pub evidence_at: u64,
    pub candidates: u32,
    pub read_at: u64,
    pub asks: Vec<AskView>,
    pub floor: Option<SolFloor>,
    pub floor_at: u64,
    pub floor_note: String,
    pub arrival: Option<Arrival>,
    /// Items in custody at `K_main`: `(mint, signature that brought it, cost)`.
    pub held: Vec<(String, String, u64)>,
    pub note: String,
    pub out: Option<String>,
    pub signed_today: u32,
    pub off: bool,
    /// The canister's M2 lookup table (2026-10-12, `table`): its address, and whether its creation landed (`None`: not made yet; it is
    /// made when an M2 ask first waits for it).
    pub lookup_table: Option<(String, bool)>,
}

pub fn view(coll: u16) -> Option<SolNftView> {
    let (collection, _) = listing_of(coll).ok()?;
    let (mut asks, evidence_at, candidates) = cache(coll, |k| {
        let asks: Vec<AskView> = k.asks.iter().map(|(a, m)| AskView { venue: a.venue, listing: b58_encode(&a.listing), asset: b58_encode(&a.asset),
            price: a.price, total: a.total, rent: a.rent(m), keyless: a.keyless() }).collect();
        (asks, k.evidence.as_ref().map(|e| e.at).unwrap_or(0), k.evidence.as_ref().map(|e| e.cands.len() as u32).unwrap_or(0))
    });
    asks.sort_by_key(|a| a.total);
    asks.truncate(ASKS_SHOWN);
    with(|s| {
        let c = s.cols.get(&coll).cloned().unwrap_or_default();
        Some(SolNftView { coll, collection: b58_encode(&collection), standard: c.standard, premium_bps: c.premium_bps, m2: c.m2, pending: c.pending.clone(),
            evidence_at, candidates,
            read_at: c.read_at, asks, floor: c.floor.clone(), floor_at: c.floor_at, floor_note: c.floor_note.clone(), arrival: c.arrival.clone(),
            held: c.held.iter().map(|(k, h)| (b58_encode(k), h.signature.clone(), h.cost)).collect(), note: c.note.clone(),
            out: s.out.as_ref().map(|p| p.step.label()), signed_today: s.signed_today, off: s.off,
            lookup_table: s.table.as_ref().map(|t| (b58_encode(&t.key), t.landed)) })
    })
}

/// The operator's knobs of collection `coll` (pure): the standard while nothing is held; the premium (at most [`MAX_PREMIUM_BPS`]) and
/// M2 at once when they narrow, [`CHANGE_DELAY_SECS`] later when they widen.
pub fn configure(coll: u16, standard: Option<Standard>, premium_bps: Option<u16>, m2: Option<bool>, now: u64) -> Result<String, String> {
    listing_of(coll)?;
    let held = crate::desk::state::with(|s| s.books.get(coll).map(|c| c.ledger.held + c.module.transit_units).unwrap_or(0));
    if premium_bps.is_some_and(|p| p > MAX_PREMIUM_BPS) {
        return Err(format!("refused: a premium over {MAX_PREMIUM_BPS} bps"));
    }
    with_mut(|s| {
        let c = s.cols.entry(coll).or_default();
        let mut out = vec![];
        if let Some(st) = standard.filter(|x| *x != c.standard) {
            if held != 0 || c.arrival.is_some() {
                return Err("refused: the standard is fixed once anything was bought".to_string());
            }
            c.standard = st;
            out.push(format!("standard {st:?}"));
            // audit C1-I5: the asks (and the hand-in) were verified under the old standard: none is bought from, the next hand-in is
            // read under the new one
            cache_mut(coll, |k| *k = Cache::default());
            c.read_at = 0;
        }
        let (p, m) = (premium_bps.unwrap_or(c.premium_bps), m2.unwrap_or(c.m2));
        // audit C1-I1: the narrowing half of a combined change (a lower premium with M2 switched on, or M2 off with a raise) applies
        // at once; only the widening half waits
        let (narrow_p, narrow_m) = (p <= c.premium_bps, !m || c.m2);
        if narrow_p != narrow_m {
            if narrow_p {
                c.premium_bps = p;
                out.push(format!("premium {p} bps: at once"));
            } else {
                c.m2 = m;
                out.push(format!("M2 {m}: at once"));
            }
        }
        if narrow_p && narrow_m {
            c.premium_bps = p;
            c.m2 = m;
            c.pending = None;
            out.push(format!("premium {p} bps, M2 {m}: at once"));
        } else {
            c.pending = Some(PendingChange { premium_bps: p, m2: m, at: now + CHANGE_DELAY_SECS });
            out.push(format!("premium {p} bps, M2 {m}: applied at {} (unix)", now + CHANGE_DELAY_SECS));
        }
        Ok(format!("collection {coll}: {}", out.join("; ")))
    })
}

fn now_secs() -> u64 {
    ic_cdk::api::time() / 1_000_000_000
}

/// The runner hands in Magic Eden's answers for collection `coll`: the listings (`/v2/collections/<symbol>/listings?listingAggMode=true`,
/// at most 64 KiB; the cheapest 40 are kept) and the stats (`/stats`, at most 4 KiB). Read on chain at the next pass; only the
/// addresses and the reference floor are used. Operator-only.
#[ic_cdk::update]
pub fn sol_nft_evidence(coll: u16, listings_json: String, stats_json: String) -> Result<String, String> {
    crate::canister::only_operator();
    crate::canister::not_paused()?;
    crate::canister::not_ticking()?;
    floor::put_evidence(coll, &listings_json, &stats_json, now_secs())
}

/// The operator sets a Solana NFT collection's standard (while nothing is held), its premium over the floor (O-T1) and whether M2
/// listings are bought buyer-only (`None`: unchanged; on by default since 2026-10-12). A lower premium or M2 switched off applies at
/// once; a raise or M2 switched (back) on waits 24 h.
#[ic_cdk::update]
pub fn sol_nft_configure(coll: u16, standard: Option<crate::solnft::Standard>, premium_bps: Option<u16>, m2: Option<bool>) -> Result<String, String> {
    crate::canister::only_operator();
    crate::canister::not_paused()?;
    crate::canister::not_ticking()?;
    configure(coll, standard, premium_bps, m2, now_secs())
}

/// The operator switches the flow off (nothing new is signed; outcomes are still booked) or on again.
#[ic_cdk::update]
pub fn sol_nft_set_off(off: bool) -> Result<String, String> {
    crate::canister::only_operator();
    with_mut(|s| s.off = off);
    Ok(format!("the Solana NFT flow is {}", if off { "off" } else { "on" }))
}

/// One Solana NFT collection: the knobs, the last asks and floor, the items in custody, what is in flight.
#[ic_cdk::query]
pub fn sol_nft_status(coll: u16) -> Option<crate::solnftflow::api::SolNftView> {
    view(coll)
}

pub(crate) struct SolNftNet {
    sol: Option<crate::solhome::SolNet>,
}

crate::sol_io_via!(SolNftNet, sol);

impl crate::chainlink::driver::DeskIo for SolNftNet {
    async fn key_of(&mut self, p: &KeyPath) -> Result<Pubkey, String> {
        crate::solkey::derive_key(p).await
    }
}

impl SolNftIo for SolNftNet {
    fn now(&self) -> u64 {
        now_secs()
    }
    fn paused(&self) -> bool {
        crate::canister::is_paused()
    }
    fn caps(&self) -> Option<crate::custodycap::CapPolicy> {
        Some(crate::canister::caps_of(&crate::canister::cfg()))
    }
    fn cap_book(&self) -> crate::custodycap::Book {
        crate::canister::CUSTODY_BOOK.with(|b| b.borrow().clone())
    }
    fn outflow_check(&mut self, key: &str, collection: &str, flow: crate::outflow::Flow, amount: u128) -> Result<(), String> {
        crate::canister::outflow_check(key, collection, flow, amount)
    }
    fn outflow_record(&mut self, key: &str, collection: &str, flow: crate::outflow::Flow, amount: u128) {
        crate::canister::outflow_record(key, collection, flow, amount)
    }
}

/// The flow's pass for the canister's timer and the runner's tick (the tick's flag held by the caller). Nothing without a Solana home.
pub(crate) async fn tick_ic() -> Vec<String> {
    match crate::solhome::SolNet::of_install() {
        Ok(Some(sol)) => super::tick(&mut SolNftNet { sol: Some(sol) }, now_secs()).await,
        Ok(None) => vec![],
        Err(e) => vec![format!("solnft: waits: {e}")],
    }
}
