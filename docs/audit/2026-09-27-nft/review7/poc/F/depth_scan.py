# Review round 7, part F: read-only. The deepest block (under the head) at which rpc.mainnet.chain.robinhood.com answers
# eth_getTransactionCount, sampled 6 times 60 s apart (is the reach a sliding window or a sawtooth?). Nothing is sent.
import json, time, urllib.request
R = "https://rpc.mainnet.chain.robinhood.com"
def q(m, p):
    r = urllib.request.Request(R, json.dumps({"jsonrpc": "2.0", "id": 1, "method": m, "params": p}).encode(), {"content-type": "application/json", "user-agent": "curl/8"})
    return json.load(urllib.request.urlopen(r, timeout=20))
for k in range(6):
    h = int(q("eth_blockNumber", [])["result"], 16)
    lo, hi = 3000, 12000   # lo served, hi not
    while hi - lo > 25:
        mid = (lo + hi) // 2
        ok = "result" in q("eth_getTransactionCount", ["0x0000000000000000000000000000000000000000", hex(h - mid)])
        lo, hi = (mid, hi) if ok else (lo, mid)
    print(time.strftime("%H:%M:%S", time.gmtime()), "head", h, "state reach", lo, "..", hi, "blocks", flush=True)
    time.sleep(60)
