import type { Activity, Board } from "./apiTypes";

/**
 * docs/v6/phase1-scope.md §8.3 (K5): the site's Phase-1 mode. A build switch, `VITE_ROOK_PHASE=1`, and nothing a visitor can
 * flip. In Phase 1 the site offers only the paths that are ON (§1 of that document): no Bots (offer-board hand-ins), no Rent, no
 * YOYO policy, no mint desk, no Pons coins; it sends only with the bundled 4663 stack file, and only to that stack's own contracts
 * or to a coin or curve its factory launched. Everything here is pure, so test/phase1.test.ts is its spec.
 */

/** The switch. Unset or empty: no phase mode. "1": Phase 1. Anything else fails the build rather than meaning something by accident. */
export function phaseFrom(v: string | undefined): 1 | null {
  const s = (v ?? "").trim();
  if (s === "") return null;
  if (s === "1") return 1;
  throw new Error(`VITE_ROOK_PHASE is "${s}": the only phase this build knows is 1 (or leave it unset)`);
}
const viteEnv = (import.meta as unknown as { env?: Record<string, string | undefined> }).env ?? {};
export const PHASE1: boolean = phaseFrom(viteEnv.VITE_ROOK_PHASE) === 1;

/** The pages that are OFF in Phase 1, and why, in the visitor's words (docs/v6/phase1-scope.md §1). */
export const OFF_IN_PHASE1: Readonly<Record<string, { title: string; why: string }>> = {
  bots: {
    title: "Bots are off in Phase 1",
    why: "Handing a listing in to the offer board is not switched on: the custodian is not pointed at the board, so nothing handed in is bought, and the board's reward has no funding yet. A bond posted to the board anyway comes back after 24 hours.",
  },
  rent: {
    title: "Rentals are off in Phase 1",
    why: "Lending an NFT to the vault is described, not built: nothing on the vault's side can take an NFT in on loan or give it back, so the site does not offer it.",
  },
};
export const isOff = (at: string, phase1: boolean): boolean => phase1 && Object.prototype.hasOwnProperty.call(OFF_IN_PHASE1, at);

/** The nav without the pages that are off. */
export const navFor = <T extends readonly [string, string]>(nav: readonly T[], phase1: boolean): T[] => nav.filter(([to]) => !isOff(to, phase1));

/** The launch wizard's policies: YOYO is OFF in Phase 1 (custody-signed relistings are a new outbound path; C5/H8 is open). */
export const policiesFor = <T extends { id: string }>(ps: readonly T[], phase1: boolean): T[] => ps.filter((p) => !(phase1 && p.id === "yoyo"));

/** The board without pons coins: a coin launched through the v1 Launcher pays its fees into a vault with no way out in Phase 1. */
export const boardFor = (b: Board, phase1: boolean): Board => (phase1 ? { ...b, rows: b.rows.filter((r) => r.kind !== "pons") } : b);
export const activityFor = (a: Activity, phase1: boolean): Activity => (phase1 ? { ...a, items: a.items.filter((i) => i.coin?.kind !== "pons") } : a);

// ---- the bundled stack file (deployments/stack-4663.json, copied by scripts/sync-content.mjs) --------------------------------------

type Hex = `0x${string}`;
export type StackCollection = {
  slug: string; kind: "inscriptions" | "nft"; name?: string; symbol?: string; listingId?: number;
  wrapper: Hex; factory: Hex; router: Hex; floorModule: Hex; lock?: Hex; queue?: Hex;
  mintDesk?: Hex | null; raffleVault?: Hex | null; burnVault?: Hex | null; yoyoVault?: Hex | null; offerBoard?: Hex | null; wrapClaim?: Hex | null;
};
export type StackFile = {
  chainId: number; label: "stack"; stage?: number; registry: Hex; owner: Hex; keeper?: Hex; treasury: Hex; directory?: Hex;
  /** docs/v7/dormant-collections.md §3.3: the WakeDesk and its bond (wei), when the stack deployed one */
  wakeDesk?: Hex; wakeBond?: string | number;
  collections: StackCollection[]; caps?: { stage: number; custodyItemsPerCollection?: number; custodyWeiPerCollection?: number | string };
};
export const ROBINHOOD = 4663;

const isAddr = (x: unknown): x is Hex => typeof x === "string" && /^0x[0-9a-fA-F]{40}$/.test(x);
const ZERO = /^0x0{40}$/;

/** The one stack file for this chain, checked; or why there is none. A Phase-1 build sends nothing without it. */
export function stackFrom(files: readonly unknown[], chainId = ROBINHOOD): { stack: StackFile | null; why: string | null } {
  const mine = files.filter((f): f is Record<string, unknown> => !!f && typeof f === "object" && (f as { label?: unknown }).label === "stack" && (f as { chainId?: unknown }).chainId === chainId);
  if (!mine.length) return { stack: null, why: `No stack deployment file for chain ${chainId} is bundled in this build (deployments/stack-${chainId}.json).` };
  if (mine.length > 1) return { stack: null, why: `${mine.length} stack files for chain ${chainId} are bundled; the build must carry exactly one.` };
  const f = mine[0];
  for (const k of ["registry", "owner", "treasury"] as const) if (!isAddr(f[k]) || ZERO.test(f[k] as string)) return { stack: null, why: `The bundled stack file's ${k} is not an address.` };
  const cols = f.collections;
  if (!Array.isArray(cols) || !cols.length) return { stack: null, why: "The bundled stack file names no collection." };
  for (const c of cols as Record<string, unknown>[]) {
    for (const k of ["wrapper", "factory", "router", "floorModule"] as const) {
      if (!isAddr(c[k]) || ZERO.test(c[k] as string)) return { stack: null, why: `The bundled stack file's ${String(c.slug ?? "?")} ${k} is not an address.` };
    }
    if (typeof c.slug !== "string" || !c.slug) return { stack: null, why: "A collection in the bundled stack file has no slug." };
  }
  const stage = (f.caps as { stage?: unknown } | undefined)?.stage ?? f.stage;
  if (stage !== 0) return { stack: null, why: `The bundled stack file is at stage ${String(stage)}; Phase 1 is stage 0.` };
  return { stack: f as unknown as StackFile, why: null };
}

export const stackCollection = (s: StackFile | null, slug: string | null | undefined): StackCollection | null =>
  !s || !slug ? null : s.collections.find((c) => c.slug.toLowerCase() === slug.toLowerCase()) ?? null;

/** Why a Phase-1 TxButton cannot send at all, before anything is asked of the wallet; null when it may. */
export function sendRefusal(s: { stack: StackFile | null; why: string | null }, chainId: number | null): string | null {
  if (!s.stack) return `This Phase-1 build cannot send: ${s.why ?? "it carries no stack deployment file."}`;
  if (chainId !== s.stack.chainId) return `This page is reading chain ${chainId ?? "none"}, not the stack's chain ${s.stack.chainId}, so it cannot send.`;
  return null;
}

const low = (a: string) => a.toLowerCase();
/** The kinds of stack contract a Phase-1 page may send to, and the stack file's key for each. */
const SENDABLE_KEYS = ["wrapper", "factory", "router", "floorModule", "raffleVault"] as const;
export type TargetKind = (typeof SENDABLE_KEYS)[number] | "coin" | "curve" | "wakeDesk";
/** The stack's contracts a Phase-1 page may send to: the wrappers (redeem, refund, resolve), the factories (launch), the routers
 *  (graduated trades), the floor modules, and the raffle vaults (the keyless draw steps: pin, resolve, claim, void). Never the mint
 *  desk, the offer board or the YOYO vault: those paths are OFF. */
export function sendable(s: StackFile): Map<string, string> {
  const m = new Map<string, string>();
  for (const c of s.collections) for (const k of SENDABLE_KEYS) { const a = c[k]; if (isAddr(a) && !ZERO.test(a)) m.set(low(a), `${c.slug} ${k}`); }
  return m;
}
const kinds = (s: StackFile): Map<string, TargetKind> => {
  const m = new Map<string, TargetKind>();
  for (const c of s.collections) for (const k of SENDABLE_KEYS) { const a = c[k]; if (isAddr(a) && !ZERO.test(a)) m.set(low(a), k); }
  // docs/v7/dormant-collections.md §3.3: the stack's WakeDesk (a bonded wake request for a dormant collection)
  if (isAddr(s.wakeDesk) && !ZERO.test(s.wakeDesk)) m.set(low(s.wakeDesk), "wakeDesk");
  return m;
};
export function offTargets(s: StackFile): Map<string, string> {
  const m = new Map<string, string>();
  const words = { mintDesk: "mint desk", offerBoard: "offer board", yoyoVault: "YOYO vault" } as const;
  for (const c of s.collections) for (const k of ["mintDesk", "offerBoard", "yoyoVault"] as const) {
    const a = c[k];
    if (isAddr(a) && !ZERO.test(a)) m.set(low(a), `${c.slug}'s ${words[k]}`);
  }
  return m;
}

/** What a target check reads from the chain: `CurveFactory.routerOf(coin)` (the coin's curve, zero if the factory never launched
 *  it) and `FrogCurve.token()` (null when the address is not a curve). */
export type StackReader = { routerOf: (factory: Hex, coin: Hex) => Promise<string>; tokenOf: (curve: Hex) => Promise<string | null> };

/** Whether one of the stack's factories launched `coin` (read on chain; an unreadable chain says no). */
async function launchedCoin(s: StackFile, coin: string, read: StackReader): Promise<boolean> {
  if (!isAddr(coin)) return false;
  for (const f of s.collections.map((c) => c.factory)) {
    try { if (!ZERO.test(low(await read.routerOf(f, coin as Hex)))) return true; } catch { /* not a coin of this factory */ }
  }
  return false;
}

/** What `target` is to this deployment, or why a Phase-1 page may not send to it. */
export async function classifyTarget(s: StackFile, target: string, read: StackReader): Promise<{ kind: TargetKind } | { refusal: string }> {
  if (!isAddr(target)) return { refusal: "The transaction has no valid target; nothing was sent." };
  const t = low(target);
  const off = offTargets(s).get(t);
  if (off) return { refusal: `That would send to ${off}, which is off in Phase 1; nothing was sent.` };
  const own = kinds(s).get(t);
  if (own) return { kind: own };
  if (await launchedCoin(s, target, read)) return { kind: "coin" };
  let token: string | null = null;
  try { token = await read.tokenOf(target as Hex); } catch { token = null; }
  if (token && isAddr(token)) {
    for (const f of s.collections.map((c) => c.factory)) {
      try { if (low(await read.routerOf(f, token as Hex)) === t) return { kind: "curve" }; } catch { /* not this factory's */ }
    }
  }
  return { refusal: `${target} is not a contract of this deployment, nor a coin or a curve its factory launched; nothing was sent.` };
}

/** null when a Phase-1 page may send to `target`: one of the stack's own contracts, or a coin or a curve one of its factories
 *  launched (read on chain, so an indexer cannot point the ticket anywhere else). Otherwise the reason, in words. */
export async function targetRefusal(s: StackFile, target: string, read: StackReader): Promise<string | null> {
  const c = await classifyTarget(s, target, read);
  return "refusal" in c ? c.refusal : null;
}

// ---- E-07: what the transaction says, not only where it goes (docs/audit/2026-09-26/fixes-W2-site.md §2) ----------------------------

/** The calls a Phase-1 page makes, by the kind of target. Anything else is refused before the wallet is asked. */
export const CALLS: Readonly<Record<TargetKind, readonly string[]>> = {
  wrapper: ["redeem", "refund", "resolve"],
  factory: ["launch"],
  router: ["buy", "sell"],
  floorModule: [],
  raffleVault: ["requestDraw", "resolve", "claim", "voidDraw"],
  coin: ["approve"],
  curve: ["buy", "sell", "refund", "graduate", "openRefunds"],
  wakeDesk: ["request"],
};
type AbiParam = { name?: string; type: string; components?: readonly AbiParam[] };
type AbiFn = { type: string; name?: string; inputs?: readonly AbiParam[] };
export type SendRequest = { address: string; abi: readonly unknown[]; functionName: string; args?: readonly unknown[]; value?: bigint };
type Ctx = { s: StackFile; kind: TargetKind; fn: string; account: string | null; read: StackReader; pageCoin: string | null };

/** Every address-typed argument has a rule, or it is refused (default deny: an address the site cannot vouch for never reaches a wallet). */
async function addressRule(c: Ctx, name: string, value: unknown): Promise<string | null> {
  const v = typeof value === "string" ? value : "";
  const what = `${c.kind}.${c.fn}(${name})`;
  if (!isAddr(v)) return `The ${name} argument is not an address; nothing was sent.`;
  switch (`${c.kind}.${c.fn}.${name}`) {
    case "coin.approve.spender": {
      const routers = new Set(c.s.collections.map((x) => low(x.router)));
      return routers.has(low(v)) ? null : `The approval would let ${v} spend your coins, and it is not this deployment's router (${[...routers].join(", ")}); nothing was sent.`;
    }
    case "router.buy.coin": case "router.sell.coin":
      // R2 N-2: the coin traded is the page's coin, not merely some coin the factory launched (the indexer names coin.address)
      if (!c.pageCoin || low(v) !== low(c.pageCoin)) return `The trade is for ${v}, not the coin this page is about (${c.pageCoin ?? "none"}); nothing was sent.`;
      return (await launchedCoin(c.s, v, c.read)) ? null : `${v} is not a coin this deployment's factory launched; nothing was sent.`;
    case "router.buy.to": case "router.sell.to":
      return c.account && low(v) === low(c.account) ? null : `The trade would pay ${v}, not your connected account; nothing was sent.`;
    case "raffleVault.claim.holder":
      return null;   // bound by the Merkle root the vault verifies (and the page checks it against the on-chain draw before asking)
    case "wakeDesk.request.wrapper": {
      // docs/v7/dormant-collections.md §3.3: a wake is asked only for one of this deployment's own wrappers
      const wrappers = new Set(c.s.collections.map((x) => low(x.wrapper)));
      return wrappers.has(low(v)) ? null : `The wake request names ${v}, which is not a wrapper of this deployment; nothing was sent.`;
    }
    default:
      return `The call ${what} carries an address this site has no rule for (${v}); nothing was sent.`;
  }
}
async function paramRefusal(c: Ctx, p: AbiParam, value: unknown): Promise<string | null> {
  const name = p.name || "unnamed";
  if (p.type === "address") return addressRule(c, name, value);
  const arr = p.type.match(/^(.*)\[\d*\]$/);
  if (arr) {
    if (!/address|tuple/.test(arr[1])) return null;
    if (!Array.isArray(value)) return `The ${name} argument is not a list; nothing was sent.`;
    for (const x of value) { const r = await paramRefusal(c, { ...p, type: arr[1] }, x); if (r) return r; }
    return null;
  }
  if (p.type === "tuple") {
    for (const [i, q] of (p.components ?? []).entries()) {
      const x = Array.isArray(value) ? value[i] : (value as Record<string, unknown> | null)?.[q.name ?? ""];
      const r = await paramRefusal(c, q, x); if (r) return r;
    }
  }
  return null;
}

/**
 * E-07: null when a Phase-1 page may ask the wallet to sign `r`. The target must be one of this deployment's contracts (or a coin or
 * curve its factory launched, read on chain); the function must be one the site makes to that kind of contract; and every address
 * among the arguments must pass its rule — the approval's spender is the stack's router, a trade pays the connected account, a coin
 * is one the factory launched — so an indexer, its domain or a man in the middle can no longer turn a button into an approval or a
 * payment to someone else.
 */
export async function requestRefusal(s: StackFile, r: SendRequest, account: string | null, read: StackReader, pageCoin: string | null = null): Promise<string | null> {
  const c = await classifyTarget(s, r.address, read);
  if ("refusal" in c) return c.refusal;
  if (!CALLS[c.kind].includes(r.functionName)) return `This site does not call ${r.functionName} on a ${c.kind}; nothing was sent.`;
  const pc = pageCoin && isAddr(pageCoin) ? pageCoin : null;
  // R2 N-2: a request to a coin or to a curve is bound to the page's coin (its URL, #/coin/<address>): a hostile indexer can no
  // longer swap the curve or the coin a coin page trades for another coin the (permissionless) factory launched
  if (c.kind === "coin" && (!pc || low(r.address) !== low(pc))) return `That would call ${r.address}, not the coin this page is about (${pc ?? "none"}); nothing was sent.`;
  if (c.kind === "curve") {
    let token: string | null = null;
    try { token = await read.tokenOf(r.address as Hex); } catch { token = null; }
    if (!pc || !token || low(token) !== low(pc)) return `The curve ${r.address} is not the curve of the coin this page is about (${pc ?? "none"}); nothing was sent.`;
  }
  const args = r.args ?? [];
  const fn = (r.abi as AbiFn[]).find((x) => x.type === "function" && x.name === r.functionName && (x.inputs ?? []).length === args.length);
  if (!fn) return `The call ${r.functionName} does not match its ABI; nothing was sent.`;
  // docs/v7/dormant-collections.md §3.3: the bond sent is the stack file's, exactly (the desk refuses any other; a page never sends more)
  if (c.kind === "wakeDesk") {
    const bond = s.wakeBond !== undefined ? BigInt(s.wakeBond) : null;
    if (bond === null || r.value !== bond) return `The wake request would send ${r.value ?? 0n} wei, not the stack file's bond (${bond ?? "none named"}); nothing was sent.`;
  }
  const ctx: Ctx = { s, kind: c.kind, fn: r.functionName, account, read, pageCoin: pc };
  for (const [i, p] of (fn.inputs ?? []).entries()) { const bad = await paramRefusal(ctx, p, args[i]); if (bad) return bad; }
  return null;
}

/** E-07's first layer: in Phase 1 the address a page asks a wallet to call or approve comes from the stack file. When the indexer
 *  names another for the same role, the page says both and sends nothing. */
export function pinnedAddress(stack: StackFile | null, slug: string | null | undefined, role: "router" | "wrapper" | "factory" | "raffleVault", served: string | null | undefined):
  { address: Hex | null; why: string | null } {
  const col = stackCollection(stack, slug);
  const pinned = col ? (col[role] as Hex | null | undefined) ?? null : null;
  const word = { router: "router", wrapper: "wrapper", factory: "factory", raffleVault: "raffle vault" }[role];
  if (!col) return { address: null, why: `This build's stack file has no ${slug ?? "such"} collection, so there is no ${word} to send to.` };
  if (!pinned || ZERO.test(pinned)) return { address: null, why: `This build's stack file names no ${word} for ${col.slug}.` };
  if (served && isAddr(served) && low(served) !== low(pinned)) return { address: null, why: `The indexer names ${word} ${served} for ${col.slug}, but this build's stack file names ${pinned}. Nothing is sent while they differ.` };
  return { address: pinned, why: null };
}
