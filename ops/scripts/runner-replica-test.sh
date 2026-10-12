#!/usr/bin/env bash
# docs/v6/p1-services.md §4 ("Replica"). The operator runner (ops/src/runner.ts) against THIS worktree's canister on a LOCAL dfx
# replica: the runner's identity is a freshly generated secp256k1 key installed as the canister's `operator`, it ticks and gets
# the canister's answer; another fresh identity is refused by the operator check. Nothing reaches any chain: every rpc_urls and
# btc_api_urls entry is an unreachable address on this machine, so every outcall the tick makes fails, by design.
#
#   ops/scripts/runner-replica-test.sh          (needs dfx, and the wasm: cd autobuyer/canister-rs && cargo build --release --target wasm32-unknown-unknown)
#
# It runs its own replica on port 4963 (REPLICA_PORT) from a scratch project directory, so it does not touch another replica on
# 4943, and the deployer is the local `default` identity, used for this local replica only.
set -uo pipefail
export PATH="$HOME/Library/Application Support/org.dfinity.dfx/bin:$PATH"
cd "$(dirname "$0")/../.."
ROOT=$(pwd)
PORT="${REPLICA_PORT:-4963}"; HOST="http://127.0.0.1:$PORT"
WASM="$ROOT/autobuyer/canister-rs/target/wasm32-unknown-unknown/release/etch_autobuyer.wasm"
DID="$ROOT/autobuyer/canister-rs/etch_autobuyer.did"
[ -f "$WASM" ] || { echo "FAIL no wasm at $WASM: build it first"; exit 1; }
WORK=$(mktemp -d "${TMPDIR:-/tmp}/runner-replica.XXXXXX")
fail=0
ok() { echo "ok   $*"; }; bad() { echo "FAIL $*"; fail=1; }
tidy() { (cd "$WORK" && dfx stop >/dev/null 2>&1); rm -rf "$WORK"; }
trap tidy EXIT

# two fresh identities, made here, for a local replica: the operator's and a stranger's (their PEMs never leave $WORK)
node -e '
const { generateKeyPairSync } = require("node:crypto"); const fs = require("node:fs");
for (const n of ["operator", "stranger"]) fs.writeFileSync(process.argv[1] + "/" + n + ".pem", generateKeyPairSync("ec", { namedCurve: "secp256k1" }).privateKey.export({ format: "pem", type: "sec1" }));
' "$WORK"
principal() { (cd ops && OPERATOR_IDENTITY_PEM="$(cat "$1")" node --import tsx -e 'import("./src/identity.ts").then((m) => console.log(m.operatorIdentity(process.env).getPrincipal().toText()))'); }
OPERATOR=$(principal "$WORK/operator.pem"); STRANGER=$(principal "$WORK/stranger.pem")
echo "     the runner's identity: $OPERATOR (a fresh key; the stranger: $STRANGER)"

cat > "$WORK/dfx.json" <<EOF
{ "canisters": { "autobuyer": { "type": "custom", "candid": "$DID", "wasm": "$WASM", "build": "true" } },
  "networks": { "local": { "bind": "127.0.0.1:$PORT", "type": "ephemeral" } } }
EOF
cd "$WORK"
dfx start --background --clean >/dev/null 2>&1 || { echo "FAIL the replica did not start"; exit 1; }
ARG="(record { chain_id = 9466301 : nat64; vault = \"0x0000000000000000000000000000000000000001\";
  rpc_urls = vec { \"https://127.0.0.1:1\"; \"https://127.0.0.2:1\" }; btc_api_urls = vec { \"https://127.0.0.1:2\"; \"https://127.0.0.2:2\" };
  key_name = \"key_1\"; accept_production_key_name = true; bridge_allowlist = vec {}; btc_network = \"signet\";
  operator = opt principal \"$OPERATOR\"; wrapper = null; evm_chains = null; custody_caps = null; outflow_limits = null })"
dfx canister create autobuyer --identity default >/dev/null 2>&1 && dfx canister install autobuyer --identity default --wasm "$WASM" --argument "$ARG" >/dev/null 2>&1 \
  && ok "installed this worktree's canister with operator = the runner's identity" || { bad "install"; exit 1; }
CID=$(dfx canister id autobuyer)
cd "$ROOT/ops"

OUT=$(CANISTER_ID=$CID IC_HOST=$HOST OPERATOR_IDENTITY_PEM="$(cat "$WORK/operator.pem")" RUNNER_TICK_EVERY_S=60 node --import tsx src/runner.ts --once 2>&1)
echo "$OUT" | grep -q "\"principal\":\"$OPERATOR\"" && ok "the runner logs its principal" || bad "no principal in the start line"
echo "$OUT" | grep -q '"event":"paused","paused":false' && ok "it read paused() = false" || bad "paused() was not read"
if echo "$OUT" | grep -q '"event":"tick-rejected"'; then bad "the operator's tick was rejected: $(echo "$OUT" | grep tick-rejected | head -1)"
else ok "tick() answered the operator: $(echo "$OUT" | grep -E '"event":"tick-(err|line)"' | head -1 | sed 's/.*"event"/"event"/' | cut -c1-200)"; fi
! echo "$OUT" | grep -q "PRIVATE KEY" && ! echo "$OUT" | grep -qF "$(sed -n 2p "$WORK/operator.pem" | cut -c1-40)" && ok "the PEM appears in no line" || bad "the PEM was printed"

OUT=$(CANISTER_ID=$CID IC_HOST=$HOST OPERATOR_IDENTITY_PEM="$(cat "$WORK/stranger.pem")" RUNNER_TICK_EVERY_S=60 node --import tsx src/runner.ts --once 2>&1); CODE=$?
[ $CODE = 1 ] && echo "$OUT" | grep -q '"event":"tick-rejected"' && echo "$OUT" | grep -q "neither the operator nor a controller" \
  && ok "a stranger's tick is refused by the canister's operator check" || bad "the stranger's tick: exit $CODE $(echo "$OUT" | tail -2)"
OUT=$(CANISTER_ID=$CID IC_HOST=$HOST node --import tsx src/runner.ts --once 2>&1); CODE=$?
[ $CODE = 2 ] && echo "$OUT" | grep -q "OPERATOR_IDENTITY_PEM is not set" && ok "no identity: refused at start" || bad "no identity: exit $CODE"

printf '\n%s\n' "$([ $fail = 0 ] && echo 'RUNNER REPLICA TEST: ALL PASS' || echo 'RUNNER REPLICA TEST: FAILURES ABOVE')"
exit $fail
