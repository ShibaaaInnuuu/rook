// Builds a real buyer Init (monitor/src/candid.mjs), as scripts/golive-dryrun.sh installhex does, with the given Ethereum URLs.
const M = "/Users/jlohayon/Documents/Rook-nft/monitor";
const { Init } = await import(M + "/src/candid.mjs");
const { IDL } = await import(M + "/node_modules/@icp-sdk/core/lib/esm/candid/index.js").catch(() => import("@icp-sdk/core/candid"));
const eth = process.argv[2].split(",");
const home = ["https://rpc.mainnet.chain.robinhood.com", "https://rpc.ordofi.network"];
const chain = (id, urls) => ({ chain_id: BigInt(id), rpc_urls: urls, bucket: 0n, seaport: "0x0000000000000068f116a894984e2db1123eb395",
  seaport_codehash: "0x" + "00".repeat(32), max_fee_wei: 50000000000n, priority_fee_wei: 1000000000n, gas_cap: 400000n, fee_recipients: [],
  max_fee_bps: 300, min_order_life_s: 600n, signed_zones: [], zone_registries: [] });
const v = { chain_id: 4663n, vault: "0x00", rpc_urls: home, btc_api_urls: ["https://mempool.space/api", "https://blockstream.info/api"],
  key_name: "key_1", accept_production_key_name: true, bridge_allowlist: [], btc_network: "mainnet", operator: [], wrapper: [],
  evm_chains: [[chain(4663, home), chain(1, eth)]], custody_caps: [], outflow_limits: [], holder_deposits: [], wrapper_from_block: [],
  frog_funding: [], eth_nft: [] };
process.stdout.write(Buffer.from(IDL.encode([Init], [v])).toString("hex") + "\n");
