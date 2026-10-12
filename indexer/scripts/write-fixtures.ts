import { mkdirSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import type { AddressInfo } from "node:net";
import { serve } from "../src/api.js";
import { openDb } from "../src/db.js";
import { apply, lotOf } from "../src/fold.js";
import { Live } from "../src/live.js";
import { Views } from "../src/views.js";
import { story, T0 } from "../test/story.js";

/**
 * Writes a static copy of every API answer, for the website's labelled demo mode when no API is reachable (docs/phase4.md §1).
 *   npm run fixtures -- demo https://staging-api-production-1b4b.up.railway.app
 *                                                       from a running indexer that is following the STAGING fork of Robinhood
 *                                                       Chain: real contracts and real fee arithmetic on that fork, but coins
 *                                                       and trades a simulator makes, so every price is simulated and every
 *                                                       Bitcoin value (receipt, delivery, address) is a FIXTURE
 *   npm run fixtures -- synthetic                       from test/story.ts: INVENTED events that reach every state the contracts
 *                                                       have (a rune distribution, a voided draw, a write-off, an overdue delivery…)
 * Each folder gets one file per request and an index.json mapping the request path to its file.
 */
const [which, base] = process.argv.slice(2);
if (which !== "demo" && which !== "synthetic") { console.error("usage: write-fixtures demo <api url> | synthetic"); process.exit(2); }
const dir = new URL(`../fixtures/${which}/`, import.meta.url).pathname;
mkdirSync(dir, { recursive: true });
for (const f of readdirSync(dir)) rmSync(dir + f);

/** docs/phase12.md §4: every window a reader can pick, so no range tab in a demo lands on an answer nobody recorded. */
const RANGES = ["5m", "1h", "6h", "1d", "all"] as const;
/**
 * How many coins a recording keeps, newest first (`/v1/coins` is ordered by block, descending).
 *
 * Each coin costs a detail file plus one per range, so recording the sixty coins on staging would ship three hundred files
 * inside the site. The cap is not only about bytes: two dozen coins whose cards, charts and range tabs all answer is a better
 * demo than sixty whose charts are blank, and the catalogue shows twenty-four to a page anyway.
 */
const COINS_KEPT = 24;
/** What the site asks for when it wants every coin (web/src/lib/catalog.ts `PAGE`). The kept page is filed under that name. */
const SITE_PAGE = 200;
/**
 * docs/phase13.md §6. The tape and the holder list, and the paths the site asks for them by.
 *
 * `TRADE_LIMIT` and `HOLDER_LIMIT` are the page sizes web/src/lib/coinchart.ts asks with, and they have to match to the
 * character: a fixture set is a map from a request path to a file, so a page size nobody recorded is a demo with no trades.
 *
 * Both lists are recorded **to exhaustion** — trades by following `next` into `?before=`, holders by walking `?offset=` — so
 * "Show more" in a demo runs out the way it runs out on a live indexer rather than reaching for a cursor nobody recorded.
 * That was the earlier tradeoff and it was taken the wrong way round: what it produced was a coin page with no trades on it
 * at all. The page caps below are a guard against a pathological tape, not the expected case: measured on staging on
 * 2026-09-21, 77 coins had a median of 67 trades and a maximum a little over 100, so three pages of 25 is typical. A list
 * that does hit its cap is reported on the console and in index.json rather than passed off as complete.
 */
const TRADE_LIMIT = 25, TRADE_PAGES = 10;
const HOLDER_LIMIT = 25, HOLDER_PAGES = 8;

let url = base ?? "", close = () => {};
if (which === "synthetic") {
  const db = openDb(":memory:");
  const ins = db.prepare(`INSERT INTO events (block_number, log_index, tx, address, contract, name, args, ts, listing_id, acquisition_id) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`);
  for (const e of story()) { apply(db, e); const l = lotOf(db, e); ins.run(e.block, e.logIndex, e.tx, e.address, e.contract, e.name, JSON.stringify(e.args), e.ts, l.listingId, l.acquisitionId); }
  db.prepare(`INSERT INTO meta (key, value) VALUES ('chain_id', '0'), ('demo', '1')`).run();
  // a fixed clock inside the story, so that an open window is open and the file does not change from run to run
  const server = serve({ views: new Views(db, new Live(null, null), () => T0 + 1_500), follower: () => ({ lastPassAt: T0 + 1_500, lastError: null, head: null, reorgs: 0, idle: false }),
    confirmations: 0, perMinute: 100_000, burst: 100_000 }, 0, "127.0.0.1");
  await new Promise((r) => server.once("listening", r));
  url = `http://127.0.0.1:${(server.address() as AddressInfo).port}`; close = () => server.close();
}
if (!url) { console.error("demo needs the API's url"); process.exit(2); }

const index: Record<string, string> = {};
/** A hosted indexer rate-limits (api.ts: 120 a minute), and a recording is a few hundred requests. Wait when told to. */
const ask = async (path: string): Promise<Response> => {
  for (let i = 0; ; i++) {
    const r = await fetch(url + path);
    if (r.status !== 429 || i === 5) return r;
    await new Promise((res) => setTimeout(res, Math.max(1, Number(r.headers.get("retry-after") ?? 2)) * 1_000));
  }
};
const get = async (path: string, file: string): Promise<any> => {
  const r = await ask(path);
  if (!r.ok) throw new Error(`${path}: ${r.status}`);
  const j = await r.json();
  writeFileSync(dir + file, JSON.stringify(j, null, 2) + "\n");
  index[path] = file;
  return j;
};
/** An endpoint an older indexer does not serve at all: recorded when it is there, noted when it is not, never invented. */
const optional = async (path: string, file: string): Promise<any | null> => {
  const r = await ask(path);
  if (r.status === 404) return null;
  if (!r.ok) throw new Error(`${path}: ${r.status}`);
  const j = await r.json();
  writeFileSync(dir + file, JSON.stringify(j, null, 2) + "\n");
  index[path] = file;
  return j;
};
/** Which lists ran out of pages before their cap, and which were cut off by it. index.json carries the ones that were. */
const truncated: string[] = [];
const noHolders: string[] = [];

/**
 * docs/phase13.md §6. A coin's whole tape, page by page, under the exact paths the site asks for them.
 *
 * `next` is the indexer's own cursor and goes back as `?before=`, encoded the way the browser encodes it, so the recorded
 * path and the requested path are the same string. Recording stops when `next` is null — the list genuinely ran out — or at
 * `TRADE_PAGES`, which is then said out loud.
 */
const tape = async (address: string, file: string): Promise<number> => {
  let path = `/v1/coins/${address}/trades?limit=${TRADE_LIMIT}`, n = 0;
  for (let page = 0; page < TRADE_PAGES; page++) {
    const p = await get(path, `${file}-trades${page ? `-${page}` : ""}.json`);
    n += p.items.length;
    if (!p.next) return n;
    path = `/v1/coins/${address}/trades?limit=${TRADE_LIMIT}&before=${encodeURIComponent(p.next)}`;
  }
  truncated.push(`${address} trades (more than ${TRADE_PAGES * TRADE_LIMIT})`);
  return n;
};
/** The same, by offset: a holder page whose rows run out is the last one, and the site asks for no more. */
const holders = async (address: string, file: string): Promise<number | null> => {
  let n = 0;
  for (let page = 0; page < HOLDER_PAGES; page++) {
    const off = page * HOLDER_LIMIT;
    const p = await optional(`/v1/coins/${address}/holders?limit=${HOLDER_LIMIT}${off ? `&offset=${off}` : ""}`, `${file}-holders${page ? `-${page}` : ""}.json`);
    if (p === null) { noHolders.push(address); return null; }     // an indexer older than docs/phase13.md
    n += p.items.length;
    if (off + p.items.length >= p.total) return n;
  }
  truncated.push(`${address} holders (more than ${HOLDER_PAGES * HOLDER_LIMIT})`);
  return n;
};

await get("/v1/health", "health.json"); await get("/v1/stats", "stats.json");
const coins = await get(`/v1/coins?limit=${COINS_KEPT}`, "coins.json"); index[`/v1/coins?limit=${SITE_PAGE}`] = "coins.json"; index["/v1/coins"] = "coins.json";
for (const c of coins.items) {
  const a = c.address.toLowerCase();
  await get(`/v1/coins/${c.address}`, `coin-${a}.json`);
  // the chart and the cards: a card asks for `1d`, a coin's page opens on whichever range its age can fill and a reader can
  // click any of them, so all five are recorded rather than the one the page happens to open on
  for (const range of RANGES) await get(`/v1/coins/${c.address}/candles?range=${range}`, `candles-${a}-${range}.json`);
  // docs/phase13.md §6: the two lists under the chart, both to exhaustion, so the demo's "Show more" runs out honestly
  const t = await tape(c.address, `coin-${a}`);
  const h = await holders(c.address, `coin-${a}`);
  console.log(`  ${c.symbol ?? a}: ${t} trades, ${h === null ? "holders not served by this indexer" : `${h} holders`}`);
}
const listings = await get("/v1/listings", "listings.json");
for (const l of listings.items) { await get(`/v1/listings/${l.id}`, `listing-${l.id}.json`); }
await get("/v1/acquisitions?limit=200", "acquisitions.json"); index["/v1/acquisitions"] = "acquisitions.json";
const draws = await get("/v1/draws?limit=200", "draws.json"); index["/v1/draws"] = "draws.json";
for (const d of draws.items) { await get(`/v1/draws/${d.id}`, `draw-${d.id}.json`); }
writeFileSync(dir + "index.json", JSON.stringify({ kind: which, note: which === "demo"
  ? "Recorded from an indexer following the STAGING fork of Robinhood Chain (chain 9466301). The contracts, the fee arithmetic and the draws are real on that fork; the coins, the trades and the wallets holding them are made by a simulator we run, so every price here is SIMULATED, every holder is a key that simulator holds, and every Bitcoin value (receipt, delivery, address, prices) is a FIXTURE. Nothing was bought. The dollars are a spot read of one on-chain pool at the moment of recording, not a market data feed."
  : "INVENTED events from indexer/test/story.ts, folded by the real indexer, to show every state. No chain, no `live` blocks.",
  // docs/phase13.md §6: a list whose recording stopped at its page cap rather than at its end, and a coin whose holders this
  // indexer does not serve. Both empty is what a complete recording looks like, and the site reads neither as "no trades".
  truncated, holdersUnavailable: noHolders, files: index }, null, 2) + "\n");
close();
console.log(`${which}: ${Object.keys(index).length} answers -> ${dir}`);
if (truncated.length) console.log(`  CUT OFF AT A PAGE CAP: ${truncated.join("; ")}`);
if (noHolders.length) console.log(`  holders not served for ${noHolders.length} coins: this indexer is older than docs/phase13.md`);
