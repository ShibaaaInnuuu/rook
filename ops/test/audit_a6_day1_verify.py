#!/usr/bin/env python3
"""Audit A6 (docs/v9/audit-2026-10/a6-launch-ops.md): an INDEPENDENT check of the day-1 plan.

Shares no code with ops/rehearsal/sol-devnet/tool (rook-ceremony) or rook_backing: its own base58, ed25519 on-curve test, PDA
derivation, legacy-message parser, Borsh decoder of DBC 0.2.1's ConfigParameters (field order read from Meteora's source at
f552f20, process_create_config.rs:40-71, fee_parameters.rs), DBC curve math (liquidity_distribution.rs, curve.rs,
concentrated_liquidity.rs), and PoolConfig / VirtualPool byte offsets (state/config.rs, state/virtual_pool.rs).

  python3 -I ops/test/audit_a6_day1_verify.py --plan <keys>/day1/day1-plan.json --launch <launch.json>
        [--rpc https://api.mainnet-beta.solana.com --simulate]   (read-only: simulateTransaction, sigVerify off, NOTHING sent)

Exit 0 when every check passes. No secret is read: only the plan (public keys and unsigned messages) and the launch file.
"""
import argparse, base64, hashlib, json, sys, urllib.request

# ---- base58 ---------------------------------------------------------------------------------------------------------------
A = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
def b58d(s):
    n = 0
    for c in s: n = n * 58 + A.index(c)
    raw = n.to_bytes((n.bit_length() + 7) // 8, "big") if n else b""
    pad = len(s) - len(s.lstrip("1"))
    out = b"\0" * pad + raw
    assert len(out) == 32, (s, len(out))
    return out
def b58e(b):
    n = int.from_bytes(b, "big"); s = ""
    while n: n, r = divmod(n, 58); s = A[r] + s
    return "1" * (len(b) - len(b.lstrip(b"\0"))) + s

# ---- ed25519 on-curve (RFC 8032 point decoding) --------------------------------------------------------------------------
P = 2**255 - 19
D = (-121665 * pow(121666, P - 2, P)) % P
def on_curve(b):
    y = int.from_bytes(b, "little") & ((1 << 255) - 1)
    if y >= P: return False
    u, v = (y * y - 1) % P, (D * y * y + 1) % P
    x2 = u * pow(v, P - 2, P) % P
    if x2 == 0: return True
    return pow(x2, (P - 1) // 2, P) == 1
def pda(seeds, program):
    pid = b58d(program)
    for bump in range(255, -1, -1):
        h = hashlib.sha256(b"".join(seeds) + bytes([bump]) + pid + b"ProgramDerivedAddress").digest()
        if not on_curve(h): return b58e(h), bump
    raise ValueError("no bump")

DBC = "dbcij3LWUppWqq96dh6gJWwBifmcGfLSB5D4DuSMaqN"
WSOL = "So11111111111111111111111111111111111111112"
SYS = "11111111111111111111111111111111"
SPL = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"
MPL = "metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s"
CB = "ComputeBudget111111111111111111111111111111"
FUND = "GGgXnNgtY4ySfz8PLAAwkoh8Ztj7p6jrM9towSaa6Wuy"
POOL_AUTH = pda([b"pool_authority"], DBC)[0]
EVT = pda([b"__event_authority"], DBC)[0]
disc = lambda name: hashlib.sha256(f"global:{name}".encode()).digest()[:8]

# ---- legacy message ------------------------------------------------------------------------------------------------------
def shortvec(b, i):
    n = s = 0
    while True:
        x = b[i]; i += 1; n |= (x & 0x7F) << s; s += 7
        if not x & 0x80: return n, i
def parse_msg(m):
    assert m[0] & 0x80 == 0, "legacy message expected"
    nsig, nro_s, nro_u = m[0], m[1], m[2]; i = 3
    n, i = shortvec(m, i); keys = [b58e(m[i + 32 * k:i + 32 * k + 32]) for k in range(n)]; i += 32 * n
    bh = m[i:i + 32]; i += 32
    n, i = shortvec(m, i); ixs = []
    for _ in range(n):
        p = m[i]; i += 1
        k, i = shortvec(m, i); acc = list(m[i:i + k]); i += k
        k, i = shortvec(m, i); data = m[i:i + k]; i += k
        ixs.append((keys[p], [keys[a] for a in acc], data))
    assert i == len(m)
    signer = lambda k: keys.index(k) < nsig
    writable = lambda k: (keys.index(k) < nsig - nro_s) or (nsig <= keys.index(k) < len(keys) - nro_u)
    return dict(nsig=nsig, keys=keys, ixs=ixs, signer=signer, writable=writable, payer=keys[0])

# ---- Borsh: DBC 0.2.1 ConfigParameters ------------------------------------------------------------------------------------
class R:
    def __init__(s, b): s.b, s.i = b, 0
    def u(s, n):
        v = int.from_bytes(s.b[s.i:s.i + n], "little"); s.i += n; return v
    def take(s, n):
        v = s.b[s.i:s.i + n]; s.i += n; return v
def config_params(r):
    o = {}
    o["base_fee"] = dict(cliff_fee_numerator=r.u(8), first_factor=r.u(2), second_factor=r.u(8), third_factor=r.u(8), base_fee_mode=r.u(1))
    o["dynamic_fee_some"] = r.u(1)
    assert o["dynamic_fee_some"] == 0, "dynamic fee present"
    for k in ["collect_fee_mode", "migration_option", "activation_type", "token_type", "token_decimal", "partner_liquidity_percentage",
              "partner_permanent_locked_liquidity_percentage", "creator_liquidity_percentage", "creator_permanent_locked_liquidity_percentage"]:
        o[k] = r.u(1)
    o["migration_quote_threshold"] = r.u(8); o["sqrt_start_price"] = r.u(16)
    o["locked_vesting"] = [r.u(8) for _ in range(5)]
    o["migration_fee_option"] = r.u(1)
    o["token_supply_some"] = r.u(1)
    if o["token_supply_some"]: o["pre_supply"], o["post_supply"] = r.u(8), r.u(8)
    o["creator_trading_fee_percentage"] = r.u(1); o["token_update_authority"] = r.u(1)
    o["migration_fee_percentage"], o["creator_migration_fee_percentage"] = r.u(1), r.u(1)
    o["migrated_pool_fee"] = (r.u(1), r.u(1), r.u(2))
    o["pool_creation_fee"] = r.u(8)
    o["partner_vesting"] = (r.u(1), r.u(2), r.u(2), r.u(4), r.u(4))
    o["creator_vesting"] = (r.u(1), r.u(2), r.u(2), r.u(4), r.u(4))
    o["migrated_pool_base_fee_mode"] = r.u(1)
    o["mcap_sched"] = (r.u(2), r.u(2), r.u(4), r.u(8))
    o["enable_first_swap_with_min_fee"] = r.u(1); o["compounding_fee_bps"] = r.u(2); o["padding"] = r.u(2)
    n = r.u(4); o["curve"] = [(r.u(16), r.u(16)) for _ in range(n)]
    assert r.i == len(r.b), f"trailing bytes {len(r.b) - r.i}"
    return o

# ---- DBC curve math (exact integers, rounding as in DBC) ------------------------------------------------------------------
MAX_SQRT = 79226673521066979257578248091; MIN_SQRT = 4295048016
cdiv = lambda a, b: -(-a // b)
def d_base(lo, hi, L, up=True):
    num, den = L * (hi - lo), lo * hi
    return cdiv(num, den) if up else num // den
def d_quote(lo, hi, L, up=True):
    p = L * (hi - lo)
    return cdiv(p, 1 << 128) if up else p >> 128
def migration_sqrt(th, s0, curve):
    tot = d_quote(s0, curve[0][0], curve[0][1])
    if tot > th: return s0 + (th << 128) // curve[0][1]
    left, sp = th - tot, curve[0][0]
    for c in curve[1:]:
        mx = d_quote(sp, c[0], c[1])
        if mx > left: return sp + (left << 128) // c[1]
        left -= mx; sp = c[0]
    raise ValueError("not enough liquidity")
def base_for_swap(s0, smig, curve):
    tot, lo = 0, s0
    for c in curve:
        if c[0] > smig: return tot + d_base(lo, smig, c[1])
        tot += d_base(lo, c[0], c[1]); lo = c[0]
    return tot
def curve_report(p):
    curve, s0, th, fee = p["curve"], p["sqrt_start_price"], p["migration_quote_threshold"], p["migration_fee_percentage"]
    smig = migration_sqrt(th, s0, curve)
    swap = base_for_swap(s0, smig, curve)
    quote_to_pool = cdiv(th * (100 - fee), 100)
    L = (quote_to_pool << 128) // (smig - MIN_SQRT)
    mig_base = d_base(smig, MAX_SQRT, L)
    buf = min(swap * 125 // 100, base_for_swap(s0, MAX_SQRT, curve))
    pre, post = p["pre_supply"], p["post_supply"]
    ok = (swap + mig_base <= post <= pre) and (buf + mig_base <= pre)
    price = lambda s: (s / 2**64) ** 2 * 1e6 / 1e9  # SOL per whole coin (6 decimals, lamports)
    return dict(valid_supply=ok, swap_base=swap, migration_base=mig_base, buffer=buf, leftover=pre - swap - mig_base,
                quote_to_pool=quote_to_pool, partner_migration_fee=th - quote_to_pool,
                start_price=price(s0), start_mcap=price(s0) * pre / 1e6, grad_price=price(smig), grad_mcap=price(smig) * pre / 1e6,
                pool_price_check=(quote_to_pool / 1e9) / (mig_base / 1e6))

# ---- checks ------------------------------------------------------------------------------------------------------------------
FAIL = []
def check(ok, what):
    print(("  ok    " if ok else "  FAIL  ") + what)
    if not ok: FAIL.append(what)

COLLECTION = dict(cliff=500_000_000, first=60, second=10, third=8_166_666, mode=0, th=85_000_000_000, fee=50, cfee=0, ctrade=0)
ROOK = dict(COLLECTION, fee=0, ctrade=100)
def check_params(p, want, tag):
    bf = p["base_fee"]
    check((bf["cliff_fee_numerator"], bf["first_factor"], bf["second_factor"], bf["third_factor"], bf["base_fee_mode"]) ==
          (want["cliff"], want["first"], want["second"], want["third"], want["mode"]), f"{tag}: base fee 50 % -> linear, 60 x 10 s")
    end = bf["cliff_fee_numerator"] - bf["first_factor"] * bf["third_factor"]
    check(2_500_000 <= end <= 990_000_000, f"{tag}: fee after 10 min = {end / 1e7:.6f} % (DBC min 0.25 %)")
    check((p["collect_fee_mode"], p["migration_option"], p["activation_type"], p["token_type"], p["token_decimal"]) == (0, 1, 1, 0, 6),
          f"{tag}: fees in quote, DAMM v2, timestamp activation, SPL, 6 decimals")
    check((p["partner_liquidity_percentage"], p["partner_permanent_locked_liquidity_percentage"], p["creator_liquidity_percentage"],
           p["creator_permanent_locked_liquidity_percentage"], p["partner_vesting"][0], p["creator_vesting"][0]) == (0, 50, 0, 50, 0, 0),
          f"{tag}: LP 50/50 permanently locked, nothing claimable or vesting")
    check(p["migration_quote_threshold"] == want["th"], f"{tag}: {want['th'] / 1e9:g} SOL threshold")
    check((p["migration_fee_percentage"], p["creator_migration_fee_percentage"]) == (want["fee"], want["cfee"]), f"{tag}: migration fee {want['fee']} %, creator share 0")
    check(p["creator_trading_fee_percentage"] == want["ctrade"], f"{tag}: creator trading fee {want['ctrade']} %")
    check(p["locked_vesting"] == [0] * 5, f"{tag}: no locked vesting")
    check((p["migration_fee_option"], p["migrated_pool_fee"], p["migrated_pool_base_fee_mode"], p["mcap_sched"], p["compounding_fee_bps"]) ==
          (6, (0, 0, 100), 0, (0, 0, 0, 0), 0), f"{tag}: customizable DAMM v2, 1 % flat pool fee in SOL, no scheduler/compounding")
    check(p["token_supply_some"] == 1 and (p["pre_supply"], p["post_supply"]) == (10**15, 10**15), f"{tag}: fixed 1B supply pre = post")
    check(p["token_update_authority"] == 1, f"{tag}: immutable metadata (token_update_authority 1)")
    check(p["pool_creation_fee"] == 0, f"{tag}: no pool creation fee")
    check(p["enable_first_swap_with_min_fee"] == 0 and p["padding"] == 0, f"{tag}: no min-fee first swap; padding 0")
    c = curve_report(p)
    check(c["valid_supply"], f"{tag}: DBC fixed-supply validation passes (swap {c['swap_base'] / 1e6:,.3f} + pool {c['migration_base'] / 1e6:,.3f} coins; buffer {c['buffer'] / 1e6:,.3f})")
    check(0 <= c["leftover"] <= 10**8, f"{tag}: leftover to leftover_receiver {c['leftover'] / 1e6:,.6f} coins (<= 100 coins)")
    print(f"        start {c['start_price']:.4e} SOL/coin (mcap {c['start_mcap']:.2f} SOL), graduation {c['grad_price']:.4e} (mcap {c['grad_mcap']:.2f} SOL);"
          f" to the pool {c['quote_to_pool'] / 1e9} SOL, migration fee {c['partner_migration_fee'] / 1e9} SOL; pool price {c['pool_price_check']:.4e}")
    return c

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--plan", required=True); ap.add_argument("--launch", required=True)
    ap.add_argument("--pilot", default=None); ap.add_argument("--rpc"); ap.add_argument("--simulate", action="store_true")
    ap.add_argument("--sim-payer", default="5tzFkiKscXHK5ZXCGbXZxdw7gTjjD1mBwuoFbhUvuAi9")
    a = ap.parse_args()
    plan, launch = json.load(open(a.plan)), json.load(open(a.launch))
    pilot = json.load(open(a.pilot or __file__.rsplit("/ops/", 1)[0] + "/deployments/launch/nft-pilot.json"))
    core, back, payer, fund = plan["rook_core"], plan["rook_backing"], plan["ceremony_payer"], plan["fund_wallet"]
    print(f"rook_core {core}  rook_backing {back}  payer {payer}  fund {fund}")
    check(fund == FUND, "fund wallet = GGgXnNgt…6Wuy")
    check(plan["rook_backing_converter"] == pda([b"converter"], back)[0], "converter PDA recomputed")
    check(plan["rook_core_registry"] == pda([b"registry"], core)[0], "registry PDA recomputed")
    # docs/v9/ordinals-collections.md: the Ethereum collections, Bitcoin Frogs (11), then nft-pilot.json `inscriptions` (12+)
    want_order = [c["slug"] for c in pilot["collections"]] + [pilot["bitcoin"]["slug"]] + [b["slug"] for b in pilot.get("inscriptions", [])]
    n = len(want_order)
    check([l["slug"] for l in plan["listings"]] == want_order and want_order[11] == "bitcoin-frogs" and n == 26,
          f"listing order = nft-pilot.json order, Bitcoin Frogs at 11, {n} listings: {','.join(want_order)}")
    check([l["slug"] for l in launch["listing_order"]["listings"]] == want_order and
          all(l["listing_id"] == i for i, l in enumerate(launch["listing_order"]["listings"])), f"launch file listing_order ids 0..{n - 1} in that order")
    # docs/v9/graduation-tiers.md: each listing's tier, from graduation-tiers.json, is the launch file's graduation_sol
    tiers = json.load(open(__file__.rsplit("/ops/", 1)[0] + "/deployments/launch/graduation-tiers.json"))
    tier_of = {l["slug"]: l.get("graduation_sol") for l in launch["listing_order"]["listings"]}
    check(all(tier_of[s] == tiers["collections"][s]["tier"] and tier_of[s] in tiers["tiers_sol"] for s in want_order),
          f"every listing's graduation_sol is graduation-tiers.json's (tiers {tiers['tiers_sol']} SOL)")
    first_of_tier = {}
    configs = set()
    for i, l in enumerate(plan["listings"]):
        tag = f"[{i:02} {l['slug']}]"
        listing = pda([b"listing", i.to_bytes(4, "little")], core)[0]
        claimer = pda([b"claimer", b58d(listing)], back)[0]
        check((l["listing"], l["claimer"]) == (listing, claimer), f"{tag} listing {listing} claimer {claimer} recomputed")
        check(not on_curve(b58d(claimer)), f"{tag} claimer is off-curve (no private key exists)")
        m = parse_msg(base64.b64decode(l["unsigned_message_b64"]))
        check(m["payer"] == payer and m["nsig"] == 2 and m["keys"][1] == l["config"], f"{tag} signers = [payer, config]; fee payer = ceremony payer")
        check(FUND not in m["keys"], f"{tag} the fund wallet is not in the transaction")
        check(len(m["ixs"]) == 2 and m["ixs"][0][0] == CB and m["ixs"][1][0] == DBC, f"{tag} [compute budget, DBC]")
        prog, acc, data = m["ixs"][1]
        check(data[:8] == disc("create_config"), f"{tag} create_config discriminator")
        check(acc == [l["config"], claimer, claimer, WSOL, payer, SYS, EVT, DBC], f"{tag} accounts: config, fee_claimer = leftover = claimer, wSOL, payer, system, event authority, DBC")
        check(m["writable"](l["config"]) and m["writable"](payer) and not m["writable"](claimer), f"{tag} only config and payer writable")
        p = config_params(R(data[8:]))
        t = tier_of[l["slug"]]
        check(l.get("graduation_sol") == t, f"{tag} the plan's graduation_sol is the launch file's ({t} SOL)")
        if t not in first_of_tier:
            check_params(p, dict(COLLECTION, th=t * 10**9), f"collection tier {t} SOL")
            first_of_tier[t] = p
        else:
            check(p == first_of_tier[t], f"{tag} parameters byte-identical to the first {t} SOL listing's")
        configs.add(l["config"])
    check(len(configs) == n, f"{n} distinct config keypairs")
    g = plan["genesis_rook"]
    m = parse_msg(base64.b64decode(g["unsigned_message_b64"]))
    check(m["nsig"] == 3 and m["payer"] == payer and set(m["keys"][:3]) == {payer, g["config"], g["mint"]}, "ROOK: signers = payer, config, mint")
    check(FUND in m["keys"] and not m["signer"](FUND) and not m["writable"](FUND), "ROOK: fund wallet appears read-only and never signs")
    check([ix[0] for ix in m["ixs"]] == [CB, DBC, DBC, DBC], "ROOK: [compute budget, create_config, initialize_virtual_pool_with_spl_token, transfer_pool_creator]")
    _, acc, data = m["ixs"][1]
    check(data[:8] == disc("create_config") and acc == [g["config"], FUND, FUND, WSOL, payer, SYS, EVT, DBC], "ROOK create_config: fee_claimer = leftover = fund wallet")
    rp = config_params(R(data[8:]))
    check_params(rp, ROOK, "ROOK")
    hi, lo = (g["mint"], WSOL) if b58d(g["mint"]) > b58d(WSOL) else (WSOL, g["mint"])
    pool = pda([b"pool", b58d(g["config"]), b58d(hi), b58d(lo)], DBC)[0]
    _, acc, data = m["ixs"][2]
    want = [g["config"], POOL_AUTH, payer, g["mint"], WSOL, pool, pda([b"token_vault", b58d(g["mint"]), b58d(pool)], DBC)[0],
            pda([b"token_vault", b58d(WSOL), b58d(pool)], DBC)[0], pda([b"metadata", b58d(MPL), b58d(g["mint"])], MPL)[0], MPL, payer, SPL, SPL, SYS, EVT, DBC]
    check(data[:8] == disc("initialize_virtual_pool_with_spl_token") and acc == want and pool == g["pool"], f"ROOK pool {pool}: accounts recomputed")
    r = R(data[8:]); md = [r.take(r.u(4)).decode() for _ in range(3)]
    check(r.i == len(r.b) and md[:2] == [g["name"], g["symbol"]], f"ROOK metadata {md}")
    check(md[2] != "", "ROOK metadata uri set (empty = placeholder: terminals show no image)")
    _, acc, data = m["ixs"][3]
    check(data == disc("transfer_pool_creator") and acc == [pool, g["config"], payer, FUND, EVT, DBC], "ROOK transfer_pool_creator(pool, config, payer -> fund wallet)")
    if a.simulate:
        simulate(a.rpc, plan, a.sim_payer)
    print(f"\n{len(FAIL)} failure(s)")
    sys.exit(1 if FAIL else 0)

def rpc(url, method, params):
    req = urllib.request.Request(url, json.dumps(dict(jsonrpc="2.0", id=1, method=method, params=params)).encode(), {"content-type": "application/json"})
    return json.load(urllib.request.urlopen(req, timeout=60))

def simulate(url, plan, sim_payer):
    """Read-only: each unsigned message with the fee payer key swapped for a funded account; sigVerify off; nothing is sent."""
    import time
    print("\nsimulation on", url)
    payer = b58d(plan["ceremony_payer"])
    rows = plan["listings"] + [plan["genesis_rook"]]
    for k, l in enumerate(rows):
        msg = bytearray(base64.b64decode(l["unsigned_message_b64"]))
        at = msg.find(payer); msg[at:at + 32] = b58d(sim_payer)
        n = msg[0]; tx = bytes([n]) + b"\0" * 64 * n + bytes(msg)
        addrs = [l["config"]] + ([l["pool"]] if "pool" in l else [])
        for attempt in range(6):
            j = rpc(url, "simulateTransaction", [base64.b64encode(tx).decode(), {"encoding": "base64", "sigVerify": False, "replaceRecentBlockhash": True,
                    "commitment": "confirmed", "accounts": {"encoding": "base64", "addresses": addrs}}])
            if "error" not in j: break
            time.sleep(3 + 3 * attempt)
        v = j["result"]["value"]
        tag = l.get("slug", "ROOK")
        check(v["err"] is None, f"sim {tag}: {v['err']} ({v.get('unitsConsumed')} CU)")
        if v["err"] is not None: continue
        a = v["accounts"][0]; d = base64.b64decode(a["data"][0])
        u = lambda o, n: int.from_bytes(d[o:o + n], "little")
        k32 = lambda o: b58e(d[o:o + 32])
        claim = l.get("claimer", FUND)
        check(a["owner"] == DBC and len(d) == 1048 and d[:8] == hashlib.sha256(b"account:PoolConfig").digest()[:8], f"sim {tag}: a DBC PoolConfig of 1048 bytes")
        check((k32(8), k32(40), k32(72)) == (WSOL, claim, claim), f"sim {tag}: written quote wSOL, fee_claimer = leftover = {claim}")
        check((u(104, 8), u(112, 8), u(120, 8), u(128, 2), u(130, 1)) == (500_000_000, 10, 8_166_666, 60, 0), f"sim {tag}: written base fee")
        check((d[232], d[233], d[234], d[235], d[237]) == (0, 1, 1, 6, 0), f"sim {tag}: written collect 0, DAMM v2, timestamp, 6 dec, SPL")
        check((d[239], d[240], d[241], d[242]) == (50, 0, 50, 0), f"sim {tag}: written LP 50/50 locked")
        fee, ct = (0, 100) if tag == "ROOK" else (50, 0)
        check((d[243], d[244], d[245], d[246], d[247], d[248]) == (6, 1, ct, 1, fee, 0), f"sim {tag}: written fee option 6, fixed supply, creator share {ct}, immutable, migration fee {fee}")
        th = 85 * 10**9 if tag == "ROOK" else l["graduation_sol"] * 10**9  # docs/v9/graduation-tiers.md (main checked it is the launch file's)
        check((u(264, 8), u(344, 8), u(352, 8), u(362, 2), u(368, 8)) == (th, 10**15, 10**15, 100, 0), f"sim {tag}: written threshold {th / 1e9:g} SOL, supply, 1 % pool fee, creation fee 0")
        print(f"        swap_base {u(256, 8) / 1e6:,.6f} coins, migration_base {u(272, 8) / 1e6:,.6f}, leftover {(10**15 - u(256, 8) - u(272, 8)) / 1e6:,.6f}")
        if tag == "ROOK":
            pd = base64.b64decode(v["accounts"][1]["data"][0])
            check(b58e(pd[104:136]) == FUND and b58e(pd[72:104]) == l["config"] and b58e(pd[136:168]) == l["mint"],
                  "sim ROOK: pool.creator = fund wallet after transfer_pool_creator; config and mint match")
        time.sleep(1)

if __name__ == "__main__":
    main()
