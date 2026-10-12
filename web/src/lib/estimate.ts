/** docs/phase1.md F1: what reaches a coin's router is 0.70% of volume plus the creator tax, in ETH, on the curve and in the pool
 *  alike (measured on a fork and against eight real coins, each within ~2%). The vault receives 80% of that. */
export const MEASURED_BASE_TAKE_BPS = 70n;      // 0.70%
export const VAULT_SHARE_BPS = 8_000n;          // the router's constant

export type NextBuy = { shortfallWei: bigint; volumeWei: bigint | null; perEthToVaultWei: bigint; why: string };

/**
 * How much more trading it takes before the vault can afford one more item under the posted cap.
 *   shortfall = cap − spendable            (spendable: attributed − reserved, shared by every coin on the listing)
 *   volume    = shortfall ÷ (0.80 × (0.70% + creator tax))
 * An estimate, and the page says so: it assumes the measured rate, a cap that does not move, that Pons's operator keeps sweeping
 * pools, and that no OTHER coin on the same listing contributes. `volumeWei` is null when there is no live cap to measure against.
 */
export function volumeUntilNextBuy(o: { capWei: bigint; capExpiry: number; nowS: number; spendableWei: bigint; creatorTaxBps: number }): NextBuy {
  const takeBps = MEASURED_BASE_TAKE_BPS + BigInt(o.creatorTaxBps);
  const perEthToVaultWei = (10n ** 18n * takeBps * VAULT_SHARE_BPS) / (10_000n * 10_000n);
  if (o.capWei === 0n) return { shortfallWei: 0n, volumeWei: null, perEthToVaultWei, why: "no price cap has been posted for this listing, so there is nothing to measure against" };
  if (o.capExpiry <= o.nowS) return { shortfallWei: 0n, volumeWei: null, perEthToVaultWei, why: "the last price cap has expired; the keeper posts a fresh one before any purchase" };
  const shortfallWei = o.capWei > o.spendableWei ? o.capWei - o.spendableWei : 0n;
  const volumeWei = shortfallWei === 0n ? 0n : (shortfallWei * 10_000n * 10_000n + takeBps * VAULT_SHARE_BPS - 1n) / (takeBps * VAULT_SHARE_BPS);
  return { shortfallWei, volumeWei, perEthToVaultWei, why: shortfallWei === 0n ? "the vault already holds enough for one item under the cap" : "" };
}
