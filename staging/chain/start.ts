import { spawn } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { bootstrap } from "../bootstrap/bootstrap.js";
import { createListings } from "../bootstrap/listings.js";
import { proxy } from "./proxy.js";
import { STAGING_CHAIN_ID, assertStagingChain } from "../chain-id.js";
import { upstreamShim } from "./upstream.js";

/**
 * docs/phase6.md. One process tree: the node (127.0.0.1 only), the bootstrap on first boot, the public proxy, the operator.
 * It refuses to be Robinhood Chain: the chain id is the repository's one number (staging/chain-id.json), never 4663 and never 46630, and checked again against what the node reports.
 */
const CHAIN_ID = STAGING_CHAIN_ID;
const env = process.env;
if (env.STAGING_CHAIN_ID !== undefined && Number(env.STAGING_CHAIN_ID) !== CHAIN_ID) { console.error(`refused: this is the staging fork, chain ${CHAIN_ID}. It does not run as ${env.STAGING_CHAIN_ID}, and never as 4663.`); process.exit(2); }
const DATA = env.STAGING_DATA ?? "/data", REPO = env.STAGING_REPO ?? new URL("../../", import.meta.url).pathname.replace(/\/$/, "");
const UPSTREAM = env.STAGING_UPSTREAM ?? "https://rpc.mainnet.chain.robinhood.com";
const NODE_PORT = Number(env.STAGING_NODE_PORT ?? 8546), PORT = Number(env.PORT ?? 8080);
const NODE = `http://127.0.0.1:${NODE_PORT}`;
const SHIM_PORT = Number(env.STAGING_SHIM_PORT ?? NODE_PORT + 1), SHIM = `http://127.0.0.1:${SHIM_PORT}`;
const log = (o: Record<string, unknown>) => console.log(JSON.stringify({ at: new Date().toISOString(), svc: "staging-chain", ...o }));
const call = async <T>(url: string, method: string, params: unknown[] = []): Promise<T> => {
  const r = await fetch(url, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params }) });
  return ((await r.json()) as { result: T }).result;
};

mkdirSync(DATA, { recursive: true });
// the fork block is chosen once and kept: a restart must come back to the same chain, not to a newer copy of the real one
let forkBlock: number;
if (existsSync(`${DATA}/fork-block`)) forkBlock = Number(readFileSync(`${DATA}/fork-block`, "utf8").trim());
else { forkBlock = Number(BigInt(await call<string>(UPSTREAM, "eth_blockNumber"))) - 20; writeFileSync(`${DATA}/fork-block`, `${forkBlock}\n`); }

// anvil reads the real chain THROUGH the shim (reads only; pruned state falls back to latest): see upstream.ts
const shim = upstreamShim({ upstream: UPSTREAM, port: SHIM_PORT, log });
const anvil = spawn("anvil", ["--fork-url", SHIM, "--fork-block-number", String(forkBlock), "--chain-id", String(CHAIN_ID), "--host", "127.0.0.1", "--port", String(NODE_PORT),
  "--auto-impersonate", "--state", `${DATA}/state.json`, "--state-interval", "120", "--silent"], { stdio: ["ignore", "inherit", "inherit"], env: { ...env, PATH: `${env.HOME}/.foundry/bin:${env.PATH}` } });
anvil.on("exit", (code) => { log({ event: "node-exited", code }); process.exit(code ?? 1); });
let up = false;
for (let i = 0; i < 240 && !up; i++) { try { up = (await call<string>(NODE, "eth_chainId")) !== undefined; } catch { await new Promise((ok) => setTimeout(ok, 500)); } }
if (!up) { log({ event: "node-did-not-start" }); anvil.kill(); process.exit(1); }
const reported = Number(BigInt(await call<string>(NODE, "eth_chainId")));
try { assertStagingChain(reported); } catch (err) { log({ event: "refused", why: String((err as Error).message) }); anvil.kill(); process.exit(2); }
// A fork's clock starts at its fork block's time and runs on from there, so every restart leaves the chain behind the real
// clock by as long as the fork block is old. A chain in the past would pin drand rounds that ALREADY EXIST, and the operator's
// waits (wall clock) would never meet the contracts' (block time). So at every boot the chain is brought to the present, never back.
{
  const latest = Number(BigInt((await call<{ timestamp: string }>(NODE, "eth_getBlockByNumber", ["latest", false])).timestamp)), now = Math.floor(Date.now() / 1000);
  if (now > latest + 2) { await call(NODE, "evm_setNextBlockTimestamp", [now]); await call(NODE, "evm_mine"); log({ event: "clock-brought-to-now", wasBehindS: now - latest }); }
}
log({ event: "node-up", chainId: reported, forkBlock, upstream: new URL(UPSTREAM).host });

try { await bootstrap({ node: NODE, data: DATA, repo: REPO, log }); }
catch (err) { log({ event: "bootstrap-failed", error: String((err as Error).message).slice(-600) }); anvil.kill(); process.exit(1); }   // never leave a node behind
/**
 * docs/phase11.md §8. Every boot, not only the first: the roles must hold far more than anvil's balance check asks for.
 *
 * Our node forks Robinhood Chain, and with it a block gas limit of 2^50. When anvil cannot estimate a transaction — which is
 * exactly when the call would revert — it checks the sender's balance against gas_limit x gas_price, and at one gwei that asks
 * for about 1,125,900 ETH. On the 100 staging ETH the first bootstrap gave them, EVERY reverting call came back as
 * "Insufficient funds for gas * price + value" instead of its own error, and a real refusal looked like an empty account.
 * The number below is meaningless — this chain's ETH is its own, and nobody holds a key to any of these addresses — and it
 * makes the node report the real reason. `anvil_setBalance` is our own node's method; the public proxy refuses it to everyone.
 */
const ROLE_FLOOR = 2_000_000n * 10n ** 18n;
try {
  const d = JSON.parse(readFileSync(`${DATA}/deployment.json`, "utf8")) as Record<string, string>;
  for (const role of ["deployer", "owner", "treasury", "keeper", "purchaser", "attestor"] as const) {
    const who = d[role];
    if (!who) continue;
    const have = BigInt(await call<string>(NODE, "eth_getBalance", [who, "latest"]));
    if (have >= ROLE_FLOOR) continue;
    await call(NODE, "anvil_setBalance", [who, `0x${ROLE_FLOOR.toString(16)}`]);
    log({ event: "role-funded", role, wasWei: String(have), nowWei: String(ROLE_FLOOR),
      note: "staging only: under about 1,125,900 ETH this fork's 2^50 gas limit makes anvil report every revert as insufficient funds" });
  }
} catch (err) { log({ event: "role-funding-skipped", why: String((err as Error).message).slice(0, 200) }); }

const waitS = Number(env.STAGING_WAIT_S ?? 120);
const p = proxy({ node: NODE, data: DATA, port: PORT, chainId: CHAIN_ID, forkBlock, faucetWei: BigInt(env.STAGING_FAUCET_WEI ?? "10000000000000000000"), publicUrl: env.STAGING_PUBLIC_URL ?? "", waitS, faucetPerIp: Number(env.STAGING_FAUCET_PER_IP ?? 5), upstream: shim.stats, log });
await p.listen();
log({ event: "proxy-up", port: PORT });

// docs/phase11.md §6. The catalogue's 58 vaults are 58 contract deployments, so they fill BESIDE the proxy that is already
// answering, in batches, with /healthz reporting the count. A restart finishes an unfinished inventory rather than repeating it.
if ((env.STAGING_LISTINGS ?? "1") !== "0") {
  void createListings({ node: NODE, data: DATA, repo: REPO, batch: Number(env.STAGING_LISTINGS_BATCH ?? 8), log })
    .catch((err) => log({ event: "listings-failed", error: String((err as Error).message).slice(-600) }));
}

let operator: ReturnType<typeof spawn> | null = null;
const startOperator = () => {
  operator = spawn("npx", ["tsx", "staging/operator.ts"], { cwd: `${REPO}/keeper`, stdio: ["ignore", "inherit", "inherit"],
    env: { ...env, KEEPER_MODE: "staging", STAGING_NODE: NODE, STAGING_DATA: DATA, STAGING_WAIT_S: String(waitS) } });
  operator.on("exit", (code) => { log({ event: "operator-exited", code }); if (!stopping) setTimeout(startOperator, 5_000); });
};
// docs/phase8.md: the simulators, beside the node so they can use its own admin methods, which the public proxy refuses to
// everyone else. They make a market for the operator to work on, so a visitor meets a product in use, not an empty grid.
let sim: ReturnType<typeof spawn> | null = null;
const startSim = () => {
  sim = spawn("npx", ["tsx", "sim/sim.ts"], { cwd: `${REPO}/staging`, stdio: ["ignore", "inherit", "inherit"],
    env: { ...env, STAGING_NODE: NODE, STAGING_DATA: DATA } });
  sim.on("exit", (code) => { log({ event: "sim-exited", code }); if (!stopping) setTimeout(startSim, 10_000); });
};

let stopping = false;
if (env.STAGING_NO_OPERATOR !== "1") startOperator();
if ((env.STAGING_SIM ?? "1") !== "0") startSim();
for (const sig of ["SIGTERM", "SIGINT"] as const) process.on(sig, () => { stopping = true; operator?.kill(); sim?.kill(); anvil.kill("SIGTERM"); setTimeout(() => process.exit(0), 8_000).unref(); });
