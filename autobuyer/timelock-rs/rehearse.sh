#!/usr/bin/env bash
# The timelock, rehearsed on the LOCAL replica only (docs/v2/timelock-controller.md, "The rehearsal"). Never mainnet: every dfx
# call here is --network local. Creates two canisters of its own, `autobuyer_timelock` and `timelock`, in this directory's dfx
# project; it touches no other canister.
#
# One principal plays proposer, guardian and operator (the rehearsal build allows proposer == guardian; a production build
# refuses it). Separation of those roles is proved by the unit tests; this proves the IC does what the design says it does.
set -euo pipefail
export PATH="$HOME/.cargo/bin:$HOME/Library/Application Support/org.dfinity.dfx/bin:$PATH"
export DFX_WARNING=-mainnet_plaintext_identity
cd "$(dirname "$0")"
HERE=$(pwd)
# the reproducible build's own output, so the deployed hashes can be checked against source at the end
BUYER_WASM=../build/buyer.wasm
TL_WASM=../build/timelock-rehearsal.wasm
DELAY=30
dfx() { local out rc; out=$(command dfx "$@" --network local 2>&1) && rc=0 || rc=$?; grep -v "^WARNING" <<<"$out" || true; return $rc; }
ok() { printf '   \033[1m✓ %s\033[0m\n' "$*"; }
fail() { printf '   ✗ %s\n' "$*"; exit 1; }
expect() { # expect <what> <needle> <output>
  if grep -q -- "$2" <<<"$3"; then ok "$1"; else printf '%s\n' "$3" | tail -5; fail "$1 (expected: $2)"; fi
}
# execute proposal <id> as the anonymous principal, retrying until the delay has passed; prints the final answer
execute_when_due() {
  local out
  for _ in $(seq 1 40); do
    out=$(command dfx canister call timelock execute "($1 : nat64)" --identity anonymous --network local 2>&1 | grep -v "^WARNING" || true)
    grep -q "may run in" <<<"$out" || { printf '%s' "$out"; return; }
    python3 -c "import time; time.sleep(3)"
  done
  printf '%s' "$out"
}
modhash() { dfx canister info "$1" | awk '/Module hash/ {print $3}'; }
controllers() { dfx canister info "$1" | awk -F': ' '/Controllers/ {print $2}'; }

echo "0. the reproducible build: the buyer, and the timelock with the rehearsal feature (10 s floor; it changes the module hash)"
../scripts/reproducible-build.sh native | sed 's/^/   /'
ME=$(command dfx identity get-principal 2>/dev/null)
echo "   me (proposer, guardian, operator): $ME"

echo "1. create and install both, fresh"
for c in autobuyer_timelock timelock; do dfx canister create "$c" >/dev/null || true; done
BUYER=$(dfx canister id autobuyer_timelock | tail -1); TL=$(dfx canister id timelock | tail -1)
echo "   buyer    $BUYER"; echo "   timelock $TL"
ARGHEX=$(cargo run -q --example encode_init -- "$ME")
INIT="(record { chain_id = 46630 : nat64; vault = \"0x0000000000000000000000000000000000000001\"; rpc_urls = vec { \"https://rpc.testnet.chain.robinhood.com\"; \"https://explorer.testnet.chain.robinhood.com/api/eth-rpc\" }; btc_api_urls = vec { \"https://mempool.space/signet/api\"; \"https://mutinynet.com/api\" }; key_name = \"key_1\"; accept_production_key_name = true; bridge_allowlist = vec {}; btc_network = \"signet\"; operator = opt principal \"$ME\" })"
dfx canister install autobuyer_timelock --mode reinstall --yes --wasm "$BUYER_WASM" --argument "$INIT" >/dev/null
dfx canister install timelock --mode reinstall --yes --wasm "$TL_WASM" \
  --argument "(record { target = principal \"$BUYER\"; roles = record { proposers = vec { principal \"$ME\" }; guardians = vec { principal \"$ME\" }; veto_threshold = 1 : nat8 }; delay_secs = $DELAY : nat64 })" >/dev/null
BEFORE=$(modhash autobuyer_timelock)
echo "   buyer module $BEFORE"
expect "the installed buyer is this source's reproducible build" "MATCH" "$(../scripts/reproducible-build.sh verify "$BUYER" buyer local)"
expect "the timelock refuses a delay under its floor" "below this build's floor" \
  "$(command dfx canister install timelock --mode reinstall --yes --wasm "$TL_WASM" --network local --argument "(record { target = principal \"$BUYER\"; roles = record { proposers = vec { principal \"$ME\" }; guardians = vec { principal \"$ME\" }; veto_threshold = 1 : nat8 }; delay_secs = 1 : nat64 })" 2>&1 || true)"
dfx canister install timelock --mode reinstall --yes --wasm "$TL_WASM" \
  --argument "(record { target = principal \"$BUYER\"; roles = record { proposers = vec { principal \"$ME\" }; guardians = vec { principal \"$ME\" }; veto_threshold = 1 : nat8 }; delay_secs = $DELAY : nat64 })" >/dev/null

# the same code gzipped: a different module hash, so neither dfx nor the IC can treat an install of it as a no-op. (dfx skips an
# install whose hash is already there and still prints "Upgraded code", which is how a check that asks the IC nothing passes.)
GZ=$(mktemp); gzip -9 -n -c "$BUYER_WASM" > "$GZ"
WANT=$(shasum -a 256 "$GZ" | cut -d' ' -f1)
[ "0x$WANT" != "$BEFORE" ] || fail "the gzipped module has the same hash as the installed one"

echo "2. the handover: the timelock becomes the buyer's ONLY controller"
dfx canister update-settings autobuyer_timelock --set-controller "$TL" --yes >/dev/null
[ "$(controllers autobuyer_timelock)" = "$TL" ] && ok "controllers: $TL, and nobody else" || fail "controllers are $(controllers autobuyer_timelock)"
expect "I can no longer upgrade it directly" "not.*controller\|Only controllers\|is not authorized\|Unauthorized" \
  "$(command dfx canister install autobuyer_timelock --mode upgrade --yes --wasm "$GZ" --argument "$INIT" --network local 2>&1 || true)"
[ "$(modhash autobuyer_timelock)" = "$BEFORE" ] && ok "and the module is unchanged" || fail "a direct install changed the module"

echo "3. operating is not controlling: the operator still operates, can pause, cannot resume"
expect "the operator reads its EVM address (an operator method)" "Ok = \"0x" "$(dfx canister call autobuyer_timelock evm_address)"
dfx canister call autobuyer_timelock set_paused '(true)' >/dev/null
expect "the operator paused it" "(true)" "$(dfx canister call autobuyer_timelock paused)"
expect "the operator cannot resume it" "not a controller" "$(dfx canister call autobuyer_timelock set_paused '(false)' || true)"
expect "a stranger cannot even pause it" "neither the operator nor a controller" \
  "$(command dfx canister call autobuyer_timelock set_paused '(true)' --identity anonymous --network local 2>&1 || true)"
expect "the guardian resumes it through the timelock" "Ok" "$(dfx canister call timelock resume_target)"
expect "and it is running" "(false)" "$(dfx canister call autobuyer_timelock paused)"
dfx canister call timelock pause_target >/dev/null
expect "paused again by the guardian, to see the pause survive the upgrade" "(true)" "$(dfx canister call autobuyer_timelock paused)"

echo "4. a proposal: the same code, gzipped, so the module hash visibly changes"
blobfile() { python3 - "$1" "$2" > "$3" <<'PY'
import sys
w=open(sys.argv[1],'rb').read(); a=bytes.fromhex(sys.argv[2])
esc=lambda b: ''.join('\\%02x'%x for x in b)
print('(blob "%s", blob "%s")' % (esc(w), esc(a)))
PY
}
ARGS=$(mktemp); blobfile "$GZ" "$ARGHEX" "$ARGS"
EMPTY=$(mktemp); blobfile "$GZ" "" "$EMPTY"
expect "a stranger cannot propose" "is not a proposer" \
  "$(command dfx canister call timelock propose_upgrade --argument-file "$ARGS" --identity anonymous --network local 2>&1 || true)"
P=$(dfx canister call timelock propose_upgrade --argument-file "$ARGS")
expect "proposal 1 published with the module's hash" "$WANT" "$P"
expect "status() shows it to anyone, pending" "$WANT" "$(command dfx canister call timelock status --identity anonymous --network local 2>&1)"
expect "executing it before the delay is refused" "may run in" "$(dfx canister call timelock execute '(1 : nat64)')"
expect "a stranger cannot veto it" "is not a guardian" \
  "$(command dfx canister call timelock veto '(1 : nat64)' --identity anonymous --network local 2>&1 || true)"
expect "the guardian vetoes it (a threshold of one guardian, in this single-identity rehearsal)" "Ok = true" "$(dfx canister call timelock veto '(1 : nat64)')"
[ "$(modhash autobuyer_timelock)" = "$BEFORE" ] && ok "the buyer is untouched" || fail "the buyer changed"

echo "5. a proposal that will fail: no install argument, so the buyer's post_upgrade traps"
expect "proposal 2 published" "Ok" "$(dfx canister call timelock propose_upgrade --argument-file "$EMPTY")"
echo "   waiting out the ${DELAY} s delay, then executing as the anonymous principal"
expect "anyone may execute it; it fails, atomically, and is recorded" "failed and was cleared" "$(execute_when_due 2)"
[ "$(modhash autobuyer_timelock)" = "$BEFORE" ] && ok "the buyer is untouched by a failed upgrade" || fail "a failed upgrade changed the buyer"

echo "6. the real one: proposed, left alone for the delay, executed by a stranger"
expect "proposal 3 published" "$WANT" "$(dfx canister call timelock propose_upgrade --argument-file "$ARGS")"
echo "   waiting out the ${DELAY} s delay, then executing as the anonymous principal"
expect "the anonymous principal executes it" "proposal 3 executed" "$(execute_when_due 3)"
AFTER=$(modhash autobuyer_timelock)
echo "   buyer module now $AFTER"
[ "$AFTER" = "0x$WANT" ] && ok "the buyer's module hash is the proposed hash" || fail "module $AFTER, proposed 0x$WANT"
expect "its state came back" "upgraded: state restored" "$(dfx canister call autobuyer_timelock log)"
expect "and the guardian's pause survived the upgrade" "(true)" "$(dfx canister call autobuyer_timelock paused)"
[ "$(controllers autobuyer_timelock)" = "$TL" ] && ok "the timelock is still its only controller" || fail "controllers changed"
expect "history records all of it" "Executed" "$(dfx canister call timelock history)"

echo "7. and the way out: a controller change waits the same delay (it is how the buyer would move to a new timelock)"
expect "proposal 4: hand the buyer back to me" "controllers" "$(dfx canister call timelock propose_controllers "(vec { principal \"$ME\" })")"
[ "$(controllers autobuyer_timelock)" = "$TL" ] && ok "nothing has changed yet" || fail "controllers changed before the delay"
expect "executed by the anonymous principal after the delay" "proposal 4 executed" "$(execute_when_due 4)"
[ "$(controllers autobuyer_timelock)" = "$ME" ] && ok "controllers: $ME (so this rehearsal can run again)" || fail "controllers are $(controllers autobuyer_timelock)"

expect "and the timelock running it is this source's rehearsal build" "MATCH" "$(../scripts/reproducible-build.sh verify "$TL" timelock-rehearsal local)"

echo
echo "PASS: buyer $BUYER is controlled by timelock $TL; nothing reached it except through a published, delayed proposal."
rm -f "$GZ" "$ARGS" "$EMPTY"
