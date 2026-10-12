/**
 * The Etch indexer's HTTP API, as types. One file, no imports: copy it as it is.
 * docs/phase4.md has the formulas; indexer/API.md has example responses.
 *
 * Conventions: every ETH amount is wei as a decimal string (`Wei`). Rune amounts are raw units as decimal strings. Times are
 * unix seconds. `null` means "not known" and always comes with a reason where a reason exists; nothing is ever estimated silently.
 * `live` blocks are read from the chain at request time (cached ~15 s) and may be null when the node cannot be reached;
 * everything outside `live` comes from indexed, confirmed events.
 */
export type Wei = string;
export type Address = `0x${string}`;
export type Hex = `0x${string}`;
export type ListingKind = "inscriptions" | "rune";

export type Health = {
  ok: boolean;
  /** false: there is no deployment file for this chain, the follower idles, and every list is empty. Nothing is invented. */
  deployment: boolean;
  chainId: number | null;
  demo: boolean;
  cursor: number | null;
  head: number | null;
  lagBlocks: number | null;
  confirmations: number;
  events: number;
  lastPassAt: number | null;
  lastError: { at: number; kind: string } | null;
  reorgs: number;
  version: string;
};

export type Stats = {
  coins: number;
  listings: number;
  claimedWei: Wei;
  toVaultsWei: Wei;
  toTreasuryWei: Wei;
  releasedWei: Wei;
  returnedWei: Wei;
  /** released − returned, for purchases that were settled */
  spentWei: Wei;
  writtenOffWei: Wei;
  acquisitions: { inscriptions: number; runeReceipts: number };
  purchases: { declared: number; released: number; settled: number; cancelled: number; writtenOff: number };
  draws: { open: number; pinned: number; resolved: number; claimed: number; delivered: number; voided: number; cancelled: number };
  distributions: { open: number; rolled: number };
  deliveries: { done: number; overdue: number; waiting: number };
  payouts: { requested: number; recorded: number; overdue: number };
};

export type CoinSummary = {
  address: Address;
  name: string | null;
  symbol: string | null;
  creator: Address;
  router: Address;
  curve: Address;
  listingId: number;
  listingName: string | null;
  launchedAt: number | null;
  launchedBlock: number;
  /** Pons's record of the creator tax, read once at launch. null if that read failed. */
  creatorTaxBps: number | null;
  /**
   * The logo the creator gave at launch, byte for byte as the chain has it — usually an `ipfs://` URI, sometimes `https://`,
   * and `data:image/…` on staging. null when the coin has none, when the launch transaction could not be read, or when the
   * string could not be an image (docs/phase10.md). Render it only as the `src` of an image, never as markup, and choose an
   * IPFS gateway yourself: this field names no gateway.
   */
  logoUrl: string | null;
  /**
   * docs/phase12.md §3. The coin's whole supply in its own base units (18 decimals for every Pons coin), read from
   * `totalSupply()` once at launch: Pons mints all of it there, so it is a constant per coin and not a series. Market cap is
   * `supplyWei / 1e18 × price`, and without this number it cannot be computed at all. null when the read has not succeeded
   * yet — the follower asks again, a few coins a pass — never a guessed round billion.
   */
  supplyWei: Wei | null;
  /**
   * docs/phase13.md §5. How many addresses hold a positive balance of this coin, **the plumbing excluded**: not the zero
   * address, not the coin, its curve or its router, not Pons's factory (which is minted 2/7 of every supply for the pool a
   * graduated coin gets), not the v4 PoolManager, not its hook. `/v1/coins/:address/holders` names each of those and says
   * what it holds.
   *
   * null means **not known yet**. `Transfer` indexing began at whatever block the follower had reached, and the history
   * before it is fetched once, a bounded piece a pass; until that meets the live edge a count would be the tail of a coin's
   * history rather than its history. It is never a partial count presented as a whole one.
   */
  holders: number | null;
  /**
   * docs/phase14.md §4. The pair asset traded in this coin over the last 24 hours, in wei: the sum of the `quote` leg of
   * every trade in the window, at either venue, exactly as the chain recorded it — on a buy what the buyer sent, on a sell
   * what the seller received, neither adjusted for Pons's fee or the creator tax (a trade keeps both its legs, so a caller
   * who wants a different figure divides them itself).
   *
   * It is the same quantity the chart reports: `sum(candle.v)` over `range=1d` and `volume24hWei / 1e18` count the same
   * trades. It is **never null** — a coin nobody traded in a day has volume `0`, which is a fact and not a gap — and it is
   * not `fees24hWei`, which sums *harvests*: a coin that traded hard and has not been swept has fees of zero and volume of
   * everything it traded.
   */
  volume24hWei: Wei;
  /** toVault + toTreasury over every harvest */
  lifetimeFeesWei: Wei;
  toVaultWei: Wei;
  toTreasuryWei: Wei;
  fees24hWei: Wei;
  fees7dWei: Wei;
  harvests: number;
  lastHarvestAt: number | null;
};

export type VolumeUntilNextBuy = {
  /** wei of trading volume in this coin alone that would bring its listing to its next declaration. null: see `reason`. */
  volumeNeededWei: Wei | null;
  reason: null | "no-live-cap" | "listing-paused" | "listing-blocked" | "tax-unknown" | "live-unavailable";
  /** true when the listing can already declare: the number is 0 because nothing more is needed */
  ready: boolean;
  inputs: {
    targetWei: Wei | null;
    capWei: Wei | null;
    capExpiry: number | null;
    minDeclarationWei: Wei;
    minEntryWei: Wei;
    spendableWei: Wei | null;
    coinWaitingWei: Wei | null;
    coinQueued: boolean | null;
    /** already earned and one `harvest()` away: the router's balance plus its Pons escrow balance; 80% of it is counted */
    harvestableWei: Wei | null;
    depositNeededWei: Wei | null;
    vaultShareBps: 8000;
    /** 70 + creatorTaxBps: the measured share of volume that reaches a router (docs/phase1.md), in basis points */
    takeBps: number | null;
  };
  assumptions: string[];
};

export type PendingFees = {
  graduated: boolean | null;
  /** fees already in Pons's escrow for the router, claimable by anyone calling harvest() */
  escrowWei: Wei | null;
  routerBalanceWei: Wei | null;
  /** after graduation: the hook's pending buckets. ETH side in wei; coin side in the coin's own units, NOT priced. */
  pool: null | { poolId: Hex; ethFeesWei: Wei; ethCreatorTaxWei: Wei; coinFees: string; coinCreatorTax: string; operatorOnly: boolean };
  /** before graduation fees accrue inside the curve; no getter for that amount has been verified */
  curvePending: null;
  curvePendingReason: string;
  error: string | null;
};

export type HarvestRow = { at: number | null; block: number; tx: Hex; claimedWei: Wei; toVaultWei: Wei; toTreasuryWei: Wei };

export type CoinDetail = CoinSummary & {
  harvestsRecent: HarvestRow[];
  tokensForwarded: { at: number | null; tx: Hex; token: Address; amount: string }[];
  treasuryDeferredWei: Wei;
  /** each acquisition some of whose ETH this coin paid */
  acquisitions: { listingId: number; acquisitionId: number; purchaseId: number; chargedWei: Wei; shareBps: number; assetRef: string; amount: string }[];
  volumeUntilNextBuy: VolumeUntilNextBuy;
  live: null | { unspentWei: Wei | null; queued: boolean | null; pending: PendingFees };
};

export type Declaration = {
  purchaseId: number;
  status: "declared" | "released" | "settled" | "cancelled" | "written-off";
  maxEthWei: Wei;
  quantity: string;
  capRef: string;
  declaredAt: number | null;
  releasableAt: number;
  /** seconds until anyone may release it; 0 once it may be */
  releasableIn: number | null;
  releasedAt: number | null;
  releasedWei: Wei | null;
  purchaser: Address | null;
  /** the receipt deadline, once released */
  deadline: number | null;
  overdue: boolean;
  returnedWei: Wei;
  writtenOffWei: Wei | null;
  bondSlashedWei: Wei;
  charges: { coin: Address; amountWei: Wei }[];
  tx: Hex;
};

export type Verify = {
  /** links for a person */
  links: { label: string; url: string }[];
  /** the same thing for a program: what to fetch and what it should show. We make none of these calls ourselves. */
  check:
    | { kind: "inscription"; txid: string; inscriptionId: string; address: string | null; expect: "the inscription's current owner is `address`, until it is delivered" }
    | { kind: "rune"; txid: string; runeId: string; address: string | null; expectAtLeast: string; indexerPath: string; expect: "the address's balance of the rune is at least the undelivered total" };
  note: string;
};

export type Acquisition = {
  listingId: number;
  acquisitionId: number;
  purchaseId: number;
  kind: ListingKind;
  coin: Address;
  coinSymbol: string | null;
  /** the inscription id, or the rune id */
  assetRef: string;
  /** "1" for an inscription; raw rune units otherwise */
  amount: string;
  /** big-endian hex as Bitcoin explorers print it, without 0x */
  btcTxid: string;
  /**
   * The sats the receipt recorded for the PURCHASE, not for this row.
   *
   * A rune settle records one acquisition per paying coin out of a single Bitcoin transaction, and every one of those rows
   * carries the same `btcTxid` and the same `satsPaid` (`AssetVault.settleRune`). Measured in the committed recording,
   * purchase 13 of listing 1 is five rows, one transaction, `satsPaid: 1000` printed five times. **Amounts sum across those
   * rows; sats do not.** Group by `purchaseId`, or by `btcTxid`, before adding this up — or do not add it up.
   */
  satsPaid: string;
  settledAt: number | null;
  tx: Hex;
  /**
   * docs/phase14.md §8.4. What the purchase this receipt came out of cost the vault — `released − returned`, in wei — and
   * how many receipts that one purchase produced.
   *
   * The vault charges coins **per purchase, not per item**: one inscription purchase in the recording receipted three
   * inscriptions, and one rune purchase receipted five rows. Both numbers are served and **neither is divided by the other
   * here**, because the contract's own division is by the *declared* quantity (`settleInscription` assigns
   * `maxEth / quantity`) and a short settle makes that a different number from what an item actually cost. A page that
   * wants a per-item figure has both numbers and must say which one it drew.
   */
  purchaseSpentWei: Wei;
  purchaseAcquisitions: number;
  delivery: {
    mode: "unset" | "hold" | "never" | "draw-weighted" | "draw-per-wallet" | "named" | "pro-rata";
    state: "undecided" | "held" | "never-delivered" | "draw-open" | "draw-pinned" | "draw-resolved" | "won" | "delivered" | "voided" | "cancelled" | "distribution-open" | "distribution-rolled";
    winner: Address | null;
    destination: string | null;
    deliveredTxid: string | null;
    overdue: boolean;
    secondsToDeliver: number | null;
  };
  verify: Verify;
};

export type ListingSummary = {
  id: number;
  kind: ListingKind;
  name: string;
  /** the collection slug, or the rune id */
  ref: string;
  divisibility: number | null;
  vault: Address;
  btcVault: string | null;
  membershipRoot: Hex | null;
  paused: boolean;
  coins: number;
  depositedWei: Wei;
  acquisitions: number;
  capWei: Wei | null;
  capExpiry: number | null;
  capExpired: boolean | null;
  /**
   * docs/phase14.md §8.4. When the cap that is current now was posted, as a unix second, or null where none ever was.
   *
   * A cap lasts one hour from the block that posted it (`AssetVault.CAP_TTL`), so "expires in 41 minutes" and "posted 19
   * minutes ago" are the same hour told from its two ends, and a reader who is deciding whether to trust a keeper's word
   * wants the second one. It is the `ts` of the same row `capWei` and `capExpiry` already come from; null where that block
   * carried no time.
   */
  capPostedAt: number | null;
  openDeclarations: number;
  /**
   * docs/v2/vault-v2.md. Which vault this listing got, from the chain's own `AutoBuyerNamed` event rather than inferred
   * from whether some other field is null.
   *
   * `false` is v1: ETH is released to a person the registry names, against a bond they posted, and a second key vouches for
   * the Bitcoin that comes back. `true` is v2: ETH is released only to `buyer`, an address fixed when the vault was built,
   * which is the EVM address of a program whose signing key is held by a subnet and by no person. There is no bond in v2 —
   * `maxInFlightWei` is the ceiling that replaces it — and `bondWei` and friends on the detail are null throughout.
   */
  autonomous: boolean;
  /** the autonomous buyer's address, or null on a v1 listing */
  buyer: Address | null;
  /** the most a v2 vault may have out and unsettled at once, or null on a v1 listing */
  maxInFlightWei: Wei | null;
};

/**
 * docs/phase14.md §8.3. What a vault has bought, and what of it the vault is still holding.
 *
 * An inscription is one of a kind and a rune is an amount, so there is no single figure that counts both: the reference
 * page this answers says "263 NFTs swept", and for a rune listing the equivalent sentence is a quantity, not a count of
 * receipts. Vault 1 of the committed recording has 26 receipts carrying 371,165.08 DEMO•RUNE between them, and 26 is not
 * what it bought. So the shape carries the count for one kind and the quantity for the other, and the half that does not
 * apply is `null` rather than zero.
 *
 * `runeUnits` are **raw units at the listing's own divisibility**, as decimal strings, exactly as everything else about a
 * rune is here. `held` is what the receipts brought in less what has been paid out on a claim and less what a resolved draw
 * delivered — the same arithmetic `Verify.expectAtLeast` hands a runes indexer, computed once so the page and that hint
 * cannot disagree.
 */
export type Holdings = {
  /** receipts in all: one per inscription, and one per paying coin for a rune settle */
  receipts: number;
  /** lots handed on: a draw's winner took delivery. It is a count of lots for either kind. */
  delivered: number;
  /** inscriptions the vault still holds, or null on a rune listing, where a count of receipts is not a quantity */
  inscriptionsHeld: number | null;
  /** raw rune units, or null on an inscriptions listing */
  runeUnits: { received: string; delivered: string; paidOut: string; held: string } | null;
};

export type ListingDetail = ListingSummary & {
  live: null | {
    balanceWei: Wei | null; attributedWei: Wei | null; waitingWei: Wei | null; reservedWei: Wei | null; spendableWei: Wei | null;
    bondWei: Wei | null; outstandingWei: Wei | null; requiredBondWei: Wei | null; blockedByPurchase: number | null; error: string | null;
  };
  /** bond ÷ outstanding in basis points; null when nothing is outstanding */
  bondCoverageBps: number | null;
  bondEvents: { at: number | null; tx: Hex; type: "posted" | "withdrawn" | "slashed"; who: Address | null; amountWei: Wei }[];
  capHistory: { at: number | null; capWei: Wei; expiry: number }[];
  declarations: Declaration[];
  acquisitionsRecent: Acquisition[];
  /**
   * docs/phase14.md §8.4. This vault's money out, over every purchase it has ever made — not over the page of
   * `declarations` above, which is capped at 100 rows and would make a total quietly wrong on the first busy vault.
   *
   * `releasedWei` is every release; `returnedWei` every return of what a purchase did not need. `spentWei` is
   * `released − returned` over **settled** purchases only, which is the same definition `/v1/stats` uses for the whole
   * protocol, so a vault's figure and the protocol's are the same arithmetic. ETH released and not yet settled is in the
   * purchaser's hands and is neither spent nor the vault's: it is `live.outstandingWei`, and `writtenOffWei` below is what
   * never came back and no bond covered.
   */
  releasedWei: Wei;
  returnedWei: Wei;
  spentWei: Wei;
  holdings: Holdings;
  writtenOffWei: Wei;
  timeToDeliver: { count: number; medianS: number | null; maxS: number | null };
  minimumPayout: { units: string; expiry: number; expired: boolean } | null;
  pending: { root: { value: Hex; readyAt: number } | null; btcVault: { value: string; readyAt: number } | null };
  coinsList: { address: Address; symbol: string | null; depositedWei: Wei }[];
};

export type DrawSummary = {
  id: string;
  listingId: number;
  acquisitionId: number;
  type: "draw" | "distribution";
  state: Acquisition["delivery"]["state"];
  mode: Acquisition["delivery"]["mode"];
  assetRef: string | null;
  openedAt: number | null;
  challengeEnds: number | null;
  challengeEndsIn: number | null;
  root: Hex | null;
  uri: string | null;
  /** tickets for a draw; rune units for a distribution */
  total: string | null;
};

export type DrawDetail = DrawSummary & {
  /** the exact command that recomputes `root` from the chain, to run during the challenge window */
  verifySnapshot: { tool: "tools/snapshot"; command: string | null; note: string };
  draw: null | {
    round: number | null; pinnedAt: number | null; drandUrl: string | null; winningTicket: string | null; resolvedAt: number | null;
    winner: Address | null; destination: string | null; destinationSetAt: number | null; deliveredTxid: string | null; deliveredAt: number | null;
    overdue: boolean; voided: { unclaimedTicket: string; at: number | null } | null;
  };
  distribution: null | { claimedUnits: string; claims: number; rolledOver: { coin: Address; amount: string; at: number | null } | null };
  timeline: { at: number | null; block: number; tx: Hex; event: string; detail: Record<string, string | number | boolean | null> }[];
};

export type Page<T> = { items: T[]; total: number; limit: number; offset: number };
export type ApiError = { error: string; status: number };

// ---- docs/phase10.md: a coin's trades, and the chart a page draws from them

/**
 * One trade in a coin, on Pons's bonding curve before graduation and in the coin's Uniswap v4 pool after it.
 *
 * `id` is "block:logIndex", the trade's place in the chain and the cursor for the page after it.
 * `quoteWei` and `coinWei` are the trader's own two legs, exactly as the chain recorded them: wei of the pair asset, and the
 * coin's own base units (18 decimals for every Pons coin).
 * `price` is the number a chart draws: **the pair asset per one whole coin**, a small JSON number like 3.1e-9. It is the
 * CURVE-side price — the quote that reached the curve over the coins that left it — so a buy and the sell after it are
 * continuous. A page that wants the price actually paid divides `quoteWei` by `coinWei` itself.
 * `feeWei` is Pons's fee and `taxWei` the creator's tax, both taken out of the quote leg on either side. Both are null for a
 * pool trade: there the hook charges outside the `Swap` event, so the event does not carry them and this API does not guess.
 * `trader` is the buyer or seller on the curve; in a pool it is the SWAP ROUTER the trade came through, not the person.
 */
export type Trade = {
  id: string;
  side: "buy" | "sell";
  venue: "curve" | "pool";
  quoteWei: Wei;
  coinWei: string;
  feeWei: Wei | null;
  taxWei: Wei | null;
  price: number;
  trader: Address;
  recipient: Address | null;
  tx: Hex;
  block: number;
  logIndex: number;
  at: number | null;
};

/**
 * A page of a coin's trades, newest first. `next` is the cursor for the page after it, or null at the end.
 * Pass it back as **`?before=`** — that is the parameter's name, and an unknown one is ignored rather than refused, so
 * asking with any other spelling silently serves page one again (docs/phase13.md §1).
 */
export type TradePage = { items: Trade[]; next: string | null; total: number };

// ---- docs/phase13.md: who holds a coin

/** Why an address is left out of a coin's holder list. Each is a contract of ours, of Pons's, or of Uniswap's. */
export type HolderExclusion = "coin" | "curve" | "router" | "pons-factory" | "pool-manager" | "hook";

/**
 * One holder of a coin: an **address**, which is not a person — one trader with three wallets is three of these, and a
 * custodian holding for ten thousand people is one.
 *
 * `balanceWei` is the exact balance in the coin's own base units (18 decimals for every Pons coin), summed from the coin's
 * `Transfer` logs and from nothing else. `share` is that as a **fraction of `totalSupply`** (0.15206112 is 15.2%), or null
 * when the supply has not been read — never a zero, and never a percentage of a guessed denominator. `rank` is over the
 * whole list, not the page, so the first row of page two is 26.
 */
export type Holder = { rank: number; address: Address; balanceWei: string; share: number | null; isCreator: boolean };

/**
 * A page of a coin's largest holders, and what the count is made of.
 *
 * `holders` is the number of addresses holding a positive balance with `excluded` left out, and is the same number as
 * `CoinSummary.holders`; it is null while `complete` is false. `excluded` carries every plumbing address that currently
 * holds something, with the reason it is not counted — on staging the curve holds up to 57% of a young coin's supply and
 * Pons's factory holds exactly 2/7 of every one, so a list that quietly dropped them would leave a reader unable to see
 * where the supply is.
 *
 * `complete` is false while the transfers from before indexing began are still being fetched (docs/phase13.md §4).
 * `scannedThrough` is the block that history is complete through and `transfersFrom` the next one it will fetch; both are
 * null on a database that has never followed a chain, where nothing was skipped because nothing was ever indexed.
 */
export type HolderPage = {
  coin: Address;
  holders: number | null;
  supplyWei: Wei | null;
  creator: Address;
  complete: boolean;
  scannedThrough: number | null;
  transfersFrom: number | null;
  items: Holder[];
  excluded: { address: Address; why: HolderExclusion; balanceWei: string; share: number | null }[];
  total: number; limit: number; offset: number;
};

/** One candle. `t` is its opening second, `v` the pair asset traded in it, `n` the trades in it. */
export type Candle = { t: number; o: number; h: number; l: number; c: number; v: number; n: number };

/**
 * A coin's chart. `candles` is in chain order, oldest first, and a bucket nobody traded in is LEFT OUT rather than filled:
 * nothing here interpolates, and a coin that has never traded gets an empty array and `tradesAllTime: 0`.
 * `range` is what a reader picks and what a page should ask for: "5m", "1h", "6h", "1d" or "all". Each one names a WINDOW and
 * derives its own bucket from it, so a caller can never land on one candle per trade — the fault docs/phase12.md §1 exists to
 * remove. `seconds` is the bucket that was used and `span` the window, both in seconds, both always reported.
 * `res` is the older parameter, still accepted and still served: "trade" (every trade at its own second, over 6 hours), "5m"
 * (24 hours) or "1h" (7 days). When a `range` was asked for, `range` is that range and `res` is only the nearest legacy label,
 * kept so a client written against the old shape still parses this one; read `range` and `seconds`, never `res`, for what was
 * actually drawn. When the older `res` was asked for, `range` is null.
 * `marks` are the largest trades in the window, in chain order, for drawing over the line.
 * `graduation` is the moment the bonding curve ended and the pool began, so a chart can show where one venue became the other.
 */
export type Chart = {
  res: "trade" | "5m" | "1h";
  range: "5m" | "1h" | "6h" | "1d" | "all" | null;
  coin: Address;
  quote: "ETH";
  priceIs: "pair asset per whole coin";
  candles: Candle[];
  marks: Trade[];
  /**
   * docs/phase12.md §3. The three numbers a dollar market cap is made of, so a page computes it rather than the API
   * guessing a unit for it: `supplyWei / 1e18 × candle.c × usdPerEth`.
   *
   * `supplyWei` is the coin's whole supply in its own base units, read from `totalSupply()` once at launch.
   * `usdPerEth` is dollars per one whole ETH, read from `slot0().sqrtPriceX96` on ONE Uniswap v3 pool (`usdPool`) on this
   * same chain and cached for a few minutes. It is a **spot read of one pool at one block, not a market data feed**, and a
   * page that shows it must say so. `usdPerEthAt` is the second it was read.
   *
   * Every one of them is `null` when it is not known: an unread supply, a chain with no such pool, a pool that would not
   * answer, or a cached rate that has gone stale. A consumer that meets a null falls back to the pair asset and says which
   * unit it is drawing. None of them is ever estimated, defaulted or carried over as a guess.
   */
  supplyWei: Wei | null;
  usdPerEth: number | null;
  usdPerEthAt: number | null;
  usdPool: Address | null;
  trades: number;
  tradesAllTime: number;
  launchedAt: number | null;
  span: number;
  seconds: number;
  graduation: { at: number | null; block: number | null; tx: Hex | null } | null;
  pool: string | null;
};

/* ---------------------------------------------------------------------------------------------------------------------------
 * docs/v5/ux-and-reserves.md. Wrapped collections (wFROG, wAPE, …): the collection page, redemption tickets, and the proof of
 * reserves. Every answer carries `source`: "live" is read from the chains named in it; "fixture" is built from the docs' worked
 * examples and must be labelled FIXTURE wherever it is shown. `configured: false` means this indexer was given no collections.
 * ------------------------------------------------------------------------------------------------------------------------- */
export type WrappedSource = { kind: "live" | "fixture"; note: string };
export type WrappedKind = "inscriptions" | "nft";
/** Where the items sit. Inscriptions: one Bitcoin custody address. NFTs: the canister's address on the collection's chain. */
export type Custody =
  | { kind: "bitcoin"; address: string; addressUrl: string }
  | { kind: "evm"; chainId: number; chainName: string; collection: Address; custodian: Address; addressUrl: string | null; providers: string[] };

export type ItemArrival = {
  /** purchase: bought into custody (a vault receipt for inscriptions, a Seaport buy for NFTs); deposit: a holder sent it;
   *  rental: lent by its owner (contracts in progress: nothing produces this yet). */
  how: "purchase" | "deposit" | "rental";
  /** the transaction that brought it in, on the item's own chain; null when the event does not carry it (a vault purchase:
   *  the Bitcoin transaction is then read from ord's satpoint, see `ItemCheck.arrivalTx`) */
  tx: string | null;
  txUrl: string | null;
  /** the 4663 transaction that minted its wX, and when */
  mintTx: Hex;
  mintedAt: number | null;
  mintBlock: number;
};
export type ItemVerdict = "in-custody" | "elsewhere" | "unverified";
export type ItemCheck = {
  verdict: ItemVerdict;
  /** why, in words, whenever it is not in-custody: which source failed, disagreed, or was stale */
  reason: string | null;
  checkedAt: number | null;
  /** the block (EVM) or the confirmed height (Bitcoin) the check read at, when there is one */
  atBlock: number | null;
  /** what each source said, verbatim enough to check */
  sources: { name: string; url: string; said: string; ok: boolean }[];
  /** the Bitcoin transaction ord says the inscription currently sits in (the arrival, if it never moved) */
  arrivalTx: string | null;
};
export type CustodyItem = {
  /** the wrapper's own id: an acquisition id or DEPOSIT_FLAG|n (Wrapper), an itemId (NftWrapper), as a decimal string */
  itemId: string;
  /** inscriptions: the inscription id; NFTs: the token id */
  ref: string;
  refUrl: string | null;
  arrival: ItemArrival;
  /** a resolved ticket names this item and it has not been delivered yet: still in custody, backs no circulating token */
  promisedTo: number | null;
  check: ItemCheck;
};
export type TicketStage = "pinned" | "resolved" | "delivered" | "refunded";
export type Ticket = {
  id: number;
  owner: Address;
  destination: string;
  toBurn: boolean;
  stage: TicketStage;
  round: number;
  burnedAt: number | null;
  burnTx: Hex;
  itemId: string | null;
  itemRef: string | null;
  deliverBy: number | null;
  overdue: boolean;
  deliveryTx: string | null;
  deliveryUrl: string | null;
  /** 4663 tx of the refund, when refunded */
  refundTx: Hex | null;
  /** docs/audit/2026-09-26/fixes-B-C-keeper-draws.md (E-03, canister M-1) and review2/fixes-R3-contracts.md §4: when anyone may refund
   *  it, by the chain's time (the parent chain's clock can add minutes): a pinned ticket `PIN_TIMEOUT` (7 days) after its pin; a
   *  resolved one `REFUND_GRACE` (72 h) after `deliverBy` once the custodian attested it undeliverable, else `LATE_DELIVERY_WINDOW`
   *  (30 days) after `deliverBy`; null once delivered or refunded, or when a time is not known. Absent from reports written before the field. */
  refundableAt?: number | null;
  /** fixes-R3-contracts §4: the custodian attested (`Wrapper.markUndeliverable`, event `Undeliverable`) that no version of this resolved
   *  ticket's delivery can confirm, so it refunds at `deliverBy + 72 h`. Absent from reports written before the field (and for NFT wrappers). */
  undeliverable?: boolean;
  /** refunded while still pinned (`Refunded` with `acquisitionId = type(uint256).max`): nobody resolved it, no item was drawn */
  refundedUnresolved?: boolean;
};
/** docs/v5/floor-module.md §13.6, as built: the module's books and its price, read at one block. Units are wX base units (1e18 = one item). */
export type FloorModuleFill = { priceWei: Wei; at: number };
export type FloorModuleRead =
  | { kind: "absent"; note: string }
  | { kind: "read"; address: Address;
      /** E = every unit backed by ETH rather than by an item (`ethBackedUnits()`), = convertible + reserve */
      ethBackedUnits: Wei;
      /** R: the standing reserve's units, never converted into an item (`reserveUnits()`) */
      reserveUnits: Wei;
      /** C = E − R − T: what the canister still buys items for (`netUnits()`) */
      convertibleUnits: Wei;
      /** units the module holds as inventory bought ahead of demand with the treasury's float allocation (`floatUnits()`) */
      floatUnits: Wei;
      /** docs/v6/relay-conversion.md §2: T, the unit whose item budget was advanced before its item arrived (`transitUnits()`, 0 or 1e18):
       *  E = C + R + T. Absent (or null) from a module built before the advance, and from reports written before the field. */
      transitUnits?: Wei | null;
      /** the open advance's wei, paid to the canister's EVM address (`advanced()`), and when (`advancedAt()`, 0 when none) */
      advancedWei?: Wei | null;
      advancedAt?: number | null;
      /** docs/v7/phase1-nft-pooled.md §3: units beyond one item before a buy is due (`buffer()`). Null: not read */
      bufferUnits?: Wei | null;
      /** B + S: every wei backing ETH-backed units, both pots (`escrowBalance()`) */
      escrowWei: Wei;
      /** B: the convertible pot (`escrow()`) */
      backingWei: Wei;
      /** S: the standing reserve (`reserve()`) */
      standingReserveWei: Wei;
      /** V: realized spread revenue above what conversions cost (`revenue()`), skimmable to the treasury past `revenueTarget` */
      revenueWei: Wei;
      /** the keeper's accepted floor (`floor()`), when it was posted, and whether the module trades on it now */
      floorWei: Wei; floorPostedAt: number; floorFresh: boolean;
      /** the fills-based price the module quotes at (`price()`): the trailing-24h TWAP of the protocol's own fills, inside ±fillBand of the floor */
      priceWei: Wei; fillBandBps: number;
      /** the last fills, newest first (`fill(i)`): each a purchase's cost or a sale's proceeds normalised to the floor it implies */
      fills: FloorModuleFill[];
      /** each side's spread now (`spreads()`): mint = base + volatility + staleness + confidence + stress; burn without the stress */
      spreadMintBps: number; spreadBurnBps: number;
      /** the backing per unit each tier holds (`betaConvertible()`, `betaReserve()`): a burn is never paid more than its tier's */
      betaConvertibleWei: Wei; betaReserveWei: Wei;
      /** what can be minted / burned now: the unit price and the most ETH in / units in (`mintCapacity()`, `burnCapacity()`) */
      mintPriceWei: Wei; mintRoomWei: Wei; burnPriceWei: Wei; burnRoomUnits: Wei;
      /** a bootstrap loan still owed to the treasury (`loanOutstanding()`); the protocol treasury it names (`treasury()`), or null */
      loanOutstandingWei: Wei; treasury: Address | null;
      /** trailing 7-day mint + burn volume (`volume7d()`): the treasury's weight for this collection */
      volume7dWei: Wei;
      /** fixes-B-C B-02: the reserve's surplus (`surplus()`): curve fee slices, the treasury's reserve share, item-sale proceeds, the
       *  spread reserve-tier burns keep. Spent only on conversions, or skimmed to the treasury after the stale window. Null: not read */
      surplusWei?: Wei | null;
      /** fixes-B-C B-04: the stale fallback is on (`escaped()`): no good floor for 7 days and an hour, so anyone burns at the backing
       *  per unit less the base spread, with no floor price. Null: not read */
      escaped?: boolean | null;
      /** `exitAtBacking()`: burns pay each tier's backing per unit less the base spread, with no floor price. Since r4-contracts
       *  (A3-M1) this is `escaped()` alone: FALSE while the keeper is paused (a round-3 module said true then). Null: not read */
      exitAtBacking?: boolean | null;
      /** r4-contracts: `burnsOpen()`: a burn is priced at all (fresh, or the keeper paused or unnamed, or escaped). Null: not read
       *  (a module built before round 4) */
      burnsOpen?: boolean | null;
      /** r4-contracts A3-M1: `pauseFloor()`, while the keeper is paused and the module not escaped: a burn pays
       *  min(backing per unit, this) less the base spread per tier (the floor the keeper left, frozen). "0" otherwise. Null: not read */
      pauseFloorWei?: Wei | null;
      /** R4-check A4-I3: the registry's `keeperPaused()`, or no keeper named, at this block: nothing is minted at the module and burns
       *  pay the pause floor, even while the last post is still fresh. Null: not read (then the site falls back to `pauseFloorWei`) */
      keeperPaused?: boolean | null;
      /** r4-contracts A3-L1: unfilled curves' refunds stay held until this time (the registry's `keeperResumedAt` + `RESUME_GRACE`,
       *  6 h) after an unpause; null when no grace is running or it could not be read */
      refundsHeldUntil?: number | null;
      at: number | null; block: number }
  | { kind: "error"; address: Address; error: string };
/** docs/v5/floor-module.md §13.3: the one treasury every income stream reaches, read at one block. */
export type TreasuryRead =
  | { kind: "absent"; note: string }
  | { kind: "read"; address: Address; balanceWei: Wei;
      /** allocated and not yet delivered; balance ≥ their sum */
      pots: { reservesWei: Wei; floatsWei: Wei; bootstrapWei: Wei; unallocatedWei: Wei };
      /** the four percentages, summing to 10,000; only the timelock changes them */
      shares: { reservesBps: number; floatsBps: number; bootstrapBps: number; opsBps: number };
      ops: Address | null; bootstrapLoanWei: Wei;
      totals: { receivedWei: Wei; allocatedWei: Wei; toOpsWei: Wei; toReservesWei: Wei; toFloatsWei: Wei; lentWei: Wei; repaidWei: Wei };
      /** every registered module with its weight (7-day volume) and what it still owes */
      collections: { module: Address; weightWei: Wei; outstandingLoanWei: Wei }[];
      weightTotalWei: Wei; at: number | null; block: number }
  | { kind: "error"; address: Address; error: string };
/** docs/v5/floor-module.md §13.2: the protocol's own liquidity in the collection's lock (`PROTOCOL = address(1)`), never withdrawable. */
export type ProtocolLiquidityRead =
  | { kind: "absent"; note: string }
  | { kind: "read"; lock: Address;
      /** ETH and wX waiting to be placed by `deploy(PROTOCOL)` (the price guard deferred them) */
      pendingEthWei: Wei; pendingWrappedUnits: Wei;
      /** protocol positions the lock holds in the shared pool */
      positions: number;
      /** the shared pool's totals from every graduation, and the lock's fee books */
      sharedEthInWei: Wei; sharedWrappedInUnits: Wei; feesInEthWei: Wei; feesPaidEthWei: Wei;
      treasury: Address | null; at: number | null; block: number }
  | { kind: "error"; lock: Address; error: string };
export type ReserveStatus = "green" | "red" | "unverified";
export type ReserveReport = {
  source: WrappedSource;
  slug: string;
  name: string;
  symbol: string;
  kind: WrappedKind;
  wrapper: Address;
  wrapperUrl: string | null;
  custody: Custody;
  status: ReserveStatus;
  /** every reason the status is not green; empty when green */
  reasons: string[];
  computedAt: number;
  /** the last time this indexer computed green for this collection; null if never since it started */
  lastGreenAt: number | null;
  /** the 4663 block every wrapper and module read was made at */
  block: number | null;
  equation: {
    itemsInCustody: number;
    itemsUnverified: number;
    itemsElsewhere: number;
    ticketsInFlight: number;
    /** (itemsInCustody − ticketsInFlight) × 1e18 + ethBackedUnits, in wX base units */
    backingUnits: Wei;
    /** the same, counting every unverified item as present: the best the unread sources could make it */
    backingUnitsIfUnverifiedHold: Wei;
    ethBackedUnits: Wei;
    /** the ETH-backed units by tier (docs/v5/floor-module.md §13.1): reserve units R, float units, convertible C = E − R; zero without a module */
    reserveUnits: Wei;
    floatUnits: Wei;
    convertibleUnits: Wei;
    /** docs/v6/relay-conversion.md §2: the unit in transit (its budget advanced, its item not yet here); E = C + R + T. Absent from
     *  reports written before the field */
    transitUnits?: Wei;
    /** every wei backing the ETH-backed units: the convertible pot and the standing reserve together (`escrowBalance()`); zero without a module */
    escrowWei: Wei;
    supplyUnits: Wei | null;
    /** the wrapper's own books at the same block */
    heldOnChain: number | null;
    openTicketsOnChain: number | null;
    heldInLedger: number;
    /** true / false when it could be decided, null when a term is not known */
    holds: boolean | null;
  };
  floorModule: FloorModuleRead;
  /** the protocol treasury the module names (§13.3), and the protocol-owned liquidity in the collection's lock (§13.2) */
  treasury: TreasuryRead;
  protocolLiquidity: ProtocolLiquidityRead;
  items: CustodyItem[];
  ticketsInFlight: Ticket[];
};

export type CoinPhase = "trading" | "graduating" | "graduated" | "refunding";
export type WrappedCoin = {
  address: Address;
  curve: Address;
  name: string;
  symbol: string;
  creator: Address;
  /** docs/v5/policies-and-fees.md §1: raffle and burn as before; yoyo relists swept items at a markup and burns the coin with the proceeds */
  policy: "raffle" | "burn" | "yoyo";
  sweepOnly: boolean;
  phase: CoinPhase;
  raiseWei: Wei;
  raisedWei: Wei | null;
  launchedAt: number | null;
  /** docs/v5/policies-and-fees.md §3: the creator's fee tier, bps of every trade, fixed in the curve at launch (`CurveFactory.feeOf`) */
  feeBps: number;
  /** §2: the decaying launch fee on buys — its opening size, how long it takes to reach zero, and what a buy pays right now
   *  (`launchExtraBps()`, at the read's block); null when the coin has none */
  launchFee: { bps: number; decayS: number; extraBpsNow: number; endsAt: number | null } | null;
  /** YOYO only: what the vault holds for this coin — items relisted and not yet sold or returned, sale proceeds waiting for a buyback,
   *  and the coin bought back and burned so far (from the vault's `BoughtBack` events) */
  yoyo: { openListings: number; proceedsWei: Wei; boughtBackWei: Wei; burnedCoins: Wei; markupBps: number; listings: YoyoListing[] } | null;
  /** fixes-A-graduation A-03: whether the buy that reaches the raise would be accepted now (`canClose()`: Trading and the queue has
   *  room for the item budget, or SWEEP_ONLY). False: the closing buy is refused (`QueueFull`), sells and smaller buys still work */
  canClose?: boolean | null;
  /** A-01: whether `graduate()` may be called now (`canGraduate()`: Graduating, and filled or SWEEP_ONLY) */
  canGraduate?: boolean | null;
  /** A-01: when `openRefunds()` becomes callable for a Graduating curve (`refundsOpenAt()`, the timestamp half): the deadline if
   *  unfilled (held to `window + 4 days` while the queue holds refunds, R3 N-3), the escape hatch's `window + 4 days` if filled or
   *  SWEEP_ONLY; null in any other state */
  refundsOpenAt?: number | null;
  /** fixes-R3-contracts §2 (N-3): the queue holds refunds (`holdsRefunds()`: the module's keeper is paused, or, since r4-contracts
   *  A3-L1, for 6 h after an unpause), so a Graduating curve
   *  still unfilled at its deadline waits up to `STUCK_GRACE` (4 days) longer before refunds open (`refundsOpenAt` already says when);
   *  `canClose` is false meanwhile. Null: not read */
  refundsHeld?: boolean | null;
};
export type YoyoListing = { ticketId: number; stage: "listed" | "sold" | "returned"; askWei: Wei; proceedsWei: Wei | null; at: number | null; tx: Hex };
export type WrappedActivity = { at: number | null; block: number; tx: Hex;
  kind: "arrived" | "redeemed" | "resolved" | "delivered" | "refunded" | "raffle-won" | "burned" | "relisted" | "sold" | "bought-back" | "returned"; text: string };
export type WrappedSummary = {
  slug: string; name: string; symbol: string; kind: WrappedKind; wrapper: Address;
  supplyUnits: Wei | null; itemsHeld: number | null;
  /** the keeper's cap as the factory serves it; null when stale or unread */
  floorWei: Wei | null; floorFresh: boolean;
  status: ReserveStatus; coins: number;
};
export type WrappedList = { configured: boolean; source: WrappedSource | null; collections: WrappedSummary[] };
export type WrappedCollection = WrappedSummary & {
  source: WrappedSource;
  custody: Custody;
  description: string;
  chainId: number;
  /** contracts a trade or a redemption talks to */
  contracts: { wrapper: Address; factory: Address | null; lock: Address | null; router: Address | null; queue: Address | null; floorModule: Address | null;
    treasury: Address | null; offerBoard: Address | null; raffleVault: Address | null; burnVault: Address | null; yoyoVault: Address | null;
    poolManager: Address | null; v4Quoter: Address | null; stateView: Address | null };
  coinsList: WrappedCoin[];
  activity: WrappedActivity[];
  openTickets: Ticket[];
  /** for the one-click ticket in fixture mode: the pool state the quote is estimated from (live mode reads the chain) */
  market: WrappedMarket | null;
  /** fixes-A-graduation A-03: whether the factory takes a launch that buys items now (`launchesOpen()`); false: "launches paused,
   *  the floor module is full"; SWEEP_ONLY launches are not affected. Null: not read */
  launchesOpen?: boolean | null;
  /** fixes-B-C §0: the registry's keeper switch, read at the report's block, with its last change (`KeeperPaused(by, paused)`) */
  keeper?: KeeperState | null;
  /** fixes-B-C C-02..C-04, E-02: the raffle vault's draws, newest first (the latest 30) */
  draws?: RaffleDraw[];
  /** docs/v7/phase1-nft-pooled.md §2: this collection's own graduation targets, the factory's immutables (`normalRaise`,
   *  `earlyRaise` while the shared pool's ETH is under `earlyUntilEth`, never above `maxRaise`). Null: not read */
  graduation?: GraduationTargets | null;
  /** docs/v7/phase1-nft-pooled.md §3: the Floor Module's pool: its convertible units (`netUnits()`, graduations not yet turned into
   *  an item) and the level at which a buy is due (one item plus `buffer()`). Pooling below it is normal. Null: no module, or not read */
  pool?: { units: Wei; dueAtUnits: Wei } | null;
};
export type GraduationTargets = { normalRaiseWei: Wei; earlyRaiseWei: Wei; earlyUntilEthWei: Wei; maxRaiseWei: Wei };
export type KeeperState = { registry: Address; paused: boolean | null; guardian: Address | null;
  lastChange: { paused: boolean; by: Address; at: number | null; tx: Hex } | null };
/** SweepVault.Stage, in the site's words; `announced` (7) and `empty` (8) are last on chain so the earlier stages keep their numbers.
 *  `empty` (fixes-R3-carry): no eligible holder at the snapshot, closed with no winner; the unit stayed with the coin. */
export type RaffleStage = "announced" | "open" | "pinned" | "resolved" | "claimed" | "cancelled" | "voided" | "empty";
export type RaffleDraw = {
  id: number; vault: Address; coin: Address; stage: RaffleStage;
  /** the drand round pinned at the announcement that picks the snapshot time, and when it was announced */
  seedRound: number | null; announcedAt: number | null;
  /** the snapshot time the seed fixed (`SnapshotFixed`): the snapshot is the last block with a timestamp at most this */
  snapshotAt: number | null;
  /** set when opened: the root (it holds the sentinel leaf committing the count and the time), the count, the published list, the
   *  end of the 15-minute window before which nobody may pin */
  root: Hex | null; totalTickets: Wei | null; uri: string | null; challengeEnds: number | null;
  /** the published list rebuilt by this indexer (with the sentinel) against the on-chain root; null before the draw is opened */
  published: { status: "matches" | "differs" | "unreadable"; why: string | null } | null;
  /** the drand round the pin fixed, and when; the winning ticket and when it was fixed; the holder paid, once claimed */
  round: number | null; pinnedAt: number | null; winningTicket: Wei | null; resolvedAt: number | null; winner: Address | null;
  /** once resolved and unclaimed: the winner's range and a proof this indexer computed from a list that reproduces the root */
  claim: { holder: Address; first: Wei; end: Wei; proof: Hex[] } | null;
  /** when anyone may void it: announced + 6 h, pinned + 7 days, resolved + 7 days; null in a final stage */
  voidableAt: number | null;
  /** the transaction of the draw's latest event */
  tx: Hex;
  /** review2 N-2 (fixes-R3-contracts §3): the earlier draw whose seed round and snapshot window this one carries (`SeedCarried`), so
   *  its snapshot time is that draw's; null when it drew its own seed. The snapshot time is only ever `SnapshotFixed`'s, never
   *  computed from `announcedAt`. Absent from reports written before the field. */
  seedOrigin?: number | null;
  /** review2 N-2: voided before it was opened, or cancelled before its pin, so the coin's next draw reuses this seed round and snapshot
   *  window (the same snapshot time) instead of drawing a new one. Absent from reports written before the field. */
  seedPassedOn?: boolean;
};
/** Fixture-only market state for quotes. Live quotes never read this: they read the curve, V4Quoter and StateView. */
export type WrappedMarket = {
  shared: { reserveEth: Wei; reserveWrapped: Wei; lpFeePips: number; protocolFeePips: number } | null;
  coins: Record<string, {
    curve: { v: Wei; t: Wei; vEnd: Wei; tEnd: Wei; feeBps: number; launchExtraBps?: number } | null;
    pool: { reserveWrapped: Wei; reserveCoin: Wei; lpFeePips: number; protocolFeePips: number } | null;
  }>;
  /** the module as the quote engine needs it: its fills price, each side's spread, and the room each side has right now */
  floorModule: { priceWei: Wei; spreadMintBps: number; spreadBurnBps: number; mintRoomWei: Wei; burnRoomUnits: Wei } | null;
};

/* ---------------------------------------------------------------------------------------------------------------------------
 * docs/v6/offer-handins.md. The offer board of a Bitcoin collection: what is on it, what each hand-in became, and what was paid.
 * Verdicts live on chain as a submission's status (Open → Claimed | Slashed | Refunded); the canister's reasons do not. Read only.
 * ------------------------------------------------------------------------------------------------------------------------- */
export type HandinStatus = "open" | "claimed" | "slashed" | "refunded";
export type HandinSubmission = {
  id: number; submitter: Address; status: HandinStatus;
  /** when it was submitted (both clocks) */
  at: number | null; atBlock: number;
  bondWei: Wei; askSats: string; offerHash: Hex; inscriptionId: string;
  /** open only: whether the custodian could slash it now, and whether the submitter could take the bond back */
  slashable: boolean; expired: boolean;
  /** claimed: the reward paid on top of the bond; slashed / refunded: null */
  rewardPaidWei: Wei | null;
  /** the transaction that settled it (claim, slash or refund), when it did */
  settledTx: Hex | null; settledAt: number | null;
};
export type HandinReport = {
  source: WrappedSource; slug: string; name: string;
  board: Address | null; boardUrl: string | null;
  /** the board's constructor parameters, all immutable */
  params: { bondWei: Wei; rewardWei: Wei; slashDelayS: number; expiryS: number; rewardDayCapWei: Wei; maxOpen: number; maxOpenPerSubmitter: number } | null;
  custodian: Address | null;
  /** the reward pool and what has been paid today toward the day cap */
  rewardPoolWei: Wei | null; paidTodayWei: Wei | null;
  open: HandinSubmission[];
  /** every settled hand-in, newest first (bounded) */
  settled: HandinSubmission[];
  totals: { submitted: number; open: number; claimed: number; slashed: number; refunded: number; rewardsPaidWei: Wei; bondsSlashedWei: Wei; fundedWei: Wei };
  computedAt: number; block: number | null;
  /** why nothing could be read, when it could not */
  error: string | null;
};
