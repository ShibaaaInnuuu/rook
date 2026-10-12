import assert from "node:assert/strict";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { test } from "node:test";
import { AbiEvent, toEventSelector } from "viem";
import { EVENTS } from "../src/abis.js";
import { READ_ONLY, rpcOf } from "../src/rpc.js";

const SRC = new URL("../src/", import.meta.url).pathname, ROOT = new URL("../", import.meta.url).pathname;
const files = (dir: string): string[] => readdirSync(dir, { withFileTypes: true }).flatMap((e) => (e.isDirectory() ? (e.name === "node_modules" ? [] : files(join(dir, e.name))) : e.name.endsWith(".ts") ? [join(dir, e.name)] : []));

test("it cannot send: the one JSON-RPC function refuses every method that is not a read, before any request is made", async () => {
  let asked = 0;
  const rpc = rpcOf("http://127.0.0.1:1", (async () => { asked++; throw new Error("unreachable"); }) as never);
  for (const m of ["eth_sendRawTransaction", "eth_sendTransaction", "eth_sign", "personal_sign", "eth_signTypedData_v4", "anvil_setBalance", "evm_mine", "debug_traceCall"]) {
    await assert.rejects(rpc(m, []), /refused/);
  }
  assert.equal(asked, 0);
  assert.ok(!READ_ONLY.some((m) => /send|sign|set|mine|impersonate/i.test(m)));
  // docs/phase10.md: the transaction read the logo needs is a read, and it is allowed; nothing else was let in with it
  assert.deepEqual([...READ_ONLY].filter((m) => !m.startsWith("eth_")), []);
  assert.ok(READ_ONLY.includes("eth_getTransactionByHash"), "the launch calldata can be read");
});

test("there is no signer in this package: no account, wallet, private-key or signing import or call anywhere in src/ or scripts/", () => {
  const banned = /privateKeyToAccount|mnemonicToAccount|createWalletClient|signTransaction|signMessage|sendRawTransaction|sendTransaction|PRIVATE_KEY|from "viem\/accounts"/;
  for (const f of [...files(SRC), ...files(join(ROOT, "scripts"))]) assert.ok(!banned.test(readFileSync(f, "utf8")), `${f} mentions signing or a key`);
});

test("docs/phase4.md §4: the indexer contacts the chain RPC and nothing else — no venue, no explorer, no drand; their URLs appear only inside strings handed to the reader", () => {
  for (const f of files(SRC)) {
    const text = readFileSync(f, "utf8");
    const fetches = [...text.matchAll(/\bfetch(?:Impl)?\s*\(([^,)]*)/g)].map((m) => m[1].trim());
    // docs/phase6.md: one more, in main.ts only — the staging fork's /info, which must be built from the RPC url and so is the same host
    const info = f.endsWith("/main.ts") && text.includes('const infoUrl = new URL("/info", url);') && (text.match(/\binfoUrl\s*=/g) ?? []).length === 1;
    for (const arg of fetches) assert.ok(arg === "url" || arg === "" || (info && arg === "infoUrl"), `${f}: fetch(${arg}) — the only thing ever fetched is the configured RPC url`);
    assert.ok(!/(unisat|magiceden|okx)\.[a-z]+/i.test(text.replace(/`[^`]*`|"[^"]*"/g, "")), `${f}: a venue hostname outside a string`);
  }
  // and a venue is named in two places only: the verification hint (a path for the reader's own key), and the collections
  // catalogue (docs/v6/collections-catalog.md), which READS the marketplaces' public directories through its own `fetch`
  // dependency — GET only, spaced, keyless but for OpenSea's key read at call time — and is the one part of this package that
  // talks to anything but the RPC. Its requests are made in catalogSources.ts and nowhere else. (Wave 2, fixes-W2-site.md §3: a raffle
  // draw's published list is also read, from the URL the keeper put on chain, in wrapped/draws.ts; that and the catalogue go through
  // the E-12 guard of outbound.ts: https only, every resolved address public, redirects re-checked, one deadline, a byte cap.)
  const named = files(SRC).filter((f) => /unisat|magiceden/i.test(readFileSync(f, "utf8"))).map((f) => f.slice(SRC.length));
  assert.deepEqual(named, ["rook/catalog.ts", "rook/catalogSources.ts", "rook/catalogTypes.ts", "views.ts"]);
  const asks = files(SRC).filter((f) => /https:\/\/(turbo\.ordinalswallet|api-mainnet\.magiceden|api\.opensea|api\.coinbase)\./.test(readFileSync(f, "utf8"))).map((f) => f.slice(SRC.length));
  assert.deepEqual(asks, ["rook/catalogSources.ts"]);
  // docs/v8 (proof-of-reserves.md, redemption-draw.md): the v8 reports also read drand's public relays (GET, keyless, one round at a
  // time, in v8/drand.ts only), Ethereum's and Robinhood Chain's keyless providers through v8/jsonrpc.ts (reads only, keyed URLs
  // refused), and the canister's records from CANISTER_RECORDS_URL (v8/routes.ts). Nothing is sent to any of them but a read.
  const relays = files(SRC).filter((f) => /"https:\/\/api2\.drand\.sh"/.test(readFileSync(f, "utf8"))).map((f) => f.slice(SRC.length));
  assert.deepEqual(relays, ["v8/drand.ts"]);
});

test("the event ABIs are the contracts': every event forge compiled is declared here with the same signature (skipped when out/ is absent)", () => {
  const out = new URL("../../out/", import.meta.url).pathname;
  const pairs: [string, keyof typeof EVENTS][] = [["ListingRegistry", "registry"], ["Launcher", "launcher"], ["FeeRouter", "router"], ["AssetVault", "vault"], ["Distributor", "distributor"]];
  if (!existsSync(join(out, "AssetVault.sol", "AssetVault.json"))) return;
  for (const [name, kind] of pairs) {
    const abi = (JSON.parse(readFileSync(join(out, `${name}.sol`, `${name}.json`), "utf8")) as { abi: (AbiEvent & { type: string })[] }).abi.filter((x) => x.type === "event");
    // OpenZeppelin's own events (Ownable's) are inherited by the registry and are not ours to index
    const theirs = new Set(abi.filter((e) => !["OwnershipTransferred", "OwnershipTransferStarted"].includes(e.name)).map((e) => toEventSelector(e)));
    const ours = new Set((EVENTS[kind] as readonly AbiEvent[]).map((e) => toEventSelector(e)));
    assert.deepEqual([...ours].sort(), [...theirs].sort(), `${name}: declared events differ from the compiled contract's`);
  }
});

test("the staging chain id is the repository's one number: src/main.ts holds the only copy here, and it equals staging/chain-id.json", () => {
  const source = (JSON.parse(readFileSync(join(ROOT, "../staging/chain-id.json"), "utf8")) as { chainId: number }).chainId;
  const copies = files(SRC).flatMap((f) => [...readFileSync(f, "utf8").matchAll(/STAGING_CHAIN_ID\s*=\s*(\d+)/g)].map((m) => Number(m[1])));
  assert.deepEqual(copies, [source]);
  assert.ok(![4663, 46630].includes(source), "never Robinhood Chain, never its public testnet");
  for (const f of files(SRC)) assert.ok(!/\b46630\b/.test(readFileSync(f, "utf8")), `${f} still names 46630`);
});
