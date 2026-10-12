// docs/v7/dormant-collections.md: the runner's side of dormant, wake-on-demand collections. A DORMANT (or queued) collection gets no
// evidence (the canister refuses it unread) and never counts as stale for /healthz; a WAKING one gets it every pass (its backfill runs a
// pass per hand-in); an ACTIVE one near a buy every pass too (the fill needs a fresh floor and sale read); the runner ticks on its idle
// cadence while every collection sleeps and nothing is open; GET /collections serves the lives for the site. The canister's dormant
// state decodes with the runner's IDL. No network.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { ethIdle, ethNftOnce, evidenceCadence } from "../src/ethnft.js";
import type { CollectionLifeV } from "../src/ic.js";
import { logger } from "../src/log.js";
import { parseRunnerConfig, sleepAfter } from "../src/runner.js";

const silent = logger(() => {});
const US = "0x76020b7093bd35d2520d47ded764efc554237efe";
const MIL = "0x5af0d9827e0c53e4799bb226655a1de152a425a5", DOODLES = "0x8a90cab2b38dba80c64b7734e58ee1db38b8992e", AZUKI = "0xed5af388653567af2f388e6224dc7c4b3241c544";
const col = (c: string) => ({ collection: c, wrapper: "0xaa", module: "0xbb", max_purchase_wei: 10n ** 18n, max_premium_bps: 500 });
const life = (c: string, state: string): CollectionLifeV => ({ collection: c, wrapper: "0xaa", module: "0xbb", state, since: 1n, eta_secs: [],
  queue_position: [], woken_by: [], cooldown_until: [], status: state, note: "" });
const NOW = 1_800_000_000;
const ID = "ryjl3-tyaaa-aaaaa-aaaba-cai";

test("the canister's dormant state (every life, a pending Fill) decodes with the runner's IDL", async () => {
  const { IDL } = await import("@icp-sdk/core/candid");
  const { buyerIdl } = await import("../src/ic.js");
  const svc = buyerIdl({ IDL }) as unknown as { _fields: [string, { retTypes: Parameters<typeof IDL.decode>[0] }][] };
  const f = svc._fields.find(([n]) => n === "eth_nft_state")![1];
  const hex = readFileSync(new URL("../../autobuyer/canister-rs/tests/fixtures/eth-state-dormant.hex", import.meta.url), "utf8").trim();
  const [st] = IDL.decode(f.retTypes, Uint8Array.from(Buffer.from(hex, "hex"))) as [{ pending: [] | [{ nonce: bigint }] }];
  assert.equal(st.pending[0]?.nonce, 3n);
});

test("the evidence cadence: dormant none, waking and near a buy every pass, the rest every 20 min; without lives as before", () => {
  const lives = [life(MIL, "dormant"), life(DOODLES, "waking"), life(AZUKI, "active")];
  assert.equal(evidenceCadence(lives, MIL, false), "skip");
  assert.equal(evidenceCadence([life(MIL, "queued")], MIL, true), "skip", "queued: still asleep, even with a stale buy record");
  assert.equal(evidenceCadence(lives, DOODLES, false), "every-pass");
  assert.equal(evidenceCadence(lives, AZUKI, false), "normal");
  assert.equal(evidenceCadence(lives, AZUKI, true), "every-pass", "an open buy: the fill's fresh read");
  assert.equal(evidenceCadence(null, MIL, false), "normal", "a canister before the dormancy");
});

test("a pass hands in evidence only for the awake collections, and a dormant one is not served (never stale for /healthz)", async () => {
  const handed: string[] = [];
  const buyer = {
    eth_nft_config: async () => [{ chain_id: 1n, relay_max_fee_bps: 300, deposits: false, collections: [col(MIL), col(DOODLES), col(AZUKI)] }] as never,
    eth_nft_collections: async () => [life(MIL, "dormant"), life(DOODLES, "waking"), life(AZUKI, "active")],
    eth_nft_state: async () => ({ buys: [], deliveries: [], readings: [], eth_out: 0n, deposits: [] }),
    evm_address: async () => ({ Ok: US }),
    eth_relay_quote: async () => ({ Err: "x" }), eth_listing: async () => ({ Err: "x" }), eth_deposit: async () => ({ Err: "x" }),
    eth_floor_evidence: async (c: string) => { handed.push(c); return { Ok: "held" }; },
  };
  const fetchImpl = (async () => new Response(JSON.stringify({ listings: [] }))) as unknown as typeof fetch;
  const served: string[] = [], lives: CollectionLifeV[] = [], lastEvidence = new Map<string, number>();
  const d = { buyer, fetch: fetchImpl, log: silent, slugs: { [MIL]: "milady", [DOODLES]: "doodles-official", [AZUKI]: "azuki" }, openseaKey: () => "k",
    ethRpc: "https://eth.example", nowS: () => NOW, served, lives, lastEvidence };
  await ethNftOnce(d);
  assert.deepEqual(handed.sort(), [AZUKI, DOODLES].sort(), "Milady sleeps: nothing is read for it on OpenSea or handed in");
  assert.deepEqual(served.sort(), [AZUKI, DOODLES].sort());
  assert.deepEqual(lives.map((l) => l.state), ["dormant", "waking", "active"], "kept for GET /collections");
  // ten minutes on: the waking one again (its backfill), the active one not yet (20 min)
  handed.length = 0;
  await ethNftOnce({ ...d, nowS: () => NOW + 600 });
  assert.deepEqual(handed, [DOODLES]);
  // a canister without eth_nft_collections (built before the dormancy): every collection as before
  handed.length = 0;
  const { eth_nft_collections: _gone, ...old } = buyer;
  await ethNftOnce({ ...d, buyer: old, nowS: () => NOW + 7_200, lastEvidence: new Map() });
  assert.deepEqual(handed.sort(), [AZUKI, DOODLES, MIL].sort());
});

test("the idle cadence: every collection asleep and nothing open; a 4663 transaction out still wins", () => {
  const asleep = [life(MIL, "dormant"), life(DOODLES, "queued")];
  const quiet = { buys: [], deliveries: [], pending: [] as [] };
  assert.equal(ethIdle(asleep, quiet), true);
  assert.equal(ethIdle([...asleep, life(AZUKI, "waking")], quiet), false, "a wake in progress");
  assert.equal(ethIdle(asleep, { ...quiet, buys: [{ phase: { Arrived: {} } }] as never }), false, "an open buy");
  assert.equal(ethIdle(asleep, { ...quiet, buys: [{ phase: { Settled: {} } }] as never }), true, "a finished one is not open");
  assert.equal(ethIdle(asleep, { ...quiet, deliveries: [{ phase: { Waiting: null } }] as never }), false, "a delivery");
  assert.equal(ethIdle(asleep, { ...quiet, pending: [{ nonce: 1n }] as never }), false, "a 4663 transaction");
  assert.equal(ethIdle(null, quiet), false, "no lives: not idle");
  const cfg = parseRunnerConfig({ ROOK_HOME: "robinhood", CANISTER_ID: ID });
  assert.equal(cfg.idleEveryS, 900);
  assert.equal(sleepAfter(cfg, false, 0, true), 900);
  assert.equal(sleepAfter(cfg, false, 0, false), 600);
  assert.equal(sleepAfter(cfg, true, 0, true), 120, "a 4663 transaction out: the pending period");
  assert.throws(() => parseRunnerConfig({ ROOK_HOME: "robinhood", CANISTER_ID: ID, RUNNER_IDLE_TICK_EVERY_S: "300" }), /RUNNER_IDLE_TICK_EVERY_S/);
  assert.throws(() => parseRunnerConfig({ ROOK_HOME: "robinhood", CANISTER_ID: ID, RUNNER_IDLE_TICK_EVERY_S: "7200" }), /RUNNER_IDLE_TICK_EVERY_S/);
  assert.equal(parseRunnerConfig({ ROOK_HOME: "robinhood", CANISTER_ID: ID, RUNNER_IDLE_TICK_EVERY_S: "1800" }).idleEveryS, 1800);
});

test("§11: a pass keeps the canister's capacity, and GET /capacity serves it (null before a canister that has it)", async () => {
  const cap = { at: 5n, balance: 40_000_000_000_000n, scaling: true, collections: 10, ceiling: 10, wakes_per_day: 12, effective_max_active: 1, awake: 1,
    pinned: 1, wake_headroom: 0, wakes_left_today: 11, next_wake_at: 52_600_000_000_000n, status: "1 of 10 collections can be awake right now (1 awake)" };
  const buyer = {
    eth_nft_config: async () => [{ chain_id: 1n, relay_max_fee_bps: 300, deposits: false, collections: [col(MIL)] }] as never,
    eth_nft_collections: async () => [life(MIL, "dormant")], eth_nft_capacity: async () => [cap] as [typeof cap],
    eth_nft_state: async () => ({ buys: [], deliveries: [], readings: [], eth_out: 0n, deposits: [] }),
    evm_address: async () => ({ Ok: US }), eth_relay_quote: async () => ({ Err: "x" }), eth_listing: async () => ({ Err: "x" }),
    eth_deposit: async () => ({ Err: "x" }), eth_floor_evidence: async () => ({ Ok: "held" }),
  };
  const capacity = { current: null as typeof cap | null };
  await ethNftOnce({ buyer, fetch: (async () => new Response("{}")) as unknown as typeof fetch, log: silent, slugs: { [MIL]: "milady" }, openseaKey: () => "k",
    ethRpc: "https://eth.example", nowS: () => NOW, capacity });
  assert.equal(capacity.current?.effective_max_active, 1);
  const { Runner, serveRunner } = await import("../src/runner.js");
  const r = new Runner({ tick: async () => ({ Ok: [] }), paused: async () => false }, silent, "p", ID);
  const srv = serveRunner(r, 600, 0, [], capacity);
  await new Promise((ok) => srv.once("listening", ok));
  const port = (srv.address() as { port: number }).port;
  try {
    const res = await fetch(`http://127.0.0.1:${port}/capacity`);
    assert.equal(res.status, 200);
    const body = await res.json() as { effective_max_active: number; collections: number; balance: string; status: string };
    assert.deepEqual([body.effective_max_active, body.collections, body.balance], [1, 10, "40000000000000"]);
    capacity.current = null;
    assert.equal(await (await fetch(`http://127.0.0.1:${port}/capacity`)).json(), null);
  } finally { srv.close(); }
});

test("GET /collections serves the lives, public and cached a minute", async () => {
  const { Runner, serveRunner } = await import("../src/runner.js");
  const r = new Runner({ tick: async () => ({ Ok: [] }), paused: async () => false }, silent, "p", ID);
  const lives = [life(MIL, "waking")];
  const srv = serveRunner(r, 600, 0, lives);
  await new Promise((ok) => srv.once("listening", ok));
  const port = (srv.address() as { port: number }).port;
  try {
    const res = await fetch(`http://127.0.0.1:${port}/collections`);
    assert.equal(res.status, 200);
    assert.match(res.headers.get("cache-control") ?? "", /max-age=60/);
    const body = await res.json() as { collection: string; state: string; since: string }[];
    assert.deepEqual([body[0].collection, body[0].state, body[0].since], [MIL, "waking", "1"]);
  } finally { srv.close(); }
});
