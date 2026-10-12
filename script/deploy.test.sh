#!/usr/bin/env bash
# docs/phase2.md §1: the wrapper's refusals, against a throwaway local node that has nothing on it. Nothing is deployed by this test.
set -uo pipefail
export PATH="$HOME/.foundry/bin:$PATH"
cd "$(dirname "$0")/.."
PORT=8599; RPC="http://127.0.0.1:$PORT"
anvil --chain-id 31337 --port $PORT --silent & NODE=$!
trap 'kill $NODE 2>/dev/null' EXIT
for i in $(seq 1 40); do curl -s -m 1 -X POST -H 'content-type: application/json' --data '{"jsonrpc":"2.0","id":1,"method":"eth_chainId","params":[]}' $RPC >/dev/null && break; sleep 0.25; done
D=0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266; O=0x70997970C51812dc3A010C7d01b50e0d17dc79C8
OK="deploy-etch:31337:0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266"
fail=0
expect() { # name, pattern, then the command
  local name="$1" pat="$2"; shift 2
  out=$("$@" 2>&1); code=$?
  if [ $code -ne 0 ] && echo "$out" | grep -q "$pat"; then echo "ok   $name"; else echo "FAIL $name: exit $code: $(echo "$out" | tail -1)"; fail=1; fi
}
expect "refuses without the confirm string"            "ETCH_DEPLOY_CONFIRM must be exactly" env -u ETCH_DEPLOY_CONFIRM ETCH_OWNER=$O script/deploy.sh $RPC $D
expect "refuses a confirm typed for another chain"     "ETCH_DEPLOY_CONFIRM must be exactly 'deploy-etch:31337" env ETCH_DEPLOY_CONFIRM="deploy-etch:4663:0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266" ETCH_OWNER=$O script/deploy.sh $RPC $D
expect "refuses a confirm typed for another deployer"  "ETCH_DEPLOY_CONFIRM must be exactly" env ETCH_DEPLOY_CONFIRM="deploy-etch:31337:0x70997970c51812dc3a010c7d01b50e0d17dc79c8" ETCH_OWNER=$O script/deploy.sh $RPC $D
expect "refuses when a private key is in the environment" "does not take a key" env PRIVATE_KEY=anything ETCH_DEPLOY_CONFIRM=$OK ETCH_OWNER=$O script/deploy.sh $RPC $D
expect "refuses an owner who is the deployer"          "owner must not be the deployer" env ETCH_DEPLOY_CONFIRM=$OK ETCH_OWNER=$D script/deploy.sh $RPC $D
expect "refuses with no owner"                         "ETCH_OWNER" env -u ETCH_OWNER ETCH_DEPLOY_CONFIRM=$OK script/deploy.sh $RPC $D
expect "refuses a signer flag on the local chain"      "no signer flag" env ETCH_DEPLOY_CONFIRM=$OK ETCH_OWNER=$O script/deploy.sh $RPC $D --ledger
expect "refuses an RPC that reports no chain"          "did not report a chain id" env ETCH_DEPLOY_CONFIRM=$OK ETCH_OWNER=$O script/deploy.sh http://127.0.0.1:1 $D
# past the wrapper, the Solidity script refuses by itself: this bare node has no Pons factory
expect "the script itself refuses where Pons is not deployed" "PonsMissing\|script failed\|revert" env ETCH_DEPLOY_CONFIRM=$OK ETCH_OWNER=$O ETCH_TREASURY=0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC ETCH_KEEPER=0x90F79bf6EB2c4f870365E785982E1f101E93b906 ETCH_PURCHASER=0x15d34AAf54267DB7D7c367839AAf71A00a2C6A65 ETCH_ATTESTOR=0x9965507D1a55bcC2695C58ba16FB37d819B0A4dc script/deploy.sh $RPC $D
# docs/phase6.md: the test label, and the staging fork
expect "refuses --label test off Robinhood Chain"      "small real test on 4663" env ETCH_DEPLOY_CONFIRM=$OK ETCH_OWNER=$O script/deploy.sh $RPC $D --label test
expect "refuses a label that is not 'test'"            "the only label is" env ETCH_DEPLOY_CONFIRM=$OK ETCH_OWNER=$O script/deploy.sh $RPC $D --label mainnet
expect "refuses an argument it does not know"          "unknown argument" env ETCH_DEPLOY_CONFIRM=$OK ETCH_OWNER=$O script/deploy.sh $RPC $D --private-key 0x01
SPORT=8598
SID=$(sed -n 's/.*"chainId"[^0-9]*\([0-9][0-9]*\).*/\1/p' staging/chain-id.json | head -1)
anvil --chain-id $SID --port $SPORT --host 0.0.0.0 --silent & SNODE=$!
trap 'kill $NODE $SNODE 2>/dev/null' EXIT
for i in $(seq 1 40); do curl -s -m 1 -X POST -H 'content-type: application/json' --data '{"jsonrpc":"2.0","id":1,"method":"eth_chainId","params":[]}' http://127.0.0.1:$SPORT >/dev/null && break; sleep 0.25; done
SOK="deploy-etch:$SID:0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266"
expect "staging is deployed to from this machine only"  "from the machine that runs it" env ETCH_DEPLOY_CONFIRM=$SOK ETCH_OWNER=$O script/deploy.sh http://0.0.0.0:$SPORT $D
expect "staging refuses Robinhood Chain's confirm"      "ETCH_DEPLOY_CONFIRM must be exactly 'deploy-etch:$SID" env ETCH_DEPLOY_CONFIRM="deploy-etch:4663:0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266" ETCH_OWNER=$O script/deploy.sh http://127.0.0.1:$SPORT $D
expect "staging refuses a hardware-wallet flag"         "no signer flag" env ETCH_DEPLOY_CONFIRM=$SOK ETCH_OWNER=$O script/deploy.sh http://127.0.0.1:$SPORT $D --ledger
# Robinhood's PUBLIC TESTNET reports 46630: a real network, and never our staging fork
TPORT=8597
anvil --chain-id 46630 --port $TPORT --silent & TNODE=$!
trap 'kill $NODE $SNODE $TNODE 2>/dev/null' EXIT
for i in $(seq 1 40); do curl -s -m 1 -X POST -H 'content-type: application/json' --data '{"jsonrpc":"2.0","id":1,"method":"eth_chainId","params":[]}' http://127.0.0.1:$TPORT >/dev/null && break; sleep 0.25; done
expect "refuses Robinhood's public testnet (46630)"     "public testnet" env ETCH_DEPLOY_CONFIRM="deploy-etch:46630:0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266" ETCH_OWNER=$O script/deploy.sh http://127.0.0.1:$TPORT $D
# docs/v6/phase1-scope.md §8.1 (K8): the keystore signer. On our own chains every signer flag is refused, --account included, and
# forge's own keystore variables are refused on every chain.
expect "refuses --account on the local chain"          "no signer flag" env ETCH_DEPLOY_CONFIRM=$OK ETCH_OWNER=$O script/deploy.sh $RPC $D --account rook-deployer
expect "refuses --account with no name"                "needs a keystore name" env ETCH_DEPLOY_CONFIRM=$OK ETCH_OWNER=$O script/deploy.sh $RPC $D --account
expect "refuses two signer flags"                      "one signer flag" env ETCH_DEPLOY_CONFIRM=$OK ETCH_OWNER=$O script/deploy.sh $RPC $D --ledger --account rook-deployer
for v in ETH_KEYSTORE ETH_KEYSTORE_ACCOUNT ETH_PASSWORD; do
  expect "refuses $v in the environment"            "$v is set in the environment" env $v=anything ETCH_DEPLOY_CONFIRM=$OK ETCH_OWNER=$O script/deploy.sh $RPC $D
done
# send_4663 itself (script/chain-guard.sh), against a throwaway HOME whose keystore files hold an address and no key material at all.
# A stand-in `cast` first on the PATH plays cast opening a keystore that does not record its address.
KH=$(mktemp -d); KS="$KH/.foundry/keystores"; mkdir -p "$KS" "$KH/bin" "$KH/elsewhere"
trap 'kill $NODE $SNODE $TNODE 2>/dev/null; rm -rf "$KH"' EXIT
echo '{"address":"f39fd6e51aad88f6f4ce6ab8827279cfffb92266","crypto":{},"version":3}' > "$KS/rook-deployer"
echo '{"address":"0xF39FD6E51AAD88F6F4CE6AB8827279CFFFB92266","crypto":{},"version":3}' > "$KS/upper-0x"
echo '{"address":"70997970c51812dc3a010c7d01b50e0d17dc79c8","crypto":{},"version":3}' > "$KS/someone-else"
echo '{"crypto":{},"version":3}' > "$KS/no-address"
cp "$KS/rook-deployer" "$KH/elsewhere/only-elsewhere"
signer() { # signer <PATH prefix or -> <cast says> <args to send_4663…>: prints SEND, or the refusal
  local castsays="$1"; shift
  printf '#!/bin/sh\n[ -n "%s" ] && echo "%s" || exit 1\n' "$castsays" "$castsays" > "$KH/bin/cast"; chmod +x "$KH/bin/cast"
  env HOME="$KH" PATH="$KH/bin:$PATH" bash -c 'source script/chain-guard.sh; send_4663 "$@"; echo "SEND=${SEND[*]}"' _ "$@" </dev/null 2>&1
}
accepts() { local name="$1" want="$2"; shift 2; local out; out=$(signer "$@"); if echo "$out" | grep -qx "SEND=$want"; then echo "ok   $name"; else echo "FAIL $name: $(echo "$out" | tail -1)"; fail=1; fi; }
refuses() { local name="$1" pat="$2"; shift 2; local out; out=$(signer "$@"); if echo "$out" | grep -q "^refused: .*$pat" && ! echo "$out" | grep -q '^SEND='; then echo "ok   $name"; else echo "FAIL $name: $(echo "$out" | tail -1)"; fail=1; fi; }
refuses "4663: no signer is refused"                          "hardware wallet (--ledger / --trezor) or a Foundry keystore" "" "" "" $D
accepts "4663: a Ledger is still accepted"                     "--ledger" "" --ledger "" $D
accepts "4663: a Trezor is still accepted"                     "--trezor" "" --trezor "" $D
accepts "4663: a keystore whose address is the deployer's"     "--account rook-deployer" "" --account rook-deployer $D
accepts "4663: the keystore's address in capitals, with 0x"    "--account upper-0x" "" --account upper-0x $D
refuses "4663: a keystore for another address"                 "is for 0x70997970c51812dc3a010c7d01b50e0d17dc79c8, not the deployer" "" --account someone-else $D
refuses "4663: a keystore that does not exist"                 "no keystore named 'nobody'" "" --account nobody $D
refuses "4663: a keystore name that climbs out"                "keystore name" "" --account ../rook-deployer $D
refuses "4663: a keystore name that looks like a flag"         "keystore name" "" --account -rook $D
refuses "4663: a keystore only in FOUNDRY_KEYSTORES_DIR (forge would not read it)" "no keystore named 'only-elsewhere'" "" --account only-elsewhere $D
accepts "4663: a keystore with no address, cast names the deployer" "--account no-address" "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266" --account no-address $D
refuses "4663: a keystore with no address, cast names another"  "is for 0x70997970c51812dc3a010c7d01b50e0d17dc79c8" "0x70997970C51812dc3A010C7d01b50e0d17dc79C8" --account no-address $D
refuses "4663: a keystore with no address that cast cannot open" "could not read the address" "" --account no-address $D
# FOUNDRY_KEYSTORES_DIR: refused when it names another directory (forge would not read it), accepted when it names forge's own
out=$(FOUNDRY_KEYSTORES_DIR="$KH/elsewhere" signer "" --account rook-deployer $D)
echo "$out" | grep -q "^refused: FOUNDRY_KEYSTORES_DIR is" && echo "ok   4663: FOUNDRY_KEYSTORES_DIR set elsewhere is refused" || { echo "FAIL FOUNDRY_KEYSTORES_DIR elsewhere: $(echo "$out" | tail -1)"; fail=1; }
out=$(FOUNDRY_KEYSTORES_DIR="$KS" signer "" --account rook-deployer $D)
echo "$out" | grep -qx "SEND=--account rook-deployer" && echo "ok   4663: FOUNDRY_KEYSTORES_DIR set to forge's own directory is accepted" || { echo "FAIL FOUNDRY_KEYSTORES_DIR = forge's: $(echo "$out" | tail -1)"; fail=1; }
[ -e deployments/46630.json ] && { echo "FAIL a refusal wrote deployments/46630.json"; fail=1; }
[ -e deployments/31337.json.test-marker ] && fail=1
[ -e deployments/$SID.json ] && { echo "FAIL a refusal wrote deployments/$SID.json"; fail=1; }
[ -e deployments/test-4663.json ] && { echo "FAIL a refusal wrote deployments/test-4663.json"; fail=1; }
exit $fail
