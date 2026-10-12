import { closeSync, existsSync, fsyncSync, mkdirSync, openSync, readFileSync, renameSync, writeSync } from "node:fs";
import { join } from "node:path";
import { decodeFunctionData, encodeFunctionData, type Hex } from "viem";
import { preJudgeEth } from "../ethnft.js";
import type { Log } from "../log.js";
import { ETH_NATIVE, RELAY_DEPOSITORY_4663 } from "../relay.js";
import type { EvmSigner, SolSigner } from "./keys.js";
import type { FuelIo } from "./io.js";
import type { SolIo } from "./solio.js";
import { allowedSol, b58, compileLegacy, cuLimitIx, cuPriceIx, depositIx, judgeSolQuote, minOutFloor, RELAY_SOL, s3Reads,
  solQuoteBody, SYSTEM_PROGRAM, transferIx, txBytes, type Ix, type Judged, type SolKind } from "./sol.js";
import { ABI, CKETH_FEE_WEI, CKETH_HELPER, depositEthData, ETHEREUM, IC, ICP_FEE_E8S, ROBINHOOD, SEL, topUpAccount } from "./pinned.js";
import { agree, decide, judgeEndToEnd, judgeSwap, split, T, type Asset, type Conversion, type FuelCaps, type Prices, type Target } from "./plan.js";

/**
 * docs/v7/cycles-from-fees.md §3–§4.2. The fueler's engine. One tick advances at most one step; every step is written to the journal on
 * /data BEFORE it is sent, so a crash between "sent" and "noted" never pays twice:
 *  - an EVM transaction is signed with the next nonce and journaled (raw bytes) before its broadcast; after a restart the same raw
 *    transaction is broadcast again, which can only mine once (the nonce);
 *  - the pool's legs are read back from the pool's own per-user balance before they are repeated;
 *  - the ICP transfer carries a journaled created-at time, so the ledger answers a repeat with the first transfer's block;
 *  - `notify_top_up` is idempotent at the CMC for a block.
 * Nothing here can name a third party except FUELER_PAYEE (the cold address) and the CMC's top-up account of the buyer or the timelock.
 *
 * Two homes (cfg.home): "robinhood" (the original: claim, allocate, Relay and forward on 4663 with the EVM key) and "books"
 * (docs/v9/canister-only-solana.md §9.2, the canister-only Solana home: Relay SOL → ETH and forward on Solana with the Solana hot key =
 * the canister's `treasury.ops`; no program, no allocate: the canister's own cranks pay the ops share `K_tre → ops` to the hot key once
 * it reaches 0.1 SOL, and the fueler watches that income through the canister's `treasury_status`). The programs' home (`solana`,
 * docs/v9/fueler-solana.md, rook_core's `allocate`) is retired with rook_core (decision D-l); its journal reads as the books home's.
 * From the ETH on
 * Ethereum onwards (the helper, ckETH, the pool, the CMC) both are the same code. On Solana a signed transaction is journaled with its
 * blockhash's last valid height before it is sent; it is sent again (the same bytes) until it is finalized, or until every provider's
 * finalized height is past that height with no trace of it, when it can never land and the step is redone.
 */

export type Chain = typeof ROBINHOOD | typeof ETHEREUM;
export type Pending = { chain: Chain; kind: "claim" | "allocate" | "forward" | "relay" | "helper"; nonce: number; raw: Hex; hash: Hex; to: Hex; value: bigint; at: number };
export type Phase = "quote" | "relay-sent" | "bridging" | "bridged" | "helper-sent" | "minting" | "minted" | "approved" | "deposited" | "swapped" | "withdrawn" | "topping" | "done" | "failed";
export type Top = { canister: string; e8s: bigint; createdAtNs: bigint; block?: bigint; cycles?: bigint };
export type Run = { id: string; startedAt: number; phase: Phase; phaseAt: number; targets: Target[]; wei: bigint; expectCycles: bigint; prices: Prices; xdr: bigint;
  before: Record<string, string>; relay?: { orderId: Hex; minOut: bigint; ethBefore: bigint; hash: string }; helper?: { hash: Hex; value: bigint; ckBefore: bigint };
  swap?: { ckIn: bigint; minOut: bigint; out?: bigint }; tops: Top[]; note?: string };
export type AlertKind = "cycles-autofund-failed" | "cycles-autofund-cap" | "cycles-autofund-short" | "cycles-income-short" | "cycles-autofund-oracle" | "fueler-halt";
export const ALERT_SEV: Record<AlertKind, "SEV1" | "SEV2" | "SEV3"> = { "fueler-halt": "SEV1", "cycles-autofund-failed": "SEV2", "cycles-autofund-cap": "SEV2",
  "cycles-autofund-oracle": "SEV2", "cycles-autofund-short": "SEV3", "cycles-income-short": "SEV3" };
export type Alert = { sev: "SEV1" | "SEV2" | "SEV3"; text: string; since: number };
export type Sent = { chain: Chain; nonce: number; hash: Hex; kind: Pending["kind"]; ok: boolean; at: number };
/** What the journal records of a Solana transaction: `allocate` is the retired programs home's (a journal with one still loads; none is ever built). */
export type SolJournalKind = SolKind | "allocate";
/** A Solana transaction out (the hot key's): its signature, its bytes (base64), and the height after which it can never land. */
export type SolPending = { kind: SolJournalKind; sig: string; raw: string; lastValid: number; at: number; lamports: bigint };
export type SolBook = { pending: SolPending | null; sent: { sig: string; kind: SolJournalKind; ok: boolean; at: number; note?: string }[]; used: string[] };
export type Home = "robinhood" | "books";
export type State = { version: 1; home?: Home; sol: SolBook; nonces: Partial<Record<Chain, number>>; pending: Partial<Record<Chain, Pending>>; run: Run | null; lastRunAt: number | null;
  conversions: Conversion[]; history: { id: string; startedAt: number; endedAt: number; phase: Phase; wei: string; cycles: string; note?: string }[];
  income: { at: number; totalToOps: string }[]; alerts: Partial<Record<AlertKind, Alert>>; sent: Sent[]; verify: { canister: string; before: string; minted: string; at: number }[];
  lastTickAt: number | null; lastDecision: string | null; halted: string | null };

export const emptyState = (): State => ({ version: 1, sol: { pending: null, sent: [], used: [] }, nonces: {}, pending: {}, run: null, lastRunAt: null, conversions: [], history: [], income: [], alerts: {},
  sent: [], verify: [], lastTickAt: null, lastDecision: null, halted: null });

// ---- the journal ---------------------------------------------------------------------------------------------------------------

const replacer = (_k: string, v: unknown) => (typeof v === "bigint" ? { $n: v.toString() } : v);
const reviver = (_k: string, v: unknown) => (v && typeof v === "object" && "$n" in (v as object) && Object.keys(v as object).length === 1 ? BigInt((v as { $n: string }).$n) : v);

export interface Journal { load(): State; save(s: State): void }
export class FileJournal implements Journal {
  private readonly path: string;
  constructor(dir: string) { mkdirSync(dir, { recursive: true }); this.path = join(dir, "fueler.json"); }
  load(): State { return existsSync(this.path) ? { ...emptyState(), ...JSON.parse(readFileSync(this.path, "utf8"), reviver) } : emptyState(); }
  save(s: State): void {
    const tmp = `${this.path}.tmp`, fd = openSync(tmp, "w");
    try { writeSync(fd, JSON.stringify(s, replacer)); fsyncSync(fd); } finally { closeSync(fd); }
    renameSync(tmp, this.path);
  }
}
export class MemoryJournal implements Journal {
  raw: string = JSON.stringify(emptyState(), replacer);
  load(): State { return JSON.parse(this.raw, reviver); }
  save(s: State): void { this.raw = JSON.stringify(s, replacer); }
}

// ---- the allowlist -------------------------------------------------------------------------------------------------------------

const lc = (s: unknown) => String(s ?? "").toLowerCase();

/**
 * §4.2 item 1: the only transactions the fueler may sign. There is no generic send: every signature goes through `sendEvm`, which
 * calls this first. Throws the refusal.
 */
export function allowed(chain: Chain, tx: { to: Hex; data: Hex; value: bigint }, ctx: { self: Hex; treasury: Hex; payee: Hex; principal: string }): string {
  const to = lc(tx.to), data = lc(tx.data);
  if (chain === ROBINHOOD) {
    if (to === RELAY_DEPOSITORY_4663 && data.startsWith(SEL.depositNative) && data.length === 2 + 68 * 2 && `0x${data.slice(34, 74)}` === lc(ctx.self)) return "relay";
    // the curve pays its own `platform` (the treasury), never the caller: value 0, any curve the indexer named
    if (data === SEL.claimPlatformFees && tx.value === 0n) return "claim";
    if (to === lc(ctx.treasury) && data === SEL.allocate && tx.value === 0n) return "allocate";
    if (to === lc(ctx.payee) && (data === "0x" || data === "") && tx.value > 0n) return "forward";
  }
  if (chain === ETHEREUM && to === CKETH_HELPER && data === lc(depositEthData(ctx.principal))) return "helper";
  throw new Error(`refused: no rule lets the fueler sign ${tx.value} wei to ${tx.to} on chain ${chain} with data ${String(tx.data).slice(0, 10)}…`);
}

// ---- the engine ----------------------------------------------------------------------------------------------------------------

export type EngineCfg = { mode: "dry" | "send"; caps: FuelCaps; buyer: string; timelock: string | null; treasury: Hex; payee: Hex;
  claimMinWei: bigint; allocateMinWei: bigint; ethMaxFeeWei: bigint; stuckS: number; mintStuckS: number; watchAddress?: Hex;
  /** robinhood when unset (the original); books: `sol` is required and `caps` are in lamports */
  home?: Home; sol?: SolCfg;
  /** `fueler.ts --dry-run --assume-cycles-t N` only: plan as if the buyer held this (a canister not installed yet); never in a service */
  assumeCycles?: bigint;
  /** `--dry-run --assume-sol N` only: plan as if the (throwaway) watched hot key held this many lamports */
  assumeBalance?: bigint };

/** docs/v9/fueler-solana.md §3: the books home's Solana settings (lamports). */
export type SolCfg = { payee: string; forwardMinLamports: bigint;
  relayMaxFeeBps: number; slippageBps: number; priceMicro: bigint; units: number; watchAddress?: string };

export type Deps = { io: FuelIo; signer: EvmSigner | null; journal: Journal; cfg: EngineCfg; log: Log; nowS?: () => number;
  sol?: { io: SolIo; signer: SolSigner | null } };

const HOUR = 3_600;
const ETH_TIP_WEI = 500_000_000n;                       // 0.5 gwei on Ethereum; 4663 (Arbitrum) ignores the tip
const HELPER_GAS = 120_000n;
const fmtE = (w: bigint) => `${(Number(w) / 1e18).toFixed(5)} ETH`;
const fmtA = (w: bigint, a: Asset) => (a === "SOL" ? `${(Number(w) / 1e9).toFixed(5)} SOL` : fmtE(w));
const SOL_GAS_RESERVE = 1_000_000n;                     // 0.001 SOL: the next transactions' fees, above the floor

export class Engine {
  s: State;
  private readonly now: () => number;
  /** what a dry run would do, in order (fueler.ts --dry-run prints it) */
  plan: string[] = [];
  constructor(private readonly d: Deps) {
    this.s = d.journal.load();
    this.s.sol ??= { pending: null, sent: [], used: [] };
    this.now = d.nowS ?? (() => Math.floor(Date.now() / 1000));
    if (this.onSol && (!d.cfg.sol || !d.sol)) throw new Error("the books home needs its Solana configuration and its Solana I/O");
    // the retired programs home's journal (the same hot key, the same unit) is the books home's
    if ((this.s.home as string) === "solana") this.s.home = "books";
  }

  get home(): Home { return this.d.cfg.home ?? "robinhood"; }
  /** The hot key is a Solana key and the first leg is Relay SOL → ETH: the books home. */
  get onSol(): boolean { return this.home === "books"; }
  get asset(): Asset { return this.onSol ? "SOL" : "ETH"; }
  get self(): Hex { if (!this.d.signer) throw new Error("no FUELER_EVM_KEY: nothing can be signed"); return this.d.signer.address; }
  /** The Ethereum address Relay pays and the helper is called from: the EVM key's, or in a keyless dry run the watched one. */
  private evmAddr(): Hex | null { return this.d.signer?.address ?? this.d.cfg.watchAddress ?? null; }
  /** The Solana hot key (the canister's treasury.ops), or in a keyless dry run the watched address. */
  get solAddr(): string | null { return this.d.sol?.signer?.address ?? this.d.cfg.sol?.watchAddress ?? null; }
  private would(what: string, o: Record<string, unknown> = {}) { this.plan.push(`${what} ${JSON.stringify(o, (_k, v) => (typeof v === "bigint" ? String(v) : v))}`); this.d.log("info", what, o); }
  private save() { this.d.journal.save(this.s); }
  alert(kind: AlertKind, text: string) {
    const prev = this.s.alerts[kind];
    this.s.alerts[kind] = { sev: ALERT_SEV[kind], text, since: prev?.since ?? this.now() };
    if (!prev || prev.text !== text) this.d.log(ALERT_SEV[kind] === "SEV3" ? "warn" : "error", kind, { text });
  }
  clear(kind: AlertKind) { delete this.s.alerts[kind]; }
  private halt(why: string) { this.s.halted = why; this.alert("fueler-halt", why); this.save(); }

  /** The nonces the monitor holds the chain to (`ops-unexpected-tx`): the next nonce this fueler would sign on each chain. */
  noncesForStatus() { return this.s.nonces; }

  /** One tick. Never throws: a failure is logged and the next tick tries again. */
  async tick(): Promise<void> {
    const now = this.now();
    this.s.lastTickAt = now;
    try {
      if (this.s.halted) { this.d.log("error", "halted", { why: this.s.halted }); return; }
      await this.verifyTopUps();
      const sol = this.onSol;
      // the origin chain's transaction out (4663 or Solana), then Ethereum's
      const busyO = sol ? await this.settleSol() : await this.settle(ROBINHOOD), busy1 = await this.settle(ETHEREUM);
      if (this.s.run) { await this.advance(busyO, busy1); return; }
      if (!busyO && (await (sol ? this.housekeepingSol() : this.housekeeping()))) return;
      await this.maybeStart(busyO);
    } catch (err) {
      this.d.log("error", "tick-failed", { error: String((err as Error).message).slice(0, 300) });
      if (this.s.run) this.s.run.note = String((err as Error).message).slice(0, 300);
    } finally { this.save(); }
  }

  // ---- EVM: one transaction in flight per chain --------------------------------------------------------------------------------

  /** The nonce book on first use: the chain's own count (the monitor's baseline). */
  private async nextNonce(chain: Chain): Promise<number> {
    if (this.s.nonces[chain] === undefined) this.s.nonces[chain] = await this.d.io.evmNonce(chain, this.self);
    return this.s.nonces[chain]!;
  }

  /** Check, sign, JOURNAL, then broadcast. */
  private async sendEvm(chain: Chain, kind: Pending["kind"], tx: { to: Hex; data: Hex; value: bigint },
    o: { onSigned?: (p: Pending) => void; maxFee?: bigint; maxGas?: bigint } = {}): Promise<Pending> {
    if (this.s.pending[chain]) throw new Error(`a ${this.s.pending[chain]!.kind} transaction is already out on chain ${chain}`);
    if (this.onSol && chain !== ETHEREUM) throw new Error("refused: the Solana home signs nothing on 4663");
    const rule = allowed(chain, tx, { self: this.self, treasury: this.d.cfg.treasury, payee: this.d.cfg.payee, principal: this.d.io.principal() });
    if (rule !== kind) throw new Error(`refused: a ${kind} step built a ${rule} transaction`);
    const nonce = await this.nextNonce(chain);
    const tip = chain === ETHEREUM ? ETH_TIP_WEI : 0n;
    let maxFee = o.maxFee ?? (await this.d.io.evmBaseFee(chain)) * 3n + tip;
    if (chain === ETHEREUM && maxFee > this.d.cfg.ethMaxFeeWei) throw new Error(`Ethereum would need ${maxFee} wei a gas, over the cap ${this.d.cfg.ethMaxFeeWei}: waits`);
    if (maxFee === 0n) maxFee = 1n;
    const est = await this.d.io.evmEstimate(chain, { from: this.self, ...tx });
    const gas = (est * 13n) / 10n;
    if (o.maxGas !== undefined && gas > o.maxGas) throw new Error(`the ${kind} transaction would need ${gas} gas, over its ${o.maxGas}: refused`);
    const { raw, hash } = await this.d.signer!.sign({ type: "eip1559", chainId: chain, nonce, to: tx.to, data: tx.data, value: tx.value, gas,
      maxFeePerGas: maxFee, maxPriorityFeePerGas: tip });
    const p: Pending = { chain, kind, nonce, raw, hash, to: tx.to, value: tx.value, at: this.now() };
    this.s.pending[chain] = p;
    this.s.nonces[chain] = nonce + 1;
    o.onSigned?.(p);                                     // the run's own record of it, in the same save
    this.save();                                         // journaled before anything leaves
    await this.d.io.evmBroadcast(chain, raw);
    this.d.log("info", "sent", { chain, kind, nonce, hash, value: tx.value });
    return p;
  }

  /** The transaction out on `chain`, resolved if it can be. Returns true while it is still out. */
  private async settle(chain: Chain): Promise<boolean> {
    const p = this.s.pending[chain];
    if (!p) return false;
    const r = await this.d.io.evmReceipt(chain, p.hash);
    if (r) {
      this.s.sent = [...this.s.sent, { chain, nonce: p.nonce, hash: p.hash, kind: p.kind, ok: r.ok, at: this.now() }].slice(-200);
      delete this.s.pending[chain];
      this.d.log(r.ok ? "info" : "error", r.ok ? "mined" : "reverted", { chain, kind: p.kind, hash: p.hash });
      if (this.s.run) this.onMined(p, r.ok);
      return false;
    }
    const used = await this.d.io.evmNonce(chain, this.self);
    if (used > p.nonce) {
      // the nonce is used but our transaction has no receipt: a provider is behind, or another transaction took the nonce
      if (this.now() - p.at > 15 * 60) this.alert("cycles-autofund-failed", `the fueler's ${p.kind} transaction ${p.hash} on chain ${chain} has no receipt although nonce ${p.nonce} is used: look it up on the explorer (ops-unexpected-tx may be paging too)`);
      return true;
    }
    await this.d.io.evmBroadcast(chain, p.raw);           // the same bytes: it can only mine once
    if (this.now() - p.at > this.d.cfg.stuckS) this.alert("cycles-autofund-failed", `the fueler's ${p.kind} transaction ${p.hash} on chain ${chain} has not mined for ${Math.round((this.now() - p.at) / 60)} min`);
    return true;
  }

  private onMined(p: Pending, ok: boolean) {
    const run = this.s.run!;
    if (p.kind === "relay" && run.phase === "relay-sent") {
      if (ok) this.phase("bridging"); else this.fail("the Relay deposit reverted: no ETH left the wallet but its gas");
    }
    if (p.kind === "helper" && run.phase === "helper-sent") {
      if (ok) this.phase("minting"); else this.fail("the ckETH helper deposit reverted on Ethereum: the ETH is still at the fueler's Ethereum address (the next run deposits it)");
    }
  }

  // ---- housekeeping: claim, allocate, forward ----------------------------------------------------------------------------------

  /** One housekeeping transaction a tick, when no run is open. Returns true when it sent (or, in a dry run, would have sent) one. */
  private async housekeeping(): Promise<boolean> {
    const io = this.d.io, c = this.d.cfg, dry = c.mode === "dry";
    // the platform's curve fees to the treasury
    for (const curve of await io.curves().catch(() => [] as Hex[])) {
      const owed = BigInt(await io.evmCall(ROBINHOOD, curve, encodeFunctionData({ abi: ABI, functionName: "platformOwed" })).catch(() => "0x0"));
      if (owed >= c.claimMinWei) {
        if (dry) { this.would("would-claim", { curve, owed }); continue; }
        await this.sendEvm(ROBINHOOD, "claim", { to: curve, data: SEL.claimPlatformFees, value: 0n });
        return true;
      }
    }
    // the treasury's split (its ops share comes here)
    const unalloc = BigInt(await io.evmCall(ROBINHOOD, c.treasury, encodeFunctionData({ abi: ABI, functionName: "unallocated" })));
    await this.incomeMark(async () => BigInt(await io.evmCall(ROBINHOOD, c.treasury, encodeFunctionData({ abi: ABI, functionName: "totalToOps" }))));
    if (unalloc >= c.allocateMinWei) {
      if (dry) this.would("would-allocate", { unallocated: unalloc });
      else { await this.sendEvm(ROBINHOOD, "allocate", { to: c.treasury, data: SEL.allocate, value: 0n }); return true; }
    }
    // what the wallet holds above its buffer, to the cold payee (a stolen key takes at most the buffer and the caps)
    if (!this.d.signer) return false;
    const bal = await io.evmBalance(ROBINHOOD, this.self);
    const gas = 2_000_000_000_000_000n;                   // 0.002 ETH kept for the forward's own gas and the next claims
    if (bal > c.caps.bufferWei + gas) {
      const value = bal - c.caps.bufferWei - gas;
      if (dry) { this.would("would-forward", { value }); return false; }
      await this.sendEvm(ROBINHOOD, "forward", { to: c.payee, data: "0x", value });
      return true;
    }
    return false;
  }

  /** §3.4: the treasury's `totalToOps` once a day; over 14 days, ops income under what the fueler converted pages SEV3. */
  private async incomeMark(readTotal: () => Promise<bigint>) {
    const now = this.now(), last = this.s.income.at(-1);
    if (last && now - last.at < 86_400) return;
    const total = await readTotal();
    const fmtE = (w: bigint) => fmtA(w, this.asset);
    this.s.income = [...this.s.income, { at: now, totalToOps: total.toString() }].slice(-60);
    const old = [...this.s.income].reverse().find((m) => now - m.at >= 14 * 86_400);
    if (!old) return;
    const income = total - BigInt(old.totalToOps);
    const spent = this.s.conversions.filter((x) => x.at > old.at).reduce((a, x) => a + x.wei, 0n);
    if (spent > income) this.alert("cycles-income-short", `over 14 days the ops share brought ${fmtE(income)} and the fueler converted ${fmtE(spent)}: the platform's fees do not cover its cycles (cycles-from-fees §1.3: the opsBps question, or keep topping up)`);
    else this.clear("cycles-income-short");
  }

  // ---- starting a run ------------------------------------------------------------------------------------------------------------

  async readPrices(): Promise<Prices | null> {
    const { cg, cb } = await this.d.io.prices();
    const a = agree(cg, cb, this.now(), 200, this.onSol);
    if (!a.prices) { this.alert("cycles-autofund-oracle", `no run: ${a.why}`); return null; }
    this.clear("cycles-autofund-oracle");
    return a.prices;
  }

  private async maybeStart(busy4663: boolean) {
    const io = this.d.io, c = this.d.cfg, sol = this.onSol, asset = this.asset;
    const buyerCycles = c.assumeCycles ?? await io.cycles(c.buyer);
    const timelockCycles = c.timelock && c.assumeCycles === undefined ? await io.cycles(c.timelock).catch(() => null) : null;
    const prices = await this.readPrices();
    if (!prices) return;
    const xdr = await io.xdrRate();
    const asIfFunded = c.caps.floorWei + c.caps.weekMaxWei + (sol ? SOL_GAS_RESERVE : 1_000_000_000_000_000n);   // a dry run with no key plans as if funded
    const opsWei = sol ? (c.assumeBalance ?? (this.solAddr ? await this.d.sol!.io.solBalance(this.solAddr) : asIfFunded))
      : this.d.signer ? await io.evmBalance(ROBINHOOD, this.self) : c.watchAddress ? await io.evmBalance(ROBINHOOD, c.watchAddress) : asIfFunded;
    const dec = decide({ nowS: this.now(), lastRunAt: this.s.lastRunAt, buyerCycles, timelockCycles, opsWei, gasReserveWei: sol ? SOL_GAS_RESERVE : 1_000_000_000_000_000n,
      conversions: this.s.conversions, prices, xdrPermyriadPerIcp: xdr, caps: c.caps, buyer: c.buyer, timelock: c.timelock, asset });
    this.s.lastDecision = dec.kind === "run" ? `run: ${fmtA(dec.wei, asset)} for ${dec.targets.map((t) => `${(Number(t.cycles) / 1e12).toFixed(2)} T to ${t.canister}`).join(", ")}`
      : `${dec.kind}: ${dec.why}`;
    if (c.mode === "dry") this.would("decision", { decision: this.s.lastDecision, buyerCycles, opsBalance: opsWei, asset, prices });
    if (dec.kind === "blocked") { this.alert(dec.alert, dec.why); return; }
    this.clear("cycles-autofund-cap"); this.clear("cycles-autofund-short");
    if (dec.kind !== "run") return;
    if (c.mode === "dry") {
      this.would("would-run", { decision: this.s.lastDecision });
      // the Solana home also takes and judges the quote it would sign (nothing is signed): the plan shows the order and its minimum
      if (sol) {
        const hot = this.solAddr, rec = this.evmAddr();
        if (!hot || !rec) { this.would("would-quote", { skipped: "no hot key or watched Solana address, or no Ethereum recipient (FUELER_SOL_WATCH_ADDRESS, FUELER_WATCH_ADDRESS)" }); return; }
        try {
          const q = await io.relayQuote(solQuoteBody(hot, rec, dec.wei, c.sol!.slippageBps));
          const j = await this.judgeSol(q, dec.wei, prices, hot, rec);
          this.would("would-deposit", { relayOrder: j.orderId, lamports: j.lamports, minOutWei: j.minOut, minOutEth: Number(j.minOut) / 1e18, deadline: j.deadline,
            to: RELAY_SOL.DEPOSITORY_PROGRAM, recipient: rec, refundTo: hot, expectedOutWei: q?.details?.currencyOut?.amount ?? null });
        } catch (e) { this.would("would-deposit", { refused: (e as Error).message.slice(0, 300) }); }
      }
      return;
    }
    if (busy4663) return;
    const before: Record<string, string> = { [c.buyer]: buyerCycles.toString() };
    if (c.timelock && timelockCycles !== null) before[c.timelock] = timelockCycles.toString();
    const now = this.now();
    this.s.run = { id: `run-${now}`, startedAt: now, phase: "quote", phaseAt: now, targets: dec.targets, wei: dec.wei, expectCycles: dec.expectCycles, prices, xdr, before, tops: [] };
    this.s.lastRunAt = now;
    this.d.log("info", "run-started", { id: this.s.run.id, decision: this.s.lastDecision });
    this.save();
  }

  // ---- the run -------------------------------------------------------------------------------------------------------------------

  private phase(p: Phase) { const r = this.s.run!; r.phase = p; r.phaseAt = this.now(); r.note = undefined; this.d.log("info", "run-phase", { id: r.id, phase: p }); }
  private fail(why: string) {
    const r = this.s.run!;
    r.phase = "failed"; r.note = why;
    this.alert("cycles-autofund-failed", `run ${r.id} failed: ${why}. Top up by hand from the fallback reserve if the buyer needs it (golive-runbook §6); the journal is on /data`);
    this.finish();
  }
  private finish() {
    const r = this.s.run!;
    const minted = r.tops.reduce((a, t) => a + (t.cycles ?? 0n), 0n);
    // the caps count what left the wallet, and once known, what it became
    const conv = this.s.conversions.find((x) => x.at === r.startedAt);
    if (conv && minted > 0n) conv.cycles = minted;
    if (r.phase === "failed" && !r.relay) this.s.conversions = this.s.conversions.filter((x) => x.at !== r.startedAt);
    this.s.history = [...this.s.history, { id: r.id, startedAt: r.startedAt, endedAt: this.now(), phase: r.phase, wei: r.wei.toString(), cycles: minted.toString(), note: r.note }].slice(-30);
    this.s.verify = r.tops.filter((t) => t.cycles).map((t) => ({ canister: t.canister, before: r.before[t.canister] ?? "0", minted: String(t.cycles), at: this.now() }));
    this.s.run = null;
    this.d.log("info", "run-ended", { id: r.id, phase: r.phase, minted });
  }

  private stuck(after: number, what: string) {
    const r = this.s.run!;
    if (this.now() - r.phaseAt > after) this.alert("cycles-autofund-failed", `run ${r.id}: ${what} for ${Math.round((this.now() - r.phaseAt) / 60)} min (phase ${r.phase})`);
  }

  private async advance(busy4663: boolean, busy1: boolean) {
    const io = this.d.io, c = this.d.cfg, r = this.s.run!;
    switch (r.phase) {
      case "quote": {
        if (this.onSol) return this.quoteSol(r, busy4663);
        if (busy4663) return;
        // §4.2 item 1: the live answers must match the pinned ones, or nothing moves
        const helper = await io.minterHelper();
        if (helper !== CKETH_HELPER) return this.halt(`the ckETH minter names helper ${helper}, not the pinned ${CKETH_HELPER}: halted (a pinned address must be re-reviewed before it changes)`);
        const tok = await io.poolTokens();
        if (tok.token0 !== IC.ICP_LEDGER || tok.token1 !== IC.CKETH_LEDGER) return this.halt(`the pool ${IC.POOL_ICP_CKETH} trades ${tok.token0}/${tok.token1}, not ICP/ckETH: halted`);
        const self = this.self;
        const ask = { buy: 0n, amountWei: r.wei, user: self, recipient: self, refundTo: self, originChainId: ROBINHOOD, destinationChainId: ETHEREUM };
        // nothing has left the wallet in this phase: after 6 h without an acceptable quote the run gives up, and housekeeping resumes
        const giveUp = (why: string) => { r.note = why; if (this.now() - r.phaseAt > 6 * HOUR) return this.fail(`${why}; for 6 h, so the run gave up (nothing left the wallet)`); this.stuck(2 * HOUR, "no acceptable Relay quote"); };
        let q: any; // eslint-disable-line @typescript-eslint/no-explicit-any
        try { q = await io.relayQuote({ user: self, originChainId: ROBINHOOD, destinationChainId: ETHEREUM, originCurrency: ETH_NATIVE, destinationCurrency: ETH_NATIVE,
          recipient: self, refundTo: self, tradeType: "EXACT_INPUT", amount: r.wei.toString(),
          // measured 2026-10-09: Relay's automatic slippage puts the guaranteed minimum 2.3 % under the amount, which §3.3's 1.5 % bound
          // refuses every time; at 0.5 % the minimum is 0.74–0.91 % under (0.005–0.02 ETH) and the bound holds
          slippageTolerance: "50" }); }
        catch (e) { return giveUp(`Relay did not quote: ${(e as Error).message}`); }
        let judged: { orderId: Hex; minOut: bigint };
        try { judged = preJudgeEth(q, ask, this.now(), 150); }        // §3.3: min-out ≥ 98.5 %, stricter than the canister's 3 %
        catch (e) { return giveUp(`Relay quote refused: ${(e as Error).message}`); }
        const ethBefore = await io.evmBalance(ETHEREUM, self);
        const d = q.steps[0].items[0].data;
        await this.sendEvm(ROBINHOOD, "relay", { to: d.to as Hex, data: d.data as Hex, value: BigInt(d.value) }, { onSigned: (p) => {
          // counted against the caps the moment the ETH can leave; the phase moves in the same save as the signed deposit, so a restart
          // never quotes and pays again
          this.s.conversions = [...this.s.conversions.filter((x) => x.at > this.now() - 30 * 86_400), { at: r.startedAt, wei: r.wei, cycles: r.expectCycles }];
          r.relay = { orderId: judged.orderId, minOut: judged.minOut, ethBefore, hash: p.hash };
          this.phase("relay-sent");
        } });
        return;
      }
      case "relay-sent": return this.stuck(c.stuckS, "the Relay deposit has not mined");
      case "bridging": {
        const bal = await io.evmBalance(ETHEREUM, this.self);
        if (bal >= r.relay!.ethBefore + r.relay!.minOut) return this.phase("bridged");
        return this.stuck(2 * HOUR, `the Relay fill (order ${r.relay!.orderId}) has not reached the fueler's Ethereum address`);
      }
      case "bridged": {
        if (busy1) return;
        const bal = await io.evmBalance(ETHEREUM, this.self);
        const maxFee = (await io.evmBaseFee(ETHEREUM)) * 3n + ETH_TIP_WEI;
        const reserve = HELPER_GAS * maxFee;             // the deposit's gas comes out of the bridged ETH: nothing is left behind
        if (bal <= reserve) return this.fail(`the fueler's Ethereum address holds ${fmtE(bal)}, not more than the deposit's gas`);
        const value = bal - reserve;
        const ckBefore = await io.ckBalance();
        await this.sendEvm(ETHEREUM, "helper", { to: CKETH_HELPER as Hex, data: depositEthData(io.principal()), value }, { maxFee, maxGas: HELPER_GAS,
          onSigned: (p) => { r.helper = { hash: p.hash, value, ckBefore }; this.phase("helper-sent"); } });
        return;
      }
      case "helper-sent": return this.stuck(c.stuckS, "the ckETH helper deposit has not mined");
      case "minting": {
        const ck = await io.ckBalance();
        if (ck >= r.helper!.ckBefore + r.helper!.value) return this.phase("minted");
        return this.stuck(c.mintStuckS, "ckETH has not been minted for the deposit");
      }
      case "minted": {
        const u = await io.poolUnused();
        if (u.ck > 0n || u.icp > 0n) return this.phase(u.ck > 0n ? "deposited" : "swapped");    // a crash after the deposit or the swap
        const ck = await io.ckBalance();
        if (ck <= 3n * CKETH_FEE_WEI) return this.fail("no ckETH to swap");
        await io.ckApprove(ck - CKETH_FEE_WEI);          // the approve's own fee comes out of the balance
        return this.phase("approved");
      }
      case "approved": {
        const u = await io.poolUnused();
        if (u.ck > 0n) return this.phase("deposited");
        const ck = await io.ckBalance();
        await io.poolDeposit(ck - CKETH_FEE_WEI);        // transfer_from charges its fee on top
        return this.phase("deposited");
      }
      case "deposited": {
        const u = await io.poolUnused();
        if (u.ck === 0n && u.icp > 0n) return this.phase("swapped");
        if (u.ck === 0n) return this.fail("the pool holds no ckETH of ours to swap");
        const prices = await this.readPrices();
        if (!prices) return this.stuck(2 * HOUR, "no agreeing price to judge the swap");
        const xdr = await io.xdrRate();
        const quote = await io.poolQuote(u.ck);
        let minOut: bigint;
        try { minOut = judgeSwap(u.ck, quote, prices).minOut; judgeEndToEnd(r.wei, quote, prices, xdr, 9_500, this.asset); }
        catch (e) { r.note = (e as Error).message; this.stuck(2 * HOUR, "the swap is refused by its price checks"); return; }
        r.swap = { ckIn: u.ck, minOut };
        r.swap.out = await io.poolSwap(u.ck, minOut);
        return this.phase("swapped");
      }
      case "swapped": {
        const u = await io.poolUnused();
        if (u.icp > ICP_FEE_E8S) await io.poolWithdraw(u.icp);
        return this.phase("withdrawn");
      }
      case "withdrawn": {
        const icp = await io.icpBalance();
        const parts = split(icp, r.targets, ICP_FEE_E8S);
        if (!parts.length) return this.fail(`the fueler's ICP account holds ${icp} e8s, too little to top anything up`);
        const ns = BigInt(this.now()) * 1_000_000_000n;
        r.tops = parts.map((p, i) => ({ canister: p.canister, e8s: p.e8s, createdAtNs: ns + BigInt(i) }));
        return this.phase("topping");                      // journaled (with the created-at times) before any transfer
      }
      case "topping": {
        for (const t of r.tops) {
          if (t.cycles !== undefined) continue;
          if (t.canister !== c.buyer && t.canister !== c.timelock) return this.halt(`refused: a top-up names ${t.canister}, neither the buyer nor the timelock`);
          if (t.block === undefined) { t.block = await io.icpTransfer(topUpAccount(t.canister), t.e8s, t.createdAtNs); this.save(); }
          const n = await io.notifyTopUp(t.block, t.canister);
          if ("ok" in n) { t.cycles = n.ok; this.d.log("info", "topped-up", { canister: t.canister, cycles: n.ok, block: t.block }); continue; }
          if ("processing" in n) return;
          return this.fail(`notify_top_up(${t.block}, ${t.canister}): ${"refunded" in n ? `refunded (${n.refunded}); the ICP is back in the fueler's account for the next run` : n.err}`);
        }
        this.phase("done");
        this.clear("cycles-autofund-failed");
        return this.finish();
      }
      default: return;
    }
  }

  // ---- the books home (docs/v9/fueler-solana.md, docs/v9/canister-only-solana.md §9.2) ---------------------------------------------

  private get solCtx() {
    const c = this.d.cfg.sol!;
    return { hot: this.d.sol!.signer!.address, payee: c.payee, maxPriceMicro: c.priceMicro, maxUnits: c.units };
  }

  /** Check (allowedSol), simulate, sign, JOURNAL, then send. The only Solana signing site. */
  private async sendSol(kind: SolKind, ix: Ix, o: { lamports: bigint; onSigned?: (p: SolPending) => void }): Promise<SolPending> {
    if (this.s.sol.pending) throw new Error(`a Solana ${this.s.sol.pending.kind} transaction is already out`);
    if (!this.d.sol?.signer) throw new Error("no Solana hot key: nothing can be signed on Solana");
    const c = this.d.cfg.sol!, io = this.d.sol.io, ctx = this.solCtx;
    const ixs = [cuLimitIx(c.units), ...(c.priceMicro > 0n ? [cuPriceIx(c.priceMicro)] : []), ix];
    const rule = allowedSol(ixs, ctx);
    if (rule !== kind) throw new Error(`refused: a ${kind} step built a ${rule} transaction`);
    const bh = await io.solBlockhash();
    const msg = compileLegacy(ctx.hot, ixs, bh.blockhash);
    const sim = await io.solSimulate(txBytes([new Uint8Array(64)], msg));
    if (sim.err) throw new Error(`the ${kind} transaction is refused in simulation: ${JSON.stringify(sim.err)} ${sim.logs.slice(-3).join(" | ").slice(0, 300)}`);
    const sig = this.d.sol.signer.signMessage(msg);
    const raw = txBytes([sig], msg);
    const p: SolPending = { kind, sig: b58(sig), raw: Buffer.from(raw).toString("base64"), lastValid: bh.lastValidBlockHeight, at: this.now(), lamports: o.lamports };
    this.s.sol.pending = p;
    o.onSigned?.(p);                                     // the run's own record of it, in the same save
    this.save();                                         // journaled before anything leaves
    await io.solSend(raw);
    this.d.log("info", "sent", { chain: "solana", kind, sig: p.sig, lamports: o.lamports });
    return p;
  }

  /** The Solana transaction out, resolved if it can be. Returns true while it is still out. */
  private async settleSol(): Promise<boolean> {
    const p = this.s.sol.pending;
    if (!p) return false;
    const io = this.d.sol!.io;
    const st = await io.solStatus(p.sig);
    const done = (ok: boolean, note?: string) => {
      this.s.sol.sent = [...this.s.sol.sent, { sig: p.sig, kind: p.kind, ok, at: this.now(), ...(note ? { note } : {}) }].slice(-200);
      this.s.sol.pending = null;
    };
    if (st && st.confirmation === "finalized") {
      const ok = st.err === null || st.err === undefined;
      done(ok, ok ? undefined : JSON.stringify(st.err).slice(0, 200));
      this.d.log(ok ? "info" : "error", ok ? "mined" : "reverted", { chain: "solana", kind: p.kind, sig: p.sig });
      if (this.s.run && p.kind === "relay" && this.s.run.phase === "relay-sent") {
        const r = this.s.run;
        if (ok) this.phase("bridging");
        else { r.relay = undefined; this.fail(`the Relay deposit ${p.sig} failed on Solana (${JSON.stringify(st.err)}): no SOL left the hot key but its fee`); }
      }
      return false;
    }
    if (!st && (await io.solBlockHeight()) > p.lastValid) {
      // past its blockhash's last valid height at every provider, and no provider has it: it can never land
      done(false, "expired");
      this.d.log("warn", "expired", { chain: "solana", kind: p.kind, sig: p.sig });
      const r = this.s.run;
      if (r && p.kind === "relay" && r.phase === "relay-sent") {
        this.s.conversions = this.s.conversions.filter((x) => x.at !== r.startedAt);
        r.relay = undefined;
        this.phase("quote");                             // nothing left the wallet: a fresh quote, a fresh order
      }
      return false;
    }
    await io.solSend(Uint8Array.from(Buffer.from(p.raw, "base64")));   // the same bytes: it can only land once
    if (this.now() - p.at > this.d.cfg.stuckS) this.alert("cycles-autofund-failed", `the fueler's Solana ${p.kind} transaction ${p.sig} has not finalized for ${Math.round((this.now() - p.at) / 60)} min`);
    return true;
  }

  /** One Solana housekeeping transaction a tick, when no run is open: the income mark, then forward the surplus to the cold payee. */
  private async housekeepingSol(): Promise<boolean> {
    const c = this.d.cfg, sc = c.sol!, io = this.d.sol!.io, dry = c.mode === "dry", hot = this.solAddr;
    // the canister's cranks pay the ops share to its `treasury.ops` on their own (TreasuryStep::Ops, once 0.1 SOL is owed): nothing to
    // allocate; the income mark and the ops-key check come from its `treasury_status`
    const t = await this.d.io.booksTreasury(c.buyer).catch((err: Error) => { this.d.log("warn", "books-treasury-unread", { error: String(err.message).slice(0, 200) }); return null; });
    if (t) {
      await this.incomeMark(async () => t.totalToOps);
      if (hot && t.ops !== hot) this.would("ops-elsewhere", { ops: t.ops, hot, note: "the canister's treasury.ops is not this fueler's hot key: its ops share goes elsewhere (treasury_set_ops)" });
    }
    // what the hot key holds above its buffer and a day's cap, to the cold payee (a stolen key takes at most those)
    if (!hot) return false;
    const bal = await io.solBalance(hot), keep = c.caps.bufferWei + c.caps.dayMaxWei;
    if (bal > keep + sc.forwardMinLamports) {
      const value = bal - keep;
      if (dry) { this.would("would-forward", { to: sc.payee, lamports: value, keep }); return false; }
      await this.sendSol("forward", transferIx(hot, sc.payee, value), { lamports: value });
      return true;
    }
    return false;
  }

  /** S3's reads, then S1–S12 on the quote, with the oracle floor (S10). */
  private async judgeSol(q: unknown, lamports: bigint, prices: Prices, hot: string, recipient: string): Promise<Judged> {
    const io = this.d.sol!.io, sc = this.d.cfg.sol!;
    const state = await io.solAccount(RELAY_SOL.RELAY_DEPOSITORY), pd = await io.solAccount(RELAY_SOL.PROGRAMDATA, 45);
    if (!state || !pd) throw new Error("rule S3: the depository state or its ProgramData is missing");
    const floor = minOutFloor(lamports, prices.solUsd!, prices.ethUsd, sc.relayMaxFeeBps);
    return judgeSolQuote(q as Record<string, unknown>, { hot, recipient, lamports, minOutFloor: floor, s3: s3Reads(state, pd), nowS: this.now(), used: this.s.sol.used });
  }

  /** The run's first leg on Solana: the pinned IC pieces checked, a Relay SOL → ETH quote judged, the deposit signed by the hot key. */
  private async quoteSol(r: Run, busy: boolean) {
    if (busy) return;
    const io = this.d.io;
    const helper = await io.minterHelper();
    if (helper !== CKETH_HELPER) return this.halt(`the ckETH minter names helper ${helper}, not the pinned ${CKETH_HELPER}: halted (a pinned address must be re-reviewed before it changes)`);
    const tok = await io.poolTokens();
    if (tok.token0 !== IC.ICP_LEDGER || tok.token1 !== IC.CKETH_LEDGER) return this.halt(`the pool ${IC.POOL_ICP_CKETH} trades ${tok.token0}/${tok.token1}, not ICP/ckETH: halted`);
    const hot = this.d.sol!.signer!.address, rec = this.self;
    const giveUp = (why: string) => { r.note = why; if (this.now() - r.phaseAt > 6 * HOUR) return this.fail(`${why}; for 6 h, so the run gave up (nothing left the wallet)`); this.stuck(2 * HOUR, "no acceptable Relay quote"); };
    const prices = await this.readPrices();
    if (!prices) return giveUp("no agreeing price to judge the Relay quote");
    let q: unknown;
    try { q = await io.relayQuote(solQuoteBody(hot, rec, r.wei, this.d.cfg.sol!.slippageBps)); }
    catch (e) { return giveUp(`Relay did not quote: ${(e as Error).message}`); }
    let j: Judged;
    try { j = await this.judgeSol(q, r.wei, prices, hot, rec); }
    catch (e) { return giveUp(`Relay quote refused: ${(e as Error).message}`); }
    const ethBefore = await io.evmBalance(ETHEREUM, rec);
    await this.sendSol("relay", depositIx(hot, j.lamports, j.orderId), { lamports: j.lamports, onSigned: (p) => {
      // counted against the caps the moment the SOL can leave; the phase moves in the same save as the signed deposit
      this.s.conversions = [...this.s.conversions.filter((x) => x.at > this.now() - 30 * 86_400), { at: r.startedAt, wei: r.wei, cycles: r.expectCycles }];
      this.s.sol.used = [...this.s.sol.used, j.orderId].slice(-500);
      r.relay = { orderId: j.orderId, minOut: j.minOut, ethBefore, hash: p.sig };
      this.phase("relay-sent");
    } });
  }

  /** The books home's start-up checks. */
  private async preflightSol(): Promise<string[]> {
    const notes: string[] = [], sc = this.d.cfg.sol!, io = this.d.sol!.io, hot = this.d.sol!.signer?.address ?? null;
    if (!hot) notes.push("dry run: no Solana hot key (FUELER_SOL_KEYPAIR_FILE), nothing will be signed on Solana");
    if (!this.d.signer) notes.push("no FUELER_EVM_KEY: nothing will be signed on Ethereum (the helper deposit)");
    if (hot && sc.payee === hot) throw new Error("the cold payee is the hot key itself: refusing");
    if (this.d.signer) {
      const code = await this.d.io.evmCode(ETHEREUM, this.self);
      if (code !== "0x") throw new Error(`the fueler's Ethereum address ${this.self} has code (${code.slice(0, 10)}…, an EIP-7702 delegation?): refusing`);
      await this.nextNonce(ETHEREUM);
    }
    if (hot) {
      const a = await io.solAccount(hot);
      if (!a) notes.push(`the hot key ${hot} holds nothing yet: fund it with about 0.05 SOL for its fees`);
      else if (a.owner !== SYSTEM_PROGRAM || a.data.length) throw new Error(`the hot key ${hot} is not a plain wallet (owner ${a.owner}, ${a.data.length} bytes of data): refusing`);
    }
    const t = await this.d.io.booksTreasury(this.d.cfg.buyer).catch((err: Error) => { notes.push(`the canister's treasury_status could not be read (${String(err.message).slice(0, 120)}): the ops key is unchecked`); return null; });
    if (t && t.ops !== hot) notes.push(`the canister's treasury.ops is ${t.ops ?? "unset"}, not this fueler's hot key ${hot ?? "(none)"}: no fee income reaches it until the operator runs treasury_set_ops(<hot key>) (a change waits a day)`);
    this.save();
    return notes;
  }

  /** §3.3, the check after a top-up: the canister's balance rose by what the CMC minted, within the burn since. */
  private async verifyTopUps() {
    const keep: State["verify"] = [];
    for (const v of this.s.verify) {
      if (this.now() - v.at < 600) { keep.push(v); continue; }
      const bal = await this.d.io.cycles(v.canister).catch(() => null);
      if (bal === null) { keep.push(v); continue; }
      if (bal + 2n * T < BigInt(v.before) + BigInt(v.minted))
        this.alert("cycles-autofund-failed", `after a top-up of ${(Number(v.minted) / 1e12).toFixed(2)} T, ${v.canister} holds ${(Number(bal) / 1e12).toFixed(2)} T, not about ${(Number(BigInt(v.before) + BigInt(v.minted)) / 1e12).toFixed(2)} T`);
    }
    this.s.verify = keep;
  }

  /**
   * The operator's two exits, read at start (FUELER_UNHALT, FUELER_ABORT_RUN), each logged and written to the journal's history:
   *  - unhalt: clear a halt once its cause is fixed (a pinned address re-reviewed, a wrong canister removed from the config);
   *  - abort a run by its id, once the operator has checked where its money is (a Relay refund on 4663, ETH left on Ethereum, ckETH or
   *    ICP left at the fueler's principal: the next run picks up whatever ckETH or ICP is there). An open transaction is never dropped.
   */
  operatorExits(o: { unhalt: boolean; abortRun: string | null }): string[] {
    const done: string[] = [];
    if (o.unhalt && this.s.halted) { done.push(`halt cleared by the operator (it was: ${this.s.halted})`); this.s.halted = null; this.clear("fueler-halt"); }
    if (o.abortRun && this.s.run) {
      if (this.s.run.id !== o.abortRun) done.push(`FUELER_ABORT_RUN=${o.abortRun} names no open run (open: ${this.s.run.id}); nothing aborted`);
      else if (Object.keys(this.s.pending).length || this.s.sol.pending) done.push(`run ${o.abortRun} has a transaction out; it is not aborted until that resolves`);
      else { const was = this.s.run.phase; this.s.run.phase = "failed"; this.s.run.note = `aborted by the operator in phase ${was}`; done.push(`run ${o.abortRun} aborted by the operator (it was ${was})`); this.finish(); }
    }
    for (const d of done) this.d.log("warn", "operator-exit", { note: d });
    if (done.length) this.save();
    return done;
  }

  /** A start-up check (§4.2 item 6): the key's address is code-less on both chains (an EIP-7702 delegation is refused). */
  async preflight(): Promise<string[]> {
    if (this.s.home && this.s.home !== this.home)
      throw new Error(`the journal on /data belongs to the ${this.s.home} home, the configuration says ${this.home}: refusing (its conversions are in another unit; move the journal aside after its runs are closed)`);
    this.s.home = this.home;
    if (this.onSol) return this.preflightSol();
    const notes: string[] = [];
    if (!this.d.signer) return ["dry run: no FUELER_EVM_KEY, nothing will be signed"];
    for (const chain of [ROBINHOOD, ETHEREUM] as Chain[]) {
      const code = await this.d.io.evmCode(chain, this.self);
      if (code !== "0x") throw new Error(`the fueler's address ${this.self} has code on chain ${chain} (${code.slice(0, 10)}…, an EIP-7702 delegation?): refusing`);
    }
    const ops = lc(await this.d.io.evmCall(ROBINHOOD, this.d.cfg.treasury, encodeFunctionData({ abi: ABI, functionName: "ops" })));
    if (`0x${ops.slice(-40)}` !== this.self) notes.push(`the treasury's ops is 0x${ops.slice(-40)}, not this fueler's ${this.self}: no fee income reaches it until the Safe calls setOps`);
    for (const chain of [ROBINHOOD, ETHEREUM] as Chain[]) await this.nextNonce(chain);
    this.save();
    return notes;
  }
}

/** For the tests and the guard: the selector of a decoded call, so a refused transaction can be named. */
export const selectorOf = (data: Hex) => { try { return decodeFunctionData({ abi: ABI, data }).functionName; } catch { return data.slice(0, 10); } };
