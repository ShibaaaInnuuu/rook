import { mkdirSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { buildWorld, NOW, worldConfig } from "../../test/wrappedWorld.js";
import { parseConfig } from "../wrapped/config.js";
import { WrappedService } from "../wrapped/service.js";
import { CatalogService, loadSeed } from "./catalog.js";

/**
 * docs/v6/collections-catalog.md §5. Writes indexer/fixtures/catalog/: the catalogue for the site's labelled demo
 * (?demo=sample). No network is asked: every figure is the seed's dated record (what the sources answered when the seed was
 * written, flagged stale, with its date), and the two nests of test/wrappedWorld.ts (docs/v3/curve.md's worked examples) are
 * what is `live` — their coins invented, their floors the worked keeper cap. Nothing here is a price now.
 * Pictures: every entry carries the public URL the seed recorded — the marketplace's logo for the collection (the brand image
 * every aggregator shows), the ordinals content gateway or the token's metadata for its items — and the site loads them itself
 * in the fixture (there is no indexer to fetch through), with a mark where a URL is missing or fails. Nothing is copied in.
 *   npm run fixtures:catalog      then copy the folder to web-rook/public/fixtures/catalog (web-rook/test/copies.test.ts checks the copy)
 */
const NOTE = "FIXTURE. Nothing on this page is deployed. Every floor, count and volume is the seed's dated record of what a marketplace answered when the seed was written, shown with its date and flagged stale; "
  + "the two live nests (Bitcoin Frogs, Bored Ape Yacht Club) are docs/v3/curve.md's worked examples with invented coins. Pictures are the collections' real public images, loaded by your browser straight from where the seed recorded them (the marketplace's logo, the ordinals content gateway, the token's own metadata), since there is no indexer here to fetch them; a mark stands in where there is none. Nothing here is a price now, and nothing was bought.";
const here = new URL("./", import.meta.url).pathname;
const outDir = `${here}../../fixtures/catalog/`;
const generatedAt = Math.floor(Date.now() / 1000);

const w = buildWorld();
const wrappedConfig = parseConfig(worldConfig());
const wrapped = new WrappedService({ config: parseConfig({ ...worldConfig(), source: { kind: "fixture", note: NOTE } }), rpc: w.rpc, fetch: w.fetch, now: () => NOW });
const svc = new CatalogService({ seed: loadSeed(), fetch: null, wrapped, wrappedConfig, now: () => generatedAt, source: { kind: "fixture", note: NOTE }, usdPerEth: () => null });

mkdirSync(outDir, { recursive: true });
for (const f of readdirSync(outDir)) rmSync(outDir + f);
const files: Record<string, string> = {};
const put = (path: string, file: string, body: unknown) => { files[path] = file; writeFileSync(outDir + file, JSON.stringify(body, null, 2) + "\n"); };

const all = await svc.list();
put("/v1/rook/catalog", "catalog.json", all);
for (const e of all.entries) {
  const one = await svc.one(e.slug);
  put(`/v1/rook/catalog/${e.slug}`, `catalog-${e.slug.toLowerCase()}.json`, one);
  if (e.rook.kind === "live" && e.rook.slug !== e.slug) files[`/v1/rook/catalog/${e.rook.slug}`] = `catalog-${e.slug.toLowerCase()}.json`;
}
writeFileSync(outDir + "index.json", JSON.stringify({ kind: "catalog", note: NOTE, generatedAt, seedWrittenAt: loadSeed().writtenAt, files }, null, 2) + "\n");
console.log(`wrote ${Object.keys(files).length} answers (${all.entries.length} entries, ${all.entries.filter((e) => e.rook.kind === "live").length} live, ${all.entries.filter((e) => e.image).length} with a picture, ${all.entries.filter((e) => e.sample.some((i) => i.image)).length} with item pictures) to ${outDir}`);
