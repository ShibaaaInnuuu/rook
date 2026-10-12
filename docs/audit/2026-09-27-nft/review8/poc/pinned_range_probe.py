import json, urllib.request
PIN_FROM, PIN_TO = 26_012_410, 26_012_909
TX="0x17133e8bbc4d8f1fbdf25e1698229bbe9f732d085069b82dd03cd75cdd55c093"
MIL="0x5af0d9827e0c53e4799bb226655a1de152a425a5"; TR="0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef"
def rpc(u,m,p):
    r=urllib.request.Request(u,data=json.dumps({"jsonrpc":"2.0","id":1,"method":m,"params":p}).encode(),headers={"content-type":"application/json","user-agent":"curl/8"})
    return json.load(urllib.request.urlopen(r,timeout=60))
for u in ["https://rpc.mevblocker.io","https://gateway.tenderly.co/public/mainnet","https://ethereum-rpc.publicnode.com"]:
    h=int(rpc(u,"eth_blockNumber",[])["result"],16)
    L=rpc(u,"eth_getLogs",[{"fromBlock":hex(PIN_FROM),"toBlock":hex(PIN_TO),"address":MIL,"topics":[TR]}])
    logs=L.get("result"); 
    R=rpc(u,"eth_getTransactionReceipt",[TX]).get("result")
    txs=sorted({l["transactionHash"] for l in logs}) if logs else None
    print(u.split('/')[2], "head",h,"depth",h-PIN_TO,"logs",len(logs) if logs is not None else L, "first tx", logs[0]["transactionHash"][:12] if logs else None,
      "rcpt", R and (int(R["blockNumber"],16), R["status"], sum(1 for l in R["logs"] if l["address"].lower()==MIL and l["topics"][0]==TR)))
