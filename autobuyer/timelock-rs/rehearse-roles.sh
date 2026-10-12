#!/usr/bin/env bash
# docs/v2/trust-points.md §3 (and §2), rehearsed on the LOCAL replica only. Never mainnet: every dfx call is --network local.
# Creates two canisters of its own, `autobuyer_trust` and `timelock_trust`, in this directory's dfx project; touches no other.
# Uses six LOCAL TEST identities (trust-p1..p3, trust-g1..g3), plaintext, created for this and nothing else — never a real key.
#
# The roles are production-shaped (2 proposers, 3 guardians, veto 2 of 3); only the delay is the rehearsal build's (15 s).
set -euo pipefail
export PATH="$HOME/.cargo/bin:$HOME/Library/Application Support/org.dfinity.dfx/bin:$PATH"
export DFX_WARNING=-mainnet_plaintext_identity
cd "$(dirname "$0")"
DELAY=15
ok() { printf '   \033[1m✓ %s\033[0m\n' "$*"; }
fail() { printf '   ✗ %s\n' "$*"; exit 1; }
expect() { if grep -q -- "$2" <<<"$3"; then ok "$1"; else printf '%s\n' "$3" | tail -6; fail "$1 (expected: $2)"; fi; }
d() { command dfx "$@" --network local 2>&1 | grep -v "^WARNING" || true; }
as() { local who=$1; shift; d canister call --identity "$who" timelock_trust "$@"; }
# a module is too large for a command line: the candid argument goes through a file
blobarg() { python3 - "$@" <<'PYX'
import sys
def blob(b): return 'blob "' + ''.join('\\%02x' % x for x in b) + '"'
wasm = open(sys.argv[1], 'rb').read()
arg = bytes.fromhex(sys.argv[2]) if len(sys.argv) > 2 and sys.argv[2] else b''
open(sys.argv[3] if len(sys.argv) > 3 else '/dev/stdout', 'w').write('(' + blob(wasm) + ', ' + blob(arg) + ')')
PYX
}
propose_file() { local who=$1 f=$2; d canister call --identity "$who" timelock_trust propose_upgrade --argument-file "$f"; }
pr() { command dfx identity get-principal --identity "$1" 2>/dev/null; }
wait_run() { # execute <id> as the anonymous principal once its delay has passed; prints the final answer
  local out
  for _ in $(seq 1 30); do
    out=$(d canister call --identity anonymous timelock_trust execute "($1 : nat64)")
    grep -q "may run in" <<<"$out" || { printf '%s' "$out"; return; }
    python3 -c "import time; time.sleep(2)"
  done
  printf '%s' "$out"
}
modhash() { d canister info "$1" | awk '/Module hash/ {print $3}'; }

echo "0. build: the buyer, and the timelock with the rehearsal feature (a 10 s floor; everything else is the production rules)"
(cd ../canister-rs && cargo build -q --release --target wasm32-unknown-unknown)
cargo build -q --release --target wasm32-unknown-unknown --features rehearsal
BUYER_WASM=../canister-rs/target/wasm32-unknown-unknown/release/etch_autobuyer.wasm
TL_WASM=$(mktemp); cp target/wasm32-unknown-unknown/release/etch_timelock.wasm "$TL_WASM"
GZ=$(mktemp); gzip -9 -n -c "$BUYER_WASM" > "$GZ"          # the same code, another module hash
ME=$(command dfx identity get-principal)
P1=$(pr trust-p1); P2=$(pr trust-p2); P3=$(pr trust-p3); G1=$(pr trust-g1); G2=$(pr trust-g2); G3=$(pr trust-g3)

echo "1. create and install both, fresh"
for c in autobuyer_trust timelock_trust; do d canister create "$c" >/dev/null; done
BUYER=$(d canister id autobuyer_trust | tail -1); TL=$(d canister id timelock_trust | tail -1)
echo "   buyer $BUYER   timelock $TL"
INIT="(record { chain_id = 46630 : nat64; vault = \"0x0000000000000000000000000000000000000001\"; rpc_urls = vec { \"https://rpc.testnet.chain.robinhood.com\"; \"https://robinhood-sepolia-rpc.publicnode.com\" }; btc_api_urls = vec { \"https://mempool.space/signet/api\"; \"https://blockstream.info/signet/api\" }; key_name = \"key_1\"; accept_production_key_name = true; bridge_allowlist = vec {}; btc_network = \"signet\"; operator = opt principal \"$ME\" })"
d canister install autobuyer_trust --mode reinstall --yes --wasm "$BUYER_WASM" --argument "$INIT" >/dev/null
ROLES="record { proposers = vec { principal \"$P1\"; principal \"$P2\" }; guardians = vec { principal \"$G1\"; principal \"$G2\"; principal \"$G3\" }; veto_threshold = 2 : nat8 }"
d canister install timelock_trust --mode reinstall --yes --wasm "$TL_WASM" --argument "(record { target = principal \"$BUYER\"; roles = $ROLES; delay_secs = $DELAY : nat64 })" >/dev/null
expect "the timelock refuses a veto threshold above its guardians" "veto threshold of 4" \
  "$(d canister install timelock_trust --mode reinstall --yes --wasm "$TL_WASM" --argument "(record { target = principal \"$BUYER\"; roles = record { proposers = vec { principal \"$P1\"; principal \"$P2\" }; guardians = vec { principal \"$G1\"; principal \"$G2\"; principal \"$G3\" }; veto_threshold = 4 : nat8 }; delay_secs = $DELAY : nat64 })")"
d canister install timelock_trust --mode reinstall --yes --wasm "$TL_WASM" --argument "(record { target = principal \"$BUYER\"; roles = $ROLES; delay_secs = $DELAY : nat64 })" >/dev/null
d canister update-settings autobuyer_trust --set-controller "$TL" --yes >/dev/null
expect "handover: the buyer's only controller is the timelock" "$TL" "$(d canister info autobuyer_trust | grep Controllers)"

echo "2. cycles, for anyone, on both (docs/v2/trust-points.md §2)"
expect "the buyer answers cycles() to the anonymous caller, with its 5T threshold" "low_threshold = 5_000_000_000_000" "$(d canister call --identity anonymous --query autobuyer_trust cycles)"
expect "the timelock answers cycles() too, with its 0.5T threshold" "low_threshold = 500_000_000_000" "$(d canister call --identity anonymous --query timelock_trust cycles)"

echo "3. a stolen proposer (p1): its proposal needs TWO guardians to veto, and it cannot withdraw anyone else's"
ARGF=$(mktemp); blobarg "$GZ" "" "$ARGF"
A=$(propose_file trust-p1 "$ARGF" | grep -o "id = [0-9]*" | head -1 | awk '{print $3}' || true)
[ -n "$A" ] || fail "p1 could not propose"; ok "p1 proposed $A"
expect "a guardian cannot propose" "is not a proposer" "$(as trust-g1 propose_controllers "(vec { principal \"$TL\" })")"
expect "p2 cannot withdraw p1's proposal" "not" "$(as trust-p2 withdraw "($A : nat64)")"
expect "one guardian's veto is not enough" "Ok = false" "$(as trust-g1 veto "($A : nat64)")"
expect "...and the proposal is still pending" "id = $A" "$(d canister call --identity anonymous --query timelock_trust status)"
expect "the same guardian twice is still one vote" "already voted" "$(as trust-g1 veto "($A : nat64)")"
expect "a second guardian vetoes it" "Ok = true" "$(as trust-g2 veto "($A : nat64)")"
expect "nothing is pending" "pending = vec {}" "$(d canister call --identity anonymous --query timelock_trust status)"

echo "4. two pending upgrades cannot land one after the other: the second is stale"
BEFORE=$(modhash autobuyer_trust)
INITHEX=$(cargo run -q --example encode_init -- "$ME")
ARGB=$(mktemp); blobarg "$GZ" "$INITHEX" "$ARGB"
B=$(propose_file trust-p1 "$ARGB" | grep -o "id = [0-9]*" | head -1 | awk '{print $3}' || true)
C=$(as trust-p2 propose_controllers "(vec { principal \"$TL\" })" | grep -o "id = [0-9]*" | head -1 | awk '{print $3}' || true)
[ -n "$B" ] && [ -n "$C" ] || fail "could not propose both"; ok "p1 proposed upgrade $B; p2 proposed controllers $C, beside it"
expect "the upgrade runs after the delay, by the anonymous principal" "executed" "$(wait_run "$B")"
AFTER=$(modhash autobuyer_trust)
[ "$AFTER" != "$BEFORE" ] && ok "the buyer's module hash moved: $BEFORE -> $AFTER" || fail "the module did not change"
expect "the controller change, proposed before it, is stale and cleared" "stale" "$(wait_run "$C")"

echo "5. roles change like code: p2 removes the stolen p1; p1 cannot stop it"
R=$(as trust-p2 propose_roles "(record { proposers = vec { principal \"$P2\"; principal \"$P3\" }; guardians = vec { principal \"$G1\"; principal \"$G2\"; principal \"$G3\" }; veto_threshold = 2 : nat8 })" | grep -o "id = [0-9]*" | head -1 | awk '{print $3}' || true)
[ -n "$R" ] || fail "p2 could not propose roles"; ok "p2 proposed roles $R"
expect "p1 cannot withdraw it" "not" "$(as trust-p1 withdraw "($R : nat64)")"
expect "one guardian cannot veto it alone" "Ok = false" "$(as trust-g3 veto "($R : nat64)")"
expect "the roles change runs after the delay" "roles are changed" "$(wait_run "$R")"
expect "p1 can no longer propose" "is not a proposer" "$(as trust-p1 propose_controllers "(vec { principal \"$TL\" })")"
P3OUT=$(as trust-p3 propose_controllers "(vec { principal \"$TL\" })")
expect "p3 now can" "Ok" "$P3OUT"
as trust-p3 withdraw "($(grep -o "id = [0-9]*" <<<"$P3OUT" | head -1 | awk '{print $3}') : nat64)" >/dev/null   # tidy: nothing left pending

echo "6. pause and resume: any guardian stops the buyer; resuming takes two"
expect "a proposer cannot pause" "not a guardian" "$(as trust-p2 pause_target)"
as trust-g3 pause_target >/dev/null
expect "g3 alone paused the buyer" "true" "$(d canister call --identity anonymous --query autobuyer_trust paused)"
expect "g3 alone cannot resume it" "Ok = false" "$(as trust-g3 resume_target)"
expect "...it is still paused" "true" "$(d canister call --identity anonymous --query autobuyer_trust paused)"
expect "g1's vote makes two: resumed" "Ok = true" "$(as trust-g1 resume_target)"
expect "the buyer runs again" "false" "$(d canister call --identity anonymous --query autobuyer_trust paused)"
expect "the timelock is still the buyer's only controller" "$TL" "$(d canister info autobuyer_trust | grep Controllers)"

echo
echo "PASS: sets, a 2-of-3 veto, stale proposals, roles that change like code, a pause any guardian can make and two must lift,"
echo "and public cycles on both canisters. buyer $BUYER, timelock $TL (local replica)."
