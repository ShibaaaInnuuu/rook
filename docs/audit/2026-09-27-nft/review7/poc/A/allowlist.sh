#!/bin/bash
# Runs golive-check's INSTALL_PY (extracted verbatim) under --phase1 on install arguments whose chain-1 pair varies one URL.
cd "$(dirname "$0")/.." && awk "/^read -r -d '' INSTALL_PY <<'PY'/{f=1;next} /^PY\$/{if(f){exit}} f" script/golive-check.sh > poc/install_py.py
T=https://gateway.tenderly.co/public/mainnet; i=0
for u in "http://rpc.mevblocker.io" "https://RPC.MEVBLOCKER.IO" "https://rpc.mevblocker.io:8443" "https://rpc.mevblocker.io/?x=1" \
  "https://rpc.mevblocker.io.evil.com" "https://rpc.mevblocker.io./" "https://x@rpc.mevblocker.io" "https://rpc.mevblocker.io/fullprivacy" \
  "https://rpc.mevblocker.io/fast/" "http://gateway.tenderly.co/public/mainnet" "https://[::1]/" "https://rpc.mevblocker.io//" ; do
  i=$((i+1)); poc/mkhex.sh "$u,$T" poc/h$i.hex
  chains=$(python3 -c 'import json,sys; print(json.dumps({"--install-arg": [["4663", ["https://rpc.mainnet.chain.robinhood.com","https://rpc.ordofi.network"]], ["1", [sys.argv[1], sys.argv[2]]]]}))' "$u" "$T")
  echo "== $u"
  GOLIVE_PHASE1=1 python3 poc/install_py.py poc/h$i.hex "" "$chains" | grep -v "tenderly.co is on\|4663\|two provider hosts" ; echo "   exit $?"
done
