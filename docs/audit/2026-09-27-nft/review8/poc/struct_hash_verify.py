import json, subprocess
C="/Users/jlohayon/.foundry/bin/cast"
def k(h): return subprocess.check_output([C,"keccak",h]).decode().strip()[2:]
def ks(s): return subprocess.check_output([C,"keccak",s]).decode().strip()[2:]
t=json.load(open('tx.json')); r=json.load(open('rcpt.json'))['result']
inp=t['input']
real=open('/Users/jlohayon/Documents/Rook-nft/autobuyer/canister-rs/tests/audit_nft_r7.rs').read().split('REAL_EXEC_INPUT: &str = "')[1].split('"')[0]
print("input == test const:", inp.lower()==real.lower(), "status", r['status'])
a=bytes.fromhex(inp[10:])
W=lambda i:a[i:i+32]; N=lambda i:int.from_bytes(W(i),'big')
req=N(0); calls_at=req+N(req); nonce=W(req+32); exp=W(req+64); n=N(calls_at); base=calls_at+32
cth=ks("Call(address to,bytes data,uint256 value,bool allowFailure)")
rth=ks("CallRequest(Call[] calls,uint256 nonce,uint256 expiration)Call(address to,bytes data,uint256 value,bool allowFailure)")
hs=""
for i in range(n):
  at=base+N(base+32*i); to=W(at); dat=at+N(at+32); val=W(at+64); fl=W(at+96); L=N(dat); data=a[dat+32:dat+32+L]
  print("call",i,to.hex()[24:],int.from_bytes(val,'big'),fl[-1],len(data))
  hs+=k("0x"+cth+to.hex()+k("0x"+data.hex())+val.hex()+fl.hex())
sh=k("0x"+rth+k("0x"+hs)+nonce.hex()+exp.hex())
print("struct hash",sh)
for l in r['logs']:
  print("log",l['address'],l['topics'],l['data'][:66])
