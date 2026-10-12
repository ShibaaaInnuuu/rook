#!/bin/bash
# read-only JSON-RPC: rpc.sh URL METHOD PARAMS
curl -s -m 60 -X POST -H 'content-type: application/json' "$1" --data "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"$2\",\"params\":$3}"
