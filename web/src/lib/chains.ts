import type { Chain } from "viem";

/** Plain objects (what viem's defineChain returns), so the landing page does not pull viem in to describe a chain. */
export const robinhood = { id: 4663, name: "Robinhood Chain", nativeCurrency: { name: "Ether", symbol: "ETH", decimals: 18 },
  rpcUrls: { default: { http: ["https://rpc.mainnet.chain.robinhood.com"] } } } as const satisfies Chain;
/** docs/phase6.md: our hosted fork of Robinhood Chain. Its own chain id, so nothing signed for it can be replayed on 4663. */
/** THE one constant in this package. It equals staging/chain-id.json (test/net.test.ts compares them); never 4663, never 46630 (Robinhood's public testnet). */
export const STAGING_ID = 9466301;
export const STAGING_NAME = "etched staging";
/** Named so that a wallet cannot show it as money: sETH, "staging ETH". */
export const STAGING_CURRENCY = { name: "staging ETH", symbol: "sETH", decimals: 18 } as const;
export const stagingChain = (rpc: string) => ({ id: STAGING_ID, name: STAGING_NAME, nativeCurrency: STAGING_CURRENCY,
  rpcUrls: { default: { http: [rpc] } } }) as const satisfies Chain;
export const local = { id: 31337, name: "Local fork (demo)", nativeCurrency: { name: "Ether", symbol: "ETH", decimals: 18 },
  rpcUrls: { default: { http: ["http://127.0.0.1:8545"] } } } as const satisfies Chain;
