#!/usr/bin/env python3
"""Record Magic Eden M2 purchase scenarios (docs/v9/spikes/s5-magic-eden-m2.md): real mainnet accounts, read-only, at `finalized`.

    python3 -I record_m2.py RPC_URL <me_symbol> <collection_mint> <n>
"""
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
M2 = "M2mx93ekt1fmXSVkTrUL9xVFHkmME8HTUi5Cyc5aF7K"
AH = "E8cU1WiRWjanGxmn96ewBgk9vPTcL6AEZ1t6F6fkgUWe"
SYSTEM = "11111111111111111111111111111111"

def me_get(path):
    req = urllib.request.Request("https://api-mainnet.magiceden.dev/v2/" + path, headers={"User-Agent": "Mozilla/5.0"})
    for i in range(6):
        try:
            time.sleep(0.7); return json.loads(urllib.request.urlopen(req, timeout=60).read())
        except Exception as e:
            print("retry ME", path, e, file=sys.stderr); time.sleep(5)
    raise SystemExit("ME API failed")

def m2_scenarios(symbol, collection, n):
    """The `n` cheapest Magic Eden M2 listings of a collection: every account a keyless M2 purchase reads (docs/v9/spikes/s5-magic-eden-m2.md)."""
    out = []
    for off in range(0, 100, 20):
        for x in me_get(f"collections/{symbol}/listings?offset={off}&limit=20&listingAggMode=true"):
            if x["listingSource"] == "M2" and x.get("auctionHouse") == AH: out.append(x)
        if len(out) >= n: break
    ah = one(AH)
    authority, treasury, notary = b58e(ah[72:104]), b58e(ah[8:40]), b58e(ah[136:168])
    for x in out[:n]:
        mint, st, ta, seller = x["tokenMint"], x["pdaAddress"], x["tokenAddress"], x["seller"]
        k, rs, creators = tm_item(mint, ta)
        keys = k + [st, AH, authority, treasury, notary, seller] + creators
        scenario(f"m2_{symbol}_{mint[:6]}", f"{symbol} listed on Magic Eden M2 at {x['price']} SOL: the listing, the item, the auction house",
                 keys, {"venue": "m2", "asset": mint, "token_account": ta, "seller": seller, "seller_trade_state": st, "collection": collection,
                        "rule_set": rs, "creators": creators, "price": x["price"], "symbol": symbol, "authority": authority, "treasury": treasury,
                        "notary": notary})

def tm_item(mint, ta=None):
    rs, creators = tm_extras(mint)
    k = [mint, metadata(mint), edition(mint)] + ([rs] if rs else []) + creators
    if ta: k += [ta, record(mint, ta)]
    return k, rs, creators

def scenario(name, note, keys, facts, absent=()):
    SCEN.append((name, note, list(dict.fromkeys(keys)), facts, list(absent)))

def main():
    os.makedirs(OUT, exist_ok=True)
    args = [a for a in sys.argv[2:]]
    sym, coll, n = args[0], args[1], int(args[2])
    m2_scenarios(sym, coll, n)
    for name, note, keys, facts, absent in SCEN:
        slot, got = fetch(keys)
        t = rpc("getBlockTime", [slot])
        accts, missing = [], []
        for k in keys:
            v = got[k]
            if v is None: missing.append(k); continue
            accts.append({"pubkey": k, "lamports": v["lamports"], "owner": v["owner"], "executable": v["executable"], "data": v["data"][0]})
        with open(os.path.join(OUT, name + ".json"), "w") as f:
            json.dump({"scenario": name, "note": note, "slot": slot, "unix_time": t, "facts": facts, "accounts": accts, "absent": missing}, f, indent=1)
        print(name, slot, len(accts), "accounts,", len(missing), "absent:", missing)

if __name__ == "__main__":
    main()
