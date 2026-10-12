/**
 * The demo's canister (`?demo`): the Rook canister's public interface (src/rook/sol/canister.ts `RookCanister`) answered from made-up
 * books, so the collection pages' backing panels, the "Open it" path, the intent desk, the coin vaults and the Reserves page draw what
 * they will draw on the live site. Nothing is sent anywhere: `open_intent` refuses (the demo desk shows sample intents instead), and
 * `coins_request_open` only moves the asked collection through Requested → Opening → Open in this browser, a few seconds each.
 *
 * The demo's world is src/rook/sol/demo.ts's `DEMO_OPEN`: collections are numbered as the canister's registry numbers them
 * (`REGISTRY_ORDER`, autobuyer/canister-rs/src/coins/unlock.rs); the ceremony's 38 (the 14 Ordinals included) and the three Robinhood
 * Chain collections are pinned (the demo's story is after their 4663 read-back), so all 41. Every book here balances: the wX supply the
 * chain side reports is the books' expected supply, the module holds what the books owe, each coin vault holds its book, so the
 * Reserves page reads green, as the live one must. Pure apart from the opening clock; test/demo.test.ts calls every method.
 */
import { b58decode } from "./base58";
import type { BooksViewV, CollectionViewV, DeskStatusV, IntentViewV, ItemV, ListingKindV, OpenStatusV, Opt, RaffleV, RookCanister, TicketSummaryV, TreasuryStatusV, VaultViewV } from "./canister";
import type { ChainSide } from "./booksReserves";
import { COLLECTIONS, LAUNCH_COUNT, REGISTRY_ORDER, collectionBySlug, type SolCollection } from "./collections";
import { DEMO_BACKED_SYMBOL, DEMO_COINS, DEMO_NOT_OPEN, DEMO_OPEN, demoAddress, demoConfigAddress, demoThresholdUnits, demoWxMint } from "./demo";

const ONE = 1_000_000_000n;
const lamports = (sol: number) => BigInt(Math.round(sol * 1e9));
const nowS = () => Math.floor(Date.now() / 1000);
const hex = (b: Uint8Array) => Array.from(b, (x) => x.toString(16).padStart(2, "0")).join("");
const bytesOf = (label: string, n: number) => b58decode(demoAddress(label)).slice(0, n);

/** The registry index of a collection (its position in `REGISTRY_ORDER`), and back. */
export const demoIndexOf = (slug: string): number => REGISTRY_ORDER.indexOf(slug);
const slugAt = (index: number): string | null => REGISTRY_ORDER[index] ?? null;
/** Pinned in the demo: the ceremony's 38 (registry 0…37) and the three Robinhood Chain collections (38…40). */
export const demoPinned = (slug: string): boolean => { const i = demoIndexOf(slug); return i >= 0 && (i < LAUNCH_COUNT || collectionBySlug(slug)?.chain === "Robinhood Chain"); };
/** One collection already asked for when the demo starts, waiting in the queue. */
export const DEMO_QUEUED = "taiyo_robotics";

// ---- the opening clock: Requested → Opening → Open, a few seconds each, in this browser only ------------------------------------------
const asked = new Map<string, number>([[DEMO_QUEUED, Number.POSITIVE_INFINITY]]);
const STEP_MS = 6_000;
/** A collection's stage in the demo (`coins_collections`' `stage`), at `nowMs`. */
export function demoStage(slug: string, nowMs: number = Date.now()): string | null {
  if (!demoPinned(slug)) return null;
  if (DEMO_OPEN.includes(slug)) return "Open";
  const at = asked.get(slug);
  if (at === undefined) return "Pinned";
  if (!Number.isFinite(at)) return "Requested";
  return nowMs - at < STEP_MS ? "Requested" : nowMs - at < 2 * STEP_MS ? "Opening" : "Open";
}
/** The demo's queued collection has waited on the float since this long before the page loaded (audit C4-12: the site shows it). */
const DEMO_QUEUED_WAIT_S = 40 * 60;
const loadedS = nowS();
/** `coins_open_queue` in the demo: the queued collection first (waiting on the float), then this session's requests not open yet. */
export function demoOpenQueue(nowMs: number = Date.now()): OpenStatusV[] {
  return [...asked].filter(([s]) => { const st = demoStage(s, nowMs); return st === "Requested" || st === "Opening"; })
    .map(([s, at], position): OpenStatusV => {
      const queued = s === DEMO_QUEUED;
      return { index: demoIndexOf(s), position, asked_at: BigInt(queued ? loadedS - DEMO_QUEUED_WAIT_S - 60 : Math.floor(at / 1000)),
        waiting_since: queued ? BigInt(loadedS - DEMO_QUEUED_WAIT_S) : 0n,
        why: queued ? "the float is below the reserve plus one opening (≈ 0.023 SOL): waiting for fees" : "", skipped: false };
    });
}
/** For tests: forget the collections asked for in this session (the queued one stays queued). */
export const demoResetOpenings = () => { for (const k of [...asked.keys()]) if (k !== DEMO_QUEUED) asked.delete(k); };

// ---- the books ---------------------------------------------------------------------------------------------------------------------------
/** Per open collection: NFTs held, redemptions in flight, SOL-backed units (in wX), 7-day volume (SOL). */
const BOOKS: Record<string, { held: number; tickets: number; solBacked: number; volume: number }> = {
  milady: { held: 3, tickets: 1, solBacked: 2.4, volume: 41.2 }, azuki: { held: 2, tickets: 0, solBacked: 3.1, volume: 18.6 },
  mayc: { held: 1, tickets: 0, solBacked: 1.7, volume: 22.9 }, pudgypenguins: { held: 2, tickets: 0, solBacked: 0.9, volume: 64.3 },
  bayc: { held: 1, tickets: 0, solBacked: 0.6, volume: 88.1 }, moonbirds: { held: 2, tickets: 0, solBacked: 4.2, volume: 12.4 },
  clonex: { held: 4, tickets: 0, solBacked: 9.5, volume: 9.8 }, lilpudgys: { held: 1, tickets: 0, solBacked: 2.2, volume: 6.1 },
  checks: { held: 3, tickets: 0, solBacked: 6.8, volume: 7.7 }, identitymd: { held: 0, tickets: 0, solBacked: 1.3, volume: 11.5 },
  "bitcoin-frogs": { held: 6, tickets: 1, solBacked: 11.4, volume: 15.2 }, mad_lads: { held: 2, tickets: 0, solBacked: 3.6, volume: 21.8 },
  smb_gen2: { held: 1, tickets: 0, solBacked: 2.9, volume: 14.0 }, claynosaurz: { held: 0, tickets: 0, solBacked: 1.8, volume: 3.2 },
  okay_bears: { held: 2, tickets: 0, solBacked: 8.0, volume: 4.4 }, chainmancers: { held: 1, tickets: 0, solBacked: 3.3, volume: 5.9 },
};

function kindOf(c: SolCollection): ListingKindV {
  if (c.chain === "Bitcoin") return { Inscriptions: { custody: "bc1p" + "q".repeat(58), membership_root: bytesOf(`root:${c.slug}`, 32) } };
  if (c.chain === "Solana") return { SolNft: { collection: b58decode(c.contract), custodian: b58decode(demoAddress("K_main")) } };
  return { EthNft: { chain_id: c.chain === "Robinhood Chain" ? 4663n : 1n, collection: Uint8Array.from((c.contract.slice(2).match(/../g) ?? []).map((h) => parseInt(h, 16))), custody: bytesOf("evm-custody", 20) } };
}
function itemOf(c: SolCollection, i: number): ItemV {
  const key = c.chain === "Bitcoin" ? hex(bytesOf(`insc:${c.slug}:${i}`, 32)) : c.chain === "Solana" ? hex(b58decode(demoAddress(`nft:${c.slug}:${i}`))) : (1_000 + i * 337).toString(16);
  return { item_id: i + 1, key, source_tx: hex(bytesOf(`src:${c.slug}:${i}`, 32)), vout: 0, deposit: i === 2, inscription_id: c.chain === "Bitcoin" ? `${key}i0` : "" };
}
function ticketOf(c: SolCollection, i: number, now: number): TicketSummaryV {
  const destination = c.chain === "Bitcoin" ? "bc1p" + "x".repeat(58) : c.chain === "Solana" ? demoAddress(`redeemer:${c.slug}`) : `0x${hex(bytesOf(`redeemer:${c.slug}`, 20))}`;
  return { id: BigInt(i + 1), stage: "Resolved", item_id: 1, deliver_by: BigInt(now + 6 * 86_400), destination, undeliverable: false };
}

/** A collection's `books_view` in the demo; null for a collection that is not open. */
export function demoBooksView(slug: string, now: number = nowS()): BooksViewV | null {
  const c = collectionBySlug(slug), b = BOOKS[slug];
  if (!c || !b || demoStage(slug) !== "Open") return null;
  const accepted = lamports(c.floorSol), solBacked = lamports(b.solBacked);
  const backing = (solBacked * accepted * 97n) / (100n * ONE), reserve = backing / 4n, surplus = accepted / 50n, revenue = accepted / 200n;
  const held = BigInt(b.held), open = BigInt(b.tickets);
  const items = Array.from({ length: b.held }, (_, i) => itemOf(c, i));
  return {
    coll: demoIndexOf(slug), kind: kindOf(c), wx_mint: demoWxMint(slug), paused: false, poster_paused: false, accepted, target: accepted, posted_at: BigInt(now - 640),
    fresh: true, escaped: false, last_good_at: BigInt(now - 640), backing, reserve, surplus, revenue, owed: backing + reserve + surplus + revenue, reserve_units: solBacked / 5n,
    transit_units: 0n, advanced: 0n, mint_price: [(accepted * 10_015n) / 10_000n], burn_pays_one: [(accepted * 9_985n) / 10_000n], volume_7d: lamports(b.volume),
    items_arrived: held, held, open_tickets: open, burned: 0n, ticket_count: open + 1n, item_count: b.held, sol_backed: solBacked, module_units: 0n,
    expected_supply: [(held - open) * ONE + solBacked], items, tickets_open: Array.from({ length: b.tickets }, (_, i) => ticketOf(c, i, now)), halted: [],
    raffle_pot: slug === "milady" ? lamports(0.62) : 0n, raffle_prize: 0n,
  };
}

/** The graduated coins' vaults of a collection: the backed one (DEMO_BACKED_SYMBOL) holds half its threshold, less what was redeemed. */
export function demoVaults(slug: string): VaultViewV[] {
  const coll = demoIndexOf(slug), half = demoThresholdUnits(slug) / 2n;
  return DEMO_COINS().filter((x) => x.slug === slug && x.stats.phase === "graduated").map((x): VaultViewV => {
    const backed = x.meta.symbol === DEMO_BACKED_SYMBOL;
    return { coll, coin: x.mint, pool: x.pool, stage: backed ? "Backed" : "Migrated", vault: backed ? [demoAddress(`vault:${x.meta.symbol}`)] : [],
      wx_in: backed ? half : 0n, coins_burned: backed ? 21_400_000_000_000n : 0n, wx_paid: backed ? 53_500_000n : 0n, backed_at: backed ? BigInt(x.launchedAt + 3 * 86_400) : 0n };
  });
}
/** A coin's live supply in the demo (1B coins, 6 decimals, less what its holders redeemed). */
export function demoCoinSupply(mint: string): bigint | null {
  const x = DEMO_COINS().find((c) => c.mint === mint);
  if (!x) return null;
  const burned = demoVaults(x.slug).find((v) => v.coin === mint)?.coins_burned ?? 0n;
  return 1_000_000_000_000_000n - burned;
}

/** What the chain says of a collection in the demo: exactly what its books say (supply, the module above its rent, each vault). */
export const DEMO_RENT = 890_880n;
export function demoChainSide(v: BooksViewV, vaults: VaultViewV[]): ChainSide {
  return { supply: v.expected_supply.length ? v.expected_supply[0] : null, moduleLamports: v.owed + DEMO_RENT + 12_000n, rent: DEMO_RENT,
    vaults: new Map(vaults.flatMap((x) => (x.vault.length ? [[x.vault[0], x.wx_in - x.wx_paid] as [string, bigint]] : []))) };
}

// ---- the intent desk's sample intents (one per kind) ---------------------------------------------------------------------------------
/** The ids of the sample intents the demo desk shows: a mint done, a burn landing, an NFT redemption waiting on its deposit, a coin redemption done. */
export const DEMO_INTENT_IDS = { mint: 4101n, burn: 4102n, redeem_nft: 4103n, redeem_coin: 4104n } as const;
export function demoIntent(id: bigint, now: number = nowS()): IntentViewV | null {
  const kind = (Object.entries(DEMO_INTENT_IDS).find(([, v]) => v === id) ?? [])[0];
  if (!kind) return null;
  const base: IntentViewV = {
    id, kind, coll: 1, state: { Open: null }, address: demoAddress(`intent:${id}`), token_account: kind === "mint" ? [] : [demoAddress(`intent-ata:${id}`)],
    recipient: demoAddress("demo-wallet"), refund_to: demoAddress("demo-wallet"), deposit: 0n, min_deposit: kind === "mint" ? 50_000_000n : kind === "burn" ? 10_000_000n : ONE,
    created_at: BigInt(now - 420), expires_at: BigInt(now + 86_000), funded_at: [], minted_units: 0n, paid_lamports: 0n, ticket: [], refunding: [], in_flight: [], note: "", attention: false, txs: [],
  };
  const tx = (op: string, result: string, label: string, memo: string, ago: number) => ({ at: BigInt(now - ago), op, result, signature: label ? demoAddress(label) + demoAddress(`${label}:2`) : "", link: 7n, memo });
  if (kind === "mint") return { ...base, state: { Done: null }, deposit: 2_000_000_000n, funded_at: [BigInt(now - 300)], minted_units: 106_872_000n,
    txs: [tx("mint", "landed (finalized)", "sig:mint", "rook:1:mint", 240)] };
  if (kind === "burn") return { ...base, state: { Executing: null }, deposit: 500_000_000n, funded_at: [BigInt(now - 90)], in_flight: ["rook:1:burn"],
    txs: [tx("burn", "sent", "sig:burn", "rook:1:burn", 20)] };
  if (kind === "redeem_nft") return { ...base, coll: 4, state: { Open: null }, note: "waiting for the deposit" };
  return { ...base, state: { Done: null }, deposit: 21_400_000_000_000n, funded_at: [BigInt(now - 600)], paid_lamports: 53_500_000n,
    txs: [tx("redeem_coin", "landed (finalized)", "sig:redeem", "rook:1:redeem_coin", 500)] };
}

// ---- the interface ---------------------------------------------------------------------------------------------------------------------
function collectionsView(): CollectionViewV[] {
  return REGISTRY_ORDER.flatMap((slug, index): CollectionViewV[] => {
    const stage = demoStage(slug), c = collectionBySlug(slug);
    if (!stage || !c) return [];
    const open = stage === "Open";
    const coins = DEMO_COINS().filter((x) => x.slug === slug);
    return [{ index, symbol: c.wxSymbol, stage, wx_mint: open || stage === "Opening" ? demoWxMint(slug) : "", config: open ? demoConfigAddress(slug) : "",
      module: demoAddress(`module:${slug}`), coins: coins.length, backed: coins.filter((x) => x.backed).length }];
  });
}
function deskStatus(now: number): DeskStatusV {
  const open = collectionsView().filter((c) => c.stage === "Open").length;
  return { incidents: [], report: [{ at: BigInt(now - 420), unexplained: [], checked: open, memos_seen: 318, findings: [], halted: [] }], open: 3, collections: open,
    last_reconcile: BigInt(now - 420), funded: 1, needing_attention: 0, open_links: 1, executing: 1, halted: [], coin_redeem: true, paused: false };
}
function treasuryStatus(): TreasuryStatusV {
  return { off: false, last: [["distribute", BigInt(nowS() - 7_200), "landed"]], owed: [{ Surplus: { lamports: 140_000_000n, coll: demoIndexOf("milady") } }],
    treasury: { reserves_pot: lamports(3.84), unallocated: lamports(0.031), total_received: lamports(61.7), total_allocated: lamports(61.67), total_to_reserves: lamports(55.5),
      total_to_raffles: lamports(2.1), total_to_ops: lamports(4.07), reserves_bps: 9_000, ops_bps: 1_000, raffle_bps: 5_000, reserve_cap_bps: 20_000,
      collections: collectionsView().filter((c) => c.stage === "Open").map((c) => c.index) } };
}

let one: RookCanister | null = null;
/** The demo canister, once. */
export function demoCanister(): RookCanister {
  return (one ??= {
    books_collections: async () => collectionsView().filter((c) => c.stage === "Open").map((c): [number, string] => [c.index, c.symbol]),
    books_view: async (coll) => { const s = slugAt(coll); const v = s ? demoBooksView(s) : null; return (v ? [v] : []) as Opt<BooksViewV>; },
    coins_collections: async () => collectionsView(),
    coins_vaults: async (coll) => { const s = slugAt(coll); return s ? demoVaults(s) : []; },
    coins_open_queue: async () => demoOpenQueue(),
    coins_log: async () => [[], [], [], [[demoIndexOf(DEMO_QUEUED), BigInt(nowS() - 300)]]],
    pool_desks: async () => [],
    desk_status: async () => deskStatus(nowS()),
    treasury_status: async () => treasuryStatus(),
    raffle_status: async () => [] as Opt<RaffleV>,
    raffle_snapshot: async () => [],
    intent_status: async (id) => { const v = demoIntent(id); return (v ? [v] : []) as Opt<IntentViewV>; },
    quote_mint: async (coll, deposit) => {
      const v = demoBooksView(slugAt(coll) ?? ""); if (!v || !v.mint_price.length) return { Err: "the collection is not open" };
      const fee = 1_500_000n, ataRent = 2_039_280n, net = deposit > fee + ataRent ? deposit - fee - ataRent : 0n;
      return { Ok: { fee, min_deposit: 50_000_000n, coll, ata_rent: ataRent, deposit, to_module: net, unit_price: v.mint_price[0], units: (net * ONE) / v.mint_price[0] } };
    },
    quote_burn: async (coll, units) => {
      const v = demoBooksView(slugAt(coll) ?? ""); if (!v || !v.burn_pays_one.length) return { Err: "the collection is not open" };
      const paid = (units * v.burn_pays_one[0]) / ONE, fee = 1_500_000n;
      return { Ok: { fee, net: paid > fee ? paid - fee : 0n, coll, paid, units, min_units: ONE / 100n } };
    },
    open_intent: async () => ({ Err: "Demo: nothing is opened. On the live site the canister gives the intent its own deposit address." }),
    coins_request_open: async (index) => {
      const s = slugAt(index);
      if (!s || !demoPinned(s)) return { Err: "not pinned" };
      if (!asked.has(s) && !DEMO_OPEN.includes(s)) asked.set(s, Date.now());
      return { Ok: "requested" };
    },
  });
}

/** Every collection's place in the demo, for its test: open, the not-open example, pinned, not pinned (none since the 41 are). */
export const demoSummary = () => ({
  open: COLLECTIONS.filter((c) => demoStage(c.slug) === "Open").map((c) => c.slug),
  notOpen: DEMO_NOT_OPEN,
  pinned: COLLECTIONS.filter((c) => demoPinned(c.slug)).map((c) => c.slug),
  coming: COLLECTIONS.filter((c) => !demoPinned(c.slug)).map((c) => c.slug),
});
