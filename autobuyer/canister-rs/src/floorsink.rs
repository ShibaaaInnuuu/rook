//! A collection's Floor Module (docs/v5/floor-feed.md): anyone mints wX with ETH at `floor × (1 + spread)` and burns it for
//! ETH at `floor × (1 − spread)`; the canister buys ONE real item whenever the NET ETH-backed demand — units minted for ETH,
//! less units burned for ETH, less items already arrived — reaches one whole item plus the module's buffer, and the posted
//! floor is fresh. It wraps the item to the module and calls `itemArrived(cost)`, which pays the cost out of the escrow.
//!
//! **The module is on another branch (`floor-module`) and was not here when this was written.** Everything below is against
//! the interface the canister ASSUMES, stated once here and in docs/v5/floor-feed.md §5.1; swapping in the real one is a
//! change to this file alone — the rules are `itemsink.rs`'s and do not know which contract they drive.
//!
//! ```solidity
//! interface IFloorModule {                                  // ASSUMED
//!     function wrapper() external view returns (address);   // the IWrappedCollection whose custodian is this canister
//!     function payee() external view returns (address);     // where itemArrived pays: the canister's EVM address
//!     function netUnits() external view returns (uint256);  // 1e18 = one item: minted for ETH − burned for ETH − arrived; ≥ 0
//!     function escrow() external view returns (uint256);    // wei held for items and burns
//!     function buffer() external view returns (uint256);    // units (1e18 scale) beyond one item before a buy is due
//!     function floor() external view returns (uint256 priceWei, uint64 postedAt, uint64 ttl);
//!     function maxItemCost() external view returns (uint256); // the most itemArrived accepts now, wei
//!     function itemsArrived() external view returns (uint256);
//!     function itemArrived(uint256 costWei) external;       // custodian only, after the unit was minted to the module:
//!                                                           // wrapper.balanceOf(this) ≥ (itemsArrived + 1) × 1e18,
//!                                                           // cost ≤ maxItemCost(), cost ≤ escrow(); pays cost to payee
//! }
//! ```

use crate::abi::{self, Arg};
use crate::itemsink::{Clock, SinkKind, SinkView};

pub const NET_UNITS: [u8; 4] = [0x27, 0x3c, 0x58, 0x9e];
pub const ESCROW: [u8; 4] = [0xe2, 0xfd, 0xcc, 0x17];
pub const BUFFER: [u8; 4] = [0xed, 0xaa, 0xfe, 0x20];
pub const FLOOR: [u8; 4] = [0x40, 0x69, 0x53, 0x63];
pub const MAX_ITEM_COST: [u8; 4] = [0x9b, 0x04, 0xdf, 0xbf];
pub const ITEMS_ARRIVED: [u8; 4] = [0xff, 0x40, 0xb1, 0xaf];
pub const ITEM_ARRIVED: [u8; 4] = [0xd9, 0x96, 0x75, 0x9a];
/// The module as merged (docs/v5/floor-module.md §13.6; docs/v6/canister-floor-followups.md §1): the fills price, the float
/// inventory, the least a sale may report, and every wei backing ETH-backed units (`B + S`; the reserve is this less `escrow`).
pub const PRICE: [u8; 4] = [0xa0, 0x35, 0xb1, 0xfe];
pub const FLOAT_UNITS: [u8; 4] = [0x3a, 0x60, 0xec, 0x4c];
pub const MIN_SALE_PROCEEDS: [u8; 4] = [0xb3, 0x6d, 0x61, 0x4f];
pub const ESCROW_BALANCE: [u8; 4] = [0xe1, 0x1e, 0x1b, 0x0c];
/// `wrapper()` / `payee()`: the same selectors `curvebuy.rs` names, so the identity check is shared.
pub const WRAPPER: [u8; 4] = crate::curvebuy::WRAPPER;
pub const PAYEE: [u8; 4] = crate::curvebuy::PAYEE;

/// One whole item in units.
pub const ONE: u128 = 1_000_000_000_000_000_000;

pub fn item_arrived(cost_wei: u128) -> Vec<u8> {
    abi::encode_call(ITEM_ARRIVED, &[Arg::Word(abi::uint_word(cost_wei))])
}

/// The reads, in the order [`view`] takes their answers: the six of the trigger, the four of the sale side, then the unit in
/// transit (docs/v6/relay-conversion.md §3.3).
pub fn reads() -> Vec<Vec<u8>> {
    [NET_UNITS, ESCROW, BUFFER, FLOOR, MAX_ITEM_COST, ITEMS_ARRIVED, PRICE, FLOAT_UNITS, MIN_SALE_PROCEEDS, ESCROW_BALANCE,
        crate::relayconv::TRANSIT_UNITS].iter()
        .map(|s| s.to_vec()).collect()
}

/// What the module says beyond the trigger's view (docs/v6/canister-floor-followups.md §1, §3.1). `float_units` is inside
/// `netUnits` already and is never added to the demand: it says only whether the module holds inventory to sell.
#[derive(candid::CandidType, serde::Serialize, serde::Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct FloorExtra {
    /// `price()`: the fills price mints and burns use; what a protocol sale asks over.
    pub price: u128,
    /// `floatUnits()`: the module's inventory (1e18 = one item), what `itemSold` sells.
    pub float_units: u128,
    /// `minSaleProceeds()`: the least `itemSold` accepts now.
    pub min_sale_proceeds: u128,
    /// The standing reserve `S = escrowBalance() − escrow()`.
    pub reserve: u128,
}

/// Items the module's net demand pays for now: one per whole unit beyond the buffer, and none until one item plus the buffer.
pub fn wanted(net: u128, buffer: u128) -> u64 {
    if net < ONE.saturating_add(buffer) { 0 } else { ((net - buffer) / ONE) as u64 }
}

/// A module, read at one pin, as the item rules see it. `limit` is the collection's purchase limit (floortrigger.rs), or
/// `None` for the policy's global maximum — so a module nobody gave a limit behaves as today's policy does.
///
/// open once a floor has ever been posted; wants one item per whole net unit beyond the buffer; holds its escrow; its
/// per-item maximum is the smaller of what `itemArrived` accepts now and the collection's limit; its price is fresh until
/// `postedAt + ttl` (`price_expiry`, which `itemsink::may_start` reads); no deadline, no refund, no settlement.
pub fn view(r: &[Vec<u8>], clock: Clock, limit: Option<u128>) -> Result<SinkView, String> {
    view_full(r, clock, limit).map(|(v, _)| v)
}

/// [`view`], with the sale side's reads beside it.
pub fn view_full(r: &[Vec<u8>], clock: Clock, limit: Option<u128>) -> Result<(SinkView, FloorExtra), String> {
    if r.len() != 10 && r.len() != 11 {
        return Err(format!("a floor module is eleven reads (ten before the advance), not {}", r.len()));
    }
    // the unit whose budget was advanced is still ours on the way: the demand counts it (netUnits() excludes it)
    let transit = if r.len() == 11 { abi::uint_at(&r[10], 0, 0, 128)? } else { 0 };
    let extra = FloorExtra {
        price: abi::uint_at(&r[6], 0, 0, 128)?,
        float_units: abi::uint_at(&r[7], 0, 0, 128)?,
        min_sale_proceeds: abi::uint_at(&r[8], 0, 0, 128)?,
        reserve: abi::uint_at(&r[9], 0, 0, 128)?.saturating_sub(abi::uint_at(&r[1], 0, 0, 128)?),
    };
    let net = abi::uint_at(&r[0], 0, 0, 128)?;
    let escrow = abi::uint_at(&r[1], 0, 0, 128)?;
    let buffer = abi::uint_at(&r[2], 0, 0, 128)?;
    let price = abi::uint_at(&r[3], 0, 0, 128)?;
    let posted_at = abi::uint_at(&r[3], 0, 1, 64)? as u64;
    let ttl = abi::uint_at(&r[3], 0, 2, 64)? as u64;
    let max_cost = abi::uint_at(&r[4], 0, 0, 128)?;
    let arrived = abi::uint_at(&r[5], 0, 0, 64)? as u64;
    let cap = max_cost.min(limit.unwrap_or(crate::policy::MAX_PURCHASE_WEI));
    let open = price > 0;
    Ok((SinkView {
        kind: SinkKind::Floor,
        status: if !open { "no floor posted".into() } else {
            format!("net {net} units (buffer {buffer}), escrow {escrow} wei, floor {price} wei posted at {posted_at} for {ttl} s")
        },
        open,
        done: false,
        recorded: arrived,
        target: None,
        want: if open { wanted(net.saturating_add(transit), buffer) } else { 0 },
        budget_left: escrow,
        item_cap: Some(cap),
        deadline: None,
        clock,
        drive_refunds: false,
        drive_settle: false,
        price_expiry: Some(posted_at.saturating_add(ttl)),
    }, extra))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::itemsink::{may_mint, may_start, per_item_cap};

    const E: u128 = ONE;
    const T0: u64 = 1_800_000_000;

    fn w(v: u128) -> Vec<u8> {
        abi::uint_word(v).to_vec()
    }
    /// A module: `net` units, 3 ETH escrowed, a 0.2-item buffer, a 0.64 ETH floor posted at `posted` for an hour; a fills
    /// price of 0.65, one float unit, a sale minimum of 0.6208, 4 ETH backing in all (so a 1 ETH reserve).
    fn reads_at(net: u128, posted: u64, max_cost: u128) -> Vec<Vec<u8>> {
        vec![w(net), w(3 * E), w(E / 5), [abi::uint_word(640 * E / 1_000), abi::uint_word(posted as u128), abi::uint_word(3_600)].concat(),
            w(max_cost), w(2), w(650 * E / 1_000), w(E), w(6_208 * E / 10_000), w(4 * E)]
    }
    fn at(s: u64) -> Clock {
        Clock { time: T0 + s, parent_block: 26_000_000 + s / 12 }
    }

    #[test]
    fn the_assumed_selectors_are_the_signatures_keccak() {
        for (sel, s) in [(NET_UNITS, "netUnits()"), (ESCROW, "escrow()"), (BUFFER, "buffer()"), (FLOOR, "floor()"),
            (MAX_ITEM_COST, "maxItemCost()"), (ITEMS_ARRIVED, "itemsArrived()"), (ITEM_ARRIVED, "itemArrived(uint256)"),
            (WRAPPER, "wrapper()"), (PAYEE, "payee()"), (PRICE, "price()"), (FLOAT_UNITS, "floatUnits()"),
            (MIN_SALE_PROCEEDS, "minSaleProceeds()"), (ESCROW_BALANCE, "escrowBalance()")] {
            assert_eq!(crate::evmtx::keccak(s.as_bytes())[..4], sel, "{s}");
        }
        assert_eq!(hex::encode(item_arrived(9)), format!("d996759a{:064x}", 9));
    }

    #[test]
    fn net_demand_below_one_item_plus_the_buffer_buys_nothing_and_above_buys_one_per_whole_unit() {
        assert_eq!(wanted(E + E / 5 - 1, E / 5), 0, "one item and the buffer, less a wei");
        assert_eq!(wanted(E + E / 5, E / 5), 1);
        assert_eq!(wanted(2 * E + E / 5 - 1, E / 5), 1);
        assert_eq!(wanted(2 * E + E / 5, E / 5), 2);
        assert_eq!(wanted(0, 0), 0);
        let limit = Some(700 * E / 1_000);
        let v = view(&reads_at(E + E / 5 - 1, T0, 660 * E / 1_000), at(0), limit).unwrap();
        assert_eq!(v.want, 0);
        assert!(may_start(&v, 0).unwrap_err().contains("nothing to buy"));
        let v = view(&reads_at(E + E / 5, T0, 660 * E / 1_000), at(0), limit).unwrap();
        assert_eq!((v.want, per_item_cap(&v, 0, 0)), (1, Some(660 * E / 1_000)), "the module's maxItemCost binds under the limit");
        assert!(may_start(&v, 0).is_ok());
        assert!(may_start(&v, 1).is_err(), "one on the way covers it");
        assert_eq!(v.kind, SinkKind::Floor);
        assert!(!v.drive_refunds && !v.drive_settle && v.deadline.is_none());
    }

    #[test]
    fn a_stale_floor_starts_nothing_and_an_unposted_one_is_closed() {
        let cm = crate::policy::CAP_MARGIN_SECS;
        let v = view(&reads_at(3 * E, T0, E), at(3_600 - cm - 1), None).unwrap();
        assert!(may_start(&v, 0).is_ok(), "fresh for more than the margin");
        let v = view(&reads_at(3 * E, T0, E), at(3_600 - cm + 1), None).unwrap();
        assert!(may_start(&v, 0).unwrap_err().contains("stale"));
        assert!(may_mint(&v, E / 2).is_ok(), "a held item is still minted and claimed on a stale price");
        let mut r = reads_at(3 * E, T0, E);
        r[3] = [abi::uint_word(0), abi::uint_word(0), abi::uint_word(0)].concat();
        let v = view(&r, at(0), None).unwrap();
        assert!(!v.open && v.want == 0);
    }

    #[test]
    fn the_cap_is_the_collection_limit_or_the_global_maximum_whichever_the_module_allows() {
        // no limit configured: today's policy (0.5 ETH) binds, so an unconfigured module behaves as the policy does
        let v = view(&reads_at(3 * E, T0, 5 * E), at(0), None).unwrap();
        assert_eq!(v.item_cap, Some(crate::policy::MAX_PURCHASE_WEI));
        // an ape's collection limit of 6 ETH lets a 5 ETH maxItemCost through
        let v = view(&reads_at(3 * E, T0, 5 * E), at(0), Some(6 * E)).unwrap();
        assert_eq!(v.item_cap, Some(5 * E));
        assert!(may_mint(&v, 5 * E + 1).is_err());
    }

    #[test]
    fn the_sale_side_reads_beside_the_view_and_six_reads_are_no_longer_a_module() {
        let (v, x) = view_full(&reads_at(E + E / 5, T0, E), at(0), None).unwrap();
        assert_eq!(x, FloorExtra { price: 650 * E / 1_000, float_units: E, min_sale_proceeds: 6_208 * E / 10_000, reserve: E });
        assert_eq!(v.budget_left, 3 * E, "escrow is the convertible pot B alone");
        assert!(view(&reads_at(E, T0, E)[..6], at(0), None).unwrap_err().contains("eleven reads"));
        // docs/v6/relay-conversion.md §3.3: the unit in transit is still wanted (netUnits() no longer counts it)
        let mut r = reads_at(E / 5, T0, E);
        assert_eq!(view(&r, at(0), None).unwrap().want, 0);
        r.push(w(E));
        assert_eq!(view(&r, at(0), None).unwrap().want, 1, "0.2 net + 1 in transit is one item past the buffer");
        let mut r = reads_at(E, T0, E);
        r[9] = w(E); // escrowBalance under escrow cannot happen; read as no reserve rather than an underflow
        assert_eq!(view_full(&r, at(0), None).unwrap().1.reserve, 0);
    }

    /// docs/v6/canister-floor-followups.md §1: a float is inside `netUnits` already. A module reading 1.2 net units with one
    /// float unit wants ONE item, not two, and the float count changes nothing about the trigger.
    #[test]
    fn float_units_are_not_added_to_the_demand() {
        let mut r = reads_at(E + E / 5, T0, E);
        let (v, x) = view_full(&r, at(0), None).unwrap();
        assert_eq!((v.want, x.float_units), (1, E));
        r[7] = w(3 * E);
        let (v2, x2) = view_full(&r, at(0), None).unwrap();
        assert_eq!((v2.want, x2.float_units), (1, 3 * E), "three float units: still one item wanted");
        r[7] = w(0);
        assert_eq!(view_full(&r, at(0), None).unwrap().0.want, 1);
        assert_eq!(v, v2, "the trigger's view does not see the float at all");
    }
}
