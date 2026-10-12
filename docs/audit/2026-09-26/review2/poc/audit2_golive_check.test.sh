#!/usr/bin/env bash
# audit2 (R2 services review, 2026-09-26): script/golive-check.sh --phase1 against a scripted chain.
# A fake `cast` is planted in a throwaway $HOME/.foundry/bin (golive-check puts that directory first on PATH), answering from a
# table; nothing reaches a network. Each case states what a go/no-go read SHOULD say and what it does say.
#   usage: scripts/audit2_golive_check.test.sh      (exit 0 = every case behaved as recorded below)
set -uo pipefail
cd "$(dirname "$0")/.."
ROOT=$(pwd)
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
if cmd == "call":
    key = rest[0].lower() + " " + rest[1].split("(")[0] + ("" if len(rest) < 3 else " " + " ".join(rest[2:]).lower())
    key = key.lower()
    if key in db["calls"]: print(db["calls"][key]); sys.exit(0)
    print("Error: execution reverted", file=sys.stderr); sys.exit(1)
print("fake cast: unknown " + " ".join(a), file=sys.stderr); sys.exit(1)
PY
chmod +x "$TMP/home/.foundry/bin/cast"

R=0x00000000000000000000000000000000000000a1; L=0x00000000000000000000000000000000000000a2; D=0x00000000000000000000000000000000000000a3
B=0x00000000000000000000000000000000000000a4; F=0x00000000000000000000000000000000000000a5; I=0x00000000000000000000000000000000000000a6
E=0x00000000000000000000000000000000000000a7; SAFE=0x00000000000000000000000000000000000005af
K=0x00000000000000000000000000000000000000b1; P=0x00000000000000000000000000000000000000b2; A=0x00000000000000000000000000000000000000b3
T=0x00000000000000000000000000000000000000b4; G=0x00000000000000000000000000000000000000c1
Z=0x0000000000000000000000000000000000000000
cat > "$TMP/4663.json" <<EOF
{"chainId": 4663, "registry": "$R", "launcher": "$L", "distributor": "$D", "beacon": "$B", "ponsFactory": "$F",
 "routerImplementation": "$I", "owner": "$SAFE", "treasury": "$T", "wired": true, "keeper": "$K", "purchaser": "$P", "attestor": "$A"}
EOF
db() { # db <safe owners> [extra python edits]: the table for a correct-looking 4663 registry, no stack file
  python3 - "$TMP/db.json" "$1" "${2:-}" <<PY
import json, sys
R,L,D,B,F,I,E,SAFE,K,P,A,T,G,Z = "$R $L $D $B $F $I $E $SAFE $K $P $A $T $G $Z".split()
calls = {
 f"{R} owner": SAFE, f"{R} treasury": T, f"{R} launcher": L, f"{R} distributor": D, f"{R} namedKeeper": K, f"{R} keeper": K,
 f"{R} purchaser": P, f"{R} attestor": A, f"{R} keeperPaused": "false", f"{R} pendingOwner": Z,
 f"{R} pendingRoles": f"{Z}\n{Z}\n{Z}\n0", f"{R} pendingTreasury": Z, f"{R} pendingBondAt": "0", f"{R} guardian": G,
 f"{R} count": "0",
 f"{SAFE} getThreshold": "2", f"{SAFE} getOwners": "[" + sys.argv[2] + "]",
 f"{L} factory": F, f"{L} registry": R, f"{F} feeEscrow": E, f"{L} escrow": E, f"{L} routerImplementation": I,
 f"{I} launcher": L, f"{I} coin": "0x0000000000000000000000000000000000000001", f"{D} registry": R, f"{D} beacon": B,
}
code = {x.lower(): "0x6080" for x in (R, L, D, B, F, I, SAFE)}
db = {"chainId": 4663, "calls": {k.lower(): v for k, v in calls.items()}, "code": code}
exec(sys.argv[3])
json.dump(db, open(sys.argv[1], "w"))
PY
}
run() { HOME="$TMP/home" FAKE_CAST_DB="$TMP/db.json" script/golive-check.sh http://fake "$TMP/4663.json" "$@" 2>&1; }
fail=0
check() { # check <name> <expected exit> <grep pattern that must appear> -- <golive-check args…>
  local name="$1" want="$2" pat="$3"; shift 4; local out code
  out=$(run "$@"); code=$?
  if [ "$code" = "$want" ] && echo "$out" | grep -q -- "$pat"; then echo "ok   $name (exit $code)"; else echo "UNEXPECTED $name: exit $code"; echo "$out" | tail -8; fail=1; fi
}

# 1. The go/no-go read on 4663 with NO stack file: --phase1 is implied, yet none of C-07 (shares), B-05 (module.wrapped), C-01
#    (cap ceilings), the wrapper's wiring, the treasury/directory, or ops is read — and it prints the go/no-go PASS.
db "0x00000000000000000000000000000000000000d1, 0x00000000000000000000000000000000000000d2"
check "--phase1 on 4663 without the stack file passes (C-08 gate fail-open)" 0 "ALL CHECKS PASS (--phase1: the go/no-go read)" --

# 2. The Safe's two owners are the keeper and the guardian: every key of the 'Safe' is a hot key on a server. Still "a Safe, 2 of 2".
db "$K, $G"
check "--phase1 passes with the Safe's owners = {keeper, guardian}" 0 "a Safe, 2 of 2" --

# 3. Any contract answering getThreshold()=2 and a two-element getOwners() is "a Safe" (no singleton / proxy / module / guard check).
#    Same table: the 'Safe' is just code 0x6080 with two answers. A Safe with an enabled module (threshold bypass) looks the same.
db "0x00000000000000000000000000000000000000d1, 0x00000000000000000000000000000000000000d2"
check "--phase1 accepts any contract with getThreshold()=2 as the 2-of-2 Safe" 0 "is a Safe, 2 of 2" --

# 4. registry.count() unreadable (RPC error / revert): no listing is checked and no FAIL is printed.
db "0x00000000000000000000000000000000000000d1, 0x00000000000000000000000000000000000000d2" 'del db["calls"][("%s count" % R).lower()]'
check "--phase1 passes when registry.count() cannot be read (listings silently skipped)" 0 "ALL CHECKS PASS" --

# 5. Control: the keeper paused still FAILs (fail-closed reads work where they are written as '= expected').
db "0x00000000000000000000000000000000000000d1, 0x00000000000000000000000000000000000000d2" 'db["calls"][("%s keeperPaused" % R).lower()] = "true"'
check "control: keeperPaused FAILs" 1 "FAIL registry.keeperPaused()" --

# 6. Control: keeperPaused unreadable FAILs (fail-closed).
db "0x00000000000000000000000000000000000000d1, 0x00000000000000000000000000000000000000d2" 'del db["calls"][("%s keeperPaused" % R).lower()]'
check "control: keeperPaused unreadable FAILs" 1 "FAIL registry.keeperPaused()" --

[ $fail = 0 ] && echo "AUDIT2 GOLIVE-CHECK: every case as recorded" || echo "AUDIT2 GOLIVE-CHECK: a case changed"
exit $fail
