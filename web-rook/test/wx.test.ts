import assert from "node:assert/strict";
import { existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { createServer } from "node:http";
import type { AddressInfo } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { inflateSync } from "node:zlib";
import { GROUND } from "../scripts/og-image.mjs";
import { IMG_BASE, NAME_BYTES, SIZE, SYMBOL_BYTES, URI_BYTES, WX_BASE, metadataText, parseOrder, registry, wxPixels } from "../scripts/wx-metadata.mjs";
import { COLLECTIONS, LAUNCH_COUNT, REGISTRY_ORDER } from "../src/rook/sol/collections";
import { createHandler } from "../server.mjs";

/**
 * The wrapped tokens' metadata (scripts/wx-metadata.mjs): every wX the launch pins can write (autobuyer/canister-rs/examples/
 * launch_pins.rs: the 38 of the default run and the 3 of --include-robinhood) points at `https://userook.fun/wx/<slug>.json`, so
 * each must be in public/wx/, valid, small, naming exactly what the pin puts on chain, with its picture beside it.
 */
const here = (p: string) => new URL(p, import.meta.url);
const WX = here("../public/wx/");
const read = (slug: string) => JSON.parse(readFileSync(new URL(`${slug}.json`, WX), "utf8"));
const utf8 = (s: string) => Buffer.byteLength(s, "utf8");
const repo = existsSync(here("../../autobuyer/canister-rs/src/coins/unlock.rs")) && existsSync(here("../../deployments/launch/robinhood-collections.json"));

test("every slug of the registry has its metadata file, valid JSON under 10 kB, and its picture; nothing else is in public/wx", () => {
  assert.equal(REGISTRY_ORDER.length, 41);
  for (const slug of REGISTRY_ORDER) {
    const file = new URL(`${slug}.json`, WX);
    assert.ok(existsSync(file), `public/wx/${slug}.json`);
    assert.ok(statSync(file).size < 10_000, `${slug}.json is under 10 kB`);
    const m = read(slug);
    assert.deepEqual(Object.keys(m), ["name", "symbol", "description", "image", "external_url", "properties"], slug);
    assert.equal(m.symbol, COLLECTIONS.find((c) => c.slug === slug)!.wxSymbol, `${slug}: the site's wX symbol`);
    assert.ok(m.name && utf8(m.name) <= NAME_BYTES && m.symbol && utf8(m.symbol) <= SYMBOL_BYTES, `${slug}: within the canister's limits`);
    assert.ok(utf8(`${WX_BASE}${slug}.json`) <= URI_BYTES, `${slug}: the default uri fits on chain`);
    assert.equal(m.image, `${IMG_BASE}${slug}.png`);
    assert.deepEqual(m.properties, { files: [{ uri: m.image, type: "image/png" }], category: "image" });
    assert.equal(m.external_url, `https://userook.fun/#/collection/${slug}`);
    assert.match(m.description, /^[^\n]+\.$/, "one sentence");
    assert.ok(existsSync(new URL(`img/${slug}.png`, WX)), `public/wx/img/${slug}.png`);
  }
  assert.deepEqual(readdirSync(WX).filter((f) => f !== "img").sort(), REGISTRY_ORDER.map((s) => `${s}.json`).sort());
  assert.deepEqual(readdirSync(new URL("img/", WX)).sort(), REGISTRY_ORDER.map((s) => `${s}.png`).sort());
});

test("the name and symbol are what launch_pins puts on chain: unlock.rs's ORDER, robinhood-collections.json's for the Robinhood Chain three", () => {
  if (!repo) return;
  const order = parseOrder(readFileSync(here("../../autobuyer/canister-rs/src/coins/unlock.rs"), "utf8"));
  assert.deepEqual(order.map((e) => e.slug), [...REGISTRY_ORDER]);
  assert.equal(order.filter((e) => e.source !== "Robinhood").length, LAUNCH_COUNT, "the default run's 38");
  const rh = JSON.parse(readFileSync(here("../../deployments/launch/robinhood-collections.json"), "utf8")).collections as { slug: string; name: string; symbol: string }[];
  for (const e of order) {
    const m = read(e.slug), r = rh.find((c) => c.slug === e.slug);
    const [name, symbol] = e.source === "Robinhood" ? [r!.name, r!.symbol] : [e.name, e.symbol];
    assert.equal(m.name, name, `${e.slug}: the pin's name`);
    assert.equal(m.symbol, symbol, `${e.slug}: the pin's symbol`);
  }
  // and every file is exactly what the script writes now
  for (const e of registry()) assert.equal(readFileSync(new URL(`${e.slug}.json`, WX), "utf8"), metadataText(e), `${e.slug}.json is stale: run node --import tsx scripts/wx-metadata.mjs`);
});

test("each picture is a 512 × 512 RGB PNG whose pixels are what the script draws for its symbol, the corners bare ground", () => {
  for (const slug of REGISTRY_ORDER) {
    const png = readFileSync(new URL(`img/${slug}.png`, WX));
    assert.deepEqual([...png.subarray(0, 8)], [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a], slug);
    assert.equal(png.readUInt32BE(16), SIZE); assert.equal(png.readUInt32BE(20), SIZE); assert.equal(png[24], 8); assert.equal(png[25], 2);
    const idat = png.indexOf("IDAT", 33, "latin1"), raw = inflateSync(png.subarray(idat + 4, idat + 4 + png.readUInt32BE(idat - 4)));
    const stride = SIZE * 3 + 1, rgb = Buffer.alloc(SIZE * SIZE * 3);
    for (let y = 0; y < SIZE; y++) { assert.equal(raw[y * stride], 0); raw.copy(rgb, y * SIZE * 3, y * stride + 1, (y + 1) * stride); }
    assert.ok(rgb.equals(wxPixels(read(slug).symbol)), `img/${slug}.png is stale: run node --import tsx scripts/wx-metadata.mjs`);
    // a wallet that crops the picture round loses nothing: everything outside the inscribed circle is ground
    for (let y = 0; y < SIZE; y += 4) for (let x = 0; x < SIZE; x += 4) {
      if (Math.hypot(x + 0.5 - SIZE / 2, y + 0.5 - SIZE / 2) <= SIZE / 2) continue;
      assert.deepEqual([...rgb.subarray((y * SIZE + x) * 3, (y * SIZE + x) * 3 + 3)], GROUND, `${slug}: (${x}, ${y})`);
    }
  }
});

test("the server serves /wx/<slug>.json as JSON and its picture as PNG, readable from any origin (no fallback to the app)", async () => {
  const root = mkdtempSync(join(tmpdir(), "wx-"));
  mkdirSync(join(root, "wx", "img"), { recursive: true });
  writeFileSync(join(root, "index.html"), "<!doctype html>");
  writeFileSync(join(root, "wx", "milady.json"), readFileSync(new URL("milady.json", WX)));
  writeFileSync(join(root, "wx", "img", "milady.png"), readFileSync(new URL("img/milady.png", WX)));
  const s = createServer(createHandler({ root, env: {} }));
  await new Promise<void>((r) => s.listen(0, "127.0.0.1", r));
  const base = `http://127.0.0.1:${(s.address() as AddressInfo).port}`;
  try {
    const j = await fetch(`${base}/wx/milady.json`);
    assert.equal(j.status, 200);
    assert.match(j.headers.get("content-type")!, /^application\/json/);
    assert.equal(j.headers.get("access-control-allow-origin"), "*");
    assert.equal((await j.json()).symbol, "wMILADY");
    const p = await fetch(`${base}/wx/img/milady.png`);
    assert.equal(p.headers.get("content-type"), "image/png");
    assert.equal(p.headers.get("access-control-allow-origin"), "*");
    assert.equal((await fetch(`${base}/wx/nope.json`)).status, 404, "a missing file is a 404, not index.html");
    assert.equal((await fetch(`${base}/`)).headers.get("access-control-allow-origin"), null, "only /wx/ is open");
  } finally { s.close(); }
});
