// Screenshots at 400 and 1440, dark and light, with console errors and horizontal-scroll checks. Needs `playwright` on the path
// (not a dependency: `npx -p playwright node scripts/screenshots.mjs http://127.0.0.1:8080 shots`).
import { chromium } from "playwright";
import { mkdirSync } from "node:fs";
const base = process.argv[2] ?? "http://127.0.0.1:8091";
const out = process.argv[3] ?? "shots";
mkdirSync(out, { recursive: true });
const PAGES = [
  ["home", "/?demo=sample#/"], ["home-empty", "/#/"], ["coins", "/?demo=sample#/coins"],
  ["coin-alien", "/?demo=sample#/coin/0x1000000000000000000000000000000000000a11"], ["coin-beta", "/?demo=sample#/coin/0x1000000000000000000000000000000000000b11"],
  ["coin-gamma", "/?demo=sample#/coin/0x1000000000000000000000000000000000000C11"],
  ["launch", "/?demo=sample#/launch"], ["launch-frogs", "/?demo=sample#/launch/frogs"], ["collections", "/?demo=sample#/collections"], ["collection-frogs", "/?demo=sample#/collection/frogs"],
  ["collection-apes", "/?demo=sample#/collection/apes"], ["collection-stonkbrokers", "/?demo=sample#/collection/stonkbrokers-434284142"], ["collection-nodemonkes", "/?demo=sample#/collection/nodemonkes"],
  ["collection-cryptopunks", "/?demo=sample#/collection/cryptopunks"], ["collection-mad-lads", "/?demo=sample#/collection/mad-lads"], ["collection-missing", "/?demo=sample#/collection/no-such-collection"],
  ["reserves", "/?demo=sample#/reserves"], ["reserves-frogs", "/?demo=sample#/reserves/frogs"], ["reserves-apes", "/?demo=sample#/reserves/apes"],
  ["redeem", "/?demo=sample#/redeem/frogs"], ["ticket-1", "/?demo=sample#/redeem/frogs/1"], ["bots", "/#/bots"], ["bots-frogs", "/?demo=sample#/bots/frogs"], ["bots-apes", "/?demo=sample#/bots/apes"],
  ["wallet", "/?demo=sample#/wallet/0x4000000000000000000000000000000000000001"], ["docs", "/#/docs"], ["trust", "/#/trust"], ["terms", "/#/terms"], ["brand", "/#/brand"],
];
const browser = await chromium.launch();
let problems = 0;
// the social card is served beside the pages: it must be a PNG
{
  const r = await fetch(`${base}/og.png`);
  const ok = r.ok && r.headers.get("content-type")?.startsWith("image/png");
  console.log(`${ok ? "ok" : "!!"} og.png ${r.status} ${r.headers.get("content-type")}`);
  if (!ok) problems++;
}
for (const theme of ["dark", "light"]) for (const width of [400, 1440]) {
  const ctx = await browser.newContext({ viewport: { width, height: width === 400 ? 860 : 900 }, colorScheme: theme, deviceScaleFactor: 1 });
  const page = await ctx.newPage();
  const errors = [];
  page.on("console", (m) => { if (m.type() === "error") errors.push(m.text()); });
  page.on("pageerror", (e) => errors.push(String(e)));
  for (const [name, path] of PAGES) {
    errors.length = 0;
    await page.goto(`${base}${path}`, { waitUntil: "networkidle" });
    await page.waitForTimeout(700);
    const scroll = await page.evaluate(() => ({ sw: document.documentElement.scrollWidth, iw: window.innerWidth, theme: getComputedStyle(document.body).backgroundColor }));
    const h = scroll.sw > scroll.iw;
    if (h || errors.length) { problems++; console.log(`!! ${theme}/${width} ${name}: ${h ? `HSCROLL ${scroll.sw}>${scroll.iw} ` : ""}${errors.join(" | ").slice(0, 300)}`); }
    else console.log(`ok ${theme}/${width} ${name} bg=${scroll.theme}`);
    await page.screenshot({ path: `${out}/${name}-${width}-${theme}.png`, fullPage: true });
  }
  await ctx.close();
}
await browser.close();
console.log(problems ? `${problems} problem(s)` : "all clean");
