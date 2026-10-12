import type { PublicClient } from "viem";
import { local, robinhood, stagingChain, STAGING_ID } from "./chains";
export { local, robinhood };
import { API_BASE, fixtureSource, httpSource, type Source } from "../api/source";
import type { Health } from "../api/types";
import { netOf, pick } from "./deployments";

/** What the deploy script writes (script/Deploy.s.sol, script/deploy.sh). */
export type Deployment = { chainId: number; fromBlock: number; registry: `0x${string}`; launcher: `0x${string}`; distributor: `0x${string}`;
  beacon: `0x${string}`; ponsFactory: `0x${string}`; owner: `0x${string}`; treasury: `0x${string}`; keeper: `0x${string}`; purchaser: `0x${string}`;
  attestor: `0x${string}`; wired: boolean; demo: boolean; label?: string };


// Deployment files for PUBLIC chains are bundled at build time from src/deployments/<chainid>.json. There is none yet.
const bundled = import.meta.glob("../deployments/*.json", { eager: true, import: "default" }) as Record<string, Deployment>;
// docs/phase6.md: a file labelled "test" is a small real test and is NEVER read as the deployment, whatever chain id it carries.
const picked = pick(Object.values(bundled), robinhood.id);
export const publicDeployment: Deployment | null = picked.live;
export const testDeployment: Deployment | null = picked.test;

const viteEnv = (import.meta as unknown as { env?: Record<string, string | undefined> }).env ?? {};
/** The staging fork's public proxy and its indexer. Unset in a build = there is no staging to offer, and the page says so. */
export const STAGING_RPC: string | null = (viteEnv.VITE_ETCH_STAGING_RPC ?? "").replace(/\/+$/, "") || null;
export const STAGING_API: string | null = (viteEnv.VITE_ETCH_STAGING_API ?? "").replace(/\/+$/, "") || null;
/** `?net=staging` reads our fork; `?net=test` reads a small real test on Robinhood Chain. Both are labelled on every page. */
export const netRequested = (): "staging" | "test" | null => (typeof location === "undefined" ? null : netOf(location.search));

/** DEMO modes are asked for in the URL and labelled on every page. `?demo` reads a fork on this machine (the keeper's e2e).
 *  `?demo=sample` replays the indexer's recorded answers from that same run; `?demo=states` replays its synthetic set, invented
 *  events folded by the real indexer so that every state a page can show is on screen. Neither touches a chain. */
const param = () => (typeof location === "undefined" ? null : new URLSearchParams(location.search).get("demo"));
export const demoRequested = (): boolean => param() !== null;
export const sampleRequested = (): "demo" | "synthetic" | null => (param() === "sample" ? "demo" : param() === "states" ? "synthetic" : null);

export type Env = { mode: "none"; net?: "staging" | "test"; why?: string }
  | { mode: "live" | "demo"; deployment: Deployment; chain: typeof robinhood | typeof local | ReturnType<typeof stagingChain>; client: PublicClient; api?: Source; net?: "staging" | "test" }
  | { mode: "sample" | "api"; deployment: Deployment; chain: typeof robinhood; client: PublicClient; source: Source; net?: undefined };

const ZERO = `0x${"0".repeat(40)}` as const;
const NO_DEPLOYMENT: Deployment = { chainId: 0, fromBlock: 0, registry: ZERO, launcher: ZERO, distributor: ZERO, beacon: ZERO, ponsFactory: ZERO, owner: ZERO, treasury: ZERO,
  keeper: ZERO, purchaser: ZERO, attestor: ZERO, wired: false, demo: true };
/** A mode with no chain behind it. Anything that reaches for the client by mistake fails loudly instead of inventing a number. */
const noClient = new Proxy({}, { get: () => () => { throw new Error("there is no chain to read in this mode"); } }) as PublicClient;

/** The hosted indexer, if one is configured AND it says it has a deployment. Anything else is "no indexer". */
async function indexer(): Promise<Source | null> {
  if (!API_BASE) return null;
  try { const src = httpSource(API_BASE); const h = await src.get<Health>("/v1/health"); return h.deployment ? src : null; } catch { return null; }
}

export async function loadEnv(): Promise<Env> {
  const set = sampleRequested();
  if (set) { try { return { mode: "sample", deployment: NO_DEPLOYMENT, chain: robinhood, client: noClient, source: await fixtureSource(set, Math.floor(Date.now() / 1000)) }; } catch { return { mode: "none" }; } }
  const net = netRequested();
  if (net === "staging") {
    if (!STAGING_RPC) return { mode: "none", net, why: "This build was not told where the staging chain is." };
    try {
      const { createPublicClient, http } = await import("viem");
      const info = (await (await fetch(`${STAGING_RPC}/info`, { cache: "no-store" })).json()) as { chainId: number; deployment: Deployment | null };
      if (info.chainId !== STAGING_ID || !info.deployment) throw new Error("the staging chain has no deployment yet");
      const chain = stagingChain(STAGING_RPC);
      const client = createPublicClient({ chain, transport: http(STAGING_RPC) }) as PublicClient;
      if ((await client.getChainId()) !== STAGING_ID) throw new Error("that node is not the staging chain");
      let api: Source | undefined;
      if (STAGING_API) { try { const src = httpSource(STAGING_API); if ((await src.get<Health>("/v1/health")).deployment) api = src; } catch { /* the chain is read directly */ } }
      return { mode: "live", net, deployment: info.deployment, chain, client, ...(api ? { api } : {}) };
    } catch (e) { return { mode: "none", net, why: `The staging chain did not answer: ${String((e as Error).message).slice(0, 120)}` }; }
  }
  if (net === "test") {
    if (!testDeployment) return { mode: "none", net, why: "No test deployment is bundled in this build." };
    const { createPublicClient, http } = await import("viem");
    return { mode: "live", net, deployment: testDeployment, chain: robinhood, client: createPublicClient({ chain: robinhood, transport: http() }) as PublicClient };
  }
  if (demoRequested()) {
    try {
      const { createPublicClient, http } = await import("viem");
      const d = (await (await fetch("/local/deployment.json", { cache: "no-store" })).json()) as Deployment;
      const client = createPublicClient({ chain: local, transport: http() }) as PublicClient;
      if ((await client.getChainId()) !== local.id) throw new Error("not the local chain");
      if (((await client.getCode({ address: d.registry })) ?? "0x") === "0x") throw new Error("the local node has no such registry: run the keeper's e2e again");
      return { mode: "demo", deployment: d, chain: local, client };
    } catch { return { mode: "none" }; }
  }
  const api = await indexer();
  if (!publicDeployment) return api ? { mode: "api", deployment: NO_DEPLOYMENT, chain: robinhood, client: noClient, source: api } : { mode: "none" };
  const { createPublicClient, http } = await import("viem");
  return { mode: "live", deployment: publicDeployment, chain: robinhood, client: createPublicClient({ chain: robinhood, transport: http() }) as PublicClient, ...(api ? { api } : {}) };
}

export const BITCOIN_TX = (txid: string) => `https://mempool.space/tx/${txid.replace(/^0x/, "")}`;
export const INSCRIPTION = (id: string) => `https://ordinals.com/inscription/${id}`;
export const RUNE = (name: string) => `https://ordinals.com/rune/${encodeURIComponent(name)}`;
export const BTC_ADDRESS = (a: string) => `https://mempool.space/address/${a}`;
export const DRAND = "https://api.drand.sh/04f1e9062b8a81f848fded9c12306733282b2727ecced50032187751166ec8c3";
