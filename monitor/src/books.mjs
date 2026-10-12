// W11 (docs/v9/canister-only-solana.md §10.3): the canister-only home's own state, read from four public queries and paged. Pure
// functions over the plain-decoded replies (`candid.mjs`: treasury_status, pool_desks, coins_log, desk_status); no network.
//  - `books-halted` (SEV1, persistent): a collection stopped by an incident (`pre_sign` refused, or the reconciliation found the chain
//    short): nothing of that collection signs until the operator looks and clears (desk_clear_halt);
//  - `books-drand-disagree` (SEV2, persistent): two drand relays served different bytes (or bytes that do not verify): no ticket and no
//    raffle of that round resolves meanwhile;
//  - `books-wait` (SEV3 at 6 h, SEV2 at 24 h): a treasury, coins or pool step that could not be made (a transfer refused, a config not
//    covered, a swap refused by simulation), still waiting;
//  - `books-commit-stuck` (SEV3): a raffle snapshot published an hour ago and not committed;
//  - `books-reconcile-stale` (SEV3): intents open and no reconciliation for an hour; `books-unexplained` (SEV1): a Rook memo on the
//    chain no link explains (a mint among them: an unknown MintTo);
//  - `books-attention` (SEV3): intents needing the operator; `books-off` (INFO): a module switched off; `books-resume` (INFO): a pause
//    lifting soon.
const HOUR = 3_600;
export const WAIT_SEV3_S = 6 * HOUR, WAIT_SEV2_S = 24 * HOUR, COMMIT_STUCK_S = HOUR, RECONCILE_STALE_S = HOUR, DESK_READ_STALE_S = HOUR;

const o = (x) => (Array.isArray(x) ? x[0] : x);
const n = (x) => Number(x ?? 0);
const hours = (s) => `${Math.round(s / HOUR)} h`;

function waitAlerts({ buyer, waits, nowS, what }) {
  const out = [];
  for (const w of waits ?? []) {
    const age = nowS - n(w.since);
    if (age < WAIT_SEV3_S) continue;
    out.push({ kind: "canister", sev: age >= WAIT_SEV2_S ? "SEV2" : "SEV3", persistent: true, key: `books-wait:${buyer}:${what}:${w.key}`,
      text: `the buyer ${buyer}'s ${what} step ${w.key} has waited ${hours(age)} (${n(w.tries)} tries): ${String(w.why).slice(0, 300)}` });
  }
  return out;
}

/** Pure. `treasury`: `treasury_status`; `desks`: `pool_desks`; `coins`: `coins_log` ([fees, waits, last]); `desk`: `desk_status`. Any may be null. */
export function booksAlerts({ buyer, treasury, desks, coins, desk, nowS }) {
  const alerts = [];
  if (treasury) {
    alerts.push(...waitAlerts({ buyer, waits: treasury.waits, nowS, what: "treasury" }));
    const dis = o(treasury.drand_disagreement);
    if (dis) alerts.push({ kind: "canister", sev: "SEV2", persistent: true, key: `books-drand-disagree:${buyer}:${dis[0]}`,
      text: `the buyer ${buyer}'s drand relays disagree on round ${dis[0]} (since ${new Date(n(dis[1]) * 1000).toISOString()}): ${String(dis[2]).slice(0, 300)}. No redemption ticket and no raffle pinned at that round resolves until two relays serve the same bytes that verify; look at the relays (drand_urls).` });
    for (const [coll, c] of treasury.commits ?? []) {
      const age = nowS - n(c.taken_at);
      if (age >= COMMIT_STUCK_S) alerts.push({ kind: "canister", sev: "SEV3", persistent: true, key: `books-commit-stuck:${buyer}:${coll}:${c.round}`,
        text: `the buyer ${buyer}'s raffle snapshot of collection ${coll} round ${c.round} was published ${hours(age)} ago and is not committed (the books want a later finalized slot than ${c.snapshot_slot}; is the clock read failing?)` });
    }
    for (const [what, coll, at] of treasury.resumes ?? []) {
      const left = n(at) - nowS;
      if (left >= 0 && left <= HOUR) alerts.push({ kind: "canister", sev: "INFO", key: `books-resume:${buyer}:${Object.keys(what)[0]}:${coll}:${at}`,
        text: `the pause of ${Object.keys(what)[0]} ${coll} on the buyer ${buyer} lifts in ${Math.round(left / 60)} min (resume asked 24 h ago); pause it again to keep it stopped.` });
    }
    if (treasury.off) alerts.push({ kind: "canister", sev: "INFO", key: `books-off:${buyer}:cranks`, text: `the buyer ${buyer}'s treasury cranks are switched off (cranks_set_off): no allocation, distribution, draw or raffle runs.` });
  }
  if (desks) {
    for (const d of desks) {
      if (d.pair && n(d.read_at) > 0 && nowS - n(d.read_at) >= DESK_READ_STALE_S) alerts.push({ kind: "canister", sev: "SEV3", persistent: true, key: `books-desk-stale:${buyer}:${d.coll}`,
        text: `the buyer ${buyer}'s pool desk of collection ${d.coll} last read its pair ${hours(nowS - n(d.read_at))} ago: the desk is off, the pool desk is switched off, or every pass fails before the read.` });
    }
  }
  if (coins) {
    const [, waits] = coins;
    alerts.push(...waitAlerts({ buyer, waits, nowS, what: "coins" }));
  }
  if (desk) {
    for (const [coll, why] of desk.halted ?? []) alerts.push({ kind: "canister", sev: "SEV1", persistent: true, key: `books-halted:${buyer}:${coll}`,
      text: `collection ${coll} of the buyer ${buyer} is STOPPED by an incident: ${String(why).slice(0, 300)}. Nothing of it signs (its intents are refunded) until the operator looks at the books against the chain and clears it (desk_clear_halt).` });
    const r = o(desk.report);
    for (const memo of r?.unexplained ?? []) alerts.push({ kind: "canister", sev: "SEV1", persistent: true, key: `books-unexplained:${buyer}:${memo}`,
      text: `the buyer ${buyer}'s reconciliation found a Rook memo on Solana that no link of its books explains: ${memo}. A mint among them is an unknown MintTo of wX: SEV1 (§10.2).` });
    const live = n(desk.open) + n(desk.funded) + n(desk.executing);
    if (live > 0 && n(desk.last_reconcile) > 0 && nowS - n(desk.last_reconcile) >= RECONCILE_STALE_S) alerts.push({ kind: "canister", sev: "SEV3", persistent: true, key: `books-reconcile-stale:${buyer}`,
      text: `the buyer ${buyer} has ${live} intents live and last reconciled its books against the chain ${hours(nowS - n(desk.last_reconcile))} ago (it reconciles every 10 min while running).` });
    if (n(desk.needing_attention) > 0) alerts.push({ kind: "canister", sev: "SEV3", persistent: true, key: `books-attention:${buyer}:${desk.needing_attention}`,
      text: `${desk.needing_attention} intent(s) of the buyer ${buyer} need the operator (a stuck transaction, a refund that cannot be made): intent_status lists them.` });
    if (desk.paused) alerts.push({ kind: "canister", sev: "INFO", key: `books-off:${buyer}:desk`, text: `the buyer ${buyer}'s intent desk is paused (desk_set_paused): no new mint, burn or redemption; open deposits are still returned.` });
  }
  return alerts;
}
