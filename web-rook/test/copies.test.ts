import assert from "node:assert/strict";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { test } from "node:test";

/**
 * docs/v6/site-design.md: what web-rook takes from web/ is data and nothing else, copied byte for byte — every module under
 * src/lib, the api client and its types (which are the indexer's own), the wagmi setup, the deployments note, and the fixture
 * sets under public/fixtures (the indexer's own). This test catches drift whenever the repository copy is beside us; a build
 * context that holds web-rook/ alone (the Dockerfile) skips it.
 */
const here = (p: string) => new URL(p, import.meta.url);
const same = (ours: string, theirs: string) => assert.equal(readFileSync(here(ours), "utf8"), readFileSync(here(theirs), "utf8"), `${ours} has drifted from ${theirs}: re-copy it`);

test("the data layer is web/'s, byte for byte: every lib module, the api client and its types, the wagmi setup, the deployments note", () => {
  if (!existsSync(here("../../web/src/lib/"))) return;
  const libs = readdirSync(here("../src/lib/")).sort();
  assert.ok(libs.length >= 30, "the copied lib is all there");
  for (const f of libs) {
    assert.ok(existsSync(here(`../../web/src/lib/${f}`)), `src/lib/${f} is not a file of web/'s: the site's own code lives under src/rook, src/ui and src/pages`);
    same(`../src/lib/${f}`, `../../web/src/lib/${f}`);
  }
  for (const f of ["api/adapt.ts", "api/source.ts", "api/types.ts", "Web3.tsx", "deployments/README.md"]) same(`../src/${f}`, `../../web/src/${f}`);
  same("../src/api/types.ts", "../../indexer/types.ts");
  same("../src/rook/apiTypes.ts", "../../indexer/src/rook/types.ts");
  same("../src/rook/catalogTypes.ts", "../../indexer/src/rook/catalogTypes.ts");
});

test("the fixture sets are the indexer's own, byte for byte, file for file", () => {
  if (!existsSync(here("../../indexer/fixtures/"))) return;
  const sets = readdirSync(here("../../indexer/fixtures/")).sort();
  assert.deepEqual(readdirSync(here("../public/fixtures/")).sort(), sets, "the same sets, and no other");
  for (const set of sets) {
    const theirs = readdirSync(here(`../../indexer/fixtures/${set}/`)).sort();
    assert.deepEqual(readdirSync(here(`../public/fixtures/${set}/`)).sort(), theirs, `${set}: the same files`);
    for (const f of theirs) same(`../public/fixtures/${set}/${f}`, `../../indexer/fixtures/${set}/${f}`);
  }
});

// R2 N-3 / E-15 item 5 (docs/audit/2026-09-26/review2/fixes-R3-services.md §6): a Docker build ships the COMMITTED src/content, so
// a document edited under docs/ without `npm run sync` reaches the public site stale. What is still copied from docs/ (the trust
// model, for the Trust page) is the docs/ file byte for byte, and src/content's top level holds nothing else.
test("what src/content copies from docs/ is docs/'s, byte for byte (run `npm run sync` and commit)", () => {
  if (!existsSync(here("../../docs/"))) return;
  const src = readFileSync(here("../scripts/sync-content.mjs"), "utf8");
  const docs = [...src.matchAll(/\["([^"]+\.md)", "([a-z0-9-]+)"\]/g)].map((m) => [m[1], m[2]] as const);
  assert.ok(docs.length >= 1, "the sync script's DOCS list was read");
  for (const [from, name] of docs) same(`../src/content/${name}.md`, `../../docs/${from}`);
  const top = readdirSync(here("../src/content/")).filter((f) => f.endsWith(".md")).sort();
  assert.deepEqual(top, docs.map(([, n]) => `${n}.md`).sort(), "src/content's top level holds the synced documents and no other");
});

// The Docs page renders the user documentation written for the site (src/content/docs/NN-<slug>.md), not the engineering documents.
const PAGES = here("../src/content/docs/");
const pages = () => readdirSync(PAGES).filter((f) => f.endsWith(".md")).sort().map((f) => ({ f, text: readFileSync(new URL(f, PAGES), "utf8") }));

test("every docs page is NN-<slug>.md with a title, a one-line summary and a known group", () => {
  const list = pages();
  assert.ok(list.length >= 8, "the docs pages are there");
  const slugs = new Set<string>();
  for (const { f, text } of list) {
    assert.match(f, /^\d{2}-[a-z0-9-]+\.md$/, `${f}: name it NN-<slug>.md`);
    const m = /^---\n([\s\S]*?)\n---\n/.exec(text);
    assert.ok(m, `${f}: opens with a front matter block`);
    const meta = Object.fromEntries(m[1].split("\n").map((l) => [l.slice(0, l.indexOf(":")).trim(), l.slice(l.indexOf(":") + 1).trim()]));
    assert.ok(meta.title, `${f}: has a title`);
    assert.ok(meta.summary && meta.summary.length <= 140, `${f}: has a one-line summary`);
    assert.ok(["Start here", "Using Rook", "Trust"].includes(meta.group), `${f}: group is one of the nav's`);
    assert.ok(!/^#\s/m.test(text.slice(m[0].length)), `${f}: no h1 in the body (the page title is the front matter's)`);
    slugs.add(f.replace(/^\d{2}-/, "").replace(/\.md$/, ""));
  }
  for (const { f, text } of list) for (const [, slug] of text.matchAll(/\]\(#\/docs\/([a-z0-9-]+)/g)) assert.ok(slugs.has(slug), `${f}: links to a docs page that does not exist: ${slug}`);
});

// Canister-only Rook has no deploy fund and no fund wallet (docs/v9/canister-only-solana.md §16); the custodian's addresses are only
// known after its install, so the docs name no account at all (an address in the docs would be one to check by hand).
test("the docs pages are public-facing: no repository paths, the live domain, no account address", () => {
  for (const { f, text } of pages()) {
    assert.ok(!/(?<!#\/)(docs|ops|solana|autobuyer|src)\/[\w.-]+/.test(text) && !/\.(md|rs|mjs|ts|sh|json)\b/.test(text), `${f}: mentions a repository path or file`);
    assert.ok(!/(^|[^e])rook\.fun/.test(text), `${f}: the site is userook.fun`);
    assert.deepEqual([...text.matchAll(/\b[1-9A-HJ-NP-Za-km-z]{43,44}\b/g)].map((m) => m[0]), [], `${f}: names an account address`);
  }
});

// The site's code tells the canister-only story the docs pages tell (docs/v9/HANDOFF-2026-10-10.md §8, W12): no Solana programs of
// Rook's own, no community deploy fund, no SOL graduation tiers. These phrases belonged to the retired two-stage model.
test("no code or copy under src/ is on the old model: programs to deploy, a deploy fund, SOL graduation tiers", () => {
  const walk = (dir: URL): string[] => readdirSync(dir, { withFileTypes: true }).flatMap((e) => (e.isDirectory() ? walk(new URL(`${e.name}/`, dir)) : /\.(ts|tsx|md)$/.test(e.name) ? [new URL(e.name, dir).pathname] : []));
  const files = walk(here("../src/"));
  assert.ok(files.length > 100, "src/ was walked");
  const old = /programs are deployed|deploy fund|community deploy|Stage 2|graduation tier|85, 170, 255|rook_backing is|waiting-deploy|fund wallet|deploy deposit/i;
  for (const f of files) {
    const text = readFileSync(f, "utf8");
    const m = old.exec(text);
    assert.equal(m, null, `${f.slice(f.indexOf("/src/") + 1)}: "${m?.[0]}" is the old model`);
  }
});
