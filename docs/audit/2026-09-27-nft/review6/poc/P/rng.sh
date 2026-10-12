#!/bin/bash
URL=$1; ADDR=$2; shift 2
rpc(){ curl -s -m 60 -H 'content-type: application/json' -X POST "$URL" -d "$1"; }
HEAD=$(( $(rpc '{"jsonrpc":"2.0","id":1,"method":"eth_blockNumber","params":[]}' | python3 -c 'import sys,json;print(json.load(sys.stdin)["result"])') ))
T=0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef
for spec in "$@"; do D=${spec%:*}; N=${spec#*:}
 TO=$((HEAD-D)); FROM=$((TO-N+1))
 curl -s -m 60 -o out.json -w '%{http_code} %{size_download}B %{time_total}s' -H 'content-type: application/json' -X POST "$URL" -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"eth_getLogs\",\"params\":[{\"fromBlock\":\"$(printf 0x%x $FROM)\",\"toBlock\":\"$(printf 0x%x $TO)\",\"address\":\"$ADDR\",\"topics\":[\"$T\"]}]}" > meta.txt
 echo "  depth $D n=$N: $(cat meta.txt) $(python3 -c 'import json;r=json.load(open("out.json"));print(("%d logs"%len(r["result"])) if "result" in r else json.dumps(r.get("error"))[:150])' 2>/dev/null)"
 sleep 2
done
