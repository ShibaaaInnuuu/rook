import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { test } from "node:test";
import type { Abi } from "viem";
import { CURVE_FACTORY, FLOOR_MODULE_FRESH, WAKE_DESK } from "../src/rook/abis.js";
import { NO_LIVES_NOTE, NO_LIVES_OFFER_AFTER_S, READ_WITHIN_S, RECLAIM_NOTE, SERVED_RULE, bondOf, capacityFrom, capacityText, capacityUrl, launchWait, lifeOf, livesFrom, wakeView, type Life } from "../src/rook/dormancy.js";
import { requestRefusal, stackFrom, type StackFile, type StackReader } from "../src/rook/phase.js";

/**
 * docs/v7/dormant-collections.md §4.2: the Launch page on a dormant collection. The runner's GET /collections says each collection's
 * life; a dormant one offers the bonded wake (WakeDesk.request of one of the stack's own wrappers, with exactly the stack file's bond) and
 * holds the launch that buys items; a waking one says "warming up (~N min)"; an active one launches as before. The Phase-1 send guard
 * refuses a wake request for any other wrapper, with any other bond, or to a desk the stack file does not name.
 */
const A = (n: number) => `0x${n.toString(16).padStart(40, "0")}` as `0x${string}`;
const DESK = A(0xde), BOND = 3_000_000_000_000_000n;
const STACK = { chainId: 4663, label: "stack", stage: 0, registry: A(1), owner: A(2), keeper: A(3), treasury: A(4), poolManager: A(6),
  wakeDesk: DESK, wakeBond: BOND.toString(),
  collections: [{ slug: "milady", kind: "nft", wrapper: A(0x10), factory: A(0x11), router: A(0x12), floorModule: A(0x13), raffleVault: A(0x16) },
    { slug: "doodles", kind: "nft", wrapper: A(0x20), factory: A(0x21), router: A(0x22), floorModule: A(0x23), raffleVault: A(0x26) }] };
const s = stackFrom([STACK]).stack as StackFile;
const read: StackReader = { routerOf: async () => A(0), tokenOf: async () => null };
const ME = A(0x3e);
const life = (wrapper: string, state: string, status = state): Life => ({ collection: A(0x99), wrapper, module: A(0x13), state, since: "1", eta_secs: state === "waking" ? ["2400"] : [],
  queue_position: [], woken_by: [], cooldown_until: [], status, note: "" });
const wakeReq = (wrapper: string, value?: bigint, to = DESK) => ({ address: to, abi: WAKE_DESK as unknown as Abi, functionName: "request", args: [wrapper], value });

test("the lives from GET /collections: checked, and matched by the stack's wrapper", () => {
  assert.equal(livesFrom({ nope: 1 }), null);
  assert.equal(livesFrom([{ wrapper: 1 }]), null);
  const lives = livesFrom([life(A(0x10), "dormant", "dormant: a launch wakes it"), life(A(0x20), "waking", "warming up (~40 min)")])!;
  assert.equal(lifeOf(lives, A(0x10).toUpperCase().replace("0X", "0x"))!.state, "dormant");
  assert.equal(lifeOf(lives, A(0x30)), null);
  assert.equal(lifeOf(null, A(0x10)), null);
});

test("what the page says: dormant is the first coin's two steps; waking and queued launch now and graduate later; active launches", () => {
  const d = wakeView(life(A(0x10), "dormant", "dormant: a launch wakes it"));
  assert.ok(d.canWake);
  assert.match(d.launchWait!, /first coin on it, so launching wakes it: step 1 asks for the wake/);
  assert.equal(wakeView(life(A(0x10), "dormant"), true).canWake, false, "the desk already holds an open request");
  const w = wakeView(life(A(0x20), "waking", "warming up (~40 min)"));
  assert.equal(w.word, "warming up (~40 min)");
  assert.match(w.launchWait!, /^warming up \(~40 min\): the canister is reading this collection's last 7 days of sales/);
  assert.equal(w.canWake, false);
  assert.match(w.launchWait!, /You can launch now: your coin trades right away and graduates once the collection's floor is live/);
  assert.match(wakeView(life(A(0x20), "queued", "queued (#2): it warms up when there is room")).launchWait!, /queued \(#2\).*You can launch now/);
  for (const st of ["active", "always-on"]) assert.deepEqual([wakeView(life(A(0x10), st)).launchWait, wakeView(life(A(0x10), st)).canWake], [null, false]);
  assert.equal(wakeView(null).launchWait, null, "no lives served: the page is as before");
});

test("the bond is the stack file's; no desk, no wake offered", () => {
  assert.equal(bondOf(s), BOND);
  assert.equal(bondOf({ wakeBond: "1" }), null);
  assert.equal(bondOf({ wakeDesk: A(0), wakeBond: "1" }), null);
  assert.equal(bondOf(null), null);
});

test("the send guard: a wake request goes only to the stack's desk, for one of its wrappers, with exactly its bond", async () => {
  assert.equal(await requestRefusal(s, wakeReq(A(0x20), BOND), ME, read), null);
  assert.match(String(await requestRefusal(s, wakeReq(A(0x77), BOND), ME, read)), /not a wrapper of this deployment/);
  assert.match(String(await requestRefusal(s, wakeReq(A(0x20), BOND * 2n), ME, read)), /not the stack file's bond/);
  assert.match(String(await requestRefusal(s, wakeReq(A(0x20)), ME, read)), /not the stack file's bond/);
  assert.match(String(await requestRefusal(s, wakeReq(A(0x20), BOND, A(0xbad)), ME, read)), /not a contract of this deployment/);
  // only `request`: nothing else is called on the desk
  assert.match(String(await requestRefusal(s, { address: DESK, abi: WAKE_DESK as unknown as Abi, functionName: "reclaim", args: [0n, 0n], value: BOND }, ME, read)),
    /does not call reclaim/);
  // a stack without a desk: the desk is not a target at all
  const noDesk = stackFrom([{ ...STACK, wakeDesk: undefined }]).stack as StackFile;
  assert.match(String(await requestRefusal(noDesk, wakeReq(A(0x20), BOND), ME, read)), /not a contract of this deployment/);
  // the launch itself is unchanged
  assert.equal(await requestRefusal(s, { address: A(0x11), abi: CURVE_FACTORY as unknown as Abi, functionName: "launch", args: ["C", "C", 0, false, 100] }, ME, read), null);
});

test("the operator, 2026-10-09: a launch never waits for the floor; it waits only for step 1, the wake request a sleeping collection needs", () => {
  const active = wakeView(life(A(0x10), "active")), dormant = wakeView(life(A(0x10), "dormant", "dormant"));
  const waking = wakeView(life(A(0x10), "waking", "warming up (~40 min)")), asked = wakeView(life(A(0x10), "dormant"), true);
  // fresh on chain: it goes, whatever the relay says
  assert.equal(launchWait({ dormancy: true, sweepOnly: false, fresh: true, view: wakeView(null) }), null);
  assert.equal(launchWait({ dormancy: true, sweepOnly: false, fresh: true, view: dormant }), null);
  // not fresh, or not read yet: it still goes (the contract holds the graduation until a fresh floor, then refunds at the window's end)
  assert.equal(launchWait({ dormancy: true, sweepOnly: false, fresh: false, view: active }), null);
  assert.equal(launchWait({ dormancy: true, sweepOnly: false, fresh: undefined, view: active }), null);
  assert.equal(launchWait({ dormancy: true, sweepOnly: false, fresh: false, view: waking }), null);
  assert.equal(launchWait({ dormancy: true, sweepOnly: false, fresh: false, view: asked }), null, "a wake already asked: launch now");
  // a sleeping collection nobody has asked to wake: step 1 first
  assert.match(String(launchWait({ dormancy: true, sweepOnly: false, fresh: false, view: dormant })), /Step 1 first: ask for the wake/);
  // floor-buys-only never waits; no dormancy: as before
  assert.equal(launchWait({ dormancy: true, sweepOnly: true, fresh: false, view: dormant }), null);
  assert.equal(launchWait({ dormancy: false, sweepOnly: false, fresh: undefined, view: active }), null);
  // the bond's terms are said where the wake is offered
  assert.ok(dormant.detail.includes(RECLAIM_NOTE));
});

test("§11: the capacity from GET /capacity says N of M collections can be awake right now, and what a wake asked now does", () => {
  const cap = { at: "5", balance: "40000000000000", scaling: true, collections: 10, ceiling: 10, wakes_per_day: 12, effective_max_active: 1, awake: 0, pinned: 0,
    wake_headroom: 1, wakes_left_today: 12, next_wake_at: "52600000000000", status: "1 of 10 collections can be awake right now (0 awake)" };
  const c = capacityFrom(cap)!;
  assert.equal(capacityText(c)!.line, "1 of 10 collections can be awake right now");
  assert.match(capacityText(c)!.note, /a wake asked now can start/);
  assert.match(capacityText({ ...c, awake: 1, wake_headroom: 0 })!.note, /waits in the queue for room/);
  const short = capacityText({ ...c, effective_max_active: 0, awake: 2, pinned: 2, wake_headroom: 0 })!;
  assert.equal(short.line, "0 of 10 collections can be awake right now");
  assert.match(short.note, /2 are awake \(2 with holders, kept awake\); a new wake waits in the queue until the canister's running budget is topped up/);
  assert.match(short.note, /grows as the budget does/);
  // anything else is not a capacity: the page says nothing
  for (const bad of [null, [], "x", { ...cap, effective_max_active: "1" }, { ...cap, awake: -1 }, { ...cap, effective_max_active: 11 }, { ...cap, scaling: "yes" }])
    assert.equal(capacityFrom(bad), null, JSON.stringify(bad));
  assert.equal(capacityText(null), null);
  // GET /capacity sits next to the runner's GET /collections
  assert.equal(capacityUrl("https://runner.example/collections"), "https://runner.example/capacity");
  assert.equal(capacityUrl("https://runner.example/api/collections/"), "https://runner.example/api/capacity");
  assert.equal(capacityUrl("https://runner.example/lives"), null);
  assert.equal(capacityUrl("/runner/collections"), "/runner/capacity", "a same-origin path");
  assert.equal(capacityUrl("//other.example/collections"), null, "a protocol-relative URL is not a same-origin path");
  assert.equal(capacityUrl(undefined), null);
  assert.equal(capacityUrl("not a url"), null);
});

// ---- independent review 1 (docs/audit/2026-09-27-nft/dormant-review-independent-1.md) ---------------------------------------------------

test("DI-M-2: an open request on a dormant collection is worded from the canister's life and the request's age, not openOf alone", () => {
  const now = 1_800_000_000, W = 7 * 86_400;
  const dormant = life(A(0x10), "dormant", "dormant: a launch wakes it");
  // just asked: the canister reads it at its next tick
  const fresh = wakeView(dormant, { id: 3n, at: now - 60 }, { dormancy: true, fresh: false, windowSecs: W, nowS: now });
  assert.equal(fresh.canWake, false);
  assert.match(fresh.launchWait!, /request #3\); the canister reads it at its next tick/);
  // asked long ago and still dormant: the canister could not serve it inside its window; its bond goes back; a new request after the window
  const old = wakeView(dormant, { id: 3n, at: now - 6 * 86_400 }, { dormancy: true, fresh: false, windowSecs: W, nowS: now });
  assert.equal(old.canWake, false);
  assert.match(old.launchWait!, /could not start that wake in time/);
  assert.match(old.launchWait!, /goes back to its requester \(WakeDesk.forfeit\) unless the collection's floor is posted inside the window by another wake/);
  assert.match(old.launchWait!, new RegExp(new Date((now - 6 * 86_400 + W) * 1000).toISOString().slice(0, 10)));
  assert.ok(!/within a tick/.test(old.launchWait!), "the old wording promised a warm-up within a tick");
  assert.ok(now - (now - 60) < READ_WITHIN_S);
  // the desk refuses a request while the module is fresh (the collection is awake): none is offered then
  assert.equal(wakeView(dormant, false, { dormancy: true, fresh: true }).canWake, false);
  assert.equal(wakeView(dormant, false, { dormancy: true, fresh: false }).canWake, true);
  // openOf not read yet: no wake offered
  assert.equal(wakeView(dormant, null, { dormancy: true, fresh: false }).canWake, false);
  // queued and waking lives keep their own words whatever openOf says
  assert.match(wakeView(life(A(0x10), "queued", "queued (#1): it warms up when there is room"), { id: 1n, at: now - 5 * 86_400 }).launchWait!, /queued \(#1\)/);
  // the bond's terms: served in time or refunded, a late wake does not charge
  assert.match(RECLAIM_NOTE, /inside that window/);
  assert.match(RECLAIM_NOTE, /WakeDesk.withdraw/);
});

test("DI-L-1: with no lives served, a module read not fresh on chain offers the wake, gated on the openOf read", () => {
  const ctx = { dormancy: true, fresh: false, postedAt: 1_800_000_000 - 3 * 86_400, nowS: 1_800_000_000 } as const;
  const v = wakeView(null, false, ctx);
  assert.equal(v.canWake, true);
  assert.match(String(launchWait({ dormancy: true, sweepOnly: false, fresh: false, view: v })), /Step 1 first/);
  assert.ok(v.detail.includes(RECLAIM_NOTE));
  assert.equal(wakeView(null, null, ctx).canWake, false, "openOf not read: no button");
  const asked = wakeView(null, true, ctx);
  assert.equal(asked.canWake, false);
  assert.match(asked.launchWait!, /already asked/);
  // fresh, or no dormancy: awake as before, nothing offered
  assert.deepEqual([wakeView(null, false, { dormancy: true, fresh: true }).canWake, wakeView(null, false, { dormancy: true, fresh: true }).word], [false, "Awake"]);
  assert.equal(wakeView(null, false, { dormancy: false, fresh: false }).canWake, false);
  // the Docker build takes VITE_COLLECTIONS_URL (declared, and passed to the build's environment)
  const docker = readFileSync(new URL("../Dockerfile", import.meta.url), "utf8");
  assert.match(docker, /^ARG VITE_COLLECTIONS_URL=$/m);
  assert.match(docker, /VITE_COLLECTIONS_URL=\$VITE_COLLECTIONS_URL/);
});

test("DI-I-1: one package manager: npm's lockfile, no stray pnpm files", () => {
  assert.ok(existsSync(new URL("../package-lock.json", import.meta.url)));
  assert.ok(!existsSync(new URL("../pnpm-lock.yaml", import.meta.url)));
  assert.ok(!existsSync(new URL("../pnpm-workspace.yaml", import.meta.url)));
  assert.match(readFileSync(new URL("../Dockerfile", import.meta.url), "utf8"), /npm ci/);
});

// ---- independent review 2 (docs/audit/2026-09-27-nft/dormant-review-independent-2.md) ---------------------------------------------------

test("D2-I-1: with no lives served, an ACTIVE collection's brief staleness offers no wake; a module stale past a post cadence does", () => {
  const now = 1_800_000_000;
  // the reviewer's check: wakeView(null, false, { dormancy: true, fresh: false }) offered the wake with "it is probably dormant"
  const unknown = wakeView(null, false, { dormancy: true, fresh: false });
  assert.equal(unknown.canWake, false, "not offered before postedAt is read");
  // an ACTIVE collection whose post is late: posted 70 minutes ago, not fresh; the same collection with its life served says Awake
  const late = wakeView(null, false, { dormancy: true, fresh: false, postedAt: now - 70 * 60, nowS: now });
  assert.equal(late.canWake, false);
  assert.match(late.launchWait!, /probably awake and its next floor post is late/);
  assert.ok(!/probably dormant/.test(late.launchWait!));
  const active = { collection: "0x" + "10".repeat(20), wrapper: "0x" + "11".repeat(20), module: "0x" + "12".repeat(20), state: "active", since: "0",
    eta_secs: [], queue_position: [], woken_by: [], cooldown_until: [], status: "active", note: "" };
  assert.equal(wakeView(active, false, { dormancy: true, fresh: false, postedAt: now - 70 * 60, nowS: now }).canWake, false);
  // stale for longer than a post cadence: dormant, the wake offered, and the page says plainly what the bond may pay for
  const asleep = wakeView(null, false, { dormancy: true, fresh: false, postedAt: now - NO_LIVES_OFFER_AFTER_S, nowS: now });
  assert.equal(asleep.canWake, true);
  assert.match(asleep.launchWait!, /probably dormant/);
  assert.ok(asleep.detail.includes(NO_LIVES_NOTE) && /the bond pays for it/.test(NO_LIVES_NOTE));
  // never posted at all (never woken): dormant
  assert.equal(wakeView(null, false, { dormancy: true, fresh: false, postedAt: 0, nowS: now }).canWake, true);
  // the page reads postedAt() from the module, and passes it
  assert.ok(FLOOR_MODULE_FRESH.some((f) => f.name === "postedAt" && f.outputs[0].type === "uint64"));
  const launch = readFileSync(new URL("../src/pages/Launch.tsx", import.meta.url), "utf8");
  assert.match(launch, /functionName: "postedAt"/);
  assert.match(launch, /postedAt: postedAt\.data !== undefined \? Number\(postedAt\.data\) : undefined/);
});

test("D2-L-3: the page words a dropped request's refund as the desk decides it: any post inside the window serves it", () => {
  const now = 1_800_000_000, W = 7 * 86_400;
  const dormant = { collection: "0x" + "10".repeat(20), wrapper: "0x" + "11".repeat(20), module: "0x" + "12".repeat(20), state: "dormant",
    since: "0", eta_secs: [], queue_position: [], woken_by: [], cooldown_until: [], status: "dormant: a launch wakes it", note: "" };
  const v = wakeView(dormant, { id: 0n, at: now - W + 17 * 3_600 }, { dormancy: true, fresh: false, windowSecs: W, nowS: now });
  assert.ok(v.launchWait!.includes(SERVED_RULE));
  assert.ok(!/unserved/.test(v.launchWait!), "no promise of a refund the desk does not make");
  assert.match(RECLAIM_NOTE, /Any post inside the window serves the request, whatever woke the collection/);
});
