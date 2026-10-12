import { existsSync, readFileSync } from "node:fs";
import { basename, join } from "node:path";
import { encodeFunctionData, type Hex } from "viem";
import { ticketLeaf, verify, type Entry, type Snapshot } from "../../tools/snapshot/src/core.js";
import { DRAW_RULES, NFT_WRAPPER, REGISTRY, STAGE, TICKET, VAULT, VAULT_PUBLIC, WRAPPER, WRAPPER_PUBLIC, type DrawView, type TicketView } from "./abis.js";
import { buildSnapshot, fetchText, lastBlockAtOrBefore } from "./draws.js";
import { provenRound } from "./drand.js";
import { beaconOf, curveOf, read, stops, type KeeperDeps, type PassCtx } from "./keeper.js";
import { elapsed } from "./plan.js";
import type { Result } from "./sender.js";
import type { StackCollection } from "./stack.js";

/**
 * docs/audit/2026-09-26/fixes-W2-services.md §2, audit E-03. The keyless resolver: only calls ANYONE may make, so no redemption and
 * no raffle depends on the keeper's key. It runs inside the keeper's process after the keeper's own work, and on its own
 * (service/resolver-main.ts) with any funded key. Per collection:
 *   - Wrapper tickets, strictly in order from `nextToResolve()`: `resolve(id, sig)` as soon as two drand relays serve the ticket's
 *     round (and the beacon accepts it); the first ticket whose round does not exist yet stops the walk.
 *   - RAFFLE draws not yet final: `requestDraw` once the challenge window has passed on both clocks (and the keeper is not paused);
 *     `resolve` once the pinned round is served; `claim` for the winner, with the proof from the published file (accepted only when
 *     its root is the draw's) or rebuilt from the chain; `voidDraw` for a draw announced and never opened (6 h), pinned and never
 *     resolved (7 days), or resolved and unclaimable (7 days).
 */
export type ResolverAction = { what: string; plan: string; result: Result | null };
export type ResolverReport = { sent: number; actions: ResolverAction[] };
export type ResolverDeps = Pick<KeeperDeps, "cfg" | "stack" | "rpc" | "sender" | "log" | "relays" | "logs" | "coinIndex" | "book" | "fetchImpl" | "nowMs" | "mem"
  | "snapshotDir" | "exclusionsFile" | "resolveHost">;

/** A ticket or draw whose round is this far past due and still unresolved is an alert (the monitor pages at 1 h). */
const LATE_S = 600;
/** A draw Resolved this long without a claim is an alert (`claim-stuck`, R2-S1); the monitor pages it from the chain too. */
export const CLAIM_STUCK_S = 3_600;

export async function resolverPass(d: ResolverDeps, ctx: PassCtx): Promise<ResolverReport> {
  const rep: ResolverReport = { sent: 0, actions: [] };
  const paused = await read<boolean>(d.rpc, d.stack.registry, REGISTRY, "keeperPaused");
  const room = () => rep.sent < d.cfg.resolver.maxTxPerPass && d.nowMs() < ctx.deadlineMs && !ctx.halted;
  const send = async (what: string, key: string, to: Hex, data: Hex): Promise<Result> => {
    const r = await d.sender.submit({ key, what, to, data, maxGas: d.cfg.resolver.maxGasPerTx });
    if (r.outcome !== "simulated" && r.outcome !== "simulation-reverted") rep.sent++;
    ctx.halted = ctx.halted || stops(r);
    if (r.alert) ctx.alerts.push(r.alert);
    rep.actions.push({ what, plan: r.detail, result: r });
    d.log(r.outcome === "mined" || r.outcome === "simulated" ? "info" : "warn", "resolver", { what, outcome: r.outcome, detail: r.detail });
    return r;
  };
  const note = (what: string, plan: string) => rep.actions.push({ what, plan, result: null });

  for (const c of d.stack.collections) {
    await tickets(d, c, ctx, room, send, note);
    await raffle(d, c, ctx, paused, room, send, note);
  }
  return rep;
}

type Send = (what: string, key: string, to: Hex, data: Hex) => Promise<Result>;
type Note = (what: string, plan: string) => void;

async function tickets(d: ResolverDeps, c: StackCollection, ctx: PassCtx, room: () => boolean, send: Send, note: Note): Promise<void> {
  const w = c.wrapper;
  const ABI = c.kind === "nft" ? NFT_WRAPPER : WRAPPER;   // docs/v7/phase1-nft.md: EthNftWrapper's ticket shape
  const beacon = await beaconOf(d, w, ABI);
  let last = -1n;
  while (room()) {
    const [next, count] = await Promise.all([read<bigint>(d.rpc, w, ABI, "nextToResolve"), read<bigint>(d.rpc, w, ABI, "ticketCount")]);
    if (next >= count || next === last) return;           // nothing waiting, or a dry run that cannot move the order
    last = next;
    const t = await read<TicketView>(d.rpc, w, ABI, "ticket", [next]);
    const what = `${c.slug} ticket ${next}`;
    if (Number(t.stage) !== TICKET.Pinned) { note(what, `stage ${t.stage}: not pinned, nothing to resolve`); return; }
    let proven;
    try { proven = await provenRound(d.relays, d.rpc, beacon, t.round); } catch (err) {
      const why = String((err as Error).message).slice(0, 200);
      note(what, why); ctx.alerts.push(`drand: ${what}: ${why}`); return;
    }
    if (!proven) {
      const due = Number(t.pinnedAt) + 60;          // PIN_AHEAD = 20 rounds of 3 s after the pin
      note(what, `waiting for drand round ${t.round}`);
      if (ctx.nowS > due + LATE_S) ctx.alerts.push(`ticket-late: ${what} is pinned to round ${t.round}, still not served by two relays ${ctx.nowS - due} s after it was due`);
      return;
    }
    if (proven.liars.length) ctx.alerts.push(`drand-liar: round ${t.round} of ${what}: ${proven.liars.join(", ")} served bytes the beacon refuses`);
    const r = await send(`resolve ${what} (round ${t.round})`, `wresolve:${w.toLowerCase()}:${next}`, w,
      encodeFunctionData({ abi: WRAPPER_PUBLIC, functionName: "resolve", args: [next, proven.signature] }));
    if (r.outcome !== "mined") return;                  // in order: never past a ticket that did not resolve
  }
}

async function raffle(d: ResolverDeps, c: StackCollection, ctx: PassCtx, paused: boolean, room: () => boolean, send: Send, note: Note): Promise<void> {
  const v = c.raffleVault, { nowS, nowEvm } = ctx;
  const pbs = Number(await read<bigint>(d.rpc, v, VAULT, "parentBlockSeconds"));
  const open = await d.book(v).open();
  const beacon = open.some(({ d: x }) => Number(x.stage) === STAGE.Pinned) ? await beaconOf(d, v) : null;
  const pub = (fn: "requestDraw" | "resolve" | "claim" | "voidDraw", args: unknown[]) => encodeFunctionData({ abi: VAULT_PUBLIC, functionName: fn, args } as never) as Hex;
  for (const { id, d: dv } of open) {
    if (!room()) return;
    const what = `${c.slug} draw ${id}`, key = (s: string) => `${s}:${v.toLowerCase()}:${id}`;
    switch (Number(dv.stage)) {
      case STAGE.Announced:
        if (elapsed(Number(dv.announcedAt), Number(dv.announcedAtBlock), DRAW_RULES.openTtlS, nowS, nowEvm, pbs)) await send(`void ${what} (announced, never opened)`, key("void"), v, pub("voidDraw", [id]));
        break;
      case STAGE.Open:
        if (!elapsed(Number(dv.challengeEnds), Number(dv.challengeEndsBlock), 0, nowS, nowEvm, pbs)) { note(what, "in its challenge window"); break; }
        if (paused) { note(what, "the keeper is paused: requestDraw is refused until the owner unpauses"); break; }
        await send(`request ${what}`, key("request"), v, pub("requestDraw", [id]));
        break;
      case STAGE.Pinned: {
        if (elapsed(Number(dv.pinnedAt), Number(dv.pinnedAtBlock), DRAW_RULES.pinTimeoutS, nowS, nowEvm, pbs)) { await send(`void ${what} (pinned, never resolved)`, key("void"), v, pub("voidDraw", [id])); break; }
        let proven;
        try { proven = await provenRound(d.relays, d.rpc, beacon!, dv.round); } catch (err) {
          const why = String((err as Error).message).slice(0, 200); note(what, why); ctx.alerts.push(`drand: ${what}: ${why}`); break;
        }
        if (!proven) {
          note(what, `waiting for drand round ${dv.round}`);
          if (nowS > Number(dv.pinnedAt) + 60 + LATE_S) ctx.alerts.push(`draw-late: ${what} is pinned to round ${dv.round}, still not served by two relays`);
          break;
        }
        if (proven.liars.length) ctx.alerts.push(`drand-liar: round ${dv.round} of ${what}: ${proven.liars.join(", ")} served bytes the beacon refuses`);
        await send(`resolve ${what} (round ${dv.round})`, key("resolve"), v, pub("resolve", [id, proven.signature]));
        break;
      }
      case STAGE.Resolved: {
        // audit R2-S1: the entry is used only when its proof checks against the on-chain root; a claim the chain refuses is tried
        // once more with the tree rebuilt from the chain; one claim and one rebuild per draw per pass, so a pass never loops
        const done = (r: Result) => r.outcome === "mined" || r.outcome === "simulated" || r.outcome === "pending";
        const claim = (e: Found, tag: string) => send(`claim ${what} for ${e.entry.holder}${tag}`, key(tag ? "claim-rebuilt" : "claim"), v,
          pub("claim", [id, e.entry.holder as Hex, BigInt(e.entry.first!), BigInt(e.entry.end!), e.entry.proof]));
        let claimed = false;
        const e = await winnerEntry(d, c, id, dv, ctx);
        if (e) {
          const r = await claim(e, "");
          claimed = done(r);
          if (!claimed && r.outcome === "simulation-reverted" && e.source !== "chain" && room()) {
            const again = await winnerEntry(d, c, id, dv, ctx, { chainOnly: true });
            if (again) claimed = done(await claim(again, " (rebuilt from the chain)"));
          }
        }
        if (claimed) break;
        if (elapsed(Number(dv.resolvedAt), Number(dv.resolvedAtBlock), DRAW_RULES.voidAfterS, nowS, nowEvm, pbs)) { await send(`void ${what} (no provable claim)`, key("void"), v, pub("voidDraw", [id])); break; }
        note(what, e ? "resolved; the claim was refused and the chain rebuild did not prove either" : "resolved; the winner's entry could not be proven from the published file or the chain");
        if (nowS - Number(dv.resolvedAt) > CLAIM_STUCK_S)
          ctx.alerts.push(`claim-stuck: ${what} has been Resolved for ${Math.floor((nowS - Number(dv.resolvedAt)) / 60)} min without a claim: the winning ticket ${dv.winningTicket} has no entry whose proof checks against the on-chain root (${e ? "its claim was refused" : "no source proves it"}); anyone may void it ${DRAW_RULES.voidAfterS / 86_400} days after the resolve`);
        break;
      }
    }
  }
}

type Found = { entry: Entry; source: "uri" | "local" | "chain" };

/**
 * The entry holding the winning ticket whose proof checks against the draw's ON-CHAIN root (audit R2-S1), or null. A file's own
 * `root` and `totalTickets` fields are not trusted: a file rewritten after `openDraw` (a mirror, a CDN, the upload credential) can
 * copy them and forge every entry. The holder zero (the sentinel) is never a winner.
 */
export function entryIn(s: Snapshot | null, dv: Pick<DrawView, "root" | "winningTicket">): Entry | null {
  if (!s || !Array.isArray(s.entries)) return null;
  const root = String(dv.root).toLowerCase() as Hex;
  for (const e of s.entries) {
    try {
      if (e.first === undefined || e.end === undefined || !(BigInt(e.first) <= dv.winningTicket && dv.winningTicket < BigInt(e.end))) continue;
      if (!/^0x[0-9a-fA-F]{40}$/.test(e.holder) || BigInt(e.holder) === 0n || !Array.isArray(e.proof)) continue;
      const leaf = ticketLeaf(e.holder, BigInt(e.first), BigInt(e.end));
      if (verify(e.proof.map((x) => String(x).toLowerCase() as Hex), root, leaf)) return { ...e, leaf };
    } catch { /* a malformed entry proves nothing */ }
  }
  return null;
}
const parse = (t: string | null): Snapshot | null => { if (t === null) return null; try { return JSON.parse(t) as Snapshot; } catch { return null; } };

/**
 * The winning entry, proven against the on-chain root: from the published file (at the draw's URI), else the keeper's copy, else
 * rebuilt from the chain (cached per draw once built, so a forged file costs one rebuild, not one a pass). `chainOnly`: skip the
 * files (a claim with a file's entry was refused).
 */
/** R3-check L-D: a rebuild that does not prove (or fails) is tried again after this, doubling, up to REBUILD_RETRY_MAX_MS. */
export const REBUILD_RETRY_MS = 10 * 60_000;
export const REBUILD_RETRY_MAX_MS = 6 * 3_600_000;
export async function winnerEntry(d: ResolverDeps, c: StackCollection, id: bigint, dv: DrawView, ctx: PassCtx, o: { chainOnly?: boolean } = {}): Promise<Found | null> {
  if (!o.chainOnly) {
    const fromUri = entryIn(parse(await fetchText(dv.uri, d.fetchImpl, d.cfg.chainId !== 4663, d.resolveHost)), dv);    // on 4663: https to a public host only
    if (fromUri) return { entry: fromUri, source: "uri" };
    const local = join(d.snapshotDir, basename(dv.uri.split("?")[0]));
    if (/^snapshot-0x[0-9a-f]{40}-\d+-weighted\.json$/.test(basename(local)) && existsSync(local)) {
      const b = entryIn(parse(readFileSync(local, "utf8")), dv);
      if (b) return { entry: b, source: "local" };
    }
  }
  const k = `${c.raffleVault.toLowerCase()}:${id}`, now = d.nowMs();
  const hit = d.mem.rebuilt.get(k);
  if (hit && (hit.entry || now < hit.retryAt)) return hit.entry ? { entry: hit.entry, source: "chain" } : null;
  const later = (): void => { const tries = (hit?.tries ?? 0) + 1; d.mem.rebuilt.set(k, { entry: null, tries, retryAt: now + Math.min(REBUILD_RETRY_MAX_MS, REBUILD_RETRY_MS * 2 ** (tries - 1)) }); };
  try {
    const block = await lastBlockAtOrBefore(d.rpc, Number(dv.snapshotAt), d.stack.fromBlock, ctx.head);
    const built = await buildSnapshot(d.logs, { stack: d.stack, c, coin: dv.coin, curve: await curveOf(d, c, dv.coin), block, snapshotAt: Number(dv.snapshotAt),
      confirmed: Math.min(block, await ctx.confirmed()), listedFile: d.exclusionsFile });
    const r = entryIn(built.snap, dv);
    if (!r) {
      d.log("error", "claim-rebuild-mismatch", { draw: String(id), onchain: dv.root, rebuilt: built.snap.root, tries: (hit?.tries ?? 0) + 1 });
      d.logs.forget(dv.coin);             // the next try reads the coin's logs from the chain again, not what this one was built from
      later(); return null;
    }
    d.mem.rebuilt.set(k, { entry: r, retryAt: Infinity, tries: 0 });
    return { entry: r, source: "chain" };
  } catch (err) {
    d.log("warn", "claim-rebuild-failed", { draw: String(id), error: String((err as Error).message).slice(0, 200), tries: (hit?.tries ?? 0) + 1 });
    later();
    return null;
  }
}
