import assert from "node:assert/strict";
import { test } from "node:test";
import { logger } from "../src/log.js";
import { parseRunnerConfig, RunnerConfigError } from "../src/runner.js";
import { GAP_MS, loadSolNftCollections, MAX_CANDIDATES, MAX_LISTINGS_BYTES, solNftCollections, solNftFloorOnce, trimListings, type SolNftCollection } from "../src/solnftfloor.js";

// docs/v9/solana-nfts-canister.md §5: the runner's Magic Eden feed for the open Solana collections, against a fake Magic Eden and a fake
// canister. The canister's endpoint is `sol_nft_evidence(coll, listings, stats)` (src/solnftfloor.ts: the TODO names its branch).
const ID = "ryjl3-tyaaa-aaaaa-aaaba-cai";
const COLS: SolNftCollection[] = loadSolNftCollections(new URL("../../deployments/launch/solana-collections.json", import.meta.url).pathname);
const listing = (price: number, i: number) => ({ pdaAddress: `pda${i}`, tokenMint: `mint${i}`, price, listingSource: "M2", seller: `s${i}` });

function fakeMe(o: { listings?: (sym: string) => unknown; stats?: (sym: string) => unknown; status?: number } = {}) {
  const calls: string[] = [];
  const f = (async (url: string, init?: RequestInit) => {
    calls.push(url);
    assert.equal(init?.redirect, "manual", "no redirect followed");
    const u = new URL(url), sym = u.pathname.split("/")[3];
    if (o.status) return new Response("busy", { status: o.status });
    if (u.pathname.endsWith("/listings")) {
      assert.equal(u.searchParams.get("listingAggMode"), "true");
      return new Response(JSON.stringify(o.listings ? o.listings(sym) : [listing(9, 1), listing(8.352, 2), listing(8.4, 3)]));
    }
    if (u.pathname.endsWith("/stats")) return new Response(JSON.stringify(o.stats ? o.stats(sym) : { symbol: sym, floorPrice: 8_352_000_000, listedCount: 200 }));
    return new Response("not found", { status: 404 });
  }) as unknown as typeof fetch;
  return { f, calls };
}

function fakeCanister(open: Record<string, number>, answer: (coll: number) => { Ok: string } | { Err: string } = () => ({ Ok: "taken" })) {
  const handed: { coll: number; listings: string; stats: string }[] = [];
  return {
    handed,
    buyer: {
      coins_collections: async () => [
        ...Object.entries(open).map(([symbol, index]) => ({ index, symbol, stage: "Open" })),
        { index: 9, symbol: "wCLAYNO", stage: "Pinned" }, { index: 10, symbol: "wFFF", stage: "Requested" },
      ],
      sol_nft_evidence: async (coll: number, listings: string, stats: string) => { handed.push({ coll, listings, stats }); return answer(coll); },
    },
  };
}

test("the launch file's Solana collections: their Magic Eden symbols, checked", () => {
  assert.equal(COLS.length, 12);
  assert.deepEqual(COLS.slice(0, 2).map((c) => [c.key, c.symbol, c.meSymbol]), [["mad_lads", "wMADLAD", "mad_lads"], ["smb_gen2", "wSMBGEN2", "solana_monkey_business"]]);
  assert.throws(() => solNftCollections({ collections: [{ key: "x", name: "X", symbol: "wX", magic_eden_symbol: "../admin" }] }), /not a Magic Eden symbol/);
});

test("only the OPEN Solana collections are read and handed in: listings (listingAggMode) and stats, one call at a time, each at its cadence", async () => {
  const me = fakeMe(), can = fakeCanister({ wMADLAD: 2, wSMBGEN2: 3 });
  const slept: number[] = [];
  let now = 1_700_000_000;
  const last = new Map<string, number>(), accepted = new Map<string, number>();
  const d = { buyer: can.buyer, fetch: me.f, log: logger(() => {}), collections: COLS, api: "https://me.example/", nowS: () => now, sleep: async (ms: number) => { slept.push(ms); }, last, accepted };
  assert.equal(await solNftFloorOnce(d), true);
  // Mad Lads and SMB Gen2 only (Claynosaurz is pinned, FFF requested: not open, nothing read)
  assert.deepEqual(can.handed.map((h) => h.coll), [2, 3]);
  assert.deepEqual(me.calls, ["https://me.example/v2/collections/mad_lads/listings?listingAggMode=true", "https://me.example/v2/collections/mad_lads/stats",
    "https://me.example/v2/collections/solana_monkey_business/listings?listingAggMode=true", "https://me.example/v2/collections/solana_monkey_business/stats"]);
  assert.deepEqual(slept, [GAP_MS, GAP_MS, GAP_MS], "a gap between every two Magic Eden calls");
  // the listings as the canister keeps them: cheapest first; the stats as Magic Eden answered
  assert.deepEqual(JSON.parse(can.handed[0].listings).map((x: { price: number }) => x.price), [8.352, 8.4, 9]);
  assert.equal(JSON.parse(can.handed[0].stats).floorPrice, 8_352_000_000);
  assert.deepEqual([...accepted.keys()], ["mad_lads", "smb_gen2"]);
  // inside the cadence: nothing read again; after it, again
  now += 60;
  assert.equal(await solNftFloorOnce(d), false);
  assert.equal(me.calls.length, 4);
  now += 1_800;
  assert.equal(await solNftFloorOnce(d), true);
  assert.equal(me.calls.length, 8);
});

test("bounded: the listings trimmed to the canister's 40 cheapest and 64 KiB; a refusal, a Magic Eden error or a canister without the endpoint is logged, never thrown", async () => {
  // 300 listings with long fields: the cheapest 40, then fewer until the text fits
  const many = Array.from({ length: 300 }, (_, i) => ({ ...listing(300 - i, i), extra: "x".repeat(3_000) }));
  const t = JSON.parse(trimListings(JSON.stringify(many)));
  assert.ok(t.length <= MAX_CANDIDATES && t.length > 0);
  assert.ok(Buffer.byteLength(JSON.stringify(t)) <= MAX_LISTINGS_BYTES);
  assert.equal(t[0].price, 1, "the cheapest first");
  assert.throws(() => trimListings(JSON.stringify({ results: [] })), /not an array/);
  const lines: string[] = [];
  const log = logger((l) => lines.push(l));
  const base = { log, collections: COLS.slice(0, 1), nowS: () => 1, sleep: async () => {} };
  // the canister refuses (not open there yet, a bad hand-in): logged, false
  const no = fakeCanister({ wMADLAD: 2 }, () => ({ Err: "refused: no listing of a known venue" }));
  assert.equal(await solNftFloorOnce({ ...base, buyer: no.buyer, fetch: fakeMe().f }), false);
  assert.ok(lines.some((l) => l.includes("sol-nft-floor-not-taken")));
  // Magic Eden answers 429: nothing handed in
  const busy = fakeCanister({ wMADLAD: 2 });
  assert.equal(await solNftFloorOnce({ ...base, buyer: busy.buyer, fetch: fakeMe({ status: 429 }).f }), false);
  assert.equal(busy.handed.length, 0);
  assert.ok(lines.some((l) => l.includes("sol-nft-floor-unread") && l.includes("HTTP 429")));
  // stats over 4 KiB: nothing handed in
  const big = fakeCanister({ wMADLAD: 2 });
  assert.equal(await solNftFloorOnce({ ...base, buyer: big.buyer, fetch: fakeMe({ stats: () => ({ floorPrice: 1, pad: "y".repeat(5_000) }) }).f }), false);
  assert.equal(big.handed.length, 0);
  // a canister built before the endpoint rejects the call: logged, the loop goes on
  const old = { ...fakeCanister({ wMADLAD: 2 }).buyer, sol_nft_evidence: async () => { throw new Error("Canister has no update method 'sol_nft_evidence'"); } };
  assert.equal(await solNftFloorOnce({ ...base, buyer: old, fetch: fakeMe().f }), false);
  assert.ok(lines.some((l) => l.includes("sol-nft-floor-call-failed")));
  // coins_collections unreadable: nothing read from Magic Eden
  const m = fakeMe();
  assert.equal(await solNftFloorOnce({ ...base, buyer: { ...old, coins_collections: async () => { throw new Error("down"); } }, fetch: m.f }), false);
  assert.equal(m.calls.length, 0);
});

test("RUNNER_SOL_NFT_FLOOR: on by default on the Solana home with the launch file's 12 collections; off on the EVM home; its settings checked", () => {
  const c = parseRunnerConfig({ CANISTER_ID: ID });
  assert.equal(c.solNftFloor, true);
  assert.equal(c.solCollections.length, 12);
  assert.equal(c.solNftEveryS, 1_800);
  assert.equal(c.meApi, "https://api-mainnet.magiceden.dev");
  assert.equal(parseRunnerConfig({ CANISTER_ID: ID, RUNNER_SOL_NFT_FLOOR: "off", RUNNER_SOL_COLLECTIONS_FILE: "/nonexistent" }).solNftFloor, false, "off: the file is not needed");
  assert.equal(parseRunnerConfig({ CANISTER_ID: ID, ROOK_HOME: "robinhood" }).solNftFloor, false);
  const p = (env: Record<string, string>) => { try { parseRunnerConfig({ CANISTER_ID: ID, ...env }); return ""; } catch (e) { return (e as RunnerConfigError).problems.join("\n"); } };
  assert.match(p({ RUNNER_SOL_NFT_FLOOR: "maybe" }), /RUNNER_SOL_NFT_FLOOR=maybe/);
  assert.match(p({ RUNNER_SOL_NFT_FLOOR_EVERY_S: "60" }), /RUNNER_SOL_NFT_FLOOR_EVERY_S=60/);
  assert.match(p({ MAGIC_EDEN_API: "http://me.example" }), /MAGIC_EDEN_API must be https/);
  assert.match(p({ RUNNER_SOL_COLLECTIONS_FILE: "/nonexistent.json" }), /RUNNER_SOL_COLLECTIONS_FILE .*missing/);
});
