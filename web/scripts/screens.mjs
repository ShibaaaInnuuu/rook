// Review screenshots of every page, three widths, both themes. Dev-only: run with
//   npm run build && PORT=8099 node server.mjs &   then   PW=<path to playwright/index.mjs> node scripts/screens.mjs [outDir] [only]
// Not part of the build or of CI; playwright is not a dependency of the site.
// playwright is not a dependency of the site: point PW at an installed copy, e.g. PW=/tmp/pw/node_modules/playwright/index.mjs
const { chromium } = await import(process.env.PW ?? "playwright");
import { mkdirSync } from "node:fs";
const out = process.argv[2] ?? "../docs/screens", only = process.argv[3];
const jpeg = process.env.FORMAT === "jpeg";      // the copies kept in docs/screens are JPEGs, to keep the repository small
const base = process.env.BASE ?? "http://127.0.0.1:8099";
const coin = "0x5707bF6c8d3D812F3fdfbA81482d346BbB63555C";      // the recorded demo's coin
const pages = [["landing", "/?#/"], ["landing-demo", "/?demo=sample#/"], ["docs", "/?#/docs"],
  ["explore", "/?demo=sample#/explore"], ["receipts", "/?demo=sample#/receipts"], ["vaults", "/?demo=sample#/vaults"],
  ["analytics", "/?demo=sample#/analytics"], ["claims", "/?demo=sample#/claims"],
  ["launch", "/?demo=sample#/launch"], ["coin", `/?demo=sample#/coin/${coin}`], ["listing", "/?demo=sample#/listing/0"],
  ["draws", "/?demo=sample#/draws"],
  ["states-explore", "/?demo=states#/explore"], ["states-receipts", "/?demo=states#/receipts"], ["states-vaults", "/?demo=states#/vaults"],
  ["states-analytics", "/?demo=states#/analytics"], ["states-draws", "/?demo=states#/draws"], ["states-listing-rune", "/?demo=states#/listing/1"],
  ["states-coin", "/?demo=states#/coin/0xC1C1c1c1C1C1C1c1c1C1C1C1c1C1C1C1c1C1c1c1"],
  ["trust", "/?#/trust"], ["not-found", "/?#/nowhere"], ["coin-nothing-live", `/?#/coin/${coin}`], ["explore-nothing-live", "/?#/explore"],
  // docs/v5/ux-and-reserves.md, docs/v5/floor-module.md §13, docs/v6/offer-handins.md: the wrapped-collection pages over the fixture set
  ["reserves", "/?demo=sample#/reserves"], ["reserves-frogs", "/?demo=sample#/reserves/frogs"], ["collection-frogs", "/?demo=sample#/collection/frogs"],
  ["wrapped-coin-gamma", "/?demo=sample#/coin/0x1000000000000000000000000000000000000c11"], ["wrapped-coin-beta", "/?demo=sample#/coin/0x1000000000000000000000000000000000000b11"],
  ["handins-frogs", "/?demo=sample#/handins/frogs"]];
const sizes = [["desktop", 1440, 900], ["phone", 400, 860]];
mkdirSync(out, { recursive: true });
const browser = await chromium.launch();
for (const theme of ["dark", "light"]) for (const [size, width, height] of sizes) {
  const ctx = await browser.newContext({ viewport: { width, height }, colorScheme: theme, deviceScaleFactor: 2, reducedMotion: "reduce" });
  const page = await ctx.newPage();
  for (const [name, path] of pages) {
    if (only && !name.startsWith(only)) continue;
    await page.goto(base + path, { waitUntil: "load" });
    await page.evaluate(() => document.fonts.ready);
    await page.waitForTimeout(1400);
    const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
    if (overflow > 1) console.log(`SIDEWAYS SCROLL ${overflow}px: ${name} ${size} ${theme}`);
    await page.screenshot(jpeg ? { path: `${out}/${name}.${size}.${theme}.jpg`, fullPage: true, type: "jpeg", quality: 78 } : { path: `${out}/${name}.${size}.${theme}.png`, fullPage: true });
  }
  await ctx.close();
}
// the social card: the slab and the sentence under it, 1200 x 630, dark
if (!only) {
  const ctx = await browser.newContext({ viewport: { width: 1200, height: 630 }, colorScheme: "dark", deviceScaleFactor: 1, reducedMotion: "reduce" });
  const page = await ctx.newPage();
  await page.goto(base + "/?#/", { waitUntil: "load" });
  await page.addStyleTag({ content: ".masthead,.banner,.skip,.hero__sub,.band,.section,.foot{display:none!important} main{padding:48px 0 0!important} .wrap-page{max-width:1104px!important;padding-inline:48px!important} .slab{max-height:none!important} .hero{gap:40px!important} .hero__say{grid-template-columns:1fr!important} .hero__say h1{font-size:48px!important;max-width:24ch!important}" });
  await page.evaluate(() => document.fonts.ready); await page.waitForTimeout(300);
  await page.screenshot({ path: process.env.CARD ?? "public/social-card.png", clip: { x: 0, y: 0, width: 1200, height: 630 } });
  await ctx.close();
}
await browser.close();
console.log("done");
