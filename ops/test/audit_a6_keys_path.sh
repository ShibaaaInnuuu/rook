#!/usr/bin/env bash
# Audit A6-01 (docs/v9/audit-2026-10/a6-launch-ops.md): the "keys directory inside the repository" refusal of
# ops/solana/ceremony-day1.sh (lines 37-40) and ops/solana/deploy-later.sh (lines 38-39) can be bypassed, so the program keypairs
# (control of all future backing) can be written into the working tree, where nothing in .gitignore keeps them out of a commit.
#
#   bash ops/test/audit_a6_keys_path.sh        exit 0 = every refusal held (fixed); exit 1 = a bypass reproduced
#
# Offline and harmless: --launch names a file that does not exist, so ceremony-day1.sh stops right after its path check (it only
# creates the empty keys directory, which this script removes); deploy-later.sh stops at its "no rook_core-keypair.json" check,
# before any RPC call. No key is generated, nothing touches a network.
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd -P)"
TMP="$(mktemp -d)"; IN="$ROOT/ops/test/.audit_a6_tmp_$$"
cleanup() { rm -rf "$TMP" "$IN" "$IN-case"; }
trap cleanup EXIT
fails=0
expect_refused() { # $1 label, $2 output
  if grep -q "inside the repository" <<<"$2"; then echo "held     $1"; else echo "BYPASSED $1"; echo "         -> $(tail -1 <<<"$2")"; fails=$((fails + 1)); fi
}

# 1. ceremony-day1.sh: a keys path that is a symlink to a directory inside the repository (only the PARENT is resolved, line 38)
mkdir -p "$IN"; ln -s "$IN" "$TMP/keys"
out="$("$ROOT/ops/solana/ceremony-day1.sh" --keys "$TMP/keys" --launch "$TMP/no-such-launch.json" 2>&1)"
expect_refused "ceremony-day1.sh --keys <symlink to a dir in the repo>" "$out"

# 2. ceremony-day1.sh and deploy-later.sh: the repository path in another letter case (APFS / HFS+ default to case-insensitive;
#    `pwd -P` keeps the case it was given, and the refusal compares strings case-sensitively)
flip() { local p="$1" last; last="${p##*/}"; local f; f="$(tr '[:upper:][:lower:]' '[:lower:][:upper:]' <<<"${last:0:1}")${last:1}"; echo "${p%/*}/$f"; }
ALT="$(flip "$ROOT")"
if [ -d "$ALT" ] && [ "$ALT" != "$ROOT" ]; then
  REL="${IN#"$ROOT"/}-case"
  out="$("$ROOT/ops/solana/ceremony-day1.sh" --keys "$ALT/$REL" --launch "$TMP/no-such-launch.json" 2>&1)"
  expect_refused "ceremony-day1.sh --keys $ALT/$REL (same directory, other case)" "$out"
  mkdir -p "$IN-case"
  out="$("$ROOT/ops/solana/deploy-later.sh" --keys "$ALT/$REL" --launch "$TMP/x.json" --rpc http://127.0.0.1:9 2>&1)"
  expect_refused "deploy-later.sh --keys $ALT/$REL (same directory, other case)" "$out"
else
  echo "skipped  case variants: the filesystem is case-sensitive here"
fi

# 3. would git keep a keypair out of a commit? (no .gitignore rule for *keypair*.json / ceremony-payer.json)
if git -C "$ROOT" check-ignore -q "ops/test/x/rook_core-keypair.json" && git -C "$ROOT" check-ignore -q "ops/test/x/ceremony-payer.json"; then
  echo "held     .gitignore ignores keypair files"
else
  echo "BYPASSED .gitignore has no rule for rook_core-keypair.json / ceremony-payer.json / *-config.json: \`git add -A\` would stage them"
  fails=$((fails + 1))
fi
echo "$fails bypass(es)"
[ "$fails" -eq 0 ]
