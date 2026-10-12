import { readFileSync } from "node:fs";
import { CATALOGUE_PATH, OUR_CHAINS, STAGING_FIXTURE_MEMBERS, listingCalls, plannedListings, type Catalogue } from "./listings.js";

/**
 * docs/phase11.md §5. The same planned listings, printed as calldata instead of sent.
 *
 *   npm run listings:calldata -- --chain 4663 [--registry 0x...] [--catalogue path]
 *
 * On a real chain the owner is a multisig, so nothing here sends anything: it prints the exact `listInscriptions` /
 * `listRune` calls for a human to review and the multisig to sign, one per listing, and REFUSES to pad a membership root
 * with staging fixture members. What a real listing still needs beyond this calldata is printed with it, every time.
 */
const arg = (name: string, fallback?: string): string | undefined => {
  const i = process.argv.indexOf(`--${name}`);
  return i > 0 && process.argv[i + 1] ? process.argv[i + 1] : fallback;
};

export function printCalldata(argv: { chainId: number; registry?: string; catalogue: Catalogue; out?: (s: string) => void }): void {
  const out = argv.out ?? ((s: string) => console.log(s));
  const ours = OUR_CHAINS.includes(argv.chainId);
  const plan = plannedListings(argv.catalogue, { chainId: argv.chainId, fixtureMembers: ours ? STAGING_FIXTURE_MEMBERS : 0 });
  const calls = listingCalls(plan);
  out(`# ${calls.length} listings for chain ${argv.chainId}, from ${plan.source} (fetched ${plan.fetchedAt})`);
  out(`# ranked by: ${argv.catalogue.rankedBy ?? "nothing this file records, so its order is not a ranking"}`);
  out(`# weights from: ${plan.weightBasis}`);
  out(`# registry: ${argv.registry ?? "<the registry address on that chain>"}`);
  out(`# owner: the multisig. Nothing here sends anything.`);
  out("#");
  if (ours) out(`# Our own chain: every collection's root is padded with ${STAGING_FIXTURE_MEMBERS} fixture ids that name nothing.`);
  else {
    out("# A REAL chain. Before any of this is signed (docs/phase11.md §4):");
    out("#   1. each membershipRoot below is built from the 3-4 inscription ids the catalogue holds, NOT from the collection's");
    out("#      membership. Rebuild it over the collection's complete, verified item list and publish that list.");
    out("#   2. each btcVault below is a marker string nobody can be paid at. Replace every one with a real taproot address");
    out("#      from the custody signer, checked against its key and test-received first.");
    out("#   3. the floors that caps are posted from must come from a live source, not from this dated catalogue.");
    out("#   4. runes carry no floor here, so a rune vault can never declare a purchase until one is supplied.");
    out("#   5. this membership is fifty collections and fifty runes on the two bases the line above states — the runes by");
    out("#      the seven days before the fetch, the collections by lifetime volume, because no marketplace states a");
    out("#      collection's week (docs/phase9.md §11.3). The runes' fifty is a different fifty a week later, and these");
    out("#      are the vaults that would exist for good.");
  }
  out("#");
  for (const c of calls) out(`${c.functionName}\t${c.ref}\t${JSON.stringify(c.name)}\t${c.calldata}`);
  for (const s of plan.skipped) out(`# skipped\t${s.slug}\t${s.why}`);
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const chainId = Number(arg("chain", "0"));
  if (!Number.isInteger(chainId) || chainId <= 0) { console.error("usage: npm run listings:calldata -- --chain <id> [--registry 0x...] [--catalogue <path>]"); process.exit(2); }
  const path = arg("catalogue", new URL(`../../${CATALOGUE_PATH}`, import.meta.url).pathname)!;
  printCalldata({ chainId, registry: arg("registry"), catalogue: JSON.parse(readFileSync(path, "utf8")) as Catalogue });
}
