import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { test } from "node:test";
import { logger, REDACTED } from "../src/log.js";
import { Esplora, FEED, mergeInscriptions, mergeOutputs, OrdFeed, OrdRecursive, OrdinalsWallet, UniSat, USER_AGENT, VerifiedOrd, runeName,
  type OrdProvider } from "../src/ordapi.js";

// docs/v6/ordinals-apis.md §5. A FAKE key throughout; every HTTP answer is mocked (the ord and Ordinals Wallet bodies are the
// real answers captured on 2026-09-23 for Bitcoin Frog #3487; the UniSat bodies follow its swagger, unverified). No test calls a provider.
const FAKE_KEY = "fake-unisat-key-0123456789abcdef";
const fixture = (n: string) => readFileSync(new URL(`../fixtures/ordapi/${n}`, import.meta.url), "utf8");
const OW = fixture("ordinalswallet-inscription-frog-3487.json");
const ORD = fixture("ord-r-inscription-frog-3487.json");
const ORD_UTXO = fixture("ord-r-utxo-frog-3487.json");
const ID = "3a5b201020e7a3ed48bdc77e25559bbdebd5926904b9b46301c79481476fa703i0";
const OUT = "579526a20898e3ae9f279241f4fd5c06d9439c1fe2528dc62ddb6e295c2a0ec0:0";
const ADDR = "bc1pya872u2nyl52xmr3uyp48yly5cv0g2auxx6cagt3f8zksfejmf3qcq7zmj";
const H = 968323;
const unisatInscription = (o: Partial<{ txid: string; vout: number; offset: number; satoshi: number }> = {}) => JSON.stringify({ code: 0, msg: "ok", data: {
  inscriptionId: ID, inscriptionNumber: 395887, address: ADDR, offset: o.offset ?? 0, contentType: "image/webp", height: 780131,
  utxo: { txid: o.txid ?? OUT.split(":")[0], vout: o.vout ?? 0, satoshi: o.satoshi ?? 10000, address: ADDR, inscriptions: [{ inscriptionId: ID, offset: 0 }] } } });
const unisatUtxo = (ids: string[] = [ID], spent = false) => JSON.stringify({ code: 0, msg: "ok", data: { txid: OUT.split(":")[0], vout: 0, satoshi: 10000, address: ADDR,
  inscriptions: ids.map((inscriptionId) => ({ inscriptionId, offset: 0 })), isSpent: spent } });
const unisatRunes = (names: string[] = []) => JSON.stringify({ code: 0, msg: "ok", data: names.map((spacedRune) => ({ spacedRune, rune: runeName(spacedRune), amount: "1" })) });
const unisatHeight = (h = H) => JSON.stringify({ code: 0, msg: "ok", data: { chain: "main", blocks: h, headers: h } });

type Reply = { status: number; body: string; headers?: Record<string, string> };
type Call = { url: string; headers: Record<string, string> };
/** A fake internet: the first matching prefix answers; a function may answer differently per call. */
function web(routes: [string, Reply | ((n: number) => Reply)][]) {
  const calls: Call[] = [];
  const seen = new Map<string, number>();
  const f = (async (url: string, init: RequestInit) => {
    calls.push({ url, headers: init.headers as Record<string, string> });
    const hit = routes.find(([p]) => url.startsWith(p));
    if (!hit) return { status: 404, headers: new Headers(), text: async () => "no route" };
    const n = (seen.get(hit[0]) ?? 0) + 1; seen.set(hit[0], n);
    const r = typeof hit[1] === "function" ? hit[1](n) : hit[1];
    return { status: r.status, headers: new Headers(r.headers ?? {}), text: async () => r.body };
  }) as unknown as typeof fetch;
  return { f, calls };
}
const ok = (body: string): Reply => ({ status: 200, body });
const NOW = 1_790_000_000_000;
let t = NOW;
const clock = { now: () => t, sleep: async (ms: number) => { t += ms; } };
const healthy = (): [string, Reply | ((n: number) => Reply)][] => [
  ["https://ordinals.com/r/blockheight", ok(String(H))], ["https://ordinals.com/r/inscription/", ok(ORD)], ["https://ordinals.com/r/utxo/", ok(ORD_UTXO)],
  ["https://turbo.ordinalswallet.com/blockheight", ok(String(H))], ["https://turbo.ordinalswallet.com/inscription/", ok(OW)],
  ["https://open-api.unisat.io/v1/indexer/blockchain/info", ok(unisatHeight())], ["https://open-api.unisat.io/v1/indexer/inscription/info/", ok(unisatInscription())],
  ["https://open-api.unisat.io/v1/indexer/utxo/", ok(unisatUtxo())], ["https://open-api.unisat.io/v1/indexer/runes/utxo/", ok(unisatRunes())],
];
/** `routes` with the route at `prefix` replaced. */
const set = (routes: [string, Reply | ((n: number) => Reply)][], prefix: string, r: Reply | ((n: number) => Reply)) => {
  const i = routes.findIndex(([p]) => p === prefix);
  assert.ok(i >= 0, `no route ${prefix}`); routes[i] = [prefix, r]; return routes;
};
function feed(routes = healthy(), providers?: (o: { fetchImpl: typeof fetch }) => OrdProvider[], log?: ReturnType<typeof logger>) {
  const { f, calls } = web(routes);
  const o = { fetchImpl: f, ...clock, apiKey: () => FAKE_KEY };
  const ps = providers ? providers(o) : [new UniSat(o), new OrdRecursive("https://ordinals.com", o), new OrdinalsWallet(o)];
  return { feed: new OrdFeed(ps, { log, now: clock.now, esplora: [new Esplora("https://mempool.space/api", o), new Esplora("https://blockstream.info/api", o)] }), calls };
}

test("agree: three providers place the frog on the same satpoint at the same height, and the merged answer carries every fact", async () => {
  const { feed: fd, calls } = feed();
  const v = await fd.inscription(ID);
  assert.ok(v.ok);
  assert.deepEqual(v.value, { id: ID, outpoint: OUT, offset: 0, value: 10000, address: ADDR, contentType: "image/webp", genesisHeight: 780131 });
  assert.equal(v.height, H);
  assert.deepEqual(v.by, ["unisat", "ord", "ordinalswallet"]);
  for (const c of calls) assert.equal(c.headers["user-agent"], USER_AGENT, `${c.url} carries a User-Agent`);
  const keyed = calls.filter((c) => c.headers.authorization !== undefined);
  assert.ok(keyed.length >= 1 && keyed.every((c) => c.url.startsWith("https://open-api.unisat.io/") && c.headers.authorization === `Bearer ${FAKE_KEY}`), "the key goes to UniSat's host and nowhere else");
  // an output: UniSat and ord answer it (Ordinals Wallet has no per-output view), and agree
  const o = await fd.output(OUT);
  assert.ok(o.ok);
  assert.deepEqual(o.value, { inscriptions: [ID], runes: [], value: 10000 });
  assert.deepEqual(o.by, ["unisat", "ord"]);
});

test("disagree: one provider naming another outpoint, offset or set of inscriptions is unverified, and nothing is believed", async () => {
  const moved = healthy(); set(moved, "https://open-api.unisat.io/v1/indexer/inscription/info/", ok(unisatInscription({ txid: "ab".repeat(32) })));
  let v = await feed(moved).feed.inscription(ID);
  assert.deepEqual([v.ok, (v as { why: string }).why], [false, "disagree"]);
  const shifted = healthy(); set(shifted, "https://open-api.unisat.io/v1/indexer/inscription/info/", ok(unisatInscription({ offset: 7 })));
  v = await feed(shifted).feed.inscription(ID);
  assert.deepEqual([v.ok, (v as { why: string }).why], [false, "disagree"]);
  const extra = healthy(); set(extra, "https://open-api.unisat.io/v1/indexer/utxo/", ok(unisatUtxo([ID, `${"cd".repeat(32)}i0`])));
  const o = await feed(extra).feed.output(OUT);
  assert.deepEqual([o.ok, (o as { why: string }).why], [false, "disagree"]);
  const runes = healthy(); set(runes, "https://open-api.unisat.io/v1/indexer/runes/utxo/", ok(unisatRunes(["DOG•GO•TO•THE•MOON"])));
  const r = await feed(runes).feed.output(OUT);
  assert.deepEqual([r.ok, (r as { why: string }).why], [false, "disagree"]);
  // known to one, unknown to another: also a disagreement, not "the one that knows wins"
  const unknown = healthy(); set(unknown, "https://turbo.ordinalswallet.com/inscription/", { status: 404, body: '{"error":true}' });
  v = await feed(unknown).feed.inscription(ID);
  assert.deepEqual([v.ok, (v as { why: string }).why], [false, "disagree"]);
  // unknown to all: agreed, and null
  const none = healthy(); set(none, "https://ordinals.com/r/inscription/", { status: 404, body: "not found" }); set(none, "https://turbo.ordinalswallet.com/inscription/", { status: 404, body: "{}" });
  set(none, "https://open-api.unisat.io/v1/indexer/inscription/info/", ok(JSON.stringify({ code: 0, msg: "ok", data: null })));
  v = await feed(none).feed.inscription(ID);
  assert.deepEqual(v, { ok: true, value: null, height: H, by: ["unisat", "ord", "ordinalswallet"] });
});

test("one down: a provider that errors, or answers something unreadable, makes the read unverified; a spent output is null on both sides", async () => {
  const down = healthy(); set(down, "https://ordinals.com/r/inscription/", { status: 503, body: "" });
  let v = await feed(down).feed.inscription(ID);
  assert.deepEqual([v.ok, (v as { why: string }).why, (v as { detail: string }).detail.startsWith("ord: HTTP 503")], [false, "one-down", true]);
  const garbage = healthy(); set(garbage, "https://turbo.ordinalswallet.com/inscription/", ok("<html>"));
  v = await feed(garbage).feed.inscription(ID);
  assert.deepEqual([v.ok, (v as { why: string }).why], [false, "one-down"]);
  const wrongId = healthy(); set(wrongId, "https://open-api.unisat.io/v1/indexer/inscription/info/", ok(unisatInscription().replace(ID, `${"ef".repeat(32)}i0`)));
  v = await feed(wrongId).feed.inscription(ID);
  assert.deepEqual([v.ok, (v as { why: string }).why], [false, "one-down"]);
  const spent = healthy(); set(spent, "https://ordinals.com/r/utxo/", { status: 404, body: "output not found" }); set(spent, "https://open-api.unisat.io/v1/indexer/utxo/", ok(unisatUtxo([ID], true)));
  const o = await feed(spent).feed.output(OUT);
  assert.deepEqual(o, { ok: true, value: null, height: H, by: ["unisat", "ord"] });
});

test("rate limited: a 429 parks that provider for Retry-After (capped at a minute), the read is 'rate-limited' until then, and the park expires", async () => {
  t = NOW;
  const busy = healthy(); set(busy, "https://turbo.ordinalswallet.com/inscription/", (n) => (n === 1 ? { status: 429, body: "", headers: { "retry-after": "30" } } : ok(OW)));
  const { feed: fd, calls } = feed(busy);
  let v = await fd.inscription(ID);
  assert.deepEqual([v.ok, (v as { why: string }).why], [false, "rate-limited"]);
  const before = calls.length;
  v = await fd.inscription(ID);
  assert.deepEqual([v.ok, (v as { why: string }).why], [false, "rate-limited"]);
  assert.ok(!calls.slice(before).some((c) => c.url.startsWith("https://turbo.ordinalswallet.com/inscription")), "a parked provider is not asked");
  t += 31_000;
  v = await fd.inscription(ID);
  assert.ok(v.ok, "the park expired");
  // without Retry-After the wait grows with each consecutive 429: 5 s, 10 s, 20 s …
  const hammered = healthy(); set(hammered, "https://ordinals.com/r/blockheight", { status: 429, body: "" });
  const g = feed(hammered);
  for (const wait of [5_000, 10_000, 20_000]) {
    const r = await g.feed.inscription(ID);
    assert.deepEqual([r.ok, (r as { why: string }).why], [false, "rate-limited"]);
    const parked = (r as { detail: string }).detail;
    assert.ok(parked.includes(new Date(t + wait).toISOString()), `${parked} parks for ${wait}`);
    t += wait;
  }
});

test("stale: providers at heights more than one block apart, or a height that moves during a read, are not the same question", async () => {
  const behind = healthy(); set(behind, "https://turbo.ordinalswallet.com/blockheight", ok(String(H - 2)));
  let v = await feed(behind).feed.inscription(ID);
  assert.deepEqual([v.ok, (v as { why: string }).why], [false, "stale"]);
  const oneOff = healthy(); set(oneOff, "https://turbo.ordinalswallet.com/blockheight", ok(String(H - 1)));
  v = await feed(oneOff).feed.inscription(ID);
  assert.ok(v.ok && v.height === H - 1, "one block apart is the boundary, and the lower height is reported");
  const moving = healthy(); set(moving, "https://ordinals.com/r/blockheight", (n) => ok(String(H + (n % 2))));
  v = await feed(moving).feed.inscription(ID);
  assert.deepEqual([v.ok, (v as { why: string }).why], [false, "stale"]);
});

test("unsupported: fewer than two providers answering a read is unverified, and the floor's ord sees null (L-unverified)", async () => {
  const { feed: fd } = feed(healthy(), (o) => [new OrdRecursive("https://ordinals.com", o), new OrdinalsWallet(o)]);
  const o = await fd.output(OUT);
  assert.deepEqual([o.ok, (o as { why: string }).why], [false, "unsupported"]);
  assert.equal(await new VerifiedOrd(fd).contents(OUT), null);
  const v = await fd.inscription(ID);
  assert.ok(v.ok, "two keyless providers still verify a location");
  const full = feed();
  assert.deepEqual(await new VerifiedOrd(full.feed).contents(OUT), { inscriptions: [ID], runes: [] });
  assert.equal(await new VerifiedOrd(full.feed).contents("not an outpoint"), null, "an invalid outpoint is unverified, never thrown at the floor");
});

test("tx facts: UniSat's confirmations against esplora's; a mempool transaction is 0, one no explorer knows is null", async () => {
  const txid = OUT.split(":")[0];
  const routes = healthy();
  routes.push([`https://open-api.unisat.io/v1/indexer/tx/${txid}`, ok(JSON.stringify({ code: 0, msg: "ok", data: { txid, height: H - 9, confirmations: 10 } }))]);
  routes.push(["https://mempool.space/api/tx/", ok(JSON.stringify({ confirmed: true, block_height: H - 9 }))], ["https://mempool.space/api/blocks/tip/height", ok(String(H))]);
  routes.push(["https://blockstream.info/api/tx/", ok(JSON.stringify({ confirmed: true, block_height: H - 9 }))], ["https://blockstream.info/api/blocks/tip/height", ok(String(H))]);
  let v = await feed(routes).feed.txConfirmations(txid);
  assert.deepEqual(v, { ok: true, value: 10, height: 0, by: ["unisat", "mempool.space", "blockstream.info"] });
  const lagging = [...routes]; lagging[lagging.length - 1] = ["https://blockstream.info/api/blocks/tip/height", ok(String(H - 3))];
  v = await feed(lagging).feed.txConfirmations(txid);
  assert.deepEqual([v.ok, (v as { why: string }).why], [false, "stale"]);
  const unknown = [...routes]; unknown[unknown.length - 2] = ["https://blockstream.info/api/tx/", { status: 404, body: "" }];
  v = await feed(unknown).feed.txConfirmations(txid);
  assert.deepEqual([v.ok, (v as { why: string }).why], [false, "disagree"]);
  // the keyless pair alone: two explorers are two sources
  const keyless = feed(routes, (o) => [new OrdRecursive("https://ordinals.com", o), new OrdinalsWallet(o)]);
  assert.deepEqual(await keyless.feed.txConfirmations(txid), { ok: true, value: 10, height: 0, by: ["mempool.space", "blockstream.info"] });
});

test("listings: one provider's book, cached 45 s; a listing never moves money by itself", async () => {
  t = NOW;
  const escrows = JSON.stringify([{ inscription_id: ID, outpoint: OUT, seller_address: ADDR, satoshi_price: "480000", created: "2026-09-20 10:00:00+00" },
    { inscription_id: `${"ab".repeat(32)}i0`, outpoint: `${"ab".repeat(32)}:1`, seller_address: ADDR, satoshi_price: 500000, created: "2026-09-20 11:00:00+00", bought_at: "x" }]);
  const routes = healthy(); routes.push(["https://turbo.ordinalswallet.com/collection/bitcoin-frogs/escrows", ok(escrows)]);
  const { feed: fd, calls } = feed(routes);
  const v = await fd.listings("bitcoin-frogs");
  assert.ok(v.ok);
  assert.deepEqual(v.value, [{ id: ID, price: 480000n, seller: ADDR, outpoint: OUT, listedAt: 1_789_898_400 }]);
  const n = calls.length;
  await fd.listings("bitcoin-frogs");
  assert.equal(calls.length, n, "served from the cache");
  t += FEED.cacheMs + 1;
  await fd.listings("bitcoin-frogs");
  assert.equal(calls.length, n + 1, "and fetched again once the cache is stale");
  const bad = await fd.listings("Bad Slug!");
  assert.deepEqual([bad.ok, (bad as { detail: string }).detail], [false, "ordinalswallet: invalid collection slug"]);
});

test("the key: read at call time, into one header of one host; missing means no request; never in a log line or an error", async () => {
  const lines: string[] = [];
  const log = logger((l) => lines.push(l), () => new Date(0), () => [FAKE_KEY]);
  const { calls } = feed();
  const none = web(healthy());
  const u = new UniSat({ fetchImpl: none.f, ...clock, apiKey: () => undefined });
  await assert.rejects(u.height(), /missing UNISAT_API_KEY/);
  assert.equal(none.calls.length, 0);
  const rejected = web([["https://open-api.unisat.io/", { status: 401, body: JSON.stringify({ code: -1, msg: `bad key ${FAKE_KEY}` }) }]]);
  const r = new UniSat({ fetchImpl: rejected.f, ...clock, apiKey: () => FAKE_KEY });
  await assert.rejects(r.height(), (e: Error) => { assert.match(e.message, /rejected the key \(HTTP 401\)/); assert.ok(!e.message.includes(FAKE_KEY)); return true; });
  // a body that echoes the key, logged through the feed's warning, is scrubbed
  const echo = healthy(); set(echo, "https://open-api.unisat.io/v1/indexer/inscription/info/", ok(unisatInscription({ txid: "ab".repeat(32) }).replace('"ok"', `"${FAKE_KEY}"`)));
  const v = await feed(echo, undefined, log).feed.inscription(ID);
  assert.equal(v.ok, false);
  log("warn", "x", { apiKey: FAKE_KEY, detail: `key=${FAKE_KEY}` });
  assert.ok(lines.length >= 2);
  for (const l of lines) assert.ok(!l.includes(FAKE_KEY), l);
  assert.ok(lines[lines.length - 1].includes(REDACTED));
  // and by default the scrubber reads UNISAT_API_KEY from the environment at log time
  process.env.UNISAT_API_KEY = FAKE_KEY;
  try { const out: string[] = []; logger((l) => out.push(l))("info", "y", { m: `x${FAKE_KEY}y` }); assert.ok(!out[0].includes(FAKE_KEY)); }
  finally { delete process.env.UNISAT_API_KEY; }
  void calls;
});

test("an unverified read is logged once an hour per kind, not once a tick", async () => {
  t = NOW;
  const lines: string[] = [];
  const log = logger((l) => lines.push(l), () => new Date(t), () => []);
  const down = healthy(); set(down, "https://ordinals.com/r/inscription/", { status: 503, body: "" });
  const { feed: fd } = feed(down, undefined, log);
  for (let i = 0; i < 5; i++) { await fd.inscription(ID); t += 60_000; }
  assert.equal(lines.filter((l) => l.includes("ord_unverified")).length, 1);
  t += FEED.muteS * 1000;
  await fd.inscription(ID);
  assert.equal(lines.filter((l) => l.includes("ord_unverified")).length, 2);
});

test("merging: facts one provider lacks are filled from another; a fact both give must match; rune names compare without spacers", () => {
  const a = { id: ID, outpoint: OUT, offset: 0, value: 10000, address: null, contentType: "image/webp", genesisHeight: null };
  const b = { ...a, address: ADDR, contentType: null, genesisHeight: 780131 };
  assert.deepEqual(mergeInscriptions([a, b]), { ...a, address: ADDR, genesisHeight: 780131 });
  assert.equal(mergeInscriptions([a, { ...b, value: 9999 }]), undefined);
  assert.equal(mergeInscriptions([a, null]), undefined);
  assert.equal(mergeInscriptions([null, null]), null);
  assert.deepEqual(mergeOutputs([{ inscriptions: [ID], runes: ["DOGGOTOTHEMOON"], value: null }, { inscriptions: [ID], runes: ["DOGGOTOTHEMOON"], value: 10000 }]),
    { inscriptions: [ID], runes: ["DOGGOTOTHEMOON"], value: 10000 });
  assert.equal(mergeOutputs([{ inscriptions: [ID], runes: [], value: 1 }, { inscriptions: [ID], runes: ["X"], value: 1 }]), undefined);
  assert.equal(runeName("DOG•GO•TO•THE•MOON"), "DOGGOTOTHEMOON");
  assert.equal(runeName("dog.go.to.the.moon"), "DOGGOTOTHEMOON");
});

test("only ordapi.ts reads UNISAT_API_KEY, and only to put it in the one header", () => {
  const dir = new URL("../src/", import.meta.url);
  for (const f of readdirSync(dir).filter((x) => x.endsWith(".ts"))) {
    const src = readFileSync(new URL(f, dir), "utf8");
    const reads = src.match(/UNISAT_API_KEY/g)?.length ?? 0;
    if (f === "ordapi.ts") assert.ok(reads >= 1 && /authorization: `Bearer \$\{key\}`/.test(src));
    else if (f === "log.ts") assert.ok(/env\.UNISAT_API_KEY/.test(src), "the scrubber reads it to cut it out");
    else assert.equal(reads, 0, `${f} names UNISAT_API_KEY`);
  }
});
