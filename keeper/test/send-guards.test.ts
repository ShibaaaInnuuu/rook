import assert from "node:assert/strict";
import { test } from "node:test";
import { readFileSync, readdirSync } from "node:fs";
import { generatePrivateKey, privateKeyToAccount } from "viem/accounts";
import { stagingSender, STAGING_CHAIN_ID } from "../staging/guard.js";
import { arm, confirmFor, NotArmed, ROBINHOOD_CHAIN_ID, type Deployment } from "../live/guard.js";
import { build, TEST_MAX_WEI_DEFAULT } from "../live/cli.js";
import { STAGING_MEMBERS, leafOf, memberId, proofOf, stagingRoot } from "../staging/members.js";
import { concat, keccak256, type Hex } from "viem";

/** docs/phase6.md. A node that reports a chain id and counts what it is asked. */
const node = (chainId: number, asked: string[] = []) => (async (_u: string, init: RequestInit) => {
  const { method } = JSON.parse(String(init.body)) as { method: string }; asked.push(method);
  return { ok: true, json: async () => ({ jsonrpc: "2.0", id: 1, result: method === "eth_chainId" ? `0x${chainId.toString(16)}` : method === "eth_getTransactionReceipt" ? { status: "0x1", transactionHash: "0xabc", blockNumber: "0x1", gasUsed: "0x1", logs: [] } : "0xabc" }) };
}) as unknown as typeof fetch;

test("staging: sends only with KEEPER_MODE=staging, only through a node on this machine, and only if that node says the staging chain id — never 4663, never 46630", async () => {
  const L = "http://127.0.0.1:8546";
  await assert.rejects(stagingSender(L, {}, node(STAGING_CHAIN_ID)), /KEEPER_MODE/);
  await assert.rejects(stagingSender(L, { KEEPER_MODE: "local" }, node(STAGING_CHAIN_ID)), /KEEPER_MODE/);
  await assert.rejects(stagingSender(L, { KEEPER_MODE: "mainnet-test" }, node(STAGING_CHAIN_ID)), /KEEPER_MODE/);
  await assert.rejects(stagingSender("https://rpc.mainnet.chain.robinhood.com", { KEEPER_MODE: "staging" }, node(STAGING_CHAIN_ID)), /this machine/);
  await assert.rejects(stagingSender("https://staging.example.com", { KEEPER_MODE: "staging" }, node(STAGING_CHAIN_ID)), /this machine/);
  for (const id of [4663, 46630, 31337, 1]) await   // 46630 is Robinhood's public testnet: a real network, never ours
  await assert.rejects(stagingSender(L, { KEEPER_MODE: "staging" }, node(id)), /refused/);
  assert.equal(STAGING_CHAIN_ID, 9466301);
  assert.equal(STAGING_CHAIN_ID, (JSON.parse(readFileSync(new URL("../../staging/chain-id.json", import.meta.url), "utf8")) as { chainId: number }).chainId, "one source");
  const asked: string[] = [];
  const s = await stagingSender(L, { KEEPER_MODE: "staging" }, node(STAGING_CHAIN_ID, asked));
  await s.send({ from: "0x" + "1".repeat(40), to: "0x" + "2".repeat(40), data: "0x" });
  assert.deepEqual(asked.filter((m) => m === "eth_chainId").length, 2, "the chain is asked at start AND before the send");
  // a node that changes its mind between start and send: the send is refused
  let calls = 0;
  const turncoat = (async (_u: string, init: RequestInit) => { const { method } = JSON.parse(String(init.body)) as { method: string };
    if (method === "eth_chainId") calls++; return { json: async () => ({ result: method === "eth_chainId" ? (calls === 1 ? "0x9071bd" : "0x1237") : "0x" }) }; }) as unknown as typeof fetch;
  const t = await stagingSender(L, { KEEPER_MODE: "staging" }, turncoat);
  await assert.rejects(t.send({ from: "0x" + "1".repeat(40), to: "0x" + "2".repeat(40), data: "0x" }), /4663/);
  const src = readFileSync(new URL("../staging/guard.ts", import.meta.url), "utf8") + readFileSync(new URL("../staging/operator.ts", import.meta.url), "utf8");
  assert.ok(!/viem\/accounts|privateKeyToAccount|mnemonicToAccount|createWalletClient|PRIVATE_KEY|_KEY\b|signTransaction|sendRawTransaction/.test(src), "staging holds no key and has no code that could use one");
});

test("tier 2 signs on FIVE facts together and says which is missing: mode, latch, confirm string, a deployment labelled test naming this signer, and a node that says 4663", async () => {
  const key = generatePrivateKey(), address = privateKeyToAccount(key).address;     // made here, for a fake node; it has never held anything
  const dep: Deployment = { chainId: 4663, label: "test", keeper: address, attestor: "0x" + "9".repeat(40) };
  const good = { KEEPER_MODE: "mainnet-test", KEEPER_LIVE: "1", KEEPER_CONFIRM: confirmFor("keeper", address), KEEPER_KEY: key };
  const U = "https://rpc.example";
  const armed = await arm("keeper", U, dep, good, node(4663));
  assert.equal(armed.address, address);
  assert.equal(confirmFor("keeper", address), `keeper-live:4663:${address.toLowerCase()}`);
  assert.equal(ROBINHOOD_CHAIN_ID, 4663);
  const missing = async (env: Record<string, string | undefined>, d: Deployment, chain: number, re: RegExp) => {
    const asked: string[] = [];
    await assert.rejects(arm("keeper", U, d, env, node(chain, asked)), (e: unknown) => e instanceof NotArmed && re.test(e.message), String(re));
    assert.ok(!asked.includes("eth_sendRawTransaction") && !asked.includes("eth_sendTransaction"));
  };
  await missing({ ...good, KEEPER_MODE: undefined }, dep, 4663, /KEEPER_MODE/);
  for (const mode of ["mainnet", "live", "staging", "local", "dry-run"]) await missing({ ...good, KEEPER_MODE: mode }, dep, 4663, /KEEPER_MODE/);
  await missing({ ...good, KEEPER_LIVE: undefined }, dep, 4663, /KEEPER_LIVE/);
  await missing({ ...good, KEEPER_LIVE: "true" }, dep, 4663, /KEEPER_LIVE/);
  await missing({ ...good, KEEPER_CONFIRM: undefined }, dep, 4663, /KEEPER_CONFIRM must be exactly/);
  await missing({ ...good, KEEPER_CONFIRM: confirmFor("attestor", address) }, dep, 4663, /KEEPER_CONFIRM/);           // another role's confirm
  await missing({ ...good, KEEPER_CONFIRM: `keeper-live:9466301:${address.toLowerCase()}` }, dep, 4663, /KEEPER_CONFIRM/); // another chain's
  await missing({ ...good, KEEPER_KEY: undefined }, dep, 4663, /KEEPER_KEY is not set/);
  await missing(good, { ...dep, label: "mainnet" }, 4663, /label/);      // THE deployment is not signed for by this package, ever
  await missing(good, { ...dep, label: undefined }, 4663, /label/);
  await missing(good, { ...dep, label: "staging" }, 4663, /label/);
  await missing(good, { ...dep, chainId: 9466301 }, 4663, /chainId/);
  await missing(good, { ...dep, keeper: "0x" + "8".repeat(40) }, 4663, /not the deployment's keeper/);
  await missing(good, dep, 9466301, /node reports chain id 9466301/);
  await missing(good, dep, 1, /node reports chain id 1/);
  // the attestor's latch is its own: the keeper's facts do not arm it
  await assert.rejects(arm("attestor", U, { ...dep, attestor: address }, good, node(4663)), NotArmed);
  // every missing fact is reported at once
  await assert.rejects(arm("keeper", U, { chainId: 1 }, {}, node(1)), (e: unknown) => e instanceof NotArmed && e.missing.length >= 4);
});

test("tier 2's commands: a declaration over the test's ceiling is refused before anything is armed, and --dry signs nothing", () => {
  assert.throws(() => build(["declare", "0", String(TEST_MAX_WEI_DEFAULT + 1n), "1", "unisat:x"]), /over this test's ceiling/);
  assert.equal(build(["declare", "0", String(TEST_MAX_WEI_DEFAULT), "1", "unisat:x"]).role, "keeper");
  assert.throws(() => build(["declare", "0", "1000", "1"]), /capRef/);
  assert.equal(build(["settle-rune", "1", "0", "500", "ab".repeat(32), "1000"]).role, "attestor");
  assert.throws(() => build(["record-delivery", "0", "0", "nothex"]), /txid/);
  assert.throws(() => build(["withdraw", "0"]), /commands:/);
  const cli = readFileSync(new URL("../live/cli.ts", import.meta.url), "utf8");
  const dryAt = cli.indexOf("if (dry)"), armAt = cli.indexOf("await arm(");
  assert.ok(dryAt > 0 && dryAt < armAt && cli.slice(dryAt, armAt).includes("process.exit(0)"), "--dry leaves before arming");
});

test("where sending code may live: src/ none at all; local/ 31337; staging/ 9466301 and keyless; live/ the only place a key is read, and only through arm()", () => {
  const banned = /viem\/accounts|privateKeyToAccount|createWalletClient|_KEY\b|sendRawTransaction/;
  // docs/v4/eth-nft-canister.md §12.2: the exceptions are API keys, not signing keys — OPENSEA_API_KEY, read by src/opensea.ts for one
  // header (test/opensea.test.ts), UNISAT_API_KEY, read by src/ordapi.ts for one header (docs/v6/ordinals-apis.md, test/ordapi.test.ts),
  // and both by src/log.ts to cut them out of every line. Exactly those names, in exactly those files.
  const API_KEYS: Record<string, string[]> = { "../src/opensea.ts": ["OPENSEA_API_KEY"], "../src/ordapi.ts": ["UNISAT_API_KEY"], "../src/log.ts": ["OPENSEA_API_KEY", "UNISAT_API_KEY"] };
  const apiKeyOk = (path: string, src: string) => (API_KEYS[path] ?? []).reduce((s, k) => s.replaceAll(k, ""), src);
  for (const dir of ["../src/", "../local/", "../staging/"]) for (const f of readdirSync(new URL(dir, import.meta.url)).filter((x) => x.endsWith(".ts"))) {
    assert.ok(!banned.test(apiKeyOk(dir + f, readFileSync(new URL(dir + f, import.meta.url), "utf8"))), `${dir}${f} must hold no key`);
  }
  const live = readdirSync(new URL("../live/", import.meta.url)).filter((x) => x.endsWith(".ts"));
  assert.deepEqual(live.sort(), ["cli.ts", "guard.ts"]);
  assert.ok(!/privateKeyToAccount|createWalletClient|_KEY/.test(readFileSync(new URL("../live/cli.ts", import.meta.url), "utf8").replace(/\/\*\*[\s\S]*?\*\//, "")), "the CLI never touches a key: it asks guard.ts to arm");
  const guard = readFileSync(new URL("../live/guard.ts", import.meta.url), "utf8");
  assert.ok(!/console\.|log\(/.test(guard), "the guard prints nothing, so it cannot print a key");
  const pkg = JSON.parse(readFileSync(new URL("../package.json", import.meta.url), "utf8")) as { scripts: Record<string, string> };
  assert.ok(Object.entries(pkg.scripts).filter(([, v]) => /staging\//.test(v)).every(([, v]) => v.startsWith("KEEPER_MODE=staging ")));
  assert.ok(!Object.values(pkg.scripts).some((v) => /mainnet-test|_LIVE=1/.test(v)), "no script arms tier 2 for you");
});

test("the staging collection: 1,024 fixture members, the root script/Deploy.s.sol lists on the fork, and proofs that OpenZeppelin's MerkleProof accepts", () => {
  assert.equal(stagingRoot(), "0xc46b9158cf27bde8deea8ab99cdb7bcf071d86c3a8ae4df8019df01f754af505");   // test/DeployChecks.t.sol pins the same value
  const verify = (leaf: Hex, proof: Hex[]) => proof.reduce<Hex>((h, p) => keccak256(BigInt(h) < BigInt(p) ? concat([h, p]) : concat([p, h])), leaf);
  for (const i of [0, 1, 511, 512, STAGING_MEMBERS - 1]) assert.equal(verify(leafOf(memberId(i)), proofOf(i)), stagingRoot());
  assert.notEqual(verify(leafOf(memberId(STAGING_MEMBERS)), proofOf(0)), stagingRoot(), "an id outside the collection proves nothing");
});
