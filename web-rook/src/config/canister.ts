// The Rook canister the site reads its books from and opens intents with (docs/v9/canister-only-solana.md §6, §10.1). WRITTEN BY HAND
// after the install (docs/v9/ceremony-canister-only.md): the canister id of the books home. Empty means the site has no canister yet:
// the intent desk, the backing panels and the Reserves page then say so and call nothing. The build argument VITE_CANISTER_ID
// overrides the id; VITE_IC_HOST the boundary node (a local replica for a rehearsal). Nothing here is a secret: every call the site
// makes is a public query or an anonymous update anyone may make.
export const CANISTER_SITE_CONFIG = {
  "canisterId": "",
  "icHost": "https://icp-api.io"
};
