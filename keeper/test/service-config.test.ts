import assert from "node:assert/strict";
import { test } from "node:test";
import { ConfigError, DEFAULT_RPC_URLS, parseServiceConfig } from "../service/config.js";
import { signerFromEnv, SignerError } from "../service/signer.js";
import { parseStack } from "../service/stack.js";
import { stackJson } from "./service-fake.js";

// docs/v6/p1-services.md §1.2–§1.3: the production keeper refuses to start on anything missing or contradictory, and says all of it.
const base = { KEEPER_CHAIN_ID: "4663", KEEPER_STACK: "/x/stack-4663.json" };
const problems = (argv: string[], env: Record<string, string | undefined>): string[] => {
  try { parseServiceConfig(argv, env); return []; } catch (e) { assert.ok(e instanceof ConfigError); return (e as ConfigError).problems; }
};

test("no chain id, no start: KEEPER_CHAIN_ID has no default and must be a number", () => {
  assert.match(problems([], { KEEPER_STACK: "/x" }).join("\n"), /KEEPER_CHAIN_ID is not set/);
  assert.match(problems([], { ...base, KEEPER_CHAIN_ID: "robinhood" }).join("\n"), /not a chain id/);
  assert.match(problems([], { ...base, KEEPER_CHAIN_ID: "0" }).join("\n"), /not a chain id/);
  assert.equal(parseServiceConfig([], base).chainId, 4663);
});

test("dry run is the default; send needs KEEPER_MODE=send, and --dry-run always wins", () => {
  assert.equal(parseServiceConfig([], base).mode, "dry-run");
  assert.equal(parseServiceConfig([], { ...base, KEEPER_MODE: "send" }).mode, "send");
  assert.equal(parseServiceConfig(["--dry-run"], { ...base, KEEPER_MODE: "send" }).mode, "dry-run");
  assert.match(problems([], { ...base, KEEPER_MODE: "live" }).join("\n"), /KEEPER_MODE=live/);
});

test("the RPCs: Robinhood's own and ordofi by default (never robinhood.drpc.org); https only, plain http only for this machine", () => {
  const c = parseServiceConfig([], base);
  assert.deepEqual(c.rpcUrls, [...DEFAULT_RPC_URLS]);
  assert.ok(!c.rpcUrls.some((u) => u.includes("drpc")));
  assert.match(problems([], { ...base, KEEPER_RPC_URLS: "http://rpc.example.com" }).join("\n"), /not https/);
  assert.deepEqual(parseServiceConfig([], { ...base, KEEPER_RPC_URLS: "http://127.0.0.1:8651" }).rpcUrls, ["http://127.0.0.1:8651"]);
  // a URL with a key in it is named by host only
  const p = problems([], { ...base, KEEPER_RPC_URLS: "http://rpc.example.com/v1/SECRETKEY123" }).join("\n");
  assert.ok(!p.includes("SECRETKEY123"), "an RPC URL's path is never repeated");
});

test("a fixture floor is never sent to 4663; every problem is listed at once; unknown flags are refused", () => {
  const p = problems(["--now"], { KEEPER_MODE: "send", KEEPER_CHAIN_ID: "4663", KEEPER_FLOOR_SOURCE: "fixture", KEEPER_FLOOR_FIXTURE: "/f.json", KEEPER_MAX_GAS_PER_TX: "lots" });
  assert.ok(p.length >= 4, p.join("\n"));
  assert.match(p.join("\n"), /fixture floor is never SENT to Robinhood Chain/);
  assert.match(p.join("\n"), /unknown argument --now/);
  assert.match(p.join("\n"), /KEEPER_STACK is not set/);
  assert.match(p.join("\n"), /KEEPER_MAX_GAS_PER_TX=lots/);
  // on a fork (another chain id) the fixture is allowed, and in a dry run on 4663 too: nothing is sent
  assert.equal(parseServiceConfig([], { ...base, KEEPER_CHAIN_ID: "9466301", KEEPER_MODE: "send", KEEPER_FLOOR_SOURCE: "fixture", KEEPER_FLOOR_FIXTURE: "/f.json" }).floorSource, "fixture");
  assert.equal(parseServiceConfig([], { ...base, KEEPER_FLOOR_SOURCE: "fixture", KEEPER_FLOOR_FIXTURE: "/f.json" }).mode, "dry-run");
});

test("the caps: defaults, bounds, and a per-transaction cap above the day's is refused", () => {
  const c = parseServiceConfig([], base);
  assert.equal(c.caps.maxGasPerTx, 400_000n);
  assert.equal(c.caps.maxFeePerGasWei, 1_000_000_000n);
  assert.equal(c.caps.maxWeiPerTx, 5n * 10n ** 14n);
  assert.equal(c.caps.maxWeiPerDay, 10n ** 16n);
  assert.match(problems([], { ...base, KEEPER_MAX_WEI_PER_TX: String(10n ** 17n), KEEPER_MAX_WEI_PER_DAY: String(10n ** 16n) }).join("\n"), /above KEEPER_MAX_WEI_PER_DAY/);
  assert.match(problems([], { ...base, KEEPER_MAX_WEI_PER_DAY: "0" }).join("\n"), /outside/);
  assert.deepEqual(parseServiceConfig([], base).draw, { everyS: 86_400, offsetS: 54_000, windowS: 3_000 });
  assert.match(problems([], { ...base, KEEPER_DRAW_WINDOW_S: "3600" }).join("\n"), /KEEPER_DRAW_WINDOW_S/, "the window stays under the contract's hour");
});

test("the stack file: its chain id and label must be the keeper's, and every address it posts to must be there", () => {
  assert.equal(parseStack(stackJson(4663), 4663).collections[0].slug, "frogs");
  assert.throws(() => parseStack(stackJson(9466301), 4663), /for chain 9466301/);
  assert.throws(() => parseStack(JSON.stringify({ ...JSON.parse(stackJson()), label: "test" }), 4663), /label/);
  const broken = JSON.parse(stackJson());
  broken.collections[0].burnVault = "0x0000000000000000000000000000000000000000";
  assert.throws(() => parseStack(JSON.stringify(broken), 4663), /burnVault is not an address/);
  // docs/v7/phase1-nft.md §3.2: an nft collection names the canister as its floor poster (the keeper never posts it)
  const nft = JSON.parse(stackJson());
  nft.collections[0].kind = "nft";
  assert.throws(() => parseStack(JSON.stringify(nft), 4663), /names no floorPoster/);
  nft.collections[0].floorPoster = "0x00000000000000000000000000000000000000c0";
  assert.equal(parseStack(JSON.stringify(nft), 4663).collections[0].floorPoster, "0x00000000000000000000000000000000000000c0");
  assert.equal(parseStack(stackJson(4663), 4663).collections[0].floorPoster, null, "a Bitcoin collection has none");
});

test("the signer: unset is null (a dry run); a malformed key is refused WITHOUT repeating it; the scrubber cuts it in any case", async () => {
  assert.equal(signerFromEnv({}), null);
  const bad = "0x" + "zz".repeat(32);
  assert.throws(() => signerFromEnv({ KEEPER_PRIVATE_KEY: bad }), (e: unknown) => e instanceof SignerError && !String((e as Error).message).includes("zz".repeat(32)));
  const key = "0x" + "ab".repeat(32);
  const s = signerFromEnv({ KEEPER_PRIVATE_KEY: key })!;
  assert.match(s.address, /^0x[0-9a-fA-F]{40}$/);
  assert.equal(s.scrub(`a ${key} b ${key.slice(2).toUpperCase()} c`), "a 0x[redacted] b [redacted] c");
  await assert.rejects(s.sign({ type: "eip1559", chainId: 1, nonce: 0, to: s.address, gas: 21_000n, maxFeePerGas: 1n, maxPriorityFeePerGas: 0n }, 4663), /refused: a transaction for chain 1/);
  assert.ok(!Object.values(s).some((v) => typeof v === "string" && v.toLowerCase().includes("ab".repeat(32))), "the key is not a property of the signer");
});
