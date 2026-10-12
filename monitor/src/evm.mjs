// The 4663 canary (docs/v6/k6-watcher-canary.md §5): the Wrapper's and the FloorModule's custodian-gated events, from two
// RPCs that must agree, each matched against a canister record. Records are read AFTER the logs (the canister saves an EVM
// transaction before it sends it). Since wave 2 (docs/audit/2026-09-26/fixes-W2-monitor.md §1.1, §1.3, §1.6) it also reads the
// registry's and the sweep vaults' logs (the keeper's floor, caps and draws; the pause, the guardian, a role rotation),
// recomputes every opened draw's root, and never lets one disputed range blind it. Round 3 (docs/audit/2026-09-26/review2/
// fixes-R3-services.md §1-§2): roots come from per-provider holder indexes (holders.mjs) read forward from saved cursors; draws are
// also found from the vaults' state; every undecided draw is considered each poll, earliest window first, with a hard deadline
// that pauses the keeper; the logs are paged per provider with retry, and a floor walked along the band's edge pages.
import { keccak_256 } from "@noble/hashes/sha3";
import { DRAW_STAGE, MODULE, VAULT, decodeStackEvent, host, lc, read, rpc } from "./abi.mjs";
import { blockAtOrBefore, recompute as recomputeDraw, verdictN } from "./draws.mjs";
import { byHand } from "./guardian.mjs";
import { Provider, pagedLogs } from "./holders.mjs";
import { recordIndex } from "./records.mjs";
import { exclusionsFor, roleOf, stackOf, watchedAddresses } from "./stack.mjs";

const hex = (b) => Buffer.from(b).toString("hex");
const topic = (sig) => `0x${hex(keccak_256(new TextEncoder().encode(sig)))}`;
/** Event signatures as the compiled ABI (out/Wrapper.sol, out/FloorModule.sol) names them. */
export const SIGS = {
  Deposited: "Deposited(uint256,string,bytes32,uint32,address)", Delivered: "Delivered(uint256,uint256,bytes32)",
  Wrapped: "Wrapped(uint256,string,address)", RentalReturned: "RentalReturned(uint256,bytes32)",
  MintLimitProposed: "MintLimitProposed(uint256,uint64)", MintLimitSet: "MintLimitSet(uint256)",
  ItemArrived: "ItemArrived(uint256,uint256,uint256)",
  // r3-contracts (canister review2 N-2): the custodian's word that a resolved ticket's delivery can no longer confirm
  Undeliverable: "Undeliverable(uint256)",
  // docs/v6/relay-conversion.md §2: the custodian's advance of one unit's budget, its settlement when the frog arrives, its return
  Advanced: "Advanced(uint256,uint256)", AdvanceSettled: "AdvanceSettled(uint256,uint256,uint256)",
  AdvanceReturned: "AdvanceReturned(uint256,uint256,uint256)",
  // audit 2026-09-27-nft D-M-2: EthNftWrapper's custodian-gated events (their own signatures; its Undeliverable is the same)
  NftWrapped: "Wrapped(uint256,uint256,bytes32,address)", NftDeposited: "Deposited(uint256,uint256,bytes32,address)",
  NftDelivered: "Delivered(uint256,uint256,address,bytes32)",
  // round 3 (D3-H-1): the custodian's note that a ticket's delivery MINED on Ethereum (noteSent): the late-window refund is then off
  NftDeliverySent: "DeliverySent(uint256,uint256,bytes32)",
};
export const TOPICS = Object.fromEntries(Object.entries(SIGS).map(([k, s]) => [k, topic(s)]));
const NAME_OF = Object.fromEntries(Object.entries(TOPICS).map(([k, t]) => [t, k]));
export const CHUNK = 5_000;
export const MAX_RANGE = 50_000;

const word = (data, i) => data.slice(2 + i * 64, 2 + (i + 1) * 64);
const uint = (w) => BigInt(`0x${String(w ?? "").replace(/^0x/, "") || "0"}`);
const eth = (wei) => `${(Number(wei / 10n ** 12n) / 1e6).toString()} ETH`;

/** `Deposited`'s non-indexed fields: (string inscriptionId, bytes32 txid, uint32 vout). */
export function decodeDeposited(log) {
  const d = log.data;
  const off = Number(uint(word(d, 0))) / 32, txid = word(d, 1), vout = Number(uint(word(d, 2)));
  const len = Number(uint(word(d, off)));
  const inscription = Buffer.from(d.slice(2 + (off + 1) * 64, 2 + (off + 1) * 64 + len * 2), "hex").toString("utf8");
  const id = uint(log.topics[1]), FLAG = 1n << 255n;   // Wrapper.DEPOSIT_FLAG
  return { frogId: id >= FLAG ? `deposit #${id - FLAG}` : `#${id}`, to: `0x${log.topics[2].slice(26)}`, inscription, txid, vout };
}

const keyOf = (l) => `${l.transactionHash}:${Number(l.logIndex)}`;
const canon = (l) => [l.address.toLowerCase(), l.transactionHash.toLowerCase(), Number(l.logIndex), l.blockHash?.toLowerCase(),
  ...l.topics.map((t) => t.toLowerCase()), l.data.toLowerCase()].join("|");

/** Two providers' logs for the same range: the same set, or the differences. Pure. */
export function compareLogs(a, b) {
  const A = new Set(a.filter((l) => !l.removed).map(canon)), B = new Set(b.filter((l) => !l.removed).map(canon));
  return { same: A.size === B.size && [...A].every((x) => B.has(x)), onlyA: [...A].filter((x) => !B.has(x)), onlyB: [...B].filter((x) => !A.has(x)) };
}

/**
 * The logs, judged against the records. Pure. cfg: { wrappers: [addr], floor_module: addr, acknowledged: [txHash] }.
 * prev: { arrivals: { logKey: frogId }, open: { logKey: alert } }. Returns { alerts, next }.
 */
export function judgeLogs({ logs, index, cfg, prev = {}, timelock = "<timelock>" }) {
  const explorer = cfg.explorer ?? "https://explorer.mainnet.chain.robinhood.com";
  const alerts = [];
  const next = { arrivals: { ...(prev.arrivals ?? {}) }, advances: { ...(prev.advances ?? {}) }, open: { ...(prev.open ?? {}) } };
  const wrappers = (cfg.wrappers ?? []).map((w) => w.toLowerCase());
  const nftWrappers = (cfg.nft_wrappers ?? []).map((w) => w.toLowerCase());
  const modules = [cfg.floor_module, ...(cfg.modules ?? [])].filter(Boolean).map((m) => m.toLowerCase());
  const ack = (cfg.acknowledged ?? []).map((x) => x.toLowerCase());
  const txl = (l) => `${explorer}/tx/${l.transactionHash}`;
  const pause = `Guardian, now: pause_target on ${timelock} (G10 step 1); the Safe pauses the listing (ListingRegistry.setPaused). Pools cannot be paused: warn holders.`;
  const raise = (l, a) => { if (ack.includes(l.transactionHash.toLowerCase())) return; alerts.push(a); next.open[keyOf(l)] = a; };

  for (const l of [...logs].sort((x, y) => Number(x.blockNumber) - Number(y.blockNumber) || Number(x.logIndex) - Number(y.logIndex))) {
    if (l.removed) continue;
    const name = NAME_OF[l.topics[0]?.toLowerCase()], from = l.address.toLowerCase(), k = keyOf(l);
    const at = `block ${Number(l.blockNumber)}, tx ${l.transactionHash}`;
    const fromWrapper = wrappers.includes(from), fromModule = modules.includes(from), fromNft = nftWrappers.includes(from);
    if (fromNft && ["NftWrapped", "NftDeposited", "NftDelivered", "NftDeliverySent", "Undeliverable"].includes(name)) {
      // audit 2026-09-27-nft D-M-2: an Ethereum NFT wrapper's events, matched against the canister's Ethereum records (eth_nft_state),
      // never against the Bitcoin redemptions
      const id = uint(l.topics[1]).toString(), token = l.topics[2] ? uint(l.topics[2]).toString() : null;
      if (name === "NftWrapped" || name === "NftDeposited") {
        const tx = `0x${word(l.data, 0)}`, to = `0x${l.topics[3]?.slice(26) ?? ""}`;
        const recs = index.ethMints?.get(`${(cfg.nft_collection_of ?? {})[from] ?? ""}:${token}`) ?? [];
        if (recs.some((m) => m.tx === tx && m.to === to && m.kind === (name === "NftWrapped" ? "buy" : "deposit"))) continue;
        raise(l, { kind: "evm-mint", sev: "SEV1", persistent: true, key: `evm:${k}`,
          text: [`AN UNBACKED MINT? The Ethereum NFT wrapper ${from} minted item ${id} (token ${token}) to ${to} for ${name === "NftWrapped" ? "purchase" : "deposit"} ${tx}, and no canister record of that ${name === "NftWrapped" ? "buy" : "deposit"} names that token, transaction and recipient.`,
            `mint / mintDeposit are custodian-only: this was signed with the canister's EVM key. ${at}`, pause, txl(l)].join("\n") });
      } else if (name === "NftDelivered") {
        const txHash = `0x${word(l.data, 1)}`;
        const d = index.ethDeliveries?.get(`${from}:${id}`);
        if (d && ["Recording", "Recorded"].includes(d.phase) && d.tx === txHash) continue;
        raise(l, { kind: "evm-mint", sev: "SEV1", persistent: true, key: `evm:${k}`,
          text: [`A DELIVERY RECORD WITH NO DELIVERY: the Ethereum NFT wrapper ${from} recorded ticket ${id} (token ${token}) as delivered in Ethereum tx ${txHash}, and the canister's delivery for that ticket ${d ? `is ${d.phase}${d.tx ? ` naming ${d.tx}` : ""}` : "does not exist"}.`,
            `recordDelivery is the custodian's alone: this was signed with the canister's EVM key (a false record releases the item). ${at}`, pause, "The ticket's holder is owed an NFT.", txl(l)].join("\n") });
      } else if (name === "NftDeliverySent") {
        // DeliverySent(ticketId indexed, tokenId indexed, bytes32 txHash): backed only by a delivery of ours whose transfer mined, in that tx
        const txHash = `0x${word(l.data, 0)}`;
        const d = index.ethDeliveries?.get(`${from}:${id}`);
        if (d && ["Moved", "Record", "Recording", "Recorded"].includes(d.phase) && d.tx === txHash) continue;
        raise(l, { kind: "evm-mint", sev: "SEV1", persistent: true, key: `evm:${k}`,
          text: [`A SENT NOTE WITH NO MINED DELIVERY: the Ethereum NFT wrapper ${from} was told ticket ${id}'s delivery (token ${token}) mined in Ethereum tx ${txHash}, and the canister's delivery for that ticket ${d ? `is ${d.phase}${d.tx ? ` naming ${d.tx}` : ""}` : "does not exist"}.`,
            `noteSent is custodian-only: signed with the canister's EVM key; it switches off the ticket's late-window refund, so its holder is refunded only if the canister marks it undeliverable. ${at}`, pause, "The ticket's holder may be owed an NFT or a refund.", txl(l)].join("\n") });
      } else {
        const d = index.ethDeliveries?.get(`${from}:${id}`);
        if (d && ["Mark", "Marking", "Marked"].includes(d.phase)) continue;
        raise(l, { kind: "evm-mint", sev: "SEV1", persistent: true, key: `evm:${k}`,
          text: [`AN UNDELIVERABLE MARK NO RECORD BACKS: the Ethereum NFT wrapper ${from} was told ticket ${id}'s delivery can no longer confirm, and the canister's delivery for that ticket ${d ? `is ${d.phase}` : "does not exist"}.`,
            `markUndeliverable is custodian-only: this was signed with the canister's EVM key; the ticket may refund at deliverBy + 72 h while its NFT can still be delivered (a LostRace). ${at}`, pause, txl(l)].join("\n") });
      }
    } else if (name === "Deposited" && fromWrapper) {
      const d = decodeDeposited(l);
      const rec = index.mintOutpoints.get(`${d.txid}:${d.vout}`);
      if (rec && (!rec.inscription || rec.inscription === d.inscription)) continue;
      raise(l, { kind: "evm-mint", sev: "SEV1", persistent: true, key: `evm:${k}`,
        text: [`AN UNBACKED MINT? The wrapper ${from} minted frog ${d.frogId} (one wX) to ${d.to} for ${d.inscription} at ${d.txid}:${d.vout}, and no canister record names that outpoint${rec ? ` with that inscription (the record names ${rec.inscription})` : ""}.`,
          `wrapDeposit is custodian-only: this was signed with the canister's EVM key. ${at}`, pause, txl(l)].join("\n") });
    } else if (name === "Delivered" && fromWrapper) {
      const ticket = uint(l.topics[1]).toString(), txid = l.data.slice(2, 66);
      const rec = index.deliveries.get(ticket);
      if (rec && (rec.txids ?? [rec.txid]).includes(txid)) continue;
      raise(l, { kind: "evm-mint", sev: "SEV1", persistent: true, key: `evm:${k}`,
        text: [`A DELIVERY RECORD WITH NO DELIVERY: the wrapper ${from} recorded ticket ${ticket} as delivered in Bitcoin tx ${txid}, and the canister's redemption for that ticket ${rec ? `names ${rec.txid} (${rec.phase})` : "does not exist"}.`,
          `recordDelivery is the custodian's alone since the B-03 fix: this was signed with the canister's EVM key (a false record releases the item's accounting). ${at}`,
          pause, "The ticket's holder is owed an item.", txl(l)].join("\n") });
    } else if (name === "Undeliverable" && fromWrapper) {
      // markUndeliverable is the custodian's alone: it lets a resolved ticket refund at deliverBy + 72 h instead of + 30 days. The
      // canister attests only a redemption it refused (nothing of its own can confirm any more); anything else is its EVM key
      // speaking without it, and a delivery that then confirms is a LostRace (one unit unbacked)
      const ticket = uint(l.topics[1]).toString();
      const a = index.attests?.get(ticket);
      if (a && a.refused && ["Sent", "Done"].includes(a.state)) continue;
      raise(l, { kind: "evm-mint", sev: "SEV1", persistent: true, key: `evm:${k}`,
        text: [`AN ATTESTATION NO REFUSAL BACKS: the wrapper ${from} was told ticket ${ticket}'s delivery can no longer confirm (markUndeliverable), and the canister's redemption for that ticket ${a ? `is ${a.refused ? "refused" : "NOT refused"} with its attestation ${a.state}` : "does not exist"}.`,
          `markUndeliverable is custodian-only: this was signed with the canister's EVM key. The ticket may now be refunded at deliverBy + 72 h while its frog can still be delivered (a LostRace). ${at}`,
          pause, txl(l)].join("\n") });
    } else if (name === "ItemArrived" && fromModule) {
      const cost = uint(word(l.data, 0)).toString();
      if (next.arrivals[k]) continue;
      const used = new Set(Object.values(next.arrivals));
      const frog = index.floorFrogs.filter((f) => f.module === from && f.cost === cost && !used.has(f.id))
        .sort((a, b) => Number(a.id) - Number(b.id))[0];
      if (frog) { next.arrivals[k] = frog.id; continue; }
      raise(l, { kind: "evm-mint", sev: "SEV1", persistent: true, key: `evm:${k}`,
        text: [`AN ARRIVAL NO FROG BACKS: the FloorModule ${from} was told an item arrived at cost ${eth(uint(word(l.data, 0)))}, and no minted frog purchase for this module at that cost is left unclaimed in the canister's records.`,
          `itemArrived is custodian-only: this was signed with the canister's EVM key. ${at}`, pause, txl(l)].join("\n") });
    } else if ((name === "Advanced" || name === "AdvanceSettled" || name === "AdvanceReturned") && fromModule) {
      // docs/v6/relay-conversion.md §6: each advance event is the canister's EVM key speaking; a conversion record must back it
      if (next.advances[k]) continue;
      const amount = uint(word(l.data, name === "AdvanceReturned" ? 1 : 0));        // Advanced: amount; Settled: advanced; Returned: basis (matched below by phase)
      const phases = { Advanced: ["Advancing", "Funded", "Depositing", "Converting", "Arrived", "Returning", "Returned", "Settled", "Stuck"],
        AdvanceSettled: ["Arrived", "Settled"], AdvanceReturned: ["Returning", "Returned"] }[name];
      const used = new Set(Object.values(next.advances));                                // one event of each kind per conversion
      const rec = (index.advances ?? []).filter((c) => c.module === from && phases.includes(c.phase) && !used.has(`${name}|${c.frog}`)
        && (name === "AdvanceReturned" || c.advanced === amount.toString())).sort((a, b) => Number(a.frog) - Number(b.frog))[0];
      if (rec) { next.advances[k] = `${name}|${rec.frog}`; continue; }
      const what = name === "Advanced" ? `AN ADVANCE NO CONVERSION BACKS: the FloorModule ${from} paid an advance of ${eth(amount)} to the canister's EVM address, and no Relay conversion for this module with that advance is in the canister's records.`
        : name === "AdvanceSettled" ? `A SETTLEMENT NO ARRIVAL BACKS: the FloorModule ${from} was told the frog for an advance of ${eth(amount)} arrived (cost ${eth(uint(word(l.data, 1)))}), and no conversion for this module with that advance has arrived or settled in the canister's records.`
        : `A RETURN NO FAILED CONVERSION BACKS: the FloorModule ${from} took an advance back (${eth(uint(word(l.data, 0)))} returned against a basis of ${eth(amount)}, loss ${eth(uint(word(l.data, 2)))}), and no conversion for this module is returning in the canister's records.`;
      raise(l, { kind: "evm-mint", sev: "SEV1", persistent: true, key: `evm:${k}`,
        text: [what, `${name === "Advanced" ? "advance" : name === "AdvanceSettled" ? "settleAdvance" : "returnAdvance"} is custodian-only: this was signed with the canister's EVM key. ${at}`, pause, txl(l)].join("\n") });
    } else if ((name === "Wrapped" || name === "RentalReturned") && fromWrapper) {
      raise(l, { kind: "evm-mint", sev: "SEV2", persistent: true, key: `evm:${k}`,
        text: `the wrapper ${from} emitted ${name}: a path that is OFF in Phase 1 (docs/v6/phase1-scope.md §1) was used. ${at}\nAssess within an hour (G10 SEV2). ${txl(l)}` });
    } else if (name === "MintLimitProposed" && fromWrapper) {
      const limit = uint(word(l.data, 0)), readyAt = Number(uint(word(l.data, 1)));
      raise(l, { kind: "evm-mint", sev: "SEV2", key: `evm:${k}`,
        text: `the Safe proposed raising the wrapper ${from}'s mint limit (H11) to ${limit} a day, ready at ${new Date(readyAt * 1000).toISOString()}. A loosening: expected only with an announcement. ${at} ${txl(l)}` });
    } else if (name === "MintLimitSet" && fromWrapper) {
      alerts.push({ kind: "governance", sev: "INFO", key: `evm:${k}`, text: `the wrapper ${from}'s mint limit is now ${uint(word(l.data, 0))} a day. ${at}` });
    }
  }
  const seen = new Set(logs.map(keyOf));
  for (const [k, a] of Object.entries(next.open)) {
    if (ack.includes(k.split(":")[0].toLowerCase())) delete next.open[k];
    else if (!seen.has(k)) alerts.push(a);   // still open from an earlier range: repeat until acknowledged
  }
  return { alerts, next };
}


// ---- the keeper's words on chain (wave 2, §1.1) ---------------------------------------------------------------------------------

export const CLAMP_BPS = 50n;                    // a post whose target differs from what the module accepted by more: out of plan
export const RUNAWAY_MS = 6 * 3600 * 1000;       // two clamped posts, same direction, this close: a runaway floor
export const CAP_MAX_RATIO_BPS = 12_500n;        // a RAFFLE/BURN cap above 125% of the accepted floor (the keeper posts 110%)
// R2-M3 (fixes-R3-services.md §2): a floor walked along the band's edge, where every post is accepted as it was asked
export const EDGE_FRAC_BPS = 8_000n;             // a move of at least 80% of what floorBounds() allowed is an edge post
export const EDGE_RUN = 3;                       // this many edge posts in a row, one direction: SEV1
export const DAY_MOVE_SEV2_BPS = 1_000n;         // the accepted floor this far from its value a day ago: SEV2
export const DAY_MOVE_SEV1_BPS = 1_800n;         // … and this far (the module allows 20 % a day): SEV1
export const MAX_MOVE_BPS_DEFAULT = 500n;        // FloorModule.maxMoveBps as deployed (5 %/h), when it cannot be read
// R3-check L-C (fixes-R4-services.md §6): rules a no-move post between edge posts cannot reset
export const WALK_IGNORE_FRAC_BPS = 1_000n;      // a post moving less than 10 % of the hourly band neither counts nor resets the edge run
export const WALK_WINDOW_MS = 6 * 3_600_000;     // the net move over this window …
export const WALK_WINDOW_EDGES = 3n;             // … of at least this many hours at the edge (3 × 80 % × 5 % = 12 %): SEV1, as an edge run
export const THREE_DAY_SEV2_BPS = 1_500n;        // the accepted floor this far from its value 72 h ago: SEV2
export const THREE_DAY_SEV1_BPS = 2_500n;        // … and this far: SEV1 (the module allows ~49 % in three days)
export const HIST_MAX = 400;                     // accepted floors kept per module (74 h), the densest thinned first
const HOUR_MS = 3_600_000, DAY_MS = 24 * HOUR_MS, HIST_MS = 74 * HOUR_MS;
const hours = (ms) => `${(ms / 3_600_000).toFixed(1)} h`;
const ethOf = (wei) => `${(Number(BigInt(wei) / 10n ** 12n) / 1e6).toString()} ETH`;
const pct = (bps) => `${(Number(bps) / 100).toFixed(1)}%`;

/**
 * The floor-walk rules for one accepted post (R2-M3, R3-check L-C). Mutates `hist` and `streaks`; returns { alerts, triggers }.
 *  - edge run: EDGE_RUN posts in a row, one direction, each at least 80 % of what floorBounds() allowed; a post that moves less
 *    than 10 % of the hourly band is ignored (a no-move post one second later no longer resets the run);
 *  - net move over 6 h of at least 3 hours at the edge (12 % at 5 %/h), whatever the pattern of posts: SEV1;
 *  - day move (10 % SEV2, 18 % SEV1) and three days (15 % SEV2, 25 % SEV1).
 * Every SEV1 carries the pause trigger when `pauseOnWalk` (evm.guardian.pause_on_floor_walk).
 */
function floorWalk({ module: m, slug, accepted, t, k, hist, streaks, maxMoveBps, registry, pauseOnWalk, at, poster = null }) {
  const alerts = [], triggers = [];
  // an Ethereum NFT collection's floor is its own PosterSource's to stop (audit 2026-09-27-nft D-M-1), never the whole registry
  const walked = (key, why) => { if (pauseOnWalk) triggers.push(poster ? { key, action: "pause-poster", poster, why } : { key, action: "pause", why }); };
  const act = pauseOnWalk ? "The guardian pauses the keeper (evm.guardian.pause_on_floor_walk)." : "The guardian does not act on this by itself (evm.guardian.pause_on_floor_walk is off).";
  const h = (hist[m] ??= []);
  const prev = h.at(-1);
  if (prev && BigInt(prev.accepted) > 0n) {
    const p = BigInt(prev.accepted), move = accepted > p ? accepted - p : p - accepted, dir = accepted > p ? "up" : accepted < p ? "down" : null;
    const el = BigInt(Math.max(0, Math.min(HOUR_MS, t - prev.at)));
    const allowed = (p * maxMoveBps * el) / (10_000n * BigInt(HOUR_MS));
    const tiny = move * 10_000n * 10_000n < p * maxMoveBps * WALK_IGNORE_FRAC_BPS;
    const edge = dir !== null && allowed > 0n && move * 10_000n >= allowed * EDGE_FRAC_BPS;
    const st = streaks[m];
    if (!tiny) streaks[m] = edge ? (st && st.dir === dir ? { ...st, n: st.n + 1, last: k } : { dir, n: 1, first: k, from: prev.accepted, since: prev.at }) : null;
    const s = streaks[m];
    if (!tiny && s && s.n >= EDGE_RUN) {
      const total = (BigInt(s.from) > accepted ? BigInt(s.from) - accepted : accepted - BigInt(s.from)) * 10_000n / BigInt(s.from);
      alerts.push({ kind: "keeper", sev: "SEV1", persistent: true, key: `floor-walk:${m}:${s.first}`,
        text: [`THE ${slug} FLOOR IS BEING WALKED ${s.dir.toUpperCase()} AT THE BAND'S EDGE: ${s.n} posts in a row each moved it at least ${pct(EDGE_FRAC_BPS * maxMoveBps / 10_000n)} an hour (${pct(total)} since ${new Date(s.since).toISOString()}), to ${ethOf(accepted)}.`,
          "Every post was accepted as asked, so nothing was clamped: a thief who reads floorBounds() walks the floor this way, and so does an honest keeper in a real crash. Compare the keeper's /status and the marketplaces now.",
          `${act} By hand: ${byHand.pause(registry)}. ${at}`].join("\n") });
      walked(`floor-walk:${m}:${s.first}`, `the ${slug} floor walked ${s.dir} at the band's edge for ${s.n} posts`);
    }
  }
  h.push({ at: t, accepted: accepted.toString() });
  while (h.length && t - h[0].at > HIST_MS) h.shift();
  while (h.length > HIST_MAX) { const i = h.findIndex((x, j) => j > 0 && x.at - h[j - 1].at < 600_000); h.splice(i > 0 ? i : 1, 1); }
  const moved = (ref) => { const r = BigInt(ref.accepted); return { r, d: (accepted > r ? accepted - r : r - accepted) * 10_000n / r, up: accepted > r }; };
  // the net move over the last 6 h, against the oldest accepted floor inside that window
  const w6 = h.find((x) => t - x.at <= WALK_WINDOW_MS && t - x.at >= HOUR_MS);
  const lim6 = WALK_WINDOW_EDGES * maxMoveBps * EDGE_FRAC_BPS / 10_000n;
  if (w6 && BigInt(w6.accepted) > 0n) {
    const { r, d, up } = moved(w6);
    if (d >= lim6) {
      const key = `floor-walk6:${m}:${Math.floor(t / WALK_WINDOW_MS)}`;
      alerts.push({ kind: "keeper", sev: "SEV1", persistent: true, key,
        text: [`THE ${slug} FLOOR MOVED ${pct(d)} ${up ? "UP" : "DOWN"} IN ${hours(t - w6.at)} (${ethOf(r)} → ${ethOf(accepted)}): at least ${WALK_WINDOW_EDGES} hours at the band's edge, whatever the pattern of posts (R3-check L-C).`,
          `A thief walking the floor, or an honest keeper in a real crash: compare the keeper's /status and the marketplaces now. ${act} By hand: ${byHand.pause(registry)}. ${at}`].join("\n") });
      walked(key, `the ${slug} floor moved ${pct(d)} in ${hours(t - w6.at)}`);
    }
  }
  // the day's move: against the most recent accepted floor at least 23 h old
  const ref = [...h].reverse().find((x) => t - x.at >= DAY_MS - HOUR_MS) ?? null;
  if (ref && BigInt(ref.accepted) > 0n) {
    const { r, d, up } = moved(ref);
    const sev = d >= DAY_MOVE_SEV1_BPS ? "SEV1" : d >= DAY_MOVE_SEV2_BPS ? "SEV2" : null;
    const key = `floor-day-move:${m}:${sev}:${Math.floor(t / DAY_MS)}`;
    if (sev) alerts.push({ kind: "keeper", sev, persistent: sev === "SEV1", key,
      text: `the ${slug} accepted floor moved ${pct(d)} ${up ? "UP" : "DOWN"} in ${hours(t - ref.at)} (${ethOf(r)} → ${ethOf(accepted)}); the module allows at most 20 % a day. A real market move, or a stolen key walking the band: compare with the marketplaces.${sev === "SEV1" ? ` ${act}` : ""} By hand: ${byHand.pause(registry)}. ${at}` });
    if (sev === "SEV1") walked(key, `the ${slug} floor moved ${pct(d)} in a day`);
  }
  // three days: against the most recent accepted floor at least 71 h old (a slow walk under the day rule, R3-check L-C)
  const ref3 = [...h].reverse().find((x) => t - x.at >= 3 * DAY_MS - HOUR_MS) ?? null;
  if (ref3 && BigInt(ref3.accepted) > 0n) {
    const { r, d, up } = moved(ref3);
    const sev = d >= THREE_DAY_SEV1_BPS ? "SEV1" : d >= THREE_DAY_SEV2_BPS ? "SEV2" : null;
    const key = `floor-3day-move:${m}:${sev}:${Math.floor(t / DAY_MS)}`;
    if (sev) alerts.push({ kind: "keeper", sev, persistent: sev === "SEV1", key,
      text: `the ${slug} accepted floor moved ${pct(d)} ${up ? "UP" : "DOWN"} in ${hours(t - ref3.at)} (${ethOf(r)} → ${ethOf(accepted)}): a slow walk under the day's rule, or a real trend. Compare with the marketplaces.${sev === "SEV1" ? ` ${act}` : ""} By hand: ${byHand.pause(registry)}. ${at}` });
    if (sev === "SEV1") walked(key, `the ${slug} floor moved ${pct(d)} in three days`);
  }
  return { alerts, triggers };
}

/**
 * The stack's own events (registry, modules, vaults), judged. Pure. `ctx`: { st (stackOf), minConf: { module: bigint },
 * maxMove: { module: bigint }, accepted: { module: bigint }, times: { logKey: ms }, now, explorer, pauseOnFloorWalk }.
 * Returns { alerts, next: { floorPosts, draws, floorHist, floorStreaks }, triggers: [{ key, action: "pause" | "cancel", vault?, id?, why }] }.
 */
export function judgeStackLogs({ logs, ctx, prev = {} }) {
  const { st, now } = ctx, explorer = ctx.explorer ?? "https://explorer.mainnet.chain.robinhood.com";
  const alerts = [], triggers = [];
  const next = { floorPosts: { ...(prev.floorPosts ?? {}) }, draws: { ...(prev.draws ?? {}) },
    floorHist: Object.fromEntries(Object.entries(prev.floorHist ?? {}).map(([k, v]) => [k, [...v]])), floorStreaks: { ...(prev.floorStreaks ?? {}) } };
  const fixed = new Map();                      // tx → snapshotAt, from SnapshotFixed (emitted just before DrawOpened)
  for (const l of logs) { const e = decodeStackEvent(l); if (e?.eventName === "SnapshotFixed") fixed.set(`${lc(l.address)}:${e.args.drawId}`, e.args.snapshotAt); }
  for (const l of [...logs].sort((x, y) => Number(x.blockNumber) - Number(y.blockNumber) || Number(x.logIndex) - Number(y.logIndex))) {
    if (l.removed) continue;
    const e = decodeStackEvent(l);
    if (!e) continue;
    const k = keyOf(l), from = lc(l.address), { role, c } = roleOf(st, from), a = e.args;
    const at = `block ${Number(l.blockNumber)}, tx ${l.transactionHash}\n${explorer}/tx/${l.transactionHash}`;
    if (e.eventName === "FloorPosted" && role === "module") {
      const min = ctx.minConf?.[from];
      if (min !== undefined && BigInt(a.confidenceBps) < min) alerts.push({ kind: "keeper", sev: "SEV2", key: `floor-lowconf:${k}`,
        text: `the keeper posted the ${c.slug} floor with confidence ${a.confidenceBps} bps, below the module's minimum ${min}: the module is NOT fresh after it (mint, fills and sweeps refused). A thief can freeze the module this way (E-02a); the stale fallback ends it after 7 days. ${at}` });
      const t = ctx.times?.[k] ?? now;
      const diff = a.target > a.accepted ? a.target - a.accepted : a.accepted - a.target;
      if (a.accepted > 0n && diff * 10_000n > a.accepted * CLAMP_BPS) {
        const dir = a.target > a.accepted ? "up" : "down";
        // audit 2026-09-27-nft D-H-2: an Ethereum NFT module's poster is the canister, which since fix round 1 posts inside
        // floorBounds() read at its pin (the bounds only widen until its transaction lands). The premise of this rule (fixes-W2-monitor
        // "Left open" 2: the poster plans inside the bounds) holds again, so a clamped post is still out of plan; but it is the
        // canister's key, and a runaway stops that collection's PosterSource, never the registry every collection shares
        const poster = c?.posterSource ?? null;
        const who = poster ? "THE CANISTER (THIS COLLECTION'S POSTER)" : "THE KEEPER";
        const plan = poster
          ? "The canister clamps its reading into floorBounds() before it posts, so this post is not one it would make: its key used by something else, or a broken build."
          : "An honest keeper plans inside floorBounds() (±5 %/h, ±20 % a day), so this post is not one its plan could make: a stolen key walking the floor, or a broken feed.";
        const stop = poster ? `the guardian pauses this collection's poster. By hand: ${byHand.pausePoster(poster)}` : `the guardian pauses the keeper. By hand: ${byHand.pause(st.registry)}`;
        alerts.push({ kind: "keeper", sev: "SEV1", persistent: true, key: `floor-clamped:${k}`,
          text: [`${who} POSTED A ${c.slug} FLOOR OUTSIDE THE MODULE'S BOUNDS: target ${ethOf(a.target)}, accepted ${ethOf(a.accepted)} (moved ${a.moveBps} bps, ${dir}).`,
            plan, `If a second one follows in the same direction ${stop}. ${at}`].join("\n") });
        next.floorPosts[k] = { at: t, dir, module: from };
        const same = Object.entries(next.floorPosts).filter(([, p]) => p.module === from && p.dir === dir && Math.abs(t - p.at) <= RUNAWAY_MS);
        const why = `a runaway ${c.slug} floor: ${same.length} posts outside the module's bounds, all ${dir}, within ${hours(RUNAWAY_MS)}`;
        if (same.length >= 2) triggers.push(poster ? { key: `runaway:${from}:${same.map(([x]) => x).sort()[0]}`, action: "pause-poster", poster, why }
          : { key: `runaway:${from}:${same.map(([x]) => x).sort()[0]}`, action: "pause", why });
      }
      if (a.accepted > 0n && (min === undefined || BigInt(a.confidenceBps) >= min)) {
        const w = floorWalk({ module: from, slug: c.slug, accepted: a.accepted, t, k, hist: next.floorHist, streaks: next.floorStreaks,
          maxMoveBps: ctx.maxMove?.[from] ?? MAX_MOVE_BPS_DEFAULT, registry: st.registry, pauseOnWalk: !!ctx.pauseOnFloorWalk, at, poster: c?.posterSource ?? null });
        alerts.push(...w.alerts);
        triggers.push(...w.triggers);
      }
    } else if (e.eventName === "CapPosted" && ["raffle", "burn", "yoyo"].includes(role)) {
      if (role === "yoyo") alerts.push({ kind: "keeper", sev: "SEV1", persistent: true, key: `cap-yoyo:${k}`,
        text: `THE KEEPER POSTED A CAP TO THE ${c.slug} YOYO VAULT (${ethOf(a.weiPerWrapped)}): YOYO is OFF in Phase 1 and the keeper service never posts it. The key is being used by something else: pause the keeper (${byHand.pause(st.registry)}). ${at}` });
      else {
        const acc = ctx.accepted?.[c.module];
        if (acc && acc > 0n && a.weiPerWrapped * 10_000n > acc * CAP_MAX_RATIO_BPS) alerts.push({ kind: "keeper", sev: "SEV2", key: `cap-high:${k}`,
          text: `the keeper posted a ${c.slug} ${role.toUpperCase()} cap of ${ethOf(a.weiPerWrapped)}, ${(Number((a.weiPerWrapped * 10_000n) / acc) / 100).toFixed(1)}% of the accepted floor ${ethOf(acc)} (the service posts 110%). Sweeps may pay up to the cap. ${at}` });
      }
    } else if (e.eventName === "DrawAnnounced" && ["raffle", "burn", "yoyo"].includes(role)) {
      const id = `${from}:${a.drawId}`;
      next.draws[id] = { ...(next.draws[id] ?? {}), vault: from, id: String(a.drawId), coin: lc(a.coin), slug: c.slug, seedRound: String(a.seedRound), announcedSeenAt: now, status: next.draws[id]?.status ?? "announced" };
      alerts.push({ kind: "draw", sev: "INFO", key: `draw-announced:${id}`, text: `the keeper announced ${c.slug} draw ${a.drawId} for coin ${lc(a.coin)} (seed round ${a.seedRound}); it must be opened within 6 h. ${at}` });
    } else if (e.eventName === "DrawOpened" && ["raffle", "burn", "yoyo"].includes(role)) {
      const id = `${from}:${a.drawId}`;
      const old = next.draws[id] ?? {};
      if (!TRACKED.includes(old.status)) next.draws[id] = { ...old, vault: from, id: String(a.drawId), coin: lc(a.coin), slug: c.slug, root: a.root,
        totalTickets: String(a.totalTickets), uri: a.uri, challengeEnds: Number(a.challengeEnds), snapshotAt: fixed.has(id) ? String(fixed.get(id)) : null, status: "pending", tries: 0, confirmed: true };
      else if (!old.confirmed && lc(old.root) === lc(a.root) && String(old.totalTickets) === String(a.totalTickets)) next.draws[id] = { ...old, confirmed: true };
      alerts.push({ kind: "draw", sev: "INFO", key: `draw-opened:${id}`, text: `the keeper opened ${c.slug} draw ${a.drawId}: root ${a.root}, ${a.totalTickets} tickets, file ${a.uri}; the challenge window ends ${new Date(Number(a.challengeEnds) * 1000).toISOString()}. The monitor is recomputing the root from its holder indexes. ${at}` });
      if (BigInt(a.totalTickets) === 0n) alerts.push(emptyOpenedAlert(c.slug, from, a.drawId, fixed.get(id), a.challengeEnds, at));
    } else if (e.eventName === "SeedCarried" && ["raffle", "burn", "yoyo"].includes(role)) {
      // review2 N-2: the coin's last draw was voided or cancelled before its pin; this one reuses its seed round and snapshot time
      const id = `${from}:${a.drawId}`;
      next.draws[id] = { ...(next.draws[id] ?? {}), vault: from, id: String(a.drawId), origin: String(a.originId), slug: c.slug, status: next.draws[id]?.status ?? "announced" };
      alerts.push(seedCarriedAlert(c.slug, from, a.drawId, a.originId, a.seedRound, at));
    } else if (e.eventName === "DrawEmpty") {
      // fixes-R3-carry: closed with no winner; the carry (if any) is consumed and the unit is back with the coin
      const id = `${from}:${a.drawId}`, old = next.draws[id];
      const unverified = old && DECIDING.includes(old.status);
      if (old) next.draws[id] = { ...old, status: old.status === "bad" ? "bad-closed-empty" : unverified ? "closed-empty-unverified" : "closed-empty" };
      alerts.push(drawEmptyAlert(c?.slug ?? from, from, a.drawId, a.snapshotAt, old?.status, at));
    } else if (e.eventName === "DrawVoided") {
      const id = `${from}:${a.drawId}`, old = next.draws[id];
      if (old?.status === "announced") alerts.push(voidedAnnouncedAlert(c?.slug ?? from, from, a.drawId, at));
      if (old) next.draws[id] = { ...old, status: old.status === "announced" ? "voided-announced" : "voided" };
    } else if (e.eventName === "DrawCancelled") {
      const id = `${from}:${a.drawId}`;
      if (next.draws[id]) next.draws[id] = { ...next.draws[id], status: next.draws[id].status === "bad" ? "bad-cancelled" : "cancelled" };
      alerts.push({ kind: "draw", sev: "SEV2", key: `draw-cancelled:${id}`, text: `${c?.slug ?? from} draw ${a.drawId} was CANCELLED (its unit returns to its coin). ${at}` });
    } else if (e.eventName === "KeeperPaused" && role === "registry") {
      alerts.push(a.paused
        ? { kind: "keeper", sev: "SEV1", key: `keeper-paused:${k}`, text: `THE KEEPER IS PAUSED, by ${lc(a.by)}${ctx.guardianAddress && lc(a.by) === ctx.guardianAddress ? " (this monitor's guardian)" : ""}: every keeper power everywhere stops (floor, caps, draws); mint, fills and sweeps are refused; burns continue. The Safe alone unpauses. Runbook: "Keeper key stolen" (docs/v5/launch-gates.md §4). ${at}` }
        : { kind: "keeper", sev: "SEV2", key: `keeper-unpaused:${k}`, text: `the keeper was UNPAUSED by ${lc(a.by)} (the Safe). The modules stay stale until the keeper's first post (mints and floor-priced burns wait for it); unfilled curves' refunds stay held for 6 h (RESUME_GRACE, from keeperResumedAt) so the keeper can post and the runner fill first. ${at}` });
    } else if (e.eventName === "GuardianSet" && role === "registry") {
      alerts.push({ kind: "keeper", sev: "SEV2", key: `guardian-set:${k}`, text: `the Safe set the guardian to ${lc(a.guardian)}${lc(a.guardian) === "0x0000000000000000000000000000000000000000" ? " (NONE: only the Safe can pause the keeper and cancel draws now)" : ""}. Who can pause the keeper changed. ${at}` });
    } else if (e.eventName === "RolesProposed" && role === "registry") {
      alerts.push({ kind: "keeper", sev: "SEV1", persistent: true, key: `roles-proposed:${k}`,
        text: `A ROLE ROTATION WAS PROPOSED: keeper ${lc(a.keeper)}, purchaser ${lc(a.purchaser)}, attestor ${lc(a.attestor)}, ready ${new Date(Number(a.readyAt) * 1000).toISOString()} (7 days, both clocks). Expected only in a keeper-key incident; otherwise the Safe's signers are compromised. ${at}` });
    } else if (e.eventName === "RolesSet" && role === "registry") {
      alerts.push({ kind: "keeper", sev: "SEV2", key: `roles-set:${k}`, text: `the roles changed: keeper ${lc(a.keeper)}, purchaser ${lc(a.purchaser)}, attestor ${lc(a.attestor)}. ${at}` });
    }
  }
  for (const [k, p] of Object.entries(next.floorPosts)) if (now - p.at > 2 * RUNAWAY_MS) delete next.floorPosts[k];
  return { alerts, next, triggers };
}

/** review2 N-2 pages: a draw that carries an earlier draw's seed, and an Announced draw voided (the keeper did not open it). */
export const seedCarriedAlert = (slug, vault, id, origin, round, at = "") => ({ kind: "draw", sev: "SEV2", key: `seed-carried:${lc(vault)}:${id}`,
  text: `${slug} draw ${id} CARRIES draw ${origin}'s seed${round !== undefined ? ` round ${round}` : ""} and snapshot time (review2 N-2): draw ${origin} was voided or cancelled before its pin (the keeper did not open it, or the Safe or guardian cancelled it). The snapshot time is draw ${origin}'s, fixed then: a re-announcement cannot re-roll it. Its root is recomputed at that time once it opens. ${at}`.trim() });
export const voidedAnnouncedAlert = (slug, vault, id, at = "") => ({ kind: "draw", sev: "SEV2", key: `draw-voided-announced:${lc(vault)}:${id}`,
  text: `${slug} draw ${id} was VOIDED while still Announced: the keeper did not open it within 6 h (its seed round never served, no snapshot it could build, or the keeper down; an empty snapshot is opened empty and closes with no winner, fixes-R3-carry). The void carries its seed round and snapshot time to the coin's next draw (review2 N-2), so declining to open is only a delay. Check the keeper's /status and alerts (draw-not-opened, drand, snapshot, publish). ${at}`.trim() });

/** fixes-R3-carry pages: the keeper opened a draw with no tickets (it says nobody eligible held a whole coin at the snapshot), and
 *  a draw closed with no winner. An empty open is recomputed like any list: a false empty is a `bad` root (pause, then cancel). */
export const emptyOpenedAlert = (slug, vault, id, snapshotAt, challengeEnds, at = "") => ({ kind: "draw", sev: "SEV2", key: `draw-empty-opened:${lc(vault)}:${id}`,
  text: `${slug} draw ${id} was opened EMPTY: the keeper says NO eligible holder had a whole coin at the snapshot time${snapshotAt !== undefined ? ` ${snapshotAt}` : ""} (the tree is the sentinel alone). After the challenge window${challengeEnds !== undefined ? ` (${new Date(Number(challengeEnds) * 1000).toISOString()})` : ""} anyone's requestDraw closes it with NO WINNER, the unit goes back to the coin and a carried seed is consumed (fixes-R3-carry). The monitor recomputes it like any list: if it finds holders, that is a false empty, and the answer is the bad-root one (pause the keeper, then cancel: the cancel carries the same snapshot time). ${at}`.trim() });
export const drawEmptyAlert = (slug, vault, id, snapshotAt, status, at = "") => {
  const trusted = status === "verified" || status === undefined;
  return { kind: "draw", sev: trusted ? "SEV2" : "SEV1", ...(trusted ? {} : { persistent: true }), key: `draw-empty:${lc(vault)}:${id}`,
    text: trusted
      ? `${slug} draw ${id} CLOSED WITH NO WINNER: no eligible holder had a whole coin at its snapshot time ${snapshotAt}${status === "verified" ? " (the monitor reproduced the empty tree)" : ""}. The unit went back to the coin; the coin's next draw pins a new seed (fixes-R3-carry). ${at}`.trim()
      : `${slug} draw ${id} CLOSED EMPTY (no winner) while its empty root was ${status === "bad" ? "a CONFIRMED BAD ROOT (holders existed at the snapshot): the pause and cancel did not land in time" : "NOT VERIFIED"}. The snapshot time ${snapshotAt} is used up and the coin's next draw gets a new seed; nobody lost a unit, but ${status === "bad" ? "the keeper threw a snapshot time away: treat it as the \"bad draw\" incident of the runbook" : "recompute it by hand"}: cd tools/snapshot && npm run snapshot -- --vault ${vault} --draw-id ${id} --stack deployments/stack-4663.json --slug ${slug} --exclusions exclusions.json --rpc <an RPC of your own>. ${at}`.trim() };
};

/** A draw the monitor is still deciding. */
const TRACKED = ["pending", "verified", "bad", "disagree"];
const DECIDING = ["pending", "disagree"];

/** The by-hand recomputation with the snapshot tool (tools/snapshot v2; exit 3 on a mismatch). */
export const byHandRoot = (d) => `cd tools/snapshot && npm run snapshot -- --vault ${d.vault} --draw-id ${d.id} --stack deployments/stack-4663.json --slug ${d.slug} --exclusions exclusions.json --rpc <an RPC of your own>`;

/**
 * One opened draw's verdict (draws.mjs `verdictN`) as alerts, guardian triggers and its new record. Pure. `rs`: each provider's
 * result, in order. A confirmed bad root pauses the keeper FIRST, then cancels (R2-L3): a paused keeper cannot have the draw
 * pinned (requestDraw reverts while paused), so the pause alone already holds it. Triggers need the draw's contents confirmed
 * on both RPCs (`d.confirmed`); otherwise the page says so and nobody acts automatically.
 */
export function drawOutcome(id, d, v, rs, st) {
  const where = `${d.slug} draw ${d.id} (vault ${d.vault}, coin ${d.coin})`;
  const ok = (Array.isArray(rs) ? rs : [rs]).filter((r) => r && !r.error);
  if (v.v === "verified") {
    const r = ok[0], n = v.matches ?? ok.length;
    const after = d.deadlineSince ? [{ kind: "draw", sev: "SEV2", key: `draw-verified-late:${id}`, text: `${where}: the root is now reproduced (${n} providers), after the deadline. If the guardian paused the keeper for it, nothing else is wrong with this draw: the Safe may unpause (unpauseKeeper) once the cause of the delay is understood.` }] : [];
    return { draw: { ...d, status: "verified", matches: n }, triggers: [],
      alerts: [{ kind: "draw", sev: "INFO", key: `draw-verified:${id}`, text: `✅ ${where}: the root is reproduced on ${n === 2 ? "both RPCs" : `${n} providers`} (${r.holders} holders, ${r.totalTickets} tickets at block ${r.block}).` }, ...after] };
  }
  if (v.v === "bad") {
    const hand = `${byHand.pause(st.registry)}; then ${byHand.cancel(d.vault, d.id)}`;
    return { draw: { ...d, status: "bad", matches: 0 },
      alerts: [{ kind: "draw", sev: "SEV1", persistent: true, key: `draw-bad:${id}`,
        text: [`A BAD DRAW ROOT: ${where}. ${v.why}.`, "The providers reproduce the same other root: the keeper published a tree that is not the holders'. Pause the keeper (the draw cannot be pinned while it is paused), then cancel the draw.",
          `${hand}.`, ...(d.confirmed ? [] : ["The draw's contents are NOT confirmed on the second RPC yet: the guardian does not act on it by itself. Check the draw by hand."])].join("\n") }],
      triggers: d.confirmed ? [{ key: `bad-root-pause:${id}`, action: "pause", why: `a confirmed bad root on ${where}` },
        { key: `bad-root:${id}`, action: "cancel", vault: d.vault, id: d.id, why: `a confirmed bad root on ${where}` }] : [] };
  }
  return { draw: { ...d, status: v.v === "disagree" ? "disagree" : "pending", matches: v.matches ?? 0, tries: (d.tries ?? 0) + 1, lastError: v.why }, triggers: [],
    alerts: v.v === "disagree" ? [{ kind: "provider-disagreement", sev: "SEV2", persistent: true, key: `draw-disagree:${id}`, text: `${where}: ${v.why}. Not guessing which is right; no automatic action before the deadline. Recompute by hand (exit 3 is a mismatch): ${byHandRoot(d)}` }] : [] };
}

// ---- evidence for the deadline (fixes-R4-services.md §3) ------------------------------------------------------------------------

/**
 * A provider's failure that says nothing about the root: its index is behind, stuck on a dense block, broken or being rebuilt, or
 * the provider is throttled, cooling down, timing out or unreachable. Outsiders can cause some of these (dense blocks, launches)
 * and ordinary flakiness the rest, so none of them may pause the keeper on its own. Anything else is "unexplained".
 */
export const DEGRADED = /behind the snapshot block|not indexed yet|is broken|is stuck|older than the index keeps|cooling|throttl|busy|rate.?limit|too many requests|timeout|timed out|aborted|ECONNRESET|ECONNREFUSED|ENOTFOUND|EAI_AGAIN|socket hang up|fetch failed|answered HTTP|header not found|-3200[05]|-32016|state .*not available|unknown state|time budget/i;
export const errorClass = (e) => (DEGRADED.test(String(e)) ? "degraded" : "unexplained");

/**
 * What each provider said about one draw this poll (`per`: host → match | other | error with its class), and the dissents: a
 * provider computing another root has its index of the coin rebuilt from the launch once for this draw (L-F, `repair`); its
 * dissent is `repaired` once the rebuilt index still computes another root, and dropped if it then reproduces the chain's. Pure but
 * for the repair it asks of `indexes`. Returns { per, dissent }.
 */
export function evidenceOf(id, d, rs, urls, v, { indexes = null, now = Date.now() } = {}) {
  const tag = `draw:${id}`, per = {}, dissent = { ...(d.dissent ?? {}) };
  const same = (r) => lc(r.root) === lc(d.root) && String(r.totalTickets) === String(d.totalTickets);
  urls.forEach((u, i) => {
    const r = rs[i], h = host(u);
    if (!r || r.error) { per[h] = { r: "error", cls: errorClass(r?.error ?? "no answer"), why: String(r?.error ?? "no answer").slice(0, 200) }; return; }
    if (same(r)) { per[h] = { r: "match", block: r.block }; delete dissent[h]; return; }
    const repaired = r.repairTag === tag;
    per[h] = { r: "other", root: r.root, tickets: String(r.totalTickets), block: r.block, repaired };
    dissent[h] = { root: r.root, tickets: String(r.totalTickets), block: r.block, at: dissent[h]?.at ?? now, repaired: repaired || !!dissent[h]?.repaired && dissent[h].root === r.root };
    if (!repaired && v.v !== "bad") indexes?.find((x) => host(x.url) === h)?.repair(d.coin, tag);
  });
  return { per, dissent };
}

/**
 * The deadline's call for one confirmed Open draw not verified (fixes-R4-services.md §3.3). Pure.
 *   single    one provider reproduces it, none dissents: SEV2, no pause (as in round 3)
 *   pause     evidence: no provider reproduces it and a caught-up index computes another root; or every dissent is confirmed after
 *             a rebuild; or a failure no degraded state explains; or (draws.pause_on_degraded) anything unverified
 *   escalate  no evidence (a dissent not yet re-read against a provider that reproduces it; every provider degraded): SEV1, by hand
 */
export function deadlineCall(d, { pauseOnDegraded = false } = {}) {
  const per = Object.entries(d.per ?? {}), dis = Object.entries(d.dissent ?? {});
  const matches = d.matches ?? per.filter(([, x]) => x.r === "match").length;
  const list = (xs) => xs.map(([h, x]) => `${h} ${x.root ? `computes ${x.root} (${x.tickets} tickets)` : x.why}`).join("; ");
  if (matches >= 1) {
    if (!dis.length && d.status !== "disagree") return { act: "single", why: "one provider reproduces it" };
    if (dis.length && dis.every(([, x]) => x.repaired)) return { act: "pause", why: `${matches} provider(s) reproduce the root, but ${list(dis)} after re-reading the coin from its launch: a provider's history disagrees with the root` };
    if (pauseOnDegraded) return { act: "pause", why: `the providers disagree (${list(dis) || d.lastError || "different snapshot blocks"}); draws.pause_on_degraded` };
    return { act: "escalate", why: `${matches} provider(s) reproduce the root; ${dis.length ? `${list(dis)}, its index being re-read from the coin's launch (a dropped log, L-F, until it says otherwise)` : d.lastError ?? "the providers disagree"}` };
  }
  if (dis.length) return { act: "pause", why: `no provider reproduces the root, and ${list(dis)}` };
  const unexplained = per.filter(([, x]) => x.r === "error" && x.cls === "unexplained");
  if (unexplained.length) return { act: "pause", why: `no provider could recompute it, for a reason no degraded state explains: ${list(unexplained)}` };
  const why = per.length ? `every provider is degraded: ${list(per)}` : (d.lastError ?? "no provider could recompute it");
  return pauseOnDegraded ? { act: "pause", why: `${why} (draws.pause_on_degraded)` } : { act: "escalate", why };
}

/** What the guardian did (`r`: { done, tx | skipped | pending }) or why it failed (`err`), as the alert. Pure. */
export function guardianAlert(t, r, err, st) {
  if (t.action === "pause-poster") {
    const hand = byHand.pausePoster(t.poster);
    if (err) return { kind: "guardian", sev: "SEV1", persistent: true, key: `guardian-failed:${t.key}`,
      text: `THE GUARDIAN FAILED to pause the poster ${t.poster}: ${String(err.message).slice(0, 300)}. ACT BY HAND NOW: ${hand}. (${t.why}) It tries again next poll.` };
    if (r.pending) return { kind: "guardian", sev: "SEV1", key: `guardian-sent:${t.key}:${r.tx}`,
      text: `🛡 the guardian SENT pause() to the poster ${t.poster} (tx ${r.tx}) and it is NOT MINED yet: ${t.why}. If it does not land, act by hand: ${hand}.` };
    return { kind: "guardian", sev: "SEV1", key: `guardian-acted:${t.key}`,
      text: r.done ? `🛡 THE GUARDIAN PAUSED ONE COLLECTION'S FLOOR (its PosterSource ${t.poster}): ${t.why}. tx ${r.tx}. Every other collection goes on; the Safe alone unpauses it (unpause()), after the cause is found.`
        : `🛡 the guardian did not pause the poster: ${r.skipped} (${t.why}).` };
  }
  const hand = t.action === "pause" ? byHand.pause(st.registry) : byHand.cancel(t.vault, t.id);
  if (err) return { kind: "guardian", sev: "SEV1", persistent: true, key: `guardian-failed:${t.key}`,
    text: `THE GUARDIAN FAILED to ${t.action}: ${String(err.message).slice(0, 300)}. ACT BY HAND NOW: ${hand}. (${t.why}) It tries again next poll.` };
  if (r.pending) return { kind: "guardian", sev: "SEV1", key: `guardian-sent:${t.key}:${r.tx}`,
    text: `🛡 the guardian SENT ${t.action === "pause" ? "pauseKeeper()" : `cancelDraw(${t.id}) on ${t.vault}`} (tx ${r.tx}) and it is NOT MINED yet: ${t.why}. It is checked again next poll; if it does not land, act by hand: ${hand}.` };
  return { kind: "guardian", sev: "SEV1", key: `guardian-acted:${t.key}`,
    text: r.done ? `🛡 THE GUARDIAN ${t.action === "pause" ? "PAUSED THE KEEPER" : `CANCELLED DRAW ${t.id} on ${t.vault}`}: ${t.why}. tx ${r.tx} (mined, status success). The Safe alone can undo a pause (unpauseKeeper), after the cause is found.`
      : `🛡 the guardian did not ${t.action === "pause" ? "pause" : "cancel"}: ${r.skipped} (${t.why}).` };
}

// ---- the poll -------------------------------------------------------------------------------------------------------------------

export const DISPUTE_LIMIT = 32;
export const STILL_DISAGREE_MS = 3600 * 1000;
export const STALL_MS = 30 * 60 * 1000;
export const VERIFY_BEFORE_S = 10 * 60;           // a draw not verified this long before its window ends: SEV2
export const DEADLINE_S = 6 * 60;                 // … and this long before: SEV1, and the guardian pauses (fixes-R3 §1.3)
export const DRAW_SCAN = 50;                      // draws read from the vaults' state a poll, from the lowest not yet final
export const SCAN_BUDGET_S = 20;                  // the canary's log scan, a poll
export const INDEX_BUDGET_S = 25;                 // the holder indexes' reads, a poll (all providers in parallel)
export const RECONCILE_BUDGET_S = 15;             // L-S4: the indexes' reconciliation, a poll, after the deadline and the guardian
export const TRIGGER_TTL_MS = 24 * 3600 * 1000;   // an unfinished guardian action is retried this long

/** The canary's two providers (`evm.rpcs`), with the page span `draws.providers` gives the same URL (default CHUNK). */
export function canaryProviders(cfg) {
  const spans = new Map((cfg.draws?.providers ?? []).map((x) => (typeof x === "string" ? [x, undefined] : [x.url, x.span])));
  return cfg.rpcs.map((u) => new Provider({ url: u, span: spans.get(u) ?? CHUNK }));
}

/** Both providers' logs for one range, paged for each; `upTo` is the last block BOTH read (below `s` if either could not). */
async function bothLogs(pa, pb, addresses, s, e, http, deadline) {
  const [ra, rb] = await Promise.all([pa, pb].map((p) => pagedLogs(p, { address: addresses }, s, e, { http, deadline })));
  const upTo = Math.min(ra.upTo, rb.upTo);
  const cut = (xs) => xs.filter((l) => Number(BigInt(l.blockNumber)) <= upTo);
  const la = cut(ra.logs), lb = cut(rb.logs);
  const c = compareLogs(la, lb), inB = new Set(lb.filter((l) => !l.removed).map(canon));
  return { c, agreed: la.filter((l) => !l.removed && inB.has(canon(l))), upTo, error: upTo < e ? (ra.upTo < e ? `${pa.host}: ${ra.error}` : `${pb.host}: ${rb.error}`) : null };
}

const disagreeText = (a, b, s, e, c, since, now) => [
  `the 4663 RPCs disagree about the stack's logs in blocks ${s}–${e}${now - since >= STILL_DISAGREE_MS ? ` — STILL, for ${hours(now - since)}` : ""}: ${host(a)} has ${c.onlyA.length} the other lacks, ${host(b)} has ${c.onlyB.length}.`,
  "Not guessing which is right. The logs BOTH return in these blocks are judged; the rest are re-read every poll until they agree, and the canary goes on past them.",
  ...[...c.onlyA.map((x) => `only ${host(a)}: ${x.slice(0, 180)}`), ...c.onlyB.map((x) => `only ${host(b)}: ${x.slice(0, 180)}`)].slice(0, 6)].join("\n");

const ROUTER_OF = [{ type: "function", name: "routerOf", stateMutability: "view", inputs: [{ type: "address" }], outputs: [{ type: "address" }] }];
const sameDraw = (x, y) => lc(x.root) === lc(y.root) && String(x.totalTickets) === String(y.totalTickets) && String(x.challengeEnds) === String(y.challengeEnds) && String(x.snapshotAt) === String(y.snapshotAt);

/**
 * Every RAFFLE vault's draws, read from its STATE (drawCount, draw(i)) on RPC A and confirmed on RPC B with eth_call, so a draw
 * is found even while a provider refuses eth_getLogs (R2-M1: DrawOpened used to be the only way in). From the lowest draw not
 * yet final, at most DRAW_SCAN a vault a poll. Mutates `draws`; returns { scan, alerts, errors }.
 */
async function scanDraws({ st, a, b, http, prevScan = {}, draws, now = Date.now() }) {
  const scan = { ...prevScan }, alerts = [], errors = [], holds = [], ended = [];
  for (const c of st.collections) {
    const v = c.raffle;
    if (!v) continue;
    try {
      const count = Number(await read(a, VAULT, v, "drawCount", [], { http }));
      const from = Math.min(scan[v] ?? 0, count);
      let lowest = null;
      for (let i = from; i < Math.min(count, from + DRAW_SCAN); i++) {
        const dv = await read(a, VAULT, v, "draw", [BigInt(i)], { http });
        const stage = DRAW_STAGE[Number(dv.stage)], id = `${v}:${i}`, old = draws[id];
        if (["Announced", "Open"].includes(stage) && lowest === null) lowest = i;
        // review2 N-2 (integration R3): the seed origin of every draw before its pin, read on chain; its window must stay indexed
        if (["Announced", "Open"].includes(stage)) {
          const origin = old?.origin !== undefined ? Number(old.origin) : await read(a, VAULT, v, "seedOrigin", [BigInt(i)], { http }).then(Number, () => i);    // a vault from before R3 has none
          const originAt = origin === i ? Number(dv.announcedAt) : Number((await read(a, VAULT, v, "draw", [BigInt(origin)], { http })).announcedAt);
          holds.push({ coin: lc(dv.coin), originAt, vault: v });
          if (origin !== i) alerts.push(seedCarriedAlert(c.slug, v, i, origin, dv.seedRound));
          if (stage === "Announced") { draws[id] = { ...(old ?? {}), vault: v, id: String(i), coin: lc(dv.coin), slug: c.slug, origin: String(origin), status: old?.status ?? "announced" }; continue; }
          draws[id] = { ...(draws[id] ?? old ?? {}), origin: String(origin) };
        }
        // a draw that ended before its pin (void of an Announced draw, or a cancel): its coin may carry the seed (checked each poll)
        const beforePin = (stage === "Voided" && Number(dv.pinnedAt) === 0 && Number(dv.resolvedAt) === 0) || (stage === "Cancelled" && Number(dv.pinnedAt) === 0);
        if (beforePin) {
          ended.push({ coin: lc(dv.coin), vault: v });
          if (stage === "Voided" && Number(dv.snapshotAt) === 0 && now - Number(dv.announcedAt) * 1000 < 2 * 86_400_000) alerts.push(voidedAnnouncedAlert(c.slug, v, i));
        }
        if (stage === "Open") {
          const rec = { vault: v, id: String(i), coin: lc(dv.coin), slug: c.slug, root: dv.root, totalTickets: String(dv.totalTickets), uri: dv.uri,
            challengeEnds: Number(dv.challengeEnds), snapshotAt: String(dv.snapshotAt) };
          if (rec.totalTickets === "0") alerts.push(emptyOpenedAlert(c.slug, v, i, rec.snapshotAt, rec.challengeEnds));     // fixes-R3-carry
          let d = old && TRACKED.includes(old.status) ? { ...old, snapshotAt: old.snapshotAt ?? rec.snapshotAt } : { ...(old ?? {}), ...rec, status: "pending", tries: 0, confirmed: false };
          if (!sameDraw(d, rec)) d = { ...d, ...rec, status: "pending", tries: 0, confirmed: false };     // A says something else now: decide again
          if (!d.confirmed) {
            const onB = await read(b, VAULT, v, "draw", [BigInt(i)], { http }).catch(() => null);
            if (onB && DRAW_STAGE[Number(onB.stage)] === "Open" && sameDraw(rec, { root: onB.root, totalTickets: onB.totalTickets, challengeEnds: onB.challengeEnds, snapshotAt: onB.snapshotAt })) d.confirmed = true;
          }
          draws[id] = d;
        } else if (old && DECIDING.includes(old.status) && stage !== "Announced") {
          draws[id] = { ...old, status: stage === "Cancelled" ? "cancelled" : `closed-${stage.toLowerCase()}` };
          if (["Pinned", "Resolved", "Claimed"].includes(stage)) alerts.push({ kind: "draw", sev: "SEV1", persistent: true, key: `draw-pinned-unverified:${id}`,
            text: `${c.slug} draw ${i} was PINNED while its root was NOT verified (${old.lastError ?? "still recomputing"}). It can no longer be cancelled. Recompute it by hand (exit 3 is a mismatch): ${byHandRoot(old)}; if it is wrong, this is the "bad draw" incident of the runbook.` });
          if (stage === "Empty") alerts.push(drawEmptyAlert(c.slug, v, i, dv.snapshotAt, old.status));
        }
      }
      scan[v] = lowest ?? Math.min(count, from + DRAW_SCAN);
    } catch (err) { errors.push(`${c.slug} RAFFLE vault ${v}: ${String(err.message).slice(0, 160)}`); }
  }
  return { scan, alerts, errors, holds, ended };
}

/** SNAPSHOT_WINDOW (24 h) and an hour of margin: a carried draw's snapshot is at most this long before its origin's announcement. */
export const HOLD_BEFORE_S = 25 * 3600;

/**
 * The holder indexes' holds (review2 N-2, integration R3): for each coin with a draw before its pin, or whose last draw ended before
 * its pin and left its seed (`carriedSeed`), the first L2 block its snapshot may need. Kept in the state; a hold ends when the
 * coin carries nothing and has no draw before its pin. Returns { holds: { coin: { originAt, block, vault } }, alerts }.
 */
async function drawHolds({ ds, prev = {}, a, st, http, safe }) {
  const holds = {}, alerts = [];
  const want = new Map();
  for (const h of ds.holds) want.set(h.coin, h);
  for (const [coin, h] of Object.entries(prev)) if (!want.has(coin)) want.set(coin, { coin, originAt: h.originAt, vault: h.vault, recheck: true, block: h.block });
  for (const e of ds.ended) if (!want.has(e.coin)) want.set(e.coin, { coin: e.coin, vault: e.vault, recheck: true });
  for (const h of want.values()) {
    try {
      let originAt = h.originAt;
      if (h.recheck) {
        const carried = Number(await read(a, VAULT, h.vault, "carriedSeed", [h.coin], { http }));
        if (carried === 0) continue;                                          // nothing carried: the hold ends
        originAt = Number((await read(a, VAULT, h.vault, "draw", [BigInt(carried - 1)], { http })).announcedAt);
      }
      const old = prev[h.coin];
      const block = old && old.originAt === originAt && Number.isSafeInteger(old.block) ? old.block
        : await blockAtOrBefore(a, originAt - HOLD_BEFORE_S, st.fromBlock, safe, http) ?? st.fromBlock;
      holds[h.coin] = { originAt, block, vault: h.vault };
    } catch (err) {
      if (prev[h.coin]) holds[h.coin] = prev[h.coin];                          // keep what we had; say why
      alerts.push({ kind: "watcher-error", sev: "SEV3", key: `hold-unread:${h.coin}:${Math.floor(Date.now() / 3_600_000)}`, text: `the holder index's hold for coin ${h.coin} could not be read: ${String(err.message).slice(0, 160)}` });
    }
  }
  return { holds, alerts };
}

/**
 * The 4663 canary's poll. cfg: the monitor's `evm` block (with `draws`, `guardian`). deps: { http, readRecords, timelock, guardian,
 * indexes (holders.mjs, one per provider), providers (the canary's two), recompute (tests) }. Returns { alerts, next, degraded }
 * and changes nothing on chain unless the guardian acts on a confirmed trigger.
 */
export async function checkEvm({ cfg, prev = {}, now = Date.now(), http = fetch, readRecords, timelock, guardian = null, recompute = null, indexes = null, providers = null, clock = Date.now, sleeping = null }) {
  const [a, b] = cfg.rpcs;
  const st = stackOf(cfg);
  const settled = await Promise.allSettled([a, b].map(async (u) => Number(BigInt(await rpc(u, "eth_blockNumber", [], http)))));
  const heads = settled.map((r) => (r.status === "fulfilled" ? r.value : null));
  if (heads.every((h) => h === null)) throw new Error(`neither 4663 RPC answers eth_blockNumber (${settled.map((r) => String(r.reason?.message).slice(0, 120)).join("; ")})`);
  const safe = Math.min(...heads.filter((h) => h !== null)) - (cfg.confirmations ?? 20);
  const from = prev.cursor ?? cfg.from_block ?? st.fromBlock ?? safe;
  const to = Math.min(safe, from + MAX_RANGE - 1);
  const addresses = watchedAddresses(st);
  const alerts = [], logs = [], escalated = [];
  let degraded = null;
  const [pa, pb] = providers ?? canaryProviders(cfg);
  const scanUntil = clock() + (cfg.scan_budget_s ?? SCAN_BUDGET_S) * 1000;

  // E-06: a disputed range is kept aside and re-read; what both RPCs return in it is judged at once; the cursor goes on
  const disputed = [];
  for (const d of prev.disputed ?? []) {
    const r = await bothLogs(pa, pb, addresses, d.from, d.to, http, scanUntil);
    logs.push(...r.agreed);
    if (r.c.same && r.upTo >= d.to) { alerts.push({ kind: "provider-disagreement", sev: "INFO", key: `evm-agree:${d.from}-${d.to}:${now}`, text: `the 4663 RPCs now agree about blocks ${d.from}–${d.to} (disputed for ${hours(now - d.since)}); every log in them is judged.` }); continue; }
    disputed.push(d);
    if (!r.c.same) {
      alerts.push({ kind: "provider-disagreement", sev: "SEV2", persistent: true, key: `evm-disagree:${d.from}-${d.to}`, text: disagreeText(a, b, d.from, d.to, r.c, d.since, now) });
      if (now - d.since >= STILL_DISAGREE_MS) alerts.push({ kind: "provider-disagreement", sev: "SEV2", key: `evm-disagree-1h:${d.from}-${d.to}`, text: disagreeText(a, b, d.from, d.to, r.c, d.since, now) });
    }
  }
  // the new blocks, until either provider cannot read further (throttled, refusing, or out of time): the cursor stops there
  let readTo = from - 1, unread = null;
  for (let s = from; s <= to;) {
    const e = Math.min(to, s + CHUNK - 1);
    const r = await bothLogs(pa, pb, addresses, s, e, http, scanUntil);
    if (r.upTo < s) { unread = r.error; break; }
    logs.push(...r.agreed);
    if (!r.c.same) {
      disputed.push({ from: s, to: r.upTo, since: now });
      alerts.push({ kind: "provider-disagreement", sev: "SEV2", persistent: true, key: `evm-disagree:${s}-${r.upTo}`, text: disagreeText(a, b, s, r.upTo, r.c, now, now) });
    }
    readTo = r.upTo; s = r.upTo + 1;
    if (r.error) { unread = r.error; break; }
  }
  const oldest = disputed.reduce((m, d) => Math.min(m, d.since), Infinity);
  if (now - oldest >= STILL_DISAGREE_MS) degraded = `the 4663 RPCs have disagreed about blocks ${disputed[0].from}–${disputed[0].to} for ${hours(now - oldest)}`;

  // a cursor that does not move while the heads do: one RPC is stuck behind, or refuses eth_getLogs (the safe block is the lower head)
  const cursor = Math.max(from, readTo + 1);
  const known = heads.filter((h) => h !== null);
  const moved = cursor > from || Math.max(...known) <= (prev.head ?? 0);
  const stalledSince = moved ? null : (prev.stalledSince ?? now);
  if (stalledSince !== null && now - stalledSince >= STALL_MS) {
    degraded ??= `the 4663 cursor has not moved for ${hours(now - stalledSince)} while the heads did (${heads.map((h, i) => `${host(cfg.rpcs[i])} ${h ?? "unreadable"}`).join(", ")})${unread ? `: ${unread}` : ""}`;
    alerts.push({ kind: "watcher-error", sev: "SEV2", persistent: true, key: `evm-stalled:${stalledSince}`, text: `the 4663 canary is STALLED: ${degraded}. One RPC is not keeping up or refuses eth_getLogs; nothing past block ${from - 1} has been judged by both. Draws are still found from the vaults' state and verified from the holder indexes.` });
  }

  // what the keeper's events need: the modules' minimum confidence and hourly move (immutable), the accepted floor, the posts' times
  const minConf = { ...(prev.minConf ?? {}) }, maxMove = { ...(prev.maxMove ?? {}) }, accepted = {}, times = {};
  for (const l of logs) {
    const ev = decodeStackEvent(l), role = roleOf(st, l.address);
    if (ev?.eventName === "FloorPosted" && role.role === "module") {
      const m = lc(l.address);
      if (minConf[m] === undefined) minConf[m] = String(await read(a, MODULE, m, "minConfidenceBps", [], { http }));
      if (maxMove[m] === undefined) { const mv = await read(a, MODULE, m, "maxMoveBps", [], { http }).catch(() => null); if (mv !== null) maxMove[m] = String(mv); }
      const blk = await rpc(a, "eth_getBlockByNumber", [l.blockNumber, false], http).catch(() => null);
      if (blk?.timestamp) times[keyOf(l)] = Number(BigInt(blk.timestamp)) * 1000;
    }
    if (ev?.eventName === "CapPosted" && role.c?.module && accepted[role.c.module] === undefined)
      accepted[role.c.module] = await read(a, MODULE, role.c.module, "accepted", [], { http }).catch(() => undefined);
  }
  const big = (o) => Object.fromEntries(Object.entries(o).map(([k, v]) => [k, BigInt(v)]));
  const sj = judgeStackLogs({ logs, ctx: { st, now, minConf: big(minConf), maxMove: big(maxMove), accepted, times, explorer: cfg.explorer, guardianAddress: guardian?.address ?? null,
    pauseOnFloorWalk: cfg.guardian?.pause_on_floor_walk === true }, prev: { floorPosts: prev.floorPosts, draws: prev.draws, floorHist: prev.floorHist, floorStreaks: prev.floorStreaks } });
  alerts.push(...sj.alerts);
  const draws = sj.next.draws, triggers = [...sj.triggers];

  // R2-M1: draws from the vaults' state too (both RPCs' eth_call), so no provider's eth_getLogs is needed to find one
  const ds = await scanDraws({ st, a, b, http, prevScan: prev.drawScan, draws, now });
  alerts.push(...ds.alerts);
  const dh = await drawHolds({ ds, prev: prev.holds ?? {}, a, st, http, safe });
  alerts.push(...dh.alerts);

  // R2-M1: the holder indexes, read forward from their saved cursors, the coins of the earliest windows first
  const deciding = Object.entries(draws).filter(([, d]) => DECIDING.includes(d.status)).sort(([, x], [, y]) => (x.challengeEnds ?? Infinity) - (y.challengeEnds ?? Infinity));
  const indexState = [];
  let indexHeads = [];
  if (indexes) {
    for (const d of Object.values(draws)) if (d.coin && TRACKED.concat(["announced"]).includes(d.status)) for (const ix of indexes) ix.addCoin(d.coin);
    const holdBlocks = Object.fromEntries(Object.entries(dh.holds).map(([coin, h]) => [coin, h.block]));
    for (const ix of indexes) { for (const coin of Object.keys(holdBlocks)) ix.addCoin(coin); ix.setHolds(holdBlocks); }
    const until = clock() + (cfg.draws?.index_budget_s ?? INDEX_BUDGET_S) * 1000;
    const res = await Promise.all(indexes.map((ix) => ix.update({ deadline: until, priority: deciding.map(([, d]) => d.coin) }).catch((err) => ({ error: String(err.message) }))));
    indexHeads = res.map((r) => r.head ?? null);
    res.forEach((r, i) => { indexes[i].safe = r.safe ?? indexes[i].safe; indexState.push({ ...indexes[i].summary(), error: r.error ?? null }); indexes[i].save(); });
    // M-B: a coin no way could read at a dense block is left behind there (the rest go on); page it, SEV1 if a deciding draw needs it
    const need = new Set(deciding.map(([, d]) => d.coin));
    for (const s0 of indexState) for (const x of s0.stuck ?? []) alerts.push({ kind: "watcher-error", sev: need.has(x.coin) ? "SEV1" : "SEV2", persistent: true, key: `index-stuck:${s0.provider}:${x.coin}:${x.block}`,
      text: `the holder index of coin ${x.coin} on ${s0.provider} is STUCK at block ${x.block} since ${new Date(x.since).toISOString()}: ${x.error}. Every other coin goes on; this one is retried every poll.${need.has(x.coin) ? " A draw being verified needs this coin: if no other provider reproduces it before its deadline, it is escalated (by hand), not paused." : ""}` });
  }
  const urls = indexes ? indexes.map((ix) => ix.url) : (cfg.draws?.providers ?? cfg.rpcs).map((x) => (typeof x === "string" ? x : x.url));
  const rootOf = recompute ?? (indexes
    ? (url, input) => { const ix = indexes.find((x) => x.url === url); return ix.rootAt({ ...input, head: Math.min(input.head, ix.safe ?? input.head) }); }
    : (url, input, h, opts) => recomputeDraw(url, input, h, opts));
  const quorum = Math.min(cfg.draws?.verify_quorum ?? 2, urls.length);

  // R2-M2: every draw still undecided, earliest window first; one that cannot be decided never holds another's turn
  const listed = cfg.draws?.exclusions ?? [];
  for (const [id, d0] of deciding) {
    let d = d0;
    const c = st.collections.find((x) => [x.raffle, x.burn, x.yoyo].includes(d.vault));
    try {
      if (!d.snapshotAt) d = { ...d, snapshotAt: String((await read(a, VAULT, d.vault, "draw", [BigInt(d.id)], { http })).snapshotAt) };
      if (!d.curve) d = { ...d, curve: lc(await read(a, ROUTER_OF, c.factory, "routerOf", [d.coin], { http })) };
      const input = { coin: d.coin, snapshotAt: BigInt(d.snapshotAt), fromBlock: st.fromBlock ?? cfg.from_block, head: safe, exclusions: exclusionsFor(st, c, d.coin, d.curve, listed) };
      const rs = await Promise.all(urls.map((u) => Promise.resolve().then(() => rootOf(u, input, http, cfg.draws ?? {})).catch((err) => ({ error: `${host(u)}: ${String(err.message).slice(0, 200)}` }))));
      const v = verdictN({ root: d.root, totalTickets: d.totalTickets, snapshotAt: d.snapshotAt }, rs, quorum);
      d = { ...d, ...evidenceOf(id, d, rs, urls, v, { indexes, now }) };
      const o = drawOutcome(id, d, v, rs, st);
      draws[id] = o.draw; alerts.push(...o.alerts); triggers.push(...o.triggers);
    } catch (err) {
      draws[id] = { ...d, tries: (d.tries ?? 0) + 1, lastError: String(err.message).slice(0, 200) };
    }
  }

  // the deadline: every Open draw still undecided, whatever happened above (fixes-R3 §1.3). R4 (fixes-R4-services.md §3): the
  // guardian pauses on EVIDENCE (a caught-up index computing another root, a dissent confirmed after a rebuild, a failure a
  // healthy index cannot explain); a draw that is only unverified because the providers are degraded is SEV1 and escalated, and
  // the checker is degraded (/healthz 503), but nobody pauses automatically unless draws.pause_on_degraded says so
  const deadlineS = cfg.draws?.deadline_s ?? DEADLINE_S, pauseUnverified = cfg.draws?.pause_unverified !== false, pauseOnDegraded = cfg.draws?.pause_on_degraded === true;
  for (const [id, d] of Object.entries(draws)) {
    if (!DECIDING.includes(d.status) || !d.challengeEnds) continue;
    const nowS = now / 1000, left = d.challengeEnds - nowS, pin = new Date(d.challengeEnds * 1000).toISOString();
    if (left <= VERIFY_BEFORE_S && left > deadlineS)
      alerts.push({ kind: "draw", sev: "SEV2", key: `draw-unverified:${id}`, text: `${d.slug} draw ${d.id}: its root is NOT verified and its challenge window ends ${pin} (${d.lastError ?? "still recomputing"}). Check it by hand (exit 3 is a mismatch): ${byHandRoot(d)}; cancel before the pin if it is wrong.` });
    if (left > deadlineS) continue;
    const call = deadlineCall(d, { pauseOnDegraded });
    if (call.act === "single") {
      alerts.push({ kind: "draw", sev: "SEV2", key: `draw-single:${id}`, text: `${d.slug} draw ${d.id}: its root is reproduced by ONE provider only before its pin at ${pin} (${d.lastError ?? "the others are behind"}); none computes another. Not pausing. Check it by hand if you can: ${byHandRoot(d)}.` });
      continue;
    }
    const act = call.act === "pause" && pauseUnverified && d.confirmed;
    const head = `DRAW ${d.id} (${d.slug}, coin ${d.coin}) NOT VERIFIED BEFORE ITS PIN AT ${pin}${left > 0 ? ` (in ${Math.max(0, Math.round(left / 60))} min)` : " (the window is over)"}: ${call.why}.`;
    const hand = `By hand, before the pin: ${byHand.pause(st.registry)}; if the root is wrong, ${byHand.cancel(d.vault, d.id)}. Recompute: ${byHandRoot(d)}`;
    if (call.act === "pause") {
      alerts.push({ kind: "draw", sev: "SEV1", persistent: true, key: `draw-deadline:${id}`,
        text: [head, act ? "The guardian PAUSES THE KEEPER now (draws.pause_unverified): a paused keeper's draw cannot be pinned, so there is time to check it. If it verifies, the Safe unpauses." :
          `${d.confirmed ? "draws.pause_unverified is off" : "the draw's contents are not confirmed on the second RPC"}: nobody acts automatically.`, hand].join("\n") });
      if (act && left > -3600) triggers.push({ key: `unverified-pause:${id}`, action: "pause", why: `${d.slug} draw ${d.id} not verified before its pin at ${pin}: ${call.why}` });
    } else {
      // escalate: SEV1 at once and again every poll until the pin (a key per 2 minutes), and the checker is degraded
      const text = [head, `ESCALATE — ACT BY HAND: the guardian does NOT pause by itself, because nothing says the root is wrong (fixes-R4-services.md §3: a pause needs evidence; ${pauseOnDegraded ? "" : "draws.pause_on_degraded is off; "}degraded providers, a coin stuck on a dense block or an index still being rebuilt are not evidence, and outsiders can cause some of them). Look at the monitor's /status (holder_indexes) and recompute it yourself now.`, hand].join("\n");
      alerts.push({ kind: "draw", sev: "SEV1", persistent: true, key: `draw-deadline:${id}`, text });
      if (left > -600) alerts.push({ kind: "draw", sev: "SEV1", key: `draw-deadline-repeat:${id}:${Math.floor(now / 120_000)}`, text });
      if (left > -3600) escalated.push(`${d.slug} draw ${d.id} unverified at its deadline (${call.why})`);
    }
    draws[id] = { ...draws[id], deadlineSince: d.deadlineSince ?? now };
  }
  for (const [id, d] of Object.entries(draws)) if (d.challengeEnds && now / 1000 > d.challengeEnds + 30 * 86400) delete draws[id];

  // R3-check A3-I1, decided in round 4 (fixes-R4-services.md §8): a keeper that stops posting a good floor (or posts at confidence 0)
  // freezes burns until someone pauses it or the week's escape. With r4-contracts' A3-M1 a pause is worth nothing to an outsider
  // (paused burns pay the frozen floor, never above the backing), so the guardian MAY pause after `pause_on_no_good_post_h` hours
  // without a good post. Opt-in: a pause also stops draws and graduations, and a stale floor is not evidence of a stolen key
  const noGood = Number(cfg.guardian?.pause_on_no_good_post_h ?? 0);
  // docs/v7/dormant-collections.md: a collection the canister keeps DORMANT or WAKING posts no floor on purpose and holds nothing: its
  // poster is never paused for it (a paused PosterSource needs the Safe, and would wedge its next wake). `sleeping` gives those modules;
  // when it cannot be read, a poster-sourced module is paged but not paused
  let asleep = null, asleepUnread = null;
  if (noGood > 0 && sleeping) {
    try { asleep = await sleeping(); } catch (err) { asleepUnread = String(err?.message ?? err).slice(0, 160); }
  }
  if (noGood > 0) for (const c of st.collections.filter((x) => x.module)) {
    if (asleep && asleep.has(lc(c.module))) continue;
    const g = await read(a, MODULE, c.module, "lastGoodAt", [], { http }).catch(() => null);
    if (g === null || Number(g) === 0) continue;                         // unreadable, or never posted (before launch): nothing to act on
    const age = now - Number(g) * 1000;
    if (age >= noGood * 3_600_000 && asleepUnread !== null && c.posterSource) {
      alerts.push({ kind: "keeper", sev: "SEV1", persistent: true, key: `no-good-post-unsure:${c.module}:${g}`,
        text: `NO GOOD ${c.slug} FLOOR POST FOR ${hours(age)}, and the canister's collection lives could not be read (${asleepUnread}): it may be DORMANT on purpose (docs/v7/dormant-collections.md), so the guardian does NOT pause its poster. Look at eth_nft_collections() and pause by hand if it is ACTIVE: ${byHand.pausePoster(c.posterSource)}.` });
      continue;
    }
    if (age >= noGood * 3_600_000) {
      const key = `no-good-post-pause:${c.module}:${g}`;
      alerts.push({ kind: "keeper", sev: "SEV1", persistent: true, key: `no-good-post-long:${c.module}:${g}`,
        text: `NO GOOD ${c.slug} FLOOR POST FOR ${hours(age)} (evm.guardian.pause_on_no_good_post_h is ${noGood}): burns are refused (Stale) until the keeper is paused or the week's escape. The guardian pauses the keeper: paused burns pay the frozen floor, never above the backing per unit (r4-contracts A3-M1). The Safe unpauses once the keeper posts again (the modules stay stale until its first post). By hand: ${byHand.pause(st.registry)}.` });
      const why = `no good ${c.slug} floor post for ${hours(age)}: burns are frozen until a pause`;
      triggers.push(c.posterSource ? { key, action: "pause-poster", poster: c.posterSource, why } : { key, action: "pause", why });
    }
  }

  // the guardian: only on a confirmed trigger; pause before cancel (R2-L3); an action counts once its receipt says success, and an
  // unfinished one (not mined, or failed) is tried again next poll, for a day
  const acted = { ...(prev.acted ?? {}) };
  const open = new Map();
  for (const t of [...(prev.openTriggers ?? []).filter((x) => now - (x.at ?? now) < TRIGGER_TTL_MS), ...triggers.map((x) => ({ ...x, at: now }))])
    if (!acted[t.key] && !open.has(t.key)) open.set(t.key, t);
  const order = [...open.values()].sort((x, y) => (x.action === y.action ? 0 : x.action.startsWith("pause") ? -1 : 1));
  const still = [];
  for (const t of order) {
    const pausing = t.action === "pause" || t.action === "pause-poster";
    if (!guardian || (pausing && !guardian.autoPause) || (t.action === "cancel" && !guardian.autoCancel)) {
      alerts.push({ kind: "guardian", sev: "SEV1", persistent: true, key: `guardian-off:${t.key}`,
        text: `ACT BY HAND: ${t.why}. ${guardian ? "The guardian is configured not to act on this." : "No guardian is configured."} ${t.action === "pause" ? byHand.pause(st.registry) : t.action === "pause-poster" ? byHand.pausePoster(t.poster) : byHand.cancel(t.vault, t.id)}` });
      acted[t.key] = { at: now, byHand: true };
      continue;
    }
    try {
      const r = t.action === "pause" ? await guardian.pauseKeeper() : t.action === "pause-poster" ? await guardian.pausePoster(t.poster) : await guardian.cancelDraw(t.vault, t.id);
      alerts.push(guardianAlert(t, r, null, st));
      if (r.pending) still.push(t); else acted[t.key] = { at: now, ...r };
    } catch (err) {
      alerts.push(guardianAlert(t, null, err, st));
      still.push(t);
    }
  }

  // L-F (fixes-R4-services.md §4): a few coins a poll checked against the chain's state; a mismatch rebuilds the coin. R4-check L-S4
  // (fixes-golive-lows.md §6): AFTER the deadline and the guardian, with a budget of its own (every read bounded by what is left,
  // one try), so a provider that hangs on state reads can never delay a pause
  if (indexes && cfg.draws?.reconcile !== false) {
    const stateUrls = [...new Set([...cfg.rpcs, ...indexes.map((x) => x.url)])];
    const reconcileUntil = clock() + (cfg.draws?.reconcile_budget_s ?? RECONCILE_BUDGET_S) * 1000;
    await Promise.all(indexes.map((ix, i) => (indexHeads[i] ? ix.reconcile({ stateUrls, head: indexHeads[i], deadline: reconcileUntil }) : Promise.resolve([]))
      .then((found) => { for (const f of found) alerts.push({ kind: "watcher-error", sev: "SEV2", key: `index-mismatch:${ix.p.host}:${f.coin}:${f.at}`,
        text: `the holder index of coin ${f.coin} on ${ix.p.host} did not match the chain's state at block ${f.block} (${f.why})${f.rebuilt ? "; it is rebuilt from the coin's launch" : "; a rebuild was made less than an hour ago, so it waits"} (R3-check L-F: a provider that dropped a log).` }); }, () => undefined)));
    indexes.forEach((ix, i) => { if (indexState[i]) indexState[i] = { ...ix.summary(), error: indexState[i].error }; });
  }
  for (const ix of indexes ?? []) ix.save();              // a repair asked this poll (L-F), and what reconciliation found, are kept
  // I-1 (R3-check): this used to throw before the deadline and the guardian ran; now it throws after them
  if (disputed.length > DISPUTE_LIMIT) throw new Error(`the 4663 RPCs disagree about ${disputed.length} ranges (more than ${DISPUTE_LIMIT}): one of them is not serving this chain honestly`);
  const index = recordIndex(await readRecords());         // AFTER the logs
  const cfgJ = { ...cfg, wrappers: st.collections.flatMap((c) => c.wrappers ?? []), floor_module: undefined, modules: st.collections.map((c) => c.module).filter(Boolean),
    nft_wrappers: st.collections.filter((c) => c.kind === "nft").flatMap((c) => c.wrappers ?? []),
    nft_collection_of: Object.fromEntries(st.collections.filter((c) => c.kind === "nft" && c.nftCollection).map((c) => [c.wrapper, c.nftCollection])) };
  const j = judgeLogs({ logs, index, cfg: cfgJ, prev, timelock });
  if (ds.errors.length && !degraded && now - (prev.drawScanOkAt ?? now) >= STALL_MS) degraded = `the vaults' draws could not be read for ${hours(now - prev.drawScanOkAt)}: ${ds.errors[0]}`;
  if (escalated.length) degraded ??= `${escalated.join("; ")}: ACT BY HAND`;
  return { alerts: [...alerts, ...j.alerts], degraded, logs: logs.length,
    next: { ...j.next, cursor, head: Math.max(...known), disputed, stalledSince, minConf, maxMove, floorPosts: sj.next.floorPosts, floorHist: sj.next.floorHist,
      floorStreaks: sj.next.floorStreaks, draws, holds: dh.holds, acted, openTriggers: still, drawScan: ds.scan, drawScanOkAt: ds.errors.length ? (prev.drawScanOkAt ?? now) : now,
      indexes: indexState.length ? indexState : prev.indexes, unread } };
}
