#!/usr/bin/env bash
# The rehearsal's SECOND path (README.md "Path B"): the REAL buyer.wasm in a throwaway canister, asked for the one Ethereum read path
# it exposes to its controller: nft_chain_status(1) = nftnet::pin_on (eth_blockNumber via head_bucket, then eth_getBlockByNumber(pin,
# false) via nft_block_at at a 16,000-byte cap, every provider agreeing) and two json_rpc_norm state reads at that pin (the throwaway
# canister's own Ethereum balance and nonce). The buyer exposes no endpoint for the sale scan's log and receipt reads: those are
# probe.wasm's (run.sh, "Path A"), whose transform bodies are held byte for byte to the buyer's.
#
#   ops/rehearsal/ic-outcall/buyer-throwaway.sh --wasm <buyer.wasm from the reproducible build> --network ic (--subnet <id> | --next-to <id>) [--yes]
#   ops/rehearsal/ic-outcall/buyer-throwaway.sh --wasm <buyer.wasm> --network local
#
# The throwaway Init (buyer-init.mjs) configures nothing custodial: no wrapper, no caps, no eth_nft, a signet Bitcoin network, and the
# test key (the local replica's key_1 locally, test_key_1 on the IC: a public-key read is free and nothing is ever signed). Always deleted after.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
DFX="${DFX:-$HOME/Library/Application Support/org.dfinity.dfx/bin/dfx}"; [ -x "$DFX" ] || DFX="$(command -v dfx || true)"
[ -x "$DFX" ] || { echo "refused: dfx not found (set DFX)"; exit 2; }
export DFX_WARNING=-mainnet_plaintext_identity
NET=""; WHERE=(); WASM=""; YES=0; CYCLES=""; TRIES=3
while [ $# -gt 0 ]; do
  case "$1" in
    --network) NET="${2:-}"; shift;;
    --subnet) WHERE=(--subnet "${2:-}"); shift;;
    --next-to) WHERE=(--next-to "${2:-}"); shift;;
    --subnet-type) WHERE=(--subnet-type "${2:-}"); shift;;
    --wasm) WASM="${2:-}"; shift;;
    --cycles) CYCLES="${2:-}"; shift;;
    --tries) TRIES="${2:-}"; shift;;
    --yes) YES=1;;
    *) echo "refused: unknown argument $1"; exit 2;;
  esac; shift
done
[ -f "$WASM" ] || { echo "refused: --wasm <buyer.wasm> (the reproducible build's, autobuyer/Dockerfile.build)"; exit 2; }
case "$NET" in
  ic) [ ${#WHERE[@]} -gt 0 ] || { echo "refused: --network ic needs --subnet <launch subnet id> or --next-to <the buyer's id>"; exit 2; }; KEY=test_key_1;;
  local) KEY=key_1;;   # dfx 0.32's local replica (PocketIC) holds key_1 only; a local key signs nothing of value, and nothing is signed
  *) echo "refused: --network ic or --network local"; exit 2;;
esac
CYCLES="${CYCLES:-2000000000000}"
# the script works in a temporary directory: a relative --wasm is made absolute first (a run on 2026-10-09 failed its install on this)
[ -f "$WASM" ] || { echo "refused: --wasm $WASM is not a file"; exit 2; }
WASM="$(cd "$(dirname "$WASM")" && pwd)/$(basename "$WASM")"
WORK=$(mktemp -d "${TMPDIR:-/tmp}/rook-buyer-throwaway.XXXXXX")
cat > "$WORK/dfx.json" <<EOF
{"version":1,"canisters":{"buyer":{"type":"custom","candid":"$HERE/../../../autobuyer/canister-rs/etch_autobuyer.did","wasm":"$WASM"}},
 "networks":{"local":{"bind":"127.0.0.1:4955","type":"ephemeral"}}}
EOF
ARG=$(node "$HERE/buyer-init.mjs" "$KEY") || { echo "FAIL the throwaway Init could not be built (monitor/node_modules? run npm ci in monitor/)"; exit 1; }
echo "buyer.wasm: $(shasum -a 256 "$WASM" | cut -d' ' -f1); identity $("$DFX" identity whoami); key $KEY"
if [ "$NET" = ic ] && [ $YES != 1 ]; then
  echo "This creates a canister on the IC (${WHERE[*]}) with $CYCLES cycles, installs the buyer with a throwaway Init, reads, deletes it."
  read -r -p "Type 'rehearse' to go on: " a; [ "$a" = rehearse ] || { echo "stopped; nothing was created"; exit 2; }
fi
cd "$WORK"
CREATED=0
cleanup() {
  [ $CREATED = 1 ] || { rm -rf "$WORK"; return 0; }
  echo "== clean up: stop, withdraw, delete"
  "$DFX" canister stop buyer --network "$NET" >/dev/null 2>&1
  "$DFX" canister delete buyer --network "$NET" -y 2>&1 | grep -vE "Attempting to transfer|Not enough margin|deprecated" | sed 's/^/   /'
  # 2026-10-09: dfx's withdrawal installed its temporary wallet, then could not call it on the stopped canister (IC0508), and the
  # canister stayed with its cycles. Started again, the same delete withdraws and deletes: retry once that way.
  if "$DFX" canister id buyer --network "$NET" >/dev/null 2>&1 && "$DFX" canister status buyer --network "$NET" >/dev/null 2>&1; then
    echo "   the delete did not finish: starting the canister and deleting again (dfx withdraws through its wallet first)"
    "$DFX" canister start buyer --network "$NET" >/dev/null 2>&1
    "$DFX" canister delete buyer --network "$NET" -y 2>&1 | grep -vE "Attempting to transfer|Not enough margin|deprecated" | sed 's/^/   /'
  fi
  "$DFX" canister id buyer --network "$NET" >/dev/null 2>&1 && echo "WARNING: the throwaway buyer still exists: delete it by hand from $WORK" || rm -rf "$WORK"
  CREATED=0
}
trap cleanup EXIT
trap 'exit 130' INT TERM
"$DFX" canister create buyer --network "$NET" --with-cycles "$CYCLES" ${WHERE[@]+"${WHERE[@]}"} 2>&1 | grep -v deprecated | sed 's/^/   /'
"$DFX" canister id buyer --network "$NET" >/dev/null 2>&1 || { echo "FAIL not created"; exit 1; }
CREATED=1
"$DFX" canister install buyer --network "$NET" --mode install --wasm "$WASM" --argument-type raw --argument "$ARG" -y 2>&1 | grep -v deprecated | sed 's/^/   /'
ok=0
for i in $(seq 1 "$TRIES"); do
  out=$("$DFX" canister call buyer nft_chain_status '(1 : nat64)' --network "$NET" 2>/dev/null | tr -s ' \n' ' ')
  if echo "$out" | grep -q "variant { Ok"; then ok=$((ok + 1)); echo "PASS read $i: $(echo "$out" | grep -oE '(pin|nonce|balance_wei) = [^;]*' | tr '\n' ' ')"
  else echo "FAIL read $i: $out"; fi
done
echo "the buyer's own Ethereum pin read (nftnet::pin_on) and two pinned reads: $ok of $TRIES passed"
[ "$ok" = "$TRIES" ] && { echo "BUYER PATH: PASS"; exit 0; }
echo "BUYER PATH: FAIL"; exit 1
