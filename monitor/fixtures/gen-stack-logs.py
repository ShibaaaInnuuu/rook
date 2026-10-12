# The stack-event fixtures (fixtures/stack-logs.json), encoded by foundry's cast, not by the monitor. Run from monitor/.
import json, os, subprocess
C = lambda *a: subprocess.check_output([os.path.expanduser("~/.foundry/bin/cast"), *a]).decode().strip()
topic = lambda sig: C("keccak", sig)
pad = lambda a: "0x" + a[2:].lower().rjust(64, "0")
num = lambda n: "0x" + format(n, "064x")
REG="0x00000000000000000000000000000000000000d1"; MOD="0x00000000000000000000000000000000000000bb"; WR="0x00000000000000000000000000000000000000aa"
RAF="0x00000000000000000000000000000000000000e1"; BUR="0x00000000000000000000000000000000000000e2"; YOY="0x00000000000000000000000000000000000000e3"
FAC="0x00000000000000000000000000000000000000f1"; COIN="0x0000000000000000000000000000000000000c01"; KEEP="0x00000000000000000000000000000000000000c3"
GUARD="0x00000000000000000000000000000000000000a9"
n=[0]
def log(addr, topics, data, block):
    n[0]+=1
    return {"address": addr, "topics": topics, "data": data, "blockNumber": hex(block), "transactionHash": "0x" + format(n[0], "064x"),
            "logIndex": "0x0", "blockHash": "0x" + format(0xb0000 + block, "064x"), "removed": False}
FP = topic("FloorPosted(uint256,uint256,uint256,uint16,uint32)")
fp = lambda t, a, m, c, d: C("abi-encode", "f(uint256,uint256,uint256,uint16,uint32)", str(t), str(a), str(m), str(c), str(d))
E = 10**18
out = {"note": "stack events ABI-encoded by foundry's `cast abi-encode` and `cast keccak` (not by the monitor); regenerate with `python3 fixtures/gen-stack-logs.py` from monitor/ (needs foundry)",
  "stack": {"chainId": 4663, "label": "stack", "deployedAtBlock": 71200000, "registry": REG, "treasury": "0x00000000000000000000000000000000000000c7",
    "poolManager": "0x8366a39cc670b4001a1121b8f6a443a643e40951", "owner": "0x00000000000000000000000000000000000005af", "keeper": KEEP,
    "collections": [{"slug": "frogs", "kind": "inscriptions", "wrapper": WR, "floorModule": MOD, "raffleVault": RAF, "burnVault": BUR, "yoyoVault": YOY,
      "factory": FAC, "lock": "0x00000000000000000000000000000000000000f2", "router": "0x00000000000000000000000000000000000000f3", "mintDesk": "0x00000000000000000000000000000000000000f4"}]},
  "coin": COIN, "guardian": GUARD, "logs": {}}
L = out["logs"]
L["floor_ok"] = log(MOD, [FP], fp(65*E//100, 65*E//100, 10, 8000, 12), 71200010)
L["floor_lowconf"] = log(MOD, [FP], fp(65*E//100, 65*E//100, 0, 0, 12), 71200020)
L["floor_clamped_1"] = log(MOD, [FP], fp(40*E//100, 6175*E//10000, 500, 8000, 12), 71200030)
L["floor_clamped_2"] = log(MOD, [FP], fp(40*E//100, 5866*E//10000, 500, 8000, 12), 71200040)
CP = topic("CapPosted(uint256,uint64)")
L["cap_raffle"] = log(RAF, [CP], C("abi-encode", "f(uint256,uint64)", str(715*E//1000), "1790000000"), 71200050)
L["cap_high"] = log(BUR, [CP], C("abi-encode", "f(uint256,uint64)", str(E), "1790000000"), 71200051)
L["cap_yoyo"] = log(YOY, [CP], C("abi-encode", "f(uint256,uint64)", str(715*E//1000), "1790000000"), 71200052)
L["draw_announced"] = log(RAF, [topic("DrawAnnounced(uint256,address,uint64)"), num(0), pad(COIN)], C("abi-encode", "f(uint64)", "21000000"), 71200060)
L["snapshot_fixed"] = log(RAF, [topic("SnapshotFixed(uint256,uint64)"), num(0)], C("abi-encode", "f(uint64)", "1789990000"), 71200070)
L["draw_opened"] = log(RAF, [topic("DrawOpened(uint256,address,bytes32,uint256,string,uint64)"), num(0), pad(COIN)],
  C("abi-encode", "f(bytes32,uint256,string,uint64)", "0x" + "ab"*32, "1000", "https://keeper.example/snapshots/s.json", "1790000900"), 71200070)
L["snapshot_fixed"]["transactionHash"] = L["draw_opened"]["transactionHash"]; L["draw_opened"]["logIndex"] = "0x1"
L["keeper_paused"] = log(REG, [topic("KeeperPaused(address,bool)"), pad(GUARD)], C("abi-encode", "f(bool)", "true"), 71200080)
L["guardian_set"] = log(REG, [topic("GuardianSet(address)")], C("abi-encode", "f(address)", GUARD), 71200081)
L["roles_proposed"] = log(REG, [topic("RolesProposed(address,address,address,uint64)")], C("abi-encode", "f(address,address,address,uint64)", "0x00000000000000000000000000000000000000c4", "0x00000000000000000000000000000000000000c5", "0x00000000000000000000000000000000000000c6", "1790604800"), 71200082)
out["topics"] = {"FloorPosted": FP, "CapPosted": CP, "KeeperPaused": topic("KeeperPaused(address,bool)"), "DrawOpened": topic("DrawOpened(uint256,address,bytes32,uint256,string,uint64)")}
out["sentinel_1000_1789990000"] = C("keccak", C("keccak", C("abi-encode", "f(address,uint256,uint256)", "0x0000000000000000000000000000000000000000", "1000", "1789990000")))
json.dump(out, open("fixtures/stack-logs.json", "w"), indent=1)
print("ok", out["sentinel_1000_1789990000"])
