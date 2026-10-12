#!/bin/sh
# Review round 7, part F: read-only 4663 probe. Block rate, each provider's head, and how far back rpc.mainnet.chain.robinhood.com serves
# eth_getTransactionCount (state) at a block number. No transaction is sent.
R=https://rpc.mainnet.chain.robinhood.com; O=https://rpc.ordofi.network
A=0x0000000000000000000000000000000000000000
q() { curl -s -m 20 -H 'content-type: application/json' -d "$2" "$1"; }
bn() { q "$1" '{"jsonrpc":"2.0","id":1,"method":"eth_blockNumber","params":[]}' | sed -E 's/.*"result":"(0x[0-9a-f]+)".*/\1/'; }
date -u +%FT%TZ
h1=$(bn $R); h2=$(bn $O); echo "heads: robinhood $((h1)) ordofi $((h2))"
t1=$(q $R "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"eth_getBlockByNumber\",\"params\":[\"$h1\",false]}" | sed -E 's/.*"timestamp":"(0x[0-9a-f]+)".*/\1/')
b=$((h1-36000)); t0=$(q $R "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"eth_getBlockByNumber\",\"params\":[\"$(printf 0x%x $b)\",false]}" | sed -E 's/.*"timestamp":"(0x[0-9a-f]+)".*/\1/')
echo "36000 blocks in $((t1-t0)) s"
for d in 3000 4000 5000 5500 6000 6200 6400 6600 7000 8000; do
  blk=$(printf 0x%x $((h1-d)))
  r=$(q $R "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"eth_getTransactionCount\",\"params\":[\"$A\",\"$blk\"]}" | cut -c1-140)
  echo "robinhood depth $d: $r"
done
