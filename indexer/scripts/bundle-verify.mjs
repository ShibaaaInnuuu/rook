// Decision P3 (docs/v8/proof-of-reserves.md §8): the CLI verifier as one self-contained file for npm (@rook/verify), built from the
// same src/v8 modules the indexer and the page run. `npm run verify:bundle`, then `npm pack ./verify-pkg` to inspect it. Publishing
// (and removing "private" from verify-pkg/package.json) is the operator's decision; nothing here publishes.
import { build } from "esbuild";
const out = new URL("../verify-pkg/rook-verify.mjs", import.meta.url).pathname;
await build({ entryPoints: [new URL("./rook-verify.ts", import.meta.url).pathname], bundle: true, platform: "node", format: "esm", target: "node20", outfile: out,
  banner: { js: "#!/usr/bin/env node\n// @rook/verify: built from indexer/src/v8 (docs/v8). Keyless; reads only." }, legalComments: "none", logLevel: "warning" });
console.log(`wrote ${out}`);
