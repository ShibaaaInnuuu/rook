# A provider that keeps no logs deeper than 10,000 blocks but answers an EMPTY LIST (not an error) for older ranges, and serves blocks
# and receipts at any depth (a pruned log index beside a full transaction index). golive-check --probe-depth accepts it.
import json, sys
from http.server import BaseHTTPRequestHandler, HTTPServer
HEAD = 23_500_000
class H(BaseHTTPRequestHandler):
    def log_message(self, *a): pass
    def do_POST(self):
        q = json.loads(self.rfile.read(int(self.headers["content-length"])))
        m, p = q["method"], q["params"]
        if m == "eth_blockNumber": r = hex(HEAD)
        elif m == "eth_getLogs": r = [] if int(p[0]["toBlock"], 16) < HEAD - 10_000 else [{"blockNumber": p[0]["toBlock"]}]
        elif m == "eth_getBlockByNumber": r = {"number": p[0], "transactions": ["0x" + "11" * 32]}
        elif m == "eth_getTransactionReceipt": r = {"status": "0x1", "logs": []}
        else: r = None
        b = json.dumps({"jsonrpc": "2.0", "id": q["id"], "result": r}).encode()
        self.send_response(200); self.send_header("content-type", "application/json"); self.send_header("content-length", str(len(b))); self.end_headers(); self.wfile.write(b)
HTTPServer(("127.0.0.1", int(sys.argv[1])), H).serve_forever()
