import assert from "node:assert/strict";
import { test } from "node:test";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { ReserveReport } from "../../indexer/types.js";
import { Telegram } from "../../monitor/src/alerts.mjs";
import { alert, runOnce, Store, type Channel, type ReconcileConfig } from "../src/reconcile.js";
import type { CapsRead } from "../src/stage.js";

/**
 * AUDIT E (services and workflows) proofs of concept; no source is changed.
 *  E-08a: a reconciler that can read nothing (every 4663 RPC down, or the canister unreadable) is UNVERIFIED forever. Each hourly
 *         run refreshes computedAt, so it is never "stale", never red, and never alerted: a week of silence.
 *  E-08b: with ALERT_TELEGRAM_BOT_TOKEN or _CHAT_ID missing (a typo in Railway), Telegram silently becomes a dry run: a RED is
 *         "delivered" to the log, recorded as sent, and never repeated to a phone.
 * Run: cd ops && node --experimental-sqlite --no-warnings --import tsx --test test/audit_e.test.ts
 */
const cfg = (dir: string): ReconcileConfig => ({ stackPath: "", chainId: 4663, rpcUrls: [], canisterId: "aaaaa-aaaaa-aaaaa-aaaaa-aaa", icHost: "", everyS: 3_600, staleS: 10_800,
  realertS: 21_600, stateDir: dir, port: null, publicUrl: null });
const capsDown = async (): Promise<CapsRead> => ({ kind: "error", error: "IC unreachable", stackStage: 0, at: 1 });

test("E-08a PoC: a reconciler that reads nothing is unverified for a week, and posts nothing", async () => {
  const dir = mkdtempSync(join(tmpdir(), "audit-e-rec-"));
  const posted: string[] = [];
  const channel: Channel = { send: async (t) => { posted.push(t); } };
  let now = 1_790_000_000_000;
  const d = { cfg: cfg(dir), service: { reserves: async (): Promise<ReserveReport[]> => { throw new Error("no RPC reports chain id 4663"); } }, caps: capsDown,
    channel, store: new Store(dir), log: () => {}, nowMs: () => now, stackStage: 0 };
  let last;
  for (let h = 0; h < 24 * 7; h++) { last = await runOnce(d); now += 3_600_000; }
  console.log(`E-08a: after 168 hourly runs: served ${last!.servedStatus}, stale=${last!.stale}, ${posted.length} alert(s) posted; reasons: ${last!.servedReasons.slice(0, 2).join(" | ")}`);
  assert.equal(last!.servedStatus, "unverified");
  assert.equal(last!.stale, false, "every run refreshes computedAt, so 'stale is red' never fires");
  assert.equal(posted.length, 0, "nothing reaches the operator for a week");
});

test("E-08b PoC: no Telegram token (or chat id) ⇒ a RED is swallowed as a dry run and recorded as sent", async () => {
  const dir = mkdtempSync(join(tmpdir(), "audit-e-rec2-"));
  const logged: string[] = [];
  const tg = new Telegram({ token: undefined, chat: "123", log: { log: (m: string) => logged.push(m), error: (m: string) => logged.push(m) } as unknown as Console });
  const store = new Store(dir);
  const red = { version: 1 as const, computedAt: 1_790_000_000, status: "red" as const, reasons: ["frogs: 1 item found outside custody"], block: 1, chainId: 4663, collections: [],
    caps: { kind: "absent" as const, note: "", stackStage: 0, at: 0 }, stage: "stage 0" };
  store.save({ last: red, alerts: { lastStatus: null, redKey: null, redAt: 0 } });
  const s = await alert({ cfg: cfg(dir), channel: tg as unknown as Channel, store, log: () => {} }, 1_790_000_060_000);
  const after = store.load().alerts;
  console.log(`E-08b: served ${s.servedStatus}; telegram.dryRun=${(tg as unknown as { dryRun: boolean }).dryRun}; alert state now ${JSON.stringify(after)}; ${logged.length} line(s) only in the log`);
  assert.equal(s.servedStatus, "red");
  assert.equal((tg as unknown as { dryRun: boolean }).dryRun, true);
  assert.ok(after.redKey, "the red is recorded as alerted: it will not be tried again for 6 hours, and nothing reached a phone");
});
