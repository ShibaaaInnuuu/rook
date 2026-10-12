// `npm run e2e:v2` — everything this branch can actually prove, in one run, on this machine.
//
// It runs the pure security core and the loop as tests, then starts a local ICP replica, installs the canister, and asks it
// for the two addresses it derives from keys nobody holds. What it cannot do is buy anything: that needs UniSat's written
// permission for commercial use, and a real bridge transfer. Those hops are fixtures, and the run says so at the end.
import { execFileSync, execSync } from "node:child_process";
import { existsSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

const DFX = join(homedir(), "Library/Application Support/org.dfinity.dfx/bin");
const CARGO = join(homedir(), ".cargo/bin");
const FOUNDRY = join(homedir(), ".foundry/bin");
const env = { ...process.env, PATH: `${DFX}:${CARGO}:${FOUNDRY}:${process.env.PATH}` };
const rs = join(import.meta.dirname, "canister-rs");

const step = (s) => console.log(`\n\x1b[1m${s}\x1b[0m`);
const run = (cmd, args, opts = {}) =>
  execFileSync(cmd, args, { env, encoding: "utf8", stdio: "pipe", ...opts });

let replicaStarted = false;
try {
  step("1. The security core and the state machine");
  const unit = run("cargo", ["test", "--lib"], { cwd: rs });
  console.log(unit.split("\n").filter((l) => l.startsWith("test result")).join("\n"));

  step("2. The whole loop, hop by hop");
  console.log(
    run("cargo", ["test", "--test", "e2e", "--", "--nocapture"], { cwd: rs })
      .split("\n")
      .filter((l) => /^\s+\d+\.|^  (Real|Fixture|Not proven|A bridge)|^test result/.test(l))
      .join("\n")
  );

  step("3. The vault's bounds, which replace a bond");
  console.log(
    run("forge", ["test"], { cwd: import.meta.dirname })
      .split("\n")
      .filter((l) => /^Suite result|tests passed/.test(l))
      .join("\n")
  );

  step("4. The canister on a local replica: two keys nobody holds");
  if (!existsSync(join(DFX, "dfx"))) throw new Error(`no dfx at ${DFX}`);
  try { run("dfx", ["stop"], { cwd: rs }); } catch { /* nothing was running */ }
  execSync("dfx start --clean --background >/tmp/etch-e2e-dfx.log 2>&1", { cwd: rs, env });
  replicaStarted = true;
  await new Promise((r) => setTimeout(r, 8000));

  // Two DISTINCT providers on each side: `init` refuses a repeat, because `rpc()` demands that every provider return the
  // same body and one URL listed twice satisfies that trivially. Testnet endpoints throughout -- this run derives
  // addresses and never makes a call, so naming a mainnet host here bought nothing and risked something.
  const init = '(record { chain_id = 46630 : nat64; vault = "0x0000000000000000000000000000000000000001"; '
    + 'rpc_urls = vec { "https://rpc.testnet.chain.robinhood.com"; "https://explorer.testnet.chain.robinhood.com/api/eth-rpc" }; '
    + 'btc_api_urls = vec { "https://mempool.space/signet/api"; "https://open-api-signet.unisat.io" }; '
    + 'key_name = "key_1"; accept_production_key_name = true; bridge_allowlist = vec {}; btc_network = "signet" })';
  run("dfx", ["deploy", "autobuyer", "--mode", "reinstall", "--yes", "--argument", init], { cwd: rs });
  const pick = (s) => (s.match(/Ok = "([^"]+)"/) || [])[1] ?? s.trim();
  console.log(`   taproot key (BIP340, threshold): ${pick(run("dfx", ["canister", "call", "autobuyer", "taproot_key"], { cwd: rs }))}`);
  console.log(`   EVM address (ECDSA, threshold):  ${pick(run("dfx", ["canister", "call", "autobuyer", "evm_address"], { cwd: rs }))}`);

  step("5. The guards refuse what they are there to refuse");
  const bad = init.replace(/rpc_urls = vec \{[^}]*\}/, 'rpc_urls = vec { "https://only-one" }');
  try {
    run("dfx", ["deploy", "autobuyer", "--mode", "reinstall", "--yes", "--argument", bad], { cwd: rs });
    throw new Error("a single RPC provider was accepted, and it must not be");
  } catch (e) {
    const msg = `${e.stdout ?? ""}${e.stderr ?? ""}${e.message}`;
    if (!msg.includes("two RPC providers")) throw e;
    console.log("   one RPC provider → refused at install: 'two RPC providers at least: one is not a check'");
  }

  console.log("\n\x1b[1mWhat this run did not prove\x1b[0m");
  console.log("   A purchase at a venue: UniSat's terms need written permission for commercial use (§3.1), asked for, not given.");
  console.log("   A real bridge transfer, a real Bitcoin broadcast, and anything that needs a multi-node subnet.");
  console.log("   Local replica only. Nothing was deployed to ICP, no real key exists, nothing was sent to Bitcoin or 4663.\n");
} finally {
  if (replicaStarted) {
    try { run("dfx", ["stop"], { cwd: rs }); } catch { /* it is going away anyway */ }
  }
}
