import json, time, urllib.request
def rpc(url, method, params):
    body = json.dumps({"jsonrpc":"2.0","id":1,"method":method,"params":params}).encode()
    req = urllib.request.Request(url, data=body, headers={"content-type":"application/json","user-agent":"rook-probe/1"})
    try:
        with urllib.request.urlopen(req, timeout=30) as r: return r.status, r.read()
    except urllib.error.HTTPError as e: return e.code, e.read()
def rlp(x):
    if isinstance(x, bytes):
        if len(x)==1 and x[0]<0x80: return x
        return enc_len(len(x),0x80)+x
    out=b"".join(rlp(i) for i in x); return enc_len(len(out),0xc0)+out
def enc_len(n,off):
    if n<56: return bytes([off+n])
    b=n.to_bytes((n.bit_length()+7)//8,'big'); return bytes([off+55+len(b)])+b
def q(h): v=int(h,16); return b"" if v==0 else v.to_bytes((v.bit_length()+7)//8,'big')
def hb(h): return bytes.fromhex(h[2:])
T="https://gateway.tenderly.co/public/mainnet"
st,b = rpc(T,"eth_getBlockByNumber",["latest",True]); blk=json.loads(b)["result"]
tx=next(t for t in blk["transactions"] if t["type"]=="0x2" and not t.get("accessList") and len(t["input"])<200)
fields=[q(tx["chainId"]),q(tx["nonce"]),q(tx["maxPriorityFeePerGas"]),q(tx["maxFeePerGas"]),q(tx["gas"]),hb(tx["to"]),q(tx["value"]),hb(tx["input"]),[],q(tx["v"]),q(tx["r"]),q(tx["s"])]
raw="0x02"+rlp(fields).hex()
import hashlib
try:
    from Crypto.Hash import keccak
    k=keccak.new(digest_bits=256); k.update(bytes.fromhex(raw[2:])); print("keccak matches the mined hash:", "0x"+k.hexdigest()==tx["hash"])
except Exception as e: print("(no keccak lib to check)", e)
print("# replay of a mined transaction", tx["hash"], "block", int(blk["number"],16), time.strftime("%H:%M:%S UTC", time.gmtime()))
time.sleep(13)
for url in ["https://rpc.mevblocker.io", T]:
    st,b = rpc(url,"eth_sendRawTransaction",[raw])
    print(f"{url} already-mined transaction (nonce used): HTTP {st} {b[:300]!r}")
