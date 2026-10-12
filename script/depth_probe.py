#!/usr/bin/env python3
"""The canister's Ethereum depth probe (review round 6, B6-M-1; round 7, D7-L-1; round 8, B8-L-1), read-only.

One module for every caller, so the measurement cannot drift:
  - script/golive-check.sh --probe-depth imports `depth_probe` (the go-live read of the install argument's chain-1 providers);
  - .github/workflows/depth-probe.yml runs it daily against the canister's keyless pair (no secrets);
  - an operator runs it by hand:  python3 script/depth_probe.py [--json] [--attempts N] [<https url> ...]

Each provider must serve, NOW: the head; Milady's `Transfer` eth_getLogs over the FIXED range 26,012,410..26,012,909, which holds exactly
10 logs (an empty or short list fails: a provider answering [] past its log retention looks deep to an empty range, D7-L-1); and the
receipt of 0x17133e8b…c093 (block 26,012,500, status 1, one Milady Transfer). The CLI also requires every provider's 10 logs to be the
SAME logs (transaction hash and log index), as the canister requires its pair to agree. Only JSON-RPC reads: eth_blockNumber,
eth_getLogs, eth_getTransactionReceipt. No key is needed, none is read, and a URL is never printed beyond scheme://host.
"""
import json
import re
import sys
import time
import urllib.error
import urllib.request
from urllib.parse import urlsplit

DEPTH_PROBE_BLOCKS = 60_000
# review round 7 (D7-L-1): an EMPTY log list proves nothing (a provider answering [] past its log retention passed). The probe reads a
# FIXED range known to hold logs, measured on both providers of the pair 2026-09-28 (docs/audit/2026-09-27-nft/fixes-round7-poc/P/
# pinned_range.py): Milady's Transfer logs over Ethereum blocks 26,012,410..26,012,909 are exactly 10, and the receipt of one of them
# (block 26,012,500) is served with its Transfer log. It was 63,500 blocks deep then and only gets deeper; past PIN_REPIN_BLOCKS deep the
# probe says to pin a newer range (it asks more depth than the 7-day window needs)
PIN_FROM, PIN_TO, PIN_LOGS = 26_012_410, 26_012_909, 10
PIN_TX, PIN_TX_BLOCK = "0x17133e8bbc4d8f1fbdf25e1698229bbe9f732d085069b82dd03cd75cdd55c093", 26_012_500
PIN_REPIN_BLOCKS = 400_000
MILADY = "0x5af0d9827e0c53e4799bb226655a1de152a425a5"
TRANSFER = "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef"
# the canister's Ethereum pair as committed (docs/v7/phase1-nft.md §5.1; scripts/golive-dryrun.sh's install argument; golive-check's DEEP)
CANISTER_PAIR = ("https://rpc.mevblocker.io", "https://gateway.tenderly.co/public/mainnet")


def measure(u):
    """(verdict, facts): the verdict is "ok…" or why the provider failed; facts are what was read (head, depth, log keys)."""
    facts = {"provider": shown(u)}

    def rpc(m, params):
        req = urllib.request.Request(u, data=json.dumps({"jsonrpc": "2.0", "id": 1, "method": m, "params": params}).encode(),
            headers={"content-type": "application/json", "user-agent": "rook-golive-check"})
        try:
            with urllib.request.urlopen(req, timeout=40) as r: raw = r.read()
        except urllib.error.HTTPError as e:
            raw = e.read()
            try: json.loads(raw)
            except Exception: raise ValueError(f"{m}: HTTP {e.code}")
        j = json.loads(raw)
        if "error" in j: raise ValueError(f"{m}: {str(j['error'])[:120]}")
        return j.get("result")
    try:
        head = int(rpc("eth_blockNumber", []), 16)
        facts.update(head=head, depth=head - PIN_TO)
        if head - PIN_TO < DEPTH_PROBE_BLOCKS: return f"the pinned range is only {head - PIN_TO:,} blocks under this provider's head (a head that low is wrong)", facts
        logs = rpc("eth_getLogs", [{"fromBlock": hex(PIN_FROM), "toBlock": hex(PIN_TO), "address": MILADY, "topics": [TRANSFER]}])
        if not isinstance(logs, list): return f"eth_getLogs of the pinned range: no list", facts
        facts["logs"] = sorted(f"{(l.get('transactionHash') or '').lower()}:{int(l.get('logIndex') or '0x0', 16)}" for l in logs)
        if len(logs) != PIN_LOGS: return f"eth_getLogs of the pinned range ({head - PIN_TO:,} blocks deep) returned {len(logs)} logs, not the {PIN_LOGS} it holds (an empty or short answer is a provider that does not keep logs that deep)", facts
        r = rpc("eth_getTransactionReceipt", [PIN_TX])
        if not r: return f"the receipt of the pinned transaction ({head - PIN_TX_BLOCK:,} blocks deep) is null", facts
        if int(r.get("blockNumber", "0x0"), 16) != PIN_TX_BLOCK or r.get("status") != "0x1" or not any(
                (l.get("address") or "").lower() == MILADY and (l.get("topics") or [""])[0] == TRANSFER for l in r.get("logs") or []):
            return "the receipt of the pinned transaction is not the one measured (block, status or its Transfer log)", facts
        facts["receipt"] = {"block": PIN_TX_BLOCK, "status": 1}
        return ("ok" if head - PIN_TO <= PIN_REPIN_BLOCKS else f"ok (note: the pinned range is {head - PIN_TO:,} blocks deep, more than the window needs: pin a newer one)"), facts
    except Exception as e:
        return re.sub(r"https?://\S+", "<url>", str(e))[:160], facts


def depth_probe(u):
    """golive-check's probe: "ok…" or why the provider fails (the text golive-check prints)."""
    return measure(u)[0]


def shown(u):
    """scheme://host only: a keyed provider's key lives in the path or the query, and is never printed."""
    try:
        p = urlsplit(u)
        return f"{p.scheme}://{p.hostname}" if p.hostname else "<not a URL>"
    except ValueError:
        return "<not a URL>"


def probe(urls, attempts=1, pause=20):
    """Every provider measured (a failure retried up to `attempts` times, for a transient network error), then their logs compared."""
    rows = []
    for u in urls:
        for i in range(max(1, attempts)):
            verdict, facts = measure(u)
            if verdict.startswith("ok") or i + 1 == attempts: break
            time.sleep(pause)
        rows.append({"verdict": verdict, "ok": verdict.startswith("ok"), "attempts": i + 1, **facts})
    keys = {tuple(r.get("logs") or ()) for r in rows if r["ok"]}
    agree = len(keys) <= 1
    return {"ok": bool(rows) and all(r["ok"] for r in rows) and agree, "agree": agree, "providers": rows,
            "range": [PIN_FROM, PIN_TO], "logs_expected": PIN_LOGS, "receipt": PIN_TX,
            "at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())}


def main(argv):
    as_json, attempts, urls = False, 1, []
    it = iter(argv)
    for a in it:
        if a == "--json": as_json = True
        elif a == "--attempts": attempts = int(next(it, "1"))
        elif a in ("-h", "--help"): print(__doc__); return 0
        elif a.startswith("https://") or a.startswith("http://"): urls.append(a)
        else: print(f"refused: unknown argument {a!r} (a provider URL, --json, --attempts N)"); return 2
    r = probe(urls or list(CANISTER_PAIR), attempts)
    if as_json:
        print(json.dumps(r, indent=2))
    else:
        for p in r["providers"]:
            depth = f"{p['depth']:,} blocks deep, head {p['head']:,}" if "depth" in p else "no head"
            print(f"{'PASS' if p['ok'] else 'FAIL'} {p['provider']}: {p['verdict']} ({depth}; logs {len(p.get('logs') or [])}/{PIN_LOGS}; attempts {p['attempts']})")
        print(f"{'PASS' if r['agree'] else 'FAIL'} the providers' logs of the pinned range are {'the same' if r['agree'] else 'NOT the same'} (transaction hash and log index)")
        print(f"DEPTH PROBE: {'PASS' if r['ok'] else 'FAIL'} ({r['at']}; Milady Transfer {PIN_FROM:,}..{PIN_TO:,}, receipt {PIN_TX[:10]}…{PIN_TX[-4:]})")
    return 0 if r["ok"] else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
