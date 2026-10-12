export const short = (a: string) => (a.length > 12 ? `${a.slice(0, 6)}…${a.slice(-4)}` : a);
export function eth(wei: bigint, digits = 4): string {
  const neg = wei < 0n, v = neg ? -wei : wei, unit = 10n ** 18n;
  const whole = v / unit, frac = (v % unit).toString().padStart(18, "0").slice(0, digits);
  return `${neg ? "−" : ""}${whole.toLocaleString("en-US")}${digits ? "." + frac : ""}`;
}
export function units(raw: bigint, divisibility: number): string {
  if (divisibility === 0) return raw.toLocaleString("en-US");
  const unit = 10n ** BigInt(divisibility);
  return `${(raw / unit).toLocaleString("en-US")}.${(raw % unit).toString().padStart(divisibility, "0")}`;
}
export function countdown(untilS: number, nowS: number): string {
  const s = untilS - nowS;
  if (s <= 0) return "now";
  const h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60), r = s % 60;
  return h ? `${h} h ${String(m).padStart(2, "0")} min` : m ? `${m} min ${String(r).padStart(2, "0")} s` : `${r} s`;
}
export const when = (s: number) => (s ? new Date(s * 1000).toISOString().replace("T", " ").slice(0, 16) + " UTC" : "not set");
export const PURCHASE_STATUS = ["none", "declared", "cancelled", "released", "settled", "written off"] as const;
export const LOT_MODE = ["not chosen", "held", "never delivered", "draw, tickets by balance", "draw, one ticket a wallet", "named recipient", "pro rata to holders"] as const;
export const LOT_STAGE = ["", "snapshot open", "round pinned", "resolved", "winner proven", "delivered"] as const;
