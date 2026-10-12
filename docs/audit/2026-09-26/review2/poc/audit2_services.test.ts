import assert from "node:assert/strict";
import { test } from "node:test";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { generatePrivateKey, privateKeyToAccount } from "viem/accounts";
import type { Snapshot } from "../../tools/snapshot/src/core.js";
import { robustFloor, type Contents } from "../src/floor.js";
import { parseOwListings, parseOwSales } from "../src/floorsources.js";
import { parseServiceConfig } from "../service/config.js";
import { fetchText } from "../service/draws.js";
import { build, type Built } from "../service/main.js";
import { runPass } from "../service/keeper.js";
import { A, COIN, CURVE, FakeChain, MOD, stackJson } from "./service-fake.js";

// Second review (R2) of the wave-2 services fixes. Each test asserts the CURRENT behaviour; a passing test demonstrates the finding.
const E18 = 10n ** 18n;
const FLOOR = 158_000_000_000_000_000n;
const H1 = A(0xa1), H2 = A(0xa2), H3 = A(0xa3);
const CHAIN = 9466301;

type World = { chain: FakeChain; dir: string; env: Record<string, string>; lines: string[]; key: `0x${string}`; now: { ms: number } };
function world(extra: Record<string, string> = {}): World {
  const key = generatePrivateKey(), address = privateKeyToAccount(key).address;
  const chain = new FakeChain({ keeper: address, chainId: CHAIN });
  const dir = mkdtempSync(join(tmpdir(), "keeper-audit2-"));
  writeFileSync(join(dir, "stack.json"), stackJson(CHAIN));
  writeFileSync(join(dir, "floors.json"), JSON.stringify({ collections: { frogs: { priceWei: String(FLOOR), confidenceBps: 8_000, depthItems: 2 } } }));
  const env: Record<string, string> = { KEEPER_CHAIN_ID: String(CHAIN), KEEPER_STACK: join(dir, "stack.json"), KEEPER_STATE_DIR: join(dir, "state"),
    KEEPER_RPC_URLS: "http://127.0.0.1:1", KEEPER_FLOOR_SOURCE: "fixture", KEEPER_FLOOR_FIXTURE: join(dir, "floors.json"),
    KEEPER_DRAW_EVERY_S: "600", KEEPER_DRAW_OFFSET_S: "150", KEEPER_DRAW_WINDOW_S: "300", KEEPER_PUBLIC_URL: "https://keeper.example",
    KEEPER_MODE: "send", KEEPER_PRIVATE_KEY: key, ...extra };
  chain.serveDir = join(dir, "state", "snapshots");
  chain.coins = [{ coin: COIN, policy: 0, curve: CURVE }];
  chain.units.set(COIN.toLowerCase(), E18);
  chain.transfers = [{ block: 500, from: A(0), to: H1, value: 50n * E18 }, { block: 600, from: A(0), to: H2, value: 25n * E18 },
    { block: 700, from: A(0), to: CURVE, value: 900n * E18 }, { block: 800, from: CURVE, to: H3, value: 5n * E18 }];
  return { chain, dir, env, lines: [], key, now: { ms: 1_790_000_000_000 } };
}
const built = (w: World): Promise<Built> =>
  build([], w.env, { fetchImpl: w.chain.fetch, write: (l) => w.lines.push(l), sleep: async () => { w.chain.advance(1); }, nowMs: () => w.now.ms });
const later = (w: World, s: number) => { w.chain.advance(s); w.now.ms += s * 1000; };

test("R2-S1: a published file whose root/totalTickets fields match but whose entries are forged blocks the resolver's claim for good, and it never falls back to the chain rebuild nor voids", async () => {
  const w = world();
  const b = await built(w);
  await runPass(b); later(w, 70); await runPass(b); later(w, 901); await runPass(b); later(w, 70); await runPass(b);
  assert.equal(w.chain.draws[0].stage, 3, "Resolved");
  const dr = w.chain.draws[0];
  // the file at the draw's URI is replaced after openDraw (a mirror/CDN/host write, or the upload credential): same root and count fields,
  // but the winning range now names another holder with a junk proof. Nothing in entryIn() checks a proof against the root.
  const real = JSON.parse(readFileSync(join(w.dir, "state", "snapshots", dr.uri.split("/").pop()!), "utf8")) as Snapshot;
  const forged = { ...real, entries: real.entries.map((e) => ({ ...e, holder: A(0xbad).toLowerCase(), proof: ["0x" + "11".repeat(32)] })) };
  w.chain.files.set(dr.uri, JSON.stringify(forged));
  for (let i = 0; i < 3; i++) { await runPass(b); later(w, 60); }
  assert.equal(w.chain.draws[0].stage, 3, "still Resolved: every claim simulation reverts (BadProof)");
  assert.deepEqual(w.chain.claimed, []);
  // seven days (and the parent blocks) later: the resolver still tries the forged entry and never reaches its voidDraw branch
  later(w, 7 * 86_400 + 60);
  w.chain.advance(0);
  for (let i = 0; i < 2; i++) { await runPass(b); later(w, 60); }
  assert.equal(w.chain.draws[0].stage, 3, "never claimed, never voided by the resolver");
  assert.ok(w.lines.some((l) => /simulation-reverted/.test(l) && /claim frogs draw 0/.test(l)), "the only trace is a warn line");
});

test("R2-S2: one relay of three answering other bytes stops every draw open (and every ticket/draw resolve), although the other two agree and the beacon would say which is right", async () => {
  const w = world();
  const b = await built(w);
  await runPass(b);
  w.chain.relayLie = true;                 // drand.cloudflare.com answers other bytes; api.drand.sh and api2.drand.sh agree
  later(w, 70);
  const r = await runPass(b);
  const open = r.collections[0].draws.find((d) => d.action === "open")!;
  assert.equal(open.result, null);
  assert.match(open.plan, /relays disagree/);
  assert.equal(w.chain.draws[0].stage, 7, "still Announced: after OPEN_TTL it is voided");
});

test("R2-S3: after a stuck nonce was cleared by a self-transfer attempt, a later re-price of the ORIGINAL call re-uses the self-transfer's 21,000 gas", async () => {
  const w = world({ KEEPER_RECEIPT_WAIT_S: "5", KEEPER_STUCK_AFTER_S: "60" });
  w.chain.coins = []; w.chain.units.clear();
  w.chain.mine = false;
  const b = await built(w);
  await runPass(b);
  const p = b.store.get().pending!;
  // the call stops simulating: the next re-price is a zero-value self-transfer (gas 21,000), which also stays pending
  w.chain.moduleKeeper = A(0x777);
  later(w, 120);
  await b.sender.resolvePending();
  const afterCancel = b.store.get().pending!;
  assert.equal(afterCancel.attempts.at(-1)!.cancel, true);
  assert.equal(afterCancel.attempts.at(-1)!.gas, "21000");
  // the call simulates again (the condition passed); the next re-price signs the ORIGINAL postFloor with gas 21,000
  w.chain.moduleKeeper = privateKeyToAccount(w.key).address;
  later(w, 120);
  await b.sender.resolvePending();
  const last = b.store.get().pending!.attempts.at(-1)!;
  assert.equal(last.cancel, false);
  assert.equal(last.to.toLowerCase(), MOD.toLowerCase(), "the original call");
  assert.equal(last.gas, "21000", "signed with the self-transfer's gas: below postFloor's intrinsic gas, a node refuses it (or it runs out of gas)");
  assert.equal(p.nonce, b.store.get().pending!.nonce);
});

test("R2-S4 (E-01 the other way): with one marketplace, the 5% pull cap removes most of the sales side's damping of a LISTING pump", () => {
  const L0 = JSON.parse(readFileSync(new URL("../fixtures/floor/ow-bitcoin-frogs-listings-2026-09-23.json", import.meta.url), "utf8"));
  const S0 = JSON.parse(readFileSync(new URL("../fixtures/floor/ow-bitcoin-frogs-sales-2026-09-23.json", import.meta.url), "utf8"));
  const AT: number = L0.fetchedAt;
  const clean = (id: string): Contents => ({ inscriptions: [id], runes: [] });
  const listings = parseOwListings(L0.listings, AT).map((l) => ({ ...l, contents: clean(l.id) }));
  const sales = parseOwSales(S0);
  const honest = robustFloor({ listings, sales, snapshotAt: AT });
  // the attacker buys the cheapest k listings (they keep the frogs); nothing about the recent sales changes
  const byPrice = [...listings].sort((a, b) => (a.price < b.price ? -1 : 1));
  const rows: string[] = [];
  let worst = 0;
  for (const k of [1, 3, 5, 8]) {
    const gone = new Set(byPrice.slice(0, k).map((l) => l.id));
    const rest = listings.filter((l) => !gone.has(l.id));
    const cost = byPrice.slice(0, k).reduce((s, l) => s + l.price, 0n);
    const now1 = robustFloor({ listings: rest, sales, snapshotAt: AT });                    // as built: one marketplace
    const old = robustFloor({ listings: rest, sales, snapshotAt: AT, saleSources: 2 });      // the pre-fix blend 0.7·L + 0.3·S
    const up = (x: bigint) => Number(((x - honest.price!) * 10_000n) / honest.price!) / 100;
    rows.push(`k=${k} cost=${cost} sats  L'=${now1.listingFloor}  S=${now1.saleFloor}  floor now ${now1.price} (+${up(now1.price!)}%)  pre-fix blend ${old.price} (+${up(old.price!)}%)  conf ${now1.confidenceBps}`);
    worst = Math.max(worst, up(now1.price!) - up(old.price!));
    assert.ok(now1.price! >= old.price!, "the cap never lowers a pumped floor");
  }
  console.log(`honest floor ${honest.price} (L ${honest.listingFloor}, S ${honest.saleFloor})\n  ${rows.join("\n  ")}`);
  assert.ok(worst > 0, "for some pump the as-built floor is higher than the pre-fix blend would have been");
});

test("R2-S5 (E-11): with a mirror configured on 4663, the draw's URI still defaults to the KEEPER's host, and publish() reads back only from that URI (the mirror is never read back)", () => {
  const cfg = parseServiceConfig([], { KEEPER_CHAIN_ID: "4663", KEEPER_STACK: "/x.json", KEEPER_MODE: "send", KEEPER_PUBLIC_URL: "https://keeper.up.railway.app",
    KEEPER_SNAPSHOT_UPLOAD_URL: "https://mirror.example/put" });
  assert.equal(cfg.snapshots.baseUrl, "https://keeper.up.railway.app/snapshots", "the URI written on chain names the hot key's host");
  const src = readFileSync(new URL("../service/draws.ts", import.meta.url), "utf8");
  assert.match(src, /const back = await fetchText\(uri, fetchImpl, true\)/);
  assert.doesNotMatch(src, /fetchText\(`\$\{p\.uploadUrl/);
});

test("R2-S6: fetchText (the resolver's read of a keeper-chosen URI) reads the whole body before the 20 MB check when no content-length is sent", async () => {
  let read = 0;
  const big = (async () => ({ ok: true, status: 200, headers: new Headers(), text: async () => { const t = "x".repeat(25 * 1024 * 1024); read = t.length; return t; } })) as unknown as typeof fetch;
  assert.equal(await fetchText("https://attacker.example/snapshot.json", big), null);
  assert.equal(read, 25 * 1024 * 1024, "the whole body was buffered first");
  // and any https host is fetched: no private-address check on the resolver's GET (a keeper-chosen URI)
  let asked = "";
  const spy = (async (u: string) => { asked = u; return { ok: false, status: 404, headers: new Headers(), text: async () => "" }; }) as unknown as typeof fetch;
  await fetchText("https://10.0.0.5.nip.io/snapshots/x.json", spy);
  assert.equal(asked, "https://10.0.0.5.nip.io/snapshots/x.json");
});

test("R2-S7 (E-16): the keeper image still runs as root and installs dev dependencies", () => {
  const df = readFileSync(new URL("../Dockerfile", import.meta.url), "utf8");
  assert.doesNotMatch(df, /^\s*USER\s/m, "no USER line");
  assert.match(df, /RUN cd keeper && npm ci --no-audit --no-fund/);
});

test("R2-S8: keeper alerts (tx-stuck, draw-slot-missed, claim, publish, drand…) do not turn /healthz 503, and the monitor's liveness poll reads only the HTTP status", async () => {
  const { healthy } = await import("../service/status.js");
  const s = { refusals: [], consecutiveFailures: 0, lastPassAt: new Date(1_000_000).toISOString(), interval: 60,
    alerts: ["tx-stuck: nonce 7 …", "draw-slot-missed: frogs 0x…", "claim: frogs draw 0: …"] } as never;
  assert.equal(healthy(s, 1_000_000 + 30_000), true, "200 with three alerts in force");
  const live = readFileSync(new URL("../../monitor/src/liveness.mjs", import.meta.url), "utf8");
  assert.match(live, /if \(r\.status !== 200\) error = /);
  assert.doesNotMatch(live, /\.json\(\)|\.text\(\)|alerts\s*\.length/, "the body (with its alert count) is never read");
});
