/**
 * docs/phase8.md §2. What a simulated coin does next, as pure functions: no chain, no clock, no randomness of its own.
 * Everything that decides behaviour lives here so it can be tested without a node.
 */
export type Mood = "steady" | "momentum" | "fading" | "runner";
export type Phase = "curve" | "graduated";

export type Coin = {
  address: string;
  curve: string;
  router: string;
  listingId: number;
  mood: Mood;
  phase: Phase;
  bornMs: number;
  /** while a momentum burst is running, when it ends */
  burstUntilMs: number;
  /** how much of the curve's graduation threshold this coin has bought so far, as a fraction */
  bought: number;
  sniped: boolean;
};

export type Step = { kind: "buy" | "sell" | "skip"; /** a share of the graduation threshold */ size: number };

/** The moods a fresh set of coins should carry: spread deliberately, not drawn at random, so the grids never look alike. */
export const MOODS: readonly Mood[] = ["steady", "momentum", "runner", "steady", "fading", "steady", "momentum", "steady", "runner", "fading"];
export const moodFor = (index: number): Mood => MOODS[index % MOODS.length];

/** Creator taxes across the whole range Pons allows, so the Launch page's estimate can be checked against every one of them. */
export const TAXES_BPS: readonly number[] = [0, 100, 250, 300, 500, 750, 1000];
export const taxFor = (index: number): number => TAXES_BPS[index % TAXES_BPS.length];

/**
 * docs/phase11.md §7. How long to wait before the next trade anywhere, with many coins sharing one simulator: the per-coin gap
 * divided by how many coins there are, so that 58 vaults' worth of coins each still trade often enough to draw a chart. Floored,
 * so the node is never asked for more than a few transactions a second.
 */
export const globalGapMs = (perCoinMs: number, coins: number, floorMs = 700): number =>
  Math.max(floorMs, Math.round(perCoinMs / Math.max(1, coins / 8)));

/** How long to wait before this coin's next trade. A burst is faster; a runner is steady; everything is jittered. */
export function gapMs(c: Coin, baseMs: number, nowMs: number, jitter: number): number {
  const bursting = c.mood === "momentum" && nowMs < c.burstUntilMs;
  const factor = bursting ? 0.4 : c.mood === "runner" ? 0.8 : c.mood === "fading" ? 1.4 : 1;
  return Math.max(500, Math.round(baseMs * factor * (0.6 + 1.1 * clamp01(jitter))));
}

const clamp01 = (x: number) => (Number.isFinite(x) ? Math.min(1, Math.max(0, x)) : 0.5);

/**
 * The next trade for a coin. `roll` and `size` are two numbers in [0,1) the caller supplies, so a test can drive every branch.
 * A runner buys and never sells until its curve sells out: that is the only way a staging chain ever reaches a graduated coin.
 */
export function nextStep(c: Coin, nowMs: number, roll: number, size: number): Step {
  const r = clamp01(roll), s = clamp01(size);
  if (c.mood === "runner" && c.phase === "curve") return { kind: "buy", size: 0.08 + 0.07 * s };
  const bursting = c.mood === "momentum" && nowMs < c.burstUntilMs;
  const buyChance = c.mood === "fading" ? 0.3 : bursting ? 0.85 : 0.62;
  // most trades are small; now and then a large one, so a chart is not a flat line of identical bars
  const big = s > 0.88;
  const amount = (big ? 0.05 + 0.06 * s : 0.004 + 0.02 * s) * (c.phase === "graduated" ? 0.6 : 1);
  return { kind: r < buyChance ? "buy" : "sell", size: amount };
}

/** A momentum coin's burst: a minute or two, and only once it has settled for a while. */
export function shouldBurst(c: Coin, nowMs: number, roll: number): boolean {
  return c.mood === "momentum" && nowMs > c.burstUntilMs + 90_000 && clamp01(roll) < 0.15;
}
export const burstEndsAt = (nowMs: number, roll: number): number => nowMs + 60_000 + Math.round(clamp01(roll) * 60_000);

/** Only the first coin is sniped, and only in its first seconds, so the anti-snipe tax appears exactly once and is explicable. */
export const shouldSnipe = (c: Coin, nowMs: number): boolean => !c.sniped && c.bornMs > 0 && nowMs - c.bornMs < 8_000;

/**
 * When to launch the next coin. The first minutes are deliberately busy — a visitor who arrives at a cold chain must not meet an
 * empty grid — and it settles to one every 20–30 minutes after that.
 */
export function nextLaunchDelayMs(coinCount: number, roll: number, fastUntil = 5, steadyMs = 20 * 60_000): number {
  if (coinCount < fastUntil) return 6_000 + Math.round(clamp01(roll) * 9_000);
  return steadyMs + Math.round(clamp01(roll) * (steadyMs / 2));
}

/** A name that says it is simulated, and a ticker that does too. */
const STONES = ["Basalt", "Gabbro", "Slate", "Quartz", "Gneiss", "Marble", "Flint", "Schist", "Pumice", "Obsidian", "Chalk", "Granite", "Shale", "Onyx", "Jasper"];
export function coinName(index: number): { name: string; symbol: string } {
  const stone = STONES[index % STONES.length], n = Math.floor(index / STONES.length);
  return { name: `${stone}${n ? ` ${n + 1}` : ""} (sim)`, symbol: `${stone.slice(0, 4).toUpperCase()}${n ? n + 1 : ""}SIM` };
}

/**
 * A logo drawn from the coin's index: a cut face on a coloured ground, as an SVG data: URI. Pons caps a logo at 512 bytes, so
 * only the characters a URI cannot carry are escaped (`encodeURIComponent` would roughly double it), the shape is kept spare,
 * and the length is checked rather than hoped for: over the cap, `logo()` returns nothing at all.
 */
export const LOGO_MAX = 512;
const escapeSvg = (s: string) => s.replace(/[<>#"%{}|\\^`\s]/g, (c) => `%${c.charCodeAt(0).toString(16).padStart(2, "0")}`);
export function logo(index: number): string {
  const h = (index * 47) % 360, l = 18 + (index % 5) * 4, tip = 14 + (index % 9), x = 12 + (index % 7);
  const svg = `<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 64 64'>` +
    `<rect width='64' height='64' fill='hsl(${h},38%,${l}%)'/>` +
    `<path d='M${x} 46L32 ${tip}L${76 - x} 46Z' fill='hsl(${h},52%,${l + 34}%)'/>` +
    `<path d='M32 ${tip}L${76 - x} 46H32Z' fill='hsl(${h},30%,${l + 14}%)'/></svg>`;
  const uri = `data:image/svg+xml,${escapeSvg(svg)}`;
  return uri.length <= LOGO_MAX ? uri : "";
}
