import { readFileSync, writeFileSync } from "node:fs";
import { DEFAULT_ETH_RPCS } from "../src/v8/selfcheck.js";
import { driftOf, takeSnapshot } from "../src/v8/snapshotJob.js";
import type { MetadataSnapshot } from "../src/v8/types.js";

/**
 * docs/v8/redemption-draw.md §2.1: take a collection's metadata snapshot once, at deploy, and print its SHA-256 to pin (in the
 * launch file, before the first draw); or re-read it against the pinned file and write the drift (shown on the page, never applied).
 *   npx tsx scripts/snapshot-metadata.ts --collection 0x… --name "Milady Maker" --first 0 --count 10000 --out milady.json [--eth-rpc URL]
 *   npx tsx scripts/snapshot-metadata.ts --diff milady.json --out milady.drift.json [--eth-rpc URL]
 */
const argv = process.argv.slice(2), one = (k: string) => { const i = argv.indexOf(`--${k}`); return i >= 0 ? argv[i + 1] : undefined; };
const ethRpc = one("eth-rpc") ?? DEFAULT_ETH_RPCS[0], out = one("out"), now = Math.floor(Date.now() / 1000);
if (!out) { console.error("--out is required"); process.exit(64); }
if (one("diff")) {
  const pinned = JSON.parse(readFileSync(one("diff")!, "utf8")) as MetadataSnapshot;
  const drift = await driftOf(pinned, { ethRpc, fetch, now });
  writeFileSync(out, JSON.stringify(drift, null, 1) + "\n");
  console.log(`${drift.changed.length} trait changes, ${drift.unreadable} unreadable, against the snapshot of ${new Date(pinned.takenAt * 1000).toISOString()}`);
} else {
  const collection = one("collection"), first = Number(one("first") ?? 0), count = Number(one("count"));
  if (!collection || !/^0x[0-9a-fA-F]{40}$/.test(collection) || !Number.isInteger(count) || count <= 0) { console.error("--collection and --count are required"); process.exit(64); }
  const { snapshot, sha256 } = await takeSnapshot({ collection: collection.toLowerCase() as `0x${string}`, name: one("name") ?? collection, tokenIds: Array.from({ length: count }, (_, i) => String(first + i)), ethRpc, fetch, now });
  writeFileSync(out, JSON.stringify(snapshot) + "\n");
  console.log(`${snapshot.tokens.length} tokens; sha256(canonical JSON) = ${sha256}  <- pin this before the first draw`);
}
