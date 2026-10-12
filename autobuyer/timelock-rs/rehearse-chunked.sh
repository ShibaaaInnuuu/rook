#!/usr/bin/env bash
# W14a (docs/v9/canister-only-solana.md §13.1, D-d) rehearsed on the LOCAL replica only (every dfx call is --network local): the buyer
# installed through the IC's chunked code upload, then upgraded THROUGH THE TIMELOCK with a module far over one message, staged in pieces,
# proposed whole with the full module's hash, executed after the delay by a stranger; and the canister's own Solana timer (W14b) running
# after each install. Creates two canisters of its own, `autobuyer_chunked` and `timelock_chunked`, in this directory's dfx project.
#
#   cd autobuyer/timelock-rs && dfx start --clean --background && ./rehearse-chunked.sh && dfx stop
#
# The buyer's module: the gzipped build (≈ 1.9 MB) installed chunked, then the RAW build (≈ 9 MB, nine 1 MiB chunks) proposed and
# installed by the timelock: a module no single message could carry. The timelock is the rehearsal build (10 s floor); production's 72 h
# floor is the same code with the `rehearsal` feature off.
set -euo pipefail
export PATH="$HOME/.cargo/bin:$HOME/Library/Application Support/org.dfinity.dfx/bin:$PATH"
export DFX_WARNING=-mainnet_plaintext_identity
cd "$(dirname "$0")"
DELAY=20
ok() { printf '   \033[1m✓ %s\033[0m\n' "$*"; }
fail() { printf '   ✗ %s\n' "$*"; exit 1; }
expect() { if grep -q -- "$2" <<<"$3"; then ok "$1"; else printf '%s\n' "$3" | tail -6; fail "$1 (expected: $2)"; fi; }
d() { command dfx "$@" --network local 2>&1 | grep -v "^WARNING" || true; }
modhash() { d canister info "$1" | awk '/Module hash/ {print $3}' | sed 's/^0x//'; }
command dfx ping local >/dev/null 2>&1 || { echo "refused: no local replica; run: cd $(pwd) && dfx start --clean --background"; exit 2; }

RAW=../canister-rs/target/wasm32-unknown-unknown/release/etch_autobuyer.wasm
TLW=target/wasm32-unknown-unknown/release/etch_timelock.wasm
WORK=$(mktemp -d); trap 'rm -rf "$WORK"' EXIT
echo "0. the builds: the buyer (raw and gzipped) and the rehearsal timelock"
(cd ../canister-rs && cargo build --locked --release --target wasm32-unknown-unknown -q)
cargo build --locked --release --target wasm32-unknown-unknown --features rehearsal -q
gzip -9 -n -c "$RAW" > "$WORK/buyer.wasm.gz"
GZ_SHA=$(shasum -a 256 "$WORK/buyer.wasm.gz" | cut -d' ' -f1); RAW_SHA=$(shasum -a 256 "$RAW" | cut -d' ' -f1)
echo "   buyer.wasm.gz $(wc -c < "$WORK/buyer.wasm.gz" | tr -d ' ') bytes $GZ_SHA"
echo "   buyer.wasm    $(wc -c < "$RAW" | tr -d ' ') bytes $RAW_SHA"
ME=$(command dfx identity get-principal 2>/dev/null)
cargo run -q --example encode_init -- "$ME" | python3 -c "import sys; open(sys.argv[1],'wb').write(bytes.fromhex(sys.stdin.read().strip()))" "$WORK/init.bin"

echo "1. the buyer installed FRESH through the chunk store (scripts/chunked-install.sh, mode install)"
for c in autobuyer_chunked timelock_chunked; do d canister create "$c" >/dev/null; done
BUYER=$(d canister id autobuyer_chunked | tail -1); TL=$(d canister id timelock_chunked | tail -1)
echo "   buyer $BUYER, timelock $TL"
d canister uninstall-code autobuyer_chunked >/dev/null
OUT=$(../scripts/chunked-install.sh --network local --mode install "$BUYER" "$WORK/buyer.wasm.gz" "$WORK/init.bin" 2>&1 || true)
expect "installed in 2 chunks; the module hash is the gzipped module's" "MATCH: $BUYER" "$OUT"
expect "its state starts" "installed" "$(d canister call autobuyer_chunked log)"
sleep 8
expect "W14b: its own Solana timer ran (no Solana home here: it idles, every 60 s)" "runs = [1-9]" "$(d canister call autobuyer_chunked sol_lines)"

echo "2. the handover: the timelock becomes the buyer's ONLY controller"
d canister install timelock_chunked --mode reinstall --yes --wasm "$TLW" \
  --argument "(record { target = principal \"$BUYER\"; roles = record { proposers = vec { principal \"$ME\" }; guardians = vec { principal \"$ME\" }; veto_threshold = 1 : nat8 }; delay_secs = $DELAY : nat64 })" >/dev/null
d canister update-settings autobuyer_chunked --set-controller "$TL" --yes >/dev/null
expect "a direct chunked install is refused now" "FAIL\|not.*controller\|Only controllers\|Unauthorized\|NO MATCH" \
  "$(../scripts/chunked-install.sh --network local "$BUYER" "$RAW" "$WORK/init.bin" 2>&1 || true)"
[ "$(modhash autobuyer_chunked)" = "$GZ_SHA" ] && ok "and the module is unchanged" || fail "the module changed"

echo "3. a module far over one message (the raw build), staged in pieces and proposed whole (scripts/timelock-propose-chunked.sh)"
expect "a single-message proposal of it cannot even be sent" "too large\|exceeds\|413\|503\|HTTP Error\|Payload\|payload\|limit" \
  "$(python3 -c "import sys; w=open(sys.argv[1],'rb').read(); open(sys.argv[2],'w').write('(blob \"' + ''.join('\\\\%02x' % x for x in w) + '\", blob \"\")')" "$RAW" "$WORK/one.did" \
     && d canister call timelock_chunked propose_upgrade --argument-file "$WORK/one.did")"
expect "a stranger cannot stage (refused at ingress)" "is not a proposer\|rejected the message" "$(command dfx canister call timelock_chunked stage_chunk '(blob "\00asm")' --identity anonymous --network local 2>&1 || true)"
OUT=$(../scripts/timelock-propose-chunked.sh --network local "$TL" "$RAW" "$WORK/init.bin" 2>&1 || true)
expect "staged piece by piece, each checked; proposed with the full module's hash" "PROPOSED: the module $RAW_SHA" "$OUT"
ID=$(d canister call timelock_chunked status | grep -o "id = [0-9]*" | head -1 | grep -o "[0-9]*")
expect "status() shows it to anyone, pending, with the full module's hash" "$RAW_SHA" "$(command dfx canister call timelock_chunked status --identity anonymous --network local 2>&1)"
expect "executing it before the delay is refused" "may run in" "$(d canister call timelock_chunked execute "($ID : nat64)")"

echo "4. executed by the anonymous principal after the ${DELAY} s delay: chunks uploaded, buyer stopped, install_chunked_code, started"
for _ in $(seq 1 40); do
  OUT=$(command dfx canister call timelock_chunked execute "($ID : nat64)" --identity anonymous --network local 2>&1 | grep -v "^WARNING" || true)
  grep -q "may run in\|rejected the message" <<<"$OUT" || break # before its delay, the ingress filter refuses a stranger's execute
  python3 -c "import time; time.sleep(3)"
done
expect "executed" "proposal $ID executed" "$OUT"
[ "$(modhash autobuyer_chunked)" = "$RAW_SHA" ] && ok "the buyer's module hash is the proposed hash ($RAW_SHA)" || fail "module $(modhash autobuyer_chunked), proposed $RAW_SHA"
expect "its state came back" "upgraded: state restored" "$(d canister call autobuyer_chunked log)"
sleep 8
expect "W14b: the timer was armed again by post_upgrade and runs" "runs = [1-9]" "$(d canister call autobuyer_chunked sol_lines)"
expect "history records it" "Executed" "$(d canister call timelock_chunked history)"

echo "5. the way back: the controllers handed back to me (so this rehearsal can run again)"
d canister call timelock_chunked propose_controllers "(vec { principal \"$ME\" })" >/dev/null
ID=$(d canister call timelock_chunked status | grep -o "id = [0-9]*" | head -1 | grep -o "[0-9]*")
for _ in $(seq 1 40); do
  OUT=$(command dfx canister call timelock_chunked execute "($ID : nat64)" --identity anonymous --network local 2>&1 | grep -v "^WARNING" || true)
  grep -q "may run in\|rejected the message" <<<"$OUT" || break # before its delay, the ingress filter refuses a stranger's execute
  python3 -c "import time; time.sleep(3)"
done
expect "controllers handed back" "executed" "$OUT"
echo
echo "PASS: a ${RAW_SHA:0:12}… module of $(wc -c < "$RAW" | tr -d ' ') bytes reached the buyer only through a published, delayed, chunked proposal."
