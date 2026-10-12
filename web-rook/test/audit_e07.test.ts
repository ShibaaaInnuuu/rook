import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import type { Abi } from "viem";
import { CALLS, pinnedAddress, requestRefusal, stackFrom, type StackFile, type StackReader } from "../src/rook/phase.js";
import { ERC20_APPROVE, ETCH_ROUTER } from "../src/lib/abisV3.js";
import { CURVE_FACTORY, FROG_CURVE_P1, RAFFLE_VAULT, WRAPPER_P1 } from "../src/rook/abis.js";

/**
 * E-07 (docs/audit/2026-09-26/E-services-workflow.md), fixed as docs/audit/2026-09-26/fixes-W2-site.md §2: the audit's PoC, turned
 * round. The same hostile indexer answer (the real coin, a router that is not the stack's) is now refused before the wallet is asked,
 * and every address a request carries has to pass a rule.
 */
const A = (n: number) => `0x${n.toString(16).padStart(40, "0")}` as `0x${string}`;
const STACK = { chainId: 4663, label: "stack", stage: 0, registry: A(1), owner: A(2), keeper: A(3), treasury: A(4), poolManager: A(6),
  collections: [{ slug: "frogs", kind: "inscriptions", wrapper: A(0x10), factory: A(0x11), router: A(0x12), floorModule: A(0x13), raffleVault: A(0x16), yoyoVault: A(0x18) }] };
const COIN = A(0xc0), CURVE = A(0xcc), ATTACKER = A(0xbad0bad), ME = A(0x3e), OTHER_COIN = A(0xc9);
const s = stackFrom([STACK]).stack as StackFile;
const read: StackReader = {
  routerOf: async (f, coin) => (f.toLowerCase() === A(0x11) && coin.toLowerCase() === COIN ? CURVE : A(0)),
  tokenOf: async (a) => (a.toLowerCase() === CURVE ? COIN : null),
};
const req = (address: string, abi: readonly unknown[], functionName: string, args: readonly unknown[]) => ({ address, abi: abi as Abi, functionName, args });

test("E-07 regression: coin.approve(<an indexer-supplied router>) is refused, naming the spender and the stack's router", async () => {
  const col = { contracts: { router: ATTACKER } };   // what a hostile /v1/wrapped/frogs answer carries
  const why = await requestRefusal(s, req(COIN, ERC20_APPROVE, "approve", [col.contracts.router, 10n ** 24n]), ME, read, COIN);
  assert.ok(why, "refused");
  assert.match(why!, new RegExp(ATTACKER, "i"));
  assert.match(why!, /not this deployment's router/);
  assert.equal(await requestRefusal(s, req(COIN, ERC20_APPROVE, "approve", [A(0x12), 10n ** 24n]), ME, read, COIN), null, "the stack's own router is allowed");
});

test("E-07: a router trade must pay the connected account and trade a coin the factory launched", async () => {
  const dl = 2_000_000_000n;
  assert.equal(await requestRefusal(s, req(A(0x12), ETCH_ROUTER, "buy", [COIN, 1n, ME, dl]), ME, read, COIN), null);
  assert.match((await requestRefusal(s, req(A(0x12), ETCH_ROUTER, "buy", [COIN, 1n, ATTACKER, dl]), ME, read, COIN))!, /not your connected account/);
  assert.match((await requestRefusal(s, req(A(0x12), ETCH_ROUTER, "sell", [COIN, 5n, 1n, ATTACKER, dl]), ME, read, COIN))!, /not your connected account/);
  assert.match((await requestRefusal(s, req(A(0x12), ETCH_ROUTER, "buy", [COIN, 1n, ME, dl]), null, read, COIN))!, /not your connected account/, "no account connected: no recipient can be vouched for");
  assert.match((await requestRefusal(s, req(A(0x12), ETCH_ROUTER, "buy", [OTHER_COIN, 1n, ME, dl]), ME, read, OTHER_COIN))!, /not a coin this deployment's factory launched/);
});

test("E-07: only the calls the site makes, per kind of target; an unknown address argument is refused by default", async () => {
  assert.match((await requestRefusal(s, req(COIN, [{ type: "function", name: "transfer", stateMutability: "nonpayable", inputs: [{ name: "to", type: "address" }, { name: "amount", type: "uint256" }], outputs: [] }], "transfer", [ATTACKER, 1n]), ME, read, COIN))!, /does not call transfer on a coin/);
  assert.match((await requestRefusal(s, req(A(0x10), WRAPPER_P1, "approve", [ATTACKER, 1n]), ME, read))!, /does not call approve on a wrapper/);
  // a future call whose argument carries an address and has no rule fails closed
  const hypothetical = [{ type: "function", name: "claim", stateMutability: "nonpayable", inputs: [{ name: "id", type: "uint256" }, { name: "holder", type: "address" }, { name: "firstTicket", type: "uint256" }, { name: "endTicket", type: "uint256" }, { name: "proof", type: "bytes32[]" }, { name: "payTo", type: "address" }], outputs: [] }];
  assert.match((await requestRefusal(s, req(A(0x16), hypothetical, "claim", [0n, ME, 0n, 1n, [], ATTACKER]), ME, read))!, /no rule for/);
  // addresses inside tuples and arrays are walked too
  const tupled = [{ type: "function", name: "launch", stateMutability: "nonpayable", inputs: [{ name: "p", type: "tuple", components: [{ name: "name", type: "string" }, { name: "beneficiary", type: "address" }] }], outputs: [] }];
  assert.match((await requestRefusal(s, req(A(0x11), tupled, "launch", [{ name: "x", beneficiary: ATTACKER }]), ME, read))!, /no rule for/);
  assert.match((await requestRefusal(s, req(A(0x11), CURVE_FACTORY, "launch", ["A", "B", 0]), ME, read))!, /does not match its ABI/, "wrong argument count");
  assert.match((await requestRefusal(s, req(A(0x18), RAFFLE_VAULT, "requestDraw", [0n]), ME, read))!, /YOYO vault, which is off/);
});

test("E-07: the honest requests every page builds pass", async () => {
  const dl = 2_000_000_000n;
  for (const [r, why] of [
    [req(A(0x11), CURVE_FACTORY, "launch", ["Frog", "FRG", 0, false, 300]), "launch"],
    [req(CURVE, FROG_CURVE_P1, "buy", [1n, dl]), "curve buy with deadline"],
    [req(CURVE, FROG_CURVE_P1, "sell", [5n, 1n, dl]), "curve sell with deadline"],
    [req(CURVE, FROG_CURVE_P1, "graduate", []), "graduate"],
    [req(CURVE, FROG_CURVE_P1, "openRefunds", []), "open refunds"],
    [req(A(0x10), WRAPPER_P1, "redeem", ["bc1p..."]), "redeem"],
    [req(A(0x10), WRAPPER_P1, "resolve", [3n, "0x"]), "resolve a ticket"],
    [req(A(0x10), WRAPPER_P1, "refund", [3n]), "refund a ticket"],
    [req(A(0x16), RAFFLE_VAULT, "requestDraw", [0n]), "pin a draw"],
    [req(A(0x16), RAFFLE_VAULT, "resolve", [0n, "0x"]), "resolve a draw"],
    [req(A(0x16), RAFFLE_VAULT, "claim", [0n, A(0xa2), 5n, 12n, ["0x" + "11".repeat(32)]]), "claim for the winner (bound by the root)"],
    [req(A(0x16), RAFFLE_VAULT, "voidDraw", [0n]), "void a draw"],
  ] as const) assert.equal(await requestRefusal(s, r, ME, read, COIN), null, why);
  // docs/v7/dormant-collections.md §3.3: the WakeDesk (request only: a bonded wake of one of the stack's own wrappers, at the stack's bond)
  assert.deepEqual(Object.keys(CALLS).sort(), ["coin", "curve", "factory", "floorModule", "raffleVault", "router", "wakeDesk", "wrapper"]);
});

test("E-07, the first layer: the router, the wrapper and the raffle vault come from the stack file; an indexer naming another is refused, both addresses said", () => {
  assert.deepEqual(pinnedAddress(s, "frogs", "router", A(0x12)), { address: A(0x12), why: null });
  assert.deepEqual(pinnedAddress(s, "frogs", "router", null), { address: A(0x12), why: null }, "the indexer naming none is fine: the stack's is used");
  const bad = pinnedAddress(s, "frogs", "router", ATTACKER);
  assert.equal(bad.address, null); assert.match(bad.why!, new RegExp(`${ATTACKER}.*${A(0x12)}`, "i"));
  assert.match(pinnedAddress(s, "apes", "wrapper", A(0x10)).why!, /no apes collection/);
  assert.match(pinnedAddress(stackFrom([{ ...STACK, collections: [{ ...STACK.collections[0], raffleVault: undefined }] }]).stack, "frogs", "raffleVault", null).why!, /names no raffle vault/);
  for (const [f, role] of [["../src/ui/Ticket.tsx", "router"], ["../src/pages/Redeem.tsx", "wrapper"], ["../src/ui/Raffles.tsx", "raffleVault"]] as const) {
    const src = readFileSync(new URL(f, import.meta.url), "utf8");
    assert.match(src, new RegExp(`pinnedAddress\\(bundledStack\\.stack, [a-z.]+slug, "${role}"`), `${f} takes its ${role} from the stack file in Phase 1`);
  }
  assert.doesNotMatch(readFileSync(new URL("../src/ui/Ticket.tsx", import.meta.url), "utf8"), /const router = col\.contracts\.router;/, "the audit's line is gone");
  assert.match(readFileSync(new URL("../src/ui/wallet.tsx", import.meta.url), "utf8"), /requestRefusal\(bundledStack\.stack!, r, account \?\? null, readerOf\(env\.client as never\), coin\)/, "TxButton checks the whole request, with the connected account and the page's coin");
});
