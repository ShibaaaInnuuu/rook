import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { decodeFunctionData, encodeFunctionData, type Abi } from "viem";
import { targetRefusal, stackFrom, type StackReader } from "../src/rook/phase.js";
import { ERC20_APPROVE } from "../src/lib/abisV3.js";

/**
 * AUDIT E (services and workflows) proof of concept; no source is changed. The Phase-1 send guard checks WHERE a transaction goes
 * (`targetRefusal`), never WHAT it says. The sell ticket builds `coin.approve(router, amount)` with `router` taken from the
 * indexer's `/v1/wrapped/<slug>` answer (`col.contracts.router`, src/ui/Ticket.tsx:41,71). A compromised or spoofed indexer can
 * therefore make the site ask the visitor to approve an attacker as the spender, and the guard lets it through: the target is a
 * coin the stack's factory launched. Run: cd web-rook && npx tsx --test test/audit_e.test.ts
 */
const A = (n: number) => `0x${n.toString(16).padStart(40, "0")}` as `0x${string}`;
const STACK = { chainId: 4663, label: "stack", stage: 0, registry: A(1), owner: A(2), keeper: A(3), treasury: A(4), poolManager: A(6),
  collections: [{ slug: "frogs", kind: "inscriptions", wrapper: A(0x10), factory: A(0x11), router: A(0x12), floorModule: A(0x13) }] };
const COIN = A(0xc0), CURVE = A(0xcc), ATTACKER = A(0xbad0bad);

test("E-07 PoC: the Phase-1 guard passes coin.approve(<attacker>, amount) built from an indexer-supplied router", async () => {
  const s = stackFrom([STACK]).stack!;
  const read: StackReader = { routerOf: async (f, coin) => (f.toLowerCase() === A(0x11) && coin.toLowerCase() === COIN ? CURVE : A(0)), tokenOf: async () => null };
  // what a hostile /v1/wrapped/frogs answer carries: the real coin, a router that is not the stack's
  const col = { contracts: { router: ATTACKER } };
  // exactly what Ticket.tsx:71 builds when the visitor presses "Allow … to be sold"
  const req = { address: COIN, abi: ERC20_APPROVE as Abi, functionName: "approve", args: [col.contracts.router, 10n ** 24n] };
  assert.equal(await targetRefusal(s, req.address, read), null, "the guard only looks at req.address: allowed");
  const data = encodeFunctionData({ abi: req.abi, functionName: "approve", args: req.args });
  const d = decodeFunctionData({ abi: req.abi, data });
  console.log(`E-07: wallet is asked to sign approve(${String(d.args?.[0])}, ${String(d.args?.[1])}) on ${COIN}; guard verdict: allowed`);
  assert.equal(String(d.args?.[0]).toLowerCase(), ATTACKER, "the spender is the indexer's, not the stack's router");
});

test("E-07 context: the sell and approve paths read the router from the indexer, not from the bundled stack file", () => {
  const src = readFileSync(new URL("../src/ui/Ticket.tsx", import.meta.url), "utf8");
  assert.match(src, /const router = col\.contracts\.router;/);
  assert.match(src, /functionName: "approve", args: \[router!, wei\]/);
  assert.doesNotMatch(src, /bundledStack|stackCollection/, "no cross-check against the stack's router in the ticket");
});
