import type { IntentViewV } from "../../src/rook/sol/canister";

/** An intent as the canister would serve it, with overrides (test/canister.test.ts, test/intent.test.ts). */
export const intentView = (over: Partial<IntentViewV> = {}): IntentViewV => ({
  id: 7n, kind: "mint", coll: 0, state: { Open: null }, address: "7UVimffxr9ow1uXYxsr4LHAcV58mLzhmwaeKvJ1pjLiE", token_account: [], recipient: "GGgXnNgtY4ySfz8PLAAwkoh8Ztj7p6jrM9towSaa6Wuy",
  refund_to: "GGgXnNgtY4ySfz8PLAAwkoh8Ztj7p6jrM9towSaa6Wuy", deposit: 0n, min_deposit: 50_000_000n, created_at: 1_700_000_000n, expires_at: 1_700_086_400n, funded_at: [], minted_units: 0n,
  paid_lamports: 0n, ticket: [], refunding: [], in_flight: [], note: "", attention: false, txs: [], ...over,
});
