/**
 * docs/phase4.md §5. One JSON object per line. URLs are reduced to their host. The package holds no signing key; it reads TWO
 * secrets, OPENSEA_API_KEY (docs/v4/eth-nft-canister.md §12.2) and UNISAT_API_KEY (docs/v6/ordinals-apis.md), each in one
 * module, and the scrubber below guarantees no line carries either: a field
 * whose name says key/secret/token/authorization is replaced whole, and the value of every secret is cut out of the finished
 * line wherever it appears — in a field, a nested object, or an error message somebody passed along.
 */
export type Level = "debug" | "info" | "warn" | "error";
export type Logger = (level: Level, event: string, fields?: Record<string, unknown>) => void;

export const hostOf = (url: string): string => { try { return new URL(url).host; } catch { return "invalid-url"; } };

export const REDACTED = "[redacted]";
/** The secrets this package may hold, read at the moment of logging (never cached): the OpenSea and UniSat API keys. */
export const envSecrets = (env: Readonly<Record<string, string | undefined>> = process.env): string[] => [env.OPENSEA_API_KEY ?? "", env.UNISAT_API_KEY ?? ""];
const SECRET_NAME = /api[_-]?key|secret|token|authorization/i;

/** `line` with every secret (of 8 characters or more: a shorter one would redact ordinary words) cut out. */
export function scrub(line: string, secrets: readonly string[]): string {
  let out = line;
  for (const s of secrets) if (s && s.length >= 8) out = out.split(s).join(REDACTED);
  return out;
}

export function logger(write: (line: string) => void = (l) => console.log(l), now: () => Date = () => new Date(),
  secrets: () => readonly string[] = () => envSecrets()): Logger {
  return (level, event, fields = {}) => {
    const safe = Object.fromEntries(Object.entries(fields).map(([k, v]) => [k, SECRET_NAME.test(k) ? REDACTED
      : typeof v === "bigint" ? String(v) : v instanceof Error ? v.message
      : typeof v === "string" && /^https?:\/\//.test(v) ? hostOf(v) : v]));
    write(scrub(JSON.stringify({ at: now().toISOString(), level, event, ...safe }, (_k, v) => (typeof v === "bigint" ? String(v) : v)), secrets()));
  };
}
