#!/bin/bash
# read-only JSON-RPC depth probe
URL=$1
rpc(){ curl -s -m 25 -H 'content-type: application/json' -X POST "$URL" -d "$1"; }
HEADHEX=$(rpc '{"jsonrpc":"2.0","id":1,"method":"eth_blockNumber","params":[]}' | python3 -c 'import sys,json;print(json.load(sys.stdin)["result"])' 2>/dev/null)
[ -z "$HEADHEX" ] && { echo "$URL: no head"; exit; }
HEAD=$((HEADHEX))
FIN=$(rpc '{"jsonrpc":"2.0","id":1,"method":"eth_getBlockByNumber","params":["finalized",false]}' | python3 -c 'import sys,json;r=json.load(sys.stdin);print(int(r["result"]["number"],16) if r.get("result") else "ERR "+str(r.get("error")))' 2>/dev/null)
echo "$URL head=$HEAD finalized=$FIN"
MIL=0x5af0d9827e0c53e4799bb226655a1de152a425a5
T=0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef
for D in 1000 9000 11000 20000 40000 52000 60000 130000; do
  FROM=$((HEAD-D-499)); TO=$((HEAD-D))
  R=$(rpc "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"eth_getLogs\",\"params\":[{\"fromBlock\":\"$(printf 0x%x $FROM)\",\"toBlock\":\"$(printf 0x%x $TO)\",\"address\":\"$MIL\",\"topics\":[\"$T\"]}]}")
  echo "  depth $D (500 blk): $(echo "$R" | python3 -c 'import sys,json
try:
 r=json.load(sys.stdin); print(("ok %d logs"%len(r["result"])) if "result" in r else "ERR "+json.dumps(r.get("error"))[:140])
except Exception as e: print("BAD", str(e)[:80])')"
done
# receipt / block at 7-day depth
B=$((HEAD-52000))
R=$(rpc "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"eth_getBlockByNumber\",\"params\":[\"$(printf 0x%x $B)\",false]}")
TX=$(echo "$R" | python3 -c 'import sys,json;r=json.load(sys.stdin);print(r["result"]["transactions"][0])' 2>/dev/null)
echo "  block@-52000: ${TX:+ok}"
R=$(rpc "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"eth_getTransactionReceipt\",\"params\":[\"$TX\"]}")
echo "  receipt@-52000: $(echo "$R" | python3 -c 'import sys,json;r=json.load(sys.stdin);print("ok" if r.get("result") else "ERR "+json.dumps(r.get("error"))[:120])' 2>/dev/null)"
