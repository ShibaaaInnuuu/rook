import { decodeFunctionResult, encodeFunctionData, parseAbi, type Hex } from "viem";
import type { Backing, BackingRead } from "./backing.js";
import { anchorDisc, b58, findProgramAddress, key } from "./fuel/sol.js";
import type { BooksViewV, Buyer, CoinsCollectionV, DeskViewV, VaultV } from "./ic.js";
import { mintSupply, type SolAccount, type SolReader } from "./solread.js";
/**
 * W11 (docs/v9/canister-only-solana.md §10.2, the proof of reserves on the BOOKS home, `rook_core = "books"`): the canister keeps the
 * books itself, so the reconciler reads them from its queries (`books_collections`, `books_view(coll)`, `coins_collections`,
 * `coins_vaults(coll)`) and checks them against Solana at `finalized`, every provider agreeing (src/solread.ts), and against the
 * items' custody (src/location.ts):
 *  - the wX mint's supply equals the ledger's `expected_supply = (held − open_tickets)·10⁹ + sol_backed − module_units`;
 *  - `M[i]`'s lamports above its rent cover `owed = backing + reserve + surplus + revenue` (the backing report of src/backing.ts, in
 *    lamports and 10⁹ units);
 *  - every held item is FOUND in custody (an inscription at the custody taproot, an Ethereum NFT owned by the custody address, a Solana
 *    NFT in the custodian's token account for its mint: amount 1, frozen fine for a pNFT, read at finalized with every provider
 *    agreeing); elsewhere is RED, undecided is UNVERIFIED, never green;
 *  - each coin vault (`K_coin[c]`'s wX account) holds at least its book (`wx_in − wx_paid`).
 * The mint authority and the desk's position are the canister's own checks (`check_wx_mint`, `pool_desks`); not repeated here.
 * Reads only; no key. (The reader of rook_core's own accounts, src/solreserves.ts, is retired with the programs: decision D-l.)
 */
export type ListingKindName = "EthNft" | "Inscriptions" | "SolNft";
export type ListingRef = { id: number; slug: string; name: string; symbol: string; kind: ListingKindName };
export type Equation = { itemsInCustody: number; itemsUnverified: number; itemsElsewhere: number; ticketsInFlight: number; backingUnits: string;
  backingUnitsIfUnverifiedHold: string; ethBackedUnits: string; reserveUnits: string; floatUnits: string; convertibleUnits: string; transitUnits: string;
  escrowWei: string; supplyUnits: string | null; heldOnChain: number | null; openTicketsOnChain: number | null; heldInLedger: number; holds: boolean | null };
export type Where = "custody" | "elsewhere" | "unverified" | "delivering";
/** What the locators read of a listing: its kind, with the custody address of that kind (the books view's, src/ic.ts `BooksViewV`). */
export type ListingV = { id: number; kind: { EthNft: { chainId: bigint; collection: string } } | { Inscriptions: { membershipRoot: string } } | { SolNft: { collection: string } };
  /** the taproot custody (Inscriptions) or the custodian's base58 address (SolNft: the canister's own wallet, `sol_address`) */
  custody: string; custodyEvm: string };
/** A held item as the locators read it: its id, its 32-byte key (an Ethereum token id, an inscription's leaf, or a Solana NFT's mint) and its inscription id. */
export type ItemV = { itemId: number; key: Uint8Array; inscriptionId: string };
/** Where a held item is: in custody, provably elsewhere, or not decided. */
export type Locate = (listing: ListingV, item: ItemV) => Promise<{ where: "custody" | "elsewhere" | "unverified"; note?: string }>;
export type BooksCanister = Pick<Buyer, "books_collections" | "books_view" | "coins_collections" | "coins_vaults" | "pool_desks">;
/** The pool desk of one collection as reconciled: the canister's books of it against the chain (null entries: not read). */
export type DeskReport = { pair: string; position: string; desk: string | null; native: [string, string | null]; wsol: [string, string | null]; wx: [string, string | null] };
export type BooksReservesDeps = { reader: SolReader; canister: BooksCanister; locate: Locate; nowS: number; names?: Map<number, ListingRef> };
/** A collection's report: the fields of the indexer's `ReserveReport` the reserves page and the alerts read, for one of the canister's collections. */
export type BooksCollectionReport = { source: "books"; slug: string; name: string; symbol: string; kind: ListingKindName; listing: string; wrapper: string;
  status: "green" | "red" | "unverified"; reasons: string[]; computedAt: number; block: number | null; equation: Equation;
  items: { itemId: number; key: string; where: Where; note?: string }[]; ticketsOpen: { id: string; stage: string; itemId: number }[];
  coinVaults: { coin: string; vault: string; book: string; onChain: string | null }[]; desk: DeskReport | null };

export const DLMM_PROGRAM = "LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo";
export const TOKEN_PROGRAM = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
export const ASSOCIATED_TOKEN_PROGRAM = "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL";
export const NATIVE_MINT = "So11111111111111111111111111111111111111112";
/** The associated token account of `owner` for `mint`. */
export const ata = (owner: string, mint: string) => findProgramAddress([key(owner), key(TOKEN_PROGRAM), key(mint)], ASSOCIATED_TOKEN_PROGRAM)[0];
const POSITION_V2_LEN = 8_120;
/** A DLMM `PositionV2` as the canister decodes it (`solix2::dlmm::PositionV2::decode`): the pair, the owner, the bin range. */
export function decodePosition(a: { owner: string; data: Uint8Array }): { pair: string; owner: string; lower: number; upper: number } | null {
  const d = a.data;
  if (a.owner !== DLMM_PROGRAM || d.length !== POSITION_V2_LEN || Buffer.compare(Buffer.from(d.subarray(0, 8)), Buffer.from(anchorDisc("account", "PositionV2"))) !== 0) return null;
  const b = Buffer.from(d.buffer, d.byteOffset, d.byteLength);
  return { pair: b58(d.subarray(8, 40)), owner: b58(d.subarray(40, 72)), lower: b.readInt32LE(7_912), upper: b.readInt32LE(7_916) };
}

const hex = (b: Uint8Array | number[]) => `0x${Buffer.from(b as Uint8Array).toString("hex")}`;
const s = (x: bigint) => x.toString();
const ONE = 1_000_000_000n;
/** An SPL token account's amount (offset 64, u64 LE). */
export function tokenAmount(d: Uint8Array): bigint {
  if (d.length < 165) throw new Error("not an SPL token account");
  return Buffer.from(d.buffer, d.byteOffset, d.byteLength).readBigUInt64LE(64);
}
const opt = <T>(x: [] | [T]): T | null => (x.length ? x[0] : null);

/** The listing a books view describes, in the shape the locators read (`Locate`). */
export function listingOf(v: BooksViewV): ListingV {
  const k = v.kind;
  if ("EthNft" in k) return { id: v.coll, kind: { EthNft: { chainId: k.EthNft.chain_id, collection: hex(k.EthNft.collection) } }, custody: "", custodyEvm: hex(k.EthNft.custody) };
  if ("Inscriptions" in k) return { id: v.coll, kind: { Inscriptions: { membershipRoot: hex(k.Inscriptions.membership_root) } }, custody: k.Inscriptions.custody, custodyEvm: "" };
  return { id: v.coll, kind: { SolNft: { collection: b58(Uint8Array.from(k.SolNft.collection)) } }, custody: b58(Uint8Array.from(k.SolNft.custodian)), custodyEvm: "" };
}

/** A classic SPL token account's fields the custody rule reads (165 bytes: mint @0, owner @32, amount @64, delegate tag @72, state @108). */
export function decodeTokenAccount(a: { owner: string; data: Uint8Array }): { mint: string; owner: string; amount: bigint; delegated: boolean; frozen: boolean } | null {
  const d = a.data;
  if (a.owner !== TOKEN_PROGRAM || d.length !== 165) return null;
  const b = Buffer.from(d.buffer, d.byteOffset, d.byteLength);
  return { mint: b58(d.subarray(0, 32)), owner: b58(d.subarray(32, 64)), amount: b.readBigUInt64LE(64), delegated: b.readUInt32LE(72) !== 0, frozen: d[108] === 2 };
}

/**
 * A Solana NFT's location (docs/v9/solana-nfts-canister.md §2, `solnft::in_custody` for Token Metadata): the custodian's associated token
 * account for the item's mint, read at `finalized` through `reader` (every provider must agree, src/solread.ts; with one provider it is
 * that provider's word). In custody: SPL Token, this mint, owned by the custodian, amount 1 (frozen is fine: a pNFT's account always is).
 * The account absent or empty, both providers agreeing: elsewhere. A delegate on it, a read that fails or disagrees: unverified.
 */
export async function locateSolNft(reader: SolReader | null, custodian: string, mint: string): Promise<{ where: "custody" | "elsewhere" | "unverified"; note?: string }> {
  if (!reader) return { where: "unverified", note: "no Solana RPC to read the custodian's token account (SOL_RPC_URLS)" };
  let at: string;
  try { at = ata(custodian, mint); } catch (err) { return { where: "unverified", note: `not an address: ${String((err as Error).message).slice(0, 120)}` }; }
  let a: SolAccount | null;
  try { a = await reader.account(at); } catch (err) { return { where: "unverified", note: `the custodian's token account ${at}: ${String((err as Error).message).slice(0, 160)}` }; }
  if (!a) return { where: "elsewhere", note: `the custodian ${custodian} has no token account for ${mint} (${at} does not exist)` };
  const t = decodeTokenAccount(a);
  if (!t || t.mint !== mint || t.owner !== custodian) return { where: "unverified", note: `${at} is not the custodian's SPL token account for ${mint}` };
  if (t.amount !== 1n) return { where: "elsewhere", note: `the custodian's token account ${at} holds ${t.amount} of ${mint}, not 1` };
  if (t.delegated) return { where: "unverified", note: `the custodian's token account ${at} for ${mint} has a delegate: someone else could move it` };
  return { where: "custody" };
}

// ---- the locators ---------------------------------------------------------------------------------------------------------------

const ERC721 = parseAbi(["function ownerOf(uint256) view returns (address)"]);

/** 2026-10-12: the RPCs that read `ownerOf` of an EthNft listing of `chainId`: Ethereum's (`ethRpcs`), or that chain's (`rpcsByChain`, by chain id:
 * Robinhood Chain's for its own collections). None: the item is unverified, never assumed. */
export function rpcsOf(o: { ethRpcs: string[]; rpcsByChain?: Record<string, string[]> }, chainId: bigint): string[] {
  return o.rpcsByChain?.[chainId.toString()] ?? (chainId === 1n ? o.ethRpcs : []);
}

/**
 * An inscription's location from `ordFetch` (src/location.ts `agreeingOrdFetch`: answered only when two providers agree, so "elsewhere"
 * is never one provider's word); an EVM NFT's owner from every RPC of its listing's chain (`rpcsOf`: Ethereum's `ethRpcs`, Robinhood
 * Chain's in `rpcsByChain`; each provider's `eth_chainId` must be the listing's chain; all answering must agree, and off Ethereum two at
 * least, every one answering: audit C3-09; none: unverified).
 */
export function locator(o: { ordFetch: typeof fetch; ord?: string; ethRpcs: string[]; rpcsByChain?: Record<string, string[]>; fetch?: typeof fetch; solReader?: SolReader | null }): Locate {
  const f = o.fetch ?? fetch, ord = o.ord ?? "https://ordinals.com";
  // audit C3-09: each EVM provider's `eth_chainId`, read once per locator (one reconciler run)
  const chainIds = new Map<string, Promise<bigint>>();
  return async (L, item) => {
    if ("SolNft" in L.kind) return locateSolNft(o.solReader ?? null, L.custody, b58(item.key));
    if ("Inscriptions" in L.kind) {
      if (!/^[0-9a-f]{64}i\d+$/.test(item.inscriptionId)) return { where: "unverified", note: `item ${item.itemId} carries no inscription id` };
      const r = await o.ordFetch(`${ord}/r/inscription/${item.inscriptionId}`);
      if (!r.ok) return { where: "unverified", note: `ordinals: HTTP ${r.status}` };
      const j = await r.json() as { address?: string };
      return j.address === L.custody ? { where: "custody" } : { where: "elsewhere", note: `${item.inscriptionId} is at ${j.address}, not the custody address ${L.custody}` };
    }
    const chain = BigInt((L.kind as { EthNft: { chainId: number | bigint } }).EthNft.chainId);
    const rpcs = rpcsOf(o, chain);
    if (!rpcs.length) return { where: "unverified", note: chain === 1n ? "no Ethereum RPC to read ownerOf (RECONCILE_ETH_RPC_URLS)"
      : `no RPC of chain ${chain} to read ownerOf${chain === 4663n ? " (RECONCILE_RH_RPC_URLS)" : ""}` };
    // audit C3-09: a chain other than Ethereum is read on two providers at least, every one answering (one provider's word is no check)
    if (chain !== 1n && rpcs.length < 2) return { where: "unverified", note: `chain ${chain}: two RPCs at least read ownerOf, every one answering (audit C3-09); ${rpcs.length} configured` };
    const collection = (L.kind as { EthNft: { collection: string } }).EthNft.collection;
    const tokenId = BigInt(`0x${Buffer.from(item.key).toString("hex")}`);
    const data = encodeFunctionData({ abi: ERC721, functionName: "ownerOf", args: [tokenId] });
    const call = async (u: string, method: string, params: unknown[]): Promise<Hex> => {
      const r = await f(u, { method: "POST", headers: { "content-type": "application/json" }, redirect: "manual", signal: AbortSignal.timeout(15_000),
        body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params }) });
      const j = await r.json() as { result?: Hex; error?: unknown };
      if (j.error || !j.result) throw new Error(`${method}: ${JSON.stringify(j.error ?? "no result").slice(0, 120)}`);
      return j.result;
    };
    const owners = await Promise.allSettled(rpcs.map(async (u) => {
      // audit C3-09: the provider serves the listing's chain (`eth_chainId`, once per provider and run): an Ethereum URL in 4663's list is refused
      let id = chainIds.get(u);
      if (!id) { id = call(u, "eth_chainId", []).then((x) => BigInt(x)); chainIds.set(u, id); id.catch(() => chainIds.delete(u)); }
      const served = await id;
      if (served !== chain) throw new Error(`${u} serves chain ${served}, not ${chain}`);
      return String(decodeFunctionResult({ abi: ERC721, functionName: "ownerOf", data: await call(u, "eth_call", [{ to: collection, data }, "finalized"]) })).toLowerCase();
    }));
    const got = owners.filter((x): x is PromiseFulfilledResult<string> => x.status === "fulfilled").map((x) => x.value);
    if (!got.length) return { where: "unverified", note: `no RPC of chain ${chain} answered ownerOf` };
    // audit C3-09: off Ethereum, every configured provider must answer: one failing leaves the other's word alone, which decides nothing
    if (chain !== 1n && got.length < rpcs.length) {
      const why = owners.find((x): x is PromiseRejectedResult => x.status === "rejected")?.reason;
      return { where: "unverified", note: `${rpcs.length - got.length} of the ${rpcs.length} RPCs of chain ${chain} did not answer ownerOf (${String((why as Error)?.message ?? why).slice(0, 120)})` };
    }
    if (got.some((x) => x !== got[0])) return { where: "unverified", note: `the RPCs of chain ${chain} disagree on ownerOf` };
    return got[0] === L.custodyEvm.toLowerCase() ? { where: "custody" } : { where: "elsewhere", note: `token ${tokenId} is owned by ${got[0]}, not ${L.custodyEvm}` };
  };
}

export async function readBooksReserves(d: BooksReservesDeps): Promise<{ collections: BooksCollectionReport[]; backing: BackingRead; slot: number | null }> {
  const collections: BooksCollectionReport[] = [], modules: Backing[] = [];
  let backingError: string | null = null, slot: number | null = null;
  try { slot = await d.reader.slot(); } catch { slot = null; }
  const ids = await d.canister.books_collections();
  const coinsCols = new Map((await d.canister.coins_collections()).map((c: CoinsCollectionV) => [c.index, c]));
  const desks = new Map((await d.canister.pool_desks().catch(() => [] as DeskViewV[])).map((x) => [x.coll, x]));
  let rent: bigint | null = null;
  for (const [coll, wxMint] of ids) {
    const ref = d.names?.get(coll);
    const rep: BooksCollectionReport = { source: "books", slug: ref?.slug ?? `collection-${coll}`, name: ref?.name ?? `collection ${coll}`, symbol: ref?.symbol ?? (coinsCols.get(coll)?.symbol ?? "wX"),
      kind: "EthNft", listing: wxMint, wrapper: wxMint, status: "green", reasons: [], computedAt: d.nowS, block: slot, items: [], ticketsOpen: [], coinVaults: [], desk: null,
      equation: { itemsInCustody: 0, itemsUnverified: 0, itemsElsewhere: 0, ticketsInFlight: 0, backingUnits: "0", backingUnitsIfUnverifiedHold: "0", ethBackedUnits: "0",
        reserveUnits: "0", floatUnits: "0", convertibleUnits: "0", transitUnits: "0", escrowWei: "0", supplyUnits: null, heldOnChain: null, openTicketsOnChain: null, heldInLedger: 0, holds: null } };
    collections.push(rep);
    try {
      const v = opt(await d.canister.books_view(coll));
      if (!v) { rep.status = "unverified"; rep.reasons.push(`the canister lists collection ${coll} but serves no books for it`); continue; }
      rep.kind = "Inscriptions" in v.kind ? "Inscriptions" : "SolNft" in v.kind ? "SolNft" : "EthNft";
      const halted = opt(v.halted);
      if (halted) rep.reasons.push(`collection ${coll} is STOPPED by an incident in the canister: ${halted.slice(0, 200)}`);
      // the module's lamports against its books (the module's address is the collection key's: coins_collections names it)
      const moduleAddr = coinsCols.get(coll)?.module || null;
      if (moduleAddr) {
        try {
          rent ??= await d.reader.rent(0);
          const ma = await d.reader.account(moduleAddr);
          const balance = ma ? ma.lamports - rent : 0n;
          modules.push({ slug: rep.slug, module: moduleAddr, wrapper: wxMint, block: slot ?? 0, balance, owed: v.owed, decimals: 9,
            books: { backing: v.backing, reserve: v.reserve, surplus: v.surplus, revenue: v.revenue, gradEth: 0n, rentalEscrow: 0n, renterPool: 0n, owedTotal: 0n },
            reserveUnits: v.reserve_units, ethBacked: v.sol_backed, accepted: v.accepted, fresh: v.fresh, escaped: v.escaped, postedAt: Number(v.posted_at),
            lastGoodAt: Number(v.last_good_at), floorTtl: 3 * 3_600, transitUnits: v.transit_units, advanced: v.advanced, advancedAt: 0 });
        } catch (err) { backingError = `${rep.slug}: ${String((err as Error).message).slice(0, 160)}`; }
      } else if (v.owed > 0n) rep.reasons.push(`the books owe ${v.owed} lamports at M[${coll}] but the canister names no module address for it (coins_collections): the balance is unverified`);
      // every held item, located
      const L = listingOf(v);
      const delivering = new Set(v.tickets_open.filter((t) => t.stage === "Resolved").map((t) => t.item_id));
      const e = rep.equation;
      for (const it of v.items) {
        const item: ItemV = { itemId: it.item_id, key: Uint8Array.from(Buffer.from(it.key, "hex")), inscriptionId: it.inscription_id };
        const r = await d.locate(L, item).catch((err) => ({ where: "unverified" as const, note: String((err as Error).message).slice(0, 160) }));
        const where: Where = r.where === "elsewhere" && delivering.has(it.item_id) ? "delivering" : r.where;
        rep.items.push({ itemId: it.item_id, key: `0x${it.key}`, where, ...(r.note ? { note: r.note } : {}) });
        if (where === "custody" || where === "delivering") e.itemsInCustody++;
        else if (where === "elsewhere") e.itemsElsewhere++;
        else e.itemsUnverified++;
      }
      rep.ticketsOpen = v.tickets_open.map((t) => ({ id: String(t.id), stage: t.stage, itemId: t.item_id }));
      // the wX supply against the ledger
      const mint = await d.reader.account(wxMint);
      const supply = mint ? mintSupply(mint.data) : null;
      const expected = opt(v.expected_supply);
      e.heldOnChain = Number(v.held); e.openTicketsOnChain = Number(v.open_tickets); e.heldInLedger = v.items.length; e.ticketsInFlight = v.tickets_open.length;
      e.ethBackedUnits = s(v.sol_backed); e.supplyUnits = supply === null ? null : s(supply);
      e.reserveUnits = s(v.reserve_units); e.transitUnits = s(v.transit_units);
      e.convertibleUnits = s(v.sol_backed > v.reserve_units + v.transit_units ? v.sol_backed - v.reserve_units - v.transit_units : 0n);
      e.escrowWei = s(v.backing + v.reserve);
      e.backingUnits = s(BigInt(e.itemsInCustody - v.tickets_open.length) * ONE + v.sol_backed);
      e.backingUnitsIfUnverifiedHold = s(BigInt(e.itemsInCustody + e.itemsUnverified - v.tickets_open.length) * ONE + v.sol_backed);
      e.holds = supply === null || expected === null ? null : supply === expected && BigInt(v.items.length) === v.held;
      // the coin vaults
      for (const vault of await d.canister.coins_vaults(coll)) {
        const addr = opt(vault.vault);
        const book = vault.wx_in - vault.wx_paid;
        if (!addr) { rep.coinVaults.push({ coin: vault.coin, vault: "", book: s(book), onChain: null }); continue; }
        const va = await d.reader.account(addr).catch(() => null);
        const onChain = va ? tokenAmount(va.data) : null;
        rep.coinVaults.push({ coin: vault.coin, vault: addr, book: s(book), onChain: onChain === null ? null : s(onChain) });
        if (onChain === null) rep.reasons.push(`coin ${vault.coin}'s vault ${addr} could not be read (its book: ${book} wX base units)`);
        else if (onChain < book) rep.reasons.push(`coin ${vault.coin}'s vault ${addr} holds ${onChain} wX base units, its book says ${book}`);
      }
      // the pool desk (§5): its position names the desk key; the key's SOL, wSOL and wX cover what the desk books (fees accrue above it)
      const dv = desks.get(coll);
      if (dv && dv.pair) {
        const desk: DeskReport = { pair: dv.pair, position: dv.position, desk: null, native: [s(dv.native), null], wsol: [s(dv.wsol), null], wx: [s(dv.wx), null] };
        rep.desk = desk;
        const pa = dv.position ? await d.reader.account(dv.position).catch(() => null) : null;
        const pos = pa ? decodePosition(pa) : null;
        if (dv.position && !pos) rep.reasons.push(`desk ${coll}: its position ${dv.position} is not a DLMM position`);
        else if (pos && pos.pair !== dv.pair) rep.reasons.push(`desk ${coll}: its position is on pair ${pos.pair}, the books say ${dv.pair}`);
        else if (pos && (pos.lower !== dv.lo || pos.upper !== dv.hi)) rep.reasons.push(`desk ${coll}: the position's range is [${pos.lower}, ${pos.upper}], the books say [${dv.lo}, ${dv.hi}]`);
        if (pos) {
          desk.desk = pos.owner;
          const [na, wa, xa] = await d.reader.accounts([pos.owner, ata(pos.owner, NATIVE_MINT), ata(pos.owner, wxMint)]).catch(() => [null, null, null] as (null)[]);
          const native = na ? na.lamports : null, wsol = wa ? tokenAmount(wa.data) : 0n, wx = xa ? tokenAmount(xa.data) : 0n;
          desk.native[1] = native === null ? null : s(native); desk.wsol[1] = s(wsol); desk.wx[1] = s(wx);
          if (native === null) rep.reasons.push(`desk ${coll}: the desk key ${pos.owner} could not be read (its books: ${dv.native} lamports)`);
          else if (native < dv.native) rep.reasons.push(`desk ${coll}: the desk key holds ${native} lamports, its books say ${dv.native}`);
          if (wsol < dv.wsol) rep.reasons.push(`desk ${coll}: the desk's wSOL account holds ${wsol}, its books say ${dv.wsol}`);
          if (wx < dv.wx) rep.reasons.push(`desk ${coll}: the desk's wX account holds ${wx} base units, its books say ${dv.wx}`);
        }
      }
      // the rules
      if (supply === null) rep.reasons.push(`the wX mint ${wxMint} could not be read`);
      else if (expected === null) rep.reasons.push(`the canister's books of collection ${coll} are inconsistent (no expected supply): held ${v.held}, open tickets ${v.open_tickets}, module units ${v.module_units}, SOL-backed ${v.sol_backed}`);
      else if (supply !== expected) rep.reasons.push(`wX supply ${supply} is not the books' expected supply (held ${v.held} − open tickets ${v.open_tickets}) × 10⁹ + SOL-backed ${v.sol_backed} − module units ${v.module_units} = ${expected}`);
      if (BigInt(v.items.length) !== v.held) rep.reasons.push(`the books count ${v.held} held items and list ${v.items.length}`);
      for (const it of rep.items.filter((x) => x.where === "elsewhere")) rep.reasons.push(`item ${it.itemId} (${it.key}) is not in custody: ${it.note ?? "both sources agree it is elsewhere"}`);
      const red = rep.reasons.length > 0;
      for (const it of rep.items.filter((x) => x.where === "unverified")) rep.reasons.push(`item ${it.itemId} could not be located: ${it.note ?? "the sources did not agree"}`);
      rep.status = red ? "red" : rep.reasons.length ? "unverified" : "green";
    } catch (err) {
      rep.status = "unverified"; rep.block = null;
      rep.reasons.push(`could not be read: ${String((err as Error).message).slice(0, 200)}`);
    }
  }
  const backing: BackingRead = backingError ? { kind: "error", at: d.nowS, error: backingError }
    : modules.length ? { kind: "read", at: d.nowS, modules } : { kind: "absent", at: d.nowS, note: "no collection names a module address yet: no SOL-backed units to check" };
  return { collections, backing, slot };
}

/** The listing refs of the canister-only launch pins (`deployments/launch/canister-only/pins.json`), by index. */
export function pinRefs(pins: { collections?: { index: number; slug: string; name: string; symbol: string; kind?: string }[] }): Map<number, ListingRef> {
  return new Map((pins.collections ?? []).map((c) => [c.index, { id: c.index, slug: c.slug, name: c.name, symbol: c.symbol,
    kind: /Inscriptions/.test(c.kind ?? "") ? "Inscriptions" : /SolNft/.test(c.kind ?? "") ? "SolNft" : "EthNft" }]));
}

