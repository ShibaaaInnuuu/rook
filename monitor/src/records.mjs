// What the canister's own records vouch for (docs/v6/k6-watcher-canary.md §4, §5). Pure. The input is the plain replies of
// redemptions(), holder_deposits(), frog_buys(), sale_state() and purchases(); txids are display order, as explorers show them.

const opt = (x) => (Array.isArray(x) ? (x.length ? x[0] : null) : x ?? null);
const phaseOf = (p) => Object.entries(p ?? {})[0] ?? [null, null];

export const EMPTY_RECORDS = { redemptions: [], holder_deposits: [], frog_buys: [], sale_state: { sales: [] }, purchases: [], conversions: [] };

/**
 * byTxid: every txid the canister signed and saved (an outbound it vouches for). inbound: txids that land at custody for a
 * reason the canister knows. saleOutpoints: custody outpoints a signed listing lets anyone spend. mintOutpoints: the Bitcoin
 * outpoints a `Deposited` mint may name. deliveries: ticket → recorded txid. floorFrogs: frogs a FloorModule `ItemArrived` may
 * report. attests: ticket → the canister's markUndeliverable (canister review2 N-2): { state: Due | Sent | Done, hash, note }, and
 * whether the redemption was refused (the only redemptions the canister ever attests). advances: the Relay conversions
 * (docs/v6/relay-conversion.md §6), { frog, module, advanced, phase }, what a FloorModule's Advanced / AdvanceSettled /
 * AdvanceReturned may report; a conversion's arrival at the float is a known inbound.
 */
export function recordIndex(r) {
  const byTxid = new Map(), inbound = new Map(), saleOutpoints = new Map(), mintOutpoints = new Map(), deliveries = new Map(), attests = new Map();
  // canister review2 M-1 (b): a conflict-out re-priced every 2 h keeps each earlier version's txid; all of them pay custody
  const conflictTxids = (c) => { const t = opt(c?.tx); return t ? [t.txid, ...(opt(t.earlier) ?? [])].filter(Boolean) : []; };
  const floorFrogs = [], advances = [];
  // audit 2026-09-27-nft D-M-2: the Ethereum NFT side's mints (collection:token → [{ kind, tx, to }]) and deliveries (wrapper:ticket)
  const ethMints = new Map(), ethDeliveries = new Map();
  for (const x of r.redemptions ?? []) {
    const [ph, body] = phaseOf(x.phase);
    // audit D, M-1: every version of a delivery the canister signed (the current one and each re-priced earlier one) pays the
    // destination; H8's conflict-out spends the fee input back to custody (it never pays the destination)
    const versions = [body?.txid, ...(opt(body?.bump)?.earlier ?? []), ...(opt(body?.conflict?.earlier) ?? [])].filter(Boolean);
    if (body?.conflict?.leaked_txid) versions.push(body.conflict.leaked_txid);
    for (const t of new Set(versions))
      byTxid.set(t, { kind: "delivery", what: `redemption of ticket ${x.ticket} (${ph}${t !== body?.txid ? ", an earlier version" : ""})`, destination: x.destination });
    for (const cx of conflictTxids(body?.conflict))
      byTxid.set(cx, { kind: "conflict", what: `the conflict-out of ticket ${x.ticket}'s delivery (${body.reason ?? ph}${cx !== opt(body.conflict.tx).txid ? ", an earlier version" : ""}): its fee input back to custody` });
    const [as, ab] = phaseOf(opt(x.attest));
    if (as) attests.set(String(x.ticket), { state: as, hash: ab?.hash ?? null, note: ab?.note ?? null, refused: ph === "Refused" });
    if (versions.length) deliveries.set(String(x.ticket), { txid: body?.txid ?? versions[0], txids: [...new Set(versions)], phase: ph });
  }
  for (const d of r.holder_deposits ?? []) {
    const [ph, body] = phaseOf(d.phase);
    inbound.set(d.outpoint.txid, { what: `holder deposit ${d.outpoint.txid}:${d.outpoint.vout} (${ph})` });
    if (body?.txid) byTxid.set(body.txid, { kind: "return", what: `return of refused deposit ${d.outpoint.txid}:${d.outpoint.vout} (${ph})` });
    const c = body?.conflict;
    if (c) {
      for (const t of [c.leaked_txid, ...(opt(c.earlier) ?? [])].filter(Boolean)) byTxid.set(t, { kind: "return", what: `return of refused deposit ${d.outpoint.txid}:${d.outpoint.vout} (${ph})` });
      for (const cx of conflictTxids(c)) byTxid.set(cx, { kind: "conflict", what: `the conflict-out of deposit ${d.outpoint.txid}:${d.outpoint.vout}'s return: its fee input back to custody` });
    }
    if (ph === "Ready" || ph === "Minted") mintOutpoints.set(`${d.outpoint.txid}:${d.outpoint.vout}`, { what: `holder deposit (${ph})`, inscription: body.inscription });
  }
  for (const f of r.frog_buys ?? []) {
    const [ph, body] = phaseOf(f.phase);
    const buy = body?.buy;
    if (!buy) continue;
    const t = buy.outpoint.txid;
    byTxid.set(t, { kind: "purchase", what: `frog purchase #${f.id} (${ph})` });
    // docs/v8/ow-protected.md §3.3: a protected buy's setup splits the float's own coins for the sale; the canister signed it
    const setup = opt(buy.setup_txid);
    if (setup) byTxid.set(String(setup), { kind: "purchase", remember: true, what: `the setup of frog purchase #${f.id} (${ph}): the float's coins split for an Ordinals Wallet protected sale` });
    // audit D, L-5: a stale buy abandoned: one of its float inputs spent back to the float
    const c = body?.conflict;
    if (c) {
      if (c.leaked_txid) byTxid.set(c.leaked_txid, { kind: "purchase", what: `frog purchase #${f.id} (${ph})` });
      for (const cx of conflictTxids(c)) byTxid.set(cx, { kind: "conflict", what: `the float's abandon of frog purchase #${f.id}: a float input spent back to the float` });
    }
    inbound.set(t, { what: `frog purchase #${f.id} landing at custody (${ph})` });
    if (["Held", "Minted", "Settled"].includes(ph)) mintOutpoints.set(`${t}:${buy.outpoint.vout}`, { what: `frog purchase #${f.id} (${ph})`, inscription: buy.inscription });
    const sink = opt(f.target)?.Sink;
    if (sink && ["Minted", "Settled"].includes(ph)) floorFrogs.push({ id: String(f.id), module: `0x${sink.toLowerCase()}`, cost: String(buy.cost_wei) });
  }
  for (const s of r.sale_state?.sales ?? []) {
    const [ph, body] = phaseOf(s.phase);
    if (body?.txid) byTxid.set(body.txid, { kind: "sale", what: `sale #${s.id} moving its item (${ph})` });
    if (body?.fill_txid) byTxid.set(body.fill_txid, { kind: "sale", what: `sale #${s.id} filled` });
    if (["Listed", "Recorded", "Delisting"].includes(ph)) saleOutpoints.set(`${body.outpoint.txid}:${body.outpoint.vout}`, { what: `sale #${s.id}'s signed listing (${ph})` });
    if (ph === "Redeposit") mintOutpoints.set(`${body.outpoint.txid}:${body.outpoint.vout}`, { what: `sale #${s.id} redeposit`, inscription: body.inscription });
  }
  for (const c of r.conversions ?? []) {
    const [ph] = phaseOf(c.phase);
    advances.push({ frog: String(c.frog), module: `0x${String(c.module).toLowerCase()}`, advanced: String(c.advanced), phase: ph });
    const a = opt(c.arrival);
    if (a) inbound.set(a.outpoint.txid, { what: `the Relay conversion for frog purchase #${c.frog} arriving at the float (${ph})` });
  }
  // docs/v7/phase1-nft.md §6: an Ethereum buy's advance, settlement and return, in the conversions' phase words
  for (const b of r.eth?.buys ?? []) {
    const [ph] = phaseOf(b.phase);
    if (ph === "NeedAdvance") continue;
    if (opt(b.sweep)) continue;   // A2-L-1: a gas-pot sweep advances nothing; its fundReserve is not an advance event
    const phase = ph === "Settling" || ph === "Settled" ? "Settled" : ["Returning", "Returned", "Stuck", "Advancing", "Funded", "Depositing"].includes(ph) ? ph : "Converting";
    advances.push({ frog: `eth:${b.id}`, module: `0x${String(b.module).toLowerCase()}`, advanced: String(b.advanced), phase });
  }
  // raw bytes, or the hex text plain() makes of a blob (ic.mjs reads eth_nft_state() through plain(): blobs are hex, no 0x)
  const hexb = (b) => (typeof b === "string" ? `0x${b.replace(/^0x/i, "").toLowerCase()}` : `0x${Buffer.from(b ?? []).toString("hex")}`);
  const dec = (b) => BigInt(hexb(b) === "0x" ? 0 : hexb(b)).toString();
  const mint = (key, m) => ethMints.set(key, [...(ethMints.get(key) ?? []), m]);
  for (const b of r.eth?.buys ?? []) {
    const [ph, body] = phaseOf(b.phase);
    if (["Minting", "Minted", "Settling", "Settled"].includes(ph) && body?.token && body?.tx)
      mint(`${hexb(b.collection)}:${dec(body.token)}`, { kind: "buy", tx: hexb(body.tx), to: hexb(b.module), what: `Ethereum buy #${b.id} (${ph})` });
  }
  for (const x of r.eth?.deposits ?? []) {
    const [ph] = phaseOf(x.phase);
    if (["Minting", "Minted"].includes(ph))
      mint(`${hexb(x.collection)}:${dec(x.token)}`, { kind: "deposit", tx: hexb(x.tx), to: hexb(x.recipient), what: `Ethereum deposit ${hexb(x.tx)} (${ph})` });
  }
  for (const d of r.eth?.deliveries ?? []) {
    const [ph, body] = phaseOf(d.phase);
    ethDeliveries.set(`${hexb(d.wrapper)}:${String(d.ticket)}`, { phase: ph, tx: body?.tx ? hexb(body.tx) : null });
  }
  for (const [id, text] of r.purchases ?? []) {
    for (const t of String(text).toLowerCase().match(/[0-9a-f]{64}/g) ?? []) byTxid.set(t, { kind: "vault purchase", what: `vault purchase #${id}` });
  }
  return { byTxid, inbound, saleOutpoints, mintOutpoints, deliveries, floorFrogs, attests, advances, ethMints, ethDeliveries };
}
