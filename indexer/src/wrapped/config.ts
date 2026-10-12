import { readFileSync } from "node:fs";
import type { Address, WrappedKind, WrappedMarket, WrappedSource } from "../../types.js";

/**
 * docs/v5/ux-and-reserves.md §3. Which wrapped collections this indexer reports on, from `WRAPPED_CONFIG=<file.json>`.
 * Collections are configured, never discovered: a wrapper is only reported if somebody wrote down its address, its custody and
 * where to check it. No config: every endpoint says `configured: false`.
 */
export type CollectionConfig = {
  slug: string; name: string; symbol: string; kind: WrappedKind; description?: string;
  wrapper: Address; fromBlock: number;
  factory?: Address | null; lock?: Address | null; router?: Address | null; queue?: Address | null; mintDesk?: Address | null;
  /** the Floor Module for this collection; null or absent = none deployed (docs/v5/ux-and-reserves.md §4) */
  floorModule?: Address | null;
  /** docs/v5/floor-module.md §13.3: the protocol treasury the module names; docs/v6/offer-handins.md: the collection's offer board */
  treasury?: Address | null;
  offerBoard?: Address | null;
  /** the three policy vaults (docs/v5/policies-and-fees.md §1); read from the factory when absent */
  raffleVault?: Address | null; burnVault?: Address | null; yoyoVault?: Address | null;
  /** the ListingRegistry whose keeper switch applies (fixes-B-C §0); read from `FloorModule.registry()` when absent */
  registry?: Address | null;
  custody: { kind: "bitcoin"; address: string } | { kind: "evm"; chainId: number; collection: Address; custodian: Address; providers?: string[] };
};
export type WrappedConfig = {
  chainId: number;
  /** a read older than this is stale (default 15 min) */
  maxAgeS?: number;
  /** blocks behind the 4663 head every read is made at (default 20) */
  confirmations?: number;
  /** eth_getLogs span per request (default 100,000) */
  logChunk?: number;
  collections: CollectionConfig[];
  /** set only by the fixture writer: everything served is then labelled FIXTURE */
  source?: WrappedSource;
  /** fixture only: the market the one-click ticket estimates from when there is no chain to quote on */
  market?: Record<string, WrappedMarket>;
};

/** Keyless providers with deep state, measured in docs/v4/eth-nft-canister.md; the reader's defaults when a config names none. */
export const EVM_CHAINS: Record<number, { name: string; providers: string[]; nft: (c: string, id: string) => string; address: (a: string) => string; tx: (h: string) => string }> = {
  1: { name: "Ethereum", providers: ["https://eth.drpc.org", "https://xrpc.cl/eth"], nft: (c, id) => `https://etherscan.io/nft/${c}/${id}`, address: (a) => `https://etherscan.io/address/${a}`, tx: (h) => `https://etherscan.io/tx/${h}` },
  8453: { name: "Base", providers: ["https://mainnet.base.org", "https://base.drpc.org"], nft: (c, id) => `https://basescan.org/nft/${c}/${id}`, address: (a) => `https://basescan.org/address/${a}`, tx: (h) => `https://basescan.org/tx/${h}` },
  33139: { name: "ApeChain", providers: ["https://rpc.apechain.com", "https://apechain.drpc.org"], nft: (c, id) => `https://apescan.io/nft/${c}/${id}`, address: (a) => `https://apescan.io/address/${a}`, tx: (h) => `https://apescan.io/tx/${h}` },
};
/** Robinhood Chain's explorer (web/src/lib/explorer.ts, measured 2026-09-21). */
export const HOME_EXPLORER: Record<number, string> = { 4663: "https://explorer.mainnet.chain.robinhood.com" };
/** The v4 contracts measured on 4663 (docs/v3/pools.md §1). */
export const V4_4663 = { poolManager: "0x8366a39CC670B4001A1121B8F6A443A643e40951", v4Quoter: "0x8dc178efb8111bb0973dd9d722ebeff267c98f94", stateView: "0xf3334192d15450cdd385c8b70e03f9a6bd9e673b" } as const;

const ADDR = /^0x[0-9a-fA-F]{40}$/;
export function parseConfig(raw: unknown): WrappedConfig {
  const c = raw as WrappedConfig;
  if (!c || !Number.isInteger(c.chainId) || !Array.isArray(c.collections)) throw new Error("a wrapped config needs chainId and collections[]");
  const slugs = new Set<string>();
  for (const x of c.collections) {
    if (!/^[a-z0-9-]{1,40}$/.test(x.slug ?? "")) throw new Error(`bad slug ${x.slug}`);
    if (slugs.has(x.slug)) throw new Error(`duplicate slug ${x.slug}`);
    slugs.add(x.slug);
    if (!ADDR.test(x.wrapper)) throw new Error(`${x.slug}: bad wrapper address`);
    if (x.kind !== "inscriptions" && x.kind !== "nft") throw new Error(`${x.slug}: kind must be inscriptions or nft`);
    if (x.kind === "inscriptions" && x.custody?.kind !== "bitcoin") throw new Error(`${x.slug}: inscriptions sit at a Bitcoin custody address`);
    if (x.kind === "nft" && (x.custody?.kind !== "evm" || !ADDR.test(x.custody.collection) || !ADDR.test(x.custody.custodian))) throw new Error(`${x.slug}: an NFT collection needs its chain, contract and custodian`);
    for (const k of ["factory", "lock", "router", "queue", "mintDesk", "floorModule", "treasury", "offerBoard", "raffleVault", "burnVault", "yoyoVault", "registry"] as const) if (x[k] && !ADDR.test(x[k]!)) throw new Error(`${x.slug}: bad ${k} address`);
  }
  return c;
}
export const loadConfig = (path: string): WrappedConfig => parseConfig(JSON.parse(readFileSync(path, "utf8")));
