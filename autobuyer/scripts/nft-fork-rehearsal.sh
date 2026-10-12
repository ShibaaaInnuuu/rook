#!/usr/bin/env bash
# The NFT fork rehearsal (docs/v4/eth-nft-canister.md §10): the buyer canister buys a REAL Seaport listing on two chains —
# Ethereum and Base — through one code path. Each chain is an anvil fork taken at the block BEFORE a real buyer filled the
# listing, so the listing is live there with its real signature. The canister checks, signs, sends, reads back and sees the
# NFT arrive; tampered copies are refused before signing.
#
#   scripts/nft-fork-rehearsal.sh forks     start the two forks (Ethereum on 8608, Base on 8609)
#   scripts/nft-fork-rehearsal.sh install   install (or upgrade) this worktree's own local canister, naming both forks
#   scripts/nft-fork-rehearsal.sh eth       tampered orders refused, then the real one bought, on the Ethereum fork
#   scripts/nft-fork-rehearsal.sh base      the same on the Base fork
#
# Forks and the local replica only. Nothing is sent to any public chain: the canister's providers are the two anvils (one
# anvil reached by two URLs, so the agreement check runs but cannot disagree). The fork keeps the real chain id, which the
# real order's EIP-712 signature needs; the local replica key's address holds nothing on either real chain.
set -euo pipefail
cd "$(dirname "$0")/../canister-rs"
export PATH="$HOME/Library/Application Support/org.dfinity.dfx/bin:$HOME/.foundry/bin:$HOME/.cargo/bin:$PATH"
FX=tests/fixtures/seaport
SEAPORT=0x0000000000000068F116a894984e2DB1123eB395
OPENSEA_FEE=0x0000a26b00c1F0DF003000390027140000fAa719
ETH=http://127.0.0.1:8608 BASE=http://127.0.0.1:8609
ETH_UPSTREAM=${ETH_UPSTREAM:-https://eth.drpc.org} BASE_UPSTREAM=${BASE_UPSTREAM:-https://xrpc.cl/base}

call() { dfx canister call autobuyer "$@" 2>&1 | grep -v -e WARNING -e '"metadata"' -e candid:service -e '^ *[][{}]* *$'; }
us() { call evm_address | sed -n 's/.*Ok = "\(0x[0-9a-f]*\)".*/\1/p'; }
field() { jq -r ".$1" "$FX/$2"; }
# fulfillBasicOrder calldata with one change: price DELTA wei, a stranger as fee recipient, or another token id
tamper() {
  python3 - "$1" "$2" <<'PY'
import sys
b = bytearray(bytes.fromhex(sys.argv[1][2:])); how = sys.argv[2]
word = lambda i: 4 + 32 + 32 * i
rec = 4 + 32 + 0x240
if how.startswith("price"):
    o = word(2); v = int.from_bytes(b[o:o+32], "big") + int(how[5:]); b[o:o+32] = v.to_bytes(32, "big")
elif how == "stranger":
    b[rec+76:rec+96] = bytes.fromhex("000000000000000000000000000000000000beef")
elif how == "token":
    o = word(6); v = int.from_bytes(b[o:o+32], "big") + 1; b[o:o+32] = v.to_bytes(32, "big")
print("0x" + b.hex())
PY
}

rehearse() { # chain rpc fixture collection token cap funding_wei bucket [time]
  local id=$1 rpc=$2 fx=$3 coll=$4 tok=$5 cap=$6 fund=$7 bucket=$8 time=${9:-}
  local order; order=$(field input "$fx")
  cast rpc anvil_setBalance "$(us)" "$(printf '0x%x' "$fund")" --rpc-url "$rpc" >/dev/null
  [ -n "$time" ] && cast rpc evm_setNextBlockTimestamp "$time" --rpc-url "$rpc" >/dev/null
  cast rpc anvil_mine "$(printf '0x%x' $((bucket * 2 + 20)))" 0x1 --rpc-url "$rpc" >/dev/null   # so the pin sees the funding
  echo "owner before: $(cast call "$coll" 'ownerOf(uint256)(address)' "$tok" --rpc-url "$rpc")"
  call nft_set_target "($id : nat64, \"$coll\", \"$tok\", $cap : nat)"
  for how in stranger price-1 price+1000000000000000000 token; do
    echo "== tampered ($how)"; call nft_sign "($id : nat64, \"$coll\", \"$tok\", \"$(tamper "$order" "$how")\")" | grep -E 'Err|Ok ='
  done
  echo "== the real order"
  call nft_sign "($id : nat64, \"$coll\", \"$tok\", \"$order\")" | grep -E 'Err|tx_hash|order_hash|total_wei|pin'
  call nft_send "($id : nat64)"
  cast rpc anvil_mine "$(printf '0x%x' $((bucket * 2 + 4)))" 0x1 --rpc-url "$rpc" >/dev/null   # past the next pin
  call nft_status "($id : nat64)"
  echo "owner after (anvil, not the canister): $(cast call "$coll" 'ownerOf(uint256)(address)' "$tok" --rpc-url "$rpc")"
}

case "${1:-}" in
  forks)
    anvil --fork-url "$ETH_UPSTREAM" --fork-block-number 26037607 --port 8608 --host 127.0.0.1 --silent &
    anvil --fork-url "$BASE_UPSTREAM" --fork-block-number 51672613 --port 8609 --host 127.0.0.1 --silent &
    wait ;;
  install)
    chain() { # id port bucket codehash max_fee prio life
      echo "record { chain_id = $1 : nat64; rpc_urls = vec { \"http://127.0.0.1:$2\"; \"http://localhost:$2\" }; bucket = $3 : nat64;
        seaport = \"$SEAPORT\"; seaport_codehash = \"$4\"; max_fee_wei = $5 : nat64; priority_fee_wei = $6 : nat64; gas_cap = 400000 : nat64;
        fee_recipients = vec { \"$OPENSEA_FEE\" }; max_fee_bps = 1500 : nat32; min_order_life_s = $7 : nat64 }"
    }
    dfx deploy autobuyer --argument "(record { chain_id = 46630 : nat64; vault = \"0x0000000000000000000000000000000000000001\";
      rpc_urls = vec { \"https://rpc.testnet.chain.robinhood.com\"; \"https://robinhood-testnet.drpc.org\" };
      btc_api_urls = vec { \"https://mempool.space/signet/api\"; \"https://blockstream.info/signet/api\" }; key_name = \"key_1\";
      accept_production_key_name = true; bridge_allowlist = vec {}; btc_network = \"signet\"; operator = null; wrapper = null;
      evm_chains = opt vec {
        $(chain 1 8608 8 0x74499ac0cce14428e4b41541d5e44f28f5a6882a1051d0118867c2a93cd5aec0 200000000000 1000000000 300);
        $(chain 8453 8609 64 0x2d5cb8553e21a19550413299c05a6493cc606fb0000b5042328a5ba06bfdaf35 10000000000 1000000 60) } })"
    echo "canister $(dfx canister id autobuyer), EVM address $(us)" ;;
  eth)  rehearse 1 "$ETH" eth-fill-26037608.json 0x97630aA70AB14ed9883B41dAfccBc11349723043 9820 60000000000000000 1000000000000000000 8 ;;
  # the Base listing ends 401 s after the fork block: the fork's clock is set back to just after it
  base) rehearse 8453 "$BASE" base-fill-51672614.json 0x15f3B4d6B019D506D719a02Ee97121Bd95f6b6a6 8901 5000000000000000 100000000000000000 64 1790134580 ;;
  *) sed -n 2,15p "$0"; exit 1 ;;
esac
