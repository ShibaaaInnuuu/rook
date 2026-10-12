// The devnet rehearsal's floor evidence for one Ethereum collection, built exactly as the runner builds it (ops/src/ethnft.ts: the
// cheapest Seaport 1.6 listings from OpenSea, each encoded as fulfillAdvancedOrder calldata for the canister's EVM address).
// Run from ops/:  OPENSEA_API_KEY=… node --import tsx rehearsal/sol-devnet/evidence.ts <openseaSlug> <collection 0x…> <canister evm 0x…> <out.json>
// The key is read from the environment only; it is never written, printed or logged.
import { writeFileSync } from "node:fs";
import { bestListings, evidenceListings, type EthNftDeps } from "../../src/ethnft.ts";

const [slug, collection, us, out] = process.argv.slice(2);
if (!slug || !/^0x[0-9a-fA-F]{40}$/.test(collection ?? "") || !/^0x[0-9a-fA-F]{40}$/.test(us ?? "") || !out) {
  console.error("usage: evidence.ts <openseaSlug> <collection 0x…> <canister evm 0x…> <out.json>");
  process.exit(2);
}
if (!process.env.OPENSEA_API_KEY) { console.error("OPENSEA_API_KEY is not set"); process.exit(2); }
const d = { fetch, openseaKey: () => process.env.OPENSEA_API_KEY, timeoutMs: 20_000, log: (lv: string, ev: string, x: unknown) => console.error(lv, ev, JSON.stringify(x)) } as unknown as EthNftDeps;
const ls = await bestListings(d, slug, 8);
// signed through fulfillment_data where the best-listings answer has no signature (ethnft.ts evidenceListings)
const listings = await evidenceListings(d, slug, "ethereum", us);
writeFileSync(out, JSON.stringify({ collection: collection.toLowerCase(), listings }));
const eth = ls.map((l) => Number(BigInt(l?.price?.current?.value ?? "0")) / 1e18);
console.log(`${listings.length} listings of ${slug}: ${eth.map((x) => x.toFixed(3)).join(", ")} ETH -> ${out}`);
