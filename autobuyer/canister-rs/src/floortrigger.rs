//! The Floor Module's side of the item loop that is not the module's interface (docs/v5/floor-feed.md §6): which item a floor
//! sink buys (a Bitcoin inscription, or an ERC-721 on another EVM chain), the per-collection purchase limits, and the NFT wants
//! an EVM floor sink starts. Pure; kept in stable memory as the upgrade tuple's EIGHTH element.
//!
//! The trigger itself is `itemsink.rs`'s, for every sink: `floorsink::view` turns one pinned read into a `SinkView`, and the
//! same `may_start` / `per_item_cap` / `may_mint` decide for a frog and for an ape. Only what a start creates differs: a frog
//! record (frogflow.rs) or an [`NftWant`] here.

use candid::CandidType;
use serde::{Deserialize, Serialize};

use crate::itemsink::{self, SinkView};

/// No collection's purchase limit may exceed this (code, so raising it passes the timelock like every policy constant).
pub const PURCHASE_LIMIT_CEILING_WEI: u128 = 25_000_000_000_000_000_000; // 25 ETH

/// What a floor sink's items are.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum FloorItem {
    /// An inscription of the listing the canister's `Wrapper` wraps: bought, confirmed and `wrapDeposit`ed as a frog is.
    Bitcoin,
    /// An ERC-721 of `collection` on `chain_id`, bought through the NFT line (`nftnet.rs`) and wrapped by `wrapper` (an
    /// `NftWrapper` on 4663), which the module's `wrapper()` must name.
    Evm { chain_id: u64, collection: [u8; 20], wrapper: [u8; 20] },
}

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct FloorCfg {
    pub module: [u8; 20],
    pub item: FloorItem,
}

/// A collection's per-purchase maximum, by the sink it buys for (a floor module, or a queue).
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PurchaseLimit {
    pub sink: [u8; 20],
    pub max_wei: u128,
    /// docs/v9/ordinals-collections.md §7 (Solana home): the collection's own minimum purchase in lamports, under the home's 1 SOL
    /// (`frogsol::ABS_MIN_PURCHASE_LAMPORTS` ..= `frogsol::MIN_PURCHASE_LAMPORTS`); `None` (and in a state saved before it): the home's.
    #[serde(default)]
    pub min_wei: Option<u128>,
}

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum WPhase {
    /// Waiting for the relayer to hand in an order for any token of the collection, at most `cap_wei`.
    Wanted,
    /// Signed on the collection's chain (the NFT line holds the bytes); waiting for the line to settle.
    Signed { token_id: String, tx_hash: String, cost_wei: u128 },
    /// The line says `Arrived`: the NFT is ours on its chain. Its unit is minted to the module by the NFT wrapper's custodian
    /// mint (docs/v4/nft-wrapper.md §5.2, not on this branch); the claim waits for it to show at the module.
    Arrived { token_id: String, tx_hash: String, cost_wei: u128 },
    /// `itemArrived(cost)` sent and mined. Finished.
    Claimed { token_id: String, tx_hash: String, cost_wei: u128 },
    /// Nothing of ours was spent (the sink stopped wanting it, or the purchase reverted or was superseded). Finished.
    Failed { why: String },
}

#[derive(CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct NftWant {
    pub id: u64,
    pub module: [u8; 20],
    pub chain_id: u64,
    pub collection: [u8; 20],
    pub cap_wei: u128,
    pub phase: WPhase,
}

impl NftWant {
    pub fn finished(&self) -> bool {
        matches!(self.phase, WPhase::Claimed { .. } | WPhase::Failed { .. })
    }
    /// What this want commits of its module's escrow: its cap while wanted, its cost once signed.
    pub fn committed_wei(&self) -> u128 {
        match &self.phase {
            WPhase::Wanted => self.cap_wei,
            WPhase::Signed { cost_wei, .. } | WPhase::Arrived { cost_wei, .. } => *cost_wei,
            _ => 0,
        }
    }
}

/// Everything this side keeps in stable memory.
#[derive(CandidType, Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct FloorState {
    pub sinks: Vec<FloorCfg>,
    pub limits: Vec<PurchaseLimit>,
    pub wants: Vec<NftWant>,
    pub next_want: u64,
}

impl FloorState {
    pub fn item_of(&self, sink: &[u8; 20]) -> Option<&FloorItem> {
        self.sinks.iter().find(|c| c.module == *sink).map(|c| &c.item)
    }
    pub fn is_evm(&self, sink: &[u8; 20]) -> bool {
        matches!(self.item_of(sink), Some(FloorItem::Evm { .. }))
    }
    /// A floor sink's item kind is set once, from nothing, and never changed (identity, like the wrapper).
    pub fn add_sink(&mut self, module: [u8; 20], item: FloorItem) -> Result<(), String> {
        match self.item_of(&module) {
            None => {
                self.sinks.push(FloorCfg { module, item });
                Ok(())
            }
            Some(x) if *x == item => Ok(()),
            Some(x) => Err(format!("refused: 0x{} already buys {x:?} and that is never changed", hex::encode(module))),
        }
    }
    pub fn limit_for(&self, sink: &[u8; 20]) -> Option<u128> {
        self.limits.iter().find(|l| l.sink == *sink).map(|l| l.max_wei)
    }
    /// The collection's own minimum purchase (§7), when a controller set one.
    pub fn min_for(&self, sink: &[u8; 20]) -> Option<u128> {
        self.limits.iter().find(|l| l.sink == *sink).and_then(|l| l.min_wei)
    }
    /// [`FloorState::set_limit_in`] with the collection's own minimum (docs/v9/ordinals-collections.md §7): `min` within
    /// `[abs_min, home_min]` and at most `max`; `None` keeps the home's minimum. A controller's act (the timelock's).
    pub fn set_limit_min_in(&mut self, sink: [u8; 20], max_wei: u128, min: Option<u128>, abs_min: u128, home_min: u128, ceiling: u128,
        unit: &str) -> Result<(), String> {
        if let Some(m) = min {
            if m < abs_min || m > home_min {
                return Err(format!("refused: a minimum purchase of {m} {unit} is outside [{abs_min}, {home_min}]"));
            }
        }
        self.set_limit_in(sink, max_wei, min.unwrap_or(home_min), ceiling, unit)?;
        if let Some(l) = self.limits.iter_mut().find(|l| l.sink == sink) {
            l.min_wei = min;
        }
        Ok(())
    }
    /// docs/v5/floor-feed.md §6.3: at least the policy's minimum, at most the ceiling. A controller's act (the timelock's).
    pub fn set_limit(&mut self, sink: [u8; 20], max_wei: u128) -> Result<(), String> {
        self.set_limit_in(sink, max_wei, crate::policy::MIN_PURCHASE_WEI, PURCHASE_LIMIT_CEILING_WEI, "wei")
    }
    /// [`FloorState::set_limit`] in the home's unit (phase B2: lamports on Solana, at most Relay's SOL to BTC capacity per order).
    pub fn set_limit_in(&mut self, sink: [u8; 20], max_wei: u128, min: u128, ceiling: u128, unit: &str) -> Result<(), String> {
        if max_wei < min {
            return Err(format!("refused: {max_wei} {unit} is under the {min} minimum purchase"));
        }
        if max_wei > ceiling {
            return Err(format!("refused: {max_wei} {unit} is over the {ceiling} ceiling"));
        }
        match self.limits.iter_mut().find(|l| l.sink == sink) {
            Some(l) => l.max_wei = max_wei,
            None => self.limits.push(PurchaseLimit { sink, max_wei, min_wei: None }),
        }
        Ok(())
    }
    /// Our wants for `sink` not finished: how many, and what they commit.
    pub fn assigned(&self, sink: &[u8; 20]) -> (u64, u128) {
        self.wants.iter().filter(|w| w.module == *sink && !w.finished()).fold((0, 0), |(n, c), w| (n + 1, c + w.committed_wei()))
    }
    pub fn want_mut(&mut self, id: u64) -> Option<&mut NftWant> {
        self.wants.iter_mut().find(|w| w.id == id)
    }
    /// The same trigger as a frog's: if the sink may start another purchase, a new want at the per-item cap. At most one
    /// want waits for an order per sink.
    pub fn plan_evm(&mut self, sink: [u8; 20], v: &SinkView) -> Result<Option<u64>, String> {
        let Some(FloorItem::Evm { chain_id, collection, .. }) = self.item_of(&sink).cloned() else {
            return Err("not an EVM floor sink".into());
        };
        let (n, w) = self.assigned(&sink);
        if self.wants.iter().any(|x| x.module == sink && x.phase == WPhase::Wanted) {
            return Ok(None);
        }
        itemsink::may_start(v, n)?;
        let cap = itemsink::per_item_cap(v, n, w).ok_or("nothing is needed")?;
        if cap < crate::policy::MIN_PURCHASE_WEI {
            return Err(format!("the cap {cap} wei is under the {} minimum purchase", crate::policy::MIN_PURCHASE_WEI));
        }
        let id = self.next_want;
        self.next_want += 1;
        self.wants.push(NftWant { id, module: sink, chain_id, collection, cap_wei: cap, phase: WPhase::Wanted });
        Ok(Some(id))
    }
    /// Wanted wants whose sink no longer wants them fail, nothing spent (a 4663 reorganisation that undid mints, a stale price).
    pub fn release(&mut self, sink: &[u8; 20], v: &SinkView) -> Vec<u64> {
        let (n, _) = self.assigned(sink);
        let mut out = Vec::new();
        for w in self.wants.iter_mut().filter(|w| w.module == *sink && w.phase == WPhase::Wanted) {
            if let Err(e) = itemsink::may_start(v, n.saturating_sub(1)) {
                w.phase = WPhase::Failed { why: format!("the module no longer wants it: {e}") };
                out.push(w.id);
            }
        }
        out
    }
    /// An order for want `id` may be signed at `cost_wei`: wanted, within its cap and the collection's limits.
    pub fn may_sign(&self, id: u64, chain_id: u64, collection: &[u8; 20], cost_wei: u128) -> Result<(), String> {
        let w = self.wants.iter().find(|w| w.id == id).ok_or("no such want")?;
        if w.phase != WPhase::Wanted {
            return Err(format!("refused: want {id} is {:?}, not waiting for an order", w.phase));
        }
        if w.chain_id != chain_id || w.collection != *collection {
            return Err("refused: the order is for another chain or collection".into());
        }
        if cost_wei > w.cap_wei {
            return Err(format!("refused: {cost_wei} wei is over the want's cap of {}", w.cap_wei));
        }
        crate::frogbuy::check_limits_for(cost_wei, self.limit_for(&w.module))
    }
}

/// A claim is due: the want's item arrived, the module holds more units than items it has recorded (the unit was minted to
/// it), and the module can take the cost now.
pub fn claim_due(w: &NftWant, module_balance: u128, v: &SinkView) -> Result<u128, String> {
    let WPhase::Arrived { cost_wei, .. } = &w.phase else { return Err("not arrived".into()) };
    if module_balance < (v.recorded as u128 + 1).saturating_mul(crate::floorsink::ONE) {
        return Err(format!("the module holds {module_balance} units for {} recorded: the unit is not minted to it yet", v.recorded));
    }
    itemsink::may_mint(v, *cost_wei)?;
    Ok(*cost_wei)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::floorsink::{self, ONE};
    use crate::itemsink::Clock;

    const E: u128 = ONE;
    const T0: u64 = 1_800_000_000;
    const APE: [u8; 20] = [0xbc; 20];
    const M: [u8; 20] = [0x71; 20];

    fn view(net: u128, max_cost: u128, limit: Option<u128>, posted: u64) -> SinkView {
        let w = |v: u128| crate::abi::uint_word(v).to_vec();
        let r = vec![w(net), w(20 * E), w(E / 5), [crate::abi::uint_word(5 * E), crate::abi::uint_word(posted as u128), crate::abi::uint_word(3_600)].concat(),
            w(max_cost), w(0), w(5 * E), w(0), w(485 * E / 100), w(20 * E)];
        floorsink::view(&r, Clock { time: T0, parent_block: 1 }, limit).unwrap()
    }
    fn ape_state() -> FloorState {
        let mut s = FloorState::default();
        s.add_sink(M, FloorItem::Evm { chain_id: 1, collection: APE, wrapper: [0xa9; 20] }).unwrap();
        s
    }

    #[test]
    fn limits_are_bounded_and_per_collection() {
        let mut s = FloorState::default();
        assert!(s.set_limit(M, crate::policy::MIN_PURCHASE_WEI - 1).unwrap_err().contains("minimum"));
        assert!(s.set_limit(M, PURCHASE_LIMIT_CEILING_WEI + 1).unwrap_err().contains("ceiling"));
        s.set_limit(M, 6 * E).unwrap();
        s.set_limit(M, 7 * E).unwrap();
        assert_eq!((s.limit_for(&M), s.limit_for(&[0; 20]), s.limits.len()), (Some(7 * E), None, 1));
    }

    #[test]
    fn an_item_kind_is_set_once() {
        let mut s = ape_state();
        assert!(s.add_sink(M, FloorItem::Evm { chain_id: 1, collection: APE, wrapper: [0xa9; 20] }).is_ok(), "the same again is fine");
        assert!(s.add_sink(M, FloorItem::Bitcoin).unwrap_err().contains("never changed"));
        assert!(s.is_evm(&M) && !s.is_evm(&[1; 20]));
    }

    #[test]
    fn an_evm_sink_is_triggered_by_the_same_rules_as_a_frog_one_want_at_a_time() {
        let mut s = ape_state();
        s.set_limit(M, 6 * E).unwrap();
        // below one item + buffer: nothing
        assert!(s.plan_evm(M, &view(E, 5 * E, Some(6 * E), T0)).unwrap_err().contains("nothing to buy"));
        // stale: nothing
        assert!(s.plan_evm(M, &view(2 * E, 5 * E, Some(6 * E), T0 - 3_000)).unwrap_err().contains("stale"));
        // funded and fresh: one want at the module's maxItemCost (under the 6 ETH limit)
        let v = view(2 * E + E / 5, 5 * E, Some(6 * E), T0);
        let id = s.plan_evm(M, &v).unwrap().unwrap();
        assert_eq!(s.wants[0].cap_wei, 5 * E);
        assert_eq!(s.plan_evm(M, &v).unwrap(), None, "one want waits for an order at a time");
        // the order: within the cap and the limit, for this collection
        assert!(s.may_sign(id, 1, &APE, 5 * E).is_ok());
        assert!(s.may_sign(id, 1, &APE, 5 * E + 1).unwrap_err().contains("cap"));
        assert!(s.may_sign(id, 8453, &APE, E).unwrap_err().contains("another chain"));
        // without a limit, the policy's 0.5 ETH refuses the ape the want would allow
        let mut t = ape_state();
        let id2 = t.plan_evm(M, &view(2 * E, 5 * E, None, T0)).unwrap().unwrap();
        assert_eq!(t.wants[0].cap_wei, crate::policy::MAX_PURCHASE_WEI, "unconfigured: the global maximum");
        assert!(t.may_sign(id2, 1, &APE, crate::policy::MAX_PURCHASE_WEI).is_ok());
    }

    #[test]
    fn a_reorg_that_drops_net_demand_fails_the_wanted_want_and_the_claim_waits_for_the_unit() {
        let mut s = ape_state();
        s.set_limit(M, 6 * E).unwrap();
        let id = s.plan_evm(M, &view(E + E / 5, 5 * E, Some(6 * E), T0)).unwrap().unwrap();
        assert_eq!(s.release(&M, &view(E + E / 5, 5 * E, Some(6 * E), T0)), Vec::<u64>::new());
        assert_eq!(s.release(&M, &view(E, 5 * E, Some(6 * E), T0)), vec![id], "net fell under one item + buffer");
        assert!(s.wants[0].finished());
        // an arrived ape is claimed once the module holds the unit
        let id = s.plan_evm(M, &view(E + E / 5, 5 * E, Some(6 * E), T0)).unwrap().unwrap();
        s.want_mut(id).unwrap().phase = WPhase::Arrived { token_id: "9".into(), tx_hash: "0xab".into(), cost_wei: 5 * E };
        let v = view(E + E / 5, 5 * E, Some(6 * E), T0 - 7_200);
        let w = s.wants.iter().find(|w| w.id == id).unwrap().clone();
        assert!(claim_due(&w, 0, &v).unwrap_err().contains("not minted"));
        assert_eq!(claim_due(&w, E, &v), Ok(5 * E), "on a stale price too: the item was bought against a fresh one");
        assert!(claim_due(&w, E, &view(E, 4 * E, Some(6 * E), T0)).is_err(), "the module cannot take 5 ETH now");
    }
}
