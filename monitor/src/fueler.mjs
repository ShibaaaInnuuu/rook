// docs/v7/cycles-from-fees.md §3.4 and §4.2 item 7, and docs/v9/fueler-solana.md §6: the fueler watched from outside (the monitor's
// project, not the fueler's). The fueler holds hot keys on the ops income; what proves it is still their only signer is the chain itself.
// Each poll, the chains are read FIRST and the fueler's /status second, so a transaction the fueler signs between the two reads can never
// look foreign:
//  - `ops-unexpected-tx` (SEV1, persistent): the EVM key's nonce on Ethereum (and on 4663 for the Robinhood home) is past the next nonce
//    the fueler journaled;
//  - `sol-unexpected-tx` (SEV1, persistent; the books home, docs/v9/canister-only-solana.md §9.2): a transaction SIGNED by the Solana hot
//    key that the fueler never journaled (its /status lists every signature it made, journaled before it was sent). Treat the key as
//    stolen: the operator runs the canister's treasury_set_ops(<a fresh key>) at once (the change applies after a day), then the fueler's
//    FUELER_SOL_KEYPAIR is rotated. The signatures that touch the hot key when the watch first starts are its baseline (incoming
//    transfers, the funding);
//  - the fueler's own alerts (cycles-autofund-failed = a stuck leg, -cap = a cap hit, -oracle, SEV2; -short and cycles-income-short, SEV3;
//    fueler-halt, SEV1);
//  - the ops wallet under its floor (SEV3): ETH on 4663, or SOL at the Solana hot key (the fueler's fee buffer);
//  - `fueler-not-fueling` (SEV2): the buyer's cycles under the fueler's trigger for `stuck_h` hours with no run open and no conversion since;
//  - a /status naming another key than the configured one (SEV1).
// The fueler being down is the liveness checker's page (its /healthz in cfg.liveness).
import { rpc } from "./abi.mjs";

const lc = (s) => String(s ?? "").toLowerCase();
const DAY = 86_400_000, HOUR = 3_600_000;
const opt = { tries: 2, timeoutMs: 15_000 };

async function nonceOf(urls, addr, http) {
  const got = [];
  for (const u of urls) { try { got.push(Number(BigInt(await rpc(u, "eth_getTransactionCount", [addr, "latest"], http, opt)))); } catch { /* another provider may answer */ } }
  return got.length ? Math.max(...got) : null;      // any provider that sees a higher nonce is evidence
}

/** The latest signatures touching `addr` at every provider (the union: any provider that sees one is evidence); null when none answered. */
async function solSigs(urls, addr, http) {
  const seen = new Map();
  let answered = false;
  for (const u of urls) {
    try {
      for (const s of (await rpc(u, "getSignaturesForAddress", [addr, { limit: 50, commitment: "confirmed" }], http, opt)) ?? []) if (s?.signature) seen.set(s.signature, s);
      answered = true;
    } catch { /* the next provider */ }
  }
  return answered ? seen : null;
}

/** Whether `addr` signed transaction `sig` (null when no provider returns it yet). */
async function solSigned(urls, sig, addr, http) {
  for (const u of urls) {
    try {
      const t = await rpc(u, "getTransaction", [sig, { encoding: "json", commitment: "confirmed", maxSupportedTransactionVersion: 0 }], http, opt);
      if (!t) continue;
      const m = t.transaction?.message ?? {}, n = m.header?.numRequiredSignatures ?? 0;
      return (m.accountKeys ?? []).slice(0, n).some((k) => (typeof k === "string" ? k : k?.pubkey) === addr);
    } catch { /* the next provider */ }
  }
  return null;
}

async function solBalance(urls, addr, http) {
  const got = [];
  for (const u of urls) { try { got.push(BigInt((await rpc(u, "getBalance", [addr, { commitment: "confirmed" }], http, opt))?.value ?? 0)); } catch { /* next */ } }
  return got.length ? got.reduce((m, x) => (x < m ? x : m)) : null;
}

/**
 * cfg.fueler: { status_url, ops, rpcs_1, rpcs_4663?, min_ops_eth?, sol_ops?, rpcs_sol?, min_sol?, trigger_t?, stuck_h? }. With `sol_ops` it
 * watches the Solana home (rpcs_4663 is then not read). `ic` (optional): { cycles(id) } for the not-fueling page. Returns { alerts, next }.
 */
export async function checkFueler({ cfg, prev = {}, now = Date.now(), http = fetch, ic = null }) {
  const f = cfg.fueler, ops = lc(f.ops), alerts = [], solHome = !!f.sol_ops;
  // ---- the chains first
  const chains = solHome ? [1] : [4663, 1];
  const chainNonce = {};
  for (const c of chains) chainNonce[c] = await nonceOf(c === 1 ? f.rpcs_1 ?? [] : f.rpcs_4663 ?? [], ops, http);
  const sigs = solHome ? await solSigs(f.rpcs_sol ?? [], f.sol_ops, http) : null;
  const solBal = solHome ? await solBalance(f.rpcs_sol ?? [], f.sol_ops, http) : null;
  let cycles = null;
  if (ic?.cycles && cfg.buyer) { try { cycles = BigInt(await ic.cycles(cfg.buyer)); } catch { /* the watcher pages an unreadable canister */ } }
  // ---- then the fueler
  let status = null;
  try {
    const r = await http(f.status_url, { signal: AbortSignal.timeout(15_000) });
    if (r.status === 200) status = await r.json();
  } catch { /* the liveness checker pages a fueler that does not answer */ }
  const next = { ...prev, chainNonce, at: now };
  // the Solana baseline: the signatures already there when the watch starts (and every one judged since) are not judged again
  const known = new Set(prev.solSeen ?? []);
  const firstSol = solHome && sigs && prev.solSeen === undefined;
  if (sigs) next.solSeen = firstSol ? [...sigs.keys()] : [...known];
  // the low-cycles clock (not-fueling): since when the buyer is under the trigger
  const triggerT = f.trigger_t ?? 40;
  if (cycles !== null) next.lowSince = cycles < BigInt(Math.round(triggerT * 1e6)) * 1_000_000n ? (prev.lowSince ?? now) : null;
  if (!status) return { alerts, next };

  if (status.address && lc(status.address) !== ops)
    alerts.push({ kind: "fueler", sev: "SEV1", persistent: true, key: `fueler-address:${lc(status.address)}`,
      text: `the fueler at ${f.status_url} signs as ${status.address}, not the configured ops ${ops}: the monitor is watching the wrong key, or the fueler runs another one. Check ${solHome ? "fueler.ops (its Ethereum address)" : "treasury.ops()"} and the fueler's FUELER_EVM_KEY.` });
  if (solHome && status.solAddress && status.solAddress !== f.sol_ops)
    alerts.push({ kind: "fueler", sev: "SEV1", persistent: true, key: `fueler-sol-address:${status.solAddress}`,
      text: `the fueler at ${f.status_url} holds the Solana hot key ${status.solAddress}, not the configured ${f.sol_ops}: check the canister's treasury_status().treasury.ops (treasury_set_ops) and the fueler's FUELER_SOL_KEYPAIR.` });
  for (const chain of chains) {
    const mine = status.nonces?.[chain], seen = chainNonce[chain];
    if (mine === undefined || mine === null || seen === null) continue;
    if (seen > Number(mine))
      alerts.push({ kind: "fueler", sev: "SEV1", persistent: true, key: `ops-unexpected-tx:${chain}:${mine}`,
        text: `ops-unexpected-tx: the ops address ${ops} has sent ${seen} transaction(s) on chain ${chain}, but the fueler journaled only ${mine}. Something else signed with the ops key. TREAT IT AS STOLEN: ${solHome ? "rotate FUELER_EVM_KEY now (on the Solana home it holds only bridged ETH, for minutes)" : "the Safe calls treasury.setOps(<a fresh address>) now, then rotate FUELER_EVM_KEY"} (docs/v7/cycles-from-fees.md §3.4).` });
  }
  if (solHome && sigs && !firstSol) {
    const mine = new Set(status.sol?.signatures ?? []);
    const judged = [...known];
    for (const sig of sigs.keys()) {
      if (known.has(sig) || mine.has(sig)) { if (!known.has(sig)) judged.push(sig); continue; }
      const signed = await solSigned(f.rpcs_sol ?? [], sig, f.sol_ops, http);
      if (signed === null) continue;                    // not readable yet: judged at the next poll
      judged.push(sig);
      if (signed)
        alerts.push({ kind: "fueler", sev: "SEV1", persistent: true, key: `sol-unexpected-tx:${sig}`,
          text: `sol-unexpected-tx: the Solana hot key ${f.sol_ops} signed ${sig}, which the fueler never journaled. Something else holds the key. TREAT IT AS STOLEN: the operator runs the canister's treasury_set_ops(<a fresh key>) now (the change applies after a day, and until then the canister's cranks pay the stolen key its share), stop the fueler, then rotate FUELER_SOL_KEYPAIR (docs/v9/fueler-solana.md §6).` });
    }
    next.solSeen = judged.slice(-500);
  }
  for (const a of status.alerts ?? []) {
    const sev = ["SEV1", "SEV2", "SEV3"].includes(a.sev) ? a.sev : "SEV2";
    alerts.push({ kind: "fueler", sev, persistent: sev !== "SEV3", key: `fueler:${a.kind}:${a.since}`, text: `${a.kind}: ${String(a.text ?? "").slice(0, 600)}` });
  }
  if (solHome) {
    const min = f.min_sol ?? 0.02;
    if (solBal !== null && solBal < BigInt(Math.round(min * 1e9)))
      alerts.push({ kind: "fueler", sev: "SEV3", key: `fueler-sol-low:${Math.floor(now / DAY)}`,
        text: `the fueler's Solana hot key ${f.sol_ops} holds ${(Number(solBal) / 1e9).toFixed(5)} SOL, under its ${min} SOL floor: it cannot pay its own fees or a run. The ops share arrives from the canister's cranks (TreasuryStep::Ops, once 0.1 SOL is owed); until fees come in, a top-up is by hand (≈ 0.05 SOL).` });
  } else {
    const minWei = BigInt(Math.round((f.min_ops_eth ?? 0.02) * 1e6)) * 1_000_000_000_000n;
    for (const u of f.rpcs_4663 ?? []) {
      try {
        const bal = BigInt(await rpc(u, "eth_getBalance", [ops, "latest"], http, opt));
        if (bal < minWei) alerts.push({ kind: "fueler", sev: "SEV3", key: `fueler-ops-low:${Math.floor(now / DAY)}`,
          text: `the ops wallet ${ops} holds ${(Number(bal) / 1e18).toFixed(5)} ETH on 4663, under its ${f.min_ops_eth ?? 0.02} ETH floor: the fueler cannot pay its own gas or a run. The ops share arrives with treasury.allocate(); until fees come in, a top-up is by hand.` });
        break;
      } catch { /* the next provider */ }
    }
  }
  // not fueling: the buyer has been under the trigger for stuck_h with no run open and no conversion since it went under
  const stuckMs = (f.stuck_h ?? 12) * HOUR;
  if (next.lowSince && now - next.lowSince >= stuckMs && !status.run) {
    const lastConv = Math.max(0, ...(status.conversions ?? []).map((c) => Number(c.at) * 1000));
    if (lastConv < next.lowSince)
      alerts.push({ kind: "fueler", sev: "SEV2", key: `fueler-not-fueling:${Math.floor(now / DAY)}`,
        text: `fueler-not-fueling: the buyer ${cfg.buyer} has held ${(Number(cycles) / 1e12).toFixed(2)} T, under the fueler's ${triggerT} T trigger, for ${Math.round((now - next.lowSince) / HOUR)} h, and the fueler has not converted since (its last decision: ${String(status.lastDecision ?? "none").slice(0, 300)}). Fund its hot key or top up by hand from the fallback reserve (golive-runbook §6).` });
  }
  return { alerts, next: { ...next, statusAt: now, fuelerNonces: status.nonces ?? null } };
}
