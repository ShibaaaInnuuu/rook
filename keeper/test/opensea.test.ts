import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { test } from "node:test";
import type { Hex } from "viem";
import { GaveUp } from "../src/backoff.js";
import { logger, REDACTED } from "../src/log.js";
import { candidArgs, fulfillmentData, OPENSEA_API, SEAPORT_1_6, toHandoff } from "../src/opensea.js";

// docs/v4/eth-nft-canister.md §12.2. A FAKE key throughout; every HTTP answer is mocked. No test calls OpenSea.
const FAKE_KEY = "fake-opensea-key-0123456789abcdef";
const ANSWER = JSON.parse(readFileSync(new URL("../fixtures/opensea-fulfillment-base-51673875.json", import.meta.url), "utf8"));
const REAL = JSON.parse(readFileSync(new URL("../../autobuyer/canister-rs/tests/fixtures/seaport/base-zone-fill-51673875.json", import.meta.url), "utf8"));
const FULFILLER = REAL.fulfiller as Hex;   // the address OpenSea issued this zone signature to
const HASH = REAL.order_hash as Hex;
const clone = <T>(x: T): T => JSON.parse(JSON.stringify(x));

type Call = { url: string; method: string; headers: Record<string, string>; body: string };
function server(replies: { status: number; body?: unknown; headers?: Record<string, string> }[]) {
  const calls: Call[] = [];
  const f = (async (url: string, init: RequestInit) => {
    calls.push({ url, method: String(init.method), headers: init.headers as Record<string, string>, body: String(init.body) });
    const r = replies[Math.min(calls.length - 1, replies.length - 1)];
    return { ok: r.status >= 200 && r.status < 300, status: r.status, headers: new Headers(r.headers ?? {}), json: async () => r.body, text: async () => JSON.stringify(r.body ?? "") };
  }) as unknown as typeof fetch;
  return { f, calls };
}

test("§12.2: one POST to OpenSea's v2 listings fulfillment route, the key in x-api-key, the canister named as fulfiller", async () => {
  const { f, calls } = server([{ status: 200, body: ANSWER }]);
  const h = await fulfillmentData({ chainId: 8453, orderHash: HASH, fulfiller: FULFILLER }, { fetchImpl: f, apiKey: () => FAKE_KEY, sleep: async () => {} });
  assert.equal(calls.length, 1);
  assert.equal(calls[0].url, "https://api.opensea.io/api/v2/listings/fulfillment_data");
  assert.equal(calls[0].url, OPENSEA_API);
  assert.equal(calls[0].method, "POST");
  assert.equal(calls[0].headers["x-api-key"], FAKE_KEY);
  assert.deepEqual(JSON.parse(calls[0].body), { listing: { hash: HASH, chain: "base", protocol_address: SEAPORT_1_6 }, fulfiller: { address: FULFILLER } });
  assert.ok(!calls[0].body.includes(FAKE_KEY), "the key is in the header and nowhere else");
  // what the canister is handed: the real calldata, with the fulfiller conduit zeroed and OpenSea's 4-byte tag dropped
  const want = REAL.input.slice(0, -8) as string;
  const zeroed = want.slice(0, 2 + 8 + 128) + "0".repeat(64) + want.slice(2 + 8 + 192);
  assert.equal(h.orderHex, zeroed);
  assert.deepEqual({ ...h, orderHex: "…" }, { chainId: 8453, collection: "0xc7c114191aa3b2225f9bb053bc55b3d6f145bd33", tokenId: "4711", orderHex: "…",
    fn: "fulfillAdvancedOrder", valueWei: BigInt(REAL.value).toString(), zoneExpiration: 1_790_137_378 });
  assert.equal(candidArgs(h), `(8453 : nat64, "0xc7c114191aa3b2225f9bb053bc55b3d6f145bd33", "4711", "${zeroed}")`);
});

test("the recipient in our calldata is the fulfiller we asked for, whatever OpenSea's answer names", () => {
  const a = clone(ANSWER);
  a.fulfillment_data.transaction.input_data.recipient = "0x000000000000000000000000000000000000beef";
  a.fulfillment_data.transaction.input_data.fulfillerConduitKey = "0x" + "ab".repeat(32);
  const h = toHandoff(a, 8453, FULFILLER);
  assert.ok(h.orderHex.endsWith("0".repeat(64)), "no criteria resolvers");
  assert.ok(h.orderHex.slice(2 + 8 + 192, 2 + 8 + 256).endsWith(FULFILLER.slice(2).toLowerCase()), "recipient = the canister");
  assert.ok(!h.orderHex.includes("000000000000000000000000000000000000beef"), "OpenSea's recipient is discarded");
  assert.equal(h.orderHex.slice(2 + 8 + 128, 2 + 8 + 192), "0".repeat(64), "and its conduit key");
});

test("an answer that is not what was asked for is refused, before anything reaches the canister", () => {
  const bad = (f: (a: typeof ANSWER) => void, chain = 8453, ful: Hex = FULFILLER) => { const a = clone(ANSWER); f(a); return () => toHandoff(a, chain, ful); };
  assert.throws(bad((a) => { a.fulfillment_data.transaction.to = "0x00000000000000adc04c56bf30ac9d3c0aaf14dc"; }), /not Seaport 1\.6/);
  assert.throws(bad(() => {}, 1), /not 1/);
  assert.throws(bad((a) => { a.fulfillment_data.transaction.function = "matchAdvancedOrders(...)"; }), /accepts fulfillAdvancedOrder and fulfillBasicOrder only/);
  assert.throws(bad(() => {}, 8453, "0x000000000000000000000000000000000000beef"), /another fulfiller/);
  assert.throws(bad((a) => { a.fulfillment_data.transaction.input_data.advancedOrder.parameters.offerer = "nope"; }), /invalid offerer/);
  assert.throws(() => toHandoff({}, 8453, FULFILLER), /no fulfillment_data/);
});

test("a 429 waits out Retry-After and is retried through backoff.ts; a rejected key or a missing listing is not retried", async () => {
  const slept: number[] = [];
  const sleep = async (ms: number) => { slept.push(ms); };
  const busy = server([{ status: 429, headers: { "retry-after": "3" } }, { status: 429, headers: { "retry-after": "999" } }, { status: 200, body: ANSWER }]);
  const h = await fulfillmentData({ chainId: 8453, orderHash: HASH, fulfiller: FULFILLER }, { fetchImpl: busy.f, apiKey: () => FAKE_KEY, sleep });
  assert.equal(h.fn, "fulfillAdvancedOrder");
  assert.equal(busy.calls.length, 3);
  assert.deepEqual(slept, [3_000, 1_000, 60_000, 2_000], "Retry-After (capped at a minute), then backoff.ts's own delay, each time");
  for (const [status, re] of [[401, /rejected the key/], [403, /rejected the key/], [404, /no such listing/]] as const) {
    const s = server([{ status }]);
    await assert.rejects(fulfillmentData({ chainId: 8453, orderHash: HASH, fulfiller: FULFILLER }, { fetchImpl: s.f, apiKey: () => FAKE_KEY, sleep }), (e: Error) => {
      assert.match(e.message, re); assert.ok(!e.message.includes(FAKE_KEY)); return true; });
    assert.equal(s.calls.length, 1, `HTTP ${status} is not retried`);
  }
  const down = server([{ status: 503 }]);
  await assert.rejects(fulfillmentData({ chainId: 8453, orderHash: HASH, fulfiller: FULFILLER }, { fetchImpl: down.f, apiKey: () => FAKE_KEY, sleep, tries: 3 }), GaveUp);
  assert.equal(down.calls.length, 3);
});

test("no key, no request; an unknown chain, no request", async () => {
  const s = server([{ status: 200, body: ANSWER }]);
  await assert.rejects(fulfillmentData({ chainId: 8453, orderHash: HASH, fulfiller: FULFILLER }, { fetchImpl: s.f, apiKey: () => undefined, sleep: async () => {} }), /missing OPENSEA_API_KEY/);
  await assert.rejects(fulfillmentData({ chainId: 4663, orderHash: HASH, fulfiller: FULFILLER }, { fetchImpl: s.f, apiKey: () => FAKE_KEY }), /no OpenSea identifier/);
  assert.equal(s.calls.length, 0);
});

test("the key never reaches a log line: not as a field, not nested, not inside an error, not from a retry", async () => {
  const lines: string[] = [];
  const log = logger((l) => lines.push(l), () => new Date(0), () => [FAKE_KEY]);
  log("info", "x", { apiKey: "anything", nested: { note: `k=${FAKE_KEY}` }, err: new Error(`failed with ${FAKE_KEY}`), header: FAKE_KEY });
  const s = server([{ status: 500, body: { echo: FAKE_KEY } }, { status: 200, body: ANSWER }]);
  await fulfillmentData({ chainId: 8453, orderHash: HASH, fulfiller: FULFILLER }, { fetchImpl: s.f, apiKey: () => FAKE_KEY, sleep: async () => {}, log });
  assert.ok(lines.length >= 3);
  for (const l of lines) assert.ok(!l.includes(FAKE_KEY), l);
  assert.ok(lines[0].includes(REDACTED));
  // and by default the scrubber reads the real variable's value at log time, so a key set only in the environment is cut out too
  process.env.OPENSEA_API_KEY = FAKE_KEY;
  try { const out: string[] = []; logger((l) => out.push(l))("info", "y", { m: `x${FAKE_KEY}y` }); assert.ok(!out[0].includes(FAKE_KEY)); }
  finally { delete process.env.OPENSEA_API_KEY; }
});

test("only opensea.ts reads the key, and only to put it in the one header", () => {
  const dir = new URL("../src/", import.meta.url);
  for (const f of readdirSync(dir).filter((x) => x.endsWith(".ts"))) {
    const src = readFileSync(new URL(f, dir), "utf8");
    const reads = src.match(/OPENSEA_API_KEY/g)?.length ?? 0;
    if (f === "opensea.ts") assert.ok(reads >= 1 && /"x-api-key": key/.test(src));
    else if (f === "log.ts") assert.ok(/env\.OPENSEA_API_KEY/.test(src), "the scrubber reads it to cut it out");
    else assert.equal(reads, 0, `${f} names OPENSEA_API_KEY`);
  }
});
