#!/usr/bin/env python3
"""Check deployments/launch/solana-collections.json (python3 stdlib only).

  python3 scripts/check-solana-collections.py --offline   # the file alone: shape, keys, symbols, the threshold rule
  python3 scripts/check-solana-collections.py             # + mainnet: collection metadata, sample members, standards, rule sets
  python3 scripts/check-solana-collections.py --market    # + Magic Eden floor now vs floor_sol (drift > 20 % is reported)

Online reads use the public RPC (https://api.mainnet-beta.solana.com, or $SOLANA_RPC_URL): getMultipleAccounts only.
Exit status 1 on any failure; market drift is a warning (the tier and T_wX are fixed by the day-1 config).
"""
import argparse, base64, hashlib, json, math, os, struct, sys, time, urllib.request
from decimal import Decimal

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DEFAULT_FILE = os.path.join(ROOT, "deployments", "launch", "solana-collections.json")
RPC = os.environ.get("SOLANA_RPC_URL", "https://api.mainnet-beta.solana.com")
ME = "https://api-mainnet.magiceden.dev/v2/"
UA = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0 Safari/537.36"

TOKEN_METADATA = "metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s"
TOKEN_PROGRAM = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"
TOKEN_2022 = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb"
MPL_CORE = "CoREENxT6tW1HoK8ypY1SxRMZTcVPm7R94rH4PZNhX7d"
TOKEN_AUTH_RULES = "auth9SigNpDKz4sJJ1DfCTuZrZNSAgh9sFD3rboVmgg"
STANDARDS = ("legacy", "pnft", "core")
TIERS_SOL = (85, 170, 255, 510)

# ---------------------------------------------------------------- base58, PDAs
B58 = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"


def b58decode(s):
    n = 0
    for c in s:
        i = B58.find(c)
        if i < 0:
            raise ValueError("not base58: %r" % s)
        n = n * 58 + i
    body = n.to_bytes((n.bit_length() + 7) // 8, "big") if n else b""
    return b"\0" * (len(s) - len(s.lstrip("1"))) + body


def b58encode(b):
    n = int.from_bytes(b, "big")
    s = ""
    while n:
        n, r = divmod(n, 58)
        s = B58[r] + s
    return "1" * (len(b) - len(b.lstrip(b"\0"))) + s


def is_pubkey(s):
    try:
        return isinstance(s, str) and len(b58decode(s)) == 32 and b58encode(b58decode(s)) == s
    except ValueError:
        return False


_P = 2 ** 255 - 19
_D = -121665 * pow(121666, _P - 2, _P) % _P


def _on_curve(b):
    y = int.from_bytes(b, "little") & ((1 << 255) - 1)
    if y >= _P:
        return False
    u = (y * y - 1) % _P
    v = (_D * y * y + 1) % _P
    x2 = u * pow(v, _P - 2, _P) % _P
    return x2 == 0 or pow(x2, (_P - 1) // 2, _P) == 1


def find_pda(seeds, program):
    prog = b58decode(program)
    for bump in range(255, -1, -1):
        h = hashlib.sha256(b"".join(seeds) + bytes([bump]) + prog + b"ProgramDerivedAddress").digest()
        if not _on_curve(h):
            return b58encode(h)
    raise ValueError("no PDA")


def metadata_pda(mint):
    return find_pda([b"metadata", b58decode(TOKEN_METADATA), b58decode(mint)], TOKEN_METADATA)


def edition_pda(mint):
    return find_pda([b"metadata", b58decode(TOKEN_METADATA), b58decode(mint), b"edition"], TOKEN_METADATA)


# ---------------------------------------------------------------- RPC
def _post(url, body):
    req = urllib.request.Request(url, data=json.dumps(body).encode(), headers={"content-type": "application/json", "User-Agent": UA})
    return json.loads(urllib.request.urlopen(req, timeout=30).read())


def get_accounts(keys):
    """{key: (owner, bytes) or None}, via getMultipleAccounts (100 per call), with retries."""
    out = {}
    for i in range(0, len(keys), 100):
        chunk = keys[i:i + 100]
        last = None
        for attempt in range(6):
            try:
                r = _post(RPC, {"jsonrpc": "2.0", "id": 1, "method": "getMultipleAccounts", "params": [chunk, {"encoding": "base64"}]})
                if "error" in r:
                    raise RuntimeError(r["error"])
                for k, v in zip(chunk, r["result"]["value"]):
                    out[k] = (v["owner"], base64.b64decode(v["data"][0])) if v else None
                last = None
                break
            except Exception as e:  # noqa: BLE001 (public RPC: 429s and resets)
                last = e
                time.sleep(2 + 3 * attempt)
        if last is not None:
            raise RuntimeError("RPC getMultipleAccounts failed: %s" % last)
        time.sleep(0.3)
    return out


# ---------------------------------------------------------------- decoders
TOKEN_STANDARDS = ["NonFungible", "FungibleAsset", "Fungible", "NonFungibleEdition", "ProgrammableNonFungible", "ProgrammableNonFungibleEdition"]


def _str(b, o):
    n = struct.unpack_from("<I", b, o)[0]
    return b[o + 4:o + 4 + n].rstrip(b"\0").decode(errors="replace"), o + 4 + n


def parse_metadata(b):
    """Token Metadata `Metadata` account (key 4)."""
    if not b or b[0] != 4:
        raise ValueError("not a Metadata account")
    o = 1
    ua = b58encode(b[o:o + 32]); o += 32
    mint = b58encode(b[o:o + 32]); o += 32
    name, o = _str(b, o)
    symbol, o = _str(b, o)
    uri, o = _str(b, o)
    sfbp = struct.unpack_from("<H", b, o)[0]; o += 2
    creators = []
    if b[o]:
        n = struct.unpack_from("<I", b, o + 1)[0]; o += 5
        for _ in range(n):
            creators.append({"address": b58encode(b[o:o + 32]), "verified": bool(b[o + 32]), "share": b[o + 33]}); o += 34
    else:
        o += 1
    o += 2  # primary_sale_happened, is_mutable
    o += 2 if b[o] else 1  # edition_nonce
    ts = None
    if b[o]:
        ts = TOKEN_STANDARDS[b[o + 1]]; o += 2
    else:
        o += 1
    col = None
    if b[o]:
        col = {"verified": bool(b[o + 1]), "key": b58encode(b[o + 2:o + 34])}; o += 34
    else:
        o += 1
    o += (1 + 1 + 8 + 8) if b[o] else 1  # uses
    size = None
    if o < len(b) and b[o]:  # collection_details
        if b[o + 1] == 0:
            size = struct.unpack_from("<Q", b, o + 2)[0]
        o += 1 + 1 + 8
    else:
        o += 1
    pc, rule_set = False, None
    if o < len(b) and b[o]:  # programmable_config: Some(V1 { rule_set: Option<Pubkey> })
        pc = True
        if b[o + 2]:
            rule_set = b58encode(b[o + 3:o + 35])
    return {"update_authority": ua, "mint": mint, "name": name, "symbol": symbol, "uri": uri, "seller_fee_bps": sfbp,
            "creators": creators, "token_standard": ts, "collection": col, "collection_size": size,
            "programmable_config": pc, "rule_set": rule_set}


def parse_mint(b):
    """SPL Token mint (first 82 bytes; Token-2022 has the same base)."""
    return {"supply": struct.unpack_from("<Q", b, 36)[0], "decimals": b[44], "initialized": bool(b[45])}


def parse_core(b):
    """mpl-core AssetV1 (key 1) / CollectionV1 (key 5): only what membership needs."""
    if b[0] == 1:
        o = 33
        t = b[o]; o += 1
        ua = (["None", "Address", "Collection"][t], b58encode(b[o:o + 32]) if t in (1, 2) else None)
        name, _ = _str(b, o + (32 if t in (1, 2) else 0))
        return {"kind": "asset", "update_authority": ua, "name": name}
    if b[0] == 5:
        ua = b58encode(b[1:33])
        name, o = _str(b, 33)
        uri, o = _str(b, o)
        num_minted, current_size = struct.unpack_from("<II", b, o)
        return {"kind": "collection", "update_authority": ua, "name": name, "current_size": current_size, "num_minted": num_minted}
    raise ValueError("not an mpl-core asset/collection")


def standard_of(ts, rule_set_present=None):
    """Token Metadata token standard -> this file's enum."""
    if ts in ("ProgrammableNonFungible", "ProgrammableNonFungibleEdition"):
        return "pnft"
    if ts in (None, "NonFungible"):
        return "legacy"
    return "other:%s" % ts


# MessagePack, enough for Token Auth Rules rule sets (RuleSetV1 is serialized with rmp-serde after a 9-byte header).
def _mp(b, o):
    t = b[o]
    if t <= 0x7f: return t, o + 1
    if 0x80 <= t <= 0x8f: return _mp_map(b, o + 1, t & 15)
    if 0x90 <= t <= 0x9f: return _mp_arr(b, o + 1, t & 15)
    if 0xa0 <= t <= 0xbf: n = t & 31; return b[o + 1:o + 1 + n].decode(), o + 1 + n
    if t == 0xc0: return None, o + 1
    if t in (0xc2, 0xc3): return t == 0xc3, o + 1
    if t == 0xc4: n = b[o + 1]; return bytes(b[o + 2:o + 2 + n]), o + 2 + n
    if t == 0xcc: return b[o + 1], o + 2
    if t == 0xcd: return struct.unpack_from(">H", b, o + 1)[0], o + 3
    if t == 0xce: return struct.unpack_from(">I", b, o + 1)[0], o + 5
    if t == 0xcf: return struct.unpack_from(">Q", b, o + 1)[0], o + 9
    if t == 0xd9: n = b[o + 1]; return b[o + 2:o + 2 + n].decode(), o + 2 + n
    if t == 0xda: n = struct.unpack_from(">H", b, o + 1)[0]; return b[o + 3:o + 3 + n].decode(), o + 3 + n
    if t == 0xdc: return _mp_arr(b, o + 3, struct.unpack_from(">H", b, o + 1)[0])
    if t == 0xde: return _mp_map(b, o + 3, struct.unpack_from(">H", b, o + 1)[0])
    if t >= 0xe0: return t - 256, o + 1
    raise ValueError("msgpack type 0x%02x" % t)


def _mp_arr(b, o, n):
    r = []
    for _ in range(n):
        v, o = _mp(b, o); r.append(v)
    if n == 32 and all(isinstance(x, int) and 0 <= x < 256 for x in r):
        r = b58encode(bytes(r))  # a Pubkey
    return r, o


def _mp_map(b, o, n):
    r = {}
    for _ in range(n):
        k, o = _mp(b, o); v, o = _mp(b, o); r[k] = v
    return r, o


def parse_rule_set(b):
    """Token Auth Rules rule set account -> {"revisions": n, "name", "owner", "latest": {operation: rule}}."""
    loc = struct.unpack_from("<Q", b, 1)[0]  # header: key u8, rev_map_location u64
    n = struct.unpack_from("<I", b, loc + 1)[0]  # rev map: version u8, Vec<u64>
    offs = [struct.unpack_from("<Q", b, loc + 5 + 8 * i)[0] for i in range(n)]
    last = _mp(b, offs[-1] + 1)[0]  # each revision: version u8, then RuleSetV1 as an array or a map (both are on mainnet)
    if isinstance(last, dict):
        last = [last.get("libVersion"), last.get("owner"), last.get("ruleSetName"), last.get("operations")]
    return {"revisions": n, "owner": last[1], "name": last[2], "latest": last[3]}


def latest_is_pass(rs):
    return all(v == "Pass" for v in rs["latest"].values())


# ---------------------------------------------------------------- the rule
def tier_for(floor_sol):
    f = Decimal(str(floor_sol))
    for t in TIERS_SOL:
        if Decimal(t) * Decimal("0.5") * Decimal("0.75") / Decimal("1.03") >= Decimal("1.33") * f:
            return t
    return None


def threshold_wx(tier, floor_sol):
    return max(3, math.ceil(Decimal(tier) / Decimal(str(floor_sol))))


# ---------------------------------------------------------------- checks
class Report:
    def __init__(self):
        self.fail = 0
        self.warn = 0

    def bad(self, where, msg):
        self.fail += 1
        print("FAIL %s: %s" % (where, msg))

    def warning(self, where, msg):
        self.warn += 1
        print("WARN %s: %s" % (where, msg))

    def ok(self, where, msg):
        print("ok   %s: %s" % (where, msg))


def check_offline(doc, rep):
    cols = doc.get("collections")
    if not isinstance(cols, list):
        rep.bad("file", "collections is not an array"); return
    if len(cols) != 12:
        rep.bad("file", "%d collections, expected 12" % len(cols))
    for k in ("_", "rule", "taken_at"):
        if not doc.get(k):
            rep.bad("file", "missing %r" % k)
    seen_sym, seen_key, seen_coll = set(), set(), set()
    pilot = os.path.join(ROOT, "deployments", "launch", "nft-pilot.json")  # the Ethereum and Ordinals listings' symbols
    taken = set()
    if os.path.exists(pilot):
        with open(pilot) as f:
            taken = {p.get("symbol", "").upper() for p in json.load(f).get("collections", [])}
    for i, c in enumerate(cols):
        w = c.get("key") or "#%d" % i
        for f in ("key", "name", "symbol", "collection_key", "standard", "update_authority", "seller_fee_bps", "supply",
                  "magic_eden_symbol", "tensor_slug", "x", "website", "floor_sol", "sales_30d", "volume_30d_sol", "listed",
                  "tier_sol", "threshold_wx", "compressed", "sample_members", "rule_sets"):
            if f not in c:
                rep.bad(w, "missing field %r" % f)
        if c.get("key") in seen_key: rep.bad(w, "duplicate key")
        seen_key.add(c.get("key"))
        s = c.get("symbol", "")
        if not (isinstance(s, str) and s.startswith("w") and 2 <= len(s) <= 10 and s[1:].isalnum() and s[1:] == s[1:].upper()):
            rep.bad(w, "symbol %r: want 'w' + uppercase alphanumerics, <= 10 chars" % s)
        if s.upper() in seen_sym: rep.bad(w, "duplicate symbol %r" % s)
        if s.upper() in taken: rep.bad(w, "symbol %r is already used in nft-pilot.json" % s)
        seen_sym.add(s.upper())
        if not is_pubkey(c.get("collection_key")):
            rep.bad(w, "collection_key is not a base58 32-byte key")
        if c.get("collection_key") in seen_coll: rep.bad(w, "duplicate collection_key")
        seen_coll.add(c.get("collection_key"))
        if c.get("update_authority") is not None and not is_pubkey(c.get("update_authority")):
            rep.bad(w, "update_authority is not a base58 32-byte key")
        if c.get("standard") not in STANDARDS:
            rep.bad(w, "standard %r not in %s" % (c.get("standard"), STANDARDS))
        for r in c.get("rule_sets") or []:
            if not is_pubkey(r.get("address")): rep.bad(w, "rule set address %r" % r.get("address"))
        if c.get("standard") != "pnft" and c.get("rule_sets"):
            rep.bad(w, "rule_sets on a non-pNFT collection")
        if c.get("compressed") is not False:
            rep.bad(w, "compressed must be false (cNFT collections are not listed)")
        sm = c.get("sample_members") or []
        if not 2 <= len(sm) <= 3:
            rep.bad(w, "%d sample_members, want 2-3" % len(sm))
        for m in sm:
            if not is_pubkey(m.get("mint")): rep.bad(w, "sample mint %r" % m.get("mint"))
            if m.get("standard") not in STANDARDS: rep.bad(w, "sample %s standard %r" % (m.get("mint"), m.get("standard")))
            if m.get("rule_set") is not None and not is_pubkey(m.get("rule_set")): rep.bad(w, "sample %s rule_set" % m.get("mint"))
        try:
            floor = float(c.get("floor_sol"))
            assert floor > 0
        except Exception:  # noqa: BLE001
            rep.bad(w, "floor_sol %r" % c.get("floor_sol")); continue
        t = tier_for(c["floor_sol"])
        if t is None:
            rep.bad(w, "floor %.4g SOL is above what the largest tier covers" % floor)
        elif c.get("tier_sol") != t:
            rep.bad(w, "tier_sol %r, the rule gives %d" % (c.get("tier_sol"), t))
        elif c.get("threshold_wx") != threshold_wx(t, c["floor_sol"]):
            rep.bad(w, "threshold_wx %r, the rule gives %d" % (c.get("threshold_wx"), threshold_wx(t, c["floor_sol"])))
        else:
            rep.ok(w, "%s %s floor %s SOL -> tier %d, T_wX %d" % (s, c.get("standard"), c["floor_sol"], t, c["threshold_wx"]))


def check_online(doc, rep):
    cols = doc["collections"]
    keys = set()
    for c in cols:
        keys.add(metadata_pda(c["collection_key"])); keys.add(c["collection_key"])
        for m in c["sample_members"]:
            keys.update([m["mint"], metadata_pda(m["mint"]), edition_pda(m["mint"])])
        for r in c.get("rule_sets") or []:
            keys.add(r["address"])
    acc = get_accounts(sorted(keys))
    rs_cache = {}
    for c in cols:
        w = c["key"]
        ck = c["collection_key"]
        if c["standard"] == "core":
            a = acc.get(ck)
            if not a or a[0] != MPL_CORE:
                rep.bad(w, "Core collection %s missing or not owned by mpl-core" % ck); continue
            info = parse_core(a[1])
            rep.ok(w, "Core collection %r, %d members" % (info["name"], info["current_size"]))
        else:
            a = acc.get(metadata_pda(ck))
            if not a or a[0] != TOKEN_METADATA:
                rep.bad(w, "collection mint %s has no Token Metadata account" % ck); continue
            md = parse_metadata(a[1])
            if md["mint"] != ck:
                rep.bad(w, "collection metadata mint mismatch")
            rep.ok(w, "collection NFT %r (update authority %s%s)" % (md["name"], md["update_authority"],
                   ", sized %d" % md["collection_size"] if md["collection_size"] is not None else ""))
        for m in c["sample_members"]:
            mw = "%s %s" % (w, m["mint"][:8])
            if c["standard"] == "core" or m["standard"] == "core":
                a = acc.get(m["mint"])
                if not a or a[0] != MPL_CORE:
                    rep.bad(mw, "not an mpl-core asset"); continue
                info = parse_core(a[1])
                if info["update_authority"] != ("Collection", ck):
                    rep.bad(mw, "update authority %s, want Collection(%s)" % (info["update_authority"], ck)); continue
                rep.ok(mw, "Core asset %r in the collection" % info["name"]); continue
            ma = acc.get(m["mint"])
            if not ma:
                rep.bad(mw, "no mint account (compressed?)"); continue
            if ma[0] not in (TOKEN_PROGRAM, TOKEN_2022):
                rep.bad(mw, "mint owned by %s" % ma[0]); continue
            mi = parse_mint(ma[1])
            if mi["supply"] != 1 or mi["decimals"] != 0:
                rep.bad(mw, "mint supply %d decimals %d, want 1/0" % (mi["supply"], mi["decimals"]))
            ed = acc.get(edition_pda(m["mint"]))
            if not ed or ed[0] != TOKEN_METADATA or ed[1][0] != 6:  # Key::MasterEditionV2
                rep.bad(mw, "no master edition")
            a = acc.get(metadata_pda(m["mint"]))
            if not a:
                rep.bad(mw, "no metadata"); continue
            md = parse_metadata(a[1])
            if md["collection"] != {"verified": True, "key": ck}:
                rep.bad(mw, "collection %s, want verified %s" % (md["collection"], ck)); continue
            std = standard_of(md["token_standard"])
            if std != m["standard"]:
                rep.bad(mw, "standard %s on chain, file says %s" % (std, m["standard"]))
            if std != c["standard"] and not c.get("mixed"):
                rep.bad(mw, "member is %s, collection is %s and not marked mixed" % (std, c["standard"]))
            if md["rule_set"] != m.get("rule_set"):
                rep.bad(mw, "rule set %s on chain, file says %s" % (md["rule_set"], m.get("rule_set")))
            declared = {r["address"] for r in c.get("rule_sets") or []}
            if md["rule_set"] and md["rule_set"] not in declared:
                rep.bad(mw, "rule set %s not among the collection's rule_sets" % md["rule_set"])
            if md["rule_set"] is None and std == "pnft" and not c.get("mixed"):
                rep.bad(mw, "pNFT without a rule set, collection not marked mixed")
            rep.ok(mw, "%r verified member, %s, rule set %s" % (md["name"], std, md["rule_set"]))
        for r in c.get("rule_sets") or []:
            ad = r["address"]
            if ad not in rs_cache:
                a = acc.get(ad)
                if not a or a[0] != TOKEN_AUTH_RULES:
                    rs_cache[ad] = None
                else:
                    rs_cache[ad] = parse_rule_set(a[1])
            rs = rs_cache[ad]
            if rs is None:
                rep.bad(w, "rule set %s missing or not owned by Token Auth Rules" % ad); continue
            latest = "pass" if latest_is_pass(rs) else "restrictive"
            if rs["name"] != r.get("name") or rs["revisions"] != r.get("revisions") or latest != r.get("latest_revision"):
                rep.bad(w, "rule set %s on chain: %r, %d revisions, latest %s; file: %r, %r, %r"
                        % (ad, rs["name"], rs["revisions"], latest, r.get("name"), r.get("revisions"), r.get("latest_revision")))
            elif latest != "pass":
                rep.warning(w, "rule set %s (%s): latest revision is restrictive" % (ad, rs["name"]))
            else:
                rep.ok(w, "rule set %s (%r, %d revisions): latest revision Pass" % (ad[:8], rs["name"], rs["revisions"]))


def check_market(doc, rep):
    for c in doc["collections"]:
        w = c["key"]
        st = None
        for attempt in range(5):
            try:
                req = urllib.request.Request(ME + "collections/%s/stats" % c["magic_eden_symbol"], headers={"User-Agent": UA})
                st = json.loads(urllib.request.urlopen(req, timeout=25).read())
                break
            except Exception:  # noqa: BLE001
                time.sleep(3 + 3 * attempt)
        time.sleep(0.6)
        if not st or not st.get("floorPrice"):
            rep.warning(w, "Magic Eden stats unavailable"); continue
        now = st["floorPrice"] / 1e9
        drift = now / float(c["floor_sol"]) - 1
        msg = "ME floor now %.4g SOL vs %s in the file (%+.1f %%); rule now: tier %s, T_wX %s" % (
            now, c["floor_sol"], 100 * drift, tier_for(now), threshold_wx(tier_for(now), now) if tier_for(now) else "-")
        (rep.warning if abs(drift) > 0.20 else rep.ok)(w, msg)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--offline", action="store_true", help="validate the file only, no network")
    ap.add_argument("--market", action="store_true", help="also re-read Magic Eden floors and report drift > 20 %%")
    ap.add_argument("--file", default=DEFAULT_FILE)
    a = ap.parse_args()
    with open(a.file) as f:
        doc = json.load(f)
    rep = Report()
    check_offline(doc, rep)
    if not a.offline and rep.fail == 0:
        check_online(doc, rep)
        if a.market:
            check_market(doc, rep)
    print("\n%s: %d failure(s), %d warning(s)" % ("FAILED" if rep.fail else "PASSED", rep.fail, rep.warn))
    return 1 if rep.fail else 0


if __name__ == "__main__":
    sys.exit(main())
