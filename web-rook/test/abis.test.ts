import assert from "node:assert/strict";
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { test } from "node:test";
import { BEACON, CURVE_FACTORY, FLOOR_MODULE_R3, FLOOR_MODULE_R4, FROG_CURVE_P1, ITEM_QUEUE_R3, LISTING_REGISTRY_ERRORS, LISTING_REGISTRY_R4, RAFFLE_VAULT, WAKE_DESK, WRAPPER_P1 } from "../src/rook/abis.js";
import { ERC20_APPROVE, ETCH_ROUTER } from "../src/lib/abisV3.js";

/**
 * docs/audit/2026-09-26/fixes-W2-site.md §0 and §1: the site names its contracts' functions by hand, from src/ — and this test holds
 * every one of them to what forge compiled (name, parameter types, return types, mutability), whenever `forge build` has left `out/`
 * beside the package. Without it (the Docker build context) it skips, like the copies test.
 */
type P = { readonly type: string; readonly components?: readonly unknown[] };
type Item = { readonly type: string; readonly name?: string; readonly stateMutability?: string; readonly inputs?: readonly (P & { indexed?: boolean })[]; readonly outputs?: readonly P[] };
const OUT = new URL("../../out/", import.meta.url);
const has = existsSync(new URL("FrogCurve.sol/FrogCurve.json", OUT));
const compiled = (c: string): Item[] => (JSON.parse(readFileSync(new URL(`${c}.sol/${c}.json`, OUT), "utf8")) as { abi: Item[] }).abi;
const t = (p: P): string => (p.type.startsWith("tuple") ? `(${(p.components as readonly P[]).map(t).join(",")})${p.type.slice(5)}` : p.type);
const sig = (x: Item) => `${x.name}(${(x.inputs ?? []).map(t).join(",")})->(${(x.outputs ?? []).map(t).join(",")}) ${x.stateMutability}`;
/** Events and errors too (R3): the name, the parameter types, and which are indexed. */
const evSig = (x: Item) => `${x.type} ${x.name}(${(x.inputs ?? []).map((p) => `${t(p)}${p.indexed ? " indexed" : ""}`).join(",")})`;
const same = (contract: string, ours: readonly unknown[]) => {
  const c = compiled(contract);
  const theirs = new Set(c.filter((x) => x.type === "function").map(sig));
  const theirEvErr = new Set(c.filter((x) => x.type === "event" || x.type === "error").map(evSig));
  for (const x of ours as Item[]) {
    if (x.type === "function") assert.ok(theirs.has(sig(x)), `${contract}: ${sig(x)} is not in the compiled ABI — re-sync src/rook/abis.ts`);
    else assert.ok(theirEvErr.has(evSig(x)), `${contract}: ${evSig(x)} is not in the compiled ABI — re-sync src/rook/abis.ts`);
  }
};

test("every function the site calls or reads is the compiled contract's: FrogCurve, CurveFactory, Wrapper, SweepVault, the beacon, EtchRouter, the coin", () => {
  if (!has) return;
  same("FrogCurve", FROG_CURVE_P1);
  same("CurveFactory", CURVE_FACTORY);
  same("Wrapper", WRAPPER_P1);
  same("SweepVault", RAFFLE_VAULT);
  same("DrandEvmnet", BEACON);
  same("EtchRouter", ETCH_ROUTER);
  same("CurveToken", ERC20_APPROVE);
  same("WakeDesk", WAKE_DESK);
});

test("R3: the round-3 entries are the committed contracts' (exitAtBacking, holdsRefunds, carriedSeed, seedOrigin, SeedCarried, undeliverable, LATE_DELIVERY_WINDOW, Undeliverable, NotUndeliverable, SameKey)", () => {
  if (!has) return;
  same("FloorModule", FLOOR_MODULE_R3);
  same("ItemQueue", ITEM_QUEUE_R3);
  same("ListingRegistry", LISTING_REGISTRY_ERRORS);
  const names = (xs: readonly unknown[]) => (xs as Item[]).map((x) => x.name);
  for (const n of ["undeliverable", "LATE_DELIVERY_WINDOW", "Undeliverable", "NotUndeliverable"]) assert.ok(names(WRAPPER_P1).includes(n), `Wrapper ${n}`);
  for (const n of ["carriedSeed", "seedOrigin", "SeedCarried"]) assert.ok(names(RAFFLE_VAULT).includes(n), `SweepVault ${n}`);
  const late = compiled("Wrapper").find((x) => x.name === "LATE_DELIVERY_WINDOW");
  assert.ok(late, "the Wrapper has the late-delivery window");
});

test("R4: the round-4 entries are the committed contracts' (burnsOpen, pauseFloor, RESUME_GRACE, keeperResumedAt, keeperResumedAtBlock)", () => {
  if (!has) return;
  same("FloorModule", FLOOR_MODULE_R4);
  same("ListingRegistry", LISTING_REGISTRY_R4);
});

test("C-02: the pinned round is final — the Wrapper and SweepVault have no redraw, and no page, ui or rook module calls one", () => {
  if (has) for (const c of ["Wrapper", "SweepVault"]) assert.ok(!compiled(c).some((x) => x.name === "redraw"), `${c} still has redraw`);
  for (const d of ["../src/pages/", "../src/ui/", "../src/rook/"]) for (const f of readdirSync(new URL(d, import.meta.url))) {
    if (!/\.tsx?$/.test(f)) continue;
    assert.doesNotMatch(readFileSync(new URL(d + f, import.meta.url), "utf8"), /functionName:\s*"redraw"|>Redraw</, `${d}${f} still offers a redraw`);
  }
});

test("A-07: every curve trade the ticket builds carries a deadline (the overloads), and the site's curve ABI names no deadline-less buy or sell", () => {
  const buys = (FROG_CURVE_P1 as readonly unknown[] as readonly Item[]).filter((x) => x.name === "buy" || x.name === "sell");
  assert.deepEqual(buys.map((x) => (x.inputs ?? []).length), [2, 3], "buy(minOut, deadline), sell(amountIn, minEth, deadline) only");
  const src = readFileSync(new URL("../src/ui/Ticket.tsx", import.meta.url), "utf8");
  assert.match(src, /functionName: "buy", args: \[quote!\.minOut, dl\]/);
  assert.match(src, /functionName: "sell", args: \[wei, quote!\.minOut, dl\]/);
  assert.doesNotMatch(src, /\bFROG_CURVE\b/, "the old ABI (no deadline) is not used by the ticket");
});
