#!/usr/bin/env python3
"""Spike S3 driver (docs/v9/canister-only-solana.md §15 S3). LOCAL replica + LOCAL solana-test-validator only.

   python3 -I drive.py <dfx> <out_dir> [--no-validator]

Needs: the `solmulti` canister installed on the local replica of this directory (run.sh does it), the Solana CLI tools on PATH
(solana, solana-keygen, solana-test-validator, spl-token). Starts its own solana-test-validator in <out_dir>/ledger (port 8899) and
stops it at the end. Throwaway keypairs only (payer, nonce, recipient), generated in <out_dir>, never printed.
Writes <out_dir>/report.json (the input of offline/: keys, transactions) and <out_dir>/summary.json (the measurements).
"""
import base64, json, os, signal, subprocess, sys, time, urllib.request

DFX, OUT = sys.argv[1], sys.argv[2]
NO_VALIDATOR = "--no-validator" in sys.argv
RPC = "http://127.0.0.1:8899"
HERE = os.path.dirname(os.path.abspath(__file__))
os.makedirs(OUT, exist_ok=True)


def num(v):
    return int(str(v).replace("_", ""))


def call(method, arg="()", query=False):
    cmd = [DFX, "canister", "call", "solmulti", method, arg, "--output", "json"] + (["--query"] if query else [])
    t0 = time.time()
    p = subprocess.run(cmd, capture_output=True, text=True, cwd=HERE)
    wall = time.time() - t0
    if p.returncode != 0:
        raise SystemExit(f"FAIL dfx call {method}: {p.stderr.strip()[-800:]}")
    v = json.loads(p.stdout)
    if isinstance(v, dict) and "Err" in v:
        raise SystemExit(f"FAIL {method}: {v['Err']}")
    return (v["Ok"] if isinstance(v, dict) and "Ok" in v else v), wall


def blob(path_elems):
    return "vec { " + "; ".join("blob \"" + "".join(f"\\{b:02x}" for b in e) + "\"" for e in path_elems) + " }"


def sh(*args, check=True):
    p = subprocess.run(list(args), capture_output=True, text=True)
    if check and p.returncode != 0:
        raise SystemExit(f"FAIL {' '.join(args[:3])}: {p.stdout[-400:]} {p.stderr[-400:]}")
    return p.stdout.strip()


def rpc(method, params):
    req = urllib.request.Request(RPC, json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode(),
                                 {"Content-Type": "application/json"})
    return json.loads(urllib.request.urlopen(req, timeout=20).read())


summary = {"pubkeys": [], "rounds": [], "bursts": [], "upgrade": {}}
report = {"keys": [], "txs": []}

# ---- 1. public keys per path (+ cost/latency) -----------------------------------------------------------------------------------
PATHS = [
    [],                                             # canister-level (for the offline derivation)
    [b"sol"],                                       # K_main
    [b"sol", b"intent", (7).to_bytes(8, "little")], # K_int[7]  Multi
    [b"sol", b"coll", (3).to_bytes(4, "little")],   # K_coll[3] Multi
    [b"sol/intent/7"],                              # K_int[7]  Flat
    [b"sol/coll/3"],                                # K_coll[3] Flat
    [b"sol-relay"],                                 # K_relay (exists in solkey.rs)
    [b"sol", b"intent", (8).to_bytes(8, "little")], # K_int[8]  Multi
]
print("== schnorr_public_key per path")
for p in PATHS:
    r, wall = call("pubkey", f"({blob(p)})")
    pk, cc = bytes(r["public_key"]), bytes(r["chain_code"])
    report["canister_id"] = r["canister_id"]
    entry = {"path": [e.hex() for e in p], "public_key": pk.hex(), "chain_code": cc.hex()}
    if p == []:
        report["root"] = entry
    else:
        report["keys"].append(entry)
    s = {"path": [e.decode() if e.isascii() and e.decode().isprintable() else "0x" + e.hex() for e in p], "address": r["address"],
         "latency_ms": num(r["latency_ms"]), "balance_delta": num(r["balance_delta"]), "wall_s": round(wall, 2)}
    summary["pubkeys"].append(s)
    print(f"   {s['path']}: {s['address']}  {s['latency_ms']} ms in canister, Δbalance {s['balance_delta']:,} cycles, {wall:.2f} s wall")
addr = {tuple(p): summary["pubkeys"][i]["address"] for i, p in enumerate(PATHS)}
K_MAIN = addr[(b"sol",)]
K_INT_M, K_COLL_M = addr[tuple(PATHS[2])], addr[tuple(PATHS[3])]
K_INT_F, K_COLL_F = addr[tuple(PATHS[4])], addr[tuple(PATHS[5])]


def multi(style, blockhash, nonce=None, mint=None, parallel=False, memo_signer=False, lamports=10_000_000, amount=1_000_000_000,
          intent=7, coll=3, memo="rook s3: open_mint intent 7"):
    arg = (f"(record {{ style = variant {{ {style} }}; intent = {intent} : nat64; coll = {coll} : nat32; blockhash = \"{blockhash}\"; "
           f"nonce_account = {('opt \"' + nonce + '\"') if nonce else 'null'}; lamports = {lamports} : nat64; "
           f"mint = {('opt record { \"' + mint[0] + '\"; \"' + mint[1] + '\" }') if mint else 'null'}; mint_amount = {amount} : nat64; "
           f"memo = \"{memo}\"; memo_signer = {str(memo_signer).lower()}; parallel = {str(parallel).lower()} }})")
    r, wall = call("multi_sign", arg)
    row = {"style": style, "parallel": parallel, "nonce": bool(nonce), "mint": bool(mint), "tx_bytes": r["tx_bytes"],
           "header": r["header"], "sign_total_ms": num(r["sign_total_ms"]), "sign_balance_delta": num(r["sign_balance_delta"]),
           "compile_instructions": num(r["compile_instructions"]), "wall_s": round(wall, 2),
           "signers": [{"role": s["role"], "address": s["address"], "cost_api": num(s["cost_api"]), "latency_ms": num(s["latency_ms"]),
                        "balance_delta": (num(s["balance_delta"][0]) if s["balance_delta"] else None)} for s in r["signers"]]}
    summary["rounds"].append(row)
    print(f"   {style} {'parallel' if parallel else 'sequential'}: {r['tx_bytes']} bytes, header {r['header']}, signing {row['sign_total_ms']} ms "
          f"(per sig {[s['latency_ms'] for s in row['signers']]}), Δbalance {row['sign_balance_delta']:,}, "
          f"cost_api {[s['cost_api'] for s in row['signers']]}, per-sig Δ {[s['balance_delta'] for s in row['signers']]}, {wall:.1f} s wall")
    return r, row


def submit(name, r, row):
    tx = {"name": name, "tx_base64": r["tx_base64"], "signers": [{"role": s["role"], "address": s["address"]} for s in r["signers"]]}
    report["txs"].append(tx)
    if NO_VALIDATOR:
        return tx
    res = rpc("sendTransaction", [r["tx_base64"], {"encoding": "base64", "preflightCommitment": "confirmed"}])
    if "error" in res:
        row["landed"] = False
        row["send_error"] = res["error"]
        print(f"   FAIL {name} sendTransaction: {json.dumps(res['error'])[:600]}")
        return tx
    sig = res["result"]
    t0 = time.time()
    while time.time() - t0 < 60:
        st = rpc("getSignatureStatuses", [[sig]])["result"]["value"][0]
        if st and st.get("confirmationStatus") in ("confirmed", "finalized"):
            row["landed"] = st.get("err") is None
            row["landed_err"] = st.get("err")
            row["signature"] = sig
            tx["landed"] = sig
            print(f"   {'PASS' if row['landed'] else 'FAIL'} {name} landed ({st['confirmationStatus']}, err={st.get('err')}) in "
                  f"{time.time() - t0:.1f} s: {sig}")
            return tx
        time.sleep(0.5)
    row["landed"] = False
    print(f"   FAIL {name}: not confirmed in 60 s ({sig})")
    return tx


validator = None
try:
    mint = mint_f = nonce = None
    if not NO_VALIDATOR:
        print("== local solana-test-validator (throwaway ledger in the out dir)")
        validator = subprocess.Popen(["solana-test-validator", "--ledger", os.path.join(OUT, "ledger"), "--reset", "--quiet"],
                                     stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
        for _ in range(60):
            try:
                if rpc("getHealth", []).get("result") == "ok":
                    break
            except Exception:
                pass
            time.sleep(1)
        else:
            raise SystemExit("FAIL the local validator did not come up")
        kp = {n: os.path.join(OUT, f"{n}.json") for n in ("payer", "nonce", "recipient")}
        for f in kp.values():
            sh("solana-keygen", "new", "--no-bip39-passphrase", "-s", "-f", "-o", f)
        U = ["-u", RPC, "--commitment", "confirmed"]
        payer = sh("solana-keygen", "pubkey", kp["payer"])
        recipient = sh("solana-keygen", "pubkey", kp["recipient"])
        for a in (payer, K_MAIN, K_INT_M, K_INT_F):
            sh("solana", "airdrop", "2", a, *U, "-k", kp["payer"])
        sh("solana", "create-nonce-account", kp["nonce"], "0.01", "--nonce-authority", K_MAIN, *U, "-k", kp["payer"])
        nonce = sh("solana-keygen", "pubkey", kp["nonce"])
        cfg = os.path.join(OUT, "cli-config.yml")  # the throwaway payer as the CLI's keypair: no default keypair is ever read
        sh("solana", "config", "set", "--config", cfg, "--keypair", kp["payer"], "--url", RPC)
        S = ["-C", cfg, "-u", RPC, "--fee-payer", kp["payer"]]

        def make_mint(authority):
            out = sh("spl-token", "create-token", "--decimals", "9", "--mint-authority", authority, *S)
            m = [l.split()[-1] for l in out.splitlines() if l.strip().startswith("Address:")][0]
            out = sh("spl-token", "create-account", m, "--owner", recipient, *S)
            ata = [l.split()[-1] for l in out.splitlines() if l.startswith("Creating account")][0]
            return m, ata

        mint = make_mint(K_COLL_M)
        mint_f = make_mint(K_COLL_F)
        summary["validator"] = {"nonce_account": nonce, "mint_multi": mint, "mint_flat": mint_f, "recipient": recipient}
        print(f"   nonce {nonce} (authority K_main); mint {mint[0]} (authority K_coll[3] Multi) → ATA {mint[1]}; "
              f"mint {mint_f[0]} (authority K_coll[3] Flat) → ATA {mint_f[1]}")

    def nonce_value():
        if NO_VALIDATOR:
            return K_MAIN  # any 32 bytes: signed, never submitted
        return sh("solana", "nonce", nonce, "-u", RPC, "--commitment", "confirmed")

    print("== the 3-signer transaction (advance nonce, transfer K_int → K_coll, MintTo by K_coll, Memo)")
    r, row = multi("Multi", nonce_value(), nonce=nonce if not NO_VALIDATOR else K_INT_M, mint=mint or (K_INT_M, K_INT_M))
    submit("R1 Multi sequential", r, row)
    r, row = multi("Multi", nonce_value(), nonce=nonce if not NO_VALIDATOR else K_INT_M, mint=mint or (K_INT_M, K_INT_M),
                   parallel=True, memo_signer=True, memo="rook s3: open_mint intent 7, parallel")
    submit("R2 Multi parallel memo-signer", r, row)
    r, row = multi("Flat", nonce_value(), nonce=nonce if not NO_VALIDATOR else K_INT_F, mint=mint_f or (K_INT_F, K_INT_F),
                   parallel=True, memo="rook s3: flat paths")
    submit("R3 Flat parallel", r, row)

    if not NO_VALIDATOR:
        bal = lambda a: rpc("getBalance", [a, {"commitment": "confirmed"}])["result"]["value"]
        tok = lambda a: rpc("getTokenAccountBalance", [a, {"commitment": "confirmed"}])["result"]["value"]["amount"]
        summary["after"] = {"K_coll_multi": bal(K_COLL_M), "K_coll_flat": bal(K_COLL_F), "ata_multi": tok(mint[1]), "ata_flat": tok(mint_f[1]),
                            "K_main": bal(K_MAIN), "K_int_multi": bal(K_INT_M), "K_int_flat": bal(K_INT_F)}
        print(f"   after: {summary['after']}  (expect K_coll 20,000,000 / 10,000,000 lamports; ATAs 2e9 / 1e9 base units)")
    print("== timing repeats (signed, not submitted)")
    for par in (False, True, False, True, False, True):
        multi("Multi", K_MAIN, nonce=K_INT_M, mint=(K_INT_M, K_INT_M), parallel=par, intent=100 + len(summary["rounds"]))

    print("== burst: N concurrent sign_with_schnorr from one message")
    for n in (10, 30, 60):
        r, wall = call("burst", f"({n} : nat32, variant {{ Multi }})")
        r = [r[k] for k in ("0", "1", "2", "3")] if isinstance(r, dict) else r
        summary["bursts"].append({"n": n, "ok": r[0], "failed": r[1], "ms": num(r[2]), "errors": r[3], "wall_s": round(wall, 1)})
        print(f"   n={n}: ok {r[0]}, failed {r[1]}, {num(r[2])} ms; errors {r[3][:2]}")
finally:
    if validator:
        os.killpg(validator.pid, signal.SIGTERM)
        try:
            validator.wait(15)
        except subprocess.TimeoutExpired:
            os.killpg(validator.pid, signal.SIGKILL)
        print("   local validator stopped")
    json.dump(report, open(os.path.join(OUT, "report.json"), "w"), indent=1)
    json.dump(summary, open(os.path.join(OUT, "summary.json"), "w"), indent=1)
