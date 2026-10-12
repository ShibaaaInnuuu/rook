#!/usr/bin/env bash
# Spike S3 (docs/v9/canister-only-solana.md §15 S3): one Solana transaction signed by three derivation paths of the canister's
# threshold Ed25519 key. LOCAL ONLY: dfx 0.32's replica (key_1) + a throwaway solana-test-validator started by drive.py.
#
#   cd ops/rehearsal/sol-multisig && dfx start --clean --background
#   ./run.sh [--no-validator] [out_dir]
#   dfx stop
#
# 1. builds the canister (Rust 1.98.1, wasm32) and the native offline tool into ./target;
# 2. creates (or reinstalls) `solmulti` on the local replica with key_1 (local cycles only);
# 3. drive.py: pubkeys per path; a local validator with a nonce account (authority K_main) and two mints (authority K_coll[3],
#    Multi and Flat paths); three 3-signer transactions (sequential, parallel, flat paths) submitted and confirmed; timing repeats;
#    the concurrency burst;
# 4. offline/: every key re-derived offline (ic-ed25519) and every signature verified (ed25519-dalek, ic-ed25519);
# 5. upgrade + reinstall with a different module: the keys must not change.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"; cd "$HERE"
DFX="${DFX:-$HOME/Library/Application Support/org.dfinity.dfx/bin/dfx}"
export DFX_WARNING=-mainnet_plaintext_identity
export PATH="$HOME/.local/share/solana/install/active_release/bin:$HOME/.cargo/bin:$PATH"
export CARGO_TARGET_DIR="$HERE/target"
NOV=""; [ "${1:-}" = --no-validator ] && { NOV=--no-validator; shift; }
OUT="${1:-${TMPDIR:-/tmp}/rook-s3}"; mkdir -p "$OUT"
"$DFX" ping local >/dev/null 2>&1 || { echo "refused: no local replica; run: cd $HERE && dfx start --clean --background"; exit 2; }

echo "== build"
( cd canister && cargo +1.98.1 build --release --target wasm32-unknown-unknown -q ) || { echo "FAIL canister build"; exit 1; }
( cd offline && cargo +1.98.1 build --release -q ) || { echo "FAIL offline build"; exit 1; }
WASM="$CARGO_TARGET_DIR/wasm32-unknown-unknown/release/rook_sol_multisig.wasm"
INIT='(record { key_name = "key_1" })'
"$DFX" canister id solmulti >/dev/null 2>&1 || "$DFX" canister create solmulti --with-cycles 50000000000000 2>&1 | grep -v deprecated
"$DFX" canister install solmulti --mode reinstall --wasm "$WASM" --argument "$INIT" -y 2>&1 | grep -v deprecated

echo "== drive"
python3 -I drive.py "$DFX" "$OUT" $NOV || exit 1
echo "== offline derivation + signature verification"
"$CARGO_TARGET_DIR/release/rook_sol_multisig_offline" "$OUT/report.json" || exit 1

echo "== stability across upgrade / reinstall (a different module each time)"
keys() { for p in 'vec {}' 'vec { blob "sol" }' 'vec { blob "sol"; blob "intent"; blob "\07\00\00\00\00\00\00\00" }' \
                  'vec { blob "sol"; blob "coll"; blob "\03\00\00\00" }' 'vec { blob "sol/intent/7" }' 'vec { blob "sol/coll/3" }'; do
  "$DFX" canister call solmulti pubkey "($p)" --output json 2>/dev/null | python3 -c "import sys,json; d=json.load(sys.stdin)['Ok']; print(d['address'], bytes(d['chain_code']).hex())"; done; }
keys > "$OUT/keys0.txt"
python3 -c "import sys; w=open(sys.argv[1],'rb').read(); n=b's3-v2'; p=bytes([len(n)])+n+b'x'; open(sys.argv[2],'wb').write(w+b'\x00'+bytes([len(p)])+p)" "$WASM" "$OUT/v2.wasm"
"$DFX" canister install solmulti --mode upgrade --wasm "$OUT/v2.wasm" --argument "$INIT" -y 2>&1 | grep -v deprecated
keys > "$OUT/keys1.txt"
"$DFX" canister install solmulti --mode reinstall --wasm "$WASM" --argument "$INIT" -y 2>&1 | grep -v deprecated
keys > "$OUT/keys2.txt"
if [ -s "$OUT/keys0.txt" ] && cmp -s "$OUT/keys0.txt" "$OUT/keys1.txt" && cmp -s "$OUT/keys0.txt" "$OUT/keys2.txt"; then
  echo "PASS the 6 keys (and chain codes) are identical after an upgrade and a reinstall"
else echo "FAIL keys changed"; diff "$OUT/keys0.txt" "$OUT/keys1.txt"; diff "$OUT/keys0.txt" "$OUT/keys2.txt"; exit 1; fi
echo "report: $OUT/report.json, summary: $OUT/summary.json"
