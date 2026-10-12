#!/usr/bin/env bash
# Rook sim, Bitcoin half: one simulated frog purchase on a throwaway REGTEST bitcoind + ord, the canister's own frog flow driving
# it (ops/rehearsal/sim/README.md). Nothing leaves 127.0.0.1; no key here is anyone's; no real money.
#   usage: ops/rehearsal/sim/run.sh            (needs: bitcoind/bitcoin-cli 31+, ord 0.29, node 22, cargo; `npm ci` in ops/)
set -euo pipefail
cd "$(dirname "$0")/../../.."
ROOT=$(pwd)
D="${SIM_DIR:-${TMPDIR:-/tmp}/rook-sim}"; RPCPORT="${SIM_RPC_PORT:-18743}"; ORDPORT="${SIM_ORD_PORT:-18780}"
export SIM_RPC="http://127.0.0.1:$RPCPORT" SIM_RPC_USER=sim SIM_RPC_PASS=sim SIM_ORD="http://127.0.0.1:$ORDPORT"
for t in bitcoind bitcoin-cli ord node cargo; do command -v $t >/dev/null || { echo "missing: $t (brew install bitcoin ord)"; exit 1; }; done
cli() { bitcoin-cli -regtest -datadir="$D/btc" -rpcuser=sim -rpcpassword=sim -rpcport=$RPCPORT "$@"; }
ORD=(ord --regtest --bitcoin-rpc-url "$SIM_RPC" --bitcoin-rpc-username sim --bitcoin-rpc-password sim --data-dir "$D/ord" --index-runes)
ORDPID=""
cleanup() { [ -z "$ORDPID" ] || kill "$ORDPID" 2>/dev/null || true; cli stop >/dev/null 2>&1 || true; }
trap cleanup EXIT
cli stop >/dev/null 2>&1 && sleep 2 || true
rm -rf "$D"; mkdir -p "$D/btc" "$D/ord"

echo "== regtest bitcoind ($D)"
bitcoind -regtest -datadir="$D/btc" -daemon -txindex=1 -server=1 -rpcuser=sim -rpcpassword=sim -rpcport=$RPCPORT -listen=0 \
  -fallbackfee=0.0001 >/dev/null
for _ in $(seq 1 60); do cli getblockcount >/dev/null 2>&1 && break; sleep 0.5; done
"${ORD[@]}" server --http-port $ORDPORT >"$D/ord-server.log" 2>&1 & ORDPID=$!
for _ in $(seq 1 60); do curl -sf "$SIM_ORD/blockheight" >/dev/null && break; sleep 0.5; done
synced() { for _ in $(seq 1 120); do [ "$(curl -s $SIM_ORD/blockheight)" = "$(cli getblockcount)" ] && return 0; sleep 0.25; done; return 1; }

echo "== wallets: the faucet (bitcoind) and the seller (ord's wallet)"
cli -named createwallet wallet_name=faucet >/dev/null
"${ORD[@]}" wallet --server-url "$SIM_ORD" create >/dev/null
SELLER=$("${ORD[@]}" wallet --server-url "$SIM_ORD" receive | python3 -c 'import json,sys;print(json.load(sys.stdin)["addresses"][0])')
cli generatetoaddress 1 "$SELLER" >/dev/null
cli generatetoaddress 110 "$(cli -rpcwallet=faucet getnewaddress)" >/dev/null
synced

echo "== the seller inscribes a frog (ord, regtest)"
printf '<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><rect width="64" height="64" fill="#3a7"/><text x="8" y="36">frog</text></svg>' >"$D/frog.svg"
"${ORD[@]}" wallet --server-url "$SIM_ORD" inscribe --fee-rate 2 --file "$D/frog.svg" >"$D/inscribe.json"
cli generatetoaddress 1 "$(cli -rpcwallet=faucet getnewaddress)" >/dev/null
synced
export SIM_INSCRIPTION=$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["inscriptions"][0]["id"])' "$D/inscribe.json")
echo "   $SIM_INSCRIPTION (seller $SELLER)"

echo "== the harness (the canister's frog flow, native)"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$D/../rook-sim-target}"
( cd ops/rehearsal/sim/harness && cargo build -q 2>&1 | grep -v '^warning' || true )
export SIM_MARKET="$ROOT/ops/rehearsal/sim/market.mjs" SIM_OPS_DIR="$ROOT/ops" SIM_OUT="$D"
"$CARGO_TARGET_DIR/debug/rook-sim-harness" 2>&1 | tee "$D/sim.log"
