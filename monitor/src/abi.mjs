// The 4663 contracts as the monitor reads them (src/ListingRegistry.sol, src/floor/FloorModule.sol, src/SweepVault.sol,
// src/Wrapper.sol, src/curve/CurveFactory.sol), and plain JSON-RPC. Reads only, except `sendRaw`, which only the guardian
// (src/guardian.mjs) calls, and only with bytes it signed itself.
import { decodeEventLog, decodeFunctionResult, encodeFunctionData, parseAbi, toEventSelector } from "viem";

export const REGISTRY = parseAbi([
  "function keeperPaused() view returns (bool)",
  "function guardian() view returns (address)",
  "function owner() view returns (address)",
  "function namedKeeper() view returns (address)",
  // r4-contracts A3-L1: the last unpause, both clocks; unfilled curves' refunds stay held RESUME_GRACE (6 h) after it
  "function keeperResumedAt() view returns (uint64)",
  "function keeperResumedAtBlock() view returns (uint64)",
  "function pauseKeeper()",
  "event KeeperPaused(address indexed by, bool paused)",
  "event GuardianSet(address guardian)",
  "event RolesProposed(address keeper, address purchaser, address attestor, uint64 readyAt)",
  "event RolesSet(address keeper, address purchaser, address attestor)",
]);
export const MODULE = parseAbi([
  "function fresh() view returns (bool)",
  "function escaped() view returns (bool)",
  "function accepted() view returns (uint256)",
  "function postedAt() view returns (uint64)",
  "function lastGoodAt() view returns (uint64)",
  "function minConfidenceBps() view returns (uint256)",
  "function maxMoveBps() view returns (uint16)",
  // r3-contracts N-1b: true while the keeper is paused (or zero) or the module is escaped: burns pay the backing per unit less the
  // base spread, at once; mints and every keeper-priced inflow stay refused
  "function exitAtBacking() view returns (bool)",
  // r4-contracts A3-M1: while the keeper is paused (not escaped) a burn pays min(backing, pauseFloor) less the base spread;
  // exitAtBacking() then means escaped() alone. Read tolerantly (a module from before R4 has neither)
  "function pauseFloor() view returns (uint256)",
  "function burnsOpen() view returns (bool)",
  "function RESUME_GRACE() view returns (uint256)",
  "event FloorPosted(uint256 target, uint256 accepted, uint256 moveBps, uint16 confidenceBps, uint32 depthItems)",
]);
export const DRAW_TUPLE = "(uint8 stage, address coin, uint64 challengeEnds, uint64 challengeEndsBlock, uint64 round, uint64 pinnedAt, uint64 pinnedAtBlock, uint64 resolvedAt, uint64 resolvedAtBlock, bytes32 root, uint256 totalTickets, uint256 winningTicket, address winner, string uri, uint64 seedRound, uint64 announcedAt, uint64 announcedAtBlock, uint64 snapshotAt)";
export const VAULT = parseAbi([
  "function capFresh() view returns (bool)",
  "function cap() view returns (uint256)",
  "function drawCount() view returns (uint256)",
  `function draw(uint256 id) view returns (${DRAW_TUPLE})`,
  "function cancelDraw(uint256 id)",
  // review2 N-2 (r3-contracts): a draw ended before its pin leaves its seed round and snapshot window to the coin's next draw
  "function seedOrigin(uint256 id) view returns (uint256)",
  "function carriedSeed(address coin) view returns (uint256)",
  "event CapPosted(uint256 weiPerWrapped, uint64 at)",
  "event DrawAnnounced(uint256 indexed drawId, address indexed coin, uint64 seedRound)",
  "event SnapshotFixed(uint256 indexed drawId, uint64 snapshotAt)",
  "event DrawOpened(uint256 indexed drawId, address indexed coin, bytes32 root, uint256 totalTickets, string uri, uint64 challengeEnds)",
  "event DrawCancelled(uint256 indexed drawId)",
  "event DrawVoided(uint256 indexed drawId)",
  "event SeedCarried(uint256 indexed drawId, uint256 indexed originId, uint64 seedRound)",
  // fixes-R3-carry: an Open draw with no tickets (the sentinel alone), closed by requestDraw with no winner
  "event DrawEmpty(uint256 indexed drawId, address indexed coin, uint64 snapshotAt)",
]);
export const WRAPPER = parseAbi([
  "function nextToResolve() view returns (uint256)",
  "function ticketCount() view returns (uint256)",
  "function ticket(uint256 id) view returns ((uint8 stage, address owner, uint64 round, uint64 pinnedAt, uint64 pinnedAtBlock, uint64 deliverBy, uint64 deliverByBlock, uint256 acquisitionId, bytes32 deliveryTxid, string destination))",
]);
export const FACTORY = parseAbi(["function routerOf(address coin) view returns (address)"]);
/** A coin, as the holder index's reconciliation reads it (fixes-R4-services.md §4, L-F). */
export const ERC20 = parseAbi(["function totalSupply() view returns (uint256)", "function balanceOf(address) view returns (uint256)"]);

/** SweepVault.Stage and Wrapper.Stage, by their enum order (`Empty`, fixes-R3-carry: closed with no winner, final). */
export const DRAW_STAGE = ["None", "Open", "Pinned", "Resolved", "Claimed", "Cancelled", "Voided", "Announced", "Empty"];
export const TICKET_STAGE = ["None", "Pinned", "Resolved", "Delivered", "Refunded"];

/** drand evmnet (src/rand/DrandEvmnet.sol): the moment a round's signature exists. */
export const DRAND = { genesis: 1_727_521_075, period: 3 };
export const roundTime = (round) => DRAND.genesis + (Number(round) - 1) * DRAND.period;

const ALL = [...REGISTRY, ...MODULE, ...VAULT].filter((x) => x.type === "event");
export const EVENT_TOPICS = Object.fromEntries(ALL.map((e) => [e.name, toEventSelector(e)]));
const BY_TOPIC = new Map(ALL.map((e) => [toEventSelector(e), e]));

/** A raw log of one of the events above, decoded: { name, args } (bigints), or null for any other log. */
export function decodeStackEvent(log) {
  const e = BY_TOPIC.get(String(log.topics?.[0] ?? "").toLowerCase());
  if (!e) return null;
  try { return decodeEventLog({ abi: [e], data: log.data, topics: log.topics }); } catch { return null; }
}

export const host = (u) => { try { return new URL(u).host; } catch { return String(u); } };

/**
 * A retryable answer: throttling and transient failures (HTTP 429 and 5xx, JSON-RPC -32005 "busy" / -32016 / -32603 on a
 * timeout, a fetch timeout or reset). Anything else (a revert, a bad request, a range the provider refuses) is not. The HTTP status is
 * matched only as "HTTP 5xx" (R4: a provider's own text now follows it, and "max 500 blocks" is a range answer, not a throttle).
 */
export const retryable = (err) => /HTTP (429|5\d\d)\b|^(429|5\d\d)$|-3200[5]|-32016|busy|rate.?limit|too many requests|timeout|timed out|ECONNRESET|socket hang up|fetch failed/i.test(String(err?.message ?? err));

/**
 * R3-check M-A: an answer about the query's SIZE (its block range, its number of results or addresses, the response's bytes), the
 * only kind that may narrow a provider's page span. Robinhood: "query exceeds limit of 10000"; drpc: "ranges over 10000 blocks are
 * not supported"; geth / others: "query returned more than 10000 results", "block range is too wide", "response size exceeded",
 * HTTP 413. A throttle ("rate limit", "too many requests") is `retryable`, and is checked first by every caller.
 */
export const rangeError = (err) => {
  const m = String(err?.message ?? err);
  if (retryable(m)) return false;
  return /exceed|too many (?!requests)|\brange|\blimit|more than \d+|too (large|big|wide)|result window|response size|\b413\b/i.test(m);
};

/** How often and how long `rpc` retries a retryable failure (R2 I-1). Tests set `baseMs` to 0. */
export const retryPolicy = { tries: 3, baseMs: 500 };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

export async function rpc(url, method, params, http = fetch, { tries = retryPolicy.tries, timeoutMs = 30_000 } = {}) {
  for (let i = 1; ; i++) {
    try { return await rpcOnce(url, method, params, http, timeoutMs); }
    catch (err) {
      if (i >= tries || !retryable(err)) throw err;
      await sleep(retryPolicy.baseMs * 2 ** (i - 1));
    }
  }
}

async function rpcOnce(url, method, params, http, timeoutMs = 30_000) {
  const r = await http(url, { method: "POST", headers: { "content-type": "application/json" },
    body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params }), signal: AbortSignal.timeout(timeoutMs) });
  if (!r.ok) {
    // the provider's own words (a "range too large" sent with HTTP 400 is then recognised by rangeError); provider text, capped
    const said = typeof r.text === "function" ? await r.text().then((t) => String(t).replace(/\s+/g, " ").slice(0, 200), () => "") : "";
    throw new Error(`${host(url)} answered HTTP ${r.status}${said ? `: ${said}` : ""}`);
  }
  const j = await r.json();
  if (j.error) throw new Error(`${host(url)}: ${j.error.code !== undefined ? `${j.error.code} ` : ""}${j.error.message ?? JSON.stringify(j.error)}`);
  return j.result;
}

/** One view call, decoded. `block`: a number, or "latest". */
export async function read(url, abi, to, functionName, args = [], { http = fetch, block = "latest", timeoutMs, tries } = {}) {
  const data = encodeFunctionData({ abi, functionName, args });
  const tag = typeof block === "number" ? `0x${block.toString(16)}` : block;
  const out = await rpc(url, "eth_call", [{ to, data }, tag], http, { ...(timeoutMs === undefined ? {} : { timeoutMs }), ...(tries === undefined ? {} : { tries }) });
  return decodeFunctionResult({ abi, functionName, data: out });
}

export const lc = (a) => String(a ?? "").toLowerCase();
