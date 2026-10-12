/**
 * docs/phase8.md §4. A page that can show six states is only tested when all six are on the screen at once.
 *
 * Pure: it is handed a census of what the chain holds and returns what should be held back and what should be pushed. The
 * operator does the ordinary work; this only stops it finishing everything, and asks for the two states the operator would
 * never produce on its own (a purchase returned unspent, and one written off).
 */
export type PurchaseCensus = {
  vault: string;
  listingId: number;
  id: number;
  status: "declared" | "released" | "settled" | "cancelled" | "written-off";
  /** the receipt deadline has passed: a write-off is possible */
  overdue: boolean;
  /** already partly returned, so it is the one occupying that state */
  returned: boolean;
};

export type LotCensus = {
  listingId: number;
  acquisitionId: number;
  /** Distributor.Mode: 0 Unset, 1 Hold, 2 Never, 3 DrawWeighted, 4 DrawPerWallet, 5 Named, 6 ProRata */
  mode: number;
  /** Distributor.Stage: 0 None, 1 Open, 2 Pinned, 3 Resolved, 4 Claimed, 5 Delivered */
  stage: number;
  /** the winner has posted an address and the hour to correct it has passed, so a delivery could be recorded */
  deliverable: boolean;
};

export type Census = { purchases: PurchaseCensus[]; lots: LotCensus[] };

export type Holds = {
  /**
   * `${vault}:${id}` whose public wait must run its course, so a countdown is always on screen. Keyed by VAULT as well as id
   * because a purchase id counts from zero in every vault: purchase 1 of one listing is not purchase 1 of another, and a bare
   * number here held the wrong listing's purchase.
   */
  declarations: string[];
  /** `${vault}:${id}` that must stay unsettled, so "awaiting its receipt" is always occupied */
  releases: string[];
  /** `${listingId}:${acquisitionId}` whose challenge window must not be shortened */
  challenges: string[];
  /** `${listingId}:${acquisitionId}` whose delivery must not be recorded, so one goes overdue */
  deliveries: string[];
};

export type Push =
  | { type: "return-unspent"; vault: string; purchase: number; share: number }
  /** the receipt deadline is three days, which a staging chain never reaches: one purchase is aged so a write-off can happen at all */
  | { type: "age-for-write-off"; vault: string; purchase: number }
  | { type: "write-off"; vault: string; purchase: number };

export type Spread = { holds: Holds; pushes: Push[] };

const key = (l: { listingId: number; acquisitionId: number }) => `${l.listingId}:${l.acquisitionId}`;
/** A purchase id counts from zero in EVERY vault, so nothing may identify one by its number alone. */
export const purchaseKey = (p: { vault: string; id: number }): string => `${p.vault.toLowerCase()}:${p.id}`;
const lowest = <T extends { id: number }>(xs: T[]): T | undefined => xs.slice().sort((a, b) => a.id - b.id)[0];
const same = (a: PurchaseCensus | undefined, b: PurchaseCensus) => a !== undefined && purchaseKey(a) === purchaseKey(b);

/**
 * One item is held in each state that would otherwise empty, and never more than one: everything else must keep moving, or the
 * chain fills with stuck rows and the pages stop looking like a product in use.
 */
export function spread(c: Census): Spread {
  const declared = c.purchases.filter((p) => p.status === "declared");
  const released = c.purchases.filter((p) => p.status === "released");
  const settled = c.purchases.filter((p) => p.status === "settled");
  const holds: Holds = { declarations: [], releases: [], challenges: [], deliveries: [] };
  const pushes: Push[] = [];

  // a countdown on screen: the oldest declaration waits it out, the rest are let through
  const holdDecl = lowest(declared);
  if (holdDecl) holds.declarations.push(purchaseKey(holdDecl));

  // The rare states are claimed FIRST. A state nothing else can reach is worth more on screen than one the ordinary path
  // refills every few minutes, so "returned unspent" takes its purchase before "awaiting its receipt" may hold one.
  const live = released.filter((p) => !p.overdue);
  let returning: PurchaseCensus | undefined, aging: PurchaseCensus | undefined;
  if (!c.purchases.some((p) => p.returned)) {
    returning = live.slice().sort((a, b) => b.id - a.id)[0];        // the newest, so the oldest is still free to be held
    if (returning) pushes.push({ type: "return-unspent", vault: returning.vault, purchase: returning.id, share: 0.25 });
  }
  if (!c.purchases.some((p) => p.status === "written-off")) {
    const w = lowest(released.filter((p) => p.overdue));
    if (w) pushes.push({ type: "write-off", vault: w.vault, purchase: w.id });
    // Nothing is overdue, and nothing will be: a receipt has three days to arrive and staging is reset long before that. One
    // release is aged so the state can exist at all. Two live releases are enough to try it, even though holding one for
    // "awaiting its receipt" then has nothing left that pass: a write-off stays on the page for ever, while releases arrive
    // every few minutes, so the permanent state is the one worth having.
    else if (live.length >= 2) {
      aging = lowest(live.filter((p) => !same(returning, p)));
      if (aging) pushes.push({ type: "age-for-write-off", vault: aging.vault, purchase: aging.id });
    }
  }

  // "released, awaiting its receipt": hold one, but only once something has settled, or a cold chain never gets its first receipt
  const holdRel = lowest(live.filter((p) => !same(returning, p) && !same(aging, p)));
  if (holdRel && settled.length > 0) holds.releases.push(purchaseKey(holdRel));

  const draws = c.lots.filter((l) => l.mode === 3 || l.mode === 4);
  // a challenge window running: hold one open draw, once another has got past that stage
  const open = draws.filter((l) => l.stage === 1);
  if (open.length && draws.some((l) => l.stage > 1)) holds.challenges.push(key(open[0]));
  // an overdue delivery: hold one deliverable lot, once another has been delivered
  const deliverable = draws.filter((l) => l.stage === 4 && l.deliverable);
  if (deliverable.length && draws.some((l) => l.stage === 5)) holds.deliveries.push(key(deliverable[0]));

  return { holds, pushes };
}

/** What a creator should choose for the next unchosen lot, so every mode is on screen. Rune listings can only be pro rata. */
export const CHOICES_INSCRIPTION: readonly number[] = [3, 4, 5, 3, 1, 3];   // weighted, per-wallet, named, weighted, hold, weighted
export function choiceFor(isRune: boolean, chosenSoFar: number): number {
  return isRune ? 6 : CHOICES_INSCRIPTION[chosenSoFar % CHOICES_INSCRIPTION.length];
}

/**
 * Published test vectors, and taproot addresses the CONTRACT accepts: `Distributor.setDestination` runs every one through
 * `BtcAddress.isTaproot`, and test/StagingFixtureAddresses.t.sol pins these three strings against that library so the two
 * cannot drift apart. A winner on staging is a throwaway wallet, and the address it "chooses" must not be one a real person
 * could ever be paid at by accident.
 *
 * docs/phase11.md §7: BIP-350's `bc1pw508d6qejxtdg4y5r3zarvary0c5xw7k…` was here and had to go. It is valid bech32m and it is
 * a FORTY-byte version-1 program, which is not a taproot output, so the contract refused it — invisibly on two listings,
 * thirty stalled deliveries on 58. A taproot address is 62 characters and nothing else is one.
 */
export const FIXTURE_TAPROOT: readonly string[] = [
  "bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqzk5jj0",
  "bc1p5d7rjq7g6rdk2yhzks9smlaqtedr4dekq08ge8ztwac72sfr9rusxg3297",
  "bc1p5cyxnuxmeuwuvkwfem96lqzszd02n6xdcjrs20cac6yqjjwudpxqkedrcr",
];
export const taprootFor = (n: number): string => FIXTURE_TAPROOT[Math.abs(n) % FIXTURE_TAPROOT.length];
