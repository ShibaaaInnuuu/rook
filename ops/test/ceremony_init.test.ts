import assert from "node:assert/strict";
import { test } from "node:test";
import { readFileSync } from "node:fs";
// @ts-ignore: plain ES modules, no declarations
import * as M from "../ceremony/mainnet-init.mjs";
// @ts-ignore
import * as T from "../ceremony/timelock-init.mjs";
// @ts-ignore
import { INIT_FIELDS, Init } from "../../monitor/src/candid.mjs";
// @ts-ignore
import { keccak_256 } from "../../monitor/node_modules/@noble/hashes/sha3.js";

/**
 * The ceremony's install arguments (docs/v9/ceremony-commands.md): ops/ceremony/mainnet-init.mjs (the buyer's MAINNET Init) and
 * ops/ceremony/timelock-init.mjs (the timelock's Config). No network: every live read goes through a fake fetch.
 */
const m: any = M, t: any = T;
const ROOT = new URL("../../", import.meta.url).pathname;
const EXAMPLE = JSON.parse(readFileSync(ROOT + "deployments/launch/ceremony-input.example.json", "utf8"));
const HELIUS_KEY = "0b1c2d3e-test-key-never-printed";
const OPERATOR = "ih4wp-rvfmw-iimsu-qnmdv-63mci-c6s5k-6dfao-fmrkp-3nsmu-mf5ul-hqe";
/** The example with its placeholders filled. */
const filled = (over: Record<string, unknown> = {}) => ({ ...EXAMPLE, operator: OPERATOR, nonce_seed: "rook-main-1",
  solana_rpc_urls: ["https://solana-rpc.publicnode.com", `https://mainnet.helius-rpc.com/?api-key=${HELIUS_KEY}`], ...over });

// ---- the fake chains --------------------------------------------------------------------------------------------------------
const kec = (hex: string) => "0x" + Buffer.from(keccak_256(Buffer.from(hex.slice(2), "hex"))).toString("hex");
const CODE: Record<string, string> = {
  [m.SEAPORT_1_6.toLowerCase()]: "0x6080604052aa", [m.SIGNED_ZONE.toLowerCase()]: "0x6080604052bb",
  [m.ZONE_REGISTRIES[0]]: "0x6080604052cc", [m.ZONE_REGISTRIES[1]]: "0x6080604052dd",
};
const EXPECT = { seaport: kec(CODE[m.SEAPORT_1_6.toLowerCase()]), zone: kec(CODE[m.SIGNED_ZONE.toLowerCase()]) };
type Over = { code?: (url: string, addr: string) => string; chainId?: (url: string) => string; genesis?: (url: string) => string; checkpoint?: (url: string) => string };
function fakeFetch(o: Over = {}) {
  const calls: string[] = [];
  const f = async (url: string, init?: any) => {
    calls.push(url);
    const reply = (x: unknown) => ({ ok: true, status: 200, json: async () => ({ jsonrpc: "2.0", id: 1, result: x }), text: async () => String(x) });
    if (!init) return reply((o.checkpoint ?? (() => m.BTC_CHECKPOINT.hash))(url));
    const { method, params } = JSON.parse(init.body);
    if (method === "eth_chainId") return reply((o.chainId ?? (() => "0x1"))(url));
    if (method === "getGenesisHash") return reply((o.genesis ?? (() => m.SOLANA_MAINNET_GENESIS))(url));
    if (method === "eth_getCode") return reply((o.code ?? ((_u: string, a: string) => CODE[a.toLowerCase()] ?? "0x"))(url, params[0]));
    throw new Error("unexpected " + method);
  };
  return Object.assign(f, { calls });
}
const LIVE = { seaport_codehash: EXPECT.seaport, signed_zones: [{ zone: m.SIGNED_ZONE, codehash: EXPECT.zone }],
  registries: m.ZONE_REGISTRIES.map((address: string) => ({ address, codehash: "0x" + "ab".repeat(32) })) };
const refuses = (f: () => unknown, re: RegExp) => assert.throws(f, (e: any) => re.test(e.message), String(re));

// ---- the example, end to end --------------------------------------------------------------------------------------------------

test("the committed example is refused until its placeholders are filled", () => {
  refuses(() => m.checkInput(EXAMPLE), /placeholder/);
  assert.equal(EXAMPLE.guardian, t.GUARDIANS[0], "the example names g1, the canister's pause guardian");
});

test("the filled example builds an Init that decodes back with exactly the expected fields", () => {
  const v = m.buildInit(filled(), LIVE);
  const { hex, decoded: d } = m.encodeChecked(v);
  assert.match(hex, /^4449444c/);
  assert.deepEqual(Object.keys(d).sort(), Init._fields.map((f: [string, unknown]) => f[0]).sort(), "every top-level Init field, no other");
  assert.ok(Object.keys(d).every((k) => INIT_FIELDS.includes(k)));
  const expected = {
    chain_id: "4663", vault: "0x0000000000000000000000000000000000000001", rpc_urls: ["https://rpc.mainnet.chain.robinhood.com", "https://rpc.ordofi.network"],
    btc_api_urls: ["https://mempool.space/api", "https://blockstream.info/api"], key_name: "key_1", accept_production_key_name: true,
    bridge_allowlist: [], btc_network: "mainnet", operator: [OPERATOR], wrapper: [], custody_caps: [], outflow_limits: [], holder_deposits: [],
    wrapper_from_block: [], frog_funding: [{ max_fee_bps: 400, float_first: false }],
    home: [{ Solana: { cluster: "mainnet-beta", rpc_urls: filled().solana_rpc_urls, rook_core: "books", nonce_seed: "rook-main-1",
      blockhash_fallback: [], rook_backing: [], drand_urls: [] } }],
    evm_chains: [[{ chain_id: "1", rpc_urls: ["https://rpc.mevblocker.io", "https://gateway.tenderly.co/public/mainnet"], bucket: "8",
      seaport: "0x0000000000000068F116a894984e2DB1123eB395", seaport_codehash: EXPECT.seaport, max_fee_wei: "50000000000",
      priority_fee_wei: "1000000000", gas_cap: "400000", fee_recipients: ["0x0000a26b00c1F0DF003000390027140000fAa719"], max_fee_bps: 300,
      min_order_life_s: "600", signed_zones: [[{ zone: "0x000056F7000000EcE9003ca63978907a00FFD100", codehash: EXPECT.zone }]],
      zone_registries: [["0x721c008fdff27bf06e7e123956e2fe03b63342e3", "0xa000027a9b2802e1ddf7000061001e5c005a0000"]] }]],
    eth_nft: [{ chain_id: "1", relay_max_fee_bps: 300, deposits: false, floor: [], gas_pot_target_wei: [], logs_max_blocks: [], dormancy: [],
      collections: [
        ["0xbc4ca0eda7647a8ab7c2061c2e118a18a936f13d", "8000000000000000000"], ["0x5af0d9827e0c53e4799bb226655a1de152a425a5", "2000000000000000000"],
        ["0xbd3531da5cf5857e7cfaa92426877b022e612cf8", "4000000000000000000"], ["0xed5af388653567af2f388e6224dc7c4b3241c544", "2000000000000000000"],
        ["0x60e4d786628fea6478f785a6d7e704777c86a7c6", "2000000000000000000"], ["0x8a90cab2b38dba80c64b7734e58ee1db38b8992e", "500000000000000000"],
        ["0x524cab2ec69124574082676e6f654a18df49a048", "700000000000000000"], ["0x23581767a106ae21c074b2276d25e5c3e136a68b", "1000000000000000000"],
        ["0x49cf6f5d44e70224e2e23fdcdd2c053f30ada28b", "300000000000000000"], ["0x036721e5a769cc48b3189efbb9cce4471e8a48b1", "350000000000000000"],
        ["0x0000ec93127baa929e58e97dd0095a2bfb38ec1d", "2500000000000000000"],
      ].map(([collection, max]) => ({ collection, wrapper: "books", module: "books", max_purchase_wei: max, max_premium_bps: 500,
        chain_id: [], token_bound: [], nft_value_bps: [] })) }],
  };
  assert.equal(m.canonical(d), m.canonical(expected));
});

test("the Ethereum collections are the Eth entries of the first LAUNCH_COUNT of ORDER, Robinhood Chain's never", () => {
  const { order, count } = m.launchOrder();
  assert.equal(count, 38);
  assert.equal(order.length, 41);
  const cols = m.launchEthCollections();
  assert.deepEqual(cols.map((c: any) => c.slug), ["bayc", "milady", "pudgypenguins", "azuki", "mayc", "doodles", "lilpudgys", "moonbirds", "clonex", "checks", "identitymd"]);
  assert.ok(!cols.some((c: any) => ["stonkbrokers", "chainmancers", "rekttradooors"].includes(c.slug)));
});

// ---- every refusal ------------------------------------------------------------------------------------------------------------

test("the input: unknown, missing, placeholder and test values are refused", () => {
  refuses(() => m.checkInput(filled({ key_name: "key_1" })), /unknown input field.*key_name/);
  refuses(() => m.checkInput(filled({ btc_network: "mainnet" })), /unknown input field/);
  refuses(() => m.checkInput({ ...filled(), operator: undefined }), /missing input field.*operator/);
  refuses(() => m.checkInput(filled({ operator: "2vxsx-fae" })), /anonymous/);
  refuses(() => m.checkInput(filled({ operator: "not-a-principal" })), /not a principal/);
  refuses(() => m.checkInput(filled({ operator: t.GUARDIANS[1] })), /holds a timelock role/);
  refuses(() => m.checkInput(filled({ operator: t.PROPOSERS[0] })), /holds a timelock role/);
  refuses(() => m.checkInput(filled({ guardian: OPERATOR })), /not one of the timelock's guardians/);
  // Solana: devnet / testnet, http, one provider, the same host twice
  refuses(() => m.checkInput(filled({ solana_rpc_urls: ["https://api.devnet.solana.com", "https://devnet.helius-rpc.com/?api-key=k"] })), /test network/);
  refuses(() => m.checkInput(filled({ solana_rpc_urls: ["https://api.testnet.solana.com", "https://solana-rpc.publicnode.com"] })), /test network/);
  refuses(() => m.checkInput(filled({ solana_rpc_urls: ["http://solana-rpc.publicnode.com", "https://api.mainnet-beta.solana.com"] })), /not https/);
  refuses(() => m.checkInput(filled({ solana_rpc_urls: ["https://solana-rpc.publicnode.com"] })), /two providers/);
  refuses(() => m.checkInput(filled({ solana_rpc_urls: ["https://mainnet.helius-rpc.com/?api-key=a", "https://mainnet.helius-rpc.com/?api-key=b"] })), /one provider/);
  // Ethereum: only the providers measured to serve the sale window
  refuses(() => m.checkInput(filled({ eth_rpc_urls: ["https://rpc.mevblocker.io", "https://ethereum-rpc.publicnode.com"] })), /measured/);
  refuses(() => m.checkInput(filled({ eth_rpc_urls: ["https://rpc.mevblocker.io", "https://rpc.mevblocker.io"] })), /twice/);
  // Bitcoin: signet / testnet4, or an explorer that never answered the checkpoint
  refuses(() => m.checkInput(filled({ btc_api_urls: ["https://mempool.space/signet/api", "https://blockstream.info/signet/api"] })), /test network/);
  refuses(() => m.checkInput(filled({ btc_api_urls: ["https://mempool.space/testnet4/api", "https://mempool.emzy.de/testnet4/api"] })), /test network/);
  refuses(() => m.checkInput(filled({ btc_api_urls: ["https://mempool.space/api", "https://open-api.unisat.io/v1"] })), /esplora/);
  // the nonce seed: permanent, never a rehearsal's
  refuses(() => m.checkInput(filled({ nonce_seed: "rook-devnet-0" })), /rehearsal seed/);
  refuses(() => m.checkInput(filled({ nonce_seed: "rook-w15-test" })), /rehearsal seed/);
  refuses(() => m.checkInput(filled({ nonce_seed: "a".repeat(33) })), /1-32 printable/);
  refuses(() => m.checkInput(filled({ nonce_seed: "has space" })), /1-32 printable/);
  assert.ok(m.checkInput(filled()));
});

test("the Init itself: test keys, test networks, chain 46630, devnet and placeholder hashes are refused", () => {
  const good = () => m.buildInit(filled(), LIVE);
  const with_ = (f: (v: any) => void) => { const v = good(); f(v); return () => m.checkInit(v); };
  refuses(with_((v) => { v.key_name = "test_key_1"; }), /key_name test_key_1/);
  refuses(with_((v) => { v.key_name = "dfx_test_key"; }), /key_name dfx_test_key/);
  refuses(with_((v) => { v.accept_production_key_name = false; }), /accept_production_key_name/);
  refuses(with_((v) => { v.btc_network = "signet"; }), /btc_network signet/);
  refuses(with_((v) => { v.btc_network = "testnet4"; }), /btc_network testnet4/);
  refuses(with_((v) => { v.chain_id = 46630n; }), /46630/);
  refuses(with_((v) => { v.evm_chains[0][0].chain_id = 46630n; }), /46630/);
  refuses(with_((v) => { v.home[0].Solana.cluster = "devnet"; }), /mainnet-beta/);
  refuses(with_((v) => { v.home[0].Solana.rook_core = "RooKcore1111111111111111111111111111111111"; }), /books/);
  refuses(with_((v) => { v.evm_chains[0][0].seaport_codehash = "0x" + "00".repeat(32); }), /placeholder/);
  refuses(with_((v) => { v.evm_chains[0][0].seaport_codehash = "0x" + "11".repeat(32); }), /placeholder/);
  refuses(with_((v) => { v.evm_chains[0][0].signed_zones[0][0].codehash = "0x" + "11".repeat(32); }), /placeholder/);
  refuses(with_((v) => { v.evm_chains[0].push({ ...v.evm_chains[0][0], chain_id: 4663n }); }), /\[Ethereum \(1\)\] alone/);
  refuses(with_((v) => { v.custody_caps = [{ collections: [], global_max_items: 0, global_max_value_wei: 0n }]; }), /custody_caps/);
  refuses(with_((v) => { v.eth_nft[0].collections[0].wrapper = "0x5af0d9827e0c53e4799bb226655a1de152a425a5"; }), /books/);
  refuses(() => m.buildInit(filled(), { ...LIVE, seaport_codehash: undefined }), /no live Seaport code hash/);
  assert.ok(m.checkInit(good()));
});

test("--local-replica-dry-run swaps ONLY key_name to dfx_test_key, and is refused without the flag", () => {
  const prod = m.buildInit(filled(), LIVE), dry = m.buildInit(filled(), LIVE, { localReplicaDryRun: true });
  assert.equal(prod.key_name, "key_1");
  assert.equal(dry.key_name, "dfx_test_key");
  assert.equal(m.canonical(m.encodeChecked({ ...dry, key_name: "key_1" }).decoded), m.canonical(m.encodeChecked(prod).decoded), "nothing else differs");
  refuses(() => m.checkInit(dry), /key_name dfx_test_key/);
  refuses(() => m.checkInit(prod, { localReplicaDryRun: true }), /key_name key_1/);
  assert.match(m.summarize(m.encodeChecked(dry).decoded, { localReplicaDryRun: true }), /dfx_test_key.*NEVER install this on the IC/);
});

// ---- the live code hash ---------------------------------------------------------------------------------------------------------

test("codehashOf: both providers agree → the keccak; they disagree, or the code is empty → refused; never a placeholder", async () => {
  const urls = ["https://rpc.mevblocker.io", "https://gateway.tenderly.co/public/mainnet"];
  const ok = fakeFetch();
  assert.equal(await m.codehashOf(urls, m.SEAPORT_1_6, { fetch: ok }), EXPECT.seaport);
  assert.equal(ok.calls.length, 2, "every provider is asked");
  await assert.rejects(m.codehashOf(urls, m.SEAPORT_1_6, { fetch: fakeFetch({ code: (u, a) => (u.includes("tenderly") ? "0x6080604052ff" : CODE[a.toLowerCase()]) }) }), /disagree/);
  await assert.rejects(m.codehashOf(urls, m.SEAPORT_1_6, { fetch: fakeFetch({ code: () => "0x" }) }), /no code/);
  await assert.rejects(m.codehashOf(urls, m.SEAPORT_1_6, { fetch: fakeFetch({ code: () => null as any }) }), /answered no code/);
  await assert.rejects(m.codehashOf(urls, m.SEAPORT_1_6, { fetch: ok, expected: m.SEAPORT_CODEHASH_ETHEREUM }), /not the one the docs recorded/);
  await assert.rejects(m.codehashOf(urls, m.SEAPORT_1_6, { fetch: async () => ({ ok: false, status: 429 }) }), /HTTP 429/);
});

test("liveChain: Ethereum chain id, Solana genesis, the Bitcoin checkpoint and both code hashes are read live and must be right", async () => {
  const i = m.checkInput(filled());
  const live = await m.liveChain(i, { fetch: fakeFetch(), expect: EXPECT });
  assert.equal(live.seaport_codehash, EXPECT.seaport);
  assert.deepEqual(live.signed_zones, [{ zone: m.SIGNED_ZONE, codehash: EXPECT.zone }]);
  await assert.rejects(m.liveChain(i, { fetch: fakeFetch() }), /not the one the docs recorded/, "the documented hashes are the default expectation");
  await assert.rejects(m.liveChain(i, { fetch: fakeFetch({ chainId: (u) => (u.includes("tenderly") ? "0xb626" : "0x1") }), expect: EXPECT }), /not Ethereum mainnet/);
  await assert.rejects(m.liveChain(i, { fetch: fakeFetch({ genesis: () => "EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG" }), expect: EXPECT }), /not mainnet-beta/);
  await assert.rejects(m.liveChain(i, { fetch: fakeFetch({ checkpoint: () => "00".repeat(32) }), expect: EXPECT }), /checkpoint/);
  await assert.rejects(m.liveChain(i, { fetch: fakeFetch({ code: (_u, a) => (a === m.ZONE_REGISTRIES[1] ? "0x" : CODE[a.toLowerCase()]) }), expect: EXPECT }), /has no code/);
});

// ---- the summary never prints a key -----------------------------------------------------------------------------------------------

test("redactUrl hides query strings, fragments and userinfo", () => {
  assert.equal(m.redactUrl(`https://mainnet.helius-rpc.com/?api-key=${HELIUS_KEY}`), "https://mainnet.helius-rpc.com?<redacted>");
  assert.equal(m.redactUrl("https://user:pw@host.example/path?k=v#f"), "https://<redacted>@host.example/path?<redacted>#<redacted>");
  assert.equal(m.redactUrl("https://gateway.tenderly.co/public/mainnet"), "https://gateway.tenderly.co/public/mainnet");
});

test("main: the hex on stdout, the summary on stderr with the Helius key redacted (and an error never prints it either)", async () => {
  const { mkdtempSync, writeFileSync } = await import("node:fs");
  const { tmpdir } = await import("node:os");
  const dir = mkdtempSync(tmpdir() + "/ceremony-test-");
  writeFileSync(dir + "/in.json", JSON.stringify(filled()));
  let out = "", err = "";
  const { decoded } = await m.main([dir + "/in.json", "--bin", dir + "/a.bin"], { fetch: fakeFetch(), expect: EXPECT,
    out: { write: (s: string) => (out += s) }, err: { write: (s: string) => (err += s) } });
  assert.ok(Buffer.from(out.trim(), "hex").toString("latin1").includes(HELIUS_KEY), "the install argument carries the key (O-13)");
  assert.ok(!err.includes(HELIUS_KEY) && !err.includes("api-key"), "the summary never prints it");
  assert.match(err, /mainnet\.helius-rpc\.com\?<redacted>/);
  assert.match(err, /restricted listings are SKIPPED/);
  assert.equal(readFileSync(dir + "/a.bin").toString("hex"), out.trim());
  assert.equal(decoded.key_name, "key_1");
  await assert.rejects(m.main([dir + "/in.json"], { fetch: fakeFetch({ genesis: () => "x" }), expect: EXPECT, out: { write() {} }, err: { write() {} } }),
    (e: any) => !e.message.includes(HELIUS_KEY));
  await assert.rejects(m.main([dir + "/in.json", "--key", "test_key_1"], { fetch: fakeFetch(), expect: EXPECT }), /unknown flag --key/);
});

// ---- the timelock: rules.rs, ported ----------------------------------------------------------------------------------------------

/** rules.rs tests' p(n): Principal::from_slice(&[n; 10]). */
// @ts-ignore
const { Principal } = await import("../../monitor/node_modules/@icp-sdk/core/lib/esm/principal/index.js");
const p = (n: number) => Principal.fromUint8Array(new Uint8Array(10).fill(n));
const roles = () => ({ proposers: [p(20), p(21)], guardians: [p(30), p(31), p(32)], veto_threshold: 2 });
const cfg = () => ({ target: p(1), roles: roles(), delay_secs: t.MIN_DELAY_SECS });

test("timelock: it_is_installed_only_with_a_delay_at_the_floor_and_sound_roles (rules.rs, production build)", () => {
  assert.ok(t.validateConfig(cfg()));
  refuses(() => t.validateConfig({ ...cfg(), delay_secs: t.MIN_DELAY_SECS - 1n }), /below this build's floor/);
  refuses(() => t.validateConfig({ ...cfg(), target: Principal.anonymous() }), /anonymous/);
  const bad = (r: any) => () => t.validateConfig({ ...cfg(), roles: r });
  refuses(bad({ ...roles(), proposers: [Principal.anonymous(), p(21)] }), /anonymous/);
  refuses(bad({ ...roles(), proposers: [] }), /at least one proposer/);
  refuses(bad({ ...roles(), guardians: [p(30), p(30), p(31)] }), /listed twice/);
  refuses(bad({ ...roles(), veto_threshold: 4 }), /veto threshold of 4/);
  refuses(bad({ ...roles(), proposers: Array(6).fill(p(1)) }), /at most 5/);
  assert.equal(t.MIN_DELAY_SECS, 72n * 3600n, "a production build waits three days");
  refuses(bad({ ...roles(), proposers: [p(20)] }), /name two at least/);
  refuses(bad({ ...roles(), guardians: [p(30), p(31)], veto_threshold: 1 }), /fewer than three guardians/);
  refuses(bad({ ...roles(), veto_threshold: 1 }), /at least 2/);
  refuses(bad({ ...roles(), veto_threshold: 3 }), /at most 2/);
  refuses(bad({ ...roles(), guardians: [p(20), p(31), p(32)] }), /both a proposer and a guardian/);
  // the rehearsal build's relaxations exist in rules.rs only behind its feature; ported for completeness, never used by the ceremony
  assert.ok(t.validateConfig({ ...cfg(), roles: { proposers: [p(20)], guardians: [p(30)], veto_threshold: 1 }, delay_secs: 10n }, { rehearsal: true }));
});

test("timelock: the ceremony's Config is p1 p2 / g1 g2 g3 / 2 / 72 h on the buyer, decodes back, and refuses a non-canister target", () => {
  const buyer = "rrkah-fqaaa-aaaaa-aaaaq-cai";
  const { hex, decoded } = t.encodeConfig(t.timelockConfig(buyer));
  assert.match(hex, /^4449444c/);
  assert.equal(decoded.target.toText(), buyer);
  assert.deepEqual(decoded.roles.proposers.map((x: any) => x.toText()), t.PROPOSERS);
  assert.deepEqual(decoded.roles.guardians.map((x: any) => x.toText()), t.GUARDIANS);
  assert.equal(decoded.roles.veto_threshold, 2);
  assert.equal(decoded.delay_secs, 259200n);
  assert.equal(t.GUARDIANS[0], "4423w-dirhd-6mmzf-zrn3b-ukl6j-3p4mb-xhljt-gu67m-zojhm-yqdwk-fae", "g1, the canister's pause guardian");
  assert.equal(new Set([...t.PROPOSERS, ...t.GUARDIANS]).size, 5, "five distinct keys");
  refuses(() => t.timelockConfig(OPERATOR), /not a canister id/);
  refuses(() => t.timelockConfig("2vxsx-fae"), /not a canister id/);
  refuses(() => t.timelockConfig("<buyer id>"), /placeholder/);
  refuses(() => t.timelockConfig("aaaaa-aa"), /not a canister id/);
  assert.match(t.candidText(decoded), /veto_threshold = 2 : nat8 }; delay_secs = 259200 : nat64/);
});
