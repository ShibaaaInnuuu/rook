# Wave 2, the site and the indexer: what the contract fixes force, E-07, E-12, E-16, C-06, C-09

Branch `w2-site`, from `fixes-2026-09-26` (every contract fix of areas A, B, C and D is in it). Written before the code, as the
design; the "As built" section at the end is filled in when the code lands. Nothing here is deployed. No key, wallet, Railway
service or live chain is touched; the only chain reads are the public read-only RPC, and only if a test asks for one (none does).

Scope: `web-rook/`, `indexer/`, and the documents they render (`docs/trust-model.md`, synced into `web-rook/src/content/`). The
keeper, `tools/snapshot`, the canister (branch `w2-services`) and `monitor/`, `ops/`, `scripts/golive-check`, the runbooks
(branch `w2-monitor`) are other branches' and are not touched.

Inputs: `fixes-B-C-keeper-draws.md` §3 (the powers table) and §5 items 7, 8, 12–14; `fixes-A-graduation.md` ("Left open": the new
views, the deadline overloads, `QueueFull`); `C-treasury-roles-deploy.md` C-06, C-09; `E-services-workflow.md` E-07, E-12, E-16;
`docs/v6/phase1-scope.md` §0 and §8.3; `docs/v6/site-design.md`.

## 0. Two constraints that shape the diff

- **The copies.** `web-rook/src/lib/*`, `src/api/{adapt,source,types}.ts` and `src/Web3.tsx` are `web/`'s files byte for byte, and
  `src/api/types.ts` is `indexer/types.ts` (`test/copies.test.ts`). So the site's new contract calls live in its own
  `src/rook/abis.ts`, not in the copied `src/lib/abisV3.ts`, and the copied lib stays `web/`'s. The one shared file that must change
  is `indexer/types.ts` (new fields for draws, the keeper switch, the surplus, the curve's new views). It is re-copied to both
  `web-rook/src/api/types.ts` and `web/src/api/types.ts`; every new field is optional, so `web/`'s pages and every committed fixture
  set stay valid without regeneration.
- **"The repo's way" of syncing ABIs** is by signature, by hand, from `src/` (`abisV3.ts`, `indexer/src/wrapped/abis.ts` say so).
  There is no generator. What is added is the check that was missing: a test in each package that, when `forge build` has left
  `out/` beside it, compares every signature the package names with the compiled ABI (name, inputs, outputs, mutability; for
  events, the indexed flags) and fails on drift. Without `out/` (a Docker build context) it skips, like the copies test.

## 1. ABIs and call sites (fixes A-07, A-03, A-01 hatch; B/C: E-03, C-02, the draw flow)

`web-rook/src/rook/abis.ts` gains, checked against `out/`:

| Contract | Entries the site now uses |
| --- | --- |
| `FrogCurve` | `buy(minOut, deadline)`, `sell(amountIn, minEth, deadline)`, `refund(amount)`, `graduate()`, `openRefunds()`; views `state`, `canClose`, `canGraduate`, `refundsOpenAt` |
| `CurveFactory` | `launch` (unchanged), `launchesOpen()` |
| `Wrapper` | `redeem(string)`, `refund(id)`, `resolve(id, signature)`, `nextToResolve()`, `balanceOf`, `PIN_TIMEOUT`, `REFUND_GRACE`. **No `redraw`** |
| `SweepVault` (raffle) | `requestDraw(id)`, `resolve(id, signature)`, `claim(id, holder, first, end, proof)`, `voidDraw(id)`, `draw(id)` |
| drand beacon | `randomnessOf(round)` (a round already proven needs no signature) |

Call sites:

- **Redeem, ticket tracker.** Redraw is removed (the button, the "unproven for a day" line, the explanatory sentence on the burn
  page, the "a redraw can be asked for" stage line on the reserves page). A ticket shows:
  - **Resolve** (anyone) when it is pinned, its drand round exists, and it is next in order (`nextToResolve`, read on chain; tickets
    resolve strictly in order, so a later one says which ticket is next). The signature is fetched in the browser from drand's
    public relays (`api.drand.sh`, `api2.drand.sh`, `api3.drand.sh`, the evmnet chain hash), must be 64 bytes, and is not needed
    when the beacon already has the round. A bad relay can only make the transaction revert: the beacon verifies the signature.
  - **Refund** (anyone) for a **pinned** ticket from `pinnedAt + PIN_TIMEOUT` (7 days) and for a **resolved** ticket from
    `deliverBy + REFUND_GRACE` (72 h). Both are "both clocks" on chain; the page says "from about" and the time, since the
    parent-chain half can lag by minutes. Before that time the page says when, and offers nothing.
  - A ticket refunded while pinned (the indexer's sentinel, §3) reads "refunded unresolved: no item was drawn".
- **The ticket (curve trades).** Curve buys and sells use the deadline overloads, with the ticket's existing deadline setting
  (default 20 minutes; 10/20/60 offered) — the old "FrogCurve takes no deadline" sentence goes. The router already took one.
- **A-03, the close gate.** While a curve is Trading and `canClose()` is false: a quote whose route closes the curve (the quote
  already knows: `route.change`) is not offered as a buy; the ticket says **"The closing buy is unavailable: the floor module has
  no room for this coin's item budget right now. You can still sell back to the curve at its own price, and the close goes
  through when room appears."** Buys short of the close and every sell stay available.
- **A-03, the launch gate.** The launch wizard reads `launchesOpen()` (through the indexer). When false, a non-SWEEP_ONLY launch
  is disabled with **"Launches paused — the floor module is full (stage 0)."** and the reason in one line; SWEEP_ONLY launches
  are unaffected, as on chain.
- **A-01's escape hatch.** A Graduating coin shows `canGraduate()` (a **Graduate** button, anyone) and `refundsOpenAt()`: "if it is
  not graduated, refunds open from about <time>" — the deadline if unfilled, `window + 4 days` if filled or SWEEP_ONLY — and, past
  that time, an **Open refunds** button (anyone; `openRefunds()`). A Refunding coin keeps its Refund button.
- **Raffles.** A raffle coin's page gets a Raffles section: each draw of the coin with its stage (announced → open → pinned →
  resolved → claimed, or cancelled / voided), the seed round, the snapshot time, the root, the published list's link and whether it
  reproduces the root (§3), the challenge end, the pinned round, the winning ticket and winner. Keyless buttons, each shown only
  when the contract would accept it:
  - **Pin a round** (`requestDraw`) once the challenge has passed and the keeper is not paused;
  - **Resolve** (`resolve`, drand signature as above) once the pinned round exists;
  - **Claim for <holder>** (`claim`) once resolved, with the range and proof the indexer computed. Before sending, the page reads
    `draw(id)` on chain and checks the stage, that the range holds the winning ticket and that the proof verifies against the
    on-chain root; the contract checks the same again;
  - **Void** (`voidDraw`) for an announced draw 6 h old, a pinned one 7 days old, a resolved-unclaimed one 7 days old.

## 2. E-07: the send guard checks what a transaction says, not only where it goes

Today `targetRefusal` allows any stack contract or any coin/curve the stack's factory launched, and never reads the arguments;
the sell ticket approves `col.contracts.router`, which is the indexer's word.

**Fix, in two layers.**

1. **Pinned addresses.** In a Phase-1 build every contract the site asks a wallet to call or approve comes from the bundled stack
   file (`src/deployments/stack-4663.json`): the router (the approval's spender and the graduated trade's target), the wrapper
   (redeem, refund, resolve), the factory (launch, already so), the raffle vault (draws). Coins and curves still come from the
   indexer, and are verified on chain as now (`CurveFactory.routerOf`). If the indexer names a different router, wrapper or vault
   for the collection, the page says both addresses and sends nothing (as the launch wizard already does for the factory).
2. **`requestRefusal(stack, request, account, reader)`** replaces the target-only check in `TxButton`. It classifies the target
   (wrapper / factory / router / floor module / raffle vault / launched coin / launched curve; the desk, board and YOYO vault are
   still refused by name), allows only that kind's functions, and then checks **every address-typed argument** of the ABI entry
   being called (tuples and arrays included) against a rule; an address argument with no rule is refused (default deny, so a
   future call that carries an API address fails closed):

   | Target and call | Address argument | Rule |
   | --- | --- | --- |
   | launched coin `approve` | `spender` | one of the stack's routers |
   | router `buy` / `sell` | `coin` | a coin one of the stack's factories launched (`routerOf ≠ 0`, read on chain) |
   | router `buy` / `sell` | `to` | the connected account |
   | raffle vault `claim` | `holder` | any: it is bound by the Merkle root the contract verifies; the site also checks it (§1) |

   Allowed calls: wrapper `redeem`, `refund`, `resolve`; factory `launch`; router `buy`, `sell`; floor module none (the site sends
   nothing to it today); raffle vault `requestDraw`, `resolve`, `claim`, `voidDraw`; coin `approve`; curve `buy`, `sell`,
   `refund`, `graduate`, `openRefunds`. The raffle vault becomes a sendable target (it was not needed before: no draw call existed).
   `sendable()`'s list grows by it; the test that pinned the four kinds is updated for that reason.

**Tests.** The audit PoC becomes a regression (`web-rook/test/audit_e07.test.ts`): the same hostile answer (the real coin, a
router that is not the stack's) is refused, naming the spender; plus: `to` ≠ account refused, an unknown address argument refused,
a function not in the kind's list refused, a coin not launched by the factory refused, the honest requests allowed; and a source
check that `Ticket.tsx` and `Redeem.tsx` take the router and wrapper from the stack file in Phase 1.

## 3. The indexer

**Events the contract fixes added or changed.**

- `Wrapper.Refunded(id, acquisitionId, owner)` with `acquisitionId = type(uint256).max`: a ticket refunded while still pinned (E-03).
  The ledger marks it `refunded`, draws no item back into the set (none was drawn), and says so in the activity line. `Ticket`
  gains `refundableAt` (pinned: `burnedAt + 7 days`; resolved: `deliverBy + 72 h`) and `refundedUnresolved`. The v1 `Repinned`
  stays decoded for `NftWrapper` (OFF, not deployed, which still has `redraw`).
- **Draws** (`SweepVault`, raffle): `DrawAnnounced`, `SnapshotFixed`, `DrawOpened`, `DrawCancelled`, `DrawPinned`,
  `DrawResolved`, `Won`, `DrawVoided`, folded into `WrappedCollection.draws` (the latest 30), with the times the site needs
  (announced, snapshot, challenge end, pinned, resolved) and `voidableAt`.
  - **The published list.** For an opened draw the indexer fetches `uri` (through the E-12 guard, https or `ipfs://` only, 8 MB
    cap), rebuilds the tree exactly as `tools/snapshot` and `SweepVault.leaf` define it (the double-hashed `(holder, first, end)`
    leaves plus the **sentinel** `leaf(address(0), totalTickets, snapshotAt)`, sorted, OpenZeppelin's commutative pair hash), and
    reports `published: "matches" | "differs" | "unreadable"` against the on-chain root. It never trusts a proof from the file: once
    the draw is resolved it computes the winner's range and proof from its own tree, and only if the tree reproduces the root. A
    list that does not reproduce the root is shown as such (that is the C-03 check a watcher makes; the monitor pages on it on its
    own branch).
- **`KeeperPaused(by, paused)`** on the registry, plus `keeperPaused()` and `guardian()` read at the report's block:
  `WrappedCollection.keeper = { registry, paused, guardian, lastChange }`. The registry is the config's `registry`, else
  `FloorModule.registry()`.
- **The surplus and the stale fallback.** The floor-module read adds `surplus()` and `escaped()` (`FloorModuleRead`
  `surplusWei`, `escaped`); the reserves page shows the surplus beside the reserve and says when the stale fallback is on
  ("burns pay the backing, less the base spread, with no floor").
- **The curve's new views**, per coin: `canClose`, `canGraduate`, `refundsOpenAt`; and per collection the factory's
  `launchesOpen()`.

**E-12: the outbound fetch guard.** A new `indexer/src/outbound.ts`:

- `isPublicIp`: refuses loopback, unspecified, private (10/8, 172.16/12, 192.168/16), CGNAT 100.64/10, link-local 169.254/16 (so
  `169.254.169.254` metadata), 192.0.0/24, benchmark 198.18/15, documentation ranges, multicast and reserved (224/4, 240/4);
  IPv6 loopback, unspecified, ULA `fc00::/7` (Railway's `fd12::/16`), link-local `fe80::/10`, multicast, documentation, and
  IPv4-mapped / NAT64 / 6to4 forms of a refused v4 address.
- `guardedFetch` (production transport: `node:https` with a `lookup` hook): every hostname is resolved **at connect time**, every
  address it resolves to must be public, and the socket connects to the address that was checked (no second resolution, so DNS
  rebinding has no window). https only; the name rules of `isPublicHost` stay (no bare names, `.local`, `.internal`, IP literals
  are checked as IPs). **Redirects are not followed by the transport**: each hop's `Location` is resolved against the URL, checked
  again, and followed at most 3 times. The whole request has one deadline (15 s by default, 30 s for pictures). The body is
  streamed with a byte cap (2 MB for JSON, the picture cap for pictures, 8 MB for a draw list) and the request is aborted past it;
  a `content-length` over the cap is refused before reading.
- `getJson`, `metadataOf`, `fetchPicture` and the draw-list fetch go through it: `metadataOf` and `getJson` gain the host check they
  lacked; `r.json()` / `r.arrayBuffer()` on unbounded bodies are replaced by the capped read. Injected `fetch` implementations
  (tests, the fixture writer) are still honoured, wrapped by the same URL, redirect and size checks, with an injectable resolver.
- Fixed hosts (`wrapped/verify.ts`'s ord and esplora, the RPCs) keep plain `fetch` but also get the capped body read.

Tests: every refused range; a public name resolving to a private address; a redirect to a private host (and to `http:`);
a fourth redirect; a body over the cap with and without `content-length`; a slow answer past the deadline; `metadataOf` of an
`https://10.0.0.5.nip.io/…` URL; the draw list's tree (sentinel included) reproducing a root the contract's `leaf` produces; the
pinned-refund sentinel in the ledger; the draw fold through every stage.

## 4. The Trust page (C-06, C-09, fixes-B-C §3 and §5 item 13)

The Phase-1 statement (`src/rook/phase1Trust.ts`) keeps §0's bullets and changes or adds, in plain words:

- **Audit status**: "AI-reviewed on 26 September 2026 (five automated reviews; findings and fixes in the repository's
  `docs/audit/2026-09-26/README.md`), **not professionally audited**. G1 is still waived for stage 0." The old "no outside reviewer"
  wording and the test that forbade the word "audited" are adjusted so "not professionally audited" is required and a bare claim
  of being audited is still refused.
- **The Safe (2-of-2)**: at once, **it can take the whole ProtocolTreasury balance, allocated pots included** (C-06: register a
  module of its own, set the shares, the operations address and the loan, then allocate, distribute and bootstrap); pause any
  listing (sweeps and draws, not redemption, not burns); cancel any draw before its pin; set cap ceilings (the first at once,
  raises after 7 days); pause and unpause the keeper; name the guardian; transfer ownership. After 7 public days: install a new
  keeper. It cannot move the floor module's ETH, a sweep vault's ETH, a curve's ETH or the locked liquidity, record a delivery, or
  post a floor or a cap without first becoming the keeper.
- **The keeper key** (a hot key on a server): what it can do (post the floor within ±5 %/h and ±20 % per 24 h, post sweep caps
  under a ceiling that must exist and rise at most 20 % a post, announce and open draws) and **the worst case until it is paused**:
  mints at a walked-down floor worth at most about **1.3 ETH a day for frogs** (`maxPublicEthBacked × floor × 20 %`), sweeps at up
  to the ceiling, forged draw lists (caught by recomputing the root, which commits the count and the time; cancellable until the
  pin), a frozen floor module. What it can no longer do: drain the reserve surplus through burns, forge a delivery, choose the
  snapshot block, re-roll a draw, keep the module's ETH locked.
- **The guardian** (optional; an automated key on the monitor's host): pause the keeper and cancel a draw before its pin, **and
  nothing else**.
- **Rotation**: a new keeper takes 7 public days (both clocks); the pause is instant; the recovery is pause → propose → 7 days →
  accept → unpause.
- **The stale fallback**: if the floor goes 7 days (plus the hour) without a good post, **anyone can burn at the backing per unit,
  less the base spread, with no floor price**, and no key is needed; burns are never paused.
- **Raffles** rely on the keeper's published list plus a pre-pin cancel; anyone can check the list against the root.
- **Redemption refunds**: a ticket nobody resolves is refundable 7 days after its pin; an undelivered one 72 h after its deadline.

`docs/trust-model.md` (the v1-era table rendered under it): the "not trusted" line "the absence of a withdraw path" becomes "the
absence of a withdraw path from the vaults — **not** from the protocol treasury, which the Safe can empty at once (C-06)"; the
owner's "void a resolved draw" is scoped to the v1 Distributor with a note that a SweepVault draw is voided by anyone after 7
days; a header line points Phase-1 readers at the Phase-1 statement. Re-synced into `web-rook/src/content/` by `npm run sync`.

`test/phase1.test.ts` pins the new required sentences (the whole-treasury sentence, the 1.3 ETH/day worst case, "pause and cancel
only", the 7-day rotation, the stale fallback, "not professionally audited" and the review's path) and refuses "no withdraw path"
anywhere on the page.

## 5. E-16: web-rook's advisories

Measured after `npm ci` on this branch: 25 advisories, 2 high.

- **`ws` (high)**: the vulnerable copies are nested under `@reown/*` and `@walletconnect/utils` (8.18); the top-level `ws` is
  already 8.21.0. A non-breaking fix exists within 8.x: an npm `overrides` entry `"ws": "8.21.0"`. The connectors are installed,
  not bundled (`Web3.tsx` uses `injected()` only), so this is hygiene, not a shipped fix.
- **`vite` (high, dev and build only)**: every 5.x is in the advisory range (`<= 6.4.2`); the first fixed lines are 7.x/8.x, a
  major. `@vitejs/plugin-react` 4.7 supports vite 7. The update is taken **only if** the build, the tests and both builds
  (with and without `VITE_ROOK_PHASE=1`) are unchanged; otherwise it is left and recorded as open. `esbuild` (moderate, dev
  server) follows vite.
- **Exact versions** for what ships: `wagmi`, `viem`, `react`, `react-dom`, `@tanstack/react-query`, `marked`,
  `lightweight-charts` and the fonts are pinned to the versions the lockfile already resolves (no version moves; `@wagmi/core`
  was already exact). Dev tools keep ranges.
- The moderates (`@metamask/*`, `@walletconnect/*`, `@reown/*`, `uuid`, `query-string`, `decode-uri-component`) only have a
  fix through wagmi 3 (a major, with API changes). Left, with the recommendation of E-16 (move to `@wagmi/core` + viem without the
  connectors package) as an open item.

## 6. Runs this branch must pass

`web-rook`: `npm test`, `npm run typecheck`, `npm run build`, and `VITE_ROOK_PHASE=1 npm run build`. `indexer`: `npm test`,
`npm run typecheck`. The ABI-drift tests run with `out/` from `forge build` (lib copied in for the build, never committed).

## 7. Not done here

- The keeper-side resolver (§5 item 2 of the B/C doc), the monitor's root recomputation and the guardian's automation: other
  branches. The site's buttons make every keyless step possible by hand; they do not replace the resolver.
- `web/` (the older site) keeps its own `redraw` button and `abisV3.ts`: it is not the Rook site and is not deployed for Phase 1.
- `NftWrapper` still has `redraw` on chain (NFT is OFF and not deployed); the site offers no redraw for any collection.

## As built

Commits on `w2-site`: `868f3b1` (this design), `9577dfc` (the indexer and the shared types), `3964014` (the site, the Trust page,
the trust model), `4fdcca5` (E-16), and the commit that adds this section (docs and a test comment).

**As designed**, with these precisions:

- **ABIs.** `web-rook/src/rook/abis.ts` holds `FROG_CURVE_P1` (only the deadline overloads of `buy`/`sell`, so no request can pick
  the old ones), `WRAPPER_P1` (no `redraw`), `RAFFLE_VAULT`, `BEACON`, and `CURVE_FACTORY.launchesOpen`. `test/abis.test.ts` checks
  every entry, and the copied `ETCH_ROUTER` / `ERC20_APPROVE` the site still uses, against `out/` (name, parameter and return types
  with tuples expanded, mutability). The indexer's `test/w2.test.ts` does the same for the new events and reads, and its existing
  event-drift test caught the registry's two new events (`KeeperPaused`, `GuardianSet`), now declared and kept in the roles history.
- **Resolve** reads the round from the beacon first (`randomnessOf`); a round nobody proved yet takes drand's signature from
  `api.drand.sh`, `api2`, `api3` in turn (64 bytes, the right round, else the next relay). The Wrapper's order is read on chain
  (`nextToResolve`) and a ticket that is not next says which one is.
- **Claim** is offered only from the indexer's own tree, and the page reads `draw(id)` on chain and checks stage, range and proof
  before the wallet is asked (`claimRefusal`); `holder` is the one address argument whose rule is "any" (the root binds it).
- **E-07.** `requestRefusal` replaces `targetRefusal` in `TxButton` (which is kept, as the target half). `pinnedAddress` supplies
  the router (ticket), the wrapper (redeem, resolve, refund) and the raffle vault (draw steps) in a Phase-1 build; a differing
  indexer answer disables the button with both addresses. The raffle vault is now a sendable target; the burn vault is not.
- **The indexer** reads the new views with a fallback to null (a contract built before the fixes, or a mocked RPC, answers
  "not read" rather than a wrong value). A list verdict is cached per vault, draw, root and stage; `matches`/`differs` for good,
  `unreadable` retried after 10 minutes. `CollectionConfig.registry` is optional; without it the registry is `FloorModule.registry()`.
- **E-12.** `indexer/src/outbound.ts`: `isPublicIp`, `hostRefusal`, `urlRefusal`, `readCapped`/`jsonCapped`, `guardedFetch`, and
  `pinnedHttpsFetch` (node:https whose socket `lookup` refuses any non-public answer, so the address checked is the one connected
  to). The catalogue's default transport is `guardedFetch({ timeoutMs: 30 s })`; the draw lists use an 8 MB, 20 s guard;
  `getJson`, `metadataOf` (through `getJson`), `fetchPicture`, the tokenURI RPC read and `wrapped/verify.ts` read bodies under caps;
  `isPublicHost` now refuses CGNAT and IPv6 literals by address. An injected test `fetch` still gets the name rules and the caps.
- **E-16.** Both highs closed: `ws` 8.21.0 by an npm `overrides` entry (the nested 8.18 copies under `@reown/*` and
  `@walletconnect/utils`), and vite 5.4.21 → **7.3.6** (`@vitejs/plugin-react` 4.7.0 already supports it; tests, typecheck, both
  builds, the og card, robots, sitemap and the Phase-1 security.txt warning unchanged; the Docker image's node 22 meets vite 7's
  engine range). `npm audit`: 25 (2 high) → **21 moderate, 0 high**. The shipped dependencies are pinned to the versions the lockfile
  already resolved (no version moved). The 21 moderates are all under wagmi's connectors and need wagmi 3 (a major): left.
- **Trust page.** Sections now: caps, curves, audit ("AI-reviewed, not professionally audited", with the review's repository path),
  safe (the whole-treasury sentence first), keeper (powers, the ≈1.3 ETH/day worst case, what it can no longer do), guardian
  ("pause and cancel only"), rotation (instant pause, 7-day rotation), stale (holders can always burn at backing after 7 days),
  keyless (raffles and redemption refunds), redemption. The addresses list gains the registry, the keeper at deploy, the router,
  the floor module and the raffle vault. `docs/trust-model.md` scopes "no withdraw path" to the vaults and names the treasury
  exception, scopes the owner's void to the v1 Distributor, and points Phase-1 readers above it; `phase1.test.ts` refuses
  "no withdraw path" and the old "income and unallocated balance" understatement anywhere on the Phase-1 statement.
- `npm run sync` also brought in `floor-module.md` and `custody-hardening.md` as the fix branches left them (the site's copies were
  stale); `docs/v6/site-design.md` gained a wave-2 paragraph and the Redeem row lost "redraw".

**Runs** (on the final tree):

| Run | Result |
| --- | --- |
| `indexer`: `npm test` | **111 pass, 0 fail** (99 before: 12 new in `test/w2.test.ts`; `fold.test.ts` counts the two new registry events) |
| `indexer`: `npm run typecheck` | clean |
| `web-rook`: `npm test` | **57 pass, 0 fail** (43 before: 14 new — `abis.test.ts` 3, `audit_e07.test.ts` 5, `keyless.test.ts` 5, `phase1.test.ts` +1) |
| `web-rook`: `npm run typecheck` | clean |
| `web-rook`: `npm run build`, `VITE_ROOK_PHASE=1 npm run build` | clean (vite 7.3.6); `VITE_ROOK_PHASE=2` still stops the build |
| `forge build` | ran only to produce `out/` for the drift tests (lib copied in, not committed) |

**Found on the way.**

- The indexer's committed `fixtures/synthetic/` are already stale at this branch's base: regenerating them from `test/story.ts`
  without any change here moves 29 files (timestamps). Not regenerated here (the site's copies would follow); the new story events
  were appended last so that nothing about them would move a fixture.
- `web/` (the older site) still offers Redraw and uses `abisV3.ts`'s `redraw`; only its `src/api/types.ts` copy was updated (additive).

**Left open, or for a decision.**

1. `docs/v6/phase1-scope.md` §8.3 still describes the target-only check and the Trust page's earlier sentences; left to whoever
   merges the wave-2 branches (the monitor branch edits the go-live parts of the same file).
2. Screenshots in `docs/v6/screens/` predate the Raffles section, the ticket's new notices and the Trust page's new sections.
3. The 21 moderate advisories need wagmi 3 or dropping the `wagmi` connectors package for `@wagmi/core` + viem (E-16's own
   recommendation): an API change, not done here.
4. The site's refund and void times are the chain's clock; the contracts also wait the parent chain's blocks, so a button can
   revert `TooEarly` for a few minutes after it appears. It says "from about" and the contract refuses harmlessly; a read of the
   parent-chain half (`Wait.elapsed`) was not added.
5. A non-Phase-1 build keeps the target-free legacy path (no stack file, so no request check); only a Phase-1 build is meant for 4663.
