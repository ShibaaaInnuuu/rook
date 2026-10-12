import assert from "node:assert/strict";
import { test } from "node:test";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { decodeFunctionData, encodeAbiParameters, encodeEventTopics, encodeFunctionResult, keccak256, parseAbi, toHex, type Hex } from "viem";
import { CATALOGUE_PATH, OUR_CHAINS, STAGING_FIXTURE_MEMBERS, WEIGHT_BASES, btcVaultFixture, fixtureMemberId, knownIdsOf,
  launchOrder, leafOf, listingCalls, membersOf, merkleProof, merkleRoot, plannedListings, pricesFor, processProof,
  weightBasisOf, weightOf, type Catalogue } from "../catalogue/listings.js";
import { createListings, MANIFEST_NOTE, readManifest, writePrices } from "../bootstrap/listings.js";

const CATALOGUE = JSON.parse(readFileSync(new URL(`../../${CATALOGUE_PATH}`, import.meta.url), "utf8")) as Catalogue;
const STAGING = 9466301;
const stagingPlan = () => plannedListings(CATALOGUE, { chainId: STAGING, fixtureMembers: STAGING_FIXTURE_MEMBERS });

/**
 * Everything the committed catalogue decides is DERIVED here rather than written down. The catalogue is the fifty
 * most-traded collections and runes of the week it was fetched (docs/phase9.md §11), so its length, its busiest entry
 * and its floors all move with every fetch. A test that pinned them would fail on a fresh catalogue instead of on a
 * defect, which is a test that stops being read.
 */
const LISTABLE = CATALOGUE.collections.filter((c) => knownIdsOf(c).length > 0);
const UNLISTABLE = CATALOGUE.collections.filter((c) => knownIdsOf(c).length === 0);
const TOTAL = LISTABLE.length + CATALOGUE.runes.length;
const ONE = LISTABLE[0];                       // any collection that carries an inscription id will do
const FIRST_RUNE = CATALOGUE.runes[0];

test("the catalogue becomes listings: every collection that has an inscription id, and every rune, with the registry's own fields", () => {
  const plan = stagingPlan();
  const collections = plan.listings.filter((l) => l.kind === "inscriptions"), runes = plan.listings.filter((l) => l.kind === "rune");
  assert.equal(collections.length + runes.length, TOTAL, `${LISTABLE.length} collections and ${CATALOGUE.runes.length} runes`);
  assert.equal(runes.length, CATALOGUE.runes.length, "every rune in the catalogue is listed");
  assert.equal(collections.length, LISTABLE.length, "every collection that carries an inscription id, and no other");

  // a collection: its own name and slug, no divisibility, a root
  const one = plan.listings.find((l) => l.ref === ONE.slug)!;
  assert.equal(one.name, ONE.name);
  assert.equal(one.divisibility, 0);
  assert.equal(one.floorSats, ONE.floorSats, "the cap is posted from the catalogue's own dated floor");
  assert.notEqual(one.membershipRoot, null);

  // a rune: the REAL rune id as the registry's ref, and the real divisibility, which is what the vault divides by
  const rune = plan.listings.find((l) => l.name === FIRST_RUNE.name)!;
  assert.equal(rune.ref, FIRST_RUNE.runeId, "the ref of a rune listing is its rune id");
  assert.match(rune.ref, /^\d+:\d+$/);
  assert.equal(rune.divisibility, FIRST_RUNE.divisibility);
  assert.equal(rune.membershipRoot, null, "a rune commits to no membership: the registry takes none");

  // the calls the registry is given
  const calls = listingCalls(plan);
  assert.equal(calls.length, TOTAL);
  assert.equal(calls.find((c) => c.ref === ONE.slug)!.functionName, "listInscriptions");
  assert.equal(calls.find((c) => c.ref === FIRST_RUNE.runeId)!.functionName, "listRune");
  const { args } = decodeFunctionData({ abi: parseAbi(["function listRune(string name, string runeId, uint8 divisibility, string btcVault)"]),
    data: calls.find((c) => c.ref === FIRST_RUNE.runeId)!.calldata });
  assert.deepEqual(args, [FIRST_RUNE.name, FIRST_RUNE.runeId, FIRST_RUNE.divisibility, btcVaultFixture(FIRST_RUNE.slug)]);
});

test("the membership root is marked a PLACEHOLDER, covers exactly the ids we have, and its proofs are ones OpenZeppelin accepts", () => {
  const plan = stagingPlan();
  for (const l of plan.listings.filter((x) => x.kind === "inscriptions")) {
    assert.equal(l.rootCovers!.placeholder, true, `${l.ref} must be marked a placeholder`);
    assert.match(l.rootCovers!.note, /placeholder/);
    assert.match(l.rootCovers!.note, /not its membership/);
    assert.match(l.rootCovers!.note, /Nothing here has verified/);
    assert.equal(l.rootCovers!.fixtureMembers, STAGING_FIXTURE_MEMBERS);
    assert.ok(l.rootCovers!.realIds.length >= 1 && l.rootCovers!.realIds.length <= 8, "only the handful of ids the catalogue holds");
    const supply = CATALOGUE.collections.find((c) => c.slug === l.ref)!.supply ?? 0;
    if (supply > 8) assert.ok(l.rootCovers!.realIds.length < supply, "and never the whole collection: nothing claims it does");
  }
  // the root is the root of those leaves, and every member can prove itself as MerkleProof.verify folds it
  const one = plan.listings.find((l) => l.ref === ONE.slug)!;
  const ids = membersOf(one.rootCovers!.realIds, STAGING_FIXTURE_MEMBERS);
  assert.deepEqual(ids.slice(0, one.rootCovers!.realIds.length), knownIdsOf(ONE));
  assert.equal(ids[one.rootCovers!.realIds.length], fixtureMemberId(0), "the fixture members follow the real ids");
  const leaves = ids.map(leafOf);
  assert.equal(merkleRoot(leaves), one.membershipRoot);
  for (const i of [0, 1, 2, 3, 17, 512, leaves.length - 1]) assert.equal(processProof(merkleProof(leaves, i), leaves[i]), one.membershipRoot, `member ${i}`);
  // and a leaf that is not in the tree cannot prove itself
  assert.notEqual(processProof(merkleProof(leaves, 4), leafOf("not-in-this-collection")), one.membershipRoot);
  // the manifest the keeper and the site read says the same thing
  assert.match(MANIFEST_NOTE, /PLACEHOLDER/);
  assert.match(MANIFEST_NOTE, /not its membership/);
});

test("a real chain is refused the fixture members: a listing there commits to real ids or it is not built", () => {
  assert.throws(() => plannedListings(CATALOGUE, { chainId: 4663, fixtureMembers: STAGING_FIXTURE_MEMBERS }), /refused: chain 4663 is not ours/);
  assert.throws(() => plannedListings(CATALOGUE, { chainId: 1, fixtureMembers: 1 }), /refused/);
  for (const id of OUR_CHAINS) assert.equal(plannedListings(CATALOGUE, { chainId: id, fixtureMembers: 4 }).listings.length, TOTAL);
  // with none, the root is over the real ids alone, and it is a different root
  const real = plannedListings(CATALOGUE, { chainId: 4663 }).listings.find((l) => l.ref === ONE.slug)!;
  const padded = stagingPlan().listings.find((l) => l.ref === ONE.slug)!;
  assert.notEqual(real.membershipRoot, padded.membershipRoot);
  assert.equal(real.rootCovers!.fixtureMembers, 0);
  assert.equal(merkleRoot(real.rootCovers!.realIds.map(leafOf)), real.membershipRoot);
});

test("runes get no cap: the catalogue has no rune floor, so nothing is posted and the vault refuses to buy", () => {
  const plan = stagingPlan();
  for (const r of plan.listings.filter((l) => l.kind === "rune")) {
    assert.equal(r.floorSats, null, `${r.ref} must carry no floor`);
    assert.equal(r.membershipRoot, null);
  }
  const prices = pricesFor(plan);
  assert.deepEqual(prices.runes, {}, "not one rune price: a cap would be invented, and an invented cap is a real spend");
  assert.equal(Object.keys(prices.inscriptions).length, LISTABLE.filter((c) => c.floorSats).length,
    "every listed collection that carries a floor, and only those");
  assert.equal(prices.inscriptions[ONE.slug], String(ONE.floorSats));
  assert.match(prices.note, /Runes carry no floor/);

  // and the keeper's own source refuses rather than guesses (keeper/src/sources.ts, FixtureSource)
  const dir = mkdtempSync(join(tmpdir(), "etch-prices-"));
  writePrices(dir, plan);
  const written = JSON.parse(readFileSync(`${dir}/prices.json`, "utf8")) as { inscriptions: Record<string, string>; runes: Record<string, string> };
  assert.equal(written.runes[FIRST_RUNE.runeId], undefined, "the rune the catalogue names has no price to post a cap from");
  assert.equal(written.inscriptions["demo-stones"], "150000", "and the demo listings keep the fixture prices they had");
});

test("the Bitcoin address is a marker nobody can be paid at, and one per vault", () => {
  const plan = stagingPlan();
  const seen = new Set<string>();
  for (const l of plan.listings) {
    assert.match(l.btcVault, /staging-fixture-not-a-real-address/, "it says what it is, wherever it is shown");
    assert.ok(l.btcVault.includes("-"), "a hyphen is not in the bech32m charset: no wallet will send to it");
    assert.ok(!/^(bc1|tb1)[02-9ac-hj-np-z]{20,}$/.test(l.btcVault), "and it must not be mistakable for a real address");
    assert.ok(!seen.has(l.btcVault), "one per vault, as a real deployment would have");
    seen.add(l.btcVault);
  }
});

test("what could not be listed is recorded, with the reason", () => {
  const plan = stagingPlan();
  assert.equal(plan.skipped.length, UNLISTABLE.length);
  assert.deepEqual(plan.skipped.map((s) => s.slug).sort(), UNLISTABLE.map((c) => c.slug).sort());
  for (const s of plan.skipped) {
    assert.match(s.why, /no inscription id/);
    const source = CATALOGUE.collections.find((c) => c.slug === s.slug)!;
    assert.equal(knownIdsOf(source).length, 0, "which is true of the catalogue");
  }
  // a collection with no id can never be given a root, not even an empty one
  assert.throws(() => merkleRoot([]), /would commit to nothing/);
});

test("coins spread across the whole inventory, weighted by the catalogue's own volume, a rune every fourth", () => {
  const plan = stagingPlan();
  const cols = plan.listings.filter((l) => l.kind === "inscriptions");
  const busiest = cols.reduce((a, b) => (b.weight > a.weight ? b : a));
  const quietest = cols.reduce((a, b) => (b.weight < a.weight ? b : a));
  const runes = plan.listings.filter((l) => l.kind === "rune");
  const order = launchOrder(plan.listings, TOTAL);
  assert.equal(new Set(order.map((l) => l.ref)).size, TOTAL, `${TOTAL} coins reach ${TOTAL} vaults before any vault gets a second`);
  assert.equal(order[0].weight, busiest.weight, "the busiest collection in the catalogue goes first");

  // How often a rune can come round is decided by the two counts, not by a number written here: the catalogue is fifty of
  // each kind, so the runes can outnumber the collections that carry an inscription id. Both claims are derived from what
  // the file actually holds, and both still say the thing that matters — the runes are SPREAD, never bunched at one end.
  const gap = Math.ceil(cols.length / Math.max(1, runes.length)) + 2;
  for (let i = 0; i + gap <= order.length; i++) assert.ok(order.slice(i, i + gap).some((l) => l.kind === "rune"), `a rune within every ${gap} coins, from ${i}`);
  const longestRun = order.reduce((acc, l) => (l.kind === "rune" ? { run: acc.run + 1, max: Math.max(acc.max, acc.run + 1) } : { run: 0, max: acc.max }), { run: 0, max: 0 }).max;
  assert.ok(longestRun <= Math.ceil(runes.length / Math.max(1, cols.length)) + 1, `runes bunched ${longestRun} deep, which is more than the two counts require`);

  // past the first pass a rune is every fourth, and the collections still come round -- whatever the weights are
  const second = launchOrder(plan.listings, TOTAL * 2).slice(TOTAL);
  for (let i = 3; i < second.length; i += 4) assert.equal(second[i].kind, "rune", `coin ${TOTAL + i} should go to a rune`);
  assert.ok(second.some((l) => l.kind === "inscriptions"),
    "a catalogue whose volumes sit inside one band weighs every collection the same, and must not launch into runes for ever");
  const many = launchOrder(plan.listings, TOTAL * 4), counts = new Map<string, number>();
  for (const l of many) counts.set(l.ref, (counts.get(l.ref) ?? 0) + 1);
  assert.ok(busiest.weight === quietest.weight || counts.get(busiest.ref)! > counts.get(quietest.ref)!,
    "a popular collection comes round again sooner");
  assert.equal(counts.size, TOTAL, "and nothing is left out");
  // the weight is the catalogue's, not a hand-picked list
  assert.equal(weightOf(1_000, 1_000), 1);
  assert.equal(weightOf(8_000, 1_000), 3);
  assert.equal(weightOf(1e9, 1), 5, "capped: the top is a band, not a runaway");
});

test("the weights say which of the catalogue's volume figures they came from, and never mix two windows", () => {
  const plan = stagingPlan();
  const basis = weightBasisOf(CATALOGUE);
  assert.equal(plan.weightBasis, basis.said, "the plan carries the basis, so nothing downstream has to assume one");

  // docs/phase9.md §11: a catalogue fetched with a key is ranked by the seven days before the fetch; one fetched before
  // that was recorded carries only a lifetime figure, and the plan must SAY lifetime rather than imply this week.
  if (CATALOGUE.collections.some((c) => (c.volume7dSats ?? 0) > 0)) {
    assert.equal(basis, WEIGHT_BASES.week);
    assert.match(plan.weightBasis, /seven-day/);
  } else {
    assert.match(plan.weightBasis, /LIFETIME|no volume figure at all/);
  }

  // whichever it is, every weight is read from that one field: a mixed pair of windows compared as one number is the
  // defect this basis exists to prevent
  const field = basis.field;
  const volumes = field ? CATALOGUE.collections.map((c) => c[field] ?? 0).filter((v) => v > 0) : [];
  const smallest = volumes.length ? Math.min(...volumes) : 0;
  for (const l of plan.listings.filter((x) => x.kind === "inscriptions")) {
    const c = CATALOGUE.collections.find((x) => x.slug === l.ref)!;
    assert.equal(l.weight, weightOf(field ? c[field] ?? 0 : 0, smallest), `${l.ref} is weighed by ${field ?? "nothing"}`);
  }
});

// ---- the batched creation, against a node that is not there -------------------------------------------------------------

const REG = parseAbi([
  "function count() view returns (uint256)",
  "function listing(uint256 id) view returns ((uint8 kind, bool paused, uint8 divisibility, address vault, bytes32 membershipRoot, string name, string ref, string btcVault))",
  "function listInscriptions(string name, string slug, bytes32 membershipRoot, string btcVault) returns (uint256)",
  "function listRune(string name, string runeId, uint8 divisibility, string btcVault) returns (uint256)",
  "event Listed(uint256 indexed id, uint8 kind, address vault, string name, string ref)",
]);

/** A registry that lives in a Map: enough JSON-RPC for the creator, and a count of how many transactions it was sent. */
function fakeNode(chainId = STAGING) {
  const listings: { kind: number; name: string; ref: string; root: Hex; btcVault: string; divisibility: number }[] = [];
  const sent = new Map<Hex, number>();
  const stats = { sends: 0, calls: 0, receipts: 0, maxInFlight: 0 };
  let inFlight = 0;
  const fetchImpl = (async (_url: string, init: { body: string }) => {
    const { method, params } = JSON.parse(init.body) as { method: string; params: unknown[] };
    const ok = (result: unknown) => ({ json: async () => ({ jsonrpc: "2.0", id: 1, result }) }) as unknown as Response;
    if (method === "eth_chainId") return ok(toHex(chainId));
    if (method === "eth_getTransactionCount") return ok("0x0");
    if (method === "eth_call") {
      stats.calls++;
      const { data } = (params[0] as { data: Hex });
      const d = decodeFunctionData({ abi: REG, data });
      if (d.functionName === "count") return ok(encodeFunctionResult({ abi: REG, functionName: "count", result: BigInt(listings.length) }));
      const l = listings[Number((d.args as readonly bigint[])[0])];
      return ok(encodeFunctionResult({ abi: REG, functionName: "listing",
        result: { kind: l.kind, paused: false, divisibility: l.divisibility, vault: vaultFor(listings.indexOf(l)), membershipRoot: l.root, name: l.name, ref: l.ref, btcVault: l.btcVault } as never }));
    }
    if (method === "eth_sendTransaction") {
      stats.sends++;
      inFlight++;
      stats.maxInFlight = Math.max(stats.maxInFlight, inFlight);
      const d = decodeFunctionData({ abi: REG, data: (params[0] as { data: Hex }).data });
      const a = d.args as readonly unknown[];
      listings.push(d.functionName === "listInscriptions"
        ? { kind: 0, name: a[0] as string, ref: a[1] as string, root: a[2] as Hex, btcVault: a[3] as string, divisibility: 0 }
        : { kind: 1, name: a[0] as string, ref: a[1] as string, root: `0x${"00".repeat(32)}` as Hex, btcVault: a[3] as string, divisibility: Number(a[2]) });
      const hash = keccak256(toHex(`tx-${listings.length}`));
      sent.set(hash, listings.length - 1);
      return ok(hash);
    }
    if (method === "eth_getTransactionReceipt") {
      stats.receipts++;
      inFlight = Math.max(0, inFlight - 1);
      const id = sent.get(params[0] as Hex)!;
      const l = listings[id];
      return ok({ status: "0x1", logs: [{ topics: encodeEventTopics({ abi: REG, eventName: "Listed", args: { id: BigInt(id) } }),
        data: encodeAbiParameters([{ type: "uint8" }, { type: "address" }, { type: "string" }, { type: "string" }], [l.kind, vaultFor(id), l.name, l.ref]) }] });
    }
    throw new Error(`the fake node was asked for ${method}`);
  }) as unknown as typeof fetch;
  return { fetchImpl, listings, stats };
}
const vaultFor = (id: number): Hex => `0x${(id + 1).toString(16).padStart(40, "0")}` as Hex;

function dataDir(): string {
  const dir = mkdtempSync(join(tmpdir(), "etch-listings-"));
  writeFileSync(`${dir}/deployment.json`, JSON.stringify({ registry: vaultFor(999), owner: vaultFor(1000), chainId: STAGING }));
  return dir;
}

test("every vault in the catalogue is created in batches, the count is written after every batch, and a restart finishes rather than repeats", async () => {
  const dir = dataDir(), node = fakeNode();
  const seenProgress: string[] = [];
  const m = await createListings({ node: "http://127.0.0.1:1", data: dir, repo: "/nowhere", catalogue: CATALOGUE, fetchImpl: node.fetchImpl, batch: 8,
    log: (o) => { if (o.event === "listings-batch") seenProgress.push(`${o.created}/${o.of}`); } });

  assert.equal(m.progress.created, TOTAL);
  assert.equal(m.progress.total, TOTAL);
  assert.equal(m.progress.done, true);
  const batches = Array.from({ length: Math.ceil(TOTAL / 8) }, (_x, i) => `${Math.min((i + 1) * 8, TOTAL)}/${TOTAL}`);
  assert.deepEqual(seenProgress, batches, "eight at a time, then whatever is left over");
  assert.ok(node.stats.maxInFlight > 1, "a batch is sent together, not one at a time");
  assert.ok(node.stats.maxInFlight <= 8, "and never more than a batch at once");
  assert.equal(node.stats.sends, TOTAL);

  // the manifest on disk: ids from the chain's own event, the placeholder claim, and what was skipped
  const written = readManifest(dir)!;
  assert.equal(written.listings.length, TOTAL);
  assert.equal(written.note, MANIFEST_NOTE);
  assert.equal(written.placeholderRoots, true);
  assert.deepEqual(written.skipped.map((s) => s.slug).sort(), UNLISTABLE.map((c) => c.slug).sort());
  assert.deepEqual(written.listings.map((l) => l.id), Array.from({ length: TOTAL }, (_x, i) => i), "each id is the one the Listed event gave");
  assert.equal(written.listings[0].vault, vaultFor(0));
  assert.equal(written.listings.find((l) => l.ref === FIRST_RUNE.runeId)!.floorSats, null);
  assert.ok(written.listings.every((l) => l.fromCatalogue));

  // run it again against the same registry: nothing new is sent
  const before = node.stats.sends;
  const again = await createListings({ node: "http://127.0.0.1:1", data: dir, repo: "/nowhere", catalogue: CATALOGUE, fetchImpl: node.fetchImpl, batch: 8 });
  assert.equal(node.stats.sends, before, "a restart lists nothing twice");
  assert.equal(again.progress.created, TOTAL);
  assert.equal(again.progress.done, true);
});

test("an interrupted inventory is finished by the next boot, and the count was honest while it filled", async () => {
  const dir = dataDir(), node = fakeNode();
  let stop = 0;
  await assert.rejects(createListings({ node: "http://127.0.0.1:1", data: dir, repo: "/nowhere", catalogue: CATALOGUE, fetchImpl: node.fetchImpl, batch: 8,
    log: (o) => { if (o.event === "listings-batch" && ++stop === 3) throw new Error("the container was restarted"); } }), /restarted/);
  const half = readManifest(dir)!;
  assert.equal(half.progress.created, 24, "what it says exists");
  assert.equal(half.progress.done, false, "and it does not claim to be finished");
  assert.equal(node.listings.length, 24, "which is what the registry holds");

  await createListings({ node: "http://127.0.0.1:1", data: dir, repo: "/nowhere", catalogue: CATALOGUE, fetchImpl: node.fetchImpl, batch: 8 });
  assert.equal(node.stats.sends, TOTAL, "the rest, and only the rest");
  assert.equal(readManifest(dir)!.progress.done, true);
});

test("the creator refuses any chain that is not ours, whatever it is told", async () => {
  const dir = dataDir();
  for (const chain of [4663, 46630, 1]) {
    await assert.rejects(createListings({ node: "http://127.0.0.1:1", data: dir, repo: "/nowhere", catalogue: CATALOGUE, fetchImpl: fakeNode(chain).fetchImpl }),
      /refused/, `chain ${chain}`);
  }
  await assert.rejects(createListings({ node: "https://rpc.mainnet.chain.robinhood.com", data: dir, repo: "/nowhere", catalogue: CATALOGUE, fetchImpl: fakeNode().fetchImpl }),
    /this machine only/);
});
