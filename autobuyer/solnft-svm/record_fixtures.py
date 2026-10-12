#!/usr/bin/env python3
"""Record the Solana NFT fixtures (docs/v9/solana-nfts-canister.md §7): real mainnet accounts, read-only, at `finalized`.

    python3 -I autobuyer/solnft-svm/record_fixtures.py [RPC_URL]

Writes autobuyer/canister-rs/tests/fixtures/solnft/<scenario>.json:
  {"scenario", "note", "slot", "unix_time", "accounts": [{"pubkey", "lamports", "owner", "executable", "data" (base64)}],
   "absent": [pubkeys that do not exist], "facts": {...what the scenario is about}}
One getMultipleAccounts per scenario, so every account of a scenario is from the same slot. Nothing is signed or sent.
"""
import base64, hashlib, json, os, struct, sys, time, urllib.request

RPC = sys.argv[1] if len(sys.argv) > 1 else "https://api.mainnet-beta.solana.com"
OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "canister-rs", "tests", "fixtures", "solnft")

A = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
def b58d(s):
    n = 0
    for c in s: n = n * 58 + A.index(c)
    b = n.to_bytes((n.bit_length() + 7) // 8, "big") if n else b""
    return b"\0" * (len(s) - len(s.lstrip("1"))) + b
def b58e(b):
    n = int.from_bytes(b, "big"); s = ""
    while n: n, r = divmod(n, 58); s = A[r] + s
    return "1" * (len(b) - len(b.lstrip(b"\0"))) + s

P = 2**255 - 19; D = -121665 * pow(121666, P - 2, P) % P
def on_curve(b):
    y = int.from_bytes(b, "little") & ((1 << 255) - 1)
    if y >= P: return False
    u = (y * y - 1) % P; v = (D * y * y + 1) % P
    x2 = u * pow(v, P - 2, P) % P
    return x2 == 0 or pow(x2, (P - 1) // 2, P) == 1
def pda(seeds, prog):
    p = b58d(prog)
    for bump in range(255, -1, -1):
        h = hashlib.sha256(b"".join(seeds) + bytes([bump]) + p + b"ProgramDerivedAddress").digest()
        if not on_curve(h): return b58e(h)

TM = "metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s"
TOKEN = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"
ATA = "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL"
CORE = "CoREENxT6tW1HoK8ypY1SxRMZTcVPm7R94rH4PZNhX7d"
TFEE = "TFEEgwDP6nn1s8mMX2tTNPPz8j2VomkphLUmyxKm17A"
def metadata(m): return pda([b"metadata", b58d(TM), b58d(m)], TM)
def edition(m): return pda([b"metadata", b58d(TM), b58d(m), b"edition"], TM)
def record(m, ta): return pda([b"metadata", b58d(TM), b58d(m), b"token_record", b58d(ta)], TM)
def ata(w, m): return pda([b58d(w), b58d(TOKEN), b58d(m)], ATA)

def rpc(method, params):
    for i in range(8):
        try:
            req = urllib.request.Request(RPC, data=json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode(),
                                         headers={"content-type": "application/json"})
            j = json.loads(urllib.request.urlopen(req, timeout=60).read())
            if "error" in j: raise RuntimeError(j["error"])
            time.sleep(0.3); return j["result"]
        except Exception as e:
            print("retry", method, e, file=sys.stderr); time.sleep(3)
    raise SystemExit(f"{method} failed")

def fetch(keys):
    r = rpc("getMultipleAccounts", [keys, {"encoding": "base64", "commitment": "finalized"}])
    return r["context"]["slot"], dict(zip(keys, r["value"]))

def one(k):
    return base64.b64decode(rpc("getAccountInfo", [k, {"encoding": "base64", "commitment": "finalized"}])["value"]["data"][0])

def tm_extras(mint):
    """The rule set and the creators of a Token Metadata item, read from its metadata."""
    d = one(metadata(mint)); o = 1 + 64
    for _ in range(3): n = struct.unpack_from("<I", d, o)[0]; o += 4 + n
    o += 2; creators = []
    if d[o]:
        n = struct.unpack_from("<I", d, o + 1)[0]; o += 5
        for i in range(n): creators.append(b58e(d[o:o + 32])); o += 34
    else: o += 1
    o += 2
    o += 2 if d[o] else 1          # edition nonce
    o += 2 if d[o] else 1          # token standard
    o += 34 if d[o] else 1         # collection
    o += 18 if d[o] else 1         # uses
    if d[o]: o += 1 + (9 if d[o + 1] == 0 else 9)
    else: o += 1                   # collection details
    rule_set = None
    if o < len(d) and d[o] and d[o + 2]: rule_set = b58e(d[o + 3:o + 35])
    return rule_set, creators

def core_extras(asset):
    """The collection and the Royalties creators of a Core asset (asset, then collection plugins)."""
    out = []; d = one(asset); coll = b58e(d[34:66]) if d[33] == 2 else None
    for acct in [d] + ([one(coll)] if coll else []):
        out += core_creators(acct)
    return coll, out

def core_creators(d):
    o = 1 + 32
    if d[0] == 1:
        o += 1 + (32 if d[33] != 0 else 0)
        for _ in range(2): n = struct.unpack_from("<I", d, o)[0]; o += 4 + n
        o += 9 if d[o] else 1
    else:
        for _ in range(2): n = struct.unpack_from("<I", d, o)[0]; o += 4 + n
        o += 8
    if o >= len(d) or d[o] != 3: return []
    reg = struct.unpack_from("<Q", d, o + 1)[0]; r = reg + 1
    n = struct.unpack_from("<I", d, r)[0]; r += 4; out = []
    for _ in range(n):
        t = d[r]; a = d[r + 1]; r += 2 + (32 if a == 3 else 0); off = struct.unpack_from("<Q", d, r)[0]; r += 8
        if t == 0:  # Royalties { basis_points, creators: Vec<{address, percentage}>, rule_set }
            q = off + 1 + 2; k = struct.unpack_from("<I", d, q)[0]; q += 4
            out += [b58e(d[q + 33 * i:q + 33 * i + 32]) for i in range(k)]
    return out

SCEN = []
def scenario(name, note, keys, facts, absent=()):
    SCEN.append((name, note, list(dict.fromkeys(keys)), facts, list(absent)))

def tm_item(mint, ta=None):
    rs, creators = tm_extras(mint)
    k = [mint, metadata(mint), edition(mint)] + ([rs] if rs else []) + creators
    if ta: k += [ta, record(mint, ta)]
    return k, rs, creators

def tensor_listing(name, note, list_state, collection, standard):
    d = one(list_state); owner = b58e(d[10:42]); asset = b58e(d[42:74])
    o = 82; o += 33 if d[o] else 1; o += 8; o += 33 if d[o] else 1
    mb = b58e(d[o + 1:o + 33]) if d[o] else None; o += 33 if d[o] else 1
    rent_payer = b58e(d[o:o + 32])
    fee_vault = pda([b"fee_vault", bytes([b58d(list_state)[31]])], TFEE)
    keys = [list_state, owner, rent_payer, fee_vault] + ([mb] if mb else [])
    facts = {"venue": "tensor_listing", "list_state": list_state, "asset": asset, "owner": owner, "maker_broker": mb,
             "rent_payer": rent_payer, "fee_vault": fee_vault, "collection": collection, "standard": standard}
    if standard == "core":
        coll, creators = core_extras(asset); keys += [asset, coll] + creators; facts["creators"] = creators
    else:
        lta = ata(list_state, asset); k, rs, creators = tm_item(asset, lta)
        keys += k; facts.update(list_ta=lta, rule_set=rs, creators=creators)
    scenario(name, note, keys, facts)

def mmm_listing(name, note, sell_state, collection, standard):
    d = one(sell_state); pool = b58e(d[8:40]); owner = b58e(d[40:72]); mint = b58e(d[72:104])
    p = one(pool); referral = b58e(p[37:69])
    escrow = pda([b"mmm_buyside_sol_escrow_account", b58d(pool)], "mmm3XBJg5gk8XJxEKBvdgptZz6SgK4tXvn36sodowMc")
    pta = ata(pool, mint); k, rs, creators = tm_item(mint, pta)
    scenario(name, note, [sell_state, pool, owner, referral, escrow] + k,
             {"venue": "mmm", "sell_state": sell_state, "pool": pool, "owner": owner, "asset": mint, "referral": referral,
              "buyside_escrow": escrow, "pool_ta": pta, "rule_set": rs, "creators": creators, "collection": collection, "standard": standard})

def me_get(path):
    req = urllib.request.Request("https://api-mainnet.magiceden.dev/v2/" + path, headers={"User-Agent": "Mozilla/5.0"})
    for i in range(6):
        try:
            time.sleep(0.7); return json.loads(urllib.request.urlopen(req, timeout=60).read())
        except Exception as e:
            print("retry ME", path, e, file=sys.stderr); time.sleep(5)
    raise SystemExit("ME API failed")

def floor_scenario(symbol, collection, n=10):
    """The cheapest `n` listings Magic Eden aggregates (M2, mmm, Tensor), trimmed to what the canister uses, and every account
    the canister reads to verify them, at one slot; plus Magic Eden's public stats floor (the reference)."""
    ls = me_get(f"collections/{symbol}/listings?offset=0&limit={n}&listingAggMode=true")
    stats = me_get(f"collections/{symbol}/stats")
    keys, listings = [], []
    for x in ls:
        m, src, pda_ = x["tokenMint"], x["listingSource"], x["pdaAddress"]
        listings.append({k: x.get(k) for k in ("listingSource", "pdaAddress", "tokenMint", "seller", "price", "tokenAddress")})
        keys += [m, metadata(m), edition(m), pda_]
        if src == "M2": keys.append(x["tokenAddress"])
        elif src == "TENSOR_MARKETPLACE_LISTING": keys.append(ata(pda_, m))
        elif src == "MMM":
            d = one(pda_); pool = b58e(d[8:40])
            keys += [pool, pda([b"mmm_buyside_sol_escrow_account", b58d(pool)], "mmm3XBJg5gk8XJxEKBvdgptZz6SgK4tXvn36sodowMc"), ata(pool, m)]
    scenario(f"floor_{symbol}", f"The {n} cheapest {symbol} listings Magic Eden aggregated, every account behind them, and its stats floor",
             keys, {"collection": collection, "standard": "pnft", "me_listings": listings, "me_stats": stats})

def main():
    os.makedirs(OUT, exist_ok=True)
    tensor_listing("tensor_pnft_mad_lads", "Mad Lads #? (pNFT, Metaplex Foundation rule set) listed on Tensor Marketplace (TComp), no cosigner",
                   "Ahq21nyFAmQrcSTAFRQJWHdDMemRn9Zu3yPvHQFL1WSU", "J1S9H3QjnRtBbbuD4HjPV6RpRhwuk4zKbxsnCHuTgh9w", "pnft")
    tensor_listing("tensor_legacy_fff", "Famous Fox Federation (legacy NonFungible, no token standard) listed on Tensor Marketplace",
                   "ZVPrHSyjLxUiwz25zW2kazKm5aFuz2stjsLv84oNcn6", "BUjZjAS2vbbb65g7Z1Ca9ZRVYoJscURG5L3AkVvHP9ac", "legacy")
    tensor_listing("tensor_core_otc", "OTC Desks (Metaplex Core, collection with a Royalties plugin only) listed on Tensor Marketplace",
                   "cbzWgVQWQjSa3fvFfxhwX1YAXzC1RKym4P4GT8n7xHH", "D7sLW9uKZG3G7bNbWfMHvKSgVhU9nXdv7huTfepF5Jrh", "core")
    mmm_listing("mmm_legacy_tff", "Transdimensional Fox Federation (legacy) in a Magic Eden mmm pool (FVCA allowlist, cosigner = ME's notary)",
                "88onf1xXA6FGEt3uSAAYGFvK356YqQ7pBaoaVTYPN4zf", "HbPtffcEzzSzZ1VJaTjaAJqTUQpfNeMkPqNG81dWE2Bi", "legacy")
    mmm_listing("mmm_pnft_smb_gen3", "SMB Gen3 (pNFT, Compatibility rule set) in a Magic Eden mmm pool (MCC allowlist, cosigner = ME's notary)",
                "EE8MVrWfXSNYJonoJJgSeqxp8tqzbbxWUH1LUEWrN2p5", "8Rt3Ayqth4DAiPnW9MDFi63TiQJHmohfTWLMQFHi4KZH", "pnft")
    # M2-listed items in their sellers' wallets: membership yes, custody of the seller but with a sale delegate
    k, rs, cr = tm_item("4vCV2b3uSsLdgFBLC33reou8wv8JtfbJohqGmxJZs4e9", "4Bvh8T9YCcrwHnXDuQXcu2TkKdFfLtZPe4GwEFbuQ6Z7")
    scenario("m2_pnft_mad_lads_wallet", "Mad Lads #9655 in its seller's wallet, listed on Magic Eden M2 (token record delegate set)",
             k + ["3jkbiou4E8DDRXxA2ZyfNm3ZdRiEtNUGczoX9iKoCYu8", "E8cU1WiRWjanGxmn96ewBgk9vPTcL6AEZ1t6F6fkgUWe"],
             {"venue": "m2", "asset": "4vCV2b3uSsLdgFBLC33reou8wv8JtfbJohqGmxJZs4e9", "token_account": "4Bvh8T9YCcrwHnXDuQXcu2TkKdFfLtZPe4GwEFbuQ6Z7",
              "seller": "EVquz5tTB2wPZDPACSXb69ytsZY26VWZ12kg4Vq1uert", "seller_trade_state": "3jkbiou4E8DDRXxA2ZyfNm3ZdRiEtNUGczoX9iKoCYu8",
              "collection": "J1S9H3QjnRtBbbuD4HjPV6RpRhwuk4zKbxsnCHuTgh9w", "standard": "pnft", "rule_set": rs})
    # Core refusals and a fungible, a cNFT (no account)
    scenario("core_refusals", "Core assets the canister must refuse: Retardio Cousins Classic (collection has PermanentFreezeDelegate), "
             "Critters #600 (asset TransferDelegate not held by its owner); a fungible (USDC) and a compressed NFT (no account)",
             ["48gxVViqxZnY8C2sH1vnp2pByACeW3P3rfQnRgEKxsKR", "4mX8wTrn4xk77rmYHh4WZ7cnkXgCm8MMhKcYWzS7wENP",
              "3L1AgNJit7R4Xecc2vhvA7snaiQ6DnEde2yNjyqb8xQ8", "CRittErsK51xrVktQeMeAW2cK7JNqv6Ko9Zrw26eF2cT",
              "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v", metadata("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v")],
             {"retardio_asset": "48gxVViqxZnY8C2sH1vnp2pByACeW3P3rfQnRgEKxsKR", "retardio_collection": "4mX8wTrn4xk77rmYHh4WZ7cnkXgCm8MMhKcYWzS7wENP",
              "critters_asset": "3L1AgNJit7R4Xecc2vhvA7snaiQ6DnEde2yNjyqb8xQ8", "critters_collection": "CRittErsK51xrVktQeMeAW2cK7JNqv6Ko9Zrw26eF2cT",
              "usdc": "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v", "cnft_tensorians": "GDEC7TRDAdU1BUhSSAK2WLc5DqLSCCcq6W2b7rVpfHcS"},
             absent=["GDEC7TRDAdU1BUhSSAK2WLc5DqLSCCcq6W2b7rVpfHcS"])
    floor_scenario("mad_lads", "J1S9H3QjnRtBbbuD4HjPV6RpRhwuk4zKbxsnCHuTgh9w")
    only = set(sys.argv[2:])
    for name, note, keys, facts, absent in SCEN:
        if only and name not in only: continue
        slot, got = fetch(keys + absent)
        t = rpc("getBlockTime", [slot]) if True else None
        accts, missing = [], []
        for k in keys + absent:
            v = got[k]
            if v is None: missing.append(k); continue
            accts.append({"pubkey": k, "lamports": v["lamports"], "owner": v["owner"], "executable": v["executable"], "data": v["data"][0]})
        for k in absent:
            if k not in missing: raise SystemExit(f"{name}: {k} exists")
        with open(os.path.join(OUT, name + ".json"), "w") as f:
            json.dump({"scenario": name, "note": note, "slot": slot, "unix_time": t, "facts": facts, "accounts": accts, "absent": missing},
                      f, indent=1)
        print(name, slot, len(accts), "accounts,", len(missing), "absent:", missing)

if __name__ == "__main__":
    main()
