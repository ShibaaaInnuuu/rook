/**
 * The Solana indexer's SQLite file (node:sqlite, as the Robinhood indexer's db.ts). Numbered migrations, applied in order and
 * recorded in `meta.schema_version`; a migration is never edited once shipped, only followed by another.
 * Exact amounts are decimal strings (lamports, token base units); the `*_f` columns are the same as doubles for sorting and sums.
 */
import { DatabaseSync } from "node:sqlite";

export const MIGRATIONS: string[] = [
  // 1: the first shape
  `
  CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
  CREATE TABLE IF NOT EXISTS configs (config TEXT PRIMARY KEY, slug TEXT NOT NULL, listing_id INTEGER, quote_mint TEXT, fee_mode INTEGER,
    decimals INTEGER, threshold TEXT, migration_option INTEGER, read_at INTEGER);
  CREATE TABLE IF NOT EXISTS coins (mint TEXT PRIMARY KEY, pool TEXT NOT NULL UNIQUE, config TEXT NOT NULL, creator TEXT, decimals INTEGER NOT NULL DEFAULT 6,
    name TEXT, symbol TEXT, uri TEXT, image TEXT, description TEXT, website TEXT, twitter TEXT, telegram TEXT, meta_at INTEGER, offchain_at INTEGER,
    supply TEXT, supply_at INTEGER, created_slot INTEGER, created_at INTEGER, created_tx TEXT, activation_point TEXT,
    base_vault TEXT, quote_vault TEXT, quote_reserve TEXT, sqrt_price TEXT, is_migrated INTEGER NOT NULL DEFAULT 0, pool_at INTEGER,
    completed_at INTEGER, damm_pool TEXT, damm_vault_a TEXT, damm_vault_b TEXT, coin_is_a INTEGER, graduated_at INTEGER, graduated_slot INTEGER, graduated_tx TEXT,
    last_ord INTEGER, last_price REAL, last_trade_at INTEGER, trades_n INTEGER NOT NULL DEFAULT 0,
    holders INTEGER, holders_at INTEGER, holders_complete INTEGER, holders_json TEXT);
  CREATE INDEX IF NOT EXISTS coins_by_config ON coins (config);
  CREATE INDEX IF NOT EXISTS coins_by_damm ON coins (damm_pool);
  CREATE INDEX IF NOT EXISTS coins_by_created ON coins (created_at DESC);
  CREATE INDEX IF NOT EXISTS coins_by_creator ON coins (creator);
  CREATE TABLE IF NOT EXISTS trades (sig TEXT NOT NULL, n INTEGER NOT NULL, mint TEXT NOT NULL, ord INTEGER NOT NULL, slot INTEGER NOT NULL, at INTEGER,
    side TEXT NOT NULL, venue TEXT NOT NULL, sol TEXT NOT NULL, sol_f REAL NOT NULL, tokens TEXT NOT NULL, tokens_f REAL NOT NULL, fee TEXT,
    price REAL NOT NULL, trader TEXT NOT NULL, PRIMARY KEY (sig, n));
  CREATE INDEX IF NOT EXISTS trades_by_mint ON trades (mint, ord DESC);
  CREATE INDEX IF NOT EXISTS trades_by_mint_at ON trades (mint, at);
  CREATE INDEX IF NOT EXISTS trades_by_at ON trades (at);
  CREATE INDEX IF NOT EXISTS trades_by_trader ON trades (trader, ord DESC);
  -- one signature stream per address: a Rook config, or a graduated coin's DAMM v2 pool
  CREATE TABLE IF NOT EXISTS streams (address TEXT PRIMARY KEY, kind TEXT NOT NULL, mint TEXT, head_sig TEXT, head_slot INTEGER,
    back_before TEXT, back_done INTEGER NOT NULL DEFAULT 0, back_count INTEGER NOT NULL DEFAULT 0, seen INTEGER NOT NULL DEFAULT 0,
    last_new_at INTEGER, polled_at INTEGER, truncated INTEGER NOT NULL DEFAULT 0);
  CREATE TABLE IF NOT EXISTS sol_usd (t INTEGER PRIMARY KEY, usd REAL NOT NULL, src TEXT NOT NULL);
  `,
  // 2: W13 (canister-only design §3, §5.1, §4.4 rule 4). A config's quote is its collection's wX, not SOL: the quote mint's decimals
  // and symbol, the canister's collection index and module address (`coins_collections`), the desk's DLMM wX/SOL pair. The wX/SOL
  // rate per wX mint over time (`wx_sol`, as `sol_usd`), and the canister's memo-labelled transactions (`desk_events`).
  `
  ALTER TABLE configs ADD COLUMN quote_decimals INTEGER;
  ALTER TABLE configs ADD COLUMN quote_symbol TEXT;
  ALTER TABLE configs ADD COLUMN coll INTEGER;
  ALTER TABLE configs ADD COLUMN module TEXT;
  ALTER TABLE configs ADD COLUMN pair TEXT;
  CREATE TABLE IF NOT EXISTS wx_sol (mint TEXT NOT NULL, t INTEGER NOT NULL, sol REAL NOT NULL, src TEXT NOT NULL, PRIMARY KEY (mint, t));
  -- one row per Rook memo in a transaction of a desk key's history; the deltas are what the transaction's balances say, the memo only labels them
  CREATE TABLE IF NOT EXISTS desk_events (sig TEXT NOT NULL, n INTEGER NOT NULL, slot INTEGER NOT NULL, at INTEGER, coll INTEGER, kind TEXT NOT NULL,
    link_id INTEGER NOT NULL, intent INTEGER, payer TEXT NOT NULL, memo TEXT NOT NULL, fee INTEGER, wx_mint TEXT, wx_delta TEXT,
    payer_lamports TEXT NOT NULL, module_lamports TEXT, deltas TEXT NOT NULL, PRIMARY KEY (sig, n));
  CREATE INDEX IF NOT EXISTS desk_events_by_coll ON desk_events (coll, slot DESC);
  CREATE INDEX IF NOT EXISTS desk_events_by_slot ON desk_events (slot DESC);
  CREATE INDEX IF NOT EXISTS desk_events_by_at ON desk_events (at);
  `,
];

export function openSolDb(path: string): DatabaseSync {
  const db = new DatabaseSync(path);
  db.exec("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL; PRAGMA busy_timeout = 5000;");
  db.exec("CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);");
  const v = Number(getMeta(db, "schema_version") ?? "0");
  for (let i = v; i < MIGRATIONS.length; i++) {
    db.exec("BEGIN");
    try { db.exec(MIGRATIONS[i]); setMeta(db, "schema_version", String(i + 1)); db.exec("COMMIT"); }
    catch (err) { db.exec("ROLLBACK"); throw err; }
  }
  return db;
}
export const getMeta = (db: DatabaseSync, key: string): string | null => ((db.prepare(`SELECT value FROM meta WHERE key = ?`).get(key) as { value: string } | undefined)?.value) ?? null;
export const setMeta = (db: DatabaseSync, key: string, value: string): void => { db.prepare(`INSERT OR REPLACE INTO meta (key, value) VALUES (?, ?)`).run(key, value); };
