// The holder index (docs/audit/2026-09-26/review2/fixes-R3-services.md §1.1, R2-M1). A draw's root used to be recomputed by
// re-reading the coin's whole Transfer history from the stack's first block, on both RPCs, every poll: past a day or two of
// stack life that outlasted the 15-minute challenge window, and on a provider that throttles eth_getLogs it never finished.
//
// Now each provider keeps its own index, read from that provider alone (so the providers stay independent of each other and of
// the keeper): every RAFFLE coin the stack's factories launch (CurveFactory.Launched, policy 0), plus any coin a draw names; per
// coin a cursor (the last block read), the balances at a base block and the Transfer logs after it. A poll reads only the new
// blocks, in pages this provider accepts; logs older than the retention are folded into the base, so the file stays small. A
// root is then a local replay at the snapshot block. The index is saved to its own file (atomically), not the per-poll state.
import { existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { dirname } from "node:path";
import { keccak_256 } from "@noble/hashes/sha3";
import { ERC20, host, rangeError, read, retryPolicy, retryable, rpc } from "./abi.mjs";
import { blockAtOrBefore, weightedTree } from "./draws.mjs";

const ZERO = "0x0000000000000000000000000000000000000000";
const topicOf = (sig) => `0x${Buffer.from(keccak_256(new TextEncoder().encode(sig))).toString("hex")}`;
export const TRANSFER = topicOf("Transfer(address,address,uint256)");
/** CurveFactory.Launched(address indexed coin, address indexed curve, address indexed creator, SweepVault.Policy policy, …). */
export const LAUNCHED = topicOf("Launched(address,address,address,uint8,uint256,bool,bool,bool)");
export const RAFFLE = 0;
/** SNAPSHOT_WINDOW (24 h) + OPEN_TTL (6 h) + 6 h of margin, at up to 12 blocks a second (4663 makes ~9.9). */
export const RETENTION_BLOCKS = 36 * 3600 * 12;
export const ADDRESS_BATCH = 200;
export const COOL_MS = 2 * 60 * 1000;
export const COOL_MAX_MS = 30 * 60 * 1000;
/** L-F (fixes-R4-services.md §4): a coin rebuilt on missing logs may be rebuilt again this long after; a broken coin is retried after this. */
export const RESET_TTL_MS = 6 * 3600 * 1000;
export const BROKEN_RETRY_MS = 3600 * 1000;
/** L-F: reconciliation of an index against the chain's state (totalSupply, the largest balances), at the index's cursor. */
export const RECONCILE = { perPoll: 2, everyMs: 30 * 60 * 1000, maxAgeBlocks: 3_000, top: 16, rebuildEveryMs: 3600 * 1000 };

const lc = (a) => String(a ?? "").toLowerCase();
const num = (h) => Number(BigInt(h));
const addrOf = (t) => `0x${t.slice(26)}`.toLowerCase();

/** R3-check M-A: how a provider's page span learns and recovers (fixes-R4-services.md §1). Tests may change these. */
export const SPAN = { pageTries: 3, failsToCool: 3, growAfter: 8, ceilingTtlMs: 10 * 60 * 1000, maxTxReceipts: 300 };

/**
 * One provider: its URL, the page span it serves, and whether it is cooling down after throttling. The span narrows ONLY on an
 * answer about the query's size (`rangeError`), which also sets a ceiling; the ceiling grows back after successes at it and
 * resets to the configured span once the refusal that set it is `ceilingTtlMs` old. Any other error leaves both alone.
 */
export class Provider {
  constructor({ url, span = 10_000 }) {
    this.url = url; this.host = host(url);
    this.span = span; this.start = span; this.ceiling = span;
    this.coolUntil = 0; this.coolMs = 0; this.lastError = null;
    this.refusedAt = 0; this.streak = 0; this.fails = 0; this.growNeed = SPAN.growAfter; this.grewTo = 0;
  }
  cooling(now) { return now < this.coolUntil; }
  throttled(now, err) {
    this.coolMs = Math.min(COOL_MAX_MS, this.coolMs ? this.coolMs * 2 : COOL_MS);
    this.coolUntil = now + this.coolMs; this.lastError = String(err?.message ?? err).slice(0, 200);
  }
  /** A page refused for its size: the next page is half as wide, and no page wider than the refused one is asked for a while. */
  refused(now, width) {
    // a ceiling grown back and refused again at once: the provider's limit is real, so probe it half as often
    if (this.grewTo && width >= this.grewTo) this.growNeed = Math.min(512, this.growNeed * 2);
    this.grewTo = 0;
    this.ceiling = Math.max(1, Math.min(this.ceiling, Math.floor(width / 2)));
    this.span = this.ceiling;
    this.refusedAt = now; this.streak = 0;
  }
  /** A fresh window: a ceiling learned more than `ceilingTtlMs` ago is forgotten. */
  refresh(now) { if (this.ceiling < this.start && now - this.refusedAt >= SPAN.ceilingTtlMs) { this.ceiling = this.start; this.streak = 0; this.grewTo = 0; this.growNeed = SPAN.growAfter; } }
  /** A page that failed for another reason (not its size, not a throttle): counted; `failsToCool` in a row cool the provider. */
  failed(now, err) {
    this.lastError = String(err?.message ?? err).slice(0, 200);
    if (++this.fails >= SPAN.failsToCool) { this.fails = 0; this.throttled(now, err); }
  }
  ok() {
    this.coolMs = 0; this.lastError = null; this.fails = 0;
    if (this.span >= this.ceiling && this.ceiling < this.start && ++this.streak >= this.growNeed) { this.ceiling = Math.min(this.start, this.ceiling * 2); this.grewTo = this.ceiling; this.streak = 0; }
    this.span = Math.min(this.ceiling, this.span * 2);
  }
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const hexN = (n) => `0x${n.toString(16)}`;

/**
 * R3-check M-B: one block whose logs a provider refuses to return in one query (more than its per-query cap). Read another way,
 * filtered to `filter`'s addresses and first topics: the block's receipts (one call); else the query split by address, each for
 * this block alone; else, for an address still refused alone, the block's transactions' receipts one by one. Returns { logs,
 * stuck: [addresses no way could read], why }.
 *
 * R4-check L-S3 (fixes-golive-lows.md §5): the block is known to hold logs of the filter's addresses (the provider just refused them
 * for their number), so an answer that says "nothing here" is a failure, never an empty block: receipts that are empty, for another
 * block, or hold no log of the filter; a null header, one with no `transactions` or an empty list; a missing receipt. Any failure of
 * one way (a 5xx or a timeout included) goes on to the next; an address that no way can read is `stuck` at this block (the holder
 * index keeps it behind there, pages it and retries it; the rest of the group goes on). Only the poll's time budget throws.
 */
async function denseBlock(p, filter, x, { http, deadline, clock }) {
  const addrs = [].concat(filter.address ?? []).map(lc);
  const t0 = filter.topics?.[0] ? [].concat(filter.topics[0]).map(lc) : null;
  const want = (set) => (l) => (!set.length || set.includes(lc(l.address))) && (!t0 || t0.includes(lc(l.topics?.[0])));
  const opts = () => ({ timeoutMs: Math.max(2_000, Math.min(30_000, deadline - clock())), tries: 1 });
  const budget = () => { if (clock() >= deadline) throw new Error("the poll's time budget ran out"); };
  const why = [];
  const at = (v) => { try { return v === undefined || v === null ? null : Number(BigInt(v)); } catch { return NaN; } };
  /** The logs of a receipts answer for block `x`, or a reason it is not one. */
  const receiptsLogs = (rs) => {
    if (!Array.isArray(rs)) return { bad: `the receipts answer is ${rs === null ? "null" : typeof rs}, not a list` };
    if (!rs.length) return { bad: "the receipts answer is empty for a block with logs" };
    const hashes = new Set();
    const out = [];
    for (const r of rs) {
      if (!r || typeof r !== "object" || !Array.isArray(r.logs)) return { bad: "a receipt without logs" };
      if (at(r.blockNumber) !== x) return { bad: `a receipt for block ${r.blockNumber ?? "(none)"}, not ${x}` };
      if (r.blockHash) hashes.add(lc(r.blockHash));
      for (const l of r.logs) { if (l?.blockNumber !== undefined && at(l.blockNumber) !== x) return { bad: `a log for block ${l.blockNumber}, not ${x}` }; out.push(l); }
    }
    if (hashes.size > 1) return { bad: "receipts from more than one block hash" };
    return { logs: out };
  };
  budget();
  try {
    const got = receiptsLogs(await rpc(p.url, "eth_getBlockReceipts", [hexN(x)], http, opts()));
    if (got.bad) why.push(`eth_getBlockReceipts: ${got.bad}`);
    else {
      const mine = got.logs.filter(want(addrs));
      if (addrs.length && !mine.length) why.push("eth_getBlockReceipts: no log of the filter's addresses in a block refused for their number");
      else return { logs: mine, stuck: [], why: null, how: "the block's receipts" };
    }
  } catch (err) {
    budget();
    why.push(`eth_getBlockReceipts: ${String(err?.message ?? err).slice(0, 120)}`);   // refused, unsupported, 5xx or timed out: the next way
  }
  let txs = null;
  const perTx = async (set) => {
    budget();
    // no address to leave behind (a filter by topic alone): the read stops before the block instead, never skips it
    const stuck = (w) => { why.push(w); if (!set.length) throw new Error(why.join("; ").slice(0, 300)); return { logs: [], stuck: set }; };
    if (txs === null) {
      let h;
      try { h = await rpc(p.url, "eth_getBlockByNumber", [hexN(x), false], http, opts()); }
      catch (err) { budget(); return stuck(`the block's header: ${String(err?.message ?? err).slice(0, 120)}`); }
      if (!h || typeof h !== "object") return stuck(`the block's header is ${h === null ? "null" : typeof h}`);
      if (!Array.isArray(h.transactions)) return stuck("the block's header has no transaction list");
      if (h.number !== undefined && at(h.number) !== x) return stuck(`the header is block ${h.number}, not ${x}`);
      if (!h.transactions.length) return stuck("the header lists no transaction in a block with logs");
      txs = h.transactions;
    }
    if (txs.length > SPAN.maxTxReceipts) return stuck(`${txs.length} transactions, more than ${SPAN.maxTxReceipts} receipts a block`);
    const out = [];
    for (const t of txs) {
      budget();
      let r;
      try { r = await rpc(p.url, "eth_getTransactionReceipt", [typeof t === "string" ? t : t?.hash], http, opts()); }
      catch (err) { budget(); return stuck(`a transaction's receipt: ${String(err?.message ?? err).slice(0, 120)}`); }
      if (!r || !Array.isArray(r.logs)) return stuck("a transaction's receipt is missing");
      if (at(r.blockNumber) !== x) return stuck(`a transaction's receipt is for block ${r.blockNumber ?? "(none)"}, not ${x}`);
      out.push(...r.logs.filter(want(set)));
    }
    if (set.length && !out.length) return stuck("the transactions' receipts hold no log of an address refused for its number of logs");
    return { logs: out, stuck: [] };
  };
  const split = async (set) => {
    if (set.length <= 1) return perTx(set);
    const out = { logs: [], stuck: [] };
    for (const part of [set.slice(0, Math.ceil(set.length / 2)), set.slice(Math.ceil(set.length / 2))]) {
      budget();
      let got;
      try {
        got = await rpc(p.url, "eth_getLogs", [{ ...filter, address: part.length === 1 ? part[0] : part, fromBlock: hexN(x), toBlock: hexN(x) }], http, opts());
      } catch (err) {
        budget();
        if (!rangeError(err)) { why.push(`eth_getLogs for ${part.length} address(es): ${String(err?.message ?? err).slice(0, 120)}`); out.stuck.push(...part); continue; }
        const r = await split(part);
        out.logs.push(...r.logs); out.stuck.push(...r.stuck);
        continue;
      }
      if (!Array.isArray(got)) { why.push(`eth_getLogs for ${part.length} address(es) answered ${got === null ? "null" : typeof got}`); out.stuck.push(...part); continue; }
      out.logs.push(...got);
    }
    return out;
  };
  const r = addrs.length ? await split(addrs) : await perTx([]);
  return { ...r, why: r.stuck.length ? why.join("; ").slice(0, 300) : null, how: "the query split by address, then the transactions' receipts" };
}

/**
 * Logs for `filter` ({ address, topics }) over [from, to], paged for this provider, until `deadline` (ms since epoch).
 * Returns { logs, upTo, error, stuck }: `upTo` is the last block fully read (from − 1 if none) for every address not in `stuck`
 * (address → the block it could not be read at). A page refused for its size is asked again narrower (M-A: only such an answer
 * narrows the span, and the span recovers); one block refused for its size is read another way (M-B, `denseBlock`); an address no
 * way can read at that block is `stuck` there when `allowStuck` (the holder index: the others go on), else the read stops before
 * the block (the canary, which may skip nothing). Any other error is retried (`pageTries`) and then ends the read, the cursor
 * before it. A throttle (rpc() already retried it with backoff) cools the provider down and ends the read. Nothing is skipped.
 */
export async function pagedLogs(p, filter, from, to, { http = fetch, deadline = Infinity, clock = Date.now, allowStuck = false } = {}) {
  const logs = [], stuck = new Map(), stuckWhy = new Map();
  let a = from, tries = 0, f = filter;
  const out = (upTo, error) => ({ logs, upTo, error, stuck, stuckWhy });
  while (a <= to) {
    if (clock() >= deadline) return out(a - 1, "the poll's time budget ran out");
    if (p.cooling(clock())) return out(a - 1, `${p.host} is cooling down after throttling (${p.lastError})`);
    p.refresh(clock());
    const b = Math.min(to, a + p.span - 1);
    let page;
    try {
      // a call never runs far past the poll's budget: its timeout is what is left (2 s at least), and it is retried only with time to spare
      const left = deadline - clock();
      page = await rpc(p.url, "eth_getLogs", [{ ...f, fromBlock: hexN(a), toBlock: hexN(b) }], http,
        { timeoutMs: Math.max(2_000, Math.min(30_000, left)), tries: left > 10_000 ? undefined : 1 });
    } catch (err) {
      if (retryable(err)) { p.throttled(clock(), err); return out(a - 1, String(err.message).slice(0, 200)); }
      if (rangeError(err)) {
        tries = 0;
        // a filter with more addresses than the provider accepts: narrower pages would not help; the caller asks with fewer
        if (/address/i.test(String(err.message)) && [].concat(f.address ?? []).length > 1) return { ...out(a - 1, String(err.message).slice(0, 200)), tooManyAddresses: true };
        if (b > a) { p.refused(clock(), b - a + 1); continue; }
        // M-B: this one block has more logs than the provider returns to one query
        let d;
        // L-S3: only the poll's time budget throws here; every other failure leaves just the unreadable addresses behind
        try { d = await denseBlock(p, f, a, { http, deadline, clock }); }
        catch (e2) { p.lastError = String(e2.message).slice(0, 200); return out(a - 1, `block ${a} (dense): ${String(e2.message).slice(0, 180)}`); }
        for (const l of d.logs) if (!l.removed) logs.push(l);
        if (d.stuck.length) {
          if (!allowStuck) return out(a - 1, `block ${a} holds more logs than ${p.host} returns, and ${d.stuck.length} address(es) could not be read another way (${d.why})`);
          p.lastError = `block ${a}: ${d.why}`.slice(0, 200);
          for (const s of d.stuck) { stuck.set(lc(s), a); stuckWhy.set(lc(s), d.why); }
          const left = [].concat(f.address ?? []).map(lc).filter((x) => !stuck.has(x));
          if (!left.length) return out(to, null);
          f = { ...f, address: left };
        }
        p.lastError = null; a += 1;
        continue;
      }
      if (++tries < SPAN.pageTries) { await sleep(retryPolicy.baseMs * 2 ** (tries - 1)); continue; }
      p.failed(clock(), err);
      return out(a - 1, String(err.message).slice(0, 200));
    }
    tries = 0;
    p.ok();
    for (const l of page ?? []) if (!l.removed) logs.push(l);
    a = b + 1;
  }
  return out(to, null);
}

const EMPTY = () => ({ v: 1, group: null, coins: {} });

/**
 * One provider's index. `factories`: the stack's CurveFactory addresses; `fromBlock`: the stack's L2 start block (the group's
 * first block). `file`: where it is kept (null: memory only, for tests).
 */
export class HolderIndex {
  constructor({ url, span = 10_000, file = null, factories = [], fromBlock, confirmations = 20, retentionBlocks = RETENTION_BLOCKS, http = fetch, clock = Date.now }) {
    if (!Number.isSafeInteger(fromBlock)) throw new Error("the holder index needs the stack's start block (the stack file's fromBlock)");
    this.p = new Provider({ url, span });
    this.url = url; this.file = file; this.factories = factories.map(lc); this.fromBlock = fromBlock;
    this.confirmations = confirmations; this.retention = retentionBlocks; this.http = http; this.clock = clock;
    this.s = EMPTY();
    if (file && existsSync(file)) {
      try { const j = JSON.parse(readFileSync(file, "utf8")); if (j.v === 1 && j.url === url) this.s = j; } catch { /* a broken file is rebuilt */ }
    }
    if (this.s.fromBlock !== undefined && this.s.fromBlock !== fromBlock) this.s = EMPTY();     // another stack: start again
    this.s.url = url; this.s.fromBlock = fromBlock;
    this.s.group ??= { cursor: fromBlock - 1 };
    this.batch = ADDRESS_BATCH;              // addresses a filter, halved while the provider refuses that many (M-B)
    this.blocks = new Map();                 // snapshotAt → block, per this provider
    this.holds = new Map();                  // coin → the first block a draw's snapshot may still need (review2 N-2 carry)
    this.dirty = false;
  }

  /**
   * Integration R3 (review2 N-2): a draw voided or cancelled before its pin leaves its seed, and its snapshot window, to the coin's
   * next draw, which may be opened days later. The retention alone (36 h) would then have folded the snapshot's block into the
   * base, and the draw could never be verified (the deadline would pause the keeper). `holds`: coin → the first block of the
   * carried window; compaction never folds a held coin past it, and a coin already folded past it is rebuilt from its launch.
   */
  setHolds(holds) {
    this.holds = new Map(Object.entries(holds ?? {}).map(([k, b]) => [lc(k), b]));
    for (const [k, b] of this.holds) {
      const c = this.s.coins[k];
      if (c && !c.broken && b <= c.baseBlock && c.baseBlock >= c.launchBlock) {
        this.s.coins[k] = { ...c, cursor: c.launchBlock - 1, baseBlock: c.launchBlock - 1, base: {}, logs: [], rebuiltForHold: b };
        this.dirty = true;
      }
    }
  }

  /** What /status shows: no balances, just where each coin stands. */
  summary() {
    const coins = Object.entries(this.s.coins);
    return { provider: this.p.host, group: this.s.group.cursor, coins: coins.length, behind: coins.filter(([, c]) => c.cursor < this.s.group.cursor).length,
      broken: coins.filter(([, c]) => c.broken).map(([k]) => k),
      stuck: coins.filter(([, c]) => c.stuck).map(([k, c]) => ({ coin: k, block: c.stuck.block, since: c.stuck.since, error: c.stuck.error })),
      mismatched: coins.filter(([, c]) => c.reconcile?.v === "mismatch").map(([k, c]) => ({ coin: k, at: c.reconcile.at, block: c.reconcile.block, why: c.reconcile.why })),
      span: this.p.span, ceiling: this.p.ceiling, cooling: this.p.cooling(this.clock()) ? this.p.lastError : null, lastError: this.p.lastError };
  }

  /**
   * L-F (fixes-R4-services.md §3.2): the coin's index is re-read from its launch, once per `tag` (a draw this provider disagreed
   * about). Returns true when a rebuild starts. What the index knew is dropped, never merged: the rebuilt coin is behind until
   * caught up, and its roots then carry `repairTag`.
   */
  repair(coin, tag) {
    const k = lc(coin), c = this.s.coins[k];
    if (!c || c.repairTag === tag) return false;
    this.s.coins[k] = { ...this.#fresh(c), repairTag: tag, repairedAt: this.clock() };
    this.dirty = true;
    return true;
  }

  /** The coin as it is at its launch: nothing read, nothing folded; stuck and broken cleared (the rebuild reads it all again). */
  #fresh(c) {
    const { stuck: _s, broken: _b, brokenAt: _ba, ...rest } = c;
    return { ...rest, cursor: c.launchBlock - 1, baseBlock: c.launchBlock - 1, base: {}, logs: [] };
  }

  has(coin) { return !!this.s.coins[lc(coin)]; }

  /** A coin to index. `launchBlock`: from its Launched event; unknown (a coin a draw names that no factory launched): the stack's start. */
  addCoin(coin, { launchBlock = this.fromBlock, curve = null } = {}) {
    const k = lc(coin);
    if (this.s.coins[k]) return false;
    this.s.coins[k] = { curve: curve && lc(curve), launchBlock, cursor: launchBlock - 1, baseBlock: launchBlock - 1, base: {}, logs: [] };
    this.dirty = true;
    return true;
  }

  #append(coin, logs) {
    const c = this.s.coins[coin];
    for (const l of logs) {
      if (l.topics?.length < 3 || lc(l.topics[0]) !== TRANSFER) continue;
      c.logs.push([num(l.blockNumber), num(l.logIndex), addrOf(l.topics[1]), addrOf(l.topics[2]), BigInt(l.data === "0x" ? "0x0" : l.data).toString()]);
    }
  }

  /** Launched events in `logs` from a factory: RAFFLE coins join the index from their launch block. */
  #discover(logs) {
    for (const l of logs) {
      if (!this.factories.includes(lc(l.address)) || lc(l.topics?.[0]) !== LAUNCHED) continue;
      const policy = Number(BigInt(`0x${l.data.slice(2, 66) || "0"}`));
      if (policy === RAFFLE) this.addCoin(addrOf(l.topics[1]), { launchBlock: num(l.blockNumber), curve: addrOf(l.topics[2]) });
    }
  }

  /** One coin behind the group, read on its own up to the group's cursor (a dense block read another way, or the coin stuck at it). */
  async #catchUp(coin, deadline) {
    const c = this.s.coins[coin], to = this.s.group.cursor;
    if (c.cursor >= to) return;
    const r = await pagedLogs(this.p, { address: coin, topics: [TRANSFER] }, c.cursor + 1, to, { http: this.http, deadline, clock: this.clock, allowStuck: true });
    const x = r.stuck.get(coin);
    const upTo = x !== undefined ? x - 1 : r.upTo;
    this.#append(coin, r.logs.filter((l) => num(l.blockNumber) <= upTo));
    if (upTo > c.cursor) { c.cursor = upTo; this.dirty = true; }
    this.#mark(coin, x, r.stuckWhy.get(coin) ?? r.error);
  }

  /** M-B: a coin no way could read at block `x` stays there (cursor x − 1) and is caught up alone; a coin read past it is cleared. */
  #mark(coin, x, error) {
    const c = this.s.coins[coin];
    if (x !== undefined) {
      if (c.stuck?.block !== x) { c.stuck = { block: x, since: this.clock(), error: `block ${x} holds more logs than ${this.p.host} returns to one query, and neither its receipts nor a narrower query could be read${error ? ` (${String(error).slice(0, 200)})` : ""}` }; this.dirty = true; }
    } else if (c.stuck && c.cursor >= c.stuck.block) { delete c.stuck; this.dirty = true; }
  }

  /**
   * Bring the index forward, until `deadline`: the coins in `priority` (a pending draw's, earliest window first) that are behind,
   * then every caught-up coin and the factories together up to the safe head, then the rest that are behind. Never throws on a
   * provider's refusal: it reports it. Returns { head, safe, group, behind, error }.
   */
  async update({ deadline = this.clock() + 25_000, priority = [] } = {}) {
    let head, error = null;
    try { head = num(await rpc(this.url, "eth_blockNumber", [], this.http)); }
    catch (err) { if (retryable(err)) this.p.throttled(this.clock(), err); return { head: null, safe: null, group: this.s.group.cursor, behind: this.behind(), error: String(err.message).slice(0, 200) }; }
    const safe = head - this.confirmations;
    for (const coin of priority.map(lc)) if (this.s.coins[coin]) await this.#catchUp(coin, deadline);
    // the group: the factories and every coin at the group's cursor, read together. M-B: one coin whose block no way can read is
    // left behind at that block (and caught up alone); it never holds the others. A factory that cannot be read holds the group
    // (a coin launched in that block could be missed).
    while (this.s.group.cursor < safe && this.clock() < deadline && !this.p.cooling(this.clock())) {
      const g = this.s.group.cursor;
      const members = Object.entries(this.s.coins).filter(([, c]) => c.cursor === g).map(([k]) => k);
      const addrs = [...this.factories, ...members];
      if (!addrs.length) { this.s.group.cursor = safe; this.dirty = true; break; }
      // one span for every batch of addresses, so the batches end at the same block
      const to = Math.min(safe, g + this.p.span * 4);
      let upTo = to; const got = [], stuck = new Map(), stuckWhy = new Map();
      for (let i = 0; i < addrs.length; i += this.batch) {
        const r = await pagedLogs(this.p, { address: addrs.slice(i, i + this.batch), topics: [[TRANSFER, LAUNCHED]] }, g + 1, upTo, { http: this.http, deadline, clock: this.clock, allowStuck: true });
        // the provider refuses this many addresses in one filter: every batch again, with half as many
        if (r.tooManyAddresses && this.batch > 1) { this.batch = Math.max(1, Math.floor(this.batch / 2)); got.length = 0; stuck.clear(); stuckWhy.clear(); upTo = to; i = -this.batch; continue; }
        got.push(...r.logs); upTo = Math.min(upTo, r.upTo);
        for (const [a, x] of r.stuck) { stuck.set(a, x); stuckWhy.set(a, r.stuckWhy.get(a)); }
        if (r.error) error = r.error;
        if (upTo <= g) break;
      }
      for (const [a, x] of stuck) if (this.factories.includes(a)) { upTo = Math.min(upTo, x - 1); error ??= `the factory ${a}'s block ${x} could not be read`; }
      if (upTo <= g) break;
      const within = got.filter((l) => num(l.blockNumber) <= upTo).sort((x, y) => num(x.blockNumber) - num(y.blockNumber) || num(x.logIndex) - num(y.logIndex));
      for (const m of members) {
        const x = stuck.get(m), end = x !== undefined && x <= upTo ? x - 1 : upTo;
        this.#append(m, within.filter((l) => lc(l.address) === m && num(l.blockNumber) <= end));
        this.s.coins[m].cursor = end;
        this.#mark(m, x !== undefined && x <= upTo ? x : undefined, stuckWhy.get(m) ?? error);
      }
      this.#discover(within);                                  // new coins start behind the group, at their launch block
      this.s.group.cursor = upTo; this.dirty = true;
    }
    for (const coin of this.behind()) { if (this.clock() >= deadline || this.p.cooling(this.clock())) break; await this.#catchUp(coin, deadline); }
    this.compact(safe);
    if (this.p.cooling(this.clock())) error ??= `${this.p.host} is cooling down (${this.p.lastError})`;
    return { head, safe, group: this.s.group.cursor, behind: this.behind(), error };
  }

  behind() { return Object.entries(this.s.coins).filter(([, c]) => c.cursor < this.s.group.cursor).map(([k]) => k); }

  /**
   * Fold every log older than the retention into the base balances. Logs found missing (a balance would go negative) rebuild the
   * coin from its launch; a second time within RESET_TTL_MS marks it broken, and a broken coin is rebuilt again after
   * BROKEN_RETRY_MS (L-F: neither flag is for ever).
   */
  compact(safe) {
    const cut = safe - this.retention, now = this.clock();
    for (const [k, c] of Object.entries(this.s.coins)) {
      if (c.broken) {
        if (now - (c.brokenAt ?? 0) >= BROKEN_RETRY_MS) { this.s.coins[k] = { ...this.#fresh(c), resetAt: now }; this.dirty = true; }
        continue;
      }
      const hold = this.holds.get(k);
      const to = Math.min(cut, c.cursor, hold === undefined ? Infinity : hold - 1);
      if (to <= c.baseBlock) continue;
      const bal = new Map(Object.entries(c.base).map(([a, v]) => [a, BigInt(v)]));
      let i = 0;
      try {
        for (; i < c.logs.length && c.logs[i][0] <= to; i++) apply(bal, c.logs[i]);
      } catch (err) {
        // logs are missing (a provider that dropped some): say so on every root of this coin rather than guess, and rebuild it
        // from its launch; a second failure within RESET_TTL_MS leaves it broken for BROKEN_RETRY_MS (its draws stay unverified)
        if (now - (c.resetAt ?? (c.resets ? 0 : -Infinity)) < RESET_TTL_MS) { c.broken = String(err.message).slice(0, 200); c.brokenAt = now; this.dirty = true; continue; }
        const { resets: _r, ...rest } = c;
        this.s.coins[k] = { ...this.#fresh(rest), resetAt: now };
        this.dirty = true; continue;
      }
      c.base = Object.fromEntries([...bal].filter(([, v]) => v > 0n).map(([a, v]) => [a, v.toString()]));
      c.logs = c.logs.slice(i); c.baseBlock = to; this.dirty = true;
    }
  }

  /**
   * L-F (fixes-R4-services.md §4): reconcile a few coins against the chain's state at the index's own cursor: the index's sum of
   * balances must be `totalSupply()`, and its largest balances must be `balanceOf`. Read from this provider, else from any of
   * `stateUrls` (drpc serves no state at a block); only while the cursor is recent (a full node keeps recent state). A mismatch
   * rebuilds the coin from its launch (at most once an hour a coin); a read that fails changes nothing. Returns what it found.
   *
   * R4-check L-S4 (fixes-golive-lows.md §6): bounded by `deadline`: it is checked before every read, and each read gets the time
   * left (at most 30 s, at least 250 ms) and one try, so a provider that hangs costs the budget, not 17 reads × 3 tries × 30 s. A
   * coin whose reads ran out of time is not marked reconciled (it is tried again at the next poll).
   */
  async reconcile({ stateUrls = [], head, deadline = Infinity } = {}) {
    const now = this.clock(), found = [];
    const left = () => deadline - this.clock();
    const opts = (block) => ({ http: this.http, block, timeoutMs: Math.max(250, Math.min(30_000, left())), tries: 1 });
    const due = Object.entries(this.s.coins)
      .filter(([, c]) => !c.broken && !c.stuck && c.cursor >= c.launchBlock && head - c.cursor <= RECONCILE.maxAgeBlocks && now - (c.reconciledAt ?? 0) >= RECONCILE.everyMs)
      .sort(([, x], [, y]) => (x.reconciledAt ?? 0) - (y.reconciledAt ?? 0)).slice(0, RECONCILE.perPoll);
    for (const [k, c] of due) {
      if (this.clock() >= deadline) break;
      const block = c.cursor;
      let bal;
      try { bal = this.balancesAt(k, block); } catch { continue; }            // the index cannot say (being rebuilt): next time
      let supply = null, src = null, out = false;
      for (const u of [this.url, ...stateUrls.filter((x) => x !== this.url)]) {
        if (left() <= 0) { out = true; break; }
        try { supply = await read(u, ERC20, k, "totalSupply", [], opts(block)); src = u; break; } catch { /* no state here: the next */ }
      }
      if (out || (supply === null && left() <= 0)) break;                      // out of time: this coin next poll
      const before = c.reconciledAt;
      c.reconciledAt = now; this.dirty = true;
      if (supply === null) { c.reconcile = { at: now, block, v: "unreadable" }; continue; }
      const why = [];
      const sum = [...bal.values()].reduce((x, y) => x + y, 0n);
      if (sum !== supply) why.push(`the index's balances sum to ${sum}, totalSupply() is ${supply}`);
      for (const [a, v] of [...bal].filter(([, v]) => v > 0n).sort((x, y) => (y[1] > x[1] ? 1 : y[1] < x[1] ? -1 : 0)).slice(0, RECONCILE.top)) {
        if (left() <= 0) { out = true; break; }
        const onChain = await read(src, ERC20, k, "balanceOf", [a], opts(block)).catch(() => null);
        if (onChain !== null && onChain !== v) why.push(`${a}: the index says ${v}, balanceOf says ${onChain}`);
      }
      // out of time before every balance was read, and nothing wrong so far: not reconciled, tried again next poll
      if (out && !why.length) { if (before === undefined) delete c.reconciledAt; else c.reconciledAt = before; break; }
      if (!why.length) { c.reconcile = { at: now, block, v: "ok" }; continue; }
      const rebuild = now - (c.reconcileRebuiltAt ?? 0) >= RECONCILE.rebuildEveryMs;
      const rec = { at: now, block, v: "mismatch", why: why.slice(0, 3).join("; ").slice(0, 300), rebuilt: rebuild };
      this.s.coins[k] = rebuild ? { ...this.#fresh(c), reconcile: rec, reconcileRebuiltAt: now } : { ...c, reconcile: rec };
      found.push({ coin: k, ...rec });
    }
    return found;
  }

  /** Balances of `coin` after block `block`, from this provider's index alone. Throws when the index cannot say. */
  balancesAt(coin, block) {
    const c = this.s.coins[lc(coin)];
    if (!c) throw new Error(`${this.p.host}: ${coin} is not indexed yet`);
    if (c.broken) throw new Error(`${this.p.host}: the index of ${coin} is broken (${c.broken})`);
    if (c.stuck && c.cursor < block) throw new Error(`${this.p.host}: the index of ${coin} is stuck at block ${c.stuck.block}, behind the snapshot block ${block}`);
    if (block < c.launchBlock) return new Map();
    if (c.cursor < block) throw new Error(`${this.p.host}: the index of ${coin} is at block ${c.cursor}, behind the snapshot block ${block}`);
    if (block < c.baseBlock) throw new Error(`${this.p.host}: block ${block} is older than the index keeps (${c.baseBlock})`);
    const bal = new Map(Object.entries(c.base).map(([a, v]) => [a, BigInt(v)]));
    for (const l of c.logs) { if (l[0] > block) break; apply(bal, l); }
    return bal;
  }

  /** The draw's root from this provider's index: { block, root, totalTickets, holders }. */
  async rootAt({ coin, snapshotAt, fromBlock, head, exclusions }) {
    const key = String(snapshotAt);
    let block = this.blocks.get(key);
    if (block === undefined) {
      block = await blockAtOrBefore(this.url, Number(snapshotAt), fromBlock, head, this.http);
      if (block === null) throw new Error(`${this.p.host}: no block at or before ${snapshotAt} after the stack's start block ${fromBlock}`);
      this.blocks.set(key, block);
    }
    const t = weightedTree(this.balancesAt(coin, block), exclusions, { sentinel: { snapshotAt } });
    return { block, root: t.root, totalTickets: t.totalTickets, holders: t.holders, repairTag: this.s.coins[lc(coin)]?.repairTag ?? null };
  }

  save() {
    if (!this.file || !this.dirty) return;
    mkdirSync(dirname(this.file), { recursive: true });
    const tmp = `${this.file}.tmp-${process.pid}`;
    writeFileSync(tmp, JSON.stringify(this.s));
    renameSync(tmp, this.file);
    this.dirty = false;
  }
}

function apply(bal, [block, , from, to, value]) {
  const v = BigInt(value);
  if (from !== ZERO) {
    const next = (bal.get(from) ?? 0n) - v;
    if (next < 0n) throw new Error(`logs are missing: ${from} would go negative at block ${block}`);
    bal.set(from, next);
  }
  if (to !== ZERO) bal.set(to, (bal.get(to) ?? 0n) + v);
}

/** The indexes the config names (`draws.providers`, default the two `evm.rpcs`), each kept in `<state dir>/holders-<host>.json`. */
export function indexesFrom(cfg, st, { http = fetch, clock = Date.now, dir = null } = {}) {
  const list = (cfg.draws?.providers ?? cfg.evm.rpcs).map((x) => (typeof x === "string" ? { url: x } : x));
  const factories = st.collections.map((c) => c.factory).filter(Boolean);
  return list.map((p) => new HolderIndex({ url: p.url, span: p.span ?? 10_000, factories, fromBlock: st.fromBlock, confirmations: cfg.evm.confirmations ?? 20,
    file: dir ? `${dir}/holders-${host(p.url).replace(/[^a-z0-9.-]/gi, "_")}.json` : null, http, clock,
    retentionBlocks: cfg.draws?.retention_blocks ?? RETENTION_BLOCKS }));
}
