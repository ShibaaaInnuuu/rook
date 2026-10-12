//! Protocol sales and relist tickets, the canister's side (docs/v6/canister-floor-followups.md §7): the state (stable, the
//! upgrade tuple's ELEVENTH element), the sale policy a controller sets per module, the operator's delist, the signed EVM
//! listing, and the queries. Nothing is decided here: `sale.rs` keeps the rules, `saleflow.rs` drives them inside
//! `frogflow`'s tick.

use std::cell::RefCell;

use crate::canister::only_operator;
use crate::custody::Outpoint;
use crate::sale::{SPhase, SalePolicy, SaleState};

thread_local! {
    /// Saved across upgrades as the upgrade tuple's ELEVENTH element.
    pub(crate) static SALES: RefCell<SaleState> = RefCell::new(SaleState::default());
}

fn only_controller() {
    let caller = ic_cdk::api::msg_caller();
    if !ic_cdk::api::is_controller(&caller) {
        ic_cdk::trap(&format!("refused: {caller} is not a controller of this canister."));
    }
}

fn h(a: &[u8; 20]) -> String {
    format!("0x{}", hex::encode(a))
}

/// Custody outputs a delist's move in flight has claimed as its fee: no delivery or return may choose them.
pub(crate) fn reserved() -> Vec<Outpoint> {
    SALES.with(|s| s.borrow().reserved())
}

/// A module's sale policy (§3.1): sell its inventory while the standing reserve is under `reserve_floor_wei`, at
/// `price() × (1 + markup_bps)`, at most `max_open` sales at once; `fee_bps` is an EVM listing's fee share. A controller's
/// act (after the handover, the timelock's). The module must be a floor sink this canister serves. Replaceable: a policy is a
/// number, not an identity.
#[ic_cdk::update]
pub fn set_sale_policy(module: String, reserve_floor_wei: candid::Nat, markup_bps: u32, max_open: u32, fee_bps: u32) -> Result<String, String> {
    only_controller();
    crate::canister::not_ticking()?;
    if crate::canister::cfg().home.is_some() {
        return Err("refused: protocol sales are OFF on the Solana home at launch (rook_core has no itemSold: docs/v9/rook-core-as-built.md §5)".into());
    }
    let m = crate::evmtx::parse_address(module.trim())?;
    let floor: u128 = reserve_floor_wei.0.to_string().parse().map_err(|_| "the reserve floor does not fit in 128 bits")?;
    let known = crate::frognet::FROGS.with(|f| f.borrow().sinks.iter().any(|x| x.sink == m && x.kind == crate::itemsink::SinkKind::Floor));
    if !known {
        return Err(format!("refused: {} is not a floor sink this canister serves", h(&m)));
    }
    let chain_max = crate::canister::cfg().evm_chains.iter().flatten().map(|c| c.max_fee_bps).min();
    SALES.with(|s| s.borrow_mut().set_policy(SalePolicy { module: m, reserve_floor_wei: floor, markup_bps, max_open, fee_bps }, chain_max))?;
    crate::canister::note(format!("sale policy: {} sells under a {floor} wei reserve at +{markup_bps} bps, {max_open} at once", h(&m)));
    Ok(format!("{} sells its inventory while the reserve is under {floor} wei", h(&m)))
}

/// The operator delists a listed item whose delivery-by-listing is recorded (§3.3): the canister moves it to a fresh custody
/// outpoint at the next tick (cancelling the listing), re-deposits it, and reclaims a vault's listing.
#[ic_cdk::update]
pub fn sale_delist(id: u64) -> Result<String, String> {
    only_operator();
    crate::canister::not_paused()?;
    crate::canister::not_ticking()?;
    SALES.with(|s| {
        let mut s = s.borrow_mut();
        let x = s.sale_mut(id).ok_or("no such sale")?;
        match x.phase.clone() {
            SPhase::Recorded { inscription, outpoint, .. } => {
                x.phase = SPhase::Delisting { inscription: inscription.clone(), outpoint };
                Ok(format!("sale {id}: {inscription} is delisted; the move goes out at the next tick"))
            }
            SPhase::Listed { .. } => Err(format!("refused: sale {id}'s delivery-by-listing is not recorded yet; delist once it is")),
            p => Err(format!("refused: sale {id} is {p:?}; only a recorded listing is delisted")),
        }
    })
}

/// §3.5: a Seaport listing of an ERC-721 the canister holds on `chain_id`, signed by its EVM key: the order's components
/// (JSON, the shape a venue's API takes), its hash and the 65-byte signature. Nothing is sent and nothing moves: the
/// collection's approval to Seaport and the posting are the operator's (docs/v6/canister-floor-followups.md §3.5, §11).
#[ic_cdk::update]
pub async fn sale_nft_listing(chain_id: u64, collection: String, token_id: String, ask_wei: candid::Nat, life_secs: u64) -> Result<String, String> {
    only_operator();
    crate::canister::not_paused()?;
    let cfg = crate::canister::cfg();
    // audit 2026-09-27-nft B-I-1: the Ethereum NFT side's custody never lists an NFT (docs/v7/phase1-nft.md §2.2); its chain's key
    // signs only that side's judged transactions
    if cfg.eth_nft.as_ref().is_some_and(|e| e.chains().contains(&chain_id)) {
        return Err(format!("refused: chain {chain_id} is the Ethereum NFT side's (eth_nft): its custody never lists an NFT"));
    }
    let c = cfg.evm_chains.iter().flatten().find(|c| c.chain_id == chain_id).cloned().ok_or_else(|| format!("chain {chain_id} is not one of this canister's EVM chains"))?;
    let coll = crate::evmtx::parse_address(collection.trim())?;
    let token = crate::nftnet::word_of_dec(token_id.trim())?;
    let ask: u128 = ask_wei.0.to_string().parse().map_err(|_| "the ask does not fit in 128 bits")?;
    if ask == 0 || life_secs < c.min_order_life_s {
        return Err(format!("refused: an ask above zero and a life of at least the chain's {} s", c.min_order_life_s));
    }
    let me = crate::canister::ours().await?.0;
    let fee_bps = SALES.with(|s| s.borrow().policies.iter().find(|p| p.fee_bps > 0).map_or(0, |p| p.fee_bps));
    let fee = match (c.rules()?.fee_recipients.first(), fee_bps) {
        (Some(r), b) if b > 0 && b <= c.max_fee_bps => Some((*r, b)),
        _ => None,
    };
    let now = ic_cdk::api::time() / 1_000_000_000;
    let mut salt = [0u8; 32];
    salt[..8].copy_from_slice(&now.to_be_bytes());
    salt[8..16].copy_from_slice(&chain_id.to_be_bytes());
    let o = crate::seaport::listing_order(&me, &coll, &token, ask, fee, now, now.saturating_add(life_secs), salt, [0u8; 32]);
    let seaport = c.seaport_addr()?;
    let hash = o.hash();
    let digest = crate::seaport::digest(&crate::seaport::domain_separator(chain_id, &seaport), &hash);
    let s = crate::canister::sign_evm_recoverable(digest).await?;
    let sig = format!("0x{}{}{:02x}", s.r, s.s, 27 + s.recovery_id);
    let sig_bytes = hex::decode(sig.trim_start_matches("0x")).map_err(|e| e.to_string())?;
    if !crate::seaport::offerer_signed(&hash, &me, &sig_bytes, chain_id, &seaport)? {
        return Err("the signature does not recover as the offerer: not returned".into());
    }
    crate::canister::note(format!("nft listing signed: chain {chain_id} {} #{token_id} at {ask} wei, order 0x{}", collection, hex::encode(hash)));
    Ok(crate::seaport::listing_json(&o, &hash, &sig))
}

#[ic_cdk::query]
pub fn sale_state() -> SaleState {
    SALES.with(|s| s.borrow().clone())
}

/// Every listing signed and not yet filled or delisted: `(sale, ticket, inscription, ask_sats, listing PSBT hex)`, for whoever
/// posts it to a venue.
#[ic_cdk::query]
pub fn sale_listings() -> Vec<(u64, u64, String, u64, String)> {
    SALES.with(|s| s.borrow().sales.iter().filter_map(|x| x.listing().map(|(i, _, ask, bytes)| (x.id, x.ticket.unwrap_or(0), i.to_string(), ask,
        hex::encode(bytes)))).collect())
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_update_method_here_is_guarded_and_the_policy_is_the_controllers() {
        let src = include_str!("salenet.rs");
        let needle = concat!("#[ic_cdk::", "update]");
        assert_eq!(src.matches(needle).count(), 3, "every update is counted here");
        for part in src.split(needle).skip(1) {
            let head: String = part.chars().take(500).collect();
            assert!(head.contains("only_operator();") || head.contains("only_controller();"), "unguarded: {head}");
        }
        assert!(src.split("pub fn set_sale_policy").nth(1).unwrap().trim_start().contains("only_controller();"));
        for op in ["pub fn sale_delist", "pub async fn sale_nft_listing"] {
            let body: String = src.split(op).nth(1).unwrap().chars().take(200).collect();
            assert!(body.contains("only_operator();") && body.contains(&format!("{}_paused()?;", "not")), "{op}: the operator's, and refused while paused (H1)");
        }
    }
}
