/**
 * docs/v6/p1-services.md §1. The production keeper's configuration, validated as a whole. It REFUSES TO START on anything missing,
 * contradictory or unknown, listing every problem at once (as src/config.ts does for the dry run). There is no default chain id:
 * the operator writes it, and every RPC must say the same number before anything is simulated or sent.
 *
 * The private key is NOT read here. service/signer.ts is the one file that reads it (test/service-guards.test.ts).
 */
import { alertLatchFloorMs } from "./status.js";
import type { S3Target } from "./s3.js";
export const ROBINHOOD_CHAIN_ID = 4663;
/** docs/v6/phase1-scope.md §3.2 C8: robinhood.drpc.org refuses every pinned read since 2026-09-24 and is not a default. */
export const DEFAULT_RPC_URLS = ["https://rpc.mainnet.chain.robinhood.com", "https://rpc.ordofi.network"] as const;

export type Mode = "dry-run" | "send";
export type Caps = { maxGasPerTx: bigint; maxFeePerGasWei: bigint; maxWeiPerTx: bigint; maxWeiPerDay: bigint };
export type DrawRule = { everyS: number; offsetS: number; windowS: number };
/**
 * Where a draw's snapshot is published: the keeper's volume (always), a mirror (a plain PUT to `uploadUrl`, or an S3-compatible
 * bucket, `s3`, such as Cloudflare R2; at most one), and the public base the URI names.
 */
export type SnapshotPublish = { baseUrl: string | null; uploadUrl: string | null; uploadAuth: string | null; uploadReadback: boolean; s3?: S3Target | null };
/** A mirror is configured: the plain PUT or the S3 bucket. */
export const hasMirror = (s: Pick<SnapshotPublish, "uploadUrl" | "s3">): boolean => s.uploadUrl !== null || (s.s3 ?? null) !== null;
export const DEFAULT_S3_PREFIX = "rook-snapshots/";
export type ServiceConfig = {
  role: "keeper" | "resolver";
  mode: Mode;
  chainId: number;
  rpcUrls: string[];
  stackPath: string;
  stateDir: string;
  intervalS: number;
  port: number | null;
  publicUrl: string | null;
  floorSource: "ordinalswallet" | "fixture";
  floorFixture: string | null;
  /** the stack's collection slug → the Ordinals Wallet collection slug (default: the same slug) */
  owSlugs: Record<string, string>;
  caps: Caps;
  capMarginBps: bigint;
  rateMaxDisagreeBps: bigint;
  draw: DrawRule;
  receiptWaitS: number;
  once: boolean;
  /** drand evmnet relays; two must give the same signature (docs/audit/2026-09-26/fixes-W2-services.md §1) */
  drandUrls: string[];
  snapshots: SnapshotPublish;
  /** bearer token for /status/full; null: the full view is off (E-14) */
  statusToken: string | null;
  /** the keyless resolver inside this process (the keeper's), and its limits */
  resolver: { enabled: boolean; maxTxPerPass: number; maxGasPerTx: bigint };
  /** E-10: seconds the newest attempt may wait for a receipt before it is re-priced */
  stuckAfterS: number;
  /** E-04: draws and the resolver stop once a pass has run this long; the floor and caps always run first */
  passBudgetS: number;
  /** E-04: coins indexed per pass (a flood of launches is absorbed over several passes) */
  coinsPerPass: number;
  /** R2-S8: /healthz answers 503 once alerts have been in force this many passes in a row */
  alertPasses: number;
  /** R3-check L-G: how long an alert stays in force after the last pass that raised it (ms) */
  alertLatchMs: number;
};

export class ConfigError extends Error {
  constructor(readonly problems: string[]) { super(`refusing to start:\n  - ${problems.join("\n  - ")}`); this.name = "ConfigError"; }
}

export const DEFAULT_CAPS: Caps = {
  maxGasPerTx: 400_000n,
  maxFeePerGasWei: 1_000_000_000n,            // 1 gwei: 4663's base fee is ~0.01 gwei; a fee above this is refused, never quietly lowered
  maxWeiPerTx: 500_000_000_000_000n,          // 0.0005 ETH worst case per transaction
  maxWeiPerDay: 10_000_000_000_000_000n,      // 0.01 ETH per UTC day, worst case reserved at send, actual at the receipt
};
export const DEFAULT_DRAW: DrawRule = { everyS: 86_400, offsetS: 54_000, windowS: 3_000 };   // daily at 15:00 UTC; open within 50 min

const FLAGS = ["--dry-run", "--once"] as const;
export const DEFAULT_DRAND = ["https://api.drand.sh", "https://drand.cloudflare.com", "https://api2.drand.sh"] as const;
export const isLocal = (url: string): boolean => { try { return ["127.0.0.1", "localhost", "[::1]", "::1"].includes(new URL(url).hostname); } catch { return false; } };
export const hostOf = (url: string): string => { try { return new URL(url).host; } catch { return "invalid-url"; } };

type HttpsUrl = (name: string, v: string | undefined) => string | null;
/**
 * The S3-compatible snapshot bucket (Cloudflare R2: endpoint `https://<account>.r2.cloudflarestorage.com`, region `auto`), or null
 * when KEEPER_SNAPSHOT_S3_ENDPOINT is not set. Every other KEEPER_SNAPSHOT_S3_* then must be unset too.
 */
function s3Target(env: Readonly<Record<string, string | undefined>>, httpsUrl: HttpsUrl, problems: string[]): S3Target | null {
  const names = ["KEEPER_SNAPSHOT_S3_BUCKET", "KEEPER_SNAPSHOT_S3_PREFIX", "KEEPER_SNAPSHOT_S3_ACCESS_KEY_ID", "KEEPER_SNAPSHOT_S3_SECRET_ACCESS_KEY", "KEEPER_SNAPSHOT_S3_REGION"];
  if (!env.KEEPER_SNAPSHOT_S3_ENDPOINT) {
    for (const n of names) if (env[n]) problems.push(`${n} is set without KEEPER_SNAPSHOT_S3_ENDPOINT`);
    return null;
  }
  const n0 = problems.length;
  const endpoint = httpsUrl("KEEPER_SNAPSHOT_S3_ENDPOINT", env.KEEPER_SNAPSHOT_S3_ENDPOINT);
  if (endpoint) {
    const u = new URL(endpoint);
    if ((u.pathname !== "/" && u.pathname !== "") || u.search || u.username || u.password)
      problems.push("KEEPER_SNAPSHOT_S3_ENDPOINT is the S3 API's origin only (https://<account>.r2.cloudflarestorage.com): the bucket is KEEPER_SNAPSHOT_S3_BUCKET");
  }
  const bucket = env.KEEPER_SNAPSHOT_S3_BUCKET ?? "";
  if (!bucket) problems.push("KEEPER_SNAPSHOT_S3_ENDPOINT is set without KEEPER_SNAPSHOT_S3_BUCKET");
  else if (!/^[a-z0-9][a-z0-9.-]{1,61}[a-z0-9]$/.test(bucket)) problems.push(`KEEPER_SNAPSHOT_S3_BUCKET=${bucket} is not a bucket name (3-63 of a-z 0-9 . -)`);
  let prefix = (env.KEEPER_SNAPSHOT_S3_PREFIX ?? DEFAULT_S3_PREFIX).replace(/^\/+/, "");
  if (prefix && !prefix.endsWith("/")) prefix += "/";
  if (!/^([A-Za-z0-9._-]+\/)*$/.test(prefix) || prefix.split("/").some((x) => x === "." || x === ".."))
    problems.push(`KEEPER_SNAPSHOT_S3_PREFIX=${env.KEEPER_SNAPSHOT_S3_PREFIX} is not a key prefix (path segments of A-Z a-z 0-9 . _ -, e.g. ${DEFAULT_S3_PREFIX})`);
  const accessKeyId = env.KEEPER_SNAPSHOT_S3_ACCESS_KEY_ID ?? "";
  if (!accessKeyId) problems.push("KEEPER_SNAPSHOT_S3_ENDPOINT is set without KEEPER_SNAPSHOT_S3_ACCESS_KEY_ID");
  else if (!/^[A-Za-z0-9]{8,128}$/.test(accessKeyId)) problems.push("KEEPER_SNAPSHOT_S3_ACCESS_KEY_ID is not an access key id (letters and digits)");
  const secretAccessKey = env.KEEPER_SNAPSHOT_S3_SECRET_ACCESS_KEY ?? "";
  if (!secretAccessKey) problems.push("KEEPER_SNAPSHOT_S3_ENDPOINT is set without KEEPER_SNAPSHOT_S3_SECRET_ACCESS_KEY");
  else if (secretAccessKey.length < 16 || /\s/.test(secretAccessKey)) problems.push("KEEPER_SNAPSHOT_S3_SECRET_ACCESS_KEY does not look like a secret access key (at least 16 characters, no spaces)");
  const region = env.KEEPER_SNAPSHOT_S3_REGION || "auto";
  if (!/^[a-z0-9-]{2,32}$/.test(region)) problems.push(`KEEPER_SNAPSHOT_S3_REGION=${region} is not a region ("auto" for R2, the default)`);
  if (problems.length > n0 || !endpoint) return null;
  return { endpoint, bucket, prefix, accessKeyId, secretAccessKey, region };
}

/**
 * `role: "resolver"` is the standalone resolver (service/resolver-main.ts): it reads the same chain, RPC and stack variables, and
 * its own RESOLVER_MODE, RESOLVER_STATE_DIR, RESOLVER_INTERVAL_S and RESOLVER_STATUS_PORT; it needs no floor feed and no public URL.
 */
export function parseServiceConfig(argv: readonly string[], env: Readonly<Record<string, string | undefined>>, role: "keeper" | "resolver" = "keeper"): ServiceConfig {
  const problems: string[] = [];
  const R = role === "resolver";
  for (const a of argv) if (!(FLAGS as readonly string[]).includes(a)) problems.push(`unknown argument ${a}: the flags are ${FLAGS.join(", ")}; everything else is an environment variable`);
  const dryFlag = argv.includes("--dry-run"), once = argv.includes("--once");

  // the chain: explicit, or nothing starts
  const rawChain = env.KEEPER_CHAIN_ID;
  let chainId = 0;
  if (rawChain === undefined || rawChain === "") problems.push("KEEPER_CHAIN_ID is not set: the keeper signs for exactly one chain, named by the operator (4663 on Robinhood Chain)");
  else if (!/^[1-9]\d{0,15}$/.test(rawChain)) problems.push(`KEEPER_CHAIN_ID=${rawChain} is not a chain id`);
  else chainId = Number(rawChain);

  const modeVar = R ? "RESOLVER_MODE" : "KEEPER_MODE";
  const rawMode = env[modeVar] ?? "dry-run";
  if (rawMode !== "dry-run" && rawMode !== "send") problems.push(`${modeVar}=${rawMode}: it is "send" or "dry-run" (the default)`);
  const mode: Mode = dryFlag || rawMode !== "send" ? "dry-run" : "send";

  const rpcUrls = (env.KEEPER_RPC_URLS ?? DEFAULT_RPC_URLS.join(",")).split(",").map((s) => s.trim()).filter(Boolean);
  if (rpcUrls.length === 0) problems.push("KEEPER_RPC_URLS names no RPC");
  for (const u of rpcUrls) {
    let url: URL | null = null;
    try { url = new URL(u); } catch { problems.push(`an RPC in KEEPER_RPC_URLS is not a URL (${hostOf(u)})`); continue; }
    if (url.protocol !== "https:" && !(url.protocol === "http:" && isLocal(u))) problems.push(`the RPC at ${url.host} is not https (plain http only for this machine)`);
  }

  const stackPath = env.KEEPER_STACK ?? "";
  if (!stackPath) problems.push("KEEPER_STACK is not set: the stack file (deployments/stack-<chainid>.json) names what the keeper posts to");

  const num = (name: string, dflt: number, min: number, max: number): number => {
    const v = env[name];
    if (v === undefined || v === "") return dflt;
    const n = Number(v);
    if (!Number.isInteger(n) || n < min || n > max) { problems.push(`${name}=${v} is not a whole number in [${min}, ${max}]`); return dflt; }
    return n;
  };
  const big = (name: string, dflt: bigint, max: bigint): bigint => {
    const v = env[name];
    if (v === undefined || v === "") return dflt;
    if (!/^\d{1,30}$/.test(v)) { problems.push(`${name}=${v} is not a whole number of wei`); return dflt; }
    const n = BigInt(v);
    if (n === 0n || n > max) { problems.push(`${name}=${v} is outside (0, ${max}]`); return dflt; }
    return n;
  };
  const intervalS = num(R ? "RESOLVER_INTERVAL_S" : "KEEPER_INTERVAL_S", 60, 15, 1_800);
  const portRaw = R ? env.RESOLVER_STATUS_PORT : env.KEEPER_STATUS_PORT ?? env.PORT;
  const port = portRaw === undefined || portRaw === "" ? null : Number(portRaw);
  if (port !== null && (!Number.isInteger(port) || port < 1 || port > 65_535)) problems.push(`the status port ${portRaw} is not a port`);

  // E-14: every URL the keeper publishes under, uploads to or reads randomness from is https (plain http only for this machine)
  const httpsUrl = (name: string, v: string | undefined): string | null => {
    if (v === undefined || v === "") return null;
    const t = v.trim().replace(/\/$/, "");
    let u: URL;
    try { u = new URL(t); } catch { problems.push(`${name} is not a URL`); return null; }
    if (u.protocol !== "https:" && !(u.protocol === "http:" && isLocal(t))) { problems.push(`${name} must be https (plain http only for this machine)`); return null; }
    return t;
  };
  const publicUrl = R ? null : httpsUrl("KEEPER_PUBLIC_URL", env.KEEPER_PUBLIC_URL);
  const snapshots: SnapshotPublish = {
    baseUrl: R ? null : httpsUrl("KEEPER_SNAPSHOT_BASE_URL", env.KEEPER_SNAPSHOT_BASE_URL) ?? (publicUrl ? `${publicUrl}/snapshots` : null),
    uploadUrl: R ? null : httpsUrl("KEEPER_SNAPSHOT_UPLOAD_URL", env.KEEPER_SNAPSHOT_UPLOAD_URL),
    uploadAuth: R ? null : (env.KEEPER_SNAPSHOT_UPLOAD_AUTH || null),
    uploadReadback: (env.KEEPER_SNAPSHOT_UPLOAD_READBACK ?? "on") !== "off",
    s3: R ? null : s3Target(env, httpsUrl, problems),
  };
  if (!["on", "off"].includes(env.KEEPER_SNAPSHOT_UPLOAD_READBACK ?? "on")) problems.push(`KEEPER_SNAPSHOT_UPLOAD_READBACK=${env.KEEPER_SNAPSHOT_UPLOAD_READBACK}: "on" (the default: the mirror's object is read back too) or "off"`);
  if (snapshots.uploadAuth && !snapshots.uploadUrl) problems.push("KEEPER_SNAPSHOT_UPLOAD_AUTH is set without KEEPER_SNAPSHOT_UPLOAD_URL");
  if (snapshots.uploadUrl && env.KEEPER_SNAPSHOT_S3_ENDPOINT) problems.push("KEEPER_SNAPSHOT_UPLOAD_URL and KEEPER_SNAPSHOT_S3_ENDPOINT are both set: the snapshot has one mirror, a plain PUT or an S3 bucket");
  // E-11, audit R2-S5: on 4663 a mirrored snapshot is NAMED by the mirror, never by the hot key's host: the URI on chain must not
  // point at the machine that holds the keeper key
  if (!R && chainId === ROBINHOOD_CHAIN_ID && (snapshots.uploadUrl || env.KEEPER_SNAPSHOT_S3_ENDPOINT)) {
    const which = snapshots.uploadUrl ? "KEEPER_SNAPSHOT_UPLOAD_URL" : "KEEPER_SNAPSHOT_S3_ENDPOINT";
    if (!env.KEEPER_SNAPSHOT_BASE_URL) problems.push(`${which} is set on 4663 without KEEPER_SNAPSHOT_BASE_URL: the draw's URI would name the keeper's own host; set the mirror's public base`);
    else if (snapshots.baseUrl && publicUrl && hostOf(snapshots.baseUrl) === hostOf(publicUrl))
      problems.push("KEEPER_SNAPSHOT_BASE_URL is on KEEPER_PUBLIC_URL's host: on 4663 the draw's URI must name the mirror, not the keeper's host");
  }
  // the S3 API answers only signed requests: the draw's URI names the bucket's PUBLIC address (R2's public dev URL or custom domain)
  if (snapshots.s3 && snapshots.baseUrl && hostOf(snapshots.baseUrl) === hostOf(snapshots.s3.endpoint))
    problems.push("KEEPER_SNAPSHOT_BASE_URL is the S3 API's host, which serves only signed requests: set the bucket's public URL (e.g. https://pub-<id>.r2.dev/<prefix> or a custom domain)");
  const drandUrls = (env.KEEPER_DRAND_URLS ?? DEFAULT_DRAND.join(",")).split(",").map((x) => x.trim()).filter(Boolean)
    .map((u, i) => httpsUrl(`KEEPER_DRAND_URLS entry ${i + 1}`, u)).filter((u): u is string => u !== null);
  if (new Set(drandUrls.map(hostOf)).size < 2) problems.push("KEEPER_DRAND_URLS must name at least two different drand relays (two must agree on every signature)");
  const statusToken = env.KEEPER_STATUS_TOKEN || null;
  if (statusToken !== null && statusToken.length < 32) problems.push("KEEPER_STATUS_TOKEN must be at least 32 characters (it guards /status/full)");
  const rawResolver = env.KEEPER_RESOLVER ?? "on";
  if (rawResolver !== "on" && rawResolver !== "off") problems.push(`KEEPER_RESOLVER=${rawResolver}: "on" (the default) or "off"`);

  const floorSource = R ? "fixture-unused" : env.KEEPER_FLOOR_SOURCE ?? "ordinalswallet";
  if (!R && floorSource !== "ordinalswallet" && floorSource !== "fixture") problems.push(`KEEPER_FLOOR_SOURCE=${floorSource}: ordinalswallet (the default) or fixture`);
  const floorFixture = env.KEEPER_FLOOR_FIXTURE ?? null;
  if (floorSource === "fixture" && !floorFixture) problems.push("KEEPER_FLOOR_SOURCE=fixture needs KEEPER_FLOOR_FIXTURE (a JSON file of fixed floors)");
  if (floorSource === "fixture" && chainId === ROBINHOOD_CHAIN_ID && mode === "send")
    problems.push("a fixture floor is never SENT to Robinhood Chain (4663): the floor must come from the agreed feed");

  const owSlugs: Record<string, string> = {};
  for (const pair of (env.KEEPER_OW_SLUGS ?? "").split(",").map((s) => s.trim()).filter(Boolean)) {
    const m = /^([a-z0-9-]{1,40})=([a-z0-9-]{1,64})$/.exec(pair);
    if (!m) { problems.push(`KEEPER_OW_SLUGS entry "${pair}" is not <stack slug>=<Ordinals Wallet slug>`); continue; }
    owSlugs[m[1]] = m[2];
  }

  const caps: Caps = {
    maxGasPerTx: big("KEEPER_MAX_GAS_PER_TX", DEFAULT_CAPS.maxGasPerTx, 30_000_000n),
    maxFeePerGasWei: big("KEEPER_MAX_FEE_PER_GAS_WEI", DEFAULT_CAPS.maxFeePerGasWei, 1_000_000_000_000n),
    maxWeiPerTx: big("KEEPER_MAX_WEI_PER_TX", DEFAULT_CAPS.maxWeiPerTx, 10n ** 18n),
    maxWeiPerDay: big("KEEPER_MAX_WEI_PER_DAY", DEFAULT_CAPS.maxWeiPerDay, 10n ** 19n),
  };
  if (caps.maxWeiPerTx > caps.maxWeiPerDay) problems.push("KEEPER_MAX_WEI_PER_TX is above KEEPER_MAX_WEI_PER_DAY");

  const capMarginBps = BigInt(num("KEEPER_CAP_MARGIN_BPS", 1_000, 0, 5_000));
  const rateMaxDisagreeBps = BigInt(num("KEEPER_RATE_MAX_DISAGREE_BPS", 100, 1, 1_000));
  const draw: DrawRule = { everyS: num("KEEPER_DRAW_EVERY_S", DEFAULT_DRAW.everyS, 300, 30 * 86_400), offsetS: num("KEEPER_DRAW_OFFSET_S", DEFAULT_DRAW.offsetS, 0, 30 * 86_400),
    windowS: num("KEEPER_DRAW_WINDOW_S", DEFAULT_DRAW.windowS, 60, 3_300) };
  if (draw.offsetS >= draw.everyS) problems.push("KEEPER_DRAW_OFFSET_S must be under KEEPER_DRAW_EVERY_S");
  const receiptWaitS = num("KEEPER_RECEIPT_WAIT_S", 90, 5, 900);
  const stateDir = (R ? env.RESOLVER_STATE_DIR : env.KEEPER_STATE_DIR) ?? (R ? "./resolver-state" : "./state");
  const resolver = { enabled: R || rawResolver === "on", maxTxPerPass: num("KEEPER_RESOLVER_MAX_TX", 10, 1, 100),
    maxGasPerTx: big("KEEPER_RESOLVER_MAX_GAS_PER_TX", 900_000n, 30_000_000n) };
  const stuckAfterS = num("KEEPER_STUCK_AFTER_S", 180, 30, 3_600);
  const passBudgetS = num("KEEPER_PASS_BUDGET_S", 240, 30, 1_800);
  const coinsPerPass = num("KEEPER_COINS_PER_PASS", 200, 1, 10_000);
  const alertPasses = num("KEEPER_ALERT_PASSES", 3, 1, 100);
  // R4-check L-S2: never shorter than the latch that still pages a one-pass alert at this interval (keeper and resolver alike)
  const alertLatchMs = Math.max(num("KEEPER_ALERT_LATCH_S", 1_200, 0, 86_400) * 1000, alertLatchFloorMs(intervalS, alertPasses));

  if (problems.length) throw new ConfigError(problems);
  return { role, mode, chainId, rpcUrls, stackPath, stateDir, intervalS, port, publicUrl, floorSource: floorSource as ServiceConfig["floorSource"], floorFixture,
    owSlugs, caps, capMarginBps, rateMaxDisagreeBps, draw, receiptWaitS, once, drandUrls, snapshots, statusToken, resolver, stuckAfterS, passBudgetS, coinsPerPass, alertPasses, alertLatchMs };
}
