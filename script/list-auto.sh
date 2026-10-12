#!/usr/bin/env bash
# docs/deploy/contracts-mainnet.md, step 4. Runs script/ListAuto.s.sol: names an autonomous buyer in a new listing.
#   usage:  ETCH_REGISTRY=0x… ETCH_LIST_KIND=inscriptions|rune ETCH_LIST_NAME=… ETCH_LIST_REF=… [ETCH_LIST_ROOT=0x… | ETCH_LIST_DIVISIBILITY=n]
#           ETCH_LIST_BTC_VAULT=… ETCH_BUYER=0x… ETCH_BUYER_CONFIRM=buyer-is:<chainid>:<buyer> ETCH_MAX_IN_FLIGHT=<wei> \
#           script/list-auto.sh [<rpc-url>]
# On our own chains it makes the call from the owner's unlocked account. On 4663 it SENDS NOTHING: it simulates, then prints the
# calldata for the multisig — the owner is the multisig, and nobody else can list.
set -euo pipefail
source "$(dirname "$0")/chain-guard.sh"
RPC="${1:-$LOCAL_RPC}"
CHAIN=$(chain_of "$RPC"); guard_chain "$RPC" "$CHAIN"
export PATH="$HOME/.foundry/bin:$PATH"
cd "$(dirname "$0")/.."
if is_ours "$CHAIN"; then
  OWNER=$(cast call --rpc-url "$RPC" "${ETCH_REGISTRY:?ETCH_REGISTRY}" "owner()(address)")
  forge script script/ListAuto.s.sol:ListAuto --rpc-url "$RPC" --sender "$OWNER" --unlocked --broadcast --slow --code-size-limit 98304
else
  forge script script/ListAuto.s.sol:ListAuto --rpc-url "$RPC" --code-size-limit 98304      # no --broadcast: a simulation that prints the multisig's call
fi
