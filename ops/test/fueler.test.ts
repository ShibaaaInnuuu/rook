// docs/v7/cycles-from-fees.md §4.2 (v1, the fueler): the review scope's eight items, each held here against a fake world that runs
// both chains, Relay, the ckETH minter, the ICPSwap pool, the ICP ledger and the CMC. The encodings are held to vectors dfx printed.
import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { test } from "node:test";
import { encodeFunctionData, parseTransaction, type Hex } from "viem";
import { Engine, MemoryJournal, allowed, emptyState, type EngineCfg, type Journal, type State } from "../src/fuel/engine.js";
import type { FuelIo, Notify } from "../src/fuel/io.js";
import { fuelEvmSigner } from "../src/fuel/keys.js";
import { accountId, CKETH_FEE_WEI, CKETH_HELPER, depositEthData, IC, ICP_FEE_E8S, principal32, principal32Hex, SEL, topUpAccount } from "../src/fuel/pinned.js";
import { agree, cyclesForWei, decide, DEFAULT_CAPS, judgeEndToEnd, judgeSwap, split, T, WEI, type Prices } from "../src/fuel/plan.js";
import { parseFuelerConfig } from "../src/fueler.js";
import { logger } from "../src/log.js";
import { orderIdOf, RELAY_DEPOSITORY_4663 } from "../src/relay.js";

const hex = (u: Uint8Array) => Buffer.from(u).toString("hex");
const silent = logger(() => {});
const KEY = "0x8f2a55949038a9610f50fb23b5883af3b4ecb3c3bb792cbcefbd1542c692be63";
const signer = fuelEvmSigner({ FUELER_EVM_KEY: KEY })!;
const ME = signer.address;
const PRINCIPAL = "ih4wp-rvfmw-iimsu-qnmdv-63mci-c6s5k-6dfao-fmrkp-3nsmu-mf5ul-hqe";
const BUYER = "ryjl3-tyaaa-aaaaa-aaaba-cai", TIMELOCK = "rrkah-fqaaa-aaaaa-aaaaq-cai";
const TREASURY = ("0x" + "7e".repeat(20)) as Hex, PAYEE = ("0x" + "c0".repeat(20)) as Hex, CURVE = ("0x" + "cc".repeat(20)) as Hex;
const T0 = 1_791_600_000;
const XDR = 22_322n;                                    // the CMC's rate read 2026-10-09: 2.2322 XDR an ICP
const PRICES = { eth: 2650, icp: 3.2 };

// ---- 1. the encodings, against dfx ---------------------------------------------------------------------------------------------

test("encodings: the ICP account id and the CMC's top-up account match what dfx prints; a principal's 32 bytes are its length then its bytes", () => {
  // `dfx ledger account-id` (identity ih4wp-…), and `dfx ledger account-id --of-principal rkp4c-… --subaccount-from-principal <x>`
  assert.equal(hex(accountId(PRINCIPAL)), "7a1b0d128f650eb3127e190ac676fe78f96716dae2dff22150d77bc0db98e80d");
  assert.equal(hex(topUpAccount("ryjl3-tyaaa-aaaaa-aaaba-cai")), "5d6eb5bf9490f9e5c02ae07f9ac5cb7370368e8363302bb5eb8520c2b9f794a0");
  assert.equal(hex(topUpAccount(PRINCIPAL)), "dae311537dc0c75fdab7b5d63faf298258b8ffd13acdb64bcc104835247940bb");
  const p = principal32(PRINCIPAL);
  assert.equal(p[0], 29, "the length byte");
  assert.equal(hex(p.slice(30)), "0000", "zero-padded");
  assert.equal(principal32Hex(PRINCIPAL), "0x1da56590864a906b075f6d8240bd2eabc3281c56454fdb64ca30bda2cf020000");
  // depositEth(bytes32 principal, bytes32 subaccount): the helper's ABI from Sourcify (0x1890…0E68)
  assert.equal(depositEthData(PRINCIPAL).slice(0, 10), "0x17c819c4");
  assert.ok(depositEthData(PRINCIPAL).endsWith("00".repeat(32)), "the default subaccount");
});

// ---- 2. the plan: prices, caps, the swap's and the run's checks ----------------------------------------------------------------

const P: Prices = { ethUsd: 2650, icpUsd: 3.2, at: T0, sources: ["coingecko", "coinbase"] };

test("prices: two sources must agree within 2 %; a missing one stops the run", () => {
  assert.ok(agree({ eth: 2650, icp: 3.2 }, { eth: 2660, icp: 3.21 }, T0).prices);
  assert.match(agree({ eth: 2650, icp: 3.2 }, { eth: 2750, icp: 3.2 }, T0).why!, /ETH\/USD disagrees/);
  assert.match(agree({ eth: 2650, icp: 3.2 }, { eth: 2650, icp: 3.4 }, T0).why!, /ICP\/USD disagrees/);
  assert.match(agree(null, { eth: 2650, icp: 3.2 }, T0).why!, /did not answer/);
  assert.match(agree({ eth: 2650 }, { eth: 2650, icp: 3.2 }, T0).why!, /did not answer/);
});

const base = { nowS: T0, lastRunAt: null, timelockCycles: 2n * T, opsWei: WEI / 5n, gasReserveWei: WEI / 1000n, conversions: [], prices: P,
  xdrPermyriadPerIcp: XDR, caps: DEFAULT_CAPS, buyer: BUYER, timelock: TIMELOCK };

test("decide: idle above the low mark; at most one run in 6 h; a run is capped at 20 T and tops the timelock first", () => {
  assert.equal(decide({ ...base, buyerCycles: 41n * T }).kind, "idle");
  assert.equal(decide({ ...base, buyerCycles: 34n * T, lastRunAt: T0 - 3_600 }).kind, "wait");
  const d = decide({ ...base, buyerCycles: 34n * T, timelockCycles: T });
  assert.equal(d.kind, "run");
  if (d.kind !== "run") return;
  assert.equal(d.targets[0].canister, TIMELOCK, "the timelock first");
  assert.equal(d.targets[0].cycles, T, "the timelock to 2 T");
  assert.equal(d.targets[1].cycles, 19n * T, "the buyer gets the rest of the run's 20 T");
  assert.equal(d.expectCycles, 20n * T);
  // the ETH is the oracle's price of 20 T grossed up by 5 % for the route
  const fair = (20n * T * WEI) / cyclesForWei(WEI, P, XDR);
  assert.ok(d.wei > fair && d.wei < (fair * 106n) / 100n, `${d.wei} vs ${fair}`);
});

test("decide: the 7-day T cap, the ETH caps and the wallet's floor each stop a run and say which", () => {
  const week = [{ at: T0 - 86_400, wei: WEI / 50n, cycles: 60n * T }];
  const a = decide({ ...base, buyerCycles: 30n * T, conversions: week });
  assert.equal(a.kind, "blocked"); assert.equal(a.kind === "blocked" && a.alert, "cycles-autofund-cap");
  const spentEth = [{ at: T0 - 3_600 * 7, wei: WEI / 10n, cycles: 10n * T }];
  const b = decide({ ...base, buyerCycles: 30n * T, conversions: spentEth });
  assert.equal(b.kind === "blocked" && b.alert, "cycles-autofund-cap", "the day's 0.10 ETH is spent");
  const c = decide({ ...base, buyerCycles: 30n * T, opsWei: WEI / 50n + WEI / 1000n + 1n });
  assert.equal(c.kind === "blocked" && c.alert, "cycles-autofund-short", "nothing above the 0.02 ETH floor and the gas");
  // part of the day's room left: the run shrinks to it, every target in proportion
  const d = decide({ ...base, buyerCycles: 30n * T, timelockCycles: null, conversions: [{ at: T0 - 3_600, wei: (WEI * 95n) / 1000n, cycles: 5n * T }] });
  assert.equal(d.kind, "run");
  if (d.kind === "run") { assert.equal(d.wei, WEI / 200n, "0.005 ETH left in the day"); assert.ok(d.expectCycles < 20n * T); }
});

test("the swap: refused more than 1.5 % under the oracle; the minimum is the quote less 0.5 %; end to end at least 95 %", () => {
  const ck = WEI / 100n, oracle = BigInt(Math.floor(0.01 * 2650 / 3.2 * 1e8));
  assert.equal(judgeSwap(ck, oracle, P).minOut, (oracle * 9_950n) / 10_000n);
  assert.doesNotThrow(() => judgeSwap(ck, (oracle * 9_860n) / 10_000n, P));
  assert.throws(() => judgeSwap(ck, (oracle * 9_840n) / 10_000n, P), /refused/);
  assert.doesNotThrow(() => judgeEndToEnd(ck, (oracle * 9_600n) / 10_000n, P, XDR));
  assert.throws(() => judgeEndToEnd(ck, (oracle * 9_400n) / 10_000n, P, XDR), /under 95/);
});

test("split: the ICP goes to the targets in proportion, each net of the ledger fee, and never to a third", () => {
  const s = split(1_000_000_000n, [{ canister: TIMELOCK, cycles: T }, { canister: BUYER, cycles: 19n * T }], ICP_FEE_E8S);
  assert.deepEqual(s.map((x) => x.canister), [TIMELOCK, BUYER]);
  assert.equal(s.reduce((a, x) => a + x.e8s, 0n) + 2n * ICP_FEE_E8S, 1_000_000_000n);
  assert.deepEqual(split(15_000n, [{ canister: BUYER, cycles: T }], ICP_FEE_E8S).length, 1);
  assert.deepEqual(split(9_000n, [{ canister: BUYER, cycles: T }], ICP_FEE_E8S), []);
});

// ---- 3. the allowlist: there is no generic send ----------------------------------------------------------------------------------

const CTX = { self: ME, treasury: TREASURY, payee: PAYEE, principal: PRINCIPAL };
const dep = (who: string) => `${SEL.depositNative}${who.slice(2).padStart(64, "0")}${"ab".repeat(32)}` as Hex;

test("allowlist: only the five transactions of the route can be signed", () => {
  assert.equal(allowed(4663, { to: RELAY_DEPOSITORY_4663 as Hex, data: dep(ME), value: WEI / 100n }, CTX), "relay");
  assert.equal(allowed(4663, { to: CURVE, data: SEL.claimPlatformFees, value: 0n }, CTX), "claim");
  assert.equal(allowed(4663, { to: TREASURY, data: SEL.allocate, value: 0n }, CTX), "allocate");
  assert.equal(allowed(4663, { to: PAYEE, data: "0x", value: 1n }, CTX), "forward");
  assert.equal(allowed(1, { to: CKETH_HELPER as Hex, data: depositEthData(PRINCIPAL), value: WEI / 100n }, CTX), "helper");
  const no = (chain: 1 | 4663, tx: { to: Hex; data: Hex; value: bigint }) => assert.throws(() => allowed(chain, tx, CTX), /refused/, JSON.stringify(tx, (_k, v) => (typeof v === "bigint" ? String(v) : v)));
  no(4663, { to: ("0x" + "66".repeat(20)) as Hex, data: "0x", value: 1n });                            // ETH to anyone but the payee
  no(4663, { to: RELAY_DEPOSITORY_4663 as Hex, data: dep(PAYEE), value: 1n });                        // a Relay deposit credited to another
  no(4663, { to: ("0x" + "66".repeat(20)) as Hex, data: SEL.allocate, value: 0n });                    // allocate on another contract
  no(4663, { to: CURVE, data: SEL.claimPlatformFees, value: 1n });                                     // value with a claim
  no(4663, { to: PAYEE, data: SEL.allocate, value: 1n });                                              // a call on the payee
  no(1, { to: CKETH_HELPER as Hex, data: depositEthData(BUYER), value: 1n });                          // ckETH minted to another principal
  no(1, { to: PAYEE, data: "0x", value: 1n });                                                         // anything else on Ethereum
  no(1, { to: RELAY_DEPOSITORY_4663 as Hex, data: dep(ME), value: 1n });                               // a Relay deposit from Ethereum
});

test("guard: the keys are read in src/fuel/keys.ts only; a signature is made only in Engine.sendEvm, after `allowed`; raw sends only in io.ts", () => {
  const dir = new URL("../src/", import.meta.url);
  const files = readdirSync(dir, { recursive: true }).map(String).filter((f) => f.endsWith(".ts"));
  const code = (f: string) => readFileSync(new URL(f, dir), "utf8");
  // a read is the name as its own string, or env.NAME; a message that mentions it is not
  assert.deepEqual(files.filter((f) => /["']FUELER_(EVM_KEY|IC_IDENTITY_PEM)["']|env\.FUELER_(EVM_KEY|IC_IDENTITY_PEM)|env\[\s*["']FUELER_(EVM_KEY|IC_IDENTITY_PEM)/.test(code(f))),
    ["fuel/keys.ts"]);
  assert.deepEqual(files.filter((f) => /signer!?\.sign\(/.test(code(f))), ["fuel/engine.ts"]);
  const e = code("fuel/engine.ts");
  assert.equal((e.match(/signer!?\.sign\(/g) ?? []).length, 1, "one signing site");
  const send = e.slice(e.indexOf("private async sendEvm"), e.indexOf("private async settle"));
  assert.ok(send.indexOf("allowed(") < send.indexOf("signer!.sign("), "the allowlist runs before the signature");
  assert.ok(send.indexOf("this.save()") < send.indexOf("evmBroadcast"), "journaled before broadcast");
  assert.deepEqual(files.filter((f) => /eth_sendRawTransaction/.test(code(f))), ["fuel/io.ts"]);
  assert.ok(!/console\.|log\(/.test(code("fuel/keys.ts")));
});

// ---- 4. a fake world -------------------------------------------------------------------------------------------------------------

class World implements FuelIo {
  bal: Record<number, Map<string, bigint>> = { 4663: new Map([[ME, WEI / 5n]]), 1: new Map() };
  nonce: Record<number, number> = { 4663: 0, 1: 0 };
  receipts = new Map<string, { ok: boolean; block: bigint }>();
  mined: { chain: number; to: string; value: bigint; data: string; nonce: number }[] = [];
  owed = new Map<string, bigint>([[CURVE, 0n]]);
  unallocated = 0n; totalToOps = 0n; opsAddr: string = ME;
  ck = 0n; ckAllowance = 0n; unused = { icp: 0n, ck: 0n }; icp = 0n;
  icpBlocks: { to: string; e8s: bigint; key: string }[] = []; notified = new Map<number, bigint>();
  canisters = new Map<string, bigint>([[BUYER, 34n * T], [TIMELOCK, 2n * T]]);
  helper: string = CKETH_HELPER; tokens = { token0: IC.ICP_LEDGER as string, token1: IC.CKETH_LEDGER as string };
  prices_ = { cg: { ...PRICES } as { eth?: number; icp?: number } | null, cb: { ...PRICES } as { eth?: number; icp?: number } | null };
  /** a crash point: when the named call happens, the journal freezes (the process is gone; what it saved is all that survives) */
  crashAt: string | null = null; onCrash: (() => void) | null = null;
  autoMine = true; mempool: Hex[] = []; relayPaysOut = true; calls: string[] = [];
  private crash(at: string) { if (this.crashAt === at) { this.crashAt = null; this.onCrash?.(); throw new Error(`CRASH at ${at}`); } }

  principal() { return PRINCIPAL; }
  async evmBalance(chain: number, a: Hex) { return this.bal[chain].get(a.toLowerCase()) ?? 0n; }
  async evmNonce(chain: number) { return this.nonce[chain]; }
  async evmReceipt(_c: number, h: Hex) { return this.receipts.get(h) ?? null; }
  async evmCode() { return "0x" as Hex; }
  async evmBaseFee(chain: number) { return chain === 1 ? 100_000_000n : 10_000_000n; }
  async evmEstimate() { return 50_000n; }
  async evmCall(_c: number, to: Hex, data: Hex): Promise<Hex> {
    const w = (x: bigint) => `0x${x.toString(16).padStart(64, "0")}` as Hex;
    if (data === encodeFunctionData({ abi: [{ type: "function", name: "platformOwed", inputs: [], outputs: [{ type: "uint256" }], stateMutability: "view" }], functionName: "platformOwed" }))
      return w(this.owed.get(to.toLowerCase()) ?? 0n);
    if (to.toLowerCase() === TREASURY) {
      if (data.startsWith("0x" + "c88a5e6d".slice(0, 0)) && data === sel("unallocated()")) return w(this.unallocated);
      if (data === sel("totalToOps()")) return w(this.totalToOps);
      if (data === sel("ops()")) return `0x${this.opsAddr.slice(2).padStart(64, "0")}` as Hex;
    }
    throw new Error(`unexpected call ${to} ${data}`);
  }
  async evmBroadcast(chain: number, raw: Hex) {
    this.crash(`broadcast-${chain}`);
    if (!this.autoMine) { this.mempool.push(raw); return; }
    this.mine(chain, raw);
  }
  mine(chain: number, raw: Hex) {
    const tx = parseTransaction(raw);
    const hash = signerHash(raw);
    if (tx.nonce! < this.nonce[chain]) return;                                  // a rebroadcast of a mined transaction: "nonce too low"
    assert.equal(tx.nonce, this.nonce[chain], "nonces in order");
    this.nonce[chain]++;
    const to = tx.to!.toLowerCase(), value = tx.value ?? 0n, data = (tx.data ?? "0x").toLowerCase();
    const b = this.bal[chain];
    b.set(ME, (b.get(ME) ?? 0n) - value - 1_000_000_000_000n);
    this.mined.push({ chain, to, value, data, nonce: tx.nonce! });
    this.receipts.set(hash, { ok: true, block: BigInt(this.mined.length) });
    if (chain === 4663 && to === RELAY_DEPOSITORY_4663 && this.relayPaysOut) this.bal[1].set(ME, (this.bal[1].get(ME) ?? 0n) + (value * 997n) / 1000n);
    if (chain === 4663 && data === SEL.claimPlatformFees) { this.unallocated += this.owed.get(to) ?? 0n; this.owed.set(to, 0n); }
    if (chain === 4663 && data === SEL.allocate) { const ops = this.unallocated / 10n; this.totalToOps += ops; this.unallocated = 0n; b.set(ME, (b.get(ME) ?? 0n) + ops); }
    if (chain === 4663 && to === PAYEE) b.set(PAYEE, (b.get(PAYEE) ?? 0n) + value);
    if (chain === 1 && to === CKETH_HELPER) this.ck += value;                 // the minter mints ckETH (about 20 min, here at once)
  }
  async relayQuote(body: Record<string, unknown>) { this.calls.push("quote"); return relayQuote(String(body.user), BigInt(String(body.amount)), T0 + 600); }
  async prices() { return { cg: this.prices_.cg, cb: this.prices_.cb }; }
  async curves() { return [CURVE]; }
  async booksTreasury(): Promise<{ ops: string | null; totalToOps: bigint }> { throw new Error("no books on the robinhood home"); }
  async cycles(c: string) { return this.canisters.get(c)!; }
  async xdrRate() { return XDR; }
  async minterHelper() { return this.helper; }
  async poolTokens() { return this.tokens; }
  async ckBalance() { return this.ck; }
  async ckApprove(a: bigint) { this.calls.push("approve"); this.ck -= CKETH_FEE_WEI; this.ckAllowance = a; }
  async poolUnused() { return { ...this.unused }; }
  async poolDeposit(a: bigint) {
    this.calls.push("deposit"); this.crash("pool-deposit");
    assert.ok(this.ckAllowance >= a + CKETH_FEE_WEI, "allowance"); this.ck -= a + CKETH_FEE_WEI; this.ckAllowance -= a + CKETH_FEE_WEI; this.unused.ck += a;
    this.crash("pool-deposit-after");
  }
  quoteOf(ck: bigint) { return BigInt(Math.floor((Number(ck) / 1e18) * (PRICES.eth / PRICES.icp) * 1e8 * 0.997)); }
  async poolQuote(ck: bigint) { return this.quoteOf(ck); }
  async poolSwap(ck: bigint, min: bigint) {
    this.calls.push("swap"); const out = this.quoteOf(ck); assert.ok(out >= min); this.unused.ck -= ck; this.unused.icp += out; this.crash("pool-swap-after"); return out;
  }
  async poolWithdraw(e8s: bigint) { this.calls.push("withdraw"); this.unused.icp -= e8s; this.icp += e8s - ICP_FEE_E8S; this.crash("pool-withdraw-after"); }
  async icpBalance() { return this.icp; }
  async icpTransfer(to: Uint8Array, e8s: bigint, at: bigint) {
    const key = `${hex(to)}:${at}`;
    const i = this.icpBlocks.findIndex((x) => x.key === key);
    if (i >= 0) return BigInt(i);                                              // the ledger's dedup: the same block again
    this.icp -= e8s + ICP_FEE_E8S;
    this.icpBlocks.push({ to: hex(to), e8s, key });
    this.crash("icp-transfer-after");
    return BigInt(this.icpBlocks.length - 1);
  }
  async notifyTopUp(block: bigint, canister: string): Promise<Notify> {
    const b = this.icpBlocks[Number(block)];
    if (b.to !== hex(topUpAccount(canister))) return { err: "InvalidTransaction" };
    const got = this.notified.get(Number(block));
    if (got !== undefined) return { ok: got };
    const cycles = (b.e8s * XDR * T) / 10_000n / 100_000_000n;
    this.notified.set(Number(block), cycles);
    this.canisters.set(canister, this.canisters.get(canister)! + cycles);
    this.crash("notify-after");
    return { ok: cycles };
  }
}
const sel = (sig: string) => encodeFunctionData({ abi: [{ type: "function", name: sig.split("(")[0], inputs: [], outputs: [], stateMutability: "view" }], functionName: sig.split("(")[0] });
import { keccak256 } from "viem";
const signerHash = (raw: Hex) => keccak256(raw);

/** A Relay quote for `user` and `amount` that hashes correctly: the fixture's order, its addresses and amounts replaced. */
function relayQuote(user: string, amount: bigint, nowS: number) {
  const q = JSON.parse(readFileSync(new URL("../../autobuyer/canister-rs/tests/fixtures/relay-quote-4663-1-2026-09-27.json", import.meta.url), "utf8"));
  const o = q.protocol.v2.orderData, u = user.toLowerCase(), dl = nowS + 7 * 86_400;
  o.inputs[0].payment.amount = amount.toString();
  for (const r of o.inputs[0].refunds) { r.recipient = u; r.deadline = dl; }
  o.output.payments[0].recipient = u;
  o.output.payments[0].minimumAmount = ((amount * 992n) / 1000n).toString();
  o.output.payments[0].expectedAmount = ((amount * 997n) / 1000n).toString();
  o.output.deadline = dl;
  const id = orderIdOf(o);
  q.protocol.v2.orderId = id;
  Object.assign(q.steps[0].items[0].data, { from: u, to: RELAY_DEPOSITORY_4663, chainId: 4663, value: amount.toString(), data: `${SEL.depositNative}${u.slice(2).padStart(64, "0")}${id.slice(2)}` });
  return q;
}

const CFG: EngineCfg = { mode: "send", caps: DEFAULT_CAPS, buyer: BUYER, timelock: TIMELOCK, treasury: TREASURY, payee: PAYEE,
  claimMinWei: WEI / 500n, allocateMinWei: WEI / 500n, ethMaxFeeWei: 50_000_000_000n, stuckS: 7_200, mintStuckS: 3_600 };

/** A journal that stops recording when the world says the process died. */
class CrashJournal implements Journal {
  inner = new MemoryJournal(); frozen = false;
  load(): State { return this.inner.load(); }
  save(s: State): void { if (!this.frozen) this.inner.save(s); }
}

function rig(w = new World(), cfg: Partial<EngineCfg> = {}) {
  const j = new CrashJournal();
  let now = T0;
  const make = () => new Engine({ io: w, signer, journal: j, cfg: { ...CFG, ...cfg }, log: silent, nowS: () => now });
  const box = { e: make() };
  w.onCrash = () => { j.frozen = true; };
  const restart = () => { j.frozen = false; box.e = make(); };
  const tick = async (n = 1, dt = 60) => { for (let i = 0; i < n; i++) { now += dt; await box.e.tick(); if (j.frozen) restart(); } };
  return { w, j, box, tick, at: () => now, setNow: (t: number) => { now = t; } };
}
const minedOf = (w: World, pred: (m: World["mined"][number]) => boolean) => w.mined.filter(pred).length;

// ---- 5. the run, end to end ----------------------------------------------------------------------------------------------------

test("a run end to end: 34 T → the buyer gets about 20 T, through Relay, the helper, the pool and the CMC; nothing left behind", async () => {
  const { w, box, tick } = rig();
  w.canisters.set(TIMELOCK, 2n * T);
  await box.e.preflight();
  await tick(30);
  assert.equal(box.e.s.run, null, `run left open: ${JSON.stringify(box.e.s.history.at(-1))}`);
  const h = box.e.s.history.at(-1)!;
  assert.equal(h.phase, "done", h.note);
  const got = w.canisters.get(BUYER)! - 34n * T;
  assert.ok(got > 19n * T && got <= 21n * T, `the buyer got ${Number(got) / 1e12} T`);
  assert.equal(minedOf(w, (m) => m.to === RELAY_DEPOSITORY_4663), 1, "one Relay deposit");
  assert.equal(minedOf(w, (m) => m.to === CKETH_HELPER), 1, "one helper deposit");
  assert.equal(w.icpBlocks.length, 1, "one ICP transfer, to the buyer's top-up account");
  assert.equal(w.icpBlocks[0].to, hex(topUpAccount(BUYER)));
  assert.equal(w.unused.ck + w.unused.icp, 0n, "nothing left in the pool");
  assert.ok(w.icp < 100_000n, `ICP dust only: ${w.icp}`);
  assert.ok(w.ck < 10n * CKETH_FEE_WEI, `ckETH dust only: ${w.ck}`);
  assert.ok((w.bal[1].get(ME) ?? 0n) < 200_000_000_000_000n, "the bridged ETH went into the deposit, less its gas");
  // the monitor's numbers: the nonces the fueler journaled are the chain's
  assert.equal(box.e.s.nonces[4663], w.nonce[4663]);
  assert.equal(box.e.s.nonces[1], w.nonce[1]);
  assert.deepEqual(Object.keys(box.e.s.alerts), []);
  assert.equal(box.e.s.conversions.length, 1);
  assert.ok(box.e.s.conversions[0].cycles > 19n * T, "the caps count what was minted");
});

for (const crashAt of ["broadcast-4663", "broadcast-1", "pool-deposit", "pool-deposit-after", "pool-swap-after", "pool-withdraw-after", "icp-transfer-after", "notify-after"]) {
  test(`a crash at ${crashAt} never pays twice: one Relay deposit, one helper deposit, one ICP transfer, one top-up`, async () => {
    const { w, box, tick } = rig();
    w.crashAt = crashAt;
    await box.e.preflight();
    await tick(40);
    assert.equal(w.crashAt, null, "the crash happened");
    assert.equal(box.e.s.run, null, `run left open at ${box.e.s.run?.phase}: ${box.e.s.run?.note}`);
    assert.equal(box.e.s.history.at(-1)!.phase, "done", box.e.s.history.at(-1)!.note);
    assert.equal(minedOf(w, (m) => m.to === RELAY_DEPOSITORY_4663), 1);
    assert.equal(minedOf(w, (m) => m.to === CKETH_HELPER), 1);
    assert.equal(w.icpBlocks.length, 1);
    assert.equal(w.notified.size, 1);
    const got = w.canisters.get(BUYER)! - 34n * T;
    assert.ok(got > 19n * T && got <= 21n * T, `the buyer got ${Number(got) / 1e12} T`);
  });
}

test("a transaction that does not mine is broadcast again with the same bytes, and pages after 2 h; it is never re-signed", async () => {
  const { w, box, tick } = rig();
  w.bal[4663].set(ME, WEI / 10n);                       // under the buffer: nothing to forward, so the first transaction is the run's
  await box.e.preflight();
  w.autoMine = false;
  await tick(2);                                        // the first tick opens the run, the second signs its deposit
  assert.equal(box.e.s.run?.phase, "relay-sent");
  await tick(5, 60);
  assert.ok(w.mempool.length >= 5 && new Set(w.mempool).size === 1, "the same raw transaction each time");
  await tick(1, 7_300);
  assert.ok(box.e.s.alerts["cycles-autofund-failed"], "paged after 2 h");
  w.autoMine = true;
  w.mine(4663, w.mempool[0]);
  await tick(30);
  assert.equal(box.e.s.history.at(-1)?.phase, "done");
  assert.equal(minedOf(w, (m) => m.to === RELAY_DEPOSITORY_4663), 1);
  assert.equal(box.e.s.alerts["cycles-autofund-failed"], undefined, "cleared by the run that finished");
});

test("dry mode signs nothing, plans the run and says so", async () => {
  const { w, box, tick } = rig(new World(), { mode: "dry" });
  w.owed.set(CURVE, WEI / 100n);
  await tick(5);
  assert.equal(w.mined.length, 0);
  assert.equal(box.e.s.run, null);
  assert.match(box.e.s.lastDecision ?? "", /^run: /);
});

test("halts (SEV1) when the minter names another helper, or the pool trades other tokens; nothing is signed", async () => {
  for (const edit of [(w: World) => { w.helper = "0x" + "66".repeat(20); }, (w: World) => { w.tokens = { token0: IC.ICP_LEDGER, token1: "xevnm-gaaaa-aaaar-qafnq-cai" }; }]) {
    const { w, box, tick } = rig();
    edit(w);
    await tick(5);
    assert.ok(box.e.s.halted);
    assert.equal(box.e.s.alerts["fueler-halt"]?.sev, "SEV1");
    assert.equal(minedOf(w, (m) => m.to === RELAY_DEPOSITORY_4663 || m.chain === 1), 0, "no money moved toward the route");
    const n = w.mined.length;
    await tick(5);
    assert.equal(w.mined.length, n, "a halted fueler signs nothing more, not even housekeeping");
  }
});

test("prices that disagree: no run, and a page", async () => {
  const { w, box, tick } = rig();
  w.prices_.cb = { eth: 2900, icp: 3.2 };
  await tick(3);
  assert.equal(minedOf(w, (m) => m.to === RELAY_DEPOSITORY_4663), 0);
  assert.ok(box.e.s.alerts["cycles-autofund-oracle"]);
});

test("the 7-day cap: three runs at 20 T, then it stops and pages until a human decides", async () => {
  const { w, box, tick } = rig();
  await box.e.preflight();
  for (let i = 0; i < 4; i++) { w.canisters.set(BUYER, 30n * T); w.bal[4663].set(ME, WEI / 5n); await tick(30); await tick(1, 6 * 3_600); }
  assert.equal(minedOf(w, (m) => m.to === RELAY_DEPOSITORY_4663), 3);
  assert.ok(box.e.s.alerts["cycles-autofund-cap"], "paged");
});

test("housekeeping: claims a curve's platform fees, allocates the treasury (the ops share arrives), forwards above the buffer to the cold payee", async () => {
  const { w, box, tick } = rig();
  w.canisters.set(BUYER, 100n * T);                     // no run: housekeeping only
  w.owed.set(CURVE, 3n * WEI);
  w.bal[4663].set(ME, WEI / 10n);
  await box.e.preflight();
  await tick(5);
  assert.ok(w.mined.some((m) => m.data === SEL.claimPlatformFees && m.to === CURVE));
  assert.ok(w.mined.some((m) => m.data === SEL.allocate && m.to === TREASURY));
  assert.equal(w.totalToOps, (3n * WEI) / 10n);
  const fwd = w.mined.filter((m) => m.to === PAYEE);
  assert.equal(fwd.length, 1);
  const left = w.bal[4663].get(ME)!;
  assert.ok(left <= DEFAULT_CAPS.bufferWei + 2_000_000_000_000_000n && left > DEFAULT_CAPS.bufferWei, `${left}`);
});

// ---- 6. the configuration ----------------------------------------------------------------------------------------------------

const ENV = { FUEL_HOME: "robinhood", FUELER_BUYER: BUYER, FUELER_TIMELOCK: TIMELOCK, FUELER_TREASURY: TREASURY, FUELER_PAYEE: PAYEE,
  FUELER_RPC_4663: "https://rpc.mainnet.chain.robinhood.com,https://rpc.ordofi.network", FUELER_RPC_1: "https://rpc.mevblocker.io,https://gateway.tenderly.co/public/mainnet",
  FUELER_INDEXER_URL: "https://indexer.example" };

test("config: dry by default; refuses one provider, http, a payee that is the treasury, inverted thresholds, a bad mode", () => {
  const c = parseFuelerConfig(ENV);
  assert.equal(c.engine.mode, "dry");
  assert.equal(c.engine.caps.runMaxT, 20);
  assert.equal(c.engine.caps.dayMaxWei, WEI / 10n);
  assert.equal(c.everyS, 1_800);
  const bad = (e: Record<string, string>, re: RegExp) => assert.throws(() => parseFuelerConfig({ ...ENV, ...e }), re);
  bad({ FUELER_RPC_1: "https://rpc.mevblocker.io" }, /at least two/);
  bad({ FUELER_RPC_4663: "http://a.example,https://b.example" }, /not https/);
  bad({ FUELER_PAYEE: TREASURY }, /cold address/);
  bad({ FUELER_BUYER_LOW_T: "70" }, /TARGET_T must be above/);
  bad({ FUELER_MODE: "yolo" }, /neither dry nor send/);
  bad({ FUELER_BUYER: "not a principal" }, /canister id/);
  bad({ FUELER_WEEK_MAX_ETH: "0.05" }, /WEEK_MAX_ETH/);
});

test("the journal round-trips big numbers exactly", () => {
  const j = new MemoryJournal();
  const s = emptyState();
  s.conversions.push({ at: 1, wei: 123456789012345678901234567890n, cycles: 2n ** 100n });
  j.save(s);
  assert.deepEqual(j.load().conversions[0], { at: 1, wei: 123456789012345678901234567890n, cycles: 2n ** 100n });
});

test("a run with no acceptable Relay quote gives up after 6 h without counting against the caps, and housekeeping resumes", async () => {
  const { w, box, tick } = rig();
  w.bal[4663].set(ME, WEI / 10n);
  await box.e.preflight();
  w.relayQuote = async () => { throw new Error("Relay refused the quote: HTTP 401 Please provide an api key"); };
  await tick(2);
  assert.equal(box.e.s.run?.phase, "quote");
  await tick(1, 3 * 3_600);
  assert.ok(box.e.s.alerts["cycles-autofund-failed"], "paged after 2 h");
  assert.equal(box.e.s.run?.phase, "quote");
  await tick(1, 4 * 3_600);
  assert.equal(box.e.s.run, null);
  assert.equal(box.e.s.history.at(-1)?.phase, "failed");
  assert.equal(box.e.s.conversions.length, 0, "nothing left the wallet, nothing is counted");
  assert.equal(w.mined.length, 0);
});

test("the operator's exits: FUELER_UNHALT clears a halt; FUELER_ABORT_RUN closes the named run, never one with a transaction out", async () => {
  const { w, box, tick } = rig();
  w.helper = "0x" + "66".repeat(20);
  await tick(3);
  assert.ok(box.e.s.halted);
  assert.deepEqual(box.e.operatorExits({ unhalt: false, abortRun: null }), []);
  w.helper = CKETH_HELPER;
  assert.match(box.e.operatorExits({ unhalt: true, abortRun: null })[0], /halt cleared/);
  assert.equal(box.e.s.halted, null);
  assert.equal(box.e.s.alerts["fueler-halt"], undefined);
  // a run whose Relay fill never arrives (a refund on 4663, say): the operator closes it once the money is located
  w.relayPaysOut = false;
  w.bal[4663].set(ME, WEI / 10n);
  await tick(6);
  assert.equal(box.e.s.run?.phase, "bridging");
  const id = box.e.s.run!.id;
  assert.match(box.e.operatorExits({ unhalt: false, abortRun: "run-1" })[0], /names no open run/);
  assert.match(box.e.operatorExits({ unhalt: false, abortRun: id })[0], /aborted/);
  assert.equal(box.e.s.run, null);
  assert.equal(box.e.s.history.at(-1)?.phase, "failed");
  // with a transaction out, it refuses
  const { w: w2, box: b2, tick: t2 } = rig();
  w2.bal[4663].set(ME, WEI / 10n); w2.autoMine = false;
  await b2.e.preflight(); await t2(2);
  assert.equal(b2.e.s.run?.phase, "relay-sent");
  assert.match(b2.e.operatorExits({ unhalt: false, abortRun: b2.e.s.run!.id })[0], /transaction out/);
  assert.ok(b2.e.s.run);
});

test("config: the operator's exits are explicit", () => {
  assert.equal(parseFuelerConfig({ ...ENV, FUELER_UNHALT: "yes" }).unhalt, true);
  assert.throws(() => parseFuelerConfig({ ...ENV, FUELER_UNHALT: "true" }), /yes/);
  assert.equal(parseFuelerConfig({ ...ENV, FUELER_ABORT_RUN: "run-1791600060" }).abortRun, "run-1791600060");
  assert.throws(() => parseFuelerConfig({ ...ENV, FUELER_ABORT_RUN: "everything" }), /run id/);
});
