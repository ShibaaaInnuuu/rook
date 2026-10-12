import assert from "node:assert/strict";
import { test } from "node:test";
import { boardRefs, type ConfigRef } from "../src/rook/sol/board";
import type { CollectionViewV } from "../src/rook/sol/canister";
import { COLLECTIONS } from "../src/rook/sol/collections";

// Operator decision 2026-10-12 ("the people choose"): a collection opens on demand and the canister names its DBC config only then, so
// the board's configs are the site config's merged with coins_collections' Open rows (src/rook/sol/board.ts `boardRefs`).
const row = (index: number, symbol: string, stage: string, config = ""): CollectionViewV =>
  ({ index, symbol, stage, wx_mint: "", config, module: "", coins: 0, backed: 0 });
const A = "So11111111111111111111111111111111111111112", B = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA", C = "11111111111111111111111111111111";
const ROOK: ConfigRef = { config: C, slug: "rook", label: "Genesis (ROOK)" };

test("no canister, or a canister that has not answered: the static list as it is", () => {
  assert.deepEqual(boardRefs([ROOK], undefined, COLLECTIONS), [ROOK]);
  assert.deepEqual(boardRefs([ROOK], null, COLLECTIONS), [ROOK]);
  assert.deepEqual(boardRefs([], [], COLLECTIONS), []);
});

test("a collection opened on demand joins the board under its canister config, labelled by the site's collection of that wX symbol", () => {
  const refs = boardRefs([ROOK], [row(0, "wBOREDAPE", "Open", A), row(2, "wMADLAD", "Pinned"), row(3, "wSMBGEN2", "Requested"), row(4, "wBFROG", "Opening")], COLLECTIONS);
  const bayc = COLLECTIONS.find((c) => c.wxSymbol === "wBOREDAPE")!;
  assert.deepEqual(refs, [ROOK, { config: A, slug: bayc.slug, label: bayc.name }], "only Open rows are added, after the static ones");
});

test("a config the site config already names is not repeated; an Open row without a valid address adds nothing; an unknown symbol keeps its own name", () => {
  const stat: ConfigRef = { config: A, slug: "bayc", label: "Bored Ape Yacht Club" };
  const refs = boardRefs([stat, ROOK], [row(0, "wBOREDAPE", "Open", A), row(1, "wMILADY", "Open", ""), row(2, "wMILADY", "Open", "not-an-address"), row(9, "wNEWONE", "Open", B), row(10, "wTWICE", "Open", B)], COLLECTIONS);
  assert.deepEqual(refs, [stat, ROOK, { config: B, slug: "wnewone", label: "wNEWONE" }]);
});
