import assert from "node:assert/strict";
import { test } from "node:test";
import { mkdtempSync, writeFileSync, mkdirSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { boundedLogs, judge, PUBLIC_METHODS, verdict, Windows } from "../chain/filter.js";
import { proxy } from "../chain/proxy.js";
import { assertBootstrapTarget } from "../bootstrap/bootstrap.js";
import { STAGING_CHAIN_ID, assertStagingChain } from "../chain-id.js";
import { upstreamShim, UPSTREAM_READS } from "../chain/upstream.js";

test("the proxy forwards by ALLOWLIST: every admin namespace, every unsigned send and every method nobody has thought of yet is refused", () => {
  for (const m of ["anvil_setBalance", "anvil_impersonateAccount", "anvil_setStorageAt", "anvil_setCode", "anvil_autoImpersonateAccount", "hardhat_setBalance", "evm_mine",
    "evm_increaseTime", "evm_setNextBlockTimestamp", "debug_traceTransaction", "personal_sign", "admin_peers", "miner_start", "txpool_content", "engine_newPayloadV1",
    "eth_sendTransaction", "eth_sendUnsignedTransaction", "eth_sign", "eth_signTransaction", "eth_signTypedData_v4", "eth_accounts", "eth_requestAccounts",
    "eth_newFilter", "eth_subscribe", "some_methodFromTheFuture", "", "ETH_CALL"]) assert.equal(verdict(m).ok, false, m);
  for (const m of [undefined, null, 7, {}, ["eth_call"]]) assert.equal(verdict(m).ok, false);
  for (const m of ["eth_call", "eth_getLogs", "eth_chainId", "eth_sendRawTransaction", "eth_getTransactionReceipt", "eth_estimateGas", "eth_getBalance"]) assert.equal(verdict(m).ok, true, m);
  assert.ok(![...PUBLIC_METHODS].some((m) => /^(anvil|evm|hardhat|debug|personal)_/.test(m) || /sendTransaction$|sign|accounts/i.test(m)), "nothing on the list acts for an address or administers the node");
});

test("a batch is judged call by call, a smuggled admin call is refused while its neighbours go through, and a long batch is refused whole", () => {
  const j = judge([{ id: 1, method: "eth_chainId" }, { id: 2, method: "anvil_setBalance" }, { id: 3, method: "eth_call" }]);
  assert.ok(!("error" in j));
  if (!("error" in j)) { assert.deepEqual(j.forward.map((c) => c.id), [1, 3]); assert.deepEqual(j.refused.map((r) => r.id), [2]); assert.equal(j.batch, true); }
  assert.ok("error" in judge(Array.from({ length: 51 }, (_x, i) => ({ id: i, method: "eth_chainId" }))));
  assert.ok("error" in judge([]));
  assert.ok("error" in judge([null]));
  assert.equal(boundedLogs({ method: "eth_getLogs", params: [{ fromBlock: "0x1", toBlock: "0x2" }] }), null);
  assert.match(String(boundedLogs({ method: "eth_getLogs", params: [{ toBlock: "latest" }] })), /numeric fromBlock/);
  assert.match(String(boundedLogs({ method: "eth_getLogs", params: [{ fromBlock: "0x0", toBlock: "0xfffffff" }] })), /more than/);
});

test("windows: a cap per key per window, which resets when the window has passed", () => {
  let t = 0; const w = new Windows(1_000, 2, () => t);
  assert.deepEqual([w.take("a"), w.take("a"), w.take("a"), w.take("b")], [true, true, false, true]);
  t = 1_001; assert.equal(w.take("a"), true);
});

test("the faucet: an address once a day, a place N times a day, never one of the deployment's own addresses; admin calls never reach the node", async () => {
  const data = mkdtempSync(join(tmpdir(), "staging-")); mkdirSync(join(data, "snapshots"));
  const KEEPER = "0x00000000000000000000000000000000000000aa";
  writeFileSync(join(data, "deployment.json"), JSON.stringify({ chainId: STAGING_CHAIN_ID, keeper: KEEPER }));
  const asked: string[] = [];
  const node = (async (_u: string, init: RequestInit) => { const b = JSON.parse(String(init.body)) as { method: string } | { method: string }[];
    for (const c of Array.isArray(b) ? b : [b]) asked.push(c.method);
    return { json: async () => (Array.isArray(b) ? b.map((c, i) => ({ jsonrpc: "2.0", id: i, result: "0x0" })) : { jsonrpc: "2.0", id: 1, result: "0x0" }) }; }) as unknown as typeof fetch;
  const p = proxy({ node: "http://node", data, port: 0, chainId: STAGING_CHAIN_ID, forkBlock: 1, faucetWei: 10n ** 18n, publicUrl: "", waitS: 120, faucetPerIp: 2, fetchImpl: node });
  await new Promise<void>((ok) => p.server.listen(0, "127.0.0.1", ok));
  const base = `http://127.0.0.1:${(p.server.address() as { port: number }).port}`;
  const drip = (address: string) => fetch(`${base}/faucet`, { method: "POST", body: JSON.stringify({ address }) });
  const A = "0x" + "1".repeat(40), B = "0x" + "2".repeat(40), C = "0x" + "3".repeat(40);
  assert.equal((await drip(A)).status, 200);
  assert.equal((await drip(A)).status, 429, "once a day for an address");
  assert.equal((await drip(KEEPER)).status, 400, "not the deployment's own roles");
  assert.equal((await drip("not an address")).status, 400);
  assert.equal((await drip(B)).status, 200);
  assert.equal((await drip(C)).status, 429, "N a day from one place");
  const before = asked.length;
  const r = (await (await fetch(base, { method: "POST", body: JSON.stringify({ jsonrpc: "2.0", id: 9, method: "anvil_setBalance", params: [A, "0xffff"] }) })).json()) as { error: { message: string } };
  assert.match(r.error.message, /refused by the staging proxy/);
  assert.equal(asked.length, before, "a refused call is never forwarded");
  const info = (await (await fetch(`${base}/info`)).json()) as { chainId: number; forkOf: number; deployment: { keeper: string } };
  assert.deepEqual([info.chainId, info.forkOf, info.deployment.keeper], [STAGING_CHAIN_ID, 4663, KEEPER]);
  assert.equal((await fetch(`${base}/snapshots/../deployment.json`)).status, 404);
  p.server.closeAllConnections(); p.server.close();
});

test("the bootstrap deploys to our own chains from this machine, and to nothing else", () => {
  assertBootstrapTarget(STAGING_CHAIN_ID, "http://127.0.0.1:8546"); assertBootstrapTarget(31337, "http://localhost:8545");
  assert.throws(() => assertBootstrapTarget(4663, "http://127.0.0.1:8546"), /refused/);
  assert.throws(() => assertBootstrapTarget(1, "http://127.0.0.1:8546"), /refused/);
  assert.throws(() => assertBootstrapTarget(STAGING_CHAIN_ID, "https://rpc.mainnet.chain.robinhood.com"), /this machine/);
  // the id is ours alone: not Robinhood Chain, not its public testnet (which reports 46630), not anything else
  assert.equal(STAGING_CHAIN_ID, 9466301);
  for (const real of [4663, 46630]) { assert.throws(() => assertBootstrapTarget(real, "http://127.0.0.1:8546"), /real network/); assert.throws(() => assertStagingChain(real), /refused/); }
  assert.throws(() => assertStagingChain(31337), /nothing else/); assertStagingChain(STAGING_CHAIN_ID);
});

test("the upstream shim: reads only, and a state read the real RPC calls too old is asked again at latest, and counted", async () => {
  for (const m of ["eth_sendRawTransaction", "eth_sendTransaction", "eth_sign", "personal_sign", "anvil_setBalance", "debug_traceCall", "eth_accounts"]) assert.ok(!UPSTREAM_READS.test(m), m);
  const seen: unknown[][] = [];
  const real = (async (_u: string, init: RequestInit) => { const c = JSON.parse(String(init.body)) as { method: string; params: unknown[] }; seen.push([c.method, ...c.params]);
    const old = c.method === "eth_getBalance" && c.params[1] !== "latest";
    return { json: async () => (old ? { jsonrpc: "2.0", id: 1, error: { code: -32000, message: "historical state abc is not available" } } : { jsonrpc: "2.0", id: 1, result: "0x7" }) }; }) as unknown as typeof fetch;
  const { server, stats } = upstreamShim({ upstream: "http://real", port: 0, fetchImpl: real });
  await new Promise<void>((ok) => server.once("listening", ok));
  const at = `http://127.0.0.1:${(server.address() as { port: number }).port}`;
  const post = async (b: unknown) => (await fetch(at, { method: "POST", body: JSON.stringify(b) })).json() as Promise<{ id: unknown; result?: string; error?: { message: string } } & unknown[]>;
  assert.equal((await post({ id: 9, method: "eth_getBalance", params: ["0xabc", "0x10"] })).result, "0x7");
  assert.deepEqual(seen, [["eth_getBalance", "0xabc", "0x10"], ["eth_getBalance", "0xabc", "latest"]]);
  assert.match((await post({ id: 1, method: "eth_sendRawTransaction", params: ["0x00"] })).error!.message, /refused/);
  assert.equal(seen.length, 2, "a refused method never reaches the real RPC");
  const batch = (await post([{ id: 1, method: "eth_chainId", params: [] }, { id: 2, method: "evm_mine", params: [] }])) as unknown as { id: number; error?: unknown }[];
  assert.deepEqual(batch.map((b) => [b.id, b.error !== undefined]), [[1, false], [2, true]]);
  assert.deepEqual(stats, { fallbacks: 1, refused: 2 });
  server.closeAllConnections(); server.close();
});
