// A draw's root, recomputed from public data and nothing the keeper wrote (docs/audit/2026-09-26/fixes-W2-monitor.md §1.3).
// The rules are docs/snapshot-v1.md for a weighted snapshot plus v2 (fixes-B-C-keeper-draws.md §6): the snapshot block is the
// last block whose timestamp is at most `snapshotAt`, and the tree carries one more leaf, the sentinel
// leaf(address(0), totalTickets, snapshotAt), which commits the count and the time. The tree code is a port of
// tools/snapshot/src/core.ts (the monitor's image has no TypeScript); test/draws.test.mjs holds it to that tool's committed
// fixture byte for byte.
import { keccak_256 } from "@noble/hashes/sha3";
import { host, rpc } from "./abi.mjs";

const ZERO = "0x0000000000000000000000000000000000000000";
const TRANSFER = "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef";
const hexOf = (b) => `0x${Buffer.from(b).toString("hex")}`;
const bytes = (h) => Buffer.from(h.replace(/^0x/, "").padStart(64, "0"), "hex");
const word = (x) => Buffer.from(BigInt(x).toString(16).padStart(64, "0"), "hex");
const k = (b) => keccak_256(b);

/** keccak256(keccak256(abi.encode(holder, first, end))): SweepVault.leaf and tools/snapshot's ticketLeaf. */
export function ticketLeaf(holder, first, end) {
  return hexOf(k(k(Buffer.concat([word(BigInt(holder)), word(first), word(end)]))));
}
const pairHash = (a, b) => (BigInt(a) <= BigInt(b) ? hexOf(k(Buffer.concat([bytes(a), bytes(b)]))) : hexOf(k(Buffer.concat([bytes(b), bytes(a)]))));

/** Sorted leaves, neighbours paired left to right with the commutative hash, an odd node moves up unchanged. */
export function merkleRoot(leaves) {
  if (!leaves.length) throw new Error("an empty snapshot has no root");
  let layer = [...leaves].sort((a, b) => (BigInt(a) < BigInt(b) ? -1 : BigInt(a) > BigInt(b) ? 1 : 0));
  for (let i = 1; i < layer.length; i++) if (layer[i] === layer[i - 1]) throw new Error("duplicate leaf");
  while (layer.length > 1) {
    const next = [];
    for (let i = 0; i < layer.length; i += 2) next.push(i + 1 < layer.length ? pairHash(layer[i], layer[i + 1]) : layer[i]);
    layer = next;
  }
  return layer[0];
}

/** Balances after every log given, in (block, log index) order. Throws if a balance would go negative (logs are missing). */
export function balancesOf(logs) {
  const sorted = [...logs].sort((x, y) => (x.blockNumber === y.blockNumber ? x.logIndex - y.logIndex : x.blockNumber < y.blockNumber ? -1 : 1));
  const bal = new Map();
  for (const l of sorted) {
    const from = l.from.toLowerCase(), to = l.to.toLowerCase();
    if (from !== ZERO) {
      const next = (bal.get(from) ?? 0n) - l.value;
      if (next < 0n) throw new Error(`logs are missing: ${from} would go negative at block ${l.blockNumber}`);
      bal.set(from, next);
    }
    if (to !== ZERO) bal.set(to, (bal.get(to) ?? 0n) + l.value);
  }
  return bal;
}

/**
 * The weighted tree: one ticket per whole coin (18 decimals), holders by address, the excluded left out; with `sentinel`
 * ({ snapshotAt }) the v2 sentinel leaf is added. Returns { root, totalTickets, holders, leaves }.
 */
export function weightedTree(balances, exclusions, { decimals = 18, sentinel = null } = {}) {
  const excluded = new Set(exclusions.map((a) => a.toLowerCase()));
  const one = 10n ** BigInt(decimals);
  const holders = [...balances].filter(([a, b]) => b > 0n && !excluded.has(a)).sort((x, y) => (BigInt(x[0]) < BigInt(y[0]) ? -1 : 1));
  const leaves = [];
  let next = 0n;
  for (const [holder, balance] of holders) {
    const n = balance / one;
    if (n === 0n) continue;
    leaves.push(ticketLeaf(holder, next, next + n));
    next += n;
  }
  if (sentinel) leaves.push(ticketLeaf(ZERO, next, BigInt(sentinel.snapshotAt)));
  return { root: merkleRoot(leaves), totalTickets: next, holders: leaves.length - (sentinel ? 1 : 0), leaves };
}

// ---- the reads -----------------------------------------------------------------------------------------------------------------

const num = (h) => Number(BigInt(h));

/** The last block whose timestamp is at most `t`, between `lo` and `hi` (binary search on headers). Null if `lo` is already later. */
export async function blockAtOrBefore(url, t, lo, hi, http = fetch) {
  const ts = async (n) => num((await rpc(url, "eth_getBlockByNumber", [`0x${n.toString(16)}`, false], http)).timestamp);
  if (await ts(lo) > t) return null;
  if (await ts(hi) <= t) return hi;
  let a = lo, b = hi;                      // ts(a) ≤ t < ts(b)
  while (b - a > 1) {
    const m = Math.floor((a + b) / 2);
    if (await ts(m) <= t) a = m; else b = m;
  }
  return a;
}

/**
 * The coin's Transfer logs over [from, to], paged: a page a provider refuses (ordofi past 10,000 blocks, Robinhood past 10,000
 * logs) is asked again at half the span, down to 16 blocks, and the span grows back after a success.
 */
export async function transferLogs(url, coin, from, to, http = fetch, { span = 10_000, budget = 5_000 } = {}) {
  const out = [];
  let a = from, s = span, calls = 0;
  while (a <= to) {
    if (++calls > budget) throw new Error(`${host(url)}: more than ${budget} log pages; not finished`);
    const b = Math.min(to, a + s - 1);
    let logs;
    try {
      logs = await rpc(url, "eth_getLogs", [{ address: coin, topics: [TRANSFER], fromBlock: `0x${a.toString(16)}`, toBlock: `0x${b.toString(16)}` }], http);
    } catch (err) {
      if (s > 16) { s = Math.max(16, Math.floor(s / 2)); continue; }
      throw err;
    }
    for (const l of logs) {
      if (l.removed || l.topics.length < 3) continue;
      out.push({ blockNumber: BigInt(l.blockNumber), logIndex: num(l.logIndex), from: `0x${l.topics[1].slice(26)}`, to: `0x${l.topics[2].slice(26)}`,
        value: BigInt(l.data === "0x" ? "0x0" : l.data) });
    }
    a = b + 1;
    s = Math.min(span, s * 2);
  }
  return out;
}

/** One provider's recomputation: { block, root, totalTickets } for `coin` at `snapshotAt`. */
export async function recompute(url, { coin, snapshotAt, fromBlock, head, exclusions }, http = fetch, opts = {}) {
  const block = await blockAtOrBefore(url, Number(snapshotAt), fromBlock, head, http);
  if (block === null) throw new Error(`${host(url)}: no block at or before ${snapshotAt} after the stack's deploy block ${fromBlock}`);
  const logs = await transferLogs(url, coin, fromBlock, block, http, opts);
  const t = weightedTree(balancesOf(logs), exclusions, { sentinel: { snapshotAt } });
  return { block, root: t.root, totalTickets: t.totalTickets, holders: t.holders };
}

/**
 * The verdict for one opened draw, from every provider's recomputation (`rs`: a result or { error }, in provider order). Pure.
 *   verified   at least `quorum` reproduce the on-chain root and count, and none computes another
 *   bad        at least two compute the SAME other root (or count), and none reproduces the chain's: a confirmed bad root
 *   disagree   the recomputations differ from each other (some match the chain and some don't, or two other roots)
 *   pending    not enough providers could recompute yet (`matches`: how many reproduce the chain so far)
 */
export function verdictN(onChain, rs, quorum = 2) {
  const ok = rs.filter((r) => !r.error), errs = rs.filter((r) => r.error).map((r) => r.error);
  const same = (r) => r.root.toLowerCase() === String(onChain.root).toLowerCase() && r.totalTickets === BigInt(onChain.totalTickets);
  const match = ok.filter(same), other = ok.filter((r) => !same(r));
  const desc = (r) => `${r.root} (${r.totalTickets} tickets at block ${r.block})`;
  if (!other.length) {
    if (new Set(match.map((r) => r.block)).size > 1) return { v: "disagree", matches: match.length, why: `the providers found different snapshot blocks: ${match.map((r) => r.block).join(" vs ")}` };
    if (match.length >= quorum) return { v: "verified", matches: match.length };
    return { v: "pending", matches: match.length, why: `${match.length} of the ${quorum} providers needed reproduce the root so far${errs.length ? `; ${errs.join("; ")}` : ""}` };
  }
  if (match.length) return { v: "disagree", matches: match.length, why: `the providers' recomputations differ: ${match.length} reproduce the chain's root, ${other.length} compute ${other.map(desc).join(", ")}` };
  const kinds = new Set(other.map((r) => `${r.root.toLowerCase()}|${r.totalTickets}|${r.block}`));
  if (kinds.size > 1) return { v: "disagree", matches: 0, why: `the providers' recomputations differ: ${other.map(desc).join(" vs ")}` };
  if (other.length >= 2) {
    const a = other[0];
    return { v: "bad", matches: 0, why: `recomputed ${a.root} with ${a.totalTickets} tickets at block ${a.block} on ${other.length} providers (snapshot time ${onChain.snapshotAt}); the chain has ${onChain.root} with ${onChain.totalTickets}` };
  }
  return { v: "pending", matches: 0, other: 1, why: `one provider computes ${desc(other[0])}, not the chain's root; waiting for a second${errs.length ? ` (${errs.join("; ")})` : ""}` };
}

/** Two providers' verdict (the wave-2 form, kept for its callers): `verdictN` with a quorum of two. */
export function verdict(onChain, a, b) {
  if (a.error || b.error) return { v: "pending", matches: [a, b].filter((r) => !r.error).length, why: a.error ?? b.error };
  return verdictN(onChain, [a, b], 2);
}
