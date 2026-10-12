import type { DatabaseSync } from "node:sqlite";
import { curvePrice, poolPrice } from "./candles.js";
import { add, resetDerived } from "./db.js";

/** One decoded log. `args` has bigints as decimal strings and addresses in lower case. */
export type Ev = { block: number; logIndex: number; tx: string; address: string; contract: string; name: string; args: Record<string, string | number | boolean>;
  ts: number | null };

const MODES = ["unset", "hold", "never", "draw-weighted", "draw-per-wallet", "named", "pro-rata"] as const;
const KINDS = ["inscriptions", "rune"] as const;
/** a bytes32 Bitcoin txid as explorers print it: hex without 0x */
export const btcTxid = (b32: string): string => b32.replace(/^0x/, "").toLowerCase();

const listingOfVault = (db: DatabaseSync, vault: string): number | null =>
  ((db.prepare(`SELECT id FROM listings WHERE vault = ?`).get(vault) as { id: number } | undefined)?.id) ?? null;
const coinOfRouter = (db: DatabaseSync, router: string): string | null =>
  ((db.prepare(`SELECT coin FROM coins WHERE router = ?`).get(router) as { coin: string } | undefined)?.coin) ?? null;
const coinOfCurve = (db: DatabaseSync, curve: string): string | null =>
  ((db.prepare(`SELECT coin FROM coins WHERE curve = ?`).get(curve) as { coin: string } | undefined)?.coin) ?? null;

/** docs/phase13.md §2: never a holder, on either leg. A mint comes from it and a burn goes to it. */
export const ZERO_ADDRESS = `0x${"0".repeat(40)}`;

/**
 * docs/phase13.md §2. The events that say what a thing IS, folded before the events that talk about it.
 *
 * Measured on staging: a coin's whole supply is minted at `logIndex` 0 of its launch block, and our own `Launched` — the only
 * event that says that coin is ours — is at `logIndex` 7 of the same block. In strict chain order the fold therefore meets
 * the mint of a coin it has never heard of and drops it, which is the rule the curve and the router already follow, leaving
 * the curve short by a billion coins for ever. So within a block these two are folded first. `rebuild` orders the same way,
 * which is what keeps the invariant that matters: the same events give the same tables.
 */
export const IDENTITY_EVENTS: readonly string[] = ["Listed", "Launched"];
const identityRank = (name: string) => (IDENTITY_EVENTS.includes(name) ? 0 : 1);
/** Chain order, identity first inside a block. Used by the follower; `FOLD_ORDER_SQL` is the same thing for `rebuild`. */
export const foldOrder = (a: { block: number; logIndex: number; name: string }, b: { block: number; logIndex: number; name: string }): number =>
  a.block - b.block || identityRank(a.name) - identityRank(b.name) || a.logIndex - b.logIndex;
export const FOLD_ORDER_SQL = `ORDER BY block_number, CASE WHEN name IN ('Listed', 'Launched') THEN 0 ELSE 1 END, log_index`;

/**
 * docs/phase13.md §2. One holder's balance, moved by one leg of one transfer. The zero address is skipped on both legs — a
 * mint comes from it, a burn goes to it, and it is never a holder — and a balance that reaches exactly zero loses its row
 * rather than staying on as a holder of nothing.
 */
function move(db: DatabaseSync, coin: string, who: string, delta: bigint): void {
  if (who === ZERO_ADDRESS || delta === 0n) return;
  const held = (db.prepare(`SELECT amount FROM balances WHERE coin = ? AND holder = ?`).get(coin, who) as { amount: string } | undefined)?.amount;
  const next = BigInt(held ?? "0") + delta;
  if (next === 0n) { db.prepare(`DELETE FROM balances WHERE coin = ? AND holder = ?`).run(coin, who); return; }
  db.prepare(`INSERT OR REPLACE INTO balances (coin, holder, amount, amt) VALUES (?, ?, ?, ?)`).run(coin, who, String(next), Number(next));
}

/** The (listingId, acquisitionId) an event belongs to, for the events table's index. Null where it has none. */
export function lotOf(db: DatabaseSync, e: Ev): { listingId: number | null; acquisitionId: number | null } {
  const a = e.args;
  if (e.contract === "distributor") return { listingId: a.listingId === undefined ? null : Number(a.listingId), acquisitionId: a.acquisitionId === undefined ? null : Number(a.acquisitionId) };
  if (e.contract === "vault") return { listingId: listingOfVault(db, e.address), acquisitionId: e.name === "Receipted" ? Number(a.acquisitionId) : null };
  if (e.contract === "registry" && a.id !== undefined) return { listingId: Number(a.id), acquisitionId: null };
  if (e.contract === "launcher") return { listingId: Number(a.listingId), acquisitionId: null };
  return { listingId: null, acquisitionId: null };
}

const lot = (db: DatabaseSync, l: number, q: number) => db.prepare(`INSERT OR IGNORE INTO lots (listing_id, acquisition_id) VALUES (?, ?)`).run(l, q);

/** Apply one event to the derived tables. Pure in the sense that matters: the same events in the same order give the same tables. */
export function apply(db: DatabaseSync, e: Ev): void {
  const a = e.args, s = (k: string) => String(a[k]), n = (k: string) => Number(a[k]);
  const run = (sql: string, ...p: (string | number | null)[]) => db.prepare(sql).run(...p);
  if (e.contract === "registry") {
    switch (e.name) {
      // docs/v2/vault-v2.md. A v2 listing emits this in the same transaction as its `Listed`, and `Listed` is folded first
      // (IDENTITY_EVENTS), so the row is always there to update. A v1 listing emits nothing and stays at the default.
      case "AutoBuyerNamed": {
        run(`UPDATE listings SET autonomous = 1, buyer = ?, max_in_flight = ? WHERE id = ?`,
          s("buyer"), String(n("maxInFlight")), n("id"));
        return;
      }
      case "Listed": {
        const m = db.prepare(`SELECT btc_vault, root FROM listing_meta WHERE id = ?`).get(n("id")) as { btc_vault: string | null; root: string | null } | undefined;
        run(`INSERT OR REPLACE INTO listings (id, kind, vault, name, ref, btc_vault, root, block, ts, tx) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
          n("id"), KINDS[n("kind")] ?? "inscriptions", s("vault"), s("name"), s("ref"), m?.btc_vault ?? null, m?.root ?? null, e.block, e.ts, e.tx);
        return;
      }
      case "Paused": run(`UPDATE listings SET paused = ? WHERE id = ?`, a.paused ? 1 : 0, n("id")); return;
      case "RootProposed": run(`UPDATE listings SET pending_root = ?, pending_root_at = ? WHERE id = ?`, s("root"), n("readyAt"), n("id")); return;
      case "RootSet": run(`UPDATE listings SET root = ?, pending_root = NULL, pending_root_at = NULL WHERE id = ?`, s("root"), n("id")); return;
      case "BtcVaultProposed": run(`UPDATE listings SET pending_btc = ?, pending_btc_at = ? WHERE id = ?`, s("btcVault"), n("readyAt"), n("id")); return;
      case "BtcVaultSet": run(`UPDATE listings SET btc_vault = ?, pending_btc = NULL, pending_btc_at = NULL WHERE id = ?`, s("btcVault"), n("id")); return;
      default: run(`INSERT OR REPLACE INTO roles (block, log_index, name, args, ts) VALUES (?, ?, ?, ?, ?)`, e.block, e.logIndex, e.name, JSON.stringify(a), e.ts); return;
    }
  }
  if (e.contract === "launcher" && e.name === "Launched") {
    run(`INSERT OR REPLACE INTO coins (coin, creator, listing_id, curve, router, block, ts, tx) VALUES (?, ?, ?, ?, ?, ?, ?, ?)`,
      s("coin"), s("creator"), n("listingId"), s("curve"), s("router"), e.block, e.ts, e.tx);
    return;
  }
  if (e.contract === "router") {
    const coin = coinOfRouter(db, e.address);
    if (!coin) return;                                   // a log from an address we never learnt as a router: not ours
    if (e.name === "Harvested") {
      run(`INSERT OR REPLACE INTO harvests (block, log_index, coin, claimed, to_vault, to_treasury, ts, tx) VALUES (?, ?, ?, ?, ?, ?, ?, ?)`,
        e.block, e.logIndex, coin, s("claimed"), s("toVault"), s("toTreasury"), e.ts, e.tx);
      if (BigInt(s("toTreasury")) > 0n) run(`UPDATE coins SET deferred = '0' WHERE coin = ?`, coin);   // a paid treasury leg carries what was owed
    } else if (e.name === "TreasuryDeferred") run(`UPDATE coins SET deferred = ? WHERE coin = ?`, s("owed"), coin);
    else if (e.name === "TokenForwarded") run(`INSERT OR REPLACE INTO forwards (block, log_index, coin, token, amount, ts, tx) VALUES (?, ?, ?, ?, ?, ?, ?)`,
      e.block, e.logIndex, coin, s("token"), s("amount"), e.ts, e.tx);
    return;
  }
  /**
   * docs/phase10.md: Pons's curve, one contract per coin. A curve we never learnt from a `Launched` is not ours and is dropped,
   * the same rule the router follows. `CurveCompleted` is graduation: the curve's last act, and the mark the chart draws.
   */
  if (e.contract === "curve") {
    const coin = coinOfCurve(db, e.address);
    if (!coin) return;
    if (e.name === "CurveCompleted") {
      run(`UPDATE coins SET graduated_block = ?, graduated_ts = ?, graduated_tx = ?, graduated_quote = ?, graduated_coins = ? WHERE coin = ?`,
        e.block, e.ts, e.tx, s("quoteOut"), s("tokenOut"), coin);
      return;
    }
    if (e.name !== "CurveBuy" && e.name !== "CurveSell") return;
    const buy = e.name === "CurveBuy";
    const quote = BigInt(buy ? s("quoteIn") : s("quoteOut")), coins = BigInt(buy ? s("tokensOut") : s("tokensIn"));
    const fee = BigInt(s("fee")), tax = BigInt(s("tax"));
    run(`INSERT OR REPLACE INTO trades (block, log_index, coin, venue, side, quote, coins, fee, tax, price, trader, recipient, ts, tx)
      VALUES (?, ?, ?, 'curve', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`, e.block, e.logIndex, coin, buy ? "buy" : "sell", String(quote), String(coins),
      String(fee), String(tax), curvePrice(buy, quote, coins, fee, tax), buy ? s("buyer") : s("seller"), s("recipient"), e.ts, e.tx);
    return;
  }
  /**
   * The v4 pool a graduated coin trades in. `Initialize` names the pool, and only one whose currencies include a coin we
   * launched is kept. In a `Swap` a negative amount is what the pool paid out, so the trader BOUGHT the coin when the coin
   * leg is negative. `sender` is the swap router the trade came through, not the person: the API says so.
   */
  if (e.contract === "pool") {
    if (e.name === "Initialize") {
      const c0 = s("currency0"), c1 = s("currency1");
      const coin = (db.prepare(`SELECT coin FROM coins WHERE coin IN (?, ?)`).get(c0, c1) as { coin: string } | undefined)?.coin;
      if (!coin) return;                                  // some other project's pool on the same PoolManager
      run(`INSERT OR REPLACE INTO pools (pool_id, coin, coin_is_0, quote_token, fee, tick_spacing, hooks, block, ts) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)`,
        s("id").toLowerCase(), coin, coin === c0 ? 1 : 0, coin === c0 ? c1 : c0, n("fee"), n("tickSpacing"), s("hooks"), e.block, e.ts);
      return;
    }
    if (e.name !== "Swap") return;
    const pool = db.prepare(`SELECT coin, coin_is_0 FROM pools WHERE pool_id = ?`).get(s("id").toLowerCase()) as { coin: string; coin_is_0: number } | undefined;
    if (!pool) return;
    const a0 = BigInt(s("amount0")), a1 = BigInt(s("amount1"));
    const coinAmt = pool.coin_is_0 ? a0 : a1, quoteAmt = pool.coin_is_0 ? a1 : a0;
    if (coinAmt === 0n) return;                           // a swap that moved no coin is not a trade in it
    const abs = (x: bigint) => (x < 0n ? -x : x);
    run(`INSERT OR REPLACE INTO trades (block, log_index, coin, venue, side, quote, coins, fee, tax, price, trader, recipient, ts, tx)
      VALUES (?, ?, ?, 'pool', ?, ?, ?, NULL, NULL, ?, ?, NULL, ?, ?)`, e.block, e.logIndex, pool.coin, coinAmt < 0n ? "buy" : "sell",
      String(abs(quoteAmt)), String(abs(coinAmt)), poolPrice(quoteAmt, coinAmt), s("sender"), e.ts, e.tx);
    return;
  }
  /**
   * docs/phase13.md §2. The coin itself, as an ERC-20: every balance this indexer reports is the sum of these deltas and
   * nothing else. A `Transfer` from an address we never learnt as a coin of ours is dropped, the rule the curve and the
   * router follow, and a zero-value transfer (which ERC-20 permits) moves nothing.
   */
  if (e.contract === "coin") {
    if (e.name !== "Transfer") return;
    const coin = (db.prepare(`SELECT coin FROM coins WHERE coin = ?`).get(e.address) as { coin: string } | undefined)?.coin;
    if (!coin) return;
    const v = BigInt(s("value"));
    move(db, coin, s("from"), -v);
    move(db, coin, s("to"), v);
    return;
  }
  if (e.contract === "vault") {
    const l = listingOfVault(db, e.address);
    if (l === null) return;
    const bondEv = (type: string, who: string | null, amount: string) => run(`INSERT OR REPLACE INTO bond_events (block, log_index, listing_id, type, who, amount, ts, tx)
      VALUES (?, ?, ?, ?, ?, ?, ?, ?)`, e.block, e.logIndex, l, type, who, amount, e.ts, e.tx);
    switch (e.name) {
      case "Deposited": run(`INSERT OR REPLACE INTO deposits (block, log_index, listing_id, coin, amount, ts) VALUES (?, ?, ?, ?, ?, ?)`, e.block, e.logIndex, l, s("coin"), s("amount"), e.ts); return;
      case "CapPosted": run(`INSERT OR REPLACE INTO caps (block, log_index, listing_id, cap, expiry, ts) VALUES (?, ?, ?, ?, ?, ?)`, e.block, e.logIndex, l, s("capWei"), n("expiry"), e.ts); return;
      case "Declared": run(`INSERT OR REPLACE INTO purchases (listing_id, purchase_id, status, max_eth, quantity, releasable_at, cap_ref, declared_ts, tx)
        VALUES (?, ?, 'declared', ?, ?, ?, ?, ?, ?)`, l, n("id"), s("maxEth"), s("quantity"), n("releasableAt"), s("capRef"), e.ts, e.tx); return;
      case "Cancelled": run(`UPDATE purchases SET status = 'cancelled', closed_ts = ? WHERE listing_id = ? AND purchase_id = ?`, e.ts, l, n("id")); return;
      case "Released": run(`UPDATE purchases SET status = 'released', released_ts = ?, released = ?, purchaser = ?, deadline = ? WHERE listing_id = ? AND purchase_id = ?`,
        e.ts, s("amount"), s("purchaser"), n("deadline"), l, n("id")); return;
      case "Charged": {
        const seq = (db.prepare(`SELECT count(*) AS c FROM charges WHERE listing_id = ? AND purchase_id = ?`).get(l, n("id")) as { c: number }).c;
        run(`INSERT INTO charges (listing_id, purchase_id, coin, amount, seq) VALUES (?, ?, ?, ?, ?)`, l, n("id"), s("coin"), s("amount"), seq); return;
      }
      case "Receipted":
        run(`INSERT OR REPLACE INTO acquisitions (listing_id, acquisition_id, purchase_id, coin, asset_ref, amount, btc_txid, sats, ts, tx, block) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
          l, n("acquisitionId"), n("id"), s("coin"), s("assetRef"), s("amount"), btcTxid(s("btcTxid")), s("satsPaid"), e.ts, e.tx, e.block);
        lot(db, l, n("acquisitionId")); return;
      case "Settled": run(`UPDATE purchases SET status = 'settled', closed_ts = ? WHERE listing_id = ? AND purchase_id = ?`, e.ts, l, n("id")); return;
      case "Returned": {
        const p = db.prepare(`SELECT returned FROM purchases WHERE listing_id = ? AND purchase_id = ?`).get(l, n("id")) as { returned: string } | undefined;
        run(`UPDATE purchases SET returned = ? WHERE listing_id = ? AND purchase_id = ?`, add(p?.returned, s("amount")), l, n("id")); return;
      }
      case "WrittenOff": run(`UPDATE purchases SET status = 'written-off', written_off = ?, closed_ts = ? WHERE listing_id = ? AND purchase_id = ?`, s("amount"), e.ts, l, n("id")); return;
      case "BondPosted": bondEv("posted", s("from"), s("amount")); return;
      case "BondWithdrawn": bondEv("withdrawn", s("to"), s("amount")); return;
      case "BondSlashed": {
        bondEv("slashed", null, s("amount"));
        const p = db.prepare(`SELECT slashed FROM purchases WHERE listing_id = ? AND purchase_id = ?`).get(l, n("id")) as { slashed: string } | undefined;
        run(`UPDATE purchases SET slashed = ? WHERE listing_id = ? AND purchase_id = ?`, add(p?.slashed, s("amount")), l, n("id")); return;
      }
    }
    return;
  }
  if (e.contract === "distributor") {
    if (e.name === "MinimumPosted") { run(`INSERT OR REPLACE INTO minimums (listing_id, units, expiry) VALUES (?, ?, ?)`, n("listingId"), s("units"), n("expiry")); return; }
    if (e.name === "PayoutRequested") { run(`INSERT OR REPLACE INTO payouts (payout_id, listing_id, holder, amount, destination, requested_ts) VALUES (?, ?, ?, ?, ?, ?)`,
      n("payoutId"), n("listingId"), s("holder"), s("amount"), s("destination"), e.ts); return; }
    if (e.name === "PayoutRecorded") { run(`UPDATE payouts SET txid = ?, recorded_ts = ? WHERE payout_id = ?`, btcTxid(s("txid")), e.ts, n("payoutId")); return; }
    if (e.name === "PayoutOverdue") { run(`UPDATE payouts SET overdue = 1 WHERE payout_id = ?`, n("payoutId")); return; }
    const l = n("listingId"), q = n("acquisitionId");
    lot(db, l, q);
    const set = (sql: string, ...p: (string | number | null)[]) => run(`UPDATE lots SET ${sql} WHERE listing_id = ? AND acquisition_id = ?`, ...p, l, q);
    switch (e.name) {
      case "Chosen": {
        const mode = MODES[n("mode")] ?? "unset";
        set(`mode = ?, named = ?, state = CASE WHEN state = 'undecided' THEN ? ELSE state END`, mode, mode === "named" ? s("named") : null,
          mode === "hold" ? "held" : mode === "never" ? "never-delivered" : "undecided");
        return;
      }
      case "DrawOpened": set(`state = 'draw-open', root = ?, total = ?, uri = ?, challenge_ends = ?, opened_ts = ?`, s("root"), s("totalTickets"), s("uri"), n("challengeEnds"), e.ts); return;
      case "DrawCancelled": set(`state = 'cancelled'`); return;
      case "DrawPinned": set(`state = 'draw-pinned', round = ?, pinned_ts = ?`, n("round"), e.ts); return;
      case "DrawResolved": set(`state = 'draw-resolved', winning_ticket = ?, resolved_ts = ?`, s("winningTicket"), e.ts); return;
      case "Won": set(`state = 'won', winner = ?, won_ts = ?`, s("winner"), e.ts); return;
      case "DrawVoided": set(`state = 'voided', voided_ticket = ?, voided_ts = ?`, s("unclaimedTicket"), e.ts); return;
      // the contract's delivery clock runs from the FIRST address; a correction inside the hour changes the address, not the clock
      case "DestinationSet": set(`destination = ?, destination_ts = COALESCE(destination_ts, ?)`, s("destination"), e.ts); return;
      case "Delivered": set(`state = 'delivered', delivered_txid = ?, delivered_ts = ?`, btcTxid(s("txid")), e.ts); return;
      case "DeliveryOverdue": set(`overdue = 1`); return;
      case "DistributionOpened":
        run(`INSERT OR REPLACE INTO distributions (listing_id, acquisition_id, state, root, total, uri, challenge_ends, opened_ts) VALUES (?, ?, 'open', ?, ?, ?, ?, ?)`,
          l, q, s("root"), s("total"), s("uri"), n("challengeEnds"), e.ts);
        set(`state = 'distribution-open'`); return;
      case "DistributionCancelled": run(`UPDATE distributions SET state = 'cancelled' WHERE listing_id = ? AND acquisition_id = ?`, l, q); set(`state = 'cancelled'`); return;
      case "ShareClaimed": {
        const d = db.prepare(`SELECT claimed FROM distributions WHERE listing_id = ? AND acquisition_id = ?`).get(l, q) as { claimed: string } | undefined;
        run(`UPDATE distributions SET claimed = ?, claims = claims + 1 WHERE listing_id = ? AND acquisition_id = ?`, add(d?.claimed, s("amount")), l, q); return;
      }
      case "RolledOver":
        run(`UPDATE distributions SET state = 'rolled', rolled_coin = ?, rolled_amount = ?, rolled_ts = ? WHERE listing_id = ? AND acquisition_id = ?`, s("coin"), s("amount"), e.ts, l, q);
        set(`state = 'distribution-rolled'`); return;
    }
  }
}

/** Empty every derived table and fold the whole events table again. Used after a rollback, and by the tests to prove the fold is a function of the events. */
export function rebuild(db: DatabaseSync): number {
  resetDerived(db);
  const rows = db.prepare(`SELECT block_number, log_index, tx, address, contract, name, args, ts FROM events ${FOLD_ORDER_SQL}`).all() as
    { block_number: number; log_index: number; tx: string; address: string; contract: string; name: string; args: string; ts: number | null }[];
  for (const r of rows) apply(db, { block: r.block_number, logIndex: r.log_index, tx: r.tx, address: r.address, contract: r.contract, name: r.name, args: JSON.parse(r.args), ts: r.ts });
  return rows.length;
}
