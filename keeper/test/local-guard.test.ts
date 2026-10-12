import assert from "node:assert/strict";
import { test } from "node:test";
import { readFileSync, readdirSync } from "node:fs";
import { LOCAL_CHAIN_ID, localSender } from "../local/guard.js";

/** docs/phase2.md §3: the local runner can send, and only to a node that itself reports chain id 31337, and only in KEEPER_MODE=local. */
const node = (chainId: number, seen: string[] = []) => (async (_u: string, init: RequestInit) => {
  const { method } = JSON.parse(String(init.body)) as { method: string };
  seen.push(method);
  const result = method === "eth_chainId" ? `0x${chainId.toString(16)}` : method === "eth_sendTransaction" ? `0x${"ab".repeat(32)}`
    : method === "eth_getTransactionReceipt" ? { status: "0x1", transactionHash: "0x", blockNumber: "0x1", gasUsed: "0x1", logs: [] } : "0x";
  return { json: async () => ({ result }) };
}) as unknown as typeof fetch;
const TX = { from: "0x0000000000000000000000000000000000000001", to: "0x0000000000000000000000000000000000000002", data: "0x" };

test("no sender without KEEPER_MODE=local — and the node is not even asked", async () => {
  for (const env of [{}, { KEEPER_MODE: "" }, { KEEPER_MODE: "live" }, { KEEPER_MODE: "LOCAL" }, { KEEPER_MODE: "local " }, { KEEPER_MODE: "production" }]) {
    const seen: string[] = [];
    await assert.rejects(localSender("http://x", env, node(LOCAL_CHAIN_ID, seen)), /KEEPER_MODE is not "local"/);
    assert.deepEqual(seen, []);
  }
});

test("no sender for any chain the node reports other than 31337: Robinhood Chain, mainnet, anything", async () => {
  for (const id of [4663, 1, 42161, 31338, 0]) {
    const seen: string[] = [];
    await assert.rejects(localSender("http://x", { KEEPER_MODE: "local" }, node(id, seen)), new RegExp(`reports chain id ${id}`));
    assert.ok(!seen.includes("eth_sendTransaction"));
  }
});

test("with both, it sends — and asks the node which chain it is again before every send, so a repointed RPC is refused mid-run", async () => {
  const seen: string[] = [];
  let id = LOCAL_CHAIN_ID;
  const f = (async (u: string, init: RequestInit) => node(id, seen)(u, init)) as unknown as typeof fetch;
  const s = await localSender("http://x", { KEEPER_MODE: "local" }, f);
  await s.send(TX);
  assert.deepEqual(seen, ["eth_chainId", "eth_chainId", "eth_sendTransaction", "eth_getTransactionReceipt"]);
  id = 4663;                                  // the URL now answers as Robinhood Chain
  seen.length = 0;
  await assert.rejects(s.send(TX), /now reports chain id 4663/);
  assert.deepEqual(seen, ["eth_chainId"], "refused before anything was sent");
});

test("the ability to send lives in ONE file, and no file anywhere in this package holds or reads a key", () => {
  const SEND = /eth_sendTransaction|eth_sendRawTransaction|sendTransaction|sendRawTransaction|createWalletClient|walletClient|writeContract|eth_sign\b|signTransaction|signMessage/;
  const KEY = /viem\/accounts|privateKeyToAccount|mnemonicToAccount|privateKey|PRIVATE_KEY|mnemonic|0x[0-9a-fA-F]{64}(?![0-9a-fA-F])/;
  const BUY = /create_bid|confirm_bid|\/buy\b|sweeping|signPsbt/i;
  const local = new URL("../local/", import.meta.url);
  const files = readdirSync(local).filter((f) => f.endsWith(".ts"));
  assert.deepEqual(files.sort(), ["abis.ts", "e2e.ts", "guard.ts"]);
  for (const f of files) {
    const src = readFileSync(new URL(f, local), "utf8");
    assert.ok(!KEY.test(src), `${f} names a key: ${String(src.match(KEY)?.[0]).slice(0, 20)}`);
    assert.ok(!BUY.test(src), `${f} names an endpoint that buys`);
    if (f !== "guard.ts") assert.ok(!SEND.test(src), `${f} can send without the guard: ${String(src.match(SEND)?.[0])}`);
  }
  const guard = readFileSync(new URL("guard.ts", local), "utf8");
  assert.ok(SEND.test(guard) && /KEEPER_MODE !== "local"/.test(guard) && /chainId !== LOCAL_CHAIN_ID/.test(guard));
  // and src/ is exactly as unable as before: it does not import the guard
  const src = new URL("../src/", import.meta.url);
  for (const f of readdirSync(src).filter((x) => x.endsWith(".ts"))) assert.ok(!/local\/|guard/.test(readFileSync(new URL(f, src), "utf8")), `${f} reaches for the guard`);
});
