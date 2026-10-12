#!/usr/bin/env bash
# docs/deploy/contracts-mainnet.md, step 5 (and again after step 6). Reads back, from the chain itself, every immutable, owner and role
# the deployment file claims, and every vault the registry has created. Read-only: `cast call` and nothing else. Exit 1 on any mismatch.
#   usage:  script/golive-check.sh [<rpc-url>] [deployments/<chainid>.json] [deployments/wrapper-*.json …] [deployments/nft-<chainid>.json] [deployments/stack-<chainid>.json]
#                                  [--canister <buyer canister id> [--network ic|local] [--min-fee-pool-sats <sats>]]
# With --canister (docs/v6/phase1-scope.md §8.2, K9) it also reads the buyer's public custody_caps() query with dfx, as the anonymous
# identity (DFX names the binary; default as scripts/timelock-watch.mjs), and holds each stack collection's cap to the stack's stage.
# Integration R3: it also reads custody_fee_pool() and FAILs unless one usable entry holds at least --min-fee-pool-sats (default
# 20,000 sats, the operator's choice; the conflict-out's fee CEILING stays 100,000 sats, and an entry under it is a note), and reads
# the stack file's `payee`, per-collection `itemUnits` and `listingPaused` instead of assuming them.
# --phase1 (docs/audit/2026-09-26/fixes-W2-monitor.md §3; audit C-08, B-05): the go/no-go read, after go-live step 7. Every owner
# step still to be made is a FAIL, not a note; the owner must be a 2-of-2 Safe; no role address may carry code (EIP-7702); and a
# guardian must be set — or, with --no-guardian (the operator's written choice), its absence is a WARN. On 4663 --phase1 is implied.
# Round 3 (docs/audit/2026-09-26/review2/fixes-R3-services.md §7; R2 G-N1..G-N3, C-08): under --phase1 the stack file is REQUIRED,
# and --canister too unless --before-reinstall says the canister is read after its reinstall; the owner must be a REAL Safe (a
# canonical Safe proxy on a canonical singleton, no module, no guard, 2 of 2 code-less owners that hold no role); every read that
# comes back empty is a FAIL (never a skip or a pass); and each collection's payees, ETH caps, rates and listing pause are read.
#   --safe-owners 0xA,0xB   the ceremony's two device addresses: the Safe's owners must be exactly these (REQUIRED under --phase1,
#                           R3-check L-H; the Safe's fallback handler and its SafeSetup event are read too, from SAFE_FROM_BLOCK
#                           (default 0) on). R4-check L-S1: the SafeSetup's transaction must also hold the ProxyCreation of this
#                           Safe by a canonical SafeProxyFactory (so no creation code but the canonical proxy's ever ran on it), and
#                           the SafeSetup's owners and threshold must be the chain's getOwners()/getThreshold() and --safe-owners
#   --launch <file>         audit 2026-09-27-nft round 2 (D2-L-5, A2-I-2): deployments/launch/nft-pilot.json. Each Ethereum collection's
#                           factory targets and module conversion / reserve are held against the LAUNCH file (not only the stack
#                           file the same deploy wrote), the file must name exactly the stack's collections, and with --canister each
#                           collection's max_purchase_wei is the file's maxPurchaseWei; round 3 (A3-L-2): each wrapper's name() and
#                           symbol() and the stack file's slug are the file's, by collection address. REQUIRED under --phase1 for an NFT stack
#   --monitor-config <file> round 2: the launch monitor configuration (from deployments/launch/monitor-nft.json, filled in): every
#                           Ethereum collection with its wrapper, module and custodian, both D8 references, an armed guardian and
#                           evm.guardian.pause_on_no_good_post_h; round 3 (A3-L-3): no <placeholder> anywhere, two Ethereum providers by
#                           host, evm.chain_id 4663, evm.guardian.address = registry.guardian(). REQUIRED under --phase1 for an NFT stack
#   --install-arg <hex|file> audit 2026-09-27-nft round 3 (bayc10-decisions.md §5, §7): the buyer's reinstall argument (Candid bytes as hex,
#                           4449444c…, or a file holding them); without it, the --monitor-config file's install_arg_hex when that is real
#                           hex. Every provider URL in it must be KEYLESS: the argument is published with every timelock proposal and the
#                           monitor's argument diff prints provider URLs. REQUIRED (here or in the monitor file) under --phase1 for an NFT stack
#   --stage <n>             review round 5 (the operator's option (c), fixes-round5.md §6): which step of the launch file's `stages` the
#                           canister is at (default: the file's "stage"): its custody caps and outflow limits are held to that step's
#                           numbers, field by field (A5-I-1), and every step of the plan to the exit rule and to the step before it
#   --proposal <hex|file>   review round 5: a PENDING upgrade's argument (Candid hex, as the timelock's proposal holds it) read instead of
#                           a stack: decoded with the monitor's IDL, its custody caps and outflow limits must be exactly --stage's step of
#                           the launch file (--launch, REQUIRED), the step before it the running pair (--canister's custody_caps() and
#                           custody_outflow(), or --baseline), the change between them only the step's own (step 2: outflow only,
#                           global_items; step 3: the custody caps only), and its pair must fit the exit rule. With --baseline <hex|file>
#                           (the argument the running install was made with) every other field of the Init must be unchanged too.
#                           Needs no RPC: `script/golive-check.sh --proposal <hex> --launch <file> --stage 2 [--canister <id> | --baseline <hex>]`
#                           Review round 6: a step is exactly the step (A6-L-1: its global caps the sums of its caps, no other collection's
#                           cap moving, nothing raised past ×4); under --phase1, --baseline (or --monitor-config's install_arg_hex) is
#                           REQUIRED, so every other Init field is compared (A6-I-2)
#   --probe-depth           review round 6 (B6-M-1): measure each of the install argument's Ethereum (chain 1) providers now, read-only
#                           (a 500-block eth_getLogs and a receipt 60,000 blocks back must be served): a failure FAILs even a listed
#                           provider, a pass admits one not on the measured list. Without it, under --phase1, a chain-1 provider must be
#                           on the list of providers measured to serve the 7-day sale window (DEEP, below)
#   GOLIVE_REHEARSAL=1      REHEARSAL ONLY (scripts/golive-dryrun.sh on our own chain ids 31337 / 9466301; refused anywhere else,
#                           4663 included): the owner may be script/local/StandInSafe.sol (a WARN, the proxy/singleton/module/guard
#                           reads skipped; threshold and owners still checked), and a missing --canister is a WARN
set -uo pipefail
export PATH="$HOME/.foundry/bin:$PATH"
cd "$(dirname "$0")/.."
CANISTER=""; CANISTER_EVM=""; ICNET="ic"; ARGS=(); PHASE1=0; NOGUARD=0; BEFORE_REINSTALL=0; SAFE_EXPECT=""; LAUNCH=""; MONCFG=""; INSTALLARG=""; INSTALLSEEN=0
STAGE_SEL=""; PROPOSAL=""; BASELINE=""; PROBE_DEPTH=0
# the smallest usable custody fee-pool entry the go/no-go read accepts (sats): --min-fee-pool-sats; the monitor's min_fee_pool_sats
FEE_POOL_MIN_SATS=20000
while [ $# -gt 0 ]; do
  case "$1" in
    --phase1) PHASE1=1;;
    --probe-depth) PROBE_DEPTH=1;;
    --no-guardian) NOGUARD=1;;
    --before-reinstall) BEFORE_REINSTALL=1;;
    --safe-owners) SAFE_EXPECT="${2:-}"; shift
      echo "$SAFE_EXPECT" | grep -Eq '^0x[0-9a-fA-F]{40},0x[0-9a-fA-F]{40}$' || { echo "refused: --safe-owners takes the two owner addresses, comma-separated"; exit 1; };;
    --canister) CANISTER="${2:-}"; shift
      echo "$CANISTER" | grep -Eq '^[a-z0-9]{5}(-[a-z0-9]{5}){3,}-[a-z0-9]{3}$' || { echo "refused: --canister needs a canister id"; exit 1; };;
    --network) ICNET="${2:-}"; shift; case "$ICNET" in ic|local) ;; *) echo "refused: --network is ic or local"; exit 1;; esac;;
    # audit 2026-09-27-nft D-L-6: the canister's EVM address (its operator-only evm_address()), tied to the stack's custodian
    --canister-evm) CANISTER_EVM="${2:-}"; shift
      echo "$CANISTER_EVM" | grep -Eq '^0x[0-9a-fA-F]{40}$' || { echo "refused: --canister-evm takes the canister's EVM address (evm_address(), read by the operator)"; exit 1; };;
    --launch) LAUNCH="${2:-}"; shift; [ -f "$LAUNCH" ] || { echo "refused: --launch takes the launch file (deployments/launch/nft-pilot.json)"; exit 1; };;
    --monitor-config) MONCFG="${2:-}"; shift; [ -f "$MONCFG" ] || { echo "refused: --monitor-config takes the monitor's configuration file"; exit 1; };;
    --install-arg) INSTALLARG="${2:-}"; shift; [ -n "$INSTALLARG" ] || { echo "refused: --install-arg takes the reinstall's argument as Candid hex (4449444c…), or a file holding it"; exit 1; };;
    --stage) STAGE_SEL="${2:-}"; shift; echo "$STAGE_SEL" | grep -Eq '^[1-9]$' || { echo "refused: --stage takes a step of the launch file's stages (1, 2, 3)"; exit 1; };;
    --proposal) PROPOSAL="${2:-}"; shift; [ -n "$PROPOSAL" ] || { echo "refused: --proposal takes the pending upgrade's argument as Candid hex, or a file holding it"; exit 1; };;
    --baseline) BASELINE="${2:-}"; shift; [ -n "$BASELINE" ] || { echo "refused: --baseline takes the running install's argument as Candid hex, or a file holding it"; exit 1; };;
    --min-fee-pool-sats) FEE_POOL_MIN_SATS="${2:-}"; shift
      echo "$FEE_POOL_MIN_SATS" | grep -Eq '^[1-9][0-9]{0,15}$' || { echo "refused: --min-fee-pool-sats takes a whole number of sats (default 20000)"; exit 1; };;
    *) ARGS+=("$1");;
  esac; shift
done
set -- ${ARGS[@]+"${ARGS[@]}"}
# ordinals at launch (nft-pilot.json `bitcoin`): the custody fee pool's minimum is at least the launch file's custodyFeePoolMinSats
if [ -n "$LAUNCH" ]; then
  LFP=$(python3 -c 'import json,sys; print(int((json.load(open(sys.argv[1])).get("bitcoin") or {}).get("custodyFeePoolMinSats") or 0))' "$LAUNCH" 2>/dev/null || echo 0)
  [ "${LFP:-0}" -gt "$FEE_POOL_MIN_SATS" ] 2>/dev/null && FEE_POOL_MIN_SATS=$LFP
fi
# review round 5 (the operator's option (c)): the pending upgrade's argument against the launch file's staged step. No chain is read
if [ -n "$PROPOSAL" ]; then
  [ -n "$LAUNCH" ] || { echo "refused: --proposal needs --launch deployments/launch/nft-pilot.json (the staged plan)"; exit 1; }
  [ -n "$STAGE_SEL" ] || { echo "refused: --proposal needs --stage <the step it carries out> (2: the outflow raise; 3: the caps back to J7)"; exit 1; }
  RUNNING=""
  if [ -n "$CANISTER" ]; then
    dfx="${DFX:-$HOME/Library/Application Support/org.dfinity.dfx/bin/dfx}"; [ -x "$dfx" ] || dfx="$(command -v dfx || true)"
    rc_caps=$(DFX_WARNING=-mainnet_plaintext_identity "$dfx" canister call --network "$ICNET" --identity anonymous --candid autobuyer/canister-rs/etch_autobuyer.did \
      --output json --query "$CANISTER" custody_caps 2>/dev/null) || rc_caps=""
    rc_out=$(DFX_WARNING=-mainnet_plaintext_identity "$dfx" canister call --network "$ICNET" --identity anonymous --candid autobuyer/canister-rs/etch_autobuyer.did \
      --output json --query "$CANISTER" custody_outflow 2>/dev/null) || rc_out=""
    [ -n "$rc_caps" ] && [ -n "$rc_out" ] || { echo "FAIL --proposal: the canister's custody_caps() / custody_outflow() could not be read"; echo "SOME CHECKS FAILED"; exit 1; }
    RUNNING=$(python3 -c 'import json,sys; print(json.dumps({"caps": json.loads(sys.argv[1]), "outflow": json.loads(sys.argv[2])}))' "$rc_caps" "$rc_out")
  fi
  p="$PROPOSAL"; [ -f "$p" ] && p=$(tr -d ' \n\t' < "$p")
  b="$BASELINE"; [ -n "$b" ] && [ -f "$b" ] && b=$(tr -d ' \n\t' < "$b")
  # review round 6 (A6-I-2): under --phase1 the other Init fields are always compared: --baseline, else the monitor's install argument
  if [ -z "$b" ] && [ $PHASE1 = 1 ] && [ -n "$MONCFG" ]; then
    b=$(python3 -c 'import json,re,sys; v=json.load(open(sys.argv[1])).get("install_arg_hex"); v="".join(v.split()) if isinstance(v, str) else ""; print(v if re.fullmatch(r"4449444c(?:[0-9a-fA-F]{2})*", v) else "")' "$MONCFG" 2>/dev/null)
    [ -n "$b" ] && echo "note --proposal: no --baseline; the monitor configuration's install_arg_hex is the baseline for the Init's other fields"
  fi
  [ -n "$RUNNING" ] || [ -n "$b" ] || { echo "refused: --proposal needs the running configuration: --canister <buyer> (its custody_caps() and custody_outflow()) or --baseline <the running install's argument>"; exit 1; }
  out=$(cd monitor && node --input-type=module - "$p" "$b" "$(cd .. && realpath "$LAUNCH")" "$STAGE_SEL" "$RUNNING" "$PHASE1" <<'JS'
const { checkStagedProposal } = await import(process.cwd() + "/src/staged.mjs");
const { decodeInit } = await import(process.cwd() + "/src/candid.mjs");
const { readFileSync } = await import("node:fs");
const [p, b, launch, stage, running, phase1] = process.argv.slice(-6);
const next = decodeInit(p.replace(/^0x/, ""));
const base = b ? decodeInit(b.replace(/^0x/, "")) : null;
const r = checkStagedProposal({ next, base, launch: JSON.parse(readFileSync(launch, "utf8")), step: Number(stage), running: running ? JSON.parse(running) : null,
  requireBaseline: phase1 === "1" });
process.stdout.write(r.lines.join("\n") + "\n");
process.exit(r.ok ? 0 : 1);
JS
  ); rc=$?
  printf '%s\n' "$out"
  if [ $rc = 0 ]; then echo "ALL CHECKS PASS (--proposal: step $STAGE_SEL of the launch plan)"; exit 0; fi
  echo "SOME CHECKS FAILED"; exit 1
fi
if [ "${1:-}" != "" ] && ! echo "$1" | grep -q '\.json$'; then RPC="$1"; shift; else RPC="http://127.0.0.1:8604"; fi
HEX=$(cast chain-id --rpc-url "$RPC" 2>/dev/null) || { echo "refused: the RPC at $RPC did not answer"; exit 1; }
FILE="${1:-deployments/$HEX.json}"; [ $# -gt 0 ] && shift
[ -f "$FILE" ] || { echo "refused: $FILE does not exist"; exit 1; }
j() { python3 -c "import json,sys; print(json.load(open(sys.argv[1]))[sys.argv[2]])" "$1" "$2"; }
lc() { tr 'A-F' 'a-f'; }
fail=0
# G-N3: two reads that both failed are not "the same"; an empty value on either side is a FAIL
same() { # same <what> <expected> <actual>
  if [ -z "$2" ] || [ -z "$3" ]; then echo "FAIL $1: unreadable (expected '${2}', the chain says '${3}'): a read that fails is a FAIL"; fail=1
  elif [ "$(echo "$2" | lc)" = "$(echo "$3" | lc)" ]; then echo "ok   $1"; else echo "FAIL $1: expected $2, the chain says $3"; fail=1; fi
}
# G-N3: `call` runs inside $(…), where `fail=1` would be lost: an empty answer is written to READFAIL and FAILs the run at the end
READFAIL=$(mktemp); trap 'rm -f "$READFAIL"' EXIT
call() { local out; out=$(cast call --rpc-url "$RPC" "$@" 2>/dev/null | awk '{print $1}')
  [ -n "$out" ] || echo "FAIL could not read $1 ${2%%(*}(${*:3}): a read error is a FAIL, never a skip" >>"$READFAIL"; printf '%s\n' "$out"; }
# tcall: a read that is only printed for the operator (never compared): it may come back empty
tcall() { cast call --rpc-url "$RPC" "$@" 2>/dev/null | awk '{print $1}'; }
code() { local c; c=$(cast code --rpc-url "$RPC" "$1" 2>/dev/null); [ -n "$c" ] && [ "$c" != "0x" ]; }
ZERO=0x0000000000000000000000000000000000000000
[ "$HEX" = 4663 ] && PHASE1=1
REHEARSAL=0
if [ "${GOLIVE_REHEARSAL:-}" = 1 ]; then
  case "$HEX" in 31337|9466301) REHEARSAL=1; echo "WARN GOLIVE_REHEARSAL=1 on chain $HEX (our own): a stand-in Safe and a missing --canister are WARNs here, never on a public chain";;
    *) echo "refused: GOLIVE_REHEARSAL is for our own chains (31337, 9466301) only, not chain $HEX"; exit 1;; esac
fi
[ $PHASE1 = 1 ] && echo "     --phase1: the go/no-go read (every owner step made, a 2-of-2 Safe, code-less roles, $([ $NOGUARD = 1 ] && echo "no guardian: declined in writing" || echo "a guardian"))"
# strict <what>: after step 7 (--phase1, or a stack whose owner steps were made) a missing step is a FAIL; before it, a note
strict() { if [ $PHASE1 = 1 ] || [ "${MADE:-}" = "True" ]; then echo "FAIL $1"; fail=1; else echo "note $1"; fi; }
# phase1only <what>: a FAIL in the go/no-go read, a note otherwise (a rehearsal's owner is a dev account, its roles 7702-delegated on a fork)
phase1only() { if [ $PHASE1 = 1 ]; then echo "FAIL $1"; fail=1; else echo "note $1"; fi; }
# nocode <what> <address>: a role address must be an account with no code at all (EIP-7702 sweepers: trust-model.md)
nocode() {
  local c; c=$(cast code --rpc-url "$RPC" "$2" 2>/dev/null)
  if [ "$c" = "0x" ]; then echo "ok   $1 $2 has no code"
  elif echo "$c" | grep -qi '^0xef0100'; then phase1only "$1 $2 has code: an EIP-7702 delegation to 0x${c:8:40} (a sweeper can take anything sent to it)"
  else phase1only "$1 $2 has code (a contract): a role must be a code-less key"; fi
}

same "the file is this chain's" "$(j "$FILE" chainId)" "$HEX"
R=$(j "$FILE" registry); L=$(j "$FILE" launcher); D=$(j "$FILE" distributor); B=$(j "$FILE" beacon); F=$(j "$FILE" ponsFactory)
IMPL=$(j "$FILE" routerImplementation)
for pair in "registry=$R" "launcher=$L" "distributor=$D" "beacon=$B" "pons factory=$F" "router implementation=$IMPL"; do
  name="${pair%%=*}"; addr="${pair#*=}"; code "$addr" && echo "ok   $name has code ($addr)" || { echo "FAIL $name $addr has no code"; fail=1; }
done
# the registry: owned by the multisig, wired once, with the roles the file names
same "registry.owner"         "$(j "$FILE" owner)"     "$(call "$R" 'owner()(address)')"
same "registry.treasury"      "$(j "$FILE" treasury)"  "$(call "$R" 'treasury()(address)')"
WIRED=$(j "$FILE" wired)
if [ "$WIRED" = "True" ] || [ "$(call "$R" 'launcher()(address)' | lc)" != "0x0000000000000000000000000000000000000000" ]; then
  same "registry.launcher"    "$L" "$(call "$R" 'launcher()(address)')"
  same "registry.distributor" "$D" "$(call "$R" 'distributor()(address)')"
  same "registry.keeper (named, paused or not)" "$(j "$FILE" keeper)" "$(call "$R" 'namedKeeper()(address)')"
  same "registry.purchaser"   "$(j "$FILE" purchaser)" "$(call "$R" 'purchaser()(address)')"
  same "registry.attestor"    "$(j "$FILE" attestor)"  "$(call "$R" 'attestor()(address)')"
else
  echo "note the registry is not wired yet: the multisig's wire() and proposeRoles() are still to be made (step 3)"
fi
# fixes-B-C-keeper-draws.md §5 item 15: the keeper not paused; nothing pending that could land later with no one watching
[ "$(call "$R" 'keeperPaused()(bool)')" = "false" ] && echo "ok   registry.keeperPaused() is false" || { echo "FAIL registry.keeperPaused(): the keeper is PAUSED (every keeper power everywhere is off; the Safe's unpauseKeeper)"; fail=1; }
PO=$(call "$R" 'pendingOwner()(address)')
[ "$(echo "$PO" | lc)" = "$ZERO" ] && echo "ok   no pending ownership transfer (registry.pendingOwner() is zero)" || { echo "FAIL registry.pendingOwner() is $PO: a transferOwnership that can be accepted at any time, with no delay"; fail=1; }
PR=$(cast call --rpc-url "$RPC" "$R" 'pendingRoles()(address,address,address,uint64)' 2>/dev/null | tail -1 | awk '{print $1}')
[ "$PR" = "0" ] && echo "ok   no pending role rotation" || { echo "FAIL registry.pendingRoles(): a role rotation is pending (ready at $PR)"; fail=1; }
PT=$(call "$R" 'pendingTreasury()(address)')
[ "$(echo "$PT" | lc)" = "$ZERO" ] && echo "ok   no pending treasury change" || { echo "FAIL registry.pendingTreasury() is $PT"; fail=1; }
[ "$(call "$R" 'pendingBondAt()(uint64)')" = "0" ] && echo "ok   no pending bond change" || { echo "FAIL registry.pendingBondAt() is set: a bond change is pending"; fail=1; }
# the owner: a Safe, 2 of 2 (docs/v6/phase1-scope.md §2), never an EOA or a 7702-delegated account. G-N2: a REAL Safe, not any
# contract answering getThreshold()/getOwners(): the proxy's runtime is a canonical SafeProxy's (1.3.0 or 1.4.1, as their proxy
# factories on 4663 create it: measured 2026-09-26), its singleton (slot 0) a canonical Safe/SafeL2 1.3.0 or 1.4.1 with that
# version's code, no module enabled (a module signs alone), no guard; the owners code-less, distinct, and none of them a role key.
SAFE_PROXY_HASHES="0xb89c1b3bdf2cf8827818646bce9a8f6e372885f8c55e5c07acbd307cb133b000 0xd7d408ebcd99b2b70be43e20253d6d92a8ea8fab29bd3be7f55b10032331fb4c"
SAFE_SINGLETONS="0xd9db270c1b5e3bd161e8c8503c55ceabee709552=0xbba688fbdb21ad2bb58bc320638b43d94e7d100f6f3ebaab0a4e4de6304b1c2e
0x69f4d1788e39c87893c980c06edf4b7f686e2938=0xbba688fbdb21ad2bb58bc320638b43d94e7d100f6f3ebaab0a4e4de6304b1c2e
0x3e5c63644e683549055b9be8653de26e0b4cd36e=0x21842597390c4c6e3c1239e434a682b054bd9548eee5e9b1d6a4482731023c0f
0xfb1bffc9d739b8d520daf37df666da4c687191ea=0x21842597390c4c6e3c1239e434a682b054bd9548eee5e9b1d6a4482731023c0f
0x41675c099f32341bf84bfc5382af534df5c7461a=0x1fe2df852ba3299d6534ef416eefa406e56ced995bca886ab7a553e6d0c5e1c4
0x29fcb43b46531bca003ddc8fcb67ffe91900c762=0xb1f926978a0f44a2c0ec8fe822418ae969bd8c3f18d61e5103100339894f81ff"
SAFE_GUARD_SLOT=0x4a204f620c8c5ccdca3fd54d003badd85ba500436a431f0cbda4f558c93c34c8   # keccak256("guard_manager.guard.address")
# R3-check L-H: the fallback handler (keccak256("fallback_manager.handler.address")) must be none or a canonical
# CompatibilityFallbackHandler (1.3.0, 1.3.0 eip155, 1.4.1), and the Safe's own SafeSetup event must show that setup() made no
# delegatecall that could have planted an owner or a module getOwners()/getModulesPaginated() do not list: its `to` (the event's
# initializer) is zero, or the canonical SafeToL2Setup (Safe{Wallet} on an L2: it only moves slot 0 to the L2 singleton)
SAFE_FALLBACK_SLOT=0x6c9a6c4a39284e37ed1cf53d337577d14212a4870fb976a4366c693b939918d5
SAFE_HANDLERS="0x0000000000000000000000000000000000000000 0xf48f2b2d2a534e402487b3ee7c18c33aec0fe5e4 0x017062a1de2fe6b99be3d9d37841fed19f573804 0xfd0732dc9e303f09fcef3a7388ad10a83459ec99"
SAFE_SETUP_TOPIC=0x141df868a6331af528e38c83b7aa03edc19be66e37ae67f9285bf4f8e3c6a1a8   # SafeSetup(address,address[],uint256,address,address)
SAFE_TO_L2_SETUP=0xbd89a1ce4dde368ffab0ec35506eece0b1ffdc54
SAFE_TO_L2_SETUP_HASH=0x2f25df28caf984366ee584e13241707e85dcd5a6ea0c14267928dafc1fd6274b   # its runtime on 4663, read 2026-09-26
# R4-check L-S1: creation code can write any storage (a module, an owner no list names) and then return the canonical proxy runtime,
# and a later ordinary setup() emits a clean SafeSetup. So the Safe must have been created by a canonical SafeProxyFactory (whose
# proxy creation code writes slot 0 only), which also called setup() as the initializer IN THE SAME TRANSACTION: the SafeSetup's
# transaction must hold exactly one ProxyCreation(proxy, singleton) naming this Safe (proxy indexed in 1.4.1, not in 1.3.0) from a
# factory below whose code on this chain has the canonical hash (1.4.1; 1.3.0 and its eip155 twin; measured on 4663, 2026-09-26)
SAFE_FACTORIES="0x4e1dcf7ad4e460cfd30791ccc4f9c8a4f820ec67=0x50c3cdc4074750a7a974204a716c999edd37482f907608d960b2b025ee0b3317
0xa6b71e26c5e0845f74c812102ca7114b6a896ab2=0x337d7f54be11b6ed55fef7b667ea5488db53db8320a05d1146aa4bd169a39a9b
0xc22834581ebc8527d974f8a1c97e1bea4ef910bc=0x337d7f54be11b6ed55fef7b667ea5488db53db8320a05d1146aa4bd169a39a9b"
PROXY_CREATION_TOPIC=0x4f51faf6c4561ff95f067657e43439f0f856d97c04d9ec9070a6199ad418e235   # ProxyCreation(address,address)
SAFE_OWNERS=""
safecheck() { # safecheck <owner> <its code>
  local O="$1" OC="$2" H SING SH MODS TH OW NO a b SETUP_TX="" SETUP_TH="" SETUP_OW="" PC FAC FH
  H=$(cast keccak "$OC" 2>/dev/null | lc)
  if echo " $SAFE_PROXY_HASHES " | grep -q " $H "; then
    echo "ok   the owner $O is a canonical Safe proxy (runtime $H)"
    SING=$(cast storage --rpc-url "$RPC" "$O" 0 2>/dev/null | lc); SING="0x${SING: -40}"
    SH=$(echo "$SAFE_SINGLETONS" | grep "^$SING=" | cut -d= -f2)
    if [ -z "$SH" ]; then phase1only "the Safe $O's singleton (slot 0) is $SING, not a canonical Safe 1.3.0 / 1.4.1 singleton"
    elif [ "$(cast keccak "$(cast code --rpc-url "$RPC" "$SING" 2>/dev/null)" 2>/dev/null | lc)" != "$SH" ]; then phase1only "the Safe $O's singleton $SING does not carry the canonical Safe code"
    else echo "ok   the Safe $O runs on the canonical singleton $SING"; fi
    MODS=$(cast call --rpc-url "$RPC" "$O" 'getModulesPaginated(address,uint256)(address[],address)' 0x0000000000000000000000000000000000000001 10 2>/dev/null | head -1 | tr -d ' ')
    [ "$MODS" = "[]" ] && echo "ok   the Safe $O has no module enabled" || phase1only "the Safe $O has modules enabled (${MODS:-unreadable}): a module executes without the owners' signatures"
    b=$(cast storage --rpc-url "$RPC" "$O" "$SAFE_GUARD_SLOT" 2>/dev/null | lc)
    if [ -n "$b" ] && [ "$(echo "${b#0x}" | tr -d 0)" = "" ]; then echo "ok   the Safe $O has no guard"; else phase1only "the Safe $O has a transaction guard set (${b:-unreadable}): the go/no-go read expects none"; fi
    b=$(cast storage --rpc-url "$RPC" "$O" "$SAFE_FALLBACK_SLOT" 2>/dev/null | lc)
    if [ -z "$b" ]; then phase1only "the Safe $O's fallback handler could not be read"
    elif echo " $SAFE_HANDLERS " | grep -q " 0x${b: -40} "; then echo "ok   the Safe $O's fallback handler is none or a canonical CompatibilityFallbackHandler (0x${b: -40})"
    else phase1only "the Safe $O's fallback handler is 0x${b: -40}, not a canonical CompatibilityFallbackHandler: it answers every call the Safe does not know (EIP-1271 signatures included)"; fi
    SETUP=$(cast logs --rpc-url "$RPC" --from-block "${SAFE_FROM_BLOCK:-0}" --to-block latest --address "$O" "$SAFE_SETUP_TOPIC" --json 2>/dev/null)
    SETUPR=$(printf '%s' "$SETUP" | python3 -c '
import json, sys
try: logs = json.load(sys.stdin)
except Exception: print("unreadable"); sys.exit(0)
if not isinstance(logs, list): print("unreadable"); sys.exit(0)
if len(logs) != 1: print("count %d" % len(logs)); sys.exit(0)
d = logs[0]["data"][2:]; w = [d[i:i+64] for i in range(0, len(d), 64)]
try:
    n = int(w[4], 16); owners = ",".join("0x" + x[-40:] for x in w[5:5 + n]); assert len(w) >= 5 + n and n > 0
except Exception: print("unreadable"); sys.exit(0)
tx = logs[0].get("transactionHash") or "none"
print("ok 0x" + w[2][-40:] + " " + str(int(w[1], 16)) + " " + owners + " " + tx)' 2>/dev/null)
    # R4-check L-S1: what the SafeSetup says (threshold, owners) and the transaction it was emitted in
    if [ "${SETUPR%% *}" = "ok" ]; then set -- $SETUPR; SETUP_TH="$3"; SETUP_OW=$(echo "$4" | lc | tr ',' '\n' | sort | tr '\n' ' '); SETUP_TX="$5"; fi
    case "$SETUPR" in
      "ok 0x0000000000000000000000000000000000000000 "*) echo "ok   the Safe $O's setup() made no delegatecall (SafeSetup: initializer zero, threshold $SETUP_TH)";;
      "ok $SAFE_TO_L2_SETUP "*)
        if [ "$(cast keccak "$(cast code --rpc-url "$RPC" "$SAFE_TO_L2_SETUP" 2>/dev/null)" 2>/dev/null | lc)" = "$SAFE_TO_L2_SETUP_HASH" ]; then echo "ok   the Safe $O's setup() delegatecalled only the canonical SafeToL2Setup (SafeSetup)"
        else phase1only "the Safe $O's setup() delegatecalled $SAFE_TO_L2_SETUP, whose code on this chain is not the canonical SafeToL2Setup's"; fi;;
      "ok "*) a=${SETUPR#ok }; phase1only "the Safe $O's setup() delegatecalled ${a%% *} (SafeSetup initializer): it could have written an owner or a module that getOwners()/getModulesPaginated() do not list; create the Safe again with no setup call";;
      "count 0") phase1only "no SafeSetup event of the Safe $O since block ${SAFE_FROM_BLOCK:-0} (set SAFE_FROM_BLOCK to a block before its creation, or the RPC does not serve its logs): its setup() cannot be checked";;
      *) phase1only "the Safe $O's SafeSetup event could not be read (${SETUPR:-no answer}): its setup() cannot be checked";;
    esac
    # R4-check L-S1: created by a canonical factory, with setup() in the same transaction
    if [ -z "$SETUP_TX" ]; then :   # no SafeSetup: already a FAIL above
    elif ! echo "$SETUP_TX" | grep -Eq '^0x[0-9a-fA-F]{64}$'; then phase1only "the Safe $O's SafeSetup event has no transaction hash ($SETUP_TX): its creation cannot be checked"
    else
      PC=$(cast receipt --rpc-url "$RPC" "$SETUP_TX" --json 2>/dev/null | python3 -c '
import json, sys
safe, topic = sys.argv[1].lower(), sys.argv[2].lower()
try: r = json.load(sys.stdin)
except Exception: print("unreadable"); sys.exit(0)
if not isinstance(r, dict) or not isinstance(r.get("logs"), list): print("unreadable"); sys.exit(0)
if str(r.get("status", "")).lower() not in ("0x1", "1", "true"): print("failed"); sys.exit(0)
hits = []
for l in r["logs"]:
    t = [x.lower() for x in l.get("topics", [])]
    if not t or t[0] != topic: continue
    d = l.get("data", "0x")[2:]; w = [d[i:i+64] for i in range(0, len(d), 64)]
    if len(t) >= 2 and len(w) >= 1: proxy, sing = "0x" + t[1][-40:], "0x" + w[0][-40:]
    elif len(w) >= 2: proxy, sing = "0x" + w[0][-40:], "0x" + w[1][-40:]
    else: continue
    if proxy == safe: hits.append((l.get("address", "").lower(), sing))
if len(hits) != 1: print("count %d" % len(hits)); sys.exit(0)
print("ok %s %s" % hits[0])' "$O" "$PROXY_CREATION_TOPIC" 2>/dev/null)
      case "$PC" in
        "ok "*)
          set -- $PC; FAC="$2"; a="$3"
          FH=$(echo "$SAFE_FACTORIES" | grep "^$FAC=" | cut -d= -f2)
          if [ -z "$FH" ]; then phase1only "the Safe $O was created by $FAC, not a canonical SafeProxyFactory (ProxyCreation in its SafeSetup's transaction $SETUP_TX): its creation code could have planted a module or an owner no list names"
          elif [ "$(cast keccak "$(cast code --rpc-url "$RPC" "$FAC" 2>/dev/null)" 2>/dev/null | lc)" != "$FH" ]; then phase1only "the factory $FAC that created the Safe $O does not carry the canonical SafeProxyFactory code on this chain"
          elif ! echo "$SAFE_SINGLETONS" | grep -q "^$a="; then phase1only "the Safe $O was created on the singleton $a, not a canonical Safe 1.3.0 / 1.4.1 singleton"
          else echo "ok   the Safe $O was created by the canonical SafeProxyFactory $FAC with setup() in the same transaction ($SETUP_TX): no creation code of anyone else's ran on it"; fi;;
        "count 0") phase1only "the Safe $O's SafeSetup transaction $SETUP_TX holds no ProxyCreation naming it: it was not created by a SafeProxyFactory together with its setup(), so creation code could have planted a module or an owner that getModulesPaginated()/getOwners() do not list; create the Safe with the canonical factory, setup() as its initializer";;
        "count "*) phase1only "the Safe $O's SafeSetup transaction $SETUP_TX holds ${PC#count } ProxyCreation events naming it: expected exactly one";;
        "failed") phase1only "the Safe $O's SafeSetup transaction $SETUP_TX did not succeed according to its receipt";;
        *) phase1only "the receipt of the Safe $O's SafeSetup transaction $SETUP_TX could not be read (${PC:-no answer}): its creation cannot be checked";;
      esac
    fi
  elif [ $REHEARSAL = 1 ]; then echo "WARN the owner $O is not a canonical Safe proxy (runtime $H): accepted as the rehearsal's stand-in (GOLIVE_REHEARSAL)"
  else phase1only "the owner $O is not a canonical Safe proxy (runtime code hash $H): any contract can answer getThreshold() and getOwners()"; fi
  TH=$(call "$O" 'getThreshold()(uint256)')
  OW=$(cast call --rpc-url "$RPC" "$O" 'getOwners()(address[])' 2>/dev/null | tr -d '[] ' | tr ',' '\n' | grep '^0x' | lc)
  NO=$(printf '%s' "$OW" | grep -c '^0x')
  if [ "$TH" = "2" ] && [ "$NO" = "2" ]; then echo "ok   the owner $O is a Safe, 2 of 2"; else phase1only "the owner $O is not a 2-of-2 Safe: threshold ${TH:-unreadable} of ${NO:-?} owners"; fi
  SAFE_OWNERS=$(printf '%s' "$OW" | tr '\n' ' ')
  # R4-check L-S1 (the round-4 fix doc said this was done; it is now): the SafeSetup's owners and threshold are the chain's
  if [ -n "$SETUP_OW" ]; then
    if [ "$SETUP_OW" = "$(printf '%s\n' $SAFE_OWNERS | sort | tr '\n' ' ')" ] && [ "$SETUP_TH" = "$TH" ]; then echo "ok   the Safe's SafeSetup owners and threshold ($SETUP_TH) are the chain's getOwners() / getThreshold()"
    else phase1only "the Safe $O's SafeSetup named owners $SETUP_OW threshold $SETUP_TH, the chain says owners $SAFE_OWNERS threshold ${TH:-unreadable}: changed since its creation (the go/no-go read expects the Safe as the ceremony made it)"; fi
    if [ -n "$SAFE_EXPECT" ]; then
      [ "$(echo "$SAFE_EXPECT" | lc | tr ',' '\n' | sort | tr '\n' ' ')" = "$SETUP_OW" ] && echo "ok   the Safe's SafeSetup owners are the ceremony's devices (--safe-owners)" \
        || phase1only "the Safe $O's SafeSetup owners $SETUP_OW are not the ceremony's --safe-owners $SAFE_EXPECT"
    fi
  fi
  [ "$(printf '%s\n' $SAFE_OWNERS | sort -u | grep -c '^0x')" = "$NO" ] || phase1only "the Safe $O names one owner twice"
  for a in $SAFE_OWNERS; do
    [ "$(cast code --rpc-url "$RPC" "$a" 2>/dev/null)" = "0x" ] && echo "ok   Safe owner $a has no code" || phase1only "Safe owner $a has code (a contract or an EIP-7702 delegation): an owner must be a device's key"
  done
  if [ -n "$SAFE_EXPECT" ]; then
    [ "$(echo "$SAFE_EXPECT" | lc | tr ',' '\n' | sort | tr '\n' ' ')" = "$(printf '%s\n' $SAFE_OWNERS | sort | tr '\n' ' ')" ] \
      && echo "ok   the Safe's owners are the ceremony's devices (--safe-owners)" || { echo "FAIL the Safe's owners are $SAFE_OWNERS, not the ceremony's --safe-owners $SAFE_EXPECT"; fail=1; }
  elif [ $PHASE1 = 1 ] && [ $REHEARSAL = 0 ]; then echo "FAIL --safe-owners is required under --phase1 (R3-check L-H): any two code-less owners would pass without it; name the ceremony's two device addresses"; fail=1
  elif [ $PHASE1 = 1 ]; then echo "WARN no --safe-owners: accepted in the rehearsal only (GOLIVE_REHEARSAL)"; fi
}
# notowner <what> <address>: a role key must not be one of the Safe's owners (a hot key on a server would then sign as the Safe)
notowner() { local a; a=$(echo "$2" | lc); [ -z "$a" ] || [ "$a" = "$ZERO" ] && return 0
  if echo " $SAFE_OWNERS " | grep -q " $a "; then phase1only "the $1 $2 is also an owner of the Safe: every role key is a hot key, and a Safe owner must be a device of its own"; fi; }
OWNER=$(call "$R" 'owner()(address)'); OC=$(cast code --rpc-url "$RPC" "$OWNER" 2>/dev/null)
if [ -z "$OC" ]; then phase1only "the owner $OWNER's code could not be read"
elif [ "$OC" = "0x" ]; then phase1only "the owner $OWNER is an account with no code, not a Safe"
elif echo "$OC" | grep -qi '^0xef0100'; then phase1only "the owner $OWNER is an EIP-7702-delegated account, not a Safe"
else safecheck "$OWNER" "$OC"
fi
# every role address, and the guardian: code-less keys
for pair in "keeper=$(call "$R" 'namedKeeper()(address)')" "purchaser=$(call "$R" 'purchaser()(address)')" "attestor=$(call "$R" 'attestor()(address)')" "registry treasury=$(call "$R" 'treasury()(address)')"; do
  [ "$(echo "${pair#*=}" | lc)" = "$ZERO" ] || nocode "role ${pair%%=*}" "${pair#*=}"
  notowner "role ${pair%%=*}" "${pair#*=}"
done
# the guardian (fixes-B-C-keeper-draws.md §3): it can only pause the keeper and cancel unpinned draws; recommended, optional
G=$(call "$R" 'guardian()(address)')
if [ "$(echo "$G" | lc)" = "$ZERO" ]; then
  if [ $PHASE1 = 1 ] && [ $NOGUARD = 1 ]; then echo "WARN no guardian: only the Safe can pause the keeper or cancel a draw (--no-guardian: the operator's written choice, in the ceremony log)"
  else phase1only "no guardian is set: the Safe's setGuardian(<the monitor's guardian EOA>), or run with --no-guardian and record that choice"; fi
else
  nocode "guardian" "$G"
  notowner "guardian" "$G"
  for pair in "owner=$OWNER" "keeper=$(call "$R" 'namedKeeper()(address)')" "purchaser=$(call "$R" 'purchaser()(address)')" "attestor=$(call "$R" 'attestor()(address)')"; do
    [ "$(echo "$G" | lc)" != "$(echo "${pair#*=}" | lc)" ] || { echo "FAIL the guardian $G is also the ${pair%%=*}: it must be a key of its own (the monitor's)"; fail=1; }
  done
  echo "ok   guardian $G"
fi
# the Launcher's immutables, the escrow it read from Pons, and the router implementation it deployed
same "launcher.factory"               "$F"    "$(call "$L" 'factory()(address)')"
same "launcher.registry"              "$R"    "$(call "$L" 'registry()(address)')"
same "launcher.escrow = Pons' escrow" "$(call "$F" 'feeEscrow()(address)')" "$(call "$L" 'escrow()(address)')"
same "launcher.routerImplementation"  "$IMPL" "$(call "$L" 'routerImplementation()(address)')"
same "router implementation.launcher" "$L"    "$(call "$IMPL" 'launcher()(address)')"
same "router implementation.coin is the burned sentinel" "0x0000000000000000000000000000000000000001" "$(call "$IMPL" 'coin()(address)')"
same "distributor.registry"           "$R"    "$(call "$D" 'registry()(address)')"
same "distributor.beacon"             "$B"    "$(call "$D" 'beacon()(address)')"
# every vault the registry created: its immutables, and for a v2 vault the buyer it can never change
N=$(call "$R" 'count()(uint256)'); echo "     ${N:-unreadable} listing(s)"
echo "${N:-x}" | grep -Eq '^[0-9]+$' || { echo "FAIL registry.count() is unreadable: no listing could be checked (G-N3)"; fail=1; N=0; }
for ((i = 0; i < N; i++)); do
  ROW=$(cast call --rpc-url "$RPC" "$R" 'listing(uint256)((uint8,bool,bool,uint8,address,bytes32,string,string,string))' "$i")
  KIND=$(echo "$ROW" | sed -E 's/^\(([0-9]+), .*/\1/'); AUTO=$(echo "$ROW" | awk -F', ' '{print $3}'); V=$(echo "$ROW" | awk -F', ' '{print $5}')
  code "$V" || { echo "FAIL listing $i: vault $V has no code"; fail=1; continue; }
  same "listing $i vault.registry"  "$R" "$(call "$V" 'registry()(address)')"
  same "listing $i vault.listingId" "$i" "$(call "$V" 'listingId()(uint256)')"
  same "listing $i vault.isRune"    "$([ "$KIND" = 1 ] && echo true || echo false)" "$(call "$V" 'isRune()(bool)')"
  if [ "$AUTO" = "true" ]; then
    BUY=$(call "$V" 'buyer()(address)')
    same "listing $i (v2) the registry knows its buyer" "true" "$(call "$R" 'isAutoBuyer(address)(bool)' "$BUY")"
    code "$BUY" && { echo "FAIL listing $i: buyer $BUY has code; it must be the canister's threshold-key address"; fail=1; } || echo "ok   listing $i buyer $BUY is an account with no code"
    echo "     listing $i maxInFlight $(tcall "$V" 'maxInFlight()(uint256)') wei — compare with the multisig's proposal"
  fi
done
has() { python3 -c "import json,sys; sys.exit(0 if sys.argv[2] in json.load(open(sys.argv[1])) else 1)" "$1" "$2"; }
# docs/v4/nft.md: the NFT side, named on the command line as deployments/nft-<chainid>.json. It borrows this registry's roles.
nftcheck() {
  local W="$1" NR NL NI SP
  NR=$(j "$W" nftRegistry); NL=$(j "$W" nftLauncher); NI=$(j "$W" nftRouterImplementation); SP=$(j "$W" seaport)
  same "nft file is this chain's" "$(j "$W" chainId)" "$HEX"
  for pair in "nft registry=$NR" "nft launcher=$NL" "nft router implementation=$NI"; do
    name="${pair%%=*}"; addr="${pair#*=}"; code "$addr" && echo "ok   $name has code ($addr)" || { echo "FAIL $name $addr has no code"; fail=1; }
  done
  same "nftRegistry.listingRegistry"  "$R"  "$(call "$NR" 'listingRegistry()(address)')"
  same "nftRegistry.beacon"           "$B"  "$(call "$NR" 'beacon()(address)')"
  same "nftRegistry.owner = registry.owner (borrowed)" "$(call "$R" 'owner()(address)')" "$(call "$NR" 'owner()(address)')"
  same "nftRegistry.keeper = registry.keeper (borrowed)" "$(call "$R" 'keeper()(address)')" "$(call "$NR" 'keeper()(address)')"
  if [ "$(echo "$SP" | lc)" != "0x0000000000000000000000000000000000000000" ]; then
    code "$SP" && echo "ok   Seaport has code ($SP)" || { echo "FAIL Seaport $SP has no code"; fail=1; }
    same "Seaport is an allow-listed sweep target" "true" "$(call "$NR" 'isSweepTarget(address)(bool)' "$SP")"
  else
    echo "note no sweep target on this chain (Seaport 1.6 has no code here)"
  fi
  local NW; NW=$(call "$NR" 'launcher()(address)')
  if [ "$(echo "$NW" | lc)" != "0x0000000000000000000000000000000000000000" ]; then same "nftRegistry.launcher" "$NL" "$NW"
  else echo "note the NFT registry is not wired yet: the multisig's wire(nftLauncher) is still to be made"; fi
  # docs/v4/nft.md §2.1: the registry reserves remote buyers against role rotations, and only the NFT registry may reserve
  local RN; RN=$(call "$R" 'nftRegistry()(address)')
  if [ "$(echo "$RN" | lc)" != "0x0000000000000000000000000000000000000000" ]; then same "registry.nftRegistry" "$NR" "$RN"
  else echo "note the registry is not wired to the NFT registry yet: the multisig's wireNft(nftRegistry) is still to be made"; fi
  same "nftLauncher.registry"                "$NR" "$(call "$NL" 'registry()(address)')"
  same "nftLauncher.factory"                 "$F"  "$(call "$NL" 'factory()(address)')"
  same "nftLauncher.escrow = Pons' escrow"   "$(call "$F" 'feeEscrow()(address)')" "$(call "$NL" 'escrow()(address)')"
  same "nftLauncher.routerImplementation"    "$NI" "$(call "$NL" 'routerImplementation()(address)')"
  same "nft router implementation.launcher"  "$NL" "$(call "$NI" 'launcher()(address)')"
  same "nft router implementation.coin is the burned sentinel" "0x0000000000000000000000000000000000000001" "$(call "$NI" 'coin()(address)')"
  local N; N=$(call "$NR" 'count()(uint256)'); echo "     ${N:-unreadable} NFT listing(s)"
  echo "${N:-x}" | grep -Eq '^[0-9]+$' || { echo "FAIL nftRegistry.count() is unreadable: no NFT listing could be checked"; fail=1; N=0; }
  for ((i = 0; i < N; i++)); do
    local ROW CH POL COL V BUY
    ROW=$(cast call --rpc-url "$RPC" "$NR" 'listing(uint256)((uint64,uint8,bool,address,address,address,string))' "$i")
    CH=$(echo "$ROW" | sed -E 's/^\(([0-9]+)[ ,].*/\1/'); POL=$(echo "$ROW" | awk -F', ' '{print $2}')
    COL=$(echo "$ROW" | awk -F', ' '{print $4}'); V=$(echo "$ROW" | awk -F', ' '{print $5}'); BUY=$(echo "$ROW" | awk -F', ' '{print $6}')
    code "$V" || { echo "FAIL nft listing $i: vault $V has no code"; fail=1; continue; }
    same "nft listing $i vault.registry"   "$NR"  "$(call "$V" 'registry()(address)')"
    same "nft listing $i vault.listingId"  "$i"   "$(call "$V" 'listingId()(uint256)')"
    same "nft listing $i vault.chainId"    "$CH"  "$(call "$V" 'chainId()(uint64)')"
    same "nft listing $i vault.collection" "$COL" "$(call "$V" 'collection()(address)')"
    same "nft listing $i vault.policy"     "$POL" "$(call "$V" 'policy()(uint8)')"
    same "nft listing $i vault.beacon"     "$B"   "$(call "$V" 'beacon()(address)')"
    if [ "$CH" = "$HEX" ]; then
      code "$COL" && echo "ok   nft listing $i collection $COL has code on this chain" || { echo "FAIL nft listing $i: collection $COL has no code"; fail=1; }
    else
      same "nft listing $i (chain $CH) vault.buyer" "$BUY" "$(call "$V" 'buyer()(address)')"
      code "$BUY" && { echo "FAIL nft listing $i: buyer $BUY has code; it must be the canister's threshold-key address"; fail=1; } || echo "ok   nft listing $i buyer $BUY is an account with no code"
      same "nft listing $i buyer is reserved in the registry (no role may rotate onto it)" "true" "$(call "$R" 'isAutoBuyer(address)(bool)' "$BUY")"
      echo "     nft listing $i ($(cast call --rpc-url "$RPC" "$NR" 'ref(uint256)(string)' "$i" | tr -d '"')) maxInFlight $(tcall "$V" 'maxInFlight()(uint256)') wei"
    fi
  done
}
# docs/v6/deploy-stack.md: the whole stack, named as deployments/stack-<chainid>.json. Every wiring of §2 read back, the owner's
# steps of §6 read back (or noted as still to be made), and the stage's caps (§5) compared with what the wrapper and the module hold.
jp() { python3 -c "
import json,sys
v=json.load(open(sys.argv[1]))
for k in sys.argv[2].split('.'): v = v[int(k)] if isinstance(v, list) else v[k]
print(v if v is not None else '')" "$1" "$2"; }
jn() { python3 -c "import json,sys; print(len(json.load(open(sys.argv[1]))[sys.argv[2]]))" "$1" "$2"; }
# K9: the buyer canister's custody_caps(), read once, and held to the stage per collection. Optional: no --canister, no read.
CAPS=""
readcaps() {
  if [ -z "$CANISTER" ]; then echo "note the canister was not read (no --canister): compare its custody_caps() with the stage above by hand"; return; fi
  local dfx="${DFX:-$HOME/Library/Application Support/org.dfinity.dfx/bin/dfx}"
  [ -x "$dfx" ] || dfx="$(command -v dfx || true)"
  if [ -z "$dfx" ]; then echo "FAIL custody_caps() of $CANISTER: no dfx to read it with (set DFX)"; fail=1; return; fi
  CAPS=$(DFX_WARNING=-mainnet_plaintext_identity "$dfx" canister call --network "$ICNET" --identity anonymous \
    --candid autobuyer/canister-rs/etch_autobuyer.did --output json --query "$CANISTER" custody_caps 2>/dev/null) || CAPS=""
  if ! CAPS=$(printf '%s' "$CAPS" | python3 -c '
import json, sys
r = json.load(sys.stdin)
p = r[0] if isinstance(r, list) else r            # (CapPolicy, vec Headroom): the policy is the first value
n = lambda x: int(str(x).replace("_", ""))          # dfx writes a nat with digit separators
print(json.dumps({"global": [n(p["global_max_items"]), n(p["global_max_value_wei"])],
                  "cols": {c["collection"].lower(): [n(c["max_items"]), n(c["max_value_wei"]), n(c["max_item_wei"])] for c in p["collections"]}}))' 2>/dev/null); then
    echo "FAIL custody_caps() of $CANISTER on $ICNET could not be read"; fail=1; CAPS=""; return
  fi
  echo "     canister $CANISTER custody_caps(): $(printf '%s' "$CAPS" | python3 -c '
import json, sys
c = json.load(sys.stdin); e = lambda w: f"{w / 10**18:g} ETH"
g = c["global"]
parts = [f"in all {g[0]} items and {e(g[1])}"] + [f"{k}: {v[0]} items, {e(v[1])} (one item up to {e(v[2])})" for k, v in sorted(c["cols"].items())]
print("; ".join(parts))')"
  # docs/v7/dormant-collections.md §6: with the allowlist, the launch file's step NAMES the global (globalItems, globalWei) and it binds;
  # the canister's global is exactly it. Without a named global, J7: no custody total binds before the collections' own caps
  local bind; bind=$([ -n "$LAUNCH" ] && python3 - "$LAUNCH" "$STAGE_SEL" <<'PY'
import json, sys
L = json.load(open(sys.argv[1])); st = int(sys.argv[2] or L.get("stage") or 1)
s = next((x for x in L.get("stages", []) if int(x.get("step", 0)) == st), None)
print(str(int(s["globalItems"])) + " " + str(int(s["globalWei"])) if s and "globalItems" in s and "globalWei" in s else "")
PY
)
  if [ -n "$bind" ]; then
    local gw; gw=$(printf '%s' "$CAPS" | python3 -c 'import json,sys; g=json.load(sys.stdin)["global"]; print(g[0], g[1])')
    same "custody caps: the global item and value caps = the launch step's globalItems and globalWei (binding: docs/v7/dormant-collections.md §6)" "$bind" "$gw"
    return 0
  fi
  # J7 (2026-09-27): no custody total binds before the collections' own caps (the canister's global is at least their sum)
  local tot; tot=$(printf '%s' "$CAPS" | python3 -c '
import json, sys
c = json.load(sys.stdin); g = c["global"]
si = sum(v[0] for v in c["cols"].values()); sv = sum(v[1] for v in c["cols"].values())
print("ok" if g[0] >= si and g[1] >= sv else f"{g[0]} items and {g[1]} wei in all, under the collections sum of {si} and {sv}")')
  [ "$tot" = ok ] && echo "ok   custody caps: the total is at least the sum of the collections' caps, so it never binds before them (J7)" \
    || { echo "FAIL custody caps: $tot: a total that binds before the collections' own caps (J7: no total)"; fail=1; }
}
# Integration R3 (fixes-R3-canister.md §2.2, §8): a conflict-out may be re-priced up to MAX_CONFLICT_FEE_SATS (100,000 sats), a
# CEILING, and only a fee-pool entry that large lets it reach it. The operator holds less (2026-09-26: 20,000 sats by default,
# --min-fee-pool-sats): the go/no-go read requires one usable (not quarantined) custody fee input of at least FEE_POOL_MIN_SATS in
# the canister's public custody_fee_pool() query, and notes when the largest is under the ceiling. The trade-off: in a large fee
# spike a stuck delivery is cancelled more slowly; the gated Wrapper (refunds only after markUndeliverable or 30 days) keeps that
# from becoming a loss.
CONFLICT_CEILING_SATS=100000
feepoolcheck() {
  [ -n "$CANISTER" ] || return 0
  local dfx="${DFX:-$HOME/Library/Application Support/org.dfinity.dfx/bin/dfx}" POOL
  [ -x "$dfx" ] || dfx="$(command -v dfx || true)"
  if [ -z "$dfx" ]; then echo "FAIL custody_fee_pool() of $CANISTER: no dfx to read it with (set DFX)"; fail=1; return; fi
  POOL=$(DFX_WARNING=-mainnet_plaintext_identity "$dfx" canister call --network "$ICNET" --identity anonymous \
    --candid autobuyer/canister-rs/etch_autobuyer.did --output json --query "$CANISTER" custody_fee_pool 2>/dev/null) || POOL=""
  if ! POOL=$(printf '%s' "$POOL" | python3 -c '
import json, sys
r = json.load(sys.stdin)
p = r[0] if isinstance(r, list) else r
n = lambda x: int(str(x).replace("_", ""))
q = p.get("quarantined") or []
q = q[0] if q and isinstance(q[0], list) else q         # opt vec: [] or [[...]]
print(json.dumps({"entries": sorted((n(e["value"]) for e in p["entries"]), reverse=True), "quarantined": len(q)}))' 2>/dev/null); then
    echo "FAIL custody_fee_pool() of $CANISTER on $ICNET could not be read"; fail=1; return
  fi
  local top cnt quar
  top=$(printf '%s' "$POOL" | python3 -c 'import json,sys; e=json.load(sys.stdin)["entries"]; print(e[0] if e else 0)')
  cnt=$(printf '%s' "$POOL" | python3 -c 'import json,sys; print(len(json.load(sys.stdin)["entries"]))')
  quar=$(printf '%s' "$POOL" | python3 -c 'import json,sys; print(json.load(sys.stdin)["quarantined"])')
  echo "     canister $CANISTER custody_fee_pool(): $cnt usable entr$([ "$cnt" = 1 ] && echo y || echo ies), the largest $top sats; $quar quarantined"
  [ "$quar" = 0 ] || echo "WARN $quar custody fee input(s) quarantined (three failed reads): look at them (remove_custody_fee_input, or re-add)"
  if [ "$top" -ge "$FEE_POOL_MIN_SATS" ]; then
    echo "ok   custody fee pool: one entry of $top sats (>= the minimum $FEE_POOL_MIN_SATS, --min-fee-pool-sats)"
    [ "$top" -ge $CONFLICT_CEILING_SATS ] || echo "note custody fee pool: the largest entry ($top sats) is under the conflict-out's $CONFLICT_CEILING_SATS-sat ceiling: in a large fee spike a stuck delivery is cancelled more slowly; the gated Wrapper (refunds only after markUndeliverable or 30 days) keeps that from becoming a loss"
  else echo "FAIL custody fee pool: no usable entry of at least $FEE_POOL_MIN_SATS sats (largest $top; --min-fee-pool-sats): a stuck delivery's conflict-out could not be re-priced (fixes-R3-canister §2.2); add_custody_fee_input one before value enters custody"; fail=1; fi
}
# J7 (the operator, 2026-09-27; custodycap.rs FIRST_EVM_*): an Ethereum collection's custody cap at stage 0 is 15 ETH (one item up to
# 15 ETH), its item count what fits 15 ETH at its floor (the launch file's custodyItems), at most 24, and no total binds
J7_WEI=15000000000000000000
J7_MAX_ITEMS=24
# capcheck <slug> <kind> <wrapper> <stack file> <collection path>: the canister's cap for this collection is the stage's (J7's for an
# Ethereum collection, the launch file's item count with --launch)
capcheck() {
  [ -n "$CAPS" ] || return 0
  local key got
  if [ "$2" = "inscriptions" ]; then key="btc:$(echo "$3" | lc)"
  else key="$(jp "$4" "$5.custody.chainId"):$(jp "$4" "$5.custody.collection" | lc)"; fi   # docs/v7/phase1-nft.md §4.1: evm:<chain>:0x<collection>
  got=$(printf '%s' "$CAPS" | python3 -c 'import json,sys; v=json.load(sys.stdin)["cols"].get(sys.argv[1]); print("" if v is None else f"{v[0]} {v[1]} {v[2]}")' "$key")
  if [ -z "$got" ]; then
    # ordinals at launch: the launch file decides this Bitcoin collection, so the canister must already name it (its install's wrapper)
    if [ "$2" = "inscriptions" ] && [ -n "$(launchbtc slug)" ]; then
      echo "FAIL $1: the canister names no cap for $key: its install's wrapper is not this stack's wrapper (or unset), so it serves no Bitcoin collection; the pre-handover reinstall names wrapper = $3 (docs/v7/golive-runbook.md §9, the Bitcoin collection at launch)"; fail=1; return 0
    fi
    echo "note $1: the canister names no cap for $key yet (unlisted means denied; the pre-handover reinstall with wrapper = $3 names it at the stage's numbers)"; return 0
  fi
  set -- "$1" "$2" "$3" "$4" "$5" $got
  if [ "$2" = "nft" ] && [ "$STAGE" = "0" ]; then
    if [ -n "$LAUNCH" ]; then
      local c; c=$(jp "$4" "$5.custody.collection")
      same "$1 canister cap: items in custody = the launch file's custodyItems (J7: what fits 15 ETH at its floor)" "$(launchval "$c" custodyItems)" "$6"
      same "$1 canister cap: wei in custody = the launch file's custodyWei (J7)" "$(launchval "$c" custodyWei)" "$7"
    else
      [ "$6" -le "$J7_MAX_ITEMS" ] 2>/dev/null && echo "ok   $1 canister cap: $6 items in custody (J7: at most $J7_MAX_ITEMS)" \
        || { echo "FAIL $1 canister cap: $6 items in custody, over J7's $J7_MAX_ITEMS"; fail=1; }
      same "$1 canister cap: wei in custody = J7's 15 ETH" "$J7_WEI" "$7"
    fi
    same "$1 canister cap: one item up to J7's 15 ETH" "$J7_WEI" "$8"
    return 0
  fi
  same "$1 canister cap: items in custody = the stage's"  "$(jp "$4" caps.custodyItemsPerCollection)" "$6"
  same "$1 canister cap: wei in custody = the stage's"    "$(jp "$4" caps.custodyWeiPerCollection)"   "$7"
  if [ "$2" = "inscriptions" ] && [ -n "$(launchbtc slug)" ]; then
    echo "ok   $1 canister names the cap btc:$(echo "$3" | lc): its install's wrapper is this stack's"
    same "$1 canister cap: items in custody = the launch file's bitcoin.custodyItems (H2 stage 0)" "$(launchbtc custodyItems)" "$6"
    same "$1 canister cap: wei in custody = the launch file's bitcoin.custodyWei (H2 stage 0)" "$(launchbtc custodyWei)" "$7"
  fi
}
# The operator's launch decisions (docs/v7/phase1-nft.md §9, decided 2026-09-27), held here so the go/no-go read enforces them:
#   D1 the Ethereum collections at launch: several at once — Milady, Azuki, Mutant Ape Yacht Club, and Pudgy Penguins and BAYC
#      through per-collection graduation targets and pooled graduations (docs/v7/phase1-nft-pooled.md; J1, decided 2026-09-27: BAYC's
#      factory at 10 ETH, its module at convBps 1000 / reserveBps 0, held by --launch against deployments/launch/nft-pilot.json).
#   D5 holder deposits ON (eth_nft.deposits = true). D7 the floor rule: rank 2 of >= 3 listings, >= 3 sales, +10 % / -15 %
#      (eth_nft.floor = null, or exactly those numbers). Relay's fee bound: 300 bps (docs §4.1).
# docs/v7/dormant-collections.md §1 (2026-09-29): the allowlist adds Doodles, Moonbirds, CloneX, Lil Pudgys and Checks (VV Originals), every
# one DORMANT until a wake; CryptoPunks, DeGods, Captainz and Meebits are left out (the reasons are there). The operator, 2026-10-09:
# Identity MD joins, dormant too (OpenSea its only D8 reference: nft-pilot.json priceRefs)
NFT_LAUNCH="0x5af0d9827e0c53e4799bb226655a1de152a425a5=milady 0xed5af388653567af2f388e6224dc7c4b3241c544=azuki 0x60e4d786628fea6478f785a6d7e704777c86a7c6=mayc 0xbd3531da5cf5857e7cfaa92426877b022e612cf8=pudgypenguins 0xbc4ca0eda7647a8ab7c2061c2e118a18a936f13d=bayc 0x8a90cab2b38dba80c64b7734e58ee1db38b8992e=doodles 0x23581767a106ae21c074b2276d25e5c3e136a68b=moonbirds 0x49cf6f5d44e70224e2e23fdcdd2c053f30ada28b=clonex 0x524cab2ec69124574082676e6f654a18df49a048=lilpudgys 0x036721e5a769cc48b3189efbb9cce4471e8a48b1=checks 0x0000ec93127baa929e58e97dd0095a2bfb38ec1d=identitymd"
NFT_LATER=""
# nftlaunch <slug> <collection>: D1 — the collection is one the operator launches with
nftlaunch() {
  local c; c=$(echo "$2" | lc)
  if echo " $NFT_LAUNCH " | grep -q " $c="; then echo "ok   $1 collection $c is in the operator's launch list (D1)"
  elif echo " $NFT_LATER " | grep -q "$c="; then phase1only "$1 collection $c is not in this launch (D1: $(echo " $NFT_LATER " | sed -E "s/.* $c=([^0]*).*/\1/"))"
  else phase1only "$1 collection $c is not one of the allowlisted collections (docs/v7/dormant-collections.md §1: Milady, Azuki, MAYC, Pudgy Penguins, BAYC, Doodles, Moonbirds, CloneX, Lil Pudgys, Checks, Identity MD)"; fi
}
# ethcheck <slug> <collection> <wrapper> <module>: with --canister, its eth_nft_config() names this collection with this wrapper and
# module, and holds the launch decisions: deposits on (D5), the floor rule (D7), Relay's 300 bps
ethcheck() {
  [ -n "$CANISTER" ] || { echo "note $1: the canister's eth_nft_config() was not read (no --canister)"; return 0; }
  local dfx="${DFX:-$HOME/Library/Application Support/org.dfinity.dfx/bin/dfx}" got
  [ -x "$dfx" ] || dfx="$(command -v dfx || true)"
  got=$(DFX_WARNING=-mainnet_plaintext_identity "$dfx" canister call --network "$ICNET" --identity anonymous \
    --candid autobuyer/canister-rs/etch_autobuyer.did --output json --query "$CANISTER" eth_nft_config 2>/dev/null | python3 -c '
import json, sys
r = json.load(sys.stdin); r = r[0] if isinstance(r, list) else r
if not r: print("none"); sys.exit()
want = sys.argv[1].lower()
n = lambda v: int(str(v).replace("_", ""))
fl = r.get("floor") or []
fl = fl[0] if isinstance(fl, list) and fl else (fl or None)
d7 = fl is None or (n(fl["min_listings"]), n(fl["listing_rank"]), n(fl["min_sales"]), n(fl["up_cap_bps"]), n(fl["down_cap_bps"])) == (3, 2, 3, 1000, 1500)
dm = r.get("dormancy") or []
dm = dm[0] if isinstance(dm, list) and dm else (dm or None)
dorm = "none" if not dm else "/".join(str(n(dm[k])) for k in ("max_active", "max_waking", "wakes_per_day", "wake_timeout_secs", "cooldown_secs", "idle_days"))
# docs/v7/dormant-collections.md §11: the scaling from the cycles balance (its B-cycle figures), or "fixed" when the caps are fixed
sc = (dm or {}).get("scaling") or []
sc = sc[0] if isinstance(sc, list) and sc else (sc if isinstance(sc, dict) and sc else None)
if dm: dorm += "/" + ("fixed" if not sc else ":".join(str(n(sc[k])) for k in ("active_b_per_day", "idle_b_per_day", "wake_b", "runway_days", "reserve_b", "exit_b_per_awake")))
wd = (dm or {}).get("wake_desk") or []
wd = wd[0] if isinstance(wd, list) and wd else (wd if isinstance(wd, str) and wd else "none")
for c in r["collections"]:
    if c["collection"].lower() == want:
        print(c["wrapper"].lower(), c["module"].lower(), "deposits=" + ("on" if r.get("deposits") else "off"), "floor=" + ("d7" if d7 else "other"),
              "relay=" + str(n(r.get("relay_max_fee_bps", 0))), "collections=" + ",".join(sorted(x["collection"].lower() for x in r["collections"])),
              "premium=" + str(n(c.get("max_premium_bps", 0))), "chain=" + str(n(r.get("chain_id", 0))), "maxp=" + str(n(c.get("max_purchase_wei", 0))),
              "dorm=" + dorm, "desk=" + str(wd).lower())
        break
else: print("absent")' "$2" 2>/dev/null) || got=""
  case "$got" in
    "") echo "FAIL $1: the canister's eth_nft_config() could not be read"; fail=1;;
    none) echo "FAIL $1: the canister has no eth_nft configuration (null): nothing would be bought or delivered"; fail=1;;
    absent) echo "FAIL $1: the canister's eth_nft_config() does not name $2"; fail=1;;
    *) set -- "$1" "$2" "$3" "$4" $got
       same "$1 canister eth_nft_config: wrapper and module" "$(echo "$3 $4" | lc)" "$5 $6"
       [ "$7" = "deposits=on" ] && echo "ok   $1 canister eth_nft.deposits = true (D5: holder deposits on at launch)" \
         || { echo "FAIL $1 canister eth_nft.deposits is off: the operator's launch decision D5 is deposits ON"; fail=1; }
       [ "$8" = "floor=d7" ] && echo "ok   $1 canister eth_nft.floor is the D7 rule (rank 2 of >= 3 listings, >= 3 sales, +10 % / -15 %)" \
         || { echo "FAIL $1 canister eth_nft.floor is not the D7 rule (null, or 3 / 2 / 3 / 1000 / 1500)"; fail=1; }
       same "$1 canister eth_nft.relay_max_fee_bps (docs §4.1)" "relay=300" "$9"
       ETHCOLS="${10#collections=}"
       # audit 2026-09-27-nft D-L-6: the buy's premium the Trust page states (5 %), and Ethereum mainnet
       same "$1 canister max_premium_bps (the Trust page's 5 % over our floor)" "premium=500" "${11}"
       same "$1 canister eth_nft.chain_id (Ethereum mainnet)" "chain=1" "${12}"
       # round 2 (D2-L-5): the purchase limit the launch decided for this collection
       if [ -n "$LAUNCH" ]; then same "$1 canister max_purchase_wei = the launch file's maxPurchaseWei" "maxp=$(launchval "$2" maxPurchaseWei)" "${13}"
       else echo "note $1: canister max_purchase_wei ${13#maxp=} wei (not held against a launch file: no --launch)"; fi
       # docs/v7/dormant-collections.md §3.2: the canister's dormancy is the launch file's, and its desk the stack file's
       local LDORM; LDORM=$([ -n "$LAUNCH" ] && launchdorm)
       if [ -n "$LDORM" ]; then
         same "$1 canister eth_nft.dormancy = the launch file's (max_active/max_waking/wakes_per_day/wake_timeout_secs/cooldown_secs/idle_days/scaling)" "dorm=$LDORM" "${14}"
         local SDESK; SDESK=$(jb "$W" wakeDesk)
         if [ -n "$SDESK" ]; then same "$1 canister eth_nft.dormancy.wake_desk = the stack file's wakeDesk" "desk=$(echo "$SDESK" | lc)" "${15}"
         else phase1only "$1: the stack file names no wakeDesk (the canister's desk is ${15#desk=}): nobody can ask for a dormant collection's wake publicly"; fi
       elif [ "${14}" != "dorm=none" ]; then echo "note $1: the canister's eth_nft.dormancy is ${14#dorm=} (not held against a launch file: it names none)"
       else echo "note $1: the canister keeps every collection always on (eth_nft.dormancy = null): the cycles burn scales with every listed collection (docs/v7/dormant-collections.md §7)"; fi;;
  esac
}
# launchval <collection> <key>: the launch file's value for this collection (the frog stack's default when unnamed; empty if absent)
launchval() { python3 - "$LAUNCH" "$1" "$2" "$STAGE_SEL" <<'PY'
import json, sys
L = json.load(open(sys.argv[1])); c = sys.argv[2].lower(); k = sys.argv[3]
d = {"normalRaise": 5 * 10**18, "earlyRaise": 8 * 10**18, "earlyUntilEth": 2 * 10**18, "maxRaise": 30 * 10**18, "convBps": 300, "reserveBps": 2500}
# review round 5 (option (c)): the item counts are the staged step's (--stage, else the file's "stage")
st = int(sys.argv[4] or L.get("stage") or 1)
step = next((s for s in L.get("stages", []) if int(s.get("step", 0)) == st), None)
for x in L.get("collections", []):
    if str(x.get("collection", "")).lower() == c:
        v = x.get(k, d.get(k))
        if k == "custodyItems" and step is not None: v = step["custodyItems"].get(x.get("slug"), v)
        print("" if v is None else int(v)); break
else: print("")
PY
}
# launchstr <collection> <key>: the launch file's text value for this collection (empty if absent)
launchstr() { python3 - "$LAUNCH" "$1" "$2" <<'PY'
import json, sys
L = json.load(open(sys.argv[1])); c = sys.argv[2].lower()
for x in L.get("collections", []):
    if str(x.get("collection", "")).lower() == c:
        v = x.get(sys.argv[3]); print("" if v is None else str(v)); break
else: print("")
PY
}
# launchdorm: the launch file's dormancy as max_active/max_waking/wakes_per_day/wake_timeout_secs/cooldown_secs/idle_days/<scaling> (empty if
# none); <scaling> is active_b_per_day:idle_b_per_day:wake_b:runway_days:reserve_b:exit_b_per_awake (docs/v7/dormant-collections.md §11),
# or "fixed" when the launch file names none
launchdorm() { python3 -c '
import json, sys
d = json.load(open(sys.argv[1])).get("dormancy")
if not d: print(""); sys.exit()
s = d.get("scaling")
print("/".join(str(int(d[k])) for k in ("max_active", "max_waking", "wakes_per_day", "wake_timeout_secs", "cooldown_secs", "idle_days")) + "/"
      + ("fixed" if not s else ":".join(str(int(s[k])) for k in ("active_b_per_day", "idle_b_per_day", "wake_b", "runway_days", "reserve_b", "exit_b_per_awake"))))' "$LAUNCH"; }
# wakecheck: docs/v7/dormant-collections.md §3.3: the stack's WakeDesk, read back (the directory and treasury the stack's, the bond and the
# window the launch file's, the chain's block time as the modules'), or its absence where the launch file names a dormancy
wakecheck() {
  local WD LD M0; WD=$(jb "$W" wakeDesk)
  LD=$([ -n "$LAUNCH" ] && python3 -c 'import json,sys; print("yes" if json.load(open(sys.argv[1])).get("dormancy") else "")' "$LAUNCH")
  if [ -z "$WD" ]; then
    [ -n "$LD" ] && phase1only "the launch file names a dormancy but the stack file names no wakeDesk: nobody can ask for a dormant collection's wake publicly (DeployStack: ETCH_WAKE_BOND)"
    return 0
  fi
  code "$WD" && echo "ok   wake desk has code ($WD)" || { echo "FAIL wake desk $WD has no code"; fail=1; return 0; }
  same "wakeDesk.directory is the stack's directory" "$DIR" "$(call "$WD" 'directory()(address)')"
  same "wakeDesk.treasury is the stack's treasury (forfeited bonds pay for the wakes)" "$T" "$(call "$WD" 'treasury()(address)')"
  same "wakeDesk.bond = the stack file's wakeBond" "$(jb "$W" wakeBond)" "$(call "$WD" 'bond()(uint256)')"
  if [ -n "$LAUNCH" ] && [ -n "$LD" ]; then
    same "wakeDesk.bond = the launch file's wake.bondWei" "$(python3 -c 'import json,sys; print(int(json.load(open(sys.argv[1]))["wake"]["bondWei"]))' "$LAUNCH" 2>/dev/null)" "$(call "$WD" 'bond()(uint256)')"
    same "wakeDesk.window = the launch file's wake.windowSecs" "$(python3 -c 'import json,sys; print(int(json.load(open(sys.argv[1]))["wake"]["windowSecs"]))' "$LAUNCH" 2>/dev/null)" "$(call "$WD" 'window()(uint256)')"
  fi
  M0=$(jp "$W" collections.0.floorModule)
  same "wakeDesk.parentBlockSeconds = the modules' (both clocks)" "$(call "$M0" 'parentBlockSeconds()(uint256)')" "$(call "$WD" 'parentBlockSeconds()(uint256)')"
  echo "     wake desk: $(tcall "$WD" 'requestCount()(uint256)') request(s) so far"
}
# cyclescheck: docs/v7/dormant-collections.md §7, §11: cycles scale with ACTIVE collections, and the dormancy's limits scale with the
# canister's own balance. With the launch file's dormancy.scaling: the liquid start (cycles.startT - cycles.freezeT) must give
# effective_max_active >= 1 (one collection's runway, its exit reserve and the reserve) AND let the first wake start (its one-time cost on
# top); it prints what each more awake collection needs. Without scaling (fixed caps): the older rule, startT covering minDays at max_active.
read -r -d '' CYCLES_PY <<'PY' || true
import json, sys
L = json.load(open(sys.argv[1])); d = L.get("dormancy"); c = L.get("cycles")
n = len(L.get("collections", []))
if not c: print("note the launch file names no cycles plan (cycles): the burn was not estimated"); sys.exit(0)
start = float(c["startT"])
s = (d or {}).get("scaling")
if d and s:
    B = 10**9
    act, idle, wake, res, ex = (int(s[k]) * B for k in ("active_b_per_day", "idle_b_per_day", "wake_b", "reserve_b", "exit_b_per_awake"))
    days = int(s["runway_days"])
    ceil = int(d["max_active"])
    fixed, slot = res + days * idle, days * act + ex
    need = lambda k: fixed + k * slot
    liquid = int(round((start - float(c.get("freezeT", 0))) * 10**12))
    eff = 0 if liquid < fixed else min(ceil, (liquid - fixed) // slot)
    head = 0 if liquid < need(0) else min(ceil, (liquid - need(0)) // (slot + wake))
    T = lambda x: f"{x / 10**12:g} T"
    usd = float(c.get("usdPerT", 0))
    print(f"     cycles (13-node subnet, docs/v7/dormant-collections.md §11): effective_max_active = min({ceil}, floor((balance - {T(fixed)}) / {T(slot)})) "
          f"[reserve {T(res)} + {days} d x {T(idle)} idle; {days} d x {T(act)} + {T(ex)} exit per awake]; a wake costs {T(wake)} once")
    print("     cycles: liquid balance needed: " + "; ".join(f"{k} awake {T(need(k))} ({T(need(k) + wake)} to start the wake that makes it {k})" for k in range(1, min(ceil, 4) + 1))
          + (f"; {ceil} awake {T(need(ceil))}" if ceil > 4 else ""))
    print(f"     cycles: at the start {start:g} T ({start - float(c.get('freezeT', 0)):g} T liquid" + (f", about US${start * usd:.0f}" if usd else "") +
          f"): effective_max_active {eff} of {ceil} (of {n} collections), wake headroom {head}")
    if eff >= 1 and head >= 1: print(f"ok   the starting balance gives effective_max_active {eff} >= 1 for {days} days with the reserve, and the first wake fits ({T(need(1) + wake)} liquid needed)")
    else: print(f"FAIL the starting balance {start:g} T ({T(liquid)} liquid) does not keep one collection awake for {days} days with the reserve and its wake: at least {T(need(1) + wake)} liquid is needed (raise cycles.startT)")
    sys.exit(0)
idle, act, wake, wpd = float(c["idleTPerDay"]), float(c["activeTPerDay"]), float(c["wakeT"]), float(c["wakesPerDay"])
mind = float(c["minDays"])
awake = int(d["max_active"]) if d else n
rows = [f"{k} active: {idle + k * act + (wpd * wake if k else 0):.1f} T a day ({start / (idle + k * act + (wpd * wake if k else 0)):.0f} days on {start:g} T)" for k in range(0, awake + 1)]
print("     cycles (13-node subnet, docs/v7/dormant-collections.md §7): " + "; ".join(rows))
worst = idle + awake * act + wpd * wake
if not d: print(f"note no dormancy: all {n} listed collections are always on: about {idle + n * act:.1f} T a day")
if start / worst >= mind: print(f"ok   the starting balance {start:g} T covers {start / worst:.1f} days with {awake} collection(s) awake (at least {mind:g}: the monitor pages under 7)")
else: print(f"FAIL the starting balance {start:g} T covers only {start / worst:.1f} days with {awake} collection(s) awake ({worst:.1f} T a day): at least {mind:g} are needed (raise cycles.startT or lower dormancy.max_active)")
PY
cyclescheck() {
  [ -n "$LAUNCH" ] || return 0
  local out; out=$(python3 -c "$CYCLES_PY" "$LAUNCH") || { echo "FAIL the launch file's cycles plan could not be read"; fail=1; return 0; }
  printf '%s\n' "$out"
  echo "$out" | grep -q '^FAIL' && fail=1
  return 0
}
# sameexact <what> <expected> <actual>: as `same`, but case-sensitive (a name or a symbol, not an address)
sameexact() {
  if [ -z "$2" ] || [ -z "$3" ]; then echo "FAIL $1: unreadable (expected '${2}', found '${3}'): a read that fails is a FAIL"; fail=1
  elif [ "$2" = "$3" ]; then echo "ok   $1"; else echo "FAIL $1: expected '$2', found '$3'"; fail=1; fi
}
# launchcheck <slug> <factory> <module> <collection> <wrapper>: round 2 (D2-L-5, A2-I-2): what the deploy landed is what the LAUNCH decided. The
# stack file's targets were written by the same deploy that read the hand-typed comma lists, so comparing with it cannot catch a
# shifted list (BAYC's 10 ETH on Pudgy) or a one-collection variable applied to all five; the launch file can
launchcheck() {
  [ -n "$LAUNCH" ] || return 0
  if [ -z "$(launchval "$4" convBps)" ]; then echo "FAIL $1: the launch file $LAUNCH does not name the collection $4"; fail=1; return; fi
  local k nm sy
  for k in normalRaise earlyRaise earlyUntilEth maxRaise; do same "$1 factory.$k = the launch file's" "$(launchval "$4" $k)" "$(call "$2" "$k()(uint256)")"; done
  for k in convBps reserveBps; do same "$1 module.$k = the launch file's" "$(launchval "$4" $k)" "$(call "$3" "$k()(uint256)")"; done
  # round 3 (A3-L-2): the labels too, by collection address. The wrapper's ERC-20 name and symbol are constructor-set (never
  # corrected after deploy) and the stack file's slug labels the collection for the site, the indexer and this script: a rotated
  # ETCH_NFT_NAMES / _SYMBOLS / _SLUGS list ("Wrapped Bored Ape / wBAYC" over the Miladys) passes every economic check above
  nm=$(cast call --rpc-url "$RPC" "$5" 'name()(string)' 2>/dev/null | sed 's/^"//; s/"$//')
  sy=$(cast call --rpc-url "$RPC" "$5" 'symbol()(string)' 2>/dev/null | sed 's/^"//; s/"$//')
  sameexact "$1 wrapper.name() = the launch file's name (A3-L-2)" "$(launchstr "$4" name)" "$nm"
  sameexact "$1 wrapper.symbol() = the launch file's symbol (A3-L-2)" "$(launchstr "$4" symbol)" "$sy"
  sameexact "$1 stack file's slug = the launch file's slug (A3-L-2)" "$(launchstr "$4" slug)" "$1"
}
# ---- Ordinals at launch (the operator, 2026-10-09: Bitcoin Frogs joins the Ethereum launch; deployments/launch/nft-pilot.json `bitcoin`
# and `_bitcoin`; docs/v8/ordinals-wave.md §3.2). The launch file names ONE Bitcoin collection (the canister holds one wrapper): the
# mixed stack's inscriptions collection is held to it, the staged plan and the exit rule count its cap (btc:0x<wrapper>), and with
# --canister the controller-only setters made before the handover are read back (the Bitcoin floor sink and its purchase limit), with
# UniSat among the content providers and the custody fee pool at the launch file's minimum.
# launchbtc <key>: the launch file's Bitcoin collection's value (custodyItems at the staged step; a number as an integer, an object
# as compact JSON); empty when the file names no Bitcoin collection or not this key
launchbtc() { [ -n "$LAUNCH" ] || return 0; python3 - "$LAUNCH" "$1" "$STAGE_SEL" <<'PY'
import json, sys
L = json.load(open(sys.argv[1])); b = L.get("bitcoin") or {}; k = sys.argv[2]
v = b.get(k)
if k == "custodyItems" and b:
    st = int(sys.argv[3] or L.get("stage") or 1)
    step = next((s for s in L.get("stages", []) if int(s.get("step", 0)) == st), None)
    if step is not None: v = step["custodyItems"].get(b.get("slug"), v)
if v is None: print("")
elif isinstance(v, bool): print("true" if v else "false")
elif isinstance(v, (int, float)): print(int(v))
elif isinstance(v, (dict, list)): print(json.dumps(v, sort_keys=True, separators=(",", ":")))
else: print(v)
PY
}
# the stack's Bitcoin collection, as btclaunch saw it (read again after the per-collection loop)
BTCWRAP=""; BTCMOD=""; BTCSLUG=""; BTCCUST=""
# btclaunch <slug> <wrapper> <module> <listing id> <custody address> <stack file>: the stack's Bitcoin collection is the launch file's
btclaunch() {
  BTCWRAP=$(echo "$2" | lc); BTCMOD=$(echo "$3" | lc); BTCSLUG="$1"; BTCCUST="$5"
  [ -n "$LAUNCH" ] || return 0
  if [ -z "$(launchbtc slug)" ]; then echo "FAIL $1: the stack deploys a Bitcoin (inscriptions) collection the launch file $LAUNCH does not decide (no \`bitcoin\` block)"; fail=1; return 0; fi
  sameexact "$1 stack file's slug = the launch file's bitcoin.slug" "$(launchbtc slug)" "$1"
  local nm sy root out
  nm=$(cast call --rpc-url "$RPC" "$2" 'name()(string)' 2>/dev/null | sed 's/^"//; s/"$//')
  sy=$(cast call --rpc-url "$RPC" "$2" 'symbol()(string)' 2>/dev/null | sed 's/^"//; s/"$//')
  sameexact "$1 wrapper.name() = the launch file's bitcoin.name" "$(launchbtc name)" "$nm"
  sameexact "$1 wrapper.symbol() = the launch file's bitcoin.symbol" "$(launchbtc symbol)" "$sy"
  # the listing's membership root is the launch file's (the root of membersFile's ids, two sources agreeing: btc/README.md)
  root=$(cast call --rpc-url "$RPC" "$R" 'listing(uint256)((uint8,bool,bool,uint8,address,bytes32,string,string,string))' "$4" 2>/dev/null \
    | python3 -c 'import sys; t=sys.stdin.read().strip(); p=t[1:].split(", ") if t.startswith("(") else []; print(p[5] if len(p) > 5 else "")')
  same "$1 listing $4 membership root = the launch file's bitcoin.membersRoot (deployments/launch/btc/README.md)" "$(launchbtc membersRoot)" "$root"
  # the stack's stage-0 cap is the launch file's (H2: 3 items / 10 ETH)
  same "$1 stack caps: items in custody = the launch file's bitcoin.custodyItems" "$(launchbtc custodyItems)" "$(jp "$6" caps.custodyItemsPerCollection)"
  same "$1 stack caps: wei in custody = the launch file's bitcoin.custodyWei" "$(launchbtc custodyWei)" "$(jp "$6" caps.custodyWeiPerCollection)"
  # the members file the runner proves from, and the decided numbers inside the canister's bounds
  out=$(python3 - "$LAUNCH" <<'PY'
import json, os, sys
b = json.load(open(sys.argv[1]))["bitcoin"]; bad = []; ok = []
f, n = b.get("membersFile") or "", b.get("membersCount")
if not os.path.isfile(f): bad.append(f"bitcoin.membersFile {f or '(none)'} is not a file (the runner proves membership from it: RUNNER_OW_MEMBERS)")
else:
    ids = json.load(open(f))
    if not (isinstance(ids, list) and all(isinstance(x, str) and len(x) >= 66 and x[64] == "i" for x in ids) and len(set(ids)) == len(ids)):
        bad.append(f"{f} is not a list of distinct inscription ids")
    elif n is None or len(ids) != int(n): bad.append(f"{f} holds {len(ids)} ids, the launch file's membersCount is {n}")
    else: ok.append(f"the members file {f}: {len(ids)} distinct inscription ids (bitcoin.membersCount)")
ff = b.get("frog_funding") or {}
if not (isinstance(ff.get("max_fee_bps"), int) and 50 <= ff["max_fee_bps"] <= 500 and isinstance(ff.get("float_first"), bool)):
    bad.append(f"bitcoin.frog_funding {ff}: max_fee_bps 50 to 500 and float_first true or false (etch_autobuyer.did FrogFunding, relayconv.rs bounds)")
else: ok.append(f"frog_funding: Relay at max_fee_bps {ff['max_fee_bps']}, float_first {str(ff['float_first']).lower()} (docs/v8/ordinals-wave.md §2 R4)")
mp = int(b.get("maxPurchaseWei") or 0)
if not 5 * 10**16 <= mp <= 25 * 10**18: bad.append(f"bitcoin.maxPurchaseWei {mp}: 0.05 ETH (policy MIN_PURCHASE_WEI) to 25 ETH (PURCHASE_LIMIT_CEILING_WEI)")
if int(b.get("custodyItems") or 0) > 3 or int(b.get("custodyWei") or 0) > 10 * 10**18:
    bad.append("the Bitcoin wrapper's cap is over stage 0's 3 items / 10 ETH (docs/v5/launch-gates.md G7, H2)")
if int(b.get("custodyFeePoolMinSats") or 0) < 20000: bad.append("bitcoin.custodyFeePoolMinSats is under the operator's 20,000-sat minimum (integration R3)")
if b.get("unisatKeyRequired") is not True: bad.append("bitcoin.unisatKeyRequired is not true: contents verify on two per-output providers only with UniSat's key (ordinals-wave §3.3)")
for x in ok: print(f"ok   {x}")
for x in bad: print(f"FAIL {x}")
PY
)
  printf '%s\n' "$out"; echo "$out" | grep -q '^FAIL' && fail=1
  return 0
}
# btcblob <dfx JSON value>: a blob as dfx --output json prints it (hex, 0x-hex, a list of bytes, or text with \xx escapes), as 0x-hex
read -r -d '' BTCBLOB_PY <<'PY' || true
import re
def blob(v):
    if isinstance(v, list): return "0x" + bytes(int(x) for x in v).hex()
    s = str(v)
    if re.fullmatch(r"0x[0-9a-fA-F]*", s): return s.lower()
    if re.fullmatch(r"[0-9a-fA-F]{40}", s): return "0x" + s.lower()
    out, i = bytearray(), 0
    while i < len(s):
        if s[i] == "\\" and re.fullmatch(r"[0-9a-fA-F]{2}", s[i + 1:i + 3]): out.append(int(s[i + 1:i + 3], 16)); i += 3
        else: out += s[i].encode(); i += 1
    return "0x" + out.hex()
PY
# btccanister: with --canister, what only a controller can set and must set BEFORE the handover (the timelock can then only Upgrade /
# Controllers / Roles: docs/v8/ordinals-wave.md §3.3) is read back: floor_state() holds a Bitcoin floor sink for the stack's Bitcoin
# module (set_floor_sink) and its purchase limit (set_purchase_limit) at the launch file's maxPurchaseWei; ord_providers() names
# UniSat (set_unisat_key, after the install and after EVERY upgrade). The cap (the install's `wrapper`) is capcheck's.
btccanister() {
  [ -n "$CANISTER" ] && [ -n "$LAUNCH" ] && [ -n "$BTCMOD" ] && [ -n "$(launchbtc slug)" ] || return 0
  local dfx="${DFX:-$HOME/Library/Application Support/org.dfinity.dfx/bin/dfx}" FS OP got
  [ -x "$dfx" ] || dfx="$(command -v dfx || true)"
  if [ -z "$dfx" ]; then echo "FAIL floor_state() of $CANISTER: no dfx to read it with (set DFX)"; fail=1; return 0; fi
  FS=$(DFX_WARNING=-mainnet_plaintext_identity "$dfx" canister call --network "$ICNET" --identity anonymous \
    --candid autobuyer/canister-rs/etch_autobuyer.did --output json --query "$CANISTER" floor_state 2>/dev/null) || FS=""
  got=$(printf '%s' "$FS" | python3 -c "$BTCBLOB_PY
import json, sys
r = json.load(sys.stdin); r = r[0] if isinstance(r, list) else r
n = lambda x: int(str(x).replace('_', ''))
m = sys.argv[1].lower()
btc = [blob(s['module']).lower() for s in r.get('sinks', []) if 'Bitcoin' in (s.get('item') if isinstance(s.get('item'), (dict, str)) else str(s.get('item')))]
lim = [n(l['max_wei']) for l in r.get('limits', []) if blob(l['sink']).lower() == m]
print(('sink' if m in btc else 'nosink:' + (','.join(btc) or 'none')) + ' ' + (str(lim[0]) if lim else 'none'))" "$BTCMOD" 2>/dev/null) || got=""
  if [ -z "$got" ]; then echo "FAIL floor_state() of $CANISTER on $ICNET could not be read: the Bitcoin floor sink was not checked"; fail=1
  else
    set -- $got
    if [ "$1" = sink ]; then echo "ok   $BTCSLUG canister floor_state(): a Bitcoin floor sink for the module $BTCMOD (set_floor_sink, before the handover)"
    else echo "FAIL $BTCSLUG canister floor_state() has no Bitcoin floor sink for the stack's module $BTCMOD (Bitcoin sinks: ${1#nosink:}): set_floor_sink($BTCMOD, null, null, null) is controller-only and must be made BEFORE the handover"; fail=1; fi
    local want; want=$(launchbtc maxPurchaseWei)
    if [ "$2" = none ]; then
      if [ "$want" = 500000000000000000 ]; then echo "note $BTCSLUG canister names no purchase limit for $BTCMOD: the policy's MAX_PURCHASE_WEI (0.5 ETH, the launch file's) applies; set_purchase_limit($BTCMOD, $want) before the handover pins it"
      else echo "FAIL $BTCSLUG canister names no purchase limit for $BTCMOD: the policy's 0.5 ETH applies, not the launch file's maxPurchaseWei $want (set_purchase_limit is controller-only: before the handover)"; fail=1; fi
    else same "$BTCSLUG canister purchase limit of the Bitcoin module = the launch file's bitcoin.maxPurchaseWei (set_purchase_limit)" "$want" "$2"; fi
  fi
  if [ "$(launchbtc unisatKeyRequired)" = true ]; then
    OP=$(DFX_WARNING=-mainnet_plaintext_identity "$dfx" canister call --network "$ICNET" --identity anonymous \
      --candid autobuyer/canister-rs/etch_autobuyer.did --output json --query "$CANISTER" ord_providers 2>/dev/null) || OP=""
    got=$(printf '%s' "$OP" | python3 -c '
import json, sys
r = json.load(sys.stdin)
r = r[0] if r and isinstance(r, list) and isinstance(r[0], list) and r[0] and isinstance(r[0][0], (list, dict)) else r
name = lambda e: str(e[0] if isinstance(e, list) else (e.get("0") if "0" in e else next(iter(e.values()), "")))
print(",".join(name(e) for e in r) or "none")' 2>/dev/null) || got=""
    if [ -z "$got" ]; then echo "FAIL ord_providers() of $CANISTER could not be read: UniSat's key was not checked"; fail=1
    elif echo ",$got," | grep -q ',unisat,'; then echo "ok   $BTCSLUG canister ord_providers() names UniSat ($got): contents verify on two per-output providers (set_unisat_key; again after every upgrade)"
    else echo "FAIL $BTCSLUG canister ord_providers() does not name UniSat ($got): set_unisat_key <key> (the operator's; lost at every upgrade: set it again after each), or no deposit, delivery or buy can be verified"; fail=1; fi
  fi
}
# btcinstall: the install argument (--install-arg, else the monitor's install_arg_hex) names this stack's wrapper and the launch file's
# frog_funding, and its custody caps the Bitcoin cap at the launch step's numbers
btcinstall() {
  [ -n "$LAUNCH" ] && [ -n "$BTCWRAP" ] && [ -n "$(launchbtc slug)" ] || return 0
  local a="$INSTALLARG" m="" out
  [ -n "$MONCFG" ] && m=$(python3 -c 'import json,sys; v=json.load(open(sys.argv[1])).get("install_arg_hex"); print("".join(v.split()) if isinstance(v, str) else "")' "$MONCFG" 2>/dev/null)
  [ -f "$a" ] && a=$(tr -d ' \n\t' < "$a")
  [ -n "$a" ] || a="$m"
  if ! echo "$a" | grep -Eqi '^(0x)?4449444c'; then echo "note $BTCSLUG: no install argument to read (--install-arg): its wrapper and frog_funding were not checked"; return 0; fi
  local ff it wi; ff=$(launchbtc frog_funding); it=$(launchbtc custodyItems); wi=$(launchbtc custodyWei)   # read here: the node below runs in monitor/
  out=$( ( cd monitor 2>/dev/null && node --input-type=module - "$a" "$BTCWRAP" "$ff" "$it" "$wi" 2>&1 ) <<'JS'
const { decodeInit } = await import(process.cwd() + "/src/candid.mjs");
const [hex, wrapper, ff, items, wei] = process.argv.slice(-5);
const d = decodeInit(hex.replace(/^0x/, ""));
if (!d.init) { console.log(`FAIL the install argument could not be decoded: ${d.error}`); process.exit(0); }
const opt = (v) => (Array.isArray(v) ? (v.length ? v[0] : null) : v ?? null);
const i = d.init, w = opt(i.wrapper), f = opt(i.frog_funding), caps = opt(i.custody_caps);
const say = (ok, t) => console.log(`${ok ? "ok  " : "FAIL"} ${t}`);
say(String(w ?? "").toLowerCase() === wrapper, `the install argument's wrapper is the stack's Bitcoin wrapper ${wrapper} (${w ?? "none"})`);
const want = JSON.parse(ff), got = f ? { float_first: Boolean(f.float_first), max_fee_bps: Number(f.max_fee_bps) } : null;
say(got && got.max_fee_bps === want.max_fee_bps && got.float_first === want.float_first,
  `the install argument's frog_funding is the launch file's (max_fee_bps ${want.max_fee_bps}, float_first ${want.float_first}): ${got ? `${got.max_fee_bps} / ${got.float_first}` : "null (Relay at 300 bps)"}`);
const c = (caps?.collections ?? []).find((x) => String(x.collection).toLowerCase() === `btc:${wrapper}`);
say(c && String(c.max_items) === items && String(c.max_value_wei).replace(/_/g, "") === wei,
  `the install argument's custody cap for btc:${wrapper} is the launch step's ${items} items / ${wei} wei${c ? ` (${c.max_items} / ${c.max_value_wei})` : " (none named)"}`);
say((i.btc_api_urls ?? []).length >= 2, `the install argument names ${(i.btc_api_urls ?? []).length} Bitcoin explorer(s) (btc_api_urls: two, agreeing)`);
JS
)
  printf '%s\n' "$out"; echo "$out" | grep -q '^FAIL' && fail=1
  return 0
}
# btcmonitor: the launch monitor configuration's Bitcoin canary (docs/v6/k6-watcher-canary.md §4): custody, float and buyer on two
# esplora providers (by host), custody the stack wrapper's custody address, the fee-pool minimum not under the launch file's
btcmonitor() {
  [ -n "$MONCFG" ] && [ -n "$LAUNCH" ] && [ -n "$BTCWRAP" ] && [ -n "$(launchbtc slug)" ] || return 0
  local got; got=$(python3 - "$MONCFG" "$BTCCUST" "$(launchbtc custodyFeePoolMinSats)" <<'PY'
import json, re, sys
from urllib.parse import urlsplit
m = json.load(open(sys.argv[1])); b = m.get("bitcoin") or {}; cust = sys.argv[2]; need = int(sys.argv[3] or 20000)
BTC = re.compile(r"^(bc1|tb1)[02-9ac-hj-np-z]{11,87}$")
bad = []
if not b: bad.append("no bitcoin block (the canary on custody, float and buyer)")
hosts = {(urlsplit(str(u)).hostname or "").lower() for u in b.get("providers") or []} - {""}
if len(b.get("providers") or []) != 2 or len(hosts) != 2: bad.append(f"bitcoin.providers must be two esplora bases on two hosts ({len(hosts)} host(s))")
a = b.get("addresses") or {}
for k in ("custody", "float", "buyer"):
    if not BTC.match(str(a.get(k, ""))): bad.append(f"bitcoin.addresses.{k} is not a bech32 address")
if len({str(a.get(k)) for k in ("custody", "float", "buyer")}) != 3: bad.append("bitcoin.addresses: custody, float and buyer are not three addresses")
if str(a.get("custody", "")) != cust: bad.append(f"bitcoin.addresses.custody {a.get('custody')} is not the wrapper's custody address {cust}")
if int(m.get("min_fee_pool_sats", 20000)) < need: bad.append(f"min_fee_pool_sats {m.get('min_fee_pool_sats')} is under the launch file's custodyFeePoolMinSats {need}")
print("ok" if not bad else "; ".join(bad))
PY
)
  [ "$got" = ok ] && echo "ok   the monitor configuration's Bitcoin canary watches custody (the wrapper's), float and buyer on two esplora providers" \
    || { echo "FAIL the monitor configuration's Bitcoin canary: $got"; fail=1; }
}
# moncheck <slug> <collection> <wrapper> <module> <custodian>: round 2: the launch monitor configuration watches this collection: its
# wrapper, module and custodian as deployed, and both of D8's references (OpenSea's slug and CoinGecko's id), or OpenSea ALONE
# (price_refs ["opensea"]) only where the launch file decides priceRefs ["opensea"] for this collection (the operator, 2026-10-09:
# Identity MD; script/d8refs.py). Without --launch, price_refs is refused: there is no decision to hold it to
moncheck() {
  [ -n "$MONCFG" ] || return 0
  local got; got=$(python3 - "$MONCFG" "$2" "$LAUNCH" <<'PY'
import json, os, sys
sys.path.insert(0, os.path.join(os.getcwd(), "script"))
from d8refs import launch_of, monitor_refs
m = json.load(open(sys.argv[1])); c = sys.argv[2].lower()
lf = sys.argv[3] if len(sys.argv) > 3 and sys.argv[3] else None
L = json.load(open(lf)) if lf else None
for x in (m.get("eth_nft") or {}).get("collections", []):
    if str(x.get("collection", "")).lower() == c:
        # review round 4 (A3-L-3): the references are the launch file's for THIS collection (a swap between two collections with
        # close floors, Milady and MAYC, passed every other check)
        want = launch_of(L, c) if L is not None else None
        r = "missing" if L is not None and want is None else monitor_refs(x, want)
        print(str(x.get("wrapper", "")).lower(), str(x.get("module", "")).lower(), str(x.get("custodian", "")).lower(), "refs=" + r); break
else: print("absent")
PY
)
  case "$got" in
    absent) echo "FAIL $1: the monitor configuration $MONCFG does not watch the collection $2 (eth_nft.collections)"; fail=1;;
    *) set -- "$1" "$2" "$3" "$4" "$5" $got
       same "$1 monitor watches this wrapper, module and custodian" "$(echo "$3 $4 $5" | lc)" "$6 $7 $8"
       case "$9" in
         refs=both) echo "ok   $1 monitor names both D8 references (OpenSea and CoinGecko)$([ -n "$LAUNCH" ] && echo ", the launch file's for this collection (A3-L-3)")";;
         refs=opensea) echo "ok   $1 monitor names its one D8 reference, OpenSea alone (price_refs [\"opensea\"]: the launch file's priceRefs, the operator's decision of 2026-10-09), the launch file's for this collection (A3-L-3)";;
         *) echo "FAIL $1: the monitor configuration lacks one of D8's references (opensea_slug, coingecko_id), leaves a <placeholder>, or names references that are not the launch file's openseaSlug / coingeckoId for this collection (A3-L-3); price_refs [\"opensea\"] (OpenSea alone) only where the launch file decides priceRefs [\"opensea\"] for it, with --launch"; fail=1;;
       esac;;
  esac
}
# monglobal: the monitor's guardian is armed for the Ethereum side: auto_pause, D8 on (eth_nft.guardian_pause not false) and the
# no-good-post poster pause set. Round 3 (A3-L-3): no <placeholder> ANYWHERE in the file (a comment key "//…" aside: <p1>, <g1>
# and <runner host> passed before, and meant standing false alarms or no liveness watch), two Ethereum providers counted by HOST
# (eth.drpc.org and eth.drpc.org/ are one), evm.chain_id 4663 when named, and evm.guardian.address the registry's guardian
monglobal() {
  [ -n "$MONCFG" ] || return 0
  local got; got=$(python3 - "$MONCFG" <<'PY'
import json, re, sys
from urllib.parse import urlsplit
m = json.load(open(sys.argv[1])); evm = m.get("evm") or {}; g = evm.get("guardian") or {}; e = m.get("eth_nft") or {}
h = g.get("pause_on_no_good_post_h")
bad = []
if not e: bad.append("no eth_nft block")
if g.get("auto_pause") is not True: bad.append("evm.guardian.auto_pause is not true")
if e.get("guardian_pause") is False: bad.append("eth_nft.guardian_pause is false (D8 off)")
if not (isinstance(h, (int, float)) and 2 <= h <= 160): bad.append("evm.guardian.pause_on_no_good_post_h is not armed (2 to 160 hours)")
ph = []
def walk(v, path):
    if isinstance(v, dict):
        for k, x in v.items():
            if not str(k).startswith("//"): walk(x, f"{path}.{k}" if path else str(k))
    elif isinstance(v, list):
        for i, x in enumerate(v): walk(x, f"{path}[{i}]")
    elif isinstance(v, str) and ("<" in v or re.search(r"\b(TODO|TBD|FIXME|CHANGEME|CHANGE_ME|XXX+|REPLACE_?ME)\b", v, re.I)): ph.append(path)
walk(m, "")
if ph: bad.append("a <placeholder> is left in the monitor file (" + ", ".join(ph[:8]) + (f" and {len(ph) - 8} more" if len(ph) > 8 else "") + ")")
hosts = set()
for u in e.get("eth_rpc") or []:
    try: hosts.add((urlsplit(str(u)).hostname or "").lower())
    except ValueError: pass
hosts.discard("")
if len(hosts) < 2: bad.append(f"eth_nft.eth_rpc names fewer than two Ethereum providers (counted by host: {len(hosts)})")
if "chain_id" in evm and str(evm.get("chain_id")) != "4663": bad.append(f"evm.chain_id is {evm.get('chain_id')}, not 4663 (Robinhood Chain)")
print("ok" if not bad else "; ".join(bad))
PY
)
  [ "$got" = ok ] && echo "ok   the monitor configuration arms the guardian for the Ethereum side (D8, pause_on_no_good_post_h $(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["evm"]["guardian"]["pause_on_no_good_post_h"])' "$MONCFG") h), with no placeholder left and two Ethereum providers by host" \
    || { echo "FAIL the monitor configuration $MONCFG: $got"; fail=1; }
  # A3-L-3: the guardian the monitor signs with is the registry's (read above as $G); otherwise no pause it signs can land
  same "the monitor's evm.guardian.address is registry.guardian() (A3-L-3)" "$G" \
    "$(python3 -c 'import json,sys; print(((json.load(open(sys.argv[1])).get("evm") or {}).get("guardian") or {}).get("address") or "")' "$MONCFG")"
}
# A4-L-1 (review round 4): holders of EVERY collection leaving at once (a bank run against a pending upgrade) must fit the timelock's
# 72 h less a day at the outflow limits: each collection's items at per_collection_items a day, and the whole custody (the collections'
# item caps, or the global cap where it binds) at global_items a day. The canister refuses an install or upgrade that does not
# (canister.rs validate_custody); this read refuses the pair before a proposal is made and prints the operator's options. It does not
# pick one: the J7 item counts and the outflow limits are the operator's (deployments/launch/nft-pilot.json).
read -r -d '' EXIT_PY <<'PY' || true
import json, math, sys
what, src = sys.argv[1], json.loads(sys.argv[2])
days = 2          # the timelock's 72 h floor less a day (canister.rs ASSUMED_DELAY_SECS, outflow::exit_fits)
items, gcap, per, glob = src["items"], src.get("global_cap"), int(src["per"]), int(src["global"])
bad = [f"{k} holds up to {n} items at {per} a day ({per * days} can leave)" for k, n in items.items() if n > per * days]
total = sum(items.values()); held = min(total, gcap) if gcap else total
room = glob * days
if bad: print(f"FAIL {what}: a collection's custody cannot leave within the timelock's 72 h less a day (A4-L-1): " + "; ".join(bad))
if held > room:
    need = math.ceil(held / days); ddays = math.ceil(held / max(glob, 1)) + 1
    cut = {k: n * room // held for k, n in items.items()}
    left = room - sum(cut.values())
    for k in sorted(items, key=lambda k: -(items[k] * room % held)):
        if left <= 0: break
        cut[k] += 1; left -= 1
    name = lambda k: src.get("names", {}).get(k, k)
    print(f"FAIL {what}: custody holds up to {held} items in all and lets {glob} out a day in all: {room} can leave within the timelock's "
          f"72 h less a day and {held - room} cannot (A4-L-1: the canister refuses this install). The operator's options: "
          f"(a) item caps summing to at most {room} at {glob} a day, for example " + ", ".join(f"{name(k)} {items[k]} -> {cut[k]}" for k in items)
          + f"; (b) a global custody cap of {room} items with these per-collection caps (the canister then holds at most {room} items at once: "
          f"J7 names no binding total, so this changes J7); (c) install at (a) or (b), then raise global_items to at least {need} a day by a "
          f"timelocked upgrade (an install starts at most at 24 a day, x4 stage 0's 6; the next proposal may go to 96), and only then the item "
          f"caps; (d) a timelock delay of at least {ddays * 24} h ({glob} a day for {ddays - 1} days), which needs the canister's assumed "
          f"delay (ASSUMED_DELAY_SECS, 72 h) raised with it: a code change")
if not bad and held <= room:
    print(f"ok   {what}: every item can leave within the timelock's 72 h less a day: each collection at {per} a day, and {held} items in all at {glob} a day ({room} can leave) (A4-L-1)")
PY
exitcheck() { # exitcheck <what> <json: items {key: n}, global_cap, per, global, names>
  local out; out=$(python3 -c "$EXIT_PY" "$1" "$2") || { echo "FAIL $1: the exit check could not be computed"; fail=1; return 0; }
  printf '%s\n' "$out"
  echo "$out" | grep -q '^FAIL' && fail=1
  return 0
}
# the launch file's custody and outflow limits, as the install argument will name them (nft-pilot.json: custodyItems, outflow)
launchexit() {
  [ -n "$LAUNCH" ] || return 0
  stagecheck
  launchrefs
  local j; j=$(python3 - "$LAUNCH" "$STAGE_SEL" "$BTCWRAP" <<'PY'
import json, sys
L = json.load(open(sys.argv[1])); o = L.get("outflow") or {}
st = int(sys.argv[2] or L.get("stage") or 1)
step = next((s for s in L.get("stages", []) if int(s.get("step", 0)) == st), None)
if step is not None: o = step.get("outflow") or {}
if not all(k in o for k in ("per_collection_items", "global_items")): print(""); sys.exit(0)
items = {f"{x.get('chainId', 1)}:{str(x['collection']).lower()}": int(step["custodyItems"].get(x.get("slug"), x.get("custodyItems")) if step else x["custodyItems"])
         for x in L.get("collections", []) if "custodyItems" in x}
names = {f"{x.get('chainId', 1)}:{str(x['collection']).lower()}": x.get("slug", "?") for x in L.get("collections", [])}
b = L.get("bitcoin") or None   # ordinals at launch: the Bitcoin wrapper's cap counts in the whole custody (A4-L-1)
if b and "custodyItems" in b:
    k = "btc:" + (sys.argv[3].lower() if len(sys.argv) > 3 and sys.argv[3] else b["slug"])
    items[k] = int(step["custodyItems"].get(b["slug"], b["custodyItems"]) if step else b["custodyItems"]); names[k] = b["slug"]
gcap = (step or {}).get("globalItems", L.get("custodyGlobalItems"))   # docs/v7/dormant-collections.md §6: a binding global
print(json.dumps({"items": items, "global_cap": gcap, "per": o["per_collection_items"], "global": o["global_items"], "names": names}))
PY
)
  if [ -z "$j" ]; then echo "FAIL the launch file $LAUNCH names no outflow limits (outflow.per_collection_items, outflow.global_items): the exit rule was not checked (A4-L-1)"; fail=1; return 0; fi
  exitcheck "the launch file's custody (custodyItems) at its outflow limits" "$j"
}
# review round 5 (the operator's option (c), A5-I-4): the launch file's staged plan. Step 1 is the file's own custodyItems and outflow;
# every step fits the exit rule; step 2 changes only outflow.global_items (to at least what step 3's items need, at most x4); step 3
# changes only the item counts (to J7's, the global the sum); no step lowers the outflow under what its items need
read -r -d '' STAGE_PY <<'PY' || true
import json, math, sys
L = json.load(open(sys.argv[1])); stages = sorted(L.get("stages") or [], key=lambda s: int(s.get("step", 0)))
if not stages: print("note the launch file names no staged plan (stages): only its own custodyItems and outflow are read"); sys.exit(0)
bad, ok = [], []
top = {x["slug"]: int(x["custodyItems"]) for x in L.get("collections", []) if "custodyItems" in x}
# ordinals at launch (the `bitcoin` block): the Bitcoin wrapper's items are in every step's custodyItems, the same at each (stage 0's)
btcb = L.get("bitcoin") or None
if btcb and "custodyItems" in btcb:
    top[btcb["slug"]] = int(btcb["custodyItems"])
    for s in stages:
        if s["custodyItems"].get(btcb["slug"]) != int(btcb["custodyItems"]):
            bad.append(f"step {s['step']} names {btcb['slug']} at {s['custodyItems'].get(btcb['slug'])} items, not the Bitcoin wrapper's {btcb['custodyItems']} (stage 0's, the same at every step)")
if [int(s["step"]) for s in stages] != list(range(1, len(stages) + 1)): bad.append("the steps are not 1, 2, 3, ... in order")
s1 = stages[0]
if s1["custodyItems"] != top: bad.append(f"step 1's item counts {s1['custodyItems']} are not the file's custodyItems {top}")
if s1["outflow"] != L.get("outflow"): bad.append("step 1's outflow is not the file's outflow")
# docs/v7/dormant-collections.md §6: a step that names its global (globalItems) holds at most that in all: the exit rule is on min(sum, global)
held = lambda s: min(sum(s["custodyItems"].values()), int(s["globalItems"])) if "globalItems" in s else sum(s["custodyItems"].values())
fits = lambda s: all(n <= int(s["outflow"]["per_collection_items"]) * 2 for n in s["custodyItems"].values()) and held(s) <= int(s["outflow"]["global_items"]) * 2
for s in stages:
    if not fits(s): bad.append(f"step {s['step']} ({s.get('name')}): {held(s)} items at {s['outflow']['per_collection_items']} / {s['outflow']['global_items']} a day do not fit the exit rule (A4-L-1)")
for a, b in zip(stages, stages[1:]):
    oc = {k for k in set(a["outflow"]) | set(b["outflow"]) if a["outflow"].get(k) != b["outflow"].get(k)}
    cc = a["custodyItems"] != b["custodyItems"]
    if int(b["step"]) == 2:
        if cc or oc != {"global_items"}: bad.append(f"step 2 must change outflow.global_items alone (it changes {sorted(oc)}{' and the item counts' if cc else ''})")
        g0, g1 = int(a["outflow"]["global_items"]), int(b["outflow"]["global_items"])
        if not (g0 < g1 <= g0 * 4): bad.append(f"step 2 raises global_items {g0} -> {g1}: a raise, at most x4 (outflow.rs)")
    elif int(b["step"]) == 3:
        if oc: bad.append(f"step 3 must change the item counts alone (it changes outflow {sorted(oc)})")
        if not cc: bad.append("step 3 changes no item count")
last = stages[-1]
need = math.ceil(held(last) / 2)
if len(stages) >= 2 and int(stages[1]["outflow"]["global_items"]) < need: bad.append(f"step 2's global_items {stages[1]['outflow']['global_items']} is under the {need} a day step {last['step']}'s {held(last)} items need")
for x in bad: print(f"FAIL the launch file's staged plan: {x}")
if not bad:
    print("ok   the launch file's staged plan (option (c)): " + "; ".join(f"step {s['step']} {s.get('name')}: {held(s)} items{' (the global binds)' if held(s) < sum(s['custodyItems'].values()) else ''} at {s['outflow']['per_collection_items']} / {s['outflow']['global_items']} a day" for s in stages)
          + ": each step fits the exit rule, step 2 changes global_items alone, step 3 the item counts alone"
          + (f" ({btcb['slug']}'s {btcb['custodyItems']} counted in each step, btc:0x<wrapper>)" if btcb else ""))
PY
# D8's references in the launch file (script/d8refs.py): every Ethereum collection names openseaSlug and coingeckoId, except one
# that carries priceRefs ["opensea"] with its _priceRefs note (the operator's decision of 2026-10-09: Identity MD, which CoinGecko does
# not list) and no coingeckoId. Anything else (a missing coingeckoId, priceRefs elsewhere or of another value, no note) FAILs
launchrefs() {
  local out; out=$(python3 script/d8refs.py launch "$LAUNCH" 2>&1)
  local alone; alone=$(printf '%s\n' "$out" | sed -n 's/^alone: //p')
  if printf '%s\n' "$out" | grep -q '^FAIL'; then
    printf '%s\n' "$out" | grep '^FAIL' | sed 's/^FAIL /FAIL the launch file'"'"'s D8 references: /'; fail=1
  elif printf '%s\n' "$out" | grep -q '^alone: '; then
    echo "ok   the launch file's D8 references: OpenSea and CoinGecko for every Ethereum collection$([ -n "$alone" ] && echo ", but OpenSea alone for $alone (priceRefs [\"opensea\"], the operator's decision of 2026-10-09, recorded in its _priceRefs)")"
  else echo "FAIL the launch file's D8 references could not be read ($(printf '%s' "$out" | tail -1))"; fail=1; fi
}
stagecheck() {
  local out; out=$(python3 -c "$STAGE_PY" "$LAUNCH") || { echo "FAIL the launch file's staged plan could not be read"; fail=1; return 0; }
  printf '%s\n' "$out"
  echo "$out" | grep -q '^FAIL' && fail=1
  return 0
}
# with --canister: the pair the canister actually holds (custody_caps() read above, custody_outflow()'s limits)
canexit() {
  [ -n "$CAPS" ] || return 0
  local dfx="${DFX:-$HOME/Library/Application Support/org.dfinity.dfx/bin/dfx}" LIM
  [ -x "$dfx" ] || dfx="$(command -v dfx || true)"
  LIM=$(DFX_WARNING=-mainnet_plaintext_identity "$dfx" canister call --network "$ICNET" --identity anonymous \
    --candid autobuyer/canister-rs/etch_autobuyer.did --output json --query "$CANISTER" custody_outflow 2>/dev/null) || LIM=""
  local j; j=$(printf '%s' "$LIM" | python3 -c '
import json, sys
r = json.load(sys.stdin); l = r[0] if isinstance(r, list) else r
n = lambda x: int(str(x).replace("_", ""))
c = json.loads(sys.argv[1])
print(json.dumps({"items": {k: v[0] for k, v in c["cols"].items()}, "global_cap": c["global"][0], "per": n(l["per_collection_items"]), "global": n(l["global_items"])}))' "$CAPS" 2>/dev/null) || j=""
  if [ -z "$j" ]; then
    if [ $PHASE1 = 1 ]; then echo "FAIL custody_outflow() of $CANISTER could not be read: the exit rule of the installed caps was not checked (A4-L-1)"; fail=1
    else echo "note custody_outflow() of $CANISTER was not read: the exit rule of the installed caps was not checked (A4-L-1)"; fi
    return 0
  fi
  exitcheck "the canister's custody_caps() at its custody_outflow() limits" "$j"
  # review round 5 (A5-I-1): the canister's whole custody_outflow() is the launch file's (its current step's), field by field
  if [ -n "$LAUNCH" ]; then
    local cmp; cmp=$(printf '%s' "$LIM" | python3 -c '
import json, sys
r = json.load(sys.stdin); l = r[0] if isinstance(r, list) else r
n = lambda x: int(str(x).replace("_", ""))
L = json.load(open(sys.argv[1])); st = int(sys.argv[2] or L.get("stage") or 1)
step = next((s for s in L.get("stages", []) if int(s.get("step", 0)) == st), None)
o = (step or {}).get("outflow") or L.get("outflow") or {}
for k in ("per_collection_items", "global_items", "per_collection_mints", "global_mints", "global_value_wei", "trip_factor"):
    print(k, o.get(k, ""), n(l[k]) if k in l else "")' "$LAUNCH" "$STAGE_SEL" 2>/dev/null)
    if [ -z "$cmp" ]; then echo "FAIL custody_outflow() of $CANISTER was not held against the launch file (A5-I-1)"; fail=1; return 0; fi
    while read -r k want got; do same "the canister's outflow limit $k = the launch file's (step ${STAGE_SEL:-$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("stage", 1))' "$LAUNCH")}, A5-I-1)" "$want" "$got"; done <<<"$cmp"
  fi
}
# installcheck: bayc10-decisions.md §5, §7 (round 3): the buyer's reinstall argument is published with every timelock proposal and
# the monitor's argument diff prints its provider URLs, so a keyed URL there (Dwellir's, say, in evm_chains[1].rpc_urls) is a
# public key. Each http(s):// in the Candid bytes is read as the text value it is (its LEB128 length just before it, 1 or 2 bytes,
# then exactly that many printable ASCII bytes) and must be keyless: no userinfo, query, fragment or ${…}, no path segment with
# 16+ key characters, and not a keyed provider's host. A keyed URL is printed as scheme://host/… only (its path and query hold
# the key). The argument: --install-arg, else the --monitor-config file's install_arg_hex when that is real Candid hex.
# (the check's Python, read here: bash 3.2 cannot parse a quoted heredoc holding apostrophes inside $(…))
read -r -d '' INSTALL_PY <<'PY' || true
import json, os, re, sys
from urllib.parse import urlsplit
arg, mon = sys.argv[1], sys.argv[2]
# review round 4 (A4-L-3): the argument decoded as the buyer's Init by the monitor's own IDL (monitor/src/candid.mjs), as JSON
# {label: [[chain id, [urls]], ...]}, so the canister's providers are counted by HOST per chain; empty when it could not be decoded
bychain = json.loads(sys.argv[3]) if len(sys.argv) > 3 and sys.argv[3] else {}
HEX = re.compile(r"^4449444c(?:[0-9a-fA-F]{2})*$")
KEYED = ("dwellir.com", "infura.io", "alchemy.com", "quiknode.pro", "blastapi.io", "chainstack.com", "getblock.io")
srcs, failed = [], False
if arg:
    v = open(arg).read() if os.path.isfile(arg) else arg
    v = "".join(v.split())
    if HEX.match(v): srcs.append(("--install-arg", v))
    else: print("FAIL --install-arg is neither Candid bytes as hex (4449444c…) nor a file holding them"); failed = True
if mon:
    try: mh = json.load(open(mon)).get("install_arg_hex")
    except Exception: mh = None
    mh = "".join(mh.split()) if isinstance(mh, str) else ""
    if HEX.match(mh) and all(mh.lower() != x.lower() for _, x in srcs): srcs.append(("the monitor's install_arg_hex", mh))
if not srcs: sys.exit(1 if failed else 3)
PHASE1 = os.environ.get("GOLIVE_PHASE1") == "1"
# review round 6 (B6-M-1): Ethereum (chain 1) providers MEASURED to serve the canister's 7-day sale window, read-only probes of
# 2026-09-28 (docs/audit/2026-09-27-nft/fixes-round6.md §1): host -> (paths, date measured, what). Add one only after measuring it the
# same way (--probe-depth does). ethereum-rpc.publicnode.com is NOT here (logs ~10,000 blocks deep, receipts null 20-30 k back), nor
# xrpc.cl/eth (an anonymous rate limit of 10: most of a 13-replica burst answers 429)
DEEP = {
    "rpc.mevblocker.io": (("", "/fast"), "2026-09-28", "logs and non-null receipts 130,000+ blocks deep, 13-way bursts ok, 10,000-result cap (-32005)"),
    "gateway.tenderly.co": (("/public/mainnet",), "2026-09-28", "logs and non-null receipts 130,000+ blocks deep, 13-way bursts ok, 20,000-result cap (-32602, data 'Query returned more than 20000 results'); IPv4 only"),
}
PROBE = os.environ.get("GOLIVE_PROBE_DEPTH") == "1"
# review round 7 (D7-L-1), round 8 (B8-L-1): the probe (a FIXED range known to hold 10 logs, and its receipt) lives in script/depth_probe.py,
# shared with the daily workflow (.github/workflows/depth-probe.yml) and the operator's own runs, so the three cannot drift
sys.path.insert(0, os.path.join(os.getcwd(), "script"))
from depth_probe import DEPTH_PROBE_BLOCKS, PIN_FROM, PIN_TO, PIN_LOGS, PIN_TX, PIN_TX_BLOCK, PIN_REPIN_BLOCKS, MILADY, TRANSFER, depth_probe
def texts(b):
    low, out, unread, end = b.lower(), [], [], 0
    for s in sorted({x.start() for x in re.finditer(rb"https?://", low)}):
        if s < end: continue                                   # inside a URL already read
        got = None
        for w in (2, 1):                                       # a two-byte length first: a one-byte read of it would cut the URL short
            if s < w: continue
            lb = b[s - w:s]
            if w == 2 and not (lb[0] & 0x80 and 0 < lb[1] < 0x80): continue
            if w == 1 and lb[0] & 0x80: continue
            n = lb[0] if w == 1 else (lb[0] & 0x7F) | (lb[1] << 7)
            t = b[s:s + n]
            if n >= 8 and len(t) == n and all(0x20 <= c <= 0x7E for c in t): got = t.decode("ascii"); break
        if got is None: unread.append(s); continue
        out.append(got); end = s + len(got)
    return out, unread
def keyed(u):
    why = []
    try: p = urlsplit(u); host = (p.hostname or "").lower()
    except ValueError: return ["it does not parse as a URL"], "?", "?"
    if "@" in p.netloc: why.append("userinfo")
    if p.query or "?" in u: why.append("a query string")
    if p.fragment or "#" in u: why.append("a fragment")
    if "${" in u: why.append("a ${...} variable")
    if any(re.search(r"[A-Za-z0-9_-]{16,}", seg) for seg in p.path.split("/")): why.append("a path segment of 16+ key characters")
    # review round 4 (A4-L-3): a shorter key, or one split by dots or slashes, percent-encoded or as a parameter, or in a subdomain
    elif any(re.search(r"[A-Za-z0-9]{12,}", seg) for seg in re.split(r"[/.]", p.path)): why.append("a path part of 12+ key characters")
    if "%" in p.path or ";" in p.path or "=" in p.path: why.append("an encoded or parameterised path")
    if any(len(re.sub(r"[^A-Za-z0-9]", "", seg)) >= 16 and len(re.sub(r"[^0-9]", "", seg)) >= 3 for seg in p.path.split("/")): why.append("a key split by separators in a path segment")
    if any(re.fullmatch(r"[A-Za-z0-9]{12,}", lab) or re.search(r"[0-9]", lab) and len(lab) >= 12 for lab in host.split(".")[:-2]): why.append("a subdomain label that looks like a key")
    if any(host == k or host.endswith("." + k) for k in KEYED) or "quicknode" in host: why.append("a keyed provider's host")
    if host == "lb.drpc.org" and ("dkey" in u.lower() or len([x for x in p.path.split("/") if x]) >= 2): why.append("drpc's keyed load balancer (a dkey)")
    return why, p.scheme or "?", host or "?"
for label, h in srcs:
    urls, unread = texts(bytes.fromhex(h))
    for s in unread:
        print(f"FAIL the install argument ({label}) has an http(s):// at byte {s} that is not a whole Candid text (no LEB128 length and printable ASCII): it cannot be checked for a key"); failed = True
    bad = 0
    for u in dict.fromkeys(urls):
        why, sch, host = keyed(u)
        # round 4 (A4-L-3): a key in a subdomain is never printed: only the registrable domain
        if "a subdomain label that looks like a key" in why: host = "…." + ".".join(host.split(".")[-2:])
        if why:
            bad += 1; failed = True
            print(f"FAIL the install argument ({label}) names a KEYED provider URL {sch}://{host}/… ({', '.join(why)}): the argument is published with every timelock proposal and the monitor's argument diff prints provider URLs; the canister's providers must stay keyless (bayc10-decisions.md §5)")
    n = len(dict.fromkeys(urls))
    if not n and not unread: print(f"note the install argument ({label}) names no provider URL")
    elif not bad and not unread:
        # round 4 (A4-L-3): only the registrable domain is printed (a key in a subdomain is refused above, and never printed)
        reg = lambda u: ".".join((urlsplit(u).hostname or "?").split(".")[-2:])
        hosts = ", ".join(dict.fromkeys(reg(u) for u in urls))
        print(f"ok   the install argument ({label}) names {n} provider URLs, all keyless (the argument is public with every timelock proposal): {hosts}")
    # round 4 (A4-L-3): each chain's providers are two HOSTS: xrpc.cl/eth and xrpc.cl/eth/ are one provider twice, and the canister's
    # agreement of two (receipts, finality, the sale scan, the delivery record) would be one provider's word
    chains = bychain.get(label)
    if chains is None:
        if PHASE1: print(f"FAIL the install argument ({label}) could not be decoded as the buyer's Init (monitor/node_modules?): its providers were not counted by host per chain (A4-L-3)"); failed = True
        else: print(f"note the install argument ({label}) was not decoded as the buyer's Init: its providers were not counted by host per chain")
    else:
        for cid, us in chains:
            hs = {(urlsplit(u).hostname or "").lower() for u in us} - {""}
            if len(hs) < 2:
                print(f"FAIL the install argument ({label}) names {len(hs)} provider host(s) for chain {cid} ({', '.join(sorted(hs)) or 'none'}): two distinct hosts, or their agreement is one provider's word (A4-L-3)"); failed = True
        if chains and all(len({(urlsplit(u).hostname or "").lower() for u in us} - {""}) >= 2 for _, us in chains):
            print(f"ok   the install argument ({label}) names two provider hosts or more for each of its {len(chains)} chain(s) (A4-L-3)")
        # review round 6 (B6-M-1): the canister's Ethereum providers must each serve eth_getLogs AND receipts (not null) over the whole
        # 7-day sale window (50,400 blocks) with a margin: a shallow one (publicnode's keyless endpoint keeps logs ~10,000 blocks, and
        # answers null for receipts 20-30 k back) freezes the canister's sale scan for days after any gap. A provider passes when it is on
        # the list below (measured) or when --probe-depth measured it now; a failed probe FAILs even a listed one
        for cid, us in chains:
            if str(cid) != "1": continue
            for u in dict.fromkeys(us):
                try: pu = urlsplit(u); host = (pu.hostname or "").lower(); path = pu.path.rstrip("/")
                except ValueError: host, path = "?", ""
                # never print a label that looks like a key (the keyless rule's own redaction, A4-L-3)
                keyish = any(re.fullmatch(r"[A-Za-z0-9]{12,}", lab) or re.search(r"[0-9]", lab) and len(lab) >= 12 for lab in host.split(".")[:-2])
                shown = "?" if host == "?" else f"{pu.scheme}://" + ("…." + ".".join(host.split(".")[-2:]) if keyish else host)
                listed = DEEP.get(host)
                # review round 7 (A7-I-3): the measured entry is https on the default port; http:// or another port is not it (the IC's
                # outcalls are https only: such a pair would fail every Ethereum read after install)
                try: port = pu.port
                except ValueError: port = -1
                on = bool(listed) and path in listed[0] and pu.scheme == "https" and port in (None, 443)
                pr = depth_probe(u) if PROBE else None
                if pr is not None and not pr.startswith("ok"):
                    print(f"FAIL the install argument ({label}): the canister's Ethereum provider {shown} failed the depth probe ({pr}): it does not serve eth_getLogs and receipts over the 7-day window (review round 6, B6-M-1)"); failed = True
                elif pr is not None:
                    print(f"ok   the install argument ({label}): the canister's Ethereum provider {shown} passed the depth probe now (the pinned range's {PIN_LOGS} Milady Transfer logs, blocks {PIN_FROM:,}..{PIN_TO:,}, and its measured receipt; B6-M-1, D7-L-1){'' if on else ' (not on the measured list)'}{pr[2:]}")
                elif on:
                    print(f"ok   the install argument ({label}): the canister's Ethereum provider {shown} is on the measured list (measured {listed[1]}: {listed[2]}; B6-M-1). --probe-depth measures it again now")
                elif PHASE1:
                    print(f"FAIL the install argument ({label}): the canister's Ethereum provider {shown} is not known to serve eth_getLogs and receipts over the 7-day window: probe it with --probe-depth or add it to the allowlist after measuring (review round 6, B6-M-1)"); failed = True
                else:
                    print(f"note the install argument ({label}): the canister's Ethereum provider {shown} is not known to serve eth_getLogs and receipts over the 7-day window (not on the measured list; --probe-depth measures it; a FAIL under --phase1, B6-M-1)")
sys.exit(1 if failed else 0)
PY
# the argument decoded as the buyer's Init (monitor/src/candid.mjs decodeInit), grouped by chain: {label: [[id, [urls]], ...]}
installchains() {
  local a="$INSTALLARG" m="" v
  [ -n "$MONCFG" ] && m=$(python3 -c 'import json,sys; v=json.load(open(sys.argv[1])).get("install_arg_hex"); print("".join(v.split()) if isinstance(v, str) else "")' "$MONCFG" 2>/dev/null)
  [ -f "$a" ] && a=$(tr -d ' \n\t' < "$a")
  ( cd monitor 2>/dev/null && node --input-type=module - "$a" "$m" 2>/dev/null ) <<'JS' || true
const { decodeInit } = await import(process.cwd() + "/src/candid.mjs");
const [a, m] = process.argv.slice(-2);
const out = {};
const group = (hex) => {
  const d = decodeInit(hex.replace(/^0x/, ""));
  if (!d.init) return null;
  const i = d.init;
  const home = [[String(i.chain_id), i.rpc_urls]];
  const evm = (i.evm_chains?.[0] ?? []).map((c) => [String(c.chain_id), c.rpc_urls]);
  return home.concat(evm);
};
if (a) { const g = group(a); if (g) out["--install-arg"] = g; }
if (m && m.toLowerCase() !== a.toLowerCase()) { const g = group(m); if (g) out["the monitor's install_arg_hex"] = g; }
process.stdout.write(JSON.stringify(out));
JS
}
installcheck() {
  INSTALLSEEN=1
  local out rc
  out=$(GOLIVE_PHASE1=$PHASE1 GOLIVE_PROBE_DEPTH=$PROBE_DEPTH python3 -c "$INSTALL_PY" "$INSTALLARG" "$MONCFG" "$(installchains)"); rc=$?
  [ -z "$out" ] || printf '%s\n' "$out"
  case "$rc" in
    0) ;;
    3) if [ $PHASE1 = 1 ]; then echo "FAIL --phase1 on an NFT stack with no install argument: pass --install-arg <the reinstall's argument>: its provider URLs are public and must be keyless"; fail=1
       else echo "note no install argument (--install-arg, or a real install_arg_hex in --monitor-config): its provider URLs were not checked for keys"; fi;;
    *) fail=1;;
  esac
}
# targetcheck <slug> <factory> <module> <stack file> <collection path>: docs/v7/phase1-nft-pooled.md §2 — the collection's own
# graduation targets and module conversion / reserve, as the stack file names them, and what one graduation buys at today's floor
targetcheck() {
  local SLUG="$1" F="$2" M="$3" W="$4" c="$5" k v nr gp rb
  if [ -z "$(jb "$W" "$c.targets.normalRaise")" ]; then phase1only "$SLUG: the stack file names no targets (per-collection graduation targets)"; return 0; fi
  for k in normalRaise earlyRaise earlyUntilEth maxRaise; do same "$SLUG factory.$k = the stack's" "$(jb "$W" "$c.targets.$k")" "$(call "$F" "$k()(uint256)")"; done
  for k in convBps reserveBps; do same "$SLUG module.$k = the stack's" "$(jb "$W" "$c.moduleParams.$k")" "$(call "$M" "$k()(uint256)")"; done
  nr=$(call "$F" 'normalRaise()(uint256)'); gp=$(tcall "$M" 'gradPrice()(uint256)'); rb=$(call "$M" 'reserveBps()(uint256)')
  if echo "${gp:-x}" | grep -Eq '^[1-9][0-9]*$'; then
    local bud; bud=$(call "$F" 'itemBudgetFor(uint256)(uint256)' "$nr")
    python3 -c "
import sys
b, gp, rb = int(sys.argv[1]), int(sys.argv[2]), int(sys.argv[3])
conv = b * (10_000 - rb) / 10_000 / gp
need = 1.05
print(f'     {sys.argv[4]}: one graduation at the normal target converts {conv:.2f} items at today\'s graduation price' + ('' if conv >= need else f' — POOLED: {int(-(-need // conv))} graduations per item (docs/v7/phase1-nft-pooled.md)'))" "$bud" "$gp" "$rb" "$SLUG"
  else echo "note $SLUG: no graduation price yet (no floor posted): items per graduation not computed"; fi
}
STACKSEEN=0
# C-08: the module's ETH caps and rates are the frog defaults (script/StackChecks.sol defaultParams) × the item unit. Integration
# R3 (docs/audit/2026-09-26/review2/integration-R3.md §5): the stack file names what used to be assumed, as DeployStack read it
# from the chain at deploy: top-level `payee` (ETCH_PAYEE, else the custodian), and per collection `itemUnits` (the wrapper's
# ONE_ITEM()) and `listingPaused` (registry.isPaused(listingId)). Under --phase1 a stack file without them FAILS; before it, the
# old assumptions (payee = custodian, a unit = 1e18, not paused) stand with a note.
# jb <file> <path>: a JSON value as golive-check compares it (booleans as true/false, missing as nothing)
jb() { python3 -c "
import json,sys
v=json.load(open(sys.argv[1]))
try:
  for k in sys.argv[2].split('.'): v = v[int(k)] if isinstance(v, list) else v[k]
except (KeyError, IndexError): v = None
print('' if v is None else json.dumps(v).strip('\"'))" "$1" "$2"; }
stackcheck() {
  local W="$1" T DIR OWN KEEP CUST STAGE MINTS ITEMS MADE FILECEIL PAYEE n i
  STACKSEEN=1
  same "stack file is this chain's" "$(j "$W" chainId)" "$HEX"
  T=$(j "$W" treasury); DIR=$(j "$W" directory); OWN=$(j "$W" owner); KEEP=$(j "$W" keeper); CUST=$(j "$W" custodian); MADE=$(j "$W" ownerStepsMade)
  STAGE=$(jp "$W" caps.stage); MINTS=$(jp "$W" caps.mintsPerDayPerCollection); ITEMS=$(jp "$W" caps.itemsPerDayPerCollection)
  FILECEIL=$(jp "$W" capCeiling 2>/dev/null || echo "")
  # G-N3: a stack file with no capCeiling cannot hold the vaults' ceilings to anything
  { [ -n "$FILECEIL" ] && [ "$FILECEIL" != "0" ]; } || phase1only "the stack file names no capCeiling: the vaults' cap ceilings cannot be compared with it"
  PAYEE=$(jb "$W" payee)
  if [ -z "$PAYEE" ]; then
    phase1only "the stack file names no payee (DeployStack writes it: ETCH_PAYEE, else the custodian): the module's and the desk's payee are held to the custodian instead"
    PAYEE="$CUST"
  else echo "     payee (the stack file's): $PAYEE$([ "$(echo "$PAYEE" | lc)" = "$(echo "$CUST" | lc)" ] && echo " (the custodian)")"; fi
  for pair in "treasury=$T" "directory=$DIR" "pool manager=$(j "$W" poolManager)"; do
    name="${pair%%=*}"; addr="${pair#*=}"; code "$addr" && echo "ok   $name has code ($addr)" || { echo "FAIL $name $addr has no code"; fail=1; }
  done
  same "stack.registry is the registry"        "$R"   "$(j "$W" registry)"
  same "stack.owner = registry.owner"          "$(call "$R" 'owner()(address)')"  "$OWN"
  same "stack.keeper = registry.keeper (named)" "$(call "$R" 'namedKeeper()(address)')" "$KEEP"
  local SG; SG=$(j "$W" guardian 2>/dev/null || echo "")
  # the guardian the stack named is the one set (when one is set at all: none is the no-guardian rule's, above)
  local CG; CG=$(call "$R" 'guardian()(address)')
  if [ -n "$SG" ] && [ "$(echo "$SG" | lc)" != "$ZERO" ] && [ "$(echo "$CG" | lc)" != "$ZERO" ]; then same "registry.guardian = the stack's ETCH_GUARDIAN" "$SG" "$CG"; fi
  # C-07: Phase 1's treasury split is the constructor's (9000/0/0/1000, no bootstrap loan); stage 1 restores the old one by setShares
  local SH; SH="$(call "$T" 'reservesBps()(uint16)')/$(call "$T" 'floatsBps()(uint16)')/$(call "$T" 'bootstrapBps()(uint16)')/$(call "$T" 'opsBps()(uint16)') loan $(call "$T" 'bootstrapLoan()(uint256)')"
  if [ "$STAGE" = "0" ]; then same "treasury shares (reserves/floats/bootstrap/ops) and bootstrap loan at stage 0" "9000/0/0/1000 loan 0" "$SH"
  else echo "     treasury shares (reserves/floats/bootstrap/ops) and bootstrap loan at stage $STAGE: $SH"; fi
  same "treasury.registry"                     "$R"   "$(call "$T" 'registry()(address)')"
  same "directory.admin = owner"               "$OWN" "$(call "$DIR" 'admin()(address)')"
  echo "     stage $STAGE: $MINTS mints a day a collection, $ITEMS item arrivals a day at the module, $(jp "$W" caps.custodyItemsPerCollection) items and $(jp "$W" caps.custodyWeiPerCollection) wei in custody a collection (launch-gates G7)"
  readcaps
  # the custody fee pool is Bitcoin custody's (audit D, H-4): an NFT-only stack (docs/v7/phase1-nft.md) has no Bitcoin custody
  if [ "$(python3 -c 'import json,sys; print(any(c.get("kind","inscriptions")=="inscriptions" for c in json.load(open(sys.argv[1]))["collections"]))' "$W")" = "True" ]; then
    feepoolcheck
  else echo "note no Bitcoin collection in this stack: the custody fee pool is not required"; fi
  local OPS; OPS=$(j "$W" ops)
  local COPS; COPS=$(call "$T" 'ops()(address)')
  if [ "$(echo "$OPS" | lc)" != "$ZERO" ]; then
    if [ "$(echo "$COPS" | lc)" = "$ZERO" ]; then strict "treasury.ops is unset: the owner's setOps($OPS) is still to be made (go-live step 7)"; else same "treasury.ops" "$OPS" "$COPS"; fi
    nocode "role ops" "$OPS"; notowner "role ops" "$OPS"
  else phase1only "treasury.ops is unset: the operations share joins the reserves pot (the Safe's setOps, go-live step 7)"; fi
  nocode "role custodian (the canister's threshold-key address)" "$CUST"; notowner "role custodian" "$CUST"
  [ "$(echo "$PAYEE" | lc)" = "$(echo "$CUST" | lc)" ] || nocode "payee" "$PAYEE"
  n=$(jn "$W" collections)
  NFTCOLS=""; ETHCOLS=""
  for ((i = 0; i < n; i++)); do
    local c="collections.$i" SLUG KIND A M L F RT DK RV BV YV OB LID WC WIRED
    SLUG=$(jp "$W" "$c.slug"); KIND=$(jp "$W" "$c.kind"); A=$(jp "$W" "$c.wrapper"); M=$(jp "$W" "$c.floorModule"); L=$(jp "$W" "$c.lock"); F=$(jp "$W" "$c.factory")
    WC=$(jp "$W" "$c.wrapClaim")
    RT=$(jp "$W" "$c.router"); DK=$(jp "$W" "$c.mintDesk"); RV=$(jp "$W" "$c.raffleVault"); BV=$(jp "$W" "$c.burnVault"); YV=$(jp "$W" "$c.yoyoVault"); OB=$(jp "$W" "$c.offerBoard"); LID=$(jp "$W" "$c.listingId")
    echo "     collection $SLUG ($KIND)"
    # the item unit: the stack file's (the wrapper's ONE_ITEM() at deploy), held to the wrapper now
    local UNIT; UNIT=$(jb "$W" "$c.itemUnits")
    if [ -z "$UNIT" ]; then
      phase1only "$SLUG: the stack file names no itemUnits (the wrapper's ONE_ITEM()): the module's caps and rates are compared at 1e18 a unit"
      UNIT=1000000000000000000
    fi
    same "$SLUG wrapper.ONE_ITEM() = the stack's itemUnits" "$UNIT" "$(call "$A" 'ONE_ITEM()(uint256)')"
    for pair in "wrapper=$A" "floor module=$M" "lock=$L" "factory=$F" "router=$RT" "mint desk=$DK" "raffle vault=$RV" "burn vault=$BV" "yoyo vault=$YV"; do
      name="${pair%%=*}"; addr="${pair#*=}"; code "$addr" && echo "ok   $SLUG $name has code ($addr)" || { echo "FAIL $SLUG $name $addr has no code"; fail=1; }
    done
    same "$SLUG queue is the floor module"       "$M" "$(jp "$W" "$c.queue")"
    if [ "$KIND" = "inscriptions" ]; then
      # docs/v6/deploy-stack.md §1: WrapClaim built the wrapper and is its deployer; the two point at each other
      code "$WC" && echo "ok   $SLUG wrap claim has code ($WC)" || { echo "FAIL $SLUG wrap claim $WC has no code"; fail=1; }
      same "$SLUG wrapClaim.wrapper is the wrapper"  "$A"  "$(call "$WC" 'wrapper()(address)')"
      same "$SLUG wrapper.distribution is the claim" "$WC" "$(call "$A" 'distribution()(address)')"
      same "$SLUG wrapper.deployer is the claim"     "$WC" "$(call "$A" 'deployer()(address)')"
      same "$SLUG wrapClaim.registry"  "$R"   "$(call "$WC" 'registry()(address)')"
      same "$SLUG wrapClaim.listingId" "$LID" "$(call "$WC" 'listingId()(uint256)')"
      same "$SLUG wrapper.registry"    "$R" "$(call "$A" 'registry()(address)')"
      same "$SLUG wrapper.distributor" "$D" "$(call "$A" 'distributor()(address)')"
      same "$SLUG wrapper.beacon"      "$B" "$(call "$A" 'beacon()(address)')"
      same "$SLUG wrapper.listingId"   "$LID" "$(call "$A" 'listingId()(uint256)')"
      local WCUST LBTC
      WCUST=$(cast call --rpc-url "$RPC" "$A" 'custodyAddress()(string)' | tr -d '"')
      same "$SLUG wrapper.custodyAddress" "$(jp "$W" "$c.custody.address")" "$WCUST"
      # K9: the listing delivers purchases to its btcVault, and wrap refuses while the two differ (as the wrapper-file mode checks)
      LBTC=$(cast call --rpc-url "$RPC" "$R" 'listing(uint256)((uint8,bool,bool,uint8,address,bytes32,string,string,string))' "$LID" \
        | python3 -c 'import sys,re; print((re.findall(r"\"([^\"]*)\"", sys.stdin.read()) or [""])[-1])')
      same "$SLUG listing $LID btcVault is the wrapper's custody address (or wrap refuses)" "$WCUST" "$LBTC"
    else
      same "$SLUG nft wrapper.beacon"     "$B" "$(call "$A" 'beacon()(address)')"
      same "$SLUG nft wrapper.chainId"    "$(jp "$W" "$c.custody.chainId")" "$(call "$A" 'chainId()(uint64)')"
      same "$SLUG nft wrapper.collection" "$(jp "$W" "$c.custody.collection")" "$(call "$A" 'collection()(address)')"
      # docs/v7/phase1-nft.md §3.1: the EthNftWrapper's listing pause and clock
      same "$SLUG nft wrapper.registry"   "$R" "$(call "$A" 'registry()(address)')"
      same "$SLUG nft wrapper.listingId"  "$LID" "$(call "$A" 'listingId()(uint256)')"
      [ "$(call "$A" 'pendingMintLimit()(uint256)')" = "0" ] && echo "ok   $SLUG no pending mint-limit raise" || { echo "FAIL $SLUG wrapper.pendingMintLimit() is set: a mint-limit raise is pending"; fail=1; }
      code "$(jp "$W" "$c.custody.collection")" && { echo "FAIL $SLUG: the collection $(jp "$W" "$c.custody.collection") has code on THIS chain: it must be the other chain's address"; fail=1; } \
        || echo "ok   $SLUG collection is another chain's (no code here)"
    fi
    same "$SLUG wrapper.custodian"          "$CUST" "$(call "$A" 'custodian()(address)')"
    WIRED=$(call "$A" 'floorModule()(address)')
    # B-05: the module names this wrapper and this registry, whatever the wiring
    same "$SLUG module.wrapped() is the wrapper (B-05)" "$A" "$(call "$M" 'wrapped()(address)')"
    if [ "$KIND" = "nft" ]; then
      # docs/v7/phase1-nft.md §3.2: the canister posts this module's floor through a PosterSource over the registry's pause
      local PS; PS=$(jb "$W" "$c.posterSource")
      if [ -z "$PS" ]; then echo "FAIL $SLUG: the stack file names no posterSource"; fail=1; PS="$ZERO"; fi
      same "$SLUG module.registry() is the poster source (the canister's floor role)" "$PS" "$(call "$M" 'registry()(address)')"
      same "$SLUG posterSource.registry is the registry (its pause zeroes the poster)" "$R" "$(call "$PS" 'registry()(address)')"
      same "$SLUG posterSource.poster is the custodian (the canister)" "$CUST" "$(call "$PS" 'poster()(address)')"
      same "$SLUG stack floorPoster is the custodian" "$CUST" "$(jb "$W" "$c.floorPoster")"
      # audit 2026-09-27-nft D-M-1: this collection's own floor switch is not set at go-live
      same "$SLUG posterSource.paused() is false (this collection's floor switch)" "false" "$(call "$PS" 'paused()(bool)')"
      # D-L-6: the stack's custodian (poster, minter, NFT holder) is the canister's own EVM address
      # round 2 (D2-L-5): at go-live this is a FAIL, not a WARN: a custodian that is not the canister's key can mint and mintDeposit
      if [ -n "${CANISTER_EVM:-}" ]; then same "$SLUG custodian is the canister's EVM address (--canister-evm)" "$CANISTER_EVM" "$CUST"
      elif [ $PHASE1 = 1 ] && [ $REHEARSAL = 1 ]; then echo "WARN $SLUG: the stack's custodian $CUST is not tied to the canister's key (GOLIVE_REHEARSAL: a WARN here only)"
      elif [ $PHASE1 = 1 ]; then echo "FAIL $SLUG: --canister-evm is required under --phase1: the stack's custodian $CUST is not tied to the canister's key (pass --canister-evm <the canister's evm_address()>)"; fail=1; fi
    else
      same "$SLUG module.registry() is the registry (the keeper indirection)" "$R" "$(call "$M" 'registry()(address)')"
    fi
    if [ "$KIND" = "inscriptions" ]; then
      [ "$(call "$A" 'pendingMintLimit()(uint256)')" = "0" ] && echo "ok   $SLUG no pending mint-limit raise" || { echo "FAIL $SLUG wrapper.pendingMintLimit() is set: a mint-limit raise is pending"; fail=1; }
    fi
    if [ "$KIND" = "inscriptions" ] && [ "$(echo "$WIRED" | lc)" = "$ZERO" ]; then
      strict "$SLUG: the wrapper is not wired: the owner's WrapClaim.initMintLimit($MINTS) and WrapClaim.wireFloorModule(module) are still to be made, BEFORE the first mint (docs/v6/deploy-stack.md §6, step 0; B-05: every graduation reverts until it is)"
    else
      same "$SLUG wrapper.floorModule"        "$M"    "$WIRED"
      same "$SLUG wrapper.maxMintsPerDay = the stage's" "$MINTS" "$(call "$A" 'maxMintsPerDay()(uint256)')"
    fi
    same "$SLUG wrapper.symbol"             "$(jp "$W" "$c.symbol")" "$(cast call --rpc-url "$RPC" "$A" 'symbol()(string)' | tr -d '"')"
    same "$SLUG desk.wrapper"               "$A" "$(call "$DK" 'wrapper()(address)')"
    same "$SLUG module.wrapper"             "$A" "$(call "$M" 'wrapper()(address)')"
    if [ "$KIND" = "nft" ]; then same "$SLUG module.keeper = the canister (the poster; zero while the keeper is paused)" "$CUST" "$(call "$M" 'keeper()(address)')"
    else same "$SLUG module.keeper = registry.keeper" "$KEEP" "$(call "$M" 'keeper()(address)')"; fi
    same "$SLUG module.platform = treasury" "$T" "$(call "$M" 'platform()(address)')"
    same "$SLUG module.treasury"            "$T" "$(call "$M" 'treasury()(address)')"
    same "$SLUG module.factory"             "$F" "$(call "$M" 'factory()(address)')"
    same "$SLUG module.seeder = lock"       "$L" "$(call "$M" 'seeder()(address)')"
    same "$SLUG module.maxItemsPerDay = the stage's" "$ITEMS" "$(call "$M" 'maxItemsPerDay()(uint256)')"
    # C-08: who is paid, and how much ETH-backed exposure and churn the module allows (the Trust page's "≈1.3 ETH/day" rests on
    # maxPublicEthBacked); payees are the canister's address (ETCH_PAYEE, default the custodian)
    same "$SLUG module.payee = the stack's payee" "$PAYEE" "$(call "$M" 'payee()(address)')"
    same "$SLUG desk.payee = the stack's payee"   "$PAYEE" "$(call "$DK" 'payee()(address)')"
    local pv pn pk pw
    for pv in "mintPerHour=5" "burnPerHour=5" "maxPublicEthBacked=10" "maxEthBacked=40"; do
      pn="${pv%%=*}"; pk="${pv#*=}"
      pw=$(python3 -c "import sys; print(int(sys.argv[1]) * int(sys.argv[2]))" "$UNIT" "$pk")
      local got; got=$(call "$M" "$pn()(uint256)")
      if [ -z "$got" ]; then echo "FAIL $SLUG module.$pn() is unreadable"; fail=1
      elif [ "$got" = "$pw" ]; then echo "ok   $SLUG module.$pn = $pk units ($got)"
      else phase1only "$SLUG module.$pn is $got, not the Phase-1 default $pk units ($pw at an item unit of $UNIT): the Trust page's figures assume the default"; fi
    done
    # the listing's pause: the stack file's `listingPaused` (registry.isPaused at deploy), not an assumption
    local LP WANTP; LP=$(call "$R" 'isPaused(uint256)(bool)' "$LID"); WANTP=$(jb "$W" "$c.listingPaused")
    if [ -z "$WANTP" ]; then phase1only "$SLUG: the stack file names no listingPaused: the listing is expected unpaused"; WANTP=false; fi
    if [ -z "$LP" ]; then echo "FAIL $SLUG listing $LID: isPaused() is unreadable"; fail=1
    elif [ "$LP" = "$WANTP" ]; then echo "ok   $SLUG listing $LID paused = $LP (the stack file's listingPaused)"
    else echo "FAIL $SLUG listing $LID is $([ "$LP" = true ] && echo PAUSED || echo "not paused") (isPaused $LP; the stack file's listingPaused is $WANTP)"; fail=1; fi
    same "$SLUG lock.wrapped"               "$A" "$(call "$L" 'wrapped()(address)')"
    same "$SLUG lock.poolManager"           "$(j "$W" poolManager)" "$(call "$L" 'poolManager()(address)')"
    same "$SLUG lock.factory"               "$F" "$(call "$L" 'factory()(address)')"
    same "$SLUG lock.treasury"              "$T" "$(call "$L" 'treasury()(address)')"
    same "$SLUG factory.wrapped"            "$A" "$(call "$F" 'wrapped()(address)')"
    same "$SLUG factory.seeder = lock"      "$L" "$(call "$F" 'seeder()(address)')"
    same "$SLUG factory.queue = module"     "$M" "$(call "$F" 'queue()(address)')"
    same "$SLUG factory.platform = treasury" "$T" "$(call "$F" 'platform()(address)')"
    same "$SLUG factory.raffleVault"        "$RV" "$(call "$F" 'raffleVault()(address)')"
    same "$SLUG factory.burnVault"          "$BV" "$(call "$F" 'burnVault()(address)')"
    same "$SLUG factory.yoyoVault"          "$YV" "$(call "$F" 'yoyoVault()(address)')"
    same "$SLUG router.lock"                "$L" "$(call "$RT" 'lock()(address)')"
    same "$SLUG router.module"              "$M" "$(call "$RT" 'module()(address)')"
    same "$SLUG router.poolManager"         "$(j "$W" poolManager)" "$(call "$RT" 'poolManager()(address)')"
    local k=0
    for V in "$RV" "$BV" "$YV"; do
      same "$SLUG vault $k policy"          "$k"   "$(call "$V" 'policy()(uint8)')"
      same "$SLUG vault $k registry"        "$R"   "$(call "$V" 'registry()(address)')"
      same "$SLUG vault $k listingId"       "$LID" "$(call "$V" 'listingId()(uint256)')"
      same "$SLUG vault $k wrapper"         "$A"   "$(call "$V" 'wrapper()(address)')"
      same "$SLUG vault $k venue = router"  "$RT"  "$(call "$V" 'venue()(address)')"
      same "$SLUG vault $k launcher = factory" "$F" "$(call "$V" 'launcher()(address)')"
      same "$SLUG vault $k desk"            "$DK"  "$(call "$V" 'desk()(address)')"
      local CC; CC=$(call "$V" 'capCeiling()(uint256)')
      if [ -z "$CC" ]; then echo "FAIL $SLUG vault $k capCeiling() is unreadable"; fail=1
      elif [ "$CC" != "0" ]; then
        if [ -n "$FILECEIL" ] && [ "$FILECEIL" != "0" ]; then same "$SLUG vault $k capCeiling = the stack's" "$FILECEIL" "$CC"; else phase1only "$SLUG vault $k has a cap ceiling ($CC wei) the stack file does not name"; fi
      else strict "$SLUG vault $k has no cap ceiling (C-01: every postCap reverts NoCeiling, so no sweep; the Safe's proposeCapCeiling applies at once)"; fi
      [ "$(call "$V" 'pendingCeiling()(uint256)')" = "0" ] && echo "ok   $SLUG vault $k no pending ceiling raise" || { echo "FAIL $SLUG vault $k pendingCeiling() is set: a cap-ceiling RAISE is pending"; fail=1; }
      k=$((k + 1))
    done
    if [ -n "$OB" ]; then
      code "$OB" && echo "ok   $SLUG offer board has code ($OB)" || { echo "FAIL $SLUG offer board $OB has no code"; fail=1; }
      same "$SLUG board.wrapper"            "$A"    "$(call "$OB" 'wrapper()(address)')"
      same "$SLUG board.custodian = wrapper.custodian" "$CUST" "$(call "$OB" 'custodian()(address)')"
      echo "     $SLUG board: bond $(tcall "$OB" 'bond()(uint256)') reward $(tcall "$OB" 'reward()(uint256)') day cap $(tcall "$OB" 'rewardDayCap()(uint256)') wei"
    else
      [ "$KIND" = "nft" ] && echo "note $SLUG has no offer board (hand-ins are Bitcoin-only)" || { echo "FAIL $SLUG names no offer board"; fail=1; }
    fi
    capcheck "$SLUG" "$KIND" "$A" "$W" "$c"
    [ "$KIND" = "inscriptions" ] && btclaunch "$SLUG" "$A" "$M" "$LID" "$(jp "$W" "$c.custody.address")" "$W"
    if [ "$KIND" = "nft" ]; then
      nftlaunch "$SLUG" "$(jp "$W" "$c.custody.collection")"
      targetcheck "$SLUG" "$F" "$M" "$W" "$c"
      launchcheck "$SLUG" "$F" "$M" "$(jp "$W" "$c.custody.collection")" "$A"
      moncheck "$SLUG" "$(jp "$W" "$c.custody.collection")" "$A" "$M" "$CUST"
      ethcheck "$SLUG" "$(jp "$W" "$c.custody.collection")" "$A" "$M"
      NFTCOLS="$NFTCOLS $(jp "$W" "$c.custody.collection" | lc)"
    fi
    if [ "$MADE" = "True" ] || [ $PHASE1 = 1 ] || [ "$(call "$T" 'isCollection(address)(bool)' "$M")" = "true" ]; then
      same "$SLUG treasury.isCollection(module)" "true" "$(call "$T" 'isCollection(address)(bool)' "$M")"
      # G-N3: positively a position, never "not 0" (an empty answer is not "in the directory")
      local IX; IX=$(call "$DIR" 'indexOf(address)(uint256)' "$A")
      echo "$IX" | grep -Eq '^[1-9][0-9]*$' && echo "ok   $SLUG is in the directory (at $IX)" || { echo "FAIL $SLUG is not in the directory (indexOf: ${IX:-unreadable})"; fail=1; }
    else
      echo "note $SLUG: the owner's steps are still to be made (treasury.addCollection, directory.add): docs/v6/deploy-stack.md §6"
    fi
    # the listing: nothing pending that changes what custody accepts
    [ "$(cast call --rpc-url "$RPC" "$R" 'pendingRoot(uint256)(bytes32,uint64)' "$LID" 2>/dev/null | tail -1 | awk '{print $1}')" = "0" ] && echo "ok   $SLUG listing $LID no pending root" || { echo "FAIL $SLUG listing $LID has a pending root"; fail=1; }
    [ "$(cast call --rpc-url "$RPC" "$R" 'pendingBtcVault(uint256)(string,uint64)' "$LID" 2>/dev/null | tail -1 | awk '{print $1}')" = "0" ] && echo "ok   $SLUG listing $LID no pending BTC vault" || { echo "FAIL $SLUG listing $LID has a pending BTC vault"; fail=1; }
    # fixes-A-graduation.md: the new graduation views, printed (A-03: launches close while the module has no room)
    echo "     $SLUG factory.launchesOpen() $(tcall "$F" 'launchesOpen()(bool)'); $(tcall "$F" 'coinCount()(uint256)') coin(s) launched"
    local NC CI COIN CURVE; NC=$(tcall "$F" 'coinCount()(uint256)'); echo "${NC:-x}" | grep -Eq '^[0-9]+$' || NC=0
    for ((CI = 0; CI < ${NC:-0} && CI < 20; CI++)); do
      COIN=$(tcall "$F" 'coins(uint256)(address)' "$CI"); CURVE=$(tcall "$F" 'routerOf(address)(address)' "$COIN")
      echo "     $SLUG coin $COIN curve $CURVE: canClose $(tcall "$CURVE" 'canClose()(bool)'), canGraduate $(tcall "$CURVE" 'canGraduate()(bool)'), refundsOpenAt $(cast call --rpc-url "$RPC" "$CURVE" 'refundsOpenAt()(uint64,uint64)' 2>/dev/null | awk '{print $1}' | tr '\n' '/' | sed 's#/$##'), module.hasRoomFor(itemBudget) $(tcall "$M" 'hasRoomFor(uint256)(bool)' "$(tcall "$CURVE" 'itemBudget()(uint256)')")"
    done
  done
  # ordinals at launch: the stack's Bitcoin collection, read back from the canister, the install argument and the monitor
  if [ -n "$BTCWRAP" ]; then
    btccanister; btcinstall; btcmonitor
    # a Bitcoin-only stack read against the launch file: its staged plan and exit rule (the Bitcoin cap counted) are still read
    if [ -z "$NFTCOLS" ] && [ -n "$LAUNCH" ]; then
      local LN; LN=$(python3 -c 'import json,sys; print(len(json.load(open(sys.argv[1])).get("collections") or []))' "$LAUNCH")
      [ "$LN" = 0 ] || phase1only "the launch file names $LN Ethereum collection(s) and this stack deploys none: the mixed launch is ONE deploy-stack.sh run (the default kind with ETCH_NFT_COLLECTIONS)"
      launchexit; canexit
    fi
  elif [ -n "$LAUNCH" ] && [ -n "$(launchbtc slug)" ]; then
    phase1only "the launch file decides the Bitcoin collection $(launchbtc slug) and this stack deploys none: deploy it in the same deploy-stack.sh run as the Ethereum collections (the default kind with ETCH_NFT_COLLECTIONS: docs/v7/golive-runbook.md §9)"
  fi
  # D1: several Ethereum collections at once; the canister serves exactly the stack's (none the stack does not vouch for)
  if [ -n "$NFTCOLS" ]; then
    echo "     Ethereum collections in this stack: $(echo $NFTCOLS | wc -w | tr -d ' ') ($NFTCOLS )"
    if [ -n "$ETHCOLS" ]; then
      same "the canister's eth_nft collections are exactly the stack's" "$(printf '%s\n' $NFTCOLS | sort | paste -sd, -)" "$ETHCOLS"
    fi
    # round 2: the launch file names exactly the stack's collections (none deployed that it does not decide, none it decides missing)
    if [ -n "$LAUNCH" ]; then
      same "the launch file's collections are exactly the stack's" "$(printf '%s\n' $NFTCOLS | sort | paste -sd, -)" \
        "$(python3 -c 'import json,sys; print(",".join(sorted(x["collection"].lower() for x in json.load(open(sys.argv[1]))["collections"])))' "$LAUNCH")"
    fi
    monglobal
    installcheck
    launchexit
    canexit
    wakecheck
    cyclescheck
    if [ $PHASE1 = 1 ]; then
      [ -n "$LAUNCH" ] || { echo "FAIL --phase1 on an NFT stack without --launch deployments/launch/nft-pilot.json: the targets, conversion and purchase limits were not held against the launch decision"; fail=1; }
      [ -n "$MONCFG" ] || { echo "FAIL --phase1 on an NFT stack without --monitor-config <the launch monitor configuration>: D8 and the no-good-post pause were not checked"; fail=1; }
    fi
  fi
}
# wrapper pairs named on the command line: WrapClaim built its Wrapper, and each must point at the other
for W in "$@"; do
  [ -f "$W" ] || { echo "FAIL $W does not exist"; fail=1; continue; }
  if has "$W" nftRegistry; then nftcheck "$W"; continue; fi
  if has "$W" directory; then stackcheck "$W"; continue; fi
  C=$(j "$W" wrapClaim); A=$(j "$W" wrapper); same "wrapper file is this chain's" "$(j "$W" chainId)" "$HEX"
  code "$C" || { echo "FAIL WrapClaim $C has no code"; fail=1; continue; }
  code "$A" || { echo "FAIL wrapper $A has no code"; fail=1; continue; }
  same "wrapClaim.wrapper is the file's wrapper"        "$A" "$(call "$C" 'wrapper()(address)')"
  same "wrapper.distribution is the WrapClaim"          "$C" "$(call "$A" 'distribution()(address)')"
  same "wrapClaim.registry"  "$R" "$(call "$C" 'registry()(address)')"
  same "wrapClaim.listingId" "$(j "$W" listingId)" "$(call "$C" 'listingId()(uint256)')"
  same "wrapper.registry"    "$R" "$(call "$A" 'registry()(address)')"
  same "wrapper.distributor" "$D" "$(call "$A" 'distributor()(address)')"
  same "wrapper.beacon"      "$B" "$(call "$A" 'beacon()(address)')"
  same "wrapper.listingId"   "$(j "$W" listingId)" "$(call "$A" 'listingId()(uint256)')"
  same "wrapper.symbol"      "$(j "$W" symbol)" "$(cast call --rpc-url "$RPC" "$A" 'symbol()(string)' | tr -d '"')"
  # docs/v3/wrapper.md, "Custody on its own key": the custodian may record deliveries, and the listing delivers to custody
  same "wrapper.custodian"      "$(j "$W" custodian)" "$(call "$A" 'custodian()(address)')"
  CUST=$(cast call --rpc-url "$RPC" "$A" 'custodyAddress()(string)' | tr -d '"')
  same "wrapper.custodyAddress" "$(j "$W" custodyAddress)" "$CUST"
  LBTC=$(cast call --rpc-url "$RPC" "$R" 'listing(uint256)((uint8,bool,bool,uint8,address,bytes32,string,string,string))' "$(j "$W" listingId)" \
    | python3 -c 'import sys,re; print(re.findall(r"\"([^\"]*)\"", sys.stdin.read())[-1])')
  same "listing.btcVault is custody (or wrap refuses)" "$CUST" "$LBTC"
  echo "     wrapper totalSupply $(tcall "$A" 'totalSupply()(uint256)') (0 until the first wrap)"
done
# G-N1: the go/no-go read is of the whole stack, and of the canister: without the stack file none of C-07, B-05, the ceilings,
# the wiring, the payees or the caps above was read
if [ $PHASE1 = 1 ]; then
  [ $STACKSEEN = 1 ] && echo "ok   --phase1: the stack file was read" || { echo "FAIL --phase1 without the stack file (deployments/stack-$HEX.json): the stack's wiring, treasury split, ceilings, payees and caps were NOT read"; fail=1; }
  if [ -z "$CANISTER" ]; then
    if [ $BEFORE_REINSTALL = 1 ]; then echo "WARN --before-reinstall: the canister's custody_caps() and custody_fee_pool() (one entry >= $FEE_POOL_MIN_SATS sats) were not read; run this again with --canister <buyer> after the reinstall"
    elif [ $REHEARSAL = 1 ]; then echo "WARN no --canister (GOLIVE_REHEARSAL): custody_caps() not read"
    else echo "FAIL --phase1 without --canister: the buyer's custody_caps() was not read (pass --canister <buyer>, or --before-reinstall and read it after)"; fail=1; fi
  fi
fi
# an --install-arg given for a stack with no Ethereum collection is still read (never silently ignored)
[ -z "$INSTALLARG" ] || [ $INSTALLSEEN = 1 ] || installcheck
if [ -s "$READFAIL" ]; then sort -u "$READFAIL"; fail=1; fi
[ $fail = 0 ] && echo "ALL CHECKS PASS$([ $PHASE1 = 1 ] && echo " (--phase1: the go/no-go read)")" || echo "SOME CHECKS FAILED"
exit $fail
