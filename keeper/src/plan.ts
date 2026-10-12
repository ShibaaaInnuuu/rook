import type { ListingState } from "./chain.js";
import type { BridgeQuote, Price } from "./sources.js";

/** docs/keeper-dryrun-v1.md, "The policy it applies". Pure: state, a price and a quote in; what a keeper WOULD do out. */
export const POLICY = {
  capMarginBps: 500n,            // the cap sits this far above the converted floor
  repostIfUnderS: 15 * 60,       // re-post a cap with less than this left
  repostIfMovedBps: 200n,        // or one that differs from the new cap by more than this
  minDeclarationWei: 5n * 10n ** 16n,   // 0.05 ETH: under it the bridge's fixed costs pass ~1% (docs/phase0.md §3.1)
  maxInscriptions: 3n,
  maxOpen: 3,
} as const;
export type Policy = typeof POLICY;

export type Action =
  | { type: "skip"; rule: string; why: string }
  | { type: "postCap"; rule: "cap"; capWei: string; why: string }
  | { type: "keepCap"; rule: "cap"; capWei: string; why: string }
  | { type: "declare"; rule: "declaration"; maxEth: string; quantity: string; capRef: string; why: string }
  | { type: "wouldBuy"; rule: "purchase"; venue: string; asset: string; quantity: string; expectSats: string; bridgeWei: string; bridge: string; bridgeSeconds: number; bridgeCostBps: number; why: string };

/** wei needed for `sats` to arrive, from a quote taken at some size: the quote's own rate, cost included. Rounded up. */
export const weiFor = (sats: bigint, q: BridgeQuote): bigint => {
  if (q.satsOut <= 0n) throw new Error("the quote delivers nothing at this size");
  return (sats * q.weiIn + q.satsOut - 1n) / q.satsOut;
};

export function plan(l: ListingState, price: Price, quote: BridgeQuote, nowS: number, noCostSatsPerEth: bigint | null, p: Policy = POLICY): Action[] {
  if (l.paused) return [{ type: "skip", rule: "paused", why: "the listing is paused" }];
  if (l.blockedBy !== null) return [{ type: "skip", rule: "blocked", why: `purchase ${l.blockedBy} is past its receipt deadline; the vault refuses declarations until it is settled or written off` }];
  const out: Action[] = [];
  // the cap: one item (or one whole rune unit) at the floor, in ETH through the bridge, plus the margin
  const itemWei = weiFor(price.sats, quote), capWei = itemWei + (itemWei * p.capMarginBps) / 10_000n;
  const left = l.capExpiry - nowS, moved = l.capWei === 0n ? 10_000n : ((capWei > l.capWei ? capWei - l.capWei : l.capWei - capWei) * 10_000n) / l.capWei;
  const repost = l.capWei === 0n || left < p.repostIfUnderS || moved > p.repostIfMovedBps;
  out.push(repost
    ? { type: "postCap", rule: "cap", capWei: String(capWei), why: l.capWei === 0n ? "no cap is posted" : left < p.repostIfUnderS ? `the posted cap has ${Math.max(0, left)} s left` : `the posted cap is ${moved} bps from the new one` }
    : { type: "keepCap", rule: "cap", capWei: String(l.capWei), why: `${left} s left and within ${p.repostIfMovedBps} bps` });
  const cap = repost ? capWei : l.capWei;

  if (l.open >= p.maxOpen) return [...out, { type: "skip", rule: "max-open", why: `${l.open} purchases are open, the vault's limit` }];
  if (l.spendableWei < p.minDeclarationWei) return [...out, { type: "skip", rule: "min-declaration", why: `spendable ${l.spendableWei} wei is under ${p.minDeclarationWei}: the bridge's fixed costs would pass about 1%` }];
  const unit = 10n ** BigInt(l.divisibility);
  // docs/phase3.md H2: `quantity` is a MINIMUM the purchaser is held to — a purchase that receives less cannot be closed until the
  // ETH declared for the missing units is back in the vault. The cap carries the headroom (fees, bridge, slippage), so buying at
  // the market under the cap delivers at least this many.
  // quantity: what the balance covers under the cap. Inscriptions: whole items, at most maxInscriptions. Runes: raw units.
  let quantity = l.kind === "inscriptions" ? l.spendableWei / cap : (l.spendableWei * unit) / cap;
  if (l.kind === "inscriptions" && quantity > p.maxInscriptions) quantity = p.maxInscriptions;
  if (quantity === 0n) return [...out, { type: "skip", rule: "under-one-item", why: `spendable ${l.spendableWei} wei does not cover one ${l.kind === "rune" ? "raw unit" : "item"} under the cap of ${cap}` }];
  let maxEth = l.kind === "inscriptions" ? cap * quantity : (cap * quantity) / unit;
  if (maxEth > l.spendableWei) maxEth = l.spendableWei;
  out.push({ type: "declare", rule: "declaration", maxEth: String(maxEth), quantity: String(quantity), capRef: price.ref,
    why: `${quantity} ${l.kind === "rune" ? "raw units" : "item(s)"} under a cap of ${cap} wei; the vault's bound is cap × quantity${l.kind === "rune" ? " ÷ 10^divisibility" : ""}` });
  const expectSats = l.kind === "inscriptions" ? price.sats * quantity : (price.sats * quantity) / unit;
  const bridgeWei = weiFor(expectSats, quote);
  const costBps = noCostSatsPerEth === null ? -1 : Number(10_000n - (quote.satsOut * 10n ** 18n * 10_000n) / (quote.weiIn * noCostSatsPerEth));
  out.push({ type: "wouldBuy", rule: "purchase", venue: price.ref.split(":")[0], asset: l.ref, quantity: String(quantity), expectSats: String(expectSats),
    bridgeWei: String(bridgeWei), bridge: quote.tool, bridgeSeconds: quote.seconds, bridgeCostBps: costBps,
    why: "recorded, never attempted: this package has no signer and no code for any endpoint that buys" });
  return out;
}
