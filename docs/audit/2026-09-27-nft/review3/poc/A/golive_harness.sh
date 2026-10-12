#!/usr/bin/env bash
# Round-3 re-review, part A: golive-check.sh's --launch / --monitor-config functions (script/golive-check.sh at 4949da0, lines 65-71
# and 507-572, extracted verbatim) driven with a stubbed chain. Run from a private copy of the repository root:
#   bash golive_harness.sh <repo root>
# Each case prints the functions' own ok/FAIL lines; a case named "PASSES (weakness)" must print no FAIL line.
R="${1:-.}"; G="$R/script/golive-check.sh"
eval "$(sed -n '65p;68,71p' "$G")"
eval "$(sed -n '507,572p' "$G")"
T=$(mktemp -d)
# the stubbed chain: what a correct five-collection deploy reads back (the launch file's values)
call() { case "$1:$2" in
  F_*:"normalRaise()(uint256)") [ "$1" = F_bayc ] && echo 9000000000000000000 || echo 5000000000000000000;;
  F_*:"earlyRaise()(uint256)")  [ "$1" = F_bayc ] && echo 9000000000000000000 || echo 8000000000000000000;;
  F_*:"earlyUntilEth()(uint256)") echo 2000000000000000000;;
  F_*:"maxRaise()(uint256)") echo 30000000000000000000;;
  M_*:"convBps()(uint256)") echo 300;;
  M_*:"reserveBps()(uint256)") echo 2500;;
  *) echo "";; esac; }
col() { case "$1" in milady) echo 0x5af0d9827e0c53e4799bb226655a1de152a425a5;; azuki) echo 0xed5af388653567af2f388e6224dc7c4b3241c544;;
  mayc) echo 0x60e4d786628fea6478f785a6d7e704777c86a7c6;; pudgypenguins) echo 0xbd3531da5cf5857e7cfaa92426877b022e612cf8;;
  bayc) echo 0xbc4ca0eda7647a8ab7c2061c2e118a18a936f13d;; esac; }
rep() { local o=""; for _ in $(seq 20); do o="$o$1"; done; echo "0x$o"; }
CUST=0x0000000000000000000000000000000000b0b0e1
fill() { # the monitor template filled as scripts/golive-dryrun.sh fills it (wrapper W_<slug>, module M_<slug> as fake addresses)
  python3 - "$R/deployments/launch/monitor-nft.json" "$CUST" <<'PY'
import json, sys
t = open(sys.argv[1]).read().replace("<the canister's EVM address>", sys.argv[2]).replace("<the guardian EOA (registry.guardian())>", "0x" + "11" * 20)
for i, s in enumerate(["milady", "azuki", "mayc", "pudgypenguins", "bayc"]):
    t = t.replace(f"<{s} wrapper>", "0x" + f"{0xa0+i:02x}" * 20).replace(f"<{s} floorModule>", "0x" + f"{0xb0+i:02x}" * 20)
print(t)
PY
}
run() { # run <name> <launch file> <monitor file>
  echo "== $1"; LAUNCH="$2"; MONCFG="$3"; fail=0; local i=0
  for s in milady azuki mayc pudgypenguins bayc; do
    launchcheck "$s" "F_$s" "M_$s" "$(col $s)" | grep -E "^FAIL"
    moncheck "$s" "$(col $s)" "$(rep $(printf '%02x' $((0xa0+i))))" "$(rep $(printf '%02x' $((0xb0+i))))" "$CUST" | grep -E "^FAIL"
    i=$((i+1))
  done
  monglobal | grep -E "^FAIL"
  echo "   ($(i=0; for s in milady azuki mayc pudgypenguins bayc; do moncheck "$s" "$(col $s)" "$(rep $(printf '%02x' $((0xa0+i))))" "$(rep $(printf '%02x' $((0xb0+i))))" "$CUST"; i=$((i+1)); done | grep -c '^ok') monitor ok lines)"
  echo "   ($(for s in milady azuki mayc pudgypenguins bayc; do launchcheck "$s" "F_$s" "M_$s" "$(col $s)"; done | grep -c '^ok') launch ok lines)"
}
fill > "$T/mon.json"
run "control: the committed launch file and the filled template: no FAIL" "$R/deployments/launch/nft-pilot.json" "$T/mon.json"
# 1. the wrapper's name / symbol / slug are not held against the launch file: a shifted ETCH_NFT_NAMES / _SYMBOLS / _SLUGS list
python3 - "$R/deployments/launch/nft-pilot.json" "$T/shifted-names.json" <<'PY'
import json, sys
L = json.load(open(sys.argv[1])); c = L["collections"]
names = [(x["name"], x["symbol"], x["slug"]) for x in c]; names = names[-1:] + names[:-1]   # rotated by one
for x, (n, s, g) in zip(c, names): x["name"], x["symbol"], x["slug"] = n, s, g
json.dump(L, open(sys.argv[2], "w"))
PY
run "1 PASSES (weakness): name/symbol/slug are never read by --launch (rotated here: nothing changes, so a rotated deploy is not caught either)" "$T/shifted-names.json" "$T/mon.json"
# 2. D8's references swapped between two collections (BAYC watched on MAYC's OpenSea slug and CoinGecko id)
python3 - "$T/mon.json" "$T/mon-swapped.json" <<'PY'
import json, sys
m = json.load(open(sys.argv[1])); c = {x["slug"]: x for x in m["eth_nft"]["collections"]}
for k in ("opensea_slug", "coingecko_id"): c["milady"][k], c["mayc"][k] = c["mayc"][k], c["milady"][k]
json.dump(m, open(sys.argv[2], "w"))
PY
run "2 PASSES (weakness): D8 references swapped between Milady and MAYC (floors 0.93 / 0.86 ETH: within D8's 20 %, silent)" "$R/deployments/launch/nft-pilot.json" "$T/mon-swapped.json"
# 3. the monitor's guardian is not the registry's guardian; 4. placeholders left outside evm.guardian / eth_nft; 5. one provider twice
python3 - "$T/mon.json" "$T/mon-other.json" <<'PY'
import json, sys
m = json.load(open(sys.argv[1]))
m["evm"]["guardian"]["address"] = "0x000000000000000000000000000000000000dead"      # not registry.guardian()
m["eth_nft"]["eth_rpc"] = ["https://eth.drpc.org", "https://eth.drpc.org/"]          # one provider, two spellings
m["evm"]["stack_file"] = "../stack-1.json"; m["evm"]["chain_id"] = 1                   # the wrong chain's stack
json.dump(m, open(sys.argv[2], "w"))                                                   # timelock, buyer, expect, liveness: still <...>
PY
run "3-5 PASSES (weakness): guardian not the registry's, <p1>/<runner host> placeholders, one Ethereum provider twice, evm.chain_id 1" "$R/deployments/launch/nft-pilot.json" "$T/mon-other.json"
echo "   placeholders left in the file that passed: $(grep -o '<[^">]*>' "$T/mon-other.json" | sort -u | tr '\n' ' ')"
# 6. control: the refusals the fixer tested still refuse
python3 - "$T/mon.json" "$T/mon-nopause.json" <<'PY'
import json, sys
m = json.load(open(sys.argv[1])); del m["evm"]["guardian"]["pause_on_no_good_post_h"]; json.dump(m, open(sys.argv[2], "w"))
PY
run "6 control: no pause_on_no_good_post_h: FAILs" "$R/deployments/launch/nft-pilot.json" "$T/mon-nopause.json"
rm -rf "$T"
