#!/usr/bin/env python3
"""Fix round 7 (D7-L-1): review7/poc/D's shallow provider turned around. golive-check's `depth_probe` (extracted verbatim from
script/golive-check.sh's INSTALL_PY) against local fakes on 127.0.0.1 and, with --live, the committed pair (read-only):
  shallow  answers [] for logs older than 10,000 blocks, serves blocks and receipts at any depth (review7/poc/D/fake_shallow_rpc.py,
           its head moved past the pinned range)                                  -> must FAIL (was 'ok')
  short    answers the pinned range with 9 of its 10 logs                         -> must FAIL
  norcpt   serves the logs, answers null for the pinned receipt                    -> must FAIL
  honest   serves the pinned range's 10 logs and its receipt                       -> 'ok'
Run from the repo root: python3 docs/audit/2026-09-27-nft/fixes-round7-poc/D/probe_depth_pinned.py [--live]"""
import json, re, sys, threading
from http.server import BaseHTTPRequestHandler, HTTPServer

src = open("script/golive-check.sh").read()
body = src[src.index("DEPTH_PROBE_BLOCKS = 60_000"):src.index("def texts(b):")]
ns = {"json": json, "re": re}
exec(body, ns)
depth_probe = ns["depth_probe"]
HEAD = 26_076_410
MIL, TR = ns["MILADY"], ns["TRANSFER"]


def serve(kind):
    class H(BaseHTTPRequestHandler):
        def log_message(self, *a): pass
        def do_POST(self):
            q = json.loads(self.rfile.read(int(self.headers["content-length"])))
            m, p = q["method"], q["params"]
            if m == "eth_blockNumber": r = hex(HEAD)
            elif m == "eth_getLogs":
                old = int(p[0]["toBlock"], 16) < HEAD - 10_000
                n = 0 if kind == "shallow" and old else 9 if kind == "short" else ns["PIN_LOGS"]
                r = [{"blockNumber": p[0]["toBlock"], "address": MIL, "topics": [TR]}] * n
            elif m == "eth_getBlockByNumber": r = {"number": p[0], "transactions": ["0x" + "11" * 32]}
            elif m == "eth_getTransactionReceipt":
                r = None if kind == "norcpt" else {"status": "0x1", "blockNumber": hex(ns["PIN_TX_BLOCK"]), "logs": [{"address": MIL, "topics": [TR]}]}
            else: r = None
            b = json.dumps({"jsonrpc": "2.0", "id": q["id"], "result": r}).encode()
            self.send_response(200); self.send_header("content-type", "application/json"); self.send_header("content-length", str(len(b)))
            self.end_headers(); self.wfile.write(b)
    s = HTTPServer(("127.0.0.1", 0), H)
    threading.Thread(target=s.serve_forever, daemon=True).start()
    return f"http://127.0.0.1:{s.server_address[1]}"


ok = True
for kind, want in [("shallow", False), ("short", False), ("norcpt", False), ("honest", True)]:
    r = depth_probe(serve(kind))
    good = r.startswith("ok") == want
    ok &= good
    print(f"{kind:8} -> {r!r}  {'as it must' if good else 'WRONG'}")
if "--live" in sys.argv:
    for u in ["https://rpc.mevblocker.io", "https://gateway.tenderly.co/public/mainnet"]:
        r = depth_probe(u)
        ok &= r.startswith("ok")
        print(f"live {u.split('/')[2]} -> {r!r}")
sys.exit(0 if ok else 1)
