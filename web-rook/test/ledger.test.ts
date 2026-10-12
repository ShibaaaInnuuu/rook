import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import type { ReserveReport } from "../src/api/types.js";
import { wrappedFixtures } from "../src/lib/wrappedSource.js";
import { items, ledgerAgrees, ledgerOf } from "../src/rook/ledger.js";

/** docs/v6/site-design.md §6, "The ledger's equation": items + to be converted + reserve + float = supply, over the fixture's reports and hand-built cases. */
const here = (p: string) => new URL(p, import.meta.url);
const read = async (f: string) => JSON.parse(readFileSync(here(`../public/fixtures/wrapped/${f}`), "utf8"));
const index = JSON.parse(readFileSync(here("../public/fixtures/wrapped/index.json"), "utf8")) as { generatedAt: number };
const E = 10n ** 18n;

test("the fixture's green nest: (6 − 2) × 1 + (1.375 + 0.625 + 0.5) = 6.5, the three ETH rows from the module's own books", async () => {
  const src = await wrappedFixtures(index.generatedAt, read);
  const r = await src.get<ReserveReport>("/v1/reserves/frogs");
  const l = ledgerOf(r);
  assert.equal(l.status, "green"); assert.equal(l.holds, true);
  assert.equal(l.backing, 6_500n * E / 1_000n); assert.equal(l.supply, l.backing);
  assert.deepEqual(l.rows.map((x) => x.key), ["items", "tickets", "convertible", "reserve", "float", "sum", "supply"]);
  assert.equal(l.rows[0].units, 6n * E); assert.equal(l.rows[1].units, -2n * E);
  assert.equal(l.rows[2].units, 1_375n * E / 1_000n, "C − float"); assert.equal(l.rows[3].units, 625n * E / 1_000n, "R"); assert.equal(l.rows[4].units, E / 2n, "float");
  assert.ok(ledgerAgrees(l, r)); assert.ok(l.splitKnown);
  assert.ok(r.floorModule.kind === "read" && r.floorModule.fills.length === 2 && r.treasury.kind === "read" && r.protocolLiquidity.kind === "read", "the books the page draws below the equation are in the report");
});

test("the fixture's unverified nest never holds: the verdict is a question, not an equals sign, and without a module the ETH row is zero by construction", async () => {
  const src = await wrappedFixtures(index.generatedAt, read);
  const r = await src.get<ReserveReport>("/v1/reserves/apes");
  const l = ledgerOf(r);
  assert.equal(l.status, "unverified"); assert.equal(l.holds, null);
  assert.ok(l.reasons.length > 0, "the page lists why");
  assert.deepEqual(l.rows.map((x) => x.key), ["items", "tickets", "eth", "sum", "supply"]);
  assert.equal(l.rows[2].units, 0n); assert.match(l.rows[2].note!, /no floor module/);
  assert.ok(ledgerAgrees(l, r)); assert.equal(l.splitKnown, false);
});

test("the split never changes the total, and a ledger that disagrees with the report says so", async () => {
  const src = await wrappedFixtures(index.generatedAt, read);
  const frogs = await src.get<ReserveReport>("/v1/reserves/frogs");
  assert.ok(frogs.floorModule.kind === "read");
  const r: ReserveReport = { ...frogs, floorModule: { ...frogs.floorModule, ethBackedUnits: String(3n * E), reserveUnits: String((E * 3n) / 4n), convertibleUnits: String(3n * E - (E * 3n) / 4n), floatUnits: String(E / 2n) },
    equation: { ...frogs.equation, ethBackedUnits: String(3n * E), backingUnits: String(7n * E), backingUnitsIfUnverifiedHold: String(7n * E), supplyUnits: String(7n * E) } };
  const l = ledgerOf(r);
  assert.equal(l.rows.find((x) => x.key === "convertible")!.units, 3n * E - (E * 3n) / 4n - E / 2n);
  assert.equal(l.rows.find((x) => x.key === "reserve")!.units, (E * 3n) / 4n);
  assert.equal(l.rows.find((x) => x.key === "float")!.units, E / 2n);
  assert.equal(l.backing, 7n * E); assert.ok(ledgerAgrees(l, r));
  const off = { ...r, equation: { ...r.equation, ethBackedUnits: String(2n * E) } };
  assert.ok(!ledgerAgrees(ledgerOf(off), off), "the ETH rows no longer sum to the report's figure");
});

test("a red nest: an item elsewhere is under-backed and the report says so; a module that failed to read is named", async () => {
  const src = await wrappedFixtures(index.generatedAt, read);
  const frogs = await src.get<ReserveReport>("/v1/reserves/frogs");
  const red: ReserveReport = { ...frogs, status: "red", reasons: ["an item was found elsewhere"], equation: { ...frogs.equation, itemsInCustody: 5, itemsElsewhere: 1, backingUnits: String(5_500n * E / 1_000n), backingUnitsIfUnverifiedHold: String(5_500n * E / 1_000n), holds: false } };
  const l = ledgerOf(red);
  assert.equal(l.status, "red"); assert.equal(l.holds, false); assert.equal(l.backing, 5_500n * E / 1_000n); assert.ok(l.backing < l.supply!);
  assert.match(l.rows[0].note!, /1 elsewhere/);
  const err = ledgerOf({ ...frogs, floorModule: { kind: "error", address: "0x1000000000000000000000000000000000000009", error: "timeout" } });
  assert.deepEqual(err.rows.map((x) => x.key), ["items", "tickets", "eth", "sum", "supply"]);
  assert.match(err.rows.find((x) => x.key === "eth")!.note!, /could not be read: timeout/); assert.equal(err.splitKnown, false);
});

test("units print as whole items, never in scientific notation", () => {
  assert.equal(items(4n * E), "4"); assert.equal(items(E / 4n), "0.25"); assert.equal(items(-2n * E), "−2"); assert.equal(items(null), "?"); assert.equal(items(1_234n * E + E / 2n), "1,234.5");
});

test("docs/v6/relay-conversion.md §2.4: a unit in transit is its own ETH row, and the rows still sum to the report", async () => {
  const src = await wrappedFixtures(index.generatedAt, read);
  const r = structuredClone(await src.get<ReserveReport>("/v1/reserves/frogs"));
  assert.ok(r.floorModule.kind === "read");
  // move one convertible unit into transit: E is unchanged, C drops by one
  r.floorModule.convertibleUnits = String(BigInt(r.floorModule.convertibleUnits) - E);
  r.floorModule.transitUnits = String(E);
  const l = ledgerOf(r);
  assert.deepEqual(l.rows.map((x) => x.key), ["items", "tickets", "convertible", "transit", "reserve", "float", "sum", "supply"]);
  assert.equal(l.rows[3].units, E); assert.match(l.rows[3].label, /in transit/);
  assert.ok(ledgerAgrees(l, r), "E = C + R + T");
});
