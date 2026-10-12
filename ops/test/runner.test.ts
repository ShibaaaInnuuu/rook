import assert from "node:assert/strict";
import { test } from "node:test";
import { generateKeyPairSync } from "node:crypto";
import { Ed25519KeyIdentity } from "@icp-sdk/core/identity";
import { Secp256k1KeyIdentity } from "@icp-sdk/core/identity/secp256k1";
import { operatorIdentity, IdentityError } from "../src/identity.js";
import { LAUNCH_FILE_DEFAULT, parseRunnerConfig, Runner, RunnerConfigError, SOL_COLLECTIONS_FILE_DEFAULT } from "../src/runner.js";
import { logger } from "../src/log.js";

// docs/v6/p1-services.md §2.1 (K4): the runner calls tick as the operator, one call at a time, and never shows the identity.
const secpPem = (): string => {
  const { privateKey } = generateKeyPairSync("ec", { namedCurve: "secp256k1" });
  return privateKey.export({ format: "pem", type: "sec1" }).toString();       // "EC PRIVATE KEY", as `dfx identity export` prints
};
const edPem = (): string => generateKeyPairSync("ed25519").privateKey.export({ format: "pem", type: "pkcs8" }).toString();

test("the identity: a dfx secp256k1 PEM and an Ed25519 PKCS#8 PEM load; unset is null; anything else is refused without repeating it", () => {
  const s = secpPem();
  const id = operatorIdentity({ OPERATOR_IDENTITY_PEM: s })!;
  assert.ok(id instanceof Secp256k1KeyIdentity);
  assert.equal(id.getPrincipal().toText(), Secp256k1KeyIdentity.fromPem(s).getPrincipal().toText());
  assert.ok(operatorIdentity({ OPERATOR_IDENTITY_PEM: edPem() }) instanceof Ed25519KeyIdentity);
  assert.ok(operatorIdentity({ OPERATOR_IDENTITY_PEM: s.replace(/\n/g, "\\n") }), "a secret store's escaped newlines");
  assert.equal(operatorIdentity({}), null);
  const junk = "-----BEGIN EC PRIVATE KEY-----\nTk9UQUtFWQ==\n-----END EC PRIVATE KEY-----";
  assert.throws(() => operatorIdentity({ OPERATOR_IDENTITY_PEM: junk }), (e: unknown) => e instanceof IdentityError && !String((e as Error).message).includes("Tk9UQUtFWQ"));
  assert.throws(() => operatorIdentity({ OPERATOR_IDENTITY_PEM: "hello" }), /neither an EC/);
  const rsa = generateKeyPairSync("rsa", { modulusLength: 1024 }).privateKey.export({ format: "pem", type: "pkcs8" }).toString();
  assert.throws(() => operatorIdentity({ OPERATOR_IDENTITY_PEM: rsa }), /not Ed25519/);
});

test("the runner's configuration: a canister id, https (or a local replica), a cadence of at least a minute", () => {
  // the EVM home (docs/v9 §5: ROOK_HOME=robinhood keeps the EVM-era flow; the Solana home's own fields are inert there)
  assert.deepEqual(parseRunnerConfig({ CANISTER_ID: "ryjl3-tyaaa-aaaaa-aaaba-cai", ROOK_HOME: "robinhood" }), { canisterId: "ryjl3-tyaaa-aaaaa-aaaba-cai", host: "https://icp-api.io", everyS: 600, port: null,
    relay: true, relayApi: "https://api.relay.link", eth: true, slugs: {}, ethLogsRpc: null, logsMaxBlocks: 500, pendingEveryS: 120, idleEveryS: 900,
    openseaChains: { "4663": "robinhood" }, inscriptions: [], frogProofs: false, home: "robinhood", relayQuotePath: "/quote", solRpcUrls: [], frogFloor: false, floorEveryS: 1800,
    floorScope: "canister", launchFile: LAUNCH_FILE_DEFAULT, listings: [], solNftFloor: false, solNftEveryS: 1800, solCollectionsFile: SOL_COLLECTIONS_FILE_DEFAULT,
    solCollections: [], meApi: "https://api-mainnet.magiceden.dev" });
  const p = (env: Record<string, string>) => { try { parseRunnerConfig(env); return ""; } catch (e) { return (e as RunnerConfigError).problems.join("\n"); } };
  assert.match(p({}), /CANISTER_ID is not set/);
  assert.match(p({ CANISTER_ID: "ryjl3-tyaaa-aaaaa-aaaba-cai", RUNNER_TICK_EVERY_S: "5" }), /RUNNER_TICK_EVERY_S=5/);
  assert.match(p({ CANISTER_ID: "ryjl3-tyaaa-aaaaa-aaaba-cai", IC_HOST: "http://ic.example.com" }), /must be https/);
  assert.equal(parseRunnerConfig({ CANISTER_ID: "ryjl3-tyaaa-aaaaa-aaaba-cai", IC_HOST: "http://127.0.0.1:4943" }).host, "http://127.0.0.1:4943");
  assert.equal(parseRunnerConfig({ CANISTER_ID: "ryjl3-tyaaa-aaaaa-aaaba-cai", RUNNER_RELAY: "off" }).relay, false);
  assert.match(p({ CANISTER_ID: "ryjl3-tyaaa-aaaaa-aaaba-cai", RUNNER_RELAY: "maybe" }), /RUNNER_RELAY=maybe/);
  assert.match(p({ CANISTER_ID: "ryjl3-tyaaa-aaaaa-aaaba-cai", RELAY_API_URL: "http://api.relay.link" }), /RELAY_API_URL must be https/);
});

test("ticks: lines logged one by one; Err and a rejected call are failures that turn health red after three; calls never overlap", async () => {
  const lines: string[] = [];
  let answer: () => Promise<{ Ok: string[] } | { Err: string }> = async () => ({ Ok: ["frog 0: wanted for Floor 0xabc at most 1 wei; waiting for an offer", "policy: waits: nothing owed"] });
  let now = 1_000_000;
  const r = new Runner({ tick: () => answer(), paused: async () => false }, logger((l) => lines.push(l)), "abc-principal", "ryjl3-tyaaa-aaaaa-aaaba-cai", () => now);
  const ok = await r.once();
  assert.equal(ok?.ok, true);
  assert.equal(lines.filter((l) => l.includes('"event":"tick-line"')).length, 2);
  assert.ok(r.healthy(600));
  answer = async () => ({ Err: "a tick is already running" });
  await r.once();
  answer = async () => { throw new Error("Call was rejected:\n  Reject code: 5\n  Reject text: Canister trapped: caller is not the operator"); };
  const rej = await r.once();
  assert.equal(rej?.ok, false);
  assert.match(rej!.error!, /not the operator/);
  assert.ok(r.healthy(600), "two failures: still healthy");
  await r.once();
  assert.equal(r.state.consecutiveFailures, 3);
  assert.ok(!r.healthy(600), "three in a row: red");
  // overlap: a second call while the first is running is skipped, not queued
  let release!: () => void;
  answer = () => new Promise((ok) => { release = () => ok({ Ok: [] }); });
  const first = r.once();
  assert.equal(await r.once(), null);
  release();
  assert.equal((await first)?.ok, true);
  assert.equal(r.state.consecutiveFailures, 0);
  now += 3 * 600_000 + 1;
  assert.ok(!r.healthy(600), "no tick for three intervals: red");
});

test("the principal is what is printed: no log line and no status carries the PEM", async () => {
  const pem = secpPem();
  const id = operatorIdentity({ OPERATOR_IDENTITY_PEM: pem })!;
  const lines: string[] = [];
  const r = new Runner({ tick: async () => ({ Ok: ["x"] }), paused: async () => true }, logger((l) => lines.push(l)), id.getPrincipal().toText(), "ryjl3-tyaaa-aaaaa-aaaba-cai");
  await r.readPaused(); await r.once();
  const body = pem.split("\n").filter((l) => l && !l.startsWith("-----")).join("");
  const all = [...lines, JSON.stringify(r.state)].join("\n");
  assert.ok(!all.includes(body.slice(0, 40)) && !all.includes("PRIVATE KEY"));
  assert.ok(JSON.stringify(r.state).includes(id.getPrincipal().toText()));
  assert.equal(r.state.paused, true);
});

test("docs/v8/ow-protected.md §4 (the old scalars): the protected step is off unless a slug is named, and then needs the members file; read as Bitcoin Frogs' entry", async () => {
  const { mkdtempSync, writeFileSync } = await import("node:fs");
  const { tmpdir } = await import("node:os");
  const { join } = await import("node:path");
  const base = { CANISTER_ID: "ryjl3-tyaaa-aaaaa-aaaba-cai" };
  assert.deepEqual(parseRunnerConfig(base).inscriptions, []);
  assert.throws(() => parseRunnerConfig({ ...base, RUNNER_OW_SLUG: "bitcoin-frogs" }), /members file/);
  const dir = mkdtempSync(join(tmpdir(), "ow-")), f = join(dir, "members.json");
  writeFileSync(f, JSON.stringify(["ab".repeat(32) + "i0"]));
  const c = parseRunnerConfig({ ...base, RUNNER_OW_SLUG: "bitcoin-frogs", RUNNER_OW_MEMBERS: f });
  assert.deepEqual(c.inscriptions.map((e) => [e.slug, e.owSlug, e.unisat, e.membersFile, e.members.length]), [["bitcoin-frogs", "bitcoin-frogs", null, f, 1]]);
  writeFileSync(f, JSON.stringify(["not an id"]));
  assert.throws(() => parseRunnerConfig({ ...base, RUNNER_OW_SLUG: "bitcoin-frogs", RUNNER_OW_MEMBERS: f }), /inscription ids/);
  assert.throws(() => parseRunnerConfig({ ...base, RUNNER_OW_SLUG: "Bitcoin Frogs!", RUNNER_OW_MEMBERS: f }), /slug/);
});

test("the UniSat step (the old scalar) is off unless a collection is named, and then needs UNISAT_API_KEY and the members file", async () => {
  const { mkdtempSync, writeFileSync } = await import("node:fs");
  const { tmpdir } = await import("node:os");
  const { join } = await import("node:path");
  const base = { CANISTER_ID: "ryjl3-tyaaa-aaaaa-aaaba-cai" };
  const dir = mkdtempSync(join(tmpdir(), "us-")), f = join(dir, "members.json");
  writeFileSync(f, JSON.stringify(["ab".repeat(32) + "i0"]));
  assert.throws(() => parseRunnerConfig({ ...base, RUNNER_UNISAT_COLLECTION: "bitcoin-frogs", RUNNER_OW_MEMBERS: f }), /UNISAT_API_KEY/);
  assert.throws(() => parseRunnerConfig({ ...base, RUNNER_UNISAT_COLLECTION: "bitcoin-frogs", UNISAT_API_KEY: "k" }), /members file/);
  const c = parseRunnerConfig({ ...base, RUNNER_UNISAT_COLLECTION: "bitcoin-frogs", UNISAT_API_KEY: "k", RUNNER_OW_MEMBERS: f });
  assert.deepEqual(c.inscriptions.map((e) => [e.slug, e.owSlug, e.unisat]), [["bitcoin-frogs", null, "bitcoin-frogs"]]);
});

test("Ordinals multi-collection: RUNNER_INSCRIPTIONS names each collection's Ordinals Wallet slug, members file and optional UniSat id; the old scalars are Bitcoin Frogs' entry beside it", async () => {
  const { mkdtempSync, writeFileSync } = await import("node:fs");
  const { tmpdir } = await import("node:os");
  const { join } = await import("node:path");
  const base = { CANISTER_ID: "ryjl3-tyaaa-aaaaa-aaaba-cai" };
  const dir = mkdtempSync(join(tmpdir(), "ins-")), a = join(dir, "omb.json"), b = join(dir, "frogs.json");
  writeFileSync(a, JSON.stringify(["aa".repeat(32) + "i0", "bb".repeat(32) + "i1"]));
  writeFileSync(b, JSON.stringify(["cc".repeat(32) + "i0"]));
  const c = parseRunnerConfig({ ...base, RUNNER_INSCRIPTIONS: `omb=omb:${a}, nodemonkes=:${a}:nodemonkes`, UNISAT_API_KEY: "k", RUNNER_OW_SLUG: "bitcoin-frogs", RUNNER_OW_MEMBERS: b });
  assert.deepEqual(c.inscriptions.map((e) => [e.slug, e.owSlug, e.unisat, e.members.length]),
    [["omb", "omb", null, 2], ["nodemonkes", null, "nodemonkes", 2], ["bitcoin-frogs", "bitcoin-frogs", null, 1]]);
  const p = (env: Record<string, string>) => { try { parseRunnerConfig({ ...base, ...env }); return ""; } catch (e) { return (e as RunnerConfigError).problems.join("\n"); } };
  assert.match(p({ RUNNER_INSCRIPTIONS: `omb` }), /is not <slug>=<owSlug>:<membersFile>/);
  assert.match(p({ RUNNER_INSCRIPTIONS: `omb=omb:${a}:x:y` }), /is not <slug>=/);
  assert.match(p({ RUNNER_INSCRIPTIONS: `omb=omb:${a},omb=omb:${a}` }), /named twice/);
  assert.match(p({ RUNNER_INSCRIPTIONS: `omb=:${a}` }), /neither an Ordinals Wallet slug nor a UniSat/);
  assert.match(p({ RUNNER_INSCRIPTIONS: `omb=omb:${a}:omb` }), /UNISAT_API_KEY/);
  assert.match(p({ RUNNER_INSCRIPTIONS: `omb=omb:/nonexistent.json` }), /omb's members file/);
  assert.match(p({ RUNNER_INSCRIPTIONS: `bitcoin-frogs=bitcoin-frogs:${b}`, RUNNER_OW_SLUG: "bitcoin-frogs", RUNNER_OW_MEMBERS: b }), /both name bitcoin-frogs/);
  // Robinhood Chain: OpenSea's chain name, "robinhood" unless overridden
  assert.deepEqual(parseRunnerConfig(base).openseaChains, { "4663": "robinhood" });
  assert.deepEqual(parseRunnerConfig({ ...base, OPENSEA_CHAIN_4663: "robinhood_chain" }).openseaChains, { "4663": "robinhood_chain" });
  assert.match(p({ OPENSEA_CHAIN_4663: "Robinhood Chain!" }), /OPENSEA_CHAIN_4663/);
  // the proof step: on by default on the Solana home, off on the EVM home
  assert.equal(parseRunnerConfig(base).frogProofs, true);
  assert.equal(parseRunnerConfig({ ...base, RUNNER_FROG_PROOFS: "off" }).frogProofs, false);
  assert.match(p({ RUNNER_FROG_PROOFS: "maybe" }), /RUNNER_FROG_PROOFS/);
});
