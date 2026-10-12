#!/usr/bin/env bash
# Spike Q3 (docs/v9/solana-migration.md §2.4; the page: docs/v9/spikes/q3-ic-ed25519-durable-nonce.md). Solana DEVNET only.
#
#   ops/rehearsal/sol-sign/run.sh --local                     (a local replica must be running here: cd ops/rehearsal/sol-sign && dfx start --clean --background)
#   ops/rehearsal/sol-sign/run.sh --ic (--subnet <id> | --next-to <canister id> | --subnet-type <type>) [--cycles <n>] [--yes]
#   common: [--fund-wait <seconds>] [--non-replicated-send] [--rpc-urls <url>,<url>]
#
# With the OPERATOR's own dfx identity (any identity holding cycles on the cycles ledger; nothing of the buyer's):
#   1. builds the canister (canister/, Rust 1.98.1, wasm32) and prints its sha256;
#   2. creates a THROWAWAY canister (local, or on IC mainnet with --cycles, default 1.5 T) and installs it with the key
#      `key_1` (local: dfx 0.32's PocketIC holds schnorr:Ed25519:key_1, NOT dfx_test_key) or `test_key_1` (IC; threshold Ed25519, the test key) and two keyless DEVNET providers;
#   3. drive.py: the canister's Solana address, a devnet airdrop to it (RPC requestAirdrop; if the faucet refuses, the CLI's
#      airdrop, then a wait for a manual https://faucet.solana.com top-up), a durable-nonce account, a durable-nonce transfer
#      resent as the SAME saved bytes, the fallback lifetime with an expiry drill (re-sign on proven expiry), and the metrics
#      (cycles per signature from balance deltas, latencies);
#   4. ALWAYS (a trap, also on failure or Ctrl-C): deletes the canister (dfx stops it itself); dfx withdraws what is left to the identity's
#      cycles-ledger balance first. The devnet SOL left at the canister's address is worthless and stays there.
# No private key is printed or written: the canister's key is a threshold key, and the recipient is a throwaway keypair generated
# and deleted at run time (only its public half is used). Exit 0 only when every check passed.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
cd "$HERE"
DFX="${DFX:-$HOME/Library/Application Support/org.dfinity.dfx/bin/dfx}"; [ -x "$DFX" ] || DFX="$(command -v dfx || true)"
[ -x "$DFX" ] || { echo "refused: dfx not found (set DFX)"; exit 2; }
export DFX_WARNING=-mainnet_plaintext_identity
NET=""; WHERE=(); CYCLES=""; YES=0; FUND_WAIT=300; REPLICATED_SEND=true
URLS='vec { "https://api.devnet.solana.com"; "https://solana-devnet.api.onfinality.io/public" }'
while [ $# -gt 0 ]; do
  case "$1" in
    --local) NET=local;;
    --ic) NET=ic;;
    --subnet) WHERE=(--subnet "${2:-}"); shift;;
    --next-to) WHERE=(--next-to "${2:-}"); shift;;
    --subnet-type) WHERE=(--subnet-type "${2:-}"); shift;;
    --cycles) CYCLES="${2:-}"; shift; echo "$CYCLES" | grep -Eq '^[0-9]+$' || { echo "refused: --cycles takes a whole number of cycles"; exit 2; };;
    --fund-wait) FUND_WAIT="${2:-}"; shift;;
    --non-replicated-send) REPLICATED_SEND=false;;
    # other providers (comma-separated; the first also serves the script's own reads and airdrop), e.g. a local
    # solana-test-validator for the landing half when the devnet faucet is dry: --rpc-urls http://127.0.0.1:8899,http://localhost:8899
    --rpc-urls) URLS="vec { $(echo "${2:-}" | tr ',' '\n' | sed 's/.*/"&";/' | tr -d '\n') }"; export SOL_RPC="${2%%,*}"; shift;;
    --yes) YES=1;;
    *) echo "refused: unknown argument $1 (see the header of $0)"; exit 2;;
  esac; shift
done
case "$NET" in
  ic) [ ${#WHERE[@]} -gt 0 ] || { echo "refused: --ic needs a subnet: --subnet <id>, --next-to <the buyer's canister id> (the launch subnet), or --subnet-type <type>"; exit 2; }
      CYCLES="${CYCLES:-1500000000000}"; KEY="${KEY:-test_key_1}";;
  local) CYCLES="${CYCLES:-1500000000000}"; KEY="${KEY:-key_1}"  # dfx 0.32 PocketIC keys: ecdsa/schnorr key_1 only (no dfx_test_key; measured)
      "$DFX" ping local >/dev/null 2>&1 || { echo "refused: no local replica; run: cd $HERE && $DFX start --clean --background"; exit 2; };;
  *) echo "refused: --local or --ic"; exit 2;;
esac

echo "== build"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HERE/canister/target}"
( cd canister && PATH="$HOME/.cargo/bin:$PATH" cargo +1.98.1 build --release --locked --target wasm32-unknown-unknown -q ) || { echo "FAIL the canister did not build"; exit 1; }
WASM="$CARGO_TARGET_DIR/wasm32-unknown-unknown/release/rook_sol_sign.wasm"
echo "solsign wasm: $(shasum -a 256 "$WASM" | cut -d' ' -f1)  ($(wc -c < "$WASM" | tr -d ' ') bytes)"
ME=$("$DFX" identity whoami); PRINCIPAL=$("$DFX" identity get-principal)
echo "identity: $ME ($PRINCIPAL); key: $KEY (threshold Ed25519); providers: $URLS; replicated send: $REPLICATED_SEND"

if [ "$NET" = ic ]; then
  BAL=$("$DFX" cycles balance --network ic 2>&1) || { echo "refused: dfx cycles balance failed: $BAL"; exit 2; }
  echo "cycles-ledger balance before: $BAL"
  cat <<EOF
This creates a canister on the IC (${WHERE[*]}) with $CYCLES cycles from $ME's cycles-ledger balance, makes 4 threshold Ed25519
signatures with $KEY (≈ 10–26 B cycles each; measured and printed) and ≈ 60–120 HTTPS outcalls to two Solana DEVNET providers
(≈ 0.1–0.2 B each on 13 nodes), then deletes it and withdraws the rest. Expected spend: the creation fee (500 B on 13 nodes,
~1.3 T on 34) plus ≈ 0.1–0.2 T of signatures and outcalls. No mainnet SOL, no real money: devnet only.
EOF
  if [ $YES != 1 ]; then read -r -p "Type 'rehearse' to go on: " a; [ "$a" = rehearse ] || { echo "stopped; nothing was created"; exit 2; }; fi
fi

CREATED=0
state() { "$DFX" canister status solsign --network "$NET" 2>/dev/null | sed -n 's/^Status: //p'; }
delete_once() {
  "$DFX" canister delete solsign --network "$NET" -y 2>&1 | grep -vE "Attempting to transfer|Not enough margin|dfx is deprecated" | sed 's/^/   /'
  return "${PIPESTATUS[0]}"
}
cleanup() {
  [ $CREATED = 1 ] || return 0
  echo "== clean up: withdraw what is left, delete"
  # 2026-10-09 (measured locally, dfx 0.32): `dfx canister delete` on a RUNNING canister stops it, installs its temporary wallet,
  # withdraws and deletes. Stopping it FIRST (what ic-outcall/run.sh does) makes the withdrawal fail (IC0509/IC0508: the wallet is
  # installed into a stopped canister) and leaves the canister alive running wallet code. So: no explicit stop; on failure,
  # start it, wait until it is Running, and delete again.
  if delete_once; then echo "deleted: the canister is gone"
  else
    for _ in 1 2 3 4 5 6; do [ "$(state)" = Running ] && break; "$DFX" canister start solsign --network "$NET" >/dev/null 2>&1; sleep 5; done
    if delete_once; then echo "deleted on the second try (started, then deleted)"
    else echo "WARNING: the delete FAILED (state: $(state)). By hand: $DFX canister start solsign --network $NET; then $DFX canister delete solsign --network $NET -y"
         echo "         (last resort, forfeits the remaining cycles: $DFX canister stop solsign --network $NET && $DFX canister delete solsign --network $NET -y --no-withdrawal)"
         return 0; fi
  fi
  [ "$NET" = local ] && rm -f .dfx/local/canister_ids.json
  rm -f canister_ids.json
  [ "$NET" = ic ] && echo "cycles-ledger balance after: $("$DFX" cycles balance --network ic 2>&1)"
  CREATED=0
}
trap cleanup EXIT
trap 'exit 130' INT TERM

echo "== create (${WHERE[*]:-local replica}) and install"
if OLD=$("$DFX" canister id solsign --network "$NET" 2>/dev/null); then
  echo "refused: a solsign canister from an earlier run still exists ($OLD, state: $(state)). Delete it first:"
  echo "   $DFX canister start solsign --network $NET; $DFX canister delete solsign --network $NET -y"; exit 2
fi
"$DFX" canister create solsign --network "$NET" --with-cycles "$CYCLES" ${WHERE[@]+"${WHERE[@]}"} 2>&1 | sed 's/^/   /'
"$DFX" canister id solsign --network "$NET" >/dev/null 2>&1 || { echo "FAIL the canister was not created"; exit 1; }
CREATED=1
echo "solsign canister: $("$DFX" canister id solsign --network "$NET")"
INIT="(record { key_name = \"$KEY\"; rpc_urls = $URLS; mode = variant { Nonce }; replicated_send = $REPLICATED_SEND })"
"$DFX" canister install solsign --network "$NET" --mode install --wasm "$WASM" --argument "$INIT" -y 2>&1 | grep -v "dfx is deprecated" | sed 's/^/   /'
[ "${PIPESTATUS[0]}" = 0 ] || { echo "FAIL the install failed"; exit 1; }
[ "$NET" = ic ] && "$DFX" canister status solsign --network ic 2>/dev/null | grep -iE "subnet|balance|module hash" | sed 's/^/   /'

echo "== the scenario (~6–10 min: two finalizations of ~15–30 s each, plus the expiry drill's ~90–150 s wait)"
REPORT="${TMPDIR:-/tmp}/rook-sol-sign-$NET-report.json"
python3 -I "$HERE/drive.py" "$DFX" "$NET" "$REPORT" --fund-wait "$FUND_WAIT"; rc=$?
echo "(the full report: $REPORT)"
exit $rc
