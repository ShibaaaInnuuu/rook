//! The floor of a Solana NFT collection (docs/v9/solana-nfts-canister.md §5), as the Ordinals and Ethereum feeds take it: from
//! **listings verified on chain** at `finalized` (every venue's escrow or seller really holds a member; Magic Eden M2 included —
//! a notary-gated ask is still a real market price), **one ask per seller and one per asset**, the `rank`-th cheapest of at least
//! `min_sellers` distinct sellers; then checked against Magic Eden's public floor and **refused when they disagree** by more
//! than `max_gap_bps`. The canister also learns the cheapest ask it can buy without a key (it may sit above the floor: §3.1).
//!
//! Where the candidates come from: Magic Eden's public listings endpoint (`listingAggMode=true` aggregates M2, mmm and Tensor
//! Marketplace), an untrusted index — only its addresses are used ([`candidates_from_me`]). Pure, except the read through
//! `solnftbuy::verified_asks`.

use candid::CandidType;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::solnft::{Member, Standard};
use crate::solnftbuy::{Ask, Candidate, Venue};
use crate::soltx::{self, Pubkey};

#[derive(CandidType, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct FloorParams {
    /// Distinct sellers with a verified ask, at least.
    pub min_sellers: u8,
    /// The floor is the `rank`-th cheapest of them (1 = the cheapest).
    pub rank: u8,
    /// The on-chain floor and the reference (Magic Eden's stats) may differ by at most this; more is a refusal.
    pub max_gap_bps: u16,
}

impl Default for FloorParams {
    fn default() -> Self {
        FloorParams { min_sellers: 3, rank: 2, max_gap_bps: 2_000 }
    }
}

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SolFloor {
    /// Lamports: the `rank`-th cheapest seller's ask (the listing price, as marketplaces quote floors).
    pub price: u64,
    /// Distinct sellers with a verified ask.
    pub sellers: u32,
    /// Distinct sellers within 5 % of the floor (the book's depth there).
    pub depth: u32,
    /// The cheapest verified ask.
    pub cheapest: u64,
    /// The cheapest ask the canister can take without a key, all in (fees, royalties; rent apart), and its venue.
    pub keyless_total: Option<u64>,
    pub keyless_venue: Option<Venue>,
    /// The reference it was checked against.
    pub reference: u64,
}

/// The floor from verified asks, or why there is none. `reference`: Magic Eden's public floor in lamports (`floorPrice` of
/// `/v2/collections/<symbol>/stats`).
pub fn floor_from(asks: &[(Ask, Member)], reference: u64, p: &FloorParams) -> Result<SolFloor, String> {
    if p.rank == 0 || p.rank > p.min_sellers {
        return Err("rank must be within 1..=min_sellers".into());
    }
    // one ask per asset (the cheapest), then the cheapest per seller
    let mut by_asset: Vec<&Ask> = Vec::new();
    for (a, _) in asks {
        match by_asset.iter_mut().find(|x| x.asset == a.asset) {
            Some(x) if a.price < x.price => *x = a,
            Some(_) => {}
            None => by_asset.push(a),
        }
    }
    let mut best: Vec<(Pubkey, u64)> = Vec::new();
    for a in &by_asset {
        match best.iter_mut().find(|(s, _)| *s == a.seller) {
            Some(x) => x.1 = x.1.min(a.price),
            None => best.push((a.seller, a.price)),
        }
    }
    if best.len() < p.min_sellers as usize {
        return Err(format!("{} distinct sellers with a verified ask; {} are needed", best.len(), p.min_sellers));
    }
    let mut prices: Vec<u64> = best.iter().map(|x| x.1).collect();
    prices.sort_unstable();
    let price = prices[p.rank as usize - 1];
    if reference == 0 {
        return Err("no reference floor".into());
    }
    let gap = price.abs_diff(reference) as u128 * 10_000 / reference as u128;
    if gap > p.max_gap_bps as u128 {
        return Err(format!("the verified floor {price} and the reference {reference} differ by {} %: refused", gap / 100));
    }
    let depth = prices.iter().filter(|x| **x as u128 * 100 <= price as u128 * 105).count() as u32;
    let keyless = asks.iter().filter(|(a, _)| a.keyless()).min_by_key(|(a, _)| (a.total, a.listing));
    Ok(SolFloor { price, sellers: best.len() as u32, depth, cheapest: prices[0], keyless_total: keyless.map(|(a, _)| a.total),
        keyless_venue: keyless.map(|(a, _)| a.venue), reference })
}

/// Candidates from Magic Eden's `/v2/collections/<symbol>/listings?listingAggMode=true` answer: `listingSource` `M2` / `MMM` /
/// `TENSOR_MARKETPLACE_LISTING`, `pdaAddress` (the listing account), `tokenMint`. Other sources (Tensor AMM pools, …) are skipped;
/// malformed entries too. Sorted and deduplicated, so every replica that saw the same set builds the same list.
pub fn candidates_from_me(v: &Value) -> Vec<Candidate> {
    let mut out: Vec<Candidate> = v.as_array().into_iter().flatten().filter_map(|x| {
        let venue = match x["listingSource"].as_str()? {
            "M2" => Venue::M2,
            "MMM" => Venue::Mmm,
            "TENSOR_MARKETPLACE_LISTING" => Venue::Tensor,
            _ => return None,
        };
        Some(Candidate { venue, listing: soltx::pubkey(x["pdaAddress"].as_str()?).ok()?, asset: soltx::pubkey(x["tokenMint"].as_str()?).ok()? })
    }).collect();
    out.sort_by(|a, b| (a.listing, a.asset).cmp(&(b.listing, b.asset)));
    out.dedup();
    out
}

/// Magic Eden's public floor (`floorPrice`, lamports) from `/v2/collections/<symbol>/stats`.
pub fn reference_from_me_stats(v: &Value) -> Result<u64, String> {
    v["floorPrice"].as_u64().or_else(|| v["floorPrice"].as_f64().filter(|f| *f >= 0.0).map(|f| f as u64))
        .filter(|f| *f > 0).ok_or_else(|| format!("no floorPrice in {v}"))
}

/// The floor, end to end: the candidates verified at `finalized` by every provider, then [`floor_from`].
pub async fn read_floor<P: crate::solrpc::SolRpc>(ps: &mut [P], cands: &[Candidate], collection: &Pubkey, listing: Standard, now: i64,
    reference: u64, p: &FloorParams) -> Result<(SolFloor, Vec<(Ask, Member)>), String> {
    let (asks, _) = crate::solnftbuy::verified_asks(ps, cands, collection, listing, now).await?;
    Ok((floor_from(&asks, reference, p)?, asks))
}
