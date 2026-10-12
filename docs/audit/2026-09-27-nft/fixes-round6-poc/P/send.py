import json, time, urllib.request
def rpc(url, method, params):
    body = json.dumps({"jsonrpc":"2.0","id":1,"method":method,"params":params}).encode()
    req = urllib.request.Request(url, data=body, headers={"content-type":"application/json","user-agent":"rook-probe/1"})
    try:
        with urllib.request.urlopen(req, timeout=30) as r: return r.status, r.read()
    except urllib.error.HTTPError as e: return e.code, e.read()
print("# sendRawTransaction reply shapes", time.strftime("%Y-%m-%d %H:%M:%S UTC", time.gmtime()))
T="https://gateway.tenderly.co/public/mainnet"
st,b = rpc(T,"eth_getBlockByNumber",["latest",False]); blk=json.loads(b)["result"]
# an already-mined type-2 transaction (a stranger's): its raw bytes, resubmitted, can never be included again (nonce used)
raw=None; txh=None
for h in blk["transactions"][:40]:
    st,b = rpc(T,"eth_getRawTransactionByHash",[h]); r=json.loads(b).get("result")
    if r and r.startswith("0x02"): raw=r; txh=h; break
print("mined tx", txh, "block", int(blk["number"],16), "raw bytes", (len(raw)-2)//2 if raw else None)
for url in ["https://rpc.mevblocker.io", T]:
    for name, p in [("garbage 0x00", ["0x00"]), ("an already-mined transaction (nonce used)", [raw])]:
        st,b = rpc(url,"eth_sendRawTransaction",p)
        print(f"{url} {name}: HTTP {st} {b[:300]!r}")
        time.sleep(1)
