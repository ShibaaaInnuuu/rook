"""Spike Q3's scenario, against an installed `solsign` canister (run.sh creates, installs and deletes it). Devnet only.

  python3 -I drive.py <dfx> <network> <report.json> [--fund-wait <s>]

Steps (each printed PASS/FAIL/INFO; the verdict at the end):
  1. sol_address (threshold Ed25519 public key → base58)
  2. fund it on devnet: RPC requestAirdrop (1 SOL); on refusal the Solana CLI's airdrop; else wait for a manual faucet top-up
  3. provider agreement: getSlot(finalized) raw (bucket 1) and rounded (bucket 20), 3 times each
  4. create_nonce_account (signed with the FALLBACK lifetime), wait for finalized, read the durable nonce
  5. send_transfer(throwaway, 1,000,000) under the durable nonce; resend at once (same bytes); wait for finalized;
     resend after landing with preflight (expect blockhash_not_found: the nonce moved) and without (accepted, dropped);
     the recipient holds exactly 1,000,000: one landing
  6. fallback lifetime: send_transfer + finalized; then the expiry drill: hold_transfer (signed, saved, NOT sent), wait past
     lastValidBlockHeight, resend() → proven expiry → re-signed on a fresh blockhash → lands; the first signature never does
  7. metrics: cycles per signature (balance deltas and the cost API), latencies, outcall cycles
No private key is printed or written: the canister's key is threshold; the recipient is a throwaway keypair created and deleted here.
"""
import json, os, re, subprocess, sys, tempfile, time, urllib.request

DFX, NET, REPORT = sys.argv[1], sys.argv[2], sys.argv[3]
FUND_WAIT = int(sys.argv[sys.argv.index("--fund-wait") + 1]) if "--fund-wait" in sys.argv else 300
DEVNET = os.environ.get("SOL_RPC", "https://api.devnet.solana.com")  # the script's own reads and airdrops (not the canister's)
SOLBIN = os.path.expanduser("~/.local/share/solana/install/active_release/bin")
LAMPORTS = 1_000_000
R = {"network": NET, "steps": [], "started": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())}
ok_all = True


def log(kind, name, detail=""):
    global ok_all
    if kind == "FAIL":
        ok_all = False
    R["steps"].append({"kind": kind, "name": name, "detail": detail})
    print(f"{kind} {name}: {detail}", flush=True)


def num(x):
    return int(str(x).replace("_", "")) if x is not None else None


OPT = {"agreed", "failed", "durable_nonce_now", "balance_lamports", "signature", "durable_nonce", "last_valid_height", "refused"}


def unopt(v):
    """dfx prints `opt T` as [] or [x]: unwrap the known optional fields."""
    if isinstance(v, dict):
        return {k: (x[0] if x else None) if k in OPT and isinstance(x, list) and len(x) <= 1 else unopt(x) for k, x in v.items()}
    if isinstance(v, list):
        return [unopt(x) for x in v]
    return v


def call(method, arg="()", query=False):
    cmd = [DFX, "canister", "call", "solsign", method, arg, "--network", NET, "--output", "json"] + (["--query"] if query else [])
    t0 = time.time()
    p = subprocess.run(cmd, capture_output=True, text=True)
    wall = round((time.time() - t0) * 1000)
    if p.returncode != 0:
        err = "\n".join(l for l in p.stderr.splitlines() if "deprecated" not in l)
        return None, wall, err.strip()
    out = p.stdout.strip()
    return (unopt(json.loads(out)) if out and out != "()" else None), wall, None


def ok(method, arg="()"):
    v, wall, err = call(method, arg)
    if err:
        raise RuntimeError(f"{method}: {err}")
    if isinstance(v, dict) and "Err" in v:
        raise RuntimeError(f"{method}: {v['Err']}")
    return (v["Ok"] if isinstance(v, dict) and "Ok" in v else v), wall


def rpc(method, params):
    req = urllib.request.Request(DEVNET, json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode(),
                                 {"content-type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=20) as r:
            return json.loads(r.read())
    except urllib.error.HTTPError as e:
        return {"error": {"code": e.code, "message": e.read().decode(errors="replace")[:200]}}
    except Exception as e:  # noqa: BLE001
        return {"error": {"code": -1, "message": str(e)}}


def balance(addr, commitment="finalized"):
    r = rpc("getBalance", [addr, {"commitment": commitment}])
    return r.get("result", {}).get("value")


def wait_status(fn, want="finalized", limit=150):
    """Polls status()/sig_status() until finalized (or failed); returns (report, seconds, polls, disagreements seen)."""
    t0, polls, dis = time.time(), 0, []
    while time.time() - t0 < limit:
        polls += 1
        st, _ = fn()
        if st.get("agreed") and "disagree" in st["agreed"]:
            dis.append(st["providers"])
        if st.get("finalized") or st.get("failed"):
            return st, round(time.time() - t0, 1), polls, dis
        time.sleep(4)
    return st, round(time.time() - t0, 1), polls, dis


def throwaway_pubkey():
    keygen = os.path.join(SOLBIN, "solana-keygen")
    if os.access(keygen, os.X_OK):
        d = tempfile.mkdtemp()
        f = os.path.join(d, "k.json")
        subprocess.run([keygen, "new", "--no-bip39-passphrase", "-s", "-o", f], capture_output=True, check=True)
        pk = subprocess.run([keygen, "pubkey", f], capture_output=True, text=True, check=True).stdout.strip()
        os.remove(f)
        os.rmdir(d)
        return pk, "solana-keygen (keypair deleted at once; only its public half is used)"
    alphabet = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
    n, s = int.from_bytes(os.urandom(32), "big"), ""
    while n:
        n, r = divmod(n, 58)
        s = alphabet[r] + s
    return s, "32 random bytes (no keypair)"


def main():
    # 1. identity
    ident, wall = ok("sol_address")
    addr = ident["address"]
    R["address"] = addr
    log("PASS", "sol_address", f"{addr} (key {ident['key_name']}, threshold Ed25519; {num(ident['latency_ms'])} ms in canister, {wall} ms wall)")

    # 2. provider agreement
    agree = {"1": [], "20": []}
    for _ in range(3):
        for b in ("1", "20"):
            v, _w, err = call("slot_agreement", f"({b} : nat64)")
            agree[b].append(v if v else {"error": err})
    R["slot_agreement"] = agree
    raw_ok = sum(1 for v in agree["1"] if v.get("agreed")) if agree["1"] else 0
    rnd_ok = sum(1 for v in agree["20"] if v.get("agreed"))
    log("INFO", "provider agreement (getSlot finalized)", f"raw: {raw_ok}/3 agreed; rounded to 20: {rnd_ok}/3 agreed; "
        f"examples raw={[v.get('providers') for v in agree['1']][:1]} rounded={[v.get('providers') for v in agree['20']][:1]}")

    # 3. funding
    bal = balance(addr) or 0
    if bal < 50_000_000:
        a = rpc("requestAirdrop", [addr, 1_000_000_000])
        if "result" in a:
            log("INFO", "airdrop", f"requestAirdrop 1 SOL: {a['result']}")
        else:
            log("INFO", "airdrop", f"requestAirdrop REFUSED (rate limit?): {json.dumps(a.get('error'))[:200]}")
            cli = os.path.join(SOLBIN, "solana")
            if os.access(cli, os.X_OK):
                p = subprocess.run([cli, "airdrop", "1", addr, "-u", "devnet"], capture_output=True, text=True)
                log("INFO", "airdrop (CLI)", (p.stdout + p.stderr).strip()[:200])
        t0 = time.time()
        while time.time() - t0 < FUND_WAIT:
            bal = balance(addr) or 0
            if bal >= 50_000_000:
                break
            if int(time.time() - t0) % 60 < 5:
                print(f"   waiting for {addr} to hold ≥ 0.05 devnet SOL (finalized); fund it at https://faucet.solana.com if the airdrop was refused", flush=True)
            time.sleep(5)
    R["funded_lamports"] = bal
    if bal < 50_000_000:
        log("INFO", "funding", f"{addr} holds {bal} lamports after {FUND_WAIT} s: the devnet faucet refused (rate limit / dry); "
            "running the UNFUNDED proof instead (fund it by hand and re-run for the landing half)")
        R["unfunded"] = True
        return unfunded(addr)
    log("PASS", "funding", f"{bal} lamports at finalized")
    if os.environ.get("SOLSIGN_ONLY") == "drill":  # the expiry drill alone (a short run, e.g. on a disk-starved local validator)
        return drill(*throwaway_pubkey(), base=0)

    # 4. nonce account
    nr, wall = ok("create_nonce_account")
    R["nonce_create"] = nr
    nonce_acct = nr["nonce_account"]
    if nr["created_now"]:
        sig = nr["signature"]
        log("INFO", "create_nonce_account sent", f"{nonce_acct} rent {num(nr['rent_lamports'])} lamports, {num(nr['tx_bytes'])} bytes, sig {sig}, providers {nr['providers']}, {wall} ms wall")
        st, secs, polls, dis = wait_status(lambda: ok("sig_status", f'("{sig}")'))
        R["nonce_create_status"] = {"status": st, "seconds": secs, "polls": polls, "disagreements": dis}
        if not st.get("finalized"):
            log("FAIL", "nonce account finalized", json.dumps(st)[:300])
            return
        log("PASS", "nonce account finalized", f"{secs} s after send ({polls} polls; {len(dis)} polls refused for provider disagreement)")
        time.sleep(2)
        nr, _ = ok("create_nonce_account")
    dn = nr["durable_nonce"]
    log("PASS", "durable nonce readable", f"{nonce_acct} → {dn}")

    to, how = throwaway_pubkey()
    R["recipient"] = {"pubkey": to, "how": how}
    log("INFO", "recipient", f"{to} ({how})")

    # 5. durable-nonce transfer, same bytes resent
    s1, wall = ok("send_transfer", f'("{to}", {LAMPORTS} : nat64)')
    R["nonce_send"] = s1
    log("PASS" if s1["tx_bytes"] and num(s1["tx_bytes"]) <= 1232 else "FAIL", "send_transfer (durable nonce)",
        f"sig {s1['signature']}, {num(s1['tx_bytes'])} bytes (≤ 1,232), sha256 {s1['tx_sha256'][:16]}…, blockhash field = nonce {s1['recent_blockhash']}, "
        f"providers {s1['providers']}, {num(s1['latency_ms'])} ms in canister / {wall} ms wall")
    r1, wall = ok("resend", "(opt true)")
    same = r1["tx_sha256"] == s1["tx_sha256"] and not r1["resigned"]
    log("PASS" if same else "FAIL", "resend before landing (same bytes)", f"sha256 equal: {same}; providers {r1['providers']}; {wall} ms wall")
    st, secs, polls, dis = wait_status(lambda: ok("status"))
    R["nonce_status"] = {"status": st, "seconds": secs, "polls": polls, "disagreements": dis}
    log("PASS" if st.get("finalized") else "FAIL", "durable-nonce transfer finalized",
        f"{secs} s after the resend ({polls} polls, {len(dis)} refused for disagreement); nonce now {st.get('durable_nonce_now')}")
    r2, _ = ok("resend", "(opt true)")
    r3, _ = ok("resend", "(opt false)")
    R["resend_after_landing"] = {"preflight": r2, "no_preflight": r3}
    log("PASS" if r2["tx_sha256"] == s1["tx_sha256"] else "FAIL", "resend after landing (same bytes)",
        f"with preflight: {r2['providers']}; without: {r3['providers']}")
    time.sleep(20)
    got = balance(to)
    log("PASS" if got == LAMPORTS else "FAIL", "at most one mined", f"recipient holds {got} lamports at finalized (want {LAMPORTS}) after 4 sends of the same bytes")

    # 6. fallback lifetime
    ok("set_mode", "(variant { Blockhash })")
    f1, wall = ok("send_transfer", f'("{to}", {LAMPORTS} : nat64)')
    R["fallback_send"] = f1
    lv = num(f1["last_valid_height"])
    log("PASS", "send_transfer (fallback: rounded slot → getBlock)", f"sig {f1['signature']}, {num(f1['tx_bytes'])} bytes, blockhash {f1['recent_blockhash']}, "
        f"lastValidBlockHeight {lv}, providers {f1['providers']}, {wall} ms wall")
    st, secs, polls, dis = wait_status(lambda: ok("status"))
    R["fallback_status"] = {"status": st, "seconds": secs, "polls": polls, "disagreements": dis}
    log("PASS" if st.get("finalized") else "FAIL", "fallback transfer finalized", f"{secs} s ({polls} polls, {len(dis)} refused for disagreement)")

    drill(to, how, base=2 * LAMPORTS)


def drill(to, how, base):
    ok("set_mode", "(variant { Blockhash })")
    h1, _ = ok("hold_transfer", f'("{to}", {LAMPORTS} : nat64)')
    R["drill_hold"] = h1
    lv = num(h1["last_valid_height"])
    log("INFO", "expiry drill: signed and saved, NOT sent", f"sig {h1['signature']}, lastValidBlockHeight {lv}")
    t0 = time.time()
    while time.time() - t0 < 240:
        h = rpc("getBlockHeight", [{"commitment": "finalized"}]).get("result", 0)
        if h > lv + 20:
            break
        time.sleep(5)
    log("INFO", "expiry drill: waited", f"{round(time.time() - t0)} s until finalized height {h} > {lv} + 20 (the canister rounds heights down to 20)")
    d1, wall = ok("resend", "(opt true)")
    R["drill_resend"] = d1
    log("PASS" if d1["resigned"] and d1["signature"] != h1["signature"] else "FAIL", "expiry proven → re-signed same intent",
        f"intent {num(d1['intent'])}, history {d1['history']}, new blockhash {d1['recent_blockhash']}, providers {d1['providers']}, {wall} ms wall")
    st, secs, polls, dis = wait_status(lambda: ok("status"))
    first, _ = ok("sig_status", f'("{h1["signature"]}")')
    R["drill_status"] = {"new": st, "old": first}
    log("PASS" if st.get("finalized") and "null" in (first.get("agreed") or "") else "FAIL", "exactly one of the two signatures landed",
        f"new {st.get('agreed')} after {secs} s; old {first.get('agreed')}")
    time.sleep(15)
    got = balance(to)
    log("PASS" if got == base + LAMPORTS else "FAIL", "recipient total", f"{got} lamports (want {base + LAMPORTS}: earlier transfers + the drill's one landing)")


def unfunded(addr):
    """Without lamports nothing lands, but the RPC verifies signatures BEFORE it simulates: `account_not_found` from sendTransaction's
    preflight proves the threshold-Ed25519 signature and the bytes valid on devnet. The fallback's expiry drill needs no funds."""
    global ok_all
    nr, wall = ok("create_nonce_account")
    R["nonce_create"] = nr
    cls = [json.loads(p).get("class") if p.startswith("{") else p for p in nr["providers"]]
    good = all(c == "account_not_found" for c in cls)
    log("PASS" if good else "FAIL", "create_nonce_account signed + sent (unfunded)",
        f"{nr['nonce_account']}, {num(nr['tx_bytes'])} bytes, sig {nr['signature']}; providers {nr['providers']} "
        f"({'signature verified by both providers before simulation' if good else 'NOT the expected class'}); {wall} ms wall")
    to, how = throwaway_pubkey()
    ok("set_mode", "(variant { Blockhash })")
    h1, _ = ok("hold_transfer", f'("{to}", {LAMPORTS} : nat64)')
    R["drill_hold"] = h1
    r1, _ = ok("resend", "(opt true)")
    r2, _ = ok("resend", "(opt true)")
    same = h1["tx_sha256"] == r1["tx_sha256"] == r2["tx_sha256"] and not r1["resigned"] and not r2["resigned"]
    log("PASS" if same else "FAIL", "saved bytes resent unchanged (twice)", f"sha256 {h1['tx_sha256'][:16]}… ×3; providers {r1['providers']}")
    lv = num(h1["last_valid_height"])
    t0 = time.time()
    h = 0
    while time.time() - t0 < 300:
        h = rpc("getBlockHeight", [{"commitment": "finalized"}]).get("result", 0)
        if h > lv + 20:
            break
        time.sleep(5)
    log("INFO", "expiry drill: waited", f"{round(time.time() - t0)} s until finalized height {h} > {lv} + 20")
    d1, wall = ok("resend", "(opt true)")
    R["drill_resend"] = d1
    log("PASS" if d1["resigned"] and d1["signature"] != h1["signature"] and d1["history"] == [h1["signature"], d1["signature"]] else "FAIL",
        "expiry proven → re-signed the same intent", f"intent {num(d1['intent'])}, history {d1['history']}, new blockhash {d1['recent_blockhash']}, "
        f"providers {d1['providers']}, {wall} ms wall")
    d2, _ = ok("resend", "(opt true)")
    log("PASS" if d2["tx_sha256"] == d1["tx_sha256"] and not d2["resigned"] else "FAIL", "not re-signed again while the new bytes are live",
        f"sha256 {d2['tx_sha256'][:16]}…")
    R["partial"] = True


try:
    main()
except Exception as e:  # noqa: BLE001
    log("FAIL", "scenario stopped", str(e)[:800])
finally:
    m, _w, err = call("metrics", query=True)
    if m:
        R["metrics"] = m
        signs = m["signs"]
        for s in signs:
            print(f"   sign: {num(s['message_bytes'])} B message, cost API {num(s['cost_api']):,} cycles, balance delta {num(s['balance_delta']):,}, {num(s['latency_ms'])} ms")
        if signs:
            d = [num(s["balance_delta"]) for s in signs]
            l = sorted(num(s["latency_ms"]) for s in signs)
            log("INFO", "cycles per signature", f"{len(d)} signatures: balance delta min {min(d):,} / max {max(d):,}; cost API {num(signs[0]['cost_api']):,}; "
                f"latency min {l[0]} / median {l[len(l) // 2]} / max {l[-1]} ms")
        by = {}
        for c in m["calls"]:
            by.setdefault(c["method"], []).append((num(c["latency_ms"]), num(c["cycles"])))
        for k, v in by.items():
            ls = sorted(x[0] for x in v)
            print(f"   outcalls {k}: {len(v)} calls (each to {m['calls'][0]['providers']} providers), latency median {ls[len(ls) // 2]} ms max {ls[-1]} ms, "
                  f"cycles median {sorted(x[1] for x in v)[len(v) // 2]:,}")
    R["pass"] = ok_all and not R.get("partial")
    json.dump(R, open(REPORT, "w"), indent=1)
    if R.get("partial") and ok_all:
        print("SPIKE Q3: PARTIAL (unfunded: signing, bytes, sends, resends and the expiry drill proven; nothing could land)")
        sys.exit(3)
    print("SPIKE Q3: PASS" if ok_all else "SPIKE Q3: FAIL")
    sys.exit(0 if ok_all else 1)
