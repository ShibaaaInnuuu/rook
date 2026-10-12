import { writeFileSync } from "node:fs";
import { openDb } from "../src/db.js";
import { Follower, loadDeployment } from "../src/follow.js";
import { Live } from "../src/live.js";
import { rpcOf, type Rpc } from "../src/rpc.js";
import { Views } from "../src/views.js";

/**
 * Records every JSON-RPC answer one full index + one read of every view needs, from a node that has just run the local e2e, so that
 * the integration test can replay the whole thing in CI with no anvil. Usage: tsx scripts/record.ts [rpc] [deployments dir] [out]
 */
const url = process.argv[2] ?? "http://127.0.0.1:8545", dir = process.argv[3] ?? new URL("../../deployments", import.meta.url).pathname;
const out = process.argv[4] ?? new URL("../test/fixtures/e2e-rpc.json", import.meta.url).pathname;
const real = rpcOf(url), tape: Record<string, unknown> = {};
const rpc: Rpc = async <T>(method: string, params: unknown[]) => { const v = await real<T>(method, params); tape[`${method}|${JSON.stringify(params)}`] = v; return v; };
const chainId = Number(BigInt(await rpc<string>("eth_chainId", [])));
const deployment = loadDeployment(dir, chainId);
if (!deployment) throw new Error(`no deployment file for chain ${chainId} in ${dir}`);
const db = openDb(":memory:");
const follower = new Follower({ db, rpc, deployment, confirmations: 0 });
const n = await follower.pass();
const views = new Views(db, new Live(rpc, deployment.ponsFactory));
for (const c of views.coins(200, 0).items) await views.coin(c.address);
for (const l of views.listings().items) await views.listing(l.id);
writeFileSync(out, JSON.stringify({ recordedAt: new Date().toISOString(), note: "a local anvil fork after keeper/local/e2e.ts; every Bitcoin value in it is a FIXTURE", deployment, tape }, null, 1) + "\n");
console.log(`recorded ${Object.keys(tape).length} answers for ${n} events -> ${out}`);
