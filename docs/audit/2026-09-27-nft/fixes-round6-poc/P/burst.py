import json, time, urllib.request, concurrent.futures as cf, collections
MIL = "0x5af0d9827e0c53e4799bb226655a1de152a425a5"
T = "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef"
def rpc(url, method, params):
    body = json.dumps({"jsonrpc":"2.0","id":1,"method":method,"params":params}).encode()
    req = urllib.request.Request(url, data=body, headers={"content-type":"application/json","user-agent":"rook-probe/1"})
    try:
        with urllib.request.urlopen(req, timeout=30) as r: return r.status, r.read(), dict(r.headers)
    except urllib.error.HTTPError as e: return e.code, e.read(), dict(e.headers)
    except Exception as e: return 0, str(e).encode(), {}
print("# bursts", time.strftime("%Y-%m-%d %H:%M:%S UTC", time.gmtime()))
for url in ["https://gateway.tenderly.co/public/mainnet", "https://rpc.mevblocker.io"]:
    st,b,_ = rpc(url,"eth_blockNumber",[]); head=int(json.loads(b)["result"],16)
    tally=collections.Counter(); sample429=None; okrounds=0
    for i in range(20):
        D = 1000 + i*2500
        p=[{"fromBlock":hex(head-D-499),"toBlock":hex(head-D),"address":MIL,"topics":[T]}]
        with cf.ThreadPoolExecutor(13) as ex: rs=list(ex.map(lambda _: rpc(url,"eth_getLogs",p), range(13)))
        codes=[r[0] for r in rs]; tally.update(codes)
        ok=sum(1 for c in codes if c==200)
        if ok>=9: okrounds+=1
        for r in rs:
            if r[0]!=200 and sample429 is None:
                sample429=(r[0], r[1][:300], {k:v for k,v in r[2].items() if 'rate' in k.lower() or 'retry' in k.lower()})
        time.sleep(1)
    print(f"{url}: 20 rounds of 13 simultaneous eth_getLogs, 1 s apart: HTTP tally {dict(tally)}; rounds with >= 9 of 13 ok (IC consensus possible): {okrounds}/20")
    if sample429: print("  a refusal:", sample429)
