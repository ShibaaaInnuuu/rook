#!/bin/bash
URL=$1; shift
rpc(){ curl -s -m 30 -H 'content-type: application/json' -X POST "$URL" -d "$1"; }
HEAD=$(( $(rpc '{"jsonrpc":"2.0","id":1,"method":"eth_blockNumber","params":[]}' | python3 -c 'import sys,json;print(json.load(sys.stdin)["result"])') ))
echo "$URL head $HEAD"
for D in "$@"; do
 B=$((HEAD-D))
 TX=$(rpc "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"eth_getBlockByNumber\",\"params\":[\"$(printf 0x%x $B)\",false]}" | python3 -c 'import sys,json;r=json.load(sys.stdin);print(r["result"]["transactions"][0])' 2>/dev/null)
 R=$(rpc "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"eth_getTransactionReceipt\",\"params\":[\"$TX\"]}" | python3 -c 'import sys,json;r=json.load(sys.stdin);print("ok" if r.get("result") else "NULL/ERR "+json.dumps(r.get("error"))[:100])' 2>/dev/null)
 R2=$(rpc "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"eth_getTransactionByHash\",\"params\":[\"$TX\"]}" | python3 -c 'import sys,json;r=json.load(sys.stdin);print("ok" if r.get("result") else "NULL/ERR "+json.dumps(r.get("error"))[:100])' 2>/dev/null)
 echo "  depth $D: receipt $R ; txByHash $R2"
 sleep 1
done
