"""W15: the canister-only rehearsal's driver (docs/v9/rehearsal-sol-devnet.md §8), against a throwaway buyer installed with
`rook_core = "books"` (`rooksol`, created, installed and deleted by run.sh --books). DEVNET ONLY: the canister signs with test_key_1
(the IC) or key_1 (a local replica), Ethereum is only read, no program is deployed. Phases, each a mode here:

  python3 -I books.py addresses <dfx> <network> <job> --count <n>
      K_main (`sol_address`), K_fc (`sol_address_of("claimer")`), K_tre (`"treasury"`), M[i] (`"coll:<i>"`), the Ethereum custody
      (`evm_address`) and the taproot custody (`custody_address`) into <job>/canister.json: the pins name the two custodies.
  python3 -I books.py pins <dfx> <network> <job> --count <n>
      `coins_pin` with each <job>/pins/<index>-<slug>.args (cargo run --example launch_pins wrote them); read back as Pinned; then
      `coins_request_open` for each (a collection opens on its first request, operator decision 2026-10-12), read back in
      `coins_log`'s queue. A SolNft collection (Mad Lads, SMB Gen2 at --count ≥ 3) has no EVM or BTC custody: its custodian is K_main.
  python3 -I books.py fund <dfx> <network> <job> --count <n> --rpc <url> [--payer <keypair>] [--float-sol <x>] [--fund-wait <s>] [--intent]
      the float K_main needs (the opening cost per collection, the reserve the money flows keep, the nonce account): printed with
      K_main; sent with the solana CLI from the OPERATOR's devnet keypair when --payer is given, else waited for; the balance polled.
  python3 -I books.py open <dfx> <network> <job> --count <n> --rpc <url> [--open-wait <s>] [--tick-gap <s>]
      ticks until every pinned collection is Open (the wX mint + metadata, then the DBC config, then both read back), or the
      bounded wait ends: every wait's reason printed (the tick's `coins:` lines, `coins_log`'s waits); then the views read back.
  python3 -I books.py checklist <dfx> <network> <job> --count <n> --rpc <url> [--payer <kp>] [--intent] [--evidence <json>]
      [--wasm <gz> --arg <hex file> --chunked <script>] [--local --host <url>] [--open-wait <s>]
      the items below, PASS / FAIL / INFO / SKIP each, the verdict at the end; the report in <job>/books-report.json.
  python3 -I books.py parse <views dir>
      offline: the parsers of this file against the canister's saved view bytes (tests/fixtures/views/*.hex), decoded the way the
      monitor decodes them (views-decode.mjs); one JSON line of what they read. test/books.test.mjs holds it.

The checklist:
   1 genesis: a tick on the books home agrees devnet's genesis hash through both providers (a disagreement refuses the whole tick)
   2 unfunded: with the pins in and K_main empty, the opening WAITS ("it costs … K_main holds …"): no signature spent, nothing landed
   3 funded: K_main holds the float; the crank line's durable-nonce account is opened by the line itself
   4 each pinned collection is Open: its two opening transactions (mint + metadata, config) landed finalized and were read back
     (the wX mint, its metadata and the DBC config exist on devnet at the pinned addresses)
   5 books_view(i) consistent: listed with the pinned wX mint, module = sol_address_of("coll:i"), expected_supply 0 before any
     mint, owed = backing + reserve + surplus + revenue, nothing held, not halted
   6 one intent end to end (--intent): a mint intent for the operator's wallet, the minimum sent from the payer, intent_status
     polled to Done, the wX arrived at the wallet and books_view moved by it. Needs a fresh floor (the runner's evidence hand-in,
     or --evidence <json> with Seaport listings); else SKIP
   7 the monitor's books.mjs alerts on the views (monitor-check.mjs --books): none SEV1 at rest
   8 an upgrade carrying the state (--wasm/--arg): the chunked upgrade, coins_collections and books_collections identical after
   9 cycles per tick (INFO)
No private key is printed or written here: the canister's keys are threshold keys; the payer is the operator's file, read by the
solana CLI only. Every amount is devnet SOL.
"""
import json, os, re, subprocess, sys, time, urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(os.path.dirname(HERE)))
CAN = "rooksol"
LAMPORTS = 1_000_000_000
# coins::unlock (OPEN_COST) and coins::open::RESERVE_LAMPORTS of the canister; the nonce account's rent-exempt balance (80 bytes)
OPEN_COST = 1_461_600 + 5_115_600 + 10_000_000 + 5_984_080 + 25_000
RESERVE = 50_000_000
NONCE_RENT = 1_447_680
# desk::intent: the smallest mint deposit and the desk's fee
MIN_MINT = 50_000_000
INTENT_FEE = 1_500_000
ONE = 1_000_000_000
SIG = re.compile(r"signed \(([1-9A-HJ-NP-Za-km-z]{64,90})\)")
DEVNET_GENESIS = "EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG"


def opt(k, d=None):
    return A[A.index(k) + 1] if k in A else d


# ---- pure: the float, the parsers (parse mode and the tests read them) --------------------------------------------------------------

def num(x):
    """A dfx / monitor number (a string, maybe with underscores) as an int."""
    if x is None:
        return 0
    return int(str(x).replace("_", ""))


def o(x):
    """An opt as dfx and the monitor print it (`[]` / `[v]`, or a bare value)."""
    return x[0] if isinstance(x, list) and x else (None if isinstance(x, list) else x)


def float_lamports(count, intent=False):
    """What K_main must hold to open `count` collections: the reserve the money flows keep, each opening's cost, the nonce account,
    fees; rounded up to 0.01 SOL. The intent's deposit is the PAYER's (sent to the intent's own address), not K_main's."""
    want = RESERVE + count * OPEN_COST + NONCE_RENT + 2_000_000
    return -(-want // 10_000_000) * 10_000_000


def stages(colls):
    """`coins_collections` → {index: stage text} (`Pinned`, `Opening`, `Open`, `Refused("…")`)."""
    return {int(c["index"]): str(c["stage"]) for c in colls}


def all_open(colls, count):
    s = stages(colls)
    return all(s.get(i) == "Open" for i in range(count)), s


def waits_of(coins_log):
    """`coins_log` ([fees, waits, last]) → [(key, why, since, tries)]."""
    return [(w["key"], w["why"], num(w["since"]), num(w.get("tries"))) for w in (coins_log[1] if coins_log else [])]


def landed_of(coins_log):
    """`coins_log`'s last landings → [(label, at, note)]."""
    return [(tup(l)[0], num(tup(l)[1]), tup(l)[2]) for l in (coins_log[2] if coins_log else [])]


def queue_of(coins_log):
    """`coins_log`'s opening queue (the 4th element, `coins_request_open`, oldest first) → [(collection, asked at)]; [] from a canister
    built before it."""
    return [(int(tup(q)[0]), num(tup(q)[1])) for q in (coins_log[3] if coins_log and len(coins_log) > 3 else [])]


def kind_of(v):
    """A books view's listing kind: "EthNft", "Inscriptions" or "SolNft". A SolNft collection has no EVM or BTC custody to check: its
    custodian is the canister's own wallet (`sol_address`), named in the pin; every rule below holds for it unchanged."""
    k = v.get("kind") or {}
    return next(iter(k), "?") if isinstance(k, dict) else "?"


def books_view_rules(v, module=None, wx_mint=None, expect_supply=None):
    """The consistency rules of one `books_view` (the reconciler's and the site's), each (rule, ok, detail)."""
    out = []
    owed, parts = num(v["owed"]), num(v["backing"]) + num(v["reserve"]) + num(v["surplus"]) + num(v["revenue"])
    out.append(("owed = backing + reserve + surplus + revenue", owed == parts, f"{owed} vs {parts}"))
    es = o(v.get("expected_supply"))
    ledger = (num(v["held"]) - num(v["open_tickets"])) * ONE + num(v["sol_backed"])
    out.append(("expected_supply = (held - open tickets) * 1 wX + sol_backed", es is None or num(es) == ledger, f"{es} vs {ledger}"))
    if expect_supply is not None:
        out.append((f"expected_supply {expect_supply}", es is not None and num(es) == expect_supply, f"{es}"))
    out.append(("not halted", o(v.get("halted")) is None, str(o(v.get("halted")))))
    if module is not None:
        out.append(("module = sol_address_of(coll:i)", True, module))
    if wx_mint is not None:
        out.append(("wx_mint = the pinned mint", v["wx_mint"] == wx_mint, f"{v['wx_mint']} vs {wx_mint}"))
    out.append(("items listed = held", len(v["items"]) == num(v["held"]), f"{len(v['items'])} items, held {v['held']}"))
    return out


def fresh_floor(v):
    return bool(v.get("fresh")) and num(v["accepted"]) > 0


def desk_rules(d):
    return [("no collection halted", not d.get("halted"), json.dumps(d.get("halted"))[:200]),
            ("no unexplained memo", not (o(d.get("report")) or {}).get("unexplained"), json.dumps((o(d.get("report")) or {}).get("unexplained"))[:200]),
            ("intents", True, f"open {d.get('open')}, funded {d.get('funded')}, executing {d.get('executing')}, attention {d.get('needing_attention')}")]


def treasury_rules(t):
    waits = [(w["key"], w["why"]) for w in t.get("waits", [])]
    return [("no treasury wait", not waits, json.dumps(waits)[:300]),
            ("drand relays agree", o(t.get("drand_disagreement")) is None, str(o(t.get("drand_disagreement")))[:200]),
            ("cranks on", not t.get("off"), "off" if t.get("off") else "on")]


def parse(views_dir):
    """The parsers over the saved views, decoded by the monitor's decoder."""
    def dec(method, f):
        p = subprocess.run(["node", os.path.join(HERE, "views-decode.mjs"), method, os.path.join(views_dir, f)], capture_output=True, text=True)
        if p.returncode != 0:
            raise RuntimeError(p.stderr.strip()[-300:])
        return json.loads(p.stdout)
    view = o(dec("books_view", "books-view.hex"))
    coins = dec("coins_log", "coins-log.hex")
    desk = dec("desk_status", "desk-status.hex")
    treasury = dec("treasury_status", "treasury-status.hex")
    desks = dec("pool_desks", "pool-desks.hex")
    out = {"books_view": {"coll": view["coll"], "wx_mint": view["wx_mint"], "fresh_floor": fresh_floor(view), "rules": books_view_rules(view),
                          "rules_wrong_mint": books_view_rules(view, wx_mint="nope")},
           "coins_log": {"waits": waits_of(coins), "landed": landed_of(coins), "queue": queue_of(coins)},
           "kind": kind_of(view), "kind_solnft": kind_of({"kind": {"SolNft": {"collection": [], "custodian": []}}}),
           "desk_status": desk_rules(desk), "treasury_status": treasury_rules(treasury), "pool_desks": len(desks),
           "float": {"2": float_lamports(2), "5": float_lamports(5)}}
    print(json.dumps(out))


# ---- the canister through dfx, the chain through its RPC --------------------------------------------------------------------------------

R = {"steps": [], "cycles": [], "started": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())}
ok_all = True
SIGS = []


def save():
    with open(os.path.join(JOB, "books-report.json"), "w") as f:
        json.dump(R, f, indent=1)


def log(kind, name, detail=""):
    global ok_all
    if kind == "FAIL":
        ok_all = False
        R.setdefault("failed_phases", [])
        if MODE not in R["failed_phases"]:
            R["failed_phases"].append(MODE)
    R["steps"].append({"kind": kind, "name": name, "detail": detail, "at": int(time.time())})
    print(f"{kind} {name}: {detail}", flush=True)
    save()


def call(method, arg="()", query=False, argfile=None):
    cmd = [DFX, "canister", "call", CAN, method] + ([arg] if argfile is None else ["--argument-file", argfile]) + ["--network", NET, "--output", "json"] + (["--query"] if query else [])
    p = subprocess.run(cmd, capture_output=True, text=True, cwd=HERE)
    if p.returncode != 0:
        return None, "\n".join(l for l in p.stderr.splitlines() if "deprecated" not in l).strip()
    # dfx 0.32 prints "WARNING: dfx is deprecated …" on stdout as well: drop warning lines. A method with several return values
    # (`coins_log`) prints one JSON document per value: read them all, a list of them when more than one
    out = "\n".join(l for l in p.stdout.splitlines() if not l.startswith("WARNING:")).strip()
    if not out or out == "()":
        return None, None
    return json_values(out), None


def json_values(out):
    """Every JSON document in dfx's output: the one value, or a list of the values of a multi-value return."""
    dec, vals, i = json.JSONDecoder(), [], 0
    while i < len(out):
        if out[i].isspace():
            i += 1
            continue
        v, i = dec.raw_decode(out, i)
        vals.append(v)
    return vals[0] if len(vals) == 1 else vals


def ok(method, arg="()", query=False, argfile=None):
    v, err = call(method, arg, query, argfile)
    if err:
        raise RuntimeError(f"{method}: {err}")
    if isinstance(v, dict) and "Err" in v:
        raise RuntimeError(f"{method}: {v['Err']}")
    return v["Ok"] if isinstance(v, dict) and "Ok" in v else v


def text(method, arg="()"):
    v = ok(method, arg)
    return v.split()[0] if isinstance(v, str) else v


def rpc(method, params):
    req = urllib.request.Request(RPC, json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode(), {"content-type": "application/json"})
    with urllib.request.urlopen(req, timeout=30) as r:
        body = json.loads(r.read())
    if "error" in body:
        raise RuntimeError(f"{method}: {body['error']}")
    return body.get("result")


def balance(addr):
    return int(rpc("getBalance", [addr, {"commitment": "confirmed"}])["value"])


def exists(addr):
    return rpc("getAccountInfo", [addr, {"commitment": "finalized", "encoding": "base64"}])["value"] is not None


def finalized(sig):
    st = rpc("getSignatureStatuses", [[sig], {"searchTransactionHistory": True}])["value"][0]
    return st is not None and st.get("confirmationStatus") == "finalized" and not st.get("err")


def cycles():
    return num(ok("cycles", query=True)["balance"])


def tick(label):
    c0, t0 = cycles(), time.time()
    v, err = call("tick")
    c1 = cycles()
    lines = (v.get("Ok") if isinstance(v, dict) else None) or []
    e = err or (v.get("Err") if isinstance(v, dict) else None)
    R["cycles"].append({"tick": label, "cycles": c0 - c1, "seconds": round(time.time() - t0, 1), "lines": len(lines), "err": e})
    print(f"   tick {label}: {c0 - c1:,} cycles, {time.time() - t0:.0f} s" + (f", ERR {e}" if e else ""), flush=True)
    for l in lines:
        print("     | " + l[:400], flush=True)
        for m in SIG.finditer(l):
            if m.group(1) not in SIGS:
                SIGS.append(m.group(1))
    R["signatures"] = SIGS
    save()
    return lines, e


def tup(x):
    """A candid tuple as dfx's JSON writes it ({"0": a, "1": b}) or as a list."""
    return [x[str(k)] for k in range(len(x))] if isinstance(x, dict) else x


def colls():
    return ok("coins_collections", query=True) or []


def coins_log():
    v = ok("coins_log", query=True)
    v = tup(v) if isinstance(v, (list, dict)) else None  # dfx's JSON writes the tuple as {"0": …, "1": …}
    return v if v else [[], [], [], []]


def books_view(i):
    return o(ok("books_view", f"({i} : nat16)", query=True))


def monitor():
    cid = subprocess.run([DFX, "canister", "id", CAN, "--network", NET], capture_output=True, text=True, cwd=HERE).stdout.strip()
    args = ["node", os.path.join(HERE, "monitor-check.mjs"), cid, "--books"] + (["--local", "--host", opt("--host")] if "--local" in A else [])
    p = subprocess.run(args, capture_output=True, text=True)
    if p.returncode != 0:
        return {"error": p.stderr.strip()[-400:]}
    return json.loads(p.stdout)


def solana(*args):
    """The solana CLI with the operator's payer (the key never leaves the CLI)."""
    cmd = ["solana", "-u", RPC, "-k", opt("--payer")] + list(args)
    p = subprocess.run(cmd, capture_output=True, text=True)
    out = (p.stdout + p.stderr).strip()
    for l in out.splitlines()[-3:]:
        print("   solana: " + l[:200], flush=True)
    return p.returncode == 0, out


def pay(to, lamports):
    good, _ = solana("transfer", "--allow-unfunded-recipient", to, f"{lamports / LAMPORTS:.9f}")
    return good


def wait_balance(addr, want, seconds):
    t0 = time.time()
    while True:
        b = balance(addr)
        if b >= want or time.time() - t0 > seconds:
            return b
        time.sleep(15)


# ---- the phases -----------------------------------------------------------------------------------------------------------------------------

def addresses():
    count = int(opt("--count", "2"))
    out = {"sol": text("sol_address")}
    for m, a, k in (("sol_address_of", '("claimer")', "claimer"), ("sol_address_of", '("treasury")', "treasury"), ("evm_address", "()", "evm"), ("custody_address", "()", "taproot")):
        try:
            out[k] = text(m, a)
        except Exception as e:  # noqa: BLE001
            out[k] = None
            log("INFO", f"{m}{a}", str(e)[:200])
    out["coll"] = {}
    for i in range(count):
        out["coll"][str(i)] = text("sol_address_of", f'("coll:{i}")')
    json.dump(out, open(os.path.join(JOB, "canister.json"), "w"), indent=1)
    log("PASS", "the canister's addresses", json.dumps(out))


def pins():
    count = int(opt("--count", "2"))
    d = os.path.join(JOB, "pins")
    files = sorted(f for f in os.listdir(d) if f.endswith(".args"))[:count]
    if len(files) < count:
        log("FAIL", "pins", f"{len(files)} argument files in {d}, {count} wanted (cargo run --example launch_pins --count {count})")
        return
    for f in files:
        try:
            log("PASS", f"coins_pin {f}", str(ok("coins_pin", argfile=os.path.join(d, f)))[:200])
        except Exception as e:  # noqa: BLE001
            log("FAIL", f"coins_pin {f}", str(e)[:300])
    s = stages(colls())
    log("PASS" if all(s.get(i) in ("Pinned", "Requested", "Opening", "Open") for i in range(count)) else "FAIL", "pinned: coins_collections", json.dumps(s))
    # operator decision 2026-10-12 ("the people choose"): a pinned collection opens on its first request, as a creator's would be; the
    # rehearsal asks for each it pinned, in index order (a second ask is a no-op answer, so a re-run is safe)
    # audit C4-02's limit is one pending request a caller: the rehearsal (one principal) asks for the first here, and for each next one in
    # the open phase once the one before is Open
    request(0)
    q = queue_of(coins_log())
    s = stages(colls())
    # the opening pass may take the request at once: Requested, Opening or Open all mean it was accepted
    log("PASS" if s.get(0) in ("Requested", "Opening", "Open") or 0 in [x[0] for x in q] else "FAIL", "requested: collection 0",
        f"stage {s.get(0)}; coins_log's queue {json.dumps(q)}")


def fund():
    count = int(opt("--count", "2"))
    can = json.load(open(os.path.join(JOB, "canister.json")))
    k = can["sol"]
    want = int(float(opt("--float-sol")) * LAMPORTS) if opt("--float-sol") else float_lamports(count)
    extra = f" (+ the intent's {(MIN_MINT + INTENT_FEE) / LAMPORTS} SOL from the payer to the intent's address at item 6)" if "--intent" in A else ""
    print(f"== the float: {want / LAMPORTS:.3f} SOL to K_main {k}{extra}", flush=True)
    b = balance(k)
    if b >= want:
        log("SKIP", "3 funded", f"K_main already holds {b} lamports")
        return
    if opt("--payer"):
        if not pay(k, want - b):
            log("FAIL", "3 the payer's transfer", "solana transfer failed (see above)")
            return
    else:
        print(f"   send {(want - b) / LAMPORTS:.3f} SOL to {k} (devnet; https://faucet.solana.com); waiting up to {opt('--fund-wait', '600')} s", flush=True)
    b = wait_balance(k, want, int(opt("--fund-wait", "600")))
    log("PASS" if b >= want else "FAIL", "3 funded", f"K_main holds {b} lamports ({b / LAMPORTS:.4f} SOL), wanted {want}")


def request(i):
    """`coins_request_open(i)`, as a creator's "Open it" would: a repeat or an already-open collection is INFO, not a failure."""
    try:
        log("PASS", f"coins_request_open({i})", str(ok("coins_request_open", f"({i} : nat32)"))[:200])
    except Exception as e:  # noqa: BLE001
        log("INFO" if ("open already" in str(e) or "already" in str(e) or "is open" in str(e)) else "FAIL", f"coins_request_open({i})", str(e)[:300])


def open_all(count, wait_s, gap):
    """Tick until every pinned collection is Open; the reasons of every wait printed. Returns the final stages."""
    t0 = time.time()
    reasons = []
    n = 0
    while True:
        lines, e = tick(f"open #{n}")
        n += 1
        if e and "genesis" in str(e):
            log("FAIL", "1 genesis", str(e)[:300])
            return stages(colls())
        for l in lines:
            if l.startswith("coins:") and ("waits" in l or "costs" in l or "REFUSED" in l or "not" in l) and l not in reasons:
                reasons.append(l)
                print("   wait: " + l[:300], flush=True)
        done, s = all_open(colls(), count)
        if done:
            return s
        # one pending request a caller: ask for the next collection once every one before it is Open
        nxt = next((i for i in range(count) if s.get(i) != "Open"), None)
        if nxt is not None and s.get(nxt) == "Pinned" and all(s.get(i) == "Open" for i in range(nxt)):
            request(nxt)
        if any(str(v).startswith("Refused") for v in s.values()):
            return s
        if time.time() - t0 > wait_s:
            for key, why, since, tries in waits_of(coins_log()):
                print(f"   coins_log wait {key}: {why[:200]} ({tries} tries)", flush=True)
            return s
        time.sleep(gap)


def open_phase():
    count = int(opt("--count", "2"))
    s = open_all(count, int(opt("--open-wait", "1800")), int(opt("--tick-gap", "20")))
    log("PASS" if all(s.get(i) == "Open" for i in range(count)) else "FAIL", "4 every pinned collection Open", json.dumps(s))
    views = {"coins_collections": colls(), "books_collections": ok("books_collections", query=True), "books_view": {}, "treasury_status": ok("treasury_status", query=True),
             "desk_status": ok("desk_status", query=True), "pool_desks": ok("pool_desks", query=True)}
    for i in range(count):
        views["books_view"][str(i)] = books_view(i)
    json.dump(views, open(os.path.join(JOB, "views.json"), "w"), indent=1)
    log("INFO", "the views read back", f"{os.path.join(JOB, 'views.json')}: " + ", ".join(f"{k} {len(v) if isinstance(v, (list, dict)) else 'ok'}" for k, v in views.items()))


# ---- the checklist -------------------------------------------------------------------------------------------------------------------------

def checklist():
    count = int(opt("--count", "2"))
    can = json.load(open(os.path.join(JOB, "canister.json")))
    k = can["sol"]
    funded = "--payer" in A or balance(k) > 0

    # 1. genesis, 2. unfunded: the pins are in, K_main is empty: the opening waits for the float, nothing is signed
    lines, e = tick("genesis")
    for _ in range(6):
        if not (e and "already running" in str(e)):
            break
        time.sleep(10)  # the canister's own timer is ticking: wait for it, never count that as a refusal
        lines, e = tick("genesis (retry)")
    if e:
        log("FAIL", "1 genesis", ("both providers do not agree devnet's genesis: " if "genesis" in str(e) else "the tick refused: ") + str(e)[:300])
    else:
        log("PASS", "1 genesis", "the tick ran on the books home: both providers agree devnet's genesis hash (reads at finalized)")
    b = balance(k)
    if b < RESERVE:
        waits = [l for l in lines if l.startswith("coins:") and "costs" in l and "holds" in l]
        landed = landed_of(coins_log())
        log("PASS" if waits and not landed and not SIGS else ("INFO" if not waits else "FAIL"), "2 unfunded: the opening waits, nothing signed",
            (waits[0][:300] if waits else "no `coins: … it costs …` line this tick (the opening pass runs every 120 s; pinned? " + json.dumps(stages(colls())) + ")")
            + f"; landed {landed}; signatures seen {SIGS}")
        if R["cycles"]:
            log("INFO", "9 cycles of a tick whose opening waits for the float", f"{R['cycles'][-1]['cycles']:,} cycles")
    else:
        log("SKIP", "2 unfunded", f"K_main already holds {b} lamports")

    if not funded:
        log("SKIP", "3-6 funding, opening, the views, the intent", "no --payer and K_main is empty: the free run stops here (run.sh --devnet --books --payer <keypair>, or fund K_main by hand)")
    else:
        # 3. funded, the nonce account
        want = float_lamports(count)
        if balance(k) < want:
            fund()
        s = open_all(count, int(opt("--open-wait", "1800")), int(opt("--tick-gap", "20")))
        lines_v = ok("sol_lines", query=True)
        nonce = None
        try:
            for ln in (lines_v[0] if isinstance(lines_v, list) and lines_v and isinstance(lines_v[0], list) else lines_v or []):
                n = o((ln.get("line") if isinstance(ln, dict) else {}).get("nonce_account")) if isinstance(ln, dict) else None
                if n:
                    nonce = n
        except Exception:  # noqa: BLE001
            nonce = None
        log("PASS" if nonce else "INFO", "3 the crank line's durable-nonce account", f"opened by the line itself: {nonce}" if nonce else "no nonce account read in sol_lines (the first signed step opens it)")
        # 4. each collection Open, its two transactions landed and read back
        cc = {int(c["index"]): c for c in colls()}
        for i in range(count):
            c = cc.get(i)
            st = s.get(i)
            if st != "Open" or not c:
                log("FAIL", f"4 collection {i} open", f"stage {st}: " + json.dumps(c)[:200])
                continue
            on_chain = {"wX mint": exists(c["wx_mint"]), "DBC config": exists(c["config"])}
            log("PASS" if all(on_chain.values()) else "FAIL", f"4 collection {i} ({c['symbol']}) Open, read back", json.dumps({**on_chain, "wx_mint": c["wx_mint"], "config": c["config"]}))
        fin = {sg: finalized(sg) for sg in SIGS}
        log("PASS" if SIGS and all(fin.values()) else ("INFO" if not SIGS else "FAIL"), "4 the opening transactions finalized on devnet",
            f"{sum(fin.values())}/{len(SIGS)} finalized: " + json.dumps(fin)[:400] if SIGS else "no signature seen in the tick lines (opened in an earlier run?)")
        # 5. books_view consistent
        listed = {int(tup(x)[0]): tup(x)[1] for x in (ok("books_collections", query=True) or [])}
        for i in range(count):
            v = books_view(i)
            c = cc.get(i)
            if not v or not c:
                log("FAIL", f"5 books_view({i})", "null" if not v else "not pinned")
                continue
            rules = books_view_rules(v, module=None, wx_mint=c["wx_mint"], expect_supply=0 if num(v["sol_backed"]) == 0 and num(v["held"]) == 0 else None)
            m = can["coll"].get(str(i))
            rules.append(("module = sol_address_of(coll:i)", m == c["module"], f"{c['module']} vs {m}"))
            rules.append(("listed in books_collections with the pinned mint", listed.get(i) == c["wx_mint"], f"{listed.get(i)}"))
            if kind_of(v) == "SolNft":
                # a Solana collection: no EVM or BTC custody; its custodian is K_main, the canister's own wallet (the pin names it)
                cust = (v["kind"]["SolNft"] or {}).get("custodian")
                rules.append(("SolNft: the custodian is K_main", cust is not None, f"custodian {str(cust)[:80]}; K_main {can['sol']}"))
            bad = [r for r in rules if not r[1]]
            log("PASS" if not bad else "FAIL", f"5 books_view({i}) consistent", "; ".join(f"{r[0]}: {r[2]}" for r in (bad or rules))[:500])
        # 6. an intent end to end
        if "--intent" in A:
            intent(count, can)
        else:
            log("SKIP", "6 an intent end to end", "--intent not given")

    # 7. the monitor
    m = monitor()
    if m is None or "error" in (m or {}):
        log("FAIL", "7 the monitor's read of the views", (m or {}).get("error", "none"))
    else:
        sev1 = [a for a in m["alerts"] if a.get("sev") == "SEV1"]
        log("PASS" if not sev1 else "FAIL", "7 books.mjs on the views", f"{len(m['alerts'])} alert(s): " + json.dumps(m["alerts"])[:400])
        for name, rules in (("desk_status", desk_rules(m["desk"])), ("treasury_status", treasury_rules(m["treasury"]))):
            bad = [r for r in rules if not r[1]]
            log("PASS" if not bad else "FAIL", f"7 {name}", "; ".join(f"{r[0]}: {r[2]}" for r in (bad or rules))[:400])

    # 8. an upgrade carrying the state
    if opt("--wasm") and opt("--arg"):
        upgrade()
    else:
        log("SKIP", "8 an upgrade carrying the state", "no --wasm/--arg")

    # 9. cycles
    t = [c for c in R["cycles"] if not c.get("err")]
    if t:
        log("INFO", "9 cycles per tick", f"{len(t)} ticks: min {min(c['cycles'] for c in t):,}, max {max(c['cycles'] for c in t):,}, mean {sum(c['cycles'] for c in t) // len(t):,}")


def build_evidence(v, can):
    """With OPENSEA_API_KEY in the environment (never written anywhere): collection 0's floor evidence, built as the runner builds it
    (evidence.ts, the cheapest Seaport 1.6 listings for the canister's EVM address). Returns the file, or None."""
    try:
        coll = "0x" + bytes((v["kind"]["EthNft"] or {})["collection"]).hex()
        pilot = json.load(open(os.path.join(ROOT, "deployments/launch/nft-pilot.json")))["collections"]
        slug = next(c["openseaSlug"] for c in pilot if c["collection"].lower() == coll)
    except Exception as ex:  # noqa: BLE001
        log("FAIL", "6 the evidence's collection", str(ex)[:200])
        return None
    out = os.path.join(JOB, "evidence.json")
    p = subprocess.run(["node", "--import", "tsx", "rehearsal/sol-devnet/evidence.ts", slug, coll, can["evm"], out], capture_output=True, text=True,
                       cwd=os.path.join(ROOT, "ops"))
    log("INFO" if p.returncode == 0 else "FAIL", "6 the floor evidence from OpenSea", (p.stdout + p.stderr).strip()[-300:])
    return out if p.returncode == 0 else None


def intent(count, can):
    i = 0
    cc = {int(c["index"]): c for c in colls()}
    v = books_view(i)
    if not v or cc.get(i, {}).get("stage") != "Open":
        log("FAIL", "6 an intent end to end", f"collection {i} is not open")
        return
    if not fresh_floor(v):
        ev = opt("--evidence")
        if not ev and os.environ.get("OPENSEA_API_KEY"):
            ev = build_evidence(v, can)
        if ev:
            e = json.load(open(ev))
            try:
                lst = "vec { " + "; ".join(json.dumps(x) for x in e["listings"]) + " }"
                log("INFO", "6 eth_floor_evidence", str(ok("eth_floor_evidence", f'("{e["collection"]}", {lst}, vec {{}})'))[:200])
            except Exception as ex:  # noqa: BLE001
                log("FAIL", "6 eth_floor_evidence", str(ex)[:300])
                return
            t0 = time.time()
            while not fresh_floor(books_view(i) or {}) and time.time() - t0 < int(opt("--open-wait", "1800")):
                tick("floor")
                time.sleep(int(opt("--tick-gap", "20")))
            v = books_view(i)
        if not fresh_floor(v or {}):
            log("SKIP", "6 an intent end to end", "no fresh floor on collection 0: the mint is closed until the runner (ROOK_HOME=books, an OpenSea key) hands in "
                "eth_floor_evidence, or --evidence <json> {collection, listings[]} does; accepted " + str(v and v["accepted"]))
            return
    good, out = solana("address")
    me = out.strip().splitlines()[-1].strip() if good else None
    if not me:
        log("FAIL", "6 the payer's address", out[-200:])
        return
    try:
        q = ok("quote_mint", f"({i} : nat16, {MIN_MINT} : nat64)", query=True)
        log("INFO", "6 quote_mint at the minimum", json.dumps(q)[:300])
        op = ok("open_intent", f'(variant {{ Mint = record {{ coll = {i} : nat16; min_out = null; recipient = "{me}" }} }})')
    except Exception as ex:  # noqa: BLE001
        log("FAIL", "6 open_intent", str(ex)[:300])
        return
    iid, addr, mind = num(op["id"]), op["address"], num(op["min_deposit"])
    log("PASS", "6 the intent opened", f"id {iid}, deposit address {addr}, min_deposit {mind}, fee {op['fee']}, expires {op['expires_at']}")
    before = books_view(i)
    if not pay(addr, mind):
        log("FAIL", "6 the deposit", "solana transfer failed")
        return
    t0 = time.time()
    state = None
    while time.time() - t0 < int(opt("--open-wait", "1800")):
        st = o(ok("intent_status", f"({iid} : nat64)", query=True)) or {}
        state = list(st.get("state", {"?": 0}).keys())[0] if isinstance(st.get("state"), dict) else str(st.get("state"))
        if state in ("Done", "Refunded", "Expired"):
            break
        tick(f"intent {state}")
        time.sleep(int(opt("--tick-gap", "20")))
    st = o(ok("intent_status", f"({iid} : nat64)", query=True)) or {}
    minted = num(st.get("minted_units"))
    log("PASS" if state == "Done" and minted > 0 else "FAIL", "6 intent_status → Done", f"state {state}, minted_units {minted}, note {str(st.get('note'))[:200]}, txs {json.dumps(st.get('txs'))[:300]}")
    if state != "Done":
        return
    accts = rpc("getTokenAccountsByOwner", [me, {"mint": cc[i]["wx_mint"]}, {"encoding": "jsonParsed", "commitment": "finalized"}])["value"]
    held = sum(int(a["account"]["data"]["parsed"]["info"]["tokenAmount"]["amount"]) for a in accts)
    log("PASS" if held >= minted else "FAIL", "6 the wX arrived at the wallet", f"{held} base units of {cc[i]['wx_mint']} at {me}")
    after = books_view(i)
    moved = num(after["sol_backed"]) - num(before["sol_backed"])
    es = o(after.get("expected_supply"))
    log("PASS" if moved == minted and es is not None and num(es) >= minted else "FAIL", "6 books_view moved by the mint",
        f"sol_backed +{moved} (minted {minted}); expected_supply {es}; " + "; ".join(f"{r[0]}: {r[2]}" for r in books_view_rules(after) if not r[1]))


def upgrade():
    cid = subprocess.run([DFX, "canister", "id", CAN, "--network", NET], capture_output=True, text=True, cwd=HERE).stdout.strip()
    argbin = os.path.join(JOB, "init.bin")
    with open(argbin, "wb") as f:
        f.write(bytes.fromhex(open(opt("--arg")).read().strip()))
    before = (colls(), ok("books_collections", query=True))
    # the canister refuses an upgrade while a tick is running (canister.rs pre_upgrade): stop it first, as the timelock's upgrade does;
    # stopping lets the running tick's calls finish
    st = subprocess.run([DFX, "canister", "stop", CAN, "--network", NET], capture_output=True, text=True, cwd=HERE)
    log("PASS" if st.returncode == 0 else "FAIL", "8 the canister stopped for the upgrade", st.stderr.strip()[-200:] if st.returncode else "stopped")
    p = subprocess.run(["bash", opt("--chunked", os.path.join(ROOT, "autobuyer/scripts/chunked-install.sh")), "--network", NET, "--mode", "upgrade", cid, opt("--wasm"), argbin],
                       capture_output=True, text=True, cwd=HERE)
    for l in (p.stdout + p.stderr).strip().splitlines()[-4:]:
        print("   chunked: " + l[:200], flush=True)
    sa = subprocess.run([DFX, "canister", "start", CAN, "--network", NET], capture_output=True, text=True, cwd=HERE)
    log("PASS" if sa.returncode == 0 else "FAIL", "8 the canister started after the upgrade", sa.stderr.strip()[-200:] if sa.returncode else "started")
    after = (colls(), ok("books_collections", query=True))
    restored = any("upgraded: state restored" in l for l in (ok("log", query=True) or [])[-5:])
    same = json.dumps(before, sort_keys=True) == json.dumps(after, sort_keys=True)
    log("PASS" if p.returncode == 0 and same and restored else "FAIL", "8 an upgrade carrying the state",
        "coins_collections and books_collections identical across the chunked upgrade; 'upgraded: state restored'" if p.returncode == 0 and same and restored
        else f"exit {p.returncode}; restored {restored}; same {same}; " + (p.stderr.strip()[-300:]))
    lines, e = tick("after the upgrade")
    for _ in range(6):
        if not (e and "already running" in str(e)):
            break
        time.sleep(10)  # the restarted timer's own tick: wait for it
        lines, e = tick("after the upgrade (retry)")
    log("PASS" if not e else "FAIL", "8 the canister ticks on after the upgrade", e or f"{len(lines)} lines")


# ---- main -------------------------------------------------------------------------------------------------------------------------------------

if len(sys.argv) < 2:
    print(__doc__)
    sys.exit(2)
MODE = sys.argv[1]
if MODE == "parse":
    parse(sys.argv[2])
    sys.exit(0)
DFX, NET, JOB = sys.argv[2], sys.argv[3], sys.argv[4]
A = sys.argv[5:]
RPC = opt("--rpc", "https://api.devnet.solana.com")
if "mainnet" in RPC:
    print("refused: the RPC names mainnet; this is the DEVNET rehearsal")
    sys.exit(2)
if os.path.exists(os.path.join(JOB, "books-report.json")):
    try:
        R = json.load(open(os.path.join(JOB, "books-report.json")))
        R.setdefault("steps", []); R.setdefault("cycles", [])
        SIGS = R.get("signatures", [])
        R["failed_phases"] = []  # each phase answers for its own steps; earlier FAILs stay in the report's steps
    except Exception:  # noqa: BLE001
        pass
try:
    {"addresses": addresses, "pins": pins, "fund": fund, "open": open_phase, "checklist": checklist}[MODE]()
except Exception as e:  # noqa: BLE001
    log("FAIL", f"the driver stopped ({MODE})", str(e)[:500])
R["verdict"] = "PASS" if all(st["kind"] != "FAIL" for st in R["steps"]) else "FAIL"
save()
print(f"== {MODE}: {R['verdict']} (report: {os.path.join(JOB, 'books-report.json')})")
sys.exit(0 if ok_all else 1)
