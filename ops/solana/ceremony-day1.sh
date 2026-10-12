#!/usr/bin/env bash
# Day 1 of the Solana launch WITHOUT deploying Rook's programs (decisions D-f, D-g: docs/v9/solana-migration.md §2.9; runbook:
# docs/v9/launch-day1-solana.md).
#
#   ops/solana/ceremony-day1.sh --keys <dir OUTSIDE the repository> --launch <launch.json> [--rpc <url>] [--fund-wallet <pubkey>]
#        [--rook-name ROOK --rook-symbol ROOK] [--rook-uri <default: the hosted https://res.cloudinary.com/…/rook-token/rook.json;
#        or ipfs://… | https://ipfs.io/ipfs/… | https://gateway.pinata.cloud/ipfs/… | https://arweave.net/…>] [--send] [--yes]
#
# 1. Reserves the mainnet program ids: generates rook_core-keypair.json and rook_backing-keypair.json in <keys> (0600, never
#    overwritten, never printed). WHOEVER HOLDS THESE FILES CAN DEPLOY THE PROGRAMS, and the programs' PDAs are what DBC pays: the
#    backing share and the partner fees of every coin wait in DBC until a program is deployed at these ids. Back them up offline.
# 2. Generates the ceremony payer (<keys>/ceremony-payer.json): a hot key that pays the day-1 rent and signs as the ROOK creator for
#    one instruction (the creator moves to the fund wallet in the same transaction). The fund wallet funds it with ONE plain transfer.
# 3. Prints the program ids, rook_backing's converter PDA, and per collection the listing PDA and the claimer PDA (the DBC fee
#    claimer), and builds every DBC create_config + the genesis ROOK config/launch (rook-ceremony day1). With --rpc every transaction
#    is simulated (nothing sent) and the exact SOL is printed; <keys>/day1/day1-plan.json holds the unsigned messages.
# 4. --send (with --rpc) signs with the ceremony payer and the config/mint keypairs and sends, after `Type 'day1'` (--yes skips it).
#    rook-ceremony refuses --send unless: the launch file's meteora parameters and listing_order are byte-identical to the committed
#    pins (ops/solana/day1-pins.json; audit A6-05), review.reviewed_by is filled in, and --rook-uri is a permanently hosted metadata
#    JSON (one of the hosts above) whose name / symbol match and whose "image" resolves (A6-06: ROOK's metadata is immutable).
#    Name and symbol default to "ROOK" / "ROOK" and the uri to the hosted Cloudinary JSON (operator decisions 2026-10-10); those two
#    Cloudinary files (rook.json, rook.png) must never be deleted or renamed: the on-chain uri is immutable.
#
# The fund wallet (default: the operator's GGgXnNgt…6Wuy) never signs anything here and its key is never asked for.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd -P)"
export PATH="$HOME/.local/share/solana/install/active_release/bin:$HOME/.cargo/bin:$PATH"
FUND_WALLET="GGgXnNgtY4ySfz8PLAAwkoh8Ztj7p6jrM9towSaa6Wuy"
KEYS="" LAUNCH="" RPC="" SEND="" YES="" EXTRA=() RNAME="ROOK" RSYMBOL="ROOK" RURI="https://res.cloudinary.com/ddwebzwrk/raw/upload/v1791607907/rook-token/rook.json"
while [ $# -gt 0 ]; do
  case "$1" in
    --keys) KEYS="$2"; shift 2;;
    --launch) LAUNCH="$2"; shift 2;;
    --rpc) RPC="$2"; shift 2;;
    --fund-wallet) FUND_WALLET="$2"; shift 2;;
    --rook-name) RNAME="$2"; shift 2;;
    --rook-symbol) RSYMBOL="$2"; shift 2;;
    --rook-uri) RURI="$2"; shift 2;;
    --sim-payer) EXTRA+=("$1" "$2"); shift 2;;
    --send) SEND=1; shift;;
    --yes) YES=1; shift;;
    *) echo "unknown argument $1 (see the header)"; exit 2;;
  esac
done
[ -n "$KEYS" ] && [ -n "$LAUNCH" ] || { sed -n 2,20p "$0"; exit 2; }
# refuse a path inside the repository / any git work tree BEFORE creating anything (audit A6-01: the whole path resolved, symlinks
# included; identity by inode, so another letter case of the same directory is caught; `git rev-parse` from the resolved path)
. "$ROOT/ops/solana/keys-guard.sh"
KEYS="$(keys_guard "$KEYS" "$ROOT")" || exit 2
[ -d "$(dirname "$KEYS")" ] || { echo "refused: $(dirname "$KEYS") does not exist"; exit 2; }
mkdir -p "$KEYS" && chmod 700 "$KEYS"
KEYS="$(keys_guard "$KEYS" "$ROOT")" || exit 2   # again on the directory as created (a race or a symlink planted meanwhile)
[ -f "$LAUNCH" ] || { echo "refused: no launch file $LAUNCH (node ops/solana/launch-params.mjs --out <file>)"; exit 2; }
command -v solana-keygen >/dev/null || { echo "solana-keygen not found (Agave CLI)"; exit 2; }

key() { # a keypair file, generated once (no BIP39 phrase printed; the file IS the secret), never overwritten
  local f="$KEYS/$1"
  if [ ! -f "$f" ]; then
    (umask 077 && solana-keygen new --no-bip39-passphrase --silent --outfile "$f" >/dev/null)
    echo "generated $1" >&2
  fi
  chmod 600 "$f"
  solana-keygen pubkey "$f"
}
CORE="$(key rook_core-keypair.json)"
BACKING="$(key rook_backing-keypair.json)"
PAYER="$(key ceremony-payer.json)"
[ "$PAYER" != "$FUND_WALLET" ] || { echo "refused: the ceremony payer equals the fund wallet"; exit 2; }

TOOL="$ROOT/ops/rehearsal/sol-devnet/tool"
(cd "$TOOL" && cargo build -q --release --bin rook-ceremony)
ARGS=(day1 --rook-core "$CORE" --rook-backing "$BACKING" --launch "$LAUNCH" --out "$KEYS/day1" --fund-wallet "$FUND_WALLET" --payer "$KEYS/ceremony-payer.json")
[ -n "$RPC" ] && ARGS+=(--rpc "$RPC")
ARGS+=(--rook-name "$RNAME" --rook-symbol "$RSYMBOL")
[ -n "$RURI" ] && ARGS+=(--rook-uri "$RURI")
ARGS+=("${EXTRA[@]+"${EXTRA[@]}"}")
if [ -n "$SEND" ]; then
  [ -n "$RPC" ] || { echo "refused: --send needs --rpc"; exit 2; }
  [ -n "$RURI" ] || { echo "refused: --send needs --rook-uri (the permanently hosted ROOK metadata JSON; the mint's metadata is immutable)"; exit 2; }
  "$TOOL/target/release/rook-ceremony" "${ARGS[@]}"   # the dry run first: every transaction simulated, the exact cost
  if [ -z "$YES" ]; then
    echo "ROOK metadata (IMMUTABLE once sent): name \"$RNAME\", symbol \"$RSYMBOL\", uri $RURI"
    read -r -p "Send these transactions from the ceremony payer $PAYER? Type 'day1': " ok
    [ "$ok" = "day1" ] || { echo "not sent"; exit 1; }
  fi
  ARGS+=(--send)
fi
"$TOOL/target/release/rook-ceremony" "${ARGS[@]}"
cat <<EOF

Keys in $KEYS (0600; back up rook_core-keypair.json and rook_backing-keypair.json OFFLINE, twice; never in the repository):
  rook_core    $CORE
  rook_backing $BACKING
  ceremony payer $PAYER   (fund it from the fund wallet with one plain transfer of the cost above + 0.01 SOL;
                           e.g. Phantom "Send", or: solana transfer --from usb://ledger $PAYER <SOL> --allow-unfunded-recipient)
  fund wallet  $FUND_WALLET   (ROOK creator and fee claimer; never signs here)
After --send, return the payer's remainder to the fund wallet: solana transfer --from $KEYS/ceremony-payer.json $FUND_WALLET ALL
EOF
