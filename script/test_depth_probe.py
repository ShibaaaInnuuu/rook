"""Offline tests of script/depth_probe.py (review round 7, D7-L-1; round 8, B8-L-1): fake JSON-RPC providers on 127.0.0.1, no network.

    python3 -m unittest discover -s script -p 'test_depth_probe.py'
"""
import json
import threading
import unittest
from http.server import BaseHTTPRequestHandler, HTTPServer

import depth_probe as dp

HEAD = dp.PIN_TO + 64_000


def logs(n, salt=""):
    return [{"transactionHash": f"0x{(salt + str(i)).rjust(64, '0')}", "logIndex": hex(i), "address": dp.MILADY,
             "topics": [dp.TRANSFER]} for i in range(n)]


RECEIPT = {"blockNumber": hex(dp.PIN_TX_BLOCK), "status": "0x1", "logs": [{"address": dp.MILADY, "topics": [dp.TRANSFER]}]}


class Fake:
    """A provider: `logs` answers eth_getLogs, `receipt` eth_getTransactionReceipt, `head` eth_blockNumber; `err` an RPC error."""

    def __init__(self, logs=None, receipt=RECEIPT, head=HEAD, err=None, fail_first=0):
        self.logs, self.receipt, self.head, self.err, self.fail_first, self.calls = logs if logs is not None else globals()["logs"](10), receipt, head, err, fail_first, 0

    def answer(self, m):
        self.calls += 1
        if self.calls <= self.fail_first:
            return {"jsonrpc": "2.0", "id": 1, "error": {"code": -32005, "message": "busy"}}
        if self.err and m == "eth_getLogs":
            return {"jsonrpc": "2.0", "id": 1, "error": self.err}
        res = {"eth_blockNumber": hex(self.head), "eth_getLogs": self.logs, "eth_getTransactionReceipt": self.receipt}[m]
        return {"jsonrpc": "2.0", "id": 1, "result": res}


def serve(fake):
    class H(BaseHTTPRequestHandler):
        def do_POST(self):
            body = json.loads(self.rfile.read(int(self.headers["content-length"])))
            out = json.dumps(fake.answer(body["method"])).encode()
            self.send_response(200); self.send_header("content-type", "application/json"); self.end_headers(); self.wfile.write(out)

        def log_message(self, *a):
            pass
    s = HTTPServer(("127.0.0.1", 0), H)
    threading.Thread(target=s.serve_forever, daemon=True).start()
    return s, f"http://127.0.0.1:{s.server_address[1]}/some/secret/path"


class DepthProbe(unittest.TestCase):
    def setUp(self):
        self.servers = []

    def tearDown(self):
        for s in self.servers: s.shutdown(); s.server_close()

    def url(self, fake):
        s, u = serve(fake)
        self.servers.append(s)
        return u

    def test_an_honest_pair_passes_and_agrees(self):
        r = dp.probe([self.url(Fake()), self.url(Fake())])
        self.assertTrue(r["ok"], r)
        self.assertTrue(r["agree"])
        self.assertEqual(r["providers"][0]["depth"], HEAD - dp.PIN_TO)

    def test_an_empty_answer_past_retention_fails(self):
        # D7-L-1: a provider answering [] past its log retention must not look deep
        v = dp.depth_probe(self.url(Fake(logs=[])))
        self.assertIn("returned 0 logs, not the 10", v)
        self.assertFalse(dp.probe([self.url(Fake()), self.url(Fake(logs=[]))])["ok"])

    def test_a_short_answer_fails(self):
        self.assertIn("returned 9 logs", dp.depth_probe(self.url(Fake(logs=logs(9)))))

    def test_a_null_receipt_fails(self):
        self.assertIn("is null", dp.depth_probe(self.url(Fake(receipt=None))))

    def test_a_deep_refusal_fails(self):
        v = dp.depth_probe(self.url(Fake(err={"code": -32602, "message": "Archive requests require a personal token"})))
        self.assertIn("eth_getLogs", v)
        self.assertFalse(v.startswith("ok"))

    def test_two_providers_with_different_logs_do_not_agree(self):
        r = dp.probe([self.url(Fake()), self.url(Fake(logs=logs(10, salt="f")))])
        self.assertTrue(all(p["ok"] for p in r["providers"]))
        self.assertFalse(r["agree"])
        self.assertFalse(r["ok"])

    def test_a_head_too_low_fails(self):
        self.assertIn("a head that low is wrong", dp.depth_probe(self.url(Fake(head=dp.PIN_TO + 10))))

    def test_a_transient_error_is_retried_by_the_cli_only(self):
        f = Fake(fail_first=1)
        u = self.url(f)
        self.assertFalse(dp.depth_probe(u).startswith("ok"))   # golive-check's single read: no retry
        f.calls = 0
        r = dp.probe([u], attempts=2, pause=0)
        self.assertTrue(r["ok"], r)
        self.assertEqual(r["providers"][0]["attempts"], 2)

    def test_a_url_is_printed_as_scheme_and_host_only(self):
        r = dp.probe([self.url(Fake())])
        self.assertNotIn("secret", json.dumps(r))
        self.assertEqual(dp.shown("https://api-ethereum-mainnet.n.dwellir.com/KEY123"), "https://api-ethereum-mainnet.n.dwellir.com")

    def test_the_default_pair_is_the_committed_one(self):
        self.assertEqual(dp.CANISTER_PAIR, ("https://rpc.mevblocker.io", "https://gateway.tenderly.co/public/mainnet"))


if __name__ == "__main__":
    unittest.main()
