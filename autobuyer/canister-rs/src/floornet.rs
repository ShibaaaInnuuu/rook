//! The Floor Module's endpoints and state (docs/v5/floor-feed.md §6): configuring a floor sink and a collection's purchase
//! limit (a controller's act — after the handover, the timelock's, as `set_item_config` is), and the relayer's two steps for an
//! EVM item (hand in an order for a want; report that the line says it arrived). The decisions are `floortrigger.rs`'s and
//! `itemsink.rs`'s; the buying is `nftnet.rs`'s, unchanged; the sequencing is `frogflow.rs`'s.

use std::cell::RefCell;

use crate::canister::only_operator;
use crate::chains::Outcome;
use crate::floortrigger::{FloorItem, FloorState, WPhase};
use crate::itemsink::{SinkKind, SinkRec};

thread_local! {
    /// Saved across upgrades as the upgrade tuple's EIGHTH element.
    pub(crate) static FLOORS: RefCell<FloorState> = RefCell::new(FloorState::default());
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

/// A Floor Module this canister buys for, and what it buys: Bitcoin inscriptions of our wrapper's listing (no chain), or the
/// ERC-721 `collection` on `chain_id`, wrapped by `nft_wrapper` on 4663. Added once and never removed or changed.
#[ic_cdk::update]
pub fn set_floor_sink(module: String, chain_id: Option<u64>, collection: Option<String>, nft_wrapper: Option<String>) -> Result<String, String> {
    only_controller();
    crate::canister::not_ticking()?;
    let solana = crate::canister::cfg().home.is_some();
    // phase B2: on the Solana home the frog listing's rook_core Module (base58), read as the wrapper's own module at its first read; an
    // Ethereum collection's module is the Ethereum NFT side's (`eth_nft`), never a floor sink here
    let m = if solana {
        if chain_id.is_some() || collection.is_some() || nft_wrapper.is_some() {
            return Err("refused: on the Solana home a floor sink buys the frog listing's inscriptions only (Ethereum NFTs are eth_nft's)".into());
        }
        crate::frogsol::module_key(&crate::soltx::pubkey(module.trim())?)
    } else {
        crate::evmtx::parse_address(module.trim())?
    };
    let item = match (chain_id, collection, nft_wrapper) {
        (None, None, None) => FloorItem::Bitcoin,
        (Some(c), Some(coll), Some(w)) => {
            if !crate::canister::cfg().evm_chains.iter().flatten().any(|x| x.chain_id == c) {
                return Err(format!("refused: chain {c} is not one of this canister's EVM chains"));
            }
            FloorItem::Evm { chain_id: c, collection: crate::evmtx::parse_address(coll.trim())?, wrapper: crate::evmtx::parse_address(w.trim())? }
        }
        _ => return Err("refused: an EVM floor sink names its chain, its collection and its NFT wrapper; a Bitcoin one names none".into()),
    };
    FLOORS.with(|f| f.borrow_mut().add_sink(m, item.clone()))?;
    crate::frognet::FROGS.with(|f| {
        let mut st = f.borrow_mut();
        if !st.sinks.iter().any(|s| s.sink == m) {
            st.sinks.push(SinkRec { kind: SinkKind::Floor, sink: m, coin: [0; 20], view: None, refused: None, drove_refunds_at: None,
                drove_settle: false });
        }
    });
    Ok(format!("Floor {} buys {item:?}", h(&m)))
}

/// A collection's per-purchase maximum, by the sink it buys for (a Floor Module or a queue): at least the policy's minimum, at
/// most `PURCHASE_LIMIT_CEILING_WEI`. Without one, the policy's global `MAX_PURCHASE_WEI` applies, as before.
///
/// docs/v9/ordinals-collections.md §7, the Solana home only: `min`, the collection's own minimum purchase in lamports (its
/// `minPurchaseSol` × 10^9), within [`frogsol::ABS_MIN_PURCHASE_LAMPORTS`] (0.15 SOL, the least Relay was measured filling) and the
/// home's 1 SOL, at most `max`; `null` keeps 1 SOL. Refused on the EVM home.
#[ic_cdk::update]
pub fn set_purchase_limit(sink: String, max_wei: candid::Nat, min: Option<candid::Nat>) -> Result<String, String> {
    only_controller();
    crate::canister::not_ticking()?;
    // phase B2: on the Solana home the sink is the frog module (base58) and the limit is in LAMPORTS, at most Relay's SOL to BTC
    // capacity per order (`frogsol::RELAY_SOL_BTC_MAX_LAMPORTS`)
    let solana = crate::canister::cfg().home.is_some();
    let s = if solana { crate::frogsol::module_key(&crate::soltx::pubkey(sink.trim())?) } else { crate::evmtx::parse_address(sink.trim())? };
    let max: u128 = max_wei.0.to_string().parse().map_err(|_| "the limit does not fit in 128 bits")?;
    let known = crate::frognet::FROGS.with(|f| f.borrow().sinks.iter().any(|x| x.sink == s));
    if !known {
        return Err(format!("refused: {} is not a sink this canister serves", h(&s)));
    }
    let least: Option<u128> = min.map(|m| m.0.to_string().parse().map_err(|_| "the minimum does not fit in 128 bits".to_string())).transpose()?;
    if least.is_some() && !solana {
        return Err("refused: a collection's own minimum purchase is the Solana home's (docs/v9/ordinals-collections.md §7)".into());
    }
    let (home_min, _, ceiling, unit) = crate::frogsol::bounds(solana);
    FLOORS.with(|f| f.borrow_mut().set_limit_min_in(s, max, least, crate::frogsol::ABS_MIN_PURCHASE_LAMPORTS, home_min, ceiling, unit))?;
    let at_least = least.map(|m| format!(", at least {m} {unit}")).unwrap_or_default();
    crate::canister::note(format!("purchase limit: {} at most {max} {unit}{at_least}", h(&s)));
    Ok(format!("{} buys at most {max} {unit} an item{at_least}", h(&s)))
}

#[ic_cdk::query]
pub fn floor_state() -> FloorState {
    FLOORS.with(|f| f.borrow().clone())
}

/// Hand in a Seaport order for an EVM want: checked (`nft_preview`, every rule of docs/v4/eth-nft-canister.md §6 and §12)
/// against a target recorded at the want's cap, then its cost against the want and the collection's limits, and only then
/// signed and saved on the chain's line (`nft_sign`). `nft_send` / `nft_status` are the existing endpoints.
#[ic_cdk::update]
pub async fn floor_nft_sign(want_id: u64, token_id: String, order_hex: String) -> Result<String, String> {
    only_operator();
    crate::canister::not_paused()?;
    let _no_tick = crate::canister::hold_tick()?; // L-4: it writes FLOORS after awaits, so no tick may hold a copy meanwhile
    let w = FLOORS.with(|f| f.borrow().wants.iter().find(|w| w.id == want_id).cloned()).ok_or("no such want")?;
    if w.phase != WPhase::Wanted {
        return Err(format!("refused: want {want_id} is {:?}", w.phase));
    }
    let coll = h(&w.collection);
    crate::nftnet::nft_set_target(w.chain_id, coll.clone(), token_id.clone(), candid::Nat::from(w.cap_wei))?;
    let preview = crate::nftnet::nft_preview(w.chain_id, coll.clone(), token_id.clone(), order_hex.clone()).await?;
    let cost: u128 = preview.total_wei.parse().map_err(|_| "an unreadable total")?;
    FLOORS.with(|f| f.borrow().may_sign(want_id, w.chain_id, &w.collection, cost))?;
    let plan = crate::nftnet::nft_sign(w.chain_id, coll, token_id.clone(), order_hex).await?;
    let cost: u128 = plan.total_wei.parse().map_err(|_| "an unreadable total")?;
    let tx_hash = plan.tx_hash.clone().ok_or("signed without a hash")?;
    FLOORS.with(|f| -> Result<(), String> {
        let mut f = f.borrow_mut();
        f.may_sign(want_id, w.chain_id, &w.collection, cost)?;
        f.want_mut(want_id).ok_or("no such want")?.phase = WPhase::Signed { token_id, tx_hash: tx_hash.clone(), cost_wei: cost };
        Ok(())
    })?;
    Ok(format!("want {want_id}: signed {tx_hash} for {cost} wei; nft_send and nft_status settle it"))
}

/// What the chain's line says about a signed want: arrived (the NFT is ours at a pin), or reverted / superseded (nothing of
/// ours was spent but gas). Read from the line, never taken from the caller.
#[ic_cdk::update]
pub fn floor_nft_arrived(want_id: u64) -> Result<String, String> {
    only_operator();
    crate::canister::not_paused()?;
    crate::canister::not_ticking()?;
    let w = FLOORS.with(|f| f.borrow().wants.iter().find(|w| w.id == want_id).cloned()).ok_or("no such want")?;
    let WPhase::Signed { token_id, tx_hash, cost_wei } = w.phase.clone() else { return Err(format!("refused: want {want_id} is {:?}", w.phase)) };
    let line = crate::nftnet::NFT.with(|n| n.borrow().line(w.chain_id));
    let Some((_, outcome)) = line.done.iter().find(|(f, _)| f.tx_hash == tx_hash) else {
        return Ok(format!("want {want_id}: {tx_hash} is not settled on chain {} yet (nft_status)", w.chain_id));
    };
    let phase = match outcome {
        Outcome::Arrived { .. } => WPhase::Arrived { token_id, tx_hash, cost_wei },
        o => WPhase::Failed { why: format!("the purchase ended {o:?}") },
    };
    let msg = format!("want {want_id}: {phase:?}");
    FLOORS.with(|f| if let Some(x) = f.borrow_mut().want_mut(want_id) { x.phase = phase; });
    Ok(msg)
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_update_method_here_is_guarded() {
        let src = include_str!("floornet.rs");
        for kind in ["\npub fn", "\npub async fn"] {
            for part in src.split(&format!("{}{kind}", concat!("#[ic_cdk::", "update]"))).skip(1) {
                let head: String = part.chars().take(400).collect();
                assert!(head.contains("only_operator();") || head.contains("only_controller();"), "unguarded: {head}");
            }
        }
        assert_eq!(src.matches(concat!("#[ic_cdk::", "update]")).count(), 4, "every update is counted above");
        // configuration is the controller's (the timelock's), the relayer's steps the operator's
        let cfg = src.split("pub fn set_purchase_limit").next().unwrap();
        assert!(cfg.contains("pub fn set_floor_sink") && src.split("pub fn set_floor_sink").nth(1).unwrap().trim_start().contains("only_controller();"));
        assert!(src.split("pub fn set_purchase_limit").nth(1).unwrap().contains("only_controller();"));
    }
}
