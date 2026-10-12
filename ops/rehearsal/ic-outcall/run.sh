#!/usr/bin/env bash
# The go-live IC-outcall rehearsal (docs/v7/phase1-nft.md §5.1; review round 7 "hard precondition"). README.md is the runbook.
#
#   ops/rehearsal/ic-outcall/run.sh --network ic (--subnet <subnet id> | --next-to <canister id> | --subnet-type <type>) [--cycles <n>] [--yes]
#   ops/rehearsal/ic-outcall/run.sh --network local                  (a local replica must be running: dfx start --clean --background)
#
# With the OPERATOR's own dfx identity (never the buyer's controller key material; any identity holding cycles on the cycles ledger):
#   1. builds the probe (probe/, Rust 1.98.1, wasm32) and prints its sha256;
#   2. creates a THROWAWAY canister on the named subnet with --cycles (default 1.5 T on ic), installs the probe;
#   3. calls probe(null): the buyer's Ethereum reads, through the buyer's transforms, on rpc.mevblocker.io and
#      gateway.tenderly.co/public/mainnet; prints PASS/FAIL per check and the cycles the outcalls cost;
#   4. ALWAYS (a trap, also on failure or Ctrl-C): stops the canister and deletes it; dfx withdraws what is left to the identity's
#      cycles-ledger balance first (dfx canister delete's default). Only the creation fee and the outcalls are spent.
# Exit 0 only when every check passed. Nothing is signed, no transaction is sent, the probe holds no key and no state.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
cd "$HERE"
DFX="${DFX:-$HOME/Library/Application Support/org.dfinity.dfx/bin/dfx}"; [ -x "$DFX" ] || DFX="$(command -v dfx || true)"
[ -x "$DFX" ] || { echo "refused: dfx not found (set DFX)"; exit 2; }
export DFX_WARNING=-mainnet_plaintext_identity
NET=""; WHERE=(); CYCLES=""; YES=0; URLS="null"
while [ $# -gt 0 ]; do
  case "$1" in
    --network) NET="${2:-}"; shift;;
    --subnet) WHERE=(--subnet "${2:-}"); shift;;
    --next-to) WHERE=(--next-to "${2:-}"); shift;;
    --subnet-type) WHERE=(--subnet-type "${2:-}"); shift;;
    --cycles) CYCLES="${2:-}"; shift; echo "$CYCLES" | grep -Eq '^[0-9]+$' || { echo "refused: --cycles takes a whole number of cycles"; exit 2; };;
    # other providers than the canister's pair (e.g. a replacement candidate): comma-separated https URLs, keyless
    --urls) URLS="(opt vec { $(echo "${2:-}" | tr ',' '\n' | sed 's/.*/"&";/' | tr -d '\n') })"; shift;;
    --yes) YES=1;;
    *) echo "refused: unknown argument $1 (see the header of $0)"; exit 2;;
  esac; shift
done
case "$NET" in
  ic) [ ${#WHERE[@]} -gt 0 ] || { echo "refused: --network ic needs the LAUNCH subnet: --subnet <id> (the buyer's subnet), --next-to <the buyer's canister id>, or --subnet-type <type>"; exit 2; }
      CYCLES="${CYCLES:-1500000000000}";;
  local) CYCLES="${CYCLES:-1500000000000}";;
  *) echo "refused: --network ic or --network local"; exit 2;;
esac

echo "== build"
( cd probe && PATH="$HOME/.cargo/bin:$PATH" cargo +1.98.1 build --release --locked --target wasm32-unknown-unknown -q ) || { echo "FAIL the probe did not build"; exit 1; }
WASM=probe/target/wasm32-unknown-unknown/release/rook_outcall_probe.wasm
echo "probe wasm: $(shasum -a 256 "$WASM" | cut -d' ' -f1)  ($(wc -c < "$WASM" | tr -d ' ') bytes)"
ME=$("$DFX" identity whoami); PRINCIPAL=$("$DFX" identity get-principal)
echo "identity: $ME ($PRINCIPAL)"

if [ "$NET" = ic ]; then
  BAL=$("$DFX" cycles balance --network ic 2>&1) || { echo "refused: dfx cycles balance failed: $BAL"; exit 2; }
  echo "cycles-ledger balance before: $BAL"
  cat <<EOF
This creates a canister on the IC (${WHERE[*]}) with $CYCLES cycles from $ME's cycles-ledger balance, makes 10 HTTPS outcalls
(estimate: ~4.4 B cycles on a 13-node subnet, measured 4.40 B locally; ~12 B on a 34-node one), then deletes it and withdraws the rest. Expected spend: the
creation fee (500 B on 13 nodes, ~1.3 T on 34) plus the outcalls, install and deletion calls: ~0.51 T (13 nodes) / ~1.32 T (34).
EOF
  if [ $YES != 1 ]; then read -r -p "Type 'rehearse' to go on: " a; [ "$a" = rehearse ] || { echo "stopped; nothing was created"; exit 2; }; fi
fi

CREATED=0
cleanup() {
  [ $CREATED = 1 ] || return 0
  echo "== clean up: stop, withdraw what is left, delete"
  "$DFX" canister stop probe --network "$NET" >/dev/null 2>&1 || echo "warning: stop failed (delete -y tries anyway)"
  if "$DFX" canister delete probe --network "$NET" -y 2>&1 | grep -vE "Attempting to transfer|Not enough margin|dfx is deprecated" | sed 's/^/   /'; [ "${PIPESTATUS[0]}" = 0 ]; then echo "deleted: the canister is gone"
  # 2026-10-09 (path B): dfx's withdrawal can fail on a stopped canister after installing its wallet (IC0508); started again, it works
  elif "$DFX" canister start probe --network "$NET" >/dev/null 2>&1 && "$DFX" canister delete probe --network "$NET" -y 2>&1 | grep -vE "Attempting to transfer|Not enough margin|dfx is deprecated" | sed 's/^/   /'; [ "${PIPESTATUS[0]}" = 0 ]; then echo "deleted on the second try (started, then deleted)"
  else echo "WARNING: the delete FAILED. Run by hand: $DFX canister start probe --network $NET && $DFX canister delete probe --network $NET -y"; fi
  rm -f canister_ids.json
  [ "$NET" = ic ] && echo "cycles-ledger balance after: $("$DFX" cycles balance --network ic 2>&1)"
  CREATED=0
}
trap cleanup EXIT
trap 'exit 130' INT TERM

echo "== create (${WHERE[*]:-local replica}) and install"
"$DFX" canister create probe --network "$NET" --with-cycles "$CYCLES" ${WHERE[@]+"${WHERE[@]}"} 2>&1 | sed 's/^/   /'
"$DFX" canister id probe --network "$NET" >/dev/null 2>&1 || { echo "FAIL the canister was not created"; exit 1; }
CREATED=1
ID=$("$DFX" canister id probe --network "$NET")
echo "probe canister: $ID"
"$DFX" canister install probe --network "$NET" --mode install --wasm "$WASM" -y 2>&1 | sed 's/^/   /'
[ "$NET" = ic ] && "$DFX" canister status probe --network ic 2>/dev/null | grep -iE "subnet|balance|module hash" | sed 's/^/   /'

echo "== probe (every read on each provider; ~1 min on ic)"
ERR="${TMPDIR:-/tmp}/rook-ic-outcall-call.err"
OUT=$("$DFX" canister call probe probe "$URLS" --network "$NET" --output json 2>"$ERR"); rc=$?
if [ $rc != 0 ]; then echo "FAIL the probe call failed: $(grep -v 'dfx is deprecated' "$ERR")"; exit 1; fi
printf '%s' "$OUT" > "${TMPDIR:-/tmp}/rook-ic-outcall-report.json"
python3 - "$OUT" <<'PY'
import json, sys
r = json.loads(sys.argv[1])
for c in r["checks"]:
    print(f"{'PASS' if c['pass'] else 'FAIL'} {c['name']}: {c['detail']}")
b, a = int(str(r["cycles_before"]).replace("_", "")), int(str(r["cycles_after"]).replace("_", ""))
print(f"pin: {r['pin'] or '-'}; the outcalls cost {b - a:,} cycles on this subnet")
for p in r["providers"]:
    print(f"   {p['url']}:\n      head {p['head']}\n      block {p['block'][:200]}\n      logs {p['logs'][:200]}\n      receipt {p['receipt'][:200]}")
print("REHEARSAL: PASS" if r["pass"] else "REHEARSAL: FAIL")
sys.exit(0 if r["pass"] else 1)
PY
rc=$?
echo "(the full report: ${TMPDIR:-/tmp}/rook-ic-outcall-report.json)"
exit $rc
