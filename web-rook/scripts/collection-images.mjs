// Every collection's picture, served by the site itself (public/collections/<slug>.<ext>), so the site hotlinks no marketplace CDN for
// a collection logo. Run from web-rook/ with network access:  node scripts/collection-images.mjs [slug ...]
//
// Each collection's official collection image, as its marketplace shows it, from the first source that answers with an accepted image:
//  - Ethereum: the OpenSea collection image recorded in the catalogue (i2c.seadn.io), else OpenSea itself (the API's `image_url` when
//    OPENSEA_API_KEY is set in the environment; the key is sent to OpenSea and written nowhere; else the collection page's own image,
//    i2c.seadn.io/<chain>/<collection id>/...);
//  - Robinhood Chain: OpenSea the same way, at robinhood-collections.json's `openseaSlug`;
//  - Solana: Magic Eden's public collection API (`image`), at the collection's Magic Eden symbol (collectionLinks.ts); IPFS pictures
//    are tried on further public gateways, and Okay Bears falls back to its collection NFT's on-chain image (Arweave);
//  - Ordinals: the icon recorded in the catalogue (Magic Eden's creator hub, Ordinals Wallet, the icon inscription), else Ordinals
//    Wallet's collection API (`icon`, or its `icon_inscription` on ordinals.com).
// Accepted: JPEG, PNG, GIF or WebP by magic bytes (the extension is the format's, whatever the URL says), at most 1 MB. A picture in
// another format (an SVG), over the cap, or whose host does not answer is taken through Magic Eden's image CDN (img-cdn.magiceden.dev, the resizer its pages use),
// 400×400 WebP, and the manifest records that URL as the source. Writes public/collections/manifest.json:
// { slug: { file, source_url, fetched_at, sha256 } }. Collections not fetched this run keep their manifest entry.
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, readdirSync, unlinkSync, writeFileSync } from "node:fs";

export const MAX_BYTES = 1024 * 1024;
const OUT = new URL("../public/collections/", import.meta.url);
const SEADN = "https://i2c.seadn.io";
const UA = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36";
const ACCEPT = "image/webp,image/png,image/jpeg,image/gif;q=0.9,*/*;q=0.5";
/** OpenSea's CDN negotiates the format and answers AVIF to a browser's Accept: asked for WebP alone, it sends WebP at the same size. */
const acceptFor = (url) => (/^https:\/\/i2c\.seadn\.io\//.test(url) ? "image/webp" : ACCEPT);

// sources, tried in order: { url } a recorded image, { opensea } an OpenSea slug, { me } a Magic Eden (Solana) symbol, { ow } an Ordinals Wallet slug
const os = (slug, url) => (url ? [{ url }, { opensea: slug }] : [{ opensea: slug }]);
const ord = (owSlug, url) => (url ? [{ url }, { ow: owSlug }] : [{ ow: owSlug }]);
export const SOURCES = {
  // Ethereum (nft-pilot.json `openseaSlug`)
  milady: os("milady", `${SEADN}/collection/milady/image/d4499e2e55f5e85ead4b0eb7f7dad7/90d4499e2e55f5e85ead4b0eb7f7dad7.jpeg`),
  azuki: os("azuki", `${SEADN}/collection/azuki/image/d62d4b9df96c961712b03f41c4231c/4dd62d4b9df96c961712b03f41c4231c.jpeg`),
  mayc: os("mutant-ape-yacht-club", `${SEADN}/collection/mutant-ape-yacht-club/image/33dcf5ce12bc4760bf912a5b3f5979/f033dcf5ce12bc4760bf912a5b3f5979.png`),
  pudgypenguins: os("pudgypenguins", `${SEADN}/collection/pudgypenguins/image/f489fb69fd11886b468c0f7ff1376c/cdf489fb69fd11886b468c0f7ff1376c.png`),
  bayc: os("boredapeyachtclub", `${SEADN}/collection/boredapeyachtclub/image/1a09c26b1c30427b26944c47fc7bb9/d81a09c26b1c30427b26944c47fc7bb9.png`),
  doodles: os("doodles-official", `${SEADN}/collection/doodles-official/image/63a85a2900fdd4bfe8f34a444b72d3/e663a85a2900fdd4bfe8f34a444b72d3.jpeg`),
  moonbirds: os("moonbirds", `${SEADN}/admin-uploads/8a589d8d6f4e5ffd1200a47be25783/678a589d8d6f4e5ffd1200a47be25783.png`),
  clonex: os("clonex", `${SEADN}/ethereum/75db99053ab444cfa0d8fb55d1f60547/2a98bfb35de339b49f3a4f0c4696c4/5c2a98bfb35de339b49f3a4f0c4696c4.png`),
  lilpudgys: os("lilpudgys", `${SEADN}/collection/lilpudgys/image/9289b91d3d0cefccfe6b9c7f83f471/649289b91d3d0cefccfe6b9c7f83f471.png`),
  checks: os("vv-checks-originals"),
  identitymd: os("identitymd", "https://raw2.seadn.io/ethereum/0x0000ec93127baa929e58e97dd0095a2bfb38ec1d/6c0ce84512a9ddb28fc350d794868b/4d6c0ce84512a9ddb28fc350d794868b.svg"),
  // Ordinals (Ordinals Wallet slugs: collectionLinks.ts / deployments/launch/btc/*.provenance.json)
  "bitcoin-frogs": ord("bitcoin-frogs", "https://ordinals.com/content/d19981a198e4a8419c8be18588d9e4919cea294deefdb5f9261a30242df578b7i0"),
  nodemonkes: ord("nodemonkes", "https://creator-hub-prod.s3.us-east-2.amazonaws.com/ord-nodemonkes_pfp_1698940091853.png"),
  omb: ord("omb", "https://ipfs.filebase.io/ipfs/bafybeidb7ltjitqivbcp22iktp5vmciee4dwzrjobwjnlzt5dkgyepj5ne"),
  "bitcoin-puppets": ord("bitcoin-puppets", "https://ord-mirror.magiceden.dev/content/170e13d878efecabbe2f3f31fedf28bc16c31d0ee4292c61829d43a067627ae5i0"),
  "bitcoin-punks": ord("bitcoinpunks", "https://ordinals.com/content/2d19554bb51891504424409677871f529a1decfa05748f1d6f64a76a36ebcb93i0"),
  natcats: ord("dmtnatcats", "https://creator-hub-prod.s3.us-east-2.amazonaws.com/ord-dmtnatcats_pfp_1708896003459.png"),
  runestone: ord("runestone", "https://media.ordinalswallet.com/693b679f6abf7d61f4e1dcffaa2571b6f1d0740cd080c9b532f54baed98bdc24.png"),
  "bitcoin-weirdos": ord("bitcoin-weirdos", "https://creator-hub-prod.s3.us-east-2.amazonaws.com/ord-bitcoin-weirdos_pfp_1748010545340.png"),
  cents: ord("cents"),
  rsic: ord("rsic", "https://creator-hub-prod.s3.us-east-2.amazonaws.com/ord-rsic_pfp_1705896366275.png"),
  "pizza-ninjas": ord("pizza-ninjas", "https://creator-hub-prod.s3.us-east-2.amazonaws.com/ord-pizza-ninjas_pfp_1706233719059.gif"),
  "quantum-cats": ord("quantum_cats", "https://creator-hub-prod.s3.us-east-2.amazonaws.com/ord-taproot_wizards_presents_pfp_1706542390359.png"),
  "wizards-of-ord": ord("wizards", "https://media.ordinalswallet.com/6b79892ee6859c2b6b9d7b9745bf0918b43550036e3bedda0f9b37940c26e8f0.png"),
  "taproot-wizards": ord("taproot-wizards", "https://ord-mirror.magiceden.dev/content/0301e0480b374b32851a9462db29dc19fe830a7f7d7a88b81612b9d42099c0aei0"),
  "bitcoin-wizards": ord("bitcoin-wizards", "https://ordinals.com/content/c7db021649a4ff4ba526ded71eb3d2bfc80f0ae56445e86766dcd66f8d97ae15i0"),
  // Solana (Magic Eden symbols, solana-collections.json `magic_eden_symbol`)
  mad_lads: [{ me: "mad_lads" }], smb_gen2: [{ me: "solana_monkey_business" }], claynosaurz: [{ me: "claynosaurz" }],
  famous_fox_federation: [{ me: "famous_fox_federation" }], degods: [{ me: "degods" }], galactic_geckos: [{ me: "galactic_geckos" }],
  taiyo_robotics: [{ me: "taiyo_robotics" }],
  // Magic Eden's picture for Okay Bears is on IPFS that no public gateway served on 2026-10-11; then the image of its collection NFT
  // (3saAedk…GvQ3: Metaplex metadata uri https://arweave.net/Us1XHZL6V-72DYA35Pp6kTFZhm-zf54JF_tgoYJ3UJQ, its `image`)
  okay_bears: [{ me: "okay_bears" }, { url: "https://arweave.net/7QhZL8C-lAWmCFbQ9saAh3ythEhBFhv0YCzKpwFRR6c" }], smb_gen3: [{ me: "smb_gen3" }],
  transdimensional_fox_federation: [{ me: "transdimensional_fox_federation" }], pythenians: [{ me: "pythenians" }], cyber_frogs: [{ me: "cyber_frogs" }],
  // Robinhood Chain (robinhood-collections.json `openseaSlug`)
  stonkbrokers: os("stonkbrokers-434284142"), chainmancers: os("chain-mancers"), rekttradooors: os("rekt-tradooor"),
};

/** The format by magic bytes, as the file's extension; null for anything else (SVG, AVIF, HTML, ...). */
export function formatOf(b) {
  if (b.length >= 3 && b[0] === 0xff && b[1] === 0xd8 && b[2] === 0xff) return "jpg";
  if (b.length >= 8 && b.subarray(0, 8).equals(Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]))) return "png";
  if (b.length >= 6 && (b.subarray(0, 6).toString("latin1") === "GIF87a" || b.subarray(0, 6).toString("latin1") === "GIF89a")) return "gif";
  if (b.length >= 12 && b.subarray(0, 4).toString("latin1") === "RIFF" && b.subarray(8, 12).toString("latin1") === "WEBP") return "webp";
  return null;
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
async function get(url, { accept = acceptFor(url), headers = {}, tries = 3 } = {}) {
  for (let i = 0; ; i++) {
    try {
      const r = await fetch(url, { headers: { "user-agent": UA, accept, ...headers }, redirect: "follow", signal: AbortSignal.timeout(45_000) });
      if ((r.status === 429 || r.status >= 500) && i + 1 < tries) { await sleep(5_000 * (i + 1)); continue; }
      if (!r.ok) throw new Error(`HTTP ${r.status}`);
      return Buffer.from(await r.arrayBuffer());
    } catch (e) {
      if (i + 1 >= tries) throw new Error(`${url}: ${e.message}`);
      await sleep(3_000 * (i + 1));
    }
  }
}
const json = async (url, opts) => JSON.parse((await get(url, { accept: "application/json", ...opts })).toString("utf8"));

/** An IPFS gateway URL (`<cid>.ipfs.<gateway>/<path>`, `<gateway>/ipfs/<cid>/<path>`), then the same content on Filebase's gateway:
 *  the public gateways the marketplaces record (nftstorage.link, w3s.link) throttle. */
function withIpfsMirror(url) {
  const m = /^https:\/\/([a-z0-9]{46,})\.ipfs\.[^/]+\/?(.*)$/i.exec(url) ?? /^https:\/\/[^/]+\/ipfs\/([a-z0-9]{46,})\/?(.*)$/i.exec(url);
  if (!m) return [url];
  const path = `${m[1]}${m[2] ? `/${m[2]}` : ""}`;
  return [...new Set([url, ...["https://ipfs.filebase.io/ipfs/", "https://ipfs.io/ipfs/", "https://dweb.link/ipfs/"].map((g) => g + path)])];
}

/** The picture's URLs a source names, in order of preference. */
async function candidates(src) {
  if (src.url) return [src.url];
  if (src.me) {
    for (const host of ["https://api-mainnet.magiceden.dev", "https://api-mainnet.magiceden.io"]) {
      try { const c = await json(`${host}/v2/collections/${src.me}`, { tries: 2 }); if (c.image) return [c.image]; } catch { /* the other host */ }
    }
    return [];
  }
  if (src.ow) {
    const c = await json(`https://turbo.ordinalswallet.com/collection/${src.ow}`);
    const urls = [];
    if (c.icon) urls.push(c.icon);
    if (c.icon_inscription?.id) urls.push(`https://ordinals.com/content/${c.icon_inscription.id}`);
    return urls;
  }
  if (src.opensea) {
    const key = process.env.OPENSEA_API_KEY;
    if (key) {
      try { const c = await json(`https://api.opensea.io/api/v2/collections/${src.opensea}`, { headers: { "x-api-key": key } }); if (c.image_url) return [c.image_url]; } catch { /* the page */ }
    }
    const html = (await get(`https://opensea.io/collection/${src.opensea}`, { accept: "text/html" })).toString("utf8").replace(/\\"/g, '"');
    const slug = src.opensea.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    // the page's own collection (the first `collectionBySlug` it carries is the slug it was asked for), else an object naming the slug
    const id = /"collectionBySlug":\{"__typename":"Collection","id":"([0-9a-f]{32})"/.exec(html)?.[1]
      ?? new RegExp(`"__typename":"Collection","id":"([0-9a-f]{32})"[^{}]*?"slug":"${slug}"`).exec(html)?.[1]
      ?? new RegExp(`"id":"([0-9a-f]{32})"[^{}]{0,400}?"slug":"${slug}"`).exec(html)?.[1];
    const count = (re, g = 0) => { const n = new Map(); for (const m of html.matchAll(re)) n.set(m[g], (n.get(m[g]) ?? 0) + 1); return [...n].sort((a, b) => b[1] - a[1]).map(([u]) => u); };
    // the collection object's own `imageUrl` (the first image field after its id), as the page shows it in the header
    const own = id ? count(new RegExp(`"id":"${id}"[\\s\\S]{0,800}?"(?:imageUrl|image)":"(https://[^"]+)"`, "g"), 1) : [];
    const byId = id ? count(new RegExp(`https://i2c\\.seadn\\.io/[a-z0-9_-]+/${id}/[0-9a-f]+/[0-9a-f]+\\.(?:png|jpe?g|gif|webp|svg)`, "g")) : [];
    const bySlug = count(new RegExp(`https://i2c\\.seadn\\.io/collection/${slug}/image/[0-9a-f]+/[0-9a-f]+\\.(?:png|jpe?g|gif|webp|svg)`, "g"));
    return [...new Set([...own.slice(0, 1), ...byId, ...bySlug])];
  }
  return [];
}

/** Magic Eden's image CDN, the resizer its own pages show collection pictures through: 400×400 WebP of any picture it can read. */
const resized = (url) => `https://img-cdn.magiceden.dev/rs:fill:400:400:0:0/plain/${url}@webp`;

async function fetchOne(slug) {
  const errors = [];
  for (const src of SOURCES[slug]) {
    let urls;
    try { urls = (await candidates(src)).flatMap(withIpfsMirror); } catch (e) { errors.push(e.message); continue; }
    if (!urls.length) errors.push(`${JSON.stringify(src)}: names no picture`);
    for (const url of urls) {
      // the picture itself; through the resizer when it is not one this accepts (its format, its size) or its host does not answer
      // (Magic Eden's CDN keeps its own copy of the pictures its pages show)
      for (const u of [url, resized(url)]) {
        try {
          const b = await get(u);
          const ext = formatOf(b);
          if (!ext) { errors.push(`${u}: not JPEG/PNG/GIF/WebP`); continue; }
          if (b.length > MAX_BYTES) { errors.push(`${u}: ${b.length} bytes, over ${MAX_BYTES}`); continue; }
          return { bytes: b, ext, source_url: u };
        } catch (e) { errors.push(e.message); }
      }
    }
  }
  throw new Error(`${slug}: no image\n  ${errors.join("\n  ")}`);
}

async function main() {
  const only = process.argv.slice(2);
  for (const s of only) if (!SOURCES[s]) throw new Error(`unknown slug ${s}`);
  mkdirSync(OUT, { recursive: true });
  const manifestUrl = new URL("manifest.json", OUT);
  const manifest = existsSync(manifestUrl) ? JSON.parse(readFileSync(manifestUrl, "utf8")) : {};
  const missing = [];
  for (const slug of only.length ? only : Object.keys(SOURCES)) {
    try {
      const { bytes, ext, source_url } = await fetchOne(slug);
      const file = `${slug}.${ext}`;
      for (const f of readdirSync(OUT)) if (f !== file && f.startsWith(`${slug}.`) && f !== "manifest.json") unlinkSync(new URL(f, OUT));
      writeFileSync(new URL(file, OUT), bytes);
      manifest[slug] = { file, source_url, fetched_at: new Date().toISOString(), sha256: createHash("sha256").update(bytes).digest("hex") };
      console.log(`${slug.padEnd(32)} ${file.padEnd(40)} ${String(bytes.length).padStart(8)}  ${source_url}`);
    } catch (e) { missing.push(slug); console.error(e.message); }
  }
  const sorted = Object.fromEntries(Object.keys(SOURCES).filter((s) => manifest[s]).map((s) => [s, manifest[s]]));
  writeFileSync(manifestUrl, JSON.stringify(sorted, null, 2) + "\n");
  if (missing.length) { console.error(`missing: ${missing.join(", ")}`); process.exitCode = 1; }
}

if (import.meta.url === `file://${process.argv[1]}`) await main();
