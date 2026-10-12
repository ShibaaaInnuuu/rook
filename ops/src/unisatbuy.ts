import type { Hex } from "viem";
import { memberTree, satsForWei } from "./frogmath.js";
import type { Log } from "./log.js";

/**
 * The runner's UniSat step: for a frog the canister wants, the cheapest UniSat listing of the collection under the cap whose
 * inscription is a member, turned into UniSat's purchase PSBT (`create_bid`) built for the canister's float (paying) and custody
 * (receiving), and handed to the canister's CLASSIC path (`frog_offer`). The canister judges it by every rule a marketplace PSBT
 * must pass (frogbuy::judge: the seller's input already signed, nothing but its own inputs to sign, no OP_RETURN, the inscribed sat
 * in custody, the spend under the cap, the fee bounded), signs, and broadcasts it itself.
 *
 * It holds the UniSat API key (UNISAT_API_KEY, sent only to open-api.unisat.io) and no other secret; it signs nothing. The shapes of
 * `create_bid` are UniSat's published spec (open-api.unisat.io/swagger.yaml: MarketCreateBidRequest / MarketCreateBidResponse),
 * read 2026-10-09; the listing search was read live the same day. A bid UniSat builds without the seller's signature in it is refused
 * by the canister (its judge requires a final seller witness), and the refusal is logged: nothing is signed in that case.
 */
export const UNISAT_API_DEFAULT = "https://open-api.unisat.io";
/** One bid per frog at most this often (a bid is an order on UniSat's side, not a read). */
export const UNISAT_BID_EVERY_S = 30 * 60;
export const UNISAT_CANDIDATES = 20;

type AnyObj = Record<string, any>; // eslint-disable-line @typescript-eslint/no-explicit-any

export type UnisatBuyer = {
  /** Ordinals multi-collection: `coll`'s frog buys (`[coll]`) */
  frog_buys(coll: [] | [number]): Promise<{ id: bigint; phase: Record<string, unknown> }[]>;
  float_address(): Promise<{ Ok: string } | { Err: string }>;
  float_internal_key(): Promise<{ Ok: string } | { Err: string }>;
  custody_address(): Promise<{ Ok: string } | { Err: string }>;
  frog_offer(coll: number, id: bigint, inscription: string, psbtHex: string, toSign: number[], proof: string[]): Promise<{ Ok: string } | { Err: string }>;
};

export type UnisatDeps = {
  buyer: UnisatBuyer; fetch: typeof fetch; log: Log; collectionId: string; members: readonly string[]; apiKey: () => string | undefined;
  /** Ordinals multi-collection: the books id of the collection (`frog_collections().coll`); `members` is its members file */
  coll: number;
  api?: string; satsPerEth: () => Promise<bigint>; feeRate: () => Promise<number>; nowS?: () => number;
  /** frog id → when its last bid was made (seconds), kept per process */
  lastBid?: Map<string, number>;
};

const okText = (r: { Ok: string } | { Err: string }, what: string): string => { if ("Ok" in r) return r.Ok; throw new Error(`${what}: ${r.Err}`); };

async function post(d: UnisatDeps, path: string, body: unknown): Promise<AnyObj> {
  const key = d.apiKey();
  if (!key) throw new Error("UNISAT_API_KEY is not set");
  const r = await d.fetch(`${d.api ?? UNISAT_API_DEFAULT}${path}`, { method: "POST", signal: AbortSignal.timeout(20_000),
    headers: { authorization: `Bearer ${key}`, "content-type": "application/json", "user-agent": "rook-runner" }, body: JSON.stringify(body) });
  const text = await r.text();
  let j: AnyObj;
  try { j = JSON.parse(text); } catch { throw new Error(`${path}: HTTP ${r.status}, not JSON`); }
  if (!r.ok || j.code !== 0) throw new Error(`${path}: HTTP ${r.status} code ${String(j.code)} ${String(j.msg ?? "").slice(0, 160)}`);
  return j.data ?? {};
}

/** A PSBT as UniSat may answer it (hex, or base64), as hex. */
export function psbtHexOf(s: string): string {
  const t = s.trim();
  if (/^70736274ff[0-9a-f]+$/i.test(t)) return t.toLowerCase();
  const b = Buffer.from(t, "base64");
  if (b.length > 5 && b.subarray(0, 5).toString("hex") === "70736274ff") return b.toString("hex");
  throw new Error("the bid is not a PSBT (neither hex nor base64)");
}

const capOf = (phase: Record<string, unknown>): bigint | null => {
  const w = phase.Wanted as { cap_wei?: bigint } | undefined;
  return w && typeof w.cap_wei === "bigint" ? w.cap_wei : null;
};

/** One cycle. Returns true when it handed a listing to the canister. */
export async function unisatOnce(d: UnisatDeps): Promise<boolean> {
  const lastBid = (d.lastBid ??= new Map()), now = d.nowS ? d.nowS() : Math.floor(Date.now() / 1000);
  const wanted = (await d.buyer.frog_buys([d.coll])).find((x) => capOf(x.phase) !== null);
  if (!wanted) return false;
  const last = lastBid.get(String(wanted.id));
  if (last !== undefined && now - last < UNISAT_BID_EVERY_S) return false;
  const capSats = satsForWei(capOf(wanted.phase)!, await d.satsPerEth());
  const member = memberTree(d.members);
  const page = await post(d, "/v3/market/collection/auction/list", { filter: { nftType: "collection", collectionId: d.collectionId, isEnd: false },
    sort: { unitPrice: 1 }, start: 0, limit: UNISAT_CANDIDATES });
  const rows = ((page.list ?? []) as AnyObj[])
    .filter((x) => x.marketType === "fixedPrice" && member.has(String(x.inscriptionId)) && Number(x.price) > 0 && BigInt(x.price) <= capSats)
    .sort((a, b) => Number(BigInt(a.price) - BigInt(b.price)));
  if (!rows.length) { d.log("info", "unisat-none", { coll: d.coll, frog: wanted.id, capSats, collection: d.collectionId }); return false; }
  const [from, pubkey, to, feeRate] = [okText(await d.buyer.float_address(), "float_address").split(/\s+/)[0],
    okText(await d.buyer.float_internal_key(), "float_internal_key"), okText(await d.buyer.custody_address(), "custody_address").split(/\s+/)[0],
    await d.feeRate()];
  const row = rows[0], id = String(row.inscriptionId);
  lastBid.set(String(wanted.id), now);                    // one bid per frog per UNISAT_BID_EVERY_S, whatever happens next
  try {
    const bid = await post(d, "/v3/market/collection/auction/create_bid", { auctionId: String(row.auctionId), bidPrice: Number(row.price),
      address: from, pubkey, feeRate, nftAddress: to });
    if (!bid.psbtBid) throw new Error("the bid carries no psbtBid");
    const psbt = psbtHexOf(String(bid.psbtBid));
    const toSign: number[] = Array.isArray(bid.bidSignIndexes) ? bid.bidSignIndexes.map(Number) : [];
    if (!toSign.length || !toSign.every((i) => Number.isInteger(i) && i >= 0)) throw new Error("the bid names no inputs to sign");
    const proof = member.proof(id);
    const r = await d.buyer.frog_offer(d.coll, wanted.id, id, psbt, toSign, proof.map((h: Hex) => h.slice(2)));
    if ("Ok" in r) { d.log("info", "unisat-handed-in", { coll: d.coll, frog: wanted.id, inscription: id, price: String(row.price), bid: bid.bidId ?? null, note: r.Ok }); return true; }
    d.log("warn", "unisat-hand-in-refused", { frog: wanted.id, inscription: id, why: r.Err.slice(0, 240) });
  } catch (e) {
    d.log("warn", "unisat-candidate-failed", { frog: wanted.id, inscription: id, error: String((e as Error).message).slice(0, 240) });
  }
  return false;
}

/** sat/vB to ask for: mempool.space's half-hour estimate, at least 1, at most 50 (the canister's fee bound refuses a dearer buy anyway). */
export function liveFeeRate(f: typeof fetch): () => Promise<number> {
  return async () => {
    const j = await f("https://mempool.space/api/v1/fees/recommended", { signal: AbortSignal.timeout(15_000) }).then((r) => r.json());
    const r = Number(j?.halfHourFee ?? j?.fastestFee);
    if (!Number.isFinite(r) || r <= 0) throw new Error("no fee estimate");
    return Math.min(Math.max(1, Math.ceil(r)), 50);
  };
}
