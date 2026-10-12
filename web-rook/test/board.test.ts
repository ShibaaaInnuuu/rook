import assert from "node:assert/strict";
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { test } from "node:test";
import type { Board, RookChart } from "../src/rook/apiTypes.js";
import { ageText, capText, changeText, defaultSort, king, progressOf, searchRows, sortRows, tabRows, trendingScore } from "../src/rook/boardlogic.js";
import { rookFixtures, shiftTimes } from "../src/rook/fixtures.js";

/** docs/v6/site-design.md §6 (amended): the board's tabs, sorts and search over the fixture, and the copies that must not drift. */
const here = (p: string) => new URL(p, import.meta.url);
const read = async (f: string) => JSON.parse(readFileSync(here(`../public/fixtures/rook/${f}`), "utf8"));
const index = JSON.parse(readFileSync(here("../public/fixtures/rook/index.json"), "utf8")) as { generatedAt: number };

test("the rook types and fixtures are byte copies of the indexer's (checked whenever the repository copy is beside us)", () => {
  if (!existsSync(here("../../indexer/src/rook/types.ts"))) return;
  assert.equal(readFileSync(here("../src/rook/apiTypes.ts"), "utf8"), readFileSync(here("../../indexer/src/rook/types.ts"), "utf8"));
  for (const f of readdirSync(here("../../indexer/fixtures/rook/"))) assert.equal(readFileSync(here(`../public/fixtures/rook/${f}`), "utf8"), readFileSync(here(`../../indexer/fixtures/rook/${f}`), "utf8"), f);
});

test("the fixture board has both kinds of coin, moved to now, and every tab and sort orders it without inventing a number", async () => {
  const src = await rookFixtures(index.generatedAt + 500, read);
  assert.equal(src.kind, "fixture"); assert.match(src.note, /^FIXTURE/); assert.equal(src.shiftS, 500);
  const b = await src.get<Board>("/v1/rook/board");
  assert.equal(b.source.kind, "fixture");
  assert.ok(b.rows.some((r) => r.kind === "curve") && b.rows.some((r) => r.kind === "pons"));
  const beta = b.rows.find((r) => r.symbol === "BETA")!;
  assert.equal(beta.launchedAt, (await read("board.json")).rows.find((r: { symbol: string }) => r.symbol === "BETA").launchedAt + 500, "times are moved");
  assert.equal(progressOf(beta), 0.24); assert.equal(progressOf(b.rows.find((r) => r.symbol === "ALIEN")!), 1);
  const now = b.at + 500;
  assert.ok(tabRows(b.rows, "graduated", now).every((r) => r.phase === "graduated"));
  assert.ok(tabRows(b.rows, "graduating", now).some((r) => r.symbol === "APEB"));
  assert.ok(tabRows(b.rows, "trending", now).every((r) => (r.trades24h ?? 0) > 0 || BigInt(r.volume24hWei ?? "0") > 0n));
  assert.equal(tabRows(b.rows, "all", now).length, b.rows.length);
  const byCap = sortRows(b.rows, "cap");
  for (let i = 1; i < byCap.length; i++) assert.ok((byCap[i - 1].marketCapEth ?? -1) >= (byCap[i].marketCapEth ?? -1), "unknown caps sort last");
  const byVol = sortRows(b.rows, "volume");
  assert.ok(BigInt(byVol[0].volume24hWei ?? "0") >= BigInt(byVol[1].volume24hWei ?? "0"));
  assert.deepEqual([defaultSort("new"), defaultSort("trending"), defaultSort("graduating"), defaultSort("all")], ["age", "trending", "progress", "cap"]);
  assert.deepEqual(tabRows(b.rows, "watch", now, [beta.address.toUpperCase()]).map((r) => r.symbol), ["BETA"], "the watch tab is the starred rows, whatever the case of the address");
  assert.equal(searchRows(b.rows, "beta")[0].symbol, "BETA"); assert.ok(searchRows(b.rows, "bitcoin frogs").length >= 2, "a nest name finds its coins"); assert.equal(searchRows(b.rows, "zzz").length, 0);
  const k = king(b.rows)!;
  assert.ok(k.phase === "trading" || k.phase === "graduated");
  assert.equal(k.address, sortRows(b.rows, "trending").find((r) => r.phase === "trading" || r.phase === "graduated")!.address, "the king is the top trending tradable coin");
  const chart = await src.get<RookChart>("/v1/rook/coins/0x1000000000000000000000000000000000000a11/chart?res=5m");
  assert.equal(chart.seconds, 300); assert.ok(chart.candles.length > 0); assert.equal(chart.graduation!.at, (await read("chart-0x1000000000000000000000000000000000000a11-5m.json")).graduation.at + 500);
  await assert.rejects(() => src.get("/v1/rook/coins/0x0000000000000000000000000000000000000000/chart?res=5m"), /no answer/);
});

test("the trending score: volume, new holders and buys, each a log; an unknown term counts zero; no term at all is null", async () => {
  const src = await rookFixtures(index.generatedAt, read);
  const b = await src.get<Board>("/v1/rook/board");
  const beta = b.rows.find((r) => r.symbol === "BETA")!;
  const vol = Number(BigInt(beta.volume24hWei!)) / 1e18;
  assert.ok(Math.abs(trendingScore(beta)! - (Math.log1p(vol) + 0.5 * Math.log1p(beta.newHolders24h!) + 0.25 * Math.log1p(beta.buys24h!))) < 1e-12);
  const pons = b.rows.find((r) => r.kind === "pons")!;
  assert.equal(pons.buys24h, null, "the recording does not know the day's buys"); assert.equal(pons.newHolders24h, null);
  assert.ok(Math.abs(trendingScore(pons)! - Math.log1p(Number(BigInt(pons.volume24hWei!)) / 1e18)) < 1e-12, "unknown terms count zero");
  assert.equal(trendingScore({ ...beta, volume24hWei: null, newHolders24h: null, buys24h: null }), null);
  assert.ok(trendingScore({ ...beta, volume24hWei: "100000000000000000000", newHolders24h: 0, buys24h: 1 })! < trendingScore({ ...beta, volume24hWei: "10000000000000000000", newHolders24h: 200, buys24h: 300 })!, "a hundred ETH from one buyer ranks under ten ETH from two hundred");
  const w = await src.get<{ holdings: { coin: { symbol: string } }[]; trades: unknown[]; launches: unknown[] }>("/v1/rook/wallets/0x4000000000000000000000000000000000000001");
  assert.ok(w.holdings.length >= 3 && w.trades.length >= 3 && w.launches.length === 0, "the fixture's first buyer holds and traded, launched nothing");
});

test("the words a row prints: caps in dollars or ETH, changes signed, ages short, and never a number for an unknown", () => {
  assert.equal(capText(null, 2800), "—"); assert.equal(capText(2, 2800), "$5.6k"); assert.equal(capText(500, 2800), "$1.40M"); assert.equal(capText(2, null), "2.00 ETH"); assert.equal(capText(0.0042, null), "0.00420 ETH");
  assert.equal(changeText(null), "—"); assert.equal(changeText(0.123), "+12.3%"); assert.equal(changeText(-0.05), "-5.00%");
  assert.equal(ageText(null, 100), "—"); assert.equal(ageText(70, 100), "30s"); assert.equal(ageText(0, 90_000), "1d");
  assert.deepEqual(shiftTimes({ at: 1, t: 2, price: 3, x: [{ launchedAt: 4 }] }, 10), { at: 11, t: 12, price: 3, x: [{ launchedAt: 14 }] });
});
