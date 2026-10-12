// Asks Magic Eden the one question worth re-asking: CAN IT RANK THE COLLECTIONS' WEEK YET?
//
//     node scripts/magiceden.mjs
//
// It writes nothing. It is a prober, not a fetch: it answers `yes`, `no` or `not today` and exits 0, 1 or 2, and the
// catalogue is built by scripts/catalogue.mjs as it always was. See docs/phase9.md §11.2 for the measurement that made
// this file necessary and for why no Magic Eden adapter exists beside it.
//
// WHY THIS IS NOT AN ADAPTER. UniSat's inscription market is effectively dead -- four collections over seventy fills in
// the seven days before the first keyed run -- so the collections' week was asked of Magic Eden. Nothing there answered
// live. Every route under `api-mainnet.magiceden.dev/v2/ord/btc` returns 503 "no healthy upstream", inscriptions and
// runes alike, while the SAME HOST's Solana routes answer normally with the rate limit barely touched -- so it is their
// Bitcoin upstream that is down, not this project being throttled. Writing an adapter would mean taking field names
// from their OpenAPI document rather than from a response, and a figure nobody has seen a marketplace serve is exactly
// what §11 forbids ranking by. So: no adapter until this says yes, and then the field names come from the body.
//
// THE 200 THAT IS NOT A READ. `/v2/ord/btc/stat?collectionSymbol=bitcoin-frogs` answers 200 with a plausible body. It
// is a frozen Cloudflare stale-if-error copy -- the same `x-kong-request-id` every time, `cf-cache-status: STALE` --
// and appending `&_=1` to that exact URL turns it into a 503. Its age is unknown and unknowable from here. This script
// therefore treats a STALE or EXPIRED 200 as a CACHE GHOST and never counts it as an answer, because a ranking built on
// one would be the §11 defect wearing a 200.
//
// PACING IS ITS OWN, NOT UNISAT'S. `CATALOGUE_GAP_MS` is 260 ms because UniSat's free tier allows five calls a second.
// Magic Eden's gateway is far less forgiving and stays unhappy for minutes once it is unhappy at all, so the gap here
// is 2,000 ms and a 503 backs off from twenty seconds, doubling to a five-minute ceiling. The two constants are
// deliberately separate: one marketplace's budget must never be spent at another's rate.
//
// Reads only. Every route below is collection or market data. Nothing here places, changes or cancels an order or a
// listing, and no route that could is named anywhere in this file.

import { pathToFileURL } from "node:url";

const UA = "etched-catalogue/1 (+https://github.com/jlo110/etch)";

/** Magic Eden's own gap, which is emphatically not UniSat's 260 ms. See the header. */
export const MAGICEDEN_GAP_MS = Number(process.env.MAGICEDEN_GAP_MS ?? 2_000);
/** Where a 503 backs off from, and the ceiling it doubles to. Their gateway sulks for minutes, not seconds. */
export const BACKOFF_FROM_MS = 20_000;
export const BACKOFF_CEILING_MS = 300_000;
/** How many times one route is re-asked before it is written down as unanswered. */
export const ATTEMPTS = Number(process.env.MAGICEDEN_ATTEMPTS ?? 3);

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const log = (...a) => console.log("[magiceden]", ...a);

/**
 * The next backoff after one that did not work. Doubling, and capped, so a long outage costs a bounded wait rather
 * than an unbounded one.
 */
export const backoffAfter = (prev) =>
  Math.min(prev ? prev * 2 : BACKOFF_FROM_MS, BACKOFF_CEILING_MS);

/**
 * A 200 that Cloudflare served from a stale cache because it could not reach the origin. It is not a read: the body is
 * a copy of unknown age, and the same body comes back however long the origin has been gone.
 *
 * `STALE` is the one actually observed; `EXPIRED` and `REVALIDATED` are the neighbouring states in which the edge, not
 * the origin, decided what the body says. Anything else -- `HIT`, `MISS`, `DYNAMIC`, `BYPASS` -- reached the origin.
 */
export const GHOST_STATES = new Set(["STALE", "EXPIRED", "REVALIDATED"]);
export const isCacheGhost = (status, cacheStatus) =>
  status === 200 && GHOST_STATES.has(String(cacheStatus ?? "").trim().toUpperCase());

/**
 * What one route's answer means for the question being asked.
 *
 *  - `live`    the origin answered and the body is its own;
 *  - `ghost`   a 200 the edge invented from a stale copy (above), which is not an answer;
 *  - `busy`    503 or 429: their upstream is unreachable or we are being asked to slow down. Retry, then give up;
 *  - `refused` any other 4xx: a LIVE origin declining. `400 "Only solana is supported"` is the one that matters --
 *              it is a decision, not an outage, so waiting does not fix it.
 */
export function classify(status, cacheStatus) {
  if (isCacheGhost(status, cacheStatus)) return "ghost";
  if (status === 200) return "live";
  if (status === 503 || status === 429 || status === 0) return "busy";
  if (status >= 400 && status < 500) return "refused";
  return "busy";
}

/**
 * The routes, and which of them would actually answer the question. Only a `windowed` route can rank a week; the two
 * `stat` calls are controls -- one is the known cache ghost, the other the same route for a collection the edge has no
 * copy of, which is what proves a 503 is the upstream rather than the path.
 *
 * Every one is a collection or market read. docs/phase9.md §11.2 records what each answered when this was written.
 */
export const ROUTES = [
  { name: "collection stats, documented host", windowed: true,
    url: "https://stats-mainnet.magiceden.dev/collection_stats/search/bitcoin"
       + "?window=7d&sort=volume&direction=desc&offset=0&limit=5" },
  { name: "collection stats, surviving host", windowed: true,
    url: "https://api-mainnet.magiceden.dev/collection_stats/search/bitcoin"
       + "?window=7d&sort=volume&direction=desc&offset=0&limit=5" },
  { name: "collection stats, under the ord tree", windowed: true,
    url: "https://api-mainnet.magiceden.dev/v2/ord/btc/collection_stats/search"
       + "?window=7d&sort=volume&direction=desc&offset=0&limit=5" },
  { name: "popular collections", windowed: true,
    url: "https://api-mainnet.magiceden.dev/v2/ord/btc/popular_collections?window=7d" },
  { name: "one collection's lifetime stat (the known cache ghost)", windowed: false,
    url: "https://api-mainnet.magiceden.dev/v2/ord/btc/stat?collectionSymbol=bitcoin-frogs" },
  { name: "the same route, uncached", windowed: false,
    url: "https://api-mainnet.magiceden.dev/v2/ord/btc/stat?collectionSymbol=nodemonkes" },
];

/**
 * The verdict, from what the routes answered. Three outcomes, because two would force a wrong one:
 *
 *  - `yes`       a windowed route answered live and carried rows. Read the body, take the field names FROM IT, and
 *                only then write the adapter §11.2 declined to write blind.
 *  - `no`        no windowed route answered AND a live origin refused one. A refusal is a decision; waiting will not
 *                turn it into data, so this is not an outage to sit out.
 *  - `not today` no windowed route answered and nothing refused either: everything was busy or unreachable. An
 *                outage, so ask again later.
 */
export function verdictOf(results) {
  const windowed = results.filter((r) => r.windowed);
  if (windowed.some((r) => r.outcome === "live" && r.rows > 0)) return "yes";
  if (windowed.some((r) => r.outcome === "refused")) return "no";
  return "not today";
}

/** How many rows a windowed body carried, whichever of the shapes their schemas describe it arrived in. */
export function rowsIn(body) {
  if (Array.isArray(body)) return body.length;
  for (const key of ["collections", "runes", "results", "list", "data", "tokens"]) {
    if (Array.isArray(body?.[key])) return body[key].length;
  }
  return 0;
}

/** Every outbound call goes through here, so the gap is never forgotten and the backoff is never skipped. */
let lastCall = 0;
async function call(url) {
  const wait = MAGICEDEN_GAP_MS - (Date.now() - lastCall);
  if (wait > 0) await sleep(wait);
  lastCall = Date.now();
  try {
    const res = await fetch(url, {
      headers: { accept: "application/json", "user-agent": UA },
      signal: AbortSignal.timeout(30_000),
    });
    return { status: res.status, cacheStatus: res.headers.get("cf-cache-status"), text: await res.text() };
  } catch (err) {
    // an unresolvable host is one of the findings, not a crash: stats-mainnet.magiceden.dev has no DNS record
    return { status: 0, cacheStatus: null, text: "", error: err.message };
  }
}

/** One route, asked patiently: a busy answer is re-asked after a growing backoff before it is written down. */
async function ask(route) {
  let backoff = 0;
  for (let attempt = 1; attempt <= ATTEMPTS; attempt++) {
    const { status, cacheStatus, text, error } = await call(route.url);
    const outcome = classify(status, cacheStatus);
    if (outcome !== "busy" || attempt === ATTEMPTS) {
      let body = null;
      try { body = JSON.parse(text); } catch { /* not json, which `rowsIn` reads as no rows */ }
      return {
        ...route, status, cacheStatus, outcome,
        rows: outcome === "live" ? rowsIn(body) : 0,
        detail: error ?? text.slice(0, 160),
      };
    }
    backoff = backoffAfter(backoff);
    log(`${route.name}: ${status || error} -- waiting ${backoff / 1000}s before asking again`);
    await sleep(backoff);
  }
  throw new Error("unreachable");
}

const SAYS = {
  live: "answered", ghost: "A CACHE GHOST, not an answer", busy: "not answering", refused: "refused",
};

async function main() {
  log(`asking ${ROUTES.length} routes, ${MAGICEDEN_GAP_MS} ms apart, backing off from `
    + `${BACKOFF_FROM_MS / 1000}s to ${BACKOFF_CEILING_MS / 1000}s. Reads only.`);

  const results = [];
  for (const route of ROUTES) {
    const r = await ask(route);
    results.push(r);
    log(`${r.windowed ? "[week]" : "[    ]"} ${r.name}: ${r.status || "unreachable"} `
      + `${r.cacheStatus ? `(cf ${r.cacheStatus}) ` : ""}-- ${SAYS[r.outcome]}`
      + `${r.outcome === "live" ? `, ${r.rows} rows` : ""}`);
    if (r.detail && r.outcome !== "live") log(`        ${r.detail}`);
  }

  const verdict = verdictOf(results);
  console.log("");
  if (verdict === "yes") {
    const won = results.find((r) => r.windowed && r.outcome === "live" && r.rows > 0);
    log(`YES -- "${won.name}" answered with ${won.rows} rows.`);
    log(`  ${won.url}`);
    log("  Read that body, take the volume field's REAL name from it, confirm the unit is sats against a floor you");
    log("  can check, and only then write the adapter. docs/phase9.md §11.2 declined to write it blind.");
    process.exit(0);
  }
  if (verdict === "no") {
    log("NO -- a live origin refused a windowed route. That is a decision, not an outage: waiting will not turn it");
    log("  into data. The collections still have no weekly source and the catalogue must not invent one.");
    process.exit(1);
  }
  log("NOT TODAY -- nothing answered and nothing refused: Magic Eden's Bitcoin upstream is unreachable from here.");
  log("  An outage, so this is worth asking again later. Nothing about the catalogue changes meanwhile.");
  process.exit(2);
}

// Only when run, so the pacing and the verdict above can be imported and read by a test without a network call.
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((err) => {
    console.error("[magiceden] failed:", err);
    process.exit(2);
  });
}
