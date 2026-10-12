#!/usr/bin/env bash
# W14a (docs/v9/canister-only-solana.md §13.1, D-d): install a module through the IC's chunked code upload, for a module (and argument)
# too large for one message. The buyer is ≈ 1.8 MB gzipped, near the 2 MiB single-message limit; dfx and the timelock both need this path.
#
#   scripts/chunked-install.sh [--dry-run] [--network local] [--mode install|reinstall|upgrade] <canister> <module.wasm[.gz]> [<arg.bin>]
#
#   1. the plan (no network): 1 MiB pieces (the IC's chunk size), each piece's SHA-256, and the FULL module's SHA-256: the canister's
#      module hash after the install, the value `reproducible-build.sh verify` compares with the source's build;
#   2. `clear_chunk_store`, then `upload_chunk` of each piece to the TARGET's own chunk store (each answered hash checked against the plan);
#   3. `install_chunked_code` (the chunk hashes in order, the full module's hash, the argument as raw Candid bytes);
#   4. `clear_chunk_store` again, and the canister's module hash read back and compared with the plan's.
#
# The caller must be a controller of <canister>. Before the handover that is the operator's identity; after it, ONLY the timelock, which
# installs the same way itself (timelock-rs: `stage_chunk` / `propose_staged_upgrade`, then `execute`; see timelock-propose-chunked.sh).
# --dry-run prints the plan and every call (with its argument file) without sending anything. Mainnet needs --network ic, typed.
set -euo pipefail
export PATH="$HOME/.cargo/bin:$HOME/Library/Application Support/org.dfinity.dfx/bin:$PATH"
export DFX_WARNING=-mainnet_plaintext_identity
DRY=""; NET=local; MODE=upgrade
while [ $# -gt 0 ]; do
  case "$1" in
    --dry-run) DRY=1; shift ;;
    --network) NET=${2:?network}; shift 2 ;;
    --mode) MODE=${2:?mode}; shift 2 ;;
    -*) echo "unknown flag $1" >&2; exit 2 ;;
    *) break ;;
  esac
done
CANISTER=${1:?usage: $0 [--dry-run] [--network N] [--mode install|reinstall|upgrade] <canister> <module.wasm[.gz]> [<arg.bin>]}
WASM=${2:?module}
ARG=${3:-}
case "$MODE" in install|reinstall|upgrade) ;; *) echo "mode: install, reinstall or upgrade" >&2; exit 2 ;; esac
[ -f "$WASM" ] || { echo "no module at $WASM" >&2; exit 2; }
WORK=$(mktemp -d); trap 'rm -rf "$WORK"' EXIT
HERE=$(cd "$(dirname "$0")" && pwd)

# 1. the plan, and one Candid argument file per call (blobs escaped; a 1 MiB piece does not fit on a command line)
python3 -I "$HERE/chunked_plan.py" "$WASM" "${ARG:-}" "$CANISTER" "$MODE" "$WORK" > "$WORK/plan.txt"
cat "$WORK/plan.txt"
N=$(awk '/^pieces /{print $2}' "$WORK/plan.txt")
WANT=$(awk '/^module_sha256 /{print $2}' "$WORK/plan.txt")
[ -n "$N" ] && [ -n "$WANT" ] || { echo "no plan" >&2; exit 1; }

call() { # call <method> <argfile>
  if [ -n "$DRY" ]; then echo "dry-run: dfx canister call --network $NET aaaaa-aa $1 --argument-file $2 ($(wc -c < "$2" | tr -d ' ') bytes)" >&2; return 0; fi
  dfx canister call --network "$NET" aaaaa-aa "$1" --argument-file "$2" --output idl
}

# 2. the target's chunk store, emptied then filled
call clear_chunk_store "$WORK/clear.did" >/dev/null
for i in $(seq 0 $((N - 1))); do
  out=$(call upload_chunk "$WORK/chunk-$i.did")
  want_i=$(awk -v i="$i" '$1=="piece" && $2==i {print $5}' "$WORK/plan.txt")
  if [ -z "$DRY" ]; then
    got=$(python3 -I -c 'import re,sys; m=re.search(r"blob \"([^\"]*)\"", sys.stdin.read()); print(bytes(int(x,16) for x in re.findall(r"\\([0-9a-f]{2})", m.group(1))).hex() if m else "")' <<<"$out")
    [ "$got" = "$want_i" ] || { echo "FAIL piece $i: the IC stored a chunk hashed $got, not $want_i" >&2; exit 1; }
  fi
  echo "uploaded piece $i ($want_i)"
done
# 3. the install, naming every chunk's hash in order and the full module's
call install_chunked_code "$WORK/install.did" >/dev/null
echo "install_chunked_code ($MODE) of $WANT sent"
# 4. the store emptied, the module hash read back
call clear_chunk_store "$WORK/clear.did" >/dev/null
if [ -z "$DRY" ]; then
  got=$(dfx canister --network "$NET" info "$CANISTER" 2>/dev/null | awk '/Module hash/ {print $3}' | sed 's/^0x//')
  echo "module hash now  $got"
  [ "$got" = "$WANT" ] && echo "MATCH: $CANISTER runs the module of $WASM" || { echo "NO MATCH (wanted $WANT)"; exit 1; }
else
  echo "dry-run: nothing was sent; the module hash after a real install would be $WANT"
fi
