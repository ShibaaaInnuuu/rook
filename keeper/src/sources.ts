import { readFileSync } from "node:fs";
import type { Fetch } from "./chain.js";

/** A price, and where it came from: the reference becomes the declaration's `capRef`, so whoever checks later knows whose word it was. */
export type Price = { sats: bigint; ref: string };
export interface PriceSource {
  readonly name: string;
  inscriptionFloor(slug: string): Promise<Price>;      // sats for the cheapest listed item of the collection
  runeUnitPrice(runeId: string, name: string): Promise<Price>;   // sats per WHOLE rune unit
}

export class FixtureSource implements PriceSource {
  readonly name = "fixture";
  private readonly data: { inscriptions: Record<string, string>; runes: Record<string, string> };
  constructor(path: string) { this.data = JSON.parse(readFileSync(path, "utf8")); }
  async inscriptionFloor(slug: string): Promise<Price> {
    const v = this.data.inscriptions[slug];
    if (v === undefined) throw new Error(`fixture has no floor for ${slug}`);
    return { sats: BigInt(v), ref: `fixture:${slug}` };
  }
  async runeUnitPrice(runeId: string): Promise<Price> {
    const v = this.data.runes[runeId];
    if (v === undefined) throw new Error(`fixture has no price for rune ${runeId}`);
    return { sats: BigInt(v), ref: `fixture:${runeId}` };
  }
}

/**
 * Magic Eden's public, keyless READ endpoints (docs/phase1.md F3; the reading of their terms is ours). At most one request per
 * `everyMs`, selected only by name, and it has no code for any endpoint that buys. The response shapes below were written from
 * general knowledge of the API and are UNVERIFIED against the live service: anything unexpected throws, and the run journals it.
 */
export class MagicEdenSource implements PriceSource {
  readonly name = "magiceden";
  private last = 0;
  constructor(private readonly fetchImpl: Fetch = fetch, private readonly everyMs = 5_000, private readonly now: () => number = Date.now,
    private readonly sleep: (ms: number) => Promise<void> = (ms) => new Promise((ok) => setTimeout(ok, ms))) {}
  private async get(path: string): Promise<unknown> {
    const wait = this.last + this.everyMs - this.now();
    if (wait > 0) await this.sleep(wait);
    this.last = this.now();
    const r = await this.fetchImpl(`https://api-mainnet.magiceden.dev/v2/ord/btc/${path}`, { headers: { accept: "application/json", "user-agent": "etch-keeper-dryrun/1 (read-only price check)" } });
    if (r.status === 429) throw new Error("magiceden: rate limited (the keyless tier is shared); not retried");
    if (!r.ok) throw new Error(`magiceden: HTTP ${r.status}`);
    return r.json();
  }
  async inscriptionFloor(slug: string): Promise<Price> {
    const j = (await this.get(`stat?collectionSymbol=${encodeURIComponent(slug)}`)) as { floorPrice?: string | number };
    if (j?.floorPrice === undefined || !/^\d+$/.test(String(j.floorPrice))) throw new Error("magiceden: no floorPrice in the collection stat");
    return { sats: BigInt(String(j.floorPrice)), ref: `magiceden:stat:${slug}` };
  }
  async runeUnitPrice(_runeId: string, name: string): Promise<Price> {
    const spaced = name.replace(/[•.\s]/g, "");
    const j = (await this.get(`runes/market/${encodeURIComponent(spaced)}/info`)) as { floorUnitPrice?: { value?: string | number } };
    const v = j?.floorUnitPrice?.value;
    if (v === undefined || !Number.isFinite(Number(v)) || Number(v) <= 0) throw new Error("magiceden: no floorUnitPrice in the rune market info");
    // sats per whole unit, rounded UP: a cap that rounds a fractional sat down could never be met
    return { sats: BigInt(Math.ceil(Number(v))), ref: `magiceden:rune-info:${spaced}` };
  }
}

/** How much native ETH on 4663 must be sent for `sats` to arrive on Bitcoin, with the route's cost inside the number. */
export type BridgeQuote = { weiIn: bigint; satsOut: bigint; tool: string; seconds: number; ref: string };
export interface Quoter { readonly name: string; quote(weiIn: bigint): Promise<BridgeQuote>; }

export class FixtureQuoter implements Quoter {
  readonly name = "fixture";
  /** satsPerEth at no cost, less a fixed cost in sats and a proportional one in bps — the shape Phase 0 measured. */
  constructor(private readonly satsPerEth: bigint, private readonly fixedSats: bigint, private readonly bps: bigint) {}
  async quote(weiIn: bigint): Promise<BridgeQuote> {
    const gross = (weiIn * this.satsPerEth) / 10n ** 18n, out = gross - (gross * this.bps) / 10_000n - this.fixedSats;
    return { weiIn, satsOut: out > 0n ? out : 0n, tool: "fixture", seconds: 1_920, ref: "fixture-quote" };
  }
}

/** LI.FI's public quote endpoint, keyless, one request per `everyMs` at most. A quote is not a fill. */
export class LifiQuoter implements Quoter {
  readonly name = "lifi";
  private last = 0;
  constructor(private readonly toBtcAddress: string, private readonly fromAddress: string, private readonly fetchImpl: Fetch = fetch, private readonly everyMs = 3_000) {}
  async quote(weiIn: bigint): Promise<BridgeQuote> {
    const wait = this.last + this.everyMs - Date.now();
    if (wait > 0) await new Promise((ok) => setTimeout(ok, wait));
    this.last = Date.now();
    const q = new URLSearchParams({ fromChain: "4663", toChain: "20000000000001", fromToken: "0x0000000000000000000000000000000000000000", toToken: "bitcoin",
      fromAmount: String(weiIn), fromAddress: this.fromAddress, toAddress: this.toBtcAddress });
    const r = await this.fetchImpl(`https://li.quest/v1/quote?${q}`, { headers: { accept: "application/json" } });
    if (!r.ok) throw new Error(`lifi: HTTP ${r.status}`);
    const j = (await r.json()) as { estimate?: { toAmountMin?: string; executionDuration?: number }; tool?: string };
    if (!j.estimate?.toAmountMin) throw new Error("lifi: no estimate in the quote");
    return { weiIn, satsOut: BigInt(j.estimate.toAmountMin), tool: j.tool ?? "unknown", seconds: j.estimate.executionDuration ?? 0, ref: `lifi:${j.tool ?? "?"}` };
  }
}
