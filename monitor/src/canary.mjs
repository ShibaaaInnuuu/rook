// The Bitcoin custody canary (docs/v6/k6-watcher-canary.md §4). Every address is read from two esplora providers; the
// canister's records are read AFTER them (the canister saves a spend before it first broadcasts it, so nothing the explorers
// already show can be missing from records read later). Every outbound spend must be one the canister recorded.
import { redactUrl } from "./config.mjs";
import { recordIndex } from "./records.mjs";

export const GRACE_MS = 15 * 60 * 1000;          // mempool propagation between two explorers
export const STILL_MS = 60 * 60 * 1000;          // G9: "provider disagreement persisting over 1 hour"
export const DOWN_MS = 30 * 60 * 1000;           // a provider silent this long is a watcher-error

export const host = (base) => { try { return new URL(base).host; } catch { return base; } };
// round 3 (bayc10-decisions.md §5): an explorer link is made from a provider base only when that base is a public esplora's (no
// userinfo, query or fragment; the path /api under /, /testnet, /testnet4 or /signet); any other path may hold a key: not printed
const link = (base, txid) => {
  let u;
  try { u = new URL(base); } catch { return `tx ${txid}`; }
  const path = u.pathname.replace(/\/api\/?$/, "").replace(/\/$/, "");
  if (u.username || u.password || u.search || u.hash || !/^(\/(testnet4?|signet))?$/.test(path)) return `${redactUrl(base)} tx ${txid}`;
  return `${u.origin}${path}/tx/${txid}`;
};
const sats = (n) => `${Number(n).toLocaleString("en-US")} sats`;

function direction(tx, address) {
  if ((tx.vin ?? []).some((i) => i.prevout?.scriptpubkey_address === address)) return "out";
  if ((tx.vout ?? []).some((o) => o.scriptpubkey_address === address)) return "in";
  return null;
}

function where(tx) {
  return tx.status?.confirmed ? `confirmed at height ${tx.status.block_height}` : "unconfirmed (mempool)";
}

function describeTx(tx, address, label) {
  const spent = (tx.vin ?? []).filter((i) => i.prevout?.scriptpubkey_address === address);
  const lines = [`tx ${tx.txid}, ${where(tx)}`];
  if (spent.length) lines.push(`spends ${sats(spent.reduce((s, i) => s + (i.prevout.value ?? 0), 0))} from ${label} (${spent.map((i) => `${i.txid}:${i.vout}`).join(", ")})`);
  lines.push(`pays: ${(tx.vout ?? []).map((o) => `${sats(o.value)} → ${o.scriptpubkey_address ?? o.scriptpubkey_type}${o.scriptpubkey_address === address ? ` (${label})` : ""}`).join("; ")}`);
  if (tx.fee !== undefined) lines.push(`fee ${sats(tx.fee)}`);
  return lines;
}

/**
 * One address, judged. Pure. `views`: [{ name, base, txs | null, error }] (one per provider, in order). `index`: recordIndex().
 * `prev`: this address's saved state. `expect`: this address's expected inbounds [{ id, min_sats, note }]. Returns
 * { alerts, next }.
 */
export function judgeAddress({ label, address, views, index, prev = {}, now, expect = [], acknowledged = [], timelock = "<timelock>" }) {
  const alerts = [];
  const next = { judged: { ...(prev.judged ?? {}) }, oneSided: { ...(prev.oneSided ?? {}) }, expectUsed: { ...(prev.expectUsed ?? {}) },
    open: { ...(prev.open ?? {}) }, remembered: { ...(prev.remembered ?? {}) } };
  // review P-09: a protected buy's setup stays vouched after its frog ends Failed (the record then carries no Bought): remembered from
  // the poll that first saw it (the frog is Handed for hours before it can fail), for 30 days
  for (const [txid, v] of index.byTxid) if (v.remember) next.remembered[txid] = { what: v.what, at: next.remembered[txid]?.at ?? now };
  for (const [txid, v] of Object.entries(next.remembered)) if (now - v.at > 30 * 86_400_000) delete next.remembered[txid];
  const raise = (a, txid) => { alerts.push(a); next.open[txid] = a; };
  const up = views.filter((v) => v.txs), down = views.filter((v) => !v.txs).map((v) => v.name);
  if (!up.length) return { alerts, next };

  const byTx = new Map();
  for (const v of up) for (const tx of v.txs) {
    const e = byTx.get(tx.txid) ?? { tx, on: new Map() };
    e.on.set(v.name, tx);
    byTx.set(tx.txid, e);
  }
  const links = (txid) => views.map((v) => link(v.base, txid));
  const pause = `Guardian, now: dfx canister --network ic --identity <guardian> call ${timelock} pause_target (G10 step 1), then read the buyer's log() and redemptions().`;

  for (const [txid, { tx, on }] of byTx) {
    if (acknowledged.includes(txid)) { next.judged[txid] = "acknowledged"; delete next.oneSided[txid]; delete next.open[txid]; continue; }
    const absent = up.filter((v) => !on.has(v.name)).map((v) => v.name);
    const onAll = !down.length && !absent.length;

    // both confirmed, at different blocks: a reorg in progress or a provider lying
    const hashes = [...new Set([...on.values()].filter((t) => t.status?.confirmed).map((t) => t.status.block_hash))].sort();
    if (hashes.length > 1) {
      alerts.push({ kind: "provider-disagreement", sev: "SEV2", key: `reorg:${txid}:${hashes.join(":")}`,
        text: [`the Bitcoin providers disagree on where ${txid} (${label}) was confirmed:`,
          ...[...on.entries()].map(([n, t]) => `${n}: ${where(t)} ${t.status?.block_hash ?? ""}`),
          "Not guessing which is right. A reorg in progress, or a provider lying: check a third explorer.", ...links(txid)].join("\n") });
      continue;
    }
    if (["matched", "inbound", "acknowledged"].includes(next.judged[txid]) && onAll) continue;

    const dir = direction(tx, address);
    if (dir === "out") {
      const vouched = index.byTxid.get(txid)
        ?? (next.remembered[txid] && { kind: "purchase", what: `${next.remembered[txid].what}, remembered` })
        ?? (tx.vin ?? []).map((i) => index.saleOutpoints.get(`${i.txid}:${i.vout}`)).find(Boolean);
      if (vouched) {
        const to = tx.vout?.[0]?.scriptpubkey_address;
        if (vouched.kind === "delivery" && vouched.destination && to !== vouched.destination) {
          raise({ kind: "custody-outbound", sev: "SEV1", persistent: true, key: `outbound-dest:${txid}`,
            text: [`A DELIVERY FROM ${label.toUpperCase()} PAID THE WRONG ADDRESS: PAUSE NOW`, ...describeTx(tx, address, label),
              `the canister's record (${vouched.what}) names ${vouched.destination}; output 0 pays ${to}`, pause, ...links(txid)].join("\n") }, txid);
          next.judged[txid] = "UNEXPECTED";
          continue;
        }
        if (onAll) { next.judged[txid] = "matched"; delete next.oneSided[txid]; continue; }
        oneSided(txid, `an outbound spend the canister recorded (${vouched.what})`);
        continue;
      }
      // nothing vouches for it
      next.judged[txid] = "UNEXPECTED";
      if (absent.length) {
        raise({ kind: "provider-disagreement", sev: "SEV1", persistent: true, key: `disagree-outbound:${txid}`,
          text: [`${[...on.keys()].join(", ")} report${on.size === 1 ? "s" : ""} an UNEXPECTED spend from ${label} (${address}); ${absent.join(", ")} do${absent.length === 1 ? "es" : ""} not show it.`,
            "Not guessing which is right: check both explorers yourself, now. If the spend is real, custody is being drained.",
            ...describeTx(tx, address, label), "no canister record names this txid", pause, ...links(txid)].join("\n") }, txid);
      } else {
        raise({ kind: "custody-outbound", sev: "SEV1", persistent: true, key: `outbound:${txid}`,
          text: [`UNEXPECTED OUTBOUND TRANSACTION FROM ${label.toUpperCase()} (${address}): PAUSE NOW`, ...describeTx(tx, address, label),
            "no canister record names this txid (redemptions, holder_deposits, frog_buys, sale_state and purchases, read after the explorers)",
            `seen by ${[...on.keys()].join(" and ")}${down.length ? `; ${down.join(", ")} did not answer (one provider only)` : ""}`,
            pause, ...links(txid),
            "When explained, add the txid to the monitor's acknowledged list to stop the 6-hourly repeat."].join("\n") }, txid);
      }
      continue;
    }
    if (dir === "in") {
      const known = index.inbound.get(txid);
      if (!known) {
        const got = (tx.vout ?? []).filter((o) => o.scriptpubkey_address === address).reduce((s, o) => s + (o.value ?? 0), 0);
        const used = next.expectUsed[txid];
        const e = used ? expect.find((x) => x.id === used) : expect.find((x) => !Object.values(next.expectUsed).includes(x.id) && got >= (x.min_sats ?? 0));
        if (e) {
          next.expectUsed[txid] = e.id;
          alerts.push({ kind: "custody-inbound", sev: "INFO", key: `inbound-expected:${txid}`,
            text: [`✅ EXPECTED INBOUND seen at ${label} (${address}): ${e.note ?? e.id}`, `${sats(got)} in ${txid}, ${where(tx)}`,
              "The canary sees this address. (Nothing left custody.)", ...links(txid)].join("\n") });
        } else {
          alerts.push({ kind: "custody-inbound", sev: "INFO", key: `inbound:${txid}`,
            text: [`unexplained INBOUND to ${label} (${address}): ${sats(got)} in ${txid}, ${where(tx)}`,
              "Nothing left custody. No canister record names it (not a frog purchase or a holder deposit): somebody sent something here.",
              ...links(txid)].join("\n") });
        }
      }
      if (onAll) { next.judged[txid] = "inbound"; delete next.oneSided[txid]; continue; }
      oneSided(txid, known ? known.what : "an inbound transaction");
    }
  }

  // one provider shows it and the other, answering, does not: wait out propagation, then say so; never pick a side
  function oneSided(txid, what) {
    const absent = up.filter((v) => !byTx.get(txid).on.has(v.name)).map((v) => v.name);
    if (!absent.length) return;   // the other provider is down, not disagreeing
    const first = (next.oneSided[txid] ??= now);
    const only = [...byTx.get(txid).on.keys()].join(", ");
    if (now - first >= GRACE_MS) alerts.push({ kind: "provider-disagreement", sev: "SEV2", key: `disagree:${txid}`,
      text: [`the Bitcoin providers disagree about ${label} (${address}): ${only} show ${txid} (${what}); ${absent.join(", ")} do not, after ${Math.round((now - first) / 60000)} min.`,
        "Not guessing which is right: check both explorers.", ...links(txid)].join("\n") });
    if (now - first >= STILL_MS) alerts.push({ kind: "provider-disagreement", sev: "SEV2", key: `disagree-1h:${txid}`,
      text: `STILL disagreeing after an hour about ${txid} at ${label} (${address}): ${only} show it, ${absent.join(", ")} do not. G9: persistent disagreement.` });
  }

  // an unexplained spend stays open, and keeps repeating, until it is acknowledged: even once it scrolls off the first page
  for (const [t, a] of Object.entries(next.open)) {
    if (acknowledged.includes(t)) delete next.open[t];
    else if (!byTx.has(t)) alerts.push(a);
  }
  // a one-sided tx that has since vanished from everyone (evicted from a mempool) is forgotten
  for (const t of Object.keys(next.oneSided)) if (!byTx.has(t)) delete next.oneSided[t];
  return { alerts, next };
}

/** One address's recent history from one esplora provider: the first page, then older confirmed pages until a txid already
 *  judged appears (or the whole history on the first run), at most `maxPages`. */
export async function fetchAddressTxs(base, address, known, http = fetch, maxPages = 40) {
  const get = async (path) => {
    const r = await http(`${base}${path}`, { signal: AbortSignal.timeout(20_000) });
    if (!r.ok) throw new Error(`${host(base)} answered HTTP ${r.status}`);
    const j = await r.json();
    if (!Array.isArray(j)) throw new Error(`${host(base)}: not a transaction list`);
    return j;
  };
  const out = await get(`/address/${address}/txs`);
  let chain = out.filter((t) => t.status?.confirmed), pages = 1;
  while (chain.length >= 25 && !chain.some((t) => known.has(t.txid))) {
    if (pages >= maxPages) return { txs: out, truncated: true };
    const more = await get(`/address/${address}/txs/chain/${chain[chain.length - 1].txid}`);
    if (!more.length) break;
    out.push(...more);
    chain = more;
    pages++;
  }
  return { txs: out, truncated: false };
}

/**
 * The canary's poll. Returns { alerts, next } and changes nothing: the caller commits `next` once the alerts are delivered.
 * cfg: { providers: [base, base], addresses: { label: address }, expect_inbound: [{ address: label, min_sats, note }],
 *        acknowledged: [txid] }.
 */
export async function checkBitcoin({ cfg, prev = {}, now, http = fetch, readRecords, timelock }) {
  const alerts = [];
  const views = {};
  for (const [label, address] of Object.entries(cfg.addresses)) {
    const known = new Set(Object.keys(prev.addresses?.[address]?.judged ?? {}));
    views[label] = await Promise.all(cfg.providers.map(async (base) => {
      try {
        const { txs, truncated } = await fetchAddressTxs(base, address, known, http);
        if (truncated) alerts.push({ kind: "watcher-error", sev: "SEV2", key: `truncated:${address}:${host(base)}`,
          text: `${host(base)}: the history of ${label} (${address}) is longer than the canary pages through; older spends were not judged.` });
        return { name: host(base), base, txs };
      } catch (err) {
        return { name: host(base), base, txs: null, error: String(err.message) };
      }
    }));
  }
  const providers = {};
  for (const base of cfg.providers) {
    const name = host(base), p = { ...(prev.providers?.[name] ?? {}) };
    const answered = Object.values(views).every((vs) => vs.find((v) => v.base === base)?.txs);
    if (answered) { p.lastOk = now; p.downSince = null; p.error = null; }
    else {
      p.downSince ??= now;
      p.error = Object.values(views).map((vs) => vs.find((v) => v.base === base)?.error).find(Boolean) ?? "no answer";
      if (now - p.downSince >= DOWN_MS) alerts.push({ kind: "watcher-error", sev: "SEV2", persistent: true, key: `provider-down:${name}:${p.downSince}`,
        text: `the Bitcoin provider ${name} has not answered for ${Math.round((now - p.downSince) / 60000)} min (${p.error}). The canary is on one provider: it still alerts on any unexplained spend, but cannot check agreement.` });
    }
    providers[name] = p;
  }
  if (Object.values(views).every((vs) => vs.every((v) => !v.txs))) throw new Error("no Bitcoin provider answered");

  const index = recordIndex(await readRecords());          // AFTER the explorers
  const addresses = { ...(prev.addresses ?? {}) };
  for (const [label, address] of Object.entries(cfg.addresses)) {
    const expect = (cfg.expect_inbound ?? []).map((e, i) => ({ ...e, id: `${e.address}#${i}` })).filter((e) => e.address === label);
    const r = judgeAddress({ label, address, views: views[label], index, prev: addresses[address], now, expect,
      acknowledged: cfg.acknowledged ?? [], timelock });
    alerts.push(...r.alerts);
    addresses[address] = r.next;
  }
  return { alerts, next: { addresses, providers } };
}
