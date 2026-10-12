import type { WrappedCoin } from "../api/types";

/**
 * docs/v5/policies-and-fees.md §2 and §3, pure. The creator's fee tier and what it promises; the launch fee that decays.
 */
export const MIN_FEE_BPS = 100, MAX_FEE_BPS = 800, DEFAULT_FEE_BPS = 100;
/** The vault's share of the fee at the worked 40 / 30 / 5 / 5 / 20 split (docs/v5/floor-module.md §13.2). */
export const VAULT_SHARE_BPS = 4_000;
/** The curve's template defaults (docs/v5/policies-and-fees.md §2): 50% at launch, gone after 10 minutes. A launched coin's own
 *  terms come from the indexer (`WrappedCoin.launchFee`); these are only the wizard's preview. */
export const DEFAULT_LAUNCH_FEE_BPS = 5_000, DEFAULT_LAUNCH_FEE_DECAY_S = 600;

/** ETH of trading volume per item bought: item / (fee × vaultShare). Rounded up to the wei. */
export function volumePerItem(itemWei: bigint, feeBps: number, vaultShareBps = VAULT_SHARE_BPS): bigint {
  if (feeBps <= 0 || vaultShareBps <= 0) throw new Error("a fee tier and a vault share are positive");
  const den = BigInt(feeBps) * BigInt(vaultShareBps);
  return (itemWei * 10_000n * 10_000n + den - 1n) / den;
}

/** The extra a buy pays `elapsedS` after launch, bps: linear from `launchFeeBps` to nothing at `decayS` (FrogCurve.launchExtraBps). */
export function launchExtraBps(launchFeeBps: number, decayS: number, elapsedS: number): number {
  if (launchFeeBps <= 0 || decayS <= 0) return 0;
  const el = Math.min(Math.max(0, elapsedS), decayS);
  return Math.floor((launchFeeBps * (decayS - el)) / decayS);
}

/** What one buy pays in fee now: the tier plus the extra, bps. */
export const feeNowBps = (feeBps: number, launchFeeBps: number, decayS: number, elapsedS: number) => feeBps + launchExtraBps(launchFeeBps, decayS, elapsedS);

/** The same decay from a coin's own terms as the indexer reports them (`WrappedCoin.launchFee`), recomputed from the clock so a
 *  fixture's countdown runs; with no end known, the read's own figure. */
export function launchExtraNow(lf: WrappedCoin["launchFee"], now: number): number {
  if (!lf || lf.endsAt === null) return lf?.extraBpsNow ?? 0;
  const left = lf.endsAt - now;
  return left <= 0 ? 0 : Math.floor((lf.bps * Math.min(left, lf.decayS)) / lf.decayS);
}

/** The indexer's own word for a policy (api/types.ts, `WrappedCoin.policy`): raffle, burn, yoyo. */
export type Policy = WrappedCoin["policy"];
/** SweepVault.Policy: RAFFLE 0, BURN 1, YOYO 2. */
export const POLICY_INDEX: Record<Policy, number> = { raffle: 0, burn: 1, yoyo: 2 };
export const POLICIES: { id: Policy; title: string; line: string; more: string }[] = [
  { id: "raffle", title: "Raffle", line: "Each NFT the fees buy goes to one holder, drawn by a random round nobody can pick.", more: "Every holder is entered in proportion to what they hold. The draw is pinned to a drand round that does not exist yet." },
  { id: "burn", title: "Burn", line: "Each NFT the fees buy is burnt against the coin: fewer coins, the same vault.", more: "One whole wrapped token is retired for every NFT; the coin's supply shrinks and every remaining coin backs more." },
  { id: "yoyo", title: "YOYO", line: "Each NFT the fees buy goes back on the market at a markup; what it sells for buys the coin and burns it.", more: "Relisted at 1.1× the floor. The sale's ETH can only ever buy the coin back and send it to a dead address." },
];
