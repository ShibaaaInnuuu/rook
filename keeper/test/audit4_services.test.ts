// Audit round 4 (R4 check), keeper L-S2, now a regression (docs/audit/2026-09-26/review4/fixes-golive-lows.md §4). The reviewer's
// PoC (review4/poc/audit4_services.test.ts) showed that with KEEPER_INTERVAL_S of 300 s or more a one-pass alert (draw-slot-missed,
// draw-not-opened) was never paged under the fixed 1,200 s latch. The latch the config computes now scales with the interval, for
// the keeper and the resolver alike, and the same simulation of the monitor's liveness rule pages it at every allowed interval.
import assert from "node:assert/strict";
import { test } from "node:test";
import { parseServiceConfig } from "../service/config.js";
import { AlertLatch, alertLatchFloorMs } from "../service/status.js";

const DOWN_MS = 10 * 60 * 1000, POLL_MS = 60 * 1000, LIMIT = 3;

/** One alert raised on the first pass only; the keeper passes every `intervalS`; the monitor polls every minute for 3 hours.
 *  Returns how long /healthz was seen 503 in a row, and whether the monitor paged it. */
function run(intervalS: number, latchMs: number): { worst503Ms: number; paged: boolean } {
  const latch = new AlertLatch(latchMs);
  let passes = 0, inForce: string[] = [];
  let downSince: number | null = null, worst = 0, paged = false;
  for (let t = 0, nextPass = 0; t <= 3 * 3600_000; t += 1000) {
    if (t >= nextPass) {
      const l = latch.pass(t === 0 ? ["draw-slot-missed: slot 123 was not announced"] : [], t);
      inForce = l.inForce; passes = l.alertPasses; nextPass += intervalS * 1000;
    }
    if (t % POLL_MS === 0) {
      const down = inForce.length > 0 && passes >= LIMIT;
      if (down) { downSince ??= t; worst = Math.max(worst, t - downSince); if (t - downSince >= DOWN_MS) paged = true; } else downSince = null;
    }
  }
  return { worst503Ms: worst, paged };
}

const INTERVALS = [15, 60, 300, 600, 900, 1_800];

test("audit4 L-S2 (fixed): the keeper's latch scales with KEEPER_INTERVAL_S, and a one-pass alert is paged at every allowed interval", () => {
  for (const i of INTERVALS) {
    const cfg = parseServiceConfig([], { KEEPER_CHAIN_ID: "9466301", KEEPER_STACK: "/x.json", KEEPER_INTERVAL_S: String(i) });
    assert.equal(cfg.alertLatchMs, Math.max(1_200_000, alertLatchFloorMs(i, 3)), `interval ${i}`);
    const r = run(i, cfg.alertLatchMs);
    assert.equal(r.paged, true, `interval ${i}: ${JSON.stringify(r)} (was: not paged at 300 s and above)`);
  }
});

test("audit4 L-S2 (fixed): the resolver's latch scales with RESOLVER_INTERVAL_S the same way", () => {
  for (const i of INTERVALS) {
    const cfg = parseServiceConfig([], { KEEPER_CHAIN_ID: "9466301", KEEPER_STACK: "/x.json", RESOLVER_INTERVAL_S: String(i) }, "resolver");
    assert.equal(cfg.alertLatchMs, Math.max(1_200_000, alertLatchFloorMs(i, 3)), `interval ${i}`);
    assert.equal(run(i, cfg.alertLatchMs).paged, true, `resolver interval ${i}`);
  }
});

test("audit4 L-S2 (fixed): a latch set below the floor (KEEPER_ALERT_LATCH_S=0 or 600) is raised to it; KEEPER_ALERT_PASSES counts too", () => {
  for (const latch of ["0", "600"]) {
    const cfg = parseServiceConfig([], { KEEPER_CHAIN_ID: "9466301", KEEPER_STACK: "/x.json", KEEPER_INTERVAL_S: "300", KEEPER_ALERT_LATCH_S: latch });
    assert.equal(cfg.alertLatchMs, alertLatchFloorMs(300, 3));
    assert.equal(run(300, cfg.alertLatchMs).paged, true);
  }
  const cfg = parseServiceConfig([], { KEEPER_CHAIN_ID: "9466301", KEEPER_STACK: "/x.json", KEEPER_INTERVAL_S: "600", KEEPER_ALERT_PASSES: "5" });
  assert.equal(cfg.alertLatchMs, alertLatchFloorMs(600, 5));
  assert.equal(cfg.alertLatchMs, (6 * 600 + 720) * 1000);
  // what the PoC used (a fixed 1,200 s latch at 300 s) is below the floor, and is exactly what was not paged
  assert.equal(run(300, 1_200_000).paged, false, "the round-4 latch at 300 s: the gap the fix closes");
});
