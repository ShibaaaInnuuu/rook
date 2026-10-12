import assert from "node:assert/strict";
import { test } from "node:test";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { generatePrivateKey, privateKeyToAccount } from "viem/accounts";
import type { Snapshot } from "../../tools/snapshot/src/core.js";
import { robustFloor, type Contents } from "../src/floor.js";
import { planPost, POST_POLICY, type Posted } from "../src/floorpost.js";
import { parseOwListings, parseOwSales } from "../src/floorsources.js";
import { ConfigError, parseServiceConfig } from "../service/config.js";
import { fetchText, MAX_SNAPSHOT_BYTES, publish } from "../service/draws.js";
import { build, type Built } from "../service/main.js";
import { runPass } from "../service/keeper.js";
import { PROD_POST_POLICY } from "../service/plan.js";
import { alertKinds, healthy, type Status } from "../service/status.js";
import { A, COIN, CURVE, FakeChain, stackJson } from "./service-fake.js";

// Second review (R2) of the wave-2 services fixes: the PoCs of docs/audit/2026-09-26/review2/poc/audit2_services.test.ts, turned
// into regressions (round 3, docs/audit/2026-09-26/review2/fixes-R3-services.md). Each one now asserts that the attack FAILS.
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

/** A draw taken to Resolved (announced, opened, requested, resolved), then its published file forged: same root and count
 *  fields, every entry naming 0x…bad with a junk proof (a mirror/CDN/upload-credential rewrite after openDraw). */
async function forgedResolved(w: World, b: Built) {
  await runPass(b); later(w, 70); await runPass(b); later(w, 901); await runPass(b); later(w, 70);
  // the resolve pass: its claim is what the forged file must not be able to block, so the file is forged BEFORE it
  const dr0 = w.chain.draws[0];
  const real = JSON.parse(readFileSync(join(w.dir, "state", "snapshots", dr0.uri.split("/").pop()!), "utf8")) as Snapshot;
  const forged = { ...real, entries: real.entries.map((e) => ({ ...e, holder: A(0xbad).toLowerCase(), proof: ["0x" + "11".repeat(32)] })) };
  w.chain.files.set(dr0.uri, JSON.stringify(forged));
  return { real, forged };
}

test("R2-S1 (fixed): a forged file's entries do not prove against the on-chain root, so the resolver claims from the chain rebuild", async () => {
  const w = world();
  const b = await built(w);
  await forgedResolved(w, b);
  // the keeper's own copy is gone too (the fallback is the chain, not the file's neighbours)
  rmSync(join(w.dir, "state", "snapshots"), { recursive: true }); w.chain.serveDir = null;
  for (let i = 0; i < 3 && w.chain.draws[0].stage !== 4; i++) { await runPass(b); later(w, 60); }
  assert.equal(w.chain.draws[0].stage, 4, "Claimed");
  assert.equal(w.chain.claimed.length, 1);
  assert.notEqual(w.chain.claimed[0].holder, A(0xbad).toLowerCase(), "never the forged holder");
  assert.ok([H1, H2, H3].map((h) => h.toLowerCase()).includes(w.chain.claimed[0].holder), "one of the real holders");
  assert.ok(!w.lines.some((l) => /claim frogs draw 0 for 0x0000000000000000000000000000000000000bad/i.test(l)), "no claim is even simulated for the forged entry");
});

test("R2-S1 (fixed): with no source that proves (forged file, no copy, a chain rebuild that does not match), the draw pages claim-stuck after 1 h and is voided after 7 days; the rebuild is retried on a backoff, not every pass", async () => {
  const w = world();
  const b = await built(w);
  await forgedResolved(w, b);
  rmSync(join(w.dir, "state", "snapshots"), { recursive: true }); w.chain.serveDir = null;
  // R3-check L-D: this test used to remove the whole logs directory, so every rebuild threw ENOENT and "one rebuild" compared 0
  // with 0. Now only the coin's cached logs go, and the rebuild really runs and really does not match
  rmSync(join(w.dir, "state", "logs", `${COIN.toLowerCase()}.json`), { force: true });
  w.chain.transfers = [{ block: 500, from: A(0), to: H1, value: 7n * E18 }];      // the rebuild now gives another tree: it proves nothing
  const mismatches = () => w.lines.filter((l) => /claim-rebuild-mismatch/.test(l)).length;
  let r = await runPass(b); later(w, 60);                                         // the resolve
  r = await runPass(b); later(w, 60);                                             // the claim: no source proves
  assert.equal(w.chain.draws[0].stage, 3);
  assert.ok(!r.alerts.some((a) => /^claim-stuck:/.test(a)), "not yet an hour");
  assert.equal(mismatches(), 1, "the rebuild ran once, and did not match");
  await runPass(b); later(w, 60);
  assert.equal(mismatches(), 1, "the next pass does not rebuild again");
  later(w, 3_600);
  r = await runPass(b);
  assert.ok(r.alerts.some((a) => /^claim-stuck: frogs draw 0 has been Resolved/.test(a)), r.alerts.join("\n"));
  assert.equal(mismatches(), 2, "after its backoff (10 min) it is tried again, once");
  later(w, 7 * 86_400 + 60);
  await runPass(b);
  assert.equal(w.chain.draws[0].stage, 6, "Voided by the resolver: the branch is reachable");
  assert.deepEqual(w.chain.claimed, []);
});

test("R2-S2 (fixed): one relay of three answering other bytes no longer stops the open; the beacon picks, and the liar is an alert", async () => {
  const w = world();
  const b = await built(w);
  await runPass(b);
  w.chain.relayLie = true;                 // drand.cloudflare.com answers other bytes; api.drand.sh and api2.drand.sh agree
  later(w, 70);
  const r = await runPass(b);
  assert.equal(w.chain.draws[0].stage, 1, "opened");
  assert.ok(r.alerts.some((a) => /^drand-liar: .*drand\.cloudflare\.com/.test(a)), r.alerts.join("\n"));
});

test("R2-S3 (fixed): after a self-transfer attempt, the re-price of the ORIGINAL call keeps the call's own gas, not 21,000", async () => {
  const w = world({ KEEPER_RECEIPT_WAIT_S: "5", KEEPER_STUCK_AFTER_S: "60" });
  w.chain.coins = []; w.chain.units.clear();
  w.chain.mine = false;
  const b = await built(w);
  await runPass(b);
  const first = b.store.get().pending!.attempts[0];
  w.chain.moduleKeeper = A(0x777);          // the call stops simulating: the next re-price is a self-transfer
  later(w, 120);
  await b.sender.resolvePending();
  assert.equal(b.store.get().pending!.attempts.at(-1)!.cancel, true);
  w.chain.moduleKeeper = privateKeyToAccount(w.key).address;     // it simulates again
  later(w, 120);
  await b.sender.resolvePending();
  const last = b.store.get().pending!.attempts.at(-1)!;
  assert.equal(last.cancel, false);
  assert.equal(last.gas, first.gas, "the original call's gas");
  assert.notEqual(last.gas, "21000");
});

test("R2-S3 (fixed): a replacement the node refuses for a reason that is not its price is not recorded, so it is never the base of the next re-price", async () => {
  const w = world({ KEEPER_RECEIPT_WAIT_S: "5", KEEPER_STUCK_AFTER_S: "60" });
  w.chain.coins = []; w.chain.units.clear();
  w.chain.mine = false;
  const b = await built(w);
  await runPass(b);
  const before = b.store.get().pending!.attempts.length;
  const orig = b.rpc.sendRaw.bind(b.rpc);
  const { RpcAnswer } = await import("../service/rpc.js");
  b.rpc.sendRaw = (async () => { throw new RpcAnswer("eth_sendRawTransaction", "intrinsic gas too low", -32000, null); }) as typeof b.rpc.sendRaw;
  later(w, 120);
  const r = await b.sender.resolvePending();
  assert.match(String(r?.alert), /tx-stuck: .*intrinsic gas too low.*not recorded/);
  assert.equal(b.store.get().pending!.attempts.length, before, "the refused attempt is not in the record");
  b.rpc.sendRaw = orig;
});

test("R2-S4 (fixed): buying the one cheap listing of the recorded 2026-09-23 book no longer lifts the posted floor at the module's full rate", () => {
  const L0 = JSON.parse(readFileSync(new URL("../fixtures/floor/ow-bitcoin-frogs-listings-2026-09-23.json", import.meta.url), "utf8"));
  const S0 = JSON.parse(readFileSync(new URL("../fixtures/floor/ow-bitcoin-frogs-sales-2026-09-23.json", import.meta.url), "utf8"));
  const AT: number = L0.fetchedAt;
  const clean = (id: string): Contents => ({ inscriptions: [id], runes: [] });
  const listings = parseOwListings(L0.listings, AT).map((l) => ({ ...l, contents: clean(l.id) }));
  const sales = parseOwSales(S0);
  const honest = robustFloor({ listings, sales, snapshotAt: AT });
  const byPrice = [...listings].sort((a, b) => (a.price < b.price ? -1 : 1));
  const T = 1_800_000_000, rows: string[] = [];
  // the module has accepted the honest floor (sats stand in for wei: only ratios matter), last posted an hour ago, ±5 %/h
  const posted: Posted = { priceWei: honest.price!, postedAt: T - 3_600, ttl: 7_200, maxMoveBps: 500, bounds: null, fresh: true, confidenceBps: 8_000, minConfidenceBps: 3_000 };
  const asBuilt = { ...PROD_POST_POLICY, liftBpsPerHour: undefined, liftDepth: undefined };
  const pct = (x: string) => Number(((BigInt(x) - honest.price!) * 10_000n) / honest.price!) / 100;
  for (const k of [1, 3]) {
    const gone = new Set(byPrice.slice(0, k).map((l) => l.id));
    const pumped = robustFloor({ listings: listings.filter((l) => !gone.has(l.id)), sales, snapshotAt: AT });
    const was = planPost(pumped, posted, T, asBuilt), now = planPost(pumped, posted, T, PROD_POST_POLICY);
    rows.push(`k=${k}: target +${pct(String(pumped.price))}% (conf ${pumped.confidenceBps}, depth ${pumped.depthItems}); one hour's post as built ${was.type === "post" ? `+${pct(was.priceWei)}%` : was.type}, now ${now.type === "post" ? `+${pct(now.priceWei)}%` : now.type}`);
    if (now.type === "post") assert.ok(pct(now.priceWei) <= 2, `k=${k}: at most +2% an hour now`);
    if (k === 1) { assert.equal(was.type, "post"); assert.ok(pct((was as { priceWei: string }).priceWei) >= 4.9, "as built: the module's full +5%/h"); assert.equal(now.type, "post"); assert.match(now.why, /R2-S4/); }
  }
  console.log(`honest floor ${honest.price} sats (L ${honest.listingFloor}, S ${honest.saleFloor})\n  ${rows.join("\n  ")}`);
  // a rise that the sales back (a sale median at or above the target), or five listings deep, is followed at the module's rate
  const backed = planPost({ ...honest, price: honest.price! * 107n / 100n, saleFloor: honest.price! * 2n }, posted, T, PROD_POST_POLICY);
  assert.equal(backed.type, "post"); assert.ok(pct((backed as { priceWei: string }).priceWei) >= 4.9);
  const deep = planPost({ ...honest, price: honest.price! * 107n / 100n, depthItems: 5 }, posted, T, PROD_POST_POLICY);
  assert.ok(pct((deep as { priceWei: string }).priceWei) >= 4.9);
  // and a fall is never slowed by this rule
  const down = planPost({ ...honest, price: honest.price! * 90n / 100n }, posted, T, PROD_POST_POLICY);
  assert.ok(pct((down as { priceWei: string }).priceWei) <= -4.9);
  assert.equal(POST_POLICY.minConfidenceBps, 3_000);
});

test("R2-S5 (fixed): on 4663 a mirror requires its own base URL off the keeper's host, and publish reads the mirror's object back too", async () => {
  const base = { KEEPER_CHAIN_ID: "4663", KEEPER_STACK: "/x.json", KEEPER_MODE: "send", KEEPER_PUBLIC_URL: "https://keeper.up.railway.app",
    KEEPER_SNAPSHOT_UPLOAD_URL: "https://mirror.example/put" };
  const problems = (env: Record<string, string>) => { try { parseServiceConfig([], env); return ""; } catch (e) { return (e as ConfigError).problems.join("\n"); } };
  assert.match(problems(base), /without KEEPER_SNAPSHOT_BASE_URL: the draw's URI would name the keeper's own host/);
  assert.match(problems({ ...base, KEEPER_SNAPSHOT_BASE_URL: "https://keeper.up.railway.app/snapshots" }), /on KEEPER_PUBLIC_URL's host/);
  assert.doesNotMatch(problems({ ...base, KEEPER_SNAPSHOT_BASE_URL: "https://cdn.example" }), /BASE_URL/);
  // publish: the URI (the mirror's public base) AND the mirror's own object, the same bytes from both
  const w = world();
  const dir = mkdtempSync(join(tmpdir(), "r3-pub-"));
  const cfg = { dir, baseUrl: "https://cdn.example", uploadUrl: "https://mirror.example/put", uploadAuth: "Bearer x", uploadReadback: true };
  const ok = await publish(cfg, "snapshot-0x" + "c0".repeat(20) + "-1-weighted.json", "{\"a\":1}\n", w.chain.fetch);
  assert.deepEqual(ok.readBack, ["cdn.example", "mirror.example"]);
  w.chain.mirrorServesPut = false;               // the mirror's object is not what it says (here: not served): no draw
  await assert.rejects(publish(cfg, "snapshot-0x" + "c0".repeat(20) + "-2-weighted.json", "{\"a\":2}\n", w.chain.fetch), /not served at mirror\.example/);
});

test("R2-S6 (fixed): fetchText stops reading at the cap, and refuses a host that resolves to a private address before it asks", async () => {
  let read = 0;
  const endless = new ReadableStream<Uint8Array>({ pull(c) { const chunk = new Uint8Array(1024 * 1024).fill(120); read += chunk.byteLength; c.enqueue(chunk); } });
  const big = (async () => ({ ok: true, status: 200, headers: new Headers(), body: endless, text: async () => { throw new Error("never buffered whole"); } })) as unknown as typeof fetch;
  const pub = async () => [{ address: "93.184.216.34", family: 4 }];
  assert.equal(await fetchText("https://attacker.example/snapshot.json", big, false, pub), null);
  assert.ok(read <= MAX_SNAPSHOT_BYTES + 2 * 1024 * 1024, `stopped after ${read} bytes`);
  let asked = "";
  const spy = (async (u: string) => { asked = u; return { ok: true, status: 200, headers: new Headers(), text: async () => "{}" }; }) as unknown as typeof fetch;
  assert.equal(await fetchText("https://10.0.0.5.nip.io/snapshots/x.json", spy, false, async () => [{ address: "10.0.0.5", family: 4 }]), null);
  assert.equal(await fetchText("https://metadata.internal/x.json", spy), null);
  assert.equal(await fetchText("https://[::ffff:169.254.169.254]/x.json", spy), null);
  assert.equal(asked, "", "nothing private was asked");
  assert.equal(await fetchText("https://cdn.example/x.json", spy, false, pub), "{}");
});

test("R2-S7 (fixed): the keeper image installs production dependencies only and runs as node", () => {
  const df = readFileSync(new URL("../Dockerfile", import.meta.url), "utf8");
  assert.match(df, /RUN cd keeper && npm ci --omit=dev --no-audit --no-fund/);
  assert.doesNotMatch(df, /RUN cd keeper && npm ci --no-audit/);
  assert.match(df, /^ENTRYPOINT \["drop-to-node"\]/m);
  assert.match(df, /COPY --chmod=0755 ops\/docker\/drop-to-node\.sh/);
  assert.match(readFileSync(new URL("../Dockerfile.dockerignore", import.meta.url), "utf8"), /^!ops\/docker\/drop-to-node\.sh$/m);
  const pkg = JSON.parse(readFileSync(new URL("../package.json", import.meta.url), "utf8"));
  assert.ok(pkg.dependencies.tsx, "tsx runs the service: a runtime dependency");
});

test("R2-S8 (fixed): alerts in force for KEEPER_ALERT_PASSES passes turn /healthz 503, and the kinds (never the texts) are what it names", () => {
  const s = { refusals: [], consecutiveFailures: 0, lastPassAt: new Date(1_000_000).toISOString(), interval: 60, alertPassesLimit: 3,
    alerts: ["tx-stuck: nonce 7 …", "draw-slot-missed: frogs 0x…", "claim-stuck: frogs draw 0: …"] } as unknown as Status;
  assert.equal(healthy({ ...s, alertPasses: 2 }, 1_000_000 + 30_000), true, "two passes: not yet (one bad pass is not a page)");
  assert.equal(healthy({ ...s, alertPasses: 3 }, 1_000_000 + 30_000), false, "three passes in a row: 503");
  assert.equal(healthy({ ...s, alerts: [], alertPasses: 0 }, 1_000_000 + 30_000), true);
  assert.deepEqual(alertKinds(s.alerts), ["claim-stuck", "draw-slot-missed", "tx-stuck"]);
  assert.equal(parseServiceConfig([], { KEEPER_CHAIN_ID: "9466301", KEEPER_STACK: "/x.json", KEEPER_ALERT_PASSES: "5" }).alertPasses, 5);
});
