#!/usr/bin/env bash
# docs/v6/deploy-stack.md. The only supported way to run script/DeployStack.s.sol: the protocol treasury, the collection directory,
# and per collection the wrapper, the mint desk, the floor module, the liquidity lock, the curve factory, the router, the three
# policy vaults and the offer board — against an EXISTING, wired ListingRegistry and one of its inscriptions listings.
#   usage:  ETCH_STACK_CONFIRM=deploy-etch-stack:<chainid>:<deployer> ETCH_REGISTRY=0x… ETCH_WRAPPER_LISTING=<id> \
#           ETCH_WRAPPER_NAME=… ETCH_WRAPPER_SYMBOL=… ETCH_WRAPPER_CUSTODIAN=0x… ETCH_WRAPPER_CUSTODY_ADDRESS=bc1p… [ETCH_STAGE=0] \
#           [ETCH_OPS=0x…] [ETCH_CAP_CEILING=<wei>] [ETCH_NFT_CHAIN_ID=… ETCH_NFT_COLLECTION=0x… ETCH_NFT_NAME=… ETCH_NFT_SYMBOL=…] \
#           script/deploy-stack.sh [<rpc-url>] <deployer-address> [--ledger|--trezor|--account <name>]
# With no RPC it deploys to the local node on port 8604. On 4663 the sender must be a hardware wallet or a Foundry keystore whose
# address is the deployer's (--account, docs/v6/phase1-scope.md §8.1), and the chain id is typed.
# It never reads a private key or a password. It writes deployments/stack-<chainid>.json, which is also the indexer's WRAPPED_CONFIG.
set -euo pipefail
source "$(dirname "$0")/chain-guard.sh"
if [ "${1:-}" != "" ] && ! echo "$1" | grep -Eq '^0x'; then RPC="$1"; shift; else RPC="$LOCAL_RPC"; fi
DEPLOYER="${1:-}"; shift || true
SIGNER=""; ACCOUNT=""
while [ $# -gt 0 ]; do
  case "$1" in
    --ledger|--trezor) [ -z "$SIGNER" ] || die "one signer flag"; SIGNER="$1";;
    --account) [ -z "$SIGNER" ] || die "one signer flag"; [ $# -ge 2 ] || die "--account needs a keystore name"; SIGNER="$1"; ACCOUNT="$2"; shift;;
    *) die "unknown argument $1";;
  esac; shift
done
echo "$DEPLOYER" | grep -Eq '^0x[0-9a-fA-F]{40}$' || die "usage: script/deploy-stack.sh [<rpc-url>] <deployer-address> [--ledger|--trezor|--account <name>]"
CHAIN=$(chain_of "$RPC"); guard_chain "$RPC" "$CHAIN"
LOWER=$(echo "$DEPLOYER" | tr 'A-F' 'a-f')
[ "${ETCH_STACK_CONFIRM:-}" = "deploy-etch-stack:${CHAIN}:${LOWER}" ] \
  || die "ETCH_STACK_CONFIRM must be exactly 'deploy-etch-stack:${CHAIN}:${LOWER}' for this RPC and this sender"
[ -n "${ETCH_REGISTRY:-}" ] || die "ETCH_REGISTRY (the wired ListingRegistry) is not set"
[ -n "${ETCH_WRAPPER_LISTING:-}" ] || die "ETCH_WRAPPER_LISTING (the inscriptions listing id) is not set"
# launch-gates G7: a first deployment on a public chain starts at stage 0; the Solidity check refuses too, this is the earlier word
if ! is_ours "$CHAIN" && [ "${ETCH_STAGE:-0}" != "0" ]; then die "custody caps start at stage 0 on chain $CHAIN (docs/v5/launch-gates.md G7); ETCH_STAGE=${ETCH_STAGE} is a raise, not a deployment"; fi
OUT="deployments/stack-${CHAIN}.json"
is_ours "$CHAIN" || [ ! -e "$(dirname "$0")/../$OUT" ] || die "$OUT already exists. A stack file is never overwritten on a real chain: move it aside yourself if you mean to."
if is_ours "$CHAIN"; then
  [ -z "$SIGNER" ] || die "a local deployment is sent from the node's unlocked accounts; no signer flag"
  SEND=(--unlocked)
else
  send_4663 "$SIGNER" "$ACCOUNT" "$DEPLOYER"
  echo "About to deploy the whole stack (treasury, directory, wrapper, desk, floor module, lock, factory, router, three vaults, offer board) over listing ${ETCH_WRAPPER_LISTING} of registry ${ETCH_REGISTRY} on chain $CHAIN from $DEPLOYER, signed by $(signer_words "$SIGNER" "$ACCOUNT"), at stage ${ETCH_STAGE:-0}. Type the chain id to go on:"; read -r TYPED
  [ "$TYPED" = "$CHAIN" ] || die "not confirmed"
fi
export PATH="$HOME/.foundry/bin:$PATH"
cd "$(dirname "$0")/.."
# the chain's own height, read before anything is sent: what eth_getLogs understands (deployments/README.md)
L2HEX=$(curl -s -m 15 -X POST -H 'content-type: application/json' --data '{"jsonrpc":"2.0","id":1,"method":"eth_blockNumber","params":[]}' "$RPC" \
  | sed -n 's/.*"result":"\(0x[0-9a-fA-F]*\)".*/\1/p')
[ -n "$L2HEX" ] || die "the RPC did not report a block number"
# Wrapper, NftWrapper and SweepVault are each over Ethereum's 24,576 and under Robinhood Chain's 98,304 (script/deploy.sh says why)
forge script script/DeployStack.s.sol:DeployStack --rpc-url "$RPC" --sender "$DEPLOYER" "${SEND[@]}" --broadcast --slow --code-size-limit 98304
python3 - "$OUT" "$((L2HEX))" <<'PY'
import json, sys
path, from_block = sys.argv[1], int(sys.argv[2])
d = json.load(open(path))
# the script writes the collections keyed by slug (what forge's serializer can nest); the indexer's WRAPPED_CONFIG wants a list
cols = d["collections"]
if isinstance(cols, dict): cols = [cols[k] for k in sorted(cols)]
for c in cols:
    c["fromBlock"] = from_block
    # a zero address means "none" (an NFT collection has no offer board): the config reader wants absence, not a zero
    for k in list(c):
        if c[k] == "0x0000000000000000000000000000000000000000": c[k] = None
d["collections"] = cols
d["fromBlock"] = from_block
# docs/v7/phase1-nft.md §3.2: an nft collection's floor is posted by the canister (its PosterSource), never by the keeper
d["forKeeper"] = { "floorModules": [c["floorModule"] for c in cols if c.get("kind") != "nft"], "sweepVaults": [v for c in cols for v in (c["raffleVault"], c["burnVault"], c["yoyoVault"])],
                "offerBoards": [c["offerBoard"] for c in cols if c.get("offerBoard")] }
d["forCanister"] = { "ethNft": [{ "collection": c["custody"]["collection"], "wrapper": c["wrapper"], "module": c["floorModule"],
                     "chainId": c["custody"]["chainId"] } for c in cols if c.get("kind") == "nft"] }
json.dump(d, open(path, "w"), indent=2, sort_keys=True); open(path, "a").write("\n")
print(f"wrote {path} (fromBlock {from_block}, {len(cols)} collection(s))")
PY
