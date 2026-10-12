#!/usr/bin/env bash
# Review round 5, part A (A4-L-3 re-attack): golive-check --install-arg's keyless rule AND its round-4 per-chain host count, probed
# with REAL buyer Inits. INSTALL_PY and installchains' decoder are extracted UNCHANGED from script/golive-check.sh of a private copy
# (COPY=/tmp/rook-r5/A), and run exactly as installcheck runs them (GOLIVE_PHASE1=1, argv[3] = the decoded chains).
# "ok ... all keyless" on a keyed URL = the rule misses it; a hostname printed in full = a printed key.
set -u
COPY=${COPY:-/tmp/rook-r5/A}; POC=$(cd "$(dirname "$0")" && pwd)
awk '/^read -r -d .. INSTALL_PY <</{f=1;next} /^PY$/{if(f){exit}} f' "$COPY/script/golive-check.sh" > "$COPY/install_check.py"
awk '/^installchains\(\) \{/{f=1} f&&/^const \{ decodeInit \}/{g=1} g{print} g&&/^process.stdout.write/{exit}' "$COPY/script/golive-check.sh" > "$COPY/monitor/installchains.mjs"
K=f3a9c2e17b4d4e0a9c1b2d3e4f5a6b7c
run(){ echo "--- $1"; echo "    urls: $2"; h=$(cd "$COPY/monitor" && node "$POC/mkinit.mjs" "$2");
  ch=$(cd "$COPY/monitor" && node --input-type=module - "$h" "" < installchains.mjs 2>/dev/null)
  GOLIVE_PHASE1=1 python3 "$COPY/install_check.py" "$h" "" "$ch" | sed 's/^/    /'; }
run "control: the template's keyless pair" "https://xrpc.cl/eth,https://ethereum-rpc.publicnode.com"
run "1 an 11-character key in the path" "https://eth.rpc.example.net/${K:0:11},https://xrpc.cl/eth"
run "2 a 32-hex key as 11/11/10 path segments" "https://eth.rpc.example.net/${K:0:11}/${K:11:11}/${K:22:10},https://xrpc.cl/eth"
run "3 a base64url key split by dots, few digits (22 chars)" "https://eth.rpc.example.net/Zm9vYmFyYmF6.cXV4cXV1eA,https://xrpc.cl/eth"
run "4 a letters-only key in a hyphenated subdomain" "https://qwertyuiop-asdfghjkl.rpc.example.net/,https://xrpc.cl/eth"
run "5 userinfo https://key@host" "https://$K@eth.rpc.example.net/,https://xrpc.cl/eth"
run "6 a query key under another name" "https://eth.rpc.example.net/v1?token=abc,https://xrpc.cl/eth"
run "7 a fragment" "https://eth.rpc.example.net/v1#abcdef,https://xrpc.cl/eth"
run "8 uppercase percent-encoding" "https://eth.rpc.example.net/%4B%45%59,https://xrpc.cl/eth"
run "9 IPv6 literal host with an 11-char key" "https://[2001:db8::1]/${K:0:11},https://xrpc.cl/eth"
run "10 IPv4 literal hosts: one machine, two URLs" "https://203.0.113.7/eth,https://203.0.113.7:8545/eth"
run "11 wss:// with a key beside two https hosts" "wss://eth.rpc.example.net/$K,https://xrpc.cl/eth,https://ethereum-rpc.publicnode.com"
run "12 one host twice: trailing-dot FQDN" "https://xrpc.cl/eth,https://xrpc.cl./eth"
run "13 one provider, two subdomains" "https://ethereum-rpc.publicnode.com,https://eth.publicnode.com"
run "14 a key subdomain on ONE host twice: the host-count FAIL line prints it" "https://$K.eth.rpc.example.net/a,https://$K.eth.rpc.example.net/b"
