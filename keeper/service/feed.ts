import { readFileSync } from "node:fs";
import { robustFloor, satsToWei, type Floor } from "../src/floor.js";
import { BookDiff, CoinbaseRate, OrdinalsWalletBook, type RateSource } from "../src/floorsources.js";
import { mainnetFeed, VerifiedOrd, type OrdFeed } from "../src/ordapi.js";
import type { Logger } from "../src/log.js";
import type { Fetch } from "./rpc.js";

/**
 * docs/v6/p1-services.md §1.4. Where the posted floor comes from, and nowhere else: the existing feed (docs/v5/floor-feed.md,
 * docs/v6/ordinals-apis.md) — Ordinals Wallet's book, every listed outpoint's contents believed only when two ordinals providers
 * agree at one height (`VerifiedOrd` over `mainnetFeed()`; unverified is L-unverified), the keeper's own book diff for list/delist
 * events, `robustFloor`'s rules — converted to wei at a rate two keyless references agree on. No agreement anywhere: no floor.
 */
export type FeedResult = { floor: Floor | null; why: string; ref: string; satsPerEth: string | null };
export interface FloorFeed { readonly name: string; floor(slug: string): Promise<FeedResult>; }

// ---- the rate: two references, agreeing, or none ------------------------------------------------------------------------------------

/** Kraken's public ticker, keyless: the last ETH/XBT trade (BTC per ETH), as sats per ETH. */
export class KrakenRate implements RateSource {
  readonly name = "kraken";
  constructor(private readonly fetchImpl: Fetch = fetch) {}
  async satsPerEth(): Promise<bigint> {
    const r = await this.fetchImpl("https://api.kraken.com/0/public/Ticker?pair=ETHXBT", { headers: { accept: "application/json", "user-agent": "rook-keeper/1 (read-only)" } });
    if (!r.ok) throw new Error(`kraken: HTTP ${r.status}`);
    const j = (await r.json()) as { error?: unknown[]; result?: Record<string, { c?: unknown[] }> };
    if (Array.isArray(j.error) && j.error.length) throw new Error(`kraken: ${String(j.error[0]).slice(0, 80)}`);
    const row = j.result ? Object.values(j.result)[0] : undefined;
    const v = String(row?.c?.[0] ?? "");
    if (!/^\d+(\.\d{1,18})?$/.test(v)) throw new Error("invalid rate in Kraken's answer");
    const [i, f = ""] = v.split(".");
    const sats = (BigInt(i) * 10n ** 18n + BigInt((f + "0".repeat(18)).slice(0, 18))) / 10n ** 10n;
    if (sats <= 0n) throw new Error("invalid rate in Kraken's answer");
    return sats;
  }
}

/**
 * Both references answer and are within `maxBps` of each other: the HIGHER number of sats per ETH, which is the LOWER floor in wei
 * (wei = sats × 1e18 / satsPerEth; §1.4; audit E-09 found the lower rate picked, i.e. the higher floor). Otherwise an error.
 */
export class AgreedRate implements RateSource {
  readonly name: string;
  constructor(private readonly sources: RateSource[], private readonly maxBps: bigint) { this.name = `agreed(${sources.map((s) => s.name).join("+")})`; }
  async satsPerEth(): Promise<bigint> {
    if (this.sources.length < 2) throw new Error("no rate: two references must agree and fewer are configured");
    const got: bigint[] = [];
    for (const s of this.sources) {
      try { got.push(await s.satsPerEth()); } catch (err) { throw new Error(`no rate: ${s.name} did not answer (${String((err as Error).message).slice(0, 120)})`); }
    }
    const lo = got.reduce((a, b) => (b < a ? b : a)), hi = got.reduce((a, b) => (b > a ? b : a));
    if (((hi - lo) * 10_000n) / lo > this.maxBps) throw new Error(`no rate: ${this.sources.map((s, i) => `${s.name} ${got[i]}`).join(" vs ")} sats/ETH differ by more than ${this.maxBps} bps`);
    return hi;
  }
}

// ---- the production feed ---------------------------------------------------------------------------------------------------------

export class OrdinalsWalletFeed implements FloorFeed {
  readonly name = "ordinalswallet+verified-ord";
  private readonly diff = new BookDiff();
  private readonly book: OrdinalsWalletBook;
  private readonly ord: VerifiedOrd;
  constructor(private readonly o: { book?: OrdinalsWalletBook; ord?: OrdFeed; rate: RateSource; slugs: Record<string, string>; log?: Logger; nowS?: () => number }) {
    this.book = o.book ?? new OrdinalsWalletBook();
    this.ord = new VerifiedOrd(o.ord ?? mainnetFeed({ log: o.log }));   // one feed for the life of the service: a 429's parking is remembered
  }
  async floor(slug: string): Promise<FeedResult> {
    const ow = this.o.slugs[slug] ?? slug, ref = `ordinalswallet:${ow}`;
    let satsPerEth: bigint;
    try { satsPerEth = await this.o.rate.satsPerEth(); } catch (err) { return { floor: null, why: String((err as Error).message).slice(0, 300), ref, satsPerEth: null }; }
    let b;
    try { b = await this.book.book(ow, this.ord); } catch (err) { return { floor: null, why: `the book could not be read: ${String((err as Error).message).slice(0, 200)}`, ref, satsPerEth: String(satsPerEth) }; }
    const events = this.diff.observe(ow, b);
    const f = robustFloor({ listings: b.listings, sales: b.sales, events, snapshotAt: this.o.nowS ? this.o.nowS() : b.snapshotAt });
    if (f.price === null) {
      const unverified = f.excluded.filter((x) => x.rule === "L-unverified").length;
      return { floor: null, ref, satsPerEth: String(satsPerEth),
        why: `no floor: fewer than 3 valid listings (${f.excluded.length} excluded, ${unverified} of them because two ordinals providers did not agree on the listed output)` };
    }
    const wei: Floor = { ...f, price: satsToWei(f.price, satsPerEth), listingFloor: f.listingFloor === null ? null : satsToWei(f.listingFloor, satsPerEth),
      saleFloor: f.saleFloor === null ? null : satsToWei(f.saleFloor, satsPerEth), upCost10: f.upCost10 === null ? null : satsToWei(f.upCost10, satsPerEth) };
    return { floor: wei, why: `floor ${f.price} sats at ${satsPerEth} sats/ETH, confidence ${f.confidenceBps} bps`, ref, satsPerEth: String(satsPerEth) };
  }
}

export const productionRate = (maxBps: bigint, fetchImpl: Fetch = fetch): AgreedRate => new AgreedRate([new CoinbaseRate(fetchImpl), new KrakenRate(fetchImpl)], maxBps);

// ---- the fixture feed: tests and local forks only (refused for a send on 4663: service/config.ts) -------------------------------

type FixtureRow = { priceWei: string; confidenceBps: number; depthItems: number };
export class FixtureFeed implements FloorFeed {
  readonly name = "fixture";
  constructor(private readonly path: string) {}
  async floor(slug: string): Promise<FeedResult> {
    const rows = (JSON.parse(readFileSync(this.path, "utf8")) as { collections?: Record<string, FixtureRow> }).collections ?? {};
    const r = rows[slug];
    if (!r) return { floor: null, why: `the fixture has no floor for ${slug}`, ref: `fixture:${slug}`, satsPerEth: null };
    const price = BigInt(r.priceWei);
    return { floor: { price, listingFloor: price, saleFloor: null, confidenceBps: r.confidenceBps, depthItems: r.depthItems, upCost10: null, used: [], salesUsed: [], excluded: [] },
      why: "FIXTURE floor (a local fork or a test)", ref: `fixture:${slug}`, satsPerEth: null };
  }
}
