import type { CatalogChain, CatalogRule, CatalogStandard, RookStatus } from "./catalogTypes.js";

/**
 * docs/v6/collections-catalog.md §3: the listing rule, pure. Which stage Rook is at, what that stage's caps are
 * (docs/v5/launch-gates.md G7), and what one collection's floor makes of it.
 *
 * Where the items live decides first: Bitcoin (the canister), Ethereum (the home-chain vault over the native bridge) and
 * Robinhood Chain itself (a vault on 4663, no bridge at all: docs/v4/nft.md §1) can be held; Base has no native route and
 * Solana, HyperEVM and the rest are not chains a vault here exists on. Then the standard: one ERC-721, CryptoPunks, or an
 * inscription collection. Then the floor against the stage's caps.
 *
 * The rule in one line: one item at the floor, with the keeper's 20% allowance over it, must fit BOTH the stage's per-item
 * maximum AND the stage's per-collection cap — `1.2 × floor ≤ min(perItemMax, perCollectionCap)`. Stage 0's numbers are
 * 10 ETH and 10 ETH, so a floor above 8.333 ETH is `later`; stage 1's per-item maximum is 1.2 × floor and its collection cap
 * 40 ETH, so a floor up to 33.333 ETH fits there; 133.333 ETH at stage 2; 533.333 ETH at stage 3. The allowance is the
 * per-item maximum the table sets from stage 1 ("≤ 1.2 × floor") and the keeper's cap step bound (custody-hardening H12).
 */
const E = 10n ** 18n;
export type Stage = { stage: number; name: string; perCollectionItems: number; perCollectionCapWei: bigint; /** null: 1.2 × floor, the allowance alone */ perItemMaxWei: bigint | null; globalItems: number; globalCapWei: bigint };
/** docs/v5/launch-gates.md G7, verbatim: every stage is ×4 the one before. */
export const STAGES: readonly Stage[] = [
  { stage: 0, name: "canary", perCollectionItems: 3, perCollectionCapWei: 10n * E, perItemMaxWei: 10n * E, globalItems: 6, globalCapWei: 10n * E },
  { stage: 1, name: "stage 1", perCollectionItems: 12, perCollectionCapWei: 40n * E, perItemMaxWei: null, globalItems: 24, globalCapWei: 40n * E },
  { stage: 2, name: "stage 2", perCollectionItems: 48, perCollectionCapWei: 160n * E, perItemMaxWei: null, globalItems: 96, globalCapWei: 160n * E },
  { stage: 3, name: "stage 3", perCollectionItems: 192, perCollectionCapWei: 640n * E, perItemMaxWei: null, globalItems: 384, globalCapWei: 640n * E },
];
/** Nothing is deployed (README): the first stage anything could open at is the canary. Read from here by the catalogue. */
export const CURRENT_STAGE = 0;
export const ALLOWANCE_BPS = 2_000;
const RULE_DOC = "docs/v5/launch-gates.md G7; docs/v6/collections-catalog.md §3";

/** The highest floor a stage admits: the tighter of its per-item maximum and its collection cap, less the allowance. */
export function maxFloorWei(s: Stage, allowanceBps = ALLOWANCE_BPS): bigint {
  const bound = s.perItemMaxWei === null ? s.perCollectionCapWei : s.perItemMaxWei < s.perCollectionCapWei ? s.perItemMaxWei : s.perCollectionCapWei;
  return (bound * 10_000n) / (10_000n + BigInt(allowanceBps));
}
export const weiOfEth = (eth: number): bigint => BigInt(Math.round(eth * 1e6)) * 10n ** 12n;
const ethText = (wei: bigint, digits = 3) => { const s = (Number(wei / 10n ** 12n) / 1e6).toFixed(digits); return s.replace(/\.?0+$/, "") || "0"; };

export function ruleOf(stageNo = CURRENT_STAGE, allowanceBps = ALLOWANCE_BPS): CatalogRule {
  const s = STAGES[stageNo];
  const max = maxFloorWei(s, allowanceBps);
  const perItem = s.perItemMaxWei ?? ((s.perCollectionCapWei * 10_000n) / (10_000n + BigInt(allowanceBps)));
  return {
    stage: s.stage, stageName: s.name, perItemMaxWei: String(perItem), perCollectionCapWei: String(s.perCollectionCapWei), perCollectionItems: s.perCollectionItems, allowanceBps,
    maxFloorWei: String(max),
    text: `A collection is listable when one item at its floor, with the keeper's ${allowanceBps / 100}% allowance, fits the stage's per-item maximum and its per-collection cap: `
      + `${(1 + allowanceBps / 10_000).toFixed(1)} × floor ≤ min(${ethText(perItem)} ETH, ${ethText(s.perCollectionCapWei)} ETH) at stage ${s.stage} (${s.name}), i.e. a floor of at most ${ethText(max)} ETH. `
      + `The stage holds ${s.perCollectionItems} items a collection and ${s.globalItems} in all.`,
    doc: RULE_DOC,
  };
}

/** The first stage whose caps hold one item at this floor; null when even stage 3's do not. */
export function stageFor(floorWei: bigint, allowanceBps = ALLOWANCE_BPS): number | null {
  for (const s of STAGES) if (floorWei <= maxFloorWei(s, allowanceBps)) return s.stage;
  return null;
}

export type StatusInput = { chain: CatalogChain; chainName: string; standard: CatalogStandard; floorWei: bigint | null; floorStale: boolean; live: { slug: string; coins: number } | null };

/** Where a collection stands, and why, in the order the reasons bind: listed already; a chain or standard no vault holds; the floor against the caps. */
export function statusOf(x: StatusInput, stageNo = CURRENT_STAGE, allowanceBps = ALLOWANCE_BPS): RookStatus {
  if (x.live) return { kind: "live", slug: x.live.slug, coins: x.live.coins, why: `Listed on Rook as a nest${x.live.coins ? `, with ${x.live.coins} coin${x.live.coins === 1 ? "" : "s"} sweeping it` : ""}.` };
  if (x.chain === "base") return { kind: "unsupported", why: "Base has no native bridge route to Robinhood Chain: the home-chain vault needs one (docs/v6/home-chain-vault.md §2.3, §8.3), and a fast bridge is a different trust question that is not built. About a week down and two up through Ethereum is not a route a coin can trade against." };
  if (x.chain === "other") return { kind: "unsupported", why: `${x.chainName} is not a chain a vault here can hold on: the home-chain vault is an EVM contract with a native bridge to Robinhood Chain (docs/v6/home-chain-vault.md §2), and the canister holds Bitcoin only.` };
  if (x.standard === "erc1155") return { kind: "unsupported", why: "An ERC-1155: the vault and the floor feed hold and price exactly one ERC-721 at a time (docs/v5/floor-feed.md L-extras), and an edition token is not one item." };
  if (x.standard === "other") return { kind: "unsupported", why: "Not an ERC-721, not CryptoPunks and not an inscription collection: nothing here can hold one of its items." };
  const s = STAGES[stageNo];
  if (x.floorWei === null) return { kind: "later", stage: null, why: "No floor is known right now, so the rule cannot be checked against it: not listable until a source reports one." };
  const max = maxFloorWei(s, allowanceBps);
  const withAllowance = (x.floorWei * (10_000n + BigInt(allowanceBps))) / 10_000n;
  if (x.floorWei <= max) {
    const slots = Math.min(s.perCollectionItems, Number(s.perCollectionCapWei / (withAllowance === 0n ? 1n : withAllowance)));
    return { kind: "listable", slotsNow: slots, why: `One item at the floor (${ethText(x.floorWei)} ETH, ${ethText(withAllowance)} with the allowance) fits stage ${s.stage}'s ${ethText(s.perItemMaxWei ?? s.perCollectionCapWei)} ETH per item and ${ethText(s.perCollectionCapWei)} ETH a collection: `
      + `${slots} of the stage's ${s.perCollectionItems} slots would hold${x.floorStale ? " — checked against a floor that is not from a fresh read" : ""}.` };
  }
  const at = stageFor(x.floorWei, allowanceBps);
  return { kind: "later", stage: at, why: at === null
    ? `One item at the floor (${ethText(x.floorWei)} ETH, ${ethText(withAllowance)} with the allowance) is above even stage 3's ${ethText(STAGES[3].perCollectionCapWei)} ETH collection cap.`
    : `One item at the floor (${ethText(x.floorWei)} ETH, ${ethText(withAllowance)} with the allowance) is above stage ${s.stage}'s ${ethText(max)} ETH limit; it fits from stage ${at}, whose collection cap is ${ethText(STAGES[at].perCollectionCapWei)} ETH${at === 1 ? " (4 weeks at stage 0 without incident)" : at === 2 ? " (8 more weeks at stage 1)" : " (12 more weeks at stage 2)"}.` };
}
