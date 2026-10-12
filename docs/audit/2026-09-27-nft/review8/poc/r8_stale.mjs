import { ethNftAlerts } from "./src/ethnft.mjs";
// a sale-scan entry of a collection no longer configured (or a scan that stopped for good): current frozen at the last pass
const col = Array.from({ length: 20 }, () => 0xaa);
const state = { buys: [], deliveries: [], readings: [], eth_out: 0n, deposits: [], pending: [], sale_scan: [[
  { collection: col, up: 26_000_000n, hole: [], lost: [], behind: [100n], rfail: [], current: [26_000_000n], hole_deep: [], rbusy: [], busy: [] }]] };
for (const days of [0.5, 2, 30]) {
  const head = 26_000_000 + 128 + Math.round(days * 7_200);
  const r = ethNftAlerts({ buyer: "b", state, nowS: 1_800_000_000, ethHead: BigInt(head) });
  console.log(days, "days:", r.alerts.filter((a) => a.key.startsWith("eth-scan")).map((a) => `${a.sev} ${a.key}`).join(", "));
}
