#!/usr/bin/env bash
# docs/v4/nft.md. The only supported way to run script/DeployNft.s.sol: the NFT registry and the NFT launcher, against an EXISTING
# ListingRegistry that is already wired (its Distributor names the beacon).
#   usage:  ETCH_NFT_CONFIRM=deploy-etch-nft:<chainid>:<deployer> ETCH_REGISTRY=0x… [ETCH_NFT_BUYER=0x…] \
#           script/deploy-nft.sh [<rpc-url>] <deployer-address> [--ledger|--trezor]
# With no RPC it deploys to the local node on port 8604. On 4663 the sender must be a hardware wallet and the chain id is typed.
set -euo pipefail
source "$(dirname "$0")/chain-guard.sh"
if [ "${1:-}" != "" ] && ! echo "$1" | grep -Eq '^0x'; then RPC="$1"; shift; else RPC="$LOCAL_RPC"; fi
DEPLOYER="${1:-}"; shift || true
SIGNER=""
while [ $# -gt 0 ]; do
  case "$1" in
    --ledger|--trezor) [ -z "$SIGNER" ] || die "one signer flag"; SIGNER="$1";;
    *) die "unknown argument $1";;
  esac; shift
done
echo "$DEPLOYER" | grep -Eq '^0x[0-9a-fA-F]{40}$' || die "usage: script/deploy-nft.sh [<rpc-url>] <deployer-address> [--ledger|--trezor]"
CHAIN=$(chain_of "$RPC"); guard_chain "$RPC" "$CHAIN"
LOWER=$(echo "$DEPLOYER" | tr 'A-F' 'a-f')
[ "${ETCH_NFT_CONFIRM:-}" = "deploy-etch-nft:${CHAIN}:${LOWER}" ] \
  || die "ETCH_NFT_CONFIRM must be exactly 'deploy-etch-nft:${CHAIN}:${LOWER}' for this RPC and this sender"
if is_ours "$CHAIN"; then
  [ -z "$SIGNER" ] || die "a local deployment is sent from the node's unlocked accounts; no signer flag"
  SEND=(--unlocked)
else
  case "$SIGNER" in --ledger|--trezor) SEND=("$SIGNER");; *) die "on chain $CHAIN the sender must be a hardware wallet: pass --ledger or --trezor";; esac
  echo "About to deploy the NFT registry and launcher against registry ${ETCH_REGISTRY:-?} on chain $CHAIN from $DEPLOYER. Type the chain id to go on:"; read -r TYPED
  [ "$TYPED" = "$CHAIN" ] || die "not confirmed"
fi
export PATH="$HOME/.foundry/bin:$PATH"
cd "$(dirname "$0")/.."
# NftRegistry is ~42.8 KB (it creates both NFT vault kinds, the sweep vault with its Seaport decoder): over Ethereum's 24,576, under Robinhood Chain's 98,304.
forge script script/DeployNft.s.sol:DeployNft --rpc-url "$RPC" --sender "$DEPLOYER" "${SEND[@]}" --broadcast --slow --code-size-limit 98304
