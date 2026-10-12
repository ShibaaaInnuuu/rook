#!/usr/bin/env bash
# docs/deploy/contracts-mainnet.md, step 6. The only supported way to run script/DeployWrapClaim.s.sol: WrapClaim, which
# deploys its own Wrapper (src/WrapClaim.sol's constructor), so one confirm covers the pair.
#   usage:  ETCH_WRAPPER_CONFIRM=deploy-etch-wrapper:<chainid>:<deployer> ETCH_REGISTRY=0x… ETCH_WRAPPER_LISTING=<id> \
#           ETCH_WRAPPER_NAME=… ETCH_WRAPPER_SYMBOL=… \
#           script/deploy-wrapclaim.sh [<rpc-url>] <deployer-address> [--ledger|--trezor|--account <name>] [--label test]
# With no RPC it deploys to the local node on port 8604. On 4663 the sender must be a hardware wallet or a Foundry keystore whose
# address is the deployer's (--account, docs/v6/phase1-scope.md §8.1), and the chain id is typed.
set -euo pipefail
source "$(dirname "$0")/chain-guard.sh"
if [ "${1:-}" != "" ] && ! echo "$1" | grep -Eq '^0x'; then RPC="$1"; shift; else RPC="$LOCAL_RPC"; fi
DEPLOYER="${1:-}"; shift || true
SIGNER=""; LABEL=""; ACCOUNT=""
while [ $# -gt 0 ]; do
  case "$1" in
    --ledger|--trezor) [ -z "$SIGNER" ] || die "one signer flag"; SIGNER="$1";;
    --account) [ -z "$SIGNER" ] || die "one signer flag"; [ $# -ge 2 ] || die "--account needs a keystore name"; SIGNER="$1"; ACCOUNT="$2"; shift;;
    --label) LABEL="${2:-}"; shift; [ "$LABEL" = "test" ] || die "the only label is 'test'";;
    *) die "unknown argument $1";;
  esac; shift
done
echo "$DEPLOYER" | grep -Eq '^0x[0-9a-fA-F]{40}$' || die "usage: script/deploy-wrapclaim.sh [<rpc-url>] <deployer-address> [--ledger|--trezor|--account <name>]"
CHAIN=$(chain_of "$RPC"); guard_chain "$RPC" "$CHAIN"
[ -z "$LABEL" ] || [ "$CHAIN" = "4663" ] || die "--label test is a small real test on 4663; chain $CHAIN is not that"
LOWER=$(echo "$DEPLOYER" | tr 'A-F' 'a-f')
[ "${ETCH_WRAPPER_CONFIRM:-}" = "deploy-etch-wrapper:${CHAIN}:${LOWER}" ] \
  || die "ETCH_WRAPPER_CONFIRM must be exactly 'deploy-etch-wrapper:${CHAIN}:${LOWER}' for this RPC and this sender"
if is_ours "$CHAIN"; then
  [ -z "$SIGNER" ] || die "a local deployment is sent from the node's unlocked accounts; no signer flag"
  SEND=(--unlocked)
else
  send_4663 "$SIGNER" "$ACCOUNT" "$DEPLOYER"
  echo "About to deploy WrapClaim and its Wrapper over listing ${ETCH_WRAPPER_LISTING:-?} to chain $CHAIN from $DEPLOYER, signed by $(signer_words "$SIGNER" "$ACCOUNT"). Type the chain id to go on:"; read -r TYPED
  [ "$TYPED" = "$CHAIN" ] || die "not confirmed"
fi
export PATH="$HOME/.foundry/bin:$PATH"
cd "$(dirname "$0")/.."
ETCH_DEPLOY_LABEL="$LABEL" forge script script/DeployWrapClaim.s.sol:DeployWrapClaim --rpc-url "$RPC" --sender "$DEPLOYER" "${SEND[@]}" --broadcast --slow --code-size-limit 98304
