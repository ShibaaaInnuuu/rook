//! A collection's buy queue (the design since 2026-09-23): coins graduate at an ETH target and their item budget is credited,
//! first in first out, to the collection's queue; the canister buys ONE item whenever the queue holds at least the item cap
//! plus the buffer, confirms it into custody, mints it to the queue, and reports it; the contract splits the unit pro rata to
//! the credits that paid for it.
//!
//! **The queue contract was not on this branch when this was written.** Everything below is against the interface the
//! canister ASSUMES, stated once here and in docs/v3/curve.md ("The buy queue: the interface the canister assumes"); swapping
//! in the real one is a change to this file alone — the rules are `itemsink.rs`'s and do not know which contract they drive.
//!
//! ```solidity
//! interface IBuyQueue {                          // ASSUMED
//!     function wrapper() external view returns (address);      // the Wrapper whose custodian is this canister
//!     function payee() external view returns (address);        // where recordItem reimburses: the canister's EVM address
//!     function queued() external view returns (uint256);       // wei credited and not yet spent or refunded
//!     function itemCap() external view returns (uint256);      // the most one item may cost now, wei; 0 = none posted
//!     function buffer() external view returns (uint256);       // held beyond the cap before an item is bought, wei
//!     function itemsRecorded() external view returns (uint256);
//!     function headDeadline() external view returns (uint64 at, uint64 atBlock); // the oldest credit's; (0, 0) when empty
//!     function recordItem(uint256 cost) external;              // custodian only, after wrapDeposit(…, to = this):
//!                                                              // balance ≥ (itemsRecorded + 1) units, cost ≤ itemCap,
//!                                                              // cost ≤ queued; pays `cost` to payee; splits the unit FIFO
//!     function refundExpired() external;                       // anyone, head deadline past on both clocks
//! }
//! ```

use crate::abi::{self, Arg};
use crate::itemsink::{both_past, Clock, SinkKind, SinkView};

pub const QUEUED: [u8; 4] = [0x16, 0x04, 0x9d, 0xdf];
pub const ITEM_CAP: [u8; 4] = [0xe2, 0x75, 0x67, 0xf6];
pub const BUFFER: [u8; 4] = [0xed, 0xaa, 0xfe, 0x20];
pub const ITEMS_RECORDED: [u8; 4] = [0x77, 0x20, 0x46, 0xdf];
pub const HEAD_DEADLINE: [u8; 4] = [0xf6, 0xce, 0x24, 0x58];
pub const RECORD_ITEM: [u8; 4] = [0x15, 0x0f, 0xd1, 0xdf];
pub const REFUND_EXPIRED: [u8; 4] = [0xfd, 0x06, 0x61, 0xc4];

pub fn record_item(cost_wei: u128) -> Vec<u8> {
    abi::encode_call(RECORD_ITEM, &[Arg::Word(abi::uint_word(cost_wei))])
}

/// The reads, in the order [`view`] takes their answers.
pub fn reads() -> Vec<Vec<u8>> {
    [QUEUED, ITEM_CAP, BUFFER, ITEMS_RECORDED, HEAD_DEADLINE].iter().map(|s| s.to_vec()).collect()
}

/// A queue, read at one pin, as the item rules see it: open once a cap is posted; wants one item per `itemCap + buffer` it
/// holds; its per-item maximum is the cap; the oldest credit's deadline is the deadline (a credit refunded mid-purchase
/// would take the money the item was bought against); refunds are driven when that deadline is past on both clocks.
pub fn view(r: &[Vec<u8>], clock: Clock) -> Result<SinkView, String> {
    if r.len() != 5 {
        return Err("a queue is five reads".into());
    }
    let queued = abi::uint_at(&r[0], 0, 0, 128)?;
    let cap = abi::uint_at(&r[1], 0, 0, 128)?;
    let buffer = abi::uint_at(&r[2], 0, 0, 128)?;
    let recorded = abi::uint_at(&r[3], 0, 0, 64)? as u64;
    let at = abi::uint_at(&r[4], 0, 0, 64)? as u64;
    let at_block = abi::uint_at(&r[4], 0, 1, 64)? as u64;
    let deadline = if at == 0 && at_block == 0 { None } else { Some((at, at_block)) };
    let per = cap.saturating_add(buffer);
    let want = if cap == 0 || per == 0 { 0 } else { (queued / per) as u64 };
    Ok(SinkView {
        kind: SinkKind::Queue,
        status: if cap == 0 { "no cap posted".into() } else { format!("{queued} wei queued, cap {cap} + buffer {buffer}") },
        open: cap > 0,
        done: false,
        recorded,
        target: None,
        want,
        budget_left: queued,
        item_cap: Some(cap),
        deadline,
        clock,
        drive_refunds: deadline.is_some_and(|(a, b)| both_past(a, b, &clock)),
        drive_settle: false,
        price_expiry: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::itemsink::{may_mint, may_start, per_item_cap};

    const E: u128 = 1_000_000_000_000_000_000;
    const T0: u64 = 1_800_000_000;
    const B0: u64 = 26_000_000;

    fn w(v: u128) -> Vec<u8> {
        abi::uint_word(v).to_vec()
    }
    fn reads_at(queued: u128, cap: u128, buffer: u128, head: (u64, u64)) -> Vec<Vec<u8>> {
        vec![w(queued), w(cap), w(buffer), w(3), [abi::uint_word(head.0 as u128), abi::uint_word(head.1 as u128)].concat()]
    }
    fn at(s: u64) -> Clock {
        Clock { time: T0 + s, parent_block: B0 + s / 12 }
    }

    #[test]
    fn the_assumed_selectors_are_the_signatures_keccak() {
        for (sel, s) in [(QUEUED, "queued()"), (ITEM_CAP, "itemCap()"), (BUFFER, "buffer()"), (ITEMS_RECORDED, "itemsRecorded()"),
            (HEAD_DEADLINE, "headDeadline()"), (RECORD_ITEM, "recordItem(uint256)"), (REFUND_EXPIRED, "refundExpired()")] {
            assert_eq!(crate::evmtx::keccak(s.as_bytes())[..4], sel, "{s}");
        }
        assert_eq!(hex::encode(record_item(9)), format!("150fd1df{:064x}", 9));
    }

    #[test]
    fn one_item_is_bought_when_the_queue_holds_the_cap_plus_the_buffer() {
        let cap = 640 * E / 1_000;
        let buf = 30 * E / 1_000;
        // just short: nothing to buy
        let v = view(&reads_at(cap + buf - 1, cap, buf, (T0 + 86_400, B0 + 7_200)), at(0)).unwrap();
        assert_eq!(v.want, 0);
        assert!(may_start(&v, 0).is_err());
        // funded: one item, at most the cap
        let v = view(&reads_at(cap + buf, cap, buf, (T0 + 86_400, B0 + 7_200)), at(0)).unwrap();
        assert_eq!((v.want, per_item_cap(&v, 0, 0)), (1, Some(cap)));
        assert!(may_start(&v, 0).is_ok());
        assert!(may_start(&v, 1).is_err(), "one on the way covers it");
        assert!(may_mint(&v, cap + 1).is_err());
        // no cap posted: closed
        let v = view(&reads_at(10 * E, 0, buf, (0, 0)), at(0)).unwrap();
        assert!(!v.open && v.want == 0);
    }

    #[test]
    fn the_oldest_credit_is_the_deadline_and_its_expiry_on_both_clocks_drives_the_refund() {
        let cap = E / 2;
        let head = (T0 + 86_400, B0 + 7_200);
        assert!(!view(&reads_at(E, cap, 0, head), at(86_399)).unwrap().drive_refunds);
        assert!(view(&reads_at(E, cap, 0, head), at(86_400)).unwrap().drive_refunds);
        let mut c = at(90_000);
        c.parent_block = head.1 - 1;
        assert!(!view(&reads_at(E, cap, 0, head), c).unwrap().drive_refunds, "both clocks, or nothing");
        let v = view(&reads_at(E, cap, 0, (0, 0)), at(0)).unwrap();
        assert!(v.deadline.is_none() && !v.drive_refunds, "an empty queue has no deadline");
        // a purchase is not started within a purchase's latency of the head credit's deadline
        assert!(may_start(&view(&reads_at(E, cap, 0, head), at(86_400 - 3_600)).unwrap(), 0).is_err());
    }
}
