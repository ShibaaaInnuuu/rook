import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { test } from "node:test";
import { COLLECTIONS } from "../src/rook/sol/collections";

/** Every collection's picture is served by the site itself: public/collections/<slug>.<ext>, fetched by scripts/collection-images.mjs. */
const dir = new URL("../public/collections/", import.meta.url);
type Entry = { file: string; source_url: string; fetched_at: string; sha256: string };
const manifest: Record<string, Entry> = JSON.parse(readFileSync(new URL("manifest.json", dir), "utf8"));

/** The format by magic bytes, named as its extension. */
function formatOf(b: Buffer): string | null {
  if (b[0] === 0xff && b[1] === 0xd8 && b[2] === 0xff) return "jpg";
  if (b.subarray(0, 8).equals(Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]))) return "png";
  if (["GIF87a", "GIF89a"].includes(b.subarray(0, 6).toString("latin1"))) return "gif";
  if (b.subarray(0, 4).toString("latin1") === "RIFF" && b.subarray(8, 12).toString("latin1") === "WEBP") return "webp";
  return null;
}

test("every collection's logo is a file the site serves, under /collections/, named for its slug", () => {
  for (const c of COLLECTIONS) {
    assert.ok(c.logo, `${c.slug} has a logo`);
    assert.match(c.logo!, new RegExp(`^/collections/${c.slug}\\.(jpg|png|gif|webp)$`), `${c.slug}: a local path, no hotlink`);
    assert.ok(statSync(new URL(c.logo!.slice("/collections/".length), dir)).isFile(), `${c.logo} exists`);
  }
});

test("the manifest covers every collection and every file, with its source, its fetch time and the file's sha256", () => {
  assert.deepEqual(Object.keys(manifest).sort(), COLLECTIONS.map((c) => c.slug).sort());
  for (const c of COLLECTIONS) {
    const e = manifest[c.slug];
    assert.equal(`/collections/${e.file}`, c.logo, c.slug);
    assert.match(e.source_url, /^https:\/\//, `${c.slug}: source`);
    assert.ok(!Number.isNaN(Date.parse(e.fetched_at)), `${c.slug}: fetched_at`);
    assert.equal(createHash("sha256").update(readFileSync(new URL(e.file, dir))).digest("hex"), e.sha256, `${c.slug}: sha256`);
  }
  const files = readdirSync(dir).filter((f) => f !== "manifest.json").sort();
  assert.deepEqual(files, Object.values(manifest).map((e) => e.file).sort(), "no file the manifest does not name");
});

test("each file is the image format its extension says (by magic bytes), at most 1 MB, and the folder stays under 25 MB", () => {
  let total = 0;
  for (const e of Object.values(manifest)) {
    const b = readFileSync(new URL(e.file, dir));
    assert.equal(formatOf(b), e.file.split(".").pop(), `${e.file}: magic bytes`);
    assert.ok(b.length > 0 && b.length <= 1024 * 1024, `${e.file}: ${b.length} bytes`);
    total += b.length;
  }
  assert.ok(total < 25 * 1024 * 1024, `public/collections is ${total} bytes`);
});
