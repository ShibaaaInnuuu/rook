//! The `FrogCurve` adapter (as merged at `40735bf`): what to read from a graduating curve, how that reads as a [`SinkView`],
//! and what to call on it. The rules themselves are `itemsink.rs`'s (docs/v3/curve.md, "Canister: …").
//!
//! Since 2026-09-23 graduations are meant to feed a collection's buy queue (`buyqueue.rs`) instead; this adapter drives the
//! curve contract that exists on the branch, through the same rules, until that lands.

use crate::abi::{self, Arg};
use crate::itemsink::{both_past, Clock, SinkKind, SinkView};

// ---- the ABI (selectors from `cast sig`, checked in the tests) ---------------------------------------------------------------

pub const STATE: [u8; 4] = [0xc1, 0x9d, 0x93, 0xfb];
pub const FROGS: [u8; 4] = [0xf8, 0x4b, 0x51, 0x3d];
pub const FROGS_BOUGHT: [u8; 4] = [0x12, 0xbb, 0x12, 0xcb];
pub const FROG_SPENT: [u8; 4] = [0x3a, 0xbb, 0x6f, 0x84];
pub const FROG_BUDGET: [u8; 4] = [0xbd, 0xfd, 0xab, 0xa1];
pub const BUFFER: [u8; 4] = [0xed, 0xaa, 0xfe, 0x20];
pub const DEADLINE: [u8; 4] = [0x29, 0xdc, 0xb0, 0xcf];
pub const PAYEE: [u8; 4] = [0xae, 0x90, 0xb2, 0x13];
pub const WRAPPER: [u8; 4] = [0xac, 0x21, 0x0c, 0xc7];
pub const FACTORY: [u8; 4] = [0xc4, 0x5a, 0x01, 0x55];
pub const RECORD_FROG: [u8; 4] = [0x37, 0xbe, 0x52, 0x2b];
pub const OPEN_REFUNDS: [u8; 4] = [0x27, 0xd4, 0xba, 0x7f];
pub const GRADUATE: [u8; 4] = [0xd3, 0x61, 0x8c, 0xca];
/// CurveFactory
pub const COIN_COUNT: [u8; 4] = [0x67, 0x8f, 0xd2, 0x89];
pub const COINS: [u8; 4] = [0xc6, 0x61, 0x06, 0x57];
pub const ROUTER_OF: [u8; 4] = [0x25, 0x35, 0x6d, 0x2b];
/// ERC-20 (the Wrapper)
pub const BALANCE_OF: [u8; 4] = [0x70, 0xa0, 0x82, 0x31];

pub fn call0(sel: [u8; 4]) -> Vec<u8> {
    sel.to_vec()
}
pub fn call_uint(sel: [u8; 4], v: u128) -> Vec<u8> {
    abi::encode_call(sel, &[Arg::Word(abi::uint_word(v))])
}
pub fn call_addr(sel: [u8; 4], a: &[u8; 20]) -> Vec<u8> {
    abi::encode_call(sel, &[Arg::Word(abi::addr_word(a))])
}
pub fn record_frog(cost_wei: u128) -> Vec<u8> {
    call_uint(RECORD_FROG, cost_wei)
}
pub fn ret_uint(b: &[u8]) -> Result<u128, String> {
    abi::uint_at(b, 0, 0, 128)
}
pub fn ret_addr(b: &[u8]) -> Result<[u8; 20], String> {
    abi::addr_at(b, 0, 0)
}
/// `deadline()`: `(uint64 at, uint64 atBlock)`.
pub fn ret_deadline(b: &[u8]) -> Result<(u64, u64), String> {
    Ok((abi::uint_at(b, 0, 0, 64)? as u64, abi::uint_at(b, 0, 1, 64)? as u64))
}

/// `FrogCurve.State`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub enum CState {
    Trading,
    Graduating,
    Graduated,
    Refunding,
}

pub fn ret_state(b: &[u8]) -> Result<CState, String> {
    match abi::uint_at(b, 0, 0, 8)? {
        0 => Ok(CState::Trading),
        1 => Ok(CState::Graduating),
        2 => Ok(CState::Graduated),
        3 => Ok(CState::Refunding),
        s => Err(format!("a curve state of {s} is not the contract's")),
    }
}

/// The reads, in the order [`view`] takes their answers.
pub fn reads() -> Vec<Vec<u8>> {
    [STATE, FROGS, FROGS_BOUGHT, FROG_SPENT, FROG_BUDGET, BUFFER, DEADLINE].iter().map(|s| call0(*s)).collect()
}

/// A curve, read at one pin, as the item rules see it: open while `Graduating`; wants `N − k`; holds
/// `frogBudget + buffer − frogSpent`; its window is the deadline; refunds are driven once both clocks are past it short of N;
/// graduation once `k = N`.
pub fn view(r: &[Vec<u8>], clock: Clock) -> Result<SinkView, String> {
    if r.len() != 7 {
        return Err("a curve is seven reads".into());
    }
    let state = ret_state(&r[0])?;
    let n = ret_uint(&r[1])? as u64;
    let k = ret_uint(&r[2])? as u64;
    let spent = ret_uint(&r[3])?;
    let budget = ret_uint(&r[4])?;
    let buffer = ret_uint(&r[5])?;
    let (at, at_block) = ret_deadline(&r[6])?;
    let graduating = state == CState::Graduating;
    Ok(SinkView {
        kind: SinkKind::Curve,
        status: format!("{state:?}"),
        open: graduating && k < n,
        done: matches!(state, CState::Graduated | CState::Refunding),
        recorded: k,
        target: Some(n),
        want: if graduating { n.saturating_sub(k) } else { 0 },
        budget_left: (budget + buffer).saturating_sub(spent),
        item_cap: None,
        deadline: if graduating { Some((at, at_block)) } else { None },
        clock,
        drive_refunds: graduating && k < n && both_past(at, at_block, &clock),
        drive_settle: graduating && k == n,
        price_expiry: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::itemsink::{may_mint, may_start, per_item_cap};

    const T0: u64 = 1_800_000_000;
    const B0: u64 = 26_000_000;
    const WINDOW: u64 = 3 * 24 * 3600;

    fn w(v: u128) -> Vec<u8> {
        abi::uint_word(v).to_vec()
    }
    /// The worked example's curve (docs/v3/curve.md): 6 frogs, 3.88 frog budget, 0.24 buffer, a three-day window.
    fn reads_at(state: u128, k: u128, spent: u128) -> Vec<Vec<u8>> {
        vec![w(state), w(6), w(k), w(spent), w(3_880_000_000_000_000_000), w(240_000_000_000_000_000),
            [abi::uint_word((T0 + WINDOW) as u128), abi::uint_word((B0 + WINDOW / 12) as u128)].concat()]
    }
    fn at(s: u64) -> Clock {
        Clock { time: T0 + s, parent_block: B0 + s / 12 }
    }

    #[test]
    fn the_selectors_are_foundrys() {
        for (sel, want) in [(STATE, "c19d93fb"), (FROGS, "f84b513d"), (FROGS_BOUGHT, "12bb12cb"), (FROG_SPENT, "3abb6f84"),
            (FROG_BUDGET, "bdfdaba1"), (BUFFER, "edaafe20"), (DEADLINE, "29dcb0cf"), (PAYEE, "ae90b213"), (WRAPPER, "ac210cc7"),
            (FACTORY, "c45a0155"), (RECORD_FROG, "37be522b"), (OPEN_REFUNDS, "27d4ba7f"), (GRADUATE, "d3618cca"),
            (COIN_COUNT, "678fd289"), (COINS, "c6610657"), (ROUTER_OF, "25356d2b"), (BALANCE_OF, "70a08231")] {
            assert_eq!(hex::encode(sel), want);
        }
        assert_eq!(hex::encode(record_frog(5)), format!("37be522b{:064x}", 5));
        assert_eq!(ret_state(&w(1)), Ok(CState::Graduating));
        assert!(ret_state(&w(4)).is_err());
    }

    #[test]
    fn a_graduating_curve_reads_as_a_sink_wanting_n_minus_k() {
        let v = view(&reads_at(1, 0, 0), at(0)).unwrap();
        assert_eq!((v.open, v.want, v.target, v.budget_left), (true, 6, Some(6), 4_120_000_000_000_000_000));
        assert_eq!(per_item_cap(&v, 0, 0), Some(4_120_000_000_000_000_000 / 6), "(3.88 + 0.24) / 6");
        // two recorded at 0.60: (4.12 − 1.20) / 4 — the savings are the next frogs'
        let v = view(&reads_at(1, 2, 1_200_000_000_000_000_000), at(3600)).unwrap();
        assert_eq!(per_item_cap(&v, 0, 0), Some(730_000_000_000_000_000));
        assert_eq!(per_item_cap(&v, 1, 650_000_000_000_000_000), Some(2_270_000_000_000_000_000 / 3));
        // floor moved past the buffer: a frog that would take the total over frogBudget + buffer is not minted
        let v = view(&reads_at(1, 5, 3_700_000_000_000_000_000), at(0)).unwrap();
        assert!(may_mint(&v, 420_000_000_000_000_001).is_err());
        assert!(may_mint(&v, 420_000_000_000_000_000).is_ok());
    }

    #[test]
    fn the_window_stops_purchases_then_mints_and_both_clocks_open_refunds() {
        let need = crate::frogbuy::PURCHASE_LATENCY_SECS + crate::frogbuy::MINT_MARGIN_SECS + crate::policy::CAP_MARGIN_SECS;
        assert!(may_start(&view(&reads_at(1, 0, 0), at(WINDOW - need - 12)).unwrap(), 0).is_ok());
        assert!(may_start(&view(&reads_at(1, 0, 0), at(WINDOW - need)).unwrap(), 0).is_err());
        assert!(may_mint(&view(&reads_at(1, 0, 0), at(WINDOW - crate::frogbuy::MINT_MARGIN_SECS)).unwrap(), 1).is_err());
        assert!(!view(&reads_at(1, 3, 0), at(WINDOW - 1)).unwrap().drive_refunds);
        assert!(view(&reads_at(1, 3, 0), at(WINDOW)).unwrap().drive_refunds);
        let mut c = at(WINDOW + 3600);
        c.parent_block = B0 + WINDOW / 12 - 1;
        assert!(!view(&reads_at(1, 3, 0), c).unwrap().drive_refunds, "the timestamp alone is the sequencer's word");
        let full = view(&reads_at(1, 6, 0), at(WINDOW)).unwrap();
        assert!(full.drive_settle && !full.drive_refunds && !full.open);
        for (s, done) in [(0, false), (2, true), (3, true)] {
            let v = view(&reads_at(s, 0, 0), at(0)).unwrap();
            assert!(!v.open && v.done == done && !v.drive_refunds);
        }
    }
}
