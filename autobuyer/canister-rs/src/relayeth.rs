//! Relay between two EVM chains, native ETH to native ETH, to our own address (docs/v7/phase1-nft.md §2.1, §8): the judge of
//! `relayconv` (docs/v6/relay-conversion.md §3.1) with the destination an EVM chain instead of Bitcoin. The runner fetches a quote
//! for exactly the ask and hands it in; nothing in it is trusted that is not recomputed here: the deposit's id must be the
//! EIP-712 hash of the order we inspect (`relayconv::order_id`), and the order must name our own address as the recipient and
//! every refund, native ETH on both sides, the pinned depository and allocator, and a minimum out within the fee bound.
//!
//! Pure. The leg is a parameter so the reverse (Ethereum → 4663, docs §9 D4) is the same code: both are wired, [`TO_ETHEREUM`] for
//! an advance and, since fix round 2 of the 2026-09-27 review, [`FROM_ETHEREUM`] for the way back (`ethflow`'s `BackFunded`).

use crate::relayconv::{self as rc, bytes_of, get, in_band, list, same_evm, text, uint_of};
use serde_json::Value;

/// One direction: the origin and destination as Relay names them, their chain ids, and the depository the deposit goes to on
/// the origin with the allocator accepted there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Leg {
    pub origin: &'static str,
    pub origin_chain: u64,
    pub destination: &'static str,
    pub destination_chain: u64,
    pub depository: [u8; 20],
    pub allocator: [u8; 20],
}

/// 4663 → Ethereum mainnet. The depository and its allocator are the same addresses on both chains (measured 2026-09-27:
/// `/chains` and `allocator()` on both).
pub const TO_ETHEREUM: Leg = Leg { origin: "robinhood", origin_chain: 4663, destination: "ethereum", destination_chain: 1,
    depository: rc::RELAY_DEPOSITORY_4663, allocator: rc::RELAY_ALLOCATOR_4663 };
/// Ethereum mainnet → 4663: the way back (docs §9 D4), wired since fix round 2 of the 2026-09-27 review.
pub const FROM_ETHEREUM: Leg = Leg { origin: "ethereum", origin_chain: 1, destination: "robinhood", destination_chain: 4663,
    depository: rc::RELAY_DEPOSITORY_4663, allocator: rc::RELAY_ALLOCATOR_4663 };

/// Ethereum → Solana: the way back on the Solana home (docs/v9/solana-migration.md §2.8; spike Q5 `eth-sol-0.5`): the same Ethereum
/// depository and allocator as [`FROM_ETHEREUM`], the output native SOL to the canister's Solana address.
pub const ETHEREUM_TO_SOLANA: Leg = Leg { origin: "ethereum", origin_chain: 1, destination: "solana",
    destination_chain: crate::relaysol::RELAY_SOLANA, depository: rc::RELAY_DEPOSITORY_4663, allocator: rc::RELAY_ALLOCATOR_4663 };

/// Robinhood Chain → Solana: the way back of a Robinhood Chain NFT's buy (2026-10-12). Relay names the chain `"robinhood"` in its
/// orders (its quotes of 2026-09-23/27 from and to 4663, `tests/fixtures/relay-quote-4663-*.json`, and the fueler's legs); the depository
/// and its allocator on 4663 are the same addresses as on Ethereum (measured 2026-09-27: `/chains` and `allocator()` on both).
pub const ROBINHOOD_TO_SOLANA: Leg = Leg { origin: "robinhood", origin_chain: 4663, destination: "solana",
    destination_chain: crate::relaysol::RELAY_SOLANA, depository: rc::RELAY_DEPOSITORY_4663, allocator: rc::RELAY_ALLOCATOR_4663 };

/// How Relay's orders name an EVM chain (`orderData.*.chainId`), for the chains this build buys NFTs on: Ethereum mainnet and Robinhood
/// Chain (both read from real quotes). `None`: a chain this build has no Relay leg for (the install refuses a collection there).
pub fn relay_name(chain_id: u64) -> Option<&'static str> {
    match chain_id {
        1 => Some("ethereum"),
        4663 => Some("robinhood"),
        _ => None,
    }
}

/// The way back to Solana from an NFT chain: [`ETHEREUM_TO_SOLANA`] or [`ROBINHOOD_TO_SOLANA`].
pub fn to_solana(chain_id: u64) -> Option<Leg> {
    match chain_id {
        1 => Some(ETHEREUM_TO_SOLANA),
        4663 => Some(ROBINHOOD_TO_SOLANA),
        _ => None,
    }
}

/// A Solana destination: the canister's Solana address (the output and the destination's refund) and the least the order must pay
/// there, in lamports, from the canister's own SOL/ETH rate less the fee bound (rule 10; ETH → ETH compares wei with wei).
#[derive(Debug, Clone, Copy)]
pub struct SolDest<'a> {
    pub recipient: &'a str,
    pub min_out_floor: u128,
}

/// Relay's min-out bound for ETH → ETH: the default and its range (the install's `relay_max_fee_bps`).
pub const DEFAULT_MAX_FEE_BPS: u16 = 300;
pub const MIN_FEE_BPS: u16 = 50;
pub const MAX_FEE_BPS: u16 = 500;

/// Everything the quote is judged against: the canister's own values and reads.
pub struct Want<'a> {
    pub leg: Leg,
    pub ours: [u8; 20],
    pub amount_wei: u128,
    pub max_fee_bps: u16,
    /// The depository's `allocator()` read at this tick's pin on the origin chain.
    pub allocator: [u8; 20],
    pub now: u64,
    pub handed_in_at: u64,
    /// Every order id this canister ever accepted (rule 12).
    pub used: &'a [[u8; 32]],
}

/// The deposit to sign on the origin chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EthDeposit {
    pub to: [u8; 20],
    pub value: u128,
    pub data: Vec<u8>,
    pub order_id: [u8; 32],
    /// The least the order pays us on the destination.
    pub min_out: u128,
    pub deadline: u64,
    /// Review round 4 (C4-M-1): the order's solver (`orderData.solver`, hashed into the order id): who fills it, so a late payout
    /// can be tied to this order by its sender.
    pub solver: Option<[u8; 20]>,
    /// Review round 5 (C5-L-1): Relay's request id for the order (`steps[0].requestId`), which Relay's fill carries in its calldata. Not
    /// bound by the order id (the quote's to name): it only ever helps the payout judge ACCEPT a transaction that names it, and every
    /// such credit is still the operator's hand-in, at most the order's value, against ETH that is at our address.
    pub request_id: Option<[u8; 32]>,
}

/// The least an ETH → ETH order may promise: `amount × (1 − max_fee_bps)`, rounded up.
pub fn min_out_floor(amount_wei: u128, max_fee_bps: u16) -> u128 {
    (amount_wei.saturating_mul(10_000 - max_fee_bps as u128)).div_ceil(10_000)
}

fn native(a: &str) -> bool {
    same_evm(a, &[0u8; 20])
}

/// Audit A5-04 (conversion review C-07, carried over): an order's deadline is a uint32: one over it is refused, never truncated.
fn deadline_of(v: &Value, k: &str, at: &str) -> Result<u64, String> {
    let d = uint_of(v, k, at)?;
    u32::try_from(d).map(u64::from).map_err(|_| format!("rule 11: {at}.{k} = {d} is over a uint32"))
}

/// Judge the runner's quote. Every refusal names the rule (numbered as in docs/v6/relay-conversion.md §3.1) and the field.
pub fn judge(body: &[u8], w: &Want) -> Result<EthDeposit, String> {
    judge_with(body, w, None)
}

/// [`judge`] for an EVM origin paying out on Solana ([`ETHEREUM_TO_SOLANA`]): the same rules, the destination's recipient, refund
/// and currency Solana's (our Solana address, native SOL), the minimum against `dest.min_out_floor`.
pub fn judge_to_solana(body: &[u8], w: &Want, dest: &SolDest) -> Result<EthDeposit, String> {
    if w.leg.destination != "solana" {
        return Err(format!("the leg pays out on {}, not solana", w.leg.destination));
    }
    judge_with(body, w, Some(dest))
}

fn judge_with(body: &[u8], w: &Want, sol: Option<&SolDest>) -> Result<EthDeposit, String> {
    if body.len() > rc::MAX_QUOTE_BYTES {
        return Err(format!("the quote is {} bytes, over {}", body.len(), rc::MAX_QUOTE_BYTES));
    }
    if !(MIN_FEE_BPS..=MAX_FEE_BPS).contains(&w.max_fee_bps) {
        return Err(format!("max_fee_bps {} is outside [{MIN_FEE_BPS}, {MAX_FEE_BPS}]", w.max_fee_bps));
    }
    // rule 11 (first: a stale hand-in is not worth parsing)
    if w.now.saturating_sub(w.handed_in_at) > rc::QUOTE_TTL_SECS {
        return Err(format!("rule 11: stale: handed in {} s ago, over the {} s a quote lives here", w.now.saturating_sub(w.handed_in_at),
            rc::QUOTE_TTL_SECS));
    }
    let q: Value = serde_json::from_slice(body).map_err(|e| format!("not JSON: {e}"))?;
    // rule 1: one deposit transaction
    let steps = list(&q, "steps", "the quote")?;
    if steps.len() != 1 {
        return Err(format!("rule 1: {} steps; only a single deposit step is signed", steps.len()));
    }
    let step = &steps[0];
    if text(step, "id", "steps[0]")? != "deposit" || text(step, "kind", "steps[0]")? != "transaction" {
        return Err("rule 1: the one step is not a deposit transaction".into());
    }
    let items = list(step, "items", "steps[0]")?;
    if items.len() != 1 {
        return Err(format!("rule 1: {} items; exactly one transaction is signed", items.len()));
    }
    let d = get(&items[0], "data", "steps[0].items[0]")?;
    let dat = "the deposit";
    // rule 2: the origin chain
    if uint_of(d, "chainId", dat)? != w.leg.origin_chain as u128 {
        return Err(format!("rule 2: the deposit is for chain {}, not {}", uint_of(d, "chainId", dat)?, w.leg.origin_chain));
    }
    // rule 3: the pinned depository, and the allocator we accepted
    let to = crate::evmtx::parse_address(text(d, "to", dat)?)?;
    if to != w.leg.depository {
        return Err(format!("rule 3: the deposit goes to 0x{}, not Relay's pinned depository", hex::encode(to)));
    }
    if w.allocator != w.leg.allocator {
        return Err(format!("rule 3: the depository's allocator is 0x{}, not the pinned 0x{}: Relay rotated its key; review before bridging",
            hex::encode(w.allocator), hex::encode(w.leg.allocator)));
    }
    if let Ok(from) = text(d, "from", dat) {
        if !same_evm(from, &w.ours) {
            return Err("rule 5: the quote was made for another sender".into());
        }
    }
    // rule 4: the value
    let value = uint_of(d, "value", dat)?;
    if value != w.amount_wei {
        return Err(format!("rule 4: the deposit sends {value} wei, not the {} asked", w.amount_wei));
    }
    // rule 5: the calldata
    let data = bytes_of(d, "data", dat)?;
    if data.len() != 68 || data[..4] != rc::DEPOSIT_NATIVE {
        return Err("rule 5: the calldata is not depositNative(address, bytes32)".into());
    }
    if data[4..16].iter().any(|b| *b != 0) || data[16..36] != w.ours {
        return Err("rule 5: the calldata names another depositor".into());
    }
    let id: [u8; 32] = data[36..68].try_into().expect("32 bytes");
    // rule 6: the order is the order we inspect
    let v2 = get(get(&q, "protocol", "the quote")?, "v2", "protocol")?;
    let o = get(v2, "orderData", "protocol.v2")?;
    let computed = rc::order_id(o).map_err(|e| format!("rule 6: {e}"))?;
    if computed != id {
        return Err(format!("rule 6: the calldata commits to 0x{}, but the order hashes to 0x{}", hex::encode(id), hex::encode(computed)));
    }
    if bytes_of(v2, "orderId", "protocol.v2")? != id {
        return Err("rule 6: protocol.v2.orderId is not the deposit's id".into());
    }
    let pd = get(v2, "paymentDetails", "protocol.v2")?;
    if crate::evmtx::parse_address(text(pd, "depository", "paymentDetails")?)? != w.leg.depository || uint_of(pd, "amount", "paymentDetails")? != value {
        return Err("rule 3/4: paymentDetails names another depository or amount".into());
    }
    // rule 7: the input
    let inputs = list(o, "inputs", "orderData")?;
    if inputs.len() != 1 {
        return Err(format!("rule 7: {} inputs, not one", inputs.len()));
    }
    let p = get(&inputs[0], "payment", "orderData.inputs[0]")?;
    if text(p, "chainId", "payment")? != w.leg.origin || !native(text(p, "currency", "payment")?) || uint_of(p, "amount", "payment")? != value {
        return Err(format!("rule 7: the order's payment is not native ETH on {} for the deposit's value", w.leg.origin));
    }
    // rule 8: every refund is ours, in native ETH, on one of the two chains; at least one on the origin
    let refunds = list(&inputs[0], "refunds", "orderData.inputs[0]")?;
    if refunds.is_empty() {
        return Err("rule 8: the order has no refund".into());
    }
    let mut origin_refund = false;
    for (i, r) in refunds.iter().enumerate() {
        let rat = format!("refunds[{i}]");
        let (chain, to, cur) = (text(r, "chainId", &rat)?, text(r, "recipient", &rat)?, text(r, "currency", &rat)?);
        let ok = match sol {
            Some(d) if chain == w.leg.destination => crate::relaysol::same(to, d.recipient, "solana") && cur == crate::relaysol::SOL_NATIVE,
            Some(_) => chain == w.leg.origin && same_evm(to, &w.ours) && native(cur),
            None => (chain == w.leg.origin || chain == w.leg.destination) && same_evm(to, &w.ours) && native(cur),
        };
        if !ok {
            return Err(format!("rule 8: {rat} pays {to} on {chain}, not our own address in its native currency"));
        }
        origin_refund |= chain == w.leg.origin;
        let dl = deadline_of(r, "deadline", &rat)?;
        if !in_band(dl, w.now) {
            return Err(format!("rule 11: {rat}'s deadline {dl} is outside [now + 1 day, now + 8 days]"));
        }
    }
    if !origin_refund {
        return Err(format!("rule 8: no refund on {} to our address", w.leg.origin));
    }
    // rule 9: the output — our own address on the destination, native ETH, no calls, no fees
    let out = get(o, "output", "orderData")?;
    if text(out, "chainId", "output")? != w.leg.destination {
        return Err(format!("rule 9: the order pays out on {}, not {}", text(out, "chainId", "output")?, w.leg.destination));
    }
    let pays = list(out, "payments", "output")?;
    if pays.len() != 1 {
        return Err(format!("rule 9: {} payments, not one", pays.len()));
    }
    let pay = &pays[0];
    let (pay_to, pay_cur) = (text(pay, "recipient", "payment")?, text(pay, "currency", "payment")?);
    let (ours, natively) = match sol {
        Some(d) => (crate::relaysol::same(pay_to, d.recipient, "solana"), pay_cur == crate::relaysol::SOL_NATIVE),
        None => (same_evm(pay_to, &w.ours), native(pay_cur)),
    };
    if !ours {
        return Err(format!("rule 9: the order pays {pay_to}, not our own address"));
    }
    if !natively {
        return Err(format!("rule 9: the order does not pay {}'s native currency", w.leg.destination));
    }
    if !list(out, "calls", "output")?.is_empty() {
        return Err("rule 9: the order carries calls".into());
    }
    if !list(o, "fees", "orderData")?.is_empty() {
        return Err("rule 9: the order carries fees".into());
    }
    // rule 10: the minimum out
    let min = uint_of(pay, "minimumAmount", "payment")?;
    let floor = sol.map_or_else(|| min_out_floor(value, w.max_fee_bps), |d| d.min_out_floor);
    if min < floor {
        return Err(format!("rule 10: the order promises at least {min} wei, under our floor of {floor} ({} bps)", w.max_fee_bps));
    }
    // rule 11: the output's deadline
    let deadline = deadline_of(out, "deadline", "output")?;
    if !in_band(deadline, w.now) {
        return Err(format!("rule 11: the output's deadline {deadline} is outside [now + 1 day, now + 8 days]"));
    }
    // rule 12: never the same order twice
    if w.used.contains(&id) {
        return Err(format!("rule 12: order 0x{} was already accepted once", hex::encode(id)));
    }
    let solver = o.get("solver").and_then(|x| x.as_str()).and_then(|x| crate::evmtx::parse_address(x).ok());
    let request_id = step.get("requestId").and_then(|x| x.as_str()).and_then(|x| hex::decode(x.trim_start_matches("0x")).ok())
        .and_then(|b| <[u8; 32]>::try_from(b).ok()).filter(|r| *r != [0; 32]);
    Ok(EthDeposit { to, value, data, order_id: id, min_out: min, deadline, solver, request_id })
}

#[cfg(test)]
mod tests {
    use super::*;

    const QUOTE: &str = include_str!("../tests/fixtures/relay-quote-4663-1-2026-09-27.json");
    const BACK: &str = include_str!("../tests/fixtures/relay-quote-1-4663-2026-09-27.json");
    const USER: &str = "76020b7093bd35d2520d47ded764efc554237efe";
    const BACK_USER: &str = "2427857b22f64a7b9cb22148928f1e410c7aa108";
    const AMOUNT: u128 = 500_000_000_000_000_000;
    // the quote's own time: its deadlines are quote + 7 days (1,791,117,628)
    const NOW: u64 = 1_791_117_628 - 7 * 86_400;

    fn us() -> [u8; 20] {
        hex::decode(USER).unwrap().try_into().unwrap()
    }
    fn want(used: &[[u8; 32]]) -> Want<'_> {
        Want { leg: TO_ETHEREUM, ours: us(), amount_wei: AMOUNT, max_fee_bps: 300, allocator: rc::RELAY_ALLOCATOR_4663, now: NOW,
            handed_in_at: NOW - 10, used }
    }
    fn tweak(f: impl Fn(&mut Value)) -> Vec<u8> {
        let mut v: Value = serde_json::from_str(QUOTE).unwrap();
        f(&mut v);
        serde_json::to_vec(&v).unwrap()
    }

    #[test]
    fn the_real_quote_passes_and_binds_the_order() {
        let d = judge(QUOTE.as_bytes(), &want(&[])).unwrap();
        assert_eq!(d.to, rc::RELAY_DEPOSITORY_4663);
        assert_eq!(d.value, AMOUNT);
        assert_eq!(d.min_out, 488_475_083_200_866_055);
        assert_eq!(hex::encode(d.order_id), "bce9b0ca025fca35649d3c604117f642667842017c6527fc2ecf002ebc2e69e2");
        assert_eq!(d.deadline, 1_791_117_628);
        assert_eq!(&d.data[..4], &rc::DEPOSIT_NATIVE);
    }

    #[test]
    fn the_reverse_leg_is_the_same_code() {
        let ours: [u8; 20] = hex::decode(BACK_USER).unwrap().try_into().unwrap();
        let v: Value = serde_json::from_str(BACK).unwrap();
        let dl = v["protocol"]["v2"]["orderData"]["output"]["deadline"].as_u64().unwrap();
        let w = Want { leg: FROM_ETHEREUM, ours, amount_wei: 200_000_000_000_000_000, max_fee_bps: 300, allocator: rc::RELAY_ALLOCATOR_4663,
            now: dl - 7 * 86_400, handed_in_at: dl - 7 * 86_400, used: &[] };
        let d = judge(BACK.as_bytes(), &w).unwrap();
        assert_eq!(d.value, 200_000_000_000_000_000);
        // and the forward leg's rules refuse it (another origin)
        let w2 = Want { leg: TO_ETHEREUM, ..w };
        assert!(judge(BACK.as_bytes(), &w2).unwrap_err().contains("rule 2"));
    }

    #[test]
    fn every_rule_refuses_its_lie() {
        let r = |b: Vec<u8>| judge(&b, &want(&[])).unwrap_err();
        let w0 = want(&[]);
        assert!(judge(QUOTE.as_bytes(), &Want { handed_in_at: NOW - 301, ..want(&[]) }).unwrap_err().contains("rule 11: stale"));
        assert!(judge(QUOTE.as_bytes(), &Want { max_fee_bps: 600, ..want(&[]) }).unwrap_err().contains("outside"));
        assert!(r(tweak(|v| { let s = v["steps"][0].clone(); v["steps"].as_array_mut().unwrap().push(s); })).contains("rule 1"));
        assert!(r(tweak(|v| v["steps"][0]["kind"] = "signature".into())).contains("rule 1"));
        assert!(r(tweak(|v| v["steps"][0]["items"][0]["data"]["chainId"] = 1.into())).contains("rule 2"));
        assert!(r(tweak(|v| v["steps"][0]["items"][0]["data"]["to"] = "0x1111111111111111111111111111111111111111".into())).contains("rule 3"));
        assert!(judge(QUOTE.as_bytes(), &Want { allocator: [9; 20], ..want(&[]) }).unwrap_err().contains("allocator"));
        assert!(r(tweak(|v| v["steps"][0]["items"][0]["data"]["value"] = "1".into())).contains("rule 4"));
        assert!(judge(QUOTE.as_bytes(), &Want { amount_wei: AMOUNT - 1, ..want(&[]) }).unwrap_err().contains("rule 4"));
        assert!(judge(QUOTE.as_bytes(), &Want { ours: [7; 20], ..want(&[]) }).unwrap_err().contains("rule 5"));
        // a different id in the calldata than the order hashes to
        assert!(r(tweak(|v| {
            let d = v["steps"][0]["items"][0]["data"]["data"].as_str().unwrap().to_string();
            v["steps"][0]["items"][0]["data"]["data"] = format!("{}00", &d[..d.len() - 2]).into();
        })).contains("rule 6"));
        // the order itself changed: the recipient on Ethereum, the min out, the chain, a call, a fee, a refund to a stranger
        for f in [
            (|v: &mut Value| v["protocol"]["v2"]["orderData"]["output"]["payments"][0]["recipient"] = "0x1111111111111111111111111111111111111111".into()) as fn(&mut Value),
            |v| v["protocol"]["v2"]["orderData"]["output"]["payments"][0]["minimumAmount"] = "1".into(),
            |v| v["protocol"]["v2"]["orderData"]["output"]["chainId"] = "base".into(),
            |v| v["protocol"]["v2"]["orderData"]["output"]["calls"] = serde_json::json!(["0x12"]),
            |v| v["protocol"]["v2"]["orderData"]["inputs"][0]["refunds"][1]["recipient"] = "0x1111111111111111111111111111111111111111".into(),
        ] {
            assert!(r(tweak(f)).contains("rule 6"), "a changed order no longer hashes to the deposit's id");
        }
        // the same lies with the id recomputed (a consistent but hostile order): each is refused by its own rule
        let consistent = |f: fn(&mut Value)| {
            tweak(move |v| {
                f(v);
                let id = rc::order_id(&v["protocol"]["v2"]["orderData"]).unwrap();
                let d = v["steps"][0]["items"][0]["data"]["data"].as_str().unwrap().to_string();
                v["steps"][0]["items"][0]["data"]["data"] = format!("{}{}", &d[..2 + 72], hex::encode(id)).into();
                v["protocol"]["v2"]["orderId"] = format!("0x{}", hex::encode(id)).into();
            })
        };
        assert!(r(consistent(|v| v["protocol"]["v2"]["orderData"]["output"]["payments"][0]["recipient"] = "0x1111111111111111111111111111111111111111".into())).contains("rule 9"));
        assert!(r(consistent(|v| v["protocol"]["v2"]["orderData"]["output"]["payments"][0]["minimumAmount"] = "400000000000000000".into())).contains("rule 10"));
        assert!(r(consistent(|v| v["protocol"]["v2"]["orderData"]["output"]["chainId"] = "base".into())).contains("rule 9"));
        assert!(r(consistent(|v| v["protocol"]["v2"]["orderData"]["output"]["calls"] = serde_json::json!(["0x12"]))).contains("rule 9"));
        assert!(r(consistent(|v| v["protocol"]["v2"]["orderData"]["inputs"][0]["refunds"][1]["recipient"] = "0x1111111111111111111111111111111111111111".into())).contains("rule 8"));
        assert!(r(consistent(|v| v["protocol"]["v2"]["orderData"]["inputs"][0]["payment"]["currency"] = "0x1111111111111111111111111111111111111111".into())).contains("rule 7"));
        assert!(r(consistent(|v| v["protocol"]["v2"]["orderData"]["output"]["payments"][0]["currency"] = "0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2".into())).contains("rule 9"));
        assert!(r(consistent(|v| v["protocol"]["v2"]["orderData"]["output"]["deadline"] = 1_900_000_000u64.into())).contains("rule 11"));
        // rule 12
        let id = judge(QUOTE.as_bytes(), &w0).unwrap().order_id;
        assert!(judge(QUOTE.as_bytes(), &want(&[id])).unwrap_err().contains("rule 12"));
        // a quote that expired days ago
        assert!(judge(QUOTE.as_bytes(), &Want { now: NOW + 6 * 86_400 + 1, handed_in_at: NOW + 6 * 86_400, ..want(&[]) }).unwrap_err().contains("rule 11"));
    }

    /// The way back on the Solana home, the real ETH → SOL quote of 2026-10-09 (spike Q5 `eth-sol-0.5`): the same Ethereum depository,
    /// the output and the destination's refund our Solana address in native SOL, the minimum against our own SOL/ETH rate.
    #[test]
    fn the_way_back_to_solana_is_judged_with_a_solana_destination() {
        const ETH_SOL: &str = include_str!("../tests/fixtures/relay-sol/quotes/eth-sol-0.5.quote.json");
        const OURS_SOL: &str = "5ecsyZA5SVEuTByKoDCUivha8uzAgeQJmgFnnvokFF21";
        let ours: [u8; 20] = hex::decode("9db3fb117ecfc8fc14ca263e401a4c57a6913d81").unwrap().try_into().unwrap();
        let now = 1_791_565_570;
        let w = Want { leg: ETHEREUM_TO_SOLANA, ours, amount_wei: 500_000_000_000_000_000, max_fee_bps: 300, allocator: rc::RELAY_ALLOCATOR_4663,
            now, handed_in_at: now, used: &[] };
        let d = judge_to_solana(ETH_SOL.as_bytes(), &w, &SolDest { recipient: OURS_SOL, min_out_floor: 11_000_000_000 }).unwrap();
        assert_eq!((d.value, d.min_out, d.to), (500_000_000_000_000_000, 11_068_534_059, rc::RELAY_DEPOSITORY_4663));
        // our floor above Relay's minimum: rule 10
        assert!(judge_to_solana(ETH_SOL.as_bytes(), &w, &SolDest { recipient: OURS_SOL, min_out_floor: 11_100_000_000 }).unwrap_err().contains("rule 10"));
        // another Solana recipient: rules 8/9
        let e = judge_to_solana(ETH_SOL.as_bytes(), &w, &SolDest { recipient: "Dodg2HifwU8rmaVVyMyUZDGTRbqAJTyVYxXPwcbNpBKc", min_out_floor: 1 }).unwrap_err();
        assert!(e.contains("rule 8") || e.contains("rule 9"), "{e}");
        // the EVM judge refuses it (a Solana output is not our EVM address), and the Solana judge needs a Solana leg
        assert!(judge(ETH_SOL.as_bytes(), &Want { leg: FROM_ETHEREUM, ..w }).is_err());
        assert!(judge_to_solana(ETH_SOL.as_bytes(), &Want { leg: FROM_ETHEREUM, ..w }, &SolDest { recipient: OURS_SOL, min_out_floor: 1 }).is_err());
    }

    #[test]
    fn the_min_out_floor() {
        assert_eq!(min_out_floor(AMOUNT, 300), 485_000_000_000_000_000);
        assert_eq!(min_out_floor(3, 300), 3);
    }
}
