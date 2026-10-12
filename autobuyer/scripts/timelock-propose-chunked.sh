#!/usr/bin/env bash
# W14a: propose an upgrade of the buyer through the timelock when the module does not fit in one message (`propose_upgrade` carries at
# most 2,000,000 bytes; the buyer is ≈ 1.8 MB gzipped and growing). The 72 h delay, the veto and the published hash are unchanged:
#
#   scripts/timelock-propose-chunked.sh [--dry-run] [--network local] <timelock> <module.wasm[.gz]> <arg.bin>
#
#   1. `unstage` (whatever this proposer staged before), then `stage_chunk` of each 1 MiB piece; after each, the timelock's running
#      size and SHA-256 are compared with this file's prefix: a lost or doubled piece stops here;
#   2. `propose_staged_upgrade(<SHA-256 of the FULL module>, <arg>)`: the proposal publishes that hash (what the buyer's module hash will be,
#      and what `reproducible-build.sh verify` compares with the source's build) and waits the delay from now;
#   3. after the delay anyone may `execute` it: the timelock uploads the pieces to the buyer's chunk store, stops the buyer, installs with
#      `install_chunked_code` naming the full module's hash, and starts it again (timelock-rs/src/canister.rs).
#
# The caller must be one of the timelock's proposers. --dry-run prints the plan and every call without sending anything.
set -euo pipefail
export PATH="$HOME/.cargo/bin:$HOME/Library/Application Support/org.dfinity.dfx/bin:$PATH"
export DFX_WARNING=-mainnet_plaintext_identity
DRY=""; NET=local
while [ $# -gt 0 ]; do
  case "$1" in
    --dry-run) DRY=1; shift ;;
    --network) NET=${2:?network}; shift 2 ;;
    -*) echo "unknown flag $1" >&2; exit 2 ;;
    *) break ;;
  esac
done
TL=${1:?usage: $0 [--dry-run] [--network N] <timelock> <module.wasm[.gz]> <arg.bin>}
WASM=${2:?module}
ARG=${3:?the upgrade argument as raw Candid bytes (e.g. timelock-rs: cargo run --example encode_init)}
WORK=$(mktemp -d); trap 'rm -rf "$WORK"' EXIT

python3 -I - "$WASM" "$ARG" "$WORK" <<'PY'
import hashlib, os, sys
wasm = open(sys.argv[1], "rb").read(); arg = open(sys.argv[2], "rb").read(); out = sys.argv[3]
if not (wasm.startswith(b"\0asm") or wasm.startswith(b"\x1f\x8b\x08")):
    sys.exit("refused: not a WebAssembly module and not gzip")
blob = lambda b: 'blob "' + "".join("\\%02x" % x for x in b) + '"'
step = 1_048_576
pieces = [(a, min(a + step, len(wasm))) for a in range(0, len(wasm), step)]
full = hashlib.sha256(wasm).hexdigest()
print(f"module {sys.argv[1]} {len(wasm)} bytes, sha256 {full}; argument {len(arg)} bytes, sha256 {hashlib.sha256(arg).hexdigest()}")
print(f"pieces {len(pieces)}")
for i, (a, b) in enumerate(pieces):
    open(os.path.join(out, f"stage-{i}.did"), "w").write(f"({blob(wasm[a:b])})")
    print(f"piece {i} {a} {b} prefix {b} {hashlib.sha256(wasm[:b]).hexdigest()}")
open(os.path.join(out, "propose.did"), "w").write(f'("{full}", {blob(arg)})')
print(f"module_sha256 {full}")
PY
python3 -I - "$WASM" "$ARG" "$WORK" > "$WORK/plan.txt" <<'PY'
import hashlib, sys
wasm = open(sys.argv[1], "rb").read()
print(f"module_sha256 {hashlib.sha256(wasm).hexdigest()}")
step = 1_048_576
n = (len(wasm) + step - 1) // step
print(f"pieces {n}")
for i in range(n):
    b = min((i + 1) * step, len(wasm))
    print(f"prefix {i} {b} {hashlib.sha256(wasm[:b]).hexdigest()}")
PY
N=$(awk '/^pieces /{print $2}' "$WORK/plan.txt")
FULL=$(awk '/^module_sha256 /{print $2}' "$WORK/plan.txt")

call() { # call <method> [argfile]
  if [ -n "$DRY" ]; then echo "dry-run: dfx canister call --network $NET $TL $1 ${2:+--argument-file $2 ($(wc -c < "$2" | tr -d ' ') bytes)}" >&2; echo "(variant { Ok })"; return 0; fi
  if [ -n "${2:-}" ]; then dfx canister call --network "$NET" "$TL" "$1" --argument-file "$2"; else dfx canister call --network "$NET" "$TL" "$1"; fi
}

call unstage >/dev/null || true # nothing staged before is fine
for i in $(seq 0 $((N - 1))); do
  out=$(call stage_chunk "$WORK/stage-$i.did")
  read -r _ _ size hash <<<"$(awk -v i="$i" '$1=="prefix" && $2==i' "$WORK/plan.txt")"
  if [ -z "$DRY" ]; then
    # Candid text writes 1_048_576: compare digits only
    grep -q "$hash" <<<"$out" && grep -q "$size" <<<"${out//_/}" || { echo "FAIL piece $i: the timelock holds $out, this file's prefix is $size bytes $hash" >&2; exit 1; }
  fi
  echo "staged piece $i: $size bytes, sha256 $hash"
done
out=$(call propose_staged_upgrade "$WORK/propose.did")
echo "$out"
if [ -z "$DRY" ]; then
  grep -q "$FULL" <<<"$out" || { echo "FAIL: the proposal does not publish $FULL" >&2; exit 1; }
  echo "PROPOSED: the module $FULL, executable after the timelock's delay"
else
  echo "dry-run: nothing was sent; the proposal would publish $FULL"
fi
