// Audit round 4 (R4 check), keeper L-G: the alert latch against the monitor's liveness rule (a /healthz not 200 for DOWN_MS =
// 10 minutes is a page). A pass means the gap is there: with an allowed KEEPER_INTERVAL_S of 300 s or more, a one-pass alert
// (draw-slot-missed, draw-not-opened) is still never paged under the default KEEPER_ALERT_LATCH_S (1,200 s).
import assert from "node:assert/strict";
import { test } from "node:test";
import { AlertLatch } from "../service/status.js";

const DOWN_MS = 10 * 60 * 1000, POLL_MS = 60 * 1000, LATCH_MS = 1_200_000, LIMIT = 3;

/** One alert raised on the first pass only; the keeper passes every `intervalS`; the monitor polls every minute for 2 hours.
 *  Returns how long /healthz was seen 503 in a row, and whether the monitor paged it. */
function run(intervalS: number): { worst503Ms: number; paged: boolean } {
  const latch = new AlertLatch(LATCH_MS);
  let passes = 0, inForce: string[] = [];
  let downSince: number | null = null, worst = 0, paged = false;
  for (let t = 0, nextPass = 0; t <= 2 * 3600_000; t += 1000) {
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

test("audit4 L-G: at the default 60 s interval a one-pass alert is paged (the fix works as designed)", () => {
  assert.equal(run(60).paged, true);
});

test("audit4 L-G (gap): at KEEPER_INTERVAL_S 300 (allowed: 15-1,800) the one-pass alert is 503 for under 10 minutes and never paged", () => {
  const r = run(300);
  assert.equal(r.paged, false, JSON.stringify(r));
});

test("audit4 L-G (gap): at KEEPER_INTERVAL_S 600 or more it never turns /healthz 503 at all", () => {
  for (const i of [600, 900, 1800]) assert.equal(run(i).worst503Ms, 0, `interval ${i}`);
  for (const i of [600, 900, 1800]) assert.equal(run(i).paged, false, `interval ${i}`);
});
