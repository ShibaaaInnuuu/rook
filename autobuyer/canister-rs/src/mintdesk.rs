//! The public mint desk (docs/v3/curve.md, "Public mint on demand", and "Canister: …", "The public mint desk").
//!
//! Pure: which requests the canister takes, whether a frog may be minted to the desk for one, and whether `fulfill` may be
//! sent. The desk expires a request as soon as EITHER clock passes its `ttl`, so the nearer clock binds everything here. The
//! one money rule: **a fill is never above the request's `amount`**.

use crate::abi::{self, Arg};
use crate::itemsink::Clock;
use crate::frogbuy::{MINT_MARGIN_SECS, PARENT_BLOCK_SECONDS, PURCHASE_LATENCY_SECS};

pub const REQUEST_COUNT: [u8; 4] = [0x5b, 0xad, 0xbe, 0x4c];
pub const REQUEST_OF: [u8; 4] = [0x32, 0x4e, 0x03, 0xbe];
pub const IS_EXPIRED: [u8; 4] = [0xd9, 0x54, 0x8e, 0x53];
pub const TTL: [u8; 4] = [0x4e, 0x8b, 0x1d, 0xd5];
pub const ASSIGNED: [u8; 4] = [0xad, 0xb4, 0xd9, 0x90];
pub const FULFILL: [u8; 4] = [0xdc, 0xf4, 0xcd, 0xf1];

pub fn fulfill(id: u64, cost_wei: u128) -> Vec<u8> {
    abi::encode_call(FULFILL, &[Arg::Word(abi::uint_word(id as u128)), Arg::Word(abi::uint_word(cost_wei))])
}

/// `MintDesk.Status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub enum DStatus {
    None,
    Open,
    Filled,
    Collected,
    Expired,
}

/// `requestOf(id)`: a struct of static fields, returned inline.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub struct Request {
    pub status: DStatus,
    pub requester: [u8; 20],
    pub at: u64,
    pub at_block: u64,
    pub amount: u128,
    pub cost: u128,
}

pub fn decode_request(b: &[u8]) -> Result<Request, String> {
    let status = match abi::uint_at(b, 0, 0, 8)? {
        0 => DStatus::None,
        1 => DStatus::Open,
        2 => DStatus::Filled,
        3 => DStatus::Collected,
        4 => DStatus::Expired,
        s => return Err(format!("a request status of {s} is not the desk's")),
    };
    Ok(Request {
        status,
        requester: abi::addr_at(b, 0, 1)?,
        at: abi::uint_at(b, 0, 2, 64)? as u64,
        at_block: abi::uint_at(b, 0, 3, 64)? as u64,
        amount: abi::uint_at(b, 0, 4, 128)?,
        cost: abi::uint_at(b, 0, 5, 128)?,
    })
}

/// Seconds before the desk expires the request, on the nearer clock (`_expired` is either clock).
pub fn secs_left(r: &Request, ttl: u64, c: &Clock) -> u64 {
    crate::itemsink::secs_left(r.at + ttl, r.at_block + ttl / PARENT_BLOCK_SECONDS, c)
}

/// What the canister keeps per request it is watching.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub struct ReqRec {
    pub id: u64,
    pub request: Request,
    /// Finished for us: filled (by us), collected, expired, or never takeable.
    pub done: bool,
    pub note: String,
}

/// The request may be taken now: open, at least the policy's minimum, and more time left than the frog needs — a mint margin
/// for a frog already held, a purchase's latency on top of it otherwise.
pub fn may_take(r: &Request, ttl: u64, c: &Clock, have_frog: bool) -> Result<(), String> {
    if r.status != DStatus::Open {
        return Err(format!("the request is {:?}", r.status));
    }
    if r.amount < crate::policy::MIN_PURCHASE_WEI {
        return Err(format!("{} wei is under the {} minimum", r.amount, crate::policy::MIN_PURCHASE_WEI));
    }
    let want = if have_frog { MINT_MARGIN_SECS } else { PURCHASE_LATENCY_SECS + MINT_MARGIN_SECS + crate::policy::CAP_MARGIN_SECS };
    let left = secs_left(r, ttl, c);
    if left <= want {
        return Err(format!("{left} s to expiry on the nearer clock, not more than {want} s"));
    }
    Ok(())
}

/// A frog costing `cost_wei` may fill the request: never above its `amount`.
pub fn fill_ok(r: &Request, cost_wei: u128) -> Result<(), String> {
    if cost_wei > r.amount {
        return Err(format!("a frog costing {cost_wei} wei would fill above the request's maximum of {}", r.amount));
    }
    Ok(())
}

/// Immediately before `wrapDeposit(…, to = desk)`: open, `isExpired` false, the mint margin still there, the cost within.
pub fn may_mint(r: &Request, expired: bool, ttl: u64, c: &Clock, cost_wei: u128) -> Result<(), String> {
    if expired {
        return Err("the desk says the request has expired".into());
    }
    may_take(r, ttl, c, true)?;
    fill_ok(r, cost_wei)
}

/// Immediately before `fulfill(id, cost)`: open, not expired, within the amount. No margin: the wFROG is already at the desk,
/// and sending now is the best that can be done with it.
pub fn may_fulfill(r: &Request, expired: bool, cost_wei: u128) -> Result<(), String> {
    if r.status != DStatus::Open {
        return Err(format!("the request is {:?}", r.status));
    }
    if expired {
        return Err("the desk says the request has expired".into());
    }
    fill_ok(r, cost_wei)
}

#[cfg(test)]
mod tests {
    use super::*;

    const E: u128 = 1_000_000_000_000_000_000;
    const T0: u64 = 1_800_000_000;
    const B0: u64 = 26_000_000;
    const TTL_S: u64 = 24 * 3600;

    fn req(amount: u128) -> Request {
        Request { status: DStatus::Open, requester: [5; 20], at: T0, at_block: B0, amount, cost: 0 }
    }
    fn at(s: u64) -> Clock {
        Clock { time: T0 + s, parent_block: B0 + s / 12 }
    }

    #[test]
    fn the_selectors_and_the_struct_are_foundrys() {
        for (sel, want) in [(REQUEST_COUNT, "5badbe4c"), (REQUEST_OF, "324e03be"), (IS_EXPIRED, "d9548e53"), (TTL, "4e8b1dd5"),
            (ASSIGNED, "adb4d990"), (FULFILL, "dcf4cdf1")] {
            assert_eq!(hex::encode(sel), want);
        }
        assert_eq!(hex::encode(fulfill(3, 7)), format!("dcf4cdf1{:064x}{:064x}", 3, 7));
        // `cast abi-encode "f(uint8,address,uint64,uint64,uint256,uint256)" 1 0x…bb 1790000000 26000000 5e17 0`
        let b = hex::decode("000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000bb000000000000000000000000000000000000000000000000000000006ab13b8000000000000000000000000000000000000000000000000000000000018cba8000000000000000000000000000000000000000000000000006f05b59d3b200000000000000000000000000000000000000000000000000000000000000000000").unwrap();
        let mut bb = [0u8; 20];
        bb[19] = 0xbb;
        assert_eq!(decode_request(&b), Ok(Request { status: DStatus::Open, requester: bb, at: 1_790_000_000, at_block: 26_000_000,
            amount: E / 2, cost: 0 }));
    }

    #[test]
    fn a_request_is_taken_only_with_time_for_what_it_needs() {
        let r = req(E);
        assert!(may_take(&r, TTL_S, &at(0), false).is_ok());
        let buy = PURCHASE_LATENCY_SECS + MINT_MARGIN_SECS + crate::policy::CAP_MARGIN_SECS;
        assert!(may_take(&r, TTL_S, &at(TTL_S - buy), false).is_err(), "no time to buy");
        assert!(may_take(&r, TTL_S, &at(TTL_S - buy), true).is_ok(), "but time to mint one already held");
        assert!(may_take(&r, TTL_S, &at(TTL_S - MINT_MARGIN_SECS), true).is_err());
        // the desk expires on EITHER clock: the parent chain ahead alone is enough to stop
        let mut c = at(0);
        c.parent_block = B0 + (TTL_S - MINT_MARGIN_SECS) / 12;
        assert!(may_take(&r, TTL_S, &c, true).is_err());
        assert!(may_take(&req(crate::policy::MIN_PURCHASE_WEI - 1), TTL_S, &at(0), true).unwrap_err().contains("minimum"));
        let mut filled = r.clone();
        filled.status = DStatus::Filled;
        assert!(may_take(&filled, TTL_S, &at(0), true).is_err());
    }

    #[test]
    fn a_fill_is_never_above_the_requests_maximum() {
        let r = req(E / 2);
        assert!(fill_ok(&r, E / 2).is_ok());
        assert!(fill_ok(&r, E / 2 + 1).unwrap_err().contains("above the request's maximum"));
        assert!(may_mint(&r, false, TTL_S, &at(0), E / 2 + 1).is_err());
        assert!(may_fulfill(&r, false, E / 2 + 1).is_err());
        assert!(may_fulfill(&r, true, 1).unwrap_err().contains("expired"), "isExpired is checked immediately before");
        assert!(may_mint(&r, true, TTL_S, &at(0), 1).is_err());
        assert!(may_fulfill(&r, false, E / 2).is_ok());
    }
}
