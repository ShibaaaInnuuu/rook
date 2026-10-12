// The site's Solana configuration. WRITTEN BY ops/solana/site-config.mjs from the launch's day1-plan.json; edit by hand only the
// fields that script leaves alone (rpcUrl, cloudinary). docs/v9/canister-only-solana.md §16: the custodian opens each collection the
// first time someone launches a coin on it (the canister names its config then); docs/v9/launch-day1-solana.md §3 for the plan file.
//
// PLACEHOLDER UNTIL LAUNCH: the collection DBC configs (one per listing, 26, each quoted in the collection's wX with its threshold in
// whole wX: src/rook/sol/graduation.ts) and the genesis ROOK coin (SOL-paired, 85 SOL, its fee claimer the custodian's wallet) are
// created by the custodian and their addresses are only known after that. While they are empty strings the site says so ("opens at
// launch") and builds no transaction. After the opening:
//   node ops/solana/site-config.mjs --plan ~/rook-keys/day1/day1-plan.json [--check-rpc <mainnet RPC>]
// then review the diff of this file, `npm test`, and rebuild. Canister-only, the collection configs may stay empty (the canister names
// each when it opens it, and the board reads them from it): `rook`'s three addresses alone, written by hand once the operator has
// launched ROOK, make the config ready (src/rook/sol/siteConfig.ts) and put ROOK on the home page's card and the board. Rook deploys no program of its own: the holder raffle is read from the
// canister (src/rook/sol/raffle.ts, src/config/canister.ts names it), not from program accounts.
//
// rpcUrl: the browser reads the chain from here (getProgramAccounts, getMultipleAccounts, simulateTransaction, sendTransaction).
// The default is PublicNode's keyless endpoint: it answers browsers (CORS) and serves account reads, simulations and sends, but it
// REFUSES getProgramAccounts without a token, so the coins board stays empty on it (the page says why). api.mainnet-beta.solana.com
// refuses browser origins altogether (HTTP 403). Production must use a DOMAIN-RESTRICTED Helius key (a key in a browser is public, so
// restrict it to userook.fun and www.userook.fun in Helius's dashboard): set the build argument VITE_SOLANA_RPC, or put it here.
// cloudinary: an UNSIGNED upload preset (Cloudinary → Settings → Upload → Upload presets → Unsigned), so the Launch page can upload a
// coin's image and metadata JSON. No secret goes here: an unsigned preset is public by design; leave both empty to turn upload off.
export const SOLANA_SITE_CONFIG = {
  "cluster": "mainnet-beta",
  "rpcUrl": "https://solana-rpc.publicnode.com",
  "generatedFrom": null,
  "collections": [
    { "slug": "milady", "listingId": 0, "listing": "", "claimer": "", "config": "" },
    { "slug": "azuki", "listingId": 1, "listing": "", "claimer": "", "config": "" },
    { "slug": "mayc", "listingId": 2, "listing": "", "claimer": "", "config": "" },
    { "slug": "pudgypenguins", "listingId": 3, "listing": "", "claimer": "", "config": "" },
    { "slug": "bayc", "listingId": 4, "listing": "", "claimer": "", "config": "" },
    { "slug": "doodles", "listingId": 5, "listing": "", "claimer": "", "config": "" },
    { "slug": "moonbirds", "listingId": 6, "listing": "", "claimer": "", "config": "" },
    { "slug": "clonex", "listingId": 7, "listing": "", "claimer": "", "config": "" },
    { "slug": "lilpudgys", "listingId": 8, "listing": "", "claimer": "", "config": "" },
    { "slug": "checks", "listingId": 9, "listing": "", "claimer": "", "config": "" },
    { "slug": "identitymd", "listingId": 10, "listing": "", "claimer": "", "config": "" },
    { "slug": "bitcoin-frogs", "listingId": 11, "listing": "", "claimer": "", "config": "" },
    { "slug": "nodemonkes", "listingId": 12, "listing": "", "claimer": "", "config": "" },
    { "slug": "omb", "listingId": 13, "listing": "", "claimer": "", "config": "" },
    { "slug": "bitcoin-puppets", "listingId": 14, "listing": "", "claimer": "", "config": "" },
    { "slug": "bitcoin-punks", "listingId": 15, "listing": "", "claimer": "", "config": "" },
    { "slug": "natcats", "listingId": 16, "listing": "", "claimer": "", "config": "" },
    { "slug": "runestone", "listingId": 17, "listing": "", "claimer": "", "config": "" },
    { "slug": "bitcoin-weirdos", "listingId": 18, "listing": "", "claimer": "", "config": "" },
    { "slug": "cents", "listingId": 19, "listing": "", "claimer": "", "config": "" },
    { "slug": "rsic", "listingId": 20, "listing": "", "claimer": "", "config": "" },
    { "slug": "pizza-ninjas", "listingId": 21, "listing": "", "claimer": "", "config": "" },
    { "slug": "quantum-cats", "listingId": 22, "listing": "", "claimer": "", "config": "" },
    { "slug": "wizards-of-ord", "listingId": 23, "listing": "", "claimer": "", "config": "" },
    { "slug": "taproot-wizards", "listingId": 24, "listing": "", "claimer": "", "config": "" },
    { "slug": "bitcoin-wizards", "listingId": 25, "listing": "", "claimer": "", "config": "" }
  ],
  "rook": {
    "config": "",
    "mint": "",
    "pool": ""
  },
  "cloudinary": {
    "cloudName": "",
    "uploadPreset": ""
  }
};
