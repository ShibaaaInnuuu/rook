import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { createElement as h } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { HandinReport, ReserveReport, WrappedCollection } from "../src/api/types.js";
import { marketVenues, shiftTimes, wrappedFixtures } from "../src/lib/wrappedSource.js";
import { quoteCoin } from "../src/lib/quote.js";
import { redeemDestinationProblem } from "../src/lib/redeem.js";
import { Equation, HandinRows, HandinTotals, ItemRows, LaunchFeeLine, LiquidityPanel, ModuleBooks, POLICY_LINE, Proof, QuoteSummary, StatusBadge, TicketRows, TreasuryPanel, YoyoPanel, launchExtraNow, ticketStage } from "../src/components/wx.js";
import { NAV, KNOWN, sectionOf } from "../src/lib/routes.js";

/** docs/v5/ux-and-reserves.md: the fixture source, the one-click summary as a visitor reads it, and the proof-of-reserves pieces. */
const here = (p: string) => new URL(p, import.meta.url);
const read = async (f: string) => JSON.parse(readFileSync(here(`../public/fixtures/wrapped/${f}`), "utf8"));
const index = JSON.parse(readFileSync(here("../public/fixtures/wrapped/index.json"), "utf8")) as { generatedAt: number };
const E = 10n ** 18n;
// renderToStaticMarkup needs `location` for href(); the shell's hash router is not under test here
(globalThis as { location?: unknown }).location ??= { search: "", hash: "" };

test("the fixture set is labelled, moved to now, and refuses paths it does not have", async () => {
  const src = await wrappedFixtures(index.generatedAt + 1_000, read);
  assert.equal(src.kind, "fixture"); assert.match(src.note, /^FIXTURE\. Nothing on this page is deployed/); assert.equal(src.shiftS, 1_000);
  const r = await src.get<ReserveReport>("/v1/reserves/frogs");
  assert.equal(r.source.kind, "fixture", "every file says it is a fixture");
  assert.equal(r.computedAt, index.generatedAt + 1_000, "times are moved so a deadline in the fixture is still ahead");
  await assert.rejects(() => src.get("/v1/reserves/nope"), /no answer/);
  assert.deepEqual(shiftTimes({ at: 1, deliverBy: null, round: 5, x: [{ checkedAt: 2 }] }, 10), { at: 11, deliverBy: null, round: 5, x: [{ checkedAt: 12 }] });
});

test("the fixture's coins quote end to end: ALIEN after graduation through both pools, BETA on its curve", async () => {
  const src = await wrappedFixtures(index.generatedAt, read);
  const c = await src.get<WrappedCollection>("/v1/wrapped/frogs");
  const alien = c.coinsList.find((x) => x.symbol === "ALIEN")!, beta = c.coinsList.find((x) => x.symbol === "BETA")!;
  const a = await quoteCoin("buy", E / 10n, marketVenues(c.market, alien.address, "graduated", false), 100);
  // the fixture's module (docs/v5/floor-module.md §13.4: price 0.64, 21 bps) beats the pool's 0.647 with impact, and the router routes there
  assert.ok(a.ok); assert.equal(a.quote.route.id, "floorModule"); assert.equal(a.quote.route.legs.length, 2); assert.equal(a.quote.route.legs[0].venue, "floorMint");
  assert.ok(a.quote.alternatives.some((r) => r.id === "pool" && r.executable), "the pool is quoted beside it");
  assert.ok(a.quote.totalCostBps > 140 && a.quote.totalCostBps < 2_000, `a plausible cost: ${a.quote.totalCostBps}`);
  const b = await quoteCoin("buy", E / 10n, marketVenues(c.market, beta.address, "trading", false), 100);
  assert.ok(b.ok); assert.equal(b.quote.route.id, "curve");
  const s = await quoteCoin("sell", 1_000_000n * E, marketVenues(c.market, alien.address, "graduated", false), 100);
  assert.ok(s.ok && s.quote.amountOut > 0n);
});

test("the ticket says one number and never names the wrapped token until Details is opened", async () => {
  const src = await wrappedFixtures(index.generatedAt, read);
  const c = await src.get<WrappedCollection>("/v1/wrapped/frogs");
  const alien = c.coinsList.find((x) => x.symbol === "ALIEN")!;
  const r = await quoteCoin("buy", E / 10n, marketVenues(c.market, alien.address, "graduated", false), 100);
  assert.ok(r.ok);
  const html = renderToStaticMarkup(h(QuoteSummary, { quote: r.quote, symbol: "ALIEN", wx: "wFROG", slippageBps: 100, deadlineMin: 20, estimate: true }));
  const [summary, details] = html.split("<details");
  assert.match(summary, /You pay.*0\.100000 ETH/); assert.match(summary, /You get.*ALIEN/); assert.match(summary, /Total cost.*\d\.\d\d%/);
  assert.ok(!/wFROG|wX|escrow|mint|burn/i.test(summary), "no wX, escrow, mint or burn outside Details");
  assert.match(details, /wFROG/); assert.match(details, /Uniswap protocol fee/); assert.match(details, /liquidity providers/); assert.match(details, /Price impact/);
  assert.match(details, /EtchRouter reverts the whole trade if it is not mined within 20 minutes/);
  assert.match(details, /estimate from fixture data/);
  // the curve has no deadline parameter, and the ticket says so rather than pretend
  const beta = c.coinsList.find((x) => x.symbol === "BETA")!;
  const q = await quoteCoin("buy", E / 10n, marketVenues(c.market, beta.address, "trading", false), 100);
  assert.ok(q.ok);
  assert.match(renderToStaticMarkup(h(QuoteSummary, { quote: q.quote, symbol: "BETA", wx: "wFROG", slippageBps: 100, deadlineMin: 20, estimate: true })), /take no deadline/);
});

test("proof of reserves: green says Verified, unverified never looks green, and a fixture's links are text", async () => {
  const src = await wrappedFixtures(index.generatedAt, read);
  const frogs = await src.get<ReserveReport>("/v1/reserves/frogs"), apes = await src.get<ReserveReport>("/v1/reserves/apes");
  assert.equal(frogs.status, "green"); assert.equal(apes.status, "unverified");
  assert.match(renderToStaticMarkup(h(StatusBadge, { status: "green" })), /wx-status--green">Verified/);
  assert.match(renderToStaticMarkup(h(StatusBadge, { status: "unverified" })), /wx-status--unverified">Unverified/);
  assert.match(renderToStaticMarkup(h(StatusBadge, { status: "red" })), /Mismatch/);
  const eq = renderToStaticMarkup(h(Equation, { r: frogs }));
  // docs/v5/floor-module.md §13.1: (6 − 2) × 1 + (1.875 convertible + 0.625 reserve) = 6.5, against 1.585 ETH in both pots
  assert.match(eq, /\(<span[^>]*>6<\/span> − <span[^>]*>2<\/span>\) × 1 \+ \(<span[^>]*>1\.875<\/span> \+ <span[^>]*>0\.625<\/span>\) = <strong>6\.5<\/strong>/);
  assert.match(eq, /0\.5 float units/); assert.match(eq, /1\.5850 ETH in escrow = 1\.1887 convertible pot \+ 0\.3962 standing reserve/);
  assert.match(renderToStaticMarkup(h(Equation, { r: apes })), /no Floor Module is deployed/);
  assert.match(renderToStaticMarkup(h(Equation, { r: apes })), /<span class="wx-eq__vs"> \? <\/span>/, "undecided is a question mark, not an equals sign");
  const rows = renderToStaticMarkup(h(ItemRows, { items: frogs.items, kind: "inscriptions", fixture: true }));
  assert.ok(!/<a /.test(rows), "no clickable link into an invented id");
  assert.match(rows, /in custody/); assert.match(rows, /drawn by ticket 1/);
  const liveRows = renderToStaticMarkup(h(ItemRows, { items: frogs.items, kind: "inscriptions", fixture: false }));
  assert.match(liveRows, /href="https:\/\/ordinals\.com\/inscription\//); assert.match(liveRows, /href="https:\/\/mempool\.space\/tx\//);
  assert.equal(renderToStaticMarkup(h(Proof, { href: "https://x", fixture: false, children: "x" })), '<a class="mono" href="https://x" target="_blank" rel="noreferrer noopener">x</a>');
  const t = renderToStaticMarkup(h(TicketRows, { tickets: frogs.ticketsInFlight, now: index.generatedAt, fixture: true }));
  assert.match(t, /Ticket 1/); assert.match(t, /deliver by/); assert.match(t, /Ticket 2/); assert.match(t, /waiting for drand round/);
  const late = { ...frogs.ticketsInFlight.find((x) => x.stage === "resolved")! };
  assert.match(ticketStage(late, late.deliverBy! + 1), /overdue.*refundable/);
});

test("redemption destinations: taproot for inscriptions; an EVM address, not zero and not the custodian, for NFTs", async () => {
  const src = await wrappedFixtures(index.generatedAt, read);
  const frogs = await src.get<WrappedCollection>("/v1/wrapped/frogs"), apes = await src.get<WrappedCollection>("/v1/wrapped/apes");
  assert.match(redeemDestinationProblem(frogs, "0x" + "1".repeat(40))!, /will not be accepted/);
  assert.equal(redeemDestinationProblem(frogs, "bc1p5d7rjq7g6rdk2yhzks9smlaqtedr4dekq08ge8ztwac72sfr9rusxg3297"), null);
  assert.equal(redeemDestinationProblem(apes, "0x" + "1".repeat(40)), null);
  assert.match(redeemDestinationProblem(apes, "0x" + "0".repeat(40))!, /zero/);
  assert.match(redeemDestinationProblem(apes, apes.custody.kind === "evm" ? apes.custody.custodian : "")!, /custody address itself/);
  assert.match(redeemDestinationProblem(apes, "bc1p…")!, /not an EVM address/);
});

test("rent is a description with nothing to press, and the new pages are routed under Reserves", () => {
  // the page imports the router (and through it the chain loader), so it is read as source: no control of any kind in it
  const rent = readFileSync(here("../src/pages/Rent.tsx"), "utf8");
  assert.ok(!/<(button|input|form|select|textarea)|TxButton|onClick|writeContract/.test(rent), "no fake functionality");
  assert.match(rent, /Not built yet/);
  assert.ok(NAV.some(([to, label]) => to === "reserves" && label === "Reserves"));
  for (const p of ["collection", "reserves", "redeem", "rent", "handins"]) { assert.ok(KNOWN.has(p)); assert.equal(sectionOf(p), "reserves"); }
});

test("the offer board page (docs/v6/offer-handins.md): what is on it, what was paid, how a bot earns it — and nothing to press", async () => {
  const handins = readFileSync(here("../src/pages/Handins.tsx"), "utf8");
  assert.ok(!/<(button|input|form|select|textarea)|TxButton|onClick|writeContract/.test(handins), "no fake functionality: a hand-in is the bot's own transaction");
  assert.match(handins, /How a bot earns the reward/); assert.match(handins, /submit\(sha256\(psbt\)/); assert.match(handins, /submit_offer/); assert.match(handins, /withdraw\(\)/);
  const src = await wrappedFixtures(index.generatedAt, read);
  const frogs = await src.get<HandinReport>("/v1/handins/frogs"), apes = await src.get<HandinReport>("/v1/handins/apes");
  assert.equal(frogs.error, null); assert.deepEqual(frogs.open.map((x) => x.status), ["open"]); assert.equal(frogs.totals.claimed, 1); assert.equal(frogs.totals.slashed, 1);
  assert.equal(apes.board, null); assert.match(apes.error!, /Bitcoin collections/);
  const totals = renderToStaticMarkup(h(HandinTotals, { h: frogs }));
  assert.match(totals, /Reward pool/); assert.match(totals, /0\.060<\/span><span class="unit"> ETH/); assert.match(totals, /Rewards paid/); assert.match(totals, /Bonds slashed/); assert.match(totals, /1 of 64/);
  const rows = renderToStaticMarkup(h(HandinRows, { rows: [...frogs.open, ...frogs.settled], now: index.generatedAt, fixture: true, custodian: frogs.custodian }));
  assert.match(rows, /bought and rewarded · reward 0\.010 ETH/); assert.match(rows, /slashed/); assert.match(rows, /Only the custodian/);
  assert.ok(!/<a /.test(rows), "a fixture's transactions are text, not links");
});

test("the coin page's policy, tier and launch fee (docs/v5/policies-and-fees.md), and the YOYO vault's books", async () => {
  const src = await wrappedFixtures(index.generatedAt + 100, read);
  const c = await src.get<WrappedCollection>("/v1/wrapped/frogs");
  const gamma = c.coinsList.find((x) => x.symbol === "GAMMA")!, alien = c.coinsList.find((x) => x.symbol === "ALIEN")!;
  assert.equal(gamma.policy, "yoyo"); assert.equal(gamma.feeBps, 500); assert.match(POLICY_LINE.yoyo, /relist them at a markup/);
  // the launch fee's countdown runs on the clock: GAMMA was 230 s into its 600 s decay at generation, and the fixture's times are
  // moved forward by the 100 s since, so 370 s are left now: 5,000 × 370 / 600
  const now = index.generatedAt + 100;
  assert.equal(launchExtraNow(gamma.launchFee, now), 3_083);
  assert.equal(launchExtraNow(gamma.launchFee, now + 10_000), 0, "decayed");
  assert.equal(launchExtraNow(alien.launchFee, now), 0);
  const line = renderToStaticMarkup(h(LaunchFeeLine, { coin: gamma, now }));
  assert.match(line, /30\.83%/); assert.match(line, /falling to nothing/); assert.match(line, /6 min 10 s/); assert.match(line, /every wei of it to the coin&#x27;s vault/);
  assert.match(renderToStaticMarkup(h(LaunchFeeLine, { coin: alien, now })), /decayed to nothing: a buy pays the 1% tier alone/);
  const yoyo = renderToStaticMarkup(h(YoyoPanel, { coin: gamma, wx: "wFROG" }));
  assert.match(yoyo, /Open listings/); assert.match(yoyo, /Proceeds waiting/); assert.match(yoyo, /0\.3500/); assert.match(yoyo, /Bought back and burned/); assert.match(yoyo, /0\.3400/);
  assert.match(yoyo, /Ticket 4/); assert.match(yoyo, /Ticket 3/); assert.match(yoyo, /sold for 0\.6900 ETH/);
  assert.equal(renderToStaticMarkup(h(YoyoPanel, { coin: alien, wx: "wFROG" })), "", "a raffle coin has no YOYO panel");
});

test("proof of reserves shows the module's books, the treasury's pots and the protocol's liquidity, each from its own read", async () => {
  const src = await wrappedFixtures(index.generatedAt, read);
  const frogs = await src.get<ReserveReport>("/v1/reserves/frogs"), apes = await src.get<ReserveReport>("/v1/reserves/apes");
  const books = renderToStaticMarkup(h(ModuleBooks, { r: frogs, now: index.generatedAt }));
  assert.match(books, /Price the module trades at/); assert.match(books, /0\.6400/); assert.match(books, /The keeper&#x27;s floor/); assert.match(books, /0\.6330/);
  assert.match(books, /\+1\.10% from the keeper&#x27;s floor/, "the fills price against the keeper floor"); assert.match(books, /0\.21% \/ 0\.15%/); assert.match(books, /Standing reserve/); assert.match(books, /Float inventory/);
  assert.match(books, /mint up to 3\.0000 ETH/); assert.match(books, /0\.6450 ETH/, "the last fill");
  assert.match(renderToStaticMarkup(h(ModuleBooks, { r: apes, now: index.generatedAt })), /no Floor Module is deployed/);
  const treasury = renderToStaticMarkup(h(TreasuryPanel, { r: frogs }));
  assert.match(treasury, /0\.8000<\/span><span class="unit"> ETH/); assert.match(treasury, /Collection reserves pot/); assert.match(treasury, /40\.00% of every wei/); assert.match(treasury, /Item floats pot/); assert.match(treasury, /Bootstrap pot/);
  assert.match(treasury, /weight 100\.00%/); assert.match(treasury, /loan outstanding 0\.0000 ETH/); assert.match(treasury, /no function that names a recipient/);
  assert.match(renderToStaticMarkup(h(TreasuryPanel, { r: apes })), /no protocol treasury is named/);
  const liq = renderToStaticMarkup(h(LiquidityPanel, { r: frogs }));
  assert.match(liq, /2 in the shared pool/); assert.match(liq, /0\.0100 ETH and 0 wFROG/); assert.match(liq, /33\.0000 ETH and 51 wFROG/); assert.match(liq, /go to the treasury/);
});

test("the site is called Rook on screen; repo and package names stay", () => {
  const app = readFileSync(here("../src/App.tsx"), "utf8"), html = readFileSync(here("../index.html"), "utf8");
  assert.match(app, /aria-label="Rook, home">Rook</); assert.match(html, /<title>Rook<\/title>/); assert.ok(!/>etched</.test(app));
  assert.equal(JSON.parse(readFileSync(here("../package.json"), "utf8")).name, "etch-web", "the package keeps its name");
});
