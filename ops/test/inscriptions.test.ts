// Ordinals multi-collection (docs/v9/ordinals-collections.md §4) and Robinhood Chain NFTs (robinhood-everything.md §3), the runner's side:
// the per-collection members trees, the pairing with the canister's collections by root, the proof step, the buying steps' turn, the
// 4663 side read beside the config's own, and the IDL against the merged did. No network.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { leafOf, memberTree, proofFor, verifyProof } from "../src/frogmath.js";
import { bitcoinStepsOnce, entryColls, frogProofOnce, ListingMembers, loadMembers, PROOF_AGAIN_S, type InscEntry } from "../src/inscriptions.js";
import { logger } from "../src/log.js";
import { sideStatesOf } from "../src/runner.js";
import { loadInscriptionListings, type FrogColl, type InscListing } from "../src/solfloor.js";

const silent = logger(() => {});
const PILOT = new URL("../../deployments/launch/nft-pilot.json", import.meta.url).pathname;
const id = (n: number) => `${n.toString(16).padStart(64, "0")}i0`;

test("the kept members tree: the same root and proofs as proofFor, for odd and even sizes; Bitcoin Frogs' file gives the launch file's root", () => {
  for (const n of [1, 2, 3, 7, 8, 13]) {
    const m = Array.from({ length: n }, (_, i) => id(i + 1)), t = memberTree(m);
    assert.equal(memberTree(m), t, "built once per list");
    for (const x of m) {
      const p = proofFor(m, x);
      assert.deepEqual([t.root, t.proof(x)], [p.root, p.proof]);
      assert.ok(verifyProof(t.proof(x), t.root, leafOf(x)));
    }
    assert.equal(t.has(id(999)), false);
    assert.throws(() => t.proof(id(999)), /not in the member list/);
  }
  const frogs = loadInscriptionListings(PILOT)[0];
  assert.equal(memberTree(loadMembers(frogs.membersFile)).root.toLowerCase(), frogs.membersRoot.toLowerCase());
});

// a small world: two collections of three members each, as the launch file and the canister would name them
const A = [id(1), id(2), id(3)], B = [id(11), id(12), id(13)];
const listing = (i: number, slug: string, m: string[]): InscListing => ({ id: i, slug, owSlug: slug, name: slug, wrapping: "now", canister: i === 11,
  membersRoot: memberTree(m).root, membersFile: `/files/${slug}.json`, coingeckoId: slug });
const LISTINGS = [listing(11, "frogs", A), listing(12, "omb", B)];
const files: Record<string, string[]> = { "/files/frogs.json": A, "/files/omb.json": B };

test("the proof step: a deposit awaiting its proof (or parked) gets it from its LIVE collection's file, under that coll; once per 6 h; a file off its root is refused", async () => {
  const colls: FrogColl[] = [{ coll: 4, root: memberTree(A).root.slice(2), live: true }, { coll: 24, root: memberTree(B).root, live: false }];
  const deposits = [{ phase: { AwaitingProof: { inscription: A[1] } } }, { phase: { Parked: { inscription: B[0] } } }, { phase: { Pending: { why: "depth" } } },
    { phase: { AwaitingProof: { inscription: id(77) } } }];
  const handed: [number, string, string[]][] = [];
  const buyer = { frog_collections: async () => colls, holder_deposits: async () => deposits,
    frog_proof: async (c: number, i: string, p: string[]) => { handed.push([c, i, p]); return { Ok: "proof kept" }; } };
  let now = 1_000_000;
  const members = new ListingMembers((p) => files[p]), seen = new Map<string, number>(), problems = new Set<string>();
  const d = { buyer, log: silent, listings: LISTINGS, members, handed: seen, problems, nowS: () => now };
  assert.equal(await frogProofOnce(d), true);
  assert.deepEqual(handed.map((h) => [h[0], h[1]]), [[4, A[1]]], "OMB is dormant: its parked deposit waits; a non-member gets nothing");
  assert.ok(verifyProof(handed[0][2].map((h) => `0x${h}` as const), memberTree(A).root, leafOf(A[1])), "the proof verifies, without 0x");
  assert.equal(await frogProofOnce(d), false, "not again within 6 h");
  colls[1].live = true; now += PROOF_AGAIN_S + 1;
  assert.equal(await frogProofOnce(d), true);
  assert.deepEqual(handed.slice(1).map((h) => [h[0], h[1]]), [[4, A[1]], [24, B[0]]], "after 6 h the still-awaited one again; OMB woke: its own");
  // a members file whose tree is not the canister's root is never used, and says so
  const bad = new ListingMembers((p) => (p.endsWith("omb.json") ? [id(50)] : files[p]));
  handed.length = 0;
  assert.equal(await frogProofOnce({ ...d, members: bad, handed: new Map() }), true);
  assert.deepEqual(handed.map((h) => h[0]), [4]);
  assert.ok([...problems].some((p) => /omb's members file \/files\/omb.json has root .*not the canister's/.test(p)));
  // nothing waiting: frog_collections is not even read
  let read = 0;
  assert.equal(await frogProofOnce({ ...d, buyer: { ...buyer, holder_deposits: async () => [], frog_collections: async () => { read++; return colls; } } }), false);
  assert.equal(read, 0);
});

test("the buying steps: each RUNNER_INSCRIPTIONS entry is the canister collection with its members' root; live ones in turn, one hand-in per cycle; unpaired ones said", async () => {
  const entry = (slug: string, m: string[]): InscEntry => ({ slug, owSlug: slug, membersFile: `/files/${slug}.json`, members: m, unisat: null });
  const C = [id(21), id(22)];
  const entries = [entry("frogs", A), entry("omb", B), entry("nodemonkes", C), entry("stray", [id(99)])];
  const colls: FrogColl[] = [{ coll: 4, root: memberTree(A).root, live: true }, { coll: 24, root: memberTree(B).root.toUpperCase().replace("0X", "0x"), live: true },
    { coll: 25, root: memberTree(C).root, live: false }];
  assert.deepEqual([...entryColls(entries, colls)].map(([s, c]) => [s, c.coll]), [["frogs", 4], ["omb", 24], ["nodemonkes", 25]]);
  const asked: [string, number][] = [];
  let acts = new Set<string>(["frogs", "omb"]);
  const problems = new Set<string>();
  const once = bitcoinStepsOnce({ entries, colls: async () => colls, log: silent, problems, step: async (e, c) => { asked.push([e.slug, c]); return acts.has(e.slug); } });
  assert.equal(await once(), true);
  assert.deepEqual(asked, [["frogs", 4]], "one hand-in per cycle: the first collection that acted ends it");
  asked.length = 0;
  assert.equal(await once(), true);
  assert.deepEqual(asked, [["omb", 24]], "the next cycle starts after the one that acted");
  asked.length = 0; acts = new Set();
  assert.equal(await once(), false);
  assert.deepEqual(asked, [["frogs", 4], ["omb", 24]], "nothing acted: every live one asked; the dormant NodeMonkes never");
  assert.ok([...problems].some((p) => p.startsWith("stray: no canister collection has the root")));
});

test("Robinhood Chain: the 4663 side is read beside the config's own (eth_nft_state_of for each other chain a collection names, once)", async () => {
  const asked: bigint[] = [];
  const st = { buys: [], deliveries: [], readings: [], eth_out: 0n, deposits: [] };
  const col = (chain: [] | [bigint]) => ({ collection: "0x01", wrapper: "books", module: "books", max_purchase_wei: 1n, max_premium_bps: 500, chain_id: chain, token_bound: [] as [] });
  const b = (cols: ReturnType<typeof col>[]) => ({ eth_nft_config: async () => [{ chain_id: 1n, relay_max_fee_bps: 300, deposits: false, collections: cols }] as never,
    eth_nft_state_of: async (c: bigint) => { asked.push(c); return c === 4663n ? [st] as never : [] as never; } });
  assert.equal((await sideStatesOf(b([col([]), col([1n]), col([4663n]), col([4663n])]))()).length, 1);
  assert.deepEqual(asked, [4663n]);
  asked.length = 0;
  assert.deepEqual(await sideStatesOf(b([col([]), col([1n])]))(), []);
  assert.deepEqual(asked, [], "every collection on the config's chain: no side read");
});

test("the runner's IDL matches the merged did for every frog method that gained `coll`, frog_collections, and the 4663 endpoints", async () => {
  const { IDL } = await import("@icp-sdk/core/candid");
  const { buyerIdl } = await import("../src/ic.js");
  const svc = buyerIdl({ IDL }) as unknown as { _fields: [string, { argTypes: { display(): string }[]; retTypes: { display(): string }[]; annotations: string[] }][] };
  const did = readFileSync(new URL("../../autobuyer/canister-rs/etch_autobuyer.did", import.meta.url), "utf8").replace(/\/\/.*$/gm, "");
  const sig = (name: string) => {
    const m = new RegExp(`\\n\\s*${name}\\s*:\\s*\\(([^)]*)\\)\\s*->\\s*\\(([^)]*)\\)\\s*(query)?`).exec(did);
    assert.ok(m, `${name} is not in the did`);
    return { args: m![1].split(",").map((x) => x.trim()).filter(Boolean), query: !!m![3] };
  };
  const prim = new Set(["nat16", "nat32", "nat64", "nat", "text", "bool", "opt nat16", "vec text", "vec nat32"]);
  for (const name of ["frog_collections", "frog_floor_reading", "frog_proof", "frog_offer", "frog_offer_protected", "frog_buys", "frog_conversions",
    "relay_give_back", "probe_frog", "frog_wake", "frog_sleep", "unmarked_custody", "holder_deposits", "eth_nft_state_of", "eth_line_clear_on", "sol_nft_evidence"]) {
    const f = svc._fields.find(([n]) => n === name)?.[1];
    assert.ok(f, `${name} is not in ops/src/ic.ts`);
    const want = sig(name), got = f!.argTypes.map((t) => t.display().replace(/\s+/g, " "));
    assert.equal(got.length, want.args.length, `${name}: ${got.join(", ")} against the did's ${want.args.join(", ")}`);
    want.args.forEach((a, i) => { if (prim.has(a)) assert.equal(got[i], a, `${name} argument ${i}`); });
    assert.equal(f!.annotations.includes("query"), want.query, `${name}: query or update as the did says`);
  }
  // EthCollection's two new fields and FrogCollection's shape decode with the runner's types
  const ec = svc._fields.find(([n]) => n === "eth_nft_config")![1].retTypes[0].display();
  assert.match(ec, /chain_id:opt nat64/);
  assert.match(ec, /token_bound:opt bool/);
  assert.match(svc._fields.find(([n]) => n === "frog_collections")![1].retTypes[0].display(), /coll:nat16.*live:bool|live:bool.*coll:nat16/);
});
