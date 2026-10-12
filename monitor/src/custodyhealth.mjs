// The buyer canister's own health, from its public queries (docs/audit/2026-09-26/fixes-D-canister.md §6, fixes-W2-monitor.md
// §1.8; audit E-08): the ticket-log scanner that deliveries now wait on (H-1), a missing UniSat key (lost at every upgrade:
// deliveries and buys then refuse as "unverified"), and holder deposits parked waiting for a proof (H-3).
import { createHash } from "node:crypto";
import { conversionAlerts } from "./conversions.mjs";
import { solLineAlerts, solLinesAlerts } from "./ethnft.mjs";
import { booksAlerts } from "./books.mjs";

export const SCAN_FAILING_MS = 60 * 60 * 1000;
/** canister review2 N-2: a refused redemption's markUndeliverable still Due or Sent after this long pages (its refund waits 30 days). */
export const ATTEST_LATE_MS = 24 * 60 * 60 * 1000;
/** canister review2 §2.2 / §8: the conflict-out's fee CEILING is 100,000 sats (MAX_CONFLICT_FEE_SATS); only an entry that large reaches it. */
export const CONFLICT_CEILING_SATS = 100_000n;
/**
 * The smallest largest-entry the custody fee pool may hold before it pages (config `min_fee_pool_sats`; script/golive-check.sh
 * --min-fee-pool-sats): 20,000 sats by default, the operator's choice (2026-09-26) not to hold 100,000 for fee spikes. Under the
 * ceiling but at or above this is an INFO note: in a large fee spike a stuck delivery is cancelled more slowly; the gated Wrapper
 * (refunds only after markUndeliverable or 30 days) keeps that from becoming a loss.
 */
export const DEFAULT_MIN_FEE_POOL_SATS = 20_000n;
/** The canister's own log lines the watcher must see (fixes-R3-canister.md §9), by what they mean. */
export const LOG_ALERTS = [
  { re: /TICKET LOGS NOT READ/, sev: "SEV2", key: "ticket-logs-line", what: "the ticket-log scanner could not read: deliveries wait (audit D, H-1)" },
  { re: /PIN WATCH/, sev: "SEV2", what: "a delivery in a mempool has an unconfirmed child on the redeemer's output, or a bump was not taken: a pin (canister review2 N-2). The re-priced conflict-out answers it inside the margin; watch that it confirms" },
  { re: /FEE ESTIMATES DISAGREE/, sev: "SEV2", what: "the explorers' fee estimates disagree by more than 4× (and 20 sat/vB): the canister pays the upper median; one explorer is wrong or lying (review2 N-3)" },
  { re: /FEE POOL: .*quarantined/, sev: "SEV2", what: "a custody fee-pool entry failed three reads while others read fine and was quarantined (review2 N-4): the operator removes or re-adds it" },
  { re: /UNREADABLE ticket events/, sev: "SEV1", what: "a ticket's events cannot be read at all (review2 N-1): its delivery waits and it will be refunded" },
  { re: /read by ticket/, sev: "INFO", what: "a block too big to read whole is being read ticket by ticket (review2 N-1): the scan goes on" },
  { re: /markUndeliverable mined \(reverted/, sev: "SEV2", what: "a markUndeliverable reverted (the ticket moved on first): check the ticket on chain" },
  { re: /markUndeliverable would not succeed/, sev: "SEV2", what: "the canister's markUndeliverable would revert now (is the canister still the wrapper's custodian?)" },
];
const opt = (x) => (Array.isArray(x) ? (x.length ? x[0] : null) : x ?? null);
const phaseOf = (p) => Object.entries(p ?? {})[0] ?? [null, null];
const h = (s) => createHash("sha256").update(s).digest("hex").slice(0, 16);

/** Pure. `read`: { ticketLogs (opt TicketLogs), log: [text], ordProviders: [[name, base]], holderDeposits, feePool (FeePool), redemptions }.
 *  `minFeePoolSats`: the fee pool's minimum largest entry (bigint; default DEFAULT_MIN_FEE_POOL_SATS). */
export function judgeCanister({ buyer, read, prev = {}, now, minFeePoolSats = DEFAULT_MIN_FEE_POOL_SATS }) {
  const alerts = [], next = { scanFailingSince: null };
  const t = opt(read.ticketLogs);
  if (t && Number(t.failures) > 0) {
    next.scanFailingSince = prev.scanFailingSince ?? now;
    if (now - next.scanFailingSince >= SCAN_FAILING_MS) alerts.push({ kind: "canister", sev: "SEV2", persistent: true, key: `ticket-logs-failing:${next.scanFailingSince}`,
      text: `the buyer ${buyer}'s ticket-log scanner has FAILED ${t.failures} tick(s) in a row, for ${Math.round((now - next.scanFailingSince) / 60000)} min: ${opt(t.last_error) ?? "(no error text)"}. Deliveries wait on these logs (audit D, H-1): no redemption is delivered until it reads again. Check both 4663 RPCs (ordofi refuses more than 10,000 blocks a query).` });
  }
  for (const line of read.log ?? []) {
    const m = LOG_ALERTS.find((x) => x.re.test(line));
    if (!m) continue;
    alerts.push({ kind: "canister", sev: m.sev, key: `${m.key ?? "canister-line"}:${h(line)}`, text: `the buyer ${buyer} says: ${line.slice(0, 600)}\n(${m.what})` });
  }
  // review2 N-1: tickets whose events no provider can return; a block being read by ticket
  const unreadable = opt(t?.unreadable) ?? [];
  if (unreadable.length) alerts.push({ kind: "canister", sev: "SEV1", persistent: true, key: `ticket-logs-unreadable:${h(JSON.stringify(unreadable))}`,
    text: `the buyer ${buyer} CANNOT READ the events of ${unreadable.length} ticket(s) (block, ticket): ${JSON.stringify(unreadable.slice(0, 10))}. Their deliveries wait and they will be refunded; a wrapper event larger than 2 MB alone should not exist: look at those blocks.` });
  const dense = opt(t?.dense);
  if (dense) alerts.push({ kind: "canister", sev: "INFO", key: `ticket-logs-dense:${dense.block}`,
    text: `the buyer ${buyer} is reading block ${dense.block} ticket by ticket (too big to read whole at 2 MB; through ticket ${opt(dense.after) ?? "none yet"}): the cursor waits on it and moves on when it is done.` });
  // review2 N-4 and §2.2: the fee pool
  const pool = read.feePool;
  if (pool) {
    const q = opt(pool.quarantined) ?? [];
    if (q.length) alerts.push({ kind: "canister", sev: "SEV2", persistent: true, key: `fee-pool-quarantined:${h(JSON.stringify(q.map((e) => e.outpoint)))}`,
      text: `the buyer ${buyer}'s custody fee pool has ${q.length} QUARANTINED entr${q.length === 1 ? "y" : "ies"} (three failed reads while others read): ${q.map((e) => `${e.outpoint.txid}:${e.outpoint.vout} (${e.value} sats, ${e.source})`).join(", ")}. The operator removes it (remove_custody_fee_input) or re-adds it once it reads (add_custody_fee_input).` });
    const top = (pool.entries ?? []).reduce((m, e) => (BigInt(e.value) > m ? BigInt(e.value) : m), 0n);
    const min = BigInt(minFeePoolSats);
    if (top < min) alerts.push({ kind: "canister", sev: "SEV2", persistent: true, key: `fee-pool-small:${buyer}`,
      text: `the buyer ${buyer}'s custody fee pool has no usable entry of ${min} sats or more (the configured minimum, min_fee_pool_sats; largest ${top}): a stuck delivery's conflict-out may not be re-priced at all, so a pin may outlast it (canister review2 N-2). The operator adds one (add_custody_fee_input).` });
    else if (top < CONFLICT_CEILING_SATS) alerts.push({ kind: "canister", sev: "INFO", key: `fee-pool-under-ceiling:${buyer}:${top}`,
      text: `the buyer ${buyer}'s custody fee pool's largest entry is ${top} sats (at or above the ${min}-sat minimum, under the conflict-out's ${CONFLICT_CEILING_SATS}-sat ceiling): during a large fee spike a conflict-out cannot reach its ceiling, so a stuck delivery is cancelled more slowly. The gated Wrapper (refunds only after markUndeliverable or 30 days) keeps that from becoming a loss. No action needed unless fees spike.` });
  }
  // review2 N-2: a refused redemption's markUndeliverable, still Due or Sent after a day (its refund then waits 30 days)
  next.attestSince = {};
  for (const r of read.redemptions ?? []) {
    const [st] = phaseOf(opt(r.attest));
    // phase B2: on the Solana home, SolSent (out on the line)
    if (st !== "Due" && st !== "Sent" && st !== "SolSent") continue;
    const k = String(r.ticket), since = prev.attestSince?.[k] ?? now;
    next.attestSince[k] = since;
    if (now - since >= ATTEST_LATE_MS) alerts.push({ kind: "canister", sev: "SEV2", persistent: true, key: `attest-late:${buyer}:${k}`,
      text: `the buyer ${buyer} has not got ticket ${k}'s markUndeliverable done for ${Math.round((now - since) / 3_600_000)} h (${st}): until it lands, the ticket's holder waits deliverBy + 30 days for the refund instead of deliverBy + 72 h. Is the buyer paused (it attests only when running), out of cycles, or no longer the wrapper's custodian?` });
  }
  const providers = read.ordProviders ?? [];
  if (!providers.some(([name, base]) => /unisat/i.test(`${name} ${base}`))) alerts.push({ kind: "canister", sev: "SEV2", persistent: true, key: `unisat-missing:${buyer}`,
    text: `the buyer ${buyer} asks no UniSat provider (ord_providers: ${providers.map(([n]) => n).join(", ") || "none"}): its key is not set (it is lost at every upgrade). Deliveries and frog buys refuse as "unverified" until the operator calls set_unisat_key.` });
  for (const d of read.holderDeposits ?? []) {
    const [ph, body] = phaseOf(d.phase);
    if (ph === "Parked") alerts.push({ kind: "canister", sev: "INFO", key: `parked:${d.outpoint.txid}:${d.outpoint.vout}`,
      text: `a holder deposit ${d.outpoint.txid}:${d.outpoint.vout} (${body.inscription}) is PARKED: 14 days without a membership proof. The operator may hand in the proof (frog_proof) or refuse it (refuse_deposit returns it).` });
  }
  return { alerts, next };
}

export async function checkCanister({ cfg, prev = {}, now = Date.now(), ic }) {
  const b = cfg.buyer;
  const [ticketLogs, log, ordProviders, holderDeposits, feePool, redemptions, conversions, solLine, solLines] = await Promise.all([ic.ticketLogs(b), ic.log(b), ic.ordProviders(b), ic.holderDeposits(b),
    ic.feePool ? ic.feePool(b) : null, ic.redemptions ? ic.redemptions(b) : [], ic.conversions ? ic.conversions(b) : [],
    // phase B2: the canister-wide Solana line (a buyer built before it has no sol_line: none)
    ic.solLine ? ic.solLine(b).catch(() => null) : null,
    // W3: the four lines and the timer (a buyer built before W3 has no sol_lines: the one line above)
    ic.solLines ? ic.solLines(b).catch(() => null) : null]);
  // W11: the canister-only home's state (a buyer built before it has none of these: null)
  const [treasury, desks, coinsLog, deskStatus] = await Promise.all(["treasuryStatus", "poolDesks", "coinsLog", "deskStatus"].map((m) => (ic[m] ? ic[m](b).catch(() => null) : null)));
  const books = booksAlerts({ buyer: b, treasury, desks, coins: coinsLog, desk: deskStatus, nowS: Math.floor(now / 1000) });
  const r = judgeCanister({ buyer: b, read: { ticketLogs, log, ordProviders, holderDeposits, feePool, redemptions }, prev, now,
    minFeePoolSats: BigInt(cfg.min_fee_pool_sats ?? DEFAULT_MIN_FEE_POOL_SATS) });
  // docs/v6/relay-conversion.md §6: the Relay conversions' states
  const c = conversionAlerts({ buyer: b, conversions, nowS: Math.floor(now / 1000), prev: prev.conversions ?? {} });
  const line = Array.isArray(solLine) ? solLine[0] : solLine;
  const s = solLines ? solLinesAlerts({ buyer: b, reply: solLines, nowS: Math.floor(now / 1000) })
    : solLineAlerts({ buyer: b, line, nowS: Math.floor(now / 1000) });
  return { alerts: [...r.alerts, ...c.alerts, ...s, ...books], next: { ...r.next, conversions: c.next } };
}
