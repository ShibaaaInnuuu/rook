"""D8's price references, per collection (docs/v7/phase1-nft.md §6 D8; deployments/launch/nft-pilot.json `priceRefs`).

The monitor's emergency poster pause reads two public references for each Ethereum collection, OpenSea and CoinGecko, and pauses
only when the canister's reading is more than 20 % from BOTH. The operator decided on 2026-10-09 that for a collection CoinGecko
does not list (Identity MD), OpenSea ALONE is acceptable: the launch file says so on that collection with "priceRefs": ["opensea"]
and a "_priceRefs" note recording the decision, and names no coingeckoId; its monitor entry says price_refs ["opensea"] and names no
coingecko_id. Every other collection keeps both references.

script/golive-check.sh imports this (as it does depth_probe.py) for --launch (launch_problems) and --monitor-config (monitor_refs);
scripts/audit2_golive_check.test.sh drives both directly. Pure: no I/O but the CLI's file reads.
"""
import json
import sys

ALONE = ["opensea"]
DECIDED = "2026-10-09"   # the operator's decision, as the `_priceRefs` note must record it


def _alone(v):
    return isinstance(v, list) and v == ALONE


def launch_problems(L):
    """(problems, alone slugs): each launch collection names both references, or priceRefs ["opensea"] with its note and no coingeckoId."""
    bad, alone = [], []
    for x in L.get("collections") or []:
        slug = x.get("slug", "?")
        if not str(x.get("openseaSlug") or ""):
            bad.append(f"{slug}: no openseaSlug (D8's OpenSea reference)")
        if "priceRefs" in x:
            if not _alone(x["priceRefs"]):
                bad.append(f"{slug}: priceRefs {json.dumps(x['priceRefs'])} is not [\"opensea\"] (the only per-collection exception D8 allows)")
                continue
            note = str(x.get("_priceRefs") or "")
            if "operator" not in note or DECIDED not in note:
                bad.append(f"{slug}: priceRefs [\"opensea\"] without its _priceRefs note recording the operator's decision of {DECIDED}")
            if x.get("coingeckoId"):
                bad.append(f"{slug}: priceRefs [\"opensea\"] reads OpenSea alone, yet the file names coingeckoId {x['coingeckoId']}: one or the other")
            alone.append(slug)
        elif not str(x.get("coingeckoId") or ""):
            bad.append(f"{slug}: no coingeckoId: D8 needs both references (OpenSea and CoinGecko) unless the launch file decides priceRefs [\"opensea\"] for it")
    return bad, alone


def monitor_refs(x, want=None):
    """'both', 'opensea' or 'missing' for a monitor collection entry `x`, against its launch collection `want` (None: no --launch)."""
    os_, cg = str(x.get("opensea_slug") or ""), str(x.get("coingecko_id") or "")
    if "price_refs" in x:
        # only where the launch file decides it: without --launch there is no decision to hold it to
        if want is None or not _alone(x["price_refs"]) or not _alone(want.get("priceRefs")):
            return "missing"
        if cg or not os_ or "<" in os_ or os_ != str(want.get("openseaSlug") or "") or want.get("coingeckoId"):
            return "missing"
        return "opensea"
    if not (os_ and cg) or "<" in os_ or "<" in cg:
        return "missing"
    if want is not None:
        if "priceRefs" in want:
            return "missing"   # the launch file decided OpenSea alone: the monitor must say so (price_refs), not invent a CoinGecko id
        if [os_, cg] != [str(want.get("openseaSlug") or ""), str(want.get("coingeckoId") or "")]:
            return "missing"
    return "both"


def launch_of(L, collection):
    c = str(collection).lower()
    return next((y for y in (L or {}).get("collections", []) if str(y.get("collection", "")).lower() == c), None)


if __name__ == "__main__":
    # d8refs.py launch <launch file>                     -> problems, one per line, then "alone: <slugs>"
    # d8refs.py monitor <monitor file> <collection> [<launch file>] -> both | opensea | missing | absent
    if sys.argv[1] == "launch":
        b, a = launch_problems(json.load(open(sys.argv[2])))
        for p in b: print(f"FAIL {p}")
        print("alone: " + ",".join(a))
        sys.exit(1 if b else 0)
    m = json.load(open(sys.argv[2])); c = sys.argv[3].lower()
    L = json.load(open(sys.argv[4])) if len(sys.argv) > 4 and sys.argv[4] else None
    x = next((x for x in (m.get("eth_nft") or {}).get("collections", []) if str(x.get("collection", "")).lower() == c), None)
    if x is None: print("absent"); sys.exit(0)
    want = launch_of(L, c) if L is not None else None
    if L is not None and want is None: print("missing"); sys.exit(0)
    print(monitor_refs(x, want))
