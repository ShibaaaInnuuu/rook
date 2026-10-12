import json, time, urllib.request, concurrent.futures as cf
def rpc(url, method, params):
    body = json.dumps({"jsonrpc":"2.0","id":1,"method":method,"params":params}).encode()
    req = urllib.request.Request(url, data=body, headers={"content-type":"application/json","user-agent":"rook-probe/1"})
    try:
        with urllib.request.urlopen(req, timeout=20) as r: return r.status, r.read()
    except urllib.error.HTTPError as e: return e.code, e.read()
    except Exception as e: return 0, str(e)[:100].encode()
C=["https://rpc.mainnet.chain.robinhood.com","https://rpc.ordofi.network","https://robinhood.drpc.org","https://robinhood-rpc.publicnode.com",
 "https://4663.rpc.thirdweb.com","https://rpc.ankr.com/robinhood","https://robinhood.gateway.tenderly.co","https://robinhood-mainnet.gateway.tenderly.co",
 "https://robinhood.api.onfinality.io/public","https://robinhood.blockpi.network/v1/rpc/public","https://rpc.robinhood.xyz","https://robinhood-mainnet.public.blastapi.io",
 "https://robinhood.rpc.subquery.network/public","https://1rpc.io/robinhood","https://robinhood.nodies.app","https://rpc.chain.robinhood.com","https://mainnet.rpc.robinhood.com"]
US="0x000000000000000000000000000000000000dead"
print("# 4663 keyless providers: state depth", time.strftime("%Y-%m-%d %H:%M:%S UTC", time.gmtime()))
def one(u):
    st,b=rpc(u,"eth_chainId",[])
    try: cid=int(json.loads(b)["result"],16)
    except Exception: return f"{u}: no chainId (HTTP {st} {b[:80]!r})"
    if cid!=4663: return f"{u}: chain {cid}"
    st,b=rpc(u,"eth_blockNumber",[]); head=int(json.loads(b)["result"],16)
    out=[]
    for D in [6000,10000,30000,100000,1000000]:
        st,b=rpc(u,"eth_getTransactionCount",[US,hex(head-D)])
        try: j=json.loads(b); out.append(f"{D}:{'ok' if 'result' in j else 'ERR '+str(j.get('error',{}).get('message',''))[:40]}")
        except Exception: out.append(f"{D}:HTTP{st}")
    return f"{u}: chain 4663 head {head}; nonce at depth " + ", ".join(out)
with cf.ThreadPoolExecutor(8) as ex:
    for r in ex.map(one,C): print(r)
