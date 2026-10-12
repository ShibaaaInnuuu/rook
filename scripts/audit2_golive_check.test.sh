#!/usr/bin/env bash
# R2 (docs/audit/2026-09-26/review2/R2-services.md G-N1..G-N3, C-08), fixed in round 3 (review2/fixes-R3-services.md §7):
# script/golive-check.sh --phase1 against a scripted chain. A fake `cast` is planted in a throwaway $HOME/.foundry/bin (golive-check
# puts that directory first on PATH), answering from a table; nothing reaches a network. The R2 PoC's cases 1-4 passed the go/no-go
# read; each now FAILS with its reason. A correct stack (a canonical Safe proxy, complete stack file) passes, and each new read FAILS
# alone when it is wrong.
#   usage: scripts/audit2_golive_check.test.sh      (exit 0 = every case as expected)
set -uo pipefail
cd "$(dirname "$0")/.."
TMP=$(mktemp -d); trap 'rm -rf "$TMP"' EXIT
mkdir -p "$TMP/home/.foundry/bin"
cat > "$TMP/home/.foundry/bin/cast" <<'PY'
#!/usr/bin/env python3
import json, os, sys
db = json.load(open(os.environ["FAKE_CAST_DB"]))
a = sys.argv[1:]
def opt(xs):
    out = []; i = 0
    while i < len(xs):
        if xs[i] == "--rpc-url": i += 2; continue
        out.append(xs[i]); i += 1
    return out
cmd, rest = a[0], opt(a[1:])
if cmd == "chain-id": print(db["chainId"]); sys.exit(0)
if cmd == "code": print(db["code"].get(rest[0].lower(), "0x")); sys.exit(0)
if cmd == "keccak":
    h = db["keccak"].get(rest[0].lower())
    if h is None: print("0x" + "ee" * 32)
    else: print(h)
    sys.exit(0)
if cmd == "storage": print(db["storage"].get(rest[0].lower() + " " + rest[1].lower(), "0x" + "00" * 32)); sys.exit(0)
if cmd == "logs":   # R3-check L-H: the Safe's SafeSetup event, as `cast logs --json` prints it
    addr = rest[rest.index("--address") + 1].lower()
    if addr in db.get("logs", {}): print(db["logs"][addr]); sys.exit(0)
    print("Error: no logs answer", file=sys.stderr); sys.exit(1)
if cmd == "receipt":   # R4-check L-S1: the SafeSetup's transaction, as `cast receipt --json` prints it
    tx = rest[0].lower()
    if tx in db.get("receipts", {}): print(json.dumps(db["receipts"][tx])); sys.exit(0)
    print("Error: transaction not found", file=sys.stderr); sys.exit(1)
if cmd == "call":
    key = (rest[0] + " " + rest[1].split("(")[0] + ("" if len(rest) < 3 else " " + " ".join(rest[2:]))).lower()
    if key in db["calls"]: print(db["calls"][key]); sys.exit(0)
    print("Error: execution reverted", file=sys.stderr); sys.exit(1)
print("fake cast: unknown " + " ".join(a), file=sys.stderr); sys.exit(1)
PY
chmod +x "$TMP/home/.foundry/bin/cast"
# a fake dfx (golive-check takes it from $DFX): the canister's public queries, answered from $FAKE_DFX_DB as `dfx --output json` prints
cat > "$TMP/dfx" <<'PY2'
#!/usr/bin/env python3
import json, os, sys
db = json.load(open(os.environ["FAKE_DFX_DB"]))
m = sys.argv[-1]
if m not in db: print("fake dfx: no " + m, file=sys.stderr); sys.exit(1)
print(json.dumps(db[m]))
PY2
chmod +x "$TMP/dfx"
CAN=rrkah-fqaaa-aaaaa-aaaaq-cai
dfxdb() { # dfxdb <largest fee entry sats> [quarantined entry sats]: custody_caps at the stage's numbers, custody_fee_pool as given
  python3 - "$TMP/dfx.json" "$1" "${2:-}" <<PY2
import json, sys
e = lambda v: {"outpoint": {"txid": "ab" * 32, "vout": 0}, "value": f"{int(v):_}", "source": "operator", "fails": []}
q = [e(sys.argv[3])] if sys.argv[3] else []
caps = {"global_max_items": "3", "global_max_value_wei": "10_000_000_000_000_000_000",
        "collections": [{"collection": "btc:$W", "max_items": "3", "max_value_wei": "10_000_000_000_000_000_000", "max_item_wei": "1"}]}
json.dump({"custody_caps": [caps, []], "custody_fee_pool": {"entries": [e(5000), e(sys.argv[2])], "quarantined": [q] if q else []}}, open(sys.argv[1], "w"))
PY2
}

h() { printf '0x%040x' "$1"; }
R=$(h 0xa1); L=$(h 0xa2); D=$(h 0xa3); B=$(h 0xa4); F=$(h 0xa5); I=$(h 0xa6); E=$(h 0xa7); SAFE=$(h 0x5af)
K=$(h 0xb1); P=$(h 0xb2); A=$(h 0xb3); T=$(h 0xb4); G=$(h 0xc1); O1=$(h 0xd1); O2=$(h 0xd2)
PT=$(h 0xe1); DIR=$(h 0xe2); PM=0x8366a39cc670b4001a1121b8f6a443a643e40951; OPS=$(h 0xe4); CUST=$(h 0xe5)
W=$(h 0xf1); M=$(h 0xf2); LK=$(h 0xf3); FA=$(h 0xf4); RT=$(h 0xf5); DK=$(h 0xf6); RV=$(h 0xf7); BV=$(h 0xf8); YV=$(h 0xf9); OB=$(h 0xfa); WC=$(h 0xfb)
SINGLETON=0x41675c099f32341bf84bfc5382af534df5c7461a
Z=0x0000000000000000000000000000000000000000
CEIL=1000000000000000000
cat > "$TMP/4663.json" <<EOF
{"chainId": 4663, "registry": "$R", "launcher": "$L", "distributor": "$D", "beacon": "$B", "ponsFactory": "$F",
 "routerImplementation": "$I", "owner": "$SAFE", "treasury": "$T", "wired": true, "keeper": "$K", "purchaser": "$P", "attestor": "$A"}
EOF
stack() { # stack [python edits of s]: a complete stack file for one inscriptions collection
  python3 - "$TMP/stack-4663.json" "${1:-}" <<PY
import json, sys
s = {"chainId": 4663, "label": "stack", "deployedAtBlock": 1, "stage": 0, "registry": "$R", "owner": "$SAFE", "keeper": "$K", "custodian": "$CUST",
     "payee": "$CUST", "treasury": "$PT", "directory": "$DIR", "poolManager": "$PM", "ops": "$OPS", "guardian": "$G", "capCeiling": $CEIL, "ownerStepsMade": False,
     "caps": {"stage": 0, "mintsPerDayPerCollection": 3, "itemsPerDayPerCollection": 3, "custodyItemsPerCollection": 3, "custodyWeiPerCollection": 10**19},
     "collections": [{"slug": "frogs", "name": "Wrapped Frog", "symbol": "wFROG", "kind": "inscriptions", "listingId": 0, "itemUnits": 10**18, "listingPaused": False, "wrapClaim": "$WC", "wrapper": "$W",
       "factory": "$FA", "lock": "$LK", "router": "$RT", "queue": "$M", "floorModule": "$M", "mintDesk": "$DK", "raffleVault": "$RV", "burnVault": "$BV",
       "yoyoVault": "$YV", "offerBoard": "$OB", "custody": {"kind": "bitcoin", "address": "bc1pcustody"}}]}
exec(sys.argv[2])
json.dump(s, open(sys.argv[1], "w"))
PY
}
db() { # db <safe owners> [python edits of db]: a correct 4663 registry, a canonical Safe 1.4.1 proxy, and the stack's contracts
  python3 - "$TMP/db.json" "$1" "${2:-}" <<PY
import json, sys
R,L,D,B,F,I,E,SAFE,K,P,A,T,G,Z = "$R $L $D $B $F $I $E $SAFE $K $P $A $T $G $Z".split()
PT,DIR,PM,OPS,CUST,W,M,LK,FA,RT,DK,RV,BV,YV,OB,WC,SING,O1,O2 = "$PT $DIR $PM $OPS $CUST $W $M $LK $FA $RT $DK $RV $BV $YV $OB $WC $SINGLETON $O1 $O2".split()
U = 10**18
calls = {
 f"{R} owner": SAFE, f"{R} treasury": T, f"{R} launcher": L, f"{R} distributor": D, f"{R} namedKeeper": K, f"{R} keeper": K,
 f"{R} purchaser": P, f"{R} attestor": A, f"{R} keeperPaused": "false", f"{R} pendingOwner": Z,
 f"{R} pendingRoles": f"{Z}\n{Z}\n{Z}\n0", f"{R} pendingTreasury": Z, f"{R} pendingBondAt": "0", f"{R} guardian": G,
 f"{R} count": "0",
 f"{SAFE} getThreshold": "2", f"{SAFE} getOwners": "[" + sys.argv[2] + "]", f"{SAFE} getModulesPaginated 0x0000000000000000000000000000000000000001 10": "[]\n0x0000000000000000000000000000000000000001",
 f"{L} factory": F, f"{L} registry": R, f"{F} feeEscrow": E, f"{L} escrow": E, f"{L} routerImplementation": I,
 f"{I} launcher": L, f"{I} coin": "0x0000000000000000000000000000000000000001", f"{D} registry": R, f"{D} beacon": B,
 # the stack
 f"{PT} reservesBps": "9000", f"{PT} floatsBps": "0", f"{PT} bootstrapBps": "0", f"{PT} opsBps": "1000", f"{PT} bootstrapLoan": "0",
 f"{PT} registry": R, f"{DIR} admin": SAFE, f"{PT} ops": OPS,
 f"{WC} wrapper": W, f"{W} distribution": WC, f"{W} deployer": WC, f"{WC} registry": R, f"{WC} listingId": "0", f"{W} registry": R,
 f"{W} distributor": D, f"{W} beacon": B, f"{W} listingId": "0", f"{W} custodyAddress": '"bc1pcustody"',
 f"{R} listing 0": '(0, false, false, 0, 0x0000000000000000000000000000000000000000, 0x00, "ref", "x", "bc1pcustody")',
 f"{W} custodian": CUST, f"{W} floorModule": M, f"{M} wrapped": W, f"{M} registry": R, f"{W} pendingMintLimit": "0", f"{W} maxMintsPerDay": "3",
 f"{W} symbol": '"wFROG"', f"{DK} wrapper": W, f"{M} wrapper": W, f"{M} keeper": K, f"{M} platform": PT, f"{M} treasury": PT, f"{M} factory": FA,
 f"{M} seeder": LK, f"{M} maxItemsPerDay": "3", f"{M} payee": CUST, f"{DK} payee": CUST,
 f"{M} mintPerHour": str(5*U), f"{M} burnPerHour": str(5*U), f"{M} maxPublicEthBacked": str(10*U), f"{M} maxEthBacked": str(40*U),
 f"{R} isPaused 0": "false", f"{W} ONE_ITEM": str(U),
 f"{LK} wrapped": W, f"{LK} poolManager": PM, f"{LK} factory": FA, f"{LK} treasury": PT, f"{FA} wrapped": W, f"{FA} seeder": LK, f"{FA} queue": M,
 f"{FA} platform": PT, f"{FA} raffleVault": RV, f"{FA} burnVault": BV, f"{FA} yoyoVault": YV, f"{RT} lock": LK, f"{RT} module": M, f"{RT} poolManager": PM,
 f"{OB} wrapper": W, f"{OB} custodian": CUST,
 f"{PT} isCollection {M}": "true", f"{DIR} indexOf {W}": "1",
 f"{R} pendingRoot 0": "0x" + "00" * 32 + "\n0", f"{R} pendingBtcVault 0": '""\n0',
 f"{FA} coinCount": "0", f"{FA} launchesOpen": "true",
}
for k, v in enumerate([RV, BV, YV]):
    calls.update({f"{v} policy": str(k), f"{v} registry": R, f"{v} listingId": "0", f"{v} wrapper": W, f"{v} venue": RT, f"{v} launcher": FA,
                  f"{v} desk": DK, f"{v} capCeiling": "$CEIL", f"{v} pendingCeiling": "0"})
PROXY = "0x6080aa"   # stands for the SafeProxy runtime; its hash is the canonical 1.4.1 proxy's
code = {x.lower(): "0x6080" for x in (R, L, D, B, F, I, PT, DIR, PM, W, M, LK, FA, RT, DK, RV, BV, YV, OB, WC)}
code[SAFE.lower()] = PROXY; code[SING] = "0x6080bb"
db = {"chainId": 4663, "calls": {k.lower(): v for k, v in calls.items()}, "code": code,
      "keccak": {PROXY: "0xd7d408ebcd99b2b70be43e20253d6d92a8ea8fab29bd3be7f55b10032331fb4c", "0x6080bb": "0x1fe2df852ba3299d6534ef416eefa406e56ced995bca886ab7a553e6d0c5e1c4"},
      "storage": {SAFE.lower() + " 0": "0x" + "00" * 12 + SING[2:]}}
CHAIN_OWNERS = tuple(x.strip() for x in sys.argv[2].split(",") if x.strip())
TX, TX2 = "0x" + "7a" * 32, "0x" + "7b" * 32
F141, F130 = "0x4e1dcf7ad4e460cfd30791ccc4f9c8a4f820ec67", "0xa6b71e26c5e0845f74c812102ca7114b6a896ab2"
w = lambda x: (x[2:] if isinstance(x, str) else "%x" % x).rjust(64, "0")
def setup_log(initializer=Z, handler=Z, owners=None, threshold=2, tx=TX):   # SafeSetup(initiator, owners[], threshold, initializer, fallbackHandler)
    owners = CHAIN_OWNERS if owners is None else owners
    data = "0x" + w(0x80) + w(threshold) + w(initializer) + w(handler) + w(len(owners)) + "".join(w(o) for o in owners)
    l = {"address": SAFE.lower(), "topics": ["0x141df868a6331af528e38c83b7aa03edc19be66e37ae67f9285bf4f8e3c6a1a8", "0x" + w(F141)], "data": data}
    if tx: l["transactionHash"] = tx
    return l
def safe_setup(**k): return json.dumps([setup_log(**k)])
PC = "0x4f51faf6c4561ff95f067657e43439f0f856d97c04d9ec9070a6199ad418e235"
def proxy_creation(factory=F141, proxy=None, singleton=None, indexed=True):   # ProxyCreation(proxy, singleton): proxy indexed in 1.4.1
    proxy, singleton = proxy or SAFE, singleton or SING
    if indexed: return {"address": factory, "topics": [PC, "0x" + w(proxy)], "data": "0x" + w(singleton)}
    return {"address": factory, "topics": [PC], "data": "0x" + w(proxy) + w(singleton)}
def receipt(*logs, status="0x1"): return {"status": status, "logs": list(logs)}
db["logs"] = {SAFE.lower(): safe_setup()}
# R4-check L-S1: the canonical 1.4.1 factory created the Safe with setup() as its initializer: one transaction holds both
db["receipts"] = {TX: receipt(setup_log(), proxy_creation())}
db["code"][F141] = "0x6080f1"; db["keccak"]["0x6080f1"] = "0x50c3cdc4074750a7a974204a716c999edd37482f907608d960b2b025ee0b3317"
db["code"][F130] = "0x6080f0"; db["keccak"]["0x6080f0"] = "0x337d7f54be11b6ed55fef7b667ea5488db53db8320a05d1146aa4bd169a39a9b"
exec(sys.argv[3])
json.dump(db, open(sys.argv[1], "w"))
PY
}
# R3-check L-H: --safe-owners is required under --phase1, so every case names the ceremony's devices unless it names its own, or NOSO=1
run() { local so=(--safe-owners "$O1,$O2"); case " $* " in *" --safe-owners "*) so=();; esac; [ "${NOSO:-}" = 1 ] && so=()
  HOME="$TMP/home" FAKE_CAST_DB="$TMP/db.json" DFX="$TMP/dfx" FAKE_DFX_DB="$TMP/dfx.json" script/golive-check.sh http://fake "$TMP/4663.json" "$@" ${so[@]+"${so[@]}"} 2>&1; }
fail=0
check() { # check <name> <expected exit> <grep pattern that must appear> -- <golive-check args…>
  local name="$1" want="$2" pat="$3"; shift 4; local out code
  out=$(run "$@"); code=$?
  if [ "$code" = "$want" ] && echo "$out" | grep -q -- "$pat"; then echo "ok   $name (exit $code)"; else echo "UNEXPECTED $name: exit $code (wanted $want, /$pat/)"; echo "$out" | grep -E "^(FAIL|WARN)" | head -8; echo "$out" | tail -3; fail=1; fi
}
OWNERS="$O1, $O2"
stack

# 1. G-N1: the go/no-go read on 4663 with NO stack file now FAILS (it passed in R2)
db "$OWNERS"
check "G-N1: --phase1 on 4663 without the stack file FAILS" 1 "FAIL --phase1 without the stack file" -- --before-reinstall

# 2. G-N2: a real Safe whose two owners are the keeper and the guardian FAILS (it passed as "a Safe, 2 of 2")
db "$K, $G"
check "G-N2: the Safe's owners = {keeper, guardian} FAILS" 1 "role keeper $K is also an owner of the Safe" -- "$TMP/stack-4663.json" --before-reinstall
check "G-N2: ... and the guardian is named too" 1 "guardian $G is also an owner of the Safe" -- "$TMP/stack-4663.json" --before-reinstall

# 3. G-N2: any contract answering getThreshold()=2 and a two-owner getOwners() is not a Safe
db "$OWNERS" 'db["code"][SAFE.lower()] = "0x6080"'
check "G-N2: a duck-typed contract as the 2-of-2 Safe FAILS" 1 "is not a canonical Safe proxy" -- "$TMP/stack-4663.json" --before-reinstall

# 4. G-N3: registry.count() unreadable FAILS (listings were silently skipped)
db "$OWNERS" 'del db["calls"][("%s count" % R).lower()]'
check "G-N3: registry.count() unreadable FAILS" 1 "FAIL registry.count() is unreadable" -- "$TMP/stack-4663.json" --before-reinstall

# 5, 6. Controls, as in R2
db "$OWNERS" 'db["calls"][("%s keeperPaused" % R).lower()] = "true"'
check "control: keeperPaused FAILs" 1 "FAIL registry.keeperPaused()" -- "$TMP/stack-4663.json" --before-reinstall
db "$OWNERS" 'del db["calls"][("%s keeperPaused" % R).lower()]'
check "control: keeperPaused unreadable FAILs" 1 "FAIL registry.keeperPaused()" -- "$TMP/stack-4663.json" --before-reinstall

# 7. The correct stack passes: a canonical Safe proxy on the canonical singleton, no module, no guard, code-less owners holding no role
db "$OWNERS"
check "a correct stack and a real Safe pass the go/no-go read" 0 "ALL CHECKS PASS (--phase1: the go/no-go read)" -- "$TMP/stack-4663.json" --before-reinstall
check "... it reads the Safe's proxy, singleton, modules and guard" 0 "the Safe $SAFE has no guard" -- "$TMP/stack-4663.json" --before-reinstall
check "... and the payees, ETH caps and rates, and the listing's pause (C-08)" 0 "frogs module.maxPublicEthBacked = 10 units" -- "$TMP/stack-4663.json" --before-reinstall
check "... --safe-owners naming the ceremony's devices" 0 "the Safe's owners are the ceremony's devices" -- "$TMP/stack-4663.json" --before-reinstall --safe-owners "$O1,$O2"
check "... --safe-owners naming other devices FAILS" 1 "FAIL the Safe's owners are" -- "$TMP/stack-4663.json" --before-reinstall --safe-owners "$O1,$(h 0xd3)"

# R3-check L-H: --safe-owners required; the fallback handler and the Safe's SafeSetup event read
NOSO=1 check "L-H: --phase1 without --safe-owners FAILS" 1 "FAIL --safe-owners is required under --phase1" -- "$TMP/stack-4663.json" --before-reinstall
check "L-H: ... it reads the fallback handler" 0 "fallback handler is none or a canonical CompatibilityFallbackHandler" -- "$TMP/stack-4663.json" --before-reinstall
check "L-H: ... and the Safe's setup() (SafeSetup, initializer zero)" 0 "setup() made no delegatecall" -- "$TMP/stack-4663.json" --before-reinstall
db "$OWNERS" 'db["storage"][SAFE.lower() + " 0x6c9a6c4a39284e37ed1cf53d337577d14212a4870fb976a4366c693b939918d5"] = "0x" + "00" * 12 + "ee" * 20'
check "L-H: a non-canonical fallback handler FAILS" 1 "not a canonical CompatibilityFallbackHandler" -- "$TMP/stack-4663.json" --before-reinstall
db "$OWNERS" 'db["storage"][SAFE.lower() + " 0x6c9a6c4a39284e37ed1cf53d337577d14212a4870fb976a4366c693b939918d5"] = "0x" + "00" * 12 + "fd0732dc9e303f09fcef3a7388ad10a83459ec99"'
check "L-H: the canonical 1.4.1 handler passes" 0 "(0xfd0732dc9e303f09fcef3a7388ad10a83459ec99)" -- "$TMP/stack-4663.json" --before-reinstall
db "$OWNERS" 'db["logs"][SAFE.lower()] = safe_setup(initializer="0x" + "de" * 20)'
check "L-H: a setup() that delegatecalled anything (hidden owners or modules) FAILS" 1 "setup() delegatecalled 0xdededededededededededededededededededede" -- "$TMP/stack-4663.json" --before-reinstall
db "$OWNERS" 'db["logs"][SAFE.lower()] = safe_setup(initializer="0xbd89a1ce4dde368ffab0ec35506eece0b1ffdc54"); db["code"]["0xbd89a1ce4dde368ffab0ec35506eece0b1ffdc54"] = "0x6080cc"; db["keccak"]["0x6080cc"] = "0x2f25df28caf984366ee584e13241707e85dcd5a6ea0c14267928dafc1fd6274b"'
check "L-H: the canonical SafeToL2Setup (Safe{Wallet} on an L2) passes" 0 "delegatecalled only the canonical SafeToL2Setup" -- "$TMP/stack-4663.json" --before-reinstall
db "$OWNERS" 'db["logs"][SAFE.lower()] = safe_setup(initializer="0xbd89a1ce4dde368ffab0ec35506eece0b1ffdc54"); db["code"]["0xbd89a1ce4dde368ffab0ec35506eece0b1ffdc54"] = "0x6080dd"'
check "L-H: ... but not other code at its address" 1 "is not the canonical SafeToL2Setup" -- "$TMP/stack-4663.json" --before-reinstall
db "$OWNERS" 'db["logs"][SAFE.lower()] = "[]"'
check "L-H: no SafeSetup event FAILS" 1 "no SafeSetup event of the Safe" -- "$TMP/stack-4663.json" --before-reinstall
db "$OWNERS" 'del db["logs"][SAFE.lower()]'
check "L-H: SafeSetup unreadable FAILS" 1 "SafeSetup event could not be read" -- "$TMP/stack-4663.json" --before-reinstall
db "$OWNERS"

# R4-check L-S1 (fixes-golive-lows.md §3): the Safe's creation is read. Review4's fork PoC (Audit4SafeHidden) planted a module with
# creation code that returned the canonical proxy runtime, then called setup() on its own: every read above passed. Its shape is a
# SafeSetup whose transaction holds no ProxyCreation from a canonical factory; each variant FAILs, and the correct Safe passes.
check "L-S1: the correct Safe: created by the canonical 1.4.1 factory with setup() in the same transaction" 0 "created by the canonical SafeProxyFactory 0x4e1dcf7ad4e460cfd30791ccc4f9c8a4f820ec67 with setup() in the same transaction" -- "$TMP/stack-4663.json" --before-reinstall
check "L-S1: ... its SafeSetup owners and threshold are compared with the chain's" 0 "SafeSetup owners and threshold (2) are the chain's getOwners() / getThreshold()" -- "$TMP/stack-4663.json" --before-reinstall
check "L-S1: ... and with --safe-owners" 0 "SafeSetup owners are the ceremony's devices (--safe-owners)" -- "$TMP/stack-4663.json" --before-reinstall
check "L-S1: ... all of it passes the go/no-go read" 0 "ALL CHECKS PASS (--phase1: the go/no-go read)" -- "$TMP/stack-4663.json" --before-reinstall
db "$OWNERS" 'db["receipts"][TX] = receipt(setup_log())'
check "L-S1: the planted Safe (SafeSetup in a transaction with no ProxyCreation, review4's PoC) FAILS" 1 "holds no ProxyCreation naming it" -- "$TMP/stack-4663.json" --before-reinstall
db "$OWNERS" 'db["receipts"][TX] = receipt(proxy_creation(proxy="0x" + "ab" * 20), setup_log())'
check "L-S1: a decoy ProxyCreation of another proxy in the same transaction FAILS" 1 "holds no ProxyCreation naming it" -- "$TMP/stack-4663.json" --before-reinstall
db "$OWNERS" 'db["receipts"][TX] = receipt(setup_log()); db["receipts"][TX2] = receipt(proxy_creation())'
check "L-S1: a ProxyCreation in ANOTHER transaction than the SafeSetup FAILS" 1 "holds no ProxyCreation naming it" -- "$TMP/stack-4663.json" --before-reinstall
db "$OWNERS" 'db["receipts"][TX] = receipt(setup_log(), proxy_creation(factory="0x" + "fa" * 20))'
check "L-S1: a ProxyCreation from a factory that is not canonical FAILS" 1 "not a canonical SafeProxyFactory" -- "$TMP/stack-4663.json" --before-reinstall
db "$OWNERS" 'db["code"][F141] = "0x6080f9"'
check "L-S1: the canonical factory address with other code on this chain FAILS" 1 "does not carry the canonical SafeProxyFactory code" -- "$TMP/stack-4663.json" --before-reinstall
db "$OWNERS" 'db["receipts"][TX] = receipt(setup_log(), proxy_creation(singleton="0x" + "5e" * 20))'
check "L-S1: created on a singleton that is not canonical FAILS" 1 "created on the singleton 0x5e5e" -- "$TMP/stack-4663.json" --before-reinstall
db "$OWNERS" 'db["receipts"][TX] = receipt(setup_log(), proxy_creation(), proxy_creation())'
check "L-S1: two ProxyCreation events naming the Safe FAIL" 1 "holds 2 ProxyCreation events naming it" -- "$TMP/stack-4663.json" --before-reinstall
db "$OWNERS" 'db["receipts"][TX] = receipt(setup_log(), proxy_creation(factory=F130, singleton="0x3e5c63644e683549055b9be8653de26e0b4cd36e", indexed=False))'
check "L-S1: a Safe made by the canonical 1.3.0 factory (proxy not indexed) passes that read" 0 "created by the canonical SafeProxyFactory 0xa6b71e26c5e0845f74c812102ca7114b6a896ab2" -- "$TMP/stack-4663.json" --before-reinstall
db "$OWNERS" 'db["receipts"][TX] = receipt(setup_log(), proxy_creation(), status="0x0")'
check "L-S1: a failed creation transaction FAILS" 1 "did not succeed according to its receipt" -- "$TMP/stack-4663.json" --before-reinstall
db "$OWNERS" 'del db["receipts"][TX]'
check "L-S1: the creation receipt unreadable FAILS" 1 "could not be read (unreadable)" -- "$TMP/stack-4663.json" --before-reinstall
db "$OWNERS" 'db["logs"][SAFE.lower()] = safe_setup(tx=None)'
check "L-S1: a SafeSetup with no transaction hash FAILS" 1 "has no transaction hash" -- "$TMP/stack-4663.json" --before-reinstall
db "$OWNERS" 'db["logs"][SAFE.lower()] = safe_setup(owners=(O1, "0x" + "d3" * 20)); db["receipts"][TX] = receipt(setup_log(owners=(O1, "0x" + "d3" * 20)), proxy_creation())'
check "L-S1: SafeSetup owners unlike the chain's getOwners() FAIL (changed since creation)" 1 "changed since its creation" -- "$TMP/stack-4663.json" --before-reinstall
db "$OWNERS" 'db["logs"][SAFE.lower()] = safe_setup(threshold=1)'
check "L-S1: a SafeSetup threshold unlike the chain's FAILS" 1 "changed since its creation" -- "$TMP/stack-4663.json" --before-reinstall
db "$OWNERS"
check "L-S1: --safe-owners other than the SafeSetup's FAILS" 1 "SafeSetup owners .* are not the ceremony's --safe-owners" -- "$TMP/stack-4663.json" --before-reinstall --safe-owners "$O1,$(h 0xd3)"

# 8. G-N1: --canister is required under --phase1 unless --before-reinstall
check "G-N1: --phase1 without --canister (and without --before-reinstall) FAILS" 1 "FAIL --phase1 without --canister" -- "$TMP/stack-4663.json"

# 9. Each new read, wrong alone
db "$OWNERS" 'db["storage"][SAFE.lower() + " 0"] = "0x" + "00" * 12 + "ab" * 20'
check "G-N2: a Safe proxy on a non-canonical singleton FAILS" 1 "not a canonical Safe 1.3.0 / 1.4.1 singleton" -- "$TMP/stack-4663.json" --before-reinstall
db "$OWNERS" 'db["calls"][("%s getModulesPaginated 0x0000000000000000000000000000000000000001 10" % SAFE).lower()] = "[0x00000000000000000000000000000000000000dd]\n0x0000000000000000000000000000000000000001"'
check "G-N2: a Safe with a module enabled FAILS" 1 "has modules enabled" -- "$TMP/stack-4663.json" --before-reinstall
db "$OWNERS" 'db["storage"][SAFE.lower() + " 0x4a204f620c8c5ccdca3fd54d003badd85ba500436a431f0cbda4f558c93c34c8"] = "0x" + "00" * 12 + "cd" * 20'
check "G-N2: a Safe with a guard FAILS" 1 "has a transaction guard set" -- "$TMP/stack-4663.json" --before-reinstall
db "$OWNERS" 'db["code"][O2.lower()] = "0xef0100" + "11" * 20'
check "G-N2: a Safe owner with code (7702) FAILS" 1 "Safe owner $O2 has code" -- "$TMP/stack-4663.json" --before-reinstall
db "$OWNERS" 'db["calls"][("%s payee" % M).lower()] = "0x00000000000000000000000000000000000000ee"'
check "C-08: module.payee() not the stack's payee FAILS" 1 "FAIL frogs module.payee = the stack's payee" -- "$TMP/stack-4663.json" --before-reinstall
db "$OWNERS" 'db["calls"][("%s payee" % DK).lower()] = "0x00000000000000000000000000000000000000ee"'
check "C-08: desk.payee() not the stack's payee FAILS" 1 "FAIL frogs desk.payee = the stack's payee" -- "$TMP/stack-4663.json" --before-reinstall
db "$OWNERS" 'db["calls"][("%s maxPublicEthBacked" % M).lower()] = str(100 * U)'
check "C-08: maxPublicEthBacked scaled up FAILS" 1 "module.maxPublicEthBacked is" -- "$TMP/stack-4663.json" --before-reinstall
db "$OWNERS" 'del db["calls"][("%s mintPerHour" % M).lower()]'
check "C-08: mintPerHour unreadable FAILS" 1 "module.mintPerHour() is unreadable" -- "$TMP/stack-4663.json" --before-reinstall
db "$OWNERS" 'db["calls"][("%s isPaused 0" % R).lower()] = "true"'
check "C-08: the listing paused FAILS" 1 "listing 0 is PAUSED" -- "$TMP/stack-4663.json" --before-reinstall
db "$OWNERS" 'db["calls"][("%s isPaused 0" % R).lower()] = "true"'; stack 's["collections"][0]["listingPaused"] = True'
check "R3: a listing paused as the stack file says passes that read" 0 "listing 0 paused = true (the stack file's listingPaused)" -- "$TMP/stack-4663.json" --before-reinstall
db "$OWNERS"
check "R3: the stack file's listingPaused true, the chain's false, FAILS" 1 "listing 0 is not paused (isPaused false; the stack file's listingPaused is true)" -- "$TMP/stack-4663.json" --before-reinstall
stack
# Integration R3: the stack file's payee, itemUnits and listingPaused are read, not assumed
db "$OWNERS"; stack 'del s["payee"]'
check "R3: a stack file without payee FAILS under --phase1" 1 "FAIL the stack file names no payee" -- "$TMP/stack-4663.json" --before-reinstall
PAYEE2=$(h 0xe6)
stack 's["payee"] = "'"$PAYEE2"'"'
check "R3: the stack's payee is not the module's payee FAILS" 1 "FAIL frogs module.payee = the stack's payee: expected $PAYEE2" -- "$TMP/stack-4663.json" --before-reinstall
db "$OWNERS" 'db["calls"][("%s payee" % M).lower()] = "'"$PAYEE2"'"; db["calls"][("%s payee" % DK).lower()] = "'"$PAYEE2"'"'
check "R3: a payee other than the custodian, as the stack names it, passes" 0 "ALL CHECKS PASS" -- "$TMP/stack-4663.json" --before-reinstall
db "$OWNERS"; stack 's["collections"][0]["itemUnits"] = 10**17'
check "R3: itemUnits not the wrapper's ONE_ITEM() FAILS" 1 "FAIL frogs wrapper.ONE_ITEM() = the stack's itemUnits" -- "$TMP/stack-4663.json" --before-reinstall
stack 'del s["collections"][0]["itemUnits"]'
check "R3: a stack file without itemUnits FAILS under --phase1" 1 "FAIL frogs: the stack file names no itemUnits" -- "$TMP/stack-4663.json" --before-reinstall
stack 'del s["collections"][0]["listingPaused"]'
check "R3: a stack file without listingPaused FAILS under --phase1" 1 "FAIL frogs: the stack file names no listingPaused" -- "$TMP/stack-4663.json" --before-reinstall
stack
# Integration R3, operator 2026-09-26: the custody fee pool holds one usable entry >= --min-fee-pool-sats (default 20,000 sats; read
# with --canister). The conflict-out's 100,000-sat ceiling is unchanged: an entry under it passes with a note
db "$OWNERS"; dfxdb 150000
check "R3: --canister with a 150,000-sat fee input passes" 0 "ok   custody fee pool: one entry of 150000 sats (>= the minimum 20000" -- "$TMP/stack-4663.json" --canister "$CAN"
out=$(run "$TMP/stack-4663.json" --canister "$CAN"); echo "$out" | grep -q "^note custody fee pool" && { echo "UNEXPECTED a 150,000-sat entry noted as under the ceiling"; fail=1; } || echo "ok   ... with no note (it reaches the 100,000-sat ceiling)"
dfxdb 20000
check "operator: a 20,000-sat fee input (the default minimum) passes" 0 "ok   custody fee pool: one entry of 20000 sats (>= the minimum 20000" -- "$TMP/stack-4663.json" --canister "$CAN"
check "... with the note that a conflict-out cannot reach its ceiling in a spike" 0 "note custody fee pool: the largest entry (20000 sats) is under the conflict-out's 100000-sat ceiling: in a large fee spike a stuck delivery is cancelled more slowly" -- "$TMP/stack-4663.json" --canister "$CAN"
dfxdb 19999
check "operator: a fee pool whose largest entry is 19,999 sats FAILS" 1 "FAIL custody fee pool: no usable entry of at least 20000 sats (largest 19999" -- "$TMP/stack-4663.json" --canister "$CAN"
dfxdb 15000 250000
check "R3: a quarantined 250,000-sat entry does not count" 1 "FAIL custody fee pool: no usable entry of at least 20000 sats (largest 15000" -- "$TMP/stack-4663.json" --canister "$CAN"
check "... and is warned about" 1 "WARN 1 custody fee input(s) quarantined" -- "$TMP/stack-4663.json" --canister "$CAN"
dfxdb 99999
check "--min-fee-pool-sats 100000: a 99,999-sat entry FAILS (the old rule, still available)" 1 "FAIL custody fee pool: no usable entry of at least 100000 sats (largest 99999" -- "$TMP/stack-4663.json" --canister "$CAN" --min-fee-pool-sats 100000
dfxdb 30000
check "--min-fee-pool-sats 25000: a 30,000-sat entry passes" 0 "ok   custody fee pool: one entry of 30000 sats (>= the minimum 25000" -- "$TMP/stack-4663.json" --canister "$CAN" --min-fee-pool-sats 25000
for bad in 0 -5 abc 20k ""; do
  check "--min-fee-pool-sats '$bad' is refused" 1 "refused: --min-fee-pool-sats takes a whole number of sats" -- "$TMP/stack-4663.json" --canister "$CAN" --min-fee-pool-sats "$bad"
done
dfxdb 150000; python3 -c "import json,sys; d=json.load(open(sys.argv[1])); del d['custody_fee_pool']; json.dump(d, open(sys.argv[1],'w'))" "$TMP/dfx.json"
check "R3: custody_fee_pool() unreadable FAILS" 1 "FAIL custody_fee_pool() of $CAN" -- "$TMP/stack-4663.json" --canister "$CAN"
db "$OWNERS" 'del db["calls"][("%s indexOf %s" % (DIR, W)).lower()]'
check "G-N3: indexOf unreadable is not 'in the directory'" 1 "is not in the directory (indexOf: unreadable)" -- "$TMP/stack-4663.json" --before-reinstall
db "$OWNERS" 'del db["calls"][("%s wrapper" % DK).lower()]'
check "G-N3: two failed reads never compare equal" 1 "desk.wrapper: unreadable" -- "$TMP/stack-4663.json" --before-reinstall
db "$OWNERS"; stack 'del s["capCeiling"]'
check "G-N3: a stack file without capCeiling FAILS" 1 "the stack file names no capCeiling" -- "$TMP/stack-4663.json" --before-reinstall
stack
out=$(GOLIVE_REHEARSAL=1 run "$TMP/stack-4663.json"); code=$?
[ $code = 1 ] && echo "$out" | grep -q "refused: GOLIVE_REHEARSAL is for our own chains" && echo "ok   GOLIVE_REHEARSAL (the dry run's stand-in Safe) is refused on 4663 (exit 1)" || { echo "UNEXPECTED GOLIVE_REHEARSAL on 4663: exit $code"; fail=1; }

# ---- Ordinals at launch (the operator, 2026-10-09: Bitcoin Frogs joins the Ethereum launch; nft-pilot.json `bitcoin`). The Bitcoin
# side of the mixed stack, read against the COMMITTED launch file on a chain that is not 4663 (so --phase1 is not forced: the
# Ethereum collections this one-collection stack lacks are a note), and each new read wrong alone.
BLAUNCH=deployments/launch/nft-pilot.json
export BCUST="bc1p$(printf 'q%.0s' $(seq 58))"
BROOT=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["bitcoin"]["membersRoot"])' "$BLAUNCH")
python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); d["chainId"]=31337; json.dump(d, open(sys.argv[2],"w"))' "$TMP/4663.json" "$TMP/31337.json"
bstack() { stack 's["chainId"] = 31337; s["collections"][0].update(slug="bitcoin-frogs", name="Wrapped Bitcoin Frog")'"${1:+; $1}"; }
bdb() { db "$OWNERS" 'db["chainId"] = 31337; db["calls"][("%s name" % W).lower()] = "\"Wrapped Bitcoin Frog\""; db["calls"][("%s listing 0" % R).lower()] = "(0, false, false, 0, 0x0000000000000000000000000000000000000000, '"$BROOT"', \"Bitcoin Frogs\", \"bitcoin-frogs\", \"bc1pcustody\")"'"${1:+; $1}"; }
bdfx() { # bdfx [python edits of d]: the canister after the pre-handover setup: the Bitcoin cap, its floor sink and limit, UniSat, the fee pool
  python3 - "$TMP/dfx.json" "$W" "$M" "${1:-}" <<'PY2'
import json, sys
w, m = sys.argv[2].lower(), sys.argv[3].lower()
e = lambda v: {"outpoint": {"txid": "ab" * 32, "vout": 0}, "value": f"{int(v):_}", "source": "operator", "fails": []}
d = {"custody_caps": [{"global_max_items": "48", "global_max_value_wei": "75_000_000_000_000_000_000",
       "collections": [{"collection": "btc:" + w, "max_items": "3", "max_value_wei": "10_000_000_000_000_000_000", "max_item_wei": "10_000_000_000_000_000_000"}]}, []],
     "custody_outflow": [{"per_collection_items": 12, "global_items": 24, "per_collection_mints": 3, "global_mints": 6, "global_value_wei": "25_000_000_000_000_000_000", "trip_factor": 3}, {}],
     "custody_fee_pool": {"entries": [e(25000)], "quarantined": []},
     "floor_state": {"sinks": [{"module": m, "item": {"Bitcoin": None}}], "limits": [{"sink": m, "max_wei": "500_000_000_000_000_000"}], "wants": [], "next_want": "0"},
     "ord_providers": [["ord", "https://ordinals.com"], ["ordinalswallet", "https://turbo.ordinalswallet.com"], ["unisat", "https://open-api.unisat.io"]]}
exec(sys.argv[4])
json.dump(d, open(sys.argv[1], "w"))
PY2
}
binit() { # binit <out file> [js edits of v]: the pre-handover reinstall's argument (a real Init, the monitor's own Candid), naming the wrapper
  ( cd monitor && node --input-type=module - "$W" "${2:-}" ) > "$1" <<'JS'
const { encodeInitPlain } = await import(process.cwd() + "/src/candid.mjs");
const [w, edit] = process.argv.slice(-2);
const v = { chain_id: 4663n, vault: "0x00", rpc_urls: ["https://rpc.mainnet.chain.robinhood.com", "https://rpc.ordofi.network"],
  btc_api_urls: ["https://mempool.space/api", "https://blockstream.info/api"], key_name: "key_1", accept_production_key_name: true,
  bridge_allowlist: [], btc_network: "mainnet", operator: [], wrapper: [w], evm_chains: [], holder_deposits: [], wrapper_from_block: [],
  frog_funding: [{ max_fee_bps: 400, float_first: false }],
  custody_caps: [{ collections: [{ collection: `btc:${w.toLowerCase()}`, max_items: 3, max_value_wei: "10000000000000000000", max_item_wei: "10000000000000000000" }],
    global_max_items: 48, global_max_value_wei: "75000000000000000000" }],
  outflow_limits: [{ per_collection_items: 12, global_items: 24, per_collection_mints: 3, global_mints: 6, global_value_wei: "25000000000000000000", trip_factor: 3 }] };
if (edit) eval(edit);
process.stdout.write(encodeInitPlain(v) + "\n");
JS
}
bmon() { # bmon <out file> [python edits of m]: a monitor configuration with the Bitcoin canary on the stack's custody address
  python3 - "$1" "${2:-}" <<'PY2'
import json, sys
import os
m = {"min_fee_pool_sats": 20000, "bitcoin": {"providers": ["https://mempool.space/api", "https://blockstream.info/api"],
     "addresses": {"custody": os.environ["BCUST"], "float": "bc1p" + "p" * 58,
                   "buyer": "bc1q" + "z" * 38}}}
exec(sys.argv[2])
json.dump(m, open(sys.argv[1], "w"))
PY2
}
brun() { HOME="$TMP/home" FAKE_CAST_DB="$TMP/db.json" DFX="$TMP/dfx" FAKE_DFX_DB="$TMP/dfx.json" script/golive-check.sh http://fake "$TMP/31337.json" "$@" 2>&1; }
bcheck() { # bcheck <name> <expected exit> <pattern> -- <golive-check args…>
  local name="$1" want="$2" pat="$3"; shift 4; local out code
  out=$(brun "$@"); code=$?
  if [ "$code" = "$want" ] && echo "$out" | grep -q -- "$pat"; then echo "ok   $name (exit $code)"; else echo "UNEXPECTED $name: exit $code (wanted $want, /$pat/)"; echo "$out" | grep -E "^(FAIL|WARN)" | head -8; echo "$out" | tail -3; fail=1; fi
}
lcopy() { # lcopy <out file> <python edits of L>: a copy of the committed launch file, changed
  python3 - "$BLAUNCH" "$1" "$2" <<'PY2'
import json, sys
L = json.load(open(sys.argv[1])); exec(sys.argv[3]); json.dump(L, open(sys.argv[2], "w"))
PY2
}
# the custody address the stack names, as the monitor must watch it
bstack 's["collections"][0]["custody"]["address"] = __import__("os").environ["BCUST"]'
bdb 'db["calls"][("%s custodyAddress" % W).lower()] = "\"%s\"" % __import__("os").environ["BCUST"]; db["calls"][("%s listing 0" % R).lower()] = db["calls"][("%s listing 0" % R).lower()].replace("bc1pcustody", __import__("os").environ["BCUST"])'
bdfx; binit "$TMP/binit.hex"; bmon "$TMP/bmon.json"
BA=("$TMP/stack-4663.json" --launch "$BLAUNCH" --canister "$CAN" --install-arg "$TMP/binit.hex" --monitor-config "$TMP/bmon.json")
out=$(brun "${BA[@]}"); code=$?
bseen() { # bseen <name> <pattern>: in the one passing run above
  if [ "$code" = 0 ] && echo "$out" | grep -q -- "$2"; then echo "ok   $1"; else echo "UNEXPECTED $1: exit $code, /$2/"; echo "$out" | grep -E "^(FAIL|WARN)" | head -8; fail=1; fi
}
bseen "ordinals at launch: the stack's Bitcoin collection, as the launch file decides it, passes" "ALL CHECKS PASS"
bseen "... the listing's membership root is the launch file's" "ok   bitcoin-frogs listing 0 membership root = the launch file's bitcoin.membersRoot"
bseen "... the wrapper's name and symbol are the launch file's" "ok   bitcoin-frogs wrapper.name() = the launch file's bitcoin.name"
bseen "... the members file the runner proves from (10,000 distinct ids)" "ok   the members file deployments/launch/btc/bitcoin-frogs-members.json: 10000 distinct inscription ids"
bseen "... frog_funding at max_fee_bps 400 (ordinals-wave R4)" "ok   frog_funding: Relay at max_fee_bps 400, float_first false"
bseen "... the staged plan counts the Bitcoin cap in every step" "(bitcoin-frogs's 3 counted in each step, btc:0x<wrapper>)"
bseen "... A4-L-1: the whole custody, the Bitcoin cap counted, leaves within the delay" "ok   the launch file's custody (custodyItems) at its outflow limits: every item can leave"
bseen "... --canister: the canister names the Bitcoin cap (its install's wrapper is the stack's)" "ok   bitcoin-frogs canister names the cap btc:$W"
bseen "... --canister: the cap is the launch file's 3 items" "ok   bitcoin-frogs canister cap: items in custody = the launch file's bitcoin.custodyItems"
bseen "... --canister: a Bitcoin floor sink for the stack's module (set_floor_sink)" "ok   bitcoin-frogs canister floor_state(): a Bitcoin floor sink for the module $M"
bseen "... --canister: its purchase limit is the launch file's 0.5 ETH (set_purchase_limit)" "ok   bitcoin-frogs canister purchase limit of the Bitcoin module = the launch file's bitcoin.maxPurchaseWei"
bseen "... --canister: UniSat among ord_providers() (set_unisat_key)" "ok   bitcoin-frogs canister ord_providers() names UniSat"
bseen "... --canister: the custody fee pool's 20,000-sat minimum applies" "ok   custody fee pool: one entry of 25000 sats (>= the minimum 20000"
bseen "... --install-arg: the wrapper and frog_funding are the stack's and the launch file's" "ok   the install argument's frog_funding is the launch file's (max_fee_bps 400"
bseen "... --monitor-config: the Bitcoin canary on custody, float and buyer" "ok   the monitor configuration's Bitcoin canary watches custody"
bseen "... and the launch file's Ethereum collections missing from this one-collection stack are a note (a FAIL under --phase1)" "note the launch file names 11 Ethereum collection(s) and this stack deploys none"
bcheck "the same stack under --phase1: the missing Ethereum collections FAIL (the mixed launch is one stack)" 1 "FAIL the launch file names 11 Ethereum collection(s) and this stack deploys none" -- "${BA[@]}" --phase1 --safe-owners "$O1,$O2"
# D8 per collection (the operator, 2026-10-09): OpenSea alone is acceptable for Identity MD, which CoinGecko does not list. The launch
# file decides it (priceRefs ["opensea"] with its _priceRefs note, no coingeckoId); every other collection names both references
bseen "D8: the launch file's references, OpenSea alone for identitymd only (priceRefs, the operator's 2026-10-09 decision)" "ok   the launch file's D8 references: OpenSea and CoinGecko for every Ethereum collection, but OpenSea alone for identitymd (priceRefs"
lcopy "$TMP/l.nocg.json" 'del next(x for x in L["collections"] if x["slug"] == "milady")["coingeckoId"]'
bcheck "D8: a launch file whose Milady has no coingeckoId (and no priceRefs) FAILS" 1 "FAIL the launch file's D8 references: milady: no coingeckoId: D8 needs both references" -- "${BA[@]}" --launch "$TMP/l.nocg.json"
lcopy "$TMP/l.refs2.json" 'next(x for x in L["collections"] if x["slug"] == "identitymd")["priceRefs"] = ["opensea", "coingecko"]'
bcheck "D8: priceRefs other than [\"opensea\"] FAILS" 1 "FAIL the launch file's D8 references: identitymd: priceRefs \[\"opensea\", \"coingecko\"\] is not \[\"opensea\"\]" -- "${BA[@]}" --launch "$TMP/l.refs2.json"
lcopy "$TMP/l.nonote.json" 'next(x for x in L["collections"] if x["slug"] == "identitymd").pop("_priceRefs")'
bcheck "D8: priceRefs [\"opensea\"] without the _priceRefs note recording the operator's decision FAILS" 1 "FAIL the launch file's D8 references: identitymd: priceRefs \[\"opensea\"\] without its _priceRefs note recording the operator's decision of 2026-10-09" -- "${BA[@]}" --launch "$TMP/l.nonote.json"
lcopy "$TMP/l.cgtoo.json" 'next(x for x in L["collections"] if x["slug"] == "identitymd")["coingeckoId"] = "identity-md"'
bcheck "D8: priceRefs [\"opensea\"] beside a coingeckoId FAILS (one or the other)" 1 "FAIL the launch file's D8 references: identitymd: priceRefs \[\"opensea\"\] reads OpenSea alone, yet the file names coingeckoId identity-md" -- "${BA[@]}" --launch "$TMP/l.cgtoo.json"
lcopy "$TMP/l.noprice.json" 'next(x for x in L["collections"] if x["slug"] == "identitymd").pop("priceRefs")'
bcheck "D8: Identity MD without priceRefs (so without a coingeckoId, both are needed) FAILS" 1 "FAIL the launch file's D8 references: identitymd: no coingeckoId" -- "${BA[@]}" --launch "$TMP/l.noprice.json"
# --monitor-config's per-collection judgment (script/d8refs.py monitor_refs, what moncheck prints) on the committed template
D8M=deployments/launch/monitor-nft.json; D8IDMD=0x0000ec93127baa929e58e97dd0095a2bfb38ec1d; D8MIL=0x5af0d9827e0c53e4799bb226655a1de152a425a5
d8() { # d8 <name> <want> <monitor file> <collection> [<launch file>]
  local got; got=$(python3 script/d8refs.py monitor "$3" "$4" "${5:-}")
  [ "$got" = "$2" ] && echo "ok   $1 ($got)" || { echo "UNEXPECTED $1: $got (wanted $2)"; fail=1; }
}
d8 "D8 --monitor-config: Identity MD on OpenSea alone, as the launch file decides" opensea "$D8M" "$D8IDMD" "$BLAUNCH"
d8 "D8 --monitor-config: Milady on both references" both "$D8M" "$D8MIL" "$BLAUNCH"
d8 "D8 --monitor-config: Identity MD's price_refs without --launch (no decision to hold it to) is refused" missing "$D8M" "$D8IDMD"
python3 -c 'import json,sys; m=json.load(open(sys.argv[1])); x=next(c for c in m["eth_nft"]["collections"] if c["slug"]=="milady"); x.pop("coingecko_id"); x["price_refs"]=["opensea"]; json.dump(m, open(sys.argv[2],"w"))' "$D8M" "$TMP/m.milady.json"
d8 "D8 --monitor-config: Milady on OpenSea alone (the launch file does not decide it) is refused" missing "$TMP/m.milady.json" "$D8MIL" "$BLAUNCH"
python3 -c 'import json,sys; m=json.load(open(sys.argv[1])); x=next(c for c in m["eth_nft"]["collections"] if c["slug"]=="identitymd"); x["coingecko_id"]="identity-md"; json.dump(m, open(sys.argv[2],"w"))' "$D8M" "$TMP/m.idcg.json"
d8 "D8 --monitor-config: Identity MD's price_refs beside a coingecko_id is refused" missing "$TMP/m.idcg.json" "$D8IDMD" "$BLAUNCH"
python3 -c 'import json,sys; m=json.load(open(sys.argv[1])); x=next(c for c in m["eth_nft"]["collections"] if c["slug"]=="identitymd"); x.pop("price_refs"); json.dump(m, open(sys.argv[2],"w"))' "$D8M" "$TMP/m.idnone.json"
d8 "D8 --monitor-config: Identity MD with neither price_refs nor a coingecko_id is refused" missing "$TMP/m.idnone.json" "$D8IDMD" "$BLAUNCH"
python3 -c 'import json,sys; m=json.load(open(sys.argv[1])); x=next(c for c in m["eth_nft"]["collections"] if c["slug"]=="identitymd"); x["opensea_slug"]="milady"; json.dump(m, open(sys.argv[2],"w"))' "$D8M" "$TMP/m.idswap.json"
d8 "D8 --monitor-config: Identity MD on another collection's OpenSea slug is refused (A3-L-3)" missing "$TMP/m.idswap.json" "$D8IDMD" "$BLAUNCH"
# A4-L-1 with the Bitcoin cap: option (c)'s first split (Azuki 17: 48 Ethereum items) and no binding global, the dormant five empty
lcopy "$TMP/l.a4.json" 'gi = [s.pop("globalItems") for s in L["stages"]]
for x in L["collections"]:
    if x["slug"] not in ("milady", "azuki", "mayc", "pudgypenguins", "bayc"): x["custodyItems"] = 0
L["collections"][1]["custodyItems"] = 17
for s in L["stages"][:2]: s["custodyItems"].update({k: 0 for k in ("doodles", "moonbirds", "clonex", "lilpudgys", "checks", "identitymd")}, azuki=17)'
bcheck "A4-L-1: 48 Ethereum items + the Bitcoin wrapper's 3 at 24 a day (51 > 48) FAILS, the Bitcoin cap counted" 1 "FAIL the launch file's custody (custodyItems) at its outflow limits: custody holds up to 51 items in all and lets 24 out a day in all: 48 can leave" -- "$TMP/stack-4663.json" --launch "$TMP/l.a4.json"
bcheck "... the options name the Bitcoin collection by its slug" 1 "bitcoin-frogs 3 -> " -- "$TMP/stack-4663.json" --launch "$TMP/l.a4.json"
lcopy "$TMP/l.a4nobtc.json" 'gi = [s.pop("globalItems") for s in L["stages"]]
for x in L["collections"]:
    if x["slug"] not in ("milady", "azuki", "mayc", "pudgypenguins", "bayc"): x["custodyItems"] = 0
L["collections"][1]["custodyItems"] = 17
for s in L["stages"]: s["custodyItems"].pop("bitcoin-frogs")
for s in L["stages"][:2]: s["custodyItems"].update({k: 0 for k in ("doodles", "moonbirds", "clonex", "lilpudgys", "checks", "identitymd")}, azuki=17)
del L["bitcoin"]'
out=$(brun "$TMP/stack-4663.json" --launch "$TMP/l.a4nobtc.json"); echo "$out" | grep -q "^ok   the launch file's custody (custodyItems) at its outflow limits: every item can leave .* 48 items in all at 24 a day" \
  && echo "ok   ... and the same 48 Ethereum items without the Bitcoin collection fit (the 3 Bitcoin items are what tips it)" || { echo "UNEXPECTED the 48-item Ethereum split alone did not fit"; echo "$out" | grep -E 'A4-L-1' | head -3; fail=1; }
lcopy "$TMP/l.step3.json" 'L["stages"][2]["custodyItems"]["bitcoin-frogs"] = 4'
bcheck "the staged plan: a step that moves the Bitcoin wrapper's cap FAILS (stage 0's 3 at every step)" 1 "FAIL the launch file's staged plan: step 3 names bitcoin-frogs at 4 items, not the Bitcoin wrapper's 3" -- "${BA[@]}" --launch "$TMP/l.step3.json"
lcopy "$TMP/l.g64.json" 'L["stages"][2]["globalItems"] = 64'
bcheck "the staged plan: step 3's global as the pilot five's J7 61 + Frogs' 3 = 64 at 31 a day FAILS (62 can leave)" 1 "step 3 (caps back to J7): 64 items at 12 / 31 a day do not fit the exit rule" -- "${BA[@]}" --launch "$TMP/l.g64.json"
lcopy "$TMP/l.root.json" 'L["bitcoin"]["membersRoot"] = "0x" + "11" * 32'
bcheck "a listing whose membership root is not the launch file's FAILS" 1 "FAIL bitcoin-frogs listing 0 membership root = the launch file's bitcoin.membersRoot" -- "${BA[@]}" --launch "$TMP/l.root.json"
lcopy "$TMP/l.count.json" 'L["bitcoin"]["membersCount"] = 9999'
bcheck "a members file whose count is not the launch file's FAILS" 1 "FAIL deployments/launch/btc/bitcoin-frogs-members.json holds 10000 ids, the launch file's membersCount is 9999" -- "${BA[@]}" --launch "$TMP/l.count.json"
lcopy "$TMP/l.policy.json" 'L["bitcoin"]["frog_funding"]["max_fee_bps"] = 600; L["bitcoin"]["unisatKeyRequired"] = False; L["bitcoin"]["custodyFeePoolMinSats"] = 10000'
bcheck "a launch file with frog_funding over 500 bps FAILS" 1 "FAIL bitcoin.frog_funding" -- "${BA[@]}" --launch "$TMP/l.policy.json"
bcheck "... with UniSat's key not required FAILS" 1 "FAIL bitcoin.unisatKeyRequired is not true" -- "${BA[@]}" --launch "$TMP/l.policy.json"
bcheck "... with a fee-pool minimum under 20,000 sats FAILS" 1 "FAIL bitcoin.custodyFeePoolMinSats is under the operator's 20,000-sat minimum" -- "${BA[@]}" --launch "$TMP/l.policy.json"
lcopy "$TMP/l.nobtc.json" 'del L["bitcoin"]; [s["custodyItems"].pop("bitcoin-frogs") for s in L["stages"]]'
bcheck "a Bitcoin stack against a launch file that decides no Bitcoin collection FAILS" 1 "FAIL bitcoin-frogs: the stack deploys a Bitcoin (inscriptions) collection the launch file" -- "${BA[@]}" --launch "$TMP/l.nobtc.json"
bstack 's["collections"][0]["custody"]["address"] = __import__("os").environ["BCUST"]; s["collections"][0]["slug"] = "frogs"'
bcheck "a stack whose slug is not the launch file's FAILS (KEEPER_OW_SLUGS keys on it)" 1 "FAIL frogs stack file's slug = the launch file's bitcoin.slug: expected 'bitcoin-frogs', found 'frogs'" -- "${BA[@]}"
bstack 's["collections"][0]["custody"]["address"] = __import__("os").environ["BCUST"]'
bdfx 'd["custody_caps"][0]["collections"] = []'
bcheck "--canister: a canister whose install names no wrapper (no btc: cap) FAILS" 1 "FAIL bitcoin-frogs: the canister names no cap for btc:$W: its install's wrapper is not this stack's wrapper" -- "${BA[@]}"
bdfx 'd["custody_caps"][0]["collections"][0]["max_items"] = "4"'
bcheck "--canister: a Bitcoin cap of 4 items FAILS (the launch file's 3)" 1 "FAIL bitcoin-frogs canister cap: items in custody = the launch file's bitcoin.custodyItems (H2 stage 0): expected 3" -- "${BA[@]}"
bdfx 'd["floor_state"]["sinks"] = []'
bcheck "--canister: no Bitcoin floor sink (set_floor_sink not made before the handover) FAILS" 1 "FAIL bitcoin-frogs canister floor_state() has no Bitcoin floor sink for the stack's module" -- "${BA[@]}"
bdfx 'd["floor_state"]["sinks"][0]["item"] = {"Evm": {"chain_id": "1", "collection": "00" * 20, "wrapper": "00" * 20}}'
bcheck "--canister: the module's sink buying an EVM collection, not Bitcoin, FAILS" 1 "has no Bitcoin floor sink for the stack's module" -- "${BA[@]}"
bdfx 'd["floor_state"]["limits"][0]["max_wei"] = "1_000_000_000_000_000_000"'
bcheck "--canister: a purchase limit of 1 ETH, not the launch file's 0.5, FAILS" 1 "FAIL bitcoin-frogs canister purchase limit of the Bitcoin module = the launch file's bitcoin.maxPurchaseWei (set_purchase_limit): expected 500000000000000000" -- "${BA[@]}"
bdfx 'd["floor_state"]["limits"] = []'
bcheck "--canister: no purchase limit while the launch file's is the policy's 0.5 ETH: a note" 0 "note bitcoin-frogs canister names no purchase limit for $M: the policy's MAX_PURCHASE_WEI" -- "${BA[@]}"
bdfx 'm2 = m; d["floor_state"]["sinks"][0]["module"] = [int(m2[i:i+2], 16) for i in range(2, 42, 2)]; d["floor_state"]["limits"][0]["sink"] = "".join("\\%02x" % int(m2[i:i+2], 16) for i in range(2, 42, 2))'
bcheck "--canister: blobs printed as a byte list or as \\xx text read the same" 0 "ok   bitcoin-frogs canister purchase limit of the Bitcoin module" -- "${BA[@]}"
bdfx 'd["ord_providers"] = d["ord_providers"][:2]'
bcheck "--canister: UniSat not among ord_providers() (no set_unisat_key, or an upgrade since) FAILS" 1 "FAIL bitcoin-frogs canister ord_providers() does not name UniSat (ord,ordinalswallet)" -- "${BA[@]}"
bdfx 'd["custody_fee_pool"]["entries"][0]["value"] = "15_000"'
bcheck "--canister: a custody fee pool under 20,000 sats FAILS" 1 "FAIL custody fee pool: no usable entry of at least 20000 sats (largest 15000" -- "${BA[@]}"
bdfx
lcopy "$TMP/l.pool.json" 'L["bitcoin"]["custodyFeePoolMinSats"] = 30000'
bcheck "--canister: the launch file's custodyFeePoolMinSats (30,000) raises the minimum: a 25,000-sat entry FAILS" 1 "FAIL custody fee pool: no usable entry of at least 30000 sats (largest 25000" -- "${BA[@]}" --launch "$TMP/l.pool.json"
binit "$TMP/binit.nofund.hex" 'v.frog_funding = []'
bcheck "--install-arg: no frog_funding (Relay at 300 bps) FAILS" 1 "FAIL the install argument's frog_funding is the launch file's (max_fee_bps 400, float_first false): null" -- "${BA[@]}" --install-arg "$TMP/binit.nofund.hex"
binit "$TMP/binit.nowrap.hex" 'v.wrapper = []'
bcheck "--install-arg: no wrapper FAILS" 1 "FAIL the install argument's wrapper is the stack's Bitcoin wrapper" -- "${BA[@]}" --install-arg "$TMP/binit.nowrap.hex"
bmon "$TMP/bmon.bad.json" 'm["bitcoin"]["providers"] = ["https://mempool.space/api"]; m["bitcoin"]["addresses"]["custody"] = "bc1q" + "x" * 38'
bcheck "--monitor-config: one esplora provider and a custody address that is not the wrapper's FAIL" 1 "FAIL the monitor configuration's Bitcoin canary: bitcoin.providers must be two esplora bases on two hosts (1 host(s)); bitcoin.addresses.custody" -- "${BA[@]}" --monitor-config "$TMP/bmon.bad.json"

[ $fail = 0 ] && echo "AUDIT2 GOLIVE-CHECK: every case as expected (R2's passing attacks now FAIL)" || echo "AUDIT2 GOLIVE-CHECK: a case changed"
exit $fail
