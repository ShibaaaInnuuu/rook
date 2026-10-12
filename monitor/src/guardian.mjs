// The optional guardian (docs/audit/2026-09-26/fixes-W2-monitor.md §1.4). ListingRegistry.guardian() can do two things, both
// of which only STOP something: `pauseKeeper()` (every keeper power everywhere, at once) and `SweepVault.cancelDraw(id)` for a draw
// not yet pinned. The Safe unpauses and can replace the guardian at once. OFF unless configured: the key is the service variable
// MONITOR_GUARDIAN_KEY, held only by the monitor's own Railway project (never the keeper's, never the runner's), read here and
// nowhere else, never logged, printed, returned by /status or written to the state file, and scrubbed from every error.
import { encodeFunctionData, parseAbi } from "viem";
import { privateKeyToAccount } from "viem/accounts";
import { DRAW_STAGE, REGISTRY, VAULT, host, lc, read, rpc } from "./abi.mjs";

/** An Ethereum NFT collection's PosterSource: its own floor switch (audit 2026-09-27-nft D-M-1). */
export const POSTER = parseAbi(["function paused() view returns (bool)", "function pause()", "function unpause()",
  "function resumedAt() view returns (uint64)"]);
import { stackOf } from "./stack.mjs";

const KEY = /^(0x)?[0-9a-fA-F]{64}$/;
export const GAS_CAP = 300_000n;
export const DEFAULT_MAX_FEE_WEI = 2_000_000_000n;      // 2 gwei: 4663's base fee is about 0.01 gwei
export const RECEIPT_WAIT_MS = 45_000;                  // R2-L3: an action is done when its receipt says so, not when it is sent
export const RECEIPT_POLL_MS = 1_500;

/**
 * Build the guardian from the config and the environment, or refuse (throws with the reason, never the key). Returns null when
 * neither the config nor the key names one: the guardian is off.
 */
export function guardianFrom(cfg, env = process.env, http = fetch) {
  const g = cfg.evm?.guardian, raw = env.MONITOR_GUARDIAN_KEY;
  if (!g && !raw) return null;
  if (!g) throw new Error("MONITOR_GUARDIAN_KEY is set but the config names no evm.guardian (address, and what it may do): refusing to hold a key nobody configured");
  if (!raw) throw new Error("evm.guardian is configured but MONITOR_GUARDIAN_KEY is not set: set it, or remove evm.guardian to run without a guardian");
  if (!KEY.test(raw.trim())) throw new Error("MONITOR_GUARDIAN_KEY is not a 32-byte hex key (the value is not repeated)");
  const account = privateKeyToAccount(`0x${raw.trim().replace(/^0x/, "")}`);
  if (lc(account.address) !== lc(g.address)) throw new Error(`MONITOR_GUARDIAN_KEY is the key of ${account.address}, not of evm.guardian.address ${g.address}`);
  const registry = stackOf(cfg.evm).registry;
  if (!registry) throw new Error("evm.guardian needs the registry (evm.stack or evm.registry): it is what the guardian pauses");
  return new Guardian({ account, cfg, registry, http });
}

export class Guardian {
  #account;
  constructor({ account, cfg, registry, http = fetch }) {
    this.#account = account;
    this.address = lc(account.address);
    this.rpcs = cfg.evm.rpcs;
    this.chainId = BigInt(cfg.evm.chain_id ?? 4663);
    this.registry = registry;
    this.autoPause = cfg.evm.guardian.auto_pause !== false;
    this.autoCancel = cfg.evm.guardian.auto_cancel !== false;
    this.maxFee = BigInt(cfg.evm.guardian.max_fee_wei ?? DEFAULT_MAX_FEE_WEI);
    this.http = http;
    this.receiptWaitMs = cfg.evm.guardian.receipt_wait_ms ?? RECEIPT_WAIT_MS;
    this.receiptPollMs = cfg.evm.guardian.receipt_poll_ms ?? RECEIPT_POLL_MS;
  }
  toJSON() { return { address: this.address, autoPause: this.autoPause, autoCancel: this.autoCancel }; }

  /** Checks made before every action, on BOTH RPCs: the chain, the registry still naming this guardian, no code at its address. */
  async preflight() {
    const why = [];
    for (const u of this.rpcs) {
      const id = BigInt(await rpc(u, "eth_chainId", [], this.http));
      if (id !== this.chainId) why.push(`${host(u)} reports chain ${id}, not ${this.chainId}`);
      const code = await rpc(u, "eth_getCode", [this.address, "latest"], this.http);
      if (code && code !== "0x") why.push(`${host(u)}: the guardian ${this.address} has code (an EIP-7702 delegation?)`);
      const named = lc(await read(u, REGISTRY, this.registry, "guardian", [], { http: this.http }));
      if (named !== this.address) why.push(`${host(u)}: the registry's guardian is ${named}, not this key's ${this.address}`);
    }
    if (why.length) throw new Error(`the guardian will not act: ${why.join("; ")}`);
  }

  /** Sign and send one call to both RPCs. Returns the transaction hash. Simulated first; gas and fee capped. */
  async #send(to, data) {
    const [a] = this.rpcs;
    await rpc(a, "eth_call", [{ from: this.address, to, data }, "latest"], this.http);          // a revert throws here, nothing is signed
    const nonces = await Promise.all(this.rpcs.map((u) => rpc(u, "eth_getTransactionCount", [this.address, "pending"], this.http)));
    const nonce = nonces.map((n) => Number(BigInt(n))).reduce((x, y) => Math.max(x, y));
    const est = BigInt(await rpc(a, "eth_estimateGas", [{ from: this.address, to, data }], this.http));
    const gas = (est * 13n) / 10n;
    if (gas > GAS_CAP) throw new Error(`the call needs ${gas} gas, above the guardian's cap ${GAS_CAP}`);
    const base = BigInt((await rpc(a, "eth_getBlockByNumber", ["latest", false], this.http)).baseFeePerGas ?? "0x0");
    const tip = 1_000_000n, maxFee = base * 3n + tip;
    if (maxFee > this.maxFee) throw new Error(`the fee would be ${maxFee} wei per gas, above the guardian's cap ${this.maxFee}`);
    const raw = await this.#account.signTransaction({ type: "eip1559", chainId: Number(this.chainId), nonce, to, data, gas, maxFeePerGas: maxFee, maxPriorityFeePerGas: tip, value: 0n });
    let hash = null, errors = [];
    for (const u of this.rpcs) {
      try { hash = (await rpc(u, "eth_sendRawTransaction", [raw], this.http)) ?? hash; }
      catch (err) { errors.push(String(err.message)); }
    }
    if (!hash) throw new Error(`no RPC accepted the transaction: ${errors.join("; ")}`);
    return this.#receipt(hash);
  }

  /** R2-L3: wait for the receipt on either RPC. Success: { tx }; reverted: throws; not mined in time: { tx, pending: true }. */
  async #receipt(hash) {
    const until = Date.now() + this.receiptWaitMs;
    for (;;) {
      for (const u of this.rpcs) {
        const r = await rpc(u, "eth_getTransactionReceipt", [hash], this.http).catch(() => null);
        if (r?.status === "0x1") return { tx: hash };
        if (r?.status === "0x0") throw new Error(`the transaction ${hash} was mined and REVERTED (block ${Number(BigInt(r.blockNumber ?? "0x0"))})`);
      }
      if (Date.now() >= until) return { tx: hash, pending: true };
      await new Promise((ok) => setTimeout(ok, this.receiptPollMs));
    }
  }

  /** `pauseKeeper()`, unless the keeper is already paused. { done, tx?, skipped?, pending? } (pending: sent, not mined yet). */
  async pauseKeeper() {
    await this.preflight();
    const paused = await Promise.all(this.rpcs.map((u) => read(u, REGISTRY, this.registry, "keeperPaused", [], { http: this.http })));
    if (paused.every(Boolean)) return { done: false, skipped: "the keeper is already paused" };
    const r = await this.#send(this.registry, encodeFunctionData({ abi: REGISTRY, functionName: "pauseKeeper" }));
    return r.pending ? { done: false, pending: true, tx: r.tx } : { done: true, tx: r.tx };
  }

  /**
   * `pause()` on one Ethereum NFT collection's PosterSource (audit 2026-09-27-nft D-M-1): that collection's floor stops, and no other
   * collection's. Only the Safe unpauses it. Skipped when both RPCs already read it paused.
   */
  async pausePoster(poster) {
    await this.preflight();
    const paused = await Promise.all(this.rpcs.map((u) => read(u, POSTER, poster, "paused", [], { http: this.http })));
    if (paused.every(Boolean)) return { done: false, skipped: `the poster ${poster} is already paused` };
    const r = await this.#send(poster, encodeFunctionData({ abi: POSTER, functionName: "pause" }));
    return r.pending ? { done: false, pending: true, tx: r.tx } : { done: true, tx: r.tx };
  }

  /** `cancelDraw(id)` on `vault`, only while the draw is Open or Announced on both RPCs. */
  async cancelDraw(vault, id) {
    await this.preflight();
    const stages = await Promise.all(this.rpcs.map(async (u) => DRAW_STAGE[Number((await read(u, VAULT, vault, "draw", [BigInt(id)], { http: this.http })).stage)]));
    if (!stages.every((s) => s === "Open" || s === "Announced")) return { done: false, skipped: `the draw is ${stages.join("/")} (only an Open or Announced draw can be cancelled)` };
    const r = await this.#send(vault, encodeFunctionData({ abi: VAULT, functionName: "cancelDraw", args: [BigInt(id)] }));
    return r.pending ? { done: false, pending: true, tx: r.tx } : { done: true, tx: r.tx };
  }
}

/** What the Safe types by hand when the guardian is off or failed. */
export const byHand = {
  pause: (registry) => `the Safe (or the guardian): cast send ${registry} "pauseKeeper()"`,
  pausePoster: (poster) => `the Safe (or the guardian): cast send ${poster} "pause()" (this collection's floor only)`,
  cancel: (vault, id) => `the Safe (or the guardian): cast send ${vault} "cancelDraw(uint256)" ${id}`,
};
