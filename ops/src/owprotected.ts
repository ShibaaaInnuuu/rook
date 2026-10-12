import type { Hex } from "viem";
import { memberTree, satsForWei } from "./frogmath.js";
import type { Log } from "./log.js";

/**
 * docs/v8/ow-protected.md §4: the runner's half of buying an Ordinals Wallet PROTECTED listing (passthrough v4). For a frog the canister
 * wants (`Wanted { cap_wei }`), it finds the cheapest protected listing of the collection under the cap whose inscription is a member,
 * asks Ordinals Wallet to build the sale for the canister's float (paying) and custody (receiving), and hands the build to the canister
 * (`frog_offer_protected`), which judges it by its own rules and signs only its own inputs. Once the canister has signed (the frog is
 * `Handed`), it reads the signed PSBTs (`frog_protected_signed`) and submits them; Ordinals Wallet co-signs and broadcasts.
 *
 * It holds no key and signs nothing. Everything it hands in is judged again by the canister: this step only picks and carries.
 * Ordinals Wallet's API needs no key (https://blog.ordinalswallet.com/docs/developers/bots/trading-from-code.md).
 */
export const OW_API_DEFAULT = "https://turbo.ordinalswallet.com";
/** Ordinals Wallet's co-signer, pinned (their docs: "if the API ever returns a different key, stop"). Also pinned in the canister. */
export const OW_COSIGNER = "1d08b7c71f6f1e97a0a4cf005db7a977c85e34652a0c9365842aee25997c7dee";
/** The fee rate asked for, at most this (sat/vB): the canister's own fee bound refuses a dearer sale anyway. */
export const OW_MAX_FEE_RATE = 50;
/** Listings tried per frog per cycle (each costs one escrow read and, for the first acceptable one, one build). */
export const OW_CANDIDATES = 5;
/** Two rates must agree this closely (bps) before the cap in sats is believed. */
export const RATE_AGREE_BPS = 100n;

type AnyObj = Record<string, any>; // eslint-disable-line @typescript-eslint/no-explicit-any

export type ProtectedListingV = { outpoint: string; seller_address: string; satoshi_price: bigint; escrow_price: [] | [bigint]; creator_address: [] | [string] };
export type ProtectedSignedV = { sale_txid: string; sale_psbt: string; setup_psbt: [] | [string] };

/** What the step asks of the canister (ops/src/ic.ts carries the IDL). */
export type OwBuyer = {
  /** Ordinals multi-collection: `coll`'s frog buys (`[coll]`), the books id of the Inscriptions listing */
  frog_buys(coll: [] | [number]): Promise<{ id: bigint; phase: Record<string, unknown> }[]>;
  float_address(): Promise<{ Ok: string } | { Err: string }>;
  float_internal_key(): Promise<{ Ok: string } | { Err: string }>;
  custody_address(): Promise<{ Ok: string } | { Err: string }>;
  frog_offer_protected(coll: number, id: bigint, inscription: string, salePsbtHex: string, parentHex: string, listing: ProtectedListingV, setupPsbtHex: [] | [string],
    proof: string[]): Promise<{ Ok: string } | { Err: string }>;
  frog_protected_signed(id: bigint): Promise<[] | [ProtectedSignedV]>;
};

export type OwDeps = {
  buyer: OwBuyer; fetch: typeof fetch; log: Log; slug: string; members: readonly string[]; api?: string;
  /** Ordinals multi-collection (docs/v9/ordinals-collections.md §4): the books id of the collection (`frog_collections().coll`) whose frogs
   * this step buys; `members` is that collection's members file (the leaves of its root) */
  coll: number;
  /** sats per ETH, the lower of two sources that agree within RATE_AGREE_BPS (or a stub in tests) */
  satsPerEth: () => Promise<bigint>;
  /** submitted sale txids, so a frog's sale is submitted once per process (Ordinals Wallet refuses a repeat anyway) */
  submitted?: Set<string>;
};

const okText = (r: { Ok: string } | { Err: string }, what: string): string => { if ("Ok" in r) return r.Ok; throw new Error(`${what}: ${r.Err}`); };

async function getJson(d: OwDeps, path: string, init?: RequestInit): Promise<AnyObj> {
  const r = await d.fetch(`${d.api ?? OW_API_DEFAULT}${path}`, { ...init, signal: AbortSignal.timeout(20_000),
    headers: { "content-type": "application/json", "user-agent": "rook-runner", ...(init?.headers ?? {}) } });
  const text = await r.text();
  let j: AnyObj;
  try { j = JSON.parse(text); } catch { throw new Error(`${path}: HTTP ${r.status}, not JSON`); }
  if (!r.ok || j?.error) throw new Error(`${path}: HTTP ${r.status} ${String(j?.code ?? "")} ${String(j?.message ?? "").slice(0, 160)}`.trim());
  return j;
}

/** The live rules, refused unless they are the ones this step and the canister were built for. */
export async function owCapabilities(d: OwDeps): Promise<void> {
  const c = (await getJson(d, "/market/secure-purchase/capabilities")).secure_purchase ?? {};
  const problems = [
    c.mode !== "passthrough_v4" && `mode ${c.mode}`,
    String(c.cosigner_public_key ?? "").toLowerCase() !== OW_COSIGNER && `co-signer ${c.cosigner_public_key} is not the pinned one`,
    !c.build_enabled && "build disabled", !c.submit_enabled && "submit disabled",
    !(c.protocols ?? []).includes("ordinal") && "ordinal purchases disabled",
  ].filter(Boolean);
  if (problems.length) throw new Error(`Ordinals Wallet's protected purchases are not as pinned: ${problems.join("; ")}`);
}

/** sat/vB to ask for: the half-hour estimate, at least 1, at most OW_MAX_FEE_RATE. */
export async function owFeeRate(d: OwDeps): Promise<number> {
  const f = await getJson(d, "/wallet/fee-estimates");
  const r = Number(f.halfHourFee ?? f.fastestFee);
  if (!Number.isFinite(r) || r <= 0) throw new Error("no fee estimate");
  return Math.min(Math.max(1, Math.ceil(r)), OW_MAX_FEE_RATE);
}

/** The lower of two ETH/BTC rates (Coinbase and Kraken), refused unless they agree; in sats per ETH. */
export function liveSatsPerEth(f: typeof fetch): () => Promise<bigint> {
  return async () => {
    const t = (u: string) => f(u, { signal: AbortSignal.timeout(15_000) }).then((r) => r.json());
    const cb = Number((await t("https://api.coinbase.com/v2/prices/ETH-BTC/spot"))?.data?.amount);
    const kj = await t("https://api.kraken.com/0/public/Ticker?pair=XETHXXBT");
    const kr = Number((Object.values(kj?.result ?? {})[0] as AnyObj)?.c?.[0]);
    if (!(cb > 0) || !(kr > 0)) throw new Error("an ETH/BTC rate did not answer");
    const a = BigInt(Math.floor(cb * 1e8)), b = BigInt(Math.floor(kr * 1e8));
    const lo = a < b ? a : b, hi = a < b ? b : a;
    if ((hi - lo) * 10_000n > lo * RATE_AGREE_BPS) throw new Error(`ETH/BTC disagrees: Coinbase ${cb}, Kraken ${kr}`);
    return lo;
  };
}

/**
 * docs/v9/solana-migration.md §2.3, the Solana home: a frog's cap (`Wanted.cap_wei`) is in LAMPORTS and the canister's frog rate is
 * satoshis per 10^18 lamports (frogsol.rs, `rates::sats_per_home_unit`), so `satsForWei` reads the same on both homes. The lower of two
 * SOL/BTC rates (Coinbase and Kraken), refused unless they agree; in sats per 10^18 lamports (sats per SOL × 10^9).
 */
export function liveSatsPerSolHomeUnit(f: typeof fetch): () => Promise<bigint> {
  return async () => {
    const t = (u: string) => f(u, { signal: AbortSignal.timeout(15_000) }).then((r) => r.json());
    const cb = Number((await t("https://api.coinbase.com/v2/prices/SOL-BTC/spot"))?.data?.amount);
    const kj = await t("https://api.kraken.com/0/public/Ticker?pair=SOLXBT");
    const kr = Number((Object.values(kj?.result ?? {})[0] as AnyObj)?.c?.[0]);
    if (!(cb > 0) || !(kr > 0)) throw new Error("a SOL/BTC rate did not answer");
    const a = BigInt(Math.floor(cb * 1e8)), b = BigInt(Math.floor(kr * 1e8));
    const lo = a < b ? a : b, hi = a < b ? b : a;
    if ((hi - lo) * 10_000n > lo * RATE_AGREE_BPS) throw new Error(`SOL/BTC disagrees: Coinbase ${cb}, Kraken ${kr}`);
    return lo * 1_000_000_000n;
  };
}

const capOf = (phase: Record<string, unknown>): bigint | null => {
  const w = phase.Wanted as { cap_wei?: bigint } | undefined;
  return w && typeof w.cap_wei === "bigint" ? w.cap_wei : null;
};

/**
 * One cycle: submit any sale the canister has signed and not yet seen submitted, then, for the first frog Wanted, hand in one protected
 * listing. Returns true when it handed something to the canister or to Ordinals Wallet (the runner then ticks again at once).
 */
export async function owProtectedOnce(d: OwDeps): Promise<boolean> {
  const submitted = (d.submitted ??= new Set());
  const frogs = await d.buyer.frog_buys([d.coll]);
  let acted = false;

  // 1. the canister signed: submit (Ordinals Wallet co-signs and relays the package)
  for (const f of frogs.filter((x) => "Handed" in x.phase)) {
    const s = await d.buyer.frog_protected_signed(f.id);
    if (!s.length || submitted.has(s[0].sale_txid)) continue;
    const link: AnyObj = { sale_txid: s[0].sale_txid, psbt: s[0].sale_psbt, ...(s[0].setup_psbt.length ? { setup_psbt: s[0].setup_psbt[0] } : {}) };
    try {
      const r = await getJson(d, "/market/secure-purchase/submit", { method: "POST", body: JSON.stringify({ sales: [link] }) });
      submitted.add(s[0].sale_txid);
      d.log("info", "ow-submitted", { frog: f.id, sale: s[0].sale_txid, accepted: r.accepted ?? null, txid: r.txid ?? null, state: r.state ?? null });
      acted = true;
    } catch (e) {
      // the canister abandons a sale unseen for 6 h (its own conflict path); here it is only logged and retried next cycle
      d.log("warn", "ow-submit-failed", { frog: f.id, sale: s[0].sale_txid, error: String((e as Error).message).slice(0, 240) });
    }
  }

  // 2. a frog wanted: one protected listing under its cap
  const wanted = frogs.find((x) => capOf(x.phase) !== null);
  if (!wanted) return acted;
  await owCapabilities(d);
  const capSats = satsForWei(capOf(wanted.phase)!, await d.satsPerEth());
  const member = memberTree(d.members);
  const rows = (await getJson(d, `/collection/${encodeURIComponent(d.slug)}/escrows`)) as unknown as AnyObj[];
  const list = (Array.isArray(rows) ? rows : ((rows as AnyObj).escrows ?? []) as AnyObj[])
    .filter((x) => x.protected === true && member.has(String(x.inscription_id)) && BigInt(x.satoshi_price ?? 0) > 0n && BigInt(x.satoshi_price) <= capSats)
    .sort((a, b) => Number(BigInt(a.satoshi_price) - BigInt(b.satoshi_price)))
    .slice(0, OW_CANDIDATES);
  if (!list.length) { d.log("info", "ow-none", { coll: d.coll, frog: wanted.id, capSats, slug: d.slug }); return acted; }

  const [from, publicKey, to, feeRate] = [okText(await d.buyer.float_address(), "float_address").split(/\s+/)[0],
    okText(await d.buyer.float_internal_key(), "float_internal_key"), okText(await d.buyer.custody_address(), "custody_address").split(/\s+/)[0], await owFeeRate(d)];
  for (const row of list) {
    const id = String(row.inscription_id);
    try {
      const one = await getJson(d, `/market/escrow/${encodeURIComponent(id)}`);
      if (one.protected !== true || one.secure_purchase_state !== "listed") continue;
      const outpoint = String(row.outpoint);
      const build = await getJson(d, "/wallet/secure-purchase/build", { method: "POST",
        body: JSON.stringify({ outpoints: [outpoint], protocol: "ordinal", from, public_key: publicKey, to, fee_rate: feeRate }) });
      if (String(build.cosigner_public_key ?? OW_COSIGNER).toLowerCase() !== OW_COSIGNER) throw new Error("the build names another co-signer");
      const sale = build.sales?.[0];
      if (!sale?.psbt || !sale?.parent?.raw || build.sales.length !== 1) throw new Error("the build is not one sale with its parent");
      const listing: ProtectedListingV = { outpoint, seller_address: String(one.seller_address), satoshi_price: BigInt(one.satoshi_price),
        escrow_price: one.escrow_price == null ? [] : [BigInt(one.escrow_price)],  // review P-05: never filled in
        creator_address: one.creator_address ? [String(one.creator_address)] : [] };
      const proof = member.proof(id);
      const r = await d.buyer.frog_offer_protected(d.coll, wanted.id, id, String(sale.psbt), String(sale.parent.raw), listing,
        build.setup?.psbt ? [String(build.setup.psbt)] : [], proof.map((h: Hex) => h.slice(2)));
      if ("Ok" in r) { d.log("info", "ow-handed-in", { coll: d.coll, frog: wanted.id, inscription: id, price: String(listing.satoshi_price), sale: sale.sale_txid, note: r.Ok }); return true; }
      d.log("warn", "ow-hand-in-refused", { frog: wanted.id, inscription: id, why: r.Err.slice(0, 240) });
    } catch (e) {
      d.log("warn", "ow-candidate-failed", { frog: wanted.id, inscription: id, error: String((e as Error).message).slice(0, 240) });
    }
  }
  return acted;
}
