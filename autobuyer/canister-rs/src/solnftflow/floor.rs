//! The floor of a Solana NFT collection on the books (docs/v9/solana-nfts-canister.md §1 step 1, §5): the runner's hand-in (Magic Eden's
//! `/v2/collections/<symbol>/listings?listingAggMode=true` and `/stats`, untrusted: only addresses and the reference are kept), one
//! agreed read at finalized per hand-in, every candidate verified (`solnftbuy::verify_all`), the floor (`solnftfloor::floor_from`)
//! posted to the books inside the module's bounds. The same read's asks are what the buy picks from (§1 step 2).
use serde_json::Value;

use super::*;
use crate::desk::state as desk;
use crate::solnftbuy as nbuy;
use crate::solnftfloor::{self as nfloor, FloorParams};

/// The hand-in, checked and kept (idempotent: the same answers give the same evidence; a newer hand-in replaces the older).
pub fn put_evidence(coll: u16, listings_json: &str, stats_json: &str, now: u64) -> Result<String, String> {
    if listings_json.len() > MAX_LISTINGS_BYTES || stats_json.len() > MAX_STATS_BYTES {
        return Err(format!("refused: the hand-in is over {MAX_LISTINGS_BYTES} bytes of listings or {MAX_STATS_BYTES} of stats"));
    }
    listing_of(coll)?;
    let v: Value = serde_json::from_str(listings_json).map_err(|e| format!("refused: the listings are not JSON: {e}"))?;
    let stats: Value = serde_json::from_str(stats_json).map_err(|e| format!("refused: the stats are not JSON: {e}"))?;
    let reference = nfloor::reference_from_me_stats(&stats)?;
    // the cheapest MAX_CANDIDATES by Magic Eden's own price (a hint: every fact is read on chain)
    let mut rows: Vec<Value> = v.as_array().cloned().ok_or("refused: the listings are not an array")?;
    rows.sort_by(|a, b| a["price"].as_f64().unwrap_or(f64::MAX).total_cmp(&b["price"].as_f64().unwrap_or(f64::MAX)));
    rows.truncate(MAX_CANDIDATES);
    let cands = nfloor::candidates_from_me(&Value::Array(rows));
    if cands.is_empty() {
        return Err("refused: no listing of a known venue (M2, MMM, TENSOR_MARKETPLACE_LISTING) in the hand-in".into());
    }
    let n = cands.len();
    with_mut(|s| {
        s.cols.entry(coll).or_default();
    });
    cache_mut(coll, |c| c.evidence = Some(Evidence { cands, reference, at: now }));
    Ok(format!("collection {coll}: {n} candidates and a reference floor of {reference} lamports, read on chain at the next pass"))
}

/// The floor's confidence from its agreement with the reference: 10,000 bps less twice the gap (the gap is at most
/// `max_gap_bps`, so at least 6,000).
pub fn confidence(f: &SolFloor) -> u16 {
    let gap = (f.price.abs_diff(f.reference) as u128 * 10_000 / f.reference.max(1) as u128).min(5_000) as u16;
    10_000u16.saturating_sub(gap.saturating_mul(2))
}

/// One read of a collection's new evidence: the asks verified and kept, the floor posted. `Ok(None)`: nothing new to read.
pub async fn pass<I: SolNftIo>(io: &mut I, coll: u16, now: u64) -> Result<Option<String>, String> {
    let read_at = with(|s| s.cols.get(&coll).map(|c| c.read_at).unwrap_or(0));
    let Some(ev) = cache(coll, |c| c.evidence.clone().filter(|e| e.at > read_at)) else { return Ok(None) };
    // audit C1-06 (review F1, as `home_books` and `frogsol::books`): no post while a link of the collection is open (its undo would
    // restore the snapshot and wipe the post); the evidence stays unread until the collection is free
    if desk::collection_busy(coll) {
        return Ok(None);
    }
    if now.saturating_sub(ev.at) > EVIDENCE_TTL_SECS {
        with_mut(|s| s.cols.entry(coll).or_default().read_at = ev.at);
        return Ok(Some(format!("solnft floor {coll}: the hand-in is {} s old: not read", now.saturating_sub(ev.at))));
    }
    let (collection, _) = listing_of(coll)?;
    let standard = with(|s| s.cols.get(&coll).map(|c| c.standard)).unwrap_or(Standard::Programmable);
    let (clock, _) = io.sol_clock().await?;
    let mut a = read(io, &nbuy::ask_keys_all(&ev.cands, &collection, standard)).await?;
    let more: Vec<Pubkey> = ev.cands.iter().flat_map(|c| nbuy::ask_keys_then(c, &a)).filter(|k| !a.contains_key(k)).collect();
    if !more.is_empty() {
        a.extend(read(io, &more).await?);
    }
    let (asks, dropped) = nbuy::verify_all(&a, &ev.cands, &collection, standard, clock as i64);
    let floor = nfloor::floor_from(&asks, ev.reference, &FloorParams::default());
    with_mut(|s| {
        let c = s.cols.entry(coll).or_default();
        c.read_at = ev.at;
        c.note = format!("{} verified, {} dropped at {now}", asks.len(), dropped.len());
    });
    cache_mut(coll, |c| {
        c.asks = asks.clone();
        c.refused.clear();
    });
    let f = match floor {
        Ok(f) => f,
        Err(e) => {
            with_mut(|s| s.cols.entry(coll).or_default().floor_note = format!("no floor at {now}: {e}"));
            return Ok(Some(format!("solnft floor {coll}: {} asks verified ({} dropped); no floor: {e}", asks.len(), dropped.len())));
        }
    };
    // the post: inside the module's bounds, refused while the poster is down
    let post = desk::with_mut(|s| {
        let kp = s.books.keeper_paused;
        let c = s.books.get(coll).map_err(|e| format!("{e}"))?;
        if c.kd(kp) {
            return Err("the poster is paused: nothing is posted".to_string());
        }
        let (lo, hi) = c.module.floor_bounds(now as i64).unwrap_or((0, u64::MAX));
        let price = f.price.max(lo).min(hi.max(lo));
        s.books.post_floor(coll, price, confidence(&f), f.depth, now as i64).map(|(acc, _)| (price, acc)).map_err(|e| format!("the books refused: {e}"))
    });
    let text = match &post {
        Ok((price, acc)) => format!("solnft floor {coll}: post_floor({price} lamports, {} bps, depth {}) from {} verified asks of {} sellers \
            (reference {}): accepted {acc}", confidence(&f), f.depth, asks.len(), f.sellers, f.reference),
        Err(e) => format!("solnft floor {coll}: a floor of {} lamports is not posted: {e}", f.price),
    };
    with_mut(|s| {
        let c = s.cols.entry(coll).or_default();
        if let Ok((_, acc)) = &post {
            c.note_floor(*acc, now);
        }
        c.floor = Some(f);
        c.floor_at = now;
        c.floor_note = text.clone();
    });
    Ok(Some(text))
}
