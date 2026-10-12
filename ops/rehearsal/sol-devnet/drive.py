"""The canister half of the Solana devnet rehearsal (docs/v9/rehearsal-sol-devnet.md §4), against an installed throwaway buyer
(`rooksol`, created, installed and deleted by run.sh). DEVNET ONLY; the Bitcoin side is signet/testnet4; Ethereum is only read.

  python3 -I drive.py addresses <dfx> <network> <job dir>
  python3 -I drive.py checklist <dfx> <network> <job dir> --tool "<rook-devnet command prefix>" --wasm <gz> --arg <init hex file>
         [--fund-wait <s>] [--eth-floor-sol 8] [--frog-floor-sol 1.5] [--skip-eth] [--frog-offer <json>] [--host <ic url>] [--local]

`addresses` writes <job>/canister.json (the canister's Solana address, its taproot custody, float and Ethereum addresses): the
listings name them (custodian, custody) before the checklist runs.

The checklist (phase B2's items, each PASS / FAIL / INFO / SKIP, the verdict at the end, the report in <job>/drive-report.json):
   1 genesis: a tick on the Solana home agrees devnet's genesis hash through both providers
   2 unfunded: the first home step's simulation is refused for want of funds (no signature spent), the line holds the error,
     the monitor pages it (SEV1) - and the cycles a refused simulation costs
   3 funded (payer -> the canister's Solana address, or a faucet top-up): the line opens its durable-nonce account
   4 simulate / sign / land through the nonce line: an advance lands, finalized
   5 ETH and frog steps on ONE line: both flows' steps in the line's outcomes, in intent order
   6 the keeper's floor post (an operator keypair as the keeper service, D-a); the canister's own frog reading is dropped
   7 the frog advance (real) -> the Relay SOL->BTC leg stubbed (no devnet: relay_give_back) -> return_advance (real); mint_item with
     a proof only with --frog-offer (a real signet purchase), else SKIP (the program drill ran mint_item with a proof on this cluster)
   8 a ticket to a test taproot address: note_sent / record_delivery / mark_undeliverable by the CANISTER need a frog it minted
     (item 7's offer); else SKIP (the program drill ran all three on this cluster)
   9 an upgrade carrying the line: sol_line identical across `install --mode upgrade`, then the next step still lands
  10 sol_line and the monitor: the query, and monitor/src/ethnft.mjs's alerts on it (monitor-check.mjs)
  11 cycles: per tick, and per refused simulation
No private key is printed or written here: the canister's keys are threshold keys; the operator's keypairs are the tool's.
"""
import json, os, re, shlex, subprocess, sys, time, urllib.request

MODE, DFX, NET, JOB = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4]
A = sys.argv[5:]


def opt(k, d=None):
    return A[A.index(k) + 1] if k in A else d


HERE = os.path.dirname(os.path.abspath(__file__))
R = {"network": NET, "steps": [], "started": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()), "cycles": []}
ok_all = True
CAN = "rooksol"


def log(kind, name, detail=""):
    global ok_all
    if kind == "FAIL":
        ok_all = False
    R["steps"].append({"kind": kind, "name": name, "detail": detail, "at": int(time.time())})
    print(f"{kind} {name}: {detail}", flush=True)
    save()


def save():
    with open(os.path.join(JOB, "drive-report.json"), "w") as f:
        json.dump(R, f, indent=1)


def call(method, arg="()", query=False):
    cmd = [DFX, "canister", "call", CAN, method, arg, "--network", NET, "--output", "json"] + (["--query"] if query else [])
    p = subprocess.run(cmd, capture_output=True, text=True, cwd=HERE)
    if p.returncode != 0:
        return None, "\n".join(l for l in p.stderr.splitlines() if "deprecated" not in l).strip()
    out = p.stdout.strip()
    return (json.loads(out) if out and out != "()" else None), None


def ok(method, arg="()", query=False):
    v, err = call(method, arg, query)
    if err:
        raise RuntimeError(f"{method}: {err}")
    if isinstance(v, dict) and "Err" in v:
        raise RuntimeError(f"{method}: {v['Err']}")
    return v["Ok"] if isinstance(v, dict) and "Ok" in v else v


def tool(*args):
    cmd = shlex.split(opt("--tool")) + list(args)
    p = subprocess.run(cmd, capture_output=True, text=True)
    out = (p.stdout + p.stderr).strip()
    for l in out.splitlines():
        print("   tool: " + l, flush=True)
    return p.returncode == 0, out


def rpc(method, params):
    url = json.load(open(os.path.join(JOB, "state.json")))["rpc"]
    req = urllib.request.Request(url, json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode(),
                                 {"content-type": "application/json"})
    with urllib.request.urlopen(req, timeout=30) as r:
        return json.loads(r.read()).get("result")


def num(x):
    return int(str(x).replace("_", ""))


def cycles():
    v = ok("cycles", query=True)
    return num(v["balance"])


# The line's outcomes as they were SEEN: a flow TAKES its outcomes off `sol_line` at its next pass (`SolLine::take`), so an outcome
# is on the line for one tick at most, and not at all when the pass that records it runs before the owner's in the same tick (run 3,
# 2026-10-09: the frog advance landed, its outcome was taken the next tick, and the checklist, reading `sol_line` later, saw none).
# Recorded after every tick from `sol_line` and from the tick's own lines ("sol line: <step> landed (<signature>)").
SEEN = []
LANDED = re.compile(r"sol line: (.+?) (landed|FAILED on chain) \((?:(success|FAILED on chain), )?([1-9A-HJ-NP-Za-km-z]{64,90})\)")


def label_name(label):
    """The `Owner.Step` name of a step label (`LineStep::label`)."""
    for pat, name in ((r"^frogs: advance \(frog", "Frog.Advance"), (r"^frogs: return_advance \(frog", "Frog.GiveBack"),
                      (r"^frogs: mint_item", "Frog.Mint"), (r"^frogs: settle_advance", "Frog.Report"), (r"^frogs: Relay deposit", "Frog.Convert"),
                      (r"^frogs: post_floor", "Frog.Post"), (r"^frogs: (record_delivery|mark_undeliverable|note_sent)", "Custody"),
                      (r"^advance \(buy", "Eth.Advance"), (r"^return_advance / fund_reserve \(buy", "Eth.GiveBack"), (r"^mint_item \(buy", "Eth.Mint"),
                      (r"^settle_advance \(buy", "Eth.Settle"), (r"^Relay deposit_native \(buy", "Eth.Deposit")):
        if re.search(pat, label):
            return name
    return "?" + label


def seen_add(name, ok, sig, intent=None, src=""):
    for x in SEEN:
        if x["signature"] == sig and x["name"] == name:
            if intent is not None and x.get("intent") is None:
                x["intent"] = intent
            return
    SEEN.append({"name": name, "ok": ok, "signature": sig, "intent": intent, "src": src})
    R["seen"] = SEEN


def collect(lines):
    for l in lines:
        for m in LANDED.finditer(l):
            seen_add(label_name(m.group(1)), m.group(2) == "landed" and m.group(3) != "FAILED on chain", m.group(4), src="tick line")
    try:
        for oc in outcomes():
            seen_add(step_name(oc["step"]), o(oc.get("ok")), oc["signature"], num(oc["intent"]), "sol_line")
    except Exception as ex:  # noqa: BLE001
        print(f"   (sol_line not read: {str(ex)[:120]})", flush=True)


def tick(label):
    c0 = cycles()
    t0 = time.time()
    v, err = call("tick")
    c1 = cycles()
    lines = (v.get("Ok") if isinstance(v, dict) else None) or []
    e = err or (v.get("Err") if isinstance(v, dict) else None)
    R["cycles"].append({"tick": label, "cycles": c0 - c1, "seconds": round(time.time() - t0, 1), "lines": len(lines), "err": e})
    print(f"   tick {label}: {c0 - c1:,} cycles, {time.time() - t0:.0f} s" + (f", ERR {e}" if e else ""), flush=True)
    for l in lines:
        print("     | " + l[:400], flush=True)
    collect(lines)
    save()
    return lines, e


def line():
    v = ok("sol_line", query=True)
    return v[0] if isinstance(v, list) and v else (v or None)


def o(x):
    return x[0] if isinstance(x, list) and x else (None if isinstance(x, list) else x)


def outcomes():
    l = line() or {}
    return l.get("outcomes", [])


def step_name(s):
    k = list(s.keys())[0]
    inner = s[k]
    return f"{k}.{list(inner.keys())[0] if isinstance(inner, dict) else inner}"


def found(pred):
    return next((x for x in SEEN if pred(x)), None)


def wait_landed(pred, what, ticks=12, gap=20, each=None):
    """Tick until an outcome satisfying `pred(seen)` has been seen (`seen`: {name, ok, signature, intent}); `each()` before a tick."""
    collect([])
    for i in range(ticks):
        if found(pred):
            return found(pred)
        if each:
            each()
        tick(f"{what} #{i}")
        time.sleep(gap)
    return found(pred)


def monitor():
    if NET != "ic" and "--local" not in A:
        return None
    cid = subprocess.run([DFX, "canister", "id", CAN, "--network", NET], capture_output=True, text=True, cwd=HERE).stdout.strip()
    args = ["node", os.path.join(HERE, "monitor-check.mjs"), cid] + (["--local", "--host", opt("--host")] if "--local" in A else [])
    p = subprocess.run(args, capture_output=True, text=True)
    if p.returncode != 0:
        return {"error": p.stderr.strip()[-400:]}
    return json.loads(p.stdout)


def upgrade(arg):
    # --upgrade-unchanged: the same module, or dfx skips the install and nothing is proven
    return subprocess.run([DFX, "canister", "install", CAN, "--network", NET, "--mode", "upgrade", "--upgrade-unchanged", "--wasm", opt("--wasm"),
                           "--argument", arg, "--argument-type", "raw", "-y"], capture_output=True, text=True, cwd=HERE)


# ---- addresses ------------------------------------------------------------------------------------------------------------------------

def addresses():
    sol = ok("sol_address")
    sol = sol.split()[0] if isinstance(sol, str) else sol
    out = {"sol": sol}
    for m, k in (("custody_address", "taproot"), ("float_address", "float"), ("evm_address", "evm")):
        try:
            v = ok(m)
            out[k] = v.split()[0] if isinstance(v, str) else v
        except Exception as e:  # noqa: BLE001
            out[k] = None
            log("INFO", m, str(e)[:200])
    json.dump(out, open(os.path.join(JOB, "canister.json"), "w"), indent=1)
    log("PASS", "the canister's addresses", json.dumps(out))


# ---- the checklist --------------------------------------------------------------------------------------------------------------------

def checklist():
    can = json.load(open(os.path.join(JOB, "canister.json")))
    plan = json.load(open(os.path.join(JOB, "plan.json")))
    sol = can["sol"]
    eth_floor, frog_floor = float(opt("--eth-floor-sol", "8")), float(opt("--frog-floor-sol", "1.5"))

    # 1. genesis
    lines, e = tick("genesis")
    if e and "genesis" in e:
        log("FAIL", "1 genesis", e)
    elif e:
        log("FAIL", "1 genesis", f"the tick refused: {e}")
    else:
        log("PASS", "1 genesis", "the tick ran on the Solana home: both providers agree devnet's genesis hash (a disagreement or another cluster refuses the whole tick)")

    # demand on both listings (the keeper key posts both floors; the holder buys), while the canister is still unfunded
    if "--skip-eth" not in A:
        tool("post", "--listing", "eth", "--sol", str(eth_floor))
        tool("mint-units", "--listing", "eth", "--sol", str(round(eth_floor * 1.8, 3)))
    good, out = tool("post", "--listing", "frog", "--sol", str(frog_floor))
    log("PASS" if good else "FAIL", "6 the keeper's floor post (frog listing)", "post_floor signed by Registry.keeper = an operator keypair playing the keeper service (D-a)")
    tool("mint-units", "--listing", "frog", "--sol", str(round(frog_floor * 1.8, 3)))
    try:
        ok("set_floor_sink", f'("{plan["frog"]["module"]}", null, null, null)')
        ok("set_purchase_limit", f'("{plan["frog"]["module"]}", {int(5e9)} : nat)')  # lamports: 5 SOL, within D-b's [1, 250] SOL
        log("PASS", "the frog module as the canister's floor sink", plan["frog"]["module"])
    except Exception as ex:  # noqa: BLE001
        log("FAIL", "set_floor_sink", str(ex)[:300])
    try:
        ok("frog_floor_reading", "(100000 : nat64, 9000 : nat16, 10 : nat32)")
    except Exception as ex:  # noqa: BLE001
        log("INFO", "frog_floor_reading", str(ex)[:200])

    # 2. unfunded: the first step's simulation refused
    bal = int(rpc("getBalance", [sol, {"commitment": "confirmed"}])["value"])
    if bal < 3_000_000:
        refused = None
        for i in range(4):
            lines, e = tick(f"unfunded #{i}")
            if any("not us: the keeper service posts" in l for l in lines):
                log("PASS", "6 the canister's own frog reading is dropped", "the registry names the keeper service, not the canister (D-a)")
            l = line() or {}
            err = o((l.get("line") or {}).get("error"))
            if err:
                refused = err
                break
            time.sleep(10)
        if refused:
            m = monitor()
            sev1 = m and any(a.get("sev") == "SEV1" and "Fund the canister" in a.get("text", "") for a in m.get("alerts", []))
            log("PASS" if re.search(r"insufficient|funds|lamports", refused, re.I) else "INFO", "2 unfunded: the simulation refused, nothing signed", refused[:300])
            log("PASS" if sev1 else ("SKIP" if m is None else "FAIL"), "10 the monitor pages the unfunded line (SEV1)", json.dumps(m)[:400] if m else "no IC monitor read here")
            sims = [c for c in R["cycles"] if c["tick"].startswith("unfunded")]
            if sims:
                log("INFO", "11 cycles of a tick whose step's simulation is refused", f"{sims[-1]['cycles']:,} cycles")
        else:
            log("INFO", "2 unfunded", "no step reached the line while unfunded (no demand yet?): see the tick lines")
    else:
        log("SKIP", "2 unfunded", f"the canister's address already holds {bal} lamports")

    # 3. funding
    want = int(float(opt("--canister-sol", "0.05")) * 1e9)
    bal = int(rpc("getBalance", [sol, {"commitment": "confirmed"}])["value"])
    if bal < want:
        good, _ = tool("pay", "--to", sol, "--sol", str(want / 1e9))
        if not good:
            print(f"   fund {sol} with {want / 1e9} SOL at https://faucet.solana.com (devnet); waiting up to {opt('--fund-wait', '600')} s", flush=True)
            t0 = time.time()
            while time.time() - t0 < int(opt("--fund-wait", "600")):
                if int(rpc("getBalance", [sol, {"commitment": "confirmed"}])["value"]) >= want:
                    break
                time.sleep(15)
    bal = int(rpc("getBalance", [sol, {"commitment": "confirmed"}])["value"])
    log("PASS" if bal >= want else "FAIL", "3 funded", f"{bal} lamports at {sol}")

    # 3-4. the line opens and an advance lands (ETH first if its demand is there, else the frog's)
    adv = wait_landed(lambda x: x["name"].endswith(".Advance") and x["ok"] is True, "advance", ticks=25)
    l = line() or {}
    nonce = o((l.get("line") or {}).get("nonce_account"))
    log("PASS" if nonce else "FAIL", "3 the line's durable-nonce account", "opened by the line itself" if nonce else
        ("the line holds none: " + json.dumps(l)[:300] if l else "sol_line is null: no flow has used the line yet (read the tick lines)"))
    if adv:
        sig = adv["signature"]
        st = rpc("getSignatureStatuses", [[sig], {"searchTransactionHistory": True}])["value"][0]
        log("PASS" if st and st.get("confirmationStatus") == "finalized" else "FAIL", "4 simulate / sign / land through the nonce line",
            f"{adv['name']} intent {adv.get('intent')}: {sig} ({st and st.get('confirmationStatus')})")
    else:
        log("FAIL", "4 an advance landed", "no Advance outcome after 25 ticks: read the tick lines (an ETH buy needs an accepted floor whose advance covers it on Ethereum: --eth-floor-sol; a frog needs net demand >= 1 unit and a fresh keeper floor)")

    # 7. the stubbed Relay legs: once an advance has landed and its buy / conversion reads Funded (the next tick), the operator's exit
    # gives it back (relay_give_back / eth_give_back): no Relay on devnet, so the advance returns through return_advance on the line
    def phase(x):
        return list(x["phase"].keys())[0] if isinstance(x["phase"], dict) else x["phase"]

    given = set()

    def give_backs():
        for c in ok("frog_conversions", query=True) or []:
            if phase(c) == "Funded" and ("frog", str(c["frog"])) not in given:
                try:
                    log("INFO", f"7 frog {c['frog']}: Relay SOL->BTC stubbed", ok("relay_give_back", f'({c["frog"]} : nat64)')[:200])
                    given.add(("frog", str(c["frog"])))
                except Exception as ex:  # noqa: BLE001
                    log("INFO", f"7 frog {c['frog']} give back", str(ex)[:200])
        if "--skip-eth" in A:
            return
        for b in (ok("eth_nft_state", query=True) or {}).get("buys", []):
            if phase(b) in ("Funded", "BackFunded") and ("eth", str(b["id"])) not in given:
                try:
                    log("INFO", f"ETH buy {b['id']}: Relay SOL->ETH stubbed", ok("eth_give_back", f'({b["id"]} : nat64)')[:200])
                    given.add(("eth", str(b["id"])))
                except Exception as ex:  # noqa: BLE001
                    log("INFO", f"ETH buy {b['id']} give back", str(ex)[:200])

    # once an advance has landed, its conversion / buy reads Funded on a later tick; the operator's exit gives it back then, and the
    # return_advance rides the line: one loop, so a late advance still gets its give-back (run 3 gave up on it before it landed)
    back = wait_landed(lambda x: x["name"].endswith(".GiveBack") and x["ok"] is True, "give back", ticks=25, each=give_backs)
    log("PASS" if back else "FAIL", "7 an advance returned through the line (return_advance)", json.dumps(back)[:300] if back else
        f"no GiveBack outcome (given back: {sorted(given)})")
    frog_adv = found(lambda x: x["name"] == "Frog.Advance" and x["ok"] is True)
    log("PASS" if frog_adv else "FAIL", "7 the frog advance (real, signed by the canister)", f"landed {frog_adv['signature']}" if frog_adv else "no Frog.Advance outcome")

    # 5. both flows on one line
    kinds = [x["name"].split(".")[0] for x in SEEN]
    intents = [x["intent"] for x in SEEN if x.get("intent") is not None]
    both = "Eth" in kinds and "Frog" in kinds
    log("PASS" if both and intents == sorted(intents) else ("SKIP" if "--skip-eth" in A else "FAIL"), "5 ETH and frog steps on one line",
        f"outcomes seen {[(x['name'], x.get('intent')) for x in SEEN]}")

    # 7-8. a real signet frog (optional)
    if opt("--frog-offer"):
        f = json.load(open(opt("--frog-offer")))
        log("INFO", "7 frog offer", "handed in; the tick judges and signs it; minting follows custody's read of the inscription (minutes to hours on signet)")
        try:
            ok("frog_offer", f'({f["frog"]} : nat64, "{f["inscription"]}", "{f["psbt_hex"]}", vec {{ {"; ".join(str(i) for i in f["to_sign"])} }}, '
                             f'vec {{ {"; ".join(chr(34) + p + chr(34) for p in f["proof"])} }})')
        except Exception as ex:  # noqa: BLE001
            log("FAIL", "7 frog_offer", str(ex)[:300])
        minted = wait_landed(lambda x: x["name"] == "Frog.Mint" and x["ok"] is True, "mint_item", ticks=30, gap=60)
        log("PASS" if minted else "FAIL", "7 mint_item with a proof, by the canister", json.dumps(minted)[:300] if minted else "not minted")
    else:
        log("SKIP", "7 mint_item by the canister", "needs a real signet frog purchase (--frog-offer); the program drill ran mint_item with a Merkle proof on this cluster (run.sh's drill phase)")
        log("SKIP", "8 the canister's ticket steps (note_sent / record_delivery / mark_undeliverable)",
            "need a frog the canister minted; the program drill ran all three on this cluster against a tb1p destination")

    # 9. an upgrade carrying the line
    before = line()
    arg = open(opt("--arg")).read().strip()
    p = upgrade(arg)
    after = line()
    if p.returncode != 0:
        log("FAIL", "9 the upgrade", p.stderr.strip()[-400:])
    else:
        same = json.dumps(before, sort_keys=True) == json.dumps(after, sort_keys=True)
        restored = any("upgraded: state restored" in l for l in (ok("log", query=True) or [])[-5:])
        same = same and before is not None  # a null line carried across proves nothing
        log("PASS" if same and restored else "FAIL", "9 an upgrade carrying the line", "sol_line identical before and after `install --mode upgrade "
            "--upgrade-unchanged` (pre_upgrade saved it, post_upgrade restored it: 'upgraded: state restored')" if same and restored
            else f"restored {restored}; before {json.dumps(before)[:200]} / after {json.dumps(after)[:200]}")
        lines, e = tick("after the upgrade")
        log("PASS" if not e else "FAIL", "9 the line ticks on after the upgrade", e or f"{len(lines)} lines")

    # 10. the query and the monitor, at rest
    m = monitor()
    if m is None:
        log("SKIP", "10 the monitor's read", "not an IC canister")
    elif "error" in m:
        log("FAIL", "10 the monitor's read", m["error"])
    else:
        log("PASS", "10 sol_line through the monitor", f"{len(m['alerts'])} alert(s) at rest: {json.dumps(m['alerts'])[:300]}")

    # 11. cycles
    t = [c for c in R["cycles"] if not c.get("err")]
    if t:
        log("INFO", "11 cycles per tick", f"{len(t)} ticks: min {min(c['cycles'] for c in t):,}, max {max(c['cycles'] for c in t):,}, "
            f"mean {sum(c['cycles'] for c in t) // len(t):,} (each tick reads accounts at finalized through two providers and simulates before signing)")
    tool("burn-units", "--listing", "eth", "--all")
    tool("burn-units", "--listing", "frog", "--all")


# ---- smoke: the canister half on a LOCAL replica (key_1 of PocketIC; no cycles spent, nothing on devnet written) ----------------------

def smoke():
    addresses()
    lines, e = tick("genesis (local replica -> devnet)")
    log("PASS" if not e else "FAIL", "1 genesis: the tick on the Solana home agrees devnet's genesis through both providers",
        e or f"{len(lines)} lines; reads at finalized: " + "; ".join(l[:120] for l in lines[:3]))
    plan = json.load(open(os.path.join(JOB, "plan.json")))
    for m, a in (("set_floor_sink", f'("{plan["frog"]["module"]}", null, null, null)'), ("set_purchase_limit", f'("{plan["frog"]["module"]}", 5000000000 : nat)'),
                 ("frog_floor_reading", "(100000 : nat64, 9000 : nat16, 10 : nat32)")):
        try:
            log("PASS", m, str(ok(m, a))[:200])
        except Exception as ex:  # noqa: BLE001
            log("FAIL", m, str(ex)[:300])
    try:
        ok("set_purchase_limit", f'("{plan["frog"]["module"]}", 251000000000 : nat)')
        log("FAIL", "the purchase limit's ceiling (D-b: 250 SOL)", "251 SOL was accepted")
    except Exception as ex:  # noqa: BLE001
        log("PASS", "the purchase limit's ceiling (D-b: 250 SOL)", str(ex)[:200])
    before = (line(), ok("floor_state", query=True))
    p = upgrade(open(opt("--arg")).read().strip())
    after = (line(), ok("floor_state", query=True))
    restored = any("upgraded: state restored" in l for l in (ok("log", query=True) or [])[-5:])
    log("PASS" if p.returncode == 0 and restored and before == after else "FAIL", "9 an upgrade carrying the state",
        "sol_line and floor_state identical across install --mode upgrade --upgrade-unchanged" if p.returncode == 0 and before == after else p.stderr[-300:])
    m = monitor()
    log("PASS" if m is not None and "error" not in m else "FAIL", "10 sol_line through the monitor's own read", json.dumps(m)[:300])
    t = [c for c in R["cycles"] if not c.get("err")]
    if t:
        log("INFO", "11 cycles of a tick (local replica's prices)", f"{t[0]['cycles']:,}")


try:
    {"addresses": addresses, "checklist": checklist, "smoke": smoke}[MODE]()
except Exception as e:  # noqa: BLE001
    log("FAIL", "the driver stopped", str(e)[:500])
R["verdict"] = "PASS" if ok_all else "FAIL"
save()
print(f"== {R['verdict']} (report: {os.path.join(JOB, 'drive-report.json')})")
sys.exit(0 if ok_all else 1)
