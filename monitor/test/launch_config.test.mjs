// Go-live prep (docs/v7/golive-runbook.md §4): the committed launch configuration, deployments/launch/monitor-nft.json, loaded through the
// monitor's OWN loader (loadConfig: the builds, launch and stack files, the draw exclusions, ${ENV} expansion, validate). Only what exists
// once the ceremony has run is left as <...>; the only ${...} is a secret; the collections are the launch file's, with both D8 references.
import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, rmSync, writeFileSync, copyFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { loadConfig, redact } from "../src/config.mjs";
import { BURN_DEFAULTS } from "../src/watcher.mjs";

const FILE = new URL("../../deployments/launch/monitor-nft.json", import.meta.url);
const LAUNCH = new URL("../../deployments/launch/nft-pilot.json", import.meta.url);
const raw = readFileSync(FILE, "utf8");
const F = JSON.parse(readFileSync(new URL("../fixtures/canister.json", import.meta.url), "utf8"));
/** What only the ceremony and the deploy produce: the stand-in each is replaced by here (shape only; nothing real). */
const DEPLOY_TIME = {
  "<timelock canister id>": "rrkah-fqaaa-aaaaa-aaaaq-cai",
  "<buyer canister id>": "ryjl3-tyaaa-aaaaa-aaaba-cai",
  "<p1>": "p1", "<p2>": "p2", "<g1>": "g1", "<g2>": "g2", "<g3>": "g3",
  "<the pre-handover reinstall's argument, as Candid hex (4449444c...), from the ceremony log>": F.args.install,
  "<the guardian EOA (registry.guardian())>": "0x" + "9a".repeat(20),
  "<the canister's EVM address>": "0x" + "c0".repeat(20),
  "<runner host>": "runner.example", "<reconciler host>": "reconciler.example", "<indexer host>": "indexer.example",
  "<keeper host>": "keeper.example", "<resolver host>": "resolver.example",
  "<fueler host>": "fueler.example", "<the fueler's address (treasury.ops())>": "0x" + "fe".repeat(20),
  // ordinals at launch (nft-pilot.json `bitcoin`): the Bitcoin canary's three addresses, the canister's after the install
  "<the canister's custody_address()>": "bc1p" + "q".repeat(58), "<the canister's float_address()>": "bc1p" + "p".repeat(58),
  "<the canister's buyer_address()>": "bc1q" + "z".repeat(38),
};
// docs/v7/dormant-collections.md §1: the allowlisted collections (the pilot five, then Doodles, Moonbirds, CloneX, Lil Pudgys, Checks, and
// since 2026-10-09 Identity MD)
const SLUGS = ["milady", "azuki", "mayc", "pudgypenguins", "bayc", "doodles", "moonbirds", "clonex", "lilpudgys", "checks", "identitymd"];
for (const [i, s] of SLUGS.entries()) {
  DEPLOY_TIME[`<${s} wrapper>`] = "0x" + (0xa0 + i).toString(16).repeat(20);
  DEPLOY_TIME[`<${s} floorModule>`] = "0x" + (0xb0 + i).toString(16).repeat(20);
}
/** Every string of the configuration outside its "//" notes. */
const values = (v, out = []) => {
  if (typeof v === "string") out.push(v);
  else if (Array.isArray(v)) v.forEach((x) => values(x, out));
  else if (v && typeof v === "object") for (const [k, x] of Object.entries(v)) if (!k.startsWith("//")) values(x, out);
  return out;
};
const placeholders = [...new Set(values(JSON.parse(raw)).flatMap((v) => v.match(/<[^<>"]+>/g) ?? []))];

test("the launch config: only deploy-time values are left as <...>, and the only ${...} is the Dwellir key (a secret)", () => {
  assert.deepEqual(placeholders.filter((p) => !(p in DEPLOY_TIME)), [], "a <...> that is neither a ceremony nor a deploy output: fill it");
  assert.deepEqual(Object.keys(DEPLOY_TIME).filter((p) => !placeholders.includes(p)), [], "a stand-in for a placeholder the file no longer has");
  assert.deepEqual([...new Set(raw.match(/\$\{[A-Z0-9_]+\}/g))], ["${DWELLIR_API_KEY}"]);
  assert.ok(!/api[_-]?key"\s*:|private|secret"\s*:|PRIVATE KEY/i.test(raw.replace(/"\/\/[^"]*"\s*:\s*"[^"]*"/g, "")), "no secret value in the file");
});

test("the launch config loads through the monitor's loader once the ceremony's values are in, the key expanded and scrubbed", () => {
  const dir = mkdtempSync(join(tmpdir(), "rook-moncfg-"));
  try {
    let filled = raw;
    for (const [k, v] of Object.entries(DEPLOY_TIME)) filled = filled.split(k).join(v);
    writeFileSync(join(dir, "monitor-nft.json"), filled);
    copyFileSync(LAUNCH, join(dir, "nft-pilot.json"));
    // the deploy's outputs (deployments/canister-ic.json, deployments/stack-4663.json): stand-ins of the shape the loader reads
    const cfgJson = JSON.parse(filled);
    cfgJson.builds_file = "canister-ic.json";
    cfgJson.evm.stack_file = "stack-4663.json";
    writeFileSync(join(dir, "monitor-nft.json"), JSON.stringify(cfgJson));
    writeFileSync(join(dir, "canister-ic.json"), JSON.stringify({ builds: [{ commit: "0".repeat(7), buyer: "a".repeat(64), timelock: "b".repeat(64) }] }));
    writeFileSync(join(dir, "stack-4663.json"), JSON.stringify({ chainId: 4663, registry: "0x" + "5e".repeat(20), fromBlock: 1, collections: {} }));
    const was = process.env.DWELLIR_API_KEY;
    process.env.DWELLIR_API_KEY = "stand-in-key-0000";
    let cfg;
    try { cfg = loadConfig({ path: join(dir, "monitor-nft.json") }); } finally { if (was === undefined) delete process.env.DWELLIR_API_KEY; else process.env.DWELLIR_API_KEY = was; }
    assert.equal(cfg.eth_nft.eth_rpc[0], "https://api-ethereum-mainnet.n.dwellir.com/stand-in-key-0000");
    assert.equal(redact(cfg.eth_nft.eth_rpc[0]), "https://api-ethereum-mainnet.n.dwellir.com/<redacted>");
    assert.equal(cfg.launch.stage, 1);
    assert.deepEqual(cfg.cycles, { ...BURN_DEFAULTS }, "the burn-rate page's thresholds are the runbook's");
    assert.equal(cfg.evm.guardian.pause_on_no_good_post_h, 6);
    // ordinals at launch: the Bitcoin canary on custody, float and buyer, two esplora providers, the two operator seeds expected
    assert.deepEqual(Object.keys(cfg.bitcoin.addresses), ["custody", "float", "buyer"]);
    assert.equal(cfg.bitcoin.providers.length, 2);
    assert.deepEqual(cfg.bitcoin.expect_inbound.map((e) => e.address), ["custody", "float"]);
    assert.equal(cfg.min_fee_pool_sats, JSON.parse(readFileSync(LAUNCH, "utf8")).bitcoin.custodyFeePoolMinSats);
    assert.deepEqual(cfg.liveness.map((l) => l.name), ["runner", "reconciler", "indexer", "keeper", "resolver", "fueler"]);
    // the collections are the launch file's, in its order, each with both D8 references, or OpenSea alone exactly where the launch file
    // decides priceRefs ["opensea"] (Identity MD, the operator 2026-10-09) (golive-check --monitor-config holds them again)
    const L = JSON.parse(readFileSync(LAUNCH, "utf8")).collections;
    assert.deepEqual(cfg.eth_nft.collections.map((c) => [c.slug, c.collection.toLowerCase(), c.opensea_slug, c.coingecko_id, c.price_refs]),
      L.map((c) => [c.slug, c.collection.toLowerCase(), c.openseaSlug, c.coingeckoId, c.priceRefs]));
    assert.deepEqual(L.filter((c) => c.priceRefs).map((c) => c.slug), ["identitymd"], "OpenSea alone for Identity MD only");
  } finally { rmSync(dir, { recursive: true, force: true }); }
});

test("the launch config as committed is REFUSED (a template is not a configuration): its canister ids are placeholders", () => {
  const dir = mkdtempSync(join(tmpdir(), "rook-moncfg-"));
  try {
    const c = JSON.parse(raw);
    delete c.builds_file; delete c.launch_file; delete c.evm.stack_file;
    c.builds = [{ commit: "0000000", buyer: "a".repeat(64), timelock: "b".repeat(64) }];
    writeFileSync(join(dir, "m.json"), JSON.stringify(c));
    const was = process.env.DWELLIR_API_KEY;
    process.env.DWELLIR_API_KEY = "stand-in-key-0000";
    try { assert.throws(() => loadConfig({ path: join(dir, "m.json") }), /timelock: a canister id is required[\s\S]*collection/); }
    finally { if (was === undefined) delete process.env.DWELLIR_API_KEY; else process.env.DWELLIR_API_KEY = was; }
  } finally { rmSync(dir, { recursive: true, force: true }); }
});
