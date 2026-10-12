/** How the Solana pages print numbers and links. */
export const LAMPORTS = 1_000_000_000n;

export function solNum(sol: number | null | undefined, dp = 2): string {
  if (sol === null || sol === undefined || !Number.isFinite(sol)) return "—";
  if (sol !== 0 && Math.abs(sol) < 10 ** -dp) return dp >= 6 ? sol.toPrecision(2) : `<${(10 ** -dp).toFixed(dp)}`;
  return sol.toLocaleString("en-US", { maximumFractionDigits: dp, minimumFractionDigits: 0 });
}
export const lamportsToSol = (l: bigint) => Number(l) / 1e9;
/** Lamports (or wX base units: 9 decimals too) as a decimal with up to `dp` decimals, trailing zeros dropped: 9_870_123_456n → "9.87". */
export function solText(l: bigint, dp = 4): string {
  const neg = l < 0n, v = neg ? -l : l;
  const whole = v / 1_000_000_000n, frac = (v % 1_000_000_000n).toString().padStart(9, "0").slice(0, dp).replace(/0+$/, "");
  return `${neg ? "−" : ""}${whole.toLocaleString("en-US")}${frac ? `.${frac}` : ""}`;
}
export function usd(v: number | null | undefined): string {
  if (v === null || v === undefined || !Number.isFinite(v)) return "—";
  if (v >= 1e6) return `$${(v / 1e6).toLocaleString("en-US", { maximumFractionDigits: 2 })}M`;
  if (v >= 1e3) return `$${(v / 1e3).toLocaleString("en-US", { maximumFractionDigits: 1 })}k`;
  return `$${v.toLocaleString("en-US", { maximumFractionDigits: v < 1 ? 4 : 2 })}`;
}
/** A token amount in base units as a decimal string (no float), trimmed. */
export function units(n: bigint, decimals: number, dp = 4): string {
  const neg = n < 0n, v = neg ? -n : n, d = 10n ** BigInt(decimals);
  const whole = (v / d).toLocaleString("en-US"), frac = (v % d).toString().padStart(decimals, "0").slice(0, dp).replace(/0+$/, "");
  return `${neg ? "−" : ""}${whole}${frac ? `.${frac}` : ""}`;
}
/** "1.5" → 1500000 base units at 6 decimals; null when it is not a positive decimal number. */
export function parseUnits(s: string, decimals: number): bigint | null {
  const t = s.trim().replace(/,/g, "");
  if (!/^\d*\.?\d*$/.test(t) || t === "" || t === ".") return null;
  const [w, f = ""] = t.split(".");
  if (f.length > decimals) return null;
  const v = BigInt(w || "0") * 10n ** BigInt(decimals) + BigInt((f + "0".repeat(decimals)).slice(0, decimals) || "0");
  return v > 0n ? v : null;
}
export const short = (a: string) => `${a.slice(0, 4)}…${a.slice(-4)}`;
export const solscan = (what: "account" | "token" | "tx", id: string) => `https://solscan.io/${what}/${id}`;
export const jupiter = (mint: string) => `https://jup.ag/swap/SOL-${mint}`;
export function age(sec: number, now: number): string {
  if (!sec) return "—";
  const d = Math.max(0, now - sec);
  return d < 60 ? `${d}s` : d < 3_600 ? `${Math.floor(d / 60)}m` : d < 86_400 ? `${Math.floor(d / 3_600)}h` : `${Math.floor(d / 86_400)}d`;
}
export const pctText = (p: number) => `${(p * 100).toFixed(p < 0.1 ? 1 : 0)}%`;
