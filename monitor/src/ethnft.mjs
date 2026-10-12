// The Ethereum NFT side (docs/v7/phase1-nft.md §6): the buys and deliveries the canister reports, the NFTs its Ethereum address must
// hold (read on two Ethereum providers, never the canister's word), and the floor it posts against two public references. Each state
// that lasts too long, and each that should never happen, pages. `ethNftAlerts` and `floorAlerts` are pure; `checkEthNft` reads.
import { decodeFunctionResult, encodeFunctionData, parseAbi } from "viem";

export const BRIDGING_LATE_S = 30 * 60;          // Relay's estimate is ~4 s to Ethereum: half an hour is late
export const FUNDED_LATE_S = 2 * 3600;           // the runner is not quoting
export const ARRIVED_LATE_S = 24 * 3600;         // ETH on Ethereum and no NFT bought (docs §9 D4)
export const BOUGHT_LATE_S = 2 * 3600;
/** Review round 5 (F5-M-1): a home transaction seen nowhere with its nonce used this long pages (the canister's HOME_NONCE_TAKEN_SECS). */
export const HOME_UNSEEN_S = 3600;
/** Review round 6 (D6-L-2): a home transaction out this long since it was signed pages SEV2, and SEV1 at HOME_OUT_SEV1_S, whatever the providers show. */
export const HOME_OUT_S = 30 * 60;
export const HOME_OUT_SEV1_S = 2 * 3600;
/** Review round 6 (D6-L-3): a written-off order holding the Relay leg pages SEV2 (persistent), SEV1 once it has held it this long. */
export const LATE_HOLD_SEV1_S = 72 * 3600;
/** Review round 6 (B6-M-1 / D6-I-1), round 7 (B7-L-3): the canister's sale scan this many final blocks behind the head pages SEV2 (8 h),
 * and SCAN_BEHIND_SEV1_BLOCKS (a day) SEV1 (the canister's SALE_LAG_SEV2_BLOCKS / SALE_LAG_SEV1_BLOCKS). The lag is the final head (the
 * monitor's own Ethereum head less SALE_FINAL_DEPTH_BLOCKS) less the head the scan was last current at (`SaleScan.current`), so it grows
 * while passes do not run, and a re-anchor cannot hide it. */
export const SCAN_BEHIND_BLOCKS = 2_400;
export const SCAN_BEHIND_SEV1_BLOCKS = 7_200;
export const SALE_FINAL_DEPTH_BLOCKS = 128;
/** Review round 7 (B7-L-1 / B7-L-2): a hole still unread after a day pages SEV2; a receipt answered as a rate limit this many times in a row
 * (the canister's SALE_BUSY_PAGE_TRIES), and a scan that met a rate limit in this many passes in a row (SALE_BUSY_PAGE_PASSES), page SEV2. */
export const SCAN_HOLE_DAY_S = 86_400;
export const SCAN_BUSY_TRIES = 6;
export const SCAN_BUSY_PASSES = 9;
/** Review round 8 (B8-L-1): the sale window (the canister's SALE_WINDOW_BLOCKS, 7 days). The walk back to its start (`SaleScan.down`) is done
 * once `down` is at the window's start; `down` still SCAN_WINDOW_GAP_BLOCKS (2 h) or more above the start, and not moved for
 * SCAN_WINDOW_UNREAD_S (12 h, 36 of the runner's 20-minute passes), pages SEV2 (`eth-scan-window-unread`): ONE provider of the pair refusing
 * the older blocks as too deep is not taken as "too deep" by the canister (B7-L-1, never on one provider's word), so no `lost` or `hole_deep`
 * is ever set for it; this page is its watch. Not a day: a stuck `down` leaves the window as the window moves (7,200 blocks a day), so a
 * provider keeping 43,000 blocks would clear before a day-long timer ever fired. The gap (2 h) is above the canister's own lag behind the
 * monitor's head (a pin a bucket back, a pass every 20 minutes: a few hundred blocks). */
export const SALE_WINDOW_BLOCKS = 50_400;
export const SCAN_WINDOW_GAP_BLOCKS = 600;
export const SCAN_WINDOW_UNREAD_S = 12 * 3600;
export const BUYING_LATE_S = 3600;               // a fill signed an hour ago and not mined           // bought and not final in two hours: Ethereum finality is ~15 min
export const RESOLVED_LATE_S = 3600;             // a resolved ticket not delivered in an hour
export const DEADLINE_NEAR_S = 86_400;           // within a day of deliverBy and not recorded
export const FLOOR_WARN_BPS = 1_000;             // the posted floor more than 10 % from both references
export const FLOOR_PAUSE_BPS = 2_000;            // more than 20 %: SEV1, and the guardian pauses the keeper (zeroing the poster)
export const GAS_POT_LOW_WEI = 5_000_000_000_000_000n;
// holder deposits (D5, on at launch): a claim is final in ~15-30 min and minted within the day's limits; a stray goes back once final
export const DEPOSIT_SEEN_LATE_S = 2 * 3600;
export const DEPOSIT_MINT_LATE_S = 26 * 3600;    // the mint limits are per day: a deposit may wait one
export const RETURN_LATE_S = 2 * 3600;
export const UNCLAIMED_LATE_BLOCKS = 600n;       // an inbound NFT the canister has not judged 2 h after it arrived
export const DEPOSIT_SCAN_BLOCKS = 7_200n;
/** Review round 4 (D4-L-2): the blocks under the last read head that every poll reads again. */
export const RESCAN_OVERLAP_BLOCKS = 64n;
/** The widest eth_getLogs range asked (the operator's Dwellir plan, 2026-09-27: 500 blocks a request); `eth_nft.logs_max_blocks`. */
export const LOGS_MAX_BLOCKS = 500n;

/**
 * The inbound-transfer logs of `collection` to `to` from `from` to `head`, in ranges of at most `max` blocks, oldest first. Throws on the
 * first range that fails (the caller keeps its cursor, so nothing is skipped).
 */
export async function logsInRanges(get, from, head, max) {
  const out = [];
  for (let a = from; a <= head; a += max) {
    const b = a + max - 1n < head ? a + max - 1n : head;
    out.push(...((await get(a, b)) ?? []));
  }
  return out;
}
// pooled graduations (docs/v7/phase1-nft-pooled.md §3): a pool that holds a whole item plus the buffer and has no open buy for a day
export const POOL_IDLE_S = 24 * 3600;
const ONE = 10n ** 18n;
const DAY = 86_400, DELIVERY_DEADLINE_S = 7 * DAY;
const phaseOf = (p) => Object.entries(p ?? {})[0] ?? [null, null];
// a blob as 0x-hex: raw bytes (IDL.decode), or the hex text `plain()` makes of them (ic.mjs's ethState: blobs as hex, no 0x). Before
// round 3 a hex text was read as UTF-8 here, so no blob of a live eth_nft_state() compared equal to anything
const hexOf = (b) => (typeof b === "string" ? `0x${b.replace(/^0x/i, "").toLowerCase()}` : `0x${Buffer.from(b ?? []).toString("hex")}`);
/** `opt bool` as plain JSON ([] / [true]), as the IDL decodes it, or a bare boolean: true only for true. */
const optTrue = (x) => (Array.isArray(x) ? x[0] === true : x === true);
/** A delivery's phases once its transfer MINED on Ethereum (the canister records it at finality, from the receipt). */
export const DELIVERY_MINED = ["Moved", "Record", "Recording", "Recorded"];
const eth = (wei) => `${(Number(BigInt(wei) / 10n ** 12n) / 1e6).toString()} ETH`;
const mins = (s) => `${Math.round(s / 60)} min`;

/** A step on the Solana line, in words: B1's (an Ethereum step) or phase B2's (`{ Eth | Frog | Custody: step }`). */
export function lineStepText(step) {
  const [owner, inner] = Object.entries(step ?? {})[0] ?? [null, null];
  if (owner === "Op") {
    // W3: a step of the mint/burn desk or the pool desk
    const who = Object.keys(inner?.owner ?? {})[0] ?? "?";
    return `the ${who === "Pool" ? "pool desk" : who === "Desk" ? "mint/burn desk" : who}'s ${inner?.what ?? "step"} #${inner?.id ?? "?"}`;
  }
  if (owner === "Eth" || owner === "Frog" || owner === "Custody") {
    const [name, body] = Object.entries(inner ?? {})[0] ?? [null, null];
    const who = owner === "Eth" ? "the Ethereum side's" : owner === "Frog" ? "the frog side's" : "the custodian's";
    const of = body?.buy !== undefined ? ` of buy #${body.buy}` : body?.frog !== undefined ? ` of frog #${body.frog}` : body?.ticket !== undefined ? ` of ticket #${body.ticket}` : "";
    return `${who} ${name}${of}`;
  }
  return owner ? `${owner}${inner?.buy !== undefined ? ` of buy #${inner.buy}` : ""}` : "nothing";
}

/**
 * Pure. The Solana home's ONE line (docs/v9/solana-migration.md §2.4): `line` is `sol_line()` (phase B2, canister-wide) or B1's
 * `EthState.sol`, as plain JSON. An incident stops every home step of every flow until the operator, having looked, clears it
 * (sol_clear_incident); a simulation every provider refuses (the canister's Solana address cannot pay the fee) holds the line until it is
 * funded; a step out too long holds every other home step.
 */
export function solLineAlerts({ buyer, line, nowS, prefix = "sol-line", name = null }) {
  const alerts = [];
  if (!line) return alerts;
  // W3: one of the four lines (money, crank, desk, pool): its own name, and its own clear
  const lineName = name ? `Solana ${name} line` : "Solana home line";
  const clear = name ? `sol_clear_line_incident(variant { ${name[0].toUpperCase()}${name.slice(1)} })` : "sol_clear_incident";
  const o = (x) => (Array.isArray(x) ? x[0] : x);
  const incident = o(line.line?.incident), error = o(line.line?.error), saved = o(line.line?.saved), sp = o(line.pending);
  const stepText = sp ? lineStepText(sp.step) : null;
  const what = saved ? `${saved.what} (intent ${saved.intent}, ${saved.signature})` : stepText ?? "nothing";
  if (incident) alerts.push({ kind: "canister", sev: "SEV1", persistent: true, key: `${prefix}-incident:${buyer}:${saved?.intent ?? ""}`,
    text: `the buyer ${buyer}'s ${lineName} has an INCIDENT: ${incident}. Nothing more is signed on ${name ? "this line (the other lines go on)" : "Solana (posts, records, mints, advances, of every flow)"} until the operator, having looked (a key used elsewhere? a bug?), clears it: ${clear}. The pending ${what} never landed and is taken back, then signed again.` });
  else if (error) alerts.push({ kind: "canister", sev: /insufficient_funds/.test(error) ? "SEV1" : "SEV2", persistent: true, key: `${prefix}-preflight:${buyer}:${saved?.intent ?? ""}`,
    text: `the buyer ${buyer}'s ${lineName}: ${error}. ${/insufficient_funds/.test(error) ? "Fund the canister's Solana address (sol_address): every home step waits on this one." : "Its instruction fails in simulation: the canister gives the step up (audit A4-01: by burning its durable nonce, or waiting for its blockhash to expire) and reports it unlanded only once that is proven on chain; if the step lands meanwhile, that landing is its outcome."}` });
  const at = sp ? Number(sp.at ?? 0) : 0;
  if (sp && at > 0 && nowS - at >= HOME_OUT_S) alerts.push({ kind: "canister", sev: nowS - at >= HOME_OUT_SEV1_S ? "SEV1" : "SEV2", persistent: true,
    key: `${prefix}-out:${buyer}:${sp.intent}`,
    text: `the buyer ${buyer}'s ${name ? `Solana ${name}-line` : "Solana home"} step ${what} was signed ${mins(nowS - at)} ago and has not reached finalized: nothing else is signed on ${name ? "that line" : "Solana"} meanwhile${name ? "" : ", by any flow"}. Look at the transaction and at the canister's Solana providers.` });
  // phase B2: a flow waiting its turn for long is a line held (the head of the queue blocks the others for TURN_SECS at most)
  for (const [owner, since] of line.queue ?? []) {
    const who = Object.keys(owner ?? {})[0] ?? "a flow";
    if (nowS - Number(since) >= HOME_OUT_S) alerts.push({ kind: "canister", sev: "SEV2", persistent: true, key: `${prefix}-queue:${buyer}:${who}:${since}`,
      text: `the buyer ${buyer}'s ${who} side has waited ${mins(nowS - Number(since))} for its turn on the Solana line (${what} out): look at what holds the line.` });
  }
  return alerts;
}

/** W14b: the canister's own Solana timer silent this long (it runs every 10 s busy, 60 s idle) pages SEV2. */
export const SOL_TIMER_SILENT_S = 10 * 60;

/**
 * Pure. W3 (docs/v9/canister-only-solana.md §13.1): the `sol_lines()` reply `[lines, timer, off]` as plain JSON. Each of the four lines
 * (money, crank, desk, pool: each its own durable nonce, none waiting for another) is paged on its own, keyed by its name (the money line
 * keeps phase B2's keys, `sol-line-*`; the others `sol-line-<name>-*`). The canister's own timer pages when it has not run for
 * SOL_TIMER_SILENT_S, unless the operator switched it off.
 */
export function solLinesAlerts({ buyer, reply, nowS }) {
  const alerts = [];
  if (!reply) return alerts;
  const [lines, timerOpt, off] = reply;
  const o = (x) => (Array.isArray(x) ? x[0] : x);
  for (const [id, lineOpt] of lines ?? []) {
    const tag = Object.keys(id ?? {})[0] ?? "?";
    const line = o(lineOpt);
    if (!line) continue;
    const money = tag === "Money";
    alerts.push(...solLineAlerts({ buyer, line, nowS, prefix: money ? "sol-line" : `sol-line-${tag.toLowerCase()}`, name: money ? null : tag.toLowerCase() }));
  }
  const timer = o(timerOpt);
  const last = Number(timer?.last_run ?? 0);
  if (!off && last > 0 && nowS - last >= SOL_TIMER_SILENT_S) alerts.push({ kind: "canister", sev: "SEV2", persistent: true, key: `sol-timer-silent:${buyer}`,
    text: `the buyer ${buyer}'s own Solana timer last ran ${mins(nowS - last)} ago (it runs every 10 s with work out, 60 s idle): the four send lines move only when the runner calls tick(). Is the canister stopped, out of cycles, or did an upgrade fail to arm it (sol_timer(true) re-arms it)?` });
  return alerts;
}

/** Pure. `state`: eth_nft_state() as plain JSON. `prev`: { firstSeen: { key: seconds } }. Returns { alerts, next }. */
/** Audit C3-02 / C3-05: a chain's block time (ms), as the canister's `chains::block_ms`; the scan's block counts above are Ethereum's. */
export const BLOCK_MS = { 1: 12_000, 4663: 104, 46630: 104, 8453: 2_000 };
/** Audit C3-05: the sale scan's page thresholds in `chain`'s own blocks (the canister's `ethnft::sale_blocks`): the same spans in time. */
export function scanBlocksOf(chain) {
  const ms = BLOCK_MS[Number(chain ?? 1)] ?? 12_000, b = (ethBlocks) => Math.floor((ethBlocks * 12_000) / ms);
  return { behind: b(SCAN_BEHIND_BLOCKS), behindSev1: b(SCAN_BEHIND_SEV1_BLOCKS), finalDepth: b(SALE_FINAL_DEPTH_BLOCKS), window: b(SALE_WINDOW_BLOCKS),
    gap: b(SCAN_WINDOW_GAP_BLOCKS), perHour: b(300) };
}

/** `chain`: the side's chain when it is another than the config's own (audit C3-05: Robinhood Chain's side, read with
 * `eth_nft_state_of(chain)`): its pages are keyed and worded by it, and its scan's thresholds are in its own blocks. */
export function ethNftAlerts({ buyer: buyer0, state, nowS, prev = {}, ethHead = null, collections = null, chain = null }) {
  const buyer = chain === null || chain === undefined ? buyer0 : `${buyer0}@chain${chain}`;
  const sb = scanBlocksOf(chain);
  const alerts = [], next = { firstSeen: {} };
  const seen = (key) => (next.firstSeen[key] = prev.firstSeen?.[key] ?? nowS);
  for (const b of state?.buys ?? []) {
    const [ph, body] = phaseOf(b.phase);
    const id = String(b.id), who = `the buyer ${buyer}'s Ethereum buy #${id} for ${hexOf(b.collection)} (advance ${eth(b.advanced)}, attempt ${b.attempts})`;
    const note = b.note ? ` Its last note: ${String(b.note).slice(0, 300)}` : "";
    // round 3, D3-I-1: a `since` of 0 is "just asked" (a sweep is created BackFunded { since: 0 } until the next tick): first seen now
    const since = body?.since !== undefined && body?.since !== null && Number(body.since) > 0 ? Number(body.since) : seen(`${id}:${ph}`);
    if (ph === "Bridging" && nowS - since >= BRIDGING_LATE_S) alerts.push({ kind: "canister", sev: "SEV2", persistent: true, key: `eth-bridge-late:${buyer}:${id}:${since}`,
      text: `${who} is LATE: its Relay deposit (4663 → Ethereum) mined ${mins(nowS - since)} ago and at least ${eth(body.min_out)} has not reached the canister's Ethereum address, and no refund is seen. Order ${hexOf(body.order_id)}; past its deadline + 1 h the canister calls it Stuck.${note}` });
    else if (ph === "Stuck") {
      // review round 4 (C4-M-1, D4-I-3): a forward order past its deadline is still WATCHED (it holds the Relay leg; a late fill or
      // refund is read as its own); one the operator wrote off is not, and its payout, if it comes after all, is credited by transaction
      // review round 5 (D5-L-2): three cases, told apart by `last` (the watch) and `late` (the written-off order): a fill-Stuck buy (the
      // NFT we paid for is not ours) has neither, and is no Relay matter at all
      const late = Array.isArray(b.late) ? b.late[0] : b.late;
      const watched = Array.isArray(b.last) ? b.last.length > 0 : !!b.last;
      // review round 6 (D6-I-2): held only while neither credited nor released (the canister's late_hold)
      const held = late && !(Array.isArray(late.released_at) ? late.released_at.length : late.released_at)
        && !(Array.isArray(late.credited) ? late.credited.length : late.credited);
      const close = optTrue(b.sweep) ? `eth_give_back(${id}) closes the sweep with nothing (its leftover written off)` : `eth_give_back(${id}) closes it as a loss (returnAdvance of 0, the module books the loss)`;
      alerts.push({ kind: "canister", sev: "SEV1", persistent: true, key: `eth-stuck:${buyer}:${id}`,
        text: late
          ? `${who} is STUCK: ${body.why}. Its order was WRITTEN OFF (no longer watched)${held ? "; until a payout of it is credited or the operator releases it (eth_late_release), it still holds the Relay leg for new orders (review round 5, C5-M-1: its payout could otherwise be read as a new order's)" : ""}: ${close}. If Relay pays it after all, credit the payout to it with eth_credit_payout(${id}, <chain id>, <the payout's tx hash>, <wei, only if the tx does not show it>) (docs/v7/phase1-nft.md §7).${note}`
          : watched
            ? `${who} is STUCK: ${body.why}. Its order is still WATCHED (it holds the Relay leg for every collection): a late fill on Ethereum or refund on 4663 is read as its own. Once Relay confirms nothing more will come (and not before its deadline + 1 h + 24 h), eth_give_back(${id}) writes it off; a second eth_give_back(${id}) closes it (docs/v7/phase1-nft.md §7).${note}`
            : `${who} is STUCK and NOT watched: ${body.why}. No Relay order is open for it: its fill mined but the NFT we paid for is not at our Ethereum address. Pause, then look (docs/v7/phase1-nft.md §7); once understood, ${close}.${note}` });
    }
    else if (ph === "Funded" && nowS - since >= FUNDED_LATE_S) alerts.push({ kind: "canister", sev: "SEV2", persistent: true, key: `eth-funded:${buyer}:${id}:${since}`,
      text: `${who} has waited ${mins(nowS - since)} for a Relay quote with ${eth(body.wei)} at the canister's 4663 address: the runner is not handing one in, or every one is refused. eth_give_back(${id}) sends it back to the module.${note}` });
    else if (ph === "Arrived" && nowS - since >= ARRIVED_LATE_S) alerts.push({ kind: "canister", sev: "SEV2", persistent: true, key: `eth-arrived:${buyer}:${id}:${since}`,
      text: `${who}: ${eth(body.wei)} has been at the canister's Ethereum address for ${Math.round((nowS - since) / 3600)} h and no listing within the bound was bought (the floor moved up, or the runner hands in none). The unit stays in transit (docs §9 D4); at 48 h the canister sends the ETH back to the module through Relay (eth_give_back(${id}) does it now).${note}` });
    else if (ph === "BackFunded" && nowS - since >= FUNDED_LATE_S) alerts.push({ kind: "canister", sev: "SEV2", persistent: true, key: `eth-back-funded:${buyer}:${id}:${since}`,
      text: `${who} is going BACK (it could not fill): ${eth(body.wei)} at the canister's Ethereum address has waited ${mins(nowS - since)} for a Relay quote Ethereum → 4663: the runner is not handing one in, every one is refused, or another buy holds the Relay leg. At 2 h (its quote window) it stops holding other collections' buys and waits again at Arrived, trying the way back again 24 h later; eth_give_back(${id}) puts it back at Arrived now.${note}` });
    else if (ph === "BackBridging" && nowS - since >= BRIDGING_LATE_S) alerts.push({ kind: "canister", sev: "SEV2", persistent: true, key: `eth-back-late:${buyer}:${id}:${since}`,
      text: `${who}: its way back (Relay Ethereum → 4663, order ${hexOf(body.order_id)}) mined ${mins(nowS - since)} ago and at least ${eth(body.min_out)} has not reached the canister's 4663 address, and no refund is seen on Ethereum. It holds the Relay leg; look at the order. It stays watched until its payout is read (review round 4, C4-M-1: a payout later than the watch was read as another order's); once it is past its deadline + 1 h + 24 h and Relay confirms nothing more will come, eth_give_back(${id}) writes it off (off the Relay leg; a later payout is credited to it with eth_credit_payout).${note}` });
    // round 3, D3-I-4: the way back's deposit signed on the one Ethereum line and not mined
    else if (ph === "BackSending" && nowS - since >= BUYING_LATE_S) alerts.push({ kind: "canister", sev: "SEV2", persistent: true, key: `eth-back-sending:${buyer}:${id}`,
      text: `${who} is going BACK (it could not fill): its way-back deposit (Relay Ethereum → 4663, order ${hexOf(body.order_id)}) ${hexOf(body.tx)} has not been mined for ${mins(nowS - since)} (under-priced for Ethereum's fees, over the chain cap, or dropped). The Ethereum line waits on it: no other collection's buy, delivery or return is signed until it mines. Look at the transaction.${note}` });
    else if (ph === "Buying" && nowS - seen(`${id}:Buying`) >= BUYING_LATE_S) alerts.push({ kind: "canister", sev: "SEV2", persistent: true, key: `eth-buying-late:${buyer}:${id}`,
      text: `${who}: its Seaport fill ${hexOf(body.tx)} has not been mined for ${mins(nowS - seen(`${id}:Buying`))} (under-priced for Ethereum's fees, or dropped). The Ethereum line waits on it (the canister re-prices it at its nonce every 30 min up to the chain cap); no other collection's buy or delivery is signed until it mines. Look at the transaction.${note}` });
    else if (ph === "Bought" && nowS - seen(`${id}:Bought`) >= BOUGHT_LATE_S) alerts.push({ kind: "canister", sev: "SEV2", persistent: true, key: `eth-bought-late:${buyer}:${id}`,
      text: `${who} bought ${hexOf(body.token)} in block ${body.block} but it is not final (or not minted) after ${mins(nowS - seen(`${id}:Bought`))}: are the Ethereum providers' finalized blocks moving?${note}` });
    else if (ph === "Returned") alerts.push({ kind: "canister", sev: "INFO", key: `eth-returned:${buyer}:${id}`,
      text: `${who} was RETURNED: ${eth(body.wei)} went back to the module; nothing was bought.${note}` });
    // round 4 (C4-M-1): a written-off order with no payout credited: its ETH may still land at one of our addresses, where no record
    // accounts for it until the operator credits it
    const lateO = Array.isArray(b.late) ? b.late[0] : b.late;
    if (lateO && !(Array.isArray(lateO.credited) ? lateO.credited.length : lateO.credited)) {
      const released = Array.isArray(lateO.released_at) ? lateO.released_at.length > 0 : lateO.released_at !== undefined && lateO.released_at !== null;
      // review round 6 (D6-L-3): while it holds the leg, a persistent SEV2 (SEV1 after LATE_HOLD_SEV1_S): no buy, way back or sweep
      // starts for ANY collection until it is credited or released
      const letGo = Number(lateO.let_go_at ?? 0);
      if (!released) alerts.push({ kind: "canister", sev: nowS - letGo >= LATE_HOLD_SEV1_S ? "SEV1" : "SEV2", persistent: true,
        key: `eth-late-hold:${buyer}:${id}`,
        text: `${who}: its written-off Relay order ${hexOf(lateO.watch?.order_id)} HOLDS THE RELAY LEG for every collection (${Math.round((nowS - letGo) / 3600)} h since the write-off): no new buy, way back or sweep starts until a payout of it is credited (eth_credit_payout(${id}, …)) or the operator, once Relay's own records show where its ETH went (refunded elsewhere, or definitively lost), releases it with eth_late_release(${id}). Never release it only to free the leg while a payout may still come (docs/v7/phase1-nft.md §5.1).` });
      alerts.push({ kind: "canister", sev: "SEV3", key: `eth-late-order:${buyer}:${id}`,
        text: `${who}: its Relay order ${hexOf(lateO.watch?.order_id)} (${lateO.back ? "the way back, Ethereum → 4663" : "4663 → Ethereum"}, ${eth(lateO.watch?.value ?? 0)}) was written off unpaid. If Relay pays it after all (its status page names the transaction, whose calldata names the order), credit it: eth_credit_payout(${id}, <chain id>, <tx hash>, <wei if the tx does not show it>). Until then that ETH at our address is nobody's in the records. ${released ? "The operator released its hold on the Relay leg: a payout that lands while a new order is watched may be read as that order's." : "It holds the Relay leg for new orders (review round 5, C5-M-1) until credited, or until the operator, once Relay's records show where its ETH went, releases it with eth_late_release(" + id + ")."}` });
    }
  }
  // review round 5 (F5-M-1): our 4663 transaction whose nonce is used and that no provider shows: never done again on that absence alone.
  // The canister reads back its effect or identifies what used the nonce; a Relay deposit or a sweep's fundReserve (nothing to read
  // back) waits until the chain shows it or the operator looks and says (eth_home_clear)
  const pend = Array.isArray(state?.pending) ? state.pending[0] : state?.pending;
  const usedSince = pend ? Number(Array.isArray(pend.used_since) ? (pend.used_since[0] ?? 0) : (pend.used_since ?? 0)) : 0;
  // review round 6 (D6-L-1): the step decides what the operator can do. eth_home_clear is applied only to a Relay deposit or a sweep's
  // fundReserve (nothing to read back); every other step is decided by its own effect on chain, and the canister refuses the call
  const [stepName, stepBody] = phaseOf(pend?.step);
  const stepBuy = stepBody?.buy !== undefined ? (state?.buys ?? []).find((b) => String(b.id) === String(stepBody.buy)) : null;
  const clearable = stepName === "Deposit" || (stepName === "GiveBack" && optTrue(stepBuy?.sweep));
  const stepText = stepName ? `${stepName}${stepBody?.buy !== undefined ? ` of buy #${stepBody.buy}` : ""}${stepName === "GiveBack" ? (clearable ? " (a sweep's fundReserve)" : " (returnAdvance)") : ""}` : "an unknown step";
  if (pend && usedSince > 0 && nowS - usedSince >= HOME_UNSEEN_S) alerts.push({ kind: "canister", sev: "SEV1", persistent: true,
    key: `eth-home-unseen:${buyer}:${hexOf(pend.hash)}`,
    text: clearable
      ? `the buyer ${buyer}'s 4663 transaction ${hexOf(pend.hash)} (${stepText}, nonce ${pend.nonce}) is seen by no provider for ${mins(nowS - usedSince)} although its nonce is used: it most likely MINED and every provider's index misses it, and it leaves nothing on chain the canister can read back. The canister never does it again on that absence alone; the one 4663 line waits. Look it up on an explorer (the 4663 explorer's page for our address, by nonce ${pend.nonce}): if it mined, eth_home_clear(${pend.nonce}, true); if another transaction of ours took the nonce, eth_home_clear(${pend.nonce}, false) (docs/v7/phase1-nft.md §5.1, §7).`
      : `the buyer ${buyer}'s 4663 transaction ${hexOf(pend.hash)} (${stepText}, nonce ${pend.nonce}) is seen by no provider for ${mins(nowS - usedSince)} although its nonce is used. The canister decides this step by its own effect on chain (read back each tick), never by the operator's word: eth_home_clear does not apply. If it keeps waiting, its effect read is failing: look at the canister's 4663 providers (the tick lines say why).` });
  // review round 6 (D6-L-2): a home transaction out too long whatever the providers show (a stale pool entry, providers that disagree)
  // review round 7 (D7-I-2 / E7-I-2): a transaction signed by a build before round 6 carries no sent_at: its age is counted from when the
  // monitor first saw it out
  const sentAt0 = pend ? Number(Array.isArray(pend.sent_at) ? (pend.sent_at[0] ?? 0) : (pend.sent_at ?? 0)) : 0;
  const sentAt = pend ? (sentAt0 > 0 ? sentAt0 : seen(`home-out:${hexOf(pend.hash)}`)) : 0;
  if (pend && sentAt > 0 && nowS - sentAt >= HOME_OUT_S) alerts.push({ kind: "canister", sev: nowS - sentAt >= HOME_OUT_SEV1_S ? "SEV1" : "SEV2", persistent: true,
    key: `eth-home-out:${buyer}:${hexOf(pend.hash)}`,
    text: `the buyer ${buyer}'s 4663 transaction ${hexOf(pend.hash)} (${stepText}, nonce ${pend.nonce}) was signed ${mins(nowS - sentAt)} ago and is still not resolved: nothing else is signed on 4663 meanwhile (posts, records, mints, advances). ${usedSince > 0 ? `Its nonce is used and no provider shows it (${mins(nowS - usedSince)}).` : "A provider still shows it in its pool, or the providers disagree on it: look at the transaction and at the canister's 4663 providers."}` });
  // docs/v9/solana-migration.md §4 (phase B1): the Solana home's one line as B1 kept it (EthState.sol; phase B2 moves it canister-wide at the
  // upgrade, read with `solLineAlerts` from the `sol_line` query): an incident, a refused simulation, a step out too long
  const sol = Array.isArray(state?.sol) ? state.sol[0] : state?.sol;
  if (sol) alerts.push(...solLineAlerts({ buyer, line: sol, nowS, prefix: "eth-sol" }));
  // review round 6 (B6-M-1 / D6-I-1): the canister's own sale scan
  // review round 8 (B8-I-1): with the configuration's collections given, a scan of a collection no longer configured (removed by an upgrade:
  // its entry frozen) pages nothing
  const configured = Array.isArray(collections) ? new Set(collections.map((c) => String(c).toLowerCase())) : null;
  for (const sc of (Array.isArray(state?.sale_scan) ? state.sale_scan[0] : state?.sale_scan) ?? []) {
    const col = hexOf(sc.collection), opt = (x) => (Array.isArray(x) ? x[0] : x);
    if (configured && !configured.has(col)) continue;
    const lost = opt(sc.lost), hole = opt(sc.hole), rfail = opt(sc.rfail) ?? [], current = opt(sc.current);
    // review round 7 (B7-L-3): the lag from where the scan was last current, against the monitor's own Ethereum head when it has one (a
    // scan that stopped running never rewrites `behind`); a canister before round 7 has no `current`: its `behind`
    const finalHead = ethHead !== null && ethHead !== undefined ? Number(ethHead) - sb.finalDepth : null;
    const behind = current !== undefined && current !== null && finalHead !== null ? Math.max(finalHead - Number(current), Number(opt(sc.behind) ?? 0))
      : Number(opt(sc.behind) ?? 0);
    if (behind >= sb.behind) alerts.push({ kind: "canister", sev: behind >= sb.behindSev1 ? "SEV1" : "SEV2", persistent: true,
      key: `eth-scan-behind:${buyer}:${col}`,
      text: `the canister's sale scan of ${col} was last current ${behind} final blocks ago (${(behind / sb.perHour).toFixed(1)} h): no newer sale is counted and rises are held. Look at the canister's Ethereum providers (a rate limit, an outage), its cycles (the evidence floor), the day's scan budget and the runner's floor hand-ins (review round 7, B7-L-3).` });
    // review round 8 (B8-L-1): the walk back to the window's start stuck above it. The canister leaves `down` there while every read below it
    // fails as anything but "every provider too deep" (one provider of the pair lost its log retention after go-live: `Other`, retried
    // every pass, never `lost`): the window's older sales are silently uncounted. Paged once `down` has not moved for SCAN_WINDOW_UNREAD_S
    // while it is still SCAN_WINDOW_GAP_BLOCKS above the window's start (measured from the monitor's own final head, like the lag above). A
    // walk back that moves (the first hours after install) never pages; a scan that stopped is eth-scan-behind's
    const down = opt(sc.down), up = opt(sc.up);
    if (finalHead !== null && down !== undefined && down !== null && Number(up ?? 0) !== 0) {
      const gap = Number(down) - (finalHead - sb.window);
      const since = gap >= sb.gap ? seen(`scan-down:${col}:${down}`) : nowS;
      if (gap >= sb.gap && nowS - since >= SCAN_WINDOW_UNREAD_S) alerts.push({ kind: "canister", sev: "SEV2", persistent: true,
        key: `eth-scan-window-unread:${buyer}:${col}`,
        text: `the canister's sale scan of ${col} has not read the oldest ${gap} blocks of its 7-day sale window (${(gap / sb.perHour).toFixed(1)} h): its walk back has been stuck at block ${down} for ${Math.round((nowS - since) / 3600)} h, so those sales are not counted and the floor's sample is short. The usual cause is ONE provider of the canister's pair no longer serving logs that deep (review round 8, B8-L-1: one provider's "too deep" is never taken, so nothing else pages it). Run python3 script/depth_probe.py <the pair> (the daily depth-probe workflow runs it too) and replace a failing provider by a timelocked upgrade; the tick's notes name the refusal.` });
    }
    const hd = opt(sc.hole_deep);
    if (hd && Number(hd[1]) >= 3) alerts.push({ kind: "canister", sev: "SEV2", persistent: true, key: `eth-scan-deep:${buyer}:${col}`,
      text: `every one of the canister's Ethereum providers refuses blocks ${hole ? `${hole[0]}..${hole[1]}` : "of a re-anchor's hole"} of ${col} as deeper than it keeps logs (${hd[1]} times since ${new Date(Number(hd[0]) * 1000).toISOString()}): those sales are not counted; the canister reads them again every 6 h until they leave the window. The pair must serve the whole window (golive-check --probe-depth); replace a shallow provider by an upgrade (review round 7, B7-L-1).` });
    if (hole) { const since = seen(`scan-hole:${col}`); if (nowS - since >= SCAN_HOLE_DAY_S) alerts.push({ kind: "canister", sev: "SEV2", persistent: true, key: `eth-scan-hole-day:${buyer}:${col}`,
      text: `the canister's sale scan of ${col} has had blocks passed over by a re-anchor unread for ${Math.round((nowS - since) / 3600)} h (now ${hole[0]}..${hole[1]}): their sales are not counted (review round 7, B7-L-1).` }); }
    const rb = (opt(sc.rbusy) ?? []).filter((r) => Number(r[1]) >= SCAN_BUSY_TRIES).length;
    if (rb) alerts.push({ kind: "canister", sev: "SEV2", persistent: true, key: `eth-scan-busy-receipts:${buyer}:${col}`,
      text: `a provider answers ${rb} receipt(s) of ${col}'s moves as a rate limit ${SCAN_BUSY_TRIES} times in a row or more (a quota, or a provider refusing on purpose): they are backed off (at most every 6 h) behind newer sales, and not counted until read (review round 7, B7-L-2).` });
    const bz = opt(sc.busy);
    if (bz && Number(bz[1]) >= SCAN_BUSY_PASSES) alerts.push({ kind: "canister", sev: "SEV2", persistent: true, key: `eth-scan-busy:${buyer}:${col}`,
      text: `a provider has answered the canister's sale scan of ${col} as a rate limit in ${bz[1]} passes in a row (since ${new Date(Number(bz[0]) * 1000).toISOString()}): look at the canister's Ethereum providers' limits from the IC's addresses (review round 7, B7-L-2).` });
    if (lost) alerts.push({ kind: "canister", sev: "SEV2", persistent: true, key: `eth-scan-lost:${buyer}:${col}`,
      text: `the canister's Ethereum providers refused blocks ${lost[0]}..${lost[1]} of ${col}'s 7-day sale window as deeper than they keep logs: those sales are not counted, the floor's sample is short (review round 6, B6-M-1). The canister's pair must serve the whole window (golive-check --probe-depth); replace a shallow provider by an upgrade.` });
    if (hole) alerts.push({ kind: "canister", sev: "SEV3", key: `eth-scan-hole:${buyer}:${col}`,
      text: `the canister's sale scan of ${col} re-anchored at the newest blocks; blocks ${hole[0]}..${hole[1]} are still being read (newest first).` });
    const slow = rfail.filter((r) => Number(r[1]) >= 3).length;
    if (slow) alerts.push({ kind: "canister", sev: "SEV3", key: `eth-scan-receipts:${buyer}:${col}`,
      text: `the canister cannot read ${slow} receipt(s) of ${col}'s moves after 3 tries (a provider that no longer serves them, or the providers disagree): they are retried every few hours behind newer sales (B6-L-1).` });
  }
  for (const d of state?.deliveries ?? []) {
    const [ph, body] = phaseOf(d.phase);
    if (["Recorded", "Marked", "Gone"].includes(ph)) continue;
    const t = String(d.ticket), by = Number(d.deliver_by), resolvedAt = by - DELIVERY_DEADLINE_S;
    const who = `ticket ${t} of ${hexOf(d.wrapper)} (token ${BigInt(hexOf(d.token)).toString()} to ${hexOf(d.destination)}, phase ${ph})`;
    // round 3 (D3-H-1): a transfer that MINED is delivered; the canister records it once it is final, whoever owns the NFT by then. The
    // late-window refund is off only once noteSent landed (`sent`): until then the ticket can still be refunded 30 days on
    if (by - nowS <= DEADLINE_NEAR_S && DELIVERY_MINED.includes(ph)) {
      const sent = optTrue(d.sent);
      alerts.push({ kind: "canister", sev: sent ? "SEV2" : "SEV1", persistent: true, key: `eth-deliver-deadline:${buyer}:${t}:mined:${sent ? "sent" : "unsent"}`,
        text: `${who} is within a day of its delivery deadline (${new Date(by * 1000).toISOString()}) and not recorded on 4663 yet, but its transfer MINED (${hexOf(body?.tx)}): the canister records it once it is final (the redeemer may already have moved the NFT on; that is fine). ${sent
          ? "Its sent note (noteSent) is on 4663: the wrapper's late-window refund is off for this ticket. Look at why the record waits (finality, the runner's ticks, the 4663 line)."
          : "Its sent note (noteSent) is NOT on 4663: the wrapper's late-window refund is still possible 30 days after the deadline although the NFT was delivered (one unit unbacked). The canister notes a mined delivery at finality (round 4, A4-L-2), just before it records it: look if this lasts past finality."}${d.note ? ` Note: ${d.note}` : ""}` });
    } else if (by - nowS <= DEADLINE_NEAR_S) alerts.push({ kind: "canister", sev: "SEV1", persistent: true, key: `eth-deliver-deadline:${buyer}:${t}`,
      text: `${who} is within a day of its delivery deadline (${new Date(by * 1000).toISOString()}) and not recorded: the canister cancels a pending transfer and marks it undeliverable, or the holder waits 30 days. Look now.${d.note ? ` Note: ${d.note}` : ""}` });
    else if (nowS - resolvedAt >= RESOLVED_LATE_S) alerts.push({ kind: "canister", sev: "SEV2", persistent: true, key: `eth-deliver-late:${buyer}:${t}`,
      text: `${who} was resolved ${mins(nowS - resolvedAt)} ago and is not delivered yet (the runner ticks, the Ethereum line, the gas pot?).${d.note ? ` Note: ${d.note}` : ""}` });
  }
  for (const x of state?.deposits ?? []) {
    const [ph, body] = phaseOf(x.phase);
    const tx = hexOf(x.tx), key = `${tx}:${hexOf(x.token)}`;
    const who = `the NFT ${BigInt(hexOf(x.token) === "0x" ? 0 : hexOf(x.token)).toString()} of ${hexOf(x.collection)} sent to the canister by ${hexOf(x.from)} (tx ${tx}, block ${x.block})`;
    const since = seen(`dep:${key}:${ph}`);
    if (ph === "Seen" && nowS - since >= DEPOSIT_SEEN_LATE_S) alerts.push({ kind: "canister", sev: "SEV2", persistent: true, key: `eth-deposit-seen:${buyer}:${key}`,
      text: `${who} is a valid deposit for ${hexOf(x.recipient)} but is not final or not past its checks after ${mins(nowS - since)}: are the Ethereum providers' finalized blocks moving? The canister's note is in its log.` });
    else if (ph === "Mint" && nowS - since >= DEPOSIT_MINT_LATE_S) alerts.push({ kind: "canister", sev: "SEV2", persistent: true, key: `eth-deposit-mint:${buyer}:${key}`,
      text: `${who} has waited ${Math.round((nowS - since) / 3600)} h for its mint (the day's mint limits, a purchase ahead of it, or the listing paused).` });
    else if ((ph === "Return" || ph === "Returning") && nowS - since >= RETURN_LATE_S) alerts.push({ kind: "canister", sev: "SEV2", persistent: true, key: `eth-return-late:${buyer}:${key}`,
      text: `${who} is a stray (${body?.why ?? "returning"}) and has not gone back to its sender after ${mins(nowS - since)}: the day's return budget, the gas pot kept for deliveries, or finality.` });
    else if (ph === "Held") alerts.push({ kind: "canister", sev: "SEV2", persistent: true, key: `eth-deposit-held:${buyer}:${key}`,
      text: `${who} is HELD at the canister's Ethereum address and will not be minted or returned automatically: ${body?.why}. The operator looks (docs/v7/phase1-nft.md §2.4).` });
    else if (ph === "Returned") alerts.push({ kind: "canister", sev: "INFO", key: `eth-returned-stray:${buyer}:${key}`,
      text: `${who} went back to its sender (${body?.tx ? hexOf(body.tx) : "?"}): it had no valid claim (docs §2.4).` });
  }
  return { alerts, next };
}

// ---- docs/v7/dormant-collections.md: dormant, wake-on-demand collections --------------------------------------------------------------

/** A wake still WAKING this long pages SEV2 (the canister gives it up at its own timeout, 12 h by default). */
export const WAKE_SLOW_S = 6 * 3600;
/** A wake queued this long pages SEV2: every slot (max_active) is held, by real positions or by griefing (dormant-review DR-M-3). */
export const WAKE_QUEUED_S = 6 * 3600;
/** A graduation queued at an ACTIVE module this long pages SEV2: the canister's fill hunt did not find its coin (DR-M-2). */
export const GRAD_QUEUED_S = 3600;
/** A wake that timed out pages SEV2 for this long after it did (one page per timeout). */
export const WAKE_TIMEOUT_PAGE_S = 24 * 3600;

/** Pure: the canister's life of each collection (lower-case hex), from `eth_nft_state().dorm`; null when the canister keeps none
 *  (no dormancy configured: every collection is watched as before). */
export function livesOf(state) {
  const d = Array.isArray(state?.dorm) ? state.dorm[0] : state?.dorm;
  if (!d) return null;
  const m = new Map();
  for (const l of d.lives ?? []) {
    const [phase, body] = phaseOf(l.phase);
    const q = Array.isArray(l.queued) ? l.queued[0] : l.queued;
    m.set(hexOf(l.collection), { phase, since: Number(l.since ?? 0), waking_since: Number(body?.since ?? 0), note: String(l.note ?? ""),
      ever_active: optTrue(l.ever_active) || l.ever_active === true, queued_at: q ? Number(q[1]) : null, queued_by: q ? Object.keys(q[0] ?? {})[0] : null });
  }
  return m;
}

/** Pure: whether the monitor watches this collection's floor and poster: always without dormancy; once dormancy is on, only an ACTIVE
 *  one. A DORMANT or WAKING collection has no holder whose burn a stale floor could refuse, and pausing its poster (D8, the no-good-post
 *  pause) would need the Safe to undo and would wedge every later wake. */
export function watched(lives, collection) {
  if (!lives) return true;
  const l = lives.get(String(collection).toLowerCase());
  return !l || l.phase === "Active";
}

/** Pure: the FloorModule addresses (lower-case) of the configured collections that are not ACTIVE: the no-good-post pause skips them. */
export function sleepingModules(state, ethCfg) {
  const lives = livesOf(state);
  if (!lives) return new Set();
  return new Set((ethCfg?.collections ?? []).filter((c) => !watched(lives, c.collection)).map((c) => String(c.module).toLowerCase()));
}

/** Pure: docs/v7/dormant-collections.md §11: the canister's capacity as of its last tick (`dorm.cap`), numbers as numbers; null without one. */
export function capacityOf(state) {
  const d = Array.isArray(state?.dorm) ? state.dorm[0] : state?.dorm;
  const c = Array.isArray(d?.cap) ? d.cap[0] : d?.cap;
  if (!c) return null;
  const n = (x) => Number(x ?? 0);
  return { at: n(c.at), balance_t: Number(BigInt(c.balance ?? 0) / 1_000_000_000n) / 1000, scaling: c.scaling === true, collections: n(c.collections),
    ceiling: n(c.ceiling), effective_max_active: n(c.effective_max_active), awake: n(c.awake), pinned: n(c.pinned), wake_headroom: n(c.wake_headroom),
    wakes_left_today: n(c.wakes_left_today), next_wake_at_t: Number(BigInt(c.next_wake_at ?? 0) / 1_000_000_000n) / 1000, status: String(c.status ?? "") };
}

/** A capacity snapshot older than this is not paged on (the canister's tick is what writes it: a stalled tick has its own page). */
export const CAPACITY_STALE_S = 3 * 3600;

/** Pure: a wake that is slow, and one that timed out; §11: fewer collections can be awake on the cycles balance than hold positions. */
export function dormancyAlerts({ buyer, state, nowS }) {
  const alerts = [];
  const cap = capacityOf(state);
  // §11: the collections with positions stay awake whatever the balance (never stranded), but the balance no longer pays their runway
  if (cap && cap.scaling && cap.effective_max_active < cap.pinned && nowS - cap.at < CAPACITY_STALE_S) alerts.push({ kind: "canister", sev: "SEV2",
    persistent: true, key: `eth-capacity-short:${buyer}:${cap.effective_max_active}:${cap.pinned}`,
    text: `the buyer ${buyer}'s cycles balance (${cap.balance_t} T) keeps only ${cap.effective_max_active} collection(s) awake for the runway with the reserve, but ${cap.pinned} hold positions: they stay awake (their holders are never stranded) and no new wake starts. TOP IT UP: one more wake starts at ${cap.next_wake_at_t} T (docs/v7/dormant-collections.md §11; dfx cycles top-up, runbook §6).` });
  const lives = livesOf(state);
  if (!lives) return alerts;
  for (const [col, l] of lives) {
    if (l.phase === "Waking" && nowS - l.waking_since >= WAKE_SLOW_S) alerts.push({ kind: "canister", sev: "SEV2", persistent: true,
      key: `eth-wake-slow:${buyer}:${col}:${l.waking_since}`,
      text: `the buyer ${buyer}'s collection ${col} has been WARMING UP for ${Math.round((nowS - l.waking_since) / 3600)} h: its 7-day sale backfill is not read, or its reading is not confirmed by the counted sales (${l.note.slice(0, 300)}). No buy or deposit mint runs until it is ACTIVE; the canister gives the wake up at its timeout (docs/v7/dormant-collections.md §2.3). Look at the runner's evidence hand-ins and the canister's Ethereum providers.` });
    if (l.phase === "Dormant" && l.queued_at !== null && nowS - l.queued_at >= WAKE_QUEUED_S) alerts.push({ kind: "canister", sev: "SEV2", persistent: true,
      key: `eth-wake-queued:${buyer}:${col}:${l.queued_at}`,
      text: `the buyer ${buyer}'s wake of ${col} has waited ${Math.round((nowS - l.queued_at) / 3600)} h in the queue (${l.queued_by}): every slot the cycles balance allows (effective_max_active, under the ceiling max_active) is awake, or the day's wakes (wakes_per_day) are spent. If the awake collections hold real positions, the remedy is a top-up (the limit follows the balance, docs/v7/dormant-collections.md §11), or a timelocked raise of the ceiling; wakes nobody uses sleep six hours after they became active (docs/v7/dormant-collections.md §5; dormant-review DR-M-3).` });
    if (l.phase === "Dormant" && /timed out/.test(l.note) && nowS - l.since < WAKE_TIMEOUT_PAGE_S) alerts.push({ kind: "canister", sev: "SEV2",
      key: `eth-wake-timeout:${buyer}:${col}:${l.since}`,
      text: `the buyer ${buyer}'s wake of ${col} TIMED OUT: ${l.note.slice(0, 300)}. It is dormant again with its cooldown; a wash-sale campaign holding the confirmation back, thin listings or a provider outage are the usual causes (docs/v7/dormant-collections.md §2.3, review §2).` });
  }
  return alerts;
}

/** Pure: inbound NFT transfers to the custodian (Transfer logs, `to` = custodian) that the canister has not judged, 2 h on. */
export function unclaimedAlerts({ buyer, slug, logs, head, state }) {
  const judged = new Set((state?.deposits ?? []).map((x) => hexOf(x.tx)));
  const ours = new Set((state?.buys ?? []).map((b) => phaseOf(b.phase)[1]?.tx).filter(Boolean).map(hexOf));
  const alerts = [], seenTx = new Set();
  for (const l of logs) {
    const tx = String(l.transactionHash).toLowerCase();
    if ((l.topics ?? []).length !== 4 || seenTx.has(tx) || judged.has(tx) || ours.has(tx)) continue;
    seenTx.add(tx);
    const age = head - BigInt(l.blockNumber);
    if (age < UNCLAIMED_LATE_BLOCKS) continue;
    alerts.push({ kind: "canister", sev: "SEV2", persistent: true, key: `eth-unclaimed:${buyer}:${tx}`,
      text: `${slug}: an NFT reached the canister's Ethereum address in ${tx} (${age} blocks ago) and the canister has not judged it: the runner's deposit discovery is not handing it in (RUNNER_ETH_NFT, ETH_LOGS_RPC). Hand it in with eth_deposit("${tx}").` });
  }
  return alerts;
}

/**
 * Pure: a collection's pool (the module's ETH-backed units not yet converted, net of transit and the reserve) is due a buy once it
 * holds one item plus the buffer; below that it is pooling, which is normal (docs/v7/phase1-nft-pooled.md §3). Due and no open buy
 * for this collection for `POOL_IDLE_S`: SEV2. `open`: whether the canister reports an open buy for this collection.
 */
export function poolAlerts({ buyer, slug, collection, netUnits, buffer, maxItemCost, open, nowS, prev = {} }) {
  const next = { dueSince: {} }, key = String(collection).toLowerCase();
  const due = netUnits >= ONE + buffer;
  if (!due || open) return { alerts: [], next, due };
  const since = (next.dueSince[key] = prev.dueSince?.[key] ?? nowS);
  if (nowS - since < POOL_IDLE_S) return { alerts: [], next, due };
  const why = maxItemCost === 0n ? " The module's maxItemCost is 0: its floor is stale or unposted, or an advance is out." : "";
  return { due, next, alerts: [{ kind: "canister", sev: "SEV2", persistent: true, key: `eth-pool-idle:${buyer}:${key}`,
    text: `${slug}: the pool holds ${(Number(netUnits / 10n ** 14n) / 1e4).toFixed(4)} units (a buy is due at ${(Number((ONE + buffer) / 10n ** 14n) / 1e4).toFixed(4)}) and the canister has had no open buy for this collection for ${Math.round((nowS - since) / 3600)} h: another collection holding the Relay leg, the custody cap, or the runner.${why}` }] };
}

// ---- the floor against the public references (D8), audit 2026-09-27-nft D-M-1 ------------------------------------------------------
export const REF_MISSING_S = 30 * 60;            // a reference unread this long pages (SEV2); both unread pages at once
export const REF_AGREE_BPS = 1_000;              // the two references must agree within 10 % before either is believed for a pause
/** D8 per collection (the operator, 2026-10-09): the monitor entry names OpenSea as its only price reference (config.mjs holds the shape). */
export const openseaAlone = (c) => Array.isArray(c?.price_refs) && c.price_refs.length === 1 && c.price_refs[0] === "opensea";
export const READING_FRESH_S = 3_600;            // the canister's own reading (ethflow READING_TTL_SECS)
export const REF_EVERY_S = 5 * 60;               // each reference is read at most this often per collection (CoinGecko's keyless 429s)

const gapBps = (x, r) => Number(((x > r ? x - r : r - x) * 10_000n) / r);

/** The fill's gas the canary allows once a line transaction's nonce is used (a fill's gas is ~200k; the cancel's 21k). */
export const LINE_GAS_ALLOW = 300_000n;

/**
 * Pure (audit 2026-09-27-nft round 2, D2-M-1a). What a buy's earmark is at the canary's block: what arrived for it (`Arrived`,
 * `Buying`, and a way back not yet mined: `BackFunded`, `BackSending`), except that once the chain shows the nonce of the line's
 * transaction for this buy used (`chainNonce > line.nonce`: its fill, its cancel or its way back MINED), the canister may not have
 * read it yet: what it may have spent (its value, or the fill's total for a cancel's rival, and its gas at the line's max fee) is
 * taken off. So an honest fill is never read as ETH gone; ETH that leaves without a transaction of ours still is.
 */
export function earmarkOf(b, line, chainNonce) {
  const [ph, body] = phaseOf(b.phase);
  const mark = ph === "Arrived" || ph === "BackFunded" || ph === "BackSending" ? BigInt(body.wei) : ph === "Buying" ? BigInt(body.arrived) : 0n;
  if (!mark || !line) return mark;
  const [kind, k] = phaseOf(line.kind);
  const ours = ["Buy", "CancelFill", "BackDeposit"].includes(kind) && BigInt(k?.buy ?? -1) === BigInt(b.id);
  if (!ours || !(BigInt(chainNonce) > BigInt(line.nonce))) return mark;
  const value = [BigInt(line.value ?? 0), BigInt(body?.total ?? 0), BigInt(body?.value ?? 0)].reduce((a, c) => (a > c ? a : c));
  const gas = BigInt(line.gas ?? 0) > LINE_GAS_ALLOW ? BigInt(line.gas) : LINE_GAS_ALLOW;
  const spent = value + BigInt(line.max_fee ?? 0) * gas;
  return mark > spent ? mark - spent : 0n;
}

/**
 * Pure. The floor against the public references. `refs`: [{ name, value }] with value 0n for a reference that could not be read.
 * `reading`: the canister's last reading `{ price, at }` or null. `prev`: { missing: { name: since } }. `latched`: this module's pause
 * latch (a pause already made, not yet acknowledged). Returns { alerts, pause, next }.
 *  - SEV2 when the module's accepted floor is more than 10 % from every reference (information: the module lags real moves);
 *  - a PAUSE only when the canister's own fresh reading (what it would post next, not the lagging accepted floor) is more than 20 %
 *    from BOTH references, on the same side, and the two references agree with each other within 10 %: one keyless answer, a
 *    missing one, or two that disagree never pause anything;
 *  - a reference missing 30 minutes pages; both missing pages at once (the floor is unwatched);
 *  - every key is stable (no poll time in it): one page per condition, not one a minute.
 */
// D8, per collection (the operator, 2026-10-09; deployments/launch/nft-pilot.json `priceRefs`): `alone` is a collection whose monitor
// entry names price_refs ["opensea"] (CoinGecko does not list it, so D8's second reference cannot exist). Its ONE configured reference
// then confirms a pause by itself: more than 20 % from OpenSea, on the same side, pauses. Its missing answer never pauses (no reference
// read is "the floor is NOT WATCHED", paged as for every collection). Every other collection still needs both, agreeing within 10 %.
export function floorAlerts({ slug, module, accepted, reading = null, refs, nowS = 0, prev = {}, latched = false, alone = false }) {
  const alerts = [], next = { missing: {} };
  for (const r of refs) if (!(r.value > 0n)) next.missing[r.name] = prev.missing?.[r.name] ?? nowS;
  const good = refs.filter((r) => r.value > 0n);
  const miss = refs.filter((r) => !(r.value > 0n));
  if (refs.length && !good.length) alerts.push({ kind: "chain", sev: "SEV2", persistent: true, key: `eth-floor-unwatched:${module}`,
    text: `${slug}: no public reference could be read (${miss.map((r) => r.name).join(", ")}): the floor is NOT WATCHED against the market this round, and the guardian cannot pause it.` });
  for (const r of miss) if (good.length && nowS - next.missing[r.name] >= REF_MISSING_S) alerts.push({ kind: "chain", sev: "SEV2", persistent: true,
    key: `eth-floor-ref-missing:${module}:${r.name}`, text: `${slug}: the ${r.name} floor reference has not been read for ${Math.round((nowS - next.missing[r.name]) / 60)} min: D8's pause needs both references, so it is off for this collection until it answers.` });
  if (!good.length || accepted === 0n) return { alerts, pause: false, next };
  const gaps = good.map((r) => gapBps(accepted, r.value)), min = Math.min(...gaps);
  const refsText = good.map((r, i) => `${r.name} ${eth(r.value)} (${(gaps[i] / 100).toFixed(1)} %)`).join(", ");
  if (min > FLOOR_WARN_BPS) alerts.push({ kind: "chain", sev: "SEV2", persistent: true, key: `eth-floor-gap:${module}`,
    text: `${slug}: the module's accepted floor ${eth(accepted)} is more than 10 % from every public reference (${refsText}). The module moves at most 5 %/h and 20 %/day, so it lags a real move.` });
  const fresh = reading && reading.price > 0n && nowS - Number(reading.at) <= READING_FRESH_S;
  const one = alone && refs.length === 1;
  const agree = one ? good.length === 1 : good.length >= 2 && good.length === refs.length && gapBps(good[0].value, good[1].value) <= REF_AGREE_BPS;
  if (!fresh) return { alerts, pause: false, next };
  const rg = good.map((r) => gapBps(reading.price, r.value));
  const sameSide = good.every((r) => reading.price > r.value) || good.every((r) => reading.price < r.value);
  const far = rg.every((g) => g > FLOOR_PAUSE_BPS) && sameSide;
  if (!far) return { alerts, pause: false, next };
  const what = `the canister's own reading ${eth(reading.price)} is more than 20 % from ${one ? "its ONE" : agree ? "BOTH" : "the"} public reference${good.length > 1 ? "s" : ""} (${good.map((r, i) => `${r.name} ${eth(r.value)}, ${(rg[i] / 100).toFixed(1)} %`).join("; ")})`;
  if (!agree) {
    alerts.push({ kind: "chain", sev: "SEV1", persistent: true, key: `eth-floor-far-unconfirmed:${module}`,
      text: `${slug}: ${what}, but the references ${good.length < 2 ? "are not both read" : "disagree with each other by more than 10 %"}: NOT paused (D8 needs two references that agree). Look now: a manipulation, a broken reading, or a broken reference.` });
    return { alerts, pause: false, next };
  }
  if (latched) {
    alerts.push({ kind: "chain", sev: "SEV2", persistent: true, key: `eth-floor-far-latched:${module}`,
      text: `${slug}: ${what}. The guardian already paused this collection's floor and does not pause again until the operator acknowledges it (eth_nft.floor_pause_ack.${module}).` });
    return { alerts, pause: false, next };
  }
  alerts.push({ kind: "chain", sev: "SEV1", persistent: true, key: `eth-floor-far:${module}`,
    text: `${slug}: ${what}. The canister posts from verified listings and sales; a gap this wide, confirmed by ${one ? `its one reference (${good[0].name} alone: price_refs, the operator's decision of 2026-10-09)` : "both references"}, is a manipulation or a broken reading: the guardian pauses THIS collection's poster (its PosterSource), no other (docs/v7/phase1-nft.md §6, D8).` });
  return { alerts, pause: true, next };
}

/**
 * Review round 4 (D4-L-1): our delivery `tx` mined on EVERY provider with success and holds `collection`'s Transfer(custodian →
 * destination, token). Returns { ok: true } or { no: why }.
 */
async function deliveredOn(http, urls, tx, collection, custodian, destination, token) {
  const pad = (a) => `0x${"0".repeat(24)}${String(a).toLowerCase().replace(/^0x/, "")}`;
  const want = [TRANSFER_TOPIC, pad(custodian), pad(destination), `0x${BigInt(token).toString(16).padStart(64, "0")}`];
  for (const u of urls) {
    const r = await rpc(http, u, "eth_getTransactionReceipt", [tx]);
    if (!r) return { no: `not mined on ${new URL(u).host}` };
    if (String(r.status).toLowerCase() !== "0x1") return { no: `it reverted (${new URL(u).host})` };
    const hit = (r.logs ?? []).some((l) => String(l.address).toLowerCase() === String(collection).toLowerCase()
      && (l.topics ?? []).length === 4 && l.topics.every((t, i) => String(t).toLowerCase() === want[i]));
    if (!hit) return { no: `its receipt on ${new URL(u).host} shows no Transfer of this token from the custodian to the destination` };
  }
  return { ok: true };
}

const ERC721 = parseAbi(["function ownerOf(uint256) view returns (address)"]);
const TRANSFER_TOPIC = "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef";
const WRAP = parseAbi(["function itemCount() view returns (uint256)", "function item(uint256) view returns ((uint256 tokenId, bytes32 sourceTx, bool deposit))",
  "function itemOf(uint256) view returns (uint256)",
  // EthNftWrapper's Ticket (src/nft/EthNftWrapper.sol): which item a resolved ticket delivers, and to whom
  "function ticket(uint256) view returns ((uint8 stage, address owner, uint64 round, uint64 pinnedAt, uint64 pinnedAtBlock, uint64 deliverBy, uint64 deliverByBlock, uint256 itemId, bytes32 deliveryTx, address destination))"]);
const MODG = parseAbi(["function gradEth() view returns (uint256)"]);
const MOD = parseAbi(["function accepted() view returns (uint256)", "function netUnits() view returns (uint256)", "function buffer() view returns (uint256)",
  "function maxItemCost() view returns (uint256)", "function registry() view returns (address)"]);

async function rpc(http, url, method, params) {
  const r = await http(url, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params }) });
  const j = await r.json();
  if (j.error) throw new Error(`${method}: ${JSON.stringify(j.error).slice(0, 160)}`);
  return j.result;
}
async function call(http, url, to, abi, functionName, args = [], block = "latest") {
  const data = encodeFunctionData({ abi, functionName, args });
  return decodeFunctionResult({ abi, functionName, data: await rpc(http, url, "eth_call", [{ to, data }, block]) });
}

export const OWNER_UNREAD_POLLS = 3;             // a token's owner unread on a provider this many polls in a row pages (once)

/**
 * `ownerOf(token)` on every provider at one block (audit 2026-09-27-nft D-L-4): an error is "unread", never an owner. Returns
 * { owner } when every provider read the same address, { disagree: [..] } when they read different ones, { unread: [..] } otherwise.
 */
async function ownerOn(http, urls, collection, token, block) {
  const rs = await Promise.all(urls.map((u) => call(http, u, collection, ERC721, "ownerOf", [token], block).then((o) => ({ o: o.toLowerCase() }),
    (err) => ({ e: `${new URL(u).host}: ${String(err.message).slice(0, 80)}` }))));
  const errs = rs.filter((r) => r.e).map((r) => r.e), owners = rs.filter((r) => r.o).map((r) => r.o);
  if (errs.length) return { unread: errs };
  if (owners.some((o) => o !== owners[0])) return { disagree: owners };
  return { owner: owners[0] };
}

/**
 * The checker. `cfg.eth_nft`: { home_rpc: [4663 urls], eth_rpc: [≥ 2 Ethereum urls], collections: [{ slug, collection, wrapper, module,
 * custodian, opensea_slug, coingecko_id, price_refs? }], floor_pause_ack?: { module: epoch seconds } }. Every collection is checked on its own
 * (D-L-5: one's failed read never skips another's custody, floor or pause). Custody: every item the wrapper holds, and every NFT the
 * canister holds on its way in (bought, deposited, a stray), owned by the custodian on BOTH Ethereum providers at one block; the
 * custodian's nonce and ETH against the canister's records (D-M-3, the outbound canary). The floor: the canister's reading against
 * OpenSea's and CoinGecko's floors (D8, D-M-1).
 */
const optOf = (x) => (Array.isArray(x) ? (x.length ? x[0] : null) : x ?? null);

/** Audit C3-05: the chains of the canister's NFT sides other than the config's own (`eth_nft_config()`: each collection's `chain_id`, none
 * meaning the config's), in the order their first collection is listed (the canister's `EthNftConfig::chains`). */
export function sideChainsOf(config) {
  const c = optOf(config);
  if (!c) return [];
  const own = String(c.chain_id), out = [];
  for (const x of c.collections ?? []) {
    const id = String(optOf(x.chain_id) ?? own);
    if (id !== own && !out.includes(id)) out.push(id);
  }
  return out;
}

/** Audit C3-05: the pages of every other chain's side: `ethNftAlerts` on `eth_nft_state_of(chain)` (a halted line, a stuck or quote-less
 * buy holding the Relay leg, a late hold, a delivery near its deadline, the sale scan's lag in that chain's blocks), the side's halted line
 * (its exit is eth_line_clear_on(chain)), and a side the monitor cannot read. `prev`/`next` by chain. */
export async function sideAlerts({ ic, buyer, nowS, prev = {} }) {
  const alerts = [], next = {};
  if (!ic?.ethConfig || !ic?.ethStateOf) return { alerts, next };
  let chains;
  try { chains = sideChainsOf(await ic.ethConfig(buyer)); } catch (err) {
    alerts.push({ kind: "canister", sev: "SEV2", persistent: true, key: `eth-sides-unread:${buyer}`,
      text: `the buyer ${buyer}'s eth_nft_config() could not be read: its other chains' NFT sides (Robinhood Chain's) are not watched this poll (${String(err.message).slice(0, 200)}).` });
    return { alerts, next: prev };
  }
  for (const chain of chains) {
    let state;
    try { state = optOf(await ic.ethStateOf(buyer, chain)); } catch (err) {
      next[chain] = prev[chain] ?? {};
      alerts.push({ kind: "canister", sev: "SEV2", persistent: true, key: `eth-side-unread:${buyer}:${chain}`,
        text: `the buyer ${buyer}'s NFT side of chain ${chain} (eth_nft_state_of(${chain})) could not be read: its buys, line and deliveries are not watched this poll (${String(err.message).slice(0, 200)}).` });
      continue;
    }
    if (!state) continue;
    const r = ethNftAlerts({ buyer, state, nowS, prev: prev[chain] ?? {}, chain });
    alerts.push(...r.alerts);
    next[chain] = r.next;
    const h = optOf(state.halted);
    if (h) alerts.push({ kind: "custody-outbound", sev: "SEV1", persistent: true, key: `eth-line-halted:${buyer}:${chain}`,
      text: `THE CANISTER'S LINE ON CHAIN ${chain} IS HALTED: ${h}. Nothing more is signed on chain ${chain} (its buys, deliveries, ways back) until the operator has looked and called eth_line_clear_on(${chain}) (audit C3-05).` });
  }
  return { alerts, next };
}

export async function checkEthNft({ cfg, prev = {}, now = Date.now(), http = fetch, ic = null, guardian = null }) {
  const e = cfg.eth_nft, alerts = [], nowS = Math.floor(now / 1000);
  let next = { firstSeen: {} };
  let state = null;
  if ((e.eth_rpc ?? []).length < 2) throw new Error("eth_nft.eth_rpc needs two Ethereum providers: one is not a check");
  // review round 7 (B7-L-3): the Ethereum head first, so the canister's scan lag is measured against it (the lowest of the two)
  const heads = await Promise.all(e.eth_rpc.map((u) => rpc(http, u, "eth_blockNumber", []).then((x) => BigInt(x))));
  const ethHead = heads.reduce((a, b) => (a < b ? a : b));
  if (ic && cfg.buyer) {
    state = await ic.ethState(cfg.buyer);
    const r = ethNftAlerts({ buyer: cfg.buyer, state, nowS, prev: prev.buys ?? {}, ethHead,
      collections: (e.collections ?? []).map((c) => c.collection).filter(Boolean) });
    alerts.push(...r.alerts);
    alerts.push(...dormancyAlerts({ buyer: cfg.buyer, state, nowS }));
    // §11: effective_max_active and the wake headroom, for /status
    next = { buys: r.next, capacity: capacityOf(state) };
    if (state?.halted?.[0] ?? (typeof state?.halted === "string" ? state.halted : null)) {
      const h = Array.isArray(state.halted) ? state.halted[0] : state.halted;
      alerts.push({ kind: "custody-outbound", sev: "SEV1", persistent: true, key: `eth-line-halted:${cfg.buyer}`,
        text: `THE CANISTER'S ETHEREUM LINE IS HALTED: ${h}. Nothing more is signed on Ethereum until the operator has looked and called eth_line_clear (audit 2026-09-27-nft B-M-3).` });
    }
    // audit C3-05: every other chain's side (Robinhood Chain's collections) is read with eth_nft_state_of(chain) and paged the same way
    const s = await sideAlerts({ ic, buyer: cfg.buyer, nowS, prev: prev.sides ?? {} });
    alerts.push(...s.alerts);
    next.sides = s.next;
  }
  // audit 2026-09-27-nft round 2, D2-L-4: every per-collection timer is carried from the last poll and replaced only by a collection
  // read in full; one failed read of a collection never restarts its pool-idle, missing-reference or owner-unread clock
  next.pools = { dueSince: { ...(prev.pools?.dueSince ?? {}) } };
  next.refs = { ...(prev.refs ?? {}) };
  next.refMissing = { ...(prev.refMissing ?? {}) };
  next.latch = { ...(prev.latch ?? {}) };
  next.ownerUnread = { ...(prev.ownerUnread ?? {}) };
  next.unclaimed = { ...(prev.unclaimed ?? {}) };
  next.scanned = { ...(prev.scanned ?? {}) };
  // round 2, D2-M-1b: an item whose ownerOf is not the custodian is judged AFTER the canister's records are read again (below): an
  // honest redemption (a delivery of ours mined, recordDelivery not yet landed on 4663) is not a custody loss
  const suspects = [];
  const [home] = e.home_rpc;
  const block = `0x${(heads.reduce((a, b) => (a < b ? a : b)) - 2n).toString(16)}`;
  // `item`: the wrapper's item index the token is held as (only a wrapper-held item can be a delivery of ours; a buy's, a deposit's or
  // a stray's token has no delivery of its current custody: D3-L-1)
  const owned = async (c, token, what, key, mine, item = null) => {
    const o = await ownerOn(http, e.eth_rpc, c.collection, token, block);
    if (o.unread) {
      const n = (mine[key] = (prev.ownerUnread?.[key] ?? 0) + 1);
      if (n >= OWNER_UNREAD_POLLS) alerts.push({ kind: "custody-outbound", sev: "SEV2", persistent: true, key: `eth-owner-unread:${c.collection}:${token}`,
        text: `${c.slug}: ownerOf(${token}) (${what}) could not be read on every Ethereum provider for ${n} polls: ${o.unread.join("; ")}. Custody of it is not checked meanwhile.` });
    } else if (o.disagree) alerts.push({ kind: "custody-outbound", sev: "SEV2", persistent: true, key: `eth-owner-disagree:${c.collection}:${token}`,
      text: `${c.slug}: the Ethereum providers disagree on ownerOf(${token}) (${what}) at block ${BigInt(block)}: ${o.disagree.join(" / ")}` });
    else if (o.owner !== c.custodian.toLowerCase()) suspects.push({ c, token, what, owner: o.owner, item });
  };
  const notHeld = (x, why = "") => ({ kind: "custody-outbound", sev: "SEV1", persistent: true, key: `eth-not-held:${x.c.collection}:${x.token}`,
    text: `${x.c.slug}: token ${x.token} (${x.what}) is owned by ${x.owner} on Ethereum, not the custodian ${x.c.custodian} (both providers, block ${BigInt(block)}). An NFT left custody without a delivery of ours of this item: pause, then look.${why ? ` ${why}` : ""}` });
  for (const c of e.collections) {
    try {
      const mine = {};   // this collection's owner-unread counts, kept only if its whole read succeeds (D2-L-4)
      // custody: every item the wrapper holds
      const n = Number(await call(http, home, c.wrapper, WRAP, "itemCount"));
      let held = 0;
      for (let i = 0; i < n; i++) {
        const it = await call(http, home, c.wrapper, WRAP, "item", [BigInt(i)]);
        if (BigInt(await call(http, home, c.wrapper, WRAP, "itemOf", [it.tokenId])) !== BigInt(i + 1)) continue;   // delivered
        held++;
        await owned(c, it.tokenId, `held by the wrapper as item ${i}`, `${c.collection}:${it.tokenId}`, mine, BigInt(i));
      }
      // D-M-3: what the canister holds on its way in (a buy not yet minted, a deposit, a stray) is custody too
      if (state) {
        const coll = c.collection.toLowerCase();
        for (const b of state.buys ?? []) {
          const [ph, body] = phaseOf(b.phase);
          if (hexOf(b.collection) !== coll || !["Bought", "Final", "Minting"].includes(ph) || !body?.token) continue;
          const t = BigInt(hexOf(body.token));
          await owned(c, t, `bought by buy #${b.id} (${ph}), not yet in the wrapper`, `${c.collection}:${t}`, mine);
        }
        for (const x of state.deposits ?? []) {
          const [ph] = phaseOf(x.phase);
          if (hexOf(x.collection) !== coll || !["Seen", "Mint", "Minting", "Held", "Return"].includes(ph)) continue;
          const t = BigInt(hexOf(x.token));
          await owned(c, t, `sent to the canister in ${hexOf(x.tx)} (${ph})`, `${c.collection}:${t}`, mine);
        }
      }
      const bal = BigInt(await rpc(http, e.eth_rpc[0], "eth_getBalance", [c.custodian, block]));
      if ((held > 0 || state?.deposits?.length || (state?.deliveries ?? []).some((d) => !["Recorded", "Marked", "Gone"].includes(phaseOf(d.phase)[0])))
        && bal < GAS_POT_LOW_WEI) alerts.push({ kind: "canister", sev: "SEV2", persistent: true, key: `eth-gas-pot:${c.custodian}`,
        text: `the canister's Ethereum address ${c.custodian} holds ${eth(bal)}: under ${eth(GAS_POT_LOW_WEI)}, deliveries and returns may wait for gas (docs §9 D3).` });
      // docs/v7/dormant-collections.md: a DORMANT or WAKING collection's floor, pool and poster are not watched (nothing is held; a
      // pause of its poster would wedge its next wake). Its custody (above) and its inbound transfers (below) still are
      const awake = watched(livesOf(state), c.collection);
      // the floor
      const accepted = awake ? BigInt(await call(http, home, c.module, MOD, "accepted")) : 0n;
      const cached = next.refs[c.module];
      let refs;
      if (!awake) refs = [];
      else if (cached && nowS - cached.at < REF_EVERY_S) refs = cached.refs.map((r) => ({ name: r.name, value: BigInt(r.value) }));
      else {
        refs = [];
        if (c.opensea_slug) refs.push({ name: "OpenSea", value: await http(`https://api.opensea.io/api/v2/collections/${c.opensea_slug}/stats`)
          .then((r) => r.json()).then((j) => BigInt(Math.round(Number(j.total.floor_price) * 1e6)) * 10n ** 12n).catch(() => 0n) });
        if (c.coingecko_id && !openseaAlone(c)) refs.push({ name: "CoinGecko", value: await http(`https://api.coingecko.com/api/v3/nfts/${c.coingecko_id}`,
          process.env.MONITOR_COINGECKO_KEY ? { headers: { "x-cg-demo-api-key": process.env.MONITOR_COINGECKO_KEY } } : undefined)
          .then((r) => r.json()).then((j) => BigInt(Math.round(Number(j.floor_price.native_currency) * 1e6)) * 10n ** 12n).catch(() => 0n) });
        next.refs[c.module] = { at: nowS, refs: refs.map((r) => ({ name: r.name, value: r.value.toString() })) };
      }
      const rd = (state?.readings ?? []).find((r) => hexOf(r.collection) === c.collection.toLowerCase());
      const reading = rd ? { price: BigInt(rd.post.price), at: Number(rd.at) } : null;
      const ack = Number(e.floor_pause_ack?.[c.module] ?? e.floor_pause_ack?.[c.module?.toLowerCase()] ?? 0);
      if (next.latch[c.module] && ack >= next.latch[c.module].at) delete next.latch[c.module];
      const f = awake ? floorAlerts({ slug: c.slug, module: c.module, accepted, reading, refs, nowS, prev: { missing: prev.refMissing?.[c.module] ?? {} },
        latched: !!next.latch[c.module], alone: openseaAlone(c) }) : { alerts: [], next: { missing: {} }, pause: false };
      next.refMissing[c.module] = f.next.missing;
      alerts.push(...f.alerts);
      // dormant-review DR-M-2: a graduation queued at an ACTIVE module (the module was stale when it came): the canister's fill hunt reads
      // only the newest coins, so an older one needs the operator's eth_fill(<collection>, <the coin of its GradQueued event>)
      if (state && awake && livesOf(state)) {
        const ge = BigInt(await call(http, home, c.module, MODG, "gradEth"));
        const k = `grad-queued:${String(c.module).toLowerCase()}`;
        if (ge > 0n) {
          const since = (next.firstSeen ??= {})[k] = prev.firstSeen?.[k] ?? nowS;
          if (nowS - since >= GRAD_QUEUED_S) alerts.push({ kind: "canister", sev: "SEV2", persistent: true, key: `eth-grad-queued:${c.module}:${since}`,
            text: `${c.slug}: ${eth(ge)} of graduation has been QUEUED at its module for ${mins(nowS - since)} (it came while the module was stale): the canister fills it with FloorModule.fill(coin) once its floor is fresh, trying the newest coins and the operator's hand-ins. If this lasts, hand in eth_fill(${c.collection}, <the coin of the module's GradQueued event>) (docs/v7/dormant-collections.md §5); the curve refunds if its window passes unfilled.` });
        }
      }
      // pooled graduations: a pool due a buy with none open for a day (per collection: one pool never counts toward another)
      if (state && awake) {
        const [netUnits, buffer, maxItemCost] = await Promise.all(["netUnits", "buffer", "maxItemCost"].map((fn) => call(http, home, c.module, MOD, fn).then(BigInt)));
        const open = (state.buys ?? []).some((b) => hexOf(b.collection) === c.collection.toLowerCase()
          && !["Settled", "Returned"].includes(phaseOf(b.phase)[0]));
        const p = poolAlerts({ buyer: cfg.buyer, slug: c.slug, collection: c.collection, netUnits, buffer, maxItemCost, open, nowS, prev: prev.pools ?? {} });
        alerts.push(...p.alerts);
        delete next.pools.dueSince[String(c.collection).toLowerCase()];
        Object.assign(next.pools.dueSince, p.next.dueSince);
      }
      // D5: every NFT that reached the custodian is judged by the canister (minted or returned); one the runner never handed in pages
      // until the canister judges it (D-L-3: remembered past the scan's window)
      if (state && e.deposits_scan !== false) {
        try {
          const head = BigInt(await rpc(http, e.eth_rpc[0], "eth_blockNumber", []));
          const to = `0x${"0".repeat(24)}${c.custodian.toLowerCase().slice(2)}`;
          // in ranges of at most logs_max_blocks (Dwellir: 500), and only the blocks not read yet (the window at the first poll)
          const key = String(c.collection).toLowerCase(), max = BigInt(e.logs_max_blocks ?? LOGS_MAX_BLOCKS);
          const done = BigInt(prev.scanned?.[key] ?? 0);
          // review round 4 (D4-L-2): the last RESCAN_OVERLAP_BLOCKS already read are read again every poll: a node behind the provider's
          // load balancer that answered short, or a reorg at the tip, is healed a poll later (the unclaimed set dedups by tx hash)
          const back = done >= head - DEPOSIT_SCAN_BLOCKS && done < head ? done + 1n - RESCAN_OVERLAP_BLOCKS : head - DEPOSIT_SCAN_BLOCKS;
          const from = back > head - DEPOSIT_SCAN_BLOCKS ? back : head - DEPOSIT_SCAN_BLOCKS;
          const logs = done >= head && done > 0n && from > head ? [] : await logsInRanges((a, b) => rpc(http, e.eth_rpc[0], "eth_getLogs", [{ address: c.collection,
            topics: [TRANSFER_TOPIC, null, to], fromBlock: `0x${a.toString(16)}`, toBlock: `0x${b.toString(16)}` }]), from, head, max);
          next.scanned[key] = String(head > done ? head : done);
          const judged = new Set((state?.deposits ?? []).map((x) => hexOf(x.tx)));
          for (const l of logs ?? []) if ((l.topics ?? []).length === 4) next.unclaimed[String(l.transactionHash).toLowerCase()] ??= { slug: c.slug, block: String(BigInt(l.blockNumber)) };
          const remembered = Object.entries(next.unclaimed).filter(([, v]) => v.slug === c.slug).map(([tx, v]) => ({ transactionHash: tx, blockNumber: `0x${BigInt(v.block).toString(16)}`, topics: [1, 2, 3, 4] }));
          for (const [tx] of Object.entries(next.unclaimed)) if (judged.has(tx)) delete next.unclaimed[tx];
          alerts.push(...unclaimedAlerts({ buyer: cfg.buyer, slug: c.slug, logs: remembered.filter((l) => !judged.has(l.transactionHash)), head, state }));
        } catch (err) {
          alerts.push({ kind: "canister", sev: "SEV3", key: `eth-deposit-scan:${c.collection}:${Math.floor(nowS / 3600)}`,
            text: `${c.slug}: the inbound-transfer scan failed (${String(err.message).slice(0, 120)}): unclaimed deposits are not watched this round.` });
        }
      }
      if (f.pause && guardian && guardian.autoPause !== false && e.guardian_pause !== false) {
        const poster = String(await call(http, home, c.module, MOD, "registry")).toLowerCase();
        const r = await guardian.pausePoster(poster).catch((err) => ({ error: String(err.message) }));
        if (!r.error && !r.pending) next.latch[c.module] = { at: nowS, tx: r.tx ?? null };
        alerts.push({ kind: "chain", sev: "SEV1", persistent: true, key: `eth-floor-pause:${c.module}:${next.latch[c.module]?.at ?? "retry"}`,
          text: `${c.slug}: the guardian's pause() of this collection's poster ${poster}: ${JSON.stringify(r).slice(0, 200)}. Latched: no further pause of this collection until eth_nft.floor_pause_ack.${c.module} is at least ${next.latch[c.module]?.at ?? nowS}.` });
      }
      // the whole collection was read: its owner-unread counts replace the last poll's
      for (const k of Object.keys(next.ownerUnread)) if (k.startsWith(`${c.collection}:`)) delete next.ownerUnread[k];
      Object.assign(next.ownerUnread, mine);
    } catch (err) {
      // D-L-5: this collection's read failed; the others are still checked
      alerts.push({ kind: "watcher-error", sev: "SEV2", key: `eth-collection-unread:${c.collection}:${Math.floor(nowS / 3600)}`,
        text: `${c.slug}: the Ethereum NFT check could not read this collection (${String(err.message).slice(0, 200)}): its custody, floor and pause are not checked this round. The other collections are.` });
    }
  }
  // D-M-3: the outbound canary. Every transaction from the canister's Ethereum address is one of its line's: the nonce the chain shows
  // is at most the canister's last mined nonce + 1, or its line's + 1. The records are read AFTER the chain (the canister saves a
  // transaction before it sends it). And the arrived buys' ETH is still at the address.
  let st2 = state;
  if (state && e.collections.length) {
    const custodian = e.collections[0].custodian;
    try {
      const nonces = await Promise.all(e.eth_rpc.map((u) => rpc(http, u, "eth_getTransactionCount", [custodian, block]).then((x) => BigInt(x))));
      st2 = ic ? await ic.ethState(cfg.buyer) : state;
      // round 3, D3-L-2: the earmarks (st2) are judged against a balance read AFTER them, at a block taken after st2 (min head - 2,
      // at or past the canister's pin, 8-16 blocks behind its head): a Relay arrival the records show is in that balance. The
      // earmark's nonce is read at that same block (a fill of ours mined by then is allowed for, as in D2-M-1a)
      const heads2 = await Promise.all(e.eth_rpc.map((u) => rpc(http, u, "eth_blockNumber", []).then((x) => BigInt(x))));
      const late = `0x${(heads2.reduce((a, b) => (a < b ? a : b)) - 2n).toString(16)}`;
      const [bal, lateNonce] = await Promise.all([rpc(http, e.eth_rpc[0], "eth_getBalance", [custodian, late]).then((x) => BigInt(x)),
        rpc(http, e.eth_rpc[0], "eth_getTransactionCount", [custodian, late]).then((x) => BigInt(x))]);
      const lm = Array.isArray(st2?.last_mined) ? st2.last_mined[0] : st2?.last_mined;
      const line = Array.isArray(st2?.line) ? st2.line[0] : st2?.line;
      const allowed = [lm ? BigInt(lm[0]) + 1n : 0n, line ? BigInt(line.nonce) + 1n : 0n].reduce((a, b) => (a > b ? a : b));
      const chain = nonces.reduce((a, b) => (a > b ? a : b));
      if (chain > allowed) alerts.push({ kind: "custody-outbound", sev: "SEV1", persistent: true, key: `eth-outbound-unknown:${custodian}:${chain}`,
        text: `A TRANSACTION FROM THE CANISTER'S ETHEREUM ADDRESS ${custodian} THAT IS NOT ON ITS LINE: the chain shows nonce ${chain} at block ${BigInt(block)}, and the canister's records account for ${allowed} (its last mined nonce and its line). Its key signed something the Ethereum side did not: pause, then look.` });
      const markNonce = lateNonce > chain ? lateNonce : chain;
      const marks = (st2?.buys ?? []).map((b) => [b.id, earmarkOf(b, line, markNonce)]).filter(([, m]) => m > 0n);
      const earmarked = marks.reduce((a, [, m]) => a + m, 0n);
      // the key names the earmarked buys: a false page never mutes a later real one (of other buys) for REPEAT_MS
      const ids = marks.map(([id]) => BigInt(id)).sort((a, b) => (a < b ? -1 : a > b ? 1 : 0)).join(",");
      if (bal < earmarked) alerts.push({ kind: "custody-outbound", sev: "SEV1", persistent: true, key: `eth-arrived-gone:${custodian}:${ids}`,
        text: `the canister's Ethereum address ${custodian} holds ${eth(bal)} at block ${BigInt(late)}, under the ${eth(earmarked)} its arrived buys (#${ids.split(",").join(", #")}) are owed: ETH left without a fill of ours. Pause, then look.` });
    } catch (err) {
      alerts.push({ kind: "watcher-error", sev: "SEV3", key: `eth-outbound-unread:${Math.floor(nowS / 3600)}`, text: `the Ethereum outbound canary could not read the custodian (${String(err.message).slice(0, 160)})` });
    }
  }
  // D2-M-1b, round 3 (D3-H-1, D3-L-1, D3-I-7): an NFT not owned by the custodian is a custody loss unless it is a WRAPPER-HELD item and
  // the canister's records (read after the chain) have a delivery of that collection and token whose ticket, read on the wrapper,
  // resolved to THAT item (and to the delivery's destination), and either
  //  (a) the transfer MINED (Moved, Record, Recording, Recorded): any owner, since the redeemer may have sold or moved it on, or
  //  (b) the transfer is being mined (Sent, or Cancelling: our cancel may lose the race), and the owner is the delivery's destination.
  // A buy's, a deposit's or a stray's token is never exempt: no delivery of its current custody can exist. A ticket read that fails
  // pages (never a silent exemption).
  const deliveriesNow = st2?.deliveries ?? state?.deliveries ?? [];
  for (const x of suspects) {
    if (x.item === null) { alerts.push(notHeld(x)); continue; }
    // review round 4 (D4-L-1): a delivery still Sent / Cancelling in the canister (it ticks only when the runner calls) whose NFT is
    // no longer at the destination is exempt only when its OWN receipt shows it mined: success and the collection's Transfer from the
    // custodian to the destination, on every Ethereum provider (as the canister judges it at finality). A redeemer who resold within one
    // tick no longer pages a false SEV1
    const cands = deliveriesNow.filter((y) => {
      const [ph] = phaseOf(y.phase);
      return hexOf(y.wrapper) === x.c.wrapper.toLowerCase() && hexOf(y.collection) === x.c.collection.toLowerCase()
        && BigInt(hexOf(y.token)) === BigInt(x.token) && (DELIVERY_MINED.includes(ph) || ["Sent", "Cancelling"].includes(ph));
    });
    let ok = false;
    const why = [];
    for (const d of cands) {
      const [ph, body] = phaseOf(d.phase);
      if (["Sent", "Cancelling"].includes(ph) && hexOf(d.destination) !== x.owner) {
        const tx = ph === "Sent" ? body?.tx : body?.of;
        const mined = tx ? await deliveredOn(http, e.eth_rpc, hexOf(tx), x.c.collection, x.c.custodian, hexOf(d.destination), x.token)
          .catch((err) => ({ no: `its receipt could not be read (${String(err.message).slice(0, 100)})` })) : { no: "no transaction" };
        if (!mined.ok) {
          why.push(`the canister's delivery of ticket ${d.ticket} (${ph}) is not seen mined to ${hexOf(d.destination)}: ${mined.no}`);
          continue;
        }
      }
      try {
        const t = await call(http, home, x.c.wrapper, WRAP, "ticket", [BigInt(d.ticket)]);
        // review round 4 (D4-I-1): and the ticket still Resolved (stage 2): a refunded ticket's stale delivery exempts nothing. Review
        // round 5 (D5-L-1): or Delivered (stage 3) by THIS delivery (its deliveryTx is the delivery's own transaction): the record landed
        // between this poll's itemOf read and this read. Refunded (stage 4) never exempts
        const [dph, dbody] = phaseOf(d.phase);
        const ours = DELIVERY_MINED.includes(dph) ? dbody?.tx : undefined;
        const recorded = Number(t.stage) === 3 && ours !== undefined && String(t.deliveryTx).toLowerCase() === hexOf(ours);
        if (BigInt(t.itemId) === x.item && String(t.destination).toLowerCase() === hexOf(d.destination) && (Number(t.stage) === 2 || recorded)) { ok = true; break; }
        why.push(`the canister's delivery of ticket ${d.ticket} (${phaseOf(d.phase)[0]}) is of item ${t.itemId} to ${String(t.destination).toLowerCase()} on the wrapper, not this item to ${hexOf(d.destination)}`);
      } catch (err) {
        why.push(`ticket ${d.ticket} of the wrapper could not be read (${String(err.message).slice(0, 120)}): not exempted`);
      }
    }
    if (!ok) alerts.push(notHeld(x, why.join("; ")));
  }
  return { alerts, next };
}
