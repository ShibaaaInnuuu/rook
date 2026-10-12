import { decodeErrorResult, type Hex } from "viem";
import { ERRORS } from "./abis.js";
import type { Caps, Mode } from "./config.js";
import { Refusal, RpcAnswer, type ChainRpc, type Log } from "./rpc.js";
import type { Signer } from "./signer.js";
import { utcDay, type Pending, type StateStore } from "./state.js";

/**
 * docs/v6/p1-services.md §1.5. Every call the keeper makes goes through `submit`, in this order: simulate (eth_call from the
 * keeper) → in a dry run, stop there → one transaction in flight → gas estimate within the per-transaction cap → fee within the
 * cap (refused, never quietly lowered) → worst case within the per-transaction and per-day money caps → nonce from the chain →
 * sign → WRITE the intent, nonce, hash and bytes (fsync) → broadcast (chain id asked first) → wait for the receipt, status checked.
 */
/** `maxGas`: this call's own gas cap when it is above the keeper's (the resolver's drand proofs, ~180k gas for the pairing alone). */
export type Call = { key: string; what: string; to: Hex; data: Hex; maxGas?: bigint };
export type Outcome = "simulated" | "simulation-reverted" | "refused" | "mined" | "reverted" | "pending" | "rejected";
/** `alert`: something an operator must see (a transaction stuck past the fee cap, a replacement signed); the pass report carries it. */
export type Result = { key: string; what: string; outcome: Outcome; detail: string; hash?: string; gasWei?: string; alert?: string };

type Receipt = { status: string; gasUsed: string; effectiveGasPrice?: string; blockNumber: string; transactionHash: string };
type Block = { baseFeePerGas?: string; number: string; timestamp: string };

export const GAS_MARGIN_PCT = 125n;

/** The revert's reason, by the contracts' error names when the data decodes; the node's message otherwise. */
export function revertReason(err: unknown): string {
  if (err instanceof RpcAnswer) {
    const data = typeof err.data === "string" ? err.data : (err.data as { data?: unknown } | null)?.data;
    if (typeof data === "string" && /^0x[0-9a-fA-F]{8}/.test(data)) {
      try {
        const d = decodeErrorResult({ abi: ERRORS, data: data as Hex });
        return `${d.errorName}(${(d.args ?? []).map(String).join(", ")})`;
      } catch { return `${err.message} [${data.slice(0, 10)}]`; }
    }
    return err.message;
  }
  return String((err as Error)?.message ?? err).slice(0, 300);
}

/**
 * `guard`: asked immediately before every signature; a non-null answer refuses the send (the keeper passes "the registry has paused
 * the keeper", read again at that moment: fixes-B-C-keeper-draws.md §5 item 5). `stuckAfterS`: how long the newest attempt for the
 * nonce in flight may wait for a receipt before it is re-priced (audit E-10).
 */
export type SenderOpts = {
  rpc: ChainRpc; chainId: number; from: Hex; signer: Signer | null; mode: Mode; caps: Caps; store: StateStore; log: Log;
  receiptWaitS: number; nowMs?: () => number; sleep?: (ms: number) => Promise<void>; guard?: () => Promise<string | null>; stuckAfterS?: number;
};

/** A node's refusal of bytes whose fee is now too low: re-price at once rather than wait. */
export const UNDERPRICED = /underpriced|fee too low|less than block base fee|max fee per gas less than|feecap too low|transaction fee too low/i;
/** The 10% replacement rule of geth-style pools, met on both fee fields with room to spare: +12.5% and one wei. */
export const bump = (x: bigint): bigint => x + x / 8n + 1n;

export class Sender {
  private readonly now: () => number;
  private readonly sleep: (ms: number) => Promise<void>;
  constructor(private readonly o: SenderOpts) {
    this.now = o.nowMs ?? Date.now;
    this.sleep = o.sleep ?? ((ms) => new Promise((ok) => setTimeout(ok, ms)));
    if (o.mode === "send") {
      if (!o.signer) throw new Error("refused: send mode without a signer");
      if (o.signer.address.toLowerCase() !== o.from.toLowerCase()) throw new Error("refused: the signer is not the keeper this sender sends as");
    }
  }

  /** Whether a transaction of ours is still waiting for its receipt. */
  inFlight(): boolean { return this.o.store.get().pending !== null; }

  /**
   * The transaction in flight, if any, resolved: a receipt for ANY attempt at its nonce recorded; or, if the chain's nonce has moved
   * past it with no receipt, recorded as dropped (its reservation counted as spent: we cannot know it was not); or, still pending,
   * the newest bytes broadcast again — and, once they have waited `stuckAfterS` or a node refuses them as underpriced, a REPLACEMENT
   * for the same nonce at a bumped fee (the same call if it still simulates, else a zero-value self-transfer that clears the nonce).
   * Never a second nonce while one of ours is open.
   */
  async resolvePending(): Promise<Result | null> {
    const p = this.o.store.get().pending;
    if (!p) return null;
    for (const a of [...p.attempts].reverse()) {
      const receipt = await this.o.rpc.call<Receipt | null>("eth_getTransactionReceipt", [a.hash]);
      if (receipt) return this.record(p.key, a.cancel ? `${p.what} (cancelled: the nonce was cleared by a self-transfer)` : p.what, a.hash, p.nonce, receipt, p.day);
    }
    const latest = Number(BigInt(await this.o.rpc.call<string>("eth_getTransactionCount", [this.o.from, "latest"])));
    if (latest > p.nonce) {
      this.o.store.finish({ key: p.key, what: p.what, hash: p.hash, nonce: p.nonce, outcome: "dropped", gasWei: p.reservedWei, at: this.now(), block: null }, p.day);
      this.o.log("warn", "tx-dropped", { key: p.key, hash: p.hash, nonce: p.nonce, note: "the nonce was used without this transaction's receipt" });
      return { key: p.key, what: p.what, outcome: "rejected", detail: `nonce ${p.nonce} was used by another transaction`, hash: p.hash };
    }
    const waitedS = Math.floor((this.now() - p.sentAt) / 1000);
    let underpriced = false;
    if (this.o.mode === "send") {
      try { await this.o.rpc.sendRaw(p.raw); this.o.log("info", "tx-rebroadcast", { key: p.key, hash: p.hash, nonce: p.nonce }); }
      catch (err) {
        underpriced = err instanceof RpcAnswer && UNDERPRICED.test(err.message);
        this.o.log("warn", "tx-rebroadcast-failed", { key: p.key, hash: p.hash, error: revertReason(err), underpriced });
      }
      if (underpriced || waitedS >= (this.o.stuckAfterS ?? 180)) {
        const r = await this.replace(p, underpriced ? "a node refused the bytes as underpriced" : `no receipt after ${waitedS} s`);
        if (r.mined) return { ...r.mined, alert: r.alert };
        return { key: p.key, what: p.what, outcome: "pending", detail: r.detail, hash: this.o.store.get().pending?.hash ?? p.hash, alert: r.alert };
      }
    }
    return { key: p.key, what: p.what, outcome: "pending", detail: `still waiting for its receipt (${waitedS} s)`, hash: p.hash };
  }

  /** E-10: sign and send a replacement for the pending nonce, within every cap; an alert when it cannot be re-priced. */
  private async replace(p: Pending, why: string): Promise<{ detail: string; alert: string; mined?: Result }> {
    const last = p.attempts[p.attempts.length - 1];
    const block = await this.o.rpc.call<Block>("eth_getBlockByNumber", ["latest", false]);
    const baseFee = BigInt(block.baseFeePerGas ?? "0x0");
    let tip = 0n;
    try { tip = BigInt(await this.o.rpc.call<string>("eth_maxPriorityFeePerGas", [])); } catch { tip = 0n; }
    let maxFee = bump(BigInt(last.maxFeePerGas));
    if (2n * baseFee + tip > maxFee) maxFee = 2n * baseFee + tip;
    let newTip = bump(BigInt(last.tip));
    if (tip > newTip) newTip = tip;
    if (newTip > maxFee) newTip = maxFee;
    const stuck = (detail: string) => {
      const alert = `tx-stuck: nonce ${p.nonce} (${p.what}) has waited ${Math.floor((this.now() - p.firstSentAt) / 1000)} s; ${detail}`;
      this.o.log("error", "tx-stuck", { key: p.key, nonce: p.nonce, hash: p.hash, why, detail });
      return { detail: `stuck: ${detail}`, alert };
    };
    if (maxFee > this.o.caps.maxFeePerGasWei) return stuck(`a replacement needs ${maxFee} wei/gas, over the fee cap of ${this.o.caps.maxFeePerGasWei}: re-broadcast only`);
    // nothing is signed while the guard refuses (the keeper paused), not even a replacement
    const guard = this.o.guard ? await this.o.guard() : null;
    if (guard) return stuck(`no replacement is signed: ${guard}`);
    // the same call while it still simulates; otherwise clear the nonce with a zero-value self-transfer
    // audit R2-S3: the original call keeps the gas of its own last attempt, never a self-transfer's 21,000
    const lastCall = [...p.attempts].reverse().find((a) => !a.cancel);
    let to = p.to as Hex, data = p.data as Hex, gas = BigInt(lastCall?.gas || "0"), cancel = false;
    try {
      await this.o.rpc.call<string>("eth_call", [{ from: this.o.from, to, data }, "latest"]);
      if (gas === 0n) gas = (BigInt(await this.o.rpc.call<string>("eth_estimateGas", [{ from: this.o.from, to, data }])) * GAS_MARGIN_PCT) / 100n;
    } catch { to = this.o.from; data = "0x"; gas = 21_000n; cancel = true; }
    const worst = gas * maxFee, day = utcDay(this.now());
    if (worst > this.o.caps.maxWeiPerTx) return stuck(`a replacement's worst case ${worst} wei is over the per-transaction cap`);
    const others = this.o.store.committed(day) - (p.day === day ? BigInt(p.reservedWei) : 0n);
    if (others + worst > this.o.caps.maxWeiPerDay) return stuck(`a replacement's worst case ${worst} wei would pass the day's cap`);
    const { raw, hash } = await this.o.signer!.sign({ type: "eip1559", chainId: this.o.chainId, nonce: p.nonce, to, data, value: 0n, gas,
      maxFeePerGas: maxFee, maxPriorityFeePerGas: newTip }, this.o.chainId);
    const attempt = { hash, raw, maxFeePerGas: String(maxFee), tip: String(newTip), gas: String(gas), sentAt: this.now(), to, data, cancel };
    const note = `${cancel ? "cancelled with a self-transfer" : "re-priced"} at ${maxFee} wei/gas (was ${last.maxFeePerGas}) because ${why}`;
    // written down before it leaves (a crash mid-send still knows its hash); audit R2-S3: an attempt stays recorded only when it may
    // be in a pool: sent, refused as underpriced (the next bump builds on it), or lost ambiguously. One the node refused for any
    // other reason (intrinsic gas, a bad nonce) is taken back out, so it is never the base of the next re-price
    this.o.store.setPending({ ...p, hash, raw, sentAt: attempt.sentAt, reservedWei: String(worst), day, attempts: [...p.attempts, attempt] });
    try { await this.o.rpc.sendRaw(raw); } catch (err) {
      if (err instanceof Refusal) { this.o.store.setPending(p); return stuck(`the replacement was refused before it left: ${err.message}`); }
      if (err instanceof RpcAnswer && !UNDERPRICED.test(err.message)) { this.o.store.setPending(p); return stuck(`the node refused the replacement (${err.message.slice(0, 160)}): not recorded`); }
      this.o.log("warn", "tx-replacement-send-failed", { key: p.key, hash, error: revertReason(err) });
    }
    this.o.log("warn", "tx-replaced", { key: p.key, nonce: p.nonce, old: last.hash, hash, maxFeePerGas: String(maxFee), cancel, why });
    const receipt = await this.o.rpc.call<Receipt | null>("eth_getTransactionReceipt", [hash]);
    if (receipt) return { detail: note, alert: `tx-replaced: nonce ${p.nonce} ${note}`, mined: this.record(p.key, cancel ? `${p.what} (cancelled)` : p.what, hash, p.nonce, receipt, day) };
    return { detail: note, alert: `tx-replaced: nonce ${p.nonce} ${note}` };
  }

  private record(key: string, what: string, hash: string, nonce: number, r: Receipt, day: string): Result {
    const gasWei = (BigInt(r.gasUsed) * BigInt(r.effectiveGasPrice ?? "0x0")).toString();
    const ok = r.status === "0x1";
    this.o.store.finish({ key, what, hash, nonce, outcome: ok ? "mined" : "reverted", gasWei, at: this.now(), block: Number(BigInt(r.blockNumber)) }, day);
    this.o.log(ok ? "info" : "error", ok ? "tx-mined" : "tx-reverted", { key, what, hash, nonce, block: Number(BigInt(r.blockNumber)), gasWei });
    return { key, what, outcome: ok ? "mined" : "reverted", detail: ok ? `mined in block ${Number(BigInt(r.blockNumber))}` : "the transaction reverted on chain", hash, gasWei };
  }

  async submit(c: Call): Promise<Result> {
    const base = { key: c.key, what: c.what };
    // 1. simulate, always
    try { await this.o.rpc.call<string>("eth_call", [{ from: this.o.from, to: c.to, data: c.data }, "latest"]); } catch (err) {
      const why = revertReason(err);
      this.o.log("warn", "simulation-reverted", { key: c.key, what: c.what, reason: why });
      return { ...base, outcome: "simulation-reverted", detail: why };
    }
    if (this.o.mode === "dry-run") {
      this.o.log("info", "simulated", { key: c.key, what: c.what, to: c.to, data: c.data, note: "dry run: nothing is signed or sent" });
      return { ...base, outcome: "simulated", detail: "the call succeeds in simulation; dry run: nothing signed or sent" };
    }
    // 2. one transaction in flight
    if (this.inFlight()) {
      const r = await this.resolvePending();
      if (r && r.outcome === "pending") return { ...base, outcome: "refused", detail: `a transaction is in flight (${r.hash}); one at a time` };
    }
    const refuse = (detail: string): Result => { this.o.log("error", "tx-refused", { key: c.key, what: c.what, reason: detail }); return { ...base, outcome: "refused", detail }; };
    // 3. gas
    let estimate: bigint;
    try { estimate = BigInt(await this.o.rpc.call<string>("eth_estimateGas", [{ from: this.o.from, to: c.to, data: c.data }])); } catch (err) {
      return { ...base, outcome: "simulation-reverted", detail: `gas estimate failed: ${revertReason(err)}` };
    }
    const gas = (estimate * GAS_MARGIN_PCT) / 100n;
    const gasCap = c.maxGas !== undefined && c.maxGas > this.o.caps.maxGasPerTx ? c.maxGas : this.o.caps.maxGasPerTx;
    if (gas > gasCap) return refuse(`gas ${gas} (estimate ${estimate} + 25%) is over the per-transaction cap of ${gasCap}`);
    // 4. fees
    const block = await this.o.rpc.call<Block>("eth_getBlockByNumber", ["latest", false]);
    const baseFee = BigInt(block.baseFeePerGas ?? "0x0");
    let tip = 0n;
    try { tip = BigInt(await this.o.rpc.call<string>("eth_maxPriorityFeePerGas", [])); } catch { tip = 0n; }
    const maxFee = 2n * baseFee + tip;
    if (maxFee > this.o.caps.maxFeePerGasWei) return refuse(`a max fee of ${maxFee} wei/gas (2 × base ${baseFee} + tip ${tip}) is over the cap of ${this.o.caps.maxFeePerGasWei}`);
    if (tip > maxFee) tip = maxFee;
    // 5. money
    const worst = gas * maxFee, day = utcDay(this.now());
    if (worst > this.o.caps.maxWeiPerTx) return refuse(`the worst case ${worst} wei is over the per-transaction cap of ${this.o.caps.maxWeiPerTx}`);
    const committed = this.o.store.committed(day);
    if (committed + worst > this.o.caps.maxWeiPerDay) return refuse(`${committed} wei spent or reserved today + ${worst} is over the day's cap of ${this.o.caps.maxWeiPerDay}`);
    // 6. the guard (the keeper not paused, read now), nonce, sign, write down, send
    const guard = this.o.guard ? await this.o.guard() : null;
    if (guard) return refuse(guard);
    const nonce = Number(BigInt(await this.o.rpc.call<string>("eth_getTransactionCount", [this.o.from, "pending"])));
    const { raw, hash } = await this.o.signer!.sign({ type: "eip1559", chainId: this.o.chainId, nonce, to: c.to, data: c.data, value: 0n, gas,
      maxFeePerGas: maxFee, maxPriorityFeePerGas: tip }, this.o.chainId);
    const sentAt = this.now();
    this.o.store.setPending({ key: c.key, what: c.what, to: c.to, data: c.data, nonce, hash, raw, sentAt, reservedWei: String(worst), day, firstSentAt: sentAt,
      attempts: [{ hash, raw, maxFeePerGas: String(maxFee), tip: String(tip), gas: String(gas), sentAt, to: c.to, data: c.data, cancel: false }] });
    try {
      const sent = await this.o.rpc.sendRaw(raw);
      this.o.log("info", "tx-sent", { key: c.key, what: c.what, hash, nonce, gas: String(gas), maxFeePerGas: String(maxFee), host: sent.host });
    } catch (err) {
      if (err instanceof Refusal) {
        // refused before the bytes left (an endpoint that now reports another chain id): nothing is in flight
        this.o.store.abandon();
        this.o.log("error", "tx-refused", { key: c.key, what: c.what, reason: err.message });
        return { ...base, outcome: "refused", detail: err.message };
      }
      if (err instanceof RpcAnswer) {
        // the node refused the bytes outright: nothing is in flight (a nonce too low, insufficient funds, a fee too low)
        this.o.store.abandon();
        this.o.log("error", "tx-rejected", { key: c.key, what: c.what, hash, reason: err.message });
        return { ...base, outcome: "rejected", detail: `the node refused the transaction: ${err.message}`, hash };
      }
      // anything else is ambiguous (it may have gone out): it stays pending, and the next pass resolves it by hash and nonce
      this.o.log("warn", "tx-send-unclear", { key: c.key, hash, error: revertReason(err) });
      return { ...base, outcome: "pending", detail: `the broadcast failed ambiguously; resolved by its hash next pass`, hash };
    }
    // 7. the receipt
    for (let i = 0; i < this.o.receiptWaitS; i++) {         // one look a second, counted (not timed): a stalled clock cannot loop it
      const r = await this.o.rpc.call<Receipt | null>("eth_getTransactionReceipt", [hash]);
      if (r) return this.record(c.key, c.what, hash, nonce, r, day);
      await this.sleep(1_000);
    }
    return { ...base, outcome: "pending", detail: `no receipt within ${this.o.receiptWaitS} s; resolved next pass`, hash };
  }
}
