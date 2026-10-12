import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { test } from "node:test";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { collectionEquation, overall, reportSummary } from "../src/rook/sol/booksReserves";
import { stateOf } from "../src/rook/sol/canister";
import { COLLECTIONS, PINNED_COLLECTIONS, REGISTRY_ORDER, collectionBySlug } from "../src/rook/sol/collections";
import * as Demo from "../src/rook/sol/demo";
import * as DC from "../src/rook/sol/demoCanister";
import { demoActivity, demoChart, demoHolders, demoMarket, demoTrades } from "../src/rook/sol/demoMarket";
import { demoRaffles, demoSnapshot } from "../src/rook/sol/demoRaffle";
import { statusCopy } from "../src/rook/sol/intent";
import { launchableOf, openingOf, queueStatusOf, requestCopy } from "../src/rook/sol/opening";
import { BOTTOM, HOW_IT_WORKS, KNOWN, NAV, STATIC, sectionOf } from "../src/rook/routes";
import { HowFlow } from "../src/ui/HowFlow";

/**
 * The demo (`?demo`) shows the whole picture (HANDOFF §8, the site brought up to date 2026-10-12): collections open on all four chains,
 * one pinned collection not open yet with its "Open it" path, the 14 other Ordinals pinned, one graduated coin with its vault, the
 * intent desk's sample intents, raffles, and a Reserves page whose every book balances. Every page under `?demo` draws from these data
 * sets; this file builds each one (every method of the demo canister for every collection, the market, the charts, the raffles, the
 * reserves equations) so a fixture that throws or contradicts the docs fails here, not in a visitor's browser.
 */
const C = DC.demoCanister();
const now = Math.floor(Date.now() / 1000);

test("the demo canister's collections: the registry's numbering, 38 pinned + 3 Robinhood Chain, the open ones on four chains, one queued, Doodles not open, the Ordinals pinned", async () => {
  const cols = await C.coins_collections();
  assert.equal(cols.length, 41, "the ceremony's 38 and the three Robinhood Chain collections");
  for (const c of cols) assert.equal(REGISTRY_ORDER[c.index], COLLECTIONS.find((x) => x.wxSymbol === c.symbol)!.slug, `${c.symbol} at the registry's index`);
  const open = cols.filter((c) => c.stage === "Open");
  assert.deepEqual(open.map((c) => c.symbol).sort(), Demo.DEMO_OPEN.map((s) => collectionBySlug(s)!.wxSymbol).sort());
  for (const c of open) { assert.ok(c.config && c.wx_mint && c.module, c.symbol); assert.equal(c.config, Demo.demoConfigAddress(COLLECTIONS.find((x) => x.wxSymbol === c.symbol)!.slug)); }
  assert.equal(cols.find((c) => c.symbol === "wTAIYO")!.stage, "Requested", "one collection already asked for, in the queue");
  assert.equal(openingOf(cols, "wDOODLE").kind, "pinned");
  assert.equal(requestCopy(openingOf(cols, "wDOODLE"), "Doodles").action, "Open it");
  assert.equal(openingOf(cols, "wOMB").kind, "pinned", "the Ordinals beyond Bitcoin Frogs are pinned at the ceremony");
  assert.match(requestCopy({ kind: "not-pinned" }, "OMB", "Bitcoin").body, /not pinned/);
  assert.equal(requestCopy(openingOf(cols, "wREKTTRADO"), "Rekt Tradooors", "Robinhood Chain").action, "Open it", "pinned after the ceremony in the demo: it opens on demand");
  assert.match(requestCopy({ kind: "not-pinned" }, "Rekt Tradooors", "Robinhood Chain").body, /Robinhood Chain/);
  // the launch wizard lists every pinned collection the canister reports, the Robinhood Chain ones and the Ordinals included
  const list = launchableOf(cols).map((c) => c.slug);
  assert.equal(list.length, 41);
  assert.ok(list.includes("chainmancers") && list.includes("stonkbrokers") && list.includes("doodles") && list.includes("omb"));
  assert.deepEqual(DC.demoSummary().coming, []);
  // audit C4-12: the queue as coins_open_queue gives it: the queued collection first, waiting on the float, with the canister's reason
  const q = await C.coins_open_queue();
  assert.deepEqual(q.map((x) => [x.index, x.position]), [[DC.demoIndexOf(DC.DEMO_QUEUED), 0]]);
  const st = queueStatusOf(q, openingOf(cols, "wTAIYO"));
  assert.ok(st && st.waiting_since > 0n && /float/.test(st.why));
  assert.match(requestCopy(openingOf(cols, "wTAIYO"), "Taiyo Robotics", "Solana", st).body, /opens next\. Waiting since \d\d:\d\d: the float/);
  assert.ok(PINNED_COLLECTIONS.every((c) => list.includes(c.slug)));
});

test("Open it in the demo: Requested, then Opening, then Open, in this browser only", async () => {
  DC.demoResetOpenings();
  const i = DC.demoIndexOf("doodles");
  assert.equal(DC.demoStage("doodles"), "Pinned");
  assert.deepEqual(await C.coins_request_open(i), { Ok: "requested" });
  const t0 = Date.now();
  assert.equal(DC.demoStage("doodles", t0), "Requested");
  assert.equal(DC.demoStage("doodles", t0 + 7_000), "Opening");
  assert.equal(DC.demoStage("doodles", t0 + 13_000), "Open");
  assert.deepEqual((await C.coins_open_queue()).map((x) => [x.index, x.position, x.waiting_since]), [[DC.demoIndexOf(DC.DEMO_QUEUED), 0, (await C.coins_open_queue())[0].waiting_since], [i, 1, 0n]], "asked: behind the queued one");
  assert.ok("Err" in (await C.coins_request_open(9_999)), "an index the canister does not pin cannot be asked for");
  DC.demoResetOpenings();
  assert.equal(DC.demoStage("doodles"), "Pinned");
});

test("every open collection's books balance against the demo chain side: the Reserves page reads green, with items on every chain", async () => {
  const ids = await C.books_collections();
  const cols = await C.coins_collections();
  assert.equal(ids.length, Demo.DEMO_OPEN.length);
  const eqs = [];
  const kinds = new Set<string>();
  for (const [coll] of ids) {
    const v = (await C.books_view(coll))[0]!;
    assert.ok(v, `books for ${coll}`);
    kinds.add(Object.keys(v.kind)[0] + ("EthNft" in v.kind ? `:${v.kind.EthNft.chain_id}` : ""));
    const vaults = await C.coins_vaults(coll);
    const e = collectionEquation(v, cols.find((c) => c.index === coll)!.module, vaults, DC.demoChainSide(v, vaults));
    assert.equal(e.status, "green", `${coll}: ${e.reasons.join("; ")}`);
    eqs.push(e);
  }
  assert.deepEqual([...kinds].sort(), ["EthNft:1", "EthNft:4663", "Inscriptions", "SolNft"], "custody on Ethereum, Robinhood Chain, Bitcoin and Solana");
  assert.equal(overall(eqs), "green");
  assert.deepEqual(await C.books_view(DC.demoIndexOf("doodles")), [], "no books for a collection that is not open");
  const desk = await C.desk_status();
  assert.equal(reportSummary(desk.report[0] ?? null, desk.last_reconcile, now).tone, "green");
  const t = await C.treasury_status();
  assert.equal(t.treasury.reserves_bps + t.treasury.ops_bps, 10_000, "90 / 10");
  assert.equal(t.treasury.raffle_bps, 5_000);
});

test("one graduated coin has its backing in its vault; the others' wait; a coin redeems for its share", async () => {
  const milady = await C.coins_vaults(DC.demoIndexOf("milady"));
  const backed = milady.find((v) => v.stage === "Backed")!;
  const coin = Demo.DEMO_COINS().find((c) => c.mint === backed.coin)!;
  assert.equal(coin.meta.symbol, Demo.DEMO_BACKED_SYMBOL);
  assert.equal(backed.wx_in, 2_500_000_000n, "half of Milady's 5 wMILADY threshold");
  assert.ok(backed.vault.length === 1);
  assert.equal(DC.demoCoinSupply(coin.mint), 1_000_000_000_000_000n - backed.coins_burned);
  const all = (await Promise.all(Demo.DEMO_OPEN.map((s) => C.coins_vaults(DC.demoIndexOf(s))))).flat();
  assert.equal(all.filter((v) => v.stage === "Backed").length, 1);
  assert.ok(all.filter((v) => v.stage === "Migrated").length >= 3, "the other graduated coins");
});

test("the intent desk's samples: a mint done, a burn landing, a redemption waiting, a coin redemption done; quotes; opening refused", async () => {
  const want = { mint: "Done", burn: "Executing", redeem_nft: "Open", redeem_coin: "Done" } as const;
  for (const [kind, id] of Object.entries(DC.DEMO_INTENT_IDS)) {
    const v = (await C.intent_status(id))[0]!;
    assert.equal(v.kind, kind); assert.equal(stateOf(v), want[kind as keyof typeof want]);
    const s = statusCopy(v, now, "wMILADY");
    assert.ok(s.headline && s.detail, kind);
  }
  assert.deepEqual(await C.intent_status(1n), []);
  const q = await C.quote_mint(DC.demoIndexOf("milady"), 2_000_000_000n);
  assert.ok("Ok" in q && q.Ok.units > 0n && q.Ok.fee === 1_500_000n);
  const b = await C.quote_burn(DC.demoIndexOf("mad_lads"), 1_000_000_000n);
  assert.ok("Ok" in b && b.Ok.net > 0n);
  assert.ok("Err" in (await C.quote_mint(DC.demoIndexOf("doodles"), 2_000_000_000n)));
  assert.ok("Err" in (await C.open_intent({ Mint: { coll: 1, recipient: "x", min_out: [] } })));
});

test("the demo's market, charts, trades, holders, activity and raffles build for every coin and every collection", () => {
  const m = demoMarket();
  assert.equal(m.length, Demo.DEMO_COINS().length);
  for (const c of m) {
    assert.ok(demoChart(c.mint, "1m") !== null, `${c.symbol}: chart`);
    assert.ok(demoTrades(c.mint) !== null && demoHolders(c.mint) !== null, `${c.symbol}: trades and holders`);
    assert.equal(c.quote.symbol, c.collection.slug === "rook" ? "SOL" : collectionBySlug(c.collection.slug)!.wxSymbol);
  }
  assert.ok(demoActivity(50).items.length > 0);
  const rs = demoRaffles(now);
  assert.equal(rs.length, COLLECTIONS.length);
  const withRaffle = rs.filter((r) => r.raffle).map((r) => collectionBySlug(r.slug)!.chain);
  assert.ok(withRaffle.includes("Solana") && withRaffle.includes("Robinhood Chain") && withRaffle.includes("Bitcoin") && withRaffle.includes("Ethereum"));
  assert.ok(demoSnapshot("milady", 3n) !== null);
});

test("How it works: a top-level route in the nav (and the docs page of the same slug), its picture renders", () => {
  assert.ok(NAV.some(([to, label]) => to === HOW_IT_WORKS && label === "How it works"), "in the masthead nav");
  assert.ok(KNOWN.has(HOW_IT_WORKS) && STATIC.has(HOW_IT_WORKS));
  assert.equal(sectionOf(HOW_IT_WORKS), HOW_IT_WORKS);
  assert.ok(BOTTOM.length === 5, "the phone's bottom bar keeps its five places");
  const md = new URL("../src/content/docs/02-how-it-works.md", import.meta.url);
  assert.ok(existsSync(md));
  const text = readFileSync(md, "utf8");
  for (const h of ["A coin's life", "Buying with SOL", "Graduation thresholds", "How the NFTs are bought, chain by chain", "Redemption and delivery", "Opening a collection", "The pool desk", "The weekly raffle", "The treasury", "What you can check, and where"])
    assert.ok(text.includes(`## ${h}`), `the page covers: ${h}`);
  for (const chain of ["Ethereum", "Robinhood Chain", "Bitcoin", "Solana"]) assert.ok(text.includes(chain), chain);
  const html = renderToStaticMarkup(createElement(HowFlow));
  assert.match(html, /<figure class="flow"/);
  for (const w of ["Launch", "Graduate", "Buy", "Redeem", "Deliver", "Robinhood Chain", "Tensor"]) assert.ok(html.includes(w), w);
  // the home page links to it
  const home = readFileSync(new URL("../src/pages/SolHome.tsx", import.meta.url), "utf8");
  assert.ok((home.match(/href\(HOW_IT_WORKS\)/g) ?? []).length >= 2, "the hero and the about section link to How it works");
});
