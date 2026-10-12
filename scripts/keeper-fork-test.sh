#!/usr/bin/env bash
# docs/v6/p1-services.md §4 ("Fork"). The production keeper (keeper/service/) against the whole stack on a LOCAL anvil fork of
# Robinhood Chain (4663), end to end, and its refusals. Nothing here reaches a public chain: the node is 127.0.0.1, every sender is
# an anvil dev account or an impersonated code-less address, and the keeper's key is anvil's dev account 3 (public, worthless).
#
#   scripts/keeper-fork-test.sh
#
# THE CHAIN ID. anvil's `--chain-id` flag sets the id the fork reports; this run keeps OUR staging id (staging/chain-id.json,
# 9466301), as scripts/golive-dryrun.sh `stack` does, so that nothing signed here is valid on 4663 (EIP-155 binds every signature
# to its chain id) and so that the deploy scripts, which refuse 4663 without a hardware wallet, run. The last step switches the
# node to REPORT 4663 (`anvil_setChainId`) and runs the keeper there in dry-run only: it simulates, it never signs for 4663.
#
# The fork is taken from the archive RPC rpc.ordofi.network (robinhood.drpc.org serves no historical state since 2026-09-24),
# KEEPER_FORK_BACK blocks behind its head (default 900,000, about 25 hours at 4663's ~0.1 s blocks: a draw's snapshot time is a
# moment of the day before its announcement, so the coin's holders must be a day old; step 5).
set -uo pipefail
export PATH="$HOME/.foundry/bin:$PATH"
cd "$(dirname "$0")/.."
ROOT=$(pwd)
PORT="${KEEPER_FORK_PORT:-8661}"; RPC="http://127.0.0.1:$PORT"
PORT2=$((PORT + 1)); RPC2="http://127.0.0.1:$PORT2"
UPSTREAM="${KEEPER_FORK_RPC:-https://rpc.ordofi.network}"
STAGING=$(sed -n 's/.*"chainId"[^0-9]*\([0-9][0-9]*\).*/\1/p' staging/chain-id.json | head -1)
WORK=$(mktemp -d "${TMPDIR:-/tmp}/keeper-fork.XXXXXX")
# anvil's dev accounts: D deploys, O owns, T treasury, K KEEPER (key below), P purchaser, A attestor, X a key that is no role
D=0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266; O=0x70997970C51812dc3A010C7d01b50e0d17dc79C8; T=0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC
K=0x90F79bf6EB2c4f870365E785982E1f101E93b906; P=0x15d34AAf54267DB7D7c367839AAf71A00a2C6A65; A=0x9965507D1a55bcC2695C58ba16FB37d819B0A4dc
K_KEY=0x7c852118294e51e653712a81e05800f419141751be58f605c371e15141b007a6          # anvil dev account 3: printed by every anvil at start
X_KEY=0x8b3a350cf5c34c9194ca85829a2df0ec3153be0318b5e2d3348e872092edffba          # anvil dev account 5: not the keeper
BUYER=0x0000000000000000000000000000000000b0b0e1                                    # the canister stand-in: code-less on 4663
CREATOR=0x00000000000000000000000000000000C0FFEE11; RBUYER=0x00000000000000000000000000000000C0ffee12; TRADER=0x00000000000000000000000000000000C0FfEE13
DRY_CUSTODY=bc1p5cyxnuxmeuwuvkwfem96lqzszd02n6xdcjrs20cac6yqjjwudpxqkedrcr
SLUG=keeper-fork-frogs
lc() { tr 'A-F' 'a-f'; }
fail=0; NODE=""; NODE2=""
KEEP=""; for f in deployments/$STAGING.json deployments/stack-$STAGING.json; do [ -e "$f" ] && KEEP="$KEEP $f"; done
tidy() { [ -z "$NODE" ] || kill "$NODE" 2>/dev/null; [ -z "$NODE2" ] || kill "$NODE2" 2>/dev/null; wait 2>/dev/null
  for f in deployments/$STAGING.json deployments/stack-$STAGING.json; do case " $KEEP " in *" $f "*) ;; *) rm -f "$f";; esac; done
  rm -f deployments/wrapper-"$STAGING"-*.json; [ "${KEEPER_FORK_KEEP_WORK:-}" = 1 ] || rm -rf "$WORK"; }
trap tidy EXIT
step() { printf '\n== %s\n' "$*"; }
ok() { echo "ok   $*"; }
bad() { echo "FAIL $*"; fail=1; }
must() { local name="$1"; shift; local out; if out=$("$@" 2>&1); then ok "$name"; LAST="$out"; return 0; else bad "$name: $(echo "$out" | grep -v '^\s*$' | tail -4)"; LAST="$out"; return 1; fi; }
call() { cast call --rpc-url $RPC "$@" | awk '{print $1}'; }
nonce() { cast nonce --rpc-url $RPC "$1"; }
# the keeper, once: prints its logs; the pass report is the JSON block after the last line that is exactly "{"
keeper() { (cd keeper && env -i PATH="$PATH" HOME="$HOME" KEEPER_CHAIN_ID="${KCHAIN:-$STAGING}" KEEPER_RPC_URLS="${KRPCS:-$RPC}" KEEPER_STACK="${KSTACK:-$ROOT/deployments/stack-$STAGING.json}" \
  KEEPER_STATE_DIR="${KSTATE:-$WORK/state}" KEEPER_FLOOR_SOURCE="${KSOURCE:-fixture}" KEEPER_FLOOR_FIXTURE="$WORK/floors.json" KEEPER_MAX_FEE_PER_GAS_WEI=5000000000 \
  KEEPER_MAX_WEI_PER_TX=2000000000000000 KEEPER_OW_SLUGS="$SLUG=bitcoin-frogs" KEEPER_DRAW_EVERY_S=300 KEEPER_DRAW_OFFSET_S="${KOFFSET:-0}" KEEPER_PUBLIC_URL="http://127.0.0.1:8790" \
  ${KMODE:+KEEPER_MODE=$KMODE} ${KKEY:+KEEPER_PRIVATE_KEY=$KKEY} npx tsx service/main.ts --once "$@" 2>&1); }
report() { python3 -c "
import sys,json
t=sys.stdin.read().split('\n')
i=max(j for j,l in enumerate(t) if l=='{')
print(json.dumps(json.loads('\n'.join(t[i:]))))"; }
q() { python3 -c "import sys,json; r=json.load(sys.stdin); print($1)"; }

step "Fork: $UPSTREAM, $((${KEEPER_FORK_BACK:-900000})) blocks behind its head, under our staging chain id $STAGING (anvil --chain-id), on port $PORT"
HEAD=$(cast block-number --rpc-url "$UPSTREAM") || { echo "FAIL the upstream did not answer"; exit 1; }
FORK_AT=$((HEAD - ${KEEPER_FORK_BACK:-900000}))
anvil --fork-url "$UPSTREAM" --fork-block-number $FORK_AT --chain-id "$STAGING" --host 127.0.0.1 --port $PORT --code-size-limit 98304 --auto-impersonate --silent & NODE=$!
for _ in $(seq 1 120); do [ "$(cast chain-id --rpc-url $RPC 2>/dev/null)" = "$STAGING" ] && break; sleep 0.5; done
[ "$(cast chain-id --rpc-url $RPC)" = "$STAGING" ] || { echo "FAIL the fork did not start"; exit 1; }
echo "     forked at block $FORK_AT (head $HEAD); the node reports chain id $(cast chain-id --rpc-url $RPC)"

step "The stack, with anvil dev account 3 as the keeper (deploy.sh, list-auto.sh, deploy-stack.sh at stage 0)"
must "script/deploy.sh" env ETCH_OWNER=$O ETCH_TREASURY=$T ETCH_KEEPER=$K ETCH_PURCHASER=$P ETCH_ATTESTOR=$A ETCH_DEPLOY_CONFIRM="deploy-etch:$STAGING:$(echo $D | lc)" script/deploy.sh $RPC $D || exit 1
R=$(python3 -c "import json;print(json.load(open('deployments/$STAGING.json'))['registry'])")
must "script/list-auto.sh" env ETCH_REGISTRY=$R ETCH_LIST_KIND=inscriptions ETCH_LIST_NAME="Keeper Fork Frogs" ETCH_LIST_REF=keeper-fork-frogs \
  ETCH_LIST_ROOT=0x$(printf 'keeper-fork-root' | shasum -a 256 | cut -c1-64) ETCH_LIST_BTC_VAULT=$DRY_CUSTODY ETCH_BUYER=$BUYER ETCH_MAX_IN_FLIGHT=1000000000000000000 \
  ETCH_BUYER_CONFIRM="buyer-is:$STAGING:$(echo $BUYER | lc)" script/list-auto.sh $RPC || exit 1
ID=$(( $(cast call --rpc-url $RPC $R 'count()(uint256)') - 1 ))
must "script/deploy-stack.sh" env ETCH_WRAPPER_NAME="Wrapped Keeper Frog" ETCH_WRAPPER_SYMBOL=wKFR ETCH_WRAPPER_CUSTODIAN=$BUYER ETCH_WRAPPER_CUSTODY_ADDRESS=$DRY_CUSTODY \
  ETCH_OPS=$T ETCH_STACK_SLUG=$SLUG ETCH_STACK_CONFIRM="deploy-etch-stack:$STAGING:$(echo $D | lc)" ETCH_REGISTRY=$R ETCH_WRAPPER_LISTING=$ID script/deploy-stack.sh $RPC $D || exit 1
S=deployments/stack-$STAGING.json
col() { python3 -c "import json;print(json.load(open('$S'))['collections'][0]['$1'])"; }
MOD=$(col floorModule); RAF=$(col raffleVault); BRN=$(col burnVault); YOY=$(col yoyoVault); FAC=$(col factory); ROUTER=$(col router)
[ "$(call $R 'keeper()(address)' | lc)" = "$(echo $K | lc)" ] && ok "registry.keeper() is dev account 3" || bad "registry.keeper() is not dev account 3"
[ "$(call $MOD 'keeper()(address)' | lc)" = "$(echo $K | lc)" ] && ok "the module's keeper (the registry's) is dev account 3" || bad "the module's keeper"
# the dev accounts carry EIP-7702 delegations on 4663, so on the fork the keeper has code, and the keeper REFUSES a key with code
KCODE=$(cast code --rpc-url $RPC $K)
echo "     the keeper's code on the fork before: ${KCODE:0:12}… (an EIP-7702 delegation designator)"
cast rpc --rpc-url $RPC anvil_setCode $K 0x >/dev/null; cast rpc --rpc-url $RPC anvil_setBalance $K 0xDE0B6B3A7640000 >/dev/null     # 1 ETH of play money
for a in $CREATOR $RBUYER $TRADER; do cast rpc --rpc-url $RPC anvil_setBalance $a 0x1B1AE4D6E2EF500000 >/dev/null; done
echo '{"collections":{"'$SLUG'":{"priceWei":"600000000000000000","confidenceBps":10000,"depthItems":50}}}' > "$WORK/floors.json"

step "1. dry run: every call simulated from registry.keeper(), nothing signed or sent"
N0=$(nonce $K)
OUT=$(keeper --dry-run); REP=$(echo "$OUT" | report)
[ "$(echo "$REP" | q "r['collections'][0]['floor']['result']['outcome']")" = simulated ] && ok "postFloor simulated" || bad "postFloor not simulated: $(echo "$REP" | q "r['collections'][0]['floor']")"
[ "$(echo "$REP" | q "','.join(c['result']['outcome'] for c in r['collections'][0]['caps'])")" = "simulated,simulated" ] && ok "postCap simulated on RAFFLE and BURN" || bad "caps: $(echo "$REP" | q "r['collections'][0]['caps']")"
[ "$(nonce $K)" = "$N0" ] && [ "$(call $MOD 'fresh()(bool)')" = false ] && ok "nothing moved: the keeper's nonce is $N0 and the module is still not fresh" || bad "the dry run moved something"
echo "$OUT" | grep -q '"event":"simulated"' && ! echo "$OUT" | grep -qi "$(echo ${K_KEY#0x})" && ok "the dry run never saw the key" || bad "dry run log"

step "2. send: the floor, then the caps on RAFFLE and BURN, mined; never YOYO"
OUT=$(KMODE=send KKEY=$K_KEY keeper); REP=$(echo "$OUT" | report)
echo "$REP" | q "'\n'.join('     '+x['what']+' -> '+(x['result'] or {}).get('outcome','-') for x in [r['collections'][0]['floor']['plan'] and {'what':'postFloor','result':r['collections'][0]['floor']['result']}]+[{'what':'postCap '+c['policy'],'result':c['result']} for c in r['collections'][0]['caps']])"
[ "$(call $MOD 'fresh()(bool)')" = true ] && ok "the module is fresh: accepted floor $(call $MOD 'accepted()(uint256)') wei" || bad "the module is not fresh"
[ "$(call $RAF 'capFresh()(bool)')" = true ] && [ "$(call $BRN 'capFresh()(bool)')" = true ] && ok "both caps fresh: RAFFLE $(call $RAF 'cap()(uint256)'), BURN $(call $BRN 'cap()(uint256)') wei (floor + 10%)" || bad "caps not fresh"
[ "$(call $YOY 'cap()(uint256)')" = 0 ] && ok "the YOYO vault has no cap" || bad "a cap reached the YOYO vault"
N1=$(nonce $K); echo "     the keeper's nonce: $N0 → $N1"
[ "$N1" = $((N0 + 3)) ] && ok "three transactions, receipts checked" || bad "expected 3 transactions, the nonce says $((N1 - N0))"
! echo "$OUT" | grep -qi "${K_KEY#0x}" && ok "the key appears in no log line" || bad "the key was printed"

step "3. a second pass, then a restart over the same state directory: nothing is re-posted"
OUT=$(KMODE=send KKEY=$K_KEY keeper); REP=$(echo "$OUT" | report)
[ "$(nonce $K)" = "$N1" ] && ok "second pass: floor $(echo "$REP" | q "r['collections'][0]['floor']['plan']['type']"), caps $(echo "$REP" | q "','.join(c['plan']['type'] for c in r['collections'][0]['caps'])"); nothing sent" || bad "the second pass sent"
OUT=$(KMODE=send KKEY=$K_KEY keeper)
[ "$(nonce $K)" = "$N1" ] && ok "a restart re-derives the same plan and sends nothing" || bad "the restart sent"

step "4. a coin launched, graduated through the module, and a sweep buys a whole wX at the router under the keeper's cap"
cast send --rpc-url $RPC --unlocked --from $CREATOR $FAC 'launch(string,string,uint8,bool,uint16)' "Keeper Fork Coin" KFC 0 false 100 >/dev/null && ok "launched (RAFFLE, 1% tier)" || bad "launch"
COIN=$(call $FAC "coins(uint256)(address)" $(( $(call $FAC 'coinCount()(uint256)') - 1 ))); CURVE=$(call $FAC "routerOf(address)(address)" $COIN)
RAISE=$(call $CURVE 'raise()(uint256)')
st() { call $CURVE 'state()(uint8)'; }
buy() { cast send --rpc-url $RPC --unlocked --from $RBUYER $CURVE 'buy(uint256)' 0 --value "$1" --json 2>&1 | python3 -c "import sys,json
t=sys.stdin.read()
try: print('status', int(json.loads(t)['status'],16))
except Exception: print('error', t.strip().splitlines()[-1][:200])"; }
echo "     the first buy (3 ETH), launch extra $(call $CURVE 'launchExtraBps()(uint256)') bps: $(buy 3000000000000000000); curve state $(st)"
echo "     the buy that reaches the raise (2 × $RAISE wei), launch extra $(call $CURVE 'launchExtraBps()(uint256)') bps: $(buy $(python3 -c "print($RAISE*2)")); curve state $(st)"
for _ in 1 2 3; do [ "$(st)" = 0 ] || break; echo "     still trading (progress $(cast call --rpc-url $RPC $CURVE 'progress()(uint256,uint256)' | tr '\n' ' ')): another buy $(buy $(python3 -c "print($RAISE*2)"))"; done
[ "$(st)" = 1 ] && ok "the raise is reached: the curve is graduating" || bad "the curve did not reach its raise (state $(st))"
[ "$(call $MOD 'isFilled(address)(bool)' $COIN)" = true ] && ok "the module filled the graduation at the keeper's floor" || bad "the module did not fill (stale floor?)"
cast send --rpc-url $RPC --unlocked --from $RBUYER $CURVE 'graduate()' >/dev/null && ok "graduated: $(call $MOD 'ethBackedUnits()(uint256)') ETH-backed units" || bad "graduate"
cast send --rpc-url $RPC --unlocked --from $TRADER $CURVE 'flushVaultFees()' >/dev/null
CREDIT=$(call $RAF 'credit(address)(uint256)' $COIN); SPEND=$(python3 -c "print(min($CREDIT, 800000000000000000))")
DEADLINE=$(( $(cast block --rpc-url $RPC latest -f timestamp) + 3600 ))
ROUTE=$(cast calldata 'buyWrapped(uint256,address,uint256)' 1 $RAF $DEADLINE)
must "sweep at the router (spend $SPEND of credit $CREDIT)" cast send --rpc-url $RPC --unlocked --from $TRADER $RAF 'sweep(address,uint256,bytes,uint256)' $COIN $SPEND $ROUTE 1000000000000000000
UNITS=$(call $RAF 'units(address)(uint256)' $COIN)
python3 -c "import sys; sys.exit(0 if $UNITS >= 10**18 else 1)" && ok "the vault holds $UNITS wX units for the coin" || bad "under one whole wX"

step "5. the draw: announced at a slot; opened with the seed round's REAL drand signature (two relays, the beacon verifies); the snapshot at the last block ≤ snapshotAt with the sentinel; recomputed by the tool; then the keyless resolver requests, resolves and claims"
# docs/audit/2026-09-26/fixes-W2-services.md §1–§2. The snapshot time is a moment of the DAY BEFORE the announcement, so the coin's
# holders must be more than a day old by then: the fork is taken ~25 h back (KEEPER_FORK_BACK), and the fork's clock is jumped to
# 20 minutes before the wall clock, so every drand round the draw pins already exists on the real network when the keeper asks.
python3 -m http.server 8790 --bind 127.0.0.1 --directory "$WORK/state" >/dev/null 2>&1 & WEB=$!      # the keeper's /snapshots/, as its status server serves them
mkdir -p "$WORK/state/snapshots"
cast rpc --rpc-url $RPC anvil_setNextBlockTimestamp $(( $(date +%s) - 1200 )) >/dev/null; cast rpc --rpc-url $RPC anvil_mine 1 >/dev/null
TS=$(cast block --rpc-url $RPC latest -f timestamp)
KOFFSET=$((TS % 300))
DRAWS0=$(call $RAF 'drawCount()(uint256)')
DRAW='draw(uint256)((uint8,address,uint64,uint64,uint64,uint64,uint64,uint64,uint64,bytes32,uint256,uint256,address,string,uint64,uint64,uint64,uint64))'
stage() { cast call --rpc-url $RPC $RAF "$DRAW" $DRAWS0 | sed 's/^(//' | awk -F', ' '{print $1}'; }
OUT=$(KOFFSET=$KOFFSET KMODE=send KKEY=$K_KEY keeper); REP=$(echo "$OUT" | report)
echo "$REP" | q "'\n'.join('     '+d['action']+': '+d['plan'] for d in r['collections'][0]['draws'])"
[ "$(call $RAF 'drawCount()(uint256)')" = $((DRAWS0 + 1)) ] && [ "$(stage)" = 7 ] && ok "draw $DRAWS0 announced (its unit reserved, a future seed round pinned)" || bad "no announcement: $(echo "$REP" | q "r['collections'][0]['draws']")"
OUT=$(KOFFSET=$KOFFSET KMODE=send KKEY=$K_KEY keeper); REP=$(echo "$OUT" | report)
echo "$REP" | q "'\n'.join('     '+d['action']+': '+d['plan'] for d in r['collections'][0]['draws'])"
[ "$(stage)" = 1 ] && ok "draw $DRAWS0 opened with the seed round's real evmnet signature; the snapshot time is fixed on chain" || bad "not opened: $(echo "$REP" | q "[d for d in r['collections'][0]['draws'] if d['action']=='open']") $(echo "$REP" | q "r['alerts']")"
URI=$(echo "$REP" | q "[d for d in r['collections'][0]['draws'] if d['action']=='open'][0].get('uri','')"); DROOT=$(echo "$REP" | q "[d for d in r['collections'][0]['draws'] if d['action']=='open'][0].get('root','')")
FILE="$WORK/state/snapshots/$(basename "$URI")"
[ -f "$FILE" ] && [ "$(python3 -c "import json; print(json.load(open('$FILE'))['version'])")" = 2 ] && ok "published (v2, with the sentinel): $(basename "$URI"), read back from $URI before the draw was opened" || bad "the snapshot file is missing"
cast call --rpc-url $RPC $RAF "$DRAW" $DRAWS0 | grep -q "$DROOT" && ok "the draw's root on chain is the published root $DROOT" || bad "the on-chain root differs"
# anyone can recompute it from the chain alone: the draw's coin and snapshot time are read from the vault, the exclusions from the stack
(cd tools/snapshot && npx tsx src/cli.ts --vault $RAF --draw-id $DRAWS0 --stack "$ROOT/$S" --slug $SLUG --exclusions exclusions.json --rpc $RPC --out "$WORK/recomputed.json" > "$WORK/recompute.log" 2>&1) \
  && ok "tools/snapshot --vault/--draw-id, run independently against the fork, rebuilds the draw's root and ticket count: $(grep -E '^draw ' "$WORK/recompute.log")" \
  || bad "the recomputation disagrees: $(tail -3 "$WORK/recompute.log")"
cmp -s "$FILE" "$WORK/recomputed.json" && echo "     and the same bytes (sha256 $(shasum -a 256 "$FILE" | cut -c1-16)…)" || echo "note the files differ outside the root (key order of the inputs)"
OUT=$(KOFFSET=$KOFFSET KMODE=send KKEY=$K_KEY keeper)
[ "$(call $RAF 'drawCount()(uint256)')" = $((DRAWS0 + 1)) ] && ok "the same slot again: not announced twice" || bad "a second draw for the same slot"
# the challenge window passes on both clocks (15 minutes, 75 parent blocks): the keeper's in-process resolver needs no keeper power
cast rpc --rpc-url $RPC evm_increaseTime 905 >/dev/null; cast rpc --rpc-url $RPC anvil_mine 0x50 >/dev/null
for i in 1 2 3; do OUT=$(KOFFSET=$KOFFSET KMODE=send KKEY=$K_KEY keeper); REP=$(echo "$OUT" | report)
  echo "$REP" | q "'\n'.join('     resolver: '+a['what']+' -> '+((a.get('result') or {}).get('outcome') or a['plan']) for a in (r.get('resolver') or {}).get('actions',[]))"; done
[ "$(stage)" = 4 ] && WIN=$(cast call --rpc-url $RPC $RAF "$DRAW" $DRAWS0 | sed 's/^(//' | awk -F', ' '{print $13}') \
  && ok "requested, resolved with the pinned round's real signature, and claimed for $WIN, who holds $(call $(col wrapper) 'balanceOf(address)(uint256)' $WIN) wX" \
  || bad "the resolver did not finish the draw (stage $(stage))"
kill $WEB 2>/dev/null

step "6. the refusals (the keeper's nonce must not move)"
N2=$(nonce $K)
OUT=$(KCHAIN=4663 KMODE=send KKEY=$K_KEY keeper); CODE=$?
[ $CODE = 2 ] && echo "$OUT" | grep -q "a fixture floor is never SENT to Robinhood Chain" && ok "KEEPER_CHAIN_ID=4663 with a fixture floor and send: refused at start" || bad "chain 4663 + fixture: exit $CODE"
OUT=$(KCHAIN=4663 KSOURCE=ordinalswallet KMODE=send KKEY=$K_KEY keeper); CODE=$?
[ $CODE = 2 ] && echo "$OUT" | grep -q "for chain $STAGING, and the keeper signs for 4663" && ok "KEEPER_CHAIN_ID=4663 against this stack: refused (the stack file is for $STAGING)" || bad "wrong chain id: exit $CODE: $(echo "$OUT" | tail -2)"
OUT=$(KMODE=send KKEY=$X_KEY keeper); CODE=$?
[ $CODE = 1 ] && echo "$OUT" | grep -q "is not registry.keeper()" && ok "a key that is not registry.keeper(): every call refused" || bad "wrong key: exit $CODE: $(echo "$OUT" | grep -i refus | head -2)"
cast rpc --rpc-url $RPC anvil_setCode $K "$KCODE" >/dev/null                # the delegation it really carries on 4663, put back
OUT=$(KMODE=send KKEY=$K_KEY keeper); CODE=$?
[ $CODE = 1 ] && echo "$OUT" | grep -q "has code" && ok "a keeper address with code (an EIP-7702 delegation): refused" || bad "code at the keeper: exit $CODE"
cast rpc --rpc-url $RPC anvil_setCode $K 0x >/dev/null
anvil --chain-id 31337 --host 127.0.0.1 --port $PORT2 --silent & NODE2=$!
for _ in $(seq 1 60); do cast chain-id --rpc-url $RPC2 >/dev/null 2>&1 && break; sleep 0.5; done
OUT=$(KRPCS="$RPC2,$RPC" KMODE=send KKEY=$K_KEY keeper); CODE=$?
[ $CODE = 0 ] && echo "$OUT" | grep -q '"event":"rpc-wrong-chain"' && ok "an RPC reporting 31337 is dropped by host, and the keeper runs on the one that reports $STAGING" || bad "mixed RPCs: exit $CODE"
OUT=$(KRPCS="$RPC2" KMODE=send KKEY=$K_KEY keeper); CODE=$?
[ $CODE = 2 ] && echo "$OUT" | grep -q "no RPC reports chain id $STAGING" && ok "only an RPC reporting another chain: refused at start" || bad "wrong-chain RPC: exit $CODE"
[ "$(nonce $K)" = "$N2" ] && ok "through every refusal the keeper's nonce stayed $N2" || bad "a refusal sent something"

step "7. the node REPORTS 4663 (anvil_setChainId): the keeper runs there in DRY RUN only — it simulates, it never signs for 4663"
python3 -c "import json; j=json.load(open('$S')); j['chainId']=4663; json.dump(j, open('$WORK/stack-4663.json','w'), indent=1)"
cast rpc --rpc-url $RPC anvil_setChainId 4663 >/dev/null
[ "$(cast chain-id --rpc-url $RPC)" = 4663 ] && ok "the fork now reports 4663" || bad "anvil_setChainId"
OUT=$(KCHAIN=4663 KSTACK="$WORK/stack-4663.json" KSTATE="$WORK/state-4663" KSOURCE=ordinalswallet keeper --dry-run); CODE=$?
REP=$(echo "$OUT" | report)
echo "     the real feed, keyless (no UNISAT_API_KEY): $(echo "$REP" | q "r['collections'][0]['floor']['feedWhy'][:180]")"
[ "$(echo "$REP" | q "r['collections'][0]['floor']['plan']['type']")" != post ] && ok "the real feed without two agreeing ordinals providers posts NO floor" || bad "a floor was planned from an unagreed feed"
! echo "$OUT" | grep -q '"event":"tx-sent"' && [ "$(nonce $K)" = "$N2" ] && ok "dry run on a node reporting 4663: nothing signed, nothing sent" || bad "something was sent on the 4663-reporting node"
cast rpc --rpc-url $RPC anvil_setChainId $STAGING >/dev/null

step "8. K7: the reserves reconciliation (ops/src/reconcile.ts) against the same fork, read-only, before any canister exists"
reconcile() { (cd ops && env -i PATH="$PATH" HOME="$HOME" RECONCILE_STACK="$ROOT/$S" RECONCILE_CHAIN_ID=$STAGING RECONCILE_RPC_URLS=$RPC RECONCILE_STATE_DIR="$WORK/rec${1:-}" \
  ${RCAN:+CANISTER_ID=$RCAN} ${RCAN:+IC_HOST=http://127.0.0.1:$RPORT} node --import tsx src/reconcile.ts --once --dry-run 2>&1); }   # --dry-run: no Telegram in a test (it refuses to start without one)
N3=$(nonce $K)
# the reserves are read CONFIRMATIONS (20) blocks behind the head, as on 4663 (about 2 s there); anvil mines only on a transaction,
# so 25 empty blocks bring everything above into the read
cast rpc --rpc-url $RPC anvil_mine 0x19 >/dev/null
OUT=$(reconcile); REP=$(echo "$OUT" | report)
echo "$REP" | q "'     collection %s: %s; items in custody %s, tickets in flight %s, ETH-backed %s; block %s' % (r['collections'][0]['slug'], r['collections'][0]['status'], r['collections'][0]['equation']['itemsInCustody'], r['collections'][0]['equation']['ticketsInFlight'], r['collections'][0]['equation']['ethBackedUnits'], r['block'])"
python3 -c "import sys; sys.exit(0 if int('$(echo "$REP" | q "r['collections'][0]['equation']['ethBackedUnits']")') > 0 else 1)" && ok "the read includes the graduation: ETH-backed units at the module" || bad "the read missed the module's units"
[ "$(echo "$REP" | q "r['collections'][0]['status']")" = green ] && ok "the collection reconciles green on the fork: the wrapper's supply equals items (none) plus the module's ETH-backed units" || bad "collection: $(echo "$REP" | q "r['collections'][0]['reasons']")"
[ "$(echo "$REP" | q "r['servedStatus']")" = unverified ] && echo "$REP" | q "r['servedReasons']" | grep -q "CANISTER_ID is not set" && ok "without the canister the caps and the stage are unverified, so the whole is unverified, never green" || bad "no canister: $(echo "$REP" | q "r['servedStatus']")"
[ "$(nonce $K)" = "$N3" ] && ok "the reconciliation sent nothing" || bad "the reconciliation moved a nonce"

step "9. K7 with the canister: this worktree's wasm on a local replica, custody_caps() read anonymously, the stage compared with the stack's"
WASM="$ROOT/autobuyer/canister-rs/target/wasm32-unknown-unknown/release/etch_autobuyer.wasm"; RPORT="${KEEPER_FORK_REPLICA_PORT:-4963}"
if ! command -v dfx >/dev/null 2>&1 && [ -x "$HOME/Library/Application Support/org.dfinity.dfx/bin/dfx" ]; then export PATH="$HOME/Library/Application Support/org.dfinity.dfx/bin:$PATH"; fi
if [ ! -f "$WASM" ] || ! command -v dfx >/dev/null 2>&1; then echo "note step 9 skipped: no dfx or no wasm (cd autobuyer/canister-rs && cargo build --release --target wasm32-unknown-unknown)"; else
  WRAPPER=$(col wrapper); mkdir -p "$WORK/replica"
  printf '{ "canisters": { "autobuyer": { "type": "custom", "candid": "%s", "wasm": "%s", "build": "true" } }, "networks": { "local": { "bind": "127.0.0.1:%s", "type": "ephemeral" } } }\n' \
    "$ROOT/autobuyer/canister-rs/etch_autobuyer.did" "$WASM" "$RPORT" > "$WORK/replica/dfx.json"
  ARG="(record { chain_id = $STAGING : nat64; vault = \"0x0000000000000000000000000000000000000001\"; rpc_urls = vec { \"https://127.0.0.1:1\"; \"https://127.0.0.2:1\" };
    btc_api_urls = vec { \"https://127.0.0.1:2\"; \"https://127.0.0.2:2\" }; key_name = \"key_1\"; accept_production_key_name = true; bridge_allowlist = vec {};
    btc_network = \"signet\"; operator = null; wrapper = opt \"$WRAPPER\"; evm_chains = null; custody_caps = null; outflow_limits = null })"
  if (cd "$WORK/replica" && dfx start --background --clean >/dev/null 2>&1 && dfx canister create autobuyer --identity default >/dev/null 2>&1 \
      && dfx canister install autobuyer --identity default --wasm "$WASM" --argument "$ARG" >/dev/null 2>&1); then
    RCAN=$(cd "$WORK/replica" && dfx canister id autobuyer 2>/dev/null)
    ok "canister $RCAN installed with custody_caps = null (stage 0) over the stack's wrapper, on a replica whose every outcall goes nowhere"
    OUT=$(RCAN=$RCAN RPORT=$RPORT reconcile 2); REP=$(echo "$OUT" | report)
    echo "     custody_caps(): $(echo "$REP" | q "r['caps']['stage']['label'] + ', ' + ', '.join('%s: %s items / %s wei' % (c['collection'], c['max_items'], c['max_value_wei']) for c in r['caps']['policy']['collections'])")"
    [ "$(echo "$REP" | q "r['caps']['match']")" = equal ] && ok "custody_caps() says stage 0, equal to the stack's stage 0" || bad "caps: $(echo "$REP" | q "r['caps']")"
    [ "$(echo "$REP" | q "r['servedStatus']")" = green ] && ok "the reconciliation is GREEN: the collection reconciles and the caps are the stack's stage" || bad "status $(echo "$REP" | q "r['servedStatus'], r['servedReasons']")"
    (cd "$WORK/replica" && dfx stop >/dev/null 2>&1)
  else bad "the replica or the install failed"; (cd "$WORK/replica" && dfx stop >/dev/null 2>&1); fi
fi

printf '\n%s\n' "$([ $fail = 0 ] && echo 'KEEPER FORK TEST: ALL PASS' || echo 'KEEPER FORK TEST: FAILURES ABOVE')"
exit $fail
