// The optional guardian (docs/audit/2026-09-26/fixes-W2-monitor.md §1.4): off unless configured, refusing every half-configured
// or mismatched setup, checking the chain before it signs, and signing only pauseKeeper() / cancelDraw(id) for chain 4663, to
// both RPCs, under a gas and fee cap. The key is a throwaway test key and never appears in an error or a JSON dump. No network.
import assert from "node:assert/strict";
import { test } from "node:test";
import { decodeFunctionData, parseTransaction, recoverTransactionAddress } from "viem";
import { privateKeyToAccount } from "viem/accounts";
import { REGISTRY, VAULT } from "../src/abi.mjs";
import { guardianFrom } from "../src/guardian.mjs";
import { fakeChain, hex } from "./fakechain.mjs";

const KEY = "0x" + "4b".repeat(32);                                   // a throwaway key, used nowhere else
const ADDR = privateKeyToAccount(KEY).address.toLowerCase();
const REG = "0x00000000000000000000000000000000000000d1", RAFFLE = "0x00000000000000000000000000000000000000e1";
const cfg = (g = { address: ADDR }) => ({ evm: { rpcs: ["https://a.example", "https://b.example"], chain_id: 4663, registry: REG, guardian: g } });

function world({ chainId = 4663, code = "0x", named = ADDR, paused = false, stage = 1, baseFee = 10_000_000n, gas = 50_000n, receipt = "0x1" } = {}) {
  const sent = [];
  const f = fakeChain({
    views: (url, to, fn) => {
      if (fn === "guardian") return named;
      if (fn === "keeperPaused") return paused;
      if (fn === "draw") return { stage, coin: "0x0000000000000000000000000000000000000c01", challengeEnds: 0n, challengeEndsBlock: 0n, round: 0n, pinnedAt: 0n, pinnedAtBlock: 0n,
        resolvedAt: 0n, resolvedAtBlock: 0n, root: `0x${"00".repeat(32)}`, totalTickets: 0n, winningTicket: 0n, winner: "0x0000000000000000000000000000000000000000", uri: "",
        seedRound: 0n, announcedAt: 0n, announcedAtBlock: 0n, snapshotAt: 0n };
      throw new Error(`no view ${fn}`);
    },
    other: (url, method, params) => {
      if (method === "eth_chainId") return hex(chainId);
      if (method === "eth_getCode") return code;
      if (method === "eth_call") return "0x";                           // the simulation (from the guardian)
      if (method === "eth_getTransactionCount") return url.includes("a.") ? "0x3" : "0x4";
      if (method === "eth_estimateGas") return hex(gas);
      if (method === "eth_getBlockByNumber") return { baseFeePerGas: hex(baseFee) };
      if (method === "eth_sendRawTransaction") { sent.push({ url, raw: params[0] }); return "0x" + "9".repeat(64); }
      if (method === "eth_getTransactionReceipt") return receipt === null ? null : { status: receipt, blockNumber: "0x10" };
      throw new Error(`unexpected ${method}`);
    },
  });
  return { ...f, sent };
}

test("off unless configured; a key without config, config without a key, a bad key, the wrong address, no registry: each refuses to start", () => {
  assert.equal(guardianFrom({ evm: { rpcs: [] } }, {}), null, "neither: no guardian, no key held");
  assert.throws(() => guardianFrom({ evm: { rpcs: [] } }, { MONITOR_GUARDIAN_KEY: KEY }), /set but the config names no evm\.guardian/);
  assert.throws(() => guardianFrom(cfg(), {}), /MONITOR_GUARDIAN_KEY is not set/);
  assert.throws(() => guardianFrom(cfg(), { MONITOR_GUARDIAN_KEY: "0x1234" }), /not a 32-byte hex key \(the value is not repeated\)/);
  assert.throws(() => guardianFrom(cfg({ address: "0x" + "12".repeat(20) }), { MONITOR_GUARDIAN_KEY: KEY }), /is the key of 0x.*, not of evm\.guardian\.address/);
  assert.throws(() => guardianFrom({ evm: { rpcs: [], guardian: { address: ADDR } } }, { MONITOR_GUARDIAN_KEY: KEY }), /needs the registry/);
  for (const bad of [() => guardianFrom(cfg({ address: "0x" + "12".repeat(20) }), { MONITOR_GUARDIAN_KEY: KEY })]) {
    try { bad(); } catch (err) { assert.ok(!String(err.message).includes(KEY.slice(2)), "the key is never in an error"); }
  }
  const g = guardianFrom(cfg(), { MONITOR_GUARDIAN_KEY: KEY });
  assert.ok(!JSON.stringify(g).includes(KEY.slice(2)) && !JSON.stringify(g).includes("4b4b"), "nor in its JSON");
  assert.deepEqual(JSON.parse(JSON.stringify(g)), { address: ADDR, autoPause: true, autoCancel: true });
});

test("pauseKeeper: checks both RPCs' chain, the guardian's code and the registry's guardian, then signs pauseKeeper() for 4663 to both RPCs", async () => {
  const w = world();
  const g = guardianFrom(cfg(), { MONITOR_GUARDIAN_KEY: KEY }, w.http);
  const r = await g.pauseKeeper();
  assert.equal(r.done, true);
  assert.deepEqual(w.sent.map((s) => s.url), ["https://a.example", "https://b.example"]);
  const tx = parseTransaction(w.sent[0].raw);
  assert.deepEqual([tx.chainId, tx.to, tx.nonce, tx.value ?? 0n], [4663, REG, 4, 0n], "chain 4663, the registry, the higher pending nonce");
  assert.equal(decodeFunctionData({ abi: REGISTRY, data: tx.data }).functionName, "pauseKeeper");
  assert.equal((await recoverTransactionAddress({ serializedTransaction: w.sent[0].raw })).toLowerCase(), ADDR);
  assert.ok(tx.gas <= 300_000n && tx.maxFeePerGas <= 2_000_000_000n);
});

test("it will not act on a wrong chain, with code at its address (EIP-7702), or when the registry names another guardian; nor twice", async () => {
  for (const [w, why] of [[world({ chainId: 1 }), /reports chain 1, not 4663/], [world({ code: "0xef0100" + "11".repeat(20) }), /has code \(an EIP-7702 delegation\?\)/],
    [world({ named: "0x" + "00".repeat(20) }), /the registry's guardian is 0x0+, not this key's/]]) {
    const g = guardianFrom(cfg(), { MONITOR_GUARDIAN_KEY: KEY }, w.http);
    await assert.rejects(g.pauseKeeper(), why);
    assert.equal(w.sent.length, 0, "nothing signed");
  }
  const p = world({ paused: true });
  assert.deepEqual(await guardianFrom(cfg(), { MONITOR_GUARDIAN_KEY: KEY }, p.http).pauseKeeper(), { done: false, skipped: "the keeper is already paused" });
  assert.equal(p.sent.length, 0);
});

test("cancelDraw: only an Open or Announced draw; the fee and gas caps refuse rather than overpay", async () => {
  const w = world({ stage: 7 });                                          // Announced
  const r = await guardianFrom(cfg(), { MONITOR_GUARDIAN_KEY: KEY }, w.http).cancelDraw(RAFFLE, "5");
  assert.equal(r.done, true);
  const tx = parseTransaction(w.sent[0].raw);
  assert.equal(tx.to, RAFFLE);
  assert.deepEqual(decodeFunctionData({ abi: VAULT, data: tx.data }), { functionName: "cancelDraw", args: [5n] });
  const pinned = world({ stage: 2 });
  assert.match((await guardianFrom(cfg(), { MONITOR_GUARDIAN_KEY: KEY }, pinned.http).cancelDraw(RAFFLE, "5")).skipped, /the draw is Pinned\/Pinned/);
  await assert.rejects(guardianFrom(cfg(), { MONITOR_GUARDIAN_KEY: KEY }, world({ baseFee: 10_000_000_000n }).http).pauseKeeper(), /above the guardian's cap/);
  await assert.rejects(guardianFrom(cfg(), { MONITOR_GUARDIAN_KEY: KEY }, world({ gas: 1_000_000n }).http).pauseKeeper(), /gas, above the guardian's cap/);
});

test("R2-L3: an action is done only when its receipt says success; not mined in time is pending (retried next poll); a revert throws", async () => {
  const quick = (g = {}) => cfg({ address: ADDR, receipt_wait_ms: 0, receipt_poll_ms: 0, ...g });
  const ok = world();
  assert.deepEqual(await guardianFrom(quick(), { MONITOR_GUARDIAN_KEY: KEY }, ok.http).pauseKeeper(), { done: true, tx: "0x" + "9".repeat(64) });
  const slow = world({ receipt: null });
  assert.deepEqual(await guardianFrom(quick(), { MONITOR_GUARDIAN_KEY: KEY }, slow.http).cancelDraw(RAFFLE, 3), { done: false, pending: true, tx: "0x" + "9".repeat(64) });
  const bad = world({ receipt: "0x0" });
  await assert.rejects(guardianFrom(quick(), { MONITOR_GUARDIAN_KEY: KEY }, bad.http).pauseKeeper(), /mined and REVERTED/);
});
