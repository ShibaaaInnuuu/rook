#!/usr/bin/env python3
# Read-only JSON-RPC probe of the canister's recommended Ethereum pair (fix round 6, B6-M-1).
import json, sys, time, urllib.request, concurrent.futures as cf, hashlib
URLS = ["https://rpc.mevblocker.io", "https://gateway.tenderly.co/public/mainnet"]
MIL = "0x5af0d9827e0c53e4799bb226655a1de152a425a5"
T = "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef"
USDT = "0xdac17f958d2ee523a2206206994597c13d831ec7"
def rpc(url, method, params, timeout=60):
    body = json.dumps({"jsonrpc":"2.0","id":1,"method":method,"params":params}).encode()
    req = urllib.request.Request(url, data=body, headers={"content-type":"application/json","user-agent":"rook-probe/1"})
    t0=time.time()
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            raw = r.read(); st = r.status
    except urllib.error.HTTPError as e:
        raw = e.read(); st = e.code
    except Exception as e:
        return (None, 0, str(e)[:120], time.time()-t0)
    try: j = json.loads(raw)
    except Exception: j = {"_raw": raw[:200].decode(errors="replace")}
    return (j, st, len(raw), time.time()-t0)
def short(j):
    if j is None: return "NONE"
    if "result" in j:
        r=j["result"]
        if isinstance(r,list): return "ok %d" % len(r)
        if r is None: return "null"
        return "ok"
    return "ERR " + json.dumps(j.get("error", j))[:160]
def main():
    print("# probe", time.strftime("%Y-%m-%d %H:%M:%S UTC", time.gmtime()))
    for u in URLS:
        j,st,n,dt = rpc(u,"eth_blockNumber",[])
        head = int(j["result"],16)
        fj,_,_,_ = rpc(u,"eth_getBlockByNumber",["finalized",False])
        fin = int(fj["result"]["number"],16)
        print(f"\n## {u} head={head} finalized={fin}")
        for D in [1000, 10000, 20000, 40000, 52000, 60000, 70000, 100000, 130000]:
            fr, to = head-D-499, head-D
            j,st,n,dt = rpc(u,"eth_getLogs",[{"fromBlock":hex(fr),"toBlock":hex(to),"address":MIL,"topics":[T]}])
            print(f"  logs depth {D:>6} (500 blk, Milady Transfer): HTTP {st} {short(j)} {n}B {dt:.1f}s")
        # a 10,000-block Milady range at 60,000 depth
        fr,to = head-70000, head-60001
        j,st,n,dt = rpc(u,"eth_getLogs",[{"fromBlock":hex(fr),"toBlock":hex(to),"address":MIL,"topics":[T]}])
        print(f"  logs 10,000-block range at depth 60,000: HTTP {st} {short(j)} {n}B {dt:.1f}s")
        for D in [1000, 30000, 52000, 60000, 70000]:
            bj,_,_,_ = rpc(u,"eth_getBlockByNumber",[hex(head-D),False])
            txs = bj["result"]["transactions"]
            tx = txs[len(txs)//2]
            rj,st,n,dt = rpc(u,"eth_getTransactionReceipt",[tx])
            tj,st2,_,_ = rpc(u,"eth_getTransactionByHash",[tx])
            print(f"  receipt depth {D:>6}: HTTP {st} {short(rj)} {n}B ; tx by hash {short(tj)}")
        # the result cap: USDT Transfer over 1000 dense blocks
        fr,to = head-2000, head-1001
        j,st,n,dt = rpc(u,"eth_getLogs",[{"fromBlock":hex(fr),"toBlock":hex(to),"address":USDT,"topics":[T]}],timeout=90)
        print(f"  USDT Transfer 1,000 blocks: HTTP {st} {short(j)} {n}B {dt:.1f}s")
        # the marketplaces' sale logs (Seaport 1.5/1.6 OrderFulfilled) over 250 and 500 blocks
        SP=["0x0000000000000068f116a894984e2db1123eb395","0x00000000000000adc04c56bf30ac9d3c0aaf14dc"]
        OF="0x9d9af8e38d66c62e2c12f0225249fd9d721c54b83f48d9352c97c6cacdcb6f31"
        for N in [250, 500]:
            j,st,n,dt = rpc(u,"eth_getLogs",[{"fromBlock":hex(head-1000-N+1),"toBlock":hex(head-1000),"address":SP,"topics":[[OF]]}],timeout=90)
            print(f"  Seaport OrderFulfilled {N} blocks: HTTP {st} {short(j)} {n}B {dt:.1f}s")
    # 13-way bursts (an IC outcall is made by each replica of a 13-node subnet)
    for u in URLS:
        j,_,_,_ = rpc(u,"eth_blockNumber",[]); head=int(j["result"],16)
        for name, m, p in [("eth_getLogs 500 blk @52k", "eth_getLogs", [{"fromBlock":hex(head-52499),"toBlock":hex(head-52000),"address":MIL,"topics":[T]}]),
                           ("eth_getTransactionReceipt @52k", "eth_getTransactionReceipt", None)]:
            if p is None:
                bj,_,_,_ = rpc(u,"eth_getBlockByNumber",[hex(head-52000),False]); p=[bj["result"]["transactions"][0]]
            with cf.ThreadPoolExecutor(13) as ex:
                rs = list(ex.map(lambda _: rpc(u,m,p), range(13)))
            bodies = set(hashlib.sha256(json.dumps(r[0],sort_keys=True).encode()).hexdigest()[:8] for r in rs)
            print(f"  13-burst {u.split('/')[2]} {name}: HTTP {[r[1] for r in rs]} ok={sum(1 for r in rs if r[0] and 'result' in r[0])}/13 distinct bodies={len(bodies)}")
            time.sleep(2)
main()
