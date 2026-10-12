import { readFileSync } from "node:fs";

/** THE staging chain id for this package, read from the repository's one source (staging/chain-id.json). */
const j = JSON.parse(readFileSync(new URL("./chain-id.json", import.meta.url), "utf8")) as { chainId: number; name: string; currency: { name: string; symbol: string; decimals: number }; never: number[] };
export const STAGING_CHAIN_ID: number = j.chainId, STAGING_NAME = j.name, STAGING_CURRENCY = j.currency, NEVER: readonly number[] = j.never;
if (!Number.isSafeInteger(STAGING_CHAIN_ID) || STAGING_CHAIN_ID <= 0 || NEVER.includes(STAGING_CHAIN_ID) || [4663, 46630].includes(STAGING_CHAIN_ID))
  throw new Error(`refused: staging/chain-id.json names chain ${STAGING_CHAIN_ID}, which is a real network's (4663 is Robinhood Chain, 46630 its public testnet)`);

/** What a node must report before anything here touches it. */
export function assertStagingChain(reported: number): void {
  if (reported === 4663) throw new Error("refused: this node is Robinhood Chain (4663)");
  if (reported === 46630) throw new Error("refused: this node is Robinhood's public testnet (46630)");
  if (reported !== STAGING_CHAIN_ID) throw new Error(`refused: this node reports chain ${reported}; staging is ${STAGING_CHAIN_ID} and nothing else`);
}
