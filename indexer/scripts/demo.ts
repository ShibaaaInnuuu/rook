import { spawn, spawnSync } from "node:child_process";
import { existsSync, rmSync } from "node:fs";

/**
 * docs/phase4.md §1 — `npm run demo` at the repo root. Everything is LOCAL: a fork of Robinhood Chain on 127.0.0.1 (chain id 31337,
 * anvil's public throwaway accounts), the Phase 2 end-to-end run against it, then the indexer and its API left serving that data.
 * Nothing touches a public chain, and every Bitcoin value in the data is a FIXTURE: nothing was bought.
 */
const ROOT = new URL("../../", import.meta.url).pathname;
const RPC = process.env.LOCAL_RPC ?? "http://127.0.0.1:8545", PORT = process.env.PORT ?? "8787";
const env = { ...process.env, PATH: `${process.env.HOME}/.foundry/bin:${process.env.PATH}` };
const sleep = (ms: number) => new Promise((ok) => setTimeout(ok, ms));
const chain = async (): Promise<number | null> => {
  try {
    const r = await fetch(RPC, { method: "POST", headers: { "content-type": "application/json" }, body: '{"jsonrpc":"2.0","id":1,"method":"eth_chainId","params":[]}' });
    return Number(BigInt(((await r.json()) as { result: string }).result));
  } catch { return null; }
};
const children: ReturnType<typeof spawn>[] = [];
const stop = () => { for (const c of children) c.kill(); };
process.on("exit", stop); process.on("SIGINT", () => process.exit(0)); process.on("SIGTERM", () => process.exit(0));

let id = await chain();
if (id === null) {
  console.log("· starting a local fork of Robinhood Chain (public RPC), chain id 31337");
  children.push(spawn("anvil", ["--fork-url", "https://rpc.mainnet.chain.robinhood.com", "--chain-id", "31337", "--port", new URL(RPC).port || "8545", "--silent"], { env, stdio: "ignore" }));
  for (let i = 0; i < 120 && (id = await chain()) === null; i++) await sleep(250);
}
if (id !== 31337) { console.error(`refusing: ${RPC} reports chain id ${id}, and the demo only ever runs against a local node that says 31337`); process.exit(2); }

for (const pkg of ["keeper", "tools/snapshot", "indexer"]) {
  if (!existsSync(`${ROOT}${pkg}/node_modules`)) { console.log(`· npm install in ${pkg}`); spawnSync("npm", ["install", "--silent"], { cwd: ROOT + pkg, stdio: "inherit", env }); }
}
console.log("· running the local end-to-end (about a minute): launch, trade, harvest, declare, release, FIXTURE receipt, snapshot, a real drand draw, claim, FIXTURE delivery");
const e2e = spawnSync("npm", ["run", "-s", "e2e:local"], { cwd: ROOT + "keeper", stdio: "inherit", env: { ...env, KEEPER_MODE: "local", LOCAL_RPC: RPC } });
if (e2e.status !== 0) { console.error("the end-to-end run failed; nothing is served"); process.exit(1); }

rmSync(`${ROOT}indexer/data/demo.db`, { force: true }); rmSync(`${ROOT}indexer/data/demo.db-wal`, { force: true }); rmSync(`${ROOT}indexer/data/demo.db-shm`, { force: true });
children.push(spawn("npm", ["run", "-s", "start"], { cwd: ROOT + "indexer", stdio: "inherit", env: { ...env, RPC_URL: RPC, PORT, DB_PATH: `${ROOT}indexer/data/demo.db`, POLL_MS: "2000" } }));
for (let i = 0; i < 60; i++) { try { if ((await fetch(`http://127.0.0.1:${PORT}/v1/health`)).ok) break; } catch { /* not up yet */ } await sleep(500); }
console.log(`\netch demo is up — LOCAL data, FIXTURE Bitcoin values, nothing was bought\n  API      http://127.0.0.1:${PORT}/v1/health   /v1/stats  /v1/coins  /v1/listings  /v1/acquisitions  /v1/draws\n  chain    ${RPC} (anvil fork, chain id 31337)\n  refresh the committed fixtures:  (cd indexer && npm run fixtures -- demo http://127.0.0.1:${PORT})\nCtrl-C stops the API${children.length > 1 ? " and the fork" : ""}.`);
await new Promise(() => {});
