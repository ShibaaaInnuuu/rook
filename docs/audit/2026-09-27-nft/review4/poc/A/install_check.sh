#!/usr/bin/env bash
# Review round 4, part A (A4-L-3): golive-check --install-arg's keyless rule, probed with REAL buyer Inits (monitor/src/candid.mjs,
# as scripts/golive-dryrun.sh installhex builds them). The checker is script/golive-check.sh's INSTALL_PY, extracted unchanged into a
# private copy. An "ok ... all keyless" line on a keyed URL = the rule misses it. Run from the repo root.
set -u
SP=${SP:?a private scratch dir}; POC=$(cd "$(dirname "$0")" && pwd)
awk '/^read -r -d .. INSTALL_PY <</{f=1;next} /^PY$/{if(f){exit}} f' script/golive-check.sh > "$SP/install_check.py"
K=f3a9c2e17b4d4e0a9c1b2d3e4f5a6b7c   # a made-up 32-hex key
run(){ echo "--- $1"; echo "    urls: $2"; h=$(cd monitor && node "$POC/mkinit.mjs" "$2"); python3 "$SP/install_check.py" "$h" "" | sed 's/^/    /'; }
run "control: the template's keyless pair" "https://xrpc.cl/eth,https://ethereum-rpc.publicnode.com"
run "control: Dwellir key, host in capitals" "https://API-ETHEREUM-MAINNET.N.DWELLIR.COM/$K,https://xrpc.cl/eth"
run "1 a 15-character key on an unlisted host" "https://eth.rpc.example.net/${K:0:15},https://xrpc.cl/eth"
run "2 a 32-hex key split by dots" "https://eth.rpc.example.net/${K:0:8}.${K:8:8}.${K:16:8}.${K:24:8},https://xrpc.cl/eth"
run "3 a key percent-encoded" "https://eth.rpc.example.net/%66%33%61%39%63%32%65%31%37%62%34%64%34%65%30%61,https://xrpc.cl/eth"
run "4 a key in a subdomain (and printed in the ok line)" "https://$K.eth.rpc.example.net/,https://xrpc.cl/eth"
run "5 a key as path segments of 15, 15 and 2" "https://eth.rpc.example.net/${K:0:15}/${K:15:15}/${K:30:2},https://xrpc.cl/eth"
run "6 wss:// with a key (not an https outcall; unusable, but unread)" "wss://eth.rpc.example.net/$K,https://xrpc.cl/eth"
run "7 drpc.live keyed" "https://lb.drpc.live/ethereum/$K,https://xrpc.cl/eth"
run "8 one provider twice (trailing slash): keyless, but one host" "https://xrpc.cl/eth,https://xrpc.cl/eth/"
run "9 ankr keyed" "https://rpc.ankr.com/eth/$K,https://xrpc.cl/eth"
run "10 a key in a ;param" "https://eth-mainnet.nodereal.io/v1;key=${K:0:15},https://xrpc.cl/eth"
