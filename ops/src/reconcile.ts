import { createHash } from "node:crypto";
import { closeSync, existsSync, fsyncSync, mkdirSync, openSync, readFileSync, renameSync, writeSync } from "node:fs";
import { createServer, type Server } from "node:http";
import { join } from "node:path";
import { rpcOf, type Rpc } from "../../indexer/src/rpc.js";
import { loadConfig } from "../../indexer/src/wrapped/config.js";
import { WrappedService } from "../../indexer/src/wrapped/service.js";
import type { ReserveReport } from "../../indexer/types.js";
import { format, Telegram } from "../../monitor/src/alerts.mjs";
import { backingStatus, readBacking, type BackingRead } from "./backing.js";
import { RecordsCache } from "./ethRecords.js";
import { buyer, IC_HOST } from "./ic.js";
import { agreeingOrdFetch } from "./location.js";
import { logger, type Log } from "./log.js";
import { capsStatus, compareCaps, type CapsRead } from "./stage.js";
import { parseSolRpcUrls, solReader } from "./solread.js";
import { locator, pinRefs, readBooksReserves } from "./booksreserves.js";

/**
 * docs/v6/p1-services.md §3 (K7, gate G5). The reserves reconciliation, run hourly against 4663 and the Bitcoin providers: the
 * indexer's own `WrappedService` (indexer/src/wrapped/, unchanged) over the stack file — a valid WRAPPED_CONFIG by design — plus
 * the canister's `custody_caps()` and the stage it implies. A report older than RECONCILE_STALE_S (3 h) is RED, judged when it is
 * served, so a hung job turns red without running. Red is alerted through the monitor's Telegram channel (monitor/src/alerts.mjs):
 * on the transition, on a change of reasons, every RECONCILE_REALERT_S while it stays red, and once on recovery.
 * It holds no key: 4663 is read through a read-only RPC, the canister anonymously (a query), Bitcoin by GET.
 *
 * Wave 2 (docs/audit/2026-09-26/fixes-W2-monitor.md §2): the module's real ETH against its own books, its units and its floor, and
 * a stale floor, are RED (E-05); an inscription's location needs two ordinals providers agreeing (E-13, src/location.ts); a
 * reconciler that cannot read, or stays unverified for 6 hours, pages (E-08a); the watchdog is its own timer and a run is
 * bounded; Telegram is required (E-08b), and `--test-alert` tests THIS service's channel.
 *
 * THE HOME (docs/v9/canister-only-solana.md §10.2, W11): ROOK_HOME=books (the default; HOME=books is read too when HOME holds exactly
 * that word) reads the canister's own books (`books_view` and the coin and desk queries, src/booksreserves.ts) and checks them against
 * Solana at `finalized` instead of the 4663 stack: the same report, the same backing arithmetic (src/backing.ts, in lamports), the same
 * alerts and page. ROOK_HOME=robinhood keeps the 4663 reconciliation unchanged. The programs home (`solana`, rook_core's accounts) is
 * retired with rook_core (decision D-l).
 *   SOL_RPC_URLS (required; two independent providers, https, comma-separated: every account must be identical at each that answers)
 *   CANISTER_ID (required: the canister whose queries ARE the books)
 *   RECONCILE_LAUNCH_FILE (default deployments/launch/canister-only/pins.json: the collections' names by index)
 *   RECONCILE_ETH_RPC_URLS (default two keyless Ethereum RPCs: ownerOf of every held Ethereum NFT; "" turns it off: those items are unverified)
 *   RECONCILE_RH_RPC_URLS (2026-10-12; default two keyless Robinhood Chain RPCs: ownerOf of every held Robinhood Chain NFT, read on chain 4663;
 *     "" turns it off: those items are unverified)
 */
export type Status = "green" | "red" | "unverified";
/** What the report, the page and the alerts read of a collection: the indexer's ReserveReport on 4663, booksreserves.ts on the books home. */
export type CollectionReport = Pick<ReserveReport, "slug" | "name" | "symbol" | "status" | "reasons" | "block">
  & { equation: Pick<ReserveReport["equation"], "itemsInCustody" | "itemsUnverified" | "itemsElsewhere" | "ticketsInFlight"> };
export type Reconciliation = { version: 1; computedAt: number; status: Status; reasons: string[]; block: number | null; chainId: number;
  collections: CollectionReport[]; caps: CapsRead; stage: string; backing?: BackingRead };
/** The Solana home's chain id in the report (Relay's id for Solana): its `block` is a finalized slot. */
export const SOLANA_CHAIN_ID = 792_703_809;
export type Served = Reconciliation & { ageS: number; stale: boolean; servedStatus: Status; servedReasons: string[] };

const rank: Record<Status, number> = { green: 0, unverified: 1, red: 2 };
const worst = (xs: Status[]): Status => xs.reduce<Status>((a, b) => (rank[b] > rank[a] ? b : a), "green");

/** R2-L5: no collection was read at all: no report, or every report came back without a 4663 block (every RPC failed). */
export const readNothing = (collections: Pick<ReserveReport, "block">[]): boolean => collections.length === 0 || collections.every((c) => c.block === null);

/** The report as served at `nowS`: stale past `staleS` is red, whatever it said when it was computed. Pure. */
export function served(r: Reconciliation | null, nowS: number, staleS: number, chainId = 0): Served {
  if (!r) return { version: 1, computedAt: 0, status: "unverified", reasons: ["no reconciliation has run yet"], block: null, chainId, collections: [],
    caps: { kind: "absent", note: "not read yet", stackStage: 0, at: 0 }, stage: "unknown", ageS: 0, stale: false, servedStatus: "unverified", servedReasons: ["no reconciliation has run yet"] };
  const ageS = Math.max(0, nowS - r.computedAt), stale = ageS > staleS;
  return { ...r, ageS, stale, servedStatus: stale ? "red" : r.status,
    servedReasons: stale ? [`stale: last computed ${new Date(r.computedAt * 1000).toISOString()}, ${Math.round(ageS / 60)} min ago (over ${staleS / 3600} h)`, ...r.reasons] : r.reasons };
}

/** Collections, caps and the module's real ETH into one status. Pure. */
export function combine(collections: CollectionReport[], caps: CapsRead, computedAt: number, chainId: number, backing?: BackingRead,
  o: { minBackingBps?: number; staleFloorRedS?: number; advanceStaleS?: number } = {}): Reconciliation {
  const cs = capsStatus(caps);
  const bs = backing ? backingStatus(backing, computedAt, o) : { status: "green" as Status, reasons: [] };
  const status = worst([...collections.map((c) => c.status as Status), cs.status, bs.status]);
  const reasons = [...collections.flatMap((c) => (c.status === "green" ? [] : c.reasons.map((x) => `${c.slug}: ${x}`))), ...cs.reasons, ...bs.reasons];
  const block = collections.find((c) => c.block !== null)?.block ?? null;
  return { version: 1, computedAt, status, reasons, block, chainId, collections, caps, stage: caps.kind === "read" ? caps.stage.label : "unknown", ...(backing ? { backing } : {}) };
}

// ---- alerts: when to say what ------------------------------------------------------------------------------------------------------

export type AlertState = { lastStatus: Status | null; redKey: string | null; redAt: number; unverifiedSince?: number | null; unverifiedAt?: number };
export type Alert = { kind: string; key: string; sev: "SEV1" | "SEV2" | "INFO"; text: string };

/**
 * Pure. Red: alert when it becomes red, when its reasons change, and every `realertMs` while it stays red. Leaving red: one
 * recovery message. Unverified: a run that could read NO collection pages at once ("cannot read"); a partial unverified pages once
 * it has lasted `unverifiedMs` (a provider's bad hour is not an incident, a silent week is: audit E-08a); both repeat every
 * `realertMs` while they last.
 */
export function alertsFor(prev: AlertState, s: Served, nowMs: number, realertMs: number, where: string, unverifiedMs = 6 * 3_600_000): { alerts: Alert[]; next: AlertState } {
  if (s.servedStatus === "unverified") {
    const since = prev.lastStatus === "unverified" && prev.unverifiedSince ? prev.unverifiedSince : nowMs;
    const blind = readNothing(s.collections);
    const due = (blind || nowMs - since >= unverifiedMs) && nowMs - (prev.unverifiedAt ?? 0) >= realertMs;
    const next: AlertState = { ...prev, lastStatus: "unverified", redKey: null, redAt: 0, unverifiedSince: since, unverifiedAt: due ? nowMs : prev.unverifiedAt ?? 0 };
    if (!due) return { alerts: [], next };
    const text = [`reserves UNVERIFIED (${where}) — ${blind ? "the reconciler CANNOT READ: no collection could be read" : `for ${Math.round((nowMs - since) / 3_600_000)} h`}`,
      ...s.servedReasons.slice(0, 12).map((r) => `• ${r}`), `computed ${s.computedAt ? new Date(s.computedAt * 1000).toISOString() : "never"}; ${s.stage}`,
      "Unverified is not green: the reserves page cannot vouch for the backing until this is read."].join("\n");
    return { alerts: [{ kind: "reserves", key: `reserves:unverified:${since}:${nowMs}`, sev: "SEV2", text }], next };
  }
  if (s.servedStatus === "red") {
    const key = `reserves:red:${createHash("sha256").update(s.servedReasons.map((r) => r.replace(/\d+ min ago/, "")).sort().join("\n")).digest("hex").slice(0, 16)}`;
    const itemRed = s.collections.some((c) => c.status === "red");
    const due = key !== prev.redKey || nowMs - prev.redAt >= realertMs;
    const text = [`reserves RED (${where})${key === prev.redKey ? " — still red" : ""}`, ...s.servedReasons.slice(0, 12).map((r) => `• ${r}`),
      `computed ${s.computedAt ? new Date(s.computedAt * 1000).toISOString() : "never"}; ${s.stage}`].join("\n");
    return due ? { alerts: [{ kind: "reserves", key, sev: itemRed ? "SEV1" : "SEV2", text }], next: { lastStatus: "red", redKey: key, redAt: nowMs } }
      : { alerts: [], next: { ...prev, lastStatus: "red" } };
  }
  if (prev.lastStatus === "red" || (prev.lastStatus === "unverified" && prev.unverifiedAt))
    return { alerts: [{ kind: "reserves", key: `reserves:recovered:${nowMs}`, sev: "INFO", text: `reserves back to ${s.servedStatus} (${where})` }], next: { lastStatus: s.servedStatus, redKey: null, redAt: 0, unverifiedSince: null, unverifiedAt: 0 } };
  return { alerts: [], next: { ...prev, lastStatus: s.servedStatus, unverifiedSince: null } };
}

export interface Channel { send(text: string): Promise<void>; }

// ---- the read side ---------------------------------------------------------------------------------------------------------------

/** A read-only RPC over several endpoints, each asked its chain id first; the indexer's `rpcOf` refuses anything that is not a read. */
export async function pinnedRpc(urls: readonly string[], chainId: number, log: Log, fetchImpl: typeof fetch = fetch): Promise<Rpc> {
  const live: Rpc[] = [];
  for (const u of urls) {
    const r = rpcOf(u, fetchImpl);
    try {
      const id = Number(BigInt(await r<string>("eth_chainId", [])));
      if (id === chainId) live.push(r); else log("error", "rpc-wrong-chain", { host: new URL(u).host, reports: id, expected: chainId });
    } catch (err) { log("warn", "rpc-unreachable", { host: new URL(u).host, error: String((err as Error).message).slice(0, 120) }); }
  }
  if (!live.length) throw new Error(`no RPC reports chain id ${chainId}`);
  return (async <T,>(method: string, params: unknown[]): Promise<T> => {
    let last: unknown;
    for (const r of live) { try { return await r<T>(method, params); } catch (err) { last = err; if ((err as { kind?: string }).kind === "rpc" || (err as { kind?: string }).kind === "refused") throw err; } }
    throw last;
  }) as Rpc;
}

export type ReconcileConfig = { stackPath: string; chainId: number; rpcUrls: string[]; canisterId: string | null; icHost: string; everyS: number; staleS: number;
  realertS: number; stateDir: string; port: number | null; publicUrl: string | null; unverifiedAlertS?: number; runTimeoutS?: number; minBackingBps?: number; staleFloorRedS?: number; advanceStaleS?: number;
  /** docs/v9 §5: the home; the Solana fields are read only on the books home */
  home?: "books" | "robinhood"; solRpcUrls?: string[]; launchFile?: string; ethRpcUrls?: string[]; rhRpcUrls?: string[] };

export const RECONCILE_PINS_DEFAULT = new URL("../../deployments/launch/canister-only/pins.json", import.meta.url).pathname;
export const ETH_RPC_KEYLESS = ["https://ethereum-rpc.publicnode.com", "https://eth.drpc.org"];
/** 2026-10-12: Robinhood Chain's (4663) keyless RPCs, for the ownerOf of its own NFTs (the two the repository already reads 4663 through). */
export const RH_RPC_KEYLESS = ["https://rpc.mainnet.chain.robinhood.com", "https://rpc.ordofi.network"];

export function parseReconcileConfig(env: Readonly<Record<string, string | undefined>>): ReconcileConfig {
  const problems: string[] = [];
  const hv = (env.ROOK_HOME ?? (["robinhood", "books"].includes(String(env.HOME).toLowerCase()) ? env.HOME : undefined) ?? "books").toLowerCase();
  if (hv !== "robinhood" && hv !== "books") problems.push(`ROOK_HOME=${env.ROOK_HOME} is neither books nor robinhood${hv === "solana" ? " (the programs home is retired with rook_core: books)" : ""}`);
  // W11: `books` is the canister-only Solana home (docs/v9/canister-only-solana.md): the books are the canister's queries, no rook_core
  const home = hv === "robinhood" ? "robinhood" as const : "books" as const;
  const n = (k: string, d: number, lo: number, hi: number) => { const v = env[k]; if (v === undefined || v === "") return d; const x = Number(v); if (!Number.isInteger(x) || x < lo || x > hi) problems.push(`${k}=${v} is not in [${lo}, ${hi}]`); return x; };
  const stackPath = env.RECONCILE_STACK ?? env.WRAPPED_CONFIG ?? "";
  if (!stackPath && home === "robinhood") problems.push("RECONCILE_STACK is not set: the stack file (deployments/stack-<chainid>.json) names what is reconciled");
  const chainRaw = home !== "robinhood" ? String(SOLANA_CHAIN_ID) : env.RECONCILE_CHAIN_ID ?? "";
  if (!/^[1-9]\d{0,15}$/.test(chainRaw)) problems.push("RECONCILE_CHAIN_ID is not set (or not a chain id): the chain is named, never assumed");
  // docs/v9 §5: the books home's Solana reads
  const solRpcUrls = home === "books" ? parseSolRpcUrls(env.SOL_RPC_URLS, problems) : [];
  if (home === "books" && !solRpcUrls.length) problems.push("SOL_RPC_URLS is not set: two independent Solana providers (decision D-h: Helius free + PublicNode or Ankr)");
  const ethRpcUrls = env.RECONCILE_ETH_RPC_URLS === undefined ? ETH_RPC_KEYLESS : parseSolRpcUrls(env.RECONCILE_ETH_RPC_URLS, problems, "RECONCILE_ETH_RPC_URLS");
  const rhRpcUrls = env.RECONCILE_RH_RPC_URLS === undefined ? RH_RPC_KEYLESS : parseSolRpcUrls(env.RECONCILE_RH_RPC_URLS, problems, "RECONCILE_RH_RPC_URLS");
  const rpcUrls = (env.RECONCILE_RPC_URLS ?? "https://rpc.mainnet.chain.robinhood.com,https://rpc.ordofi.network").split(",").map((s) => s.trim()).filter(Boolean);
  const canisterId = env.CANISTER_ID || null;
  if (canisterId && !/^[a-z0-9]{5}(-[a-z0-9]{5}){3}-[a-z0-9]{3}$/.test(canisterId)) problems.push(`CANISTER_ID "${canisterId}" is not a canister id`);
  if (home === "books" && !canisterId) problems.push("CANISTER_ID is not set: on the books home the canister's queries ARE the books");
  const everyS = n("RECONCILE_EVERY_S", 3_600, 60, 3_600), staleS = n("RECONCILE_STALE_S", 10_800, 600, 86_400), realertS = n("RECONCILE_REALERT_S", 21_600, 600, 7 * 86_400);
  const unverifiedAlertS = n("RECONCILE_UNVERIFIED_ALERT_S", 21_600, 600, 7 * 86_400), runTimeoutS = n("RECONCILE_RUN_TIMEOUT_S", 900, 60, 3_600);
  const minBackingBps = n("RECONCILE_MIN_BACKING_BPS", 5_000, 0, 10_000), staleFloorRedS = n("RECONCILE_STALE_FLOOR_RED_S", 3_600, 0, 7 * 86_400);
  const advanceStaleS = n("RECONCILE_ADVANCE_STALE_S", 48 * 3_600, 3_600, 30 * 86_400);   // docs/v6/relay-conversion.md §6
  const p = env.PORT ?? env.RECONCILE_PORT;
  const port = p === undefined || p === "" ? null : Number(p);
  if (problems.length) throw new Error(`refusing to start:\n  - ${problems.join("\n  - ")}`);
  return { stackPath, chainId: Number(chainRaw), rpcUrls, canisterId, icHost: env.IC_HOST ?? IC_HOST, everyS, staleS, realertS, stateDir: env.RECONCILE_STATE_DIR ?? "./state",
    port, publicUrl: env.RECONCILE_PUBLIC_URL ?? null, unverifiedAlertS, runTimeoutS, minBackingBps, staleFloorRedS, advanceStaleS,
    home, solRpcUrls, launchFile: env.RECONCILE_LAUNCH_FILE || RECONCILE_PINS_DEFAULT, ethRpcUrls, rhRpcUrls };
}

/** The last report and the alert state, kept on disk so a restart serves the real age instead of forgetting it. */
export class Store {
  readonly path: string;
  constructor(dir: string) { mkdirSync(dir, { recursive: true }); this.path = join(dir, "reconcile.json"); }
  load(): { last: Reconciliation | null; alerts: AlertState } {
    if (!existsSync(this.path)) return { last: null, alerts: { lastStatus: null, redKey: null, redAt: 0 } };
    return JSON.parse(readFileSync(this.path, "utf8"));
  }
  save(v: { last: Reconciliation | null; alerts: AlertState }): void {
    const tmp = `${this.path}.tmp`, fd = openSync(tmp, "w");
    try { writeSync(fd, JSON.stringify(v, (_k, x) => (typeof x === "bigint" ? String(x) : x))); fsyncSync(fd); } finally { closeSync(fd); }
    renameSync(tmp, this.path);
  }
}

export type Deps = { cfg: ReconcileConfig; service: { reserves(): Promise<CollectionReport[]> }; caps: () => Promise<CapsRead>; channel: Channel; store: Store; log: Log;
  nowMs?: () => number; stackStage: number; backing?: () => Promise<BackingRead> };

/** One run: read, combine, persist, alert. */
export async function runOnce(d: Deps): Promise<Served> {
  const now = d.nowMs ?? Date.now;
  let collections: CollectionReport[] = [];
  try { collections = await d.service.reserves(); } catch (err) { d.log("error", "reserves-unread", { error: String((err as Error).message).slice(0, 200) }); }
  const caps = await d.caps();
  const backing = d.backing ? await d.backing() : undefined;
  const r = combine(collections, caps, Math.floor(now() / 1000), d.cfg.chainId, backing, d.cfg);
  // R2-L5: a run in which NOTHING was read (no report, or every report without a 4663 block) is blind, and pages at once
  if (readNothing(collections)) { r.status = r.status === "red" ? "red" : "unverified"; r.reasons.unshift(`no collection could be read (no ${d.cfg.home === "books" ? "Solana" : "4663"} read succeeded)`); }
  const st = d.store.load();
  st.last = r;
  d.store.save(st);
  d.log(r.status === "red" ? "error" : "info", "reconciled", { status: r.status, reasons: r.reasons.length, block: r.block, stage: r.stage, collections: collections.map((c) => `${c.slug}:${c.status}`).join(",") });
  return await alert(d, now());
}

/** Judge the stored report as served now, and send what is due. A failed post is not recorded: it is tried again next time. */
export async function alert(d: Pick<Deps, "cfg" | "channel" | "store" | "log">, nowMs: number): Promise<Served> {
  const st = d.store.load();
  const s = served(st.last, Math.floor(nowMs / 1000), d.cfg.staleS, d.cfg.chainId);
  const { alerts, next } = alertsFor(st.alerts, s, nowMs, d.cfg.realertS * 1000, d.cfg.publicUrl ?? "the reconciler", (d.cfg.unverifiedAlertS ?? 21_600) * 1000);
  for (const a of alerts) {
    try { await d.channel.send(format(a)); d.log("info", "alert-sent", { key: a.key, sev: a.sev }); }
    catch (err) { d.log("error", "alert-failed", { key: a.key, error: String((err as Error).message).slice(0, 120) }); return s; }
  }
  st.alerts = next;
  d.store.save(st);
  return s;
}

// ---- the page ---------------------------------------------------------------------------------------------------------------------

const fmt = (w: bigint | string, decimals: 9 | 18 = 18) => decimals === 9 ? `${(Number(BigInt(w) / 1_000n) / 1e6).toString()}` : `${(Number(BigInt(w) / 10n ** 12n) / 1e6).toString()}`;
const esc = (s: unknown) => String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]!);
export function page(s: Served): string {
  const color = { green: "#1a7f37", red: "#cf222e", unverified: "#9a6700" }[s.servedStatus];
  const caps = s.caps.kind === "read"
    ? `<p>custody_caps(): <b>${esc(s.caps.stage.label)}</b> (stack: stage ${s.caps.stackStage}). Global ${s.caps.policy.global_max_items} items, ${esc(s.caps.policy.global_max_value_wei)} wei.</p>
       <table><tr><th>collection</th><th>max items</th><th>max wei</th><th>held</th><th>in flight</th><th>items left</th></tr>${s.caps.policy.collections.map((c) => {
         const h = (s.caps as { headroom: { collection: string; items_held: number; items_in_flight: number; items_left: number }[] }).headroom.find((x) => x.collection === c.collection);
         return `<tr><td>${esc(c.collection)}</td><td>${c.max_items}</td><td>${esc(c.max_value_wei)}</td><td>${h?.items_held ?? "?"}</td><td>${h?.items_in_flight ?? "?"}</td><td>${h?.items_left ?? "?"}</td></tr>`; }).join("")}</table>`
    : `<p>custody_caps(): ${esc(s.caps.kind === "error" ? s.caps.error : s.caps.note)}</p>`;
  return `<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Rook reserves</title>
<style>body{font:15px/1.5 system-ui,sans-serif;margin:0 auto;max-width:900px;padding:16px;background:#fff;color:#111}@media (prefers-color-scheme:dark){body{background:#0d1117;color:#e6edf3}}
.b{padding:12px 16px;border-radius:8px;color:#fff;background:${color};font-weight:600}table{border-collapse:collapse;width:100%;overflow-x:auto;display:block}td,th{padding:4px 8px;border-bottom:1px solid #8884;text-align:left}</style></head><body>
<div class="b">${esc(s.servedStatus.toUpperCase())} — computed ${s.computedAt ? esc(new Date(s.computedAt * 1000).toISOString()) : "never"} (${Math.round(s.ageS / 60)} min ago)${s.stale ? " — STALE" : ""}</div>
<p>${s.chainId === SOLANA_CHAIN_ID ? "Solana slot (finalized)" : "4663 block"} ${esc(s.block)} · chain ${s.chainId} · red when older than 3 hours · JSON at <a href="/v1/reserves">/v1/reserves</a></p>
${s.servedReasons.length ? `<ul>${s.servedReasons.map((r) => `<li>${esc(r)}</li>`).join("")}</ul>` : "<p>Every item was found where it should be, every read is fresh, and the equation holds.</p>"}
${caps}
${s.collections.map((c) => `<h2>${esc(c.name)} (${esc(c.symbol)}): ${esc(c.status)}</h2><p>items in custody ${c.equation.itemsInCustody}, unverified ${c.equation.itemsUnverified}, elsewhere ${c.equation.itemsElsewhere}, tickets in flight ${c.equation.ticketsInFlight}</p>`).join("")}
${s.backing?.kind === "read" ? `<h2>The floor modules' ${s.backing.modules[0]?.decimals === 9 ? "SOL (slot" : "ETH (block"} ${s.backing.modules[0]?.block ?? "?"})</h2><table><tr><th>collection</th><th>balance</th><th>owed by its books</th><th>backing</th><th>reserve</th><th>surplus</th><th>revenue</th><th>ETH-backed units</th><th>floor</th><th>fresh</th></tr>${s.backing.modules.map((m) =>
  `<tr><td>${esc(m.slug)}</td><td>${fmt(m.balance, m.decimals)}</td><td>${fmt(m.owed, m.decimals)}</td><td>${fmt(m.books.backing, m.decimals)}</td><td>${fmt(m.books.reserve, m.decimals)}</td><td>${fmt(m.books.surplus, m.decimals)}</td><td>${fmt(m.books.revenue, m.decimals)}</td><td>${fmt(m.ethBacked, m.decimals)}</td><td>${fmt(m.accepted, m.decimals)}</td><td>${m.escaped ? "ESCAPED" : m.fresh ? "yes" : "STALE"}</td></tr>`).join("")}</table>` : s.backing ? `<p>FloorModule: ${esc(s.backing.kind === "error" ? s.backing.error : s.backing.note)}</p>` : ""}
</body></html>`;
}

/** `records`: docs/v8/proof-of-reserves.md P4, the canister's own records by slug at /v1/eth-records (src/ethRecords.ts); absent: not served. */
export function serve(get: () => Served, port: number, records?: () => Promise<unknown>): Server {
  return createServer(async (req, res) => {
    const path = (req.url ?? "/").split("?")[0], s = get();
    const json = (code: number, v: unknown) => { res.writeHead(code, { "content-type": "application/json", "cache-control": "no-store", "access-control-allow-origin": "*" }); res.end(JSON.stringify(v, (_k, x) => (typeof x === "bigint" ? String(x) : x))); };
    if (req.method !== "GET") { res.writeHead(405); res.end(); return; }
    if (path === "/healthz") return json(200, { ok: true, status: s.servedStatus, ageS: s.ageS });
    if (path === "/v1/reserves") return json(200, s);
    if (path === "/v1/eth-records" && records) { try { return json(200, await records()); } catch (err) { return json(503, { error: String((err as Error).message).slice(0, 200) }); } }
    if (path === "/") { res.writeHead(200, { "content-type": "text/html; charset=utf-8", "cache-control": "no-store" }); res.end(page(s)); return; }
    res.writeHead(404); res.end();
  }).listen(port, "::");
}

/** A run that has not finished in `ms` is abandoned (logged; the report ages, and the watchdog turns it red). */
export async function bounded<T>(p: Promise<T>, ms: number): Promise<T> {
  let t: NodeJS.Timeout | undefined;
  try { return await Promise.race([p, new Promise<never>((_ok, no) => { t = setTimeout(() => no(new Error(`the run did not finish in ${Math.round(ms / 1000)} s`)), ms); })]); }
  finally { clearTimeout(t); }
}

/** `--test-alert`: one RED, one UNVERIFIED and one recovery, through THIS service's channel and variables (E-15 item 2). */
export async function testAlerts(channel: Channel, where: string, nowMs = Date.now()): Promise<number> {
  const at = Math.floor(nowMs / 1000), base = served(null, at, 10_800, 4663);
  const red = { ...base, computedAt: at, servedStatus: "red" as Status, servedReasons: ["frogs: 1 item found outside custody (TEST SAMPLE)"], stage: "stage 0" };
  const unv = { ...base, computedAt: at, servedStatus: "unverified" as Status, collections: [], servedReasons: ["no collection could be read (TEST SAMPLE)"], stage: "stage 0" };
  const msgs = [alertsFor({ lastStatus: null, redKey: null, redAt: 0 }, red, nowMs, 1, where).alerts[0],
    alertsFor({ lastStatus: null, redKey: null, redAt: 0 }, unv, nowMs, 1, where).alerts[0],
    alertsFor({ lastStatus: "red", redKey: "k", redAt: 0 }, { ...red, servedStatus: "green" as Status, servedReasons: [] }, nowMs, 1, where).alerts[0]];
  for (const a of msgs) await channel.send(format(a, { test: true }));
  return msgs.length;
}

/** The canister's custody_caps() against the stage the launch (or stack) file names; anonymous, a query. */
function capsOf(b: Awaited<ReturnType<typeof buyer>> | null, stackStage: number): () => Promise<CapsRead> {
  return async () => {
    const at = Math.floor(Date.now() / 1000);
    if (!b) return { kind: "absent", note: "CANISTER_ID is not set: custody_caps() is not read, so the caps and the stage are unverified", stackStage, at };
    try { const [p, h] = await b.custody_caps(); return compareCaps(p, h, stackStage, at); }
    catch (err) { return { kind: "error", error: String((err as Error).message).split("\n")[0].slice(0, 200), stackStage, at }; }
  };
}

async function main(): Promise<void> {
  const log = logger();
  const argv = process.argv.slice(2), dryRun = argv.includes("--dry-run");
  let tg: Telegram;
  try { tg = new Telegram({ dryRun, log: { log: (m: string) => log("info", "telegram", { m: m.slice(0, 200) }), error: (m: string) => log("error", "telegram", { m: m.slice(0, 200) }) } as unknown as Console }); }
  catch (err) { log("error", "refusing-to-start", { problem: (err as Error).message }); process.exit(2); }
  if (argv.includes("--test-alert")) {
    try { const n = await testAlerts(tg, process.env.RECONCILE_PUBLIC_URL ?? "the reconciler"); console.log(`sent ${n} test alert(s) through the reconciler's channel${dryRun ? " (dry run: printed, not posted)" : ""}`); process.exit(0); }
    catch (err) { console.error(`test alerts failed: ${(err as Error).message}`); process.exit(1); }
  }
  let cfg: ReconcileConfig;
  try { cfg = parseReconcileConfig(process.env); } catch (err) { log("error", "refusing-to-start", { problem: (err as Error).message }); process.exit(2); }
  const b = cfg.canisterId ? await buyer(cfg.canisterId, { host: cfg.icHost }) : null;       // anonymous: queries only
  const store = new Store(cfg.stateDir);
  let d: Deps, records: RecordsCache | null, names_: string[];
  if (cfg.home === "books") {
    // W11 (canister-only): the books from the canister's queries, the chain at finalized, the items located by two sources
    if (!b) { log("error", "refusing-to-start", { problem: "the books home needs CANISTER_ID" }); process.exit(2); }
    const names = existsSync(cfg.launchFile!) ? pinRefs(JSON.parse(readFileSync(cfg.launchFile!, "utf8"))) : new Map();
    const reader = solReader(cfg.solRpcUrls!);
    if (cfg.solRpcUrls!.length < 2) log("warn", "one-solana-provider", { note: "SOL_RPC_URLS names one provider: every account is that provider's word (D-h: two)" });
    // a Solana NFT is located on Solana through the same agreeing reader (two providers: D-h); an EthNft item's owner is read on its
    // own chain (Ethereum's RPCs, Robinhood Chain's for its own collections: 2026-10-12)
    const locate = locator({ ordFetch: agreeingOrdFetch(fetch), ethRpcs: cfg.ethRpcUrls ?? [], rpcsByChain: { "4663": cfg.rhRpcUrls ?? [] }, solReader: reader });
    let lastBacking: BackingRead = { kind: "absent", at: 0, note: "not read yet" };
    const service = { reserves: async () => {
      const r = await readBooksReserves({ reader, canister: b, locate, nowS: Math.floor(Date.now() / 1000), names });
      lastBacking = r.backing;
      return r.collections;
    } };
    d = { cfg, service, caps: capsOf(b, 0), channel: tg, store, log, stackStage: 0, backing: async () => lastBacking };
    records = null;
    names_ = [...names.values()].map((l) => l.slug);
  } else {
    const config = loadConfig(cfg.stackPath);
    if (config.chainId !== cfg.chainId) { log("error", "refusing-to-start", { problem: `the stack file is for chain ${config.chainId}, not RECONCILE_CHAIN_ID=${cfg.chainId}` }); process.exit(2); }
    const stackJson = JSON.parse(readFileSync(cfg.stackPath, "utf8")) as { stage?: number; collections?: never };
    const stackStage = Number(stackJson.stage ?? 0);
    const rpc = await pinnedRpc(cfg.rpcUrls, cfg.chainId, log);
    // E-13: an inscription's location is believed only when two ordinals providers agree (src/location.ts)
    const service = new WrappedService({ config, rpc, fetch: agreeingOrdFetch(fetch), log: (o) => log("warn", String(o.event ?? "indexer"), o), ttlMs: 1 });
    const backing = () => readBacking(rpc as never, stackJson, Math.floor(Date.now() / 1000));      // E-05
    d = { cfg, service, caps: capsOf(b, stackStage), channel: tg, store, log, stackStage, backing };
    const ethCollections = config.collections.flatMap((c) => (c.custody.kind === "evm" && c.custody.chainId === 1 ? [{ slug: c.slug, collection: c.custody.collection }] : []));
    records = b && ethCollections.length ? new RecordsCache(b, ethCollections) : null;
    names_ = config.collections.map((c) => c.slug);
  }
  if (cfg.port !== null) serve(() => served(store.load().last, Math.floor(Date.now() / 1000), cfg.staleS, cfg.chainId), cfg.port, records ? () => records.get() : undefined);
  log("info", "start", { home: cfg.home, chainId: cfg.chainId, collections: names_, canister: cfg.canisterId, everyS: cfg.everyS, staleS: cfg.staleS,
    ...(cfg.home === "books" ? { solRpc: cfg.solRpcUrls!.map((u) => new URL(u).host), ethRpc: (cfg.ethRpcUrls ?? []).map((u) => new URL(u).host),
      rhRpc: (cfg.rhRpcUrls ?? []).map((u) => new URL(u).host) } : {}),
    telegram: dryRun ? "--dry-run: alerts are printed, not posted" : "set" });
  const once = argv.includes("--once");
  for (const sig of ["SIGINT", "SIGTERM"] as const) process.on(sig, () => process.exit(0));
  const run = () => bounded(runOnce(d), (cfg.runTimeoutS ?? 900) * 1000).catch((err) => { log("error", "run-failed", { error: String((err as Error).message).slice(0, 200) }); return null; });
  if (once) { const s = await run(); console.log(JSON.stringify(s, (_k, x) => (typeof x === "bigint" ? String(x) : x), 1)); process.exit(s && s.servedStatus !== "red" ? 0 : 1); }
  // E-08a: the watchdog is its own timer, so a hung run cannot silence it: a report going stale turns red, and is alerted
  setInterval(() => { alert(d, Date.now()).catch((err) => log("error", "watchdog-failed", { error: String((err as Error).message).slice(0, 200) })); }, Math.min(cfg.everyS, 300) * 1000);
  for (;;) {
    await run();
    await new Promise((ok) => setTimeout(ok, cfg.everyS * 1000));
  }
}

if (import.meta.url === `file://${process.argv[1]}`) await main();
