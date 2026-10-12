import type { Address, ArrivalCheck, CanisterRecords, ChainRead, EthItem, EthReserveReport, Hex, ModuleBooks, Num, OwnerSaid, SourceKind, Status, Stray, WrapperBooks } from "./types.js";

/**
 * docs/v8/proof-of-reserves.md §2–§3: the Ethereum side's status, from public reads only, with the G5 rules unchanged:
 *   - GREEN only when every held item is owned by the custodian at Ethereum's finalized block on every provider (two at least),
 *     every arrival checks out, the wrapper's books agree with themselves and with the module, every read is fresh and came from
 *     two providers that agree, and (items − tickets in flight) × 1e18 + E = totalSupply exactly;
 *   - RED on anything proven wrong: an item every provider places elsewhere, books that disagree with themselves, an arrival that
 *     does not check out, the module's tiers not adding up, or backing short of supply even counting every unverified item;
 *   - UNVERIFIED otherwise: a read failed, came from one provider only, disagreed, or is stale.
 * Decision P1 (the recommended default): recomputed every 10 min; a number older than 1 h is unverified; a report older than 3 h is red.
 * Decision P4 (the recommended default): the canister's own records are shown, labelled, and never turn anything green (or red):
 * a discrepancy with them is a warning. Pure: every read is an input with its source, block and time.
 */
export const LIMITS = { maxAgeS: 3_600, redAfterS: 3 * 3_600, cadenceS: 600 } as const;
/** The monitor's thresholds for transient states (proof-of-reserves.md §3): past these a transient turns amber. */
export const TRANSIENT_S = { arrived: 24 * 3_600, resolvedTicket: 3_600, unjudgedInbound: 2 * 3_600 } as const;
export const ONE_ITEM = 10n ** 18n;

/** The canister's buy phases (ops/src/ic.ts BPhase) in which the advance's ETH sits at the custodian's Ethereum address. */
export const AT_CUSTODIAN = new Set(["Arrived", "Buying", "BackFunded"]);
/** Where an advance's ETH is, by its buy's phase (docs/v7/phase1-nft.md §2; the canister's word). */
export function whereIs(phase: string): string {
  if (phase === "Advancing" || phase === "Funded") return "at the canister's address on Robinhood Chain, waiting for its Relay order";
  if (phase === "Depositing" || phase === "Bridging") return "in Relay's bridge, from Robinhood Chain to Ethereum";
  if (phase === "Arrived") return "at the canister's address on Ethereum, earmarked for this buy";
  if (phase === "Buying") return "at the canister's address on Ethereum, in a purchase not yet final";
  if (["Bought", "Final", "Minting", "Minted", "Settling"].includes(phase)) return "spent: the item is bought and is being minted";
  if (phase.startsWith("Back") || phase === "Returning") return "on its way back to the module";
  if (phase === "Stuck") return "stuck: the canister has stopped this buy and says why in its note";
  return `in phase ${phase}`;
}
export type LedgerItemIn = { itemId: number; tokenId: string; how: "purchase" | "deposit"; arrivalTx: Hex; mintedAt: number | null; promisedTo: number | null };
export type EthReserveInput = {
  now: number;
  limits?: { maxAgeS: number; redAfterS: number; cadenceS: number };
  meta: { source: SourceKind; slug: string; name: string; symbol: string; collection: Address; custodian: Address; wrapper: Address; module: Address | null;
    providers: { eth: string[]; home: string[] }; lastGreenAt: number | null; links: EthReserveReport["links"]; nftUrl: (tokenId: string) => string | null; txUrl: (tx: string) => string | null };
  /** the wrapper's views, one read per Robinhood Chain provider at one block */
  wrapper: ChainRead<WrapperBooks>[];
  /** the wrapper's events folded (or, in the self-check, its item views): held items and tickets in flight */
  ledger: { complete: boolean; items: LedgerItemIn[]; ticketsInFlight: number; note: string };
  /** Ethereum's finalized block per provider; the reads below are made at the lowest */
  finalized: ChainRead<number>[];
  owners: Map<number, OwnerSaid[]>;
  arrivals: Map<number, ArrivalCheck>;
  costs: Map<number, { advancedWei: string; costWei: string }>;
  /** the module's views per Robinhood Chain provider; null: no module */
  module: ChainRead<ModuleBooks>[] | null;
  /** the custodian's ETH on Ethereum at N, per provider */
  custodianWei: ChainRead<string>[];
  canister: CanisterRecords | null;
  canisterNote: string;
  strays: Stray[];
  relay: { orderId: Hex; status: string | null; url: string } | null;
};

const host = (u: string) => { try { return new URL(u).host; } catch { return u; } };
const plural = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;
export function fmtUnits(units: bigint): string {
  const neg = units < 0n, v = neg ? -units : units, frac = (v % ONE_ITEM).toString().padStart(18, "0").replace(/0+$/, "");
  return `${neg ? "−" : ""}${v / ONE_ITEM}${frac ? "." + frac : ""}`;
}

/** Two providers or more that answered, all with the same value at the same block: the value; else why not. */
export function agreed<T>(reads: ChainRead<T>[], eq: (a: T, b: T) => boolean = (a, b) => JSON.stringify(a) === JSON.stringify(b)):
  { ok: true; value: T; block: number; at: number | null; sources: string[] } | { ok: false; why: string; sources: string[]; single: boolean } {
  const got = reads.filter((r): r is Extract<ChainRead<T>, { ok: true }> => r.ok);
  const failed = reads.filter((r): r is Extract<ChainRead<T>, { ok: false }> => !r.ok);
  const sources = reads.map((r) => r.source);
  if (got.length === 0) return { ok: false, why: failed.length ? `no provider answered (${failed.map((f) => `${f.source}: ${f.error}`).join("; ")})` : "no provider is configured", sources, single: false };
  if (new Set(got.map((g) => g.source)).size < 2) return { ok: false, why: `only ${got[0].source} answered${failed.length ? ` (${failed.map((f) => `${f.source}: ${f.error}`).join("; ")})` : ""}: one source is not a verdict`, sources, single: true };
  const first = got[0];
  if (got.some((g) => !eq(g.value, first.value) || g.block !== first.block)) return { ok: false, why: `the providers disagree (${got.map((g) => `${g.source} at ${g.block}`).join(" vs ")})`, sources, single: false };
  if (failed.length) return { ok: false, why: `${failed.map((f) => `${f.source}: ${f.error}`).join("; ")}; the others agree`, sources, single: false };
  const at = got.reduce<number | null>((a, g) => (g.at === null ? a : a === null ? g.at : Math.min(a, g.at)), null);
  return { ok: true, value: first.value, block: first.block, at, sources: got.map((g) => g.source) };
}

/** An item's owner, from what each provider said at N: in custody only if every provider (two at least) names the custodian. */
export function ownerVerdict(said: OwnerSaid[], custodian: Address): { verdict: EthItem["verdict"]; reason: string | null } {
  const c = custodian.toLowerCase();
  const answered = said.filter((s) => s.owner !== null || s.reverted);
  if (said.length === 0) return { verdict: "unverified", reason: "no provider was asked" };
  const names = new Set(answered.filter((s) => s.owner).map((s) => s.owner!.toLowerCase()));
  if (answered.length === said.length && new Set(said.map((s) => s.source)).size >= 2) {
    if (said.every((s) => s.owner?.toLowerCase() === c)) return { verdict: "in-custody", reason: null };
    if (said.every((s) => s.reverted)) return { verdict: "elsewhere", reason: "ownerOf reverts on every provider: the token does not exist or was burned" };
    if (names.size === 1 && !names.has(c) && said.every((s) => s.owner)) return { verdict: "elsewhere", reason: `every provider names ${[...names][0]} as its owner` };
    return { verdict: "unverified", reason: "the providers disagree about its owner" };
  }
  if (answered.length === 1 && said.length >= 1 && new Set(said.map((s) => s.source)).size < 2) return { verdict: "unverified", reason: `only ${answered[0].source} was read: one source is not a verdict` };
  const failed = said.filter((s) => s.owner === null && !s.reverted);
  return { verdict: "unverified", reason: `${failed.map((f) => f.source).join(", ")} did not answer${answered.length ? `; ${answered.map((a) => `${a.source} said ${a.owner ?? "revert"}`).join(", ")}` : ""}` };
}

export function computeEthReserve(x: EthReserveInput): EthReserveReport {
  const L = x.limits ?? LIMITS, now = x.now;
  const red: string[] = [], unverified: string[] = [], warnings: string[] = [];
  const stale = (at: number | null) => at === null || now - at > L.maxAgeS;
  const numbers: Num[] = [];
  const num = (key: string, label: string, value: string | null, sources: string[], block: number | null, at: number | null, ok: boolean | null, note = "") =>
    numbers.push({ key, label, value, sources, block, at, ok, note });

  // 1–2. the wrapper's views on 4663, on two providers
  const w = agreed(x.wrapper);
  let books: WrapperBooks | null = null, homeBlock: number | null = null, homeAt: number | null = null;
  if (w.ok) {
    books = w.value; homeBlock = w.block; homeAt = w.at;
    if (stale(w.at)) unverified.push(`the wrapper read at block ${w.block} is stale (older than ${L.maxAgeS / 60} min)`);
  } else unverified.push(`the wrapper's books on Robinhood Chain: ${w.why}`);
  const wsrc = x.wrapper.map((r) => r.source), wok = w.ok && !stale(w.at);
  num("supply", "wX in circulation (totalSupply)", books ? books.totalSupply : null, wsrc, homeBlock, homeAt, wok);
  num("held", "items the wrapper holds (held)", books ? String(books.held) : null, wsrc, homeBlock, homeAt, wok);
  num("openTickets", "tickets in flight (openTickets)", books ? String(books.openTickets) : null, wsrc, homeBlock, homeAt, wok);
  if (!x.ledger.complete) unverified.push(`the wrapper's event log could not be read in full (${x.ledger.note}), so the list of items may be incomplete`);
  num("ledger", "items in the wrapper's own events", x.ledger.complete ? String(x.ledger.items.length) : null, [x.ledger.note], homeBlock, homeAt, x.ledger.complete, "the fold of Wrapped, Deposited, Redeemed, Resolved, Delivered, Refunded");
  if (books && x.ledger.complete) {
    if (books.held !== x.ledger.items.length) red.push(`the wrapper says it holds ${books.held} and its own events account for ${x.ledger.items.length}: its books disagree with themselves`);
    if (books.openTickets !== x.ledger.ticketsInFlight) red.push(`the wrapper counts ${books.openTickets} open tickets and its events ${x.ledger.ticketsInFlight}`);
  }

  // 3. every held item's owner at Ethereum's finalized block, on every provider
  const fin = x.finalized.filter((f): f is Extract<ChainRead<number>, { ok: true }> => f.ok);
  const ethBlock = fin.length ? Math.min(...fin.map((f) => f.value)) : null;
  const ethAt = fin.length ? fin.find((f) => f.value === ethBlock)?.at ?? null : null;
  if (fin.length < 2) unverified.push(fin.length ? `Ethereum's finalized block came from ${fin[0].source} only: one source is not a verdict` : `Ethereum's finalized block could not be read (${x.finalized.map((f) => (f.ok ? "" : `${f.source}: ${f.error}`)).filter(Boolean).join("; ") || "no provider"})`);
  num("finalized", "Ethereum finalized block the custody reads are made at (the lower of the providers')", ethBlock === null ? null : String(ethBlock), x.finalized.map((f) => f.source), ethBlock, ethAt, fin.length >= 2 && !stale(ethAt));

  const items: EthItem[] = x.ledger.items.map((it) => {
    const owners = x.owners.get(it.itemId) ?? [];
    let { verdict, reason } = ownerVerdict(owners, x.meta.custodian);
    if (verdict === "in-custody" && stale(ethAt)) { verdict = "unverified"; reason = `the owner read is stale (older than ${L.maxAgeS / 60} min)`; }
    const arrival = x.arrivals.get(it.itemId) ?? { verdict: "unread" as const, block: null, final: null, pricePaidWei: null, detail: "not read", sources: [] };
    const cost = x.costs.get(it.itemId);
    return { itemId: it.itemId, tokenId: it.tokenId, how: it.how, arrivalTx: it.arrivalTx, arrivalUrl: x.meta.txUrl(it.arrivalTx), mintedAt: it.mintedAt, nftUrl: x.meta.nftUrl(it.tokenId),
      promisedTo: it.promisedTo, owners, ownerBlock: ethBlock, ownerAt: ethAt, verdict, reason, arrival,
      cost: cost ? { advancedWei: cost.advancedWei, costWei: cost.costWei, expectedCostWei: null, ok: null } : null };
  });
  const inCustody = items.filter((i) => i.verdict === "in-custody").length, elsewhere = items.filter((i) => i.verdict === "elsewhere");
  const unread = items.filter((i) => i.verdict === "unverified");
  if (elsewhere.length) red.push(`${plural(elsewhere.length, "item")} found outside custody: ${elsewhere.slice(0, 5).map((i) => `#${i.tokenId} (${i.reason})`).join(", ")}${elsewhere.length > 5 ? ", …" : ""}`);
  if (unread.length) unverified.push(`${plural(unread.length, "item")} could not be verified (#${unread[0].tokenId}: ${unread[0].reason ?? "no reason given"}${unread.length > 1 ? ", and others" : ""})`);
  num("inCustody", "items owned by the custodian at the finalized block, by every provider", String(inCustody), x.meta.providers.eth.map(host), ethBlock, ethAt, unread.length === 0 && elsewhere.length === 0 && fin.length >= 2 && !stale(ethAt));

  // 4. each item's arrival transaction
  const badArrivals = items.filter((i) => i.arrival.verdict === "bad"), unreadArrivals = items.filter((i) => i.arrival.verdict === "unread" || i.arrival.final === false);
  if (badArrivals.length) red.push(`${plural(badArrivals.length, "arrival")} do${badArrivals.length === 1 ? "es" : ""} not check out: ${badArrivals.slice(0, 3).map((i) => `#${i.tokenId} (${i.arrival.detail})`).join(", ")}`);
  if (unreadArrivals.length) unverified.push(`${plural(unreadArrivals.length, "arrival")} could not be checked (#${unreadArrivals[0].tokenId}: ${unreadArrivals[0].arrival.final === false ? "its receipt is above the finalized block" : unreadArrivals[0].arrival.detail})`);
  num("arrivals", "arrival transactions checked (final, status 1, the transfer to the custodian, the fill or the claim)", `${items.filter((i) => i.arrival.verdict === "ok" && i.arrival.final).length} / ${items.length}`,
    x.meta.providers.eth.map(host), ethBlock, ethAt, badArrivals.length ? false : unreadArrivals.length ? null : true);

  // 6. the ETH side at the module: C + R + T = E, and the wrapper's ethBacked is the module's E
  let E = 0n, mod: ModuleBooks | null = null;
  if (x.module === null) {
    if (books && BigInt(books.ethBacked) !== 0n) red.push(`the wrapper counts ${fmtUnits(BigInt(books.ethBacked))} ETH-backed units and no module is configured`);
  } else {
    const m = agreed(x.module);
    if (!m.ok) unverified.push(`the floor module: ${m.why}`);
    else {
      mod = m.value; E = BigInt(mod.ethBackedUnits);
      if (stale(m.at)) unverified.push("the floor module read is stale");
      const C = BigInt(mod.convertibleUnits), R = BigInt(mod.reserveUnits), T = BigInt(mod.transitUnits), esc = BigInt(mod.escrowWei), bal = BigInt(mod.balanceWei);
      if (C + R + T !== E) red.push(`the module's tiers do not add up: ${fmtUnits(C)} convertible + ${fmtUnits(R)} reserve + ${fmtUnits(T)} in transit against ${fmtUnits(E)} ETH-backed`);
      if (T > ONE_ITEM) red.push(`the module counts ${fmtUnits(T)} units in transit: it allows one advance at a time`);
      if (E > 0n && esc === 0n) red.push(`${fmtUnits(E)} ETH-backed units stand against an empty escrow`);
      if (bal < esc) red.push(`the module holds ${bal} wei and its pots claim ${esc}: its ETH is short of its own books`);
      if (books && BigInt(books.ethBacked) !== E) red.push(`the wrapper counts ${fmtUnits(BigInt(books.ethBacked))} ETH-backed units and the module ${fmtUnits(E)}: their books disagree`);
      const src = x.module.map((r) => r.source), ok = !stale(m.at);
      num("E", "ETH-backed units at the module (E)", mod.ethBackedUnits, src, m.block, m.at, ok);
      num("C", "convertible units (C)", mod.convertibleUnits, src, m.block, m.at, ok);
      num("R", "standing reserve units (R)", mod.reserveUnits, src, m.block, m.at, ok);
      num("T", "the unit in transit (T)", mod.transitUnits, src, m.block, m.at, ok);
      num("escrow", "ETH in the module's pots (escrowBalance)", mod.escrowWei, src, m.block, m.at, ok);
      num("moduleBalance", "the module's ETH (eth_getBalance)", mod.balanceWei, src, m.block, m.at, ok && bal >= esc);
    }
  }

  // the equation
  const supply = books ? BigInt(books.totalSupply) : null;
  const backing = BigInt(inCustody - x.ledger.ticketsInFlight) * ONE_ITEM + E;
  const best = BigInt(inCustody + unread.length - x.ledger.ticketsInFlight) * ONE_ITEM + E;
  let holds: boolean | null = null;
  if (supply !== null && x.ledger.complete) {
    if (best < supply) red.push(`under-backed: even if every unverified item is in custody, ${fmtUnits(best)} wX of backing stands against ${fmtUnits(supply)} wX in supply`);
    if (unread.length === 0 && elsewhere.length === 0) {
      holds = backing === supply;
      if (!holds && best >= supply) red.push(`mismatch: ${fmtUnits(backing)} wX of backing against ${fmtUnits(supply)} wX in supply (${backing > supply ? "more backing than supply: the books are wrong" : "under-backed"})`);
    } else if (best < supply) holds = false;
  }

  // 8–9. the advance's whereabouts and the gas pot: explanation, labelled; discrepancies are warnings
  const cw = agreed(x.custodianWei);
  const earmarked = x.canister ? x.canister.buys.filter((b) => AT_CUSTODIAN.has(b.phase)).reduce((a, b) => a + BigInt(b.advancedWei), 0n) : null;
  const leftover = x.canister?.leftoverWei != null ? BigInt(x.canister.leftoverWei) : null;
  const gas = { custodianWei: cw.ok ? cw.value : null, earmarkedWei: earmarked === null ? null : String(earmarked), leftoverWei: leftover === null ? null : String(leftover),
    potWei: cw.ok && earmarked !== null && leftover !== null ? String(BigInt(cw.value) - earmarked - leftover) : null, sources: cw.sources };
  num("custodianEth", "the custodian's ETH on Ethereum (earmarks + leftovers + the gas pot; backs no unit)", gas.custodianWei, cw.sources, cw.ok ? cw.block : null, cw.ok ? cw.at : null, cw.ok ? !stale(cw.at) : null);
  if (cw.ok && earmarked !== null && BigInt(cw.value) < earmarked) warnings.push(`the custodian holds ${cw.value} wei on Ethereum and the canister's records earmark ${earmarked} for arrived buys`);

  const T = mod ? BigInt(mod.transitUnits) : 0n;
  let advance: EthReserveReport["advance"] = null;
  if (mod && (T > 0n || BigInt(mod.advancedWei) > 0n)) {
    const open = x.canister?.buys.filter((b) => b.phase !== "Settled" && b.phase !== "Returned" && b.phase !== "NeedAdvance") ?? null;
    const b = open && open.length === 1 ? open[0] : null;
    const where = !x.canister ? "the canister's records were not read: where the ETH is now is not shown"
      : !b ? `the module has an open advance and the canister's records show ${open!.length} open buys`
      : whereIs(b.phase);
    const overdue = mod.advancedAt !== null && now - mod.advancedAt > TRANSIENT_S.arrived;
    advance = { open: T > 0n, advancedWei: mod.advancedWei, advancedAt: mod.advancedAt, phase: b?.phase ?? null, where, relay: x.relay, overdue };
    if (x.canister && open && open.length !== 1) warnings.push(`the module has an advance open and the canister's records show ${open.length} open buys (expected exactly one)`);
    if (overdue) warnings.push(`the advance has been open ${Math.round((now - mod.advancedAt!) / 3600)} h, past the ${TRANSIENT_S.arrived / 3600} h the monitor allows`);
  }

  // 7. the canister's word against the chain's: warnings only (P4)
  if (x.canister) {
    const settled = new Set(x.canister.buys.filter((b) => b.phase === "Settled" && b.tokenId).map((b) => b.tokenId!));
    const deposits = new Set(x.canister.deposits.map((d) => d.tokenId));
    const unexplained = items.filter((i) => (i.how === "purchase" ? !settled.has(i.tokenId) : !deposits.has(i.tokenId)));
    if (unexplained.length) warnings.push(`${plural(unexplained.length, "held item")} ${unexplained.length === 1 ? "has" : "have"} no matching record in the canister's own books (#${unexplained.slice(0, 3).map((i) => i.tokenId).join(", #")})`);
  }
  for (const s of x.strays) if (s.overdue) warnings.push(`#${s.tokenId} reached the custodian and is not in the wrapper's books: ${s.state}, for longer than its timeout`);

  const status: Status = red.length ? "red" : unverified.length ? "unverified" : holds === true ? "green" : "unverified";
  if (status === "unverified" && !unverified.length) unverified.push("the equation could not be decided");
  return {
    source: x.meta.source, slug: x.meta.slug, name: x.meta.name, symbol: x.meta.symbol, collection: x.meta.collection, custodian: x.meta.custodian, wrapper: x.meta.wrapper,
    moduleAddress: x.meta.module, providers: x.meta.providers, status, reasons: [...red, ...unverified], warnings, computedAt: now,
    lastGreenAt: status === "green" ? now : x.meta.lastGreenAt, limits: { ...L }, homeBlock, homeAt, ethBlock, ethAt, numbers,
    equation: { inCustody, unverified: unread.length, elsewhere: elsewhere.length, ticketsInFlight: x.ledger.ticketsInFlight, ethBackedUnits: String(E),
      backingUnits: String(backing), bestCaseUnits: String(best), supplyUnits: supply === null ? null : String(supply), holds },
    items, module: mod, advance, gas, strays: x.strays, canister: x.canister, canisterNote: x.canisterNote, links: x.meta.links,
  };
}

/**
 * The status as SHOWN at `now`, whatever it was when computed: a report older than `redAfterS` is red; a green report with any
 * number older than `maxAgeS` is unverified. The page, the CLI and the API all show this, so a cached green never outlives its reads.
 */
export function servedStatus(r: Pick<EthReserveReport, "status" | "reasons" | "computedAt" | "numbers" | "limits">, now: number): { status: Status; reasons: string[]; ageS: number } {
  const ageS = Math.max(0, now - r.computedAt);
  if (ageS > r.limits.redAfterS) return { status: "red", reasons: [`stale: this report was computed ${Math.round(ageS / 60)} min ago, over the ${r.limits.redAfterS / 3600} h limit`, ...r.reasons], ageS };
  const old = r.numbers.filter((n) => n.ok !== null && (n.at === null || now - n.at > r.limits.maxAgeS));
  if (r.status === "green" && old.length) return { status: "unverified", reasons: [`${plural(old.length, "number")} ${old.length === 1 ? "is" : "are"} older than ${r.limits.maxAgeS / 60} min (${old.map((n) => n.label).slice(0, 3).join("; ")})`], ageS };
  return { status: r.status, reasons: r.reasons, ageS };
}
