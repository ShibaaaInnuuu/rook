import { createServer, type Server } from "node:http";
import { buyer, IC_HOST, type Buyer } from "./ic.js";
import { operatorIdentity } from "./identity.js";
import { envSecrets, logger, urlSecrets, type Log } from "./log.js";
import { convertOnce, RELAY_API_DEFAULT } from "./relay.js";
import { ethIdle, ethNftOnce, ethStepProblems, parseSlugs } from "./ethnft.js";
import { existsSync, readFileSync } from "node:fs";
import { liveSatsPerEth, liveSatsPerSolHomeUnit, OW_API_DEFAULT, owProtectedOnce } from "./owprotected.js";
import { liveFeeRate, unisatOnce } from "./unisatbuy.js";
import { preJudgeSolAsk, readS3 } from "./relaysol.js";
import { parseSolRpcUrls, solReader } from "./solread.js";
import { coingeckoReader, FLOOR_EVERY_DEFAULT_S, frogFloorOnce, loadInscriptionListings, productionReader, staleFloors, type Held, type InscListing } from "./solfloor.js";
import { bitcoinStepsOnce, frogProofOnce, ListingMembers, parseInscriptionsEnv, type InscEntry } from "./inscriptions.js";
import type { CapacityV, CollectionLifeV, EthStateV } from "./ic.js";
import { loadSolNftCollections, ME_API_DEFAULT, SOL_NFT_EVERY_DEFAULT_S, solNftFloorOnce, type SolNftCollection } from "./solnftfloor.js";

/**
 * docs/v6/p1-services.md §2.1 (K4). The operator's runner: the canister's `tick` has no timer and is operator-only
 * (canister.rs:1129), so nothing is bought, delivered or reported until this calls it. One call at a time (the next waits for the
 * previous; the canister's own guard refuses an overlap anyway), every RUNNER_TICK_EVERY_S seconds (default 600: docs/deploy/
 * canister-mainnet.md prices an idle loop in cycles), every answer logged line by line. The identity is read in src/identity.ts
 * alone; only its principal is ever printed.
 *
 *   OPERATOR_IDENTITY_PEM  (secret) the operator's hot identity   CANISTER_ID  the buyer canister   IC_HOST  (default icp-api.io)
 *   RUNNER_TICK_EVERY_S    (default 600, at least 60)             PORT         GET /healthz, /status
 *   RUNNER_PENDING_EVERY_S (default 120, 60 to 300 and at most RUNNER_TICK_EVERY_S) the PERIOD, from the start of the last cycle, while
 *                          the canister's Ethereum NFT side has a 4663 transaction out (review round 6, F6-L-1: 4663's first provider
 *                          keeps state ≈ 10 minutes, and the canister identifies a transaction every index misses only from state inside
 *                          that reach; round 7, F7-L-1 / D7-I-3: ≈ 520 s from signing at worst, so the next tick is due 120 s after the
 *                          signing cycle started, not 240 s after it ended)
 *   RUNNER_RELAY           (default on; "off" disables)           RELAY_API_URL (default https://api.relay.link)
 *   RUNNER_INSCRIPTIONS    (default unset: off) Ordinals multi-collection (docs/v9/ordinals-collections.md §4), the Inscriptions collections
 *                          whose listings the runner BUYS: <slug>=<owSlug>:<membersFile>[:<unisat>],… (src/inscriptions.ts). Per collection: its
 *                          Ordinals Wallet PROTECTED listings under owSlug (docs/v8/ow-protected.md; empty owSlug: off), then, when that handed
 *                          nothing in, its fixed-price UniSat listings under <unisat> through the canister's classic judge (frog_offer; needs
 *                          UNISAT_API_KEY), each proved from membersFile (a JSON array of inscription ids, the leaves of the collection's root).
 *                          The canister's collection is the one whose root is the file's tree root (frog_collections()), served only while live;
 *                          one Bitcoin hand-in per cycle across them (the float is shared), the collections in turn.
 *   RUNNER_OW_SLUG, RUNNER_OW_MEMBERS, RUNNER_UNISAT_COLLECTION  the one-collection form, still read: Bitcoin Frogs' entry
 *                          (bitcoin-frogs=<RUNNER_OW_SLUG>:<RUNNER_OW_MEMBERS>[:<RUNNER_UNISAT_COLLECTION>]); refused beside a
 *                          RUNNER_INSCRIPTIONS that names bitcoin-frogs too
 *
 *   RUNNER_ETH_NFT         (default on; "off" disables)           ETH_NFT_SLUGS  0x<collection>=<opensea slug>,…   OPENSEA_API_KEY (secret,
 *                          read at each call, never logged)       ETH_LOGS_RPC   (secret when keyed) an Ethereum RPC serving eth_getLogs:
 *                                                                  the operator's Dwellir URL (2026-09-27), never logged or printed
 *   ETH_LOGS_MAX_BLOCKS    (default 500, Dwellir's plan) the widest eth_getLogs range asked
 *   OPENSEA_CHAIN_4663     (default "robinhood", NOT verified against OpenSea's chain list) OpenSea's API name for Robinhood Chain: the floor
 *                          evidence and listings of a 4663 collection (EthCollection.chain_id) are asked under it. The 4663 side's own state
 *                          (eth_nft_state_of(4663): its buys, readings, deposits) is served beside the config's chain's, every pass
 *   RUNNER_IDLE_TICK_EVERY_S (default 900, from RUNNER_TICK_EVERY_S to 3600) docs/v7/dormant-collections.md §7: the cadence while every
 *                          collection is DORMANT and nothing is open (a tick then only reads the wake desk: the idle burn)
 *   GET /collections       each collection's life and status ("warming up (~N min)") for the site, from eth_nft_collections()
 *   GET /capacity          how many collections the canister's cycles balance keeps awake ("N of M collections can be awake right now"),
 *                          from eth_nft_capacity() (null before a canister that has it)
 *
 * The Ethereum NFT step (docs/v7/phase1-nft.md §6): after each tick, a buy that asks for a Relay quote gets one (4663 → Ethereum,
 * to the canister's own address), an arrived buy gets OpenSea's cheapest listings as fulfillment calldata for the canister's
 * address, and every 20 minutes each collection's floor evidence (listings and recent sale transactions) is handed in. The
 * canister judges every one on its own Ethereum reads.
 *
 * THE HOME (docs/v9/solana-migration.md §5 "Runner"): ROOK_HOME=solana (the default since the pivot of 2026-10-09; HOME=solana is read
 * too when HOME holds exactly that word and not the Unix home directory) or ROOK_HOME=robinhood (the EVM-era flow, unchanged).
 * On the Solana home:
 *   - every Relay leg has a Solana side (SOL → BTC for a frog conversion, SOL → ETH for an Ethereum buy, ETH → SOL for its way back):
 *     quotes from /quote/v2 for exactly the canister's ask (its dedicated Relay key the depositor, its main address every Solana
 *     refund: audit A5-03), pre-judged by src/relaysol.ts with the canister's rules S1–S12 (relaysol.rs judge_sol) or relayeth.rs
 *     judge_to_solana, S3 (Relay's depository not upgraded, its allocator pinned) read from SOL_RPC_URLS when set;
 *   - the frog floor READING is handed in with frog_floor_reading(coll, …) for every LIVE Inscriptions collection (D-i: the canister is the
 *     registry keeper and posts it; src/solfloor.ts), and only inside the 20 % band of CoinGecko's floor (ordinals-collections.md §5);
 *   - a holder deposit waiting for its membership proof gets it (frog_proof(coll, …), src/inscriptions.ts);
 *   - the Bitcoin steps are unchanged (a frog's cap is in lamports: the rate is SOL/BTC);
 *   - no pending cadence: it followed 4663's state window (F6-L-1 / F7-L-1), which a Solana home does not have.
 *   SOL_RPC_URLS          (optional; https, comma-separated) the runner's own reads for rule S3; without it S3 is the canister's alone
 *   RELAY_API_KEY         (optional secret) sent as x-api-key to the Relay API only; /quote/v2 is keyless today
 *   RUNNER_FROG_FLOOR     (default on; "off" disables)           RUNNER_FROG_FLOOR_EVERY_S (default 1800, 300 to 3600)
 *   RUNNER_FLOOR_LISTINGS (default "canister": the canister's LIVE Inscriptions collections (frog_collections()), each paired with its
 *                          launch-file listing by members root; "all": also every dormant listing not marked wrapping "later", read and held
 *                          at GET /floors, never posted (an operator-slept one: frog_wake(coll) lifts it))
 *   RUNNER_COINGECKO_KEY  (optional secret) CoinGecko's demo key, sent only to CoinGecko: the band gate's reference (/nfts/<coingeckoId>);
 *                          keyless by default. No answer: the reading is held (the module keeps its last post)
 *   RUNNER_FROG_PROOFS    (default on on the Solana home; "off" disables) holder deposits waiting for a membership proof get it from the
 *                          live collection's members file (the launch file's membersFile, checked against the canister's root first)
 *   RUNNER_LAUNCH_FILE    (default deployments/launch/nft-pilot.json beside ops/) the listings, their Ordinals Wallet slugs, members files
 *                          and CoinGecko ids
 *   UNISAT_API_KEY        (secret) REQUIRED by the frog floor reading: the feed believes a listed outpoint's contents only when two
 *                          providers agree, and of its three only UniSat and ordinals.com serve an output (Ordinals Wallet does not), so
 *                          without the key every listing is L-unverified and there is no floor (measured 2026-10-10); also the UniSat buying step's
 *   GET /floors           the last reading of each Inscriptions listing (posted or held)
 *   RUNNER_SOL_NFT_FLOOR  (default on on the Solana home; "off" disables) docs/v9/solana-nfts-canister.md §5: for every Solana collection
 *                          of RUNNER_SOL_COLLECTIONS_FILE the canister has OPEN, Magic Eden's keyless listings (listingAggMode) and stats,
 *                          handed in with sol_nft_evidence (src/solnftfloor.ts), each collection every RUNNER_SOL_NFT_FLOOR_EVERY_S (default
 *                          1800, 300 to 3600), one Magic Eden call at a time
 *   RUNNER_SOL_COLLECTIONS_FILE (default deployments/launch/solana-collections.json beside ops/) the Solana collections and their
 *                          Magic Eden symbols          MAGIC_EDEN_API (default https://api-mainnet.magiceden.dev; https only)
 *
 * The relay step (docs/v6/relay-conversion.md §1 step 4): after each tick, a conversion that asks for a quote gets one from
 * Relay's public API, pre-judged by src/relay.ts and handed in with `relay_quote`; the runner then ticks again at once, so the
 * canister judges it (on its own reads) inside its 300 s window. The runner cannot move anything: the canister refuses any quote
 * that does not pay its own float from its own address at its own amount.
 */
export type Home = "solana" | "robinhood";
export type RunnerConfig = { canisterId: string; host: string; everyS: number; port: number | null; relay: boolean; relayApi: string; eth: boolean;
  slugs: Record<string, string>; ethLogsRpc: string | null; logsMaxBlocks: number; pendingEveryS: number; idleEveryS: number;
  /** 2026-10-12: OpenSea's chain names by chain id over ethnft.ts's defaults (OPENSEA_CHAIN_4663) */
  openseaChains: Record<string, string>;
  /** Ordinals multi-collection: the collections whose listings are bought (RUNNER_INSCRIPTIONS, or the old scalars as Bitcoin Frogs') */
  inscriptions: InscEntry[]; frogProofs: boolean;
  /** docs/v9 §5: the canister's home chain; the Solana fields below are read only on the Solana home */
  home: Home; relayQuotePath: string; solRpcUrls: string[]; frogFloor: boolean; floorEveryS: number; floorScope: "canister" | "all";
  launchFile: string; listings: InscListing[];
  /** docs/v9/solana-nfts-canister.md §5: the Solana collections' Magic Eden feed (the Solana home only) */
  solNftFloor: boolean; solNftEveryS: number; solCollectionsFile: string; solCollections: SolNftCollection[]; meApi: string };

/** ROOK_HOME, or HOME when it holds exactly "solana" / "robinhood" (HOME is otherwise the Unix home directory, never a chain). */
export function homeOf(env: Readonly<Record<string, string | undefined>>, problems: string[] = []): Home {
  const v = (env.ROOK_HOME ?? (["solana", "robinhood"].includes(String(env.HOME).toLowerCase()) ? env.HOME : undefined) ?? "solana").toLowerCase();
  if (v !== "solana" && v !== "robinhood") { problems.push(`ROOK_HOME=${env.ROOK_HOME} is neither solana nor robinhood`); return "solana"; }
  return v;
}
/** The launch file beside ops/ (the image copies it to /app/deployments/launch/nft-pilot.json). */
export const LAUNCH_FILE_DEFAULT = new URL("../../deployments/launch/nft-pilot.json", import.meta.url).pathname;
/** The Solana collections' launch file beside ops/ (the image copies it to /app/deployments/launch/solana-collections.json). */
export const SOL_COLLECTIONS_FILE_DEFAULT = new URL("../../deployments/launch/solana-collections.json", import.meta.url).pathname;
/** The runner's secrets, cut out of every log line: log.ts's, and (docs/v9 §5) UniSat's and Relay's keys and the Solana RPC URLs (a Helius
 * URL carries its key in the query). Read at the moment of logging. */
export const runnerSecrets = (env: Readonly<Record<string, string | undefined>> = process.env): string[] => [...envSecrets(env),
  env.UNISAT_API_KEY ?? "", env.RELAY_API_KEY ?? "", env.RUNNER_COINGECKO_KEY ?? "", ...(env.SOL_RPC_URLS ?? "").split(",").map((u) => u.trim()).filter(Boolean).flatMap(urlSecrets)].filter(Boolean);
/** /healthz: no frog floor reading accepted for this long (once the runner has run that long) is unhealthy: the module goes stale at 3 h. */
export const FROG_FLOOR_STALE_S = 2 * 3_600;
export class RunnerConfigError extends Error { constructor(readonly problems: string[]) { super(`refusing to start:\n  - ${problems.join("\n  - ")}`); this.name = "RunnerConfigError"; } }

export function parseRunnerConfig(env: Readonly<Record<string, string | undefined>>): RunnerConfig {
  const problems: string[] = [];
  const canisterId = env.CANISTER_ID ?? "";
  if (!/^[a-z0-9]{5}(-[a-z0-9]{5}){3}-[a-z0-9]{3}$/.test(canisterId)) problems.push(`CANISTER_ID ${canisterId ? `"${canisterId}" is not a canister id` : "is not set"}`);
  const host = env.IC_HOST ?? IC_HOST;
  try { const u = new URL(host); if (u.protocol !== "https:" && !["127.0.0.1", "localhost"].includes(u.hostname)) problems.push("IC_HOST must be https (plain http only for a local replica)"); }
  catch { problems.push("IC_HOST is not a URL"); }
  const everyS = Number(env.RUNNER_TICK_EVERY_S ?? 600);
  if (!Number.isInteger(everyS) || everyS < 60 || everyS > 86_400) problems.push(`RUNNER_TICK_EVERY_S=${env.RUNNER_TICK_EVERY_S} is not a whole number of seconds in [60, 86400]`);
  const home = homeOf(env, problems);
  const pe = env.RUNNER_PENDING_EVERY_S;
  // review round 7 (D7-I-3): plain decimal digits only (no 0x3c, no 6e1), 60 to 300 (the canister's search reach is ≈ 520 s from signing).
  // docs/v9 §5: the Solana home has no pending cadence (it followed 4663's state window): the variable is not read there
  const pendingEveryS = home === "solana" || pe === undefined || pe === "" ? Math.min(PENDING_EVERY_DEFAULT_S, everyS) : /^[0-9]+$/.test(pe) ? Number(pe) : NaN;
  if (home === "robinhood" && (!Number.isInteger(pendingEveryS) || pendingEveryS < 60 || pendingEveryS > Math.min(Math.max(everyS, 60), PENDING_EVERY_MAX_S))) problems.push(`RUNNER_PENDING_EVERY_S=${pe} is not a whole number of seconds from 60 to ${PENDING_EVERY_MAX_S} (and at most RUNNER_TICK_EVERY_S)`);
  const p = env.PORT ?? env.RUNNER_STATUS_PORT;
  const port = p === undefined || p === "" ? null : Number(p);
  if (port !== null && (!Number.isInteger(port) || port < 1 || port > 65_535)) problems.push(`the status port ${p} is not a port`);
  const relayFlag = (env.RUNNER_RELAY ?? "on").toLowerCase();
  if (!["on", "off"].includes(relayFlag)) problems.push(`RUNNER_RELAY=${env.RUNNER_RELAY} is neither on nor off`);
  const relayApi = env.RELAY_API_URL ?? RELAY_API_DEFAULT;
  try { if (new URL(relayApi).protocol !== "https:") problems.push("RELAY_API_URL must be https"); } catch { problems.push("RELAY_API_URL is not a URL"); }
  const ethFlag = (env.RUNNER_ETH_NFT ?? "on").toLowerCase();
  if (!["on", "off"].includes(ethFlag)) problems.push(`RUNNER_ETH_NFT=${env.RUNNER_ETH_NFT} is neither on nor off`);
  let slugs: Record<string, string> = {};
  try { slugs = parseSlugs(env.ETH_NFT_SLUGS); } catch (err) { problems.push((err as Error).message); }
  const ethLogsRpc = env.ETH_LOGS_RPC ?? null;
  if (ethLogsRpc) { try { if (new URL(ethLogsRpc).protocol !== "https:") problems.push("ETH_LOGS_RPC must be https"); } catch { problems.push("ETH_LOGS_RPC is not a URL"); } }
  const lm = env.ETH_LOGS_MAX_BLOCKS;
  const logsMaxBlocks = lm === undefined || lm === "" ? 500 : Number(lm);
  if (!Number.isInteger(logsMaxBlocks) || logsMaxBlocks < 10 || logsMaxBlocks > 10_000) problems.push(`ETH_LOGS_MAX_BLOCKS=${lm} is not 10 to 10,000 blocks`);
  const ie = env.RUNNER_IDLE_TICK_EVERY_S;
  const idleEveryS = ie === undefined || ie === "" ? Math.max(IDLE_EVERY_DEFAULT_S, everyS) : /^[0-9]+$/.test(ie) ? Number(ie) : NaN;
  if (!Number.isInteger(idleEveryS) || idleEveryS < everyS || idleEveryS > IDLE_EVERY_MAX_S) problems.push(`RUNNER_IDLE_TICK_EVERY_S=${ie} is not a whole number of seconds from RUNNER_TICK_EVERY_S to ${IDLE_EVERY_MAX_S}`);
  // 2026-10-12 (Robinhood Chain NFTs): OpenSea's name for chain 4663, "robinhood" unless overridden (not verified offline)
  const oc = (env.OPENSEA_CHAIN_4663 ?? "").trim();
  if (oc && !/^[a-z0-9_-]{1,40}$/.test(oc)) problems.push(`OPENSEA_CHAIN_4663=${oc} is not an OpenSea chain name`);
  const openseaChains: Record<string, string> = { "4663": oc || "robinhood" };
  // Ordinals multi-collection: the buying steps per collection (docs/v8/ow-protected.md §4; the old scalars as Bitcoin Frogs' entry)
  const inscriptions = parseInscriptionsEnv(env, problems);
  // docs/v9 §5: the Solana home's own settings
  const solRpcUrls = home === "solana" ? parseSolRpcUrls(env.SOL_RPC_URLS, problems) : [];
  const ff = (env.RUNNER_FROG_FLOOR ?? "on").toLowerCase();
  if (!["on", "off"].includes(ff)) problems.push(`RUNNER_FROG_FLOOR=${env.RUNNER_FROG_FLOOR} is neither on nor off`);
  const fe = env.RUNNER_FROG_FLOOR_EVERY_S;
  const floorEveryS = fe === undefined || fe === "" ? FLOOR_EVERY_DEFAULT_S : /^[0-9]+$/.test(fe) ? Number(fe) : NaN;
  if (!Number.isInteger(floorEveryS) || floorEveryS < 300 || floorEveryS > 3_600) problems.push(`RUNNER_FROG_FLOOR_EVERY_S=${fe} is not a whole number of seconds from 300 to 3600 (the canister drops a reading not posted within the hour)`);
  const scope = (env.RUNNER_FLOOR_LISTINGS ?? "canister").toLowerCase();
  if (scope !== "canister" && scope !== "all") problems.push(`RUNNER_FLOOR_LISTINGS=${env.RUNNER_FLOOR_LISTINGS} is neither canister nor all`);
  const launchFile = env.RUNNER_LAUNCH_FILE || LAUNCH_FILE_DEFAULT;
  const frogFloor = home === "solana" && ff === "on";
  const fp = (env.RUNNER_FROG_PROOFS ?? "on").toLowerCase();
  if (!["on", "off"].includes(fp)) problems.push(`RUNNER_FROG_PROOFS=${env.RUNNER_FROG_PROOFS} is neither on nor off`);
  const frogProofs = home === "solana" && fp === "on";
  let listings: InscListing[] = [];
  if (frogFloor || frogProofs) {
    try { listings = loadInscriptionListings(launchFile); if (!listings.length) problems.push(`RUNNER_LAUNCH_FILE ${launchFile} names no Inscriptions listing: no frog floor to read`); }
    catch (err) { problems.push(`RUNNER_LAUNCH_FILE ${launchFile}: ${(err as Error).message.slice(0, 160)}${existsSync(launchFile) ? "" : " (missing)"}`); }
  }
  // docs/v9/solana-nfts-canister.md §5: the Solana NFT collections' Magic Eden feed, on by default on the Solana home
  const sf = (env.RUNNER_SOL_NFT_FLOOR ?? "on").toLowerCase();
  if (!["on", "off"].includes(sf)) problems.push(`RUNNER_SOL_NFT_FLOOR=${env.RUNNER_SOL_NFT_FLOOR} is neither on nor off`);
  const se = env.RUNNER_SOL_NFT_FLOOR_EVERY_S;
  const solNftEveryS = se === undefined || se === "" ? SOL_NFT_EVERY_DEFAULT_S : /^[0-9]+$/.test(se) ? Number(se) : NaN;
  if (!Number.isInteger(solNftEveryS) || solNftEveryS < 300 || solNftEveryS > 3_600) problems.push(`RUNNER_SOL_NFT_FLOOR_EVERY_S=${se} is not a whole number of seconds from 300 to 3600`);
  const meApi = env.MAGIC_EDEN_API || ME_API_DEFAULT;
  try { if (new URL(meApi).protocol !== "https:") problems.push("MAGIC_EDEN_API must be https"); } catch { problems.push("MAGIC_EDEN_API is not a URL"); }
  const solCollectionsFile = env.RUNNER_SOL_COLLECTIONS_FILE || SOL_COLLECTIONS_FILE_DEFAULT;
  const solNftFloor = home === "solana" && sf === "on";
  let solCollections: SolNftCollection[] = [];
  if (solNftFloor) {
    try { solCollections = loadSolNftCollections(solCollectionsFile); if (!solCollections.length) problems.push(`RUNNER_SOL_COLLECTIONS_FILE ${solCollectionsFile} names no Solana collection`); }
    catch (err) { problems.push(`RUNNER_SOL_COLLECTIONS_FILE ${solCollectionsFile}: ${(err as Error).message.slice(0, 160)}${existsSync(solCollectionsFile) ? "" : " (missing)"}`); }
  }
  if (problems.length) throw new RunnerConfigError(problems);
  return { canisterId, host, everyS, port, relay: relayFlag === "on", relayApi, eth: ethFlag === "on", slugs, ethLogsRpc, logsMaxBlocks, pendingEveryS, idleEveryS,
    openseaChains, inscriptions, frogProofs, home, relayQuotePath: home === "solana" ? "/quote/v2" : "/quote", solRpcUrls, frogFloor, floorEveryS,
    floorScope: scope === "all" ? "all" : "canister", launchFile, listings, solNftFloor, solNftEveryS, solCollectionsFile, solCollections, meApi };
}

/** docs/v7/dormant-collections.md §7: the idle cadence's default and ceiling, seconds (a wake request waits at most this for its tick). */
export const IDLE_EVERY_DEFAULT_S = 900;
export const IDLE_EVERY_MAX_S = 3_600;
/** Review round 7 (F7-L-1 / D7-I-3): the pending period's default and ceiling, seconds. */
export const PENDING_EVERY_DEFAULT_S = 120;
export const PENDING_EVERY_MAX_S = 300;
/** Review round 7 (F7-I-2): a 4663 transaction whose nonce has been used and seen nowhere this long is past the canister's state reach:
 * the operator's path decides it, and the runner goes back to its normal cadence. */
export const PENDING_STALE_S = 15 * 60;

/**
 * Review round 7 (F7-I-2): whether the pending 4663 transaction (`eth_nft_state().pending`) still wants the faster cadence: out, and not
 * seen nowhere for longer than PENDING_STALE_S (then only the operator or its effect decides it).
 */
export function pendingWantsHurry(pending: { used_since?: [] | [bigint] }[] | null | undefined, nowS: number): boolean | null {
  if (pending === null || pending === undefined) return null;
  const p = pending[0];
  if (!p) return false;
  const used = p.used_since?.[0];
  return used === undefined || nowS - Number(used) < PENDING_STALE_S;
}

/**
 * Review round 6 (F6-L-1): how long the runner sleeps before its next tick. While the canister's Ethereum NFT side has a 4663
 * transaction out (`eth_nft_state().pending`), the next tick comes sooner, so a transaction that mined while every provider's index
 * misses it is identified from state inside the ≈ 10 minutes 4663's first provider keeps (at 600 s + a cycle it never was). An
 * unreadable state is no reason to hurry.
 */
export function sleepAfter(cfg: Pick<RunnerConfig, "everyS" | "pendingEveryS"> & Partial<Pick<RunnerConfig, "idleEveryS" | "home">>, homeTxOut: boolean | null,
  elapsedS = 0, idle = false): number {
  // docs/v9 §5: the pending cadence followed 4663's state window; a Solana home has none (its line is the canister's saved bytes)
  if (cfg.home === "solana") homeTxOut = false;
  // review round 7 (D7-I-3): while a transaction is out the pending cadence is a PERIOD from the cycle's start (a slow cycle no longer
  // pushes the next tick out of the canister's reach), at least 10 s apart
  if (homeTxOut) return Math.max(10, Math.min(cfg.pendingEveryS, cfg.everyS) - Math.max(0, Math.floor(elapsedS)));
  // docs/v7/dormant-collections.md §7: every collection asleep and nothing open: the idle cadence (the idle burn is one tick's reads)
  return idle && cfg.idleEveryS ? Math.max(cfg.everyS, cfg.idleEveryS) : cfg.everyS;
}

/**
 * 2026-10-12 (Robinhood Chain NFTs): the other chains' sides, read beside the config's own: `eth_nft_state_of(chain)` for each chain a
 * collection names (`EthCollection.chain_id`) that is not the config's (4663 for a Robinhood Chain collection). A chain with no side yet
 * (`[]`) adds nothing.
 */
export function sideStatesOf(b: Pick<Buyer, "eth_nft_config" | "eth_nft_state_of">): () => Promise<EthStateV[]> {
  return async () => {
    const ec = (await b.eth_nft_config())[0];
    if (!ec) return [];
    const chains = [...new Set(ec.collections.map((c) => c.chain_id?.[0]).filter((x): x is bigint => x !== undefined && x !== ec.chain_id))];
    const out: EthStateV[] = [];
    for (const ch of chains) { const st = (await b.eth_nft_state_of(ch))[0]; if (st) out.push(st); }
    return out;
  };
}

export type TickOutcome = { at: string; ok: boolean; lines: string[]; error: string | null; ms: number };
export type RunnerState = { principal: string; canisterId: string; startedAt: string; ticks: number; lastTick: TickOutcome | null; consecutiveFailures: number; paused: boolean | null };

/** audit 2026-09-27-nft D-L-2: a collection whose evidence has not been accepted for this long makes /healthz unhealthy. */
export const EVIDENCE_STALE_S = 40 * 60;

/** The collections (of `served`) whose floor evidence was last accepted more than EVIDENCE_STALE_S ago (or never, once the runner has run that long). */
export function staleEvidence(served: string[], ok: Map<string, number>, nowS: number, startedS: number): string[] {
  if (nowS - startedS < EVIDENCE_STALE_S) return [];
  return served.filter((c) => nowS - (ok.get(c) ?? startedS) > EVIDENCE_STALE_S).map((c) => `no floor evidence accepted for ${c} in ${Math.round((nowS - (ok.get(c) ?? startedS)) / 60)} min`);
}

export class Runner {
  readonly state: RunnerState;
  private busy = false;
  /** D-L-2: what keeps the Ethereum step from working (read by /healthz); empty when healthy or off. */
  ethProblems: () => string[] = () => [];
  constructor(private readonly b: Pick<Buyer, "tick" | "paused">, private readonly log: Log, principal: string, canisterId: string, private readonly now: () => number = Date.now,
    private readonly relay: (() => Promise<boolean>) | null = null) {
    this.state = { principal, canisterId, startedAt: new Date(now()).toISOString(), ticks: 0, lastTick: null, consecutiveFailures: 0, paused: null };
  }

  /** One tick, then the relay step; a quote handed in is judged by a second tick at once (its window is 300 s). */
  async cycle(): Promise<TickOutcome | null> {
    const out = await this.once();
    if (!out || !this.relay) return out;
    let submitted = false;
    try { submitted = await this.relay(); } catch (err) { this.log("warn", "relay-step-failed", { error: String((err as Error).message).slice(0, 200) }); }
    if (!submitted) return out;
    this.log("info", "relay-tick", { why: "a quote was handed in: tick again so the canister judges it now" });
    return (await this.once()) ?? out;
  }

  /** One tick. Refused (not queued) while another is running: calls never overlap. */
  async once(): Promise<TickOutcome | null> {
    if (this.busy) { this.log("warn", "tick-skipped", { why: "the previous tick is still running" }); return null; }
    this.busy = true;
    const t0 = this.now();
    try {
      let out: TickOutcome;
      try {
        const r = await this.b.tick();
        if ("Ok" in r) {
          for (const line of r.Ok) this.log("info", "tick-line", { line });
          out = { at: new Date(t0).toISOString(), ok: true, lines: r.Ok, error: null, ms: this.now() - t0 };
        } else {
          this.log("warn", "tick-err", { error: r.Err });
          out = { at: new Date(t0).toISOString(), ok: false, lines: [], error: r.Err, ms: this.now() - t0 };
        }
      } catch (err) {
        // a rejected call (the operator check trapped, the canister is stopped, the IC answered an error): an error, not a line
        const full = String((err as Error).message ?? err).replace(/\s+/g, " ");
        const text = /Reject text: (.*?)(?: Error code:| Request ID:|$)/.exec(full)?.[1];
        const msg = (text ? `rejected: ${text}` : full).slice(0, 400);
        this.log("error", "tick-rejected", { error: msg });
        out = { at: new Date(t0).toISOString(), ok: false, lines: [], error: msg, ms: this.now() - t0 };
      }
      this.state.ticks++;
      this.state.lastTick = out;
      this.state.consecutiveFailures = out.ok ? 0 : this.state.consecutiveFailures + 1;
      return out;
    } finally { this.busy = false; }
  }

  async readPaused(): Promise<void> {
    try { this.state.paused = await this.b.paused(); this.log("info", "paused", { paused: this.state.paused }); } catch (err) { this.log("warn", "paused-unread", { error: String((err as Error).message).slice(0, 200) }); }
  }

  healthy(everyS: number): boolean {
    const last = this.state.lastTick;
    return this.state.consecutiveFailures < 3 && (last === null ? this.now() - Date.parse(this.state.startedAt) < everyS * 2000 : this.now() - Date.parse(last.at) <= everyS * 3000)
      && this.ethProblems().length === 0;
  }
}

export function serveRunner(r: Runner, everyS: number, port: number, lives: CollectionLifeV[] = [], capacity: { current: CapacityV | null } = { current: null },
  floors: Map<number, Held> = new Map()): Server {
  return createServer((req, res) => {
    const path = (req.url ?? "/").split("?")[0];
    if (req.method !== "GET") { res.writeHead(405); res.end(); return; }
    // docs/v9 §2.9 D-i: the last floor reading of each Inscriptions listing (public, read-only; posted or held)
    if (path === "/floors") { res.writeHead(200, { "content-type": "application/json", "cache-control": "public, max-age=60", "access-control-allow-origin": "*" }); res.end(JSON.stringify([...floors.values()])); return; }
    // docs/v7/dormant-collections.md §4.2: the collections' lives for the site (public, read-only; the canister's own query, as of the last pass)
    if (path === "/collections") { res.writeHead(200, { "content-type": "application/json", "cache-control": "public, max-age=60", "access-control-allow-origin": "*" }); res.end(JSON.stringify(lives, (_k, v) => (typeof v === "bigint" ? String(v) : v))); return; }
    // docs/v7/dormant-collections.md §11: the capacity on the canister's cycles balance, as of the last pass (public, read-only)
    if (path === "/capacity") { res.writeHead(200, { "content-type": "application/json", "cache-control": "public, max-age=60", "access-control-allow-origin": "*" }); res.end(JSON.stringify(capacity.current, (_k, v) => (typeof v === "bigint" ? String(v) : v))); return; }
    if (path === "/healthz") { const ok = r.healthy(everyS); res.writeHead(ok ? 200 : 503, { "content-type": "application/json" }); res.end(JSON.stringify({ ok, ...r.state, eth: r.ethProblems(), lastTick: r.state.lastTick && { ...r.state.lastTick, lines: r.state.lastTick.lines.length } }, (_k, v) => (typeof v === "bigint" ? String(v) : v))); return; }
    if (path === "/status") { res.writeHead(200, { "content-type": "application/json", "cache-control": "no-store" }); res.end(JSON.stringify(r.state)); return; }
    res.writeHead(404); res.end();
  }).listen(port, "::");
}

async function main(): Promise<void> {
  const log = logger(undefined, () => runnerSecrets());
  let cfg: RunnerConfig;
  try { cfg = parseRunnerConfig(process.env); } catch (err) {
    for (const p of err instanceof RunnerConfigError ? err.problems : [String((err as Error).message)]) log("error", "refusing-to-start", { problem: p });
    process.exit(2);
  }
  let identity;
  try { identity = operatorIdentity(process.env); } catch (err) { log("error", "refusing-to-start", { problem: (err as Error).message }); process.exit(2); }
  if (!identity) { log("error", "refusing-to-start", { problem: "OPERATOR_IDENTITY_PEM is not set: tick is operator-only, and the runner calls it as the operator" }); process.exit(2); }
  const principal = identity.getPrincipal().toText();
  const b = await buyer(cfg.canisterId, { host: cfg.host, identity });
  const lastEvidence = new Map<string, number>();
  const claimed = new Map<string, number>();
  const tried = new Map<string, Map<string, number>>(), evidenceOk = new Map<string, number>();
  // audit 2026-09-27-nft D-L-2: the Ethereum step starts fully configured for what the canister serves, or the runner does not start;
  // round 2 (D2-L-3): and it is checked again on every pass (ethNftOnce), which /healthz reports
  const served: string[] = [], problems: string[] = [];
  const lives: CollectionLifeV[] = [];
  const capacity: { current: CapacityV | null } = { current: null };
  if (cfg.eth) {
    const ec = await b.eth_nft_config().then((x) => x[0] ?? null, () => null);
    if (ec) {
      const p0 = ethStepProblems(ec, { ethLogsRpc: cfg.ethLogsRpc, slugs: cfg.slugs, openseaKey: process.env.OPENSEA_API_KEY });
      if (p0.length) { for (const p of p0) log("error", "refusing-to-start", { problem: p }); process.exit(2); }
      // docs/v7/dormant-collections.md: a DORMANT collection gets no evidence, so it never counts as stale (the first pass narrows this)
      const l0 = await b.eth_nft_collections().catch(() => null);
      served.push(...ec.collections.map((c) => c.collection.toLowerCase())
        .filter((c) => !l0 || !["dormant", "queued"].includes(l0.find((x) => x.collection.toLowerCase() === c)?.state ?? "")));
    } else problems.push("the canister's eth_nft_config() could not be read at start: checked again on the next pass");
  }
  const steps: (() => Promise<boolean>)[] = [];
  // docs/v9 §5: the Solana home's Relay legs (/quote/v2, the optional key, rule S3 from the runner's own reads when SOL_RPC_URLS is set)
  const sol = cfg.home === "solana";
  const reader = sol && cfg.solRpcUrls.length ? solReader(cfg.solRpcUrls) : null;
  const s3 = sol ? () => readS3(reader ? (a, slice) => reader.account(a, slice) : null) : undefined;
  const relayKey = () => process.env.RELAY_API_KEY || undefined;
  if (sol && !reader) log("warn", "s3-not-read", { note: "SOL_RPC_URLS is not set: rule S3 (Relay's depository not upgraded, its allocator pinned) is the canister's alone" });
  if (cfg.relay) steps.push(() => convertOnce({ buyer: b, fetch, log, api: cfg.relayApi, quotePath: cfg.relayQuotePath, apiKey: relayKey,
    solJudge: sol ? preJudgeSolAsk : undefined, s3 }));
  const owSubmitted = new Set<string>();
  const unisatBids = new Map<string, number>();
  // a frog's cap is in the home's unit: wei on 4663, lamports on Solana (the rate then sats per 10^18 lamports: frogsol.rs)
  const homeRate = sol ? liveSatsPerSolHomeUnit(fetch) : liveSatsPerEth(fetch);
  // Ordinals multi-collection: per live collection of RUNNER_INSCRIPTIONS, the protected step then (when it handed nothing in, or failed)
  // UniSat's; one Bitcoin hand-in per cycle across the collections (src/inscriptions.ts)
  const inscProblems = new Set<string>();
  const frogColls = () => b.frog_collections();
  if (cfg.inscriptions.length) steps.push(bitcoinStepsOnce({ entries: cfg.inscriptions, colls: frogColls, log, problems: inscProblems, step: async (e, coll) => {
    let acted = false;
    if (e.owSlug) {
      try { acted = await owProtectedOnce({ buyer: b, fetch, log, coll, slug: e.owSlug, members: e.members, api: OW_API_DEFAULT, satsPerEth: homeRate, submitted: owSubmitted }); }
      catch (err) { log("warn", "step-failed", { step: "ow", coll, slug: e.slug, error: String((err as Error).message).slice(0, 200) }); }
    }
    return acted || (e.unisat ? unisatOnce({ buyer: b, fetch, log, coll, collectionId: e.unisat, members: e.members, apiKey: () => process.env.UNISAT_API_KEY,
      satsPerEth: homeRate, feeRate: liveFeeRate(fetch), lastBid: unisatBids }) : false);
  } }));
  const sideStates = sideStatesOf(b);
  if (cfg.eth) steps.push(() => ethNftOnce({ buyer: b, fetch, log, slugs: cfg.slugs, openseaKey: () => process.env.OPENSEA_API_KEY, ethRpc: cfg.ethLogsRpc,
    openseaChains: cfg.openseaChains, sideStates,
    logsMaxBlocks: cfg.logsMaxBlocks, relayApi: cfg.relayApi, relayQuotePath: cfg.relayQuotePath, relayKey, solJudge: sol ? preJudgeSolAsk : undefined, s3, lastEvidence, claimed, tried, evidenceOk, problems,
    served, lives, capacity }));
  // docs/v9 §2.9 D-i: the frog floor reading, every RUNNER_FROG_FLOOR_EVERY_S (the canister posts it at the tick that follows)
  const floors = new Map<number, Held>(), floorAccepted = new Map<number, number>(), floorLive = new Set<number>();
  if (cfg.frogFloor) {
    // the feed's two agreeing output reads are UniSat and ordinals.com (Ordinals Wallet serves none): no key, no floor (/healthz says so)
    if (!process.env.UNISAT_API_KEY) { problems.push("UNISAT_API_KEY is not set: the frog floor feed cannot verify any listed output (two providers must agree), so no reading is ever handed in"); log("error", "frog-floor-needs-unisat", { problem: problems[problems.length - 1] }); }
    const read = productionReader({ fetch, log }), last = new Map<number, number>();
    const ref = coingeckoReader(fetch, () => process.env.RUNNER_COINGECKO_KEY || undefined);
    steps.push(() => frogFloorOnce({ buyer: b, log, listings: cfg.listings, read, ref, scope: cfg.floorScope, everyS: cfg.floorEveryS, last, accepted: floorAccepted,
      live: floorLive, held: floors }));
  }
  // Ordinals multi-collection: a holder deposit's membership proof, from its live collection's members file
  if (cfg.frogProofs) {
    // audit C4-06: `owned` lets the step clear its own problems each pass; C4-10: the members files' overlaps are logged as they load
    const members = new ListingMembers(undefined, { log }), handed = new Map<string, number>(), owned = new Set<string>();
    steps.push(() => frogProofOnce({ buyer: b, log, listings: cfg.listings, members, handed, problems: inscProblems, owned }));
  }
  // docs/v9/solana-nfts-canister.md §5: the Solana collections' Magic Eden listings and stats, for the open ones only
  if (cfg.solNftFloor) {
    const last = new Map<string, number>(), accepted = new Map<string, number>();
    steps.push(() => solNftFloorOnce({ buyer: b, fetch, log, collections: cfg.solCollections, api: cfg.meApi, everyS: cfg.solNftEveryS, last, accepted }));
  }
  const relay = steps.length ? async () => { let any = false; for (const s of steps) { try { any = (await s()) || any; } catch (err) { log("warn", "step-failed", { error: String((err as Error).message).slice(0, 200) }); } } return any; } : null;
  const runner = new Runner(b, log, principal, cfg.canisterId, Date.now, relay);
  const startedS = Math.floor(Date.now() / 1000);
  runner.ethProblems = () => {
    const nowS = Math.floor(Date.now() / 1000);
    // per live Inscriptions collection: a reading held out of the band for 2 h is unhealthy too (the operator looks: the market, or the feed)
    const floor = cfg.frogFloor ? staleFloors(floorLive, floorAccepted, nowS, startedS, FROG_FLOOR_STALE_S) : [];
    return [...problems, ...inscProblems, ...staleEvidence(served, evidenceOk, nowS, startedS), ...floor];
  };
  log("info", "start", { principal, canister: cfg.canisterId, host: new URL(cfg.host).host, everyS: cfg.everyS, relay: cfg.relay ? new URL(cfg.relayApi).host : "off",
    home: cfg.home, solRpc: cfg.solRpcUrls.map((u) => new URL(u).host), frogFloor: cfg.frogFloor ? `${cfg.floorScope}, every ${cfg.floorEveryS} s` : "off",
    solNftFloor: cfg.solNftFloor ? `${cfg.solCollections.length} collections (the open ones), every ${cfg.solNftEveryS} s, ${new URL(cfg.meApi).host}` : "off",
    inscriptions: cfg.inscriptions.length ? cfg.inscriptions.map((e) => `${e.slug} (${[e.owSlug && `ow ${e.owSlug}`, e.unisat && `unisat ${e.unisat}`].filter(Boolean).join(", ")}, ${e.members.length} members)`) : "off",
    frogProofs: cfg.frogProofs ? "on" : "off", openseaChains: cfg.openseaChains,
    relayKey: process.env.RELAY_API_KEY ? "set" : "unset (keyless)", unisatKey: process.env.UNISAT_API_KEY ? "set" : "unset",
    note: "calls tick() as the operator, and hands in what the canister asks for (Relay quotes, listings, floor evidence and readings); nothing else" });
  if (cfg.port !== null) serveRunner(runner, cfg.everyS, cfg.port, lives, capacity, floors);
  await runner.readPaused();
  const once = process.argv.includes("--once");
  for (const s of ["SIGINT", "SIGTERM"] as const) process.on(s, () => { log("info", "stop", { signal: s }); process.exit(0); });
  for (;;) {
    const c0 = Date.now();
    const out = await runner.cycle();
    if (once) process.exit(out?.ok ? 0 : 1);
    const st = cfg.eth ? await b.eth_nft_state().catch(() => null) : null;
    // docs/v9 §5: no pending cadence on the Solana home (sleepAfter ignores it there)
    const out4663 = st && !sol ? pendingWantsHurry(st.pending ?? [], Math.floor(Date.now() / 1000)) : null;
    const idle = cfg.eth && ethIdle(lives.length ? lives : null, st);
    const sleepS = sleepAfter(cfg, out4663, (Date.now() - c0) / 1000, idle);
    if (sleepS < cfg.everyS) log("info", "tick-soon", { why: "a 4663 transaction of the Ethereum NFT side is out: the next tick comes sooner (F6-L-1)", sleepS });
    else if (sleepS > cfg.everyS) log("info", "tick-idle", { why: "every collection is dormant and nothing is open: the idle cadence (docs/v7/dormant-collections.md §7)", sleepS });
    await new Promise((ok) => setTimeout(ok, sleepS * 1000));
  }
}

if (import.meta.url === `file://${process.argv[1]}`) await main();
