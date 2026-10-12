// Audit round C, part C4 (docs/v9/audit-2026-10/c4-opening-runner.md): the runner's side of the integration. These were the PoCs of
// C4-06 and C4-08 (each passed by exhibiting its finding); they are inverted to assert the fix, and C4-09 and C4-10 are tested too.
// No network.
import assert from "node:assert/strict";
import { test } from "node:test";
import { memberTree } from "../src/frogmath.js";
import { bitcoinStepsOnce, frogProofOnce, ListingMembers, MEMBERS_RETRY_S, type InscEntry } from "../src/inscriptions.js";
import { logger } from "../src/log.js";
import { coingeckoReader, type FrogColl, type InscListing } from "../src/solfloor.js";
import { guarded } from "../src/solnftfloor.js";

const silent = logger(() => {});
const id = (n: number) => `${n.toString(16).padStart(64, "0")}i0`;

// C4-06 (Low): a buying entry unpaired once (its collection pinned after the runner started) must not keep /healthz red: the step's
// problems are recomputed each pass, and a problem another source put in the shared set is kept.
test("fixed C4-06: an entry's 'unpaired' problem clears once it pairs; other sources' problems stay", async () => {
  const members = [id(1), id(2), id(3)];
  const entry: InscEntry = { slug: "omb", owSlug: "omb", membersFile: "omb-members.json", members, unisat: null };
  let colls: FrogColl[] = [];
  const problems = new Set<string>(["someone else's problem"]);
  let stepped = 0;
  const once = bitcoinStepsOnce({ entries: [entry], colls: async () => colls, log: silent, problems, step: async () => { stepped++; return false; } });
  await once();
  assert.equal(problems.size, 2, "unpaired: reported");
  await once();
  assert.equal(problems.size, 2, "still unpaired: still reported, once");
  // the operator pins and wakes OMB: the canister now has the collection, live
  colls = [{ coll: 24, root: memberTree(members).root, live: true }];
  await once();
  assert.equal(stepped, 1, "paired and served");
  assert.deepEqual([...problems], ["someone else's problem"], "the step's own problem cleared; the other kept");
});

const A = [id(1), id(2), id(3)], B = [id(11), id(12), id(13)];
const listing = (i: number, slug: string, m: string[]): InscListing => ({ id: i, slug, owSlug: slug, name: slug, wrapping: "now", canister: true,
  membersRoot: memberTree(m).root, membersFile: `/files/${slug}.json`, coingeckoId: slug });

test("fixed C4-06: the proof step's members problem clears when the file is fixed (retried after MEMBERS_RETRY_S), not kept for the process", async () => {
  let now = 1_000_000;
  const files: Record<string, string[]> = { "/files/frogs.json": [id(50)] };
  let loads = 0;
  const members = new ListingMembers((p) => { loads++; return files[p]; }, { nowS: () => now });
  const handed: string[] = [];
  const buyer = { frog_collections: async () => [{ coll: 4, root: memberTree(A).root, live: true }], holder_deposits: async () => [{ phase: { AwaitingProof: { inscription: A[0] } } }],
    frog_proof: async (_c: number, i: string) => { handed.push(i); return { Ok: "kept" }; } };
  const problems = new Set<string>(["other"]), owned = new Set<string>();
  const d = { buyer, log: silent, listings: [listing(11, "frogs", A)], members, problems, owned, nowS: () => now };
  assert.equal(await frogProofOnce(d), false);
  assert.ok([...problems].some((p) => /frogs's members file .* has root/.test(p)), "off its root: reported");
  files["/files/frogs.json"] = A; // the operator fixes the file on disk
  now += 60;
  assert.equal(await frogProofOnce(d), false, "not re-read within MEMBERS_RETRY_S");
  assert.equal(loads, 1);
  now += MEMBERS_RETRY_S;
  assert.equal(await frogProofOnce(d), true, "re-read: the proof is handed in");
  assert.deepEqual(handed, [A[0]]);
  assert.deepEqual([...problems], ["other"], "the members problem cleared; the other source's kept");
});

// C4-08 (Informational): the band gate's reference must name BTC and be the asked collection's; otherwise no reading (fail closed).
test("fixed C4-08: a CoinGecko answer with no currency, another currency or another collection's id is no reference", async () => {
  const f = (body: unknown) => (async () => new Response(JSON.stringify(body), { status: 200 })) as typeof fetch;
  assert.equal(await coingeckoReader(f({ id: "some-other-collection", floor_price: { native_currency: 0.05 } }))("omb"), null);
  assert.equal(await coingeckoReader(f({ id: "omb", floor_price: { native_currency: 0.05 } }))("omb"), null, "no currency");
  assert.equal(await coingeckoReader(f({ id: "some-other-collection", native_currency_symbol: "btc", floor_price: { native_currency: 0.05 } }))("omb"), null, "another id");
  assert.equal(await coingeckoReader(f({ id: "omb", native_currency_symbol: "ETH", floor_price: { native_currency: 0.05 } }))("omb"), null, "another currency");
  assert.equal(await coingeckoReader(f({ id: "omb", native_currency_symbol: "BTC", floor_price: { native_currency: 0.05 } }))("omb"), 5_000_000n, "BTC, any case");
});

// C4-09 (Informational): Magic Eden's answer is read with a byte cap, never whole before the check.
test("fixed C4-09: a Magic Eden body over the cap is refused without reading it all; a Content-Length over the cap unread", async () => {
  const chunk = new Uint8Array(64 * 1024).fill(0x20);
  let pulled = 0, cancelled = false;
  const endless = new ReadableStream<Uint8Array>({ pull(c) { pulled++; c.enqueue(chunk); }, cancel() { cancelled = true; } }, { highWaterMark: 0 });
  const f = (async () => new Response(endless, { status: 200 })) as typeof fetch;
  await assert.rejects(guarded({ fetch: f }, "https://me.example/v2/collections/x/listings", 1_000_000), /over 1000000 bytes/);
  assert.ok(pulled <= 17, `stopped at the cap (${pulled} chunks pulled)`);
  assert.ok(cancelled, "the stream is cancelled");
  let read = false;
  const declared = new ReadableStream<Uint8Array>({ pull(c) { read = true; c.enqueue(chunk); c.close(); } }, { highWaterMark: 0 });
  const g = (async () => new Response(declared, { status: 200, headers: { "content-length": "5000000" } })) as typeof fetch;
  await assert.rejects(guarded({ fetch: g }, "https://me.example/v2/collections/x/stats", 1_000_000), /over 1000000 bytes/);
  assert.equal(read, false, "refused on its Content-Length, unread");
  const ok = (async () => new Response(JSON.stringify({ floorPrice: 1 }), { status: 200 })) as typeof fetch;
  assert.equal(await guarded({ fetch: ok }, "https://me.example/v2/collections/x/stats"), '{"floorPrice":1}');
});

// C4-10 (Informational): an inscription two live collections' members hold gets no proof (the runner does not pick); it is logged and
// reported, and the problem clears (C4-06) once it no longer holds; the overlap is also logged when the files load.
test("fixed C4-10: an inscription in two live collections' members gets no proof, is reported naming both, and clears when resolved", async () => {
  const shared = A[0], B2 = [...B, shared];
  const files: Record<string, string[]> = { "/files/frogs.json": A, "/files/omb.json": B2 };
  const lines: string[] = [];
  const log = logger((l) => lines.push(l));
  let colls: FrogColl[] = [{ coll: 4, root: memberTree(A).root, live: true }, { coll: 24, root: memberTree(B2).root, live: true }];
  const handed: [number, string][] = [];
  const buyer = { frog_collections: async () => colls,
    holder_deposits: async () => [{ phase: { AwaitingProof: { inscription: shared } } }, { phase: { AwaitingProof: { inscription: B[1] } } }],
    frog_proof: async (c: number, i: string) => { handed.push([c, i]); return { Ok: "kept" }; } };
  const problems = new Set<string>(), owned = new Set<string>();
  const d = { buyer, log, listings: [listing(11, "frogs", A), listing(12, "omb", B2)], members: new ListingMembers((p) => files[p], { log }), problems, owned };
  assert.equal(await frogProofOnce(d), true);
  assert.deepEqual(handed, [[24, B[1]]], "only the unambiguous one");
  assert.equal(problems.size, 1);
  assert.match([...problems][0], new RegExp(`${shared} is in the members of frogs \\(coll 4\\) and omb \\(coll 24\\)`));
  assert.ok(lines.some((l) => /frog-proof-ambiguous/.test(l)), "logged");
  assert.ok(lines.some((l) => /frog-proof-members-overlap/.test(l) && /frogs/.test(l) && /omb/.test(l)), "the overlap logged at load time");
  // OMB goes dormant: only Bitcoin Frogs holds it among the live ones; the proof goes there and the problem clears
  colls = [colls[0], { ...colls[1], live: false }];
  assert.equal(await frogProofOnce(d), true);
  assert.deepEqual(handed.at(-1), [4, shared]);
  assert.equal(problems.size, 0, "cleared");
});
