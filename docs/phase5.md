# Phase 5: the website

Written 2026-09-21, before its code. Branch `website`, worktree `../Etch-website`, `web/` only.

**Design read.** Reading this as: a protocol landing page and working app for crypto-native traders and the sceptics who check
them, with a language of permanence (cut stone under raking light), leaning toward native CSS tokens, one SVG lighting filter, and
restrained motion. Dials: variance 7, motion 5, density 4.

## The plan

**Subject.** To etch is to cut a line that stays. The product's promise is the same: a split that is a constant in bytecode, a
pairing that cannot be changed, a purchase declared in public before it happens. So nothing on the page floats. There are no drop
shadows and no cards hovering over a background: every rule, field, button and figure is cut INTO one surface, lit from the upper
left. That single rule is the identity; the rest stays quiet.

**Colour.** Cool mineral neutrals, one warm accent, locked.

| Token | Dark (basalt, default) | Light (limestone) | Role |
|---|---|---|---|
| `--stone` | `#171C1F` | `#E7EBE9` | the surface |
| `--stone-deep` | `#101416` | `#D5DBD8` | the floor of a cut: fields, the hero slab's grooves |
| `--seam` | `#2D363B` | `#BCC5C2` | the shadow side of a scored rule |
| `--glint` | `#3F4A50` | `#F8FAF9` | the lit side of a scored rule |
| `--chalk` | `#E4E9E7` | `#1A2023` | text |
| `--dust` | `#93A0A3` | `#56626A` | secondary text (AA on its surface in both themes) |
| `--ember` | `#EE8A2A` | `#B4570A` | Bitcoin's orange, a little desaturated; the only accent: the primary action, the live line, a countdown |

Semantic states are separate from the accent and used only for state: `--warn` `#D9A441` / `#8A5A00`, `--good` `#7FB49A` / `#2F6B4F`.
Not the cream-and-terracotta default, not acid-on-black, not purple. The dark theme is a blue-green slate, not tinted black.

**Type.** Three roles, all self-hosted (the CSP allows fonts from `'self'` only).
- Display: **Tenor Sans**. A humanist sans drawn on Roman inscriptional proportions: the letterforms a mason cuts, without being a
  serif and without being Trajan. One weight, used large and only for the wordmark, page titles and the big figures' labels.
- Text: **Hanken Grotesk** (variable). Calm, open, good numerals; `font-variant-numeric: tabular-nums` wherever digits line up.
- Mono: **IBM Plex Mono**, for addresses, hashes and commands only. Figures are set in the text face, not in mono.
Scale 1.25 from 1rem (16 / 20 / 25 / 31 / 39 / 49 / 61), display line-height 1.05, text 1.6, measure ≤ 68ch. Sentence case
everywhere. No tracked-out capital labels, no eyebrows.

**Layout.** A twelve-column grid with a mason's margin: on wide screens section titles sit in a narrow left margin column and the
content hangs to its right, so the left edge reads as one continuous scored line down the page. Sections are divided by a scored
rule (a dark hairline with a lit hairline beneath it), never by boxes. Radius 2px everywhere: chiselled, not rounded. Below 820px
the margin column folds above the content; below 480px everything is one column with 20px gutters.

```
| margin |  content                                   |
| etch   |  [ slab: the word etch, cut and lit      ] |
|        |  A coin whose fees buy Bitcoin assets ...  |
|        |  [Launch a coin]  what is trusted          |
|--------|--------------------------------------------|
| How it |  launch ───╮                               |
| works  |            ╰─── trade ───╮                 |
|        |                          ╰── buy ── distribute
|--------|--------------------------------------------|
| What   |  enforced by contracts | left to a word    |
| holds  |  (a ledger, row against row)               |
```

**The one bold thing.** The hero is a slab with the word `etch` cut into it: SVG text run through a lighting filter (a height map
from the glyphs, a distant light), over procedural stone grain. The light's azimuth follows the pointer a little and the scroll a
little, written straight to the filter attribute in a frame callback, never through React state. With reduced motion, or with no
pointer, it rests lit from the upper left and is complete as a still image. A static copy of the slab sits in `index.html` so the
first paint and a no-script visit show it too.

**Motion.** One load sequence (the light sweeps across the slab once, 900ms, then rests). "How it works" is a single engraved
line through four stations that draws with scroll where `animation-timeline: view()` exists and is simply drawn where it does not.
Controls answer touch by cutting deeper (the inset grows) rather than lifting. Nothing loops.

**Copy.** Ours, plain, sentence case. No price talk, no claim that anything moves a floor, no promises. No em dashes.

## Review of the plan against the defaults (what changed and why)

- First instinct for "engraved" was a Trajan-style serif in capitals on warm sandstone. That is the cream-and-serif default in a
  costume, and the brief rules the palette out. Changed to a lowercase inscriptional SANS on cool basalt.
- First instinct for structure was numbered cards for the four steps. They are a real sequence, but the brief asks for the line to
  BE the diagram, so there are no numerals and no cards: the stations are verbs on the line.
- First instinct for the ledger was green ticks and red crosses. Semantic colour would moralise a trade-off the brand wants stated
  flatly. It is two plain columns; the only emphasis is which column a sentence is in.
- Dropped: mono for small labels, middle-dot metadata strips, arrows on links, a stats hero. The hero's subject is the cut word.

## Demo data

The Phase 4 fixtures (`indexer/fixtures/demo/*.json`) are not on this branch's base, so the sample mode reads typed fixtures in
`web/src/demo/` shaped like the Phase 4 endpoints (`/v1/coins`, `/v1/listings`, `/v1/acquisitions`, `/v1/draws`, `/v1/stats`).
**At merge the parent swaps them for the real fixtures**; the adapter (`web/src/demo/adapter.ts`) is the only file that knows
the shape. Every screen that shows them carries a DEMO DATA banner.

## Results (2026-09-21)

Built as planned, in `web/` only. `npm run build` (content sync, typecheck, Vite) and `npm test` (6 tests) pass.

**Pages.** Landing (the slab; "How it works" as one engraved line through Launch, Trade, Buy, Distribute; the two-column ledger;
inscriptions against runes; the 80 / 20 bar drawn to scale; a live strip; listings and coins when there is something to read;
questions), and every Phase 2 page moved onto the same system without touching its chain reads: Launch, Coin (volume until the next
buy is the headline figure, with a carved progress groove under it), Listing, Draws (the challenge window's countdown is the largest
thing on its notice; "verify this snapshot yourself" sits under a scored rule), Trust (still the repository's document, rendered
word for word, which is why it alone contains dashes this site's own copy avoids). A 404 page. A light and dark toggle that is
remembered on the device; with no choice made, the system decides.

**The slab.** `web/src/components/Slab.tsx`. The glyphs are blurred into a height map and lit by `feDiffuseLighting` with a distant
light at 30 degrees, where a flat surface lights to exactly mid grey; laid over the grain with `hard-light`, only the walls of the
cut show. The grain is two `feTurbulence` layers (a fine speckle and a broad mottle), not a lighting pass: lit noise read as water.
The light's azimuth follows the pointer by up to 38 degrees, written to the filter attribute in a frame callback that stops when it
settles and whenever the slab is off screen. Reduced motion, touch, and no script all show the resting frame; `index.html` carries a
static copy for the first paint. Surface scale, blur and opacities were tuned by looking at screenshots in both themes.

**Demo data.** `?demo=sample` (linked from the footer and from every "nothing is deployed" notice) renders the pages from
`web/src/demo/fixtures.ts` through `web/src/demo/adapter.ts`, which maps them onto the types the chain reads produce, so the demo
exercises the real page code. A banner on every screen says the data is made up; every transaction button is disabled with the
reason. `?demo` alone still means the Phase 2 local fork. **At merge:** swap `fixtures.ts` for the Phase 4 fixtures
(`indexer/fixtures/demo/*.json`) by editing `adapter.ts` only; the landing strip reads `${VITE_ETCH_API}/v1/stats` and expects
`{ coins, listings, toVaultsWei, acquisitions, delivered }`, so check that against the real endpoint.

**Finish.** `public/favicon.svg`, `public/social-card.png` (1200 x 630, captured from the live hero), `public/theme.js` (the
remembered theme is applied before first paint from a file, because the server's CSP refuses inline scripts), copy-to-clipboard on
addresses, skeletons in the shape of a figure and two lines, empty states that say what will appear, focus rings, reduced motion
and reduced transparency honoured, tables and code scrolling inside their own boxes. Fonts are self-hosted (Tenor Sans, Hanken
Grotesk variable, IBM Plex Mono). Icons are Phosphor. No analytics, no third-party request.

**Review.** `web/scripts/screens.mjs` shoots every page at 1440, 820 and 400 wide in both themes and reports any sideways scroll.
Two rounds. Round one found: the grain reading as water (replaced), the dark carving reading as chrome (toned down), a code block
stretching the Draws notice past the viewport at every width (a grid column without `minmax(0, 1fr)`; fixed, and the script now
reports none), two snapshots sharing one cache key (the rune's notice showed the inscription's block), figures in the display face
whose crossed seven misread (figures moved to the text face, as the plan said). Round two: the word clipped by the slab at wide
viewports (smaller, recentred), a three-line headline (rewritten to two), the wordmark's mark reading as a dropdown caret (now a
scored underline with a notch). Final screenshots: `docs/screens/<page>.<desktop|phone>.<dark|light>.jpg` (downscaled JPEGs).

**Tests added.** `web/test/demo.test.ts`: the adapter's conversions and time anchoring, that fixtures are well formed and coins point
at listings that exist, that lots line up with acquisitions, that the coin page's headline number is computed from the fixtures and
not stored, and the Bitcoin address form's four cases.

**What I would still improve.** The slab's filter costs a few milliseconds a frame on a slow GPU while the pointer moves; it is
confined to the glyphs' box and stops at rest, but a canvas renderer would be cheaper. The "How it works" line is only drawn with
scroll where `animation-timeline` exists (Chromium today); elsewhere it is simply drawn. The Trust page inherits the document's own
long tables, which scroll sideways inside their box on a phone. The bundle is 610 kB (viem and wagmi); route-level splitting would
help first load. Nobody has clicked a wallet through the restyled Launch form.

## Rounds three and four (2026-09-21, after the coordinator's review)

The review was right: round two's hero was a soft grey banner, the type was even and polite, and the page below it was uniform.

**The hero, rebuilt.** SVG lighting filters cannot give a crisp cut: they light a blurred alpha at the filter's own resolution. The
slab is now a canvas at device pixel ratio (`web/src/components/Slab.tsx`). The glyphs are rasterised; an exact Euclidean distance
transform gives each inside pixel its distance to the letter's edge, which IS the depth of a V-cut; the gradient of that depth is a
constant direction per plane, so each plane is one flat tone and the arris is a sharp line where two meet. Each pixel is lit by one
distant light (Lambert, elevation 34 degrees), darkened a little with depth, and given a hairline of specular where the gradient
collapses. The stone is per-pixel grain over a faint drift and a fall-off away from the light: honed, not cloudy. The field is
computed once per size (about 150 ms at 2880 px wide); moving the light re-shades only the pixels inside the letters, and the loop
stops when the light settles or the slab leaves the screen. The pointer swings the azimuth by at most 26 degrees, so the bright and
dark planes trade places plausibly. Under the canvas the same word is set in CSS with two hairlines, which is also what
`index.html` paints first and what a visit without scripts sees. Composition: full bleed, the word large on the page's left edge,
the headline directly under it. Before and after: `docs/screens/hero-before.jpg`, `docs/screens/hero-after.dark.2x.jpg`,
`docs/screens/hero-after.light.2x.jpg`.

**Type.** One confident jump: the landing H1 runs to 5.4rem at 0.98 leading on two deliberate lines; section titles went from
1.3rem to 1.9rem in a wider margin column; body is unchanged. Tenor Sans has one weight and at this size it carries; figures stay
in Hanken Grotesk at light weights, larger, tabular, with units set at their own size and tracking. **The unit bug** ("ETHoftrading")
was the headline figure's negative tracking inherited by the unit; every unit now resets letter and word spacing.

**Below the fold.** A band of facts cut large (80/20, 1 h, 15 min, 0 withdraw functions); a scored vertical line runs down the
margin through every section with an ember notch at each title, so the engraved line travels the page; both listing illustrations
redrawn as tiles cut into their panel (96 holders with one lit; ten bars whose heights are the shares); the FAQ is larger with a
cut plus that turns; the footer carries the word cut at 17rem. Tap targets are at least 44 px. The ledger stacks as paired rows on
a phone.

**Weight.** The landing page's script went from 610 kB to 228 kB (73 kB gzipped). viem and wagmi are out of it: app pages are one
lazy chunk, chain reads are imported only when there is a deployment, and the wallet code is not fetched on the landing page until
somebody reaches for the button (the placeholder is the same size, so nothing moves). Fonts: latin subsets only, the two faces the
first screen uses are preloaded.

**Real data shapes (Phase 4).** Rebased onto main. `web/src/api/types.ts` and `web/public/fixtures/{demo,synthetic}` are
byte-identical copies from `indexer/` (source commit in `web/src/api/SOURCE.txt`; a test fails if they drift). `source.ts` reads
either the hosted indexer (`VITE_ETCH_API`, unset by default) or a fixture set through its `index.json`; `adapt.ts` maps the
indexer's shapes onto the types the pages already render. Three states everywhere: live figures (indexer when configured and
reporting a deployment, else the chain when a deployment file is bundled), a labelled demo (`?demo=sample` replays the recorded
run, `?demo=states` replays the synthetic set so every state is on screen, `?demo` alone is still the local fork), or plainly
"nothing is live yet". Unknowns stay unknown: a null `live` block prints its reason and no balances, `curvePending: null` prints
`curvePendingReason`, an unknown creator tax is said to be unknown. The stats strip reads the real `/v1/stats`
(`acquisitions.inscriptions + runeReceipts`, `deliveries.done`). The coin page shows the indexer's own volume-until-next-buy with its
reason when it has none. My placeholder fixtures are deleted.

**Review.** Shot at 2x, desktop and phone, both themes, no sideways scroll on any page. Found and fixed this round: carved numerals
invisible in dark and overflowing; the footer mark inheriting paragraph colour; streaks on curves from the binary distance field
(two passes of a small separable mean); a rolled-over distribution showing an open challenge countdown.

**Remaining weaknesses, honestly.** Faint banding is still visible on the tightest curves of the cut at 2x: a sub-pixel distance
transform would remove it. The light theme's lit plane has little headroom above pale limestone, so the cut reads mostly by its
shadow plane there. The recorded demo has one coin and small amounts, so the demo pages look sparse; the synthetic set uses toy wei
and shows zeros to four decimals. The Trust page is still the raw document. No human has moved a pointer over the slab or clicked
a wallet through Launch; both were only exercised headless. Lighthouse was not run.

## Polish pass on main (2026-09-21): the cut arrives at once, and lands where the fallback was

Measured in the page with `performance.now()` (`window.__etchSlab`), headless Chromium, 1204 px wide at DPR 2, served locally so
there is no network in these numbers; "x4" is Chrome's 4x CPU throttle, a stand-in for a slow laptop. Time is from navigation to
the carved canvas being on screen.

| | before | after |
|---|---|---|
| carved canvas ready, full speed | 487–494 ms | 216–224 ms |
| carved canvas ready, CPU x4 | 1,218–1,290 ms | 399–440 ms |
| of which the distance field and setup | not split out | 63 ms (149 ms at x4) |
| fade once ready | 500 ms | 220 ms |
| one lighting frame (196,440 lit pixels) | whole canvas written back | 2.7–4.2 ms, the letters' rectangle only |

On the live site the parent saw about three seconds; most of that was not this code (script download and parse before React
mounts), but the half-second fade and a 140 ms resize debounce on the FIRST build were ours and are gone.

What changed in `web/src/components/Slab.tsx`: the stone is drawn by the GPU (a flat tone, a 192 px grain tile as a pattern, one
gradient for the light's fall-off) instead of a per-pixel loop over 2.9 M pixels; the distance transform runs on the letters'
rectangle at reduced resolution (about half here) with sub-pixel edge distances from coverage (the anti-aliased transform used for
signed-distance fonts), smoothed twice, and is sampled bilinearly at full resolution, where only COVERAGE is rasterised so the edge
stays crisp; typed arrays are sized by a counting pass; the first build does not wait for the resize debounce, and does not await
the font when it is already loaded (it is preloaded). I did not precompute the field at build time: rasterising the face in Node
needs a native canvas dependency, and at 63 ms the runtime transform is no longer what a visitor waits for. If the live number is
still high, the next thing to cut is what runs before React mounts, not the slab.

**Registration.** The canvas no longer derives the baseline from line-box arithmetic. It reads the word's left edge and a zero-size
inline probe (`.slab__base`) that sits on the CSS word's baseline, with the same font size and tracking from computed style, so both
come from one metrics source. Checked by overlaying the CSS word in red under a half-transparent canvas at 1204 and 400 px:
`docs/screens/hero-registration-1204.2x.jpg`, `docs/screens/hero-registration-400.2x.jpg`. They coincide. **Full bleed:** the slab
measured 1204 of 1204 px and 400 of 400 px; the 1,000 px the parent measured was not reproduced here (the static first paint in
`index.html` sits inside the page wrapper for a moment before React mounts, which is one candidate).

**The pointer light, exercised.** Real pointer events dispatched in a headless run at 1440 px, DPR 2: about 82 lighting frames in
1.6 s per sweep (about 50 per second, headless, including the settle), each 2.7 to 4.2 ms, no field recompute.
`docs/screens/hero-light-left.2x.jpg` and `docs/screens/hero-light-right.2x.jpg` show the bright and shadowed planes trading sides.

**MetaMask's SDK.** `injected` is now imported from `@wagmi/core` (pinned, already in the tree) rather than the `wagmi/connectors`
barrel, so no connector of ours references `@metamask/sdk` and none of it is bundled. npm still prints its deprecation notice at
install, because the package is a dependency of `@wagmi/connectors`, which `wagmi` itself depends on; only dropping wagmi removes
that, and the notice is about a package we do not load.

Still visible at 2x on the tightest curves: faint ripples along the arris, from the medial axis wandering across the reduced grid.

## The name is etched (2026-09-21)

The domain is etched.meme, so the product is **etched**, lowercase, and the name and the address are one word. The carved word, the
nav wordmark, the footer mark, the title, the description, Open Graph and Twitter tags (with `og:url` and a canonical link to
https://etched.meme/), the social card, the 404 and every sentence that NAMED the product now say etched. "Etch" as a verb, and
"etching" as the runes term, are untouched. **Identifiers keep the `etch` prefix on purpose**: `VITE_ETCH_API`,
`ETCH_DEPLOY_CONFIRM` and its `deploy-etch:` confirm string, package names, `etch.css`, `etch-theme` in localStorage, the repository.
Renaming them buys nothing and the confirm string is a guard.

Six letters instead of four. The word is set at `min(80cqh, 25.5cqw)` (phones: `min(86cqh, 27cqw)`), which puts it on the grid's left
edge and ends it at the content column's right edge: 367 px type in a 1440 x 496 slab (ink from 144 to about 1310), 307 px at 1204,
209 px at 820, 108 px at 400 (20 to 378 of 400, inside both gutters). The "d" and "h" ascenders clear the slab's top by 85 px or
more at every width. Nothing is cached by word: the field is rebuilt from whatever the CSS word says, so registration stayed exact
without touching it (fresh overlays: `docs/screens/hero-registration-1204.2x.jpg`, `hero-registration-400.2x.jpg`). First carved
paint, same method as above: 103 ms at full speed, 435 ms at CPU x4 (the longer word is 218,861 lit pixels, up from 196,440; the
x4 figure is just over the 400 ms target, the full-speed one far under). A lighting frame is 4.2 to 4.4 ms.

Reads slightly awkwardly: "the etched contracts", and possessives ("etched's"), which the copy avoids by rephrasing.
