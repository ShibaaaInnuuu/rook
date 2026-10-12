#!/bin/sh
# Review 6, part F: read-only probe of the canister's 4663 providers (scripts/golive-dryrun.sh:115, the install's rpc_urls) and of
# robinhood.drpc.org (the monitor's second): how far back each serves STATE (eth_getTransactionCount at a block number, what
# canister::home_nonce_tx bisects on) and BLOCKS (eth_getBlockByNumber, full), and 4663's block time.
q() { curl -s -m 15 -X POST -H 'content-type: application/json' --data "$1" "$2"; }
for u in https://rpc.mainnet.chain.robinhood.com https://rpc.ordofi.network https://robinhood.drpc.org; do
  H=$(q '{"jsonrpc":"2.0","id":1,"method":"eth_blockNumber","params":[]}' $u | python3 -c 'import sys,json;print(int(json.load(sys.stdin)["result"],16))')
  echo "== $u head $H"
  for d in 0 100 3000 6000 6100 6300 6500 10000 100000 1000000; do
    b=$(printf %x $((H-d)))
    s=$(q "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"eth_getTransactionCount\",\"params\":[\"0x0000000000000000000000000000000000000001\",\"0x$b\"]}" $u | cut -c1-110)
    k=$(q "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"eth_getBlockByNumber\",\"params\":[\"0x$b\",true]}" $u | python3 -c 'import sys,json;r=json.load(sys.stdin);print("block ok, %d txs" % len(r["result"]["transactions"]) if r.get("result") else "block: "+str(r)[:80])')
    echo "back $d: state: $s | $k"
  done
done
u=https://rpc.mainnet.chain.robinhood.com
H=$(q '{"jsonrpc":"2.0","id":1,"method":"eth_blockNumber","params":[]}' $u | python3 -c 'import sys,json;print(int(json.load(sys.stdin)["result"],16))')
for d in 0 1000 6200; do printf "timestamp at back %d: " $d; q "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"eth_getBlockByNumber\",\"params\":[\"0x$(printf %x $((H-d)))\",false]}" $u | python3 -c 'import sys,json;print(int(json.load(sys.stdin)["result"]["timestamp"],16))'; done
date -u
