/**
 * docs/v8/redemption-draw.md and docs/v8/proof-of-reserves.md: the shapes the v8 endpoints serve (/v1/v8/draws/:slug,
 * /v1/v8/reserves/:slug, /v1/v8/snapshots/:slug) and the verifiers return. This directory (indexer/src/v8/, every file but
 * reads.ts, service.ts and routes.ts) is copied byte for byte to web-rook/src/rook/v8/core/, so the page, the CLI and the indexer
 * run the same code: it imports viem, @noble/curves and itself, and nothing else.
 */
export type Hex = `0x${string}`;
export type Address = `0x${string}`;
export type Wei = string;
export type Status = "green" | "red" | "unverified";
export type SourceKind = { kind: "live" | "fixture"; note: string };

// ---- the metadata snapshot (redemption-draw.md §2.1) ------------------------------------------------------------------------------

export type SnapshotToken = { tokenId: string; traits: Record<string, string> };
/** One snapshot of a whole collection's metadata, taken once, hashed, and used instead of the projects' live servers. */
export type MetadataSnapshot = {
  version: 1;
  collection: Address;
  chainId: number;
  name: string;
  takenAt: number;
  /** how the metadata was fetched: the tokenURI pattern, and the IPFS CID when the collection's metadata is content-addressed */
  source: { tokenURI: string; ipfsCid: string | null; note: string };
  tokens: SnapshotToken[];
};
/** What a report says about the snapshot it used: where it is, its hash (pinned before the first draw) and any later change. */
export type SnapshotRef = {
  url: string;
  sha256: Hex;
  /** where the hash was published before the first draw, so the indexer cannot swap both the file and the hash */
  pinnedIn: string;
  ipfsCid: string | null;
  takenAt: number;
  tokenCount: number;
  /** a later re-read of the projects' metadata, shown as a diff and never applied (null: not re-read) */
  drift: SnapshotDrift | null;
};
export type SnapshotDrift = { checkedAt: number; changed: { tokenId: string; trait: string; was: string | null; now: string | null }[]; unreadable: number };

// ---- the vault and its draws (redemption-draw.md §2.2–§2.3) -------------------------------------------------------------------------

export type VaultItemState = "free" | "pending" | "promised";
export type VaultItem = {
  itemId: number;
  tokenId: string;
  how: "purchase" | "deposit";
  /** the Ethereum transaction that brought it in (`Wrapped.purchaseTx` / `Deposited.depositTx`) */
  arrivalTx: Hex;
  mintedAt: number | null;
  state: VaultItemState;
  /** a pending item's stamp: only tickets with an id ≥ its stamp may draw it */
  stamp: number | null;
  /** a resolved ticket that drew it and is not delivered yet */
  promisedTo: number | null;
};
/** The wrapper's redemption books at one 4663 block, from its own views (`freeAt`, `pendingAt`, `ticket`, `item`, `itemOf`). */
export type VaultState = {
  block: number;
  at: number | null;
  ticketCount: number;
  nextToResolve: number;
  held: number;
  openTickets: number;
  /** `_free` in the contract's array order (item ids) */
  free: number[];
  pending: { itemId: number; stamp: number }[];
  /** burned, not yet resolved, not refunded: each will draw before a ticket burned now */
  pinned: { id: number; round: number; burnedAt: number | null }[];
  items: VaultItem[];
};

export type Posting = {
  /** the batch that carried the burn's block to Ethereum (Nitro's NodeInterface), and the Ethereum block and time it landed */
  batch: number | null;
  parentBlock: number | null;
  postedAt: number | null;
  note: string;
};
export type DrawProof = {
  ticket: number;
  owner: Address;
  destination: Address;
  toBurn: boolean;
  burnTx: Hex;
  burnBlock: number;
  /** the burn block's timestamp as the sequencer set it */
  burnAt: number;
  /** `ticket(id).round` = roundAt(burnAt) + 20 */
  round: number;
  resolveTx: Hex;
  resolveBlock: number;
  resolvedAt: number | null;
  /** the drand evmnet beacon of `round`: its BLS signature (64 bytes) and randomness (sha256 of the signature) */
  beacon: { signature: Hex | null; randomness: Hex | null; relays: string[] };
  /** the draw's set in the contract's array order at resolution: a replay of the wrapper's events, and the same read from its views */
  freeSet: number[];
  freeSetRead: number[] | null;
  freeSetNote: string;
  /** `Resolved(id, itemId, tokenId)` */
  itemId: number;
  tokenId: string;
  delivery: { tx: string; url: string | null; at: number | null } | null;
  posting: Posting | null;
};
export type DrawReport = {
  source: SourceKind;
  slug: string;
  name: string;
  chainId: number;
  collection: Address;
  wrapper: Address;
  beacon: Address | null;
  computedAt: number;
  state: VaultState | null;
  stateError: string | null;
  /** newest first */
  proofs: DrawProof[];
  snapshot: SnapshotRef | null;
  /** the wrapper's and the explorers' addresses, for links */
  links: { wrapper: string | null; tx: string | null; ethTx: string | null; ethNft: string | null };
};

// ---- verification (the browser, the CLI, the indexer) ----------------------------------------------------------------------------

export type Check = { id: string; label: string; ok: boolean | null; detail: string };
export type TimingLevel = "clear" | "within-batch-delay" | "flagged" | "unknown";
export type TimingFlag = { level: TimingLevel; roundPublishedAt: number; postedAt: number | null; leadS: number | null; text: string };
export type DrawVerdict = { ticket: number; verdict: "verified" | "mismatch" | "incomplete"; checks: Check[]; timing: TimingFlag };

/** One exact probability: `n / d`, reduced. */
export type Frac = { n: string; d: string };
export type ItemOdds = { itemId: number; tokenId: string; p: Frac; pct: number };
export type Odds = {
  /** the ticket id a burn mined now would get */
  forTicket: number;
  /** items a burn now could receive */
  size: number;
  /** unresolved tickets ahead that draw first, each from its own set */
  ahead: number;
  method: "uniform" | "exact" | "monte-carlo";
  /** Monte Carlo only: the largest standard error of any item's probability */
  stderr: number | null;
  samples: number | null;
  items: ItemOdds[];
  /** false when no item can be drawn (the wrapper refuses the burn: `held < openTickets + 1`) */
  redeemable: boolean;
};
export type TraitOdds = { trait: string; values: { value: string; items: number; p: Frac; pct: number; inCollection: number }[] };

// ---- proof of reserves, the Ethereum side (proof-of-reserves.md §2) -------------------------------------------------------------------

/** One number on the page: its value, who said it, at which block, when; `fresh` is judged against the limit at compute time. */
export type Num = { key: string; label: string; value: string | null; sources: string[]; block: number | null; at: number | null; ok: boolean | null; note: string };

export type OwnerSaid = { source: string; owner: Address | null; reverted: boolean; error: string | null };
export type ArrivalCheck = {
  verdict: "ok" | "bad" | "unread";
  /** the receipt's block, and whether it is at or below the finalized block the custody reads were made at */
  block: number | null;
  final: boolean | null;
  /** a Seaport fill's price (purchases), or the claim's recipient on 4663 (deposits); null when not read */
  pricePaidWei: Wei | null;
  detail: string;
  sources: string[];
};
export type EthItem = {
  itemId: number;
  tokenId: string;
  how: "purchase" | "deposit";
  arrivalTx: Hex;
  arrivalUrl: string | null;
  mintedAt: number | null;
  nftUrl: string | null;
  promisedTo: number | null;
  owners: OwnerSaid[];
  /** the owner read's block (Ethereum finalized N) and that block's time */
  ownerBlock: number | null;
  ownerAt: number | null;
  verdict: "in-custody" | "elsewhere" | "unverified";
  reason: string | null;
  arrival: ArrivalCheck;
  /** `AdvanceSettled(item, advanced, cost)`: what the module paid for a purchase, against the receipt's price */
  cost: { advancedWei: Wei; costWei: Wei; expectedCostWei: Wei | null; ok: boolean | null } | null;
};
export type ChainRead<T> = { source: string; ok: true; value: T; block: number; at: number | null } | { source: string; ok: false; error: string };
export type WrapperBooks = { totalSupply: Wei; held: number; openTickets: number; ethBacked: Wei };
export type ModuleBooks = {
  address: Address;
  ethBackedUnits: Wei; reserveUnits: Wei; convertibleUnits: Wei; transitUnits: Wei;
  escrowWei: Wei; revenueWei: Wei; advancedWei: Wei; advancedAt: number | null;
  /** `eth_getBalance(module)` at the same block */
  balanceWei: Wei;
};
/** The canister's own records (`eth_nft_state()`, an ICP query: one replica's answer). Explanation only: never a basis for green. */
export type CanisterRecords = {
  readAt: number;
  how: "query" | "update";
  buys: { id: string; collection: Address; phase: string; advancedWei: Wei; orderId: Hex | null; tokenId: string | null; tx: Hex | null; note: string }[];
  deliveries: { ticket: number; tokenId: string; phase: string; deliverBy: number; tx: Hex | null }[];
  deposits: { tx: Hex; tokenId: string }[];
  leftoverWei: Wei | null;
  caps: { maxItems: number; maxValueWei: Wei; itemsHeld: number; itemsInFlight: number; valueWei: Wei } | null;
};
export type Advance = {
  /** from the module: an open advance (T = 1 unit) and its wei and time */
  open: boolean;
  advancedWei: Wei;
  advancedAt: number | null;
  /** from the canister's records (labelled): the buy's phase and where its ETH is */
  phase: string | null;
  where: string;
  relay: { orderId: Hex; status: string | null; url: string } | null;
  overdue: boolean;
};
export type Stray = { tokenId: string; from: Address; tx: Hex; block: number; at: number | null; state: string; overdue: boolean };
export type EthReserveReport = {
  source: SourceKind;
  slug: string;
  name: string;
  symbol: string;
  collection: Address;
  custodian: Address;
  wrapper: Address;
  moduleAddress: Address | null;
  providers: { eth: string[]; home: string[] };
  status: Status;
  reasons: string[];
  /** amber: transient states past their own timeouts, and canister-record discrepancies; never a status */
  warnings: string[];
  computedAt: number;
  lastGreenAt: number | null;
  /** the limits the status was judged with (P1) */
  limits: { maxAgeS: number; redAfterS: number; cadenceS: number };
  homeBlock: number | null;
  homeAt: number | null;
  ethBlock: number | null;
  ethAt: number | null;
  numbers: Num[];
  equation: { inCustody: number; unverified: number; elsewhere: number; ticketsInFlight: number; ethBackedUnits: Wei; backingUnits: Wei; bestCaseUnits: Wei; supplyUnits: Wei | null; holds: boolean | null };
  items: EthItem[];
  module: ModuleBooks | null;
  advance: Advance | null;
  gas: { custodianWei: Wei | null; earmarkedWei: Wei | null; leftoverWei: Wei | null; potWei: Wei | null; sources: string[] } | null;
  strays: Stray[];
  canister: CanisterRecords | null;
  canisterNote: string;
  links: { wrapper: string | null; custodian: string | null; module: string | null; collection: string | null };
};

/** One call the self-check made, shown to the visitor verbatim enough to repeat it. */
export type CallLog = { chain: "ethereum" | "robinhood" | "drand"; host: string; method: string; params: string; result: string; ok: boolean; ms: number };
