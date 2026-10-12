//! C5-I-1 / C7-L-1 (docs/v7/phase1-nft.md §5.1; ops/rehearsal/relay/README.md): would the canister's payout judge credit this real
//! Relay payout? Reads a Relay payout (a fill or a refund) and, optionally, the deposit that opened its order, as the JSON-RPC answers
//! of `eth_getTransactionByHash` and `eth_getTransactionReceipt`, and runs them through the canister's OWN code: `relayconv::
//! decode_execute`, `call_request_hash`, `call_executed` and `ethflow::judge_payout`. Offline and read-only: no network, no key.
//!
//!   cargo run --example relay_verify -- --us 0x<the address the payout should pay> --payout payout.json [--deposit deposit.json]
//!       [--named <wei>] [--order-value <wei>] [--order-id 0x<32 bytes>] [--request-id 0x<32 bytes>]
//!
//! Each file is `{ "chain": 1 | 4663, "tx": <eth_getTransactionByHash result>, "receipt": <eth_getTransactionReceipt result> }`
//! (ops/rehearsal/relay/verify.mjs writes them). With `--deposit` the order id and its value come from the deposit's
//! `depositNative(depositor, id)` call and its value, and the deposit's block is the order's start on its chain; without it they are
//! the flags (a real execute that is not ours: `--order-value` defaults to what the payout pays `--us`, so only the shape is judged).
//! This file is an example binary: it is never part of buyer.wasm.
use etch_autobuyer::ethflow::{judge_payout, LateOrder, PayoutTx, Watch};
use etch_autobuyer::ethnft::LogV;
use etch_autobuyer::relayconv as rc;
use serde_json::Value;

const HOME: u64 = 4663;

fn unhex(s: &str) -> Result<Vec<u8>, String> {
    hex::decode(s.trim_start_matches("0x")).map_err(|_| format!("{s:.20}… is not hex"))
}

fn fixed<const N: usize>(s: &str, what: &str) -> Result<[u8; N], String> {
    unhex(s)?.try_into().map_err(|_| format!("{what} is not {N} bytes"))
}

fn qty(v: &Value, k: &str) -> Result<u128, String> {
    let s = v.get(k).and_then(Value::as_str).ok_or(format!("no {k}"))?;
    u128::from_str_radix(s.trim_start_matches("0x"), 16).map_err(|_| format!("{k} is not a quantity"))
}

struct Leg {
    chain: u64,
    hash: [u8; 32],
    tx: PayoutTx,
    logs: Vec<LogV>,
}

fn leg(path: &str) -> Result<Leg, String> {
    let j: Value = serde_json::from_str(&std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?).map_err(|e| format!("{path}: {e}"))?;
    let chain = j.get("chain").and_then(Value::as_u64).ok_or(format!("{path}: no chain (1 or 4663)"))?;
    let (t, r) = (j.get("tx").ok_or(format!("{path}: no tx"))?, j.get("receipt").ok_or(format!("{path}: no receipt"))?);
    if t.is_null() || r.is_null() {
        return Err(format!("{path}: the provider has no such transaction or receipt"));
    }
    let block = qty(r, "blockNumber")? as u64;
    let success = r.get("status").and_then(Value::as_str) == Some("0x1");
    let hash = fixed::<32>(t.get("hash").and_then(Value::as_str).ok_or("no hash")?, "hash")?;
    let tx = PayoutTx::of(t, block, success)?;
    let mut logs = Vec::new();
    for l in r.get("logs").and_then(Value::as_array).cloned().unwrap_or_default() {
        let topics = l.get("topics").and_then(Value::as_array).cloned().unwrap_or_default().iter()
            .map(|x| fixed::<32>(x.as_str().unwrap_or(""), "topic")).collect::<Result<Vec<_>, _>>()?;
        logs.push(LogV { address: fixed::<20>(l.get("address").and_then(Value::as_str).unwrap_or(""), "log address")?, topics,
            data: unhex(l.get("data").and_then(Value::as_str).unwrap_or("0x"))? });
    }
    Ok(Leg { chain, hash, tx, logs })
}

fn eth(w: u128) -> String {
    format!("{w} wei ({:.9} ETH)", w as f64 / 1e18)
}

fn run() -> Result<bool, String> {
    let mut a = std::env::args().skip(1);
    let (mut us, mut payout, mut deposit, mut named, mut value, mut order, mut request) = (None, None, None, 0u128, None, None, None);
    while let Some(k) = a.next() {
        let mut v = || a.next().ok_or(format!("{k} needs a value"));
        match k.as_str() {
            "--us" => us = Some(fixed::<20>(&v()?, "--us")?),
            "--payout" => payout = Some(v()?),
            "--deposit" => deposit = Some(v()?),
            "--named" => named = v()?.parse().map_err(|_| "--named: wei")?,
            "--order-value" => value = Some(v()?.parse::<u128>().map_err(|_| "--order-value: wei")?),
            "--order-id" => order = Some(fixed::<32>(&v()?, "--order-id")?),
            "--request-id" => request = Some(fixed::<32>(&v()?, "--request-id")?),
            _ => return Err(format!("unknown argument {k}")),
        }
    }
    let us = us.ok_or("--us <address> is required (the address the payout should pay)")?;
    let p = leg(&payout.ok_or("--payout <file> is required")?)?;
    let home = p.chain == HOME;
    println!("payout 0x{} on chain {} (block {}, {})", hex::encode(p.hash), p.chain, p.tx.block, if p.tx.success { "success" } else { "REVERTED" });

    // the deposit that opened the order: depositNative(address depositor, bytes32 id) into the depository, its value the order's
    let mut w = Watch { order_id: [0u8; 32], value: 0, min_out: 0, deadline: 0, balance_after: 0, eth_before: 0, eth_out_before: 0,
        pin: None, solver: None, request_id: request, home_pin: None };
    if let Some(d) = deposit {
        let d = leg(&d)?;
        let inp = &d.tx.input;
        println!("deposit 0x{} on chain {} (block {}): from 0x{} to 0x{}, value {}", hex::encode(d.hash), d.chain, d.tx.block,
            hex::encode(d.tx.from), hex::encode(d.tx.to), eth(d.tx.value));
        if d.tx.to != rc::RELAY_DEPOSITORY_4663 || inp.len() != 68 || inp[..4] != rc::DEPOSIT_NATIVE {
            return Err("the deposit is not depositNative(address, bytes32) into Relay's depository 0x4cd0…bc31".into());
        }
        w.order_id = inp[36..68].try_into().expect("32");
        w.value = d.tx.value;
        println!("  depositNative(depositor 0x{}, order id 0x{})", hex::encode(&inp[16..36]), hex::encode(w.order_id));
        if d.chain == HOME { w.home_pin = Some(d.tx.block) } else { w.pin = Some(d.tx.block) }
    }
    if let Some(o) = order {
        w.order_id = o;
    }
    if w.order_id == [0u8; 32] {
        // no order named: a stand-in id no real input carries (an all-zero id would match any zero word of the input)
        w.order_id = [0x5a; 32];
        println!("(no --deposit or --order-id: the order id is a stand-in, so only the shape and the records are judged)");
    }

    // the fill's shape
    let t = &p.tx;
    let sel = t.input.get(..4).map(hex::encode).unwrap_or_else(|| "(none: a plain transfer)".into());
    println!("shape: from 0x{} to 0x{}, value {}, selector {sel}, {} input bytes", hex::encode(t.from), hex::encode(t.to), eth(t.value), t.input.len());
    let depository = t.to == rc::RELAY_DEPOSITORY_4663;
    let request_decoded = if depository { rc::decode_execute(&t.input) } else { None };
    if t.to == us && t.value > 0 {
        println!("  a PLAIN TRANSFER to --us (a solver's fill or refund): the judge credits it only if its input names the order or request id");
    } else if depository && request_decoded.is_some() {
        println!("  a call of the depository's execute(CallRequest, signature) (selector 0x2d9fb478): Relay's allocator-signed payout");
    } else if depository {
        println!("  a call to the depository that is NOT its execute (a deposit, or something else): never a payout");
    } else {
        println!("  neither a payment to --us nor a call to the depository: the judge refuses it (a payee-claimed withdrawal pays its sender, not us)");
    }
    if t.from == us {
        println!("  SENT BY --us: a withdrawal the payee had to claim itself; the canister signs no such call, so such a refund can never be credited");
    }
    let names = |id: &[u8; 32]| t.input.windows(32).any(|x| x == id);
    println!("  input carries the order id: {}; Relay's request id: {}", names(&w.order_id), request.map_or("(not given)".into(), |r| names(&r).to_string()));

    // the depository's own records
    let sh = request_decoded.as_ref().map(rc::call_request_hash);
    if let Some(r) = &request_decoded {
        println!("  request: {} call(s), nonce 0x{}, expiration 0x{}; EIP-712 struct hash 0x{}", r.calls.len(), hex::encode(r.nonce),
            hex::encode(r.expiration), hex::encode(sh.expect("hash")));
        for (i, c) in r.calls.iter().enumerate() {
            println!("    call {i}: to 0x{} value {} data {} bytes allowFailure {}", hex::encode(c.to), eth(c.value), c.data.len(), c.allow_failure);
        }
    }
    let mut to_us = Vec::new();
    let mut records = 0;
    for l in &p.logs {
        if l.topics.first() != Some(&rc::RELAY_CALL_EXECUTED) {
            continue;
        }
        records += 1;
        match rc::call_executed(l) {
            Some((id, to, v)) => {
                let from_dep = l.address == rc::RELAY_DEPOSITORY_4663;
                let matches = sh == Some(id);
                println!("  RelayCallExecuted (emitter 0x{}{}): id 0x{} {} the recomputed struct hash; pays 0x{} {}{}", hex::encode(l.address),
                    if from_dep { ", the depository" } else { ", NOT the depository" }, hex::encode(id), if matches { "EQUALS" } else { "is NOT" },
                    hex::encode(to), eth(v), if to == us { "  <- --us" } else { "" });
                if from_dep && matches && to == us && v > 0 {
                    to_us.push(v);
                }
            }
            None => println!("  RelayCallExecuted (emitter 0x{}) in a shape the canister does not read (not one topic with the id in the data)", hex::encode(l.address)),
        }
    }
    println!("  RelayCallExecuted records: {records}; of this request, from the depository, paying --us: {to_us:?}");

    // the verdict, by the canister's own judge
    let paid_us = if t.to == us { t.value } else { to_us.iter().copied().max().unwrap_or(0) };
    if w.value == 0 {
        w.value = value.unwrap_or(paid_us);
        println!("(no --deposit: the order's value is {}{})", eth(w.value), if value.is_none() { ", assumed = what it pays --us" } else { "" });
    } else if let Some(v) = value {
        w.value = v;
    }
    let o = LateOrder { watch: w, back: false, let_go_at: 0, credited: None, released_at: None };
    let unnamed = judge_payout(&o, t, &p.logs, &p.hash, home, &us, 0, &[], false);
    let stated = if named != 0 { named } else { to_us.first().copied().unwrap_or(0) };
    let with = (stated != 0).then(|| judge_payout(&o, t, &p.logs, &p.hash, home, &us, stated, &[], false));
    println!("judge_payout, no amount stated: {}", unnamed.as_ref().map_or_else(|e| format!("REFUSED: {e}"), |v| format!("CREDITED {}", eth(*v))));
    if let Some(r) = &with {
        println!("judge_payout, the operator stating {}: {}", eth(stated), r.as_ref().map_or_else(|e| format!("REFUSED: {e}"), |v| format!("CREDITED {}", eth(*v))));
    }
    let ok = unnamed.is_ok() || with.as_ref().is_some_and(|r| r.is_ok());
    println!("VERDICT: {}", if ok {
        if unnamed.is_ok() { "creditable as built (eth_credit_payout(<buy>, <chain>, <tx>, 0))" }
        else { "creditable as built only with the amount stated (eth_credit_payout(<buy>, <chain>, <tx>, <wei>)), from Relay's records" }
    } else { "NOT creditable as built (C7-L-1): record it; the judge must follow this shape before any order can be credited late" });
    Ok(ok)
}

fn main() {
    match run() {
        Ok(ok) => std::process::exit(if ok { 0 } else { 3 }),
        Err(e) => {
            eprintln!("refused: {e}");
            std::process::exit(2);
        }
    }
}
