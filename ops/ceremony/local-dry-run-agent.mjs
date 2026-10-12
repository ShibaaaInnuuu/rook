// The local dry run's caller (ops/ceremony/local-dry-run.sh): throwaway Ed25519 identities held in a file in the dry run's temp directory
// (no dfx identity is created), and one operator-only call made as one of them, to show the installed `operator` is the one named.
//   node ops/ceremony/local-dry-run-agent.mjs new <key file>                        → the identity's principal
//   node ops/ceremony/local-dry-run-agent.mjs sol_address <key file> <host> <canister id>  → "ok <answer>" | "rejected <why>"
import { readFileSync, writeFileSync } from "node:fs";
import { randomBytes } from "node:crypto";

const ROOT = new URL("../../", import.meta.url).pathname;
const { Ed25519KeyIdentity } = await import(ROOT + "ops/node_modules/@icp-sdk/core/lib/esm/identity/index.js");
const { HttpAgent, Actor } = await import(ROOT + "ops/node_modules/@icp-sdk/core/lib/esm/agent/index.js");

const [cmd, file, host, canisterId] = process.argv.slice(2);
if (cmd === "new") {
  const id = Ed25519KeyIdentity.generate(new Uint8Array(randomBytes(32)));
  writeFileSync(file, JSON.stringify(id.toJSON()), { mode: 0o600 });
  process.stdout.write(id.getPrincipal().toText() + "\n");
} else if (cmd === "sol_address") {
  const identity = Ed25519KeyIdentity.fromJSON(readFileSync(file, "utf8"));
  const agent = await HttpAgent.create({ host, identity, shouldFetchRootKey: true });
  const idl = ({ IDL }) => IDL.Service({ sol_address: IDL.Func([], [IDL.Variant({ Ok: IDL.Text, Err: IDL.Text })], []) });
  try {
    const r = await Actor.createActor(idl, { agent, canisterId }).sol_address();
    process.stdout.write(`ok ${JSON.stringify(r)}\n`);
  } catch (e) {
    process.stdout.write(`rejected ${String(e.message ?? e).split("\n").filter(Boolean).slice(0, 4).join(" | ")}\n`);
  }
} else {
  process.stderr.write("usage: local-dry-run-agent.mjs new <file> | sol_address <file> <host> <canister>\n");
  process.exit(2);
}
