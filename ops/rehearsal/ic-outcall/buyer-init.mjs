// A THROWAWAY buyer Init for the rehearsal's second path (buyer-throwaway.sh): the canister's Ethereum pair as evm_chains' only chain,
// nothing custodial configured (no vault in use, no wrapper, no caps, no eth_nft), so the installed buyer can only answer the
// operator's diagnostic reads. Encoded by the monitor's own IDL (monitor/src/candid.mjs), as scripts/golive-dryrun.sh's installhex.
//   node ops/rehearsal/ic-outcall/buyer-init.mjs <key name> [<eth url>,<eth url>]   → the Candid hex on stdout
const root = new URL("../../../", import.meta.url).pathname;
const { Init, decodeInit } = await import(root + "monitor/src/candid.mjs");
const { IDL } = await import(root + "monitor/node_modules/@icp-sdk/core/lib/esm/candid/index.js").catch(() => import("@icp-sdk/core/candid"));
const [key = "dfx_test_key", urls = "https://rpc.mevblocker.io,https://gateway.tenderly.co/public/mainnet"] = process.argv.slice(2);
const eth = urls.split(",");
const chain = (id, rpc) => ({ chain_id: BigInt(id), rpc_urls: rpc, bucket: 8n, seaport: "0x0000000000000068f116a894984e2db1123eb395",
  seaport_codehash: "0x" + "11".repeat(32), max_fee_wei: 50000000000n, priority_fee_wei: 1000000000n, gas_cap: 400000n, fee_recipients: [],
  max_fee_bps: 300, min_order_life_s: 600n, signed_zones: [], zone_registries: [] });
const v = { chain_id: 4663n, vault: "0x00000000000000000000000000000000000000aa",
  rpc_urls: ["https://rpc.mainnet.chain.robinhood.com", "https://rpc.ordofi.network"], btc_api_urls: ["https://mempool.space/api", "https://blockstream.info/api"],
  key_name: key, accept_production_key_name: key === "key_1", bridge_allowlist: [], btc_network: "signet", operator: [], wrapper: [],
  evm_chains: [[chain(1, eth)]], custody_caps: [], outflow_limits: [], holder_deposits: [], wrapper_from_block: [], frog_funding: [], eth_nft: [] };
const hex = Buffer.from(IDL.encode([Init], [v])).toString("hex");
const d = decodeInit(hex);
if (!d.ok || d.init.evm_chains[0][0].rpc_urls.join(",") !== eth.join(",")) throw new Error("the Init does not decode back: " + d.error);
process.stdout.write(hex + "\n");
