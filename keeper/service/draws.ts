import { existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { createHash } from "node:crypto";
import { decodeFunctionResult, encodeFunctionData, type Abi, type Hex } from "viem";
import { balancesOf, serialise, snapshot, type Snapshot, type TransferLog } from "../../tools/snapshot/src/core.js";
import { lastBlockAtOrBefore as lastBlockBy } from "../../tools/snapshot/src/rpc.js";
import { readExclusions, stackExclusions } from "../../tools/snapshot/src/stack.js";
import { STAGE, VAULT, type DrawView } from "./abis.js";
import { mapLimit } from "./coins.js";
import type { LogCache } from "./logs.js";
import type { ChainRpc, Fetch } from "./rpc.js";
import type { Stack, StackCollection } from "./stack.js";
import { snapshotName } from "./plan.js";
import { systemResolver, textCapped, urlRefusal, type Resolver } from "./outbound.js";
import { objectKey, objectUrl, s3ErrorCode, s3Request, type S3Target } from "./s3.js";

/**
 * fixes-B-C-keeper-draws.md §5 items 1 and 11; docs/snapshot-v1.md "Version 2". The draw's snapshot is taken at the LAST block whose timestamp
 * is at most the draw's `snapshotAt` (fixed on chain by the seed round), built by tools/snapshot's own core over the coin's
 * Transfer logs with the sentinel leaf, written where the status server serves it, uploaded to a mirror off the keeper's volume
 * (a plain PUT, or an S3-compatible bucket such as Cloudflare R2),
 * and read back from the URI the draw will carry before the draw is opened.
 */
export type Block = { number: string; timestamp: string; l1BlockNumber?: string };

/** The EVM's `block.number` for this block: on 4663 (Arbitrum Nitro) the parent chain's (`l1BlockNumber`); elsewhere its own. */
export const evmNumber = (b: Block): number => Number(BigInt(b.l1BlockNumber ?? b.number));

export async function blockAt(rpc: ChainRpc, n: number | "latest"): Promise<Block> {
  const b = await rpc.call<Block | null>("eth_getBlockByNumber", [n === "latest" ? "latest" : `0x${n.toString(16)}`, false]);
  if (!b) throw new Error(`block ${n} is not served`);
  return b;
}

/** The last block stamped at or before `t` (4663's own numbering), searched from `lo`; throws while no later block exists. */
export async function lastBlockAtOrBefore(rpc: ChainRpc, t: number, lo: number, head: Block): Promise<number> {
  const ts = async (n: bigint) => BigInt((await blockAt(rpc, Number(n))).timestamp);
  return Number(await lastBlockBy(ts, BigInt(head.number), BigInt(t), BigInt(Math.max(0, lo))));
}

export async function readDraw(rpc: ChainRpc, vault: Hex, id: bigint): Promise<DrawView> {
  const out = await rpc.call<Hex>("eth_call", [{ to: vault, data: encodeFunctionData({ abi: VAULT as Abi, functionName: "draw", args: [id] }) }, "latest"]);
  return decodeFunctionResult({ abi: VAULT as Abi, functionName: "draw", data: out }) as DrawView;
}

/** Addresses that hold the coin without being holders (docs/snapshot-v1.md "Version 2", "Exclusions"): tools/snapshot's rule, the list fail-closed (E-14). */
export function exclusionsFor(stack: Stack, c: StackCollection, coin: Hex, curve: Hex, listedFile: string): string[] {
  return stackExclusions(stack, c.slug, coin, curve, readExclusions(listedFile));
}

/** The v2 snapshot of `coin` at `block` for a draw whose snapshot time is `snapshotAt`: weighted, with the sentinel leaf. */
export async function buildSnapshot(logs: LogCache, o: { stack: Stack; c: StackCollection; coin: Hex; curve: Hex; block: number; snapshotAt: number; confirmed: number;
  listedFile: string }): Promise<{ snap: Snapshot; text: string; name: string; sha256: string }> {
  const exclusions = exclusionsFor(o.stack, o.c, o.coin, o.curve, o.listedFile);     // first: an unreadable list stops everything
  const transfers = await logs.upTo(o.coin, o.stack.fromBlock, o.block, o.confirmed);
  const snap = snapshot(balancesOf(transfers), { coin: o.coin, block: String(o.block), kind: "weighted", decimals: 18, exclusions, snapshotAt: String(o.snapshotAt) });
  const text = serialise(snap);
  return { snap, text, name: snapshotName(o.coin, o.block), sha256: createHash("sha256").update(text).digest("hex") };
}

/**
 * Integration R3 (review2 N-2): a draw voided or cancelled before its pin leaves its seed, and so its snapshot time, to the coin's
 * next announcement. A snapshot time that falls on a moment when nobody held a whole coin is opened empty and closes with no winner
 * (fixes-R3-carry), which consumes the carry; that is a round the coin's holders gain nothing from. So a coin is announced with a
 * new seed only if EVERY moment its snapshot time could land on (the `SNAPSHOT_WINDOW` before now) has at least one ticket: the
 * balances at `windowStart` (the last block at or before now − window) and after each later block. Pure. Returns the first empty
 * block, or null when the whole window has tickets.
 */
export function firstEmptyBlock(logs: readonly TransferLog[], windowStart: number, exclusions: readonly string[], decimals = 18): number | null {
  const one = 10n ** BigInt(decimals), excluded = new Set(exclusions.map((a) => a.toLowerCase()));
  const sorted = [...logs].sort((x, y) => (x.blockNumber === y.blockNumber ? x.logIndex - y.logIndex : x.blockNumber < y.blockNumber ? -1 : 1));
  const before = sorted.filter((l) => Number(l.blockNumber) <= windowStart), after = sorted.filter((l) => Number(l.blockNumber) > windowStart);
  const bal = balancesOf(before);
  const whole = new Set([...bal].filter(([a, b]) => b >= one && !excluded.has(a.toLowerCase())).map(([a]) => a.toLowerCase()));
  if (whole.size === 0) return windowStart;
  const ZERO = "0x0000000000000000000000000000000000000000";
  const touch = (a: string, d: bigint) => {
    const k = a.toLowerCase();
    if (k === ZERO) return;
    const v = (bal.get(k) ?? 0n) + d;
    if (v < 0n) throw new Error(`logs are missing: ${k} would go negative`);
    bal.set(k, v);
    if (v >= one && !excluded.has(k)) whole.add(k); else whole.delete(k);
  };
  for (let i = 0; i < after.length; i++) {
    touch(after[i].from, -after[i].value); touch(after[i].to, after[i].value);
    const endOfBlock = i + 1 === after.length || after[i + 1].blockNumber !== after[i].blockNumber;
    if (endOfBlock && whole.size === 0) return Number(after[i].blockNumber);
  }
  return null;
}

// ---- publishing: the keeper's volume, a mirror, and the URI read back ---------------------------------------------------------------

/**
 * Where the snapshot goes besides the volume: `uploadUrl`, a plain `PUT <uploadUrl>/<name>` (optional `Authorization: uploadAuth`);
 * or `s3`, an S3-compatible bucket (Cloudflare R2) written with a SigV4-signed PUT of `<prefix><name>` (service/s3.ts). At most one.
 */
export type PublishConfig = { dir: string; baseUrl: string | null; uploadUrl: string | null; uploadAuth: string | null; uploadReadback?: boolean; s3?: S3Target | null;
  nowMs?: () => number };
export type Published = { uri: string; path: string; sha256: string; mirrored: boolean; readBack: string[] };

const sha = (t: string) => createHash("sha256").update(t).digest("hex");
export const MAX_SNAPSHOT_BYTES = 20 * 1024 * 1024;

/**
 * GET a published snapshot: https only (plain http to this machine only when `allowLocal`, for a local fork), no redirects, at most
 * MAX_SNAPSHOT_BYTES, streamed and abandoned past the cap (audit R2-S6). Unless `allowLocal`, the host must resolve to public
 * addresses only (the indexer's rules, service/outbound.ts; `resolve: null` skips the lookup for a URL the operator configured).
 * Null on any failure. A draw's URI is the keeper's word on chain, so the resolver reads it as untrusted data: an entry from it is
 * used only when its proof checks against the draw's on-chain root.
 */
export async function fetchText(uri: string, fetchImpl: Fetch, allowLocal = false, resolve: Resolver | null = systemResolver,
  headers: Record<string, string> = {}): Promise<string | null> {
  try {
    const u = new URL(uri);
    const local = ["127.0.0.1", "localhost", "[::1]", "::1"].includes(u.hostname);
    if (u.protocol !== "https:" && !(u.protocol === "http:" && local && allowLocal)) return null;
    if (local && !allowLocal) return null;
    if (!allowLocal && resolve !== null && (await urlRefusal(uri, resolve)) !== null) return null;
    const r = await fetchImpl(uri, { headers: { accept: "application/json", "user-agent": "rook-keeper/1", ...headers }, signal: AbortSignal.timeout(20_000), redirect: "error" });
    if (!r.ok) { await r.body?.cancel?.().catch(() => undefined); return null; }
    return await textCapped(r, MAX_SNAPSHOT_BYTES);
  } catch { return null; }
}

/**
 * Write `text` as `name` to the keeper's volume, upload it to the mirror when one is configured (a plain PUT, or a signed S3 PUT),
 * and read it back from the URI the draw will carry: the draw is opened only when that URI serves exactly these bytes. Throws with
 * the reason otherwise; no message carries a credential.
 */
export async function publish(p: PublishConfig, name: string, text: string, fetchImpl: Fetch): Promise<Published> {
  if (!p.baseUrl) throw new Error("no public base URL for snapshots (KEEPER_SNAPSHOT_BASE_URL or KEEPER_PUBLIC_URL)");
  if (p.uploadUrl && p.s3) throw new Error("two snapshot mirrors configured (KEEPER_SNAPSHOT_UPLOAD_URL and KEEPER_SNAPSHOT_S3_*): one only");
  const now = p.nowMs ?? Date.now;
  mkdirSync(p.dir, { recursive: true });
  const path = join(p.dir, name);
  if (!existsSync(path) || readFileSync(path, "utf8") !== text) { writeFileSync(`${path}.tmp`, text); renameSync(`${path}.tmp`, path); }
  let mirrored = false;
  // the mirror's own object, and how to read it back (a signed GET is signed at the moment it is sent)
  let object: { url: string; headers: () => Record<string, string> } | null = null;
  if (p.s3) {
    const t = p.s3, put = s3Request(t, "PUT", name, text, now());
    const r = await fetchImpl(put.url, { method: "PUT", body: text, redirect: "error", signal: AbortSignal.timeout(30_000),
      headers: { ...put.headers, "user-agent": "rook-keeper/1" } });
    if (!r.ok) {
      const code = s3ErrorCode((await textCapped(r, 16_384).catch(() => null)) ?? "");
      throw new Error(`the snapshot bucket refused the upload of ${objectKey(t, name)}: HTTP ${r.status}${code ? ` ${code}` : ""}`);
    }
    mirrored = true;
    object = { url: objectUrl(t, name), headers: () => s3Request(t, "GET", name, null, now()).headers };
  } else if (p.uploadUrl) {
    const url = `${p.uploadUrl.replace(/\/$/, "")}/${name}`;
    const auth = (): Record<string, string> => (p.uploadAuth ? { authorization: p.uploadAuth } : {});
    const r = await fetchImpl(url, { method: "PUT", body: text, redirect: "error", signal: AbortSignal.timeout(30_000),
      headers: { "content-type": "application/json", "user-agent": "rook-keeper/1", ...auth() } });
    if (!r.ok) throw new Error(`the snapshot mirror refused the upload: HTTP ${r.status}`);
    mirrored = true;
    object = { url, headers: auth };
  }
  const uri = `${p.baseUrl.replace(/\/$/, "")}/${name}`;
  // read back what the chain will name, and (E-11, audit R2-S5) the mirror's own object where it differs: the same bytes from both
  const reads: { url: string; headers: () => Record<string, string> }[] = [{ url: uri, headers: () => ({}) }];
  if (object && object.url !== uri && p.uploadReadback !== false) reads.push(object);
  for (const x of reads) {
    const back = await fetchText(x.url, fetchImpl, true, null, x.headers());    // our own configured URLs (https, or this machine's on a fork)
    if (back === null) throw new Error(`the snapshot is not served at ${new URL(x.url).host} yet: the draw is not opened`);
    if (sha(back) !== sha(text)) throw new Error(`${new URL(x.url).host} serves other bytes for ${name}: the draw is not opened`);
  }
  return { uri, path, sha256: sha(text), mirrored, readBack: reads.map((x) => new URL(x.url).host) };
}

// ---- the draw book: every draw of a vault walked once, from the lowest one not yet final ------------------------------------------

const FINAL = new Set<number>([STAGE.Claimed, STAGE.Cancelled, STAGE.Voided, STAGE.Empty]);
type BookFile = { version: 1; vault: string; cursor: number; announcedAt: Record<string, number> };

/**
 * Audit E-04: a vault's draws are read from `cursor`, the lowest id that is not final (Claimed, Cancelled, Voided, Empty), and the cursor
 * moves past final draws for good; so a pass reads only the draws still in flight. Each coin's latest announcement time is kept, so
 * "announced this slot?" never needs an old draw again.
 */
export class DrawBook {
  private f: BookFile;
  constructor(private readonly file: string, private readonly rpc: ChainRpc, readonly vault: Hex, private readonly concurrency = 8) {
    mkdirSync(dirname(file), { recursive: true });
    let f: BookFile = { version: 1, vault: vault.toLowerCase(), cursor: 0, announcedAt: {} };
    if (existsSync(file)) { try { const j = JSON.parse(readFileSync(file, "utf8")) as BookFile; if (j.version === 1 && j.vault === vault.toLowerCase()) f = j; } catch { /* rebuilt */ } }
    this.f = f;
  }

  lastAnnouncedAt(coin: string): number { return this.f.announcedAt[coin.toLowerCase()] ?? 0; }

  /** The draws not yet final, with their ids, oldest first; the cursor advances past the final ones. */
  async open(): Promise<{ id: bigint; d: DrawView }[]> {
    const count = Number(decodeFunctionResult({ abi: VAULT as Abi, functionName: "drawCount",
      data: await this.rpc.call<Hex>("eth_call", [{ to: this.vault, data: encodeFunctionData({ abi: VAULT as Abi, functionName: "drawCount" }) }, "latest"]) }) as bigint);
    const ids = Array.from({ length: Math.max(0, count - this.f.cursor) }, (_, k) => this.f.cursor + k);
    const draws = await mapLimit(ids, this.concurrency, async (i) => ({ id: BigInt(i), d: await readDraw(this.rpc, this.vault, BigInt(i)) }));
    let cursor = this.f.cursor;
    for (const { id, d } of draws) {
      const coin = d.coin.toLowerCase();
      if (Number(d.announcedAt) > (this.f.announcedAt[coin] ?? 0)) this.f.announcedAt[coin] = Number(d.announcedAt);
      if (Number(id) === cursor && FINAL.has(Number(d.stage))) cursor++;
    }
    this.f.cursor = cursor;
    const tmp = `${this.file}.tmp`;
    writeFileSync(tmp, JSON.stringify(this.f));
    renameSync(tmp, this.file);
    return draws.filter(({ d }) => !FINAL.has(Number(d.stage)));
  }

  /** Remember an announcement the moment it is mined (the next walk would find it too). */
  noteAnnounced(coin: string, at: number): void { if (at > this.lastAnnouncedAt(coin)) this.f.announcedAt[coin.toLowerCase()] = at; }
}
