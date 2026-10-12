import type { BoardRow } from "./apiTypes";

/** The board's tabs, sorts and search, in the browser over one response (docs/v6/site-design.md §6). Pure, so it can be tested. */
export type Tab = "new" | "trending" | "graduating" | "graduated" | "all" | "watch";
export const TABS: [Tab, string][] = [["new", "New"], ["trending", "Trending"], ["graduating", "Graduating"], ["graduated", "Graduated"], ["all", "All"], ["watch", "Watching"]];
export type Sort = "trending" | "cap" | "volume" | "age" | "holders" | "progress" | "swept";
export const SORTS: [Sort, string][] = [["trending", "Trending"], ["cap", "Market cap"], ["volume", "24h volume"], ["age", "Age"], ["holders", "Holders"], ["progress", "Progress"], ["swept", "Floor swept"]];
export const DAY = 86_400;

const n = (x: number | null | undefined) => (x === null || x === undefined ? -1 : x);
const wei = (x: string | null | undefined) => (x === null || x === undefined ? -1 : Number(BigInt(x)) / 1e18);
/** how far a coin is to graduation, 0..1; graduated is 1; a pons coin, which has no raise here, is unknown */
export const progressOf = (r: BoardRow): number | null => r.phase === "graduated" ? 1 : r.raise ? Math.min(1, Number(BigInt(r.raise.raisedWei) * 10_000n / (BigInt(r.raise.raiseWei) || 1n)) / 10_000) : null;

/**
 * docs/v6/site-design.md §6.2, the trending score: what traded, who arrived, how often it was bought — never age alone.
 *   score = ln(1 + volume24h in ETH) + 0.5 · ln(1 + new holders in 24h) + 0.25 · ln(1 + buys per hour × 24)
 * Each term is a log so one whale's volume cannot drown a coin that many people are buying; a term the source does not know
 * counts zero, and a coin with no term at all scores null and sorts last rather than as a quiet coin.
 */
export function trendingScore(r: BoardRow): number | null {
  const vol = r.volume24hWei === null ? null : wei(r.volume24hWei);
  if (vol === null && r.newHolders24h === null && r.buys24h === null) return null;
  return Math.log1p(Math.max(0, vol ?? 0)) + 0.5 * Math.log1p(Math.max(0, r.newHolders24h ?? 0)) + 0.25 * Math.log1p(Math.max(0, r.buys24h ?? 0));
}

export function tabRows(rows: BoardRow[], tab: Tab, now: number, watch: readonly string[] = []): BoardRow[] {
  if (tab === "watch") { const w = new Set(watch.map((x) => x.toLowerCase())); return rows.filter((r) => w.has(r.address.toLowerCase())); }
  if (tab === "new") return rows.filter((r) => r.launchedAt !== null && now - r.launchedAt < DAY);
  if (tab === "trending") return rows.filter((r) => (trendingScore(r) ?? 0) > 0);
  if (tab === "graduating") return rows.filter((r) => r.phase === "graduating" || (r.phase === "trading" && (progressOf(r) ?? 0) >= 0.5));
  if (tab === "graduated") return rows.filter((r) => r.phase === "graduated");
  return rows;
}

/** The sort a tab opens on: newest for New, the day's volume for Trending, progress for Graduating, market cap otherwise. */
export const defaultSort = (tab: Tab): Sort => (tab === "new" ? "age" : tab === "trending" ? "trending" : tab === "graduating" ? "progress" : "cap");

export function sortRows(rows: BoardRow[], by: Sort): BoardRow[] {
  const out = [...rows];
  const cmp = (f: (r: BoardRow) => number) => (a: BoardRow, b: BoardRow) => f(b) - f(a) || a.symbol.localeCompare(b.symbol);
  if (by === "trending") return out.sort(cmp((r) => n(trendingScore(r))));
  if (by === "cap") return out.sort(cmp((r) => n(r.marketCapEth)));
  if (by === "volume") return out.sort(cmp((r) => wei(r.volume24hWei)));
  if (by === "age") return out.sort(cmp((r) => n(r.launchedAt)));
  if (by === "holders") return out.sort(cmp((r) => n(r.holders)));
  if (by === "progress") return out.sort(cmp((r) => n(progressOf(r))));
  return out.sort(cmp((r) => wei(r.sweptWei)));
}

export function searchRows(rows: BoardRow[], q: string): BoardRow[] {
  const s = q.trim().toLowerCase();
  if (!s) return rows;
  return rows.filter((r) => r.name.toLowerCase().includes(s) || r.symbol.toLowerCase().includes(s) || r.address.toLowerCase().includes(s) || (r.nest.name ?? "").toLowerCase().includes(s));
}

/** The king of the hoard: the top of the trending score, among the coins that can still be traded. */
export function king(rows: BoardRow[]): BoardRow | null {
  const live = rows.filter((r) => (r.phase === "trading" || r.phase === "graduated") && (trendingScore(r) ?? 0) > 0);
  return sortRows(live, "trending")[0] ?? null;
}

/** "$1.2M", "0.42 ETH", "—": a market cap in the unit at hand. */
export function capText(ethCap: number | null, usdPerEth: number | null): string {
  if (ethCap === null) return "—";
  if (usdPerEth !== null) { const v = ethCap * usdPerEth; return v >= 1e6 ? `$${(v / 1e6).toFixed(2)}M` : v >= 1e3 ? `$${(v / 1e3).toFixed(1)}k` : `$${v.toFixed(0)}`; }
  return ethCap >= 1000 ? `${(ethCap / 1000).toFixed(1)}k ETH` : ethCap >= 1 ? `${ethCap.toFixed(2)} ETH` : `${small(ethCap, (v) => v.toPrecision(3))} ETH`;
}
export function volText(volWei: string | null): string {
  if (volWei === null) return "—";
  const v = wei(volWei);
  return v === 0 ? "0" : v >= 1 ? `${v.toFixed(2)} ETH` : `${small(v, (x) => x.toPrecision(3))} ETH`;
}
export function changeText(c: number | null): string { return c === null ? "—" : `${c > 0 ? "+" : ""}${(c * 100).toFixed(Math.abs(c) >= 0.1 ? 1 : 2)}%`; }
export function ageText(launchedAt: number | null, now: number): string {
  if (launchedAt === null) return "—";
  const d = Math.max(0, now - launchedAt);
  return d < 60 ? `${d}s` : d < 3_600 ? `${Math.floor(d / 60)}m` : d < DAY ? `${Math.floor(d / 3_600)}h` : `${Math.floor(d / DAY)}d`;
}
/**
 * A very small number the way a trader reads it (design review 2026-09-27): the zeros after the point counted as a subscript,
 * never an exponent — 0.00000002778 is "0.0₇2778". Four significant figures.
 */
const SUB = "₀₁₂₃₄₅₆₇₈₉";
export function subZeros(p: number, sig = 4): string {
  const [m, e] = p.toExponential(sig - 1).split("e");
  const zeros = -Number(e) - 1;
  const digits = m.replace(".", "").replace(/0+$/, "") || "0";
  return `0.0${String(zeros).split("").map((d) => SUB[Number(d)]).join("")}${digits}`;
}
/** A positive number below a ten-thousandth, in subscript-zero notation; anything larger as `larger` says. */
const small = (v: number, larger: (v: number) => string): string => (v > 0 && v < 1e-4 ? subZeros(v) : larger(v));
/** The whole decimal, for a title or a screen reader: "0.00000002778". */
export function priceFull(p: number | null): string {
  if (p === null || !Number.isFinite(p) || p <= 0) return "";
  return p >= 1e-4 ? String(p) : p.toFixed(Math.min(20, -Math.floor(Math.log10(p)) + 3)).replace(/0+$/, "");
}
/** A price in the pair asset: four significant figures; below a ten-thousandth, the zeros are counted as a subscript. */
export function priceText(p: number | null): string {
  if (p === null || !Number.isFinite(p) || p <= 0) return "—";
  if (p >= 0.01) return p.toPrecision(4);
  if (p >= 0.0001) return p.toFixed(8).replace(/0+$/, "");
  return subZeros(p);
}

/** Where a coin's trade fee goes, in one sentence: the fee cell's tooltip, and the reason a fee above 5% is not a typo. */
export const HIGH_FEE_BPS = 500;
export function feeNote(r: Pick<BoardRow, "kind" | "feeBps">): string {
  if (r.feeBps === null) return "The fee is not known here.";
  const f = `${(r.feeBps / 100).toFixed(r.feeBps % 100 ? 2 : 0)}%`;
  return r.kind === "curve"
    ? `${f} of every trade, fixed at launch by the coin's creator: at the worked split, 40% of it buys NFTs from the collection's floor into the vault, 30% goes to the creator, 5% to the collection's reserve, 5% to locked liquidity and 20% to the protocol.${r.feeBps > HIGH_FEE_BPS ? " A higher fee buys the floor faster and makes the coin dearer to trade." : ""}`
    : `${f} of every trade: Pons's base fee plus the creator's tax; 80% of what reaches the router goes to the collection's vault.${r.feeBps > HIGH_FEE_BPS ? " Most of a fee this high is the creator's tax." : ""}`;
}
