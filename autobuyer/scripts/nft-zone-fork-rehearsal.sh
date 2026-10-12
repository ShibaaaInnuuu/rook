#!/usr/bin/env bash
# The SignedZone fork rehearsal (docs/v4/eth-nft-canister.md §12.5): the canister buys a REAL restricted OpenSea listing on a
# Base fork, through OpenSea's REAL SignedZone, with the order handed over by the keeper's OpenSea code path (src/opensea.ts,
# OpenSea's answer mocked: no OpenSea call is made here).
#
#   scripts/nft-zone-fork-rehearsal.sh fork      anvil on :8629, Base forked at the block before the real fill
#
# ORDER=51673570 (default) is a real fill whose creator fee goes to the collection's ERC-2981 receiver; ORDER=51673875 is one whose
# collection has none, which the canister refuses (rule 6) — the run shows both.
#   scripts/nft-zone-fork-rehearsal.sh install   install (or upgrade) this worktree's own local canister for chain 8453 on :8629
#   scripts/nft-zone-fork-rehearsal.sh run       the refusals, then the purchase
#
# What is substituted, and only on the fork: WHO SIGNED THE ZONE'S APPROVAL. OpenSea issues a zone signature only to the
# fulfiller it names, and it named the real buyer, so the real extraData cannot be used by the canister. Instead a test signer
# (anvil's PUBLIC dev key #1, not a secret) is added to the real zone through the real controller path — the controller is
# impersonated and calls updateSigner(signer, true); the zone's code and storage are not touched any other way — and it signs
# SIP-7 SignedOrder for the canister's address. Everything else is real: the order, the offerer's bulk signature, Seaport 1.6,
# the zone's code, the transfer validator its substandard 8 names, and the chain state at the fork block.
#
# Fork and local replica only. Nothing is sent to a public chain.
set -euo pipefail
cd "$(dirname "$0")/../canister-rs"
export PATH="$HOME/Library/Application Support/org.dfinity.dfx/bin:$HOME/.foundry/bin:$HOME/.cargo/bin:$PATH"
ORDER=${ORDER:-51673570}
FX=tests/fixtures/seaport/base-zone-fill-$ORDER.json
R=http://127.0.0.1:8629
SEAPORT=0x0000000000000068F116a894984e2DB1123eB395
ZONE=0x000056F7000000EcE9003ca63978907a00FFD100
CONTROLLER=0x000000657e498d54000053b69bdaad008a001dbf
TEST_SIGNER=0x70997970C51812dc3A010C7d01b50e0d17dc79C8
TEST_SIGNER_KEY=0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d   # anvil dev account #1: public, fork only
STRANGER_KEY=0x1111111111111111111111111111111111111111111111111111111111111111   # an arbitrary key never added to the zone
COLL=$(jq -r .collection "$FX") TOKEN=$(jq -r .token_id "$FX")
SIG="fulfillAdvancedOrder(((address,address,(uint8,address,uint256,uint256,uint256)[],(uint8,address,uint256,uint256,uint256,address)[],uint8,uint256,uint256,bytes32,uint256,bytes32,uint256),uint120,uint120,bytes,bytes),(uint256,uint8,uint256,uint256,bytes32[])[],bytes32,address)"
REAL_EXTRA=$(cast calldata-decode "$SIG" "$(jq -r .input "$FX")" | head -1 | grep -o ', 0x00[0-9a-f]*)$' | tr -d ',) ')

call() { dfx canister call autobuyer "$@" 2>&1 | grep -v -e WARNING -e '"metadata"' -e candid:service -e '^ *[][{}]* *$'; }
us() { call evm_address | sed -n 's/.*Ok = "\(0x[0-9a-f]*\)".*/\1/p'; }
fx() { jq -r ".$1" "$FX"; }

# SIP-6 v0 extraData: fulfiller | expiration | EIP-2098 signature of SIP-7 SignedOrder by $key | the real context (substandard 8)
extra() { # fulfiller expiration key
  local ful=$1 exp=$2 key=$3 ctx oh th s dig sig
  ctx=0x${REAL_EXTRA:188}   # the real order's own context (substandard 8, its identifier and transfer validator)
  oh=$(fx order_hash)
  th=$(cast keccak "SignedOrder(address fulfiller,uint64 expiration,bytes32 orderHash,bytes context)")
  s=$(cast keccak "$(cast abi-encode 'f(bytes32,address,uint64,bytes32,bytes32)' "$th" "$ful" "$exp" "$oh" "$(cast keccak "$ctx")")")
  dig=$(cast keccak "0x1901$(fx zone_domain_separator | cut -c3-)${s:2}")
  sig=$(cast wallet sign --no-hash --private-key "$key" "$dig")
  python3 - "$ful" "$exp" "$sig" "$ctx" <<'PY'
import sys
ful, exp, sig, ctx = sys.argv[1], int(sys.argv[2]), bytes.fromhex(sys.argv[3][2:]), sys.argv[4][2:]
r, s, v = sig[:32], int.from_bytes(sig[32:64], "big"), sig[64]
vs = (s | ((v - 27) << 255)).to_bytes(32, "big")          # EIP-2098
print("0x00" + ful[2:].lower() + exp.to_bytes(8, "big").hex() + (r + vs).hex() + ctx)
PY
}

# The keeper's code path: OpenSea's answer (the documented shape, values from the real fill) with this extraData, through
# src/opensea.ts toHandoff for the canister as fulfiller. Prints the order hex the canister is handed.
handoff() { # extraData fulfiller
  (cd ../../keeper && ORDER=$ORDER EXTRA=$1 FUL=$2 npx --yes tsx -e '
    import { readFileSync } from "node:fs";
    import { toHandoff } from "./src/opensea.ts";
    const a = JSON.parse(readFileSync(`fixtures/opensea-fulfillment-base-${process.env.ORDER}.json`, "utf8"));
    a.fulfillment_data.transaction.input_data.advancedOrder.extraData = process.env.EXTRA;
    console.log(toHandoff(a, 8453, process.env.FUL as `0x${string}`).orderHex);')
}
# The real calldata with only its extraData spliced (same length): what a relayer that skips the keeper could hand in.
splice() { python3 -c "import sys,json; i=json.load(open('$FX'))['input']; old=sys.argv[1][2:]; new=sys.argv[2][2:]; assert len(old)==len(new) and old in i; print(i.replace(old,new)[:-8])" "$1" "$2"; }

case "${1:-}" in
  fork) exec anvil --fork-url "${BASE_UPSTREAM:-https://mainnet.base.org}" --fork-block-number "$(fx fork_block)" --port 8629 --host 127.0.0.1 --silent ;;
  install)
    dfx deploy autobuyer --upgrade-unchanged --argument "(record { chain_id = 46630 : nat64; vault = \"0x0000000000000000000000000000000000000001\";
      rpc_urls = vec { \"https://rpc.testnet.chain.robinhood.com\"; \"https://robinhood-testnet.drpc.org\" };
      btc_api_urls = vec { \"https://mempool.space/signet/api\"; \"https://blockstream.info/signet/api\" }; key_name = \"key_1\";
      accept_production_key_name = true; bridge_allowlist = vec {}; btc_network = \"signet\"; operator = null; wrapper = null;
      evm_chains = opt vec { record { chain_id = 8453 : nat64; rpc_urls = vec { \"http://127.0.0.1:8629\"; \"http://localhost:8629\" };
        bucket = 64 : nat64; seaport = \"$SEAPORT\"; seaport_codehash = \"0x2d5cb8553e21a19550413299c05a6493cc606fb0000b5042328a5ba06bfdaf35\";
        max_fee_wei = 10000000000 : nat64; priority_fee_wei = 1000000 : nat64; gas_cap = 600000 : nat64;
        fee_recipients = vec { \"0x0000a26b00c1F0DF003000390027140000fAa719\" }; max_fee_bps = 1500 : nat32; min_order_life_s = 60 : nat64;
        signed_zones = opt vec { record { zone = \"$ZONE\"; codehash = \"$(fx zone_codehash)\" } };
        zone_registries = opt vec { \"0x721c008fdff27bf06e7e123956e2fe03b63342e3\"; \"0xa000027a9b2802e1ddf7000061001e5c005a0000\" } } } })"
    echo "canister $(dfx canister id autobuyer), EVM address $(us)" ;;
  run)
    US=$(us); VALUE=$(printf '%d' "$(fx value)"); NOW=$(date +%s)
    # the fork: fund us, add the test signer through the controller, and mine past a pin (the clock stays at the fork's hour)
    cast rpc anvil_setBalance "$US" 0x16345785D8A0000 --rpc-url $R >/dev/null
    cast rpc anvil_impersonateAccount $CONTROLLER --rpc-url $R >/dev/null
    cast rpc anvil_setBalance $CONTROLLER 0xDE0B6B3A7640000 --rpc-url $R >/dev/null
    echo "an outsider adding a signer: $(cast send $ZONE 'updateSigner(address,bool)' $TEST_SIGNER true --from 0x000000000000000000000000000000000000dEaD --unlocked --rpc-url $R 2>&1 | grep -o 'InvalidController' | head -1)"
    cast send $ZONE 'updateSigner(address,bool)' $TEST_SIGNER true --from $CONTROLLER --unlocked --rpc-url $R >/dev/null
    echo "the controller adding the test signer: isActiveSigner = $(cast call $ZONE 'isActiveSigner(address)(bool)' $TEST_SIGNER --rpc-url $R)"
    cast rpc anvil_mine 0x94 0x1 --rpc-url $R >/dev/null
    echo "owner before: $(cast call $COLL 'ownerOf(uint256)(address)' $TOKEN --rpc-url $R)"
    call nft_set_target "(8453 : nat64, \"$COLL\", \"$TOKEN\", $VALUE : nat)" | grep -E 'Ok|Err'
    sign() { echo "== $1"; call nft_sign "(8453 : nat64, \"$COLL\", \"$TOKEN\", \"$2\")" | grep -E 'Err|tx_hash|order_hash|total_wei|signature_checked' | cut -c1-260; }
    sign "(a) the REAL order and extraData, issued by OpenSea to the real buyer" "$(splice "$REAL_EXTRA" "$REAL_EXTRA")"
    sign "(c) a zone signature for us that expires within the minimum life" "$(handoff "$(extra "$US" $((NOW + 30)) $TEST_SIGNER_KEY)" "$US")"
    sign "(c) a zone signature for us from a signer the zone does not know" "$(splice "$REAL_EXTRA" "$(extra "$US" $((NOW + 900)) $STRANGER_KEY)")"
    BAD=$(extra "$US" $((NOW + 900)) $TEST_SIGNER_KEY); BAD="${BAD:0:188}05${BAD:190}"
    sign "(c) a substandard the canister does not know" "$(splice "$REAL_EXTRA" "$BAD")"
    sign "(b) the real order, a zone signature for us from the fork's test signer, handed over by the keeper's code" \
      "$(handoff "$(extra "$US" $((NOW + 900)) $TEST_SIGNER_KEY)" "$US")"
    call nft_send "(8453 : nat64)" | grep -E 'Accepted|NonceTooLow|Refused|seen|Err'
    cast rpc anvil_mine 0x84 0x1 --rpc-url $R >/dev/null
    call nft_status "(8453 : nat64)" | grep -E 'Ok|Err'
    echo "owner after (anvil, not the canister): $(cast call $COLL 'ownerOf(uint256)(address)' $TOKEN --rpc-url $R)" ;;
  *) sed -n 2,19p "$0"; exit 1 ;;
esac
