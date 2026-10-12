// Holds the page's own functions (between its lib-start and lib-end markers) to the fueler's tested encoder, then runs the page's
// Relay judge on a live, read-only quote and on tampered copies. Nothing is signed or sent.
//   cd ops && npx tsx rehearsal/cketh/check.mts
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { depositEthData } from "../../src/fuel/pinned.ts";

const html = readFileSync(new URL("./index.html", import.meta.url), "utf8");
const lib = html.slice(html.indexOf("// lib-start"), html.indexOf("// lib-end"));
const L = new Function(`${lib}; return { calldata, toWei, principalBytes, relayBody, judgeRelay, HELPER };`)();

for (const p of ["ih4wp-rvfmw-iimsu-qnmdv-63mci-c6s5k-6dfao-fmrkp-3nsmu-mf5ul-hqe", "ryjl3-tyaaa-aaaaa-aaaba-cai", "rrkah-fqaaa-aaaaa-aaaaq-cai", "2vxsx-fae", "aaaaa-aa"])
  assert.equal(L.calldata(p), depositEthData(p), p);
assert.throws(() => L.calldata("ih4wp-rvfmw-iimsu-qnmdv-63mci-c6s5k-6dfao-fmrkp-3nsmu-mf5ul-hqf"), /exact text/, "a typo in the last character");
assert.throws(() => L.calldata("ih4wp-rvfmw-iimsu-qnmdv-63mci-c6s5k-6dfao-fmrkp-3nsmu-mf5ul-hqe".replace("rvfmw", "rvfmx")), /checksum/, "a typo in the middle");
assert.throws(() => L.calldata("hello world"), /not a principal/);
assert.throws(() => L.calldata("ih4wprvfmw-iimsu-qnmdv-63mci-c6s5k-6dfao-fmrkp-3nsmu-mf5ul-hqe"), /exact text/, "a missing dash");
assert.throws(() => L.calldata("ih4wp-rvfmw-iimsu-qnmdv-63mci-c6s5k-6dfao-fmrkp-3nsmu-mf5ul-hqa"), /checksum|exact text/);
assert.equal(L.toWei("0.004"), 4_000_000_000_000_000n);
assert.equal(L.toWei("1"), 10n ** 18n);
assert.throws(() => L.toWei("1e-3"));
console.log("encoding: the page matches the fueler's encoder on 5 principals; typos refused");

const USER = "0x000000000000000000000000000000000000dEaD", P = "ih4wp-rvfmw-iimsu-qnmdv-63mci-c6s5k-6dfao-fmrkp-3nsmu-mf5ul-hqe", WEI = 4_000_000_000_000_000n;
for (const origin of [4663, 8453]) {
  const r = await fetch("https://api.relay.link/quote/v2", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(L.relayBody(USER, origin, WEI, P)) });
  const q = await r.json();
  assert.ok(r.ok, JSON.stringify(q).slice(0, 200));
  const tx = L.judgeRelay(q, USER, origin, WEI, P);
  console.log(`relay ${origin}: accepted, pay ${Number(BigInt(tx.value)) / 1e18} ETH to ${tx.to}`);
  const bad = (f: (x: any) => void, re: RegExp) => { const c = structuredClone(q); f(c); assert.throws(() => L.judgeRelay(c, USER, origin, WEI, P), re); };
  bad((c) => { c.steps[0].items[0].data.to = "0x" + "66".repeat(20); }, /depository/);
  bad((c) => { c.steps[0].items[0].data.chainId = 1; }, /chain/);
  bad((c) => { c.steps[0].items[0].data.value = String(WEI * 2n); }, /3 %/);
  bad((c) => { c.protocol.v2.orderData.output.calls = []; }, /helper deposit/);
  assert.throws(() => L.judgeRelay(q, USER, origin, WEI, "ryjl3-tyaaa-aaaaa-aaaba-cai"), /helper deposit/, "a quote for another principal");
  console.log(`relay ${origin}: tampered quotes refused (depository, chain, fee, missing call, other principal)`);
}
console.log("CHECK: PASS");
