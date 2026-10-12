#!/usr/bin/env python3
# Review round 7 (part P), read-only JSON-RPC probe of the canister's Ethereum pair (rpc.mevblocker.io + Tenderly's public
# gateway): depth of logs and receipts, and the words of every honest error shape we can provoke with READS ONLY, each classified
# with a line-for-line copy of pin::error_class (a2d747ba). No eth_sendRawTransaction of any kind.
import json, time, socket, urllib.request
URLS = ["https://rpc.mevblocker.io", "https://gateway.tenderly.co/public/mainnet"]
MIL = "0x5af0d9827e0c53e4799bb226655a1de152a425a5"
T = "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef"
USDT = "0xdac17f958d2ee523a2206206994597c13d831ec7"
BUSY = ["rate limit", "rate-limit", "ratelimit", "too many requests", "request rate", "capacity", "busy", "throttl", "quota",
        "try again later", "error code: 1015"]
DEEP = ["archive", "pruned", "missing trie node", "historical state", "unknown state", "first available state", "personal token",
        "state is not available", "state not available"]
LIMIT = ["more than", "too many", "too large", "too wide", "limit exceeded", "exceeds", "exceeded", "response size", "block range",
         "range is too", "range too", "timeout", "timed out", "results"]
def error_class(words):
    l = words.lower()
    for name, ws in (("busy", BUSY), ("deep", DEEP), ("limit", LIMIT)):
        if any(w in l for w in ws):
            return name
    return None
def rpc(url, method, params, timeout=60):
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode()
    req = urllib.request.Request(url, data=body, headers={"content-type": "application/json", "user-agent": "rook-review7/1"})
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            raw, st = r.read(), r.status
    except urllib.error.HTTPError as e:
        raw, st = e.read(), e.code
    except Exception as e:
        return None, 0, str(e)[:160]
    try:
        return json.loads(raw), st, len(raw)
    except Exception:
        return {"_raw": raw[:200].decode(errors="replace")}, st, len(raw)
def show(j, st, n):
    if j is None:
        return f"transport error {n}"
    if "result" in j:
        r = j["result"]
        return f"HTTP {st} " + (f"ok {len(r)} ({n} B)" if isinstance(r, list) else ("null" if r is None else "ok"))
    e = j.get("error", j)
    words = (e.get("message", "") + " " + (e.get("data") if isinstance(e.get("data"), str) else "")) if isinstance(e, dict) else json.dumps(e)
    code = e.get("code") if isinstance(e, dict) else None
    return f"HTTP {st} ERR code={code} class={error_class(words)} words={json.dumps(words.strip())[:170]}"
print("# review7 probe", time.strftime("%Y-%m-%d %H:%M:%S UTC", time.gmtime()))
for h in ["rpc.mevblocker.io", "gateway.tenderly.co"]:
    fam = sorted({a[0].name for a in socket.getaddrinfo(h, 443)})
    print(f"DNS {h}: {fam}")
for u in URLS:
    j, _, _ = rpc(u, "eth_blockNumber", [])
    head = int(j["result"], 16)
    fj, _, _ = rpc(u, "eth_getBlockByNumber", ["finalized", False])
    print(f"\n## {u} head={head} finalized={int(fj['result']['number'], 16)}")
    for D in [30000, 60000, 80000]:
        print(f"  logs 500 blk Milady Transfer at depth {D}:", show(*rpc(u, "eth_getLogs", [{"fromBlock": hex(head - D - 499), "toBlock": hex(head - D), "address": MIL, "topics": [T]}])))
    for D in [30000, 60000]:
        bj, _, _ = rpc(u, "eth_getBlockByNumber", [hex(head - D), False])
        tx = bj["result"]["transactions"][0]
        print(f"  receipt at depth {D}:", show(*rpc(u, "eth_getTransactionReceipt", [tx])))
    cases = [
        ("USDT Transfer 1,000 blocks (result cap)", "eth_getLogs", [{"fromBlock": hex(head - 2000), "toBlock": hex(head - 1001), "address": USDT, "topics": [T]}]),
        ("Milady 1,000,000-block range", "eth_getLogs", [{"fromBlock": hex(head - 1_000_000), "toBlock": hex(head - 1), "address": MIL, "topics": [T]}]),
        ("fromBlock > toBlock", "eth_getLogs", [{"fromBlock": hex(head - 10), "toBlock": hex(head - 20), "address": MIL, "topics": [T]}]),
        ("toBlock past the head (+50)", "eth_getLogs", [{"fromBlock": hex(head - 10), "toBlock": hex(head + 50), "address": MIL, "topics": [T]}]),
        ("fromBlock past the head (+50..+60)", "eth_getLogs", [{"fromBlock": hex(head + 50), "toBlock": hex(head + 60), "address": MIL, "topics": [T]}]),
        ("malformed topic", "eth_getLogs", [{"fromBlock": hex(head - 10), "toBlock": hex(head - 1), "address": MIL, "topics": ["0x12"]}]),
        ("receipt of an unknown hash", "eth_getTransactionReceipt", ["0x" + "11" * 32]),
        ("receipt with a malformed hash", "eth_getTransactionReceipt", ["0x1234"]),
        ("unknown method", "eth_notAMethod", []),
    ]
    for name, m, p in cases:
        print(f"  {name}:", show(*rpc(u, m, p, timeout=90)))
        time.sleep(1)
