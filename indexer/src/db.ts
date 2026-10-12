import { DatabaseSync } from "node:sqlite";

/**
 * docs/phase4.md §2. `events` is the only source of truth (append-only, one row per log). `blocks` holds the hash and time of each
 * block events came from. `meta`, `coin_meta` and `listing_meta` hold what was read by eth_call and cannot be refolded from events.
 * Every other table is DERIVED: `resetDerived` empties them and `rebuild` (fold.ts) refills them from `events`.
 */
export const SCHEMA = `
PRAGMA journal_mode = WAL;
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS blocks (number INTEGER PRIMARY KEY, hash TEXT NOT NULL, ts INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS events (
  block_number INTEGER NOT NULL, log_index INTEGER NOT NULL, tx TEXT NOT NULL, address TEXT NOT NULL, contract TEXT NOT NULL,
  name TEXT NOT NULL, args TEXT NOT NULL, ts INTEGER, listing_id INTEGER, acquisition_id INTEGER,
  PRIMARY KEY (block_number, log_index));
CREATE INDEX IF NOT EXISTS events_by_lot ON events (listing_id, acquisition_id);
CREATE TABLE IF NOT EXISTS coin_meta (coin TEXT PRIMARY KEY, name TEXT, symbol TEXT, tax_bps INTEGER, pool_fee INTEGER, tick_spacing INTEGER, pair_token TEXT, logo TEXT, logo_at INTEGER, supply TEXT, supply_at INTEGER);
CREATE TABLE IF NOT EXISTS listing_meta (id INTEGER PRIMARY KEY, divisibility INTEGER, btc_vault TEXT, root TEXT);
`;

export const DERIVED = `
CREATE TABLE IF NOT EXISTS listings (id INTEGER PRIMARY KEY, kind TEXT NOT NULL, vault TEXT NOT NULL, name TEXT NOT NULL, ref TEXT NOT NULL,
  paused INTEGER NOT NULL DEFAULT 0, btc_vault TEXT, root TEXT, pending_root TEXT, pending_root_at INTEGER, pending_btc TEXT, pending_btc_at INTEGER,
  autonomous INTEGER NOT NULL DEFAULT 0, buyer TEXT, max_in_flight TEXT,
  block INTEGER NOT NULL, ts INTEGER, tx TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS coins (coin TEXT PRIMARY KEY, creator TEXT NOT NULL, listing_id INTEGER NOT NULL, curve TEXT NOT NULL, router TEXT NOT NULL,
  deferred TEXT NOT NULL DEFAULT '0', block INTEGER NOT NULL, ts INTEGER, tx TEXT NOT NULL,
  graduated_block INTEGER, graduated_ts INTEGER, graduated_tx TEXT, graduated_quote TEXT, graduated_coins TEXT);
CREATE INDEX IF NOT EXISTS coins_by_router ON coins (router);
CREATE INDEX IF NOT EXISTS coins_by_curve ON coins (curve);
-- docs/phase10.md: one row per trade, from Pons's curve before graduation and from the coin's v4 pool after it. quote and
-- coins are the trader's own legs in wei and in the coin's base units; price is the curve-side price in the pair asset per
-- whole coin, the one number a chart draws. fee and tax are null for a pool trade: the hook charges them outside the Swap.
CREATE TABLE IF NOT EXISTS trades (block INTEGER NOT NULL, log_index INTEGER NOT NULL, coin TEXT NOT NULL, venue TEXT NOT NULL, side TEXT NOT NULL,
  quote TEXT NOT NULL, coins TEXT NOT NULL, fee TEXT, tax TEXT, price REAL NOT NULL, trader TEXT NOT NULL, recipient TEXT,
  ts INTEGER, tx TEXT NOT NULL, PRIMARY KEY (block, log_index));
CREATE INDEX IF NOT EXISTS trades_by_coin ON trades (coin, block DESC, log_index DESC);
CREATE INDEX IF NOT EXISTS trades_by_coin_ts ON trades (coin, ts);
-- docs/phase13.md §2: one row per (coin, holder), the running sum of that holder's Transfer deltas. amount is the exact
-- figure in the coin's own base units as a decimal string; amt is the same number as a double, because SQLite can neither
-- order nor compare 27-digit strings, and it is what the index and every "holds anything at all" test read. The zero address
-- never gets a row, and a row that reaches exactly zero is deleted rather than kept as a holder of nothing.
CREATE TABLE IF NOT EXISTS balances (coin TEXT NOT NULL, holder TEXT NOT NULL, amount TEXT NOT NULL, amt REAL NOT NULL, PRIMARY KEY (coin, holder));
CREATE INDEX IF NOT EXISTS balances_by_coin ON balances (coin, amt DESC);
-- The v4 pool a graduated coin trades in, learnt from the PoolManager's own Initialize log.
CREATE TABLE IF NOT EXISTS pools (pool_id TEXT PRIMARY KEY, coin TEXT NOT NULL, coin_is_0 INTEGER NOT NULL, quote_token TEXT NOT NULL,
  fee INTEGER, tick_spacing INTEGER, hooks TEXT, block INTEGER NOT NULL, ts INTEGER);
CREATE INDEX IF NOT EXISTS pools_by_coin ON pools (coin);
CREATE TABLE IF NOT EXISTS harvests (block INTEGER NOT NULL, log_index INTEGER NOT NULL, coin TEXT NOT NULL, claimed TEXT NOT NULL, to_vault TEXT NOT NULL,
  to_treasury TEXT NOT NULL, ts INTEGER, tx TEXT NOT NULL, PRIMARY KEY (block, log_index));
CREATE TABLE IF NOT EXISTS forwards (block INTEGER NOT NULL, log_index INTEGER NOT NULL, coin TEXT NOT NULL, token TEXT NOT NULL, amount TEXT NOT NULL, ts INTEGER, tx TEXT NOT NULL,
  PRIMARY KEY (block, log_index));
CREATE TABLE IF NOT EXISTS deposits (block INTEGER NOT NULL, log_index INTEGER NOT NULL, listing_id INTEGER NOT NULL, coin TEXT NOT NULL, amount TEXT NOT NULL, ts INTEGER,
  PRIMARY KEY (block, log_index));
CREATE TABLE IF NOT EXISTS caps (block INTEGER NOT NULL, log_index INTEGER NOT NULL, listing_id INTEGER NOT NULL, cap TEXT NOT NULL, expiry INTEGER NOT NULL, ts INTEGER,
  PRIMARY KEY (block, log_index));
CREATE TABLE IF NOT EXISTS purchases (listing_id INTEGER NOT NULL, purchase_id INTEGER NOT NULL, status TEXT NOT NULL, max_eth TEXT NOT NULL, quantity TEXT NOT NULL,
  releasable_at INTEGER NOT NULL, cap_ref TEXT NOT NULL, declared_ts INTEGER, tx TEXT NOT NULL, released_ts INTEGER, released TEXT, purchaser TEXT, deadline INTEGER,
  returned TEXT NOT NULL DEFAULT '0', written_off TEXT, slashed TEXT NOT NULL DEFAULT '0', closed_ts INTEGER, PRIMARY KEY (listing_id, purchase_id));
CREATE TABLE IF NOT EXISTS charges (listing_id INTEGER NOT NULL, purchase_id INTEGER NOT NULL, coin TEXT NOT NULL, amount TEXT NOT NULL, seq INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS acquisitions (listing_id INTEGER NOT NULL, acquisition_id INTEGER NOT NULL, purchase_id INTEGER NOT NULL, coin TEXT NOT NULL, asset_ref TEXT NOT NULL,
  amount TEXT NOT NULL, btc_txid TEXT NOT NULL, sats TEXT NOT NULL, ts INTEGER, tx TEXT NOT NULL, block INTEGER NOT NULL, PRIMARY KEY (listing_id, acquisition_id));
CREATE TABLE IF NOT EXISTS bond_events (block INTEGER NOT NULL, log_index INTEGER NOT NULL, listing_id INTEGER NOT NULL, type TEXT NOT NULL, who TEXT, amount TEXT NOT NULL,
  ts INTEGER, tx TEXT NOT NULL, PRIMARY KEY (block, log_index));
CREATE TABLE IF NOT EXISTS lots (listing_id INTEGER NOT NULL, acquisition_id INTEGER NOT NULL, mode TEXT NOT NULL DEFAULT 'unset', named TEXT, state TEXT NOT NULL DEFAULT 'undecided',
  root TEXT, total TEXT, uri TEXT, challenge_ends INTEGER, opened_ts INTEGER, round INTEGER, pinned_ts INTEGER, winning_ticket TEXT, resolved_ts INTEGER, winner TEXT, won_ts INTEGER,
  destination TEXT, destination_ts INTEGER, delivered_txid TEXT, delivered_ts INTEGER, overdue INTEGER NOT NULL DEFAULT 0, voided_ticket TEXT, voided_ts INTEGER,
  PRIMARY KEY (listing_id, acquisition_id));
CREATE TABLE IF NOT EXISTS distributions (listing_id INTEGER NOT NULL, acquisition_id INTEGER NOT NULL, state TEXT NOT NULL, root TEXT, total TEXT, uri TEXT, challenge_ends INTEGER,
  opened_ts INTEGER, claimed TEXT NOT NULL DEFAULT '0', claims INTEGER NOT NULL DEFAULT 0, rolled_coin TEXT, rolled_amount TEXT, rolled_ts INTEGER,
  PRIMARY KEY (listing_id, acquisition_id));
CREATE TABLE IF NOT EXISTS minimums (listing_id INTEGER PRIMARY KEY, units TEXT NOT NULL, expiry INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS payouts (payout_id INTEGER PRIMARY KEY, listing_id INTEGER NOT NULL, holder TEXT NOT NULL, amount TEXT NOT NULL, destination TEXT NOT NULL,
  requested_ts INTEGER, txid TEXT, recorded_ts INTEGER, overdue INTEGER NOT NULL DEFAULT 0);
CREATE TABLE IF NOT EXISTS roles (block INTEGER NOT NULL, log_index INTEGER NOT NULL, name TEXT NOT NULL, args TEXT NOT NULL, ts INTEGER, PRIMARY KEY (block, log_index));
`;
export const DERIVED_TABLES = ["listings", "coins", "harvests", "forwards", "deposits", "caps", "purchases", "charges", "acquisitions", "bond_events", "lots",
  "distributions", "minimums", "payouts", "roles", "trades", "pools", "balances"] as const;

/**
 * Bumped whenever a derived table's shape changes. A database written by an older version has its derived tables dropped and
 * refolded from `events`, which is the only source of truth — so a shape change costs a rebuild, never a re-read of the chain.
 */
export const DERIVED_VERSION = 3;

export function openDb(path: string): DatabaseSync {
  const db = new DatabaseSync(path);
  db.exec(SCHEMA);
  // a database whose derived tables were written by an older shape: drop them and mark the rebuild the Follower then runs
  const has = (db.prepare(`SELECT count(*) AS n FROM sqlite_master WHERE type = 'table' AND name = 'coins'`).get() as { n: number }).n > 0;
  if (has && getMeta(db, "derived_version") !== String(DERIVED_VERSION)) {
    for (const t of DERIVED_TABLES) db.exec(`DROP TABLE IF EXISTS ${t}`);
    setMeta(db, "rebuild_derived", "1");
  }
  db.exec(DERIVED);
  setMeta(db, "derived_version", String(DERIVED_VERSION));
  // docs/phase10.md: a database written before the logo existed gains the column; its coins are then re-read by the
  // follower's own retry, which asks again for any coin whose meta is incomplete.
  const cols = db.prepare(`PRAGMA table_info(coin_meta)`).all() as { name: string }[];
  if (!cols.some((c) => c.name === "logo")) db.exec(`ALTER TABLE coin_meta ADD COLUMN logo TEXT`);
  // `logo_at` separates "asked, and this coin has none" from "never asked", so the retry is bounded rather than forever.
  if (!cols.some((c) => c.name === "logo_at")) db.exec(`ALTER TABLE coin_meta ADD COLUMN logo_at INTEGER`);
  // docs/phase12.md §1: the coin's whole supply, in its own base units. A database written before this phase gains the
  // column empty, and the follower's own retry fills it coin by coin, the way it already fills a missing Pons record.
  if (!cols.some((c) => c.name === "supply")) db.exec(`ALTER TABLE coin_meta ADD COLUMN supply TEXT`);
  // `supply_at` bounds the retry the way `logo_at` does: a coin is asked once, and again at most once an hour until it answers.
  if (!cols.some((c) => c.name === "supply_at")) db.exec(`ALTER TABLE coin_meta ADD COLUMN supply_at INTEGER`);
  // docs/v2/vault-v2.md: which vault a listing got. A database written before v2 existed gains the columns, and every row
  // in it is a v1 listing, which is what the default says — the follower does not need to re-read anything to know that.
  const lcols = db.prepare(`PRAGMA table_info(listings)`).all() as { name: string }[];
  if (!lcols.some((c) => c.name === "autonomous")) db.exec(`ALTER TABLE listings ADD COLUMN autonomous INTEGER NOT NULL DEFAULT 0`);
  if (!lcols.some((c) => c.name === "buyer")) db.exec(`ALTER TABLE listings ADD COLUMN buyer TEXT`);
  if (!lcols.some((c) => c.name === "max_in_flight")) db.exec(`ALTER TABLE listings ADD COLUMN max_in_flight TEXT`);
  return db;
}
export const resetDerived = (db: DatabaseSync): void => { for (const t of DERIVED_TABLES) db.exec(`DELETE FROM ${t}`); };

export const getMeta = (db: DatabaseSync, key: string): string | null =>
  ((db.prepare(`SELECT value FROM meta WHERE key = ?`).get(key) as { value: string } | undefined)?.value) ?? null;
export const setMeta = (db: DatabaseSync, key: string, value: string): void => { db.prepare(`INSERT OR REPLACE INTO meta (key, value) VALUES (?, ?)`).run(key, value); };

export const add = (a: string | null | undefined, b: string | null | undefined): string => String(BigInt(a ?? "0") + BigInt(b ?? "0"));
export const sum = (xs: readonly (string | null | undefined)[]): string => String(xs.reduce((t, x) => t + BigInt(x ?? "0"), 0n));
