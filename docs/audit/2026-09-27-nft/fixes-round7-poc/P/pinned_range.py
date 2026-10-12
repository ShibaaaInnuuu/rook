#!/usr/bin/env python3
"""Fix round 7 (D7-L-1): pick the fixed historical range golive-check --probe-depth reads. Read-only JSON-RPC (eth_blockNumber,
eth_getLogs, eth_getTransactionReceipt) on the canister's Ethereum pair. For 500-block ranges of Milady `Transfer` logs about 62,000
blocks under the head, count the logs on each provider; the range picked must hold logs, the same on both, and one of its transactions'
receipts must be served (non-null, the block and the Transfer log in it)."""
import json, sys, urllib.request

PAIR = ["https://rpc.mevblocker.io", "https://gateway.tenderly.co/public/mainnet"]
MILADY = "0x5af0d9827e0c53e4799bb226655a1de152a425a5"
TRANSFER = "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef"


def rpc(u, m, p):
    req = urllib.request.Request(u, data=json.dumps({"jsonrpc": "2.0", "id": 1, "method": m, "params": p}).encode(),
                                 headers={"content-type": "application/json", "user-agent": "rook-fix7-probe"})
    with urllib.request.urlopen(req, timeout=60) as r:
        j = json.loads(r.read())
    if "error" in j:
        raise ValueError(f"{m}: {j['error']}")
    return j["result"]


head = int(rpc(PAIR[0], "eth_blockNumber", []), 16)
print(f"head (mevblocker) {head}")
start = int(sys.argv[1]) if len(sys.argv) > 1 else head - 64_000
for k in range(8):
    a = start + k * 500
    b = a + 499
    counts, first = [], None
    for u in PAIR:
        logs = rpc(u, "eth_getLogs", [{"address": MILADY, "topics": [TRANSFER], "fromBlock": hex(a), "toBlock": hex(b)}])
        counts.append(len(logs))
        if logs and first is None:
            first = (logs[0]["transactionHash"], int(logs[0]["blockNumber"], 16))
    print(f"blocks {a}..{b} ({head - b} under the head): Milady Transfer logs {counts}; first {first}")
    if counts[0] == counts[1] and counts[0] >= 3 and first:
        for u in PAIR:
            r = rpc(u, "eth_getTransactionReceipt", [first[0]])
            n = sum(1 for l in r["logs"] if l["address"].lower() == MILADY and l["topics"][0] == TRANSFER)
            print(f"  {u.split('/')[2]}: receipt of {first[0]}: block {int(r['blockNumber'], 16)}, status {r['status']}, "
                  f"{n} Milady Transfer log(s)")
        break
