import { randomBytes } from "node:crypto";
import { spawn } from "node:child_process";
import { chmodSync, copyFileSync, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { STAGING_CHAIN_ID } from "../chain-id.js";

/**
 * docs/phase6.md. First boot of the staging fork: roles, the guarded deployment, the file the proxy publishes.
 *
 * THERE IS NO KEY. Each role is twenty random bytes: an address nobody can sign for, and so one that cannot carry the sweeper
 * delegations docs/phase2.md found on the well-known development addresses. The node sends for them (it sends for anyone who asks
 * from this machine), and the deployment goes through the same wrapper as every other one: `deploy-etch:<the staging chain id>:<deployer>`.
 */
export const ALLOWED_CHAINS: readonly number[] = [STAGING_CHAIN_ID, 31337];
const thisMachine = (url: string) => { try { return ["127.0.0.1", "localhost"].includes(new URL(url).hostname); } catch { return false; } };

export function assertBootstrapTarget(chainId: number, url: string): void {
  if (chainId === 4663 || chainId === 46630) throw new Error(`refused: chain ${chainId} is a real network (Robinhood Chain, or its public testnet); the bootstrap never deploys there`);
  if (!ALLOWED_CHAINS.includes(chainId)) throw new Error(`refused: the bootstrap deploys to our own chains (${ALLOWED_CHAINS.join(", ")}), and this node reports ${chainId}`);
  if (!thisMachine(url)) throw new Error("refused: the bootstrap talks to a node on this machine only");
}

export async function bootstrap(o: { node: string; data: string; repo: string; fetchImpl?: typeof fetch; log?: (r: Record<string, unknown>) => void }): Promise<Record<string, unknown>> {
  const f = o.fetchImpl ?? fetch, log = o.log ?? (() => {});
  const rpc = async <T>(method: string, params: unknown[] = []): Promise<T> => {
    const r = await f(o.node, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params }) });
    const j = (await r.json()) as { result?: T; error?: { message: string } };
    if (j.error) throw new Error(`${method}: ${j.error.message}`);
    return j.result as T;
  };
  const chainId = Number(BigInt(await rpc<string>("eth_chainId")));
  assertBootstrapTarget(chainId, o.node);
  if (existsSync(`${o.data}/deployment.json`)) return JSON.parse(readFileSync(`${o.data}/deployment.json`, "utf8")) as Record<string, unknown>;

  const fresh = () => `0x${randomBytes(20).toString("hex")}`;
  const roles = { deployer: fresh(), owner: fresh(), treasury: fresh(), keeper: fresh(), purchaser: fresh(), attestor: fresh() };
  for (const a of Object.values(roles)) {
    if ((await rpc<string>("eth_getCode", [a, "latest"])) !== "0x") throw new Error("a random address has code on the fork; run again");
    // docs/phase11.md §8: two million staging ETH, not a hundred. The fork carries Robinhood Chain's 2^50 block gas limit, and
    // anvil checks a sender's balance against gas_limit x gas_price whenever it cannot estimate — which is exactly when a call
    // would revert. Under about 1,125,900 ETH every revert came back as "Insufficient funds" instead of its own error.
    await rpc("anvil_setBalance", [a, "0x1a784379d99db42000000"]);   // 2,000,000 staging ETH; this chain's ETH is its own
  }
  // NOT spawnSync: the node reads the real chain through a shim in THIS process (chain/upstream.ts), and a blocked event loop
  // would starve the very reads the deployment is waiting for.
  const out = await new Promise<{ status: number | null; stdout: string; stderr: string }>((done) => {
    const c = spawn("bash", [`${o.repo}/script/deploy.sh`, o.node, roles.deployer], { stdio: ["ignore", "pipe", "pipe"], env: { ...process.env,
      ETCH_DEPLOY_CONFIRM: `deploy-etch:${chainId}:${roles.deployer}`, ETCH_OWNER: roles.owner, ETCH_TREASURY: roles.treasury, ETCH_KEEPER: roles.keeper,
      ETCH_PURCHASER: roles.purchaser, ETCH_ATTESTOR: roles.attestor } });
    let stdout = "", stderr = "";
    c.stdout.on("data", (d) => { stdout += String(d); }); c.stderr.on("data", (d) => { stderr += String(d); });
    c.on("close", (status) => done({ status, stdout, stderr }));
  });
  if (out.status !== 0) throw new Error(`the deployment failed: ${(out.stderr || out.stdout).slice(-800)}`);
  mkdirSync(o.data, { recursive: true });
  copyFileSync(`${o.repo}/deployments/${chainId}.json`, `${o.data}/deployment.json`);
  chmodSync(`${o.data}/deployment.json`, 0o644);
  writeFileSync(`${o.data}/roles.txt`, "No key exists for any role on this chain. Each is twenty random bytes; the node sends for them, from this machine only.\n");
  const d = JSON.parse(readFileSync(`${o.data}/deployment.json`, "utf8")) as Record<string, unknown>;
  log({ event: "bootstrapped", chainId, registry: d.registry, launcher: d.launcher, distributor: d.distributor, keys: "none exist" });
  return d;
}
