import assert from "node:assert/strict";
import { test } from "node:test";
import { giveawayExclusions, SELECTOR, addressWord, encodeUint } from "../src/giveaway.js";
import { snapshot } from "../src/core.js";

const A = (n: number) => `0x${n.toString(16).padStart(40, "0")}`;
const ONE = 10n ** 18n;
const w = (a: string) => `0x${a.replace(/^0x/, "").padStart(64, "0")}`;

const REGISTRY = A(0x1e), TREASURY = A(0x7ea), LAUNCHER = A(0x1a), DISTRIBUTOR = A(0xd15), ROUTER = A(0x201);
const VAULTS = [A(0xfa01), A(0xfa02), A(0xfa03)];
const TOKEN = A(0xe7c4ed);

/** A chain that answers only the reads this module is allowed to make, and shouts about any other. */
const chain = (to: string, data: string): Promise<string> => {
  const t = to.toLowerCase();
  if (t === REGISTRY) {
    if (data === SELECTOR.treasury) return Promise.resolve(w(TREASURY));
    if (data === SELECTOR.launcher) return Promise.resolve(w(LAUNCHER));
    if (data === SELECTOR.distributor) return Promise.resolve(w(DISTRIBUTOR));
    if (data === SELECTOR.count) return Promise.resolve(`0x${encodeUint(BigInt(VAULTS.length))}`);
    for (let i = 0; i < VAULTS.length; i++) if (data === `${SELECTOR.vaultOf}${encodeUint(BigInt(i))}`) return Promise.resolve(w(VAULTS[i]));
  }
  if (t === LAUNCHER && data === `${SELECTOR.routerOf}${TOKEN.replace(/^0x/, "").padStart(64, "0")}`) return Promise.resolve(w(ROUTER));
  return Promise.reject(new Error(`unexpected read ${to} ${data}`));
};

test("every etched-side exclusion is derived from the registry, deduplicated and sorted", async () => {
  const got = await giveawayExclusions(chain, REGISTRY, TOKEN);
  assert.deepEqual(got, [REGISTRY, TREASURY, LAUNCHER, DISTRIBUTOR, ROUTER, ...VAULTS].sort());
  assert.equal(new Set(got).size, got.length);
  // EVERY vault, not only the one being drawn: docs/token/protocol-token-v1.md §2.1
  for (const v of VAULTS) assert.ok(got.includes(v));
});

test("the address is the last 20 bytes of the word, and a registry that is not an address is refused", async () => {
  assert.equal(addressWord(w(TREASURY)), TREASURY);
  assert.equal(addressWord(`0x${"ff".repeat(12)}${TREASURY.slice(2)}`), TREASURY);
  await assert.rejects(giveawayExclusions(chain, "not-an-address", TOKEN), /ListingRegistry address/);
});

test("a registry with no listings yet, and a token the launcher does not know, still give a usable list", async () => {
  const empty = (to: string, data: string) =>
    data === SELECTOR.count ? Promise.resolve(`0x${encodeUint(0n)}`)
    : data.startsWith(SELECTOR.routerOf) ? Promise.resolve(w(A(0)))
    : chain(to, data);
  const got = await giveawayExclusions(empty, REGISTRY, TOKEN);
  assert.deepEqual(got, [REGISTRY, TREASURY, LAUNCHER, DISTRIBUTOR].sort());   // the zero address is never an exclusion here
  assert.ok(!got.includes(A(0)));
});

test("the derived list really removes the protocol's own balances from a snapshot", async () => {
  const exclusions = await giveawayExclusions(chain, REGISTRY, TOKEN);
  const balances = new Map([
    [A(0xa11ce), 60n * ONE], [A(0xb0b), 30n * ONE], [A(0xca401), 10n * ONE],
    [TREASURY, 5_000n * ONE], [DISTRIBUTOR, 50n * ONE], [VAULTS[0], 50n * ONE], [ROUTER, 7n * ONE],
  ]);
  const s = snapshot(balances, { coin: TOKEN, block: "100", kind: "weighted", decimals: 18, exclusions });
  assert.deepEqual(s.entries.map((e) => e.holder), [A(0xa11ce), A(0xb0b), A(0xca401)].sort());
  assert.equal(s.totalTickets, "100");                       // 60 + 30 + 10, and not one ticket for the 5,107 the protocol holds
  assert.ok(s.inputs.exclusions.includes(TREASURY));         // and the file says so, so a checker need not take our word
});

test("with every eligible holder excluded there is no list to publish at all", async () => {
  const exclusions = await giveawayExclusions(chain, REGISTRY, TOKEN);
  // docs/token/protocol-token-v1.md §2.1: no eligible holder, no root. The tool refuses rather than publishing an empty one,
  // and the contract refuses a draw with no tickets besides (test/Giveaway.t.sol). The asset simply waits for the next snapshot.
  assert.throws(() => snapshot(new Map([[TREASURY, 5_000n * ONE], [DISTRIBUTOR, 50n * ONE]]),
    { coin: TOKEN, block: "100", kind: "weighted", decimals: 18, exclusions }), /empty snapshot has no root/);
  // one eligible holder is enough, and takes every ticket
  const s = snapshot(new Map([[TREASURY, 5_000n * ONE], [A(0xa11ce), 3n * ONE]]),
    { coin: TOKEN, block: "100", kind: "weighted", decimals: 18, exclusions });
  assert.deepEqual(s.entries.map((e) => [e.holder, e.first, e.end]), [[A(0xa11ce), "0", "3"]]);
  assert.equal(s.totalTickets, "3");
});
