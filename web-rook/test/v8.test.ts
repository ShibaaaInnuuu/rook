import assert from "node:assert/strict";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { test } from "node:test";
import { servedStatus } from "../src/rook/v8/core/ethReserve";
import { nextTicketOdds } from "../src/rook/v8/core/odds";
import type { DrawReport, EthReserveReport, MetadataSnapshot } from "../src/rook/v8/core/types";
import { BANNED_WORDS, HOW_IT_WORKS, bannedIn, headline, oddsFor, snapshotCheck, verifyAll } from "../src/rook/v8/present";
import { shiftDraws, v8Fixtures } from "../src/rook/v8/fixtures";

/** docs/v8: the redemption page and the reserves page's Ethereum section, from the fixture set the indexer wrote. */
const here = (p: string) => new URL(p, import.meta.url);
const fx = <T,>(f: string) => JSON.parse(readFileSync(here(`../public/fixtures/v8/${f}`), "utf8")) as T;
const index = fx<{ generatedAt: number; files: Record<string, string> }>("index.json");
const draws = fx<DrawReport>("draws-milady.json"), reserve = fx<EthReserveReport>("reserves-milady.json"), snap = fx<MetadataSnapshot>("snapshot-milady.json");
const GEN = index.generatedAt;

test("the v8 core is the indexer's, byte for byte (indexer/src/v8, every file but its reads, service, routes and snapshot job)", () => {
  if (!existsSync(here("../../indexer/src/v8/"))) return;
  const ours = readdirSync(here("../src/rook/v8/core/")).sort();
  const theirs = readdirSync(here("../../indexer/src/v8/")).filter((f) => !["reads.ts", "service.ts", "routes.ts", "snapshotJob.ts"].includes(f)).sort();
  assert.deepEqual(ours, theirs);
  for (const f of ours) assert.equal(readFileSync(here(`../src/rook/v8/core/${f}`), "utf8"), readFileSync(here(`../../indexer/src/v8/${f}`), "utf8"), `src/rook/v8/core/${f} has drifted: re-copy it`);
});

test("every published proof in the fixture verifies in the browser's code: real drand signatures, the pin, the set, the index", () => {
  const rows = verifyAll(draws);
  assert.equal(rows.length, 8);
  for (const r of rows) assert.equal(r.verdict.verdict, "verified", `ticket ${r.ticket}`);
  assert.equal(rows.find((r) => r.ticket === 4)!.verdict.timing.level, "flagged");
  assert.equal(rows.find((r) => r.ticket === 5)!.verdict.timing.level, "unknown");
});
test("a tampered fixture proof is a mismatch in the browser", () => {
  const bad = structuredClone(draws);
  const p = bad.proofs[2]; p.itemId = p.freeSet.find((x) => x !== p.itemId)!;
  const sig = bad.proofs[4]; sig.beacon.signature = bad.proofs[5].beacon.signature;
  const rows = verifyAll(bad);
  assert.equal(rows.find((r) => r.ticket === p.ticket)!.verdict.verdict, "mismatch");
  assert.equal(rows.find((r) => r.ticket === sig.ticket)!.verdict.verdict, "mismatch");
});
test("the odds: exact for the next ticket, shown only from a fresh vault read with no disagreement", () => {
  const moved = shiftDraws(draws, 0);
  const fresh = oddsFor(moved, GEN + 60);
  assert.ok(fresh.odds); assert.equal(fresh.odds!.forTicket, 9); assert.equal(fresh.odds!.method, "exact");
  assert.equal(fresh.odds!.items.find((i) => i.p.d === "11")!.p.n, "1");
  const stale = oddsFor(moved, GEN + 2 * 3_600);
  assert.equal(stale.odds, null); assert.match(stale.why!, /older than an hour/);
  assert.equal(oddsFor({ ...moved, stateError: "the views and the replay disagree" }, GEN).odds, null);
  assert.equal(oddsFor({ ...moved, state: null }, GEN).odds, null);
});
test("the wording (R1): the vault's composition with equal or exact odds; none of the words §2.4 rules out", () => {
  const o = nextTicketOdds(draws.state!);
  const h = headline(o, "Milady Maker");
  assert.match(h.title, /^One of these 11 Milady Makers, chosen at random$/);
  assert.match(h.body, /burned before yours draws first/);
  const uniform = headline({ ...o, ahead: 0 }, "Milady Maker");
  assert.equal(uniform.body, "Each is equally likely: 1 in 11.");
  for (const text of [h.title, h.body, uniform.body, ...HOW_IT_WORKS]) assert.deepEqual(bannedIn(text), [], text);
  const page = readFileSync(here("../src/pages/Draw.tsx"), "utf8");
  assert.deepEqual(bannedIn(page), [], "the redemption page's source uses none of the ruled-out words");
  assert.ok(!/rarity|rank/i.test(page.replace(/\/\*\*[\s\S]*?\*\//g, "")), "R2: no rarity rank on the redemption page");
  assert.deepEqual(bannedIn("You could win a rare drop!"), ["win", "rare drop"]);
  assert.ok(BANNED_WORDS.includes("jackpot"));
});
test("the snapshot served matches the hash the report pins; a changed trait does not", () => {
  assert.equal(snapshotCheck(snap, draws.snapshot!.sha256).ok, true);
  const t = structuredClone(snap); t.tokens[0].traits.Core = "Chrome";
  assert.equal(snapshotCheck(t, draws.snapshot!.sha256).ok, false);
});
test("the reserves status is re-judged when shown: fresh green, an old number unverified, past 3 h red", () => {
  assert.equal(reserve.status, "green");
  assert.equal(servedStatus(reserve, reserve.computedAt + 30).status, "green");
  assert.equal(servedStatus(reserve, reserve.computedAt + 3_700).status, "unverified");
  assert.equal(servedStatus(reserve, reserve.computedAt + 3 * 3_600 + 1).status, "red");
});
test("the fixture source moves reserve reports forward and never a draw proof's times", async () => {
  const read = async (f: string) => fx<unknown>(f);
  const later = GEN + 5 * 86_400;
  const src = await v8Fixtures(later, read);
  assert.equal(src.kind, "fixture"); assert.equal(src.shiftS, 5 * 86_400);
  const r = await src.get<EthReserveReport>("/v1/v8/reserves/milady");
  assert.equal(r.computedAt, reserve.computedAt + src.shiftS);
  assert.equal(servedStatus(r, later).status, "green", "a fixture's green stays fresh");
  const d = await src.get<DrawReport>("/v1/v8/draws/milady");
  assert.deepEqual(d.proofs, draws.proofs, "proofs are pinned to their real rounds");
  assert.equal(d.state!.at, draws.state!.at! + src.shiftS);
  assert.ok(oddsFor(d, later).odds, "the fixture's odds are current");
  const self = await src.get<{ report: EthReserveReport; calls: unknown[] }>("selfcheck/milady");
  assert.equal(servedStatus(self.report, later).status, "green"); assert.ok(self.calls.length > 50);
  await assert.rejects(src.get("/v1/v8/reserves/nothing"), /no answer/);
});
test("the fixture's reserves: every number has its sources and block; providers are hosts, never URLs with a path", () => {
  for (const n of reserve.numbers) assert.ok(n.sources.length > 0, n.key);
  assert.ok([...reserve.providers.eth, ...reserve.providers.home].every((p) => !p.includes("/")));
  assert.ok(reserve.items.every((i) => i.owners.length === 2 && i.arrival.verdict === "ok"));
  assert.match(reserve.canisterNote, /never turn anything green/);
});
