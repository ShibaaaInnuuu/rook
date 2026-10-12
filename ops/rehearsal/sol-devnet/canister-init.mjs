// The THROWAWAY buyer's Init for the Solana devnet rehearsal (docs/v9/rehearsal-sol-devnet.md): `home = Solana(devnet)` with two devnet
// providers, rook_core's program id, the frog wrapper of this job's frog listing (rook_core PDA, base58), the Ethereum NFT side naming the
// job's EthNft listing (wrapper and module PDAs), Bitcoin = the registry's network (signet or testnet4), the threshold key `test_key_1`
// (IC) or `key_1` (a local replica). Encoded with the monitor's own IDL (monitor/src/candid.mjs) and decoded back, as buyer-init.mjs does.
//   node ops/rehearsal/sol-devnet/canister-init.mjs <plan.json> <key name> <solana url>,<solana url> [btc network] [nonce seed]
// W15, the canister-only rehearsal (docs/v9/ceremony-canister-only.md §1): no program and no plan. `rook_core = "books"` (the canister keeps
// the books itself), the Ethereum collections of the launch set with `wrapper = module = "books"`, no frog wrapper (unused on the books home:
// the Inscriptions listing in the books is the frog collection, pinned with `coins_pin`). A Solana collection is a books listing of kind
// SolNft with no EVM record: it comes only through its pin (`launch_pins --sol-custodian <sol_address>`).
//   node ops/rehearsal/sol-devnet/canister-init.mjs --books <key name> <solana url>,<solana url> [btc network] [nonce seed] [--count 5]
// → the Candid hex on stdout. Nothing here is a secret. The pure parts (`booksInit`, `LAUNCH`) are exported for test/books.test.mjs.
import { readFileSync } from "node:fs";
import { pathToFileURL } from "node:url";

const root = new URL("../../../", import.meta.url).pathname;
const { Init, decodeInit } = await import(root + "monitor/src/candid.mjs");
const { IDL } = await import(root + "monitor/node_modules/@icp-sdk/core/lib/esm/candid/index.js").catch(() => import("@icp-sdk/core/candid"));

// Ethereum mainnet is READ only (sale scans, pins) and never written: the rehearsal's buys stop at the stubbed Relay leg (runbook §4)
const eth = ["https://rpc.mevblocker.io", "https://gateway.tenderly.co/public/mainnet"];
const chain = (id, urls) => ({ chain_id: BigInt(id), rpc_urls: urls, bucket: 8n, seaport: "0x0000000000000068f116a894984e2db1123eb395",
  seaport_codehash: "0x" + "00".repeat(32), max_fee_wei: 50000000000n, priority_fee_wei: 1000000000n, gas_cap: 400000n, fee_recipients: [],
  max_fee_bps: 300, min_order_life_s: 600n, signed_zones: [], zone_registries: [] });
const MILADY = "0x5Af0D9827E0c53E4799BB226655A1de152A425a5";
const btcApiOf = (btc) => (btc === "signet" ? ["https://mempool.space/signet/api", "https://blockstream.info/signet/api"]
  : ["https://mempool.space/testnet4/api", "https://mempool.emzy.de/testnet4/api"]);
// the EVM fields stay required by the Init: chain 46630 (Robinhood testnet, the rehearsal chain), never read on a Solana home
const evmFields = (key, btc) => ({
  chain_id: 46630n, vault: "0x00000000000000000000000000000000000000aa",
  rpc_urls: ["https://rpc.testnet.chain.robinhood.com", "https://rpc.testnet.chain.robinhood.com/rpc"],
  btc_api_urls: btcApiOf(btc), key_name: key, accept_production_key_name: key === "key_1", bridge_allowlist: [], btc_network: btc, operator: [],
  evm_chains: [[chain(1, eth)]], custody_caps: [], outflow_limits: [], holder_deposits: [], wrapper_from_block: [],
});

/** The collections the ceremony pins, in the canister's registry order (`coins::unlock::ORDER[..LAUNCH_COUNT]`, docs/v9/canister-only-solana.md
 *  §16, operator decisions 2026-10-12: every collection with a pipeline, the 14 Ordinals included ("no picking"); each opens when a creator *  first asks for it; the Robinhood Chain three are pinned after the ceremony). The rehearsal
 *  pins the first `count` (2 by default: devnet SOL is scarce; mainnet pins all 38); the Ethereum ones among them are the install's
 *  `eth_nft.collections` (the Inscriptions collections and the Solana collections need nothing in the Init: their pins carry their custody). */
export const LAUNCH = ["bayc", "milady", "mad_lads", "smb_gen2", "bitcoin-frogs", "pudgypenguins", "azuki", "mayc", "doodles", "lilpudgys",
  "moonbirds", "clonex", "checks", "identitymd", "claynosaurz", "famous_fox_federation", "degods", "galactic_geckos", "taiyo_robotics",
  "okay_bears", "smb_gen3", "transdimensional_fox_federation", "pythenians", "cyber_frogs", "omb", "nodemonkes", "bitcoin-puppets", "bitcoin-punks", "natcats", "runestone", "bitcoin-weirdos", "cents", "rsic", "pizza-ninjas", "quantum-cats", "wizards-of-ord", "taproot-wizards", "bitcoin-wizards"];

/** The Ethereum collections of the first `count` of the launch set, their ERC-721 addresses from the launch file. */
export function launchEthCollections(count, pilotPath = root + "deployments/launch/nft-pilot.json") {
  const pilot = JSON.parse(readFileSync(pilotPath, "utf8"));
  const bySlug = Object.fromEntries(pilot.collections.map((c) => [c.slug, c]));
  return LAUNCH.slice(0, count).filter((s) => bySlug[s]).map((s) => {
    if (bySlug[s].chainId !== 1) throw new Error(`${s}: chain ${bySlug[s].chainId}, not Ethereum mainnet`);
    return { slug: s, collection: bySlug[s].collection };
  });
}

/** The install checks that are the tool's to make before anything is encoded (shared by both forms). */
export function checkArgs({ key, sol, btc }) {
  if (key === "key_1" && process.env.ROOK_LOCAL_REPLICA !== "1") throw new Error("refused: key_1 is the production key's name; the IC rehearsal signs with test_key_1");
  if (!["signet", "testnet4"].includes(btc)) throw new Error(`refused: btc network ${btc}: the rehearsal's registry serves signet or testnet4`);
  if (sol.length < 2 || new Set(sol).size !== sol.length) throw new Error("two distinct Solana providers at least");
  if (sol.some((u) => /mainnet/i.test(u))) throw new Error("refused: a Solana provider names mainnet; this is the DEVNET rehearsal");
}

/** The books Init (W10 §1) as the monitor's IDL takes it: `home.Solana.rook_core = "books"`, every Ethereum collection `wrapper = module = "books"`. */
export function booksInit({ key = "test_key_1", sol, btc = "signet", seed = "rook-devnet-0", count = LAUNCH.length }) {
  checkArgs({ key, sol, btc });
  if (!Number.isInteger(count) || count < 1 || count > LAUNCH.length) throw new Error(`refused: --count takes 1 to ${LAUNCH.length} (the collections the ceremony pins)`);
  const cols = launchEthCollections(count);
  if (cols.length === 0) throw new Error(`refused: no Ethereum collection in the first ${count} of the launch set (the Init needs one)`);
  return {
    ...evmFields(key, btc),
    // no frog wrapper on the books home (runbook §1) and no float: the books home's frogs are the Inscriptions listing in the books
    wrapper: [], frog_funding: [],
    eth_nft: [{ chain_id: 1n, collections: cols.map((c) => ({ collection: c.collection, wrapper: "books", module: "books", max_purchase_wei: 10n ** 19n,
      max_premium_bps: 500, chain_id: [], token_bound: [], nft_value_bps: [] })), relay_max_fee_bps: 100, deposits: false, floor: [], gas_pot_target_wei: [], logs_max_blocks: [], dormancy: [] }],
    home: [{ Solana: { cluster: "devnet", rpc_urls: sol, rook_core: "books", nonce_seed: seed, blockhash_fallback: [], rook_backing: [],
      // drand_urls: null = the canister's default drand relays (D-i: the canister proves rounds itself)
      drand_urls: [] } }],
  };
}

/** The program rehearsal's Init: rook_core's program and the job's PDAs from `plan.json`. */
export function planInit({ plan, key, sol, btc, seed }) {
  checkArgs({ key, sol, btc });
  return {
    ...evmFields(key, btc),
    wrapper: [plan.frog.wrapper],
    // float_first: a frog bought from an operator float (signet sats sent by the operator) is reimbursed by item_arrived — the
    // rehearsal's stand-in for Relay's SOL -> BTC leg, which has no devnet
    frog_funding: [{ max_fee_bps: 300, float_first: true }],
    eth_nft: [{ chain_id: 1n, collections: [{ collection: MILADY, wrapper: plan.eth.wrapper, module: plan.eth.module, max_purchase_wei: 10n ** 19n,
      max_premium_bps: 500, chain_id: [], token_bound: [], nft_value_bps: [] }], relay_max_fee_bps: 100, deposits: false, floor: [], gas_pot_target_wei: [], logs_max_blocks: [], dormancy: [] }],
    home: [{ Solana: { cluster: "devnet", rpc_urls: sol, rook_core: plan.rook_core, nonce_seed: seed, blockhash_fallback: [], rook_backing: [],
      // drand_urls: null = the canister's default drand relays (D-i: the canister proves rounds as the registry keeper)
      drand_urls: [] } }],
  };
}

/** The Candid hex of an Init, decoded back with the monitor's decoder before it is handed out. */
export function encodeChecked(v) {
  const hex = Buffer.from(IDL.encode([Init], [v])).toString("hex");
  const d = decodeInit(hex);
  if (!d.ok) throw new Error("the Init does not decode back: " + d.error);
  return hex;
}

/** `argv` (after node and the script) → the Init; `null` for a usage error. */
export function initOfArgv(argv) {
  argv = [...argv];
  const books = argv.includes("--books");
  if (books) argv.splice(argv.indexOf("--books"), 1);
  let count = LAUNCH.length;
  if (argv.includes("--count")) {
    const i = argv.indexOf("--count");
    count = Number(argv[i + 1]);
    argv.splice(i, 2);
  }
  const [planPath, key = "test_key_1", solUrls = "https://api.devnet.solana.com,https://devnet.rpcpool.com", btc = "signet", seed = "rook-devnet-0"] =
    books ? [null, ...argv] : argv;
  if (!books && !planPath) return null;
  const sol = solUrls.split(",");
  return books ? booksInit({ key, sol, btc, seed, count }) : planInit({ plan: JSON.parse(readFileSync(planPath, "utf8")), key, sol, btc, seed });
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const v = initOfArgv(process.argv.slice(2));
  if (!v) throw new Error("usage: canister-init.mjs <plan.json> <key name> <solana url>,<solana url> [btc network] [nonce seed]\n"
    + "       canister-init.mjs --books <key name> <solana url>,<solana url> [btc network] [nonce seed] [--count 5]");
  process.stdout.write(encodeChecked(v) + "\n");
}
