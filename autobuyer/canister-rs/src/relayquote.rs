//! Relay (relay.link), judged as far as it can be (`docs/v2/venue-bridge-policy.md` §2a).
//!
//! Relay has the only live one-hop route from Robinhood Chain 4663 to native Bitcoin (quoted 2026-09-23: 0.1 ETH → 315,473
//! sats, ~210 s). Its deposit is `depositNative(address depositor, bytes32 id)` to its depository: the Bitcoin recipient is NOT
//! in the transaction, and before the deposit Relay publishes no record binding `id` to a recipient (`/requests` is empty,
//! `/intents/status/v2` says only "waiting"; `/requests/v3` needs a key). So nothing can PROVE where the BTC goes before signing.
//!
//! What this module does is the most that can be checked: the canister asks Relay for the quote ITSELF, naming its own
//! taproot address as recipient (so no relayer can substitute its own), and accepts the answer only if every checkable field
//! is right — one deposit step, on 4663, to the known depository, for exactly the amount, calldata naming us as depositor, the
//! quote's stated recipient our own address, native BTC out, at least our floor. What remains trusted is stated: Relay, and the
//! one replica that served the (non-replicated) quote. Arrival is still proven only by reading Bitcoin.
//!
//! Pure; tested on a real captured quote. NOT configured as the buyer's bridge: accepting Relay's residual trust is the user's
//! decision.

use crate::bridgecall::RELAY_DEPOSIT_NATIVE;

/// Relay's depository on 4663 (the quote's deposit target, 2026-09-23). Not a proxy (no EIP-1967 slots); `owner()`
/// 0xF61A…775A and `allocator()` 0x63C1…1b56 on chain: deposits are released by Relay's allocator.
pub const RELAY_DEPOSITORY_4663: [u8; 20] = [0x4c, 0xd0, 0x0e, 0x38, 0x76, 0x22, 0xc3, 0x5b, 0xdd, 0xb9, 0xb4, 0xc9, 0x62, 0xc1,
    0x36, 0x46, 0x23, 0x38, 0xbc, 0x31];
/// Relay's id for Bitcoin, and its id for native BTC.
pub const RELAY_BITCOIN: u64 = 8_253_038;
pub const RELAY_BTC_NATIVE: &str = "bc1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqmql8k8";

/// The deposit the canister would sign, taken from a quote that passed every check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayDeposit {
    pub to: [u8; 20],
    pub value: u128,
    pub data: Vec<u8>,
    /// Relay's id in the calldata, and the request id it reports: kept so the request can be looked up after the deposit.
    pub deposit_id: [u8; 32],
    pub request_id: String,
    pub sats_out: u64,
}

pub struct Want<'a> {
    pub chain_id: u64,
    pub our_evm: [u8; 20],
    pub our_btc_address: &'a str,
    pub amount_wei: u128,
    pub floor_sats: u64,
}

fn s<'a>(v: &'a serde_json::Value, path: &[&str]) -> Result<&'a serde_json::Value, String> {
    path.iter().try_fold(v, |x, k| x.get(*k).ok_or_else(|| format!("the quote has no {}", path.join("."))))
}
fn text<'a>(v: &'a serde_json::Value, path: &[&str]) -> Result<&'a str, String> {
    s(v, path)?.as_str().ok_or_else(|| format!("{} is not a string", path.join(".")))
}
fn num_text(v: &serde_json::Value, path: &[&str]) -> Result<u128, String> {
    let x = s(v, path)?;
    let t = x.as_str().map(|t| t.to_string()).or_else(|| x.as_u64().map(|n| n.to_string())).ok_or_else(|| format!("{} is not a number", path.join(".")))?;
    t.parse().map_err(|_| format!("{} = {t:?} is not a whole number", path.join(".")))
}
fn addr(t: &str) -> Result<[u8; 20], String> {
    let b = hex::decode(t.trim_start_matches("0x")).map_err(|_| format!("{t:?} is not hex"))?;
    b.try_into().map_err(|_| format!("{t:?} is not 20 bytes"))
}

/// Judge Relay's `/quote` answer. Every refusal names the field.
pub fn judge(body: &[u8], w: &Want) -> Result<RelayDeposit, String> {
    let q: serde_json::Value = serde_json::from_slice(body).map_err(|e| format!("not JSON: {e}"))?;
    let steps = s(&q, &["steps"])?.as_array().ok_or("steps is not a list")?;
    if steps.len() != 1 {
        return Err(format!("{} steps: only a single deposit step is signed", steps.len()));
    }
    let step = &steps[0];
    if text(step, &["id"])? != "deposit" || text(step, &["kind"])? != "transaction" {
        return Err("the one step is not a deposit transaction".into());
    }
    let items = s(step, &["items"])?.as_array().ok_or("items is not a list")?;
    if items.len() != 1 {
        return Err(format!("{} items: exactly one transaction is signed", items.len()));
    }
    let d = s(&items[0], &["data"])?;
    if num_text(d, &["chainId"])? != w.chain_id as u128 {
        return Err("the deposit is for another chain".into());
    }
    let to = addr(text(d, &["to"])?)?;
    if to != RELAY_DEPOSITORY_4663 {
        return Err(format!("the deposit goes to 0x{}, not Relay's depository", hex::encode(to)));
    }
    if let Ok(from) = text(d, &["from"]) {
        if addr(from)? != w.our_evm {
            return Err("the quote was made for another sender".into());
        }
    }
    let value = num_text(d, &["value"])?;
    if value != w.amount_wei {
        return Err(format!("the deposit sends {value} wei, not {}", w.amount_wei));
    }
    let data = hex::decode(text(d, &["data"])?.trim_start_matches("0x")).map_err(|_| "calldata is not hex")?;
    if data.len() != 68 || data[..4] != RELAY_DEPOSIT_NATIVE {
        return Err("the calldata is not depositNative(address, bytes32)".into());
    }
    if data[4..16].iter().any(|b| *b != 0) || data[16..36] != w.our_evm {
        return Err("the calldata names another depositor".into());
    }
    let details = s(&q, &["details"])?;
    if text(details, &["recipient"])? != w.our_btc_address {
        return Err("the quote's recipient is not our own address".into());
    }
    let out = s(details, &["currencyOut"])?;
    if num_text(out, &["currency", "chainId"])? != RELAY_BITCOIN as u128 || text(out, &["currency", "address"])? != RELAY_BTC_NATIVE {
        return Err("the quote does not pay out native BTC".into());
    }
    let sats = num_text(out, &["amount"])?;
    let sats: u64 = sats.try_into().map_err(|_| "the amount out is out of range")?;
    if sats < w.floor_sats {
        return Err(format!("{sats} sats out is under our floor of {}", w.floor_sats));
    }
    Ok(RelayDeposit {
        to,
        value,
        deposit_id: data[36..68].try_into().expect("32 bytes"),
        data,
        request_id: text(step, &["requestId"]).unwrap_or_default().to_string(),
        sats_out: sats,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const REAL: &str = include_str!("../tests/fixtures/relay-quote-4663-btc-2026-09-23.json");
    const OURS: [u8; 20] = [0x5b, 0x0b, 0x06, 0x49, 0xf0, 0x30, 0x6d, 0x08, 0xb4, 0xc0, 0xad, 0x37, 0x40, 0x6a, 0x70, 0x82, 0xfc, 0x25,
        0xeb, 0xc2];
    const BTC: &str = "bc1p5cyxnuxmeuwuvkwfem96lqzszd02n6xdcjrs20cac6yqjjwudpxqkedrcr";

    fn want() -> Want<'static> {
        Want { chain_id: 4663, our_evm: OURS, our_btc_address: BTC, amount_wei: 100_000_000_000_000_000, floor_sats: 300_000 }
    }
    fn edit(f: impl Fn(&mut serde_json::Value)) -> Vec<u8> {
        let mut v: serde_json::Value = serde_json::from_str(REAL).unwrap();
        f(&mut v);
        serde_json::to_vec(&v).unwrap()
    }

    #[test]
    fn the_real_quote_passes_every_check_and_yields_the_deposit_to_sign() {
        let d = judge(REAL.as_bytes(), &want()).unwrap();
        assert_eq!((d.to, d.value, d.sats_out, d.data.len()), (RELAY_DEPOSITORY_4663, 100_000_000_000_000_000, 315_473, 68));
        assert_eq!(hex::encode(d.deposit_id), "a6e892a9d77d4c236bfdd5e1cabd0bae64883fbe6ac354653da65f1072c47475");
        assert!(d.request_id.starts_with("0x1790131616"));
    }

    #[test]
    fn every_checkable_lie_is_refused() {
        let w = want();
        let cases: Vec<(Vec<u8>, &str)> = vec![
            (edit(|v| v["details"]["recipient"] = "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4".into()), "recipient"),
            (edit(|v| v["steps"][0]["items"][0]["data"]["to"] = "0x000000000000000000000000000000000000beef".into()), "depository"),
            (edit(|v| v["steps"][0]["items"][0]["data"]["value"] = "200000000000000000".into()), "sends"),
            (edit(|v| v["steps"][0]["items"][0]["data"]["chainId"] = 1.into()), "another chain"),
            (edit(|v| v["steps"][0]["items"][0]["data"]["from"] = "0x000000000000000000000000000000000000beef".into()), "another sender"),
            (edit(|v| v["details"]["currencyOut"]["currency"]["chainId"] = 1.into()), "native BTC"),
            (edit(|v| { let s = v["steps"][0].clone(); v["steps"].as_array_mut().unwrap().push(s); }), "2 steps"),
            (edit(|v| { let d = v["steps"][0]["items"][0]["data"]["data"].as_str().unwrap().replace("5b0b0649", "deadbeef");
                v["steps"][0]["items"][0]["data"]["data"] = d.into(); }), "another depositor"),
        ];
        for (body, why) in cases {
            let e = judge(&body, &w).unwrap_err();
            assert!(e.contains(why), "{why}: {e}");
        }
        let high_floor = Want { floor_sats: 400_000, ..want() };
        assert!(judge(REAL.as_bytes(), &high_floor).unwrap_err().contains("under our floor"));
        assert!(judge(b"<html>", &w).unwrap_err().contains("not JSON"));
    }
}
