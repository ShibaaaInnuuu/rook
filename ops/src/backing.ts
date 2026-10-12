import { decodeFunctionResult, encodeFunctionData, parseAbi, type Hex } from "viem";

/**
 * Audit E-05 (docs/audit/2026-09-26/fixes-W2-monitor.md §2). The reserves report counts every ETH-backed unit as backed, and the
 * module's `escrowBalance()` is its own bookkeeping. This reads what the module really holds, at one pinned block, and holds it to
 * the module's own books (the invariant test's `_books()`: balance ≥ backing + reserve + surplus + revenue + gradEth +
 * rentalEscrow + renterPool + owedTotal), to the units it backs, and to its floor; and a stale floor, or the stale fallback, is
 * red. Reads only: `eth_call` and `eth_getBalance`.
 */
export type Rpc = <T>(method: string, params: unknown[]) => Promise<T>;

const MODULE = parseAbi([
  "function backing() view returns (uint256)", "function reserve() view returns (uint256)", "function surplus() view returns (uint256)",
  "function revenue() view returns (uint256)", "function gradEth() view returns (uint256)", "function rentalEscrow() view returns (uint256)",
  "function renterPool() view returns (uint256)", "function owedTotal() view returns (uint256)", "function reserveUnits() view returns (uint256)",
  "function accepted() view returns (uint256)", "function fresh() view returns (bool)", "function escaped() view returns (bool)",
  "function postedAt() view returns (uint64)", "function lastGoodAt() view returns (uint64)", "function FLOOR_TTL() view returns (uint256)",
  // docs/v6/relay-conversion.md §2: the advance (a module built before it answers none of these: read as no advance)
  "function transitUnits() view returns (uint256)", "function advanced() view returns (uint256)", "function advancedAt() view returns (uint64)",
]);
const WRAPPER = parseAbi(["function ethBacked() view returns (uint256)"]);
const BOOKS = ["backing", "reserve", "surplus", "revenue", "gradEth", "rentalEscrow", "renterPool", "owedTotal"] as const;

export type Backing = {
  slug: string; module: string; wrapper: string; block: number; balance: bigint; books: Record<(typeof BOOKS)[number], bigint>; owed: bigint;
  reserveUnits: bigint; ethBacked: bigint; accepted: bigint; fresh: boolean; escaped: boolean; postedAt: number; lastGoodAt: number; floorTtl: number;
  /** docs/v6/relay-conversion.md §2: the unit whose item budget left before its item (0 or 1e18), what was advanced, and when */
  transitUnits?: bigint; advanced?: bigint; advancedAt?: number;
  /** docs/v9 §5, the Solana home: amounts in lamports and units of 10^9 (the books' module M[i] and its wX); absent: wei and 10^18 */
  decimals?: 9 | 18;
};
export type BackingRead = { kind: "read"; at: number; modules: Backing[] } | { kind: "error"; at: number; error: string } | { kind: "absent"; at: number; note: string };

type StackCol = { slug: string; wrapper: string; floorModule?: string };
export const stackCollections = (stack: { collections?: StackCol[] | Record<string, StackCol> }): StackCol[] =>
  (Array.isArray(stack.collections) ? stack.collections : Object.values(stack.collections ?? {}));

/** Every module of the stack, read at one block (the RPC's latest, less a margin every provider still serves). */
export async function readBacking(rpc: Rpc, stack: { collections?: StackCol[] | Record<string, StackCol> }, nowS: number, margin = 20): Promise<BackingRead> {
  const cols = stackCollections(stack).filter((c) => c.floorModule && !/^0x0{40}$/i.test(c.floorModule));
  if (!cols.length) return { kind: "absent", at: nowS, note: "the stack names no FloorModule: no ETH-backed units to check" };
  try {
    const block = Number(BigInt(await rpc<string>("eth_blockNumber", []))) - margin;
    const tag = `0x${block.toString(16)}`;
    const call = async <T>(abi: typeof MODULE | typeof WRAPPER, to: string, functionName: string): Promise<T> => {
      const data = encodeFunctionData({ abi, functionName } as never);
      const out = await rpc<Hex>("eth_call", [{ to, data }, tag]);
      return decodeFunctionResult({ abi, functionName, data: out } as never) as T;
    };
    const modules: Backing[] = [];
    for (const c of cols) {
      const m = c.floorModule!;
      const books = Object.fromEntries(await Promise.all(BOOKS.map(async (k) => [k, await call<bigint>(MODULE, m, k)]))) as Backing["books"];
      const [balance, reserveUnits, ethBacked, accepted, fresh, escaped, postedAt, lastGoodAt, ttl] = await Promise.all([
        rpc<string>("eth_getBalance", [m, tag]).then((x) => BigInt(x)), call<bigint>(MODULE, m, "reserveUnits"), call<bigint>(WRAPPER, c.wrapper, "ethBacked"),
        call<bigint>(MODULE, m, "accepted"), call<boolean>(MODULE, m, "fresh"), call<boolean>(MODULE, m, "escaped"), call<bigint>(MODULE, m, "postedAt"),
        call<bigint>(MODULE, m, "lastGoodAt"), call<bigint>(MODULE, m, "FLOOR_TTL")]);
      const [transitUnits, advanced, advancedAt] = await Promise.all([call<bigint>(MODULE, m, "transitUnits").catch(() => 0n),
        call<bigint>(MODULE, m, "advanced").catch(() => 0n), call<bigint>(MODULE, m, "advancedAt").catch(() => 0n)]);
      modules.push({ transitUnits, advanced, advancedAt: Number(advancedAt), slug: c.slug, module: m.toLowerCase(), wrapper: c.wrapper.toLowerCase(), block, balance, books, owed: BOOKS.reduce((s, k) => s + books[k], 0n),
        reserveUnits, ethBacked, accepted, fresh, escaped, postedAt: Number(postedAt), lastGoodAt: Number(lastGoodAt), floorTtl: Number(ttl) });
    }
    return { kind: "read", at: nowS, modules };
  } catch (err) {
    return { kind: "error", at: nowS, error: String((err as Error).message).split("\n")[0].slice(0, 200) };
  }
}

export const MIN_BACKING_BPS_DEFAULT = 5_000;
export const STALE_FLOOR_RED_S_DEFAULT = 3_600;
/** docs/v6/relay-conversion.md §6: an advance still open this long (its frog neither arrived nor its ETH returned) is red. */
export const ADVANCE_STALE_S_DEFAULT = 48 * 3_600;
const ethOf = (w: bigint, decimals: 9 | 18 = 18) => decimals === 9 ? `${(Number(w / 1_000n) / 1e6).toString()} SOL` : `${(Number(w / 10n ** 12n) / 1e6).toString()} ETH`;
const iso = (s: number) => (s ? new Date(s * 1000).toISOString() : "never");

/** Pure. RED on a module short of its own books, units it cannot back, escrow far under the floor, a stale floor, the stale fallback. */
export function backingStatus(b: BackingRead, nowS: number, o: { minBackingBps?: number; staleFloorRedS?: number; advanceStaleS?: number } = {}): { status: "green" | "red" | "unverified"; reasons: string[] } {
  if (b.kind === "absent") return { status: "green", reasons: [] };
  if (b.kind === "error") return { status: "unverified", reasons: [`the FloorModule could not be read: ${b.error}`] };
  const minBps = BigInt(o.minBackingBps ?? MIN_BACKING_BPS_DEFAULT), staleS = o.staleFloorRedS ?? STALE_FLOOR_RED_S_DEFAULT;
  const advS = o.advanceStaleS ?? ADVANCE_STALE_S_DEFAULT;
  const reasons: string[] = [];
  for (const m of b.modules) {
    const sol = m.decimals === 9, one = sol ? 10n ** 9n : 10n ** 18n;
    const eth = (w: bigint) => ethOf(w, m.decimals ?? 18);
    const p = sol ? `${m.slug}: Module ${m.module}` : `${m.slug}: FloorModule ${m.module}`;
    if (m.balance < m.owed) reasons.push(`${p} holds ${eth(m.balance)}, less than it owes by its own books (${eth(m.owed)}: backing ${eth(m.books.backing)} + reserve ${eth(m.books.reserve)} + surplus ${eth(m.books.surplus)} + revenue ${eth(m.books.revenue)} + the rest), at block ${m.block}`);
    if (m.reserveUnits > m.ethBacked) reasons.push(`${p} counts ${m.reserveUnits} reserve units but the wrapper has only ${m.ethBacked} ETH-backed units`);
    if (m.ethBacked === 0n && m.books.backing > 0n) reasons.push(`${p} holds backing ${eth(m.books.backing)} for no ETH-backed unit`);
    const escrow = m.books.backing + m.books.reserve;
    // audit 2026-09-27-nft D-I-3: the unit in transit has no escrow behind it (its backing left with the advance, and its item is on
    // its way): the per-unit rule counts the units the escrow backs
    const escrowed = m.ethBacked - (m.transitUnits ?? 0n);
    if (escrowed > 0n && m.accepted > 0n && escrow * one * 10_000n < escrowed * m.accepted * minBps)
      reasons.push(`${p}: escrow ${eth(escrow)} backs ${eth(escrowed)} units at ${eth((escrow * one) / escrowed)} a unit, under ${Number(minBps) / 100}% of the floor ${eth(m.accepted)}`);
    const t = m.transitUnits ?? 0n;
    if (t > one) reasons.push(`${p} counts ${t} units in transit: it allows one advance at a time`);
    if (t > 0n && m.advancedAt && nowS > m.advancedAt + advS)
      reasons.push(`${p}: an advance of ${eth(m.advanced ?? 0n)} has been open since ${iso(m.advancedAt)} (over ${advS / 3600} h): its frog has not arrived and its ETH has not come back (docs/v6/relay-conversion.md §5)`);
    if (m.escaped) reasons.push(`${p} is in the stale fallback (escaped): no good floor post since ${iso(m.lastGoodAt)}`);
    // a module that never posted and backs nothing is a stack before its launch, not a stale one. R2-L4 (E-05): the rule keys on
    // the module's REAL freshness, the last GOOD post (confidence at or above its minimum), not on postedAt: a thief re-posting
    // confidence 0 keeps postedAt recent while fresh() is false, and that module is frozen, not green.
    else if (!m.fresh && (m.postedAt > 0 || m.lastGoodAt > 0 || m.ethBacked > 0n) && nowS > m.lastGoodAt + m.floorTtl + staleS)
      reasons.push(`${p}: the floor is stale since ${iso(m.lastGoodAt + m.floorTtl)} (last good post ${iso(m.lastGoodAt)}${m.postedAt > m.lastGoodAt ? `; last post ${iso(m.postedAt)}, below the module's minimum confidence` : ""}): mint, fills and sweeps are refused`);
  }
  return { status: reasons.length ? "red" : "green", reasons };
}
