import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import type { Abi } from "viem";
import { requestRefusal, stackFrom, type StackFile, type StackReader } from "../src/rook/phase.js";
import { ERC20_APPROVE, ETCH_ROUTER, NFT_WRAPPER_REDEEM } from "../src/lib/abisV3.js";
import { FROG_CURVE_P1 } from "../src/rook/abis.js";

/**
 * Second review (R2) of E-07, kept as regressions of the round-3 fix (fixes-R3-services.md §8, N-2): the guard now binds a trade to
 * the coin the page (its URL, #/coin/<address>) is about, so the hostile indexer's swap is REFUSED; the page's own trades pass.
 */
const A = (n: number) => `0x${n.toString(16).padStart(40, "0")}` as `0x${string}`;
const STACK = { chainId: 4663, label: "stack", stage: 0, registry: A(1), owner: A(2), keeper: A(3), treasury: A(4),
  collections: [{ slug: "frogs", kind: "inscriptions", wrapper: A(0x10), factory: A(0x11), router: A(0x12), floorModule: A(0x13), raffleVault: A(0x16) }] };
const s = stackFrom([STACK]).stack as StackFile;
const PAGE_COIN = A(0xc0), PAGE_CURVE = A(0xcc), EVIL_COIN = A(0xe0), EVIL_CURVE = A(0xee), ME = A(0x3e);
// both coins were launched by the stack's factory (launching is permissionless and nearly free)
const curveOf: Record<string, string> = { [PAGE_COIN]: PAGE_CURVE, [EVIL_COIN]: EVIL_CURVE };
const tokenOf: Record<string, string> = { [PAGE_CURVE]: PAGE_COIN, [EVIL_CURVE]: EVIL_COIN };
const read: StackReader = {
  routerOf: async (f, coin) => (f.toLowerCase() === A(0x11) ? curveOf[coin.toLowerCase()] ?? A(0) : A(0)),
  tokenOf: async (a) => tokenOf[a.toLowerCase()] ?? null,
};
const req = (address: string, abi: readonly unknown[], functionName: string, args: readonly unknown[]) => ({ address, abi: abi as Abi, functionName, args });

test("R2 N-2 (fixed): a hostile indexer swapping the curve/coin a coin page trades is refused; the page's own coin passes", async () => {
  const dl = 2_000_000_000n;
  // the page is #/coin/<PAGE_COIN>; the indexer answers coin.curve = EVIL_CURVE (Ticket.tsx sends to coin.curve)
  assert.match(String(await requestRefusal(s, req(EVIL_CURVE, FROG_CURVE_P1, "buy", [1n, dl]), ME, read, PAGE_COIN)), /not the curve of the coin this page is about/);
  // graduated: coin.address comes from the indexer too; router.buy.coin must be the page's coin
  assert.match(String(await requestRefusal(s, req(A(0x12), ETCH_ROUTER, "buy", [EVIL_COIN, 1n, ME, dl]), ME, read, PAGE_COIN)), /not the coin this page is about/);
  assert.match(String(await requestRefusal(s, req(EVIL_COIN, ERC20_APPROVE, "approve", [A(0x12), 1n]), ME, read, PAGE_COIN)), /not the coin this page is about/);
  // no page coin at all: a trade is refused (only the coin page trades)
  assert.match(String(await requestRefusal(s, req(PAGE_CURVE, FROG_CURVE_P1, "buy", [1n, dl]), ME, read)), /not the curve of the coin this page is about \(none\)/);
  // the page's own curve and coin pass
  assert.equal(await requestRefusal(s, req(PAGE_CURVE, FROG_CURVE_P1, "buy", [1n, dl]), ME, read, PAGE_COIN), null);
  assert.equal(await requestRefusal(s, req(A(0x12), ETCH_ROUTER, "buy", [PAGE_COIN, 1n, ME, dl]), ME, read, PAGE_COIN), null);
  assert.equal(await requestRefusal(s, req(PAGE_COIN, ERC20_APPROVE, "approve", [A(0x12), 1n]), ME, read, PAGE_COIN), null);
  // the site passes the URL's coin: Coin.tsx gives Ticket the page's address and every TxButton in Ticket carries it
  const ticket = readFileSync(new URL("../src/ui/Ticket.tsx", import.meta.url), "utf8");
  assert.equal((ticket.match(/<TxButton /g) ?? []).length, (ticket.match(/<TxButton coin=\{pageCoin\} /g) ?? []).length, "every TxButton in the ticket is bound to the page's coin");
  assert.match(readFileSync(new URL("../src/pages/Coin.tsx", import.meta.url), "utf8"), /<Ticket [^>]*pageCoin=\{row\.address\}/);
});

test("R2 Info: an indexer that calls an inscriptions collection 'nft' turns Redeem into redeem(address) and the guard refuses it (DoS only)", async () => {
  const why = await requestRefusal(s, req(A(0x10), NFT_WRAPPER_REDEEM, "redeem", [ME]), ME, read);
  assert.match(String(why), /no rule for/);
});
