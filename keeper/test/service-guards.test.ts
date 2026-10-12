import assert from "node:assert/strict";
import { test } from "node:test";
import { readFileSync, readdirSync, existsSync } from "node:fs";
import { MODULE, VAULT, VAULT_PUBLIC, WRAPPER, WRAPPER_PUBLIC } from "../service/abis.js";
import { ChainRpc, METHODS } from "../service/rpc.js";

// docs/v6/p1-services.md §1.1: where the production keeper's key may be read, what it may call, and what it may never do.
const dir = (d: string) => readdirSync(new URL(`../${d}/`, import.meta.url)).filter((f) => f.endsWith(".ts")).map((f) => [`${d}/${f}`, readFileSync(new URL(`../${d}/${f}`, import.meta.url), "utf8")] as const);
const noComments = (s: string) => s.replace(/\/\*[\s\S]*?\*\//g, "").replace(/(^|[^:])\/\/.*$/gm, "$1");

test("KEEPER_PRIVATE_KEY and RESOLVER_PRIVATE_KEY are read in service/signer.ts and nowhere else in the package; that file prints nothing", () => {
  const all = [...dir("src"), ...dir("local"), ...dir("staging"), ...dir("live"), ...dir("service")];
  const readers = all.filter(([, s]) => /KEEPER_PRIVATE_KEY|RESOLVER_PRIVATE_KEY/.test(s)).map(([f]) => f);
  // other files name them only in prose (comments) and messages: the values are read in signer.ts alone
  const code = all.filter(([, s]) => /KEEPER_PRIVATE_KEY|RESOLVER_PRIVATE_KEY|\bVAR\]|\bVARS\[/.test(noComments(s).replace(/"[^"\n]*"|`[^`\n]*`/g, '""'))).map(([f]) => f);
  assert.deepEqual(code, ["service/signer.ts"], `read in: ${readers.join(", ")}`);
  const signer = readFileSync(new URL("../service/signer.ts", import.meta.url), "utf8");
  assert.ok(!/console\.|process\.stdout|process\.stderr|\blog\(/.test(noComments(signer)), "the signer prints nothing");
  assert.ok(!/export (const|let|var) .*key/i.test(signer), "the key is not exported");
});

test("in service/, only signer.ts builds an account; nothing asks a node to sign (eth_sendTransaction, eth_sign, personal_sign)", () => {
  for (const [f, s] of dir("service")) {
    const c = noComments(s);
    if (f !== "service/signer.ts") assert.ok(!/viem\/accounts|privateKeyToAccount|mnemonicToAccount|createWalletClient|signTransaction\(/.test(c), `${f} must not build an account`);
    assert.ok(!/eth_sendTransaction|eth_sign\b|personal_sign|eth_signTypedData/.test(c), `${f} names a node-signing method`);
  }
  // exactly one method on the list sends, and the read path refuses it
  assert.deepEqual(METHODS.filter((m) => /send|sign/i.test(m)), ["eth_sendRawTransaction"]);
  const rpc = new ChainRpc(["http://127.0.0.1:1"], 4663, (async () => { throw new Error("no request may be made"); }) as unknown as typeof fetch);
  return assert.rejects(rpc.call("eth_sendRawTransaction", ["0x"]), /go through sendRaw/);
});

test("what each role may call is closed: the keeper postFloor, postCap, announceDraw, openDraw; the resolver only calls anyone may make; nobody records a delivery", () => {
  const writes = (abi: readonly unknown[]) => (abi as { type: string; name: string; stateMutability?: string }[]).filter((x) => x.type === "function" && !["view", "pure"].includes(x.stateMutability ?? "")).map((x) => x.name).sort();
  assert.deepEqual(writes(MODULE), ["postFloor"]);
  assert.deepEqual(writes(VAULT), ["announceDraw", "openDraw", "postCap"]);
  assert.deepEqual(writes(VAULT_PUBLIC), ["claim", "requestDraw", "resolve", "voidDraw"], "SweepVault's permissionless draw calls");
  assert.deepEqual(writes(WRAPPER_PUBLIC), ["resolve"]);
  assert.deepEqual(writes(WRAPPER), [], "the Wrapper is only read otherwise");
  // B-03/C-05: Wrapper.recordDelivery is the custodian's alone; no service ABI names it
  for (const [f, src] of dir("service")) assert.ok(!/recordDelivery/.test(noComments(src)), `${f} names recordDelivery`);
  // the vault calls are made against the RAFFLE and BURN vaults of the stack file, never its YOYO vault
  const keeper = noComments(readFileSync(new URL("../service/keeper.ts", import.meta.url), "utf8"));
  assert.ok(!/yoyoVault/.test(keeper), "keeper.ts never names the YOYO vault");
  assert.equal((keeper.match(/sender\.submit\(/g) ?? []).length, 4, "four keeper submits: the floor, a cap, an announcement, an opening");
  assert.ok(!/VAULT_PUBLIC|WRAPPER_PUBLIC/.test(keeper), "the keeper's own code sends no resolver call");
  // the resolver encodes only the permissionless ABIs, never a keeper power
  const resolver = noComments(readFileSync(new URL("../service/resolver.ts", import.meta.url), "utf8"));
  assert.ok(!/"(postFloor|postCap|announceDraw|openDraw)"/.test(resolver), "the resolver never encodes a keeper call");
  assert.ok(!/abi: VAULT,|abi: MODULE/.test(resolver), "the resolver encodes with VAULT_PUBLIC / WRAPPER_PUBLIC only");
});

test("no script arms send mode for you, and the image does not either: KEEPER_MODE=send is the operator's to set", () => {
  const pkg = JSON.parse(readFileSync(new URL("../package.json", import.meta.url), "utf8")) as { scripts: Record<string, string>; dependencies: Record<string, string> };
  assert.equal(pkg.scripts.service, "tsx service/main.ts");
  assert.ok(!Object.values(pkg.scripts).some((v) => /KEEPER_MODE=send|KEEPER_PRIVATE_KEY/.test(v)));
  assert.deepEqual(Object.keys(pkg.dependencies).sort(), ["tsx", "viem"], "viem, and tsx that runs the TypeScript (R2-S7: production dependencies only in the image)");
  const docker = new URL("../Dockerfile", import.meta.url);
  // what the image DOES (its instructions), not what its comments tell the operator to do
  const instructions = existsSync(docker) ? readFileSync(docker, "utf8").split("\n").filter((l) => !l.trimStart().startsWith("#")).join("\n") : "";
  assert.ok(instructions.includes("service/main.ts"), "the image runs the service");
  assert.ok(!/KEEPER_MODE|KEEPER_PRIVATE_KEY|UNISAT_API_KEY/.test(instructions), "the image arms nothing and carries no key");
});
