import assert from "node:assert/strict";
import { test } from "node:test";
import { readFileSync, writeFileSync, mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { generatePrivateKey, privateKeyToAccount } from "viem/accounts";
import { robustFloor, satsToWei, type Contents, type Listing, type Sale } from "../src/floor.js";
import { parseOwListings, parseOwSales, FixtureRate } from "../src/floorsources.js";
import { planPost, type Posted } from "../src/floorpost.js";
import { PROD_POST_POLICY } from "../service/plan.js";
import { AgreedRate } from "../service/feed.js";
import { build } from "../service/main.js";
import { runPass } from "../service/keeper.js";
import { FakeChain, stackJson } from "./service-fake.js";

/**
 * AUDIT E (services and workflows) proof-of-concept tests. They do not change any source; each one demonstrates a finding in
 * the audit report (E-services-workflow.md). Run: cd keeper && npx tsx --test test/audit_e.test.ts
 */

const L0 = JSON.parse(readFileSync(new URL("../fixtures/floor/ow-bitcoin-frogs-listings-2026-09-23.json", import.meta.url), "utf8"));
const S0 = JSON.parse(readFileSync(new URL("../fixtures/floor/ow-bitcoin-frogs-sales-2026-09-23.json", import.meta.url), "utf8"));
const AT: number = L0.fetchedAt;
const clean = (id: string): Contents => ({ inscriptions: [id], runes: [] });

/** The recorded Ordinals Wallet book, every listed outpoint taken as verified clean (the best case for the honest feed). */
function realBook(): { listings: Listing[]; sales: Sale[] } {
  const listings = parseOwListings(L0.listings, AT).map((l) => ({ ...l, contents: clean(l.id) }));
  const sales = parseOwSales(S0);
  return { listings, sales };
}
const addr = (tag: string, n: number) => `bc1p${tag}${String(n).padStart(4, "0")}${"q".repeat(50 - tag.length)}`;
const insc = (n: number) => `${(0xa0d17e00 + n).toString(16).padStart(64, "0")}i0`;

test("E-01 PoC: wash sales at just over L/4 between fresh wallet pairs pull the posted floor to 0.775 × the listing floor (−13% on the recorded book), with confidence still above every minimum", () => {
  const { listings, sales } = realBook();
  const honest = robustFloor({ listings, sales, snapshotAt: AT });
  assert.ok(honest.price !== null && honest.listingFloor !== null);
  const L = honest.listingFloor!;
  // The attacker owns ONE frog and moves it around 11 fresh, unlinked wallet pairs through the marketplace at ⌈L/4⌉+1 sats:
  // not S-self, not S-wash (no pair trades both ways, and ≥ L/4 is not "value moved at under a quarter of the floor"),
  // not S-outlier (price·4 ≥ L), each buyer and seller counted once (S-repeat). The attacker pays only fees: both sides are his.
  const wash = L / 4n + 1n;
  const fake: Sale[] = Array.from({ length: 11 }, (_, i) => ({ tx: `${(0xbeef00 + i).toString(16).padStart(64, "0")}`, at: AT - 60 * (i + 1), item: insc(i),
    seller: addr("s", i), buyer: addr("b", i), price: wash, itemsInTx: 1 }));
  const attacked = robustFloor({ listings, sales: [...fake, ...sales], snapshotAt: AT });
  assert.equal(attacked.listingFloor, L, "the listing side is untouched: nothing was bought or listed");
  assert.equal(attacked.saleFloor, wash, "the 11 newest sales are the attacker's, so the sales median is his price");
  const drop = Number(((honest.price! - attacked.price!) * 10_000n) / honest.price!);
  console.log(`E-01: listing floor ${L} sats; honest floor ${honest.price} (conf ${honest.confidenceBps}); after 11 wash sales at ${wash}: ${attacked.price} (conf ${attacked.confidenceBps}), −${drop / 100}%`);
  assert.ok(attacked.price! < honest.price!, "the floor moves down");
  assert.ok(attacked.price! <= (L * 7_750n) / 10_000n + 1n, "floor = 0.7·L + 0.3·(L/4) = 0.775·L");
  assert.ok(attacked.confidenceBps >= 3_000, "confidence stays over the keeper's and the module's 3,000 bps minimum: it is posted");
  const wei = satsToWei(attacked.price!, 3_700_000n);
  // The module accepts a 5 %/hour step; the keeper steps toward the target every pass. Five hours later the accepted floor is there.
  let posted: Posted = { priceWei: satsToWei(honest.price!, 3_700_000n), postedAt: AT - 3_600, ttl: 3_600, maxMoveBps: 500, bounds: null };
  let now = AT, hours = 0;
  while (posted.priceWei > wei && hours < 24) {
    const plan = planPost({ ...attacked, price: wei }, posted, now, PROD_POST_POLICY);
    assert.equal(plan.type, "post");
    posted = { ...posted, priceWei: BigInt((plan as { priceWei: string }).priceWei), postedAt: now };
    now += 3_600; hours++;
  }
  console.log(`E-01: the accepted floor reaches the manipulated target after ${hours} hourly steps`);
  assert.ok(hours <= 6);
});

test("E-01b PoC: the upward side is cheap on the recorded book: upCost10 is what it costs to lift the listing floor 10%", () => {
  const { listings, sales } = realBook();
  const f = robustFloor({ listings, sales, snapshotAt: AT });
  console.log(`E-01b: depth ${f.depthItems} valid listings within +10% of the floor; lifting the listing floor 10% costs ${f.upCost10} sats gross (items kept)`);
  assert.ok(f.upCost10 !== null);
});

test("E-09 PoC: the 'agreed' rate takes the LOWER sats/ETH, which gives the HIGHER floor in wei (the docs say the opposite)", async () => {
  const r = new AgreedRate([new FixtureRate(3_700_000n), new FixtureRate(3_665_000n)], 100n);
  const picked = await r.satsPerEth();
  assert.equal(picked, 3_665_000n, "the lower number of sats per ETH is picked");
  const sats = 480_000n;
  const atPicked = satsToWei(sats, picked), atOther = satsToWei(sats, 3_700_000n);
  console.log(`E-09: ${sats} sats → ${atPicked} wei at the picked rate vs ${atOther} wei at the other: +${Number(((atPicked - atOther) * 10_000n) / atOther) / 100}%`);
  assert.ok(atPicked > atOther, "p1-services.md §1.4: 'The lower is used, which gives a lower floor in wei' — it gives a higher one");
});

test("E-10 PoC: a transaction whose re-broadcast is refused (fee now under the base fee) stays 'pending' pass after pass; no replacement is ever signed and the module goes stale", async () => {
  const key = generatePrivateKey(), address = privateKeyToAccount(key).address;
  const chain = new FakeChain({ keeper: address, chainId: 9466301 });
  const dir = mkdtempSync(join(tmpdir(), "audit-e-"));
  writeFileSync(join(dir, "stack.json"), stackJson(9466301));
  writeFileSync(join(dir, "floors.json"), JSON.stringify({ collections: { frogs: { priceWei: "158000000000000000", confidenceBps: 8_000, depthItems: 5 } } }));
  const env = { KEEPER_CHAIN_ID: "9466301", KEEPER_STACK: join(dir, "stack.json"), KEEPER_STATE_DIR: join(dir, "state"), KEEPER_RPC_URLS: "http://127.0.0.1:1",
    KEEPER_FLOOR_SOURCE: "fixture", KEEPER_FLOOR_FIXTURE: join(dir, "floors.json"), KEEPER_MODE: "send", KEEPER_PRIVATE_KEY: key, KEEPER_RECEIPT_WAIT_S: "5" };
  const b = await build([], env, { fetchImpl: chain.fetch, write: () => {}, sleep: async () => { chain.advance(1); } });
  // 1. a first floor is posted and mined
  await runPass(b);
  assert.equal(chain.fresh(), true);
  // 2. the next post is broadcast ambiguously (the node never got it), then the base fee rises above the signed max fee
  chain.advance(3_000);
  chain.down = false; chain.mine = false;
  const origFetch = chain.fetch;
  let failSend = true;
  const flaky = (async (u: string, init: RequestInit) => {
    const m = JSON.parse(String(init.body)).method;
    if (m === "eth_sendRawTransaction" && failSend) { failSend = false; throw new TypeError("fetch failed (timeout)"); }
    return origFetch(u, init);
  }) as unknown as typeof fetch;
  const b2 = await build([], env, { fetchImpl: flaky, write: () => {}, sleep: async () => { chain.advance(1); } });
  const r = await runPass(b2);
  assert.equal(r.collections[0].floor.result?.outcome, "pending", "ambiguous broadcast: kept as pending");
  chain.baseFee = chain.baseFee * 3n;                          // a fee spike: the signed 2×base max fee is now under the base fee
  chain.rejectSend = "max fee per gas less than block base fee";
  let last;
  for (let h = 0; h < 3; h++) { chain.advance(3_600); last = await runPass(b2); }
  assert.equal(last!.pending?.outcome, "pending", "still pending: the same unmineable bytes, re-broadcast and refused, forever while the fee stays up");
  assert.equal(last!.collections[0].floor.result, null, "nothing else is sent: no fee bump, no replacement for that nonce");
  assert.equal(chain.fresh(), false, "the module has gone stale: mint, burn, sweeps and conversions stop");
  console.log(`E-10: after 3 hours the keeper still reports '${last!.pending?.detail}', and module.fresh() = ${chain.fresh()}`);
});

test("E-04 PoC: every keeper pass reads every coin the factory ever launched, one call at a time; launches are free, so the pass grows without bound", async () => {
  const key = generatePrivateKey(), address = privateKeyToAccount(key).address;
  const counts: number[] = [];
  for (const n of [0, 500, 2_000]) {
    const chain = new FakeChain({ keeper: address, chainId: 9466301 });
    for (let i = 0; i < n; i++) chain.coins.push({ coin: `0x${(0x300000 + i).toString(16).padStart(40, "0")}` as `0x${string}`, policy: 1, curve: `0x${(0x400000 + i).toString(16).padStart(40, "0")}` as `0x${string}` });
    const dir = mkdtempSync(join(tmpdir(), "audit-e4-"));
    writeFileSync(join(dir, "stack.json"), stackJson(9466301));
    writeFileSync(join(dir, "floors.json"), JSON.stringify({ collections: { frogs: { priceWei: "158000000000000000", confidenceBps: 8_000, depthItems: 5 } } }));
    const env = { KEEPER_CHAIN_ID: "9466301", KEEPER_STACK: join(dir, "stack.json"), KEEPER_STATE_DIR: join(dir, "state"), KEEPER_RPC_URLS: "http://127.0.0.1:1",
      KEEPER_FLOOR_SOURCE: "fixture", KEEPER_FLOOR_FIXTURE: join(dir, "floors.json") };
    const b = await build(["--dry-run"], env, { fetchImpl: chain.fetch, write: () => {}, sleep: async () => {} });
    chain.asked.length = 0;
    await runPass(b);
    counts.push(chain.asked.filter((m) => m === "eth_call").length);
  }
  console.log(`E-04: eth_call per pass with 0 / 500 / 2,000 launched coins: ${counts.join(" / ")} (sequential, every KEEPER_INTERVAL_S)`);
  assert.ok(counts[2] - counts[0] >= 2 * 2_000, "at least two sequential reads per launched coin, every pass, whatever its policy or state");
});

test("E-03 PoC (static): no Phase-1 service or page ever proves a drand round — Wrapper.resolve, SweepVault.requestDraw/resolve/claim are sent by nothing", async () => {
  const { readdirSync, statSync } = await import("node:fs");
  const root = new URL("../../", import.meta.url).pathname;
  const walk = (d: string): string[] => readdirSync(d).flatMap((f) => { const p = join(d, f); return f === "node_modules" || f === "fixtures" || f === "test" ? [] : statSync(p).isDirectory() ? walk(p) : /\.(ts|tsx|mjs)$/.test(f) ? [p] : []; });
  // what runs in Phase 1: the keeper service, the runner/offer/reconciler, the monitor, the indexer, the site
  const files = ["keeper/service", "ops/src", "monitor/src", "indexer/src", "web-rook/src"].flatMap((d) => walk(join(root, d)));
  const sends = /functionName:\s*"(resolve|requestDraw|claim|prove)"|"function (resolve|requestDraw|prove)\(uint/;
  const hits = files.filter((f) => sends.test(readFileSync(f, "utf8"))).map((f) => f.slice(root.length));
  console.log(`E-03: ${files.length} Phase-1 source files scanned; files that name resolve/requestDraw/claim/prove as a call or ABI: ${JSON.stringify(hits)}`);
  // web-rook/src/lib/abis.ts names the v1 Distributor's resolve (listingId, acquisitionId) for the old pons pages; nothing calls it
  for (const h of hits) assert.match(h, /web-rook\/src\/lib\/abis\.ts$/, `${h} is a sender of a drand proof`);
  const canister = readFileSync(join(root, "autobuyer/canister-rs/src/delivery.rs"), "utf8");
  assert.match(canister, /Stage::Pinned => return Decision::Wait \{ why: "not drawn yet: the ticket's round is still unproven" \}/, "the custodian only waits for someone else to prove the round");
  const wrapper = readFileSync(join(root, "src/Wrapper.sol"), "utf8");
  assert.match(wrapper, /function refund\(uint256 id\) external \{\s*Ticket storage t = _tickets\[id\];\s*if \(t\.stage != Stage\.Resolved\) revert WrongStage\(\);/, "a Pinned ticket has no refund path");
  assert.match(wrapper, /if \(id != nextToResolve\) revert OutOfOrder\(nextToResolve\);/, "and one unresolved ticket blocks every later one");
});

test("E-02 PoC: a post at the same price with confidence 0 (a stolen keeper key) makes the module stale, and the honest keeper does not re-post: planPost ignores the posted confidence", async () => {
  const key = generatePrivateKey(), address = privateKeyToAccount(key).address;
  const chain = new FakeChain({ keeper: address, chainId: 9466301 });
  const dir = mkdtempSync(join(tmpdir(), "audit-e2-"));
  writeFileSync(join(dir, "stack.json"), stackJson(9466301));
  writeFileSync(join(dir, "floors.json"), JSON.stringify({ collections: { frogs: { priceWei: "158000000000000000", confidenceBps: 8_000, depthItems: 5 } } }));
  const env = { KEEPER_CHAIN_ID: "9466301", KEEPER_STACK: join(dir, "stack.json"), KEEPER_STATE_DIR: join(dir, "state"), KEEPER_RPC_URLS: "http://127.0.0.1:1",
    KEEPER_FLOOR_SOURCE: "fixture", KEEPER_FLOOR_FIXTURE: join(dir, "floors.json"), KEEPER_MODE: "send", KEEPER_PRIVATE_KEY: key };
  const b = await build([], env, { fetchImpl: chain.fetch, write: () => {}, sleep: async () => { chain.advance(1); } });
  await runPass(b);
  assert.equal(chain.fresh(), true);
  // the thief (same key, so the module's immutable keeper) posts the same price with confidence 0, now and every ~25 minutes
  let staleMinutes = 0;
  for (let m = 0; m < 180; m += 5) {
    if (m % 25 === 0) { chain.confidence = 0; chain.postedAt = chain.headTs; }
    chain.advance(300);
    const r = await runPass(b);
    if (!chain.fresh()) staleMinutes += 5;
    if (m === 5) console.log(`E-02: honest keeper's plan after the thief's post: ${r.collections[0].floor.plan?.type} — ${r.collections[0].floor.plan?.why}`);
  }
  console.log(`E-02: module stale for ${staleMinutes} of 180 minutes while the honest keeper runs every pass`);
  assert.ok(staleMinutes >= 170, "mint and burn stay refused; the ETH-backed units cannot be burnt; the keeper address can never be changed");
});
