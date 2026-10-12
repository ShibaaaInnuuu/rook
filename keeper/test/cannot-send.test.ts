import assert from "node:assert/strict";
import { test } from "node:test";
import { readFileSync, readdirSync } from "node:fs";
import { READ_ONLY, readListings, rpc } from "../src/chain.js";
import { MagicEdenSource } from "../src/sources.js";

test("structurally unable to send: no source file constructs a signer, holds a key, or names a send method or a buy endpoint", () => {
  const dir = new URL("../src/", import.meta.url);
  const banned = /viem\/accounts|privateKeyToAccount|mnemonicToAccount|createWalletClient|walletClient|privateKey|PRIVATE_KEY|mnemonic|signTransaction|signMessage|signPsbt|sendTransaction|sendRawTransaction|writeContract|eth_sign|create_bid|confirm_bid|\/buy\b|sweeping/i;
  const files = readdirSync(dir).filter((f) => f.endsWith(".ts"));
  assert.ok(files.length >= 4);
  for (const f of files) {
    // chain.ts names eth_sendRawTransaction nowhere; the read-only list is an allowlist, so there is nothing to exempt
    const src = readFileSync(new URL(f, dir), "utf8");
    assert.ok(!banned.test(src), `${f} matches ${String(src.match(banned)?.[0])}`);
  }
  const pkg = JSON.parse(readFileSync(new URL("../package.json", import.meta.url), "utf8"));
  // viem for ABI encoding and decoding of reads; tsx only runs the TypeScript (audit R2-S7: the image installs production dependencies only)
  assert.deepEqual(Object.keys(pkg.dependencies).sort(), ["tsx", "viem"], "two dependencies: viem for the ABI, tsx to run the TypeScript");
  // docs/phase2.md §3: the package gained a LOCAL runner (local/), which can send to chain 31337 only. This directory did not change:
  // the dry run that would face a real chain is still everything under src/, and it is still unable. See test/local-guard.test.ts.
  const scripts = pkg.scripts as Record<string, string>;
  assert.equal(scripts.dryrun, "tsx src/run.ts");
  assert.ok(Object.entries(scripts).filter(([, v]) => /local\//.test(v)).every(([, v]) => v.startsWith("KEEPER_MODE=local ")), "every script that runs local/ says so");
});

test("the one RPC function refuses every method that is not a read, before any request is made", async () => {
  let requests = 0;
  const spy = (async () => { requests++; return { json: async () => ({ result: "0x1" }) }; }) as unknown as typeof fetch;
  for (const m of ["eth_sendRawTransaction", "eth_sendTransaction", "eth_sign", "personal_sign", "eth_signTypedData_v4", "anvil_impersonateAccount", "debug_traceCall"]) {
    await assert.rejects(rpc("http://x", m, [], spy), /refused/);
  }
  assert.equal(requests, 0);
  assert.equal(await rpc("http://x", "eth_blockNumber", [], spy), "0x1");
  assert.deepEqual([...READ_ONLY], ["eth_call", "eth_getLogs", "eth_blockNumber", "eth_chainId", "eth_getBlockByNumber"]);
});

test("reading the contracts is eth_call and nothing else", async () => {
  const methods: string[] = [];
  const word = (n: bigint) => n.toString(16).padStart(64, "0");
  const node = (async (_u: string, init: RequestInit) => {
    const { method, params } = JSON.parse(String(init.body)) as { method: string; params: [{ data: string }] };
    methods.push(method);
    const sel = params[0].data.slice(0, 10);
    // count() → 0 listings: enough to show the call path without hand-encoding a struct
    return { json: async () => ({ result: "0x" + word(0n) + (sel ? "" : "") }) };
  }) as unknown as typeof fetch;
  assert.deepEqual(await readListings("http://x", "0x0000000000000000000000000000000000000001", node), []);
  assert.deepEqual(methods, ["eth_call"]);
});

test("the Magic Eden source reads and nothing else: GET only, spaced out, and an answer it does not understand is an error, not a price", async () => {
  const calls: { url: string; method: string }[] = [];
  let t = 0; const slept: number[] = [];
  const f = (async (url: string, init?: RequestInit) => { calls.push({ url, method: init?.method ?? "GET" });
    return { ok: true, status: 200, json: async () => (url.includes("/stat?") ? { floorPrice: "250000" } : url.includes("GOOD") ? { floorUnitPrice: { value: "11.2" } } : { nothing: true }) }; }) as unknown as typeof fetch;
  const me = new MagicEdenSource(f, 5_000, () => t, async (ms) => { slept.push(ms); t += ms; });
  t = 10_000;   // a first call long after start needs no wait
  assert.deepEqual(await me.inscriptionFloor("test-collection"), { sats: 250000n, ref: "magiceden:stat:test-collection" });
  assert.deepEqual(await me.runeUnitPrice("840000:1", "GOOD•RUNE"), { sats: 12n, ref: "magiceden:rune-info:GOODRUNE" });
  await assert.rejects(me.runeUnitPrice("1:1", "OTHER"), /no floorUnitPrice/);
  assert.deepEqual(slept, [5_000, 5_000], "one request per five seconds at most");
  assert.ok(calls.every((c) => c.method === "GET" && c.url.startsWith("https://api-mainnet.magiceden.dev/v2/ord/btc/")));
});
