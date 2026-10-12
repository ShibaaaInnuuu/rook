// The buyer canister's MAINNET install argument (the Candid `Init` of autobuyer/canister-rs, monitor/src/candid.mjs `Init`) for the
// launch ceremony (docs/v9/ceremony-canister-only.md §1, docs/v9/ceremony-commands.md). The rehearsal generators
// (ops/rehearsal/sol-devnet/canister-init.mjs, ops/rehearsal/ic-outcall/buyer-init.mjs) refuse mainnet and fill test values; this one
// is their production counterpart and refuses every test value they use.
//
//   node ops/ceremony/mainnet-init.mjs <ceremony-input.json> [--bin <out.bin>] [--local-replica-dry-run]
//
// The input (deployments/launch/ceremony-input.example.json) names ONLY what the operator decides: the operator's principal, the
// guardian, the Solana / Ethereum / Bitcoin providers and the nonce seed. Everything else is derived here, each value with the doc or
// code it comes from (`SOURCES`, printed in the summary). The Seaport 1.6 and SignedZone code hashes are READ LIVE from both Ethereum
// providers (eth_getCode, keccak256), must agree with each other AND with the hash the docs recorded, or nothing is printed.
// The Solana providers must answer mainnet-beta's genesis hash, the Ethereum ones chain id 1, the Bitcoin ones the mainnet checkpoint.
//
// stdout: the Candid hex (this is the install argument; it holds the Solana provider URLs, a Helius key included: the argument is
// public once installed, and the timelock publishes every later one, docs/v9/ceremony-canister-only.md O-13). stderr: the argument
// decoded back and summarised, URL query strings redacted. `--bin` also writes the raw Candid bytes (autobuyer/scripts/chunked-install.sh
// takes them). `--local-replica-dry-run` swaps ONLY key_name to `dfx_test_key` (a local replica has no `key_1`); it says so, loudly.
//
// The pure core (`buildInit`, `checkInput`, `liveChain`, `codehashOf`, `summarize`, `redactUrl`, `encodeChecked`) is exported for
// ops/test/ceremony_init.test.ts.
import { readFileSync, writeFileSync } from "node:fs";
import { pathToFileURL } from "node:url";

const ROOT = new URL("../../", import.meta.url).pathname;
const { Init, decodeInit, plain } = await import(ROOT + "monitor/src/candid.mjs");
// the monitor's own copy of the SDK: the same IDL and Principal classes candid.mjs builds its types with
const { IDL } = await import(ROOT + "monitor/node_modules/@icp-sdk/core/lib/esm/candid/index.js");
const { Principal } = await import(ROOT + "monitor/node_modules/@icp-sdk/core/lib/esm/principal/index.js");
const { keccak_256 } = await import(ROOT + "monitor/node_modules/@noble/hashes/sha3.js");
const { GUARDIANS, PROPOSERS } = await import("./timelock-init.mjs");

// ---- derived constants, each with its source ------------------------------------------------------------------------------------

/** Seaport 1.6, the only Seaport the canister accepts (autobuyer/canister-rs/src/seaport.rs SEAPORT_1_6). */
export const SEAPORT_1_6 = "0x0000000000000068F116a894984e2DB1123eB395";
/** keccak256 of Seaport 1.6's runtime code ON ETHEREUM, as docs/v4/eth-nft-canister.md §1 recorded it (it differs per chain). */
export const SEAPORT_CODEHASH_ETHEREUM = "0x74499ac0cce14428e4b41541d5e44f28f5a6882a1051d0118867c2a93cd5aec0";
/** OpenSea's SignedZone, the same address on every chain (docs/v4/eth-nft-canister.md §12.1). */
export const SIGNED_ZONE = "0x000056F7000000EcE9003ca63978907a00FFD100";
/** Its runtime code's keccak ON ETHEREUM (docs/v4/eth-nft-canister.md §12.1: 6,967 bytes, not a proxy). */
export const SIGNED_ZONE_CODEHASH_ETHEREUM = "0x9e6930be6759e905574791278e46db4d510c072733fe62a0c7d4e6e76f13e324";
/** The two transfer validators seen in real zone extraData (Limit Break's), docs/v4/eth-nft-canister.md §12.7. Each must have code on
 *  Ethereum (read live) or the tool refuses. */
export const ZONE_REGISTRIES = ["0x721c008fdff27bf06e7e123956e2fe03b63342e3", "0xa000027a9b2802e1ddf7000061001e5c005a0000"];
/** OpenSea's fee recipient: its 1 % is REQUIRED on every launch collection (docs/v7/phase1-nft.md §1.1). Without it in
 *  fee_recipients every OpenSea listing is refused ("unknown recipient", docs/v7/dormant-collections.md §1). */
export const OPENSEA_FEE_RECIPIENT = "0x0000a26b00c1F0DF003000390027140000fAa719";
/** The EVM home fields the Init still requires and the Solana home never reads (canister.rs `Init`: "nothing reads it yet but the
 *  Solana layer"): chain 4663 = PRODUCTION_EVM_CHAIN (canister.rs), which every rehearsal guard refuses (`rehearse_*` sign only for
 *  46630; `open_purchase` refuses on 4663), with the two keyless providers docs/deploy/canister-mainnet.md (phase1-scope C8) names. */
export const EVM_HOME_CHAIN = 4663n;
export const EVM_HOME_RPC = ["https://rpc.mainnet.chain.robinhood.com", "https://rpc.ordofi.network"];
/** No AssetVault exists (the v2 vault buyer is not part of the books home). The placeholder docs/deploy/canister-mainnet.md ("The vault
 *  argument", step 1) names; `CanisterIo::new` parses it, so it must be an address. Identity: an upgrade keeps it. */
export const VAULT_PLACEHOLDER = "0x0000000000000000000000000000000000000001";
export const SOLANA_MAINNET_GENESIS = "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d"; // solrpc.rs genesis_of("mainnet-beta")
export const BTC_CHECKPOINT = { height: 900000, hash: "000000000000000000010538edbfd2d5b809a33dd83f284aeea41c6d0d96968a" }; // btcsend.rs, canister-mainnet.md C7
/** Ethereum providers MEASURED to serve the 7-day sale window (script/golive-check.sh DEEP, docs/v7/phase1-nft.md §4.1 B6-M-1). */
export const ETH_MEASURED = { "rpc.mevblocker.io": ["", "/fast"], "gateway.tenderly.co": ["/public/mainnet"] };
/** Esplora explorers of Bitcoin mainnet that answered the C7 checkpoint (docs/deploy/canister-mainnet.md "The published C7 check"). */
export const BTC_ESPLORA = ["https://mempool.space/api", "https://blockstream.info/api", "https://btcscan.org/api"];

/** The Ethereum chain's fee caps and order rules (EvmChain), with where each value comes from. */
export const ETH_CHAIN_RULES = {
  bucket: [8n, "docs/v7/phase1-nft.md §5 (nftnet's pin, bucket 8: 1-2 buckets behind the head)"],
  max_fee_wei: [50_000_000_000n, "docs/v7/phase1-nft.md §7 ('evm_chains[1].max_fee_wei 50 gwei'), §9 (measured max base fee 1.5 gwei)"],
  priority_fee_wei: [1_000_000_000n, "scripts/golive-dryrun.sh (the EVM-era go-live dry run's evm_chains[1]: 1 gwei; measured tip median 0.036 gwei, phase1-nft.md §9)"],
  gas_cap: [400_000n, "docs/v7/phase1-nft.md §7 ('gas cap 400k')"],
  max_fee_bps: [300, "scripts/golive-dryrun.sh (EVM-era: 300); docs/v5/custody-threat-model.md T11 (per-chain max_fee_bps <= 1,000). OpenSea's 1 % fits; a listing paying more than 3 % in fees+royalties is refused"],
  min_order_life_s: [600n, "scripts/golive-dryrun.sh (EVM-era: 600 s); docs/v7/phase1-nft.md §5 (a signed buy is found ~415-520 s after signing)"],
};

/** Every derived Init field and its source (printed in the summary). */
export const SOURCES = {
  key_name: "\"key_1\" with accept_production_key_name = true: canister.rs PRODUCTION_KEY, docs/deploy/canister-mainnet.md",
  btc_network: "\"mainnet\": canister.rs BITCOIN_MAINNET / KNOWN_NETWORKS (spelled exactly so)",
  chain_id: "4663, PRODUCTION_EVM_CHAIN (canister.rs): required by the Init, never read on the Solana home; every rehearse_* guard refuses it",
  vault: "placeholder 0x…01 (docs/deploy/canister-mainnet.md 'The vault argument'): no vault on the books home",
  rpc_urls: "Robinhood Chain's keyless pair (docs/deploy/canister-mainnet.md, phase1-scope C8): required (2 distinct), unused on the Solana home",
  bridge_allowlist: "empty: no v2 bridge on the books home (both rehearsal generators)",
  wrapper: "null: unused on the books home; the Inscriptions listing in the books is the frog collection (ceremony-canister-only.md §1)",
  evm_chains: "[Ethereum (1)] only: Robinhood Chain 4663 is added after the ceremony by an upgrade (ceremony-canister-only.md §1-§2, audit C4-03)",
  custody_caps: "null: stage-0 defaults name every listing (ceremony-canister-only.md §1 'Custody caps'; canister.rs caps_of)",
  outflow_limits: "null: stage 0 (outflow.rs Limits::stage0: 3/collection, 6 in all a day), the limits the default caps are sized against (canister.rs caps_of, A4-L-1)",
  holder_deposits: "null (off): refused on the Solana home (canister.rs validate_frog_home, solana-migration.md §2.7)",
  wrapper_from_block: "null: there is no 4663 wrapper to scan",
  frog_funding: "Relay at 400 bps, no operator float: deployments/launch/nft-pilot.json bitcoin.frog_funding (ordinals-collections.md §485, golive-check)",
  "eth_nft.chain_id": "1 (Ethereum): docs/v7/phase1-nft.md §4.1",
  "eth_nft.collections": "the Ethereum collections among the first LAUNCH_COUNT of coins::unlock::ORDER, from deployments/launch/nft-pilot.json (collection, maxPurchaseWei; max_premium_bps 500 per its _canister note); wrapper = module = \"books\" (D-l); chain_id/token_bound/nft_value_bps null (Ethereum, plain ERC-721s)",
  "eth_nft.relay_max_fee_bps": "300: docs/v7/phase1-nft.md §4.1 default, held by script/golive-check.sh ('relay=300')",
  "eth_nft.deposits": "false: refused on the Solana home (ethnft.rs validate_home)",
  "eth_nft.floor": "null = the default floor rule (ethnft.rs FloorParams::default; D7 in docs/deploy/canister-mainnet.md)",
  "eth_nft.gas_pot_target_wei": "null = 0.05 ETH (ethnft.rs GAS_POT_TARGET_WEI)",
  "eth_nft.logs_max_blocks": "null = 500 (ethnft.rs SALE_SCAN_CHUNK_BLOCKS), within both measured providers' result caps",
  "eth_nft.dormancy": "null: every collection active (no wake desk on the books home, O-11; the intent rehearsal ran so; ceremony §0 '8 T floor until each Ethereum collection has posted once')",
  "home.cluster": "\"mainnet-beta\": canister.rs SOLANA_CLUSTERS",
  "home.rook_core": "\"books\": decision D-l, canister.rs validate_home",
  "home.rook_backing": "null: retired (D-l)",
  "home.blockhash_fallback": "null: the durable nonce",
  "home.drand_urls": "null: the canister's default relays (D-i)",
  seaport_codehash: "READ LIVE: keccak256(eth_getCode(Seaport 1.6)) on both Ethereum providers, equal to docs/v4/eth-nft-canister.md §1's",
  signed_zones: "OpenSea SignedZone (eth-nft-canister.md §12.1), its code hash READ LIVE on both providers, equal to §12.1's",
  zone_registries: "Limit Break's two transfer validators (eth-nft-canister.md §12.7), code present on Ethereum (read live)",
  fee_recipients: "OpenSea's fee recipient (phase1-nft.md §1.1: its 1 % is required on every launch collection)",
};

const PLACEHOLDER = /[<>]|REPLACE|TODO|XXX|your[-_ ]?key/i;
const TEST_WORDS = /devnet|testnet|signet|testnet4|rehears|\btest\b|test_key|dfx_test|w15|throwaway/i;
const INPUT_FIELDS = ["operator", "guardian", "solana_rpc_urls", "nonce_seed", "eth_rpc_urls", "btc_api_urls"];

// ---- helpers ------------------------------------------------------------------------------------------------------------------

/** A URL with its query string, fragment and userinfo hidden: a Helius key sits in the query and must never reach a log. */
export function redactUrl(u) {
  try {
    const p = new URL(u);
    const auth = p.username || p.password ? "<redacted>@" : "";
    return `${p.protocol}//${auth}${p.host}${p.pathname === "/" ? "" : p.pathname}${p.search ? "?<redacted>" : ""}${p.hash ? "#<redacted>" : ""}`;
  } catch {
    return "<an unparseable URL, redacted>";
  }
}

function principalOf(text, what) {
  if (typeof text !== "string" || PLACEHOLDER.test(text)) throw new Error(`refused: ${what} is a placeholder (${JSON.stringify(text)}); fill it in`);
  let p;
  try { p = Principal.fromText(text.trim()); } catch (e) { throw new Error(`refused: ${what} ${JSON.stringify(text)} is not a principal: ${e.message}`); }
  if (p.isAnonymous()) throw new Error(`refused: ${what} is the anonymous principal`);
  if (p.toText() === Principal.managementCanister().toText()) throw new Error(`refused: ${what} is the management canister`);
  return p;
}

function httpsUrls(list, what) {
  if (!Array.isArray(list) || list.length < 2) throw new Error(`refused: ${what}: two providers at least (one is not a check)`);
  const parsed = list.map((u) => {
    if (typeof u !== "string" || PLACEHOLDER.test(u)) throw new Error(`refused: ${what}: ${redactUrl(String(u))} is a placeholder; fill it in`);
    let p;
    try { p = new URL(u); } catch { throw new Error(`refused: ${what}: ${redactUrl(u)} does not parse`); }
    if (p.protocol !== "https:") throw new Error(`refused: ${what}: ${redactUrl(u)} is not https`);
    if (u !== u.trim() || /\s/.test(u)) throw new Error(`refused: ${what}: ${redactUrl(u)} has whitespace`);
    if (TEST_WORDS.test(p.host + p.pathname)) throw new Error(`refused: ${what}: ${redactUrl(u)} names a test network; this is MAINNET`);
    return p;
  });
  if (new Set(list).size !== list.length) throw new Error(`refused: ${what}: the same URL twice agrees with itself`);
  if (new Set(parsed.map((p) => p.hostname)).size !== parsed.length) throw new Error(`refused: ${what}: two URLs of one host are one provider; name distinct providers`);
  return list;
}

// ---- the input --------------------------------------------------------------------------------------------------------------

/** The ceremony input, checked: every field present, nothing unknown, no placeholder, no test value. */
export function checkInput(input) {
  if (!input || typeof input !== "object" || Array.isArray(input)) throw new Error("refused: the input is not a JSON object");
  const unknown = Object.keys(input).filter((k) => !k.startsWith("_") && !INPUT_FIELDS.includes(k));
  if (unknown.length) throw new Error(`refused: unknown input field(s) ${unknown.join(", ")}: everything else is derived, never typed (key_name, networks, caps, hashes…)`);
  const missing = INPUT_FIELDS.filter((k) => input[k] === undefined);
  if (missing.length) throw new Error(`refused: missing input field(s) ${missing.join(", ")}`);
  const operator = principalOf(input.operator, "operator");
  const guardian = principalOf(input.guardian, "guardian");
  // the canister has no guardian field: its pause guardian acts through the timelock (pause_target), so g must be one of its guardians
  if (!GUARDIANS.includes(guardian.toText())) throw new Error(`refused: guardian ${guardian.toText()} is not one of the timelock's guardians (timelock-init.mjs GUARDIANS); the canister's pause guardian pauses through the timelock's pause_target`);
  if ([...GUARDIANS, ...PROPOSERS].includes(operator.toText())) throw new Error(`refused: operator ${operator.toText()} holds a timelock role; the runner's hot key must not be a proposer or a guardian`);
  const sol = httpsUrls(input.solana_rpc_urls, "solana_rpc_urls");
  const eth = httpsUrls(input.eth_rpc_urls, "eth_rpc_urls");
  for (const u of eth) {
    const p = new URL(u);
    const paths = ETH_MEASURED[p.hostname];
    if (!paths || !paths.includes(p.pathname.replace(/\/$/, "")) || p.search) throw new Error(`refused: eth_rpc_urls: ${redactUrl(u)} is not one of the providers measured to serve the 7-day sale window (script/golive-check.sh DEEP: ${Object.keys(ETH_MEASURED).join(", ")}); measure it first (golive-check --probe-depth)`);
  }
  const btc = httpsUrls(input.btc_api_urls, "btc_api_urls").map((u) => u.replace(/\/$/, ""));
  for (const u of btc) if (!BTC_ESPLORA.includes(u)) throw new Error(`refused: btc_api_urls: ${redactUrl(u)} is not a mainnet esplora explorer that answered the C7 checkpoint (${BTC_ESPLORA.join(", ")})`);
  const seed = input.nonce_seed;
  if (typeof seed !== "string" || PLACEHOLDER.test(seed)) throw new Error("refused: nonce_seed is a placeholder; fill it in");
  if (seed.length < 1 || seed.length > 32 || ![...seed].every((c) => c.charCodeAt(0) > 0x20 && c.charCodeAt(0) < 0x7f)) throw new Error("refused: nonce_seed must be 1-32 printable ASCII bytes, no space (canister.rs validate_home)");
  if (TEST_WORDS.test(seed) || /^rook-devnet|^rook-w\d/i.test(seed)) throw new Error(`refused: nonce_seed ${JSON.stringify(seed)} is a rehearsal seed; the mainnet seed is permanent (the nonce account's address)`);
  return { operator, guardian, sol, eth, btc, seed };
}

// ---- the launch set -----------------------------------------------------------------------------------------------------------

/** `coins::unlock::ORDER` and `LAUNCH_COUNT`, read from the canister's source (the registry the pins come from). */
export function launchOrder(unlockPath = ROOT + "autobuyer/canister-rs/src/coins/unlock.rs") {
  const src = readFileSync(unlockPath, "utf8");
  const body = src.split("pub const ORDER")[1]?.split("];")[0] ?? "";
  const order = [...body.matchAll(/e\("([^"]+)",\s*"[^"]*",\s*"[^"]*",\s*(\w+)\)/g)].map((m) => ({ slug: m[1], source: m[2] }));
  const count = Number(src.match(/pub const LAUNCH_COUNT: usize = (\d+);/)?.[1]);
  if (!order.length || !Number.isInteger(count) || count < 1 || count > order.length) throw new Error("could not read ORDER / LAUNCH_COUNT from unlock.rs");
  return { order, count };
}

/** The install's eth_nft.collections: the Ethereum (`Eth`) entries of the first LAUNCH_COUNT of ORDER, in registry order, each with every
 *  parameter the launch file gives. Robinhood Chain collections are never here (pinned after the ceremony). */
export function launchEthCollections({ pilotPath = ROOT + "deployments/launch/nft-pilot.json", unlockPath } = {}) {
  const pilot = JSON.parse(readFileSync(pilotPath, "utf8"));
  const { order, count } = launchOrder(unlockPath);
  const bySlug = Object.fromEntries(pilot.collections.map((c) => [c.slug, c]));
  // max_premium_bps: the launch file's _canister note ("max_premium_bps 500 each"); a per-collection maxPremiumBps would win
  const premium = /max_premium_bps (\d+) each/.exec(pilot._canister ?? "")?.[1];
  if (!premium) throw new Error("nft-pilot.json's _canister note no longer states max_premium_bps");
  return order.slice(0, count).filter((e) => e.source === "Eth").map(({ slug }) => {
    const c = bySlug[slug];
    if (!c) throw new Error(`${slug}: an Ethereum collection of the launch set is not in nft-pilot.json`);
    if (c.chainId !== 1) throw new Error(`${slug}: chain ${c.chainId}, not Ethereum mainnet (a Robinhood Chain collection is added after the ceremony)`);
    if (!/^0x[0-9a-fA-F]{40}$/.test(c.collection) || /^0x0{40}$/i.test(c.collection)) throw new Error(`${slug}: collection ${c.collection} is not an address`);
    const max = BigInt(c.maxPurchaseWei);
    if (max < 50_000_000_000_000_000n || max > 25_000_000_000_000_000_000n) throw new Error(`${slug}: maxPurchaseWei ${max} outside [0.05, 25] ETH (ethnft.rs validate)`);
    const bps = Number(c.maxPremiumBps ?? premium);
    if (!Number.isInteger(bps) || bps < 0 || bps > 1000) throw new Error(`${slug}: max_premium_bps ${bps} over 1,000 (ethnft.rs MAX_PREMIUM_BPS)`);
    return { slug, collection: c.collection.toLowerCase(), wrapper: "books", module: "books", max_purchase_wei: max, max_premium_bps: bps,
      chain_id: [], token_bound: [], nft_value_bps: [] };
  });
}

// ---- live reads ---------------------------------------------------------------------------------------------------------------

async function rpc(fetchFn, url, method, params) {
  const r = await fetchFn(url, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params }) });
  if (!r.ok) throw new Error(`${redactUrl(url)} ${method}: HTTP ${r.status}`);
  const j = await r.json();
  if (j.error) throw new Error(`${redactUrl(url)} ${method}: ${JSON.stringify(j.error)}`);
  return j.result;
}

/** keccak256 of an address's code, read from EVERY provider: they must agree and the code must not be empty; with `expected`, the hash
 *  must equal it. Never a placeholder: a failed read is a refusal. */
export async function codehashOf(urls, address, { fetch: fetchFn = globalThis.fetch, expected, what = address } = {}) {
  const hashes = [];
  for (const u of urls) {
    const code = await rpc(fetchFn, u, "eth_getCode", [address, "latest"]);
    if (typeof code !== "string" || !/^0x([0-9a-fA-F]{2})*$/.test(code)) throw new Error(`refused: ${what}: ${redactUrl(u)} answered no code`);
    if (code.length <= 2) throw new Error(`refused: ${what}: ${redactUrl(u)} says ${address} has no code on this chain`);
    hashes.push("0x" + Buffer.from(keccak_256(Buffer.from(code.slice(2), "hex"))).toString("hex"));
  }
  if (new Set(hashes).size !== 1) throw new Error(`refused: ${what}: the providers disagree on ${address}'s code (${hashes.join(" vs ")})`);
  if (expected && hashes[0] !== expected.toLowerCase()) throw new Error(`refused: ${what}: the live code hash ${hashes[0]} is not the one the docs recorded (${expected})`);
  if (/^0x(00)+$|^0x(11)+$/.test(hashes[0])) throw new Error(`refused: ${what}: a placeholder hash`);
  return hashes[0];
}

/** Everything the Init takes from the chains, read live: chain ids, genesis, checkpoint, and the code hashes. */
export async function liveChain({ eth, sol, btc }, { fetch: fetchFn = globalThis.fetch,
  expect = { seaport: SEAPORT_CODEHASH_ETHEREUM, zone: SIGNED_ZONE_CODEHASH_ETHEREUM } } = {}) {
  for (const u of eth) {
    const id = await rpc(fetchFn, u, "eth_chainId", []);
    if (BigInt(id) !== 1n) throw new Error(`refused: eth_rpc_urls: ${redactUrl(u)} is chain ${BigInt(id)}, not Ethereum mainnet (1)`);
  }
  for (const u of sol) {
    const g = await rpc(fetchFn, u, "getGenesisHash", []);
    if (g !== SOLANA_MAINNET_GENESIS) throw new Error(`refused: solana_rpc_urls: ${redactUrl(u)} answers genesis ${g}, not mainnet-beta's (${SOLANA_MAINNET_GENESIS})`);
  }
  for (const u of btc) {
    const r = await fetchFn(`${u}/block-height/${BTC_CHECKPOINT.height}`);
    const h = r.ok ? (await r.text()).trim() : `HTTP ${r.status}`;
    if (h !== BTC_CHECKPOINT.hash) throw new Error(`refused: btc_api_urls: ${redactUrl(u)} answers ${h} at ${BTC_CHECKPOINT.height}, not the mainnet checkpoint`);
  }
  const seaport = await codehashOf(eth, SEAPORT_1_6, { fetch: fetchFn, expected: expect.seaport, what: "Seaport 1.6" });
  const zone = await codehashOf(eth, SIGNED_ZONE, { fetch: fetchFn, expected: expect.zone, what: "OpenSea SignedZone" });
  const registries = [];
  for (const r of ZONE_REGISTRIES) registries.push({ address: r, codehash: await codehashOf(eth, r, { fetch: fetchFn, what: `zone registry ${r}` }) });
  return { seaport_codehash: seaport, signed_zones: [{ zone: SIGNED_ZONE, codehash: zone }], registries };
}

// ---- the Init -----------------------------------------------------------------------------------------------------------------

/** The production Init (pure): the checked input + what was read live. `localReplicaDryRun` swaps ONLY key_name to dfx_test_key. */
export function buildInit(input, live, { localReplicaDryRun = false, pilotPath, unlockPath } = {}) {
  const i = checkInput(input);
  if (!live || !/^0x[0-9a-f]{64}$/.test(live.seaport_codehash ?? "")) throw new Error("refused: no live Seaport code hash (liveChain must run first)");
  const cols = launchEthCollections({ pilotPath, unlockPath });
  if (!cols.length) throw new Error("refused: no Ethereum collection in the launch set");
  const r = Object.fromEntries(Object.entries(ETH_CHAIN_RULES).map(([k, [v]]) => [k, v]));
  const v = {
    chain_id: EVM_HOME_CHAIN, vault: VAULT_PLACEHOLDER, rpc_urls: EVM_HOME_RPC, btc_api_urls: i.btc,
    key_name: localReplicaDryRun ? "dfx_test_key" : "key_1", accept_production_key_name: true, bridge_allowlist: [], btc_network: "mainnet",
    operator: [i.operator], wrapper: [],
    evm_chains: [[{ chain_id: 1n, rpc_urls: i.eth, bucket: r.bucket, seaport: SEAPORT_1_6, seaport_codehash: live.seaport_codehash,
      max_fee_wei: r.max_fee_wei, priority_fee_wei: r.priority_fee_wei, gas_cap: r.gas_cap, fee_recipients: [OPENSEA_FEE_RECIPIENT],
      max_fee_bps: r.max_fee_bps, min_order_life_s: r.min_order_life_s,
      signed_zones: [live.signed_zones.map(({ zone, codehash }) => ({ zone, codehash }))],
      zone_registries: [live.registries.map((x) => x.address)] }]],
    custody_caps: [], outflow_limits: [], holder_deposits: [], wrapper_from_block: [],
    frog_funding: [frogFunding(pilotPath)],
    eth_nft: [{ chain_id: 1n, collections: cols.map(({ slug, ...c }) => c), relay_max_fee_bps: 300, deposits: false, floor: [],
      gas_pot_target_wei: [], logs_max_blocks: [], dormancy: [] }],
    home: [{ Solana: { cluster: "mainnet-beta", rpc_urls: i.sol, rook_core: "books", nonce_seed: i.seed, blockhash_fallback: [],
      rook_backing: [], drand_urls: [] } }],
  };
  checkInit(v, { localReplicaDryRun });
  return v;
}

function frogFunding(pilotPath = ROOT + "deployments/launch/nft-pilot.json") {
  const ff = JSON.parse(readFileSync(pilotPath, "utf8")).bitcoin?.frog_funding;
  if (!ff || !Number.isInteger(ff.max_fee_bps) || ff.max_fee_bps < 50 || ff.max_fee_bps > 500 || typeof ff.float_first !== "boolean") throw new Error("nft-pilot.json bitcoin.frog_funding is not { max_fee_bps 50..500, float_first }");
  return { max_fee_bps: ff.max_fee_bps, float_first: ff.float_first };
}

/** The last line of defence, on the Init itself: no test key, no test network, no test chain, no placeholder hash, the books home. */
export function checkInit(v, { localReplicaDryRun = false } = {}) {
  if (localReplicaDryRun ? v.key_name !== "dfx_test_key" : v.key_name !== "key_1") throw new Error(`refused: key_name ${v.key_name}`);
  if (["test_key_1", "dfx_test_key"].includes(v.key_name) && !localReplicaDryRun) throw new Error(`refused: ${v.key_name} is a test key`);
  if (!v.accept_production_key_name) throw new Error("refused: accept_production_key_name must be true for key_1");
  if (v.btc_network !== "mainnet") throw new Error(`refused: btc_network ${v.btc_network}`);
  const chains = [v.chain_id, ...(v.evm_chains[0] ?? []).map((c) => c.chain_id), ...(v.eth_nft[0] ? [v.eth_nft[0].chain_id] : [])];
  if (chains.some((c) => c === 46630n)) throw new Error("refused: chain 46630 (Robinhood Chain testnet) is the rehearsal chain");
  const evm = v.evm_chains[0] ?? [];
  if (evm.length !== 1 || evm[0].chain_id !== 1n) throw new Error("refused: evm_chains must be [Ethereum (1)] alone at the ceremony");
  const hashes = [evm[0].seaport_codehash, ...(evm[0].signed_zones[0] ?? []).map((z) => z.codehash)];
  for (const h of hashes) if (!/^0x[0-9a-f]{64}$/.test(h) || /^0x(00)+$|^0x(11)+$/.test(h)) throw new Error(`refused: code hash ${h} is a placeholder`);
  const home = v.home[0]?.Solana;
  if (!home || home.cluster !== "mainnet-beta" || home.rook_core !== "books" || home.rook_backing.length) throw new Error("refused: the home must be Solana mainnet-beta on the books");
  const urls = [...v.rpc_urls, ...v.btc_api_urls, ...home.rpc_urls, ...evm[0].rpc_urls];
  for (const u of urls) if (TEST_WORDS.test(new URL(u).host + new URL(u).pathname)) throw new Error(`refused: ${redactUrl(u)} names a test network`);
  if (v.custody_caps.length) throw new Error("refused: custody_caps must be null at the ceremony (runbook §1)");
  for (const c of v.eth_nft[0].collections) if (c.wrapper !== "books" || c.module !== "books") throw new Error("refused: an Ethereum collection not on the books");
  return true;
}

/** JSON with every object's keys sorted (a decoded record's fields come in Candid's hash order). */
export function canonical(x) {
  const sort = (v) => (Array.isArray(v) ? v.map(sort) : v && typeof v === "object" ? Object.fromEntries(Object.keys(v).sort().map((k) => [k, sort(v[k])])) : v);
  return JSON.stringify(sort(x));
}

/** The Candid bytes (hex), decoded back with the monitor's strict decoder and compared field by field with what was meant. */
export function encodeChecked(v) {
  const hex = Buffer.from(IDL.encode([Init], [v])).toString("hex");
  const d = decodeInit(hex);
  if (!d.ok) throw new Error("the Init does not decode back: " + d.error);
  if (canonical(d.init) !== canonical(plain(v))) throw new Error("the Init decodes back to something else than was encoded");
  return { hex, decoded: d.init };
}

// ---- the summary --------------------------------------------------------------------------------------------------------------

const eth = (wei) => `${Number(BigInt(wei)) / 1e18} ETH`;

/** The decoded Init, for a human, with each value's source. URLs keep their host and path only. */
export function summarize(d, { localReplicaDryRun = false } = {}) {
  const c = d.evm_chains[0][0];
  const home = d.home[0].Solana;
  const n = d.eth_nft[0];
  const L = [];
  const row = (k, val, src = SOURCES[k]) => L.push(`  ${k.padEnd(27)} ${val}${src ? `\n  ${"".padEnd(27)} ← ${src}` : ""}`);
  L.push("The buyer's MAINNET Init, decoded back from the hex on stdout:");
  if (localReplicaDryRun) L.push("  !!! --local-replica-dry-run: key_name is dfx_test_key (swapped from key_1 for a LOCAL replica). NEVER install this on the IC. !!!");
  row("operator", d.operator[0], "the input (the runner's IC principal)");
  row("key_name", `${d.key_name}, accept_production_key_name = ${d.accept_production_key_name}`);
  row("btc_network", d.btc_network);
  row("btc_api_urls", d.btc_api_urls.map(redactUrl).join(", "), "the input; each answered the mainnet checkpoint 900,000 live");
  row("home", `Solana ${home.cluster}, rook_core = ${home.rook_core}, nonce_seed = ${JSON.stringify(home.nonce_seed)}`, `${SOURCES["home.cluster"]}; ${SOURCES["home.rook_core"]}; nonce_seed from the input (PERMANENT)`);
  row("home.rpc_urls", home.rpc_urls.map(redactUrl).join(", "), "the input (O-13); each answered mainnet-beta's genesis live. Query strings redacted here; they ARE in the hex");
  row("home.(others)", `blockhash_fallback ${JSON.stringify(home.blockhash_fallback)}, rook_backing ${JSON.stringify(home.rook_backing)}, drand_urls ${JSON.stringify(home.drand_urls)}`, `${SOURCES["home.blockhash_fallback"]}; ${SOURCES["home.rook_backing"]}; ${SOURCES["home.drand_urls"]}`);
  row("chain_id / vault / rpc_urls", `${d.chain_id} / ${d.vault} / ${d.rpc_urls.map(redactUrl).join(", ")}`, `${SOURCES.chain_id}; ${SOURCES.vault}; ${SOURCES.rpc_urls}`);
  row("bridge_allowlist", JSON.stringify(d.bridge_allowlist));
  row("wrapper", JSON.stringify(d.wrapper));
  row("custody_caps", JSON.stringify(d.custody_caps));
  row("outflow_limits", JSON.stringify(d.outflow_limits));
  row("holder_deposits", JSON.stringify(d.holder_deposits));
  row("wrapper_from_block", JSON.stringify(d.wrapper_from_block));
  row("frog_funding", JSON.stringify(d.frog_funding));
  row("evm_chains", `[chain ${c.chain_id}] via ${c.rpc_urls.map(redactUrl).join(", ")}`, SOURCES.evm_chains);
  row("  seaport", `${c.seaport}, code hash ${c.seaport_codehash}`, SOURCES.seaport_codehash);
  for (const k of Object.keys(ETH_CHAIN_RULES)) row(`  ${k}`, String(c[k]), ETH_CHAIN_RULES[k][1]);
  row("  fee_recipients", c.fee_recipients.join(", "), SOURCES.fee_recipients);
  row("  signed_zones", (c.signed_zones[0] ?? []).map((z) => `${z.zone} @ ${z.codehash}`).join(", ") || "none", SOURCES.signed_zones);
  row("  zone_registries", (c.zone_registries[0] ?? []).join(", ") || "none", SOURCES.zone_registries);
  if ((c.signed_zones[0] ?? []).length && BigInt(c.min_order_life_s) >= 280n)
    L.push(`  NOTE restricted (SignedZone) OpenSea listings: the zone is allow-listed, but a zone signature lives ~276-288 s (eth-nft-canister.md §12.1)\n`
      + `       and min_order_life_s = ${c.min_order_life_s} s refuses one expiring sooner (seaport.rs): in practice restricted listings are SKIPPED until an\n`
      + `       upgrade lowers min_order_life_s (the fork rehearsal used 60 s); open (zone 0) listings, 99 % of Ethereum fills (§1), are bought.`);
  row("eth_nft", `chain ${n.chain_id}, relay_max_fee_bps ${n.relay_max_fee_bps}, deposits ${n.deposits}, floor ${JSON.stringify(n.floor)}, gas_pot ${JSON.stringify(n.gas_pot_target_wei)}, logs_max ${JSON.stringify(n.logs_max_blocks)}, dormancy ${JSON.stringify(n.dormancy)}`,
    [SOURCES["eth_nft.chain_id"], SOURCES["eth_nft.relay_max_fee_bps"], SOURCES["eth_nft.deposits"], SOURCES["eth_nft.floor"], SOURCES["eth_nft.gas_pot_target_wei"], SOURCES["eth_nft.logs_max_blocks"], SOURCES["eth_nft.dormancy"]].join("; "));
  row("eth_nft.collections", `${n.collections.length}`, SOURCES["eth_nft.collections"]);
  for (const x of n.collections) L.push(`    ${x.collection}  max ${eth(x.max_purchase_wei)}, premium ${x.max_premium_bps} bps, ${x.wrapper}/${x.module}`);
  L.push("  Not in this Init: Robinhood Chain (4663) and its 3 collections (added after the ceremony, by upgrade). The guardian: the canister has no");
  L.push("  guardian field; the pause guardian (g1) pauses through the timelock's pause_target (timelock-init.mjs), and the operator may set_paused(true).");
  return L.join("\n");
}

// ---- the command line -----------------------------------------------------------------------------------------------------------

export async function main(argv, { fetch: fetchFn = globalThis.fetch, expect, out = process.stdout, err = process.stderr } = {}) {
  argv = [...argv];
  const take = (flag) => { const i = argv.indexOf(flag); if (i < 0) return null; const v = argv[i + 1]; argv.splice(i, 2); return v; };
  const dry = argv.includes("--local-replica-dry-run");
  if (dry) argv.splice(argv.indexOf("--local-replica-dry-run"), 1);
  const bin = take("--bin");
  const unknownFlag = argv.find((a) => a.startsWith("--"));
  if (unknownFlag) throw new Error(`refused: unknown flag ${unknownFlag}`);
  if (argv.length !== 1) throw new Error("usage: node ops/ceremony/mainnet-init.mjs <ceremony-input.json> [--bin <out.bin>] [--local-replica-dry-run]");
  const input = JSON.parse(readFileSync(argv[0], "utf8"));
  const checked = checkInput(input);
  const live = await liveChain(checked, expect ? { fetch: fetchFn, expect } : { fetch: fetchFn });
  const v = buildInit(input, live, { localReplicaDryRun: dry });
  const { hex, decoded } = encodeChecked(v);
  if (bin) writeFileSync(bin, Buffer.from(hex, "hex"));
  out.write(hex + "\n");
  err.write(summarize(decoded, { localReplicaDryRun: dry }) + "\n");
  if (dry) err.write("--local-replica-dry-run: key_name was swapped to dfx_test_key. This argument is for a LOCAL replica only.\n");
  return { hex, decoded };
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main(process.argv.slice(2)).catch((e) => { process.stderr.write(String(e.message ?? e) + "\n"); process.exit(1); });
}
