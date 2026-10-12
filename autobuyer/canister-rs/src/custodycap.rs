//! Custody caps: how much the canister may hold, per collection and in all, before it refuses to take in more
//! (docs/v5/custody-hardening.md, H2; docs/v5/custody-threat-model.md, T3, T5, T7, T10).
//!
//! **Wired** (docs/v5/custody-hardening.md, H2): the policy is `Init::custody_caps` (stage 0 by default), the book is the upgrade
//! tuple's ninth element, and the hooks are in `nftnet` (target, sign, status), `frogflow` (starts, frog buys, deposits,
//! returns, the floor claim) and `redeem` (release on delivery). Still pure but for `soltx`'s base58 (the `sol:` keys of phase B2).
//!
//! Pure: no network, no keys, no clock. The one idea: **custody is a honeypot whose size is a choice.** A subnet compromise, a
//! stolen operator key, colluding providers or a bug in a signing path can each reach everything custody holds. Caps do not make
//! any of those less likely; they make the answer to "how much could we lose?" a number written in the install argument, raised
//! only by a timelocked upgrade, and never more than [`MAX_RAISE_FACTOR`] times at once.
//!
//! What counts against a cap: every item **held** (arrived, not yet delivered or returned) and every acquisition **in flight**
//! (signed or started, not yet settled), each at its value in wei: what was paid, or for a holder deposit the mark the wiring
//! passes (the collection's fresh keeper cap). An acquisition in flight counts before it lands, so two purchases racing each
//! other cannot both fit under the last slot.

use serde::{Deserialize, Serialize};

/// A raise may multiply a cap by at most this much per change. Stage to stage in docs/v5/launch-gates.md is exactly ×4.
pub const MAX_RAISE_FACTOR: u128 = 4;
/// A collection that had no cap before may start at most here (launch stage 0 of docs/v5/launch-gates.md).
pub const FIRST_MAX_ITEMS: u32 = 3;
/// 10 ETH: room for one blue-chip item (an ape is ≈ 5.1 ETH all-in, docs/v3/curve.md) at stage 0.
pub const FIRST_MAX_VALUE_WEI: u128 = 10_000_000_000_000_000_000;
/// A policy with no global cap before may start at most here (stage 0), or at the sum of its collections' stage-0 caps when that
/// is more (J7, below): the global cap never binds before the collections' own caps at stage 0.
pub const FIRST_GLOBAL_MAX_ITEMS: u32 = 6;
pub const FIRST_GLOBAL_MAX_VALUE_WEI: u128 = 10_000_000_000_000_000_000;
/// The operator's J7 (2026-09-27, docs/v7/phase1-nft-pooled.md §6): an Ethereum collection (`<chain id>:0x<collection>`) new to
/// the policy may start at 15 ETH in custody, one item up to 15 ETH, with no canister-wide total beyond the sum of the
/// collections' caps. 15 ETH holds two apes at the 6.79 floor, four Pudgys, 16 Miladys, 22 Azukis, 17 MAYC.
pub const FIRST_EVM_MAX_VALUE_WEI: u128 = 15_000_000_000_000_000_000;
/// J7: an Ethereum collection new to the policy may start at up to 24 items: what 12 deliveries a day (the most an install's
/// outflow limits may start at, ×4 stage 0's 3) let out within the timelock's 72 h less a day (`outflow::exit_fits`). The install
/// argument names each collection's count so 15 ETH fits at its floor; the canister refuses a count holders could not exit.
pub const FIRST_EVM_MAX_ITEMS: u32 = 24;
/// J7: an Ethereum collection the argument does not name (`custody_caps = null`) gets 15 ETH and 6 items: what stage 0's outflow
/// (3 deliveries a day) lets out within the delay, so the default install always passes `exit_fits`.
pub const STAGE0_EVM_ITEMS: u32 = 6;

/// The cap for one collection. `collection` is a canonical key: `btc:0x<wrapper>` for inscriptions custodied for a Bitcoin
/// wrapper, `<chain id>:0x<collection>` for ERC-721s held on another EVM chain; lower-case hex, 40 digits.
#[derive(candid::CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct CollectionCap {
    pub collection: String,
    /// Items held plus items in flight.
    pub max_items: u32,
    /// Their value, wei.
    pub max_value_wei: u128,
    /// The most one item may cost. Bounds `nft_set_target`'s cap and a frog buy's cost; a relayer never sets it.
    pub max_item_wei: u128,
}

#[derive(candid::CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
pub struct CapPolicy {
    /// Only these collections may be taken in at all: anything else is refused.
    pub collections: Vec<CollectionCap>,
    pub global_max_items: u32,
    pub global_max_value_wei: u128,
}

/// One item the canister holds or is acquiring. `id` is unique across the book: an inscription id, `<chain>:<collection>:<token
/// id>` for an NFT, or while in flight the key the wiring chose (the signed transaction's hash).
#[derive(candid::CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    pub collection: String,
    pub value_wei: u128,
    pub in_flight: bool,
}

/// What custody holds and is acquiring, as the caps see it. Bounded by the caps themselves: it can never hold more entries than
/// `global_max_items`, which keeps the saved state small (docs/v5/custody-threat-model.md, T13).
#[derive(candid::CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
pub struct Book {
    pub entries: Vec<Entry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The policy does not list this collection: custody never takes it in.
    NotListed(String),
    /// One item over the collection's per-item maximum.
    ItemOverMax { cost: u128, max: u128 },
    CollectionItems { collection: String, would_hold: u32, max: u32 },
    CollectionValue { collection: String, would_hold: u128, max: u128 },
    GlobalItems { would_hold: u32, max: u32 },
    GlobalValue { would_hold: u128, max: u128 },
    /// The id is already in the book.
    Duplicate(String),
}

/// What is left under a collection's cap, for the public page.
#[derive(candid::CandidType, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Headroom {
    pub collection: String,
    pub items_held: u32,
    pub items_in_flight: u32,
    pub value_wei: u128,
    pub items_left: u32,
    pub value_left_wei: u128,
}

fn canonical_key(k: &str) -> Result<(), String> {
    // phase B2: the inscriptions custodied for a Solana wrapper (rook_core's Wrapper PDA, base58)
    if let Some(addr) = k.strip_prefix("sol:") {
        return match crate::soltx::pubkey(addr) {
            Ok(p) if crate::soltx::b58_encode(&p) == addr => Ok(()),
            _ => Err(format!("{k:?}: a Solana collection key is sol:<the wrapper's base58 address>")),
        };
    }
    let (head, addr) = k.split_once(':').ok_or_else(|| format!("{k:?}: a collection key is <btc|chain id>:0x<40 hex>"))?;
    let chain_ok = head == "btc" || (!head.is_empty() && head.bytes().all(|b| b.is_ascii_digit()) && !head.starts_with('0'));
    let addr_ok = addr.len() == 42 && addr.starts_with("0x") && addr[2..].bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    if !chain_ok || !addr_ok {
        return Err(format!("{k:?}: a collection key is <btc|chain id>:0x<40 lower-case hex>"));
    }
    Ok(())
}

/// Everything a policy must satisfy on its own.
pub fn validate(p: &CapPolicy) -> Result<(), String> {
    for (i, c) in p.collections.iter().enumerate() {
        canonical_key(&c.collection)?;
        if p.collections[..i].iter().any(|d| d.collection == c.collection) {
            return Err(format!("{} is listed twice", c.collection));
        }
        if c.max_item_wei > c.max_value_wei {
            return Err(format!("{}: one item ({}) may not be worth more than the whole cap ({})", c.collection, c.max_item_wei, c.max_value_wei));
        }
        if c.max_items > p.global_max_items || c.max_value_wei > p.global_max_value_wei {
            return Err(format!("{}: a collection's cap may not exceed the global cap", c.collection));
        }
    }
    Ok(())
}

fn raise_ok(old: u128, new: u128, first: u128) -> bool {
    if new <= old {
        return true;
    }
    if old == 0 {
        return new <= first;
    }
    new <= old.saturating_mul(MAX_RAISE_FACTOR)
}

/// A change from the running policy to a proposed one (the upgrade argument). Lowering anything is always allowed. A raise is
/// at most ×[`MAX_RAISE_FACTOR`], and a collection or a global cap that did not exist starts no higher than stage 0. So even a
/// proposer and k guardians colluding (the timelock's stated limit) move custody's exposure up by ×4 per 72 public hours, not to
/// infinity at once.
pub fn validate_change(old: &CapPolicy, new: &CapPolicy) -> Result<(), String> {
    validate(new)?;
    let (first_items, first_value) = first_global(&new.collections);
    if !raise_ok(old.global_max_items as u128, new.global_max_items as u128, first_items) {
        return Err(format!("global items {} → {}: more than ×{MAX_RAISE_FACTOR} (or above stage 0 from nothing)", old.global_max_items, new.global_max_items));
    }
    if !raise_ok(old.global_max_value_wei, new.global_max_value_wei, first_value) {
        return Err(format!("global value {} → {}: more than ×{MAX_RAISE_FACTOR} (or above stage 0 from nothing)", old.global_max_value_wei, new.global_max_value_wei));
    }
    for c in &new.collections {
        let prev = old.collections.iter().find(|o| o.collection == c.collection);
        let (items, value, item) = prev.map_or((0, 0, 0), |o| (o.max_items as u128, o.max_value_wei, o.max_item_wei));
        let (fi, fv, fitem) = first_of(&c.collection);
        let ok = raise_ok(items, c.max_items as u128, fi as u128)
            && raise_ok(value, c.max_value_wei, fv)
            && raise_ok(item, c.max_item_wei, fitem);
        if !ok {
            return Err(format!("{}: a raise of more than ×{MAX_RAISE_FACTOR} (or above stage 0 for a new collection)", c.collection));
        }
    }
    Ok(())
}

/// The canonical key of the inscriptions custodied for a Bitcoin wrapper: `btc:0x<wrapper>`.
pub fn btc_key(wrapper: &[u8; 20]) -> String {
    format!("btc:0x{}", hex::encode(wrapper))
}

/// The canonical key of the inscriptions custodied for a SOLANA wrapper (phase B2; rook_core's Inscriptions `Wrapper` PDA):
/// `sol:<base58>`. Counted like a Bitcoin wrapper's (stage 0: 3 items, 10 ETH), values in wei.
pub fn sol_key(wrapper: &[u8; 32]) -> String {
    format!("sol:{}", crate::soltx::b58_encode(wrapper))
}

/// The canonical key of an ERC-721 held on another EVM chain: `<chain id>:0x<collection>`.
pub fn evm_key(chain_id: u64, collection: &[u8; 20]) -> String {
    format!("{chain_id}:0x{}", hex::encode(collection))
}

/// Whether a canonical key names an ERC-721 on an EVM chain (`<chain id>:0x…`), not a Bitcoin wrapper's inscriptions.
fn is_evm(key: &str) -> bool {
    !key.starts_with("btc:") && !key.starts_with("sol:")
}

/// The most a collection new to the policy may start at: (items, value, one item). J7 for an Ethereum collection.
fn first_of(key: &str) -> (u32, u128, u128) {
    if is_evm(key) { (FIRST_EVM_MAX_ITEMS, FIRST_EVM_MAX_VALUE_WEI, FIRST_EVM_MAX_VALUE_WEI) } else { (FIRST_MAX_ITEMS, FIRST_MAX_VALUE_WEI, FIRST_MAX_VALUE_WEI) }
}

/// The most a global cap may start at, from nothing: stage 0's, or the sum of the collections' own caps when that is more, so the
/// global cap never binds before theirs (J7: no canister-wide total at stage 0).
fn first_global(collections: &[CollectionCap]) -> (u128, u128) {
    let items = collections.iter().fold(0u128, |a, c| a.saturating_add(c.max_items as u128));
    let value = collections.iter().fold(0u128, |a, c| a.saturating_add(c.max_value_wei));
    (items.max(FIRST_GLOBAL_MAX_ITEMS as u128), value.max(FIRST_GLOBAL_MAX_VALUE_WEI))
}

/// Launch stage 0 (docs/v5/launch-gates.md, G7): each Bitcoin wrapper's inscriptions at 3 items and 10 ETH (one item up to 10 ETH);
/// each Ethereum collection at 6 items and 15 ETH, one item up to 15 ETH (J7); in all, 6 items and 10 ETH or the sum of the
/// collections' caps when that is more, so the global cap never binds (J7). What an install gets when its argument names no caps;
/// every later raise is ×4 at most.
pub fn stage0(collections: &[String]) -> CapPolicy {
    let collections: Vec<CollectionCap> = collections.iter().map(|c| {
        let (items, value, item) = if is_evm(c) { (STAGE0_EVM_ITEMS, FIRST_EVM_MAX_VALUE_WEI, FIRST_EVM_MAX_VALUE_WEI) } else { first_of(c) };
        CollectionCap { collection: c.clone(), max_items: items, max_value_wei: value, max_item_wei: item }
    }).collect();
    let (gi, gv) = first_global(&collections);
    CapPolicy { global_max_items: gi.min(u32::MAX as u128) as u32, global_max_value_wei: gv, collections }
}

impl CapPolicy {
    pub fn cap(&self, collection: &str) -> Option<&CollectionCap> {
        self.collections.iter().find(|c| c.collection == collection)
    }
}

impl Book {
    fn totals<'a>(&'a self, pred: impl Fn(&Entry) -> bool + 'a) -> (u32, u128) {
        self.entries.iter().filter(|e| pred(e)).fold((0u32, 0u128), |(n, v), e| (n + 1, v.saturating_add(e.value_wei)))
    }

    pub fn contains(&self, id: &str) -> bool {
        self.entries.iter().any(|e| e.id == id)
    }

    pub fn headroom(&self, p: &CapPolicy) -> Vec<Headroom> {
        p.collections.iter().map(|c| {
            let (held, _) = self.totals(|e| e.collection == c.collection && !e.in_flight);
            let (flight, _) = self.totals(|e| e.collection == c.collection && e.in_flight);
            let (n, v) = self.totals(|e| e.collection == c.collection);
            Headroom {
                collection: c.collection.clone(), items_held: held, items_in_flight: flight, value_wei: v,
                items_left: c.max_items.saturating_sub(n), value_left_wei: c.max_value_wei.saturating_sub(v),
            }
        }).collect()
    }
}

/// May one more item of `collection`, costing `cost_wei`, be taken in? Called before a purchase is signed or started, and before
/// a holder deposit is minted (with the mark as its cost). Refuses by default: an unlisted collection, a zero cap.
pub fn check_acquire(p: &CapPolicy, b: &Book, collection: &str, cost_wei: u128) -> Result<(), Refusal> {
    let c = p.cap(collection).ok_or_else(|| Refusal::NotListed(collection.to_string()))?;
    if cost_wei > c.max_item_wei {
        return Err(Refusal::ItemOverMax { cost: cost_wei, max: c.max_item_wei });
    }
    // a sum that would overflow is over every cap: checked, never saturated into something that fits
    let over = |held: u128, add: u128, max: u128| held.checked_add(add).is_none_or(|x| x > max);
    let (n, v) = b.totals(|e| e.collection == collection);
    if over(n as u128, 1, c.max_items as u128) {
        return Err(Refusal::CollectionItems { collection: collection.into(), would_hold: n.saturating_add(1), max: c.max_items });
    }
    if over(v, cost_wei, c.max_value_wei) {
        return Err(Refusal::CollectionValue { collection: collection.into(), would_hold: v.saturating_add(cost_wei), max: c.max_value_wei });
    }
    let (gn, gv) = b.totals(|_| true);
    if over(gn as u128, 1, p.global_max_items as u128) {
        return Err(Refusal::GlobalItems { would_hold: gn.saturating_add(1), max: p.global_max_items });
    }
    if over(gv, cost_wei, p.global_max_value_wei) {
        return Err(Refusal::GlobalValue { would_hold: gv.saturating_add(cost_wei), max: p.global_max_value_wei });
    }
    Ok(())
}

/// The cap a relayer asks to record for an NFT target (`nft_set_target`): the collection must be listed and the cap within its
/// per-item maximum. This is what turns "the cap was recorded before the order" into "the cap was bounded by governance".
pub fn check_target_cap(p: &CapPolicy, collection: &str, cap_wei: u128) -> Result<(), Refusal> {
    let c = p.cap(collection).ok_or_else(|| Refusal::NotListed(collection.to_string()))?;
    if cap_wei > c.max_item_wei {
        return Err(Refusal::ItemOverMax { cost: cap_wei, max: c.max_item_wei });
    }
    Ok(())
}

/// Check and record one acquisition in flight. Idempotent in effect: a second reserve of the same id is refused, never counted
/// twice.
pub fn reserve(p: &CapPolicy, b: &mut Book, id: &str, collection: &str, cost_wei: u128) -> Result<(), Refusal> {
    if b.contains(id) {
        return Err(Refusal::Duplicate(id.to_string()));
    }
    check_acquire(p, b, collection, cost_wei)?;
    b.entries.push(Entry { id: id.into(), collection: collection.into(), value_wei: cost_wei, in_flight: true });
    Ok(())
}

/// Settle an acquisition in flight: `Some(held_id)` when the item arrived (it is then held under its own id, e.g. the token
/// id, not the transaction's), `None` when it did not (reverted, superseded, refused): the reservation is released.
pub fn settle(b: &mut Book, flight_id: &str, held_id: Option<&str>) -> Result<(), String> {
    let i = b.entries.iter().position(|e| e.id == flight_id && e.in_flight).ok_or_else(|| format!("{flight_id} is not in flight"))?;
    match held_id {
        None => {
            b.entries.remove(i);
        }
        Some(h) => {
            if h != flight_id && b.contains(h) {
                return Err(format!("{h} is already held"));
            }
            let e = &mut b.entries[i];
            e.id = h.into();
            e.in_flight = false;
        }
    }
    Ok(())
}

/// Audit C2-01: a purchase's item arrived in custody. Its reservation in flight becomes the held item ([`settle`]); with none in flight
/// (it was released while the purchase was still on its way) the item is booked held all the same, uncapped (it is in custody already:
/// refusing to count it would only make the book undercount custody), and the answer says so (`Ok(Some(..))`, loud). `Err` only when
/// the item is already held.
pub fn hold_arrived(b: &mut Book, flight_id: &str, held_id: &str, collection: &str, value_wei: u128) -> Result<Option<String>, String> {
    if b.entries.iter().any(|e| e.id == flight_id && e.in_flight) {
        return settle(b, flight_id, Some(held_id)).map(|_| None);
    }
    if b.contains(held_id) {
        return Err(format!("{held_id} is already held"));
    }
    b.entries.push(Entry { id: held_id.into(), collection: collection.into(), value_wei, in_flight: false });
    Ok(Some(format!("{flight_id} was not in flight: {held_id} booked held under {collection} at its arrival")))
}

/// Record an item that entered custody without a purchase (a holder deposit, once minted). Checked like an acquisition.
pub fn admit(p: &CapPolicy, b: &mut Book, id: &str, collection: &str, mark_wei: u128) -> Result<(), Refusal> {
    reserve(p, b, id, collection, mark_wei)?;
    settle(b, id, Some(id)).map_err(|_| Refusal::Duplicate(id.into()))
}

/// An item left custody (delivered, returned, burned): its room under the cap is free again.
pub fn release(b: &mut Book, held_id: &str) -> Result<Entry, String> {
    let i = b.entries.iter().position(|e| e.id == held_id && !e.in_flight).ok_or_else(|| format!("{held_id} is not held"))?;
    Ok(b.entries.remove(i))
}

#[cfg(test)]
mod tests {
    use super::*;

    const E: u128 = 1_000_000_000_000_000_000;
    const FROGS: &str = "btc:0x1111111111111111111111111111111111111111";
    const APES: &str = "1:0xbc4ca0eda7647a8ab7c2061c2e118a18a936f13d";

    fn stage1() -> CapPolicy {
        CapPolicy {
            collections: vec![
                CollectionCap { collection: FROGS.into(), max_items: 12, max_value_wei: 10 * E, max_item_wei: E },
                CollectionCap { collection: APES.into(), max_items: 3, max_value_wei: 18 * E, max_item_wei: 6 * E },
            ],
            global_max_items: 24,
            global_max_value_wei: 20 * E,
        }
    }

    #[test]
    fn a_policy_is_refused_when_its_parts_do_not_fit_together() {
        assert!(validate(&stage1()).is_ok());
        let mut p = stage1();
        p.collections[0].collection = "btc:0x11".into();
        assert!(validate(&p).unwrap_err().contains("40 lower-case hex"));
        let mut p = stage1();
        p.collections[1].collection = APES.to_uppercase().replace("0X", "0x");
        assert!(validate(&p).is_err(), "upper-case hex is not canonical: two spellings would be two caps");
        let mut p = stage1();
        p.collections.push(p.collections[0].clone());
        assert!(validate(&p).unwrap_err().contains("twice"));
        let mut p = stage1();
        p.collections[0].max_item_wei = 11 * E;
        assert!(validate(&p).unwrap_err().contains("more than the whole cap"));
        let mut p = stage1();
        p.collections[1].max_value_wei = 21 * E;
        assert!(validate(&p).unwrap_err().contains("global"));
        let mut p = stage1();
        p.collections[0].collection = "01:0x1111111111111111111111111111111111111111".into();
        assert!(validate(&p).is_err(), "a chain id with a leading zero is a second spelling");
    }

    #[test]
    fn an_unlisted_collection_is_never_taken_in() {
        let b = Book::default();
        let other = "8453:0x2222222222222222222222222222222222222222";
        assert_eq!(check_acquire(&stage1(), &b, other, 1), Err(Refusal::NotListed(other.into())));
        assert_eq!(check_target_cap(&stage1(), other, 1), Err(Refusal::NotListed(other.into())));
        assert_eq!(check_acquire(&CapPolicy::default(), &b, FROGS, 1), Err(Refusal::NotListed(FROGS.into())), "the empty policy refuses everything");
    }

    #[test]
    fn the_operators_target_cap_is_bounded_by_the_policy_not_by_the_operator() {
        // docs/v5/custody-threat-model.md, T5: nft_set_target(c, junk, token, E_c) is the drain; the policy refuses it
        assert!(check_target_cap(&stage1(), APES, 6 * E).is_ok());
        assert_eq!(check_target_cap(&stage1(), APES, 6 * E + 1), Err(Refusal::ItemOverMax { cost: 6 * E + 1, max: 6 * E }));
        let junk = "1:0x3333333333333333333333333333333333333333";
        assert_eq!(check_target_cap(&stage1(), junk, 1), Err(Refusal::NotListed(junk.into())));
    }

    #[test]
    fn every_cap_binds_at_its_edge_and_not_before() {
        let p = stage1();
        let mut b = Book::default();
        for i in 0..10 {
            reserve(&p, &mut b, &format!("f{i}"), FROGS, E).unwrap();
        }
        // 10 frogs worth 10 ETH: the value cap binds before the item cap (12)
        assert_eq!(check_acquire(&p, &b, FROGS, 1), Err(Refusal::CollectionValue { collection: FROGS.into(), would_hold: 10 * E + 1, max: 10 * E }));
        release_after_settle(&mut b, "f0");
        assert!(check_acquire(&p, &b, FROGS, E).is_ok(), "a delivered frog frees its room");
        assert_eq!(check_acquire(&p, &b, FROGS, E + 1), Err(Refusal::ItemOverMax { cost: E + 1, max: E }));

        // the item cap: 12 cheap frogs fit, the 13th does not
        let mut b = Book::default();
        for i in 0..12 {
            reserve(&p, &mut b, &format!("c{i}"), FROGS, E / 10).unwrap();
        }
        assert_eq!(check_acquire(&p, &b, FROGS, 1), Err(Refusal::CollectionItems { collection: FROGS.into(), would_hold: 13, max: 12 }));

        // the global value cap binds across collections: 10 ETH of frogs + 6 of apes = 16; one more ape (6) would be 22 > 20
        let mut b = Book::default();
        for i in 0..10 {
            reserve(&p, &mut b, &format!("f{i}"), FROGS, E).unwrap();
        }
        reserve(&p, &mut b, "ape1", APES, 6 * E).unwrap();
        assert_eq!(check_acquire(&p, &b, APES, 6 * E), Err(Refusal::GlobalValue { would_hold: 22 * E, max: 20 * E }));
        assert!(check_acquire(&p, &b, APES, 4 * E).is_ok());
    }

    #[test]
    fn the_global_item_cap_binds_across_collections() {
        let mut p = stage1();
        p.global_max_items = 12;
        p.collections[1].max_items = 3;
        let mut b = Book::default();
        for i in 0..11 {
            reserve(&p, &mut b, &format!("f{i}"), FROGS, 1).unwrap();
        }
        reserve(&p, &mut b, "ape", APES, 1).unwrap();
        assert_eq!(check_acquire(&p, &b, APES, 1), Err(Refusal::GlobalItems { would_hold: 13, max: 12 }));
    }

    fn release_after_settle(b: &mut Book, id: &str) {
        settle(b, id, Some(id)).unwrap();
        release(b, id).unwrap();
    }

    #[test]
    fn in_flight_counts_before_it_lands_and_a_failed_purchase_frees_its_room() {
        let mut p = stage1();
        p.collections[1].max_items = 1;
        let mut b = Book::default();
        reserve(&p, &mut b, "0xtx1", APES, 5 * E).unwrap();
        // a second purchase racing the first cannot take the same last slot
        assert_eq!(reserve(&p, &mut b, "0xtx2", APES, 5 * E), Err(Refusal::CollectionItems { collection: APES.into(), would_hold: 2, max: 1 }));
        // the first reverted: its room is free
        settle(&mut b, "0xtx1", None).unwrap();
        reserve(&p, &mut b, "0xtx2", APES, 5 * E).unwrap();
        // it arrived: held under the token's id, no longer in flight
        settle(&mut b, "0xtx2", Some("1:0xbc4c…:9820")).unwrap();
        let h = b.headroom(&p);
        assert_eq!((h[1].items_held, h[1].items_in_flight, h[1].items_left, h[1].value_wei), (1, 0, 0, 5 * E));
        // settling twice, or releasing what is in flight, is an error and changes nothing
        assert!(settle(&mut b, "0xtx2", None).is_err());
        reserve(&p, &mut b, "f", FROGS, 1).unwrap();
        assert!(release(&mut b, "f").is_err(), "an item in flight has not arrived, so it cannot leave");
        assert_eq!(b.entries.len(), 2);
    }

    #[test]
    fn an_id_is_counted_once() {
        let p = stage1();
        let mut b = Book::default();
        reserve(&p, &mut b, "x", FROGS, 1).unwrap();
        assert_eq!(reserve(&p, &mut b, "x", FROGS, 1), Err(Refusal::Duplicate("x".into())));
        admit(&p, &mut b, "i0", FROGS, E / 2).unwrap();
        assert_eq!(admit(&p, &mut b, "i0", FROGS, E / 2), Err(Refusal::Duplicate("i0".into())));
        assert_eq!(b.entries.len(), 2);
        assert!(!b.entries[1].in_flight, "a deposit is held the moment it is admitted");
        settle(&mut b, "x", Some("x")).unwrap();
        assert!(settle(&mut b, "x", Some("i0")).is_err());
    }

    #[test]
    fn a_deposit_counts_against_the_cap_like_a_purchase() {
        let mut p = stage1();
        p.collections[0].max_items = 2;
        let mut b = Book::default();
        admit(&p, &mut b, "i0", FROGS, E / 2).unwrap();
        admit(&p, &mut b, "i1", FROGS, E / 2).unwrap();
        assert!(matches!(admit(&p, &mut b, "i2", FROGS, E / 2), Err(Refusal::CollectionItems { .. })));
        assert_eq!(b.entries.len(), 2, "a refused admission leaves nothing behind");
    }

    #[test]
    fn raises_are_at_most_four_times_and_new_caps_start_at_stage_zero() {
        let s0 = CapPolicy {
            collections: vec![CollectionCap { collection: FROGS.into(), max_items: 3, max_value_wei: 5 * E, max_item_wei: E }],
            global_max_items: 6,
            global_max_value_wei: 5 * E,
        };
        assert!(validate_change(&CapPolicy::default(), &s0).is_ok(), "stage 0 from nothing");
        let mut too_big = s0.clone();
        too_big.global_max_value_wei = 10 * E + 1;
        too_big.collections[0].max_value_wei = 10 * E + 1;
        assert!(validate_change(&CapPolicy::default(), &too_big).is_err(), "nothing to above stage 0 in one step");

        let mut s1 = s0.clone();
        s1.global_max_items = 24;
        s1.global_max_value_wei = 20 * E;
        s1.collections[0] = CollectionCap { collection: FROGS.into(), max_items: 12, max_value_wei: 20 * E, max_item_wei: 4 * E };
        assert!(validate_change(&s0, &s1).is_ok(), "×4 exactly");
        let mut over = s1.clone();
        over.collections[0].max_items = 13;
        assert!(validate_change(&s0, &over).unwrap_err().contains(FROGS));
        let mut over = s1.clone();
        over.global_max_value_wei = 20 * E + 1;
        assert!(validate_change(&s0, &over).unwrap_err().contains("global value"));

        // a new collection added beside a running one starts at stage 0 (an Ethereum one at J7's: 24 items, 15 ETH)
        let mut add = s1.clone();
        add.collections.push(CollectionCap { collection: APES.into(), max_items: 3, max_value_wei: 5 * E, max_item_wei: 5 * E });
        assert!(validate_change(&s1, &add).is_ok());
        add.collections[1].max_items = 25;
        assert!(validate_change(&s1, &add).is_err());
        add.collections[1].max_items = 24;
        add.collections[1].max_value_wei = 15 * E + 1;
        assert!(validate_change(&s1, &add).is_err());
        // a new Bitcoin wrapper's inscriptions still start at 3 items and 10 ETH
        let mut addb = s1.clone();
        addb.collections.push(CollectionCap { collection: "btc:0x2222222222222222222222222222222222222222".into(), max_items: 4, max_value_wei: E, max_item_wei: E });
        assert!(validate_change(&s1, &addb).is_err());

        // lowering anything, or dropping a collection, is always allowed
        assert!(validate_change(&s1, &s0).is_ok());
        assert!(validate_change(&s1, &CapPolicy::default()).is_ok());
    }

    #[test]
    fn a_sum_that_would_overflow_is_refused_not_saturated() {
        let p = CapPolicy {
            collections: vec![CollectionCap { collection: FROGS.into(), max_items: u32::MAX, max_value_wei: u128::MAX, max_item_wei: u128::MAX }],
            global_max_items: u32::MAX,
            global_max_value_wei: u128::MAX,
        };
        let mut b = Book::default();
        reserve(&p, &mut b, "a", FROGS, u128::MAX).unwrap();
        // u128::MAX + 1 does not fit in anything: a saturating sum would have read as exactly the cap and passed
        assert!(matches!(check_acquire(&p, &b, FROGS, 1), Err(Refusal::CollectionValue { .. })));
        assert!(check_acquire(&p, &b, FROGS, 0).is_ok());
        // ×4 of a huge cap saturates at u128::MAX rather than wrapping to a small number that would refuse a lowering
        let mut lower = p.clone();
        lower.global_max_value_wei = u128::MAX - 1;
        lower.collections[0].max_value_wei = u128::MAX - 1;
        lower.collections[0].max_item_wei = u128::MAX - 1;
        assert!(validate_change(&lower, &p).is_ok());
    }

    /// J7 (2026-09-27): the Ethereum launch collections at 15 ETH each, item counts that fit 15 ETH at each floor, and a global cap
    /// that is the sum of theirs, from nothing; the global never refuses what the collections' own caps admit.
    #[test]
    fn j7_the_ethereum_collections_start_at_fifteen_eth_each_with_no_binding_total() {
        let key = |a: &str| format!("1:0x{a}");
        let cols = [("5af0d9827e0c53e4799bb226655a1de152a425a5", 16), ("ed5af388653567af2f388e6224dc7c4b3241c544", 22),
            ("60e4d786628fea6478f785a6d7e704777c86a7c6", 17), ("bd3531da5cf5857e7cfaa92426877b022e612cf8", 4),
            ("bc4ca0eda7647a8ab7c2061c2e118a18a936f13d", 2)];
        let launch = CapPolicy {
            collections: cols.iter().map(|(a, n)| CollectionCap { collection: key(a), max_items: *n, max_value_wei: 15 * E, max_item_wei: 15 * E }).collect(),
            global_max_items: 61,
            global_max_value_wei: 75 * E,
        };
        assert_eq!(validate_change(&CapPolicy::default(), &launch), Ok(()), "J7 from nothing");
        let mut over = launch.clone();
        over.global_max_value_wei = 75 * E + 1;
        assert!(validate_change(&CapPolicy::default(), &over).unwrap_err().contains("global value"), "no more than the sum from nothing");
        let mut over = launch.clone();
        over.collections[1].max_items = 25;
        over.global_max_items = 64;
        assert!(validate_change(&CapPolicy::default(), &over).is_err(), "no Ethereum collection above 24 items from nothing");
        // every collection full to its own value cap at once: the global cap admits it all
        let mut b = Book::default();
        for (a, n) in cols {
            let per = 15 * E / n as u128;
            for i in 0..n {
                reserve(&launch, &mut b, &format!("{a}:{i}"), &key(a), per).unwrap();
            }
            assert!(matches!(check_acquire(&launch, &b, &key(a), 1), Err(Refusal::CollectionItems { .. } | Refusal::CollectionValue { .. })));
        }
        assert_eq!(b.entries.len(), 61);
        // two apes at the floor: 6.79 held and 7.47 (the J1 item cap) in flight fit 15 ETH; a third does not
        let mut b = Book::default();
        let ape = key("bc4ca0eda7647a8ab7c2061c2e118a18a936f13d");
        admit(&launch, &mut b, "ape1", &ape, 6_790_000_000_000_000_000).unwrap();
        reserve(&launch, &mut b, "0xtx", &ape, 7_469_000_000_000_000_000).unwrap();
        assert!(matches!(check_acquire(&launch, &b, &ape, 1), Err(Refusal::CollectionItems { .. })));
        // the default (custody_caps = null): 6 items and 15 ETH each, and the sum in all
        let d = stage0(&cols.iter().map(|(a, _)| key(a)).collect::<Vec<_>>());
        assert!(d.collections.iter().all(|c| (c.max_items, c.max_value_wei, c.max_item_wei) == (6, 15 * E, 15 * E)));
        assert_eq!((d.global_max_items, d.global_max_value_wei), (30, 75 * E));
        assert_eq!(validate_change(&CapPolicy::default(), &d), Ok(()));
        // a Bitcoin wrapper's inscriptions alone: stage 0 as before
        let f = stage0(&[FROGS.to_string()]);
        assert_eq!((f.collections[0].max_items, f.collections[0].max_value_wei, f.global_max_items, f.global_max_value_wei), (3, 10 * E, 6, 10 * E));
    }
}
