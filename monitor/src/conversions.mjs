// The Relay conversions, from the buyer's frog_conversions() (docs/v6/relay-conversion.md §6): an advance of one unit's budget
// turned into sats by Relay, then a frog. One is in flight at a time; each state that lasts too long, and each that should never
// happen, pages. Pure: the records in, the alerts and the next state out.

export const CONVERTING_LATE_S = 30 * 60;        // Relay's own estimate is ~5 min: half an hour is late
export const FUNDED_LATE_S = 2 * 3600;           // the runner is not quoting (RUNNER_RELAY off, Relay refusing, or every quote judged bad)
export const ARRIVED_LATE_S = 24 * 3600;         // sats in the float and no frog bought
const phaseOf = (p) => Object.entries(p ?? {})[0] ?? [null, null];
const eth = (wei) => `${(Number(BigInt(wei) / 10n ** 12n) / 1e6).toString()} ETH`;
const mins = (s) => `${Math.round(s / 60)} min`;

/**
 * Pure. `conversions`: frog_conversions() as plain JSON. `nowS`: seconds. `prev`: { firstSeen: { "frog:phase": seconds } } (for
 * phases that carry no time of their own). Returns { alerts, next }.
 */
export function conversionAlerts({ buyer, conversions = [], nowS, prev = {} }) {
  const alerts = [], next = { firstSeen: {} };
  for (const c of conversions) {
    const [ph, body] = phaseOf(c.phase);
    const f = String(c.frog), key = `${f}:${ph}`;
    const seen = next.firstSeen[key] = prev.firstSeen?.[key] ?? nowS;
    const who = `the buyer ${buyer}'s conversion for frog purchase #${f} (advance ${eth(c.advanced)}, attempt ${c.attempts})`;
    const note = c.note ? ` Its last note: ${String(c.note).slice(0, 300)}` : "";
    if (Number(c.attempts) > 1 && !["Returned", "Settled"].includes(ph))
      alerts.push({ kind: "canister", sev: "INFO", key: `conv-refund:${buyer}:${f}:${c.attempts}`,
        text: `${who}: Relay did not fill an earlier attempt and the ETH came back (a refund); it is quoting again (at most 3 attempts, then the ETH goes back to the module).${note}` });
    if (ph === "Converting") {
      const since = Number(body.since), late = nowS - since;
      if (late >= CONVERTING_LATE_S) alerts.push({ kind: "canister", sev: "SEV2", persistent: true, key: `conv-late:${buyer}:${f}:${since}`,
        text: `${who} is LATE: the deposit to Relay mined ${mins(late)} ago and no BTC (at least ${body.min_sats} sats) has reached the float three blocks deep, and no refund is seen. The advance is in Relay's hands (the trust stated on the Trust page). Check the order ${body.order_id} on relay.link; past its deadline (${new Date(Number(body.deadline) * 1000).toISOString()}) + 1 h the canister calls it Stuck.${note}` });
    } else if (ph === "Stuck") {
      alerts.push({ kind: "canister", sev: "SEV1", persistent: true, key: `conv-stuck:${buyer}:${f}`,
        text: `${who} is STUCK: ${body.why}. Nothing new is bought until it is resolved. Relay kept the advance past its deadline without a fill or a refund: recovery is Relay's support (docs/v6/relay-conversion.md §5.3). The exposure is this one advance.${note}` });
    } else if (ph === "Funded") {
      const since = Number(body.since ?? seen), late = nowS - since;
      if (late >= FUNDED_LATE_S) alerts.push({ kind: "canister", sev: "SEV2", persistent: true, key: `conv-funded:${buyer}:${f}:${since}`,
        text: `${who} has waited ${mins(late)} for a Relay quote with ${eth(body.wei)} at the canister's EVM address: the runner is not handing one in, or every one is refused. Check the runner's log (relay-quote-rejected, RUNNER_RELAY) and the canister's note. relay_give_back(${f}) sends the ETH back to the module instead.${note}` });
    } else if (ph === "Arrived") {
      if (nowS - seen >= ARRIVED_LATE_S) alerts.push({ kind: "canister", sev: "SEV2", persistent: true, key: `conv-arrived:${buyer}:${f}:${seen}`,
        text: `${who}: ${body.sats} sats have been in the float for ${Math.round((nowS - seen) / 3600)} h and no frog is bought: hand in an offer within them (frog_offer). The unit stays in transit, backed by these sats, until then.${note}` });
    } else if (ph === "Returned") {
      alerts.push({ kind: "canister", sev: "INFO", key: `conv-returned:${buyer}:${f}`,
        text: `${who} was RETURNED: ${eth(body.wei)} went back to the FloorModule (returnAdvance) after ${c.attempts} attempt(s); nothing was bought. Any shortfall against the unit's backing is in the module's AdvanceReturned event (loss).${note}` });
    }
  }
  return { alerts, next };
}
