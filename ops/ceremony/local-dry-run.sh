#!/usr/bin/env bash
# The ceremony's install arguments, installed on a LOCAL replica (docs/v9/ceremony-commands.md §0): the buyer's Init made by
# ops/ceremony/mainnet-init.mjs --local-replica-dry-run (key_name swapped to dfx_test_key, NOTHING else: a local replica has no key_1),
# installed through the same chunked path as the ceremony (autobuyer/scripts/chunked-install.sh), then the timelock's Config
# (ops/ceremony/timelock-init.mjs) on the buyer's id, then the handover. Read back: the Ethereum collections, the Solana home's line,
# the operator (an operator-only call as the named operator passes, a stranger's is refused), the timelock's status, the controllers.
#
#   ops/ceremony/local-dry-run.sh <ceremony-input.json>
#
# The input's operator is replaced by a throwaway identity made here (its key in the temp directory, never a dfx identity) so the
# operator check can sign as it; every other field is the input's. The live reads (Ethereum code hashes, genesis, checkpoint) are real.
# The replica runs in a temp directory on port ${PORT:-4963} and is stopped at the end, whatever happens.
set -euo pipefail
export DFX_WARNING=-mainnet_plaintext_identity
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
INPUT=$(cd "$(dirname "${1:?usage: $0 <ceremony-input.json>}")" && pwd)/$(basename "$1")
PORT=${PORT:-4963}
BUYER_WASM=${BUYER_WASM:-$ROOT/autobuyer/canister-rs/target/wasm32-unknown-unknown/release/etch_autobuyer.wasm}
TIMELOCK_WASM=${TIMELOCK_WASM:-$ROOT/autobuyer/timelock-rs/target/wasm32-unknown-unknown/release/etch_timelock.wasm}
[ -f "$BUYER_WASM" ] && [ -f "$TIMELOCK_WASM" ] || { echo "build first: (cd autobuyer/canister-rs && cargo build --locked --release --target wasm32-unknown-unknown), the same in timelock-rs" >&2; exit 2; }
WORK=$(mktemp -d)
cleanup() { (cd "$WORK" && dfx stop >/dev/null 2>&1 || true); rm -rf "$WORK"; }
trap cleanup EXIT
fail() { echo "FAIL $*" >&2; exit 1; }
ok() { echo "ok   $*"; }

cat > "$WORK/dfx.json" <<EOF
{ "version": 1,
  "canisters": {
    "buyer": { "type": "custom", "candid": "$ROOT/autobuyer/canister-rs/etch_autobuyer.did", "wasm": "$WORK/buyer.wasm.gz", "build": [] },
    "timelock": { "type": "custom", "candid": "$ROOT/autobuyer/timelock-rs/etch_timelock.did", "wasm": "$TIMELOCK_WASM", "build": [] } },
  "networks": { "local": { "bind": "127.0.0.1:$PORT", "type": "ephemeral" } } }
EOF
gzip -9 -n -c "$BUYER_WASM" > "$WORK/buyer.wasm.gz"
cd "$WORK"
dfx start --clean --background >"$WORK/replica.log" 2>&1
ok "local replica on 127.0.0.1:$PORT (temp dir $WORK)"

AGENT="node $ROOT/ops/ceremony/local-dry-run-agent.mjs"
OP=$($AGENT new "$WORK/op.json")
$AGENT new "$WORK/stranger.json" >/dev/null
node -e 'const fs=require("fs");const i=JSON.parse(fs.readFileSync(process.argv[1],"utf8"));i.operator=process.argv[2];fs.writeFileSync(process.argv[3],JSON.stringify(i))' "$INPUT" "$OP" "$WORK/input.json"
node "$ROOT/ops/ceremony/mainnet-init.mjs" "$WORK/input.json" --bin "$WORK/buyer-init.bin" --local-replica-dry-run >"$WORK/buyer-init.hex" 2>"$WORK/summary.txt" \
  || { cat "$WORK/summary.txt" >&2; fail "mainnet-init.mjs refused"; }
grep -q "key_name was swapped to dfx_test_key" "$WORK/summary.txt" || fail "the dry-run flag did not say it swapped key_name"
ok "the Init: $(wc -c < "$WORK/buyer-init.bin" | tr -d ' ') bytes, key_name dfx_test_key (swapped, as printed), operator = a throwaway $OP"

dfx canister create buyer >/dev/null 2>&1
BUYER=$(dfx canister id buyer)
"$ROOT/autobuyer/scripts/chunked-install.sh" --network local --mode install "$BUYER" "$WORK/buyer.wasm.gz" "$WORK/buyer-init.bin" >"$WORK/install.log" 2>&1 \
  || { tail -20 "$WORK/install.log" >&2; fail "the buyer's install"; }
grep -q "^MATCH" "$WORK/install.log" || fail "the module hash after the chunked install"
ok "buyer $BUYER installed (chunked, module hash MATCH): the canister accepted the Init"

dfx canister call buyer log --query | grep -q '"installed"' && ok "buyer log: installed" || fail "no 'installed' in the buyer's log"
cfg=$(dfx canister call buyer eth_nft_config --query)
n=$(grep -o 'module = "books"' <<<"$cfg" | wc -l | tr -d ' ')
[ "$n" = 11 ] || fail "eth_nft_config names $n books collections, not 11"
for c in 0xbc4ca0eda7647a8ab7c2061c2e118a18a936f13d 0x5af0d9827e0c53e4799bb226655a1de152a425a5 0x0000ec93127baa929e58e97dd0095a2bfb38ec1d; do
  grep -q "$c" <<<"$cfg" || fail "eth_nft_config lacks $c"
done
grep -q "relay_max_fee_bps = 300" <<<"$cfg" && grep -q "chain_id = 1 " <<<"$cfg" || fail "eth_nft_config: chain 1 / relay 300"
ok "eth_nft_config: chain 1, 11 Ethereum collections, wrapper = module = \"books\", relay 300 bps"
dfx canister call buyer sol_line --query >"$WORK/sol_line.txt" && ok "sol_line: $(head -c 120 "$WORK/sol_line.txt" | tr -s ' \n' ' ')"
dfx canister call buyer coins_collections --query | head -c 200 | tr -s ' \n' ' '; echo
dfx canister call buyer paused --query | grep -q false && ok "paused: false"
dfx canister call buyer custody_caps --query >"$WORK/caps.txt" 2>/dev/null
echo "ok   custody_caps (stage-0 defaults, before any pin): $(grep -o 'collection = "[^"]*"' "$WORK/caps.txt" | wc -l | tr -d ' ') collections named, $(grep -o 'global_max_items = [0-9_]*' "$WORK/caps.txt")"
dfx canister call buyer cycles --query | tr -s ' \n' ' '; echo

r=$($AGENT sol_address "$WORK/op.json" "http://127.0.0.1:$PORT" "$BUYER")
case "$r" in ok*) ok "operator $OP: sol_address (only_operator) passed: $r" ;; *) grep -q "neither the operator" <<<"$r" && fail "the named operator was refused: $r"; ok "operator $OP passed the guard (the call itself: $r)" ;; esac
r=$($AGENT sol_address "$WORK/stranger.json" "http://127.0.0.1:$PORT" "$BUYER")
case "$r" in rejected*) ok "a stranger's sol_address is refused: ${r:0:160}" ;; *) fail "a stranger called an operator method: $r" ;; esac

dfx canister create timelock >/dev/null 2>&1
TL=$(dfx canister id timelock)
node "$ROOT/ops/ceremony/timelock-init.mjs" "$BUYER" >"$WORK/timelock-init.hex" 2>"$WORK/timelock-summary.txt" || { cat "$WORK/timelock-summary.txt" >&2; fail "timelock-init.mjs"; }
dfx canister install timelock --mode install --wasm "$TIMELOCK_WASM" --argument-type raw --argument "$(cat "$WORK/timelock-init.hex")" >/dev/null 2>&1 || fail "the timelock's install"
st=$(dfx canister call --identity anonymous timelock status --query)
grep -q "target = principal \"$BUYER\"" <<<"$st" && grep -q "delay_secs = 259_200" <<<"$st" && grep -q "veto_threshold = 2" <<<"$st" \
  && grep -q "rehearsal_build = false" <<<"$st" && grep -q 4423w-dirhd <<<"$st" || { echo "$st" >&2; fail "the timelock's status"; }
ok "timelock $TL: target $BUYER, p1 p2 / g1 g2 g3, veto 2, delay 259,200 s, rehearsal_build false"

# the exact production bytes too (key_1 untouched): dfx 0.32's local replica serves ONLY key_1 (its keys: ecdsa / schnorr Bip340 /
# schnorr Ed25519 / vetkd, all named key_1; "dfx_test_key" is unknown to it), so the argument the ceremony installs installs here as it is,
# and the key it names answers. A reinstall, before the handover, as canister-mainnet.md's pre-handover reinstall would be.
node "$ROOT/ops/ceremony/mainnet-init.mjs" "$WORK/input.json" --bin "$WORK/buyer-init-prod.bin" >"$WORK/buyer-init-prod.hex" 2>/dev/null || fail "mainnet-init.mjs (production bytes)"
node --input-type=module -e '
  import { readFileSync } from "node:fs";
  const { decodeInit } = await import(process.argv[1] + "/monitor/src/candid.mjs");
  const { canonical } = await import(process.argv[1] + "/ops/ceremony/mainnet-init.mjs");
  const [a, b] = [process.argv[2], process.argv[3]].map((f) => decodeInit(readFileSync(f, "utf8").trim()).init);
  if (a.key_name !== "dfx_test_key" || b.key_name !== "key_1" || canonical({ ...a, key_name: "key_1" }) !== canonical(b)) { console.error("the two arguments differ beyond key_name"); process.exit(1); }
' "$ROOT" "$WORK/buyer-init.hex" "$WORK/buyer-init-prod.hex" || fail "the dry-run argument differs from the production one beyond key_name"
ok "the dry-run argument and the production argument differ in key_name only"
"$ROOT/autobuyer/scripts/chunked-install.sh" --network local --mode reinstall "$BUYER" "$WORK/buyer.wasm.gz" "$WORK/buyer-init-prod.bin" >"$WORK/install-prod.log" 2>&1 \
  || { tail -20 "$WORK/install-prod.log" >&2; fail "the reinstall with the exact production argument"; }
grep -q "^MATCH" "$WORK/install-prod.log" || fail "the module hash after the production reinstall"
r=$($AGENT sol_address "$WORK/op.json" "http://127.0.0.1:$PORT" "$BUYER")
case "$r" in *'"Ok"'*) ok "the EXACT production argument (key_1) installs too, and K_main derives with key_1: $r" ;; *) fail "the production argument's key on the local replica: $r" ;; esac
dfx canister call buyer eth_nft_config --query | grep -o 'module = "books"' | wc -l | grep -q 11 || fail "eth_nft_config after the production reinstall"
ok "after the production reinstall: eth_nft_config still names the 11 books collections"

dfx canister update-settings buyer --set-controller "$TL" --yes >"$WORK/handover.log" 2>&1 || { cat "$WORK/handover.log" >&2; fail "the handover"; }
ctl=$(dfx canister info buyer | awk '/Controllers/ {print $2, $3}')
[ "$(echo $ctl)" = "$TL" ] || fail "controllers after the handover: $ctl"
ok "handover: the buyer's only controller is the timelock ($TL)"
r=$($AGENT sol_address "$WORK/op.json" "http://127.0.0.1:$PORT" "$BUYER")
case "$r" in rejected*neither*) fail "the operator lost its methods at the handover: $r" ;; *) ok "after the handover the operator still operates ($r)" ;; esac
if dfx canister call buyer coins_set_auto_open '(false)' >/dev/null 2>&1; then fail "the former controller still operates"; else ok "the former controller (default identity) is refused after the handover"; fi
echo "DRY RUN PASSED (the replica is stopped and $WORK removed on exit)"
