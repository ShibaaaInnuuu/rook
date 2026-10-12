#!/usr/bin/env bash
# The item-sink / mint-desk / holder-deposit side on a LOCAL replica (docs/v3/curve.md, "Canister: …"): install this worktree's
# own canister, set the item config once (and see a change refused), derive the float's address beside the buyer's and
# custody's, call every new endpoint so the candid interface is exercised both ways, then upgrade and see the state come back.
#
#   scripts/items-replica-rehearsal.sh
#
# The local replica only: no tick is run, so nothing is read from or sent to any chain. The EVM and Bitcoin halves are
# rehearsed in `cargo test --test frogs` against a fake world; the queue contract they would drive does not exist yet.
set -euo pipefail
cd "$(dirname "$0")/../canister-rs"
export PATH="$HOME/Library/Application Support/org.dfinity.dfx/bin:$HOME/.cargo/bin:$PATH"

call() { dfx canister call autobuyer "$@" 2>&1 | grep -v -e WARNING -e '"metadata"' -e candid:service -e '^ *[][{}]* *$' | tr -s ' ' | tr -d '\n'; echo; }
ARG="(record { chain_id = 46630 : nat64; vault = \"0x0000000000000000000000000000000000000001\";
  rpc_urls = vec { \"https://rpc.testnet.chain.robinhood.com\"; \"https://robinhood-testnet.drpc.org\" };
  btc_api_urls = vec { \"https://mempool.space/signet/api\"; \"https://blockstream.info/signet/api\" }; key_name = \"key_1\";
  accept_production_key_name = true; bridge_allowlist = vec {}; btc_network = \"signet\"; operator = null;
  wrapper = opt \"0x00000000000000000000000000000000000000aa\"; evm_chains = null })"
QUEUE=0x5151515151515151515151515151515151515151
DESK=0x5555555555555555555555555555555555555555
FACTORY=0x5454545454545454545454545454545454545454

echo "== install"
dfx deploy autobuyer --argument "$ARG" 2>&1 | grep -v WARNING | tail -3
echo "== three keys, three addresses"
call buyer_address; call custody_address; call float_address
echo "== the item config: set once, a change refused, a queue only added"
call set_item_config "(opt \"$FACTORY\", opt \"$DESK\", vec { \"$QUEUE\" })"
call set_item_config "(opt \"0x0000000000000000000000000000000000000bad\", null, vec {})"
call set_item_config "(null, null, vec { \"$QUEUE\" })"
echo "== the queries"
call item_sinks; call item_progress "(\"$QUEUE\")"; call frog_buys; call holder_deposits; call desk_requests; call unmarked_custody
echo "== the operator's doors refuse what is not theirs"
call frog_offer "(0 : nat64, \"$(printf 'ab%.0s' {1..32})i0\", \"00\", vec {}, vec {})"
call frog_proof "(\"$(printf 'ab%.0s' {1..32})i0\", vec { \"$(printf '11%.0s' {1..32})\" })"
call refuse_deposit "(\"$(printf 'cd%.0s' {1..32}):0\", \"not a frog\")"
echo "== upgrade: the config comes back"
# the same code under a new module hash (one custom section appended, which no runtime reads), so the replica really runs
# pre_upgrade and post_upgrade: an unchanged hash is skipped as "already installed"
WASM=target/wasm32-unknown-unknown/release/etch_autobuyer.wasm
python3 - "$WASM" <<'PY2'
import sys, time
def leb(n):
    out = b""
    while True:
        b = n & 0x7f; n >>= 7
        out += bytes([b | (0x80 if n else 0)])
        if not n: return out
name = b"rehearsal"; payload = str(time.time()).encode()
content = leb(len(name)) + name + payload
open(sys.argv[1] + ".upgrade", "wb").write(open(sys.argv[1], "rb").read() + b"\x00" + leb(len(content)) + content)
PY2
dfx canister install autobuyer --mode upgrade --wasm "$WASM.upgrade" --argument "$ARG" --yes 2>&1 | grep -v WARNING | tail -2
call item_sinks
call set_item_config "(null, null, vec {})"
call log
