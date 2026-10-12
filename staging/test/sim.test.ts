import assert from "node:assert/strict";
import { test } from "node:test";
import { readFileSync, readdirSync } from "node:fs";
import { burstEndsAt, coinName, gapMs, LOGO_MAX, logo, MOODS, moodFor, nextLaunchDelayMs, nextStep, shouldBurst, shouldSnipe, taxFor, TAXES_BPS, type Coin } from "../sim/moods.js";
import { choiceFor, FIXTURE_TAPROOT, purchaseKey, spread, taprootFor, type Census, type LotCensus, type PurchaseCensus } from "../sim/spread.js";
import { openSim } from "../sim/wallets.js";

const coin = (over: Partial<Coin> = {}): Coin => ({ address: "0xc0", curve: "0xcu", router: "0xr0", listingId: 0, mood: "steady",
  phase: "curve", bornMs: 1_000, burstUntilMs: 0, bought: 0, sniped: true, ...over });

test("moods: a runner only ever buys, so a staging chain can actually reach a graduated coin; a fader mostly sells", () => {
  const runner = coin({ mood: "runner" });
  for (const roll of [0, 0.3, 0.7, 0.99]) assert.equal(nextStep(runner, 5_000, roll, 0.5).kind, "buy", `roll ${roll}`);
  assert.equal(nextStep(coin({ mood: "runner", phase: "graduated" }), 5_000, 0.99, 0.5).kind, "sell", "once graduated it is an ordinary coin again");

  const fading = coin({ mood: "fading" });
  assert.equal(nextStep(fading, 5_000, 0.1, 0.5).kind, "buy");
  assert.equal(nextStep(fading, 5_000, 0.5, 0.5).kind, "sell", "a fader sells where a steady coin would buy");
  assert.equal(nextStep(coin(), 5_000, 0.5, 0.5).kind, "buy");

  // a burst buys harder than the same coin at rest
  const bursting = coin({ mood: "momentum", burstUntilMs: 9_000 });
  assert.equal(nextStep(bursting, 5_000, 0.8, 0.5).kind, "buy");
  assert.equal(nextStep(coin({ mood: "momentum" }), 5_000, 0.8, 0.5).kind, "sell", "the same roll, once the burst is over");
  assert.ok(gapMs(bursting, 4_000, 5_000, 0.5) < gapMs(coin({ mood: "momentum" }), 4_000, 5_000, 0.5), "a burst trades faster");

  // sizes: mostly small, occasionally large, never zero or negative, and smaller once the coin is in a pool
  for (const s of [0, 0.4, 0.9, 1]) {
    const step = nextStep(coin(), 5_000, 0.1, s);
    assert.ok(step.size > 0 && step.size < 0.2, `size ${step.size}`);
  }
  assert.ok(nextStep(coin(), 5_000, 0.1, 0.95).size > nextStep(coin(), 5_000, 0.1, 0.1).size, "a large trade now and then");
  assert.ok(gapMs(coin(), 4_000, 5_000, 0.5) >= 500 && gapMs(coin(), 4_000, 5_000, NaN) >= 500, "a gap is never absurdly short, even on nonsense input");
});

test("moods: bursts need a rest first, a snipe happens once in a coin's first seconds, and launches start fast then settle", () => {
  assert.equal(shouldBurst(coin({ mood: "momentum", burstUntilMs: 0 }), 200_000, 0.05), true);
  assert.equal(shouldBurst(coin({ mood: "momentum", burstUntilMs: 199_000 }), 200_000, 0.05), false, "not again straight away");
  assert.equal(shouldBurst(coin({ mood: "steady", burstUntilMs: 0 }), 200_000, 0.05), false, "only a momentum coin bursts");
  assert.ok(burstEndsAt(1_000, 0.5) > 1_000 + 60_000);

  assert.equal(shouldSnipe(coin({ sniped: false, bornMs: 1_000 }), 5_000), true);
  assert.equal(shouldSnipe(coin({ sniped: false, bornMs: 1_000 }), 20_000), false, "the window is seconds wide");
  assert.equal(shouldSnipe(coin({ sniped: true, bornMs: 1_000 }), 5_000), false, "once only");

  assert.ok(nextLaunchDelayMs(0, 0.5) < 30_000, "the first minutes are busy: nobody should meet an empty grid");
  assert.ok(nextLaunchDelayMs(9, 0) >= 20 * 60_000 && nextLaunchDelayMs(9, 1) <= 30 * 60_000, "then one every 20-30 minutes");
});

test("identity: every mood and every creator tax is used, names say they are simulated, and a logo fits Pons's 512-byte cap", () => {
  assert.deepEqual([...new Set(MOODS)].sort(), ["fading", "momentum", "runner", "steady"], "all four moods are handed out");
  const used = new Set(Array.from({ length: 30 }, (_x, i) => moodFor(i)));
  assert.equal(used.size, 4);
  const taxes = new Set(Array.from({ length: 30 }, (_x, i) => taxFor(i)));
  assert.equal(taxes.size, TAXES_BPS.length);
  assert.ok(Math.min(...taxes) === 0 && Math.max(...taxes) === 1000, "0% through Pons's 10% cap");

  const names = new Set<string>(), symbols = new Set<string>();
  for (let i = 0; i < 30; i++) {
    const { name, symbol } = coinName(i);
    assert.match(name, /\(sim\)$/, name);
    assert.match(symbol, /SIM$/, symbol);
    names.add(name); symbols.add(symbol);
    const l = logo(i);
    assert.ok(l.length <= LOGO_MAX, `logo ${i} is ${l.length} bytes, over Pons's ${LOGO_MAX}`);
    assert.match(l, /^data:image\/svg\+xml/);
  }
  assert.equal(names.size, 30, "no two simulated coins share a name");
  assert.equal(symbols.size, 30, "nor a ticker");
});

const p = (id: number, status: PurchaseCensus["status"], over: Partial<PurchaseCensus> = {}): PurchaseCensus =>
  ({ vault: "0xv", listingId: 0, id, status, overdue: false, returned: false, ...over });
const lot = (acquisitionId: number, stage: number, over: Partial<LotCensus> = {}): LotCensus =>
  ({ listingId: 0, acquisitionId, mode: 3, stage, deliverable: false, ...over });

test("the spreader holds one item in each state that would otherwise empty, and never more than one", () => {
  const census: Census = {
    // three live releases: one is returned, one is aged toward a write-off, and one is left awaiting its receipt
    purchases: [p(0, "settled"), p(1, "declared"), p(2, "declared"), p(3, "released"), p(4, "released"), p(5, "released")],
    lots: [lot(0, 1), lot(1, 1), lot(2, 2), lot(3, 4, { deliverable: true }), lot(4, 5)],
  };
  const { holds, pushes } = spread(census);
  assert.deepEqual(holds.declarations, ["0xv:1"], "the oldest declaration waits it out; the other is let through");
  assert.deepEqual(holds.releases, ["0xv:4"], "one release awaits its receipt");
  assert.deepEqual(holds.challenges, ["0:0"], "one challenge window stays open");
  assert.deepEqual(holds.deliveries, ["0:3"], "one delivery is left to go overdue");
  // and the three live releases are used for three different things, never the same one twice
  const used = [holds.releases[0], ...pushes.map((x) => purchaseKey({ vault: x.vault, id: x.purchase }))];
  assert.equal(new Set(used).size, used.length, `one purchase cannot serve two states: ${used}`);
});

test("a purchase is identified by its vault as well as its number, because every vault counts from zero", () => {
  // the same number in two vaults: returning one must not also age the other's, and a hold must name only one of them
  const a = { vault: "0xAAA", listingId: 0 }, b = { vault: "0xBBB", listingId: 1 };
  const out = spread({
    purchases: [{ ...a, id: 0, status: "settled", overdue: false, returned: false }, { ...a, id: 1, status: "released", overdue: false, returned: false },
      { ...b, id: 1, status: "released", overdue: false, returned: false }, { ...b, id: 2, status: "released", overdue: false, returned: false }],
    lots: [],
  });
  const ret = out.pushes.find((x) => x.type === "return-unspent")!;
  const age = out.pushes.find((x) => x.type === "age-for-write-off");
  assert.ok(ret);
  if (age) assert.notEqual(purchaseKey({ vault: age.vault, id: age.purchase }), purchaseKey({ vault: ret.vault, id: ret.purchase }));
  for (const h of [...out.holds.declarations, ...out.holds.releases]) assert.match(h, /^0x[ab]{3}:\d+$/, `a hold must name its vault: ${h}`);
  assert.equal(purchaseKey({ vault: "0xAAA", id: 1 }) === purchaseKey({ vault: "0xBBB", id: 1 }), false, "two vaults' purchase 1 are different purchases");
});

test("the spreader never stalls a cold chain: nothing is held back until something has got past that state", () => {
  const cold = spread({ purchases: [p(0, "released")], lots: [lot(0, 1)] });
  assert.deepEqual(cold.holds.releases, [], "with nothing settled yet, the first release must be allowed to settle");
  assert.deepEqual(cold.holds.challenges, [], "with no draw past the window, the first one must be allowed through");
  assert.deepEqual(cold.holds.deliveries, []);
  assert.deepEqual(cold.holds.declarations, [], "and there is no declaration to hold");
});

test("the spreader asks for the states the operator's ordinary path never reaches, once each", () => {
  const both = spread({ purchases: [p(0, "settled"), p(1, "released"), p(2, "released", { overdue: true })], lots: [] });
  assert.deepEqual(both.pushes.map((x) => x.type).sort(), ["return-unspent", "write-off"]);
  const ret = both.pushes.find((x) => x.type === "return-unspent")!;
  assert.ok(ret.type === "return-unspent" && ret.share > 0 && ret.share < 1);
  assert.notEqual(ret.purchase, 2, "the overdue one is for the write-off, not the return");

  const done = spread({ purchases: [p(0, "settled"), p(1, "released", { returned: true }), p(2, "written-off")], lots: [] });
  assert.deepEqual(done.pushes, [], "once each state is occupied, nothing more is pushed");
});

test("a receipt deadline is three days, which staging never reaches, so one release is aged — but only when others can be spared", () => {
  // one live release is all there is: it is the one being returned, so nothing may be aged
  const thin = spread({ purchases: [p(0, "settled"), p(1, "released")], lots: [] });
  assert.ok(!thin.pushes.some((x) => x.type === "age-for-write-off"), "not while a single release is the only one there is");

  // two live releases: both are spent on the rare states, and "awaiting its receipt" goes empty for one pass on purpose —
  // a write-off stays on the page for ever, while another release arrives within minutes
  const two = spread({ purchases: [p(0, "settled"), p(1, "released"), p(2, "released")], lots: [] });
  const aged = two.pushes.find((x) => x.type === "age-for-write-off");
  assert.ok(aged, "two live releases are enough to reach for the state nothing else can produce");
  assert.deepEqual(two.holds.releases, [], "and nothing is left to hold that pass, deliberately");

  const enough = spread({ purchases: [p(0, "settled"), p(1, "released"), p(2, "released"), p(3, "released")], lots: [] });
  const age = enough.pushes.find((x) => x.type === "age-for-write-off");
  assert.ok(age, "with three live releases, one is aged so the state can exist at all");
  const ret = enough.pushes.find((x) => x.type === "return-unspent")!;
  assert.notEqual(age!.purchase, ret.purchase, "never the one being returned");
  assert.ok(!enough.holds.releases.includes(purchaseKey({ vault: age!.vault, id: age!.purchase })), "nor the one held awaiting its receipt");

  // once something IS overdue, it is written off rather than aging another
  const overdue = spread({ purchases: [p(0, "settled"), p(1, "released", { overdue: true }), p(2, "released"), p(3, "released"), p(4, "released")], lots: [] });
  assert.ok(overdue.pushes.some((x) => x.type === "write-off"));
  assert.ok(!overdue.pushes.some((x) => x.type === "age-for-write-off"));
});

test("choices cover every mode a creator can pick, and a rune listing can only be pro rata", () => {
  const modes = new Set(Array.from({ length: 12 }, (_x, i) => choiceFor(false, i)));
  assert.deepEqual([...modes].sort((a, b) => a - b), [1, 3, 4, 5], "hold, weighted draw, per-wallet lottery, named recipient");
  for (let i = 0; i < 6; i++) assert.equal(choiceFor(true, i), 6, "pro rata, and the contract refuses anything else for a rune");
});

test("a winner's address is a published test vector nobody holds, and it is one the CONTRACT accepts as taproot", () => {
  // Distributor.setDestination refuses anything BtcAddress.isTaproot refuses, and that library takes the 62-character bc1p
  // form and nothing else. A 40-byte version-1 program is valid bech32m and is NOT a taproot output: one used to be in this
  // list and every delivery handed it stalled at BadAddress(). test/StagingFixtureAddresses.t.sol pins these three strings
  // against the library itself; this pins the shape, so a new one cannot be added without meeting it. docs/phase11.md §7.
  for (const a of FIXTURE_TAPROOT) {
    assert.equal(a.length, 62, `${a} is not 62 characters, so the contract will refuse it`);
    assert.match(a, /^bc1p[023456789acdefghjklmnpqrstuvwxyz]{58}$/, a);
    assert.equal(a, a.toLowerCase(), "the library refuses upper case");
  }
  assert.equal(new Set(FIXTURE_TAPROOT).size, FIXTURE_TAPROOT.length, "and they are distinct");
  for (const n of [0, 1, 2, 7, -3]) assert.ok(FIXTURE_TAPROOT.includes(taprootFor(n)));
});

test("the simulator refuses any node but ours: a public address, and any chain id that is not the staging one", async () => {
  await assert.rejects(openSim("https://staging-chain-production.up.railway.app", 1), /only|never a public address/);
  const chainId = (id: number) => (async () => new Response(JSON.stringify({ jsonrpc: "2.0", id: 1, result: `0x${id.toString(16)}` }), { headers: { "content-type": "application/json" } })) as unknown as typeof fetch;
  const real = globalThis.fetch;
  try {
    for (const id of [4663, 46630, 1, 31337]) {
      globalThis.fetch = chainId(id);
      await assert.rejects(openSim("http://127.0.0.1:8546", 1), /refused/, `chain ${id} must be refused`);
    }
  } finally { globalThis.fetch = real; }
});

test("no key is ever logged or written: the simulator's sources hold no path from a private key to output", () => {
  const files = readdirSync(new URL("../sim/", import.meta.url)).filter((f) => f.endsWith(".ts"));
  assert.ok(files.length >= 4);
  for (const f of files) {
    const src = readFileSync(new URL(`../sim/${f}`, import.meta.url), "utf8");
    // the key never reaches a log line, a file, or a request body
    for (const bad of [/console\.[a-z]+\([^)]*privateKey/i, /privateKey[^;\n]*console/i, /writeFileSync\([^)]*privateKey/i,
      /JSON\.stringify\([^)]*privateKey/i, /log\([^)]*\.key\b/]) assert.ok(!bad.test(src), `${f} may leak a key: ${bad}`);
    // and it is never held anywhere but in the account object viem makes
    if (/generatePrivateKey/.test(src)) assert.match(src, /privateKeyToAccount\(generatePrivateKey\(\)\)/, `${f} must hand the key straight to viem and keep no copy`);
  }
});
