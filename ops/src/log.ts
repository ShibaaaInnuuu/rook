/**
 * One JSON object per line, as keeper/src/log.ts writes them. ops/ holds two secrets: the operator's identity (which never
 * reaches a logger: src/identity.ts hands out only an Identity object and its principal) and the Telegram bot token, which is
 * cut out of every finished line wherever it appears, as a field, in an error message, or inside a URL somebody passed along.
 */
export type Level = "debug" | "info" | "warn" | "error";
export type Log = (level: Level, event: string, fields?: Record<string, unknown>) => void;

export const REDACTED = "[redacted]";
const SECRET_NAME = /token|secret|pem|authorization|api[_-]?key/i;

export function scrub(line: string, secrets: readonly string[]): string {
  let out = line;
  for (const s of secrets) if (s && s.length >= 8) out = out.split(s).join(REDACTED);
  return out;
}

/**
 * A keyed provider URL's secret parts: the whole URL, and its userinfo, query and every path segment of 16+ characters (Dwellir's key is
 * the path: `https://api-ethereum-mainnet.n.dwellir.com/<key>`). A URL that does not parse is scrubbed whole.
 */
export function urlSecrets(u: string | undefined): string[] {
  if (!u) return [];
  try {
    const x = new URL(u);
    return [u, x.password, x.username, x.search.slice(1), ...x.pathname.split("/").filter((seg) => seg.length >= 16)].filter((v) => v && v.length >= 8);
  } catch { return [u]; }
}

/** The Telegram token, read at the moment of logging. (The operator's identity is not here: it is read in src/identity.ts only, and never reaches a logger.) */
export const envSecrets = (env: Readonly<Record<string, string | undefined>> = process.env): string[] => [env.ALERT_TELEGRAM_BOT_TOKEN ?? "",
  // audit 2026-09-27-nft D-I-4: the OpenSea key is scrubbed from every line, whatever carries it
  env.OPENSEA_API_KEY ?? "",
  // go-live prep: the runner's ETH_LOGS_RPC is the operator's keyed Dwellir URL (deployments/launch/railway/runner.env.example)
  ...urlSecrets(env.ETH_LOGS_RPC)].filter(Boolean);

export function logger(write: (line: string) => void = (l) => console.log(l), secrets: () => readonly string[] = () => envSecrets(), now: () => Date = () => new Date()): Log {
  return (level, event, fields = {}) => {
    const safe = Object.fromEntries(Object.entries(fields).map(([k, v]) => [k, SECRET_NAME.test(k) ? REDACTED : v instanceof Error ? v.message : v]));
    write(scrub(JSON.stringify({ at: now().toISOString(), level, event, ...safe }, (_k, v) => (typeof v === "bigint" ? String(v) : v)), secrets()));
  };
}
