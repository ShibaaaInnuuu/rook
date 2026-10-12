// Second review (R2), 2026-09-26: PoCs for the reconciler's wave-2 fixes. A passing test means the gap is present.
import assert from "node:assert/strict";
import { test } from "node:test";
import { backingStatus, type Backing } from "../src/backing.js";

const E18 = 10n ** 18n;
const NOW = 1_790_000_000;
const mod = (o: Partial<Backing> = {}): Backing => ({
  slug: "frogs", module: "0xbb", wrapper: "0xaa", block: 1, balance: 10n * E18,
  books: { backing: 5n * E18, reserve: 1n * E18, surplus: 0n, revenue: 0n, gradEth: 0n, rentalEscrow: 0n, renterPool: 0n, owedTotal: 0n }, owed: 6n * E18,
  reserveUnits: 1n * E18, ethBacked: 10n * E18, accepted: 6n * E18 / 10n, fresh: true, escaped: false, postedAt: NOW - 60, lastGoodAt: NOW - 60, floorTtl: 3600, ...o });

test("R2-O1: a module frozen by confidence-0 re-posts (E-02a) stays GREEN on the reserves page: the stale rule reads postedAt, not lastGoodAt", () => {
  // the thief posts confidence 0 every 50 minutes: postedAt stays recent, fresh() is false, the last GOOD post was 3 days ago
  const frozen = mod({ fresh: false, postedAt: NOW - 50 * 60, lastGoodAt: NOW - 3 * 86_400 });
  assert.deepEqual(backingStatus({ kind: "read", at: NOW, modules: [frozen] }, NOW), { status: "green", reasons: [] });
  // the same module, silent instead of frozen, is red: the rule is only about silence
  const silent = mod({ fresh: false, postedAt: NOW - 3 * 86_400, lastGoodAt: NOW - 3 * 86_400 });
  assert.equal(backingStatus({ kind: "read", at: NOW, modules: [silent] }, NOW).status, "red");
});

import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { parseConfig } from "../../indexer/src/wrapped/config.js";
import { WrappedService } from "../../indexer/src/wrapped/service.js";
import { worldConfig } from "../../indexer/test/wrappedWorld.js";
import { runOnce, Store, type ReconcileConfig } from "../src/reconcile.js";

test("R2-O2: with every 4663 read failing, WrappedService does not throw: the run is 'unverified' with collections, so the immediate 'cannot read' page never fires (only the 6-hour one)", async () => {
  const rpc = (async () => { throw Object.assign(new Error("fetch failed"), { kind: "http" }); }) as never;
  const svc = new WrappedService({ config: parseConfig(worldConfig()), rpc, fetch: (async () => { throw new Error("offline"); }) as never, now: () => NOW, ttlMs: 1 });
  const reports = await svc.reserves();
  assert.ok(reports.length > 0, "reports come back, one per collection");
  assert.ok(reports.every((r) => r.status !== "green"));
  const dir = mkdtempSync(join(tmpdir(), "r2-rec-"));
  const cfg = { stackPath: "", chainId: 4663, rpcUrls: [], canisterId: null, icHost: "", everyS: 3600, staleS: 10_800, realertS: 21_600, stateDir: dir, port: null,
    publicUrl: null, unverifiedAlertS: 21_600 } as ReconcileConfig;
  const posted: string[] = [];
  const s = await runOnce({ cfg, service: svc, caps: async () => ({ kind: "absent", note: "x", stackStage: 0, at: NOW }), channel: { send: async (t) => { posted.push(t); } },
    store: new Store(dir), log: () => {}, nowMs: () => NOW * 1000, stackStage: 0 });
  assert.equal(s.servedStatus, "unverified");
  assert.equal(posted.length, 0, "nothing is paged for the first 6 hours of a reconciler that can read nothing");
});
