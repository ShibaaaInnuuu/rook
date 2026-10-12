import assert from "node:assert/strict";
import { test } from "node:test";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import type { AddressInfo } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { generatePrivateKey, privateKeyToAccount } from "viem/accounts";
import { robustFloor, satsToWei, type Contents, type Sale } from "../src/floor.js";
import { parseOwListings, parseOwSales } from "../src/floorsources.js";
import { planPost, type Posted } from "../src/floorpost.js";
import { ConfigError, parseServiceConfig } from "../service/config.js";
import { build, type Built } from "../service/main.js";
import { mirrorRequired, runPass } from "../service/keeper.js";
import { PROD_POST_POLICY } from "../service/plan.js";
import { ChainRpc, isRateLimited, RpcAnswer } from "../service/rpc.js";
import { StateStore } from "../service/state.js";
import { authorized, serveStatus, summaryOf, type Status } from "../service/status.js";
import { A, BURN, FakeChain, RAFFLE, stackJson } from "./service-fake.js";

// docs/audit/2026-09-26/fixes-W2-services.md §3–§7: the audit's keeper PoCs (E-01, E-02, E-04, E-09, E-10, E-14), inverted.
const CHAIN = 9466301;
const FLOOR = 158_000_000_000_000_000n;

type World = { chain: FakeChain; dir: string; env: Record<string, string>; lines: string[]; key: `0x${string}`; now: { ms: number } };
function world(extra: Record<string, string> = {}): World {
  const key = generatePrivateKey(), address = privateKeyToAccount(key).address;
  const chain = new FakeChain({ keeper: address, chainId: CHAIN });
  const dir = mkdtempSync(join(tmpdir(), "keeper-w2-"));
  writeFileSync(join(dir, "stack.json"), stackJson(CHAIN));
  writeFileSync(join(dir, "floors.json"), JSON.stringify({ collections: { frogs: { priceWei: String(FLOOR), confidenceBps: 8_000, depthItems: 5 } } }));
  const env: Record<string, string> = { KEEPER_CHAIN_ID: String(CHAIN), KEEPER_STACK: join(dir, "stack.json"), KEEPER_STATE_DIR: join(dir, "state"), KEEPER_RPC_URLS: "http://127.0.0.1:1",
    KEEPER_FLOOR_SOURCE: "fixture", KEEPER_FLOOR_FIXTURE: join(dir, "floors.json"), KEEPER_MODE: "send", KEEPER_PRIVATE_KEY: key, ...extra };
  return { chain, dir, env, lines: [], key, now: { ms: 1_790_000_000_000 } };
}
const built = (w: World, fetchImpl?: typeof fetch): Promise<Built> =>
  build([], w.env, { fetchImpl: fetchImpl ?? w.chain.fetch, write: (l) => w.lines.push(l), sleep: async () => { w.chain.advance(1); }, nowMs: () => w.now.ms });
const later = (w: World, s: number) => { w.chain.advance(s); w.now.ms += s * 1000; };

test("E-02 inverted: a thief's confidence-0 post at the same price is REPAIRED by the honest keeper's next pass; the module stays fresh", async () => {
  const w = world();
  const b = await built(w);
  await runPass(b);
  assert.equal(w.chain.fresh(), true);
  let staleMinutes = 0;
  for (let m = 0; m < 180; m += 5) {
    if (m % 25 === 0) { w.chain.confidence = 0; w.chain.postedAt = w.chain.headTs; }      // the stolen key's post
    later(w, 300);
    const r = await runPass(b);
    if (m === 0) assert.match(r.collections[0].floor.plan!.why, /repair: the posted confidence 0 bps is under the module's minimum 3000/);
    if (!w.chain.fresh()) staleMinutes += 5;
  }
  assert.equal(staleMinutes, 0, "every thief's post is overwritten within the pass (the audit measured 180 of 180 minutes stale)");
  // the pure rule: a module that is not fresh is re-posted even at the same price
  const p: Posted = { priceWei: FLOOR, postedAt: 1_000, ttl: 3_600, maxMoveBps: 500, bounds: { lo: FLOOR, hi: FLOOR }, fresh: false, confidenceBps: 8_000, minConfidenceBps: 3_000 };
  const plan = planPost({ price: FLOOR, listingFloor: FLOOR, saleFloor: null, confidenceBps: 8_000, depthItems: 5, upCost10: null, used: [], salesUsed: [], excluded: [] }, p, 1_100, PROD_POST_POLICY);
  assert.equal(plan.type, "post");
  assert.match(plan.why, /repair: the module is not fresh/);
});

test("E-10 inverted: a stuck transaction is re-priced at the same nonce (the same call while it simulates), mined, and alerted; one that cannot be re-priced within the fee cap is an alert", async () => {
  const w = world({ KEEPER_RECEIPT_WAIT_S: "5", KEEPER_STUCK_AFTER_S: "60" });
  const b = await built(w);
  await runPass(b);
  assert.equal(w.chain.fresh(), true);
  // the next post is broadcast ambiguously (the node never got it), then the base fee rises above the signed max fee
  later(w, 3_000);
  w.chain.mine = false;
  const orig = w.chain.fetch;
  let failSend = true;
  const flaky = (async (u: string, init: RequestInit) => {
    if (JSON.parse(String(init.body ?? "{}")).method === "eth_sendRawTransaction" && failSend) { failSend = false; throw new TypeError("fetch failed (timeout)"); }
    return orig(u, init);
  }) as unknown as typeof fetch;
  const b2 = await built(w, flaky);
  const r = await runPass(b2);
  assert.equal(r.collections[0].floor.result?.outcome, "pending");
  const nonce = b2.store.get().pending!.nonce;
  w.chain.baseFee *= 3n;                                         // the signed 2 × base is now under the base fee
  w.chain.mine = true;
  later(w, 30);
  const r2 = await runPass(b2);                                  // the re-broadcast is refused as underpriced: replaced at once
  assert.match(r2.pending!.alert ?? "", /tx-replaced: nonce \d+ re-priced/);
  assert.equal(r2.pending?.outcome, "mined", r2.pending?.detail);
  const mined = [...w.chain.txs.values()].find((t) => t.block !== null && t.nonce === nonce)!;
  assert.ok(mined.maxFeePerGas >= w.chain.baseFee, "the replacement pays the new base fee");
  assert.equal(w.chain.fresh(), true, "the floor is posted, not lost for hours (the audit: stale for 3 hours and counting)");
  assert.equal(JSON.parse(readFileSync(join(w.dir, "state", "keeper-state.json"), "utf8")).version, 2);
  // a fee the cap does not allow: re-broadcast only, and an alert
  const v = world({ KEEPER_RECEIPT_WAIT_S: "5", KEEPER_STUCK_AFTER_S: "60", KEEPER_MAX_FEE_PER_GAS_WEI: "30000000" });
  v.chain.mine = false;
  const c = await built(v);
  await runPass(c);
  v.chain.baseFee = 40_000_000n;
  later(v, 120);
  const r3 = await runPass(c);
  assert.equal(r3.pending?.outcome, "pending");
  assert.match(r3.alerts.join("\n"), /tx-stuck: nonce 0 .* over the fee cap/);
});

test("E-10: a stuck transaction that simulates no more is cleared by a zero-value self-transfer at the same nonce; while the keeper is paused nothing is signed, not even that", async () => {
  const w = world({ KEEPER_RECEIPT_WAIT_S: "5", KEEPER_STUCK_AFTER_S: "60" });
  w.chain.mine = false;
  const b = await built(w);
  await runPass(b);
  const p = b.store.get().pending!;
  const sent = w.chain.sent.length;
  // paused: the pass is refused, and a direct look at the pending nonce signs nothing either (an alert says so)
  w.chain.keeperPaused = true;
  later(w, 120);
  const r = await runPass(b);
  assert.equal(r.refusals.length, 1);
  const paused = await b.sender.resolvePending();
  assert.equal(paused?.outcome, "pending");
  assert.match(paused?.alert ?? "", /no replacement is signed: the registry has paused the keeper/);
  assert.equal(w.chain.sent.length, sent);
  // unpaused, but the call no longer simulates (the module's keeper moved): the nonce is cleared by a self-transfer
  w.chain.keeperPaused = false; w.chain.moduleKeeper = A(0x777); w.chain.mine = true;
  const res = await b.sender.resolvePending();
  assert.equal(res?.outcome, "mined");
  assert.match(res!.alert ?? "", /cancelled with a self-transfer/);
  const t = [...w.chain.txs.values()].find((x) => x.block !== null && x.nonce === p.nonce)!;
  assert.equal(t.to.toLowerCase(), privateKeyToAccount(w.key).address.toLowerCase(), "a self-transfer");
  assert.equal(t.data, "0x");
});

test("E-04 inverted: a JSON-RPC throttle answered in HTTP 200 moves to the next endpoint; a revert is still an answer", async () => {
  const w = world();
  const throttled = (async (url: string, init: RequestInit) => {
    const body = JSON.parse(String(init.body));
    if (url.endsWith(":1") && body.method !== "eth_chainId") return { ok: true, status: 200, json: async () => ({ jsonrpc: "2.0", id: body.id, error: { code: -32005, message: "the network is busy, please try again in a moment" } }) };
    return w.chain.fetch(url, init);
  }) as unknown as typeof fetch;
  const log: string[] = [];
  const rpc = new ChainRpc(["http://127.0.0.1:1", "http://127.0.0.1:2"], CHAIN, throttled, (_l, e, f) => log.push(`${e} ${JSON.stringify(f)}`), 5_000, { sleep: async () => {} });
  await rpc.init();
  assert.equal(await rpc.call<string>("eth_blockNumber", []), `0x${w.chain.head.toString(16)}`);
  assert.ok(log.some((l) => /rpc-fallback .*"throttled":true/.test(l)));
  const { encodeFunctionData } = await import("viem");
  const { VAULT } = await import("../service/abis.js");
  const notKeeper = encodeFunctionData({ abi: VAULT, functionName: "postCap", args: [1n] });
  await assert.rejects(rpc.call("eth_call", [{ from: A(0x999), to: RAFFLE, data: notKeeper }, "latest"]), (e: unknown) => e instanceof RpcAnswer, "a revert is final: not failed over");
  // all endpoints throttled: a backoff, then a clear failure (never a silent partial answer)
  const allBusy = (async (_u: string, init: RequestInit) => { const b = JSON.parse(String(init.body)); return { ok: true, status: 200,
    json: async () => (b.method === "eth_chainId" ? { id: b.id, result: `0x${CHAIN.toString(16)}` } : { id: b.id, error: { code: 429, message: "Too Many Requests" } }) }; }) as unknown as typeof fetch;
  let pauses = 0;
  const r2 = new ChainRpc(["http://127.0.0.1:1", "http://127.0.0.1:2"], CHAIN, allBusy, () => {}, 5_000, { sleep: async () => { pauses++; } });
  await r2.init();
  await assert.rejects(r2.call("eth_blockNumber", []), /every RPC failed/);
  assert.equal(pauses, 2, "two backoff rounds");
  assert.ok(isRateLimited(new RpcAnswer("eth_getLogs", "Too Many Requests", 429, null)));
  assert.ok(!isRateLimited(new RpcAnswer("eth_getLogs", "logs matched by query exceeds limit of 10000", -32000, null)), "too large is split by the caller, not failed over");
});

test("E-09 inverted and E-01 inverted, on the recorded 2026-09-23 book: wash sales between fresh wallet pairs move the posted floor at most 5%; one frog churned counts once", () => {
  const L0 = JSON.parse(readFileSync(new URL("../fixtures/floor/ow-bitcoin-frogs-listings-2026-09-23.json", import.meta.url), "utf8"));
  const S0 = JSON.parse(readFileSync(new URL("../fixtures/floor/ow-bitcoin-frogs-sales-2026-09-23.json", import.meta.url), "utf8"));
  const AT: number = L0.fetchedAt;
  const clean = (id: string): Contents => ({ inscriptions: [id], runes: [] });
  const listings = parseOwListings(L0.listings, AT).map((l) => ({ ...l, contents: clean(l.id) }));
  const sales = parseOwSales(S0);
  const honest = robustFloor({ listings, sales, snapshotAt: AT });
  const L = honest.listingFloor!;
  const addr = (tag: string, n: number) => `bc1p${tag}${String(n).padStart(4, "0")}${"q".repeat(50 - tag.length)}`;
  const insc = (n: number) => `${(0xa0d17e00 + n).toString(16).padStart(64, "0")}i0`;
  const wash = L / 4n + 1n;
  // the audit's attack: 11 distinct items, 11 fresh unlinked wallet pairs, at just over L/4
  const fake: Sale[] = Array.from({ length: 11 }, (_, i) => ({ tx: `${(0xbeef00 + i).toString(16).padStart(64, "0")}`, at: AT - 60 * (i + 1), item: insc(i),
    seller: addr("s", i), buyer: addr("b", i), price: wash, itemsInTx: 1 }));
  const attacked = robustFloor({ listings, sales: [...fake, ...sales], snapshotAt: AT });
  assert.equal(attacked.saleFloor, wash, "the attacker still owns the sales median");
  assert.ok(attacked.price! >= L - (L * 500n) / 10_000n, `the floor ${attacked.price} stays within 5% of the listing floor ${L} (was 0.775 × L)`);
  assert.ok(honest.price! - attacked.price! <= (honest.price! * 500n) / 10_000n);
  // the realistic attack with ONE frog: the same item sold around 11 pairs is one sale
  const churn: Sale[] = fake.map((s) => ({ ...s, item: insc(0) }));
  const one = robustFloor({ listings, sales: [...churn, ...sales], snapshotAt: AT });
  assert.equal(one.excluded.filter((e) => e.rule === "S-item-repeat").length >= 10, true);
  assert.ok(one.price! >= honest.price! - (honest.price! * 100n) / 10_000n, "one churned frog barely moves it");
  // E-09: the conservative wei floor (the higher sats/ETH) is what a post rests on
  assert.ok(satsToWei(attacked.price!, 3_700_000n) < satsToWei(attacked.price!, 3_665_000n));
});

test("C-01 at the keeper: a vault with no ceiling gets no cap and an alert; the pass says why", async () => {
  const w = world();
  w.chain.vaults[BURN.toLowerCase()].ceiling = 0n;
  const b = await built(w);
  const r = await runPass(b);
  const burn = r.collections[0].caps.find((c) => c.policy === "burn")!;
  assert.equal(burn.plan.type, "skip");
  assert.equal(burn.result, null, "nothing sent to a vault that would revert NoCeiling");
  assert.ok(r.alerts.some((a) => /no-ceiling: frogs's burn vault/.test(a)));
  assert.equal(r.collections[0].caps.find((c) => c.policy === "raffle")!.result?.outcome, "mined");
});

test("E-14: public URLs and relays are https; the status token is long; no mirror on 4663 in send mode means no draw is announced", () => {
  const base = { KEEPER_CHAIN_ID: "4663", KEEPER_STACK: "/x" };
  const problems = (env: Record<string, string>) => { try { parseServiceConfig([], { ...base, ...env }); return []; } catch (e) { return (e as ConfigError).problems; } };
  assert.match(problems({ KEEPER_PUBLIC_URL: "http://keeper.example" }).join("\n"), /KEEPER_PUBLIC_URL must be https/);
  assert.deepEqual(problems({ KEEPER_PUBLIC_URL: "http://127.0.0.1:8790" }), [], "plain http for this machine (the fork test)");
  assert.match(problems({ KEEPER_SNAPSHOT_UPLOAD_URL: "http://mirror.example" }).join("\n"), /UPLOAD_URL must be https/);
  assert.match(problems({ KEEPER_DRAND_URLS: "https://api.drand.sh" }).join("\n"), /at least two different drand relays/);
  assert.match(problems({ KEEPER_DRAND_URLS: "https://api.drand.sh,http://relay.example" }).join("\n"), /must be https/);
  assert.match(problems({ KEEPER_STATUS_TOKEN: "short" }).join("\n"), /at least 32 characters/);
  assert.match(problems({ KEEPER_SNAPSHOT_UPLOAD_AUTH: "x" }).join("\n"), /without KEEPER_SNAPSHOT_UPLOAD_URL/);
  const cfg = parseServiceConfig([], { ...base, KEEPER_PUBLIC_URL: "https://keeper.example/" });
  assert.equal(cfg.snapshots.baseUrl, "https://keeper.example/snapshots");
  assert.equal(mirrorRequired({ mode: "send", chainId: 4663, snapshots: { baseUrl: "x", uploadUrl: null, uploadAuth: null, uploadReadback: true } }), true);
  assert.equal(mirrorRequired({ mode: "send", chainId: 4663, snapshots: { baseUrl: "x", uploadUrl: "https://m", uploadAuth: null, uploadReadback: true } }), false);
  assert.equal(mirrorRequired({ mode: "dry-run", chainId: 4663, snapshots: { baseUrl: "x", uploadUrl: null, uploadAuth: null, uploadReadback: true } }), false);
});

test("E-14: /status is a public summary (no pending nonce, no refusal texts); /status/full needs the bearer token; the token is scrubbed from logs", async () => {
  const TOKEN = "t".repeat(20) + "status-token-0123456789";
  const w = world({ KEEPER_STATUS_TOKEN: TOKEN });
  const b = await built(w);
  const s: Status = { mode: "send", chainId: CHAIN, keeper: A(5), rpc: [], startedAt: "", interval: 60, passes: 3, lastPassAt: new Date().toISOString(), lastPassMs: 1, consecutiveFailures: 0,
    lastError: { at: "", message: "secret-ish detail" }, refusals: ["a refusal text"], alerts: ["tx-stuck: nonce 7"], drawRule: "", gas: { day: "", spentOrReservedWei: "", capWei: "", perTxCapWei: "", maxGasPerTx: "", maxFeePerGasWei: "" },
    pending: { nonce: 7, hash: "0xabc" }, history: [{ nonce: 6 }], last: null, alertPasses: 1, alertPassesLimit: 3 };
  const srv = serveStatus(() => s, join(w.dir, "state", "snapshots"), 0, (x) => x, "127.0.0.1", TOKEN);
  await new Promise((ok) => srv.once("listening", ok));
  const port = (srv.address() as AddressInfo).port;
  try {
    const pub = await (await fetch(`http://127.0.0.1:${port}/status`)).text();
    assert.deepEqual(JSON.parse(pub), JSON.parse(JSON.stringify(summaryOf(s, Date.now()))));
    for (const hidden of ["a refusal text", "nonce", "0xabc", "secret-ish", "tx-stuck"]) assert.ok(!pub.includes(hidden), `the summary hides ${hidden}`);
    assert.equal((await fetch(`http://127.0.0.1:${port}/status/full`)).status, 401);
    assert.equal((await fetch(`http://127.0.0.1:${port}/status/full`, { headers: { authorization: "Bearer wrong" } })).status, 401);
    const full = await fetch(`http://127.0.0.1:${port}/status/full`, { headers: { authorization: `Bearer ${TOKEN}` } });
    assert.equal(full.status, 200);
    assert.equal(full.headers.get("access-control-allow-origin"), null, "no wildcard CORS on the full view");
    assert.match(await full.text(), /a refusal text/);
    const hz = await (await fetch(`http://127.0.0.1:${port}/healthz`)).json() as Record<string, unknown>;
    assert.deepEqual(Object.keys(hz).sort(), ["alertKinds", "alertPasses", "alertPassesLimit", "alerts", "lastPassAt", "ok", "refusals"]);
    assert.deepEqual(hz.alertKinds, ["tx-stuck"], "the kinds only (R2-S8), never the texts");
    assert.equal(hz.refusals, 1, "counts only");
  } finally { srv.close(); }
  assert.equal(authorized({ headers: {} }, null), false, "no token configured: the full view is off");
  b.log("info", "x", { note: `echo ${TOKEN}` });
  assert.ok(!w.lines.join("\n").includes(TOKEN), "the status token is cut out of every log line");
});

test("the state file: version 1 is read and upgraded (its pending transaction becomes the first attempt); anything else is refused", () => {
  const dir = mkdtempSync(join(tmpdir(), "keeper-state-"));
  writeFileSync(join(dir, "keeper-state.json"), JSON.stringify({ version: 1, day: "2026-09-26", spentWei: "5", history: [],
    pending: { key: "k", what: "w", to: A(1), data: "0x", nonce: 3, hash: "0xh", raw: "0xr", sentAt: 10, reservedWei: "7", day: "2026-09-26" } }));
  const s = new StateStore(dir).get();
  assert.equal(s.version, 2);
  assert.deepEqual(s.pending!.attempts.map((a) => a.hash), ["0xh"]);
  assert.equal(s.pending!.firstSentAt, 10);
  assert.throws(() => StateStore.parse(JSON.stringify({ version: 3, spentWei: "0", history: [] })), /not version 1 or 2/);
});
