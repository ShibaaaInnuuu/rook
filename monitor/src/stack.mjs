// What the monitor watches on 4663, from the stack file (deployments/stack-<chainid>.json, as script/DeployStack.s.sol writes
// it) given inline as `evm.stack`, or read from `evm.stack_file`, or from the older explicit `evm.wrappers` / `evm.floor_module`.
// Pure after the file is read.
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

const A = /^0x[0-9a-fA-F]{40}$/;
const ZERO = "0x0000000000000000000000000000000000000000";
const lc = (a) => String(a ?? "").toLowerCase();
const addr = (a) => (A.test(a ?? "") && lc(a) !== ZERO ? lc(a) : null);

/** The stack file's collections: an array, or the object keyed by slug that DeployStack's serialiser writes. */
const collectionsOf = (s) => (Array.isArray(s.collections) ? s.collections : Object.values(s.collections ?? {}));

export function loadStackFile(path, base = process.cwd()) {
  return JSON.parse(readFileSync(resolve(base, path), "utf8"));
}

/**
 * { registry, treasury, poolManager, fromBlock, collections: [{ slug, wrapper, module, raffle, burn, yoyo, factory, lock, router,
 *   desk }] }. With no stack: the explicit wrappers and module, as one collection.
 */
export function stackOf(evm) {
  const s = evm.stack;
  if (!s) {
    const wrappers = (evm.wrappers ?? []).map(lc);
    return { registry: addr(evm.registry), treasury: null, poolManager: null, fromBlock: evm.from_block ?? null,
      collections: wrappers.length || evm.floor_module
        ? [{ slug: "configured", wrapper: wrappers[0] ?? null, wrappers, module: addr(evm.floor_module), raffle: null, burn: null, yoyo: null, factory: null, lock: null, router: null, desk: null }]
        : [] };
  }
  return {
    registry: addr(s.registry), treasury: addr(s.treasury), poolManager: addr(s.poolManager), fromBlock: startBlock(evm.from_block, s),
    collections: collectionsOf(s).map((c) => ({ slug: c.slug, wrapper: addr(c.wrapper), wrappers: [addr(c.wrapper)].filter(Boolean), module: addr(c.floorModule),
      raffle: addr(c.raffleVault), burn: addr(c.burnVault), yoyo: addr(c.yoyoVault), factory: addr(c.factory), lock: addr(c.lock), router: addr(c.router),
      desk: addr(c.mintDesk),
      // docs/v7/phase1-nft.md §3.2: an Ethereum NFT collection's module takes its floor from the canister through its own PosterSource,
      // which is also that collection's own floor switch (audit 2026-09-27-nft D-M-1): the guardian pauses it, not the registry
      kind: c.kind ?? null, posterSource: addr(c.posterSource), nftCollection: addr(c.custody?.collection) })),
  };
}

/**
 * The first L2 block to read (R2-L1): deploy-stack.sh's `fromBlock` (4663's own height, what eth_getLogs understands), the
 * keeper's order; `deployedAtBlock` only for an older file (on 4663 it is the parent chain's block.number, far too low: slow, never
 * wrong). A configured `evm.from_block` may only start EARLIER (config.mjs refuses a later one: honest draws would recompute "bad").
 */
export function startBlock(configured, s) {
  const own = s.fromBlock ?? s.deployedAtBlock ?? null;
  if (configured === undefined || configured === null) return own;
  return own === null ? configured : Math.min(configured, Number(own));
}

/** Every address whose logs the EVM canary reads. */
export function watchedAddresses(st) {
  const out = new Set();
  if (st.registry) out.add(st.registry);
  for (const c of st.collections) for (const a of [...(c.wrappers ?? []), c.module, c.raffle, c.burn, c.yoyo]) if (a) out.add(a);
  return [...out];
}

/** Which collection, and which role in it, an address is. */
export function roleOf(st, a) {
  const x = lc(a);
  if (x === st.registry) return { role: "registry" };
  for (const c of st.collections) {
    if ((c.wrappers ?? []).includes(x)) return { role: "wrapper", c };
    if (x === c.module) return { role: "module", c };
    if (x === c.raffle) return { role: "raffle", c };
    if (x === c.burn) return { role: "burn", c };
    if (x === c.yoyo) return { role: "yoyo", c };
  }
  return { role: null };
}

/**
 * The addresses a draw's snapshot leaves out, by the keeper's rule (keeper/service/draws.ts `exclusionsFor`, docs/snapshot-v1.md
 * "Exclusions"): the zero and dead addresses, the coin, its curve, the collection's contracts, the pool manager, the treasury and
 * the registry, plus the listed ones (tools/snapshot/exclusions.json, copied into the config as `draws.exclusions`).
 */
export function exclusionsFor(st, c, coin, curve, listed = []) {
  return [...new Set([ZERO, "0x000000000000000000000000000000000000dead", coin, curve, c.factory, c.lock, c.router, st.poolManager, c.raffle, c.burn, c.yoyo,
    c.module, c.desk, st.treasury, st.registry, ...listed.map((x) => (typeof x === "string" ? x : x.address))].map(lc))].filter((a) => A.test(a));
}
