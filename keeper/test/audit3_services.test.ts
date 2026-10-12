import assert from "node:assert/strict";
import { test } from "node:test";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { generatePrivateKey, privateKeyToAccount } from "viem/accounts";
import type { Snapshot } from "../../tools/snapshot/src/core.js";
import { parseServiceConfig } from "../service/config.js";
import { build, type Built } from "../service/main.js";
import { runPass } from "../service/keeper.js";
import { REBUILD_RETRY_MS } from "../service/resolver.js";
import { AlertLatch, healthy, type Status } from "../service/status.js";
import { A, COIN, CURVE, FakeChain, stackJson } from "./service-fake.js";

// The targeted check of round 3 (docs/audit/2026-09-26/review3/R3-check.md), keeper Lows L-D and L-G, fixed in round 4
// (review3/fixes-R4-services.md §7). Each test asserts the gap is closed.
const E18 = 10n ** 18n;
const H1 = A(0xa1), H2 = A(0xa2), H3 = A(0xa3);
const CHAIN = 9466301;
const TRANSFERS = [{ block: 500, from: A(0), to: H1, value: 50n * E18 }, { block: 600, from: A(0), to: H2, value: 25n * E18 },
  { block: 700, from: A(0), to: CURVE, value: 900n * E18 }, { block: 800, from: CURVE, to: H3, value: 5n * E18 }];

type World = { chain: FakeChain; dir: string; env: Record<string, string>; lines: string[]; now: { ms: number } };
function world(): World {
  const key = generatePrivateKey(), address = privateKeyToAccount(key).address;
  const chain = new FakeChain({ keeper: address, chainId: CHAIN });
  const dir = mkdtempSync(join(tmpdir(), "keeper-audit3-"));
  writeFileSync(join(dir, "stack.json"), stackJson(CHAIN));
  writeFileSync(join(dir, "floors.json"), JSON.stringify({ collections: { frogs: { priceWei: "158000000000000000", confidenceBps: 8_000, depthItems: 2 } } }));
  const env: Record<string, string> = { KEEPER_CHAIN_ID: String(CHAIN), KEEPER_STACK: join(dir, "stack.json"), KEEPER_STATE_DIR: join(dir, "state"),
    KEEPER_RPC_URLS: "http://127.0.0.1:1", KEEPER_FLOOR_SOURCE: "fixture", KEEPER_FLOOR_FIXTURE: join(dir, "floors.json"),
    KEEPER_DRAW_EVERY_S: "600", KEEPER_DRAW_OFFSET_S: "150", KEEPER_DRAW_WINDOW_S: "300", KEEPER_PUBLIC_URL: "https://keeper.example",
    KEEPER_MODE: "send", KEEPER_PRIVATE_KEY: key };
  chain.serveDir = join(dir, "state", "snapshots");
  chain.coins = [{ coin: COIN, policy: 0, curve: CURVE }];
  chain.units.set(COIN.toLowerCase(), E18);
  chain.transfers = [...TRANSFERS];
  return { chain, dir, env, lines: [], now: { ms: 1_790_000_000_000 } };
}
const built = (w: World): Promise<Built> =>
  build([], w.env, { fetchImpl: w.chain.fetch, write: (l) => w.lines.push(l), sleep: async () => { w.chain.advance(1); }, nowMs: () => w.now.ms });
const later = (w: World, s: number) => { w.chain.advance(s); w.now.ms += s * 1000; };

test("audit3 L-D (fixed): a chain rebuild that did not match ONCE (the resolver's log provider returned a log short) is not cached as 'no entry' for the life of the process: after its backoff it is rebuilt from the chain again, not from the short copy, and the winner is claimed", async () => {
  const w = world();
  const b = await built(w);
  await runPass(b); later(w, 70); await runPass(b); later(w, 901); await runPass(b); later(w, 70);
  const dr0 = w.chain.draws[0];
  const real = JSON.parse(readFileSync(join(w.dir, "state", "snapshots", dr0.uri.split("/").pop()!), "utf8")) as Snapshot;
  // the published file forged, the keeper's own copy gone: only the chain rebuild can prove
  w.chain.files.set(dr0.uri, JSON.stringify({ ...real, entries: real.entries.map((e) => ({ ...e, holder: A(0xbad).toLowerCase(), proof: ["0x" + "11".repeat(32)] })) }));
  rmSync(join(w.dir, "state", "snapshots"), { recursive: true }); w.chain.serveDir = null;
  rmSync(join(w.dir, "state", "logs", `${COIN.toLowerCase()}.json`), { force: true });
  w.chain.transfers = TRANSFERS.filter((t) => t.to !== H2);                  // the provider is short one log, this once
  await runPass(b); later(w, 60);                                             // the resolve
  await runPass(b); later(w, 60);                                             // the claim: the rebuild does not prove
  assert.equal(w.chain.draws[0].stage, 3, "not claimed on a short read");
  assert.equal(w.lines.filter((l) => /claim-rebuild-mismatch/.test(l)).length, 1);
  w.chain.transfers = [...TRANSFERS];                                          // the provider is whole again
  await runPass(b); later(w, 60);
  assert.equal(w.chain.draws[0].stage, 3, "within the backoff: not rebuilt yet");
  later(w, REBUILD_RETRY_MS / 1000);
  for (let i = 0; i < 3 && Number(w.chain.draws[0].stage) !== 4; i++) { await runPass(b); later(w, 60); }
  assert.equal(Number(w.chain.draws[0].stage), 4, "Claimed, from the second rebuild (before, the draw waited for its void after 7 days)");
  assert.notEqual(w.chain.claimed[0].holder, A(0xbad).toLowerCase());
});

test("audit3 L-G (fixed): an alert raised in ONE pass (draw-slot-missed) stays in force KEEPER_ALERT_LATCH_S, so /healthz turns 503 at the third pass and stays so past the monitor's 10 minutes", () => {
  const l = new AlertLatch(1_200_000);
  const base = { refusals: [], consecutiveFailures: 0, interval: 60, alertPassesLimit: 3 } as unknown as Status;
  let t = 1_000_000;
  const st = (alerts: string[]) => { const r = l.pass(alerts, t); const s = { ...base, lastPassAt: new Date(t).toISOString(), alerts: r.inForce, alertPasses: r.alertPasses } as Status; t += 60_000; return s; };
  assert.equal(healthy(st(["draw-slot-missed: frogs 0xc had a whole unit at slot 1 and no draw was announced in the window"]), t), true, "pass 1");
  assert.equal(healthy(st([]), t), true, "pass 2: the alert is not raised again, but it is in force");
  const third = st([]);
  assert.equal(healthy(third, t), false, "pass 3: 503");
  assert.deepEqual(third.alerts.map((a) => a.split(":")[0]), ["draw-slot-missed"]);
  let down = 0;
  for (let i = 0; i < 30; i++) if (!healthy(st([]), t)) down++;
  assert.ok(down >= 15, `503 for ${down} more minutes: the monitor's liveness (DOWN_MS 10 min) pages it`);
  assert.equal(healthy(st([]), t), true, "and it clears once the latch has passed");
});

test("audit3 L-G (fixed): an alert that flaps (every other pass) reaches KEEPER_ALERT_PASSES; the latch is configurable, never under the interval's floor (R4-check L-S2)", () => {
  const l = new AlertLatch(1_200_000);
  let t = 0, passes = 0;
  for (let i = 0; i < 6; i++) { passes = l.pass(i % 2 ? [] : ["tx-stuck: nonce 7"], t).alertPasses; t += 60_000; }
  assert.equal(passes, 6);
  const none = new AlertLatch(0);
  for (let i = 0; i < 6; i++) { passes = none.pass(i % 2 ? [] : ["tx-stuck: nonce 7"], t).alertPasses; t += 60_000; }
  assert.ok(passes <= 1, "without a latch a flapping alert never counts past one");
  // R4-check L-S2: a latch under the floor for the interval (60 s: (3 + 1) × 60 + 720 = 960 s) is raised to it
  assert.equal(parseServiceConfig([], { KEEPER_CHAIN_ID: "9466301", KEEPER_STACK: "/x.json", KEEPER_ALERT_LATCH_S: "600" }).alertLatchMs, 960_000);
  assert.equal(parseServiceConfig([], { KEEPER_CHAIN_ID: "9466301", KEEPER_STACK: "/x.json", KEEPER_ALERT_LATCH_S: "3600" }).alertLatchMs, 3_600_000);
  assert.equal(parseServiceConfig([], { KEEPER_CHAIN_ID: "9466301", KEEPER_STACK: "/x.json" }).alertLatchMs, 1_200_000);
});
