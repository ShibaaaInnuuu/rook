#!/usr/bin/env bash
# docs/phase2.md §1. The only supported way to run script/Deploy.s.sol.
# It asks the RPC which chain it is BEFORE forge is started, and refuses unless ETCH_DEPLOY_CONFIRM names that chain and that sender.
# It never reads a private key or a password. The sender is --sender: unlocked at a local node, or on 4663 a hardware wallet
# (--ledger / --trezor) or a Foundry keystore (--account <name>: ~/.foundry/keystores/<name>, whose address must be the deployer's;
# forge asks for its password when it signs — docs/v6/phase1-scope.md §8.1).
#
#   usage:  ETCH_DEPLOY_CONFIRM=deploy-etch:<chainid>:<deployer> ETCH_OWNER=0x… ETCH_TREASURY=0x… ETCH_KEEPER=0x… \
#           ETCH_PURCHASER=0x… ETCH_ATTESTOR=0x…  script/deploy.sh <rpc-url> <deployer-address> [--ledger|--trezor|--account <name>] [--label test]
#
# docs/phase6.md. `--label test` is a SMALL REAL TEST on 4663: its confirm string is deploy-etch-test:4663:<deployer>, it writes
# deployments/test-4663.json and never 4663.json, and its owner may be the tester's own hardware wallet. The chain in staging/chain-id.json is our
# staging fork: it is deployed to only through a node on THIS machine, from an address that node sends for.
set -euo pipefail
# die, the refusal of a key (or a keystore variable) in the environment, and the 4663 signer check (send_4663): one copy, shared
source "$(dirname "$0")/chain-guard.sh"

RPC="${1:-}"; DEPLOYER="${2:-}"; shift $(( $# < 2 ? $# : 2 ))
SIGNER=""; LABEL=""; ACCOUNT=""
while [ $# -gt 0 ]; do
  case "$1" in
    --ledger|--trezor) [ -z "$SIGNER" ] || die "one signer flag"; SIGNER="$1";;
    --account) [ -z "$SIGNER" ] || die "one signer flag"; [ $# -ge 2 ] || die "--account needs a keystore name"; SIGNER="$1"; ACCOUNT="$2"; shift;;
    --label) LABEL="${2:-}"; shift; [ "$LABEL" = "test" ] || die "the only label is 'test'";;
    *) die "unknown argument $1";;
  esac; shift
done
[ -n "$RPC" ] && [ -n "$DEPLOYER" ] || die "usage: script/deploy.sh <rpc-url> <deployer-address> [--ledger|--trezor|--account <name>]"
echo "$DEPLOYER" | grep -Eq '^0x[0-9a-fA-F]{40}$' || die "the deployer is not an address"

HEX=$(curl -s -m 15 -X POST -H 'content-type: application/json' --data '{"jsonrpc":"2.0","id":1,"method":"eth_chainId","params":[]}' "$RPC" \
  | sed -n 's/.*"result":"\(0x[0-9a-fA-F]*\)".*/\1/p' || true)
[ -n "$HEX" ] || die "the RPC did not report a chain id"
CHAIN=$((HEX))
# our staging fork's id comes from the repository's one source; it is never a real network's number
STAGING=$(sed -n 's/.*"chainId"[^0-9]*\([0-9][0-9]*\).*/\1/p' "$(dirname "$0")/../staging/chain-id.json" | head -1)
[ -n "$STAGING" ] || die "staging/chain-id.json does not name a chain id"
case "$STAGING" in 4663|46630|31337|1) die "staging/chain-id.json names chain $STAGING, which is not ours alone";; esac
[ "$CHAIN" != "46630" ] || die "chain 46630 is Robinhood's public testnet, not our staging fork ($STAGING): this script does not deploy there"
case "$CHAIN" in 4663|31337|"$STAGING") ;; *) die "chain $CHAIN is not one this script deploys to (4663, 31337, or our staging fork $STAGING)";; esac
if [ "$CHAIN" = "$STAGING" ]; then
  HOST=$(echo "$RPC" | sed -E 's#^[a-z]+://([^/:]+).*#\1#')
  case "$HOST" in 127.0.0.1|localhost) ;; *) die "the staging fork is deployed to from the machine that runs it, not through $HOST";; esac
fi
[ -z "$LABEL" ] || [ "$CHAIN" = "4663" ] || die "--label test is a small real test on 4663; chain $CHAIN is not that"

LOWER=$(echo "$DEPLOYER" | tr 'A-F' 'a-f')
if [ "$LABEL" = "test" ]; then EXPECTED="deploy-etch-test:${CHAIN}:${LOWER}"; else EXPECTED="deploy-etch:${CHAIN}:${LOWER}"; fi
OUT="deployments/${LABEL:+test-}${CHAIN}.json"
[ "$CHAIN" = "31337" ] || [ "$CHAIN" = "$STAGING" ] || [ ! -e "$(dirname "$0")/../$OUT" ] || die "$OUT already exists. A deployment file is never overwritten on a real chain: move it aside yourself if you mean to."
[ "${ETCH_DEPLOY_CONFIRM:-}" = "$EXPECTED" ] || die "ETCH_DEPLOY_CONFIRM must be exactly '$EXPECTED' for this RPC and this sender"
[ -n "${ETCH_OWNER:-}" ] || die "ETCH_OWNER (the multisig) is not set"
[ "$(echo "$ETCH_OWNER" | tr 'A-F' 'a-f')" != "$LOWER" ] || die "the owner must not be the deployer"

if [ "$CHAIN" = "31337" ] || [ "$CHAIN" = "$STAGING" ]; then
  [ -z "$SIGNER" ] || die "a local deployment is sent from the node's unlocked accounts; no signer flag"
  SEND=(--unlocked)
else
  send_4663 "$SIGNER" "$ACCOUNT" "$DEPLOYER"
  echo "About to deploy etch${LABEL:+ (a SMALL REAL TEST, unaudited, written to $OUT)} to chain $CHAIN from $DEPLOYER, signed by $(signer_words "$SIGNER" "$ACCOUNT"), owned by $ETCH_OWNER. Type the chain id to go on:"; read -r TYPED
  [ "$TYPED" = "$CHAIN" ] || die "not confirmed"
fi

export PATH="$HOME/.foundry/bin:$PATH"
cd "$(dirname "$0")/.."
# Robinhood Chain is an Arbitrum chain: inside a contract block.number is the L1 block. Log scans need the chain's own number,
# so it is read here, before anything is deployed, and written into the deployment file afterwards.
L2HEX=$(curl -s -m 15 -X POST -H 'content-type: application/json' --data '{"jsonrpc":"2.0","id":1,"method":"eth_blockNumber","params":[]}' "$RPC" \
  | sed -n 's/.*"result":"\(0x[0-9a-fA-F]*\)".*/\1/p')
# Robinhood Chain's own code-size limit, as its node states it ("max code size exceeded: ... limit 98304", eth_estimateGas,
# 2026-09-23). ListingRegistry is 40,845 bytes (2026-09-23, after the single-clock fix): it creates both vault kinds, so it carries both their init codes. Forge's default is
# Ethereum's 24,576, which would stop this deployment on the day for a limit this chain does not have (docs/deploy/contracts-mainnet.md).
ETCH_DEPLOY_LABEL="$LABEL" forge script script/Deploy.s.sol:Deploy --rpc-url "$RPC" --sender "$DEPLOYER" "${SEND[@]}" --broadcast --slow \
  --code-size-limit 98304
python3 - "$OUT" "$((L2HEX))" <<'PY'
import json, sys
path = sys.argv[1]
d = json.load(open(path)); d["fromBlock"] = int(sys.argv[2])
json.dump(d, open(path, "w"), indent=2, sort_keys=True); open(path, "a").write("\n")
print(f"wrote {path} (fromBlock {d['fromBlock']})")
PY
