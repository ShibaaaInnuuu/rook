// docs/v7/cycles-from-fees.md §3.4, §4.2 item 7: the fueler watched from the chain. No network.
import assert from "node:assert/strict";
import { test } from "node:test";
import { due } from "../src/alerts.mjs";
import { validate } from "../src/config.mjs";
import { checkFueler } from "../src/fueler.mjs";

const OPS = "0x" + "ab".repeat(20), NOW = 1_791_600_000_000;
const cfg = { fueler: { ops: OPS, status_url: "https://fueler.example/status", rpcs_4663: ["https://a.example", "https://b.example"], rpcs_1: ["https://c.example", "https://d.example"] } };

/** A fake network: each RPC's nonce per chain, and the fueler's /status. `order` records what was read, in order. */
function world({ nonce = { 4663: 5, 1: 2 }, behind = {}, status, balance = 10n ** 17n, statusCode = 200 }) {
  const order = [];
  const http = async (url, init) => {
    if (url === cfg.fueler.status_url) { order.push("status"); return { status: statusCode, json: async () => status }; }
    const body = JSON.parse(init.body), chain = /\/\/(a|b)\./.test(url) ? 4663 : 1;
    order.push(`rpc:${chain}`);
    const n = behind[url] ?? nonce[chain];
    const result = body.method === "eth_getTransactionCount" ? `0x${n.toString(16)}` : `0x${balance.toString(16)}`;
    return { ok: true, status: 200, json: async () => ({ jsonrpc: "2.0", id: body.id, result }), text: async () => "" };
  };
  return { http, order };
}
const st = (o = {}) => ({ address: OPS, nonces: { 4663: 5, 1: 2 }, alerts: [], ...o });

test("quiet when the chain's nonces are the fueler's journaled ones; the chain is read before /status", async () => {
  const w = world({ status: st() });
  const r = await checkFueler({ cfg, now: NOW, http: w.http });
  assert.deepEqual(r.alerts, []);
  assert.ok(w.order.indexOf("status") > w.order.lastIndexOf("rpc:1"), w.order.join(","));
});

test("ops-unexpected-tx: a nonce past the fueler's journal on either chain is SEV1, persistent, and says what to do", async () => {
  for (const chain of [4663, 1]) {
    const w = world({ nonce: { 4663: 5, 1: 2, [chain]: chain === 4663 ? 6 : 3 }, status: st() });
    const r = await checkFueler({ cfg, now: NOW, http: w.http });
    const a = r.alerts.find((x) => x.key.startsWith(`ops-unexpected-tx:${chain}:`));
    assert.ok(a, `chain ${chain}`);
    assert.equal(a.sev, "SEV1"); assert.equal(a.persistent, true);
    assert.match(a.text, /setOps/);
  }
  // one provider behind and one ahead: the higher count is the evidence
  const w = world({ nonce: { 4663: 6, 1: 2 }, behind: { "https://a.example": 5 }, status: st() });
  assert.ok((await checkFueler({ cfg, now: NOW, http: w.http })).alerts.some((x) => x.key.startsWith("ops-unexpected-tx:4663")));
});

test("the fueler's own alerts are relayed with their severity; a /status naming another address is SEV1; the ops wallet under its floor is SEV3", async () => {
  const w = world({ status: st({ address: "0x" + "cd".repeat(20), alerts: [{ kind: "cycles-autofund-cap", sev: "SEV2", text: "the 7-day cap", since: 1 },
    { kind: "cycles-income-short", sev: "SEV3", text: "fees short", since: 2 }, { kind: "fueler-halt", sev: "SEV1", text: "halted", since: 3 }] }), balance: 10n ** 15n });
  const r = await checkFueler({ cfg, now: NOW, http: w.http });
  const by = (k) => r.alerts.find((a) => a.key.startsWith(k));
  assert.equal(by("fueler-address:").sev, "SEV1");
  assert.equal(by("fueler:cycles-autofund-cap").sev, "SEV2");
  assert.equal(by("fueler:cycles-income-short").sev, "SEV3");
  assert.equal(by("fueler:fueler-halt").sev, "SEV1");
  assert.equal(by("fueler-ops-low:").sev, "SEV3");
  assert.doesNotThrow(() => due(r.alerts, {}, NOW), "every alert has a key and a known kind");
});

test("a fueler that does not answer: no unexpected-tx page on stale numbers (liveness pages it)", async () => {
  const w = world({ nonce: { 4663: 9, 1: 9 }, status: null, statusCode: 503 });
  assert.deepEqual((await checkFueler({ cfg, now: NOW, http: w.http })).alerts, []);
});

test("config: the fueler block needs the ops address, the /status URL, and two providers per chain", () => {
  const base = { network: "ic", timelock: "rrkah-fqaaa-aaaaa-aaaaq-cai", expect: { controllers: [] } };
  const errs = (f) => { try { validate({ ...base, fueler: f }); return []; } catch (e) { return String(e.message).split("\n- ").filter((x) => x.startsWith("fueler")); } };
  assert.deepEqual(errs(cfg.fueler), []);
  assert.ok(errs({ ...cfg.fueler, ops: "nope" }).length);
  assert.ok(errs({ ...cfg.fueler, rpcs_1: ["https://c.example"] }).length);
  assert.ok(errs({ ...cfg.fueler, extra: 1 }).length);
});

// ---- docs/v9/fueler-solana.md §6: the Solana home ----------------------------------------------------------------------------------

const HOT = "8rxE3gMhWTmYmBCaP6kRcCk1cLoxyQU4uYn5bq26VZZa", OTHER = "GGgXnNgtY4ySfz8PLAAwkoh8Ztj7p6jrM9towSaa6Wuy";
const scfg = { buyer: "ryjl3-tyaaa-aaaaa-aaaba-cai", fueler: { ops: OPS, sol_ops: HOT, status_url: "https://fueler.example/status",
  rpcs_sol: ["https://s1.example", "https://s2.example"], rpcs_1: ["https://c.example", "https://d.example"] } };

/** Solana providers: the signatures touching the hot key, who signed each, its balance; Ethereum's nonce; the fueler's /status. */
function solWorld({ sigs = [], signers = {}, lamports = 50_000_000n, nonce = 2, status, seenBy = {} }) {
  const order = [];
  const http = async (url, init) => {
    if (url === scfg.fueler.status_url) { order.push("status"); return { status: status ? 200 : 503, json: async () => status }; }
    const body = JSON.parse(init.body);
    order.push(`rpc:${body.method}`);
    let result;
    if (body.method === "eth_getTransactionCount") result = `0x${nonce.toString(16)}`;
    else if (body.method === "getSignaturesForAddress") result = sigs.filter((s) => !seenBy[s] || seenBy[s] === url).map((signature) => ({ signature }));
    else if (body.method === "getBalance") result = { value: Number(lamports) };
    else if (body.method === "getTransaction") {
      const sig = body.params[0];
      result = { transaction: { message: { header: { numRequiredSignatures: 1 }, accountKeys: [signers[sig] ?? OTHER, HOT] } } };
    } else throw new Error(body.method);
    return { ok: true, status: 200, json: async () => ({ jsonrpc: "2.0", id: body.id, result }), text: async () => "" };
  };
  return { http, order };
}
const sst = (o = {}) => ({ address: OPS, solAddress: HOT, nonces: { 1: 2 }, alerts: [], sol: { signatures: [] }, conversions: [], run: null, ...o });

test("Solana: the first poll takes the baseline; a later signature the hot key signed that the fueler never journaled is SEV1 sol-unexpected-tx", async () => {
  const w0 = solWorld({ sigs: ["fund1"], status: sst() });
  const r0 = await checkFueler({ cfg: scfg, now: NOW, http: w0.http });
  assert.deepEqual(r0.alerts, []);
  assert.deepEqual(r0.next.solSeen, ["fund1"]);
  assert.ok(w0.order.indexOf("status") > w0.order.lastIndexOf("rpc:getSignaturesForAddress"), "the chain before /status");
  // the fueler's own (journaled) signature, an incoming transfer (signed by another), and one the hot key signed behind its back
  const w1 = solWorld({ sigs: ["mine", "incoming", "stolen", "fund1"], signers: { mine: HOT, stolen: HOT }, status: sst({ sol: { signatures: ["mine"] } }) });
  const r1 = await checkFueler({ cfg: scfg, prev: r0.next, now: NOW + 60_000, http: w1.http });
  const a = r1.alerts.filter((x) => x.key.startsWith("sol-unexpected-tx:"));
  assert.deepEqual(a.map((x) => x.key), ["sol-unexpected-tx:stolen"]);
  assert.equal(a[0].sev, "SEV1"); assert.equal(a[0].persistent, true); assert.match(a[0].text, /treasury_set_ops/); assert.doesNotMatch(a[0].text, /allocate|rook_core/);
  // judged once: the next poll does not fetch them again
  const w2 = solWorld({ sigs: ["mine", "incoming", "stolen", "fund1"], signers: { mine: HOT, stolen: HOT }, status: sst({ sol: { signatures: ["mine"] } }) });
  await checkFueler({ cfg: scfg, prev: r1.next, now: NOW + 120_000, http: w2.http });
  assert.ok(!w2.order.includes("rpc:getTransaction"), w2.order.join(","));
  // one provider that sees a foreign signature is enough
  const w3 = solWorld({ sigs: ["fund1", "late"], signers: { late: HOT }, seenBy: { late: "https://s2.example" }, status: sst() });
  assert.ok((await checkFueler({ cfg: scfg, prev: r0.next, now: NOW, http: w3.http })).alerts.some((x) => x.key === "sol-unexpected-tx:late"));
});

test("Solana: the hot key under its floor is SEV3; another hot key in /status is SEV1; a stuck leg and a cap hit are relayed; Ethereum's nonce still watched", async () => {
  const w = solWorld({ lamports: 10_000_000n, nonce: 3, status: sst({ solAddress: OTHER, alerts: [{ kind: "cycles-autofund-failed", sev: "SEV2", text: "run run-1: the Relay fill has not reached", since: 1 },
    { kind: "cycles-autofund-cap", sev: "SEV2", text: "the 7-day cap", since: 2 }] }) });
  const r = await checkFueler({ cfg: scfg, prev: { solSeen: [] }, now: NOW, http: w.http });
  const by = (k) => r.alerts.find((a) => a.key.startsWith(k));
  assert.equal(by("fueler-sol-low:").sev, "SEV3");
  assert.match(by("fueler-sol-low:").text, /0\.01000 SOL/);
  assert.equal(by("fueler-sol-address:").sev, "SEV1");
  assert.equal(by("fueler:cycles-autofund-failed").sev, "SEV2");
  assert.equal(by("fueler:cycles-autofund-cap").sev, "SEV2");
  assert.equal(by("ops-unexpected-tx:1:").sev, "SEV1");
  assert.ok(!r.alerts.some((a) => a.key.startsWith("ops-unexpected-tx:4663")), "nothing on 4663");
  assert.doesNotThrow(() => due(r.alerts, {}, NOW));
});

test("Solana: the buyer under the trigger for 12 h with no run and no conversion since pages fueler-not-fueling (SEV2); a conversion clears it", async () => {
  const ic = { cycles: async () => 30_000_000_000_000n };
  const w = solWorld({ status: sst({ lastDecision: "blocked: the ops wallet holds 0.04 SOL" }) });
  const r0 = await checkFueler({ cfg: scfg, prev: { solSeen: [] }, now: NOW, http: w.http, ic });
  assert.equal(r0.next.lowSince, NOW);
  assert.ok(!r0.alerts.some((a) => a.key.startsWith("fueler-not-fueling")));
  const r1 = await checkFueler({ cfg: scfg, prev: r0.next, now: NOW + 13 * 3_600_000, http: w.http, ic });
  const a = r1.alerts.find((x) => x.key.startsWith("fueler-not-fueling"));
  assert.equal(a.sev, "SEV2"); assert.match(a.text, /0\.04 SOL/);
  const w2 = solWorld({ status: sst({ conversions: [{ at: String((NOW + 3_600_000) / 1000), wei: "1", cycles: "1" }] }) });
  assert.ok(!(await checkFueler({ cfg: scfg, prev: r0.next, now: NOW + 13 * 3_600_000, http: w2.http, ic })).alerts.some((x) => x.key.startsWith("fueler-not-fueling")));
  const up = await checkFueler({ cfg: scfg, prev: r1.next, now: NOW + 14 * 3_600_000, http: w.http, ic: { cycles: async () => 50_000_000_000_000n } });
  assert.equal(up.next.lowSince, null, "back above the trigger: the clock stops");
});

test("config: the Solana fueler block needs sol_ops (base58) and two Solana providers, not 4663's", () => {
  const base = { network: "ic", timelock: "rrkah-fqaaa-aaaaa-aaaaq-cai", expect: { controllers: [] } };
  const errs = (f) => { try { validate({ ...base, fueler: f }); return []; } catch (e) { return String(e.message).split("\n- ").filter((x) => x.startsWith("fueler")); } };
  assert.deepEqual(errs(scfg.fueler), []);
  assert.deepEqual(errs({ ...scfg.fueler, min_sol: 0.02, trigger_t: 40, stuck_h: 12 }), []);
  assert.ok(errs({ ...scfg.fueler, sol_ops: "0xabc" }).length);
  assert.ok(errs({ ...scfg.fueler, rpcs_sol: ["https://s1.example"] }).length);
  assert.ok(errs({ ...scfg.fueler, stuck_h: 0 }).length);
});
