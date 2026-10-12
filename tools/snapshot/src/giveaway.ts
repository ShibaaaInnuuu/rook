/**
 * docs/token/protocol-token-v1.md §2.1. The etched-side exclusions for a protocol-token giveaway, read from the registry rather
 * than typed in, so that a third party rebuilding a giveaway's root gets the same list we did without being told what it is.
 *
 * Everything here is a READ. The reader is passed in, so this module can be tested without a chain and cannot reach one by itself.
 */

/** Selectors, spelled out so a checker can verify them with `cast sig` rather than trust this file. */
export const SELECTOR = {
  treasury: "0x61d027b3",      // treasury()
  launcher: "0x16eebd1e",      // launcher()
  distributor: "0xbfe10928",   // distributor()
  count: "0x06661abd",         // count()
  vaultOf: "0xdaa0bfba",       // vaultOf(uint256)
  routerOf: "0x25356d2b",      // routerOf(address)
} as const;

/** A single `eth_call`, returning the raw hex word(s). */
export type Reader = (to: string, data: string) => Promise<string>;

const ADDRESS = /^0x[0-9a-f]{40}$/;
const pad = (hex: string) => hex.replace(/^0x/, "").toLowerCase().padStart(64, "0");
export const addressWord = (hex: string): string => `0x${hex.replace(/^0x/, "").slice(-40)}`.toLowerCase();
export const encodeUint = (n: bigint): string => n.toString(16).padStart(64, "0");

/**
 * The addresses that hold the protocol token but are not holders of it: the protocol's own plumbing. Named in full in
 * docs/token/protocol-token-v1.md §2.1; derived here so the list cannot quietly differ from the chain.
 *
 * NOT derived, because no contract knows them: the team's addresses (open question (d)) and the token's Uniswap pool. Pass those
 * with --exclusions, as any coin's snapshot already does.
 */
export async function giveawayExclusions(read: Reader, registry: string, coin: string): Promise<string[]> {
  const reg = registry.toLowerCase();
  if (!ADDRESS.test(reg)) throw new Error(`--giveaway needs the ListingRegistry address, got ${registry}`);

  const one = async (selector: string) => addressWord(await read(reg, selector));
  const [treasury, launcher, distributor] = await Promise.all([one(SELECTOR.treasury), one(SELECTOR.launcher), one(SELECTOR.distributor)]);

  // the token's own router: fees in transit are nobody's balance
  const router = launcher === "0x0000000000000000000000000000000000000000"
    ? ""
    : addressWord(await read(launcher, `${SELECTOR.routerOf}${pad(coin)}`));

  // every vault, not just the one being drawn: an asset bought for one listing is paid for out of vaults that all hold tokens
  const count = BigInt(await read(reg, SELECTOR.count));
  const vaults: string[] = [];
  for (let i = 0n; i < count; i++) vaults.push(addressWord(await read(reg, `${SELECTOR.vaultOf}${encodeUint(i)}`)));

  const all = [reg, treasury, launcher, distributor, router, ...vaults].map((a) => a.toLowerCase());
  return [...new Set(all.filter((a) => ADDRESS.test(a) && a !== "0x0000000000000000000000000000000000000000"))].sort();
}
