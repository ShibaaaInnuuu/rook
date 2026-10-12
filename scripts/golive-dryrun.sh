#!/usr/bin/env bash
# docs/deploy/contracts-mainnet.md, "Rehearse first". The whole contract go-live, twice, on throwaway LOCAL nodes on port 8604:
#   bare   anvil, chain 31337, with scripts' LocalPonsFactory planted at Pons' address (a bare node has no Pons)
#   fork   anvil forking Robinhood Chain mainnet (read-only public RPC) under OUR staging chain id, so nothing signed there could
#          be replayed on 4663 — the environment differences (real Pons, real gas rules, Arbitrum block numbers) without the chain
# and then every new refusal, including a node that reports 4663, where the listing step must SEND NOTHING.
# Nothing here reaches a public chain: every node is 127.0.0.1, every sender an anvil dev account. Nothing reads a private key.
#   usage:  scripts/golive-dryrun.sh [bare|fork|refusals|all|stack]
# `stack` (docs/v6/deploy-stack.md §7): the whole v3–v6 stack on a fork at port 8651 (STACK_PORT), then one launch → graduation →
# module mint → sweep → raffle draw, end to end. It waits for two real drand rounds (the draw's seed, then its pin: about 16
# minutes in all) unless DRYRUN_NO_DRAND=1, which stops at the draw's announcement. STACK_FORK_RPC / STACK_FORK_BACK pick the fork's
# upstream and depth (default the archive rpc.ordofi.network, 2,000 blocks back); STACK_PHASES (default "phase1 nft mixed") runs a
# subset of the three go/no-go phases after the frog stack (the public RPC keeps ~15 minutes of state: a full run outlives it).
set -uo pipefail
export PATH="$HOME/.foundry/bin:$PATH"
cd "$(dirname "$0")/.."
PORT="${DRYRUN_PORT:-8604}"; RPC="http://127.0.0.1:$PORT"
EXTRA_ANVIL=""
MAINNET_RPC="${MAINNET_RPC:-https://rpc.mainnet.chain.robinhood.com}"
STAGING=$(sed -n 's/.*"chainId"[^0-9]*\([0-9][0-9]*\).*/\1/p' staging/chain-id.json | head -1)
PONS=0x7eD598BcEf8bd9Edd8C97A195C6d13f40801EC7e
# anvil's first dev accounts, each with one role and no other
D=0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266; O=0x70997970C51812dc3A010C7d01b50e0d17dc79C8; T=0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC
K=0x90F79bf6EB2c4f870365E785982E1f101E93b906; P=0x15d34AAf54267DB7D7c367839AAf71A00a2C6A65; A=0x9965507D1a55bcC2695C58ba16FB37d819B0A4dc
# the buyer is NOT a dev account: every anvil dev account carries an EIP-7702 delegation on Robinhood mainnet (0xef0100…), so on the
# fork it has code, and the buyer check rightly refuses it. This address has no code on 4663 (checked); OTHER is another such one.
BUYER=0x0000000000000000000000000000000000b0b0e1; OTHER=0x0000000000000000000000000000000000d15700
lc() { tr 'A-F' 'a-f'; }
NODE=""
fail=0
stop() { [ -z "$NODE" ] || kill "$NODE" 2>/dev/null; wait "$NODE" 2>/dev/null; NODE=""; }
# the deployment files this run writes are removed afterwards unless they were already there: script/deploy.test.sh checks that no
# refusal ever leaves one behind, and a dry run's leftovers would read as exactly that
KEEP=""; for f in deployments/31337.json deployments/$STAGING.json; do [ -e "$f" ] && KEEP="$KEEP $f"; done
tidy() { stop; for f in deployments/31337.json deployments/$STAGING.json; do case " $KEEP " in *" $f "*) ;; *) rm -f "$f";; esac; done
  rm -f deployments/wrapper-31337-*.json deployments/wrapper-"$STAGING"-*.json deployments/nft-31337.json deployments/nft-"$STAGING".json
  # the stack rehearsal's files are removed only by a stack run: another mode running beside it must not pull them away mid-wait
  [ "${MODE:-}" != stack ] || rm -f deployments/stack-31337.json deployments/stack-"$STAGING".json deployments/rehearsal-*.json deployments/rehearsal-pin-*.json; }
trap tidy EXIT
start() { # start <chainid> [fork]
  stop
  if [ "${2:-}" = fork ]; then anvil --fork-url "$MAINNET_RPC" --chain-id "$1" --host 127.0.0.1 --port $PORT --code-size-limit 98304 --silent $EXTRA_ANVIL & NODE=$!
  else anvil --chain-id "$1" --host 127.0.0.1 --port $PORT --code-size-limit 98304 --silent $EXTRA_ANVIL & NODE=$!; fi
  for _ in $(seq 1 120); do [ "$(cast chain-id --rpc-url $RPC 2>/dev/null)" = "$1" ] && return 0; sleep 0.5; done
  echo "FAIL the node for chain $1 did not start"; exit 1
}
step() { printf '\n== %s\n' "$*"; }
expect() { # expect <name> <pattern> <command…>: the command must FAIL and say <pattern>
  local name="$1" pat="$2"; shift 2; local out code
  out=$("$@" 2>&1); code=$?
  # a here-string, not a pipe: grep -q stops at its first match, and under pipefail a long output's echo then dies of SIGPIPE
  if [ $code -ne 0 ] && grep -q -- "$pat" <<< "$out"; then echo "ok   refused: $name"; else echo "FAIL $name: exit $code: $(echo "$out" | grep -v '^\s*$' | tail -2)"; fail=1; fi
}
must() { # must <name> <command…>: the command must succeed; returns its status so a caller can stop
  local name="$1"; shift; local out
  if out=$("$@" 2>&1); then echo "ok   $name"; LAST="$out"; return 0; else echo "FAIL $name: $( { echo "$out" | grep '^FAIL' | head -6; echo "$out" | grep -v '^\s*$' | grep -v '^FAIL' | tail -2; } )"; fail=1; LAST="$out"; return 1; fi
}
roles() { env ETCH_OWNER=$O ETCH_TREASURY=$T ETCH_KEEPER=$K ETCH_PURCHASER=$P ETCH_ATTESTOR=$A "$@"; }
# the dry run's custody address: BIP-86's first test vector (a real taproot address nobody here holds a key to)
DRY_CUSTODY=bc1p5cyxnuxmeuwuvkwfem96lqzszd02n6xdcjrs20cac6yqjjwudpxqkedrcr
listenv() { env ETCH_LIST_KIND=inscriptions ETCH_LIST_NAME="Dry Run Frogs" ETCH_LIST_REF=dry-run-frogs \
  ETCH_LIST_ROOT=0x$(printf 'dry-run-root' | shasum -a 256 | cut -c1-64) ETCH_LIST_BTC_VAULT=$DRY_CUSTODY \
  ETCH_BUYER=$BUYER ETCH_MAX_IN_FLIGHT=1000000000000000000 "$@"; }

rehearse() { # rehearse <chainid> <label>
  local CH="$1"; local F="deployments/$CH.json"
  rm -f "$F" deployments/wrapper-"$CH"-*.json
  step "$2: 1-3. the system (registry, beacon, launcher, distributor), wired, with the v1 demo listings"
  must "script/deploy.sh" roles env ETCH_DEPLOY_CONFIRM="deploy-etch:$CH:$(echo $D | lc)" script/deploy.sh $RPC $D
  local R; R=$(python3 -c "import json;print(json.load(open('$F'))['registry'])")
  step "$2: 4. an autonomous-buyer listing, naming the buyer"
  must "script/list-auto.sh" listenv env ETCH_REGISTRY=$R ETCH_BUYER_CONFIRM="buyer-is:$CH:$(echo $BUYER | lc)" script/list-auto.sh $RPC
  local ID=$(( $(cast call --rpc-url $RPC $R 'count()(uint256)') - 1 ))
  step "$2: 6. WrapClaim over that listing, which deploys its own Wrapper"
  must "script/deploy-wrapclaim.sh" env ETCH_WRAPPER_CONFIRM="deploy-etch-wrapper:$CH:$(echo $D | lc)" ETCH_REGISTRY=$R ETCH_WRAPPER_LISTING=$ID \
    ETCH_WRAPPER_NAME="Wrapped Dry Run Frog" ETCH_WRAPPER_SYMBOL=wDRY ETCH_WRAPPER_CUSTODIAN=$BUYER ETCH_WRAPPER_CUSTODY_ADDRESS=$DRY_CUSTODY \
    script/deploy-wrapclaim.sh $RPC $D
  step "$2: 7. the NFT side (docs/v4/nft.md): NftRegistry + NftLauncher over this registry, wired, with its demo listings"
  must "script/deploy-nft.sh" env ETCH_NFT_CONFIRM="deploy-etch-nft:$CH:$(echo $D | lc)" ETCH_REGISTRY=$R ETCH_NFT_BUYER=$BUYER script/deploy-nft.sh $RPC $D
  step "$2: 5. read everything back from the chain"
  must "script/golive-check.sh" script/golive-check.sh $RPC "$F" "deployments/wrapper-$CH-$ID.json" "deployments/nft-$CH.json"
  echo "$LAST" | grep -E "^(ok|FAIL|note|     )" | sed 's/^/     /'
  # the new refusals, on a node where everything else would succeed
  step "$2: the refusals that guard the two new steps"
  expect "a buyer confirm typed for another address" "BuyerConfirmMismatch" listenv env ETCH_REGISTRY=$R ETCH_BUYER_CONFIRM="buyer-is:$CH:$(echo $OTHER | lc)" script/list-auto.sh $RPC
  expect "a buyer that is a contract (the registry itself)" "BuyerHasCode" listenv env ETCH_REGISTRY=$R ETCH_BUYER=$R ETCH_BUYER_CONFIRM="buyer-is:$CH:$(echo $R | lc)" script/list-auto.sh $RPC
  expect "a buyer that is the owner (the registry's SameKey; on the fork, BuyerHasCode first: the owner there is a delegated dev account)" "SameKey\|BuyerHasCode" listenv env ETCH_REGISTRY=$R ETCH_BUYER=$O ETCH_BUYER_CONFIRM="buyer-is:$CH:$(echo $O | lc)" script/list-auto.sh $RPC
  expect "maxInFlight of zero" "MaxInFlightZero" listenv env ETCH_REGISTRY=$R ETCH_MAX_IN_FLIGHT=0 ETCH_BUYER_CONFIRM="buyer-is:$CH:$(echo $BUYER | lc)" script/list-auto.sh $RPC
  expect "a wrapper pair with no confirm" "ETCH_WRAPPER_CONFIRM must be exactly" env -u ETCH_WRAPPER_CONFIRM ETCH_REGISTRY=$R ETCH_WRAPPER_LISTING=$ID ETCH_WRAPPER_NAME=x ETCH_WRAPPER_SYMBOL=x script/deploy-wrapclaim.sh $RPC $D
  expect "a wrapper pair over the demo RUNE listing" "ListingIsRune" env ETCH_WRAPPER_CONFIRM="deploy-etch-wrapper:$CH:$(echo $D | lc)" ETCH_REGISTRY=$R ETCH_WRAPPER_LISTING=1 ETCH_WRAPPER_NAME=x ETCH_WRAPPER_SYMBOL=x script/deploy-wrapclaim.sh $RPC $D
  expect "a key in the environment" "does not take a key" env PRIVATE_KEY=anything ETCH_WRAPPER_CONFIRM="deploy-etch-wrapper:$CH:$(echo $D | lc)" script/deploy-wrapclaim.sh $RPC $D
  expect "an NFT deployment with no confirm" "ETCH_NFT_CONFIRM must be exactly" env -u ETCH_NFT_CONFIRM ETCH_REGISTRY=$R script/deploy-nft.sh $RPC $D
  expect "an NFT deployment over a registry that is not a contract" "RegistryIsNotAContract" env ETCH_NFT_CONFIRM="deploy-etch-nft:$CH:$(echo $D | lc)" ETCH_REGISTRY=$OTHER script/deploy-nft.sh $RPC $D
  rm -f deployments/wrapper-"$CH"-*.json deployments/nft-"$CH".json
}

# ---- docs/v6/deploy-stack.md §7: the whole stack, and one launch → graduation → module mint → sweep → raffle draw ------------------
# the rehearsal's actors are code-less addresses (the dev accounts carry EIP-7702 code on the fork); anvil impersonates them
CREATOR=0x00000000000000000000000000000000C0FFEE01; RBUYER=0x00000000000000000000000000000000C0ffee02; TRADER=0x00000000000000000000000000000000C0FfEE03
DRAND_CHAIN=04f1e9062b8a81f848fded9c12306733282b2727ecced50032187751166ec8c3   # drand evmnet (src/rand/DrandEvmnet.sol, test/fixtures/drand-evmnet.json)
DRAND_GENESIS=1727521075; DRAND_PERIOD=3
# the guardian (fixes-B-C-keeper-draws.md §3): a code-less address, as the monitor's key would be (checked on the fork below)
GUARD=0x0000000000000000000000000000000000c0a4d1
stackenv() { env ETCH_WRAPPER_NAME="Wrapped Dry Run Frog" ETCH_WRAPPER_SYMBOL=wDRY ETCH_WRAPPER_CUSTODIAN=$BUYER ETCH_WRAPPER_CUSTODY_ADDRESS=$DRY_CUSTODY \
  ETCH_OPS=$T ETCH_GUARDIAN=$GUARD ETCH_STACK_SLUG=dry-run-frogs "$@"; }
phase() { # phase <name> [VAR=value …]: one phase of script/StackRehearsal.s.sol
  local name="$1"; shift
  env REHEARSAL_PHASE="$name" "$@" forge script script/StackRehearsal.s.sol:StackRehearsal --rpc-url $RPC --sender $D --unlocked --broadcast --slow --code-size-limit 98304
}
# bayc10-decisions.md §5, §7 (round 3): the buyer's reinstall argument, a REAL Init (monitor/src/candid.mjs's Init type, encoded with
# the monitor's own Candid library and decoded back by its decodeInit), with the home chain's providers (the 4663 URLs this dry run
# uses) and <Ethereum provider URLs> in evm_chains[1].rpc_urls: what golive-check --install-arg reads for keys
installhex() { # installhex <Ethereum provider URLs, comma-separated> <out file>
  ( cd monitor && node --input-type=module - "$1" ) > "$2" <<'JS'
const { Init, decodeInit } = await import(process.cwd() + "/src/candid.mjs");
const { IDL } = await import("@icp-sdk/core/candid");
const eth = process.argv[process.argv.length - 1].split(",");
const home = ["https://rpc.mainnet.chain.robinhood.com", "https://rpc.ordofi.network"];
const chain = (id, urls) => ({ chain_id: BigInt(id), rpc_urls: urls, bucket: 0n, seaport: "0x0000000000000068f116a894984e2db1123eb395",
  seaport_codehash: "0x" + "00".repeat(32), max_fee_wei: 50000000000n, priority_fee_wei: 1000000000n, gas_cap: 400000n, fee_recipients: [],
  max_fee_bps: 300, min_order_life_s: 600n, signed_zones: [], zone_registries: [] });
const v = { chain_id: 4663n, vault: "0x00", rpc_urls: home, btc_api_urls: ["https://mempool.space/api", "https://blockstream.info/api"],
  key_name: "key_1", accept_production_key_name: true, bridge_allowlist: [], btc_network: "mainnet", operator: [], wrapper: [],
  evm_chains: [[chain(4663, home), chain(1, eth)]], custody_caps: [], outflow_limits: [], holder_deposits: [], wrapper_from_block: [],
  frog_funding: [], eth_nft: [] };
const hex = Buffer.from(IDL.encode([Init], [v])).toString("hex");
const d = decodeInit(hex);
if (!d.ok || d.init.evm_chains[0][1].rpc_urls.join(",") !== eth.join(",")) throw new Error("the Init does not decode back: " + d.error);
process.stdout.write(hex + "\n");
JS
  grep -q '^4449444c' "$2"
}
stagedhex() { # stagedhex <1|2|3|3x> <out file>: the buyer's Init at a step of the committed launch plan (3x: step 3's caps at step 1's outflow)
  ( cd monitor && node --input-type=module - "$1" ) > "$2" <<'JS'
const { encodeInitPlain, decodeInit } = await import(process.cwd() + "/src/candid.mjs");
const { readFileSync } = await import("node:fs");
const L = JSON.parse(readFileSync("../deployments/launch/nft-pilot.json", "utf8"));
const which = process.argv[process.argv.length - 1];
const st = L.stages[Number(which[0]) - 1], out = which === "3x" ? L.stages[0].outflow : st.outflow;
const collections = L.collections.map((c) => ({ collection: `1:${c.collection.toLowerCase()}`, max_items: st.custodyItems[c.slug], max_value_wei: String(c.custodyWei), max_item_wei: String(c.custodyWei) }));
const v = { chain_id: 4663n, vault: "0x00", rpc_urls: ["https://rpc.mainnet.chain.robinhood.com", "https://rpc.ordofi.network"], btc_api_urls: ["https://mempool.space/api", "https://blockstream.info/api"],
  key_name: "key_1", accept_production_key_name: true, bridge_allowlist: [], btc_network: "mainnet", operator: [], wrapper: [], evm_chains: [], holder_deposits: [], wrapper_from_block: [],
  // docs/v7/dormant-collections.md §6: a step that names its global (globalItems, globalWei) has it binding; otherwise J7's sum
  custody_caps: [{ collections, global_max_items: st.globalItems ?? collections.reduce((a, c) => a + c.max_items, 0), global_max_value_wei: String(st.globalWei ?? collections.reduce((a, c) => a + BigInt(c.max_value_wei), 0n)) }],
  outflow_limits: [{ ...out, global_value_wei: String(out.global_value_wei) }],
  eth_nft: [{ chain_id: 1n, relay_max_fee_bps: 300, deposits: true, floor: [], gas_pot_target_wei: [], logs_max_blocks: [],
    collections: L.collections.map((c) => ({ collection: c.collection, wrapper: "0x" + "aa".repeat(20), module: "0x" + "bb".repeat(20), max_purchase_wei: String(c.maxPurchaseWei), max_premium_bps: 500 })) }] };
const hex = encodeInitPlain(v);
if (!decodeInit(hex).ok) throw new Error("the staged Init does not decode back");
process.stdout.write(hex + "\n");
JS
  grep -q '^4449444c' "$2"
}
drandsig() { # drandsig <round>: wait for the round to exist, then print its evmnet signature (empty if it could not be read)
  local R="$1" NOW AT=$(( DRAND_GENESIS + ($1 - 1) * DRAND_PERIOD + 5 )) SIG=""
  while NOW=$(date +%s); [ "$NOW" -lt "$AT" ]; do echo "     round $R in $((AT - NOW)) s" >&2; sleep $(( AT - NOW > 60 ? 60 : AT - NOW )); done
  for _ in $(seq 1 20); do
    SIG=$(curl -s -m 15 "https://api.drand.sh/$DRAND_CHAIN/public/$R" | python3 -c "import json,sys; d=json.load(sys.stdin); print(d.get('signature',''))" 2>/dev/null || true)
    [ -n "$SIG" ] && break; sleep 5
  done
  echo "$SIG"
}
# docs/audit/2026-09-26/fixes-W2-monitor.md §3 (audit C-08, B-05): the go/no-go read. The fork is made to look like a real go-live
# first: the role keys become code-less EOAs (anvil's dev accounts carry EIP-7702 delegations on 4663, so their code is cleared on
# this LOCAL fork), and the owner a 2-of-2 Safe (script/local/StandInSafe.sol's runtime planted at the owner's address; the owner's
# own code is swapped back in for each owner call). Then `--phase1` must pass, and FAIL on each skipped or undone step.
golive_phase1() { # golive_phase1 <chainid> <deployment file> <stack file> <registry> <listing id>
  local CH="$1" F="$2" S="$3" R="$4" ID="$5" OCODE SAFE RAFFLE CEIL SNAP
  step "STACK: the go/no-go read (golive-check.sh --phase1): a correct deploy passes, and each skipped or undone step FAILS"
  [ "$(cast code --rpc-url $RPC $GUARD)" = "0x" ] && echo "ok   the guardian $GUARD has no code on the fork" || { echo "FAIL the guardian $GUARD has code on the fork"; fail=1; }
  OCODE=$(cast code --rpc-url $RPC $O); SAFE=$(forge inspect script/local/StandInSafe.sol:StandInSafe deployedBytecode)
  RAFFLE=$(python3 -c "import json;print(json.load(open('$S'))['collections'][0]['raffleVault'])"); CEIL=$(python3 -c "import json;print(json.load(open('$S'))['capCeiling'])")
  for a in $K $P $A $T; do cast rpc --rpc-url $RPC anvil_setCode $a 0x >/dev/null; done
  plant() { cast rpc --rpc-url $RPC anvil_setCode $O "$SAFE" >/dev/null
    cast rpc --rpc-url $RPC anvil_setStorageAt $O 0x0 "$(printf '0x%064x' "${1:-2}")" >/dev/null; cast rpc --rpc-url $RPC anvil_setStorageAt $O 0x1 "$(printf '0x%064x' 2)" >/dev/null; }
  as_owner() { cast rpc --rpc-url $RPC anvil_setCode $O "$OCODE" >/dev/null; cast send --rpc-url $RPC --unlocked --from $O "$@" >/dev/null 2>&1; local r=$?; plant; return $r; }
  plant
  local P1=(env GOLIVE_REHEARSAL=1 script/golive-check.sh $RPC "$F" "$S" --phase1)
  must "golive-check.sh --phase1 on the correct deploy" "${P1[@]}"
  echo "$LAST" | grep -E "^(FAIL|WARN|note)|a Safe, 2 of 2|^ok   guardian|launchesOpen|ALL CHECKS" | sed 's/^/     /'
  grep -q "treasury shares (reserves/floats/bootstrap/ops) and bootstrap loan at stage 0" <<< "$LAST" && echo "ok   it reads the treasury's split and loan (C-07)" || { echo "FAIL no treasury split check"; fail=1; }
  grep -q "module.registry() is the registry" <<< "$LAST" && grep -q "module.wrapped() is the wrapper (B-05)" <<< "$LAST" \
    && echo "ok   it reads module.registry() and module.wrapped() (B-05)" || { echo "FAIL no module.registry()/wrapped() check"; fail=1; }
  # each one undone, alone, then put back
  cast rpc --rpc-url $RPC anvil_setBalance $GUARD 0xDE0B6B3A7640000 >/dev/null
  cast send --rpc-url $RPC --unlocked --from $GUARD $R 'pauseKeeper()' >/dev/null 2>&1 && echo "ok   the guardian paused the keeper (its one power but cancelDraw)" || { echo "FAIL the guardian could not pause the keeper"; fail=1; }
  expect "--phase1 with the keeper paused" "FAIL registry.keeperPaused(): the keeper is PAUSED" "${P1[@]}"
  as_owner $R 'unpauseKeeper()' || { echo "FAIL the owner could not unpause"; fail=1; }
  as_owner $R 'setGuardian(address)' 0x0000000000000000000000000000000000000000
  expect "--phase1 with no guardian" "FAIL no guardian is set" "${P1[@]}"
  must "--phase1 --no-guardian with no guardian (the operator's written choice: a WARN)" "${P1[@]}" --no-guardian
  grep -q "^WARN no guardian" <<< "$LAST" && echo "ok   and it says WARN" || { echo "FAIL no WARN for the declined guardian"; fail=1; }
  as_owner $R 'setGuardian(address)' $GUARD
  plant 1
  expect "--phase1 with the owner a 1-of-2 Safe" "is not a 2-of-2 Safe: threshold 1 of 2" "${P1[@]}"
  plant 2
  as_owner $R 'transferOwnership(address)' $OTHER
  expect "--phase1 with a pending ownership transfer" "FAIL registry.pendingOwner() is" "${P1[@]}"
  as_owner $R 'transferOwnership(address)' 0x0000000000000000000000000000000000000000
  cast rpc --rpc-url $RPC anvil_setCode $K "0xef0100$(echo ${OTHER#0x} | lc)" >/dev/null
  expect "--phase1 with the keeper's key 7702-delegated (a sweeper)" "role keeper .* has code: an EIP-7702 delegation" "${P1[@]}"
  cast rpc --rpc-url $RPC anvil_setCode $K 0x >/dev/null
  as_owner $RAFFLE 'proposeCapCeiling(uint256)' $((CEIL * 2))
  expect "--phase1 with a cap-ceiling raise pending" "pendingCeiling() is set: a cap-ceiling RAISE is pending" "${P1[@]}"
  as_owner $RAFFLE 'proposeCapCeiling(uint256)' $CEIL
  must "--phase1 passes again once every step is put back" "${P1[@]}"
  # step 7 skipped altogether: a second stack over the same listing whose owner steps are printed, not made, as on 4663
  SNAP=$(cast rpc --rpc-url $RPC evm_snapshot | tr -d '"'); mv "$S" "$S.first"
  if must "script/deploy-stack.sh, owner steps NOT made (ETCH_REHEARSE_UNMADE_OWNER_STEPS=1)" stackenv env ETCH_REHEARSE_UNMADE_OWNER_STEPS=1 ETCH_STACK_CONFIRM="deploy-etch-stack:$CH:$(echo $D | lc)" ETCH_REGISTRY=$R ETCH_WRAPPER_LISTING=$ID script/deploy-stack.sh $RPC $D; then
    must "golive-check.sh before step 7 (notes, not failures)" script/golive-check.sh $RPC "$F" "$S"
    grep -q "^note .*the wrapper is not wired" <<< "$LAST" && echo "ok   and it notes the unwired wrapper" || { echo "FAIL no note for the unwired wrapper"; fail=1; }
    expect "--phase1 when step 7 was skipped: the wrapper not wired" "FAIL dry-run-frogs: the wrapper is not wired" "${P1[@]}"
    expect "--phase1 when step 7 was skipped: no cap ceiling" "FAIL dry-run-frogs vault 0 has no cap ceiling" "${P1[@]}"
    expect "--phase1 when step 7 was skipped: not in the treasury" "FAIL dry-run-frogs treasury.isCollection(module)" "${P1[@]}"
  fi
  cast rpc --rpc-url $RPC evm_revert "$SNAP" >/dev/null; mv "$S.first" "$S"
  must "--phase1 on the first stack again, after the snapshot is restored" "${P1[@]}"
}

# docs/v7/phase1-nft.md §3.4, §9 D1 (2026-09-27): the Ethereum-NFT stack alone (ETCH_STACK_KIND=nft) — the operator's launch
# collections at once (Milady, Azuki, MAYC, Pudgy Penguins, BAYC, from deployments/launch/nft-pilot.json), each its own wrapper,
# poster, module and graduation targets (docs/v7/phase1-nft-pooled.md J1: BAYC at 10 ETH, convBps 1000, reserveBps 0, one graduation
# per ape) — over the same registry and listing, on a
# snapshot: the go-live read passes (the launch list, D1), the canister's eth_nft_config() is read through a stand-in dfx (D5
# deposits on, D7 floor, Relay 300 bps, exactly the stack's collections), the canister (the custodian, through each PosterSource)
# posts a floor and the keeper EOA cannot, and the registry's pause zeroes every poster. Then the snapshot is restored and the
# Bitcoin stack's file put back.
rehearse_nft_stack() { # rehearse_nft_stack <chainid> <deployment file> <stack file> <registry> <listing id>
  local CH="$1" F="$2" S="$3" R="$4" ID="$5" SNAP M PS NC ETHDIR CAN=rrkah-fqaaa-aaaaa-aaaaq-cai BF BM
  step "STACK (NFT): the allowlisted collections on Ethereum (Milady, Azuki, MAYC, Pudgy Penguins, BAYC, Doodles, Moonbirds, CloneX, Lil Pudgys, Checks, Identity MD: docs/v7/dormant-collections.md §1) and the WakeDesk, each collection with its own graduation targets (BAYC at 10 ETH, convBps 1000, reserveBps 0), each module's floor posted by the canister"
  SNAP=$(cast rpc --rpc-url $RPC evm_snapshot | tr -d '"'); mv "$S" "$S.btc"
  # the launch file (deployments/launch/nft-pilot.json) as the deploy's comma lists (docs/v7/phase1-nft-pooled.md §2)
  local L_COLS L_NAMES L_SYMS L_SLUGS L_NORMAL L_EARLY L_CONV L_RES L_BOND L_WINDOW L_N
  eval "$(python3 - <<'PY'
import json, shlex
c = json.load(open("deployments/launch/nft-pilot.json"))["collections"]
j = lambda k, d=None: ",".join(str(x.get(k, d)) for x in c)
for var, val in [("L_COLS", j("collection")), ("L_NAMES", j("name")), ("L_SYMS", j("symbol")), ("L_SLUGS", j("slug")),
                 ("L_NORMAL", j("normalRaise", 5 * 10**18)), ("L_EARLY", j("earlyRaise", 8 * 10**18)),
                 ("L_CONV", j("convBps", 300)), ("L_RES", j("reserveBps", 2500)),
                 # docs/v7/dormant-collections.md §3.3: the WakeDesk bond and window, and how many collections the allowlist names
                 ("L_BOND", str(json.load(open("deployments/launch/nft-pilot.json")).get("wake", {}).get("bondWei", ""))),
                 ("L_WINDOW", str(json.load(open("deployments/launch/nft-pilot.json")).get("wake", {}).get("windowSecs", ""))), ("L_N", str(len(c)))]:
    print(f"{var}={shlex.quote(val)}")
PY
)"
  if must "script/deploy-stack.sh, ETCH_STACK_KIND=nft, the $L_N allowlisted collections and the WakeDesk (deployments/launch/nft-pilot.json)" env ETCH_STACK_KIND=nft ETCH_NFT_CHAIN_ID=1 ETCH_WAKE_BOND="$L_BOND" ETCH_WAKE_WINDOW="$L_WINDOW" \
      ETCH_NFT_COLLECTIONS="$L_COLS" ETCH_NFT_NAMES="$L_NAMES" ETCH_NFT_SYMBOLS="$L_SYMS" ETCH_NFT_SLUGS="$L_SLUGS" \
      ETCH_NFT_NORMAL_RAISES="$L_NORMAL" ETCH_NFT_EARLY_RAISES="$L_EARLY" ETCH_NFT_CONV_BPS_LIST="$L_CONV" ETCH_NFT_RESERVE_BPS_LIST="$L_RES" ETCH_WRAPPER_CUSTODIAN=$BUYER ETCH_OPS=$T ETCH_GUARDIAN=$GUARD \
      ETCH_STACK_CONFIRM="deploy-etch-stack:$CH:$(echo $D | lc)" ETCH_REGISTRY=$R ETCH_WRAPPER_LISTING=$ID script/deploy-stack.sh $RPC $D; then
    echo "$LAST" | grep -E "^(treasury|directory|nft wrapper|wrote)" | sed 's/^/     /'
    NC=$(python3 -c "import json;print(len(json.load(open('$S'))['collections']))")
    [ "$(python3 -c "import json;c=json.load(open('$S'))['collections'];print(len(c), all(x['kind']=='nft' for x in c), len({x['floorModule'] for x in c}))")" = "$L_N True $L_N" ] \
      && echo "ok   the stack file names the $L_N allowlisted nft collections, each with its own module (no Bitcoin wrapper, no offer board)" || { echo "FAIL the NFT-only stack file is not the $L_N allowlisted nft collections"; fail=1; }
    [ -n "$(python3 -c "import json;print(json.load(open('$S')).get('wakeDesk', ''))")" ] && echo "ok   the stack file names the WakeDesk (docs/v7/dormant-collections.md §3.3)" || { echo "FAIL the stack file names no wakeDesk"; fail=1; }
    BF=$(python3 -c "import json;print([x for x in json.load(open('$S'))['collections'] if x['slug']=='bayc'][0]['factory'])")
    BM=$(python3 -c "import json;print([x for x in json.load(open('$S'))['collections'] if x['slug']=='bayc'][0]['floorModule'])")
    [ "$(cast call --rpc-url $RPC $BF 'normalRaise()(uint256)' | awk '{print $1}')" = "10000000000000000000" ] \
      && [ "$(cast call --rpc-url $RPC $BF 'earlyRaise()(uint256)' | awk '{print $1}')" = "10000000000000000000" ] \
      && [ "$(cast call --rpc-url $RPC $BM 'convBps()(uint256)' | awk '{print $1}')" = "1000" ] \
      && [ "$(cast call --rpc-url $RPC $BM 'reserveBps()(uint256)' | awk '{print $1}')" = "0" ] \
      && [ "$(cast call --rpc-url $RPC $(python3 -c "import json;print(json.load(open('$S'))['collections'][0]['factory'])") 'normalRaise()(uint256)' | awk '{print $1}')" = "5000000000000000000" ] \
      && echo "ok   BAYC's factory graduates at 10 ETH with its module at convBps 1000 / reserveBps 0, and Milady's at the default 5 ETH (per-collection targets, J1)" || { echo "FAIL the per-collection targets did not land"; fail=1; }
    must "golive-check.sh on the NFT-only stack" script/golive-check.sh $RPC "$F" "$S"
    echo "$LAST" | grep -E "^(FAIL|note)" | sed 's/^/     /'; echo "     $(echo "$LAST" | grep -c '^ok ') checks ok"
    grep -q "bayc factory.normalRaise = the stack's" <<< "$LAST" && echo "ok   golive-check reads each collection's own targets and module parameters" || { echo "FAIL no per-collection target check"; fail=1; }
    grep -q "posterSource.poster is the custodian" <<< "$LAST" && grep -q "module.keeper = the canister" <<< "$LAST" \
      && echo "ok   it reads each poster source and module's keeper (the canister)" || { echo "FAIL no poster-source check"; fail=1; }
    [ "$(echo "$LAST" | grep -c "is in the operator's launch list (D1)")" = "$L_N" ] && echo "ok   the $L_N collections are the allowlist (D1; docs/v7/dormant-collections.md §1)" \
      || { echo "FAIL the allowlist (D1) was not read for each collection"; fail=1; }
    # the canister's eth_nft_config(), through a stand-in dfx that answers what an install at launch would (no IC is reached)
    ETHDIR=$(mktemp -d)
    printf '#!/bin/sh\ncase "$*" in *eth_nft_config*) f=eth;; *custody_outflow*) f=outflow;; *) f=caps;; esac\n[ -f "%s/$f" ] || exit 1\ncat "%s/$f"\n' "$ETHDIR" "$ETHDIR" > "$ETHDIR/dfx"; chmod +x "$ETHDIR/dfx"
    # review round 6 (B6-M-1): the canister's Ethereum pair is MEV Blocker + Tenderly's public gateway (keyless, independent, both
    # measured to serve logs and receipts over the 7-day sale window); xrpc.cl + publicnode (publicnode ~10,000 blocks deep) is refused below
    installhex "https://rpc.mevblocker.io,https://gateway.tenderly.co/public/mainnet" "$ETHDIR/install.keyless.hex" \
      && echo "ok   the reinstall's argument: a real buyer Init (monitor/src/candid.mjs), keyless providers (rpc.mevblocker.io, gateway.tenderly.co) in evm_chains[1]" \
      || { echo "FAIL the keyless install argument could not be built (monitor/node_modules?)"; fail=1; }
    # a FAKE Dwellir key (all zeros), as the operator's keyed URL would put it in the path
    installhex "https://api-ethereum-mainnet.n.dwellir.com/00000000-0000-0000-0000-000000000000,https://xrpc.cl/eth" "$ETHDIR/install.keyed.hex" \
      || { echo "FAIL the keyed install argument could not be built"; fail=1; }
    python3 - "$S" "$ETHDIR" "$BUYER" "$CAN" <<'PY'
import json, re, sys
s = json.load(open(sys.argv[1])); d = sys.argv[2]; buyer = sys.argv[3]; can = sys.argv[4]
cols = s["collections"]
LF = json.load(open("deployments/launch/nft-pilot.json"))
# review round 5 (the operator's option (c), 2026-09-28): the go/no-go read runs on the COMMITTED launch file, whose step 1 is the
# 48-item split at 12 / 24 a day; J7's 61 items (step 3) at the launch's 24 a day are shown refused from a copy (launch.j7at24.json)
L = {x["collection"].lower(): x for x in LF["collections"]}
J7 = {x["collection"].lower(): dict(x, custodyItems=LF["stages"][2]["custodyItems"][x["slug"]]) for x in LF["collections"]}
REAL = J7
j7 = json.loads(json.dumps(LF))
for x in j7["collections"]: x["custodyItems"] = LF["stages"][2]["custodyItems"][x["slug"]]
for k in ("stage", "_stage", "stages"): j7.pop(k, None)
json.dump(j7, open(f"{d}/launch.j7at24.json", "w"))
maxp = lambda c: str(L[c["custody"]["collection"].lower()]["maxPurchaseWei"])
# J7 (the operator, 2026-09-27): 15 ETH a collection, the launch file's item count, and the total the sum of the collections' (no binding total)
# docs/v7/dormant-collections.md §6: the install names the launch step's binding global (globalItems, globalWei); J7's copy the sum
def capsof(src, glob=None):
    citems = lambda c: int(src[c["custody"]["collection"].lower()]["custodyItems"])
    gi, gv = glob if glob else (sum(citems(c) for c in cols), 15 * 10**18 * len(cols))
    return [{"collections": [{"collection": f"1:{c['custody']['collection'].lower()}", "max_items": citems(c), "max_value_wei": "15_000_000_000_000_000_000",
             "max_item_wei": "15_000_000_000_000_000_000"} for c in cols], "global_max_items": gi,
             "global_max_value_wei": str(gv)}, []]
S1 = LF["stages"][0]
BIND = (int(S1["globalItems"]), int(S1["globalWei"])) if "globalItems" in S1 else None
caps = capsof(L, BIND)
json.dump(caps, open(f"{d}/caps", "w"))
json.dump(capsof(REAL), open(f"{d}/caps.real", "w"))
# custody_outflow(): the launch file's limits (nft-pilot.json outflow), and an empty limiter
o = LF["outflow"]
json.dump([{k: str(v) if k == "global_value_wei" else v for k, v in o.items()}, {"done": [], "asked": [], "freeze": {"Open": None}, "acked": []}],
    open(f"{d}/outflow", "w"))
# review round 5 (A5-I-1): a canister whose trip factor is 12 (within x4 of stage 0, and not the launch decision); step 2's limits
bad = dict(o, trip_factor=12)
json.dump([{k: str(v) if k == "global_value_wei" else v for k, v in bad.items()}, {"done": [], "asked": [], "freeze": {"Open": None}, "acked": []}],
    open(f"{d}/outflow.trip12", "w"))
o2 = LF["stages"][1]["outflow"]
json.dump([{k: str(v) if k == "global_value_wei" else v for k, v in o2.items()}, {"done": [], "asked": [], "freeze": {"Open": None}, "acked": []}],
    open(f"{d}/outflow.step2", "w"))
# docs/v7/dormant-collections.md §6: a canister whose global is J7's sum (not the launch step's binding 48 / 75 ETH), and one whose BAYC
# item count is not the launch file's
t = capsof(L)
json.dump(t, open(f"{d}/caps.total", "w"))
t = json.loads(json.dumps(caps)); [c.update(max_items=3) for c in t[0]["collections"] if c["collection"].endswith("0xbc4ca0eda7647a8ab7c2061c2e118a18a936f13d")]
t[0]["global_max_items"] += 1
json.dump(t, open(f"{d}/caps.items", "w"))
# docs/v7/dormant-collections.md §3.2: the launch file's dormancy, with the stack's WakeDesk
DORM = dict(LF["dormancy"], wake_desk=[s.get("wakeDesk", "")]) if LF.get("dormancy") else None
def eth(deposits, floor=None, extra=(), dorm=DORM):
    return [{"chain_id": "1", "relay_max_fee_bps": 300, "deposits": deposits, "floor": [] if floor is None else [floor],
             "dormancy": [] if dorm is None else [dorm],
             "collections": [{"collection": c["custody"]["collection"], "wrapper": c["wrapper"], "module": c["floorModule"],
                              "max_purchase_wei": maxp(c), "max_premium_bps": 500} for c in cols] + list(extra)}]
json.dump(eth(True), open(f"{d}/eth.launch", "w"))
json.dump(eth(True, dorm=None), open(f"{d}/eth.nodorm", "w"))
if DORM:
    json.dump(eth(True, dorm=dict(DORM, max_active=3)), open(f"{d}/eth.dorm4", "w"))
    # docs/v7/dormant-collections.md §11: a canister with fixed caps (no scaling), and one whose burn estimate is lower than the launch file's
    json.dump(eth(True, dorm={k: v for k, v in DORM.items() if k != "scaling"}), open(f"{d}/eth.fixed", "w"))
    json.dump(eth(True, dorm=dict(DORM, scaling=dict(DORM["scaling"], active_b_per_day=1500))), open(f"{d}/eth.lowburn", "w"))
    json.dump(eth(True, dorm=dict(DORM, wake_desk=["0x" + "de" * 20])), open(f"{d}/eth.desk", "w"))
json.dump(eth(False), open(f"{d}/eth.nodeposits", "w"))
json.dump(eth(True, {"min_listings": 3, "listing_rank": 1, "min_sales": 3, "up_cap_bps": 1000, "down_cap_bps": 1500}), open(f"{d}/eth.rank1", "w"))
json.dump(eth(True, extra=[{"collection": "0x8a90cab2b38dba80c64b7734e58ee1db38b8992e", "wrapper": "0x" + "ee" * 20, "module": "0x" + "ef" * 20,
          "max_purchase_wei": "7", "max_premium_bps": 500}]), open(f"{d}/eth.extra", "w"))
p = eth(True); p[0]["collections"][0]["max_premium_bps"] = 1000
json.dump(p, open(f"{d}/eth.premium", "w"))
# round 2 (D2-L-5): BAYC's purchase limit is the launch file's 8 ETH; one of 2 ETH is not the launch decision
p = eth(True); [c.update(max_purchase_wei="2000000000000000000") for c in p[0]["collections"] if c["collection"].lower() == "0xbc4ca0eda7647a8ab7c2061c2e118a18a936f13d"]
json.dump(p, open(f"{d}/eth.maxp", "w"))
# the launch file with BAYC's values shifted onto Pudgy (what shifted ETCH_NFT_* lists would have deployed)
lf = json.load(open("deployments/launch/nft-pilot.json"))
for x in lf["collections"]:
    if x["slug"] == "pudgypenguins": x.update(normalRaise=10 * 10**18, earlyRaise=10 * 10**18, convBps=1000, reserveBps=0)
    if x["slug"] == "bayc": [x.pop(k) for k in ("normalRaise", "earlyRaise", "convBps", "reserveBps")]
json.dump(lf, open(f"{d}/launch.shifted.json", "w"))
# the launch monitor configuration (deployments/launch/monitor-nft.json) filled in from the stack file
t = open("deployments/launch/monitor-nft.json").read().replace("<the canister's EVM address>", buyer)
for c in cols:
    t = t.replace(f"<{c['slug']} wrapper>", c["wrapper"]).replace(f"<{c['slug']} floorModule>", c["floorModule"])
t = t.replace("<the guardian EOA (registry.guardian())>", s.get("guardian") or "0x" + "11" * 20)
# round 3 (A3-L-3): golive-check refuses a <placeholder> ANYWHERE, so every one is filled: the canister ids, the timelock's roles and
# the liveness hosts with plausible stand-ins, the install argument with the keyless Init built above
t = re.sub(r"<(\w+) host>", r"\1.dry-run.invalid", t)
# the fueler's ops address (treasury.ops(), docs/v7/cycles-from-fees.md §4.2): a stand-in (the fueler block left it unfilled: every
# monitor read below FAILed on fueler.ops before this)
t = t.replace("<the fueler's address (treasury.ops())>", "0x" + "fe" * 20)
# ordinals at launch (nft-pilot.json `bitcoin`): the Bitcoin canary's three addresses (the canister's, after the install): stand-ins
t = t.replace("<the canister's custody_address()>", "bc1p5cyxnuxmeuwuvkwfem96lqzszd02n6xdcjrs20cac6yqjjwudpxqkedrcr")
t = t.replace("<the canister's float_address()>", "bc1p" + "q" * 58).replace("<the canister's buyer_address()>", "bc1q" + "z" * 38)
m = json.loads(t)
m["timelock"] = "r7inp-6aaaa-aaaaa-aaabq-cai"; m["buyer"] = can
m["expect"]["proposers"] = ["rdmx6-jaaaa-aaaaa-aaadq-cai", "qoctq-giaaa-aaaaa-aaaea-cai"]
m["expect"]["guardians"] = ["renrk-eyaaa-aaaaa-aaada-cai", "rno2w-sqaaa-aaaaa-aaacq-cai", "ryjl3-tyaaa-aaaaa-aaaba-cai"]
m["install_arg_hex"] = open(f"{d}/install.keyless.hex").read().strip()
t = json.dumps(m)
json.dump(m, open(f"{d}/monitor.json", "w"))
# A3-L-3: a guardian that is not the registry's; a placeholder left (liveness), one Ethereum provider twice, the wrong chain id
g = json.loads(t); g["evm"]["guardian"]["address"] = "0x000000000000000000000000000000000000dead"; json.dump(g, open(f"{d}/monitor.guard.json", "w"))
g = json.loads(t); g["liveness"][0]["url"] = "https://<runner host>/healthz"; g["eth_nft"]["eth_rpc"] = ["https://eth.drpc.org", "https://eth.drpc.org/"]
g["evm"]["chain_id"] = 1; json.dump(g, open(f"{d}/monitor.a3l3.json", "w"))
# the go/no-go read with no install argument anywhere (bayc10-decisions.md §5)
g = json.loads(t); del g["install_arg_hex"]; json.dump(g, open(f"{d}/monitor.noarg.json", "w"))
# A3-L-2: the launch file's names, symbols and slugs rotated by one (what a rotated ETCH_NFT_NAMES / _SYMBOLS / _SLUGS deploy is
# against the true file: the last collection's "Wrapped Identity MD / wIDMD" expected over the Miladys)
lr = json.load(open("deployments/launch/nft-pilot.json")); c = lr["collections"]
lab = [(x["name"], x["symbol"], x["slug"]) for x in c]; lab = lab[-1:] + lab[:-1]
for x, (n, y, g) in zip(c, lab): x["name"], x["symbol"], x["slug"] = n, y, g
json.dump(lr, open(f"{d}/launch.rotated.json", "w"))
# docs/v7/dormant-collections.md §11: the old 20 T start (one collection's runway with no reserve): under the first wake's 32.1 T liquid
lc = json.load(open("deployments/launch/nft-pilot.json")); lc["cycles"]["startT"] = 20; json.dump(lc, open(f"{d}/launch.cycles20.json", "w"))
m2 = json.loads(t); del m2["evm"]["guardian"]["pause_on_no_good_post_h"]; json.dump(m2, open(f"{d}/monitor.nopause.json", "w"))
m3 = json.loads(t); m3["eth_nft"]["collections"][2]["coingecko_id"] = ""; json.dump(m3, open(f"{d}/monitor.oneref.json", "w"))
# D8 per collection (the operator, 2026-10-09): OpenSea alone only where the launch file decides it (Identity MD): Milady's monitor entry
# on OpenSea alone is refused, and so is a launch file whose Identity MD has lost its _priceRefs note
m6 = json.loads(t); mm = next(x for x in m6["eth_nft"]["collections"] if x["slug"] == "milady"); mm.pop("coingecko_id"); mm["price_refs"] = ["opensea"]
json.dump(m6, open(f"{d}/monitor.miladyalone.json", "w"))
ln = json.load(open("deployments/launch/nft-pilot.json")); next(x for x in ln["collections"] if x["slug"] == "identitymd").pop("_priceRefs")
json.dump(ln, open(f"{d}/launch.nonote.json", "w"))
# review round 4 (A3-L-3): Milady's and MAYC's D8 references swapped (their floors are close); a placeholder not written as <...>
m4 = json.loads(t); ec = m4["eth_nft"]["collections"]
mi = next(i for i, x in enumerate(ec) if x["slug"] == "milady"); ma = next(i for i, x in enumerate(ec) if x["slug"] == "mayc")
for k in ("opensea_slug", "coingecko_id"): ec[mi][k], ec[ma][k] = ec[ma][k], ec[mi][k]
json.dump(m4, open(f"{d}/monitor.swapped.json", "w"))
m5 = json.loads(t); m5["liveness"][0]["url"] = "https://TODO/healthz"; json.dump(m5, open(f"{d}/monitor.todo.json", "w"))
PY
    cp "$ETHDIR/eth.launch" "$ETHDIR/eth"
    cp "$ETHDIR/caps" "$ETHDIR/caps.launch.fits"
    local LA=(--launch deployments/launch/nft-pilot.json --monitor-config "$ETHDIR/monitor.json")
    must "golive-check.sh --canister --launch --monitor-config: the launch install (D5 deposits on, D7 floor, Relay 300 bps, 5 % premium, chain 1, the stack's five collections, the custodian is the canister's key), the launch file's targets and purchase limits, the launch monitor configuration" \
      env DFX="$ETHDIR/dfx" script/golive-check.sh $RPC "$F" "$S" --canister $CAN --canister-evm $BUYER "${LA[@]}" --install-arg "$ETHDIR/install.keyless.hex"
    echo "     $(echo "$LAST" | grep -c '^ok ') checks ok"
    echo "$LAST" | grep "^ok   the install argument (--install-arg) names" | sed 's/^/     /'
    grep -q "^ok   the install argument (--install-arg) names 6 provider URLs, all keyless" <<< "$LAST" \
      && echo "ok   golive-check reads the reinstall's argument and finds its six provider URLs keyless (bayc10-decisions.md §5)" || { echo "FAIL the install argument was not read for keys"; fail=1; }
    [ "$(echo "$LAST" | grep -c "^ok   .*(A3-L-2)$")" = "$((3 * L_N))" ] \
      && echo "ok   every wrapper's name and symbol and the stack's slug are the launch file's, by collection address (A3-L-2)" || { echo "FAIL the wrappers' names, symbols and slugs were not held against the launch file"; fail=1; }
    grep -q "^ok   the monitor's evm.guardian.address is registry.guardian() (A3-L-3)" <<< "$LAST" && grep -q "with no placeholder left and two Ethereum providers by host" <<< "$LAST" \
      && echo "ok   the monitor file has no placeholder left, two Ethereum providers by host, and the registry's guardian (A3-L-3)" || { echo "FAIL no A3-L-3 monitor checks"; fail=1; }
    [ "$(echo "$LAST" | grep -c "= the launch file's")" -ge $((7 * L_N)) ] && [ "$(echo "$LAST" | grep -cE "monitor names (both D8 references|its one D8 reference, OpenSea alone)")" = "$L_N" ] \
      && [ "$(echo "$LAST" | grep -c "^ok   identitymd monitor names its one D8 reference, OpenSea alone")" = 1 ] \
      && grep -q "^ok   the launch file's D8 references: OpenSea and CoinGecko for every Ethereum collection, but OpenSea alone for identitymd" <<< "$LAST" \
      && echo "ok   every collection's targets, conversion, reserve and purchase limit are the launch file's, and the monitor watches all $L_N with both references (OpenSea alone for Identity MD, the launch file's priceRefs: the operator 2026-10-09)" \
      || { echo "FAIL the launch file or the monitor configuration was not read for each collection"; fail=1; }
    # J7 (the operator, 2026-09-27): 15 ETH a collection, the launch file's item counts; docs/v7/dormant-collections.md §6: the launch step's
    # binding global
    grep -q "bayc canister cap: items in custody = the launch file's custodyItems" <<< "$LAST" && grep -q "^ok   custody caps: the global item and value caps = the launch step's globalItems and globalWei" <<< "$LAST" \
      && echo "ok   J7: every collection's custody cap is the launch file's (15 ETH, its item count), and the global is the launch step's binding one" || { echo "FAIL no J7 / binding-global custody check"; fail=1; }
    # docs/v7/dormant-collections.md: the canister's dormancy and desk, the stack's WakeDesk read back, the cycles plan
    [ "$(echo "$LAST" | grep -c "^ok   .* canister eth_nft.dormancy = the launch file's")" = "$L_N" ] && [ "$(echo "$LAST" | grep -c "^ok   .* canister eth_nft.dormancy.wake_desk = the stack file's wakeDesk")" = "$L_N" ] \
      && grep -q "^ok   wakeDesk.bond = the launch file's wake.bondWei" <<< "$LAST" && grep -q "^ok   wakeDesk.directory is the stack's directory" <<< "$LAST" \
      && grep -q "^ok   the starting balance gives effective_max_active 1 >= 1 for 7 days with the reserve, and the first wake fits" <<< "$LAST" \
      && echo "ok   the dormancy: the canister's is the launch file's, its desk the stack's; the WakeDesk read back; the cycles plan covers its days (docs/v7/dormant-collections.md)" \
      || { echo "FAIL no dormancy, wake desk or cycles check"; fail=1; }
    echo "$LAST" | grep "^     cycles" | sed 's/^/     /' 
    # review round 4 (A4-L-1): the whole custody leaves within the delay, on the launch file and on the canister's installed pair
    grep -q "^ok   the launch file's custody (custodyItems) at its outflow limits: every item can leave" <<< "$LAST" \
      && grep -q "^ok   the canister's custody_caps() at its custody_outflow() limits: every item can leave" <<< "$LAST" \
      && echo "ok   A4-L-1: every item of the five collections can leave within the timelock's 72 h less a day (the COMMITTED launch file and the canister's pair; step 1's item caps: $(python3 -c 'import json,sys; print(", ".join(f"{x["slug"]} {x["custodyItems"]}" for x in json.load(open(sys.argv[1]))["collections"]))' deployments/launch/nft-pilot.json))" \
      || { echo "FAIL no A4-L-1 exit check"; fail=1; }
    # review round 5 (option (c)): the staged plan, and the canister's whole custody_outflow() held to the launch file (A5-I-1)
    echo "$LAST" | grep "^ok   the launch file's staged plan (option (c))" | sed 's/^/     /'
    grep -q "^ok   the launch file's staged plan (option (c))" <<< "$LAST" && [ "$(echo "$LAST" | grep -c "^ok   the canister's outflow limit .* = the launch file's (step 1, A5-I-1)")" = 6 ] \
      && echo "ok   option (c): the launch file's three steps (48 at 24, 48 at 31, 62 at 31; Bitcoin Frogs' 3 counted in each) each fit, and the canister's six outflow limits are step 1's (A5-I-1)" \
      || { echo "FAIL no staged-plan or A5-I-1 check"; fail=1; }
    grep -q "^ok   the install argument (--install-arg) names two provider hosts or more for each of its 3 chain(s) (A4-L-3)" <<< "$LAST" \
      && echo "ok   A4-L-3: the canister's providers are two hosts on each chain" || { echo "FAIL no A4-L-3 host count"; fail=1; }
    grep -q "the launch file's for this collection (A3-L-3)" <<< "$LAST" && echo "ok   A3-L-3: the monitor's D8 references are the launch file's, by collection" \
      || { echo "FAIL the D8 references were not held against the launch file"; fail=1; }
    # J7's 61 items against the launch's 24 deliveries a day in all (review round 4, A4-L-1; step 3 without step 2): refused, with the options
    local XOUT; cp "$ETHDIR/caps.real" "$ETHDIR/caps"
    XOUT=$(env DFX="$ETHDIR/dfx" script/golive-check.sh $RPC "$F" "$S" --canister $CAN --canister-evm $BUYER --launch "$ETHDIR/launch.j7at24.json" \
      --monitor-config "$ETHDIR/monitor.json" --install-arg "$ETHDIR/install.keyless.hex" 2>&1)
    if [ $? -ne 0 ] && echo "$XOUT" | grep -q "^FAIL the launch file's custody (custodyItems) at its outflow limits: custody holds up to [0-9]* items in all and lets 24 out a day in all: 48 can leave" \
      && echo "$XOUT" | grep -q "^FAIL the canister's custody_caps() at its custody_outflow() limits: custody holds up to [0-9]* items"; then
      echo "ok   refused: J7's item counts with no binding global at the launch's 24 deliveries a day in all (48 can leave: step 3 before step 2, and no global), on a launch file and on the canister (A4-L-1)"
      echo "$XOUT" | grep "^FAIL the launch file's custody" | sed 's/^FAIL /     /' | fold -s -w 150 | sed 's/^/     /'
    else echo "FAIL the operator's launch pair was not refused: $(echo "$XOUT" | grep -E 'A4-L-1' | head -2)"; fail=1; fi
    cp "$ETHDIR/caps.launch.fits" "$ETHDIR/caps" 2>/dev/null || true
    # review round 5 (A5-I-1): a canister whose trip factor is not the launch file's
    cp "$ETHDIR/outflow" "$ETHDIR/outflow.launch"; cp "$ETHDIR/outflow.trip12" "$ETHDIR/outflow"
    expect "a canister whose outflow trip factor is 12, not the launch file's 3 (A5-I-1)" "FAIL the canister's outflow limit trip_factor = the launch file's (step 1, A5-I-1): expected 3, the chain says 12" \
      env DFX="$ETHDIR/dfx" script/golive-check.sh $RPC "$F" "$S" --canister $CAN --canister-evm $BUYER "${LA[@]}"
    # option (c) step 2 installed: the canister at 31 a day is held to step 2 with --stage 2, and refused as step 1
    cp "$ETHDIR/outflow.step2" "$ETHDIR/outflow"
    must "golive-check.sh --stage 2: the canister after the outflow raise (option (c) step 2)" env DFX="$ETHDIR/dfx" script/golive-check.sh $RPC "$F" "$S" --canister $CAN --canister-evm $BUYER "${LA[@]}" --stage 2
    grep -q "^ok   the canister's outflow limit global_items = the launch file's (step 2, A5-I-1)" <<< "$LAST" && echo "ok   step 2's canister is held to step 2's numbers" || { echo "FAIL --stage 2 was not read"; fail=1; }
    expect "the canister after the outflow raise read as step 1 (the launch file's stage)" "FAIL the canister's outflow limit global_items = the launch file's (step 1, A5-I-1): expected 24, the chain says 31" \
      env DFX="$ETHDIR/dfx" script/golive-check.sh $RPC "$F" "$S" --canister $CAN --canister-evm $BUYER "${LA[@]}"
    cp "$ETHDIR/outflow.launch" "$ETHDIR/outflow"
    # option (c): the pending upgrade's argument against the staged step (golive-check --proposal): the outflow raise passes, an
    # "outflow raise" that also carries J7's caps (built from J7's numbers) is refused, and a caps-first one too
    stagedhex 1 "$ETHDIR/step1.hex" && stagedhex 2 "$ETHDIR/step2.hex" && stagedhex 3 "$ETHDIR/step3.hex" && stagedhex 3x "$ETHDIR/capsfirst.hex" \
      || { echo "FAIL the staged arguments could not be built"; fail=1; }
    must "golive-check.sh --proposal: option (c) step 2 (the outflow raise alone) against the install's argument" \
      script/golive-check.sh --proposal "$ETHDIR/step2.hex" --launch deployments/launch/nft-pilot.json --stage 2 --baseline "$ETHDIR/step1.hex"
    echo "$LAST" | grep -E "^(ok|FAIL|note)" | sed 's/^/     /'
    must "golive-check.sh --proposal: step 2 against the running canister (its custody_caps() and custody_outflow() at step 1)" \
      env DFX="$ETHDIR/dfx" script/golive-check.sh --proposal "$ETHDIR/step2.hex" --launch deployments/launch/nft-pilot.json --stage 2 --canister $CAN
    must "golive-check.sh --proposal: option (c) step 3 (the caps back to J7) against step 2's argument" \
      script/golive-check.sh --proposal "$ETHDIR/step3.hex" --launch deployments/launch/nft-pilot.json --stage 3 --baseline "$ETHDIR/step2.hex"
    expect "an outflow-raise proposal that carries J7's caps too (both at once: the two-week gate skipped)" "FAIL .*custody caps move too\|FAIL the proposal's custody caps and outflow limits are step 2" \
      script/golive-check.sh --proposal "$ETHDIR/step3.hex" --launch deployments/launch/nft-pilot.json --stage 2 --baseline "$ETHDIR/step1.hex"
    expect "a caps-first proposal (J7's caps, the global 62, at 24 a day)" "does not fit the exit rule" \
      script/golive-check.sh --proposal "$ETHDIR/capsfirst.hex" --launch deployments/launch/nft-pilot.json --stage 3 --baseline "$ETHDIR/step1.hex"
    expect "a --proposal without its step" "needs --stage" script/golive-check.sh --proposal "$ETHDIR/step2.hex" --launch deployments/launch/nft-pilot.json
    # review round 4 (A3-L-3): swapped D8 references, a TODO placeholder
    expect "a monitor configuration with Milady's and MAYC's D8 references swapped (A3-L-3)" "names references that are not the launch file's openseaSlug / coingeckoId for this collection (A3-L-3)" \
      env DFX="$ETHDIR/dfx" script/golive-check.sh $RPC "$F" "$S" --canister $CAN --canister-evm $BUYER --launch deployments/launch/nft-pilot.json --monitor-config "$ETHDIR/monitor.swapped.json"
    expect "a monitor configuration with a TODO left in a URL (A3-L-3)" "a <placeholder> is left in the monitor file (liveness.0..url)" \
      env DFX="$ETHDIR/dfx" script/golive-check.sh $RPC "$F" "$S" --canister $CAN --canister-evm $BUYER --launch deployments/launch/nft-pilot.json --monitor-config "$ETHDIR/monitor.todo.json"
    # review round 4 (A4-L-3): a key in a subdomain, a 15-character key, and one Ethereum host twice in the canister's install argument
    installhex "https://f3a9c1d2e4b56b7c.eth.rpc.example.net/,https://xrpc.cl/eth" "$ETHDIR/install.sub.hex" || { echo "FAIL install.sub"; fail=1; }
    installhex "https://rpc.example.net/abcdefghij12345,https://xrpc.cl/eth" "$ETHDIR/install.short.hex" || { echo "FAIL install.short"; fail=1; }
    installhex "https://xrpc.cl/eth,https://xrpc.cl/eth/" "$ETHDIR/install.onehost.hex" || { echo "FAIL install.onehost"; fail=1; }
    local SOUT; SOUT=$(env DFX="$ETHDIR/dfx" script/golive-check.sh $RPC "$F" "$S" --canister $CAN --canister-evm $BUYER "${LA[@]}" --install-arg "$ETHDIR/install.sub.hex" 2>&1)
    if [ $? -ne 0 ] && echo "$SOUT" | grep -q "a subdomain label that looks like a key" && ! echo "$SOUT" | grep -q "f3a9c1d2e4b56b7c"; then
      echo "ok   refused: a key in a provider's subdomain (A4-L-3), and the key is printed nowhere"
    else echo "FAIL a subdomain key: $(echo "$SOUT" | grep -E 'KEYED|keyless|f3a9' | head -2)"; fail=1; fi
    expect "an install argument with a 15-character key in a provider's path (A4-L-3)" "a path part of 12+ key characters" \
      env DFX="$ETHDIR/dfx" script/golive-check.sh $RPC "$F" "$S" --canister $CAN --canister-evm $BUYER "${LA[@]}" --install-arg "$ETHDIR/install.short.hex"
    expect "an install argument whose Ethereum providers are one host twice (xrpc.cl/eth and xrpc.cl/eth/, A4-L-3)" "names 1 provider host(s) for chain 1 (xrpc.cl)" \
      env DFX="$ETHDIR/dfx" script/golive-check.sh $RPC "$F" "$S" --canister $CAN --canister-evm $BUYER "${LA[@]}" --install-arg "$ETHDIR/install.onehost.hex"
    # review round 6 (B6-M-1): the round-5 pair (xrpc.cl + publicnode, whose keyless logs stop ~10,000 blocks back) under --phase1
    installhex "https://xrpc.cl/eth,https://ethereum-rpc.publicnode.com" "$ETHDIR/install.shallow.hex" || { echo "FAIL install.shallow"; fail=1; }
    expect "an install argument whose canister Ethereum providers are not known to serve the 7-day sale window (xrpc.cl + publicnode, B6-M-1)" "is not known to serve eth_getLogs and receipts over the 7-day window" \
      env DFX="$ETHDIR/dfx" GOLIVE_REHEARSAL=1 script/golive-check.sh $RPC "$F" "$S" --phase1 --canister $CAN --canister-evm $BUYER "${LA[@]}" --install-arg "$ETHDIR/install.shallow.hex"
    # audit 2026-09-27-nft round 2 (D2-L-5, A2-I-2): what the go/no-go read now refuses
    expect "a deploy whose targets are not the launch file's (BAYC's 10 ETH / 1000 / 0 shifted onto Pudgy)" "FAIL pudgypenguins factory.normalRaise = the launch file's" \
      env DFX="$ETHDIR/dfx" script/golive-check.sh $RPC "$F" "$S" --canister $CAN --canister-evm $BUYER --launch "$ETHDIR/launch.shifted.json" --monitor-config "$ETHDIR/monitor.json"
    # round 3 (bayc10-decisions.md §5, §7): a keyed provider URL in the reinstall's argument is refused, and its key never printed
    local KOUT KCODE
    KOUT=$(env DFX="$ETHDIR/dfx" script/golive-check.sh $RPC "$F" "$S" --canister $CAN --canister-evm $BUYER "${LA[@]}" --install-arg "$ETHDIR/install.keyed.hex" 2>&1); KCODE=$?
    if [ $KCODE -ne 0 ] && echo "$KOUT" | grep -q "^FAIL the install argument (--install-arg) names a KEYED provider URL https://api-ethereum-mainnet.n.dwellir.com/…" \
      && ! echo "$KOUT" | grep -q "00000000-0000-0000-0000-000000000000"; then
      echo "ok   refused: an install argument whose Ethereum provider carries a (fake) Dwellir key, and the key is printed nowhere"
      echo "$KOUT" | grep "KEYED provider URL" | sed 's/^/     /'
    else echo "FAIL a keyed provider URL in the install argument: exit $KCODE: $(echo "$KOUT" | grep -E 'KEYED|install argument' | head -3)"; fail=1; fi
    expect "the go/no-go read (--phase1) of an NFT stack with no install argument (none given, none in the monitor file)" "pass --install-arg <the reinstall's argument>: its provider URLs are public and must be keyless" \
      env DFX="$ETHDIR/dfx" script/golive-check.sh $RPC "$F" "$S" --phase1 --canister $CAN --canister-evm $BUYER --launch deployments/launch/nft-pilot.json --monitor-config "$ETHDIR/monitor.noarg.json"
    # round 3 (A3-L-3, A3-L-2)
    expect "a monitor configuration whose guardian is not the registry's (A3-L-3)" "FAIL the monitor's evm.guardian.address is registry.guardian() (A3-L-3)" \
      env DFX="$ETHDIR/dfx" script/golive-check.sh $RPC "$F" "$S" --canister $CAN --canister-evm $BUYER --launch deployments/launch/nft-pilot.json --monitor-config "$ETHDIR/monitor.guard.json"
    expect "a monitor configuration with <runner host> left, eth.drpc.org twice (with and without /) and evm.chain_id 1 (A3-L-3)" \
      "a <placeholder> is left in the monitor file (liveness.0..url).*counted by host: 1.*evm.chain_id is 1, not 4663" \
      env DFX="$ETHDIR/dfx" script/golive-check.sh $RPC "$F" "$S" --canister $CAN --canister-evm $BUYER --launch deployments/launch/nft-pilot.json --monitor-config "$ETHDIR/monitor.a3l3.json"
    expect "a launch file whose names, symbols and slugs are rotated by one (A3-L-2: the last collection's Wrapped Identity MD / wIDMD expected over the Miladys)" \
      "FAIL milady wrapper.name() = the launch file's name (A3-L-2): expected 'Wrapped Identity MD', found 'Wrapped Milady'" \
      env DFX="$ETHDIR/dfx" script/golive-check.sh $RPC "$F" "$S" --canister $CAN --canister-evm $BUYER --launch "$ETHDIR/launch.rotated.json" --monitor-config "$ETHDIR/monitor.json"
    cp "$ETHDIR/eth.maxp" "$ETHDIR/eth"
    expect "a canister whose max_purchase_wei is not the launch file's (BAYC at 2 ETH, not 8)" "FAIL bayc canister max_purchase_wei = the launch file's" \
      env DFX="$ETHDIR/dfx" script/golive-check.sh $RPC "$F" "$S" --canister $CAN --canister-evm $BUYER "${LA[@]}"
    cp "$ETHDIR/eth.launch" "$ETHDIR/eth"
    cp "$ETHDIR/caps" "$ETHDIR/caps.launch"
    cp "$ETHDIR/caps.total" "$ETHDIR/caps"
    expect "a canister whose global is J7's sum, not the launch step's binding 48 items / 75 ETH (docs/v7/dormant-collections.md §6)" "FAIL custody caps: the global item and value caps = the launch step's globalItems and globalWei" \
      env DFX="$ETHDIR/dfx" script/golive-check.sh $RPC "$F" "$S" --canister $CAN --canister-evm $BUYER "${LA[@]}"
    cp "$ETHDIR/caps.items" "$ETHDIR/caps"
    expect "a canister whose BAYC custody item count is not the launch file's (3, not 2)" "FAIL bayc canister cap: items in custody = the launch file's custodyItems" \
      env DFX="$ETHDIR/dfx" script/golive-check.sh $RPC "$F" "$S" --canister $CAN --canister-evm $BUYER "${LA[@]}"
    cp "$ETHDIR/caps.launch" "$ETHDIR/caps"
    expect "a monitor configuration without the no-good-post poster pause" "pause_on_no_good_post_h is not armed" \
      env DFX="$ETHDIR/dfx" script/golive-check.sh $RPC "$F" "$S" --canister $CAN --canister-evm $BUYER --launch deployments/launch/nft-pilot.json --monitor-config "$ETHDIR/monitor.nopause.json"
    expect "a monitor configuration missing one collection's CoinGecko reference" "lacks one of D8's references" \
      env DFX="$ETHDIR/dfx" script/golive-check.sh $RPC "$F" "$S" --canister $CAN --canister-evm $BUYER --launch deployments/launch/nft-pilot.json --monitor-config "$ETHDIR/monitor.oneref.json"
    expect "a monitor configuration with Milady on OpenSea alone (price_refs where the launch file does not decide it)" "FAIL milady: the monitor configuration lacks one of D8's references" \
      env DFX="$ETHDIR/dfx" script/golive-check.sh $RPC "$F" "$S" --canister $CAN --canister-evm $BUYER --launch deployments/launch/nft-pilot.json --monitor-config "$ETHDIR/monitor.miladyalone.json"
    expect "a launch file whose Identity MD has priceRefs but no _priceRefs note (the operator's decision unrecorded)" "FAIL the launch file's D8 references: identitymd: priceRefs \\[\"opensea\"\\] without its _priceRefs note" \
      env DFX="$ETHDIR/dfx" script/golive-check.sh $RPC "$F" "$S" --canister $CAN --canister-evm $BUYER --launch "$ETHDIR/launch.nonote.json" --monitor-config "$ETHDIR/monitor.json"
    expect "the go/no-go read (--phase1) of an NFT stack without --canister-evm (a FAIL now, not a WARN)" "--canister-evm is required under --phase1" \
      env DFX="$ETHDIR/dfx" script/golive-check.sh $RPC "$F" "$S" --phase1 --canister $CAN "${LA[@]}"
    expect "the go/no-go read (--phase1) of an NFT stack without the launch file and the monitor configuration" "without --launch" \
      env DFX="$ETHDIR/dfx" script/golive-check.sh $RPC "$F" "$S" --phase1 --canister $CAN --canister-evm $BUYER
    expect "a stack whose custodian is not the canister's EVM address (audit 2026-09-27-nft D-L-6)" "custodian is the canister's EVM address" \
      env DFX="$ETHDIR/dfx" script/golive-check.sh $RPC "$F" "$S" --canister $CAN --canister-evm 0x000000000000000000000000000000000000beef
    cp "$ETHDIR/eth.premium" "$ETHDIR/eth"
    expect "a canister whose buy premium is 10 % (the Trust page and the launch say 5 %)" "max_premium_bps" env DFX="$ETHDIR/dfx" script/golive-check.sh $RPC "$F" "$S" --canister $CAN
    cp "$ETHDIR/eth.launch" "$ETHDIR/eth"
    echo "$LAST" | grep -E "eth_nft|collections are exactly" | head -6 | sed 's/^/     /'
    cp "$ETHDIR/eth.nodeposits" "$ETHDIR/eth"
    expect "a canister with holder deposits off (D5 is on at launch)" "D5 is deposits ON" env DFX="$ETHDIR/dfx" script/golive-check.sh $RPC "$F" "$S" --canister $CAN
    # docs/v7/dormant-collections.md §3.2: a canister whose dormancy is not the launch file's, or whose desk is not the stack's
    cp "$ETHDIR/eth.dorm4" "$ETHDIR/eth"
    expect "a canister whose ceiling max_active is 3 (the launch file's is 10)" "canister eth_nft.dormancy = the launch file's" env DFX="$ETHDIR/dfx" script/golive-check.sh $RPC "$F" "$S" --canister $CAN "${LA[@]}"
    cp "$ETHDIR/eth.fixed" "$ETHDIR/eth"
    expect "a canister with fixed caps (no scaling from its cycles balance: docs/v7/dormant-collections.md §11)" "canister eth_nft.dormancy = the launch file's" env DFX="$ETHDIR/dfx" script/golive-check.sh $RPC "$F" "$S" --canister $CAN "${LA[@]}"
    cp "$ETHDIR/eth.lowburn" "$ETHDIR/eth"
    expect "a canister whose scaling counts 1.5 T a day per awake collection (the launch file's 2.5 T: more would wake on the same balance)" "canister eth_nft.dormancy = the launch file's" env DFX="$ETHDIR/dfx" script/golive-check.sh $RPC "$F" "$S" --canister $CAN "${LA[@]}"
    cp "$ETHDIR/eth.launch" "$ETHDIR/eth"
    expect "a launch file starting at 20 T (under the first wake's 32.1 T liquid: one collection's runway, its exit reserve, the reserve and the wake)" "does not keep one collection awake for 7 days with the reserve" \
      env DFX="$ETHDIR/dfx" script/golive-check.sh $RPC "$F" "$S" --canister $CAN --launch "$ETHDIR/launch.cycles20.json" --monitor-config "$ETHDIR/monitor.json"
    cp "$ETHDIR/eth.desk" "$ETHDIR/eth"
    expect "a canister reading a wake desk that is not the stack's" "canister eth_nft.dormancy.wake_desk = the stack file's wakeDesk" env DFX="$ETHDIR/dfx" script/golive-check.sh $RPC "$F" "$S" --canister $CAN "${LA[@]}"
    cp "$ETHDIR/eth.nodorm" "$ETHDIR/eth"
    # read without a launch file, the stand-in's binding global (the launch step's 48 items) is J7's FAIL, and nothing else is
    LAST=$(env DFX="$ETHDIR/dfx" script/golive-check.sh $RPC "$F" "$S" --canister $CAN 2>&1)
    if [ -z "$(grep '^FAIL' <<< "$LAST" | grep -v '^FAIL custody caps: .*J7: no total')" ]; then echo "ok   a canister with no dormancy, read without a launch file: no FAIL but J7's (the stand-in holds the launch step's binding global)"
    else echo "FAIL a canister with no dormancy, read without a launch file: $(grep '^FAIL' <<< "$LAST" | head -3)"; fail=1; fi
    grep -q "^note .*keeps every collection always on (eth_nft.dormancy = null)" <<< "$LAST" && echo "ok   and it notes that the burn then scales with every listed collection" || { echo "FAIL no note for a canister with no dormancy"; fail=1; }
    expect "a canister with no dormancy against the launch file that names one" "canister eth_nft.dormancy = the launch file's" env DFX="$ETHDIR/dfx" script/golive-check.sh $RPC "$F" "$S" --canister $CAN "${LA[@]}"
    cp "$ETHDIR/eth.rank1" "$ETHDIR/eth"
    expect "a canister whose floor is the cheapest listing (D7 is rank 2 of >= 3)" "is not the D7 rule" env DFX="$ETHDIR/dfx" script/golive-check.sh $RPC "$F" "$S" --canister $CAN
    cp "$ETHDIR/eth.extra" "$ETHDIR/eth"
    expect "a canister that serves a collection the stack does not name" "the canister's eth_nft collections are exactly the stack's" env DFX="$ETHDIR/dfx" script/golive-check.sh $RPC "$F" "$S" --canister $CAN
    rm -rf "$ETHDIR"
    cast rpc --rpc-url $RPC anvil_setBalance $BUYER 0xDE0B6B3A7640000 >/dev/null
    # each module takes its own collection's floor (the operator's measured floors, 2026-09-27); a first post is not move-limited
    local SL FL
    for ((k = 0; k < NC; k++)); do
      M=$(python3 -c "import json;print(json.load(open('$S'))['collections'][$k]['floorModule'])")
      SL=$(python3 -c "import json;print(json.load(open('$S'))['collections'][$k]['slug'])")
      case "$SL" in bayc) FL=6790000000000000000;; pudgypenguins) FL=3353000000000000000;; azuki) FL=670000000000000000;; mayc) FL=873500000000000000;;
        doodles) FL=251756311900000000;; moonbirds) FL=409163870000000000;; clonex) FL=135306190000000000;; lilpudgys) FL=314989990000000000;; checks) FL=159000000000000000;; identitymd) FL=1780000000000000000;; *) FL=930000000000000000;; esac
      must "the canister posts $SL's floor (postFloor from the custodian's address)" cast send --rpc-url $RPC --unlocked --from $BUYER $M 'postFloor(uint256,uint16,uint32)' $FL 10000 5
      [ "$(cast call --rpc-url $RPC $M 'accepted()(uint256)' | awk '{print $1}')" = "$FL" ] && echo "ok   $SL's accepted floor is the canister's post ($FL)" || { echo "FAIL the post did not land"; fail=1; }
    done
    must "golive-check after the posts (items per graduation at today's floor)" script/golive-check.sh $RPC "$F" "$S"
    echo "$LAST" | grep "bayc: one graduation at the normal target converts 1.1[0-9] items" | grep -qv POOLED \
      && echo "ok   it says one BAYC graduation converts 1.14 apes at 6.79 (J1: one graduation, one ape)" || { echo "FAIL BAYC's items per graduation is not J1's"; echo "$LAST" | grep "converts" | sed 's/^/     /'; fail=1; }
    grep -q "pudgypenguins: one graduation at the normal target converts 0.8[0-9] items .* POOLED: 2 graduations per item" <<< "$LAST" \
      && echo "ok   it says Pudgy pools two graduations per item at its 5 ETH normal target" || { echo "FAIL no pooled note for Pudgy"; echo "$LAST" | grep "converts" | sed 's/^/     /'; fail=1; }
    M=$(python3 -c "import json;print(json.load(open('$S'))['collections'][0]['floorModule'])")
    expect "the keeper EOA cannot post an NFT module's floor" "revert" cast send --rpc-url $RPC --unlocked --from $K $M 'postFloor(uint256,uint16,uint32)' 1 10000 5
    cast rpc --rpc-url $RPC anvil_setBalance $GUARD 0xDE0B6B3A7640000 >/dev/null
    # audit 2026-09-27-nft D-M-1: the guardian's D8 pause is ONE collection's poster: that module stops, every other goes on
    local PS0 PS1 M1
    PS0=$(python3 -c "import json;print(json.load(open('$S'))['collections'][0]['posterSource'])"); PS1=$(python3 -c "import json;print(json.load(open('$S'))['collections'][1]['posterSource'])")
    M1=$(python3 -c "import json;print(json.load(open('$S'))['collections'][1]['floorModule'])")
    must "the guardian pauses one collection's poster (PosterSource.pause)" cast send --rpc-url $RPC --unlocked --from $GUARD $PS0 'pause()'
    [ "$(cast call --rpc-url $RPC $M 'keeper()(address)' | lc)" = "0x0000000000000000000000000000000000000000" ] && [ "$(cast call --rpc-url $RPC $M1 'keeper()(address)' | lc)" = "$(echo $BUYER | lc)" ] \
      && echo "ok   one poster paused: that module's keeper is zero, the next collection's is still the canister" || { echo "FAIL the poster pause is not per collection"; fail=1; }
    expect "golive-check with a paused poster" "posterSource.paused() is false" script/golive-check.sh $RPC "$F" "$S"
    must "the canister still posts the next collection's floor" cast send --rpc-url $RPC --unlocked --from $BUYER $M1 'postFloor(uint256,uint16,uint32)' 670000000000000000 10000 5
    expect "the guardian cannot unpause a poster (the Safe only)" "revert" cast send --rpc-url $RPC --unlocked --from $GUARD $PS0 'unpause()'
    cast send --rpc-url $RPC --unlocked --from $O $PS0 'unpause()' >/dev/null 2>&1 || true
    cast send --rpc-url $RPC --unlocked --from $GUARD $R 'pauseKeeper()' >/dev/null 2>&1
    local zero=1
    for ((k = 0; k < NC; k++)); do
      M=$(python3 -c "import json;print(json.load(open('$S'))['collections'][$k]['floorModule'])"); PS=$(python3 -c "import json;print(json.load(open('$S'))['collections'][$k]['posterSource'])")
      [ "$(cast call --rpc-url $RPC $M 'keeper()(address)' | lc)" = "0x0000000000000000000000000000000000000000" ] && [ "$(cast call --rpc-url $RPC $PS 'keeper()(address)' | lc)" = "0x0000000000000000000000000000000000000000" ] || zero=0
    done
    [ $zero = 1 ] && echo "ok   the registry's pause zeroes every collection's poster (module.keeper() = 0: the canister-wide switch)" || { echo "FAIL the pause did not zero every poster"; fail=1; }
    expect "the canister cannot post while the keeper is paused" "revert" cast send --rpc-url $RPC --unlocked --from $BUYER $M 'postFloor(uint256,uint16,uint32)' 930000000000000000 10000 5
  fi
  cast rpc --rpc-url $RPC evm_revert "$SNAP" >/dev/null; rm -f "$S"; mv "$S.btc" "$S"
}

# Ordinals at launch (the operator, 2026-10-09; deployments/launch/nft-pilot.json `bitcoin`, docs/v7/golive-runbook.md §9): the MIXED
# stack, one deploy-stack.sh run of the default kind with the eleven Ethereum collections (ETCH_NFT_COLLECTIONS …), over a Bitcoin listing
# made with the launch file's membership root; then golive-check --launch --canister --install-arg --monitor-config reads it whole
# (the canister through a stand-in dfx answering what the pre-handover setup leaves: the Bitcoin cap, its floor sink and limit, UniSat,
# the fee pool, the Ethereum config), and each Bitcoin setter left undone FAILs. On a snapshot, then restored with the frog stack's file.
rehearse_mixed_stack() { # rehearse_mixed_stack <chainid> <deployment file> <stack file> <registry>
  local CH="$1" F="$2" S="$3" R="$4" SNAP MID MDIR CAN=rrkah-fqaaa-aaaaa-aaaaq-cai
  step "STACK (MIXED): Bitcoin Frogs and the eleven Ethereum collections in ONE deploy-stack.sh run (the launch: docs/v7/golive-runbook.md §9)"
  SNAP=$(cast rpc --rpc-url $RPC evm_snapshot | tr -d '"'); mv "$S" "$S.frogs"
  local B_ROOT B_NAME B_SYM B_SLUG B_LNAME B_LREF L_COLS L_NAMES L_SYMS L_SLUGS L_NORMAL L_EARLY L_CONV L_RES L_BOND L_WINDOW L_N
  eval "$(python3 - <<'PY'
import json, shlex
L = json.load(open("deployments/launch/nft-pilot.json")); c = L["collections"]; b = L["bitcoin"]
j = lambda k, d=None: ",".join(str(x.get(k, d)) for x in c)
for var, val in [("B_ROOT", b["membersRoot"]), ("B_NAME", b["name"]), ("B_SYM", b["symbol"]), ("B_SLUG", b["slug"]), ("B_LNAME", b["listName"]), ("B_LREF", b["listRef"]),
                 ("L_COLS", j("collection")), ("L_NAMES", j("name")), ("L_SYMS", j("symbol")), ("L_SLUGS", j("slug")),
                 ("L_NORMAL", j("normalRaise", 5 * 10**18)), ("L_EARLY", j("earlyRaise", 8 * 10**18)), ("L_CONV", j("convBps", 300)), ("L_RES", j("reserveBps", 2500)),
                 ("L_BOND", str(L["wake"]["bondWei"])), ("L_WINDOW", str(L["wake"]["windowSecs"])), ("L_N", str(len(c)))]:
    print(f"{var}={shlex.quote(val)}")
PY
)"
  must "script/list-auto.sh: the Bitcoin listing with the launch file's membership root ($B_ROOT)" listenv env ETCH_LIST_NAME="$B_LNAME" ETCH_LIST_REF="$B_LREF" ETCH_LIST_ROOT="$B_ROOT" \
    ETCH_REGISTRY=$R ETCH_BUYER_CONFIRM="buyer-is:$CH:$(echo $BUYER | lc)" script/list-auto.sh $RPC || { cast rpc --rpc-url $RPC evm_revert "$SNAP" >/dev/null; mv "$S.frogs" "$S"; return; }
  MID=$(( $(cast call --rpc-url $RPC $R 'count()(uint256)') - 1 ))
  if ! must "script/deploy-stack.sh, the default kind: $B_SLUG over listing $MID AND the $L_N Ethereum collections with the WakeDesk (one stack file)" stackenv env \
      ETCH_STACK_SLUG="$B_SLUG" ETCH_WRAPPER_NAME="$B_NAME" ETCH_WRAPPER_SYMBOL="$B_SYM" ETCH_NFT_CHAIN_ID=1 ETCH_WAKE_BOND="$L_BOND" ETCH_WAKE_WINDOW="$L_WINDOW" \
      ETCH_NFT_COLLECTIONS="$L_COLS" ETCH_NFT_NAMES="$L_NAMES" ETCH_NFT_SYMBOLS="$L_SYMS" ETCH_NFT_SLUGS="$L_SLUGS" \
      ETCH_NFT_NORMAL_RAISES="$L_NORMAL" ETCH_NFT_EARLY_RAISES="$L_EARLY" ETCH_NFT_CONV_BPS_LIST="$L_CONV" ETCH_NFT_RESERVE_BPS_LIST="$L_RES" \
      ETCH_STACK_CONFIRM="deploy-etch-stack:$CH:$(echo $D | lc)" ETCH_REGISTRY=$R ETCH_WRAPPER_LISTING=$MID script/deploy-stack.sh $RPC $D; then
    cast rpc --rpc-url $RPC evm_revert "$SNAP" >/dev/null; rm -f "$S"; mv "$S.frogs" "$S"; return
  fi
  [ "$(python3 -c "import json;c=json.load(open('$S'))['collections'];print(len(c), sum(x.get('kind', 'inscriptions')=='inscriptions' for x in c), sum(x.get('kind')=='nft' for x in c), [x['slug'] for x in c if x.get('kind', 'inscriptions')=='inscriptions'][0])")" = "$((L_N + 1)) 1 $L_N $B_SLUG" ] \
    && echo "ok   one stack file names $B_SLUG (inscriptions) and the $L_N Ethereum collections (no stack-file merge)" || { echo "FAIL the mixed stack file is not $B_SLUG plus the $L_N Ethereum collections"; fail=1; }
  # the stand-in canister (no IC is reached), the reinstall's argument and the launch monitor configuration, as the pre-handover setup leaves them
  MDIR=$(mktemp -d)
  printf '#!/bin/sh\nfor a; do m="$a"; done\n[ -f "%s/$m" ] || exit 1\ncat "%s/$m"\n' "$MDIR" "$MDIR" > "$MDIR/dfx"; chmod +x "$MDIR/dfx"
  python3 - "$S" "$MDIR" "$BUYER" "$CAN" "$DRY_CUSTODY" <<'PY'
import json, sys
s = json.load(open(sys.argv[1])); d, buyer, can, cust = sys.argv[2:6]
LF = json.load(open("deployments/launch/nft-pilot.json")); b = LF["bitcoin"]; st = LF["stages"][0]
cols = s["collections"]; btc = [c for c in cols if c["kind"] == "inscriptions"][0]; nft = [c for c in cols if c["kind"] == "nft"]
L = {x["collection"].lower(): x for x in LF["collections"]}
w = lambda f, v: json.dump(v, open(f"{d}/{f}", "w"))
caps = [{"collection": f"1:{c['custody']['collection'].lower()}", "max_items": st["custodyItems"][L[c['custody']['collection'].lower()]["slug"]],
         "max_value_wei": "15_000_000_000_000_000_000", "max_item_wei": "15_000_000_000_000_000_000"} for c in nft]
caps.append({"collection": "btc:" + btc["wrapper"].lower(), "max_items": b["custodyItems"], "max_value_wei": f"{b['custodyWei']:_}", "max_item_wei": f"{b['custodyWei']:_}"})
w("custody_caps", [{"collections": caps, "global_max_items": st["globalItems"], "global_max_value_wei": str(st["globalWei"])}, []])
o = LF["outflow"]
w("custody_outflow", [{k: str(v) if k == "global_value_wei" else v for k, v in o.items()}, {"done": [], "asked": [], "freeze": {"Open": None}, "acked": []}])
w("custody_fee_pool", {"entries": [{"outpoint": {"txid": "21" * 32, "vout": 0}, "value": "25_000", "source": "operator", "fails": []}], "quarantined": []})
w("floor_state", {"sinks": [{"module": btc["floorModule"].lower(), "item": {"Bitcoin": None}}], "limits": [{"sink": btc["floorModule"].lower(), "max_wei": str(b["maxPurchaseWei"])}], "wants": [], "next_want": "0"})
w("ord_providers", [["ord", "https://ordinals.com"], ["ordinalswallet", "https://turbo.ordinalswallet.com"], ["unisat", "https://open-api.unisat.io"]])
dorm = dict(LF["dormancy"], wake_desk=[s.get("wakeDesk", "")])
w("eth_nft_config", [{"chain_id": "1", "relay_max_fee_bps": 300, "deposits": True, "floor": [], "dormancy": [dorm],
   "collections": [{"collection": c["custody"]["collection"], "wrapper": c["wrapper"], "module": c["floorModule"],
                    "max_purchase_wei": str(L[c["custody"]["collection"].lower()]["maxPurchaseWei"]), "max_premium_bps": 500} for c in nft]}])
# the launch monitor configuration, filled from the stack file and stand-ins
t = open("deployments/launch/monitor-nft.json").read().replace("<the canister's EVM address>", buyer)
for c in nft: t = t.replace(f"<{c['slug']} wrapper>", c["wrapper"]).replace(f"<{c['slug']} floorModule>", c["floorModule"])
t = t.replace("<the guardian EOA (registry.guardian())>", s.get("guardian") or "0x" + "11" * 20)
t = t.replace("<the canister's custody_address()>", cust).replace("<the canister's float_address()>", "bc1p" + "q" * 58).replace("<the canister's buyer_address()>", "bc1q" + "z" * 38)
import re
t = re.sub(r"<(\w+) host>", r"\1.dry-run.invalid", t).replace("<the fueler's address (treasury.ops())>", "0x" + "fe" * 20)
m = json.loads(t); m["timelock"] = "r7inp-6aaaa-aaaaa-aaabq-cai"; m["buyer"] = can
m["expect"]["proposers"] = ["rdmx6-jaaaa-aaaaa-aaadq-cai", "qoctq-giaaa-aaaaa-aaaea-cai"]
m["expect"]["guardians"] = ["renrk-eyaaa-aaaaa-aaada-cai", "rno2w-sqaaa-aaaaa-aaacq-cai", "ryjl3-tyaaa-aaaaa-aaaba-cai"]
m["install_arg_hex"] = "00"   # replaced below by the mixed Init
json.dump(m, open(f"{d}/monitor.json", "w"))
json.dump({"wrapper": btc["wrapper"].lower(), "caps": caps, "global": [st["globalItems"], str(st["globalWei"])]}, open(f"{d}/init.json", "w"))
PY
  # the pre-handover reinstall's argument: a real Init (the monitor's Candid) naming the Bitcoin wrapper, frog_funding and every cap
  ( cd monitor && node --input-type=module - "$MDIR/init.json" ) > "$MDIR/install.hex" <<'JS'
const { Init, decodeInit } = await import(process.cwd() + "/src/candid.mjs");
const { IDL } = await import("@icp-sdk/core/candid");
const { readFileSync } = await import("node:fs");
const x = JSON.parse(readFileSync(process.argv[process.argv.length - 1], "utf8"));
const home = ["https://rpc.mainnet.chain.robinhood.com", "https://rpc.ordofi.network"], eth = ["https://rpc.mevblocker.io", "https://gateway.tenderly.co/public/mainnet"];
const chain = (id, urls) => ({ chain_id: BigInt(id), rpc_urls: urls, bucket: 0n, seaport: "0x0000000000000068f116a894984e2db1123eb395",
  seaport_codehash: "0x" + "00".repeat(32), max_fee_wei: 50000000000n, priority_fee_wei: 1000000000n, gas_cap: 400000n, fee_recipients: [],
  max_fee_bps: 300, min_order_life_s: 600n, signed_zones: [], zone_registries: [] });
const n = (v) => BigInt(String(v).replace(/_/g, ""));
const v = { chain_id: 4663n, vault: "0x00", rpc_urls: home, btc_api_urls: ["https://mempool.space/api", "https://blockstream.info/api"],
  key_name: "key_1", accept_production_key_name: true, bridge_allowlist: [], btc_network: "mainnet", operator: [], wrapper: [x.wrapper],
  evm_chains: [[chain(4663, home), chain(1, eth)]],
  custody_caps: [{ collections: x.caps.map((c) => ({ collection: c.collection, max_items: c.max_items, max_value_wei: n(c.max_value_wei), max_item_wei: n(c.max_item_wei) })),
    global_max_items: x.global[0], global_max_value_wei: n(x.global[1]) }],
  outflow_limits: [], holder_deposits: [], wrapper_from_block: [], frog_funding: [{ max_fee_bps: 400, float_first: false }], eth_nft: [] };
const hex = Buffer.from(IDL.encode([Init], [v])).toString("hex");
if (!decodeInit(hex).ok) throw new Error("the mixed Init does not decode back");
process.stdout.write(hex + "\n");
JS
  grep -q '^4449444c' "$MDIR/install.hex" && python3 -c 'import json,sys; m=json.load(open(sys.argv[1])); m["install_arg_hex"]=open(sys.argv[2]).read().strip(); json.dump(m, open(sys.argv[1],"w"))' "$MDIR/monitor.json" "$MDIR/install.hex" \
    && echo "ok   the reinstall's argument: a real Init naming the Bitcoin wrapper, frog_funding 400 / false, the Bitcoin cap beside the Ethereum ones" || { echo "FAIL the mixed install argument could not be built"; fail=1; }
  local MA=(env DFX="$MDIR/dfx" script/golive-check.sh $RPC "$F" "$S" --canister $CAN --canister-evm $BUYER --launch deployments/launch/nft-pilot.json --monitor-config "$MDIR/monitor.json" --install-arg "$MDIR/install.hex")
  must "golive-check.sh --canister --launch --monitor-config --install-arg on the MIXED stack: the Bitcoin side and the eleven Ethereum collections at once" "${MA[@]}"
  echo "     $(echo "$LAST" | grep -c '^ok ') checks ok"; echo "$LAST" | grep -E "^(FAIL|note)" | sed 's/^/     /'
  local want k; for k in "$B_SLUG listing $MID membership root = the launch file's bitcoin.membersRoot" "$B_SLUG canister names the cap btc:" \
    "$B_SLUG canister floor_state(): a Bitcoin floor sink" "$B_SLUG canister purchase limit of the Bitcoin module = the launch file's" \
    "$B_SLUG canister ord_providers() names UniSat" "custody fee pool: one entry of 25000 sats" "the install argument's wrapper is the stack's Bitcoin wrapper" \
    "the install argument's frog_funding is the launch file's" "the monitor configuration's Bitcoin canary watches custody" \
    "the launch file's custody (custodyItems) at its outflow limits: every item can leave" "the canister's custody_caps() at its custody_outflow() limits: every item can leave" \
    "counted in each step, btc:0x<wrapper>" "the launch file's collections are exactly the stack's"; do
    grep -q "^ok   .*$k" <<< "$LAST" && echo "ok   the mixed read: $k" || { echo "FAIL the mixed read has no ok line for: $k"; fail=1; }
  done
  [ "$(echo "$LAST" | grep -c "is in the operator's launch list (D1)")" = "$L_N" ] && echo "ok   and the $L_N Ethereum collections read as in the NFT-only stack (D1)" || { echo "FAIL the Ethereum collections of the mixed stack were not read"; fail=1; }
  # each Bitcoin pre-handover step left undone, alone
  cp "$MDIR/floor_state" "$MDIR/floor_state.ok"; echo '{"sinks": [], "limits": [], "wants": [], "next_want": "0"}' > "$MDIR/floor_state"
  expect "the mixed stack with no Bitcoin floor sink (set_floor_sink not made before the handover)" "has no Bitcoin floor sink for the stack's module" "${MA[@]}"
  cp "$MDIR/floor_state.ok" "$MDIR/floor_state"; cp "$MDIR/ord_providers" "$MDIR/ord.ok"; echo '[["ord", "https://ordinals.com"], ["ordinalswallet", "https://turbo.ordinalswallet.com"]]' > "$MDIR/ord_providers"
  expect "the mixed stack with no UniSat key (set_unisat_key not made, or an upgrade since)" "does not name UniSat" "${MA[@]}"
  cp "$MDIR/ord.ok" "$MDIR/ord_providers"; cp "$MDIR/custody_caps" "$MDIR/caps.ok"
  python3 -c 'import json,sys; c=json.load(open(sys.argv[1])); c[0]["collections"]=[x for x in c[0]["collections"] if not x["collection"].startswith("btc:")]; json.dump(c, open(sys.argv[1],"w"))' "$MDIR/custody_caps"
  expect "the mixed stack on a canister whose install names no wrapper" "the canister names no cap for btc:" "${MA[@]}"
  cp "$MDIR/caps.ok" "$MDIR/custody_caps"
  python3 -c 'import json,sys; L=json.load(open("deployments/launch/nft-pilot.json")); L["bitcoin"]["membersRoot"]="0x"+"11"*32; json.dump(L, open(sys.argv[1],"w"))' "$MDIR/launch.root.json"
  expect "the mixed stack against a launch file with another membership root" "membership root = the launch file's bitcoin.membersRoot" env DFX="$MDIR/dfx" script/golive-check.sh $RPC "$F" "$S" --launch "$MDIR/launch.root.json"
  rm -rf "$MDIR"
  cast rpc --rpc-url $RPC evm_revert "$SNAP" >/dev/null; rm -f "$S"; mv "$S.frogs" "$S"
}

rehearse_stack() { # rehearse_stack <chainid>
  local CH="$1"; local F="deployments/$CH.json"; local S="deployments/stack-$CH.json"
  rm -f "$F" "$S" deployments/wrapper-"$CH"-*.json deployments/rehearsal-"$CH".json deployments/rehearsal-pin-"$CH".json deployments/rehearsal-open-"$CH".json
  step "STACK: 1-3. the system (registry, beacon, launcher, distributor), wired"
  must "script/deploy.sh" roles env ETCH_DEPLOY_CONFIRM="deploy-etch:$CH:$(echo $D | lc)" script/deploy.sh $RPC $D
  local R; R=$(python3 -c "import json;print(json.load(open('$F'))['registry'])")
  step "STACK: 4. an autonomous-buyer listing, naming the canister's code-less address"
  must "script/list-auto.sh" listenv env ETCH_REGISTRY=$R ETCH_BUYER_CONFIRM="buyer-is:$CH:$(echo $BUYER | lc)" script/list-auto.sh $RPC
  local ID=$(( $(cast call --rpc-url $RPC $R 'count()(uint256)') - 1 ))
  step "STACK: the whole stack over listing $ID (docs/v6/deploy-stack.md §2), at stage 0"
  must "script/deploy-stack.sh" stackenv env ETCH_STACK_CONFIRM="deploy-etch-stack:$CH:$(echo $D | lc)" ETCH_REGISTRY=$R ETCH_WRAPPER_LISTING=$ID script/deploy-stack.sh $RPC $D
  echo "$LAST" | grep -E "^(treasury|directory|wrapClaim|wrapper|wrote)" | sed 's/^/     /'
  [ -f "$S" ] || { echo "FAIL no stack file was written; the rest of the rehearsal is skipped"; fail=1; return; }
  # The public RPC serves historical state for about fifteen minutes (docs/staging.md), and the drand wait below is longer than
  # that. Anvil reads an account from upstream the first time it is touched, so the BN254 precompiles the beacon's verify uses are
  # touched NOW, with a fixture round, while the fork block is still served; afterwards they come from anvil's cache.
  local BEACON SIG1000 RAND1000
  BEACON=$(python3 -c "import json;print(json.load(open('$F'))['beacon'])")
  SIG1000=$(python3 -c "import json;b=[x for x in json.load(open('test/fixtures/drand-evmnet.json'))['beacons'] if x['round']==1000][0];print(b['signature'])")
  RAND1000=$(python3 -c "import json;b=[x for x in json.load(open('test/fixtures/drand-evmnet.json'))['beacons'] if x['round']==1000][0];print(b['randomness'])")
  if [ "$(cast call --rpc-url $RPC $BEACON 'verify(uint64,bytes)(bytes32)' 1000 $SIG1000 2>/dev/null)" = "$RAND1000" ]; then echo "ok   the beacon verifies fixture round 1000 on the fork (precompiles warmed)"; else echo "FAIL the beacon did not verify fixture round 1000 on the fork"; fail=1; fi
  step "STACK: read every wiring back from the chain"
  must "script/golive-check.sh" script/golive-check.sh $RPC "$F" "$S"
  echo "$LAST" | grep -E "^(FAIL|note)" | sed 's/^/     /'; echo "     $(echo "$LAST" | grep -c '^ok ') checks ok"
  grep -q "listing $(python3 -c "import json;print(json.load(open('$S'))['collections'][0]['listingId'])") btcVault is the wrapper's custody address" <<< "$LAST" \
    && echo "ok   stack mode compares listing.btcVault with wrapper.custodyAddress (K9)" || { echo "FAIL stack mode did not compare listing.btcVault with the wrapper's custody"; fail=1; }
  grep -q "^note the canister was not read" <<< "$LAST" && echo "ok   with no --canister it says the canister was not read" || { echo "FAIL no note that the canister was not read"; fail=1; }
  # K9: the canister's custody_caps() beside the stage, through a stand-in dfx (DFX) that answers what a buyer would; no IC is reached
  step "STACK: the canister's custody_caps() held to the stage (a stand-in dfx)"
  local CAPSDIR WRAP ETH10 CAN=rrkah-fqaaa-aaaaa-aaaaq-cai
  CAPSDIR=$(mktemp -d); WRAP=$(python3 -c "import json;print(json.load(open('$S'))['collections'][0]['wrapper'].lower())"); ETH10=10_000_000_000_000_000_000
  # integration R3: the stand-in answers custody_fee_pool() from its own file (golive-check requires one entry >= 100,000 sats)
  printf '#!/bin/sh\ncase "$*" in *custody_fee_pool*) f=pool;; *) f=answer;; esac\n[ -f "%s/$f" ] || exit 1\ncat "%s/$f"\n' "$CAPSDIR" "$CAPSDIR" > "$CAPSDIR/dfx"; chmod +x "$CAPSDIR/dfx"
  answer() { echo "[{\"collections\":[$1],\"global_max_items\":6,\"global_max_value_wei\":\"$ETH10\"},[]]" > "$CAPSDIR/answer"; }
  pool() { echo "{\"entries\":[{\"outpoint\":{\"txid\":\"$(printf '21%.0s' $(seq 32))\",\"vout\":0},\"value\":\"$1\",\"source\":\"operator\",\"fails\":[]}],\"quarantined\":[]}" > "$CAPSDIR/pool"; }
  pool 150_000
  answer "{\"collection\":\"btc:$WRAP\",\"max_items\":3,\"max_value_wei\":\"$ETH10\",\"max_item_wei\":\"$ETH10\"}"
  must "golive-check.sh --canister, the canister at stage 0" env DFX="$CAPSDIR/dfx" script/golive-check.sh $RPC "$F" "$S" --canister $CAN
  echo "$LAST" | grep -E "custody_caps\(\)|canister cap|custody_fee_pool|custody fee pool" | sed 's/^/     /'
  pool 19_999
  expect "a custody fee pool with no entry of the 20,000-sat minimum (integration R3; the default since ops-config)" "FAIL custody fee pool: no usable entry of at least 20000 sats" env DFX="$CAPSDIR/dfx" script/golive-check.sh $RPC "$F" "$S" --canister $CAN
  pool 150_000
  answer "{\"collection\":\"btc:$WRAP\",\"max_items\":12,\"max_value_wei\":\"40_000_000_000_000_000_000\",\"max_item_wei\":\"$ETH10\"}"
  expect "a canister whose cap is stage 1's while the stack says stage 0" "canister cap: items in custody = the stage's: expected 3" env DFX="$CAPSDIR/dfx" script/golive-check.sh $RPC "$F" "$S" --canister $CAN
  answer ""
  must "golive-check.sh --canister, a canister that does not name the wrapper yet (a note, before the reinstall)" env DFX="$CAPSDIR/dfx" script/golive-check.sh $RPC "$F" "$S" --canister $CAN
  grep -q "names no cap for btc:$WRAP" <<< "$LAST" && echo "ok   and it says so" || { echo "FAIL no note for a wrapper the canister does not name"; fail=1; }
  rm -f "$CAPSDIR/answer"
  expect "a canister that cannot be read, when one was asked for" "could not be read" env DFX="$CAPSDIR/dfx" script/golive-check.sh $RPC "$F" "$S" --canister $CAN
  expect "a --canister that is not a canister id" "needs a canister id" script/golive-check.sh $RPC "$F" "$S" --canister nope
  rm -rf "$CAPSDIR"
  # STACK_PHASES (default all three): a subset, e.g. STACK_PHASES=mixed, keeps a run inside a short-history fork RPC's window
  case " ${STACK_PHASES:-phase1 nft mixed} " in *" phase1 "*) golive_phase1 "$CH" "$F" "$S" "$R" "$ID";; esac
  case " ${STACK_PHASES:-phase1 nft mixed} " in *" nft "*) rehearse_nft_stack "$CH" "$F" "$S" "$R" "$ID";; esac
  case " ${STACK_PHASES:-phase1 nft mixed} " in *" mixed "*) rehearse_mixed_stack "$CH" "$F" "$S" "$R";; esac
  step "STACK: the refusals that guard this step"
  expect "a stack confirm typed for another chain" "ETCH_STACK_CONFIRM must be exactly" stackenv env ETCH_STACK_CONFIRM="deploy-etch-stack:4663:$(echo $D | lc)" ETCH_REGISTRY=$R ETCH_WRAPPER_LISTING=$ID script/deploy-stack.sh $RPC $D
  expect "a custodian that is a contract (the registry itself)" "CustodianHasCode" stackenv env ETCH_WRAPPER_CUSTODIAN=$R ETCH_STACK_CONFIRM="deploy-etch-stack:$CH:$(echo $D | lc)" ETCH_REGISTRY=$R ETCH_WRAPPER_LISTING=$ID script/deploy-stack.sh $RPC $D
  expect "a stack over the demo RUNE listing" "ListingIsRune" stackenv env ETCH_STACK_CONFIRM="deploy-etch-stack:$CH:$(echo $D | lc)" ETCH_REGISTRY=$R ETCH_WRAPPER_LISTING=1 script/deploy-stack.sh $RPC $D
  expect "a stage past the table" "StageTooHigh" stackenv env ETCH_STAGE=4 ETCH_STACK_CONFIRM="deploy-etch-stack:$CH:$(echo $D | lc)" ETCH_REGISTRY=$R ETCH_WRAPPER_LISTING=$ID script/deploy-stack.sh $RPC $D
  expect "a key in the environment" "does not take a key" env PRIVATE_KEY=anything ETCH_STACK_CONFIRM="deploy-etch-stack:$CH:$(echo $D | lc)" script/deploy-stack.sh $RPC $D
  step "STACK: launch → graduation through the module → a mint at the module → a sweep at the router → a raffle draw announced → an advance and its return"
  for a in $CREATOR $RBUYER $TRADER; do cast rpc --rpc-url $RPC anvil_setBalance $a 0x1B1AE4D6E2EF500000 >/dev/null; done   # 500 ETH each, play money
  must "phase launch" phase launch || { echo "     the rest of the rehearsal is skipped"; return; }
  echo "$LAST" | grep -E "^  (coin|launch fee|ETH-backed|minted|swept|draw)" | sed 's/^/   /'
  # docs/v6/relay-conversion.md §2: the custodian advances one unit's item budget before its item, then returns it
  cast rpc --rpc-url $RPC anvil_setBalance $BUYER 0x8AC7230489E80000 >/dev/null   # 10 ETH of gas, play money
  must "phase advance (the graduation's own ETH out before its frog, then back)" phase advance || { echo "     the rest of the rehearsal is skipped"; return; }
  echo "$LAST" | grep -E "^  (advanced|in transit|returned)" | sed 's/^/   /'
  # docs/audit/2026-09-26/fixes-B-C-keeper-draws.md: the announcement pinned a seed round, which picks the snapshot time; the draw
  # is opened with that round's real signature and a root whose sentinel leaf commits the count and the time
  # read FROM THE CHAIN, like the pinned round below: the file is written during forge's simulation, and the broadcast
  # announcement is mined seconds later, often in a later 3-second round
  local SEEDROUND SIMSEED DRAW0 RAFFLE0
  SIMSEED=$(python3 -c "import json;print(json.load(open('deployments/rehearsal-$CH.json'))['seedRound'])")
  DRAW0=$(python3 -c "import json;print(json.load(open('deployments/rehearsal-$CH.json'))['drawId'])")
  RAFFLE0=$(python3 -c "import json;print(json.load(open('$S'))['collections'][0]['raffleVault'])")
  SEEDROUND=$(cast call --rpc-url $RPC $RAFFLE0 'draw(uint256)((uint8,address,uint64,uint64,uint64,uint64,uint64,uint64,uint64,bytes32,uint256,uint256,address,string,uint64,uint64,uint64,uint64))' $DRAW0 | awk -F', ' '{print $15}' | awk '{print $1}')
  echo "$SEEDROUND" | grep -Eq '^[0-9]+$' || { echo "FAIL could not read the seed round back from the raffle vault"; fail=1; return; }
  echo "     seed round $SEEDROUND (the chain's; the simulation had $SIMSEED)"
  if [ "${DRYRUN_NO_DRAND:-}" = "1" ]; then
    echo "note DRYRUN_NO_DRAND=1: the draw is rehearsed to the announcement only (seed round $SEEDROUND); open, pin, resolve and claim were NOT run"; return
  fi
  step "STACK: the seed round $SEEDROUND exists, then the draw is opened with its real signature (the snapshot time is fixed)"
  local SEEDSIG; SEEDSIG=$(drandsig "$SEEDROUND")
  if [ -z "$SEEDSIG" ]; then echo "FAIL drand round $SEEDROUND could not be read from api.drand.sh"; fail=1; return; fi
  must "phase open (the real evmnet signature of seed round $SEEDROUND)" phase open DRAND_SEED_SIGNATURE=0x$SEEDSIG || { echo "     the rest of the rehearsal is skipped"; return; }
  echo "$LAST" | grep -E "^  draw" | sed 's/^/   /'
  step "STACK: the challenge window passes on both clocks (15 minutes, 80 blocks), then a drand round is pinned"
  cast rpc --rpc-url $RPC evm_increaseTime 900 >/dev/null; cast rpc --rpc-url $RPC anvil_mine 0x50 >/dev/null
  must "phase draw" phase draw || { echo "     the rest of the rehearsal is skipped"; return; }
  # The round is read back FROM THE CHAIN, not from the pin file: forge writes that file during its simulation, and the broadcast
  # transaction is mined a few seconds later — on a 3-second beacon that is often a different round, and the signature fetched
  # for the simulated round then fails the vault's verify (InvalidSignature) although both are real. Seen twice on 2026-09-23.
  local ROUND SIMROUND RAFFLE DRAWID
  SIMROUND=$(python3 -c "import json;print(json.load(open('deployments/rehearsal-pin-$CH.json'))['round'])")
  DRAWID=$(python3 -c "import json;print(json.load(open('deployments/rehearsal-pin-$CH.json'))['drawId'])")
  RAFFLE=$(python3 -c "import json;print(json.load(open('$S'))['collections'][0]['raffleVault'])")
  ROUND=$(cast call --rpc-url $RPC $RAFFLE 'draw(uint256)((uint8,address,uint64,uint64,uint64,uint64,uint64,uint64,uint64,bytes32,uint256,uint256,address,string,uint64,uint64,uint64,uint64))' $DRAWID | awk -F', ' '{print $5}' | awk '{print $1}')   # cast annotates big numbers: "20891712 [2.089e7]"
  echo "$ROUND" | grep -Eq '^[0-9]+$' || { echo "FAIL could not read the pinned round back from the raffle vault"; fail=1; return; }
  echo "     pinned to drand evmnet round $ROUND (the chain's; the simulation had pinned $SIMROUND)"
  step "STACK: waiting for round $ROUND to exist (the fork's clock ran ahead of the wall clock), then resolve and claim"
  local SIG; SIG=$(drandsig "$ROUND")
  if [ -z "$SIG" ]; then echo "FAIL drand round $ROUND could not be read from api.drand.sh"; fail=1; return; fi
  must "phase resolve (the real evmnet signature of round $ROUND)" phase resolve DRAND_SIGNATURE=0x$SIG
  echo "$LAST" | grep -E "^  (draw|one whole)" | sed 's/^/   /'
}

MODE="${1:-all}"
if [ "$MODE" = bare ] || [ "$MODE" = all ]; then
  step "BARE: a local node, chain 31337, with the Pons stand-in planted at $PONS"
  start 31337
  cast rpc --rpc-url $RPC anvil_setCode $PONS "$(forge inspect script/local/LocalPons.sol:LocalPonsFactory deployedBytecode)" >/dev/null && echo "ok   planted"
  rehearse 31337 BARE
fi
if [ "$MODE" = fork ] || [ "$MODE" = all ]; then
  step "FORK: Robinhood Chain mainnet ($MAINNET_RPC), forked locally under our staging chain id $STAGING"
  start "$STAGING" fork
  [ "$(cast code --rpc-url $RPC $PONS | cut -c1-12)" != "0x" ] && echo "ok   real Pons factory code is here: $(cast code --rpc-url $RPC $PONS | wc -c | tr -d ' ') hex chars"
  echo "     forked at block $(cast block-number --rpc-url $RPC)"
  rehearse "$STAGING" FORK
fi
if [ "$MODE" = refusals ] || [ "$MODE" = all ]; then
  step "4663: a LOCAL node that reports Robinhood Chain's id. Nothing may be sent without a hardware wallet or the deployer's keystore, and listing sends nothing at all"
  start 4663
  cast rpc --rpc-url $RPC anvil_setCode $PONS "$(forge inspect script/local/LocalPons.sol:LocalPonsFactory deployedBytecode)" >/dev/null
  expect "deploy.sh on 4663 without a signer" "hardware wallet" roles env ETCH_DEPLOY_CONFIRM="deploy-etch:4663:$(echo $D | lc)" script/deploy.sh $RPC $D
  expect "deploy-wrapclaim.sh on 4663 without a signer" "hardware wallet" env ETCH_WRAPPER_CONFIRM="deploy-etch-wrapper:4663:$(echo $D | lc)" script/deploy-wrapclaim.sh $RPC $D
  expect "deploy-nft.sh on 4663 without a hardware wallet" "hardware wallet" env ETCH_NFT_CONFIRM="deploy-etch-nft:4663:$(echo $D | lc)" ETCH_REGISTRY=$D script/deploy-nft.sh $RPC $D
  # docs/v6/phase1-scope.md §8.1 (K8): a Foundry keystore, in a throwaway HOME whose keystore files hold an address and no key.
  # Accepted only when its address is the deployer's, and then only up to the typed chain id: a wrong id is typed, so nothing is sent.
  KH=$(mktemp -d); mkdir -p "$KH/.foundry/keystores" "$KH/elsewhere"
  echo "{\"address\":\"$(echo ${D#0x} | lc)\",\"crypto\":{},\"version\":3}" > "$KH/.foundry/keystores/rook-deployer"
  echo "{\"address\":\"$(echo ${O#0x} | lc)\",\"crypto\":{},\"version\":3}" > "$KH/.foundry/keystores/someone-else"
  cp "$KH/.foundry/keystores/rook-deployer" "$KH/elsewhere/rook-deployer"
  typed() { local t="$1"; shift; printf '%s\n' "$t" | "$@"; }
  NONCE_D=$(cast nonce --rpc-url $RPC $D)
  expect "deploy.sh on 4663 with a keystore for another address" "is for $(echo $O | lc), not the deployer" roles env HOME="$KH" ETCH_DEPLOY_CONFIRM="deploy-etch:4663:$(echo $D | lc)" script/deploy.sh $RPC $D --account someone-else
  expect "deploy.sh on 4663 with a keystore that does not exist" "no keystore named 'nobody'" roles env HOME="$KH" ETCH_DEPLOY_CONFIRM="deploy-etch:4663:$(echo $D | lc)" script/deploy.sh $RPC $D --account nobody
  expect "deploy.sh on 4663 with a keystore name that climbs out" "keystore name" roles env HOME="$KH" ETCH_DEPLOY_CONFIRM="deploy-etch:4663:$(echo $D | lc)" script/deploy.sh $RPC $D --account ../rook-deployer
  expect "deploy.sh on 4663 with FOUNDRY_KEYSTORES_DIR elsewhere" "FOUNDRY_KEYSTORES_DIR is" roles env HOME="$KH" FOUNDRY_KEYSTORES_DIR="$KH/elsewhere" ETCH_DEPLOY_CONFIRM="deploy-etch:4663:$(echo $D | lc)" script/deploy.sh $RPC $D --account rook-deployer
  expect "deploy.sh on 4663 with ETH_KEYSTORE_ACCOUNT in the environment" "does not take a key" roles env HOME="$KH" ETH_KEYSTORE_ACCOUNT=rook-deployer ETCH_DEPLOY_CONFIRM="deploy-etch:4663:$(echo $D | lc)" script/deploy.sh $RPC $D
  expect "deploy.sh on 4663 with ETH_PASSWORD in the environment" "does not take a key" roles env HOME="$KH" ETH_PASSWORD=/dev/null ETCH_DEPLOY_CONFIRM="deploy-etch:4663:$(echo $D | lc)" script/deploy.sh $RPC $D --account rook-deployer
  expect "deploy.sh on 4663 with two signer flags" "one signer flag" roles env HOME="$KH" ETCH_DEPLOY_CONFIRM="deploy-etch:4663:$(echo $D | lc)" script/deploy.sh $RPC $D --ledger --account rook-deployer
  expect "deploy.sh on 4663 with the deployer's keystore: accepted, then stops at the typed chain id" "not confirmed" typed 1 roles env HOME="$KH" ETCH_DEPLOY_CONFIRM="deploy-etch:4663:$(echo $D | lc)" script/deploy.sh $RPC $D --account rook-deployer
  echo "$(typed 1 roles env HOME="$KH" ETCH_DEPLOY_CONFIRM="deploy-etch:4663:$(echo $D | lc)" script/deploy.sh $RPC $D --account rook-deployer 2>&1)" | grep -q "signed by the keystore 'rook-deployer'" \
    && echo "ok   and the prompt names the keystore it will sign with" || { echo "FAIL deploy.sh's prompt does not name the keystore"; fail=1; }
  expect "deploy-wrapclaim.sh on 4663 with a keystore for another address" "is for $(echo $O | lc)" env HOME="$KH" ETCH_WRAPPER_CONFIRM="deploy-etch-wrapper:4663:$(echo $D | lc)" script/deploy-wrapclaim.sh $RPC $D --account someone-else
  expect "deploy-wrapclaim.sh on 4663 with the deployer's keystore: stops at the typed chain id" "not confirmed" typed 1 env HOME="$KH" ETCH_WRAPPER_CONFIRM="deploy-etch-wrapper:4663:$(echo $D | lc)" script/deploy-wrapclaim.sh $RPC $D --account rook-deployer
  expect "deploy-nft.sh on 4663 with a keystore (the NFT side is OFF in Phase 1: hardware wallet only)" "unknown argument --account" env HOME="$KH" ETCH_NFT_CONFIRM="deploy-etch-nft:4663:$(echo $D | lc)" ETCH_REGISTRY=$D script/deploy-nft.sh $RPC $D --account rook-deployer
  [ "$(cast nonce --rpc-url $RPC $D)" = "$NONCE_D" ] && [ ! -e deployments/4663.json ] \
    && echo "ok   and none of them sent anything: the deployer's nonce is unchanged and no deployments/4663.json was written" || { echo "FAIL a keystore run of deploy.sh or deploy-wrapclaim.sh sent something on 4663"; fail=1; }
  # a registry put here by hand, owned by O, so list-auto has something real to point at
  BYTECODE=$(forge inspect src/ListingRegistry.sol:ListingRegistry bytecode)$(cast abi-encode 'c(address,address)' $O $T | cut -c3-)
  R=$(cast send --rpc-url $RPC --unlocked --from $D --create "$BYTECODE" --json | python3 -c "import sys,json; print(json.load(sys.stdin)['contractAddress'])")
  NONCE_D=$(cast nonce --rpc-url $RPC $D)   # the registry above was created from D, by hand; the stack's keystore runs below must send nothing more
  NONCE0=$(cast nonce --rpc-url $RPC $O)
  must "list-auto.sh on 4663 runs" listenv env ETCH_REGISTRY=$R ETCH_BUYER_CONFIRM="buyer-is:4663:$(echo $BUYER | lc)" script/list-auto.sh $RPC
  grep -q "NOTHING WAS SENT" <<< "$LAST" && echo "ok   it says NOTHING WAS SENT and prints the multisig's calldata" || { echo "FAIL list-auto on 4663 did not say NOTHING WAS SENT"; fail=1; }
  [ "$(cast nonce --rpc-url $RPC $O)" = "$NONCE0" ] && [ "$(cast call --rpc-url $RPC $R 'count()(uint256)')" = 0 ] \
    && echo "ok   and it did not: the owner's nonce and the registry's listing count are unchanged" || { echo "FAIL list-auto sent something on 4663"; fail=1; }
  expect "deploy-stack.sh on 4663 without a signer" "hardware wallet" stackenv env ETCH_STACK_CONFIRM="deploy-etch-stack:4663:$(echo $D | lc)" ETCH_REGISTRY=$R ETCH_WRAPPER_LISTING=0 script/deploy-stack.sh $RPC $D
  expect "deploy-stack.sh on 4663 with a keystore for another address" "is for $(echo $O | lc)" stackenv env HOME="$KH" ETCH_STACK_CONFIRM="deploy-etch-stack:4663:$(echo $D | lc)" ETCH_REGISTRY=$R ETCH_WRAPPER_LISTING=0 script/deploy-stack.sh $RPC $D --account someone-else
  expect "deploy-stack.sh on 4663 with the deployer's keystore: stops at the typed chain id" "not confirmed" typed 1 stackenv env HOME="$KH" ETCH_STACK_CONFIRM="deploy-etch-stack:4663:$(echo $D | lc)" ETCH_REGISTRY=$R ETCH_WRAPPER_LISTING=0 script/deploy-stack.sh $RPC $D --account rook-deployer
  [ "$(cast nonce --rpc-url $RPC $D)" = "$NONCE_D" ] && [ ! -e deployments/4663.json ] && [ ! -e deployments/stack-4663.json ] \
    && echo "ok   and neither sent anything: the deployer's nonce is unchanged and no stack file was written" || { echo "FAIL a keystore run of deploy-stack.sh sent something on 4663"; fail=1; }
  rm -rf "$KH"
  expect "deploy-stack.sh on 4663 at a stage above 0" "stage 0" stackenv env ETCH_STAGE=1 ETCH_STACK_CONFIRM="deploy-etch-stack:4663:$(echo $D | lc)" ETCH_REGISTRY=$R ETCH_WRAPPER_LISTING=0 script/deploy-stack.sh $RPC $D
  expect "the rehearsal itself on a node reporting 4663" "NotOurs\|script failed\|revert" env REHEARSAL_PHASE=launch forge script script/StackRehearsal.s.sol:StackRehearsal --rpc-url $RPC --sender $D --unlocked --code-size-limit 98304
  start 46630
  expect "Robinhood's public testnet (46630)" "public testnet" env ETCH_WRAPPER_CONFIRM="deploy-etch-wrapper:46630:$(echo $D | lc)" script/deploy-wrapclaim.sh $RPC $D
  expect "list-auto on 46630" "public testnet" listenv env ETCH_REGISTRY=$D script/list-auto.sh $RPC
  expect "deploy-stack.sh on 46630" "public testnet" stackenv env ETCH_STACK_CONFIRM="deploy-etch-stack:46630:$(echo $D | lc)" ETCH_REGISTRY=$D ETCH_WRAPPER_LISTING=0 script/deploy-stack.sh $RPC $D
fi
if [ "$MODE" = stack ]; then
  PORT="${STACK_PORT:-8651}"; RPC="http://127.0.0.1:$PORT"
  # docs/v6/deploy-stack.md §7: the fork is taken from an ARCHIVE read-only RPC, so no upstream read ever falls outside a
  # historical-state window (the public RPC keeps about fifteen minutes; the first runs stopped there). The default was
  # robinhood.drpc.org until 2026-09-24, when it stopped answering any state read at a block number ("Unknown state. First
  # available state is 1"). rpc.ordofi.network is an archive node behind a load balancer: the fork is taken STACK_FORK_BACK
  # blocks (default 2,000, about 3.5 minutes at 4663's ~0.1 s blocks) behind its head so that every backend has the block.
  # The fork's clock then runs only ~3.5 minutes behind the wall clock, so the drand round a draw pins (the 15-minute challenge
  # window + 20 rounds ahead of the fork's clock) is ~13 minutes in the future and the script waits for it (the countdown below);
  # STACK_FORK_BACK=13000 (~22 minutes) removes the wait if the upstream serves that deep.
  MAINNET_RPC="${STACK_FORK_RPC:-https://rpc.ordofi.network}"
  HEAD=$(cast block-number --rpc-url "$MAINNET_RPC"); FORK_AT=$(( HEAD - ${STACK_FORK_BACK:-2000} ))
  EXTRA_ANVIL="--auto-impersonate --fork-block-number $FORK_AT"
  step "STACK: Robinhood Chain mainnet ($MAINNET_RPC) at block $FORK_AT (head $HEAD), forked locally under our staging chain id $STAGING on port $PORT"
  start "$STAGING" fork
  echo "     the fork's clock is $(( $(date +%s) - $(cast block --rpc-url $RPC latest -f timestamp) )) s behind the wall clock"
  echo "     forked at block $(cast block-number --rpc-url $RPC); the PoolManager has $(cast code --rpc-url $RPC 0x8366a39CC670B4001A1121B8F6A443A643e40951 | wc -c | tr -d ' ') hex chars of code"
  rehearse_stack "$STAGING"
fi
stop
printf '\n%s\n' "$([ $fail = 0 ] && echo 'DRY RUN: ALL PASS' || echo 'DRY RUN: FAILURES ABOVE')"
exit $fail
