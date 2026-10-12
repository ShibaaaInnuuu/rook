import { encodeInitPlain } from "./monitor/src/candid.mjs";
import { stepNumbers } from "./monitor/src/staged.mjs";
import { readFileSync, writeFileSync } from "node:fs";
const L = JSON.parse(readFileSync("deployments/launch/nft-pilot.json", "utf8"));
const WRAP = "0x" + "77".repeat(20);
const mk = (step, btcItems, gItems, gValue) => {
  const s = stepNumbers(L, step);
  const collections = Object.entries(s.caps).map(([collection, c]) => ({ collection, max_items: Number(c.items), max_value_wei: String(c.value), max_item_wei: String(c.item) }));
  collections.push({ collection: `btc:${WRAP}`, max_items: btcItems, max_value_wei: String(BigInt(btcItems) * 10n ** 19n / 3n), max_item_wei: String(10n ** 19n) });
  return encodeInitPlain({ chain_id: 4663n, vault: "0x00", rpc_urls: ["https://a", "https://b"], btc_api_urls: ["c", "d"], key_name: "key_1", accept_production_key_name: true,
    bridge_allowlist: [], btc_network: "mainnet", operator: [], wrapper: [WRAP], evm_chains: [], holder_deposits: [], wrapper_from_block: [], frog_funding: [],
    custody_caps: [{ collections, global_max_items: gItems ?? collections.reduce((a, c) => a + c.max_items, 0), global_max_value_wei: String(gValue ?? collections.reduce((a, c) => a + BigInt(c.max_value_wei), 0n)) }],
    outflow_limits: [{ ...Object.fromEntries(Object.entries(s.limits).map(([k, v]) => [k, k === "global_value_wei" ? String(v) : Number(v)])) }],
    eth_nft: [{ chain_id: 1n, relay_max_fee_bps: 300, deposits: true, floor: [], gas_pot_target_wei: [], logs_max_blocks: [],
      collections: L.collections.map((c) => ({ collection: c.collection, wrapper: "0xaa", module: "0xbb", max_purchase_wei: String(c.maxPurchaseWei), max_premium_bps: 500 })) }] });
};
writeFileSync("s2.hex", mk(2, 3));
writeFileSync("evil3.hex", mk(3, 12, 62, 340n * 10n ** 18n));
