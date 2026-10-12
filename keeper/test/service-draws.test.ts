import assert from "node:assert/strict";
import { test } from "node:test";
import { existsSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { generatePrivateKey, privateKeyToAccount } from "viem/accounts";
import { balancesOf, snapshot, verify, sentinelLeaf, type Snapshot } from "../../tools/snapshot/src/core.js";
import { build, type Built } from "../service/main.js";
import { runPass } from "../service/keeper.js";
import { buildResolver, runResolverPass } from "../service/resolver-main.js";
import { Refusal } from "../service/rpc.js";
import { A, COIN, CURVE, FakeChain, roundAt, stackJson, timeOf, type FakeChain as FC } from "./service-fake.js";
import { S3Stub, stubFetch } from "./s3-stub.js";

// docs/audit/2026-09-26/fixes-W2-services.md §1, §2: the draw flow (announce → seed round → snapshot at snapshotAt with the sentinel →
// published and read back → openDraw), and the keyless resolver (requestDraw, resolve, claim, voidDraw, the Wrapper's tickets).
const E18 = 10n ** 18n;
const FLOOR = 158_000_000_000_000_000n;
const H1 = A(0xa1), H2 = A(0xa2), H3 = A(0xa3);
const CHAIN = 9466301;

type World = { chain: FC; dir: string; env: Record<string, string>; lines: string[]; key: `0x${string}`; address: `0x${string}`; now: { ms: number } };
function world(extra: Record<string, string> = {}, o: { send?: boolean } = { send: true }): World {
  const key = generatePrivateKey(), address = privateKeyToAccount(key).address;
  const chain = new FakeChain({ keeper: address, chainId: CHAIN });
  const dir = mkdtempSync(join(tmpdir(), "keeper-draws-"));
  writeFileSync(join(dir, "stack.json"), stackJson(CHAIN));
  writeFileSync(join(dir, "floors.json"), JSON.stringify({ collections: { frogs: { priceWei: String(FLOOR), confidenceBps: 8_000, depthItems: 2 } } }));
  const env: Record<string, string> = { KEEPER_CHAIN_ID: String(CHAIN), KEEPER_STACK: join(dir, "stack.json"), KEEPER_STATE_DIR: join(dir, "state"),
    KEEPER_RPC_URLS: "http://127.0.0.1:1", KEEPER_FLOOR_SOURCE: "fixture", KEEPER_FLOOR_FIXTURE: join(dir, "floors.json"),
    KEEPER_DRAW_EVERY_S: "600", KEEPER_DRAW_OFFSET_S: "150", KEEPER_DRAW_WINDOW_S: "300", KEEPER_PUBLIC_URL: "https://keeper.example", ...extra };
  if (o.send) { env.KEEPER_MODE = "send"; env.KEEPER_PRIVATE_KEY = key; }
  chain.serveDir = join(dir, "state", "snapshots");
  // one RAFFLE coin with a whole unit; three holders (the curve excluded), long before any snapshot time
  chain.coins = [{ coin: COIN, policy: 0, curve: CURVE }, { coin: A(0x2003), policy: 1, curve: A(0x2004) }];
  chain.units.set(COIN.toLowerCase(), E18);
  chain.units.set(A(0x2003).toLowerCase(), 5n * E18);     // a BURN coin: never drawn
  chain.transfers = [{ block: 500, from: A(0), to: H1, value: 50n * E18 }, { block: 600, from: A(0), to: H2, value: 25n * E18 },
    { block: 700, from: A(0), to: CURVE, value: 900n * E18 }, { block: 800, from: CURVE, to: H3, value: 5n * E18 }];
  return { chain, dir, env, lines: [], key, address, now: { ms: 1_790_000_000_000 } };
}
const built = (w: World, argv: string[] = []): Promise<Built> =>
  build(argv, w.env, { fetchImpl: w.chain.fetch, write: (l) => w.lines.push(l), sleep: async () => { w.chain.advance(1); }, nowMs: () => w.now.ms });
/** Move the chain (and the keeper's wall clock with it). */
const later = (w: World, s: number) => { w.chain.advance(s); w.now.ms += s * 1000; };
const expected = (w: World, block: number, at: number) => snapshot(balancesOf(w.chain.transfers.filter((t) => t.block <= block).map((t, i) => ({ blockNumber: BigInt(t.block), logIndex: i,
  from: t.from, to: t.to, value: t.value }))), { coin: COIN, block: String(block), kind: "weighted", decimals: 18, exclusions: [CURVE.toLowerCase()], snapshotAt: String(at) });

test("the draw: announced at the slot; opened once two relays serve the seed round and the beacon verifies it; the snapshot at the last block ≤ snapshotAt, with the sentinel; published and read back", async () => {
  const w = world();
  const b = await built(w);
  const r1 = await runPass(b);
  const ann = r1.collections[0].draws.find((d) => d.action === "announce")!;
  assert.equal(ann.result?.outcome, "mined", ann.plan);
  assert.equal(w.chain.draws.length, 1);
  assert.equal(w.chain.draws[0].stage, 7, "Announced");
  assert.equal(w.chain.units.get(COIN.toLowerCase()), 0n, "the unit is reserved at the announcement");
  assert.equal(r1.collections[0].draws.filter((d) => d.coin.toLowerCase() === A(0x2003).toLowerCase()).length, 0, "a BURN coin is never drawn");
  // the seed round does not exist yet (20 rounds ahead): nothing is opened, the pass says why
  const r2 = await runPass(b);
  assert.match(r2.collections[0].draws.find((d) => d.action === "open")!.plan, /waiting for drand round/);
  assert.equal(w.chain.draws[0].stage, 7);
  // 70 s later the round exists: opened with the round's signature and a tree that holds the sentinel
  later(w, 70);
  const r3 = await runPass(b);
  const open = r3.collections[0].draws.find((d) => d.action === "open")!;
  assert.equal(open.result?.outcome, "mined", open.plan);
  const dr = w.chain.draws[0];
  assert.equal(dr.stage, 1, "Open");
  const at = dr.snapshotAt;
  assert.ok(at >= dr.announcedAt - 86_400 && at < dr.announcedAt, "the snapshot time is a moment of the day before the announcement");
  assert.equal(open.snapshotBlock, w.chain.lastBlockAtOrBefore(at), "the last block stamped at or before snapshotAt");
  const want = expected(w, open.snapshotBlock!, at);
  assert.equal(dr.root, want.root, "the root is tools/snapshot's v2 root (the curve excluded, the sentinel in)");
  assert.equal(dr.totalTickets, BigInt(want.totalTickets!));
  assert.equal(dr.uri, `https://keeper.example/snapshots/snapshot-${COIN.toLowerCase()}-${open.snapshotBlock}-weighted.json`);
  const file = JSON.parse(readFileSync(join(w.dir, "state", "snapshots", `snapshot-${COIN.toLowerCase()}-${open.snapshotBlock}-weighted.json`), "utf8")) as Snapshot;
  assert.equal(file.version, 2);
  assert.ok(verify(file.sentinel!.proof, file.root, sentinelLeaf(BigInt(file.totalTickets!), BigInt(at))));
  // the same slot again: not announced twice (one per coin per slot; the vault: one per coin before its pin)
  w.chain.units.set(COIN.toLowerCase(), E18);
  const r4 = await runPass(b);
  assert.match(r4.collections[0].draws.find((d) => d.action === "announce")!.plan, /already announced/);
  assert.equal(w.chain.draws.length, 1);
});

test("the resolver finishes the draw with no keeper power: requestDraw after the challenge window, resolve with the pinned round, claim for the winner with the published proof", async () => {
  const w = world();
  const b = await built(w);
  await runPass(b); later(w, 70); await runPass(b);
  assert.equal(w.chain.draws[0].stage, 1);
  // in the challenge window: nothing
  let r = await runPass(b);
  assert.ok(r.resolver!.actions.some((a) => /challenge window/.test(a.plan)));
  // 15 minutes and 75 parent blocks later: requested, pinned to a round that does not exist yet
  later(w, 901);
  r = await runPass(b);
  assert.equal(w.chain.draws[0].stage, 2, "Pinned");
  assert.ok(r.resolver!.actions.some((a) => /request frogs draw 0/.test(a.what) && a.result?.outcome === "mined"));
  later(w, 70);
  r = await runPass(b);
  assert.equal(w.chain.draws[0].stage, 3, "Resolved with the pinned round's real (fake) signature");
  r = await runPass(b);
  assert.equal(w.chain.draws[0].stage, 4, "Claimed");
  const dr = w.chain.draws[0];
  const file = JSON.parse(readFileSync(join(w.dir, "state", "snapshots", dr.uri.split("/").pop()!), "utf8")) as Snapshot;
  const winner = file.entries.find((e) => BigInt(e.first!) <= dr.winningTicket && dr.winningTicket < BigInt(e.end!))!;
  assert.deepEqual(w.chain.claimed, [{ id: 0, holder: winner.holder }], "the holder whose range holds the winning ticket");
  assert.ok([H1, H2, H3].map((h) => h.toLowerCase()).includes(winner.holder));
});

test("claims never depend on the file alone: with the published file gone, the resolver rebuilds the tree from the chain and claims only under the draw's root", async () => {
  const w = world();
  const b = await built(w);
  await runPass(b); later(w, 70); await runPass(b); later(w, 901); await runPass(b); later(w, 70); await runPass(b);
  assert.equal(w.chain.draws[0].stage, 3);
  w.chain.serveDir = null;                                        // the keeper's host is gone, and so is its volume
  const { rmSync } = await import("node:fs");
  rmSync(join(w.dir, "state", "snapshots"), { recursive: true });
  await runPass(b);
  assert.equal(w.chain.draws[0].stage, 4, "claimed from a tree rebuilt from Transfer logs at snapshotAt");
});

test("drand: two relays must agree; one relay down is 'waiting'; a relay serving other bytes is outvoted by the beacon and alerted (R2-S2)", async () => {
  const w = world();
  const b = await built(w);
  await runPass(b); later(w, 70);
  w.chain.relayDown = new Set(["api.drand.sh", "api2.drand.sh"]);       // only one relay answers
  let r = await runPass(b);
  assert.match(r.collections[0].draws.find((d) => d.action === "open")!.plan, /waiting for drand round/);
  assert.equal(w.chain.draws[0].stage, 7);
  w.chain.relayDown = new Set(["api2.drand.sh"]); w.chain.relayLie = true;   // Cloudflare lies and only one honest relay answers
  r = await runPass(b);
  assert.match(r.collections[0].draws.find((d) => d.action === "open")!.plan, /second relay/);
  assert.ok(r.alerts.some((a) => /drand: draw 0/.test(a) && /drand\.cloudflare\.com/.test(a)));
  assert.equal(w.chain.draws[0].stage, 7, "the beacon's signature from one relay only: waiting");
  w.chain.relayDown = new Set();                                          // two honest relays and the liar: the beacon picks
  r = await runPass(b);
  assert.equal(w.chain.draws[0].stage, 1, "opened with the signature the beacon accepts");
  assert.ok(r.alerts.some((a) => /^drand-liar: /.test(a) && /drand\.cloudflare\.com/.test(a)), "the liar is named");
});

test("never a quiet skip: a slot whose due draw is not announced in the window is an alert; a draw not opened in time is voided by the resolver", async () => {
  const w = world({ KEEPER_PUBLIC_URL: "" });
  delete w.env.KEEPER_PUBLIC_URL;
  const b = await built(w);
  let r = await runPass(b);
  assert.match(r.collections[0].draws[0].plan, /no public URL/);
  assert.ok(r.alerts.some((a) => /draw-not-announced/.test(a)));
  later(w, 400);                                                  // past the 300 s window
  r = await runPass(b);
  assert.ok(r.alerts.some((a) => /draw-slot-missed: frogs/.test(a)), r.alerts.join("\n"));
  // a draw announced and never opened (no relay ever serves its round) is voided after 6 h, and its unit returns to the coin
  const v = world();
  const c = await built(v);
  await runPass(c);
  v.chain.relayDown = new Set(["api.drand.sh", "api2.drand.sh", "drand.cloudflare.com"]);
  later(v, 6 * 3_600 - 800);
  r = await runPass(c);
  assert.ok(r.alerts.some((a) => /draw-not-opened: draw 0/.test(a)), "too late to open: said so");
  later(v, 900);
  r = await runPass(c);
  assert.equal(v.chain.draws[0].stage, 6, "Voided by the resolver");
  assert.ok(r.resolver!.actions.some((a) => /void frogs draw 0 \(announced, never opened\)/.test(a.what)));
});

test("E-11: a log scan over the provider's 10,000-log cap is split, not skipped; logs are cached, so the next draw reads only new blocks", async () => {
  const w = world();
  w.chain.logsLimitBlocks = 5_000;                                 // every range over 5,000 blocks is refused as too large
  const b = await built(w);
  await runPass(b); later(w, 70);
  const r = await runPass(b);
  assert.equal(r.collections[0].draws.find((d) => d.action === "open")!.result?.outcome, "mined");
  const cache = join(w.dir, "state", "logs", `${COIN.toLowerCase()}.json`);
  assert.ok(existsSync(cache));
  const cursor = JSON.parse(readFileSync(cache, "utf8")).cursor as number;
  assert.ok(cursor > 1_000, "the confirmed history is written down");
  // a later pass that needs the logs again reads only past the cursor
  const before = w.chain.asked.filter((m) => m === "eth_getLogs").length;
  const logs = await b.logs.upTo(COIN, 10, w.chain.head - 1, cursor);
  const after = w.chain.asked.filter((m) => m === "eth_getLogs").length;
  assert.equal(logs.length, 4);
  assert.ok(after - before <= Math.ceil((w.chain.head - 1 - cursor) / 5_000) + 2, `${after - before} reads for the blocks past the cursor only`);
});

test("E-14: an unreadable exclusions list stops the draw (alerted), never an empty list", async () => {
  const w = world();
  const b = await built(w);
  await runPass(b); later(w, 70);
  b.exclusionsFile = join(w.dir, "missing-exclusions.json");
  const r = await runPass(b);
  const open = r.collections[0].draws.find((d) => d.action === "open")!;
  assert.match(open.plan, /exclusions file .* could not be read/);
  assert.ok(r.alerts.some((a) => /snapshot: draw 0/.test(a)));
  assert.equal(w.chain.draws[0].stage, 7, "not opened");
});

test("the mirror: the snapshot is PUT off the keeper's volume (the credential never logged) and the draw's URI is the mirror's, read back byte for byte before openDraw", async () => {
  const AUTH = "Bearer mirror-credential-0123456789abcdef";
  const w = world({ KEEPER_SNAPSHOT_UPLOAD_URL: "https://mirror.example/put", KEEPER_SNAPSHOT_BASE_URL: "https://cdn.example", KEEPER_SNAPSHOT_UPLOAD_AUTH: AUTH });
  const b = await built(w);
  await runPass(b); later(w, 70);
  const r = await runPass(b);
  const open = r.collections[0].draws.find((d) => d.action === "open")!;
  assert.equal(open.result?.outcome, "mined", open.plan);
  assert.match(open.plan, /mirrored/);
  assert.equal(w.chain.puts.length, 1);
  assert.equal(w.chain.puts[0].auth, AUTH);
  assert.match(w.chain.draws[0].uri, /^https:\/\/cdn\.example\/snapshot-0x/);
  assert.ok(existsSync(join(w.dir, "state", "snapshots", w.chain.draws[0].uri.split("/").pop()!)), "and kept on the volume too");
  assert.ok(!w.lines.join("\n").includes("mirror-credential"), "the upload credential is in no log line");
});

test("the R2 mirror: the snapshot is PUT, SigV4-signed, to <bucket>/rook-snapshots/<name>; the draw's URI is the public r2.dev URL, read back byte for byte before openDraw; the secret in no line", async () => {
  const SECRET = "r2-secret-0123456789abcdef0123456789abcdef0123456789abcdef";
  const API = "https://0123abcd.r2.cloudflarestorage.com", PUB = "https://pub-736187aaa62744868846472f72e8d77b.r2.dev";
  const w = world({ KEEPER_SNAPSHOT_S3_ENDPOINT: API, KEEPER_SNAPSHOT_S3_BUCKET: "rook-media", KEEPER_SNAPSHOT_S3_ACCESS_KEY_ID: "0123456789abcdef0123456789abcdef",
    KEEPER_SNAPSHOT_S3_SECRET_ACCESS_KEY: SECRET, KEEPER_SNAPSHOT_BASE_URL: `${PUB}/rook-snapshots` });
  const stub = new S3Stub({ bucket: "rook-media", accessKeyId: "0123456789abcdef0123456789abcdef", secretAccessKey: SECRET });
  const b = await build([], w.env, { fetchImpl: stubFetch(stub, API, PUB, w.chain.fetch as typeof fetch), write: (l) => w.lines.push(l), sleep: async () => { w.chain.advance(1); },
    nowMs: () => w.now.ms });
  await runPass(b); later(w, 70);
  const r = await runPass(b);
  const open = r.collections[0].draws.find((d) => d.action === "open")!;
  assert.equal(open.result?.outcome, "mined", open.plan);
  assert.match(open.plan, /mirrored/);
  const name = w.chain.draws[0].uri.split("/").pop()!;
  assert.equal(w.chain.draws[0].uri, `${PUB}/rook-snapshots/${name}`, "the URI on chain is the bucket's public URL");
  assert.deepEqual([...stub.objects.keys()], [`rook-snapshots/${name}`]);
  assert.equal(stub.objects.get(`rook-snapshots/${name}`)!.body, readFileSync(join(w.dir, "state", "snapshots", name), "utf8"), "the bytes on the volume");
  assert.ok(stub.log.every((q) => q.headers.authorization?.startsWith("AWS4-HMAC-SHA256 ")), "every API request signed");
  assert.ok(!w.lines.join("\n").includes(SECRET), "the secret access key is in no log line");
  assert.ok(w.lines.some((l) => l.includes(`s3 0123abcd.r2.cloudflarestorage.com/rook-media/rook-snapshots/`)), "the start line names the bucket");
});

test("the Wrapper's tickets are resolved in order as their rounds exist; the walk stops at the first round not yet served", async () => {
  const w = world();
  const b = await built(w);
  const now = w.chain.headTs;
  w.chain.tickets = [0, 1, 2].map((i) => ({ stage: 1, owner: A(0xb0 + i), round: roundAt(now - 300 + i * 100) + 20n, pinnedAt: now - 300 + i * 100, pinnedAtBlock: 0, deliverBy: 0, acquisitionId: 0n }));
  w.chain.tickets.push({ stage: 1, owner: A(0xb3), round: roundAt(now + 600) + 20n, pinnedAt: now + 600, pinnedAtBlock: 0, deliverBy: 0, acquisitionId: 0n });
  const r = await runPass(b);
  assert.deepEqual(w.chain.tickets.map((t) => t.stage), [2, 2, 2, 1], "three resolved in order; the fourth's round is in the future");
  assert.equal(w.chain.nextToResolve, 3);
  assert.ok(r.resolver!.actions.some((a) => /ticket 3/.test(a.what) && /waiting for drand round/.test(a.plan)));
  assert.ok(timeOf(w.chain.tickets[3].round) > w.chain.headTs);
});

test("the standalone resolver: its own key, no role; it resolves while the keeper is paused (the keeper sends nothing then); the keeper's key is refused", async () => {
  const w = world();
  const k = await built(w);
  await runPass(k); later(w, 70); await runPass(k); later(w, 901);
  const now = w.chain.headTs;
  w.chain.tickets = [{ stage: 1, owner: A(0xb0), round: roundAt(now - 200) + 20n, pinnedAt: now - 200, pinnedAtBlock: 0, deliverBy: 0, acquisitionId: 0n }];
  w.chain.keeperPaused = true;
  const kr = await runPass(k);
  assert.match(kr.refusals.join("\n"), /paused the keeper/);
  const rkey = generatePrivateKey();
  const renv = { ...w.env, RESOLVER_MODE: "send", RESOLVER_PRIVATE_KEY: rkey, RESOLVER_STATE_DIR: join(w.dir, "resolver") };
  delete (renv as Record<string, string>).KEEPER_PRIVATE_KEY; delete (renv as Record<string, string>).KEEPER_MODE;
  const res = await buildResolver([], renv, { fetchImpl: w.chain.fetch, write: (l) => w.lines.push(l), sleep: async () => { w.chain.advance(1); }, nowMs: () => w.now.ms });
  assert.equal(res.from.toLowerCase(), privateKeyToAccount(rkey).address.toLowerCase());
  const r = await runResolverPass(res);
  assert.equal(w.chain.tickets[0].stage, 2, "the redemption is resolved with the keeper paused");
  assert.ok(r.actions.some((a) => /the keeper is paused: requestDraw is refused/.test(a.plan)), "requestDraw waits for the unpause (the vault refuses it)");
  await assert.rejects(buildResolver([], { ...renv, RESOLVER_PRIVATE_KEY: w.key }, { fetchImpl: w.chain.fetch, write: () => {}, nowMs: () => w.now.ms }),
    (e: unknown) => e instanceof Refusal && /is the registry's keeper/.test((e as Error).message));
  // a dry-run resolver holds no key and simulates from a nobody address
  const dry = await buildResolver([], { ...renv, RESOLVER_MODE: "dry-run", RESOLVER_PRIVATE_KEY: "" }, { fetchImpl: w.chain.fetch, write: () => {}, nowMs: () => w.now.ms });
  assert.equal(dry.signer, null);
});

test("E-04: the RAFFLE vault's draws are walked from the lowest one not final; coins are indexed once, a bounded number per pass", async () => {
  const w = world({ KEEPER_COINS_PER_PASS: "50" });
  for (let i = 0; i < 180; i++) w.chain.coins.push({ coin: A(0x300000 + i), policy: 1, curve: A(0x400000 + i) });
  const b = await built(w, ["--dry-run"]);
  const calls = async () => { w.chain.asked.length = 0; const r = await runPass(b); return { n: w.chain.asked.filter((m) => m === "eth_call").length, r }; };
  const p1 = await calls();
  assert.deepEqual(p1.r.coins, { indexed: 50, behind: 132 }, "50 of 182 indexed; the rest next passes");
  await calls(); await calls(); await calls();
  const idle = await calls();
  assert.deepEqual(idle.r.coins, { indexed: 182, behind: 0 });
  const more = w.chain.coins.length;
  for (let i = 0; i < 1_000; i++) w.chain.coins.push({ coin: A(0x500000 + i), policy: 1, curve: A(0x600000 + i) });
  const flood = await calls();
  assert.equal(flood.r.coins!.indexed, more + 50, "a flood of launches is absorbed over passes");
  assert.ok(flood.n - idle.n <= 3 * 50 + 5, `${flood.n - idle.n} more calls for 50 new coins, not ${2 * 1_000}`);
});

test("the keeper's pause: refused every pass while paused, and a pause that lands mid-pass stops the next signature", async () => {
  const w = world();
  const b = await built(w);
  w.chain.keeperPaused = true;
  const r = await runPass(b);
  assert.deepEqual(r.refusals.length, 1);
  assert.equal(w.chain.sent.length, 0);
  w.chain.keeperPaused = false;
  await runPass(b);
  const sent = w.chain.sent.length;
  // paused between the pass's check and a signature: the sender's guard reads it again and refuses (a call whose simulation
  // would still pass: a plain transfer, which no keeper power guards)
  w.chain.keeperPaused = true;
  const res = await b.sender.submit({ key: "x", what: "a call that simulates", to: A(0x999), data: "0x" });
  assert.equal(res.outcome, "refused");
  assert.match(res.detail, /paused the keeper/);
  assert.equal(w.chain.sent.length, sent, "nothing signed while paused");
});
