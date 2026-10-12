import { robinhood } from "./chains";

/**
 * docs/phase13.md §7. The only EVM explorer this site knows of, and the one chain it is for.
 *
 * Measured 2026-09-21: `https://explorer.mainnet.chain.robinhood.com` answers `301` to `https://robinhoodchain.blockscout.com`,
 * so Robinhood Chain has a Blockscout and the first of those is the durable address for it. Nothing else is guessed. Our
 * staging fork and a local fork are chains we run and have no explorer at all, so no link is drawn on them: an address is a
 * string that copies itself. A trade row used to link to **basescan.org**, which indexes Base and has never heard of any of
 * this, and a recorded demo is a recording of our fork, so it gets no link either.
 */
export const EXPLORERS: Record<number, string> = { [robinhood.id]: "https://explorer.mainnet.chain.robinhood.com" };
export const explorerFor = (chainId: number | null | undefined, recorded = false): string | null =>
  (recorded || chainId === null || chainId === undefined ? null : EXPLORERS[chainId] ?? null);
export const EXPLORER_TX = (base: string, tx: string) => `${base}/tx/${tx}`;
export const EXPLORER_ADDRESS = (base: string, address: string) => `${base}/address/${address}`;
