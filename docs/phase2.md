# Phase 2: a guarded deploy script, a web app, a keeper that runs locally, and nothing public

Written 2026-09-21, before its code. Findings are appended below as they are verified, each marked *verified (how)* or *unverified*.

## What Phase 2 is, and the line it does not cross

Phase 2 makes etch something a person can look at and click through: a script that can deploy the contracts, a web app that reads
them, and a keeper that performs the whole cycle — cap, declaration, wait, release, receipt, snapshot, draw, claim, delivery —
against a **local** chain. It does not put a contract, a key or a transaction on any public network. Deploying to Robinhood Chain,
funding anything, and holding any key that could hold value are the operator's acts, not this phase's.

## 1. The deploy script

`script/Deploy.s.sol` deploys, in this order: `ListingRegistry` (owner and treasury from the environment), `DrandEvmnet`,
`Launcher` (which itself deploys the `FeeRouter` implementation it clones), `Distributor`; then wires the registry
(`wire(launcher, distributor)`) and names the roles. There is no separate vault factory: **the registry deploys each listing's
vault itself** (`listInscriptions` / `listRune`), which is what stops a listing pointing at somebody else's contract; the brief's
"vault implementation/factory" is the registry.

Ownership. The registry is `Ownable2Step` and its constructor takes the owner, so the script constructs it with the **multisig as
owner from the first block**. But `wire` and `proposeRoles` are owner-only, so a deployer that is not the owner cannot call them.
Two honest options: (a) the deployer owns for the length of the script and hands over, or (b) the script deploys everything and
stops, and the multisig makes the two wiring calls. Phase 2 takes **(b) on any public chain** — the deployer never owns anything,
which is the property asked for — and prints the two calls (target, calldata) for the multisig to make. On the local chain, where
the "multisig" is an unlocked dev account, the script makes those two calls as that account so the app has something to read.

Refusals, all before any broadcast:
- `ETCH_DEPLOY_CONFIRM` must equal exactly `deploy-etch:<chainid>:<deployer address>` (lowercase hex address), built from the
  chain the RPC actually reports and the account that would actually send. A confirm string for another chain or another
  deployer is a refusal.
- `ETCH_OWNER` must be set, must not be the deployer, and on a public chain must have code (a multisig is a contract).
- keeper, purchaser and attestor must be three different addresses, none of them the owner or the deployer.
- the chain must be one the script knows: 4663 (Robinhood Chain) or 31337 (local). On 4663 the Pons factory must have code.

The refusals are in the Solidity script itself (so `forge script` cannot be pointed past them) and again in the wrapper
`script/deploy.sh`, which asks the RPC for its chain id first. A Foundry test runs the script's checks and proves the refusals.
It writes `deployments/<chainid>.json`.

**Where it is run in this phase: only against a local anvil fork of 4663** (`anvil --fork-url <public RPC> --chain-id 31337`),
sent from anvil's unlocked dev accounts by `eth_sendTransaction`. Those accounts' keys are printed by anvil at every start and are
known to everyone; they are worthless anywhere real. No private key appears in this repository, not even those.

## 2. The web app (`web/`)

Vite, React, TypeScript, viem and wagmi. Reads only what the chain says, through the deployment file for the connected chain.
**No mocked numbers.** With no deployment for the chain, every page says so in words and shows no figures. A DEMO mode, labelled on
every page, reads the local anvil deployment.

Pages: Home (what etch is, what is trusted), Launch, Coin, Listing, Draws and distributions, Trust. The Coin page's headline number
is **volume until the next buy**: `(cap − the vault's spendable ETH attributable next) ÷ (0.80 × (0.70% + creator tax))`, with
the 0.70% being Phase 1's measured creator-side take (docs/phase1.md F1) and the formula printed beside the number. It is an
estimate and says so: it assumes the measured rate, a cap that does not move, and that Pons's operator keeps sweeping pools.

Writes a visitor can make, all permissionless or their own: `harvest()`, `release(id)` once matured, `requestDraw`, `resolve` with
a drand signature the page fetches, `claimWin` with a proof from the published snapshot, `setDestination` / `requestPayout` with a
Bitcoin address checked client-side as bech32m before the wallet is asked to sign, `launch`.

Identity: stone and incision. A warm-grey-to-slate mineral palette with one oxide accent, a lapidary display face used sparingly,
hairline "cut" rules. No gradients-as-decoration, no countdown theatrics, no confetti: this is a page about where money goes.
No analytics, no trackers, no third-party scripts; fonts are self-hosted or system.

## 3. The keeper, locally real

The dry-run package keeps its property: nothing under `keeper/src/` can send. The local runner lives apart in `keeper/local/` and
gets its ability to send from **one** function, `localSender()`, which refuses unless `KEEPER_MODE=local` **and** the RPC reports
chain id 31337. It holds no key: it sends with `eth_sendTransaction` from anvil's unlocked accounts. The grep test is extended:
`src/` still matches none of the banned words; in `local/` only `guard.ts` may name a wallet client; and the guard's two refusals
are tested against a node that reports the wrong chain.

`npm run e2e:local` (from `keeper/`): deploy → launch a coin through the Launcher on the fork → buy on Pons's real curve → harvest →
post a cap → declare → advance an hour → release (with a bond) → the attestor posts a **fixture** receipt (no Bitcoin transaction
exists; the receipt is labelled as a fixture in the journal) → the creator chooses a weighted draw → the snapshot tool builds the
root from the fork's real Transfer logs → open → advance 15 minutes → request → **fetch the pinned round from drand's public HTTP
API** and prove it on chain → claim with the tool's proof → the winner posts a taproot address → the keeper records a fixture
delivery txid. The one real external input is drand; that is the point of using it.

Time on the fork is advanced with `evm_increaseTime`, which a real chain does not have. The pinned drand round is therefore in
the *real* future only if the fork's clock is near real time when the draw is requested; the runner sets the fork's next block
time to the wall clock before requesting, so the round it pins is one drand is about to publish.

## 4. Railway: ready, not deployed

`web/Dockerfile` builds the static site and serves it with a small Node server (no framework) that has `/healthz`, sets a strict
Content-Security-Policy, and nothing else. The site can go up before any contract exists because it says plainly when there is
none. Commands for the operator are at the end of this file. The keeper is not deployed anywhere in Phase 2.

## 5. CI

`.github/workflows/ci.yml`: forge build and unit tests; snapshot-tool and keeper tests; web typecheck and build. Fork tests run
on a nightly schedule and by hand, not on every push: they depend on a public RPC.

## 6. Letters

`docs/letters/unisat.md` and `docs/letters/magiceden.md`: drafts for the operator to send. Nothing in this repository sends them.

## How this could mislead

- A local end-to-end run proves the contracts and the tools agree with each other. It proves nothing about a real purchase, a
  real bridge transfer, a real delivery, or a venue's permission.
- The app's "volume until the next buy" is arithmetic on a measured rate, not a promise.
- A deploy script that refuses well is not an audit.

## Findings

### F1. The deploy script refuses as written, and ran only against a local fork — *verified (forge test, script/deploy.test.sh, a local run)*

`test/DeployChecks.t.sol` (8 tests) runs the script's own checks: no confirm string, a confirm typed for another chain (both
directions), a confirm typed for another deployer, an unknown chain, an owner who is missing / is the deployer / is not a contract
on a public chain, roles that are not three different addresses or that are the owner or the deployer, and a chain with no Pons
factory. `script/deploy.test.sh` (9 cases, against a bare anvil with nothing on it) proves the wrapper's refusals, including a
private key present in the environment and an RPC that reports no chain, and that past the wrapper the Solidity script still
refuses by itself. The one real run was against `anvil --fork-url <public RPC> --chain-id 31337`, sent with `--unlocked` from
anvil's dev accounts. **No private key is in this repository, and none was read.**

Two things found while doing it:
- **`block.number` inside a contract on this chain is the L1 block** (it is an Arbitrum chain), so the script's own record of its
  block is useless for log scans. The wrapper reads `eth_blockNumber` before deploying and writes it as `fromBlock`.
- *Decided differently from the brief:* there is no vault implementation or factory to deploy. The registry deploys each
  listing's vault itself, by design (a listing cannot point at somebody else's contract). And on a public chain the script does
  **not** wire the registry or name the roles: those are owner-only calls, the owner is the multisig from the first block, and
  the deployer never owns anything — so the script prints the two calls, with calldata, for the multisig to make.

### F2. Anvil's well-known dev addresses are swept on the real chain — *verified (on the fork)*

On Robinhood Chain the addresses anvil ships with (`0xf39F…`, `0x7099…`, `0x15d3…` and the rest) carry EIP-7702 delegations to a
sweeper contract (`0xef0100‖8a5b10eb…`). Forking brings that code along: the first local run released 0.0499 ETH to the dev
"purchaser" and its balance did not move — the delegate took it in the same transaction. The harness now clears the code on its
local accounts. **For production this is a rule, not a curiosity: a role address (purchaser above all, since it receives ETH)
must be a fresh address whose key nobody else has ever seen, and should be checked for code before it is named.** The deploy
script's checks do not look at the roles' code; on a public chain the operator must.

### F3. The whole cycle runs locally, ending in a claim and a delivery, with a drand round that did not exist when it was pinned — *verified (npm run e2e:local)*

14 steps, about a minute: deploy through the guarded wrapper → launch through the Launcher on Pons's real (forked) factory, Pons
recording the router as creator-fee recipient → three wallets buy 2.7 ETH on Pons's real curve → a stranger harvests (0.058 ETH
to the vault at a 2% tax) → the keeper's own policy (`src/plan.ts`) plans the cap and the declaration → bond posted → release by
a stranger, exactly the declared amount → the attestor settles a **fixture** receipt → the creator chooses a weighted draw →
`tools/snapshot` builds the root from the fork's Transfer logs (3 holders, 4 transfers) → open → a stranger pins round
20825116, **due 3 s after it was pinned** → the signature is fetched from `api.drand.sh`, verified on chain by `DrandEvmnet`, and the
chain's randomness equals drand's → a stranger proves the winner with the tool's Merkle proof → the winner posts a taproot
address → the keeper records a **fixture** delivery. The journal (`keeper/journal/e2e-latest.json`) labels every fixture.

*Decided differently from the brief, and why:* the brief said to time-travel the waits. An advanced clock makes the contract pin
a drand round that far in the **real** future (75 minutes for the two waits), which cannot be fetched — so the fast run does not
advance the clock at all. It rewrites the two stored deadlines (`releasableAt`, `challengeEnds`) with `anvil_setStorageAt`, which
keeps the chain's clock on the wall clock, so the pinned round is genuinely a few seconds ahead and the real beacon can be used.
Both tricks skip a wait that a real chain enforces; neither exists on a real chain. `npm run e2e:local -- --real-waits` uses
neither and takes an hour and a quarter. (Also tried and not possible: forking from a block 100 minutes old — the public RPC
keeps no state that far back; and `anvil --timestamp`, which a fork ignores.)

The guard: `keeper/local/guard.ts` is the only file that can send; it refuses unless `KEEPER_MODE=local` **and** the node itself
reports chain id 31337, and asks the node again before every send. It holds no key (it uses the node's unlocked accounts).
`keeper/test/local-guard.test.ts` proves the refusals (six wrong modes, five wrong chains, an RPC repointed mid-run) and that no
other file in `local/` names a send method, no file names a key or a buy endpoint, and nothing under `src/` reaches for the guard.

### F4. The web app — *verified (typecheck, build, the committed files alone build and serve, one look in a browser in each mode)*

With no deployment (today, on Robinhood Chain) every page says so and shows no figure; the Launch page is switched off with a
banner. In DEMO mode (`?demo`, labelled on every page) it read the local run: the coin page showed ≈ 1.92 ETH of trading until
the next buy, from `(0.0499 − 0.0084) ÷ 2.160%`, and the draws page showed the delivered lot with its fixture labelled as one.
Unverified: every **write** from a browser wallet (harvest, release, request, resolve, claim, destination, launch) — none was
clicked with a wallet attached; the same calls are exercised by the keeper's local run, not through the page. Phone width was
built for (single column under 48 rem, tables scroll in their own box) and not looked at on a phone.
The Trust page renders `docs/trust-model.md` itself, copied into `web/src/content/` by `npm run sync` and checked in CI to be identical.
No analytics, no trackers, no third-party request: fonts are bundled, the only connector is the browser's own wallet, and the
server's Content-Security-Policy is `default-src 'none'` with `connect-src` open to https (the RPC, drand, a published snapshot).

### F5. Unverified

- The Dockerfile was not built (no Docker daemon on this machine). What was done instead: the committed contents of `web/` alone
  were extracted, `npm ci && npm run build` run, and `server.mjs` served `/healthz` and the page from them.
- CI: *verified* — the workflow ran green on GitHub on its second push (contracts incl. the deploy wrapper's refusals, snapshot tool,
  keeper, web; the fork job is nightly/manual and was skipped). Its first push failed to parse: an expression inside a YAML flow mapping.
- A real deployment, a real purchase, a real bridge transfer, a real delivery, a venue's answer. As before.

## Deploying the web to Railway (the operator's commands; nothing here was run)

The site can go up before any contract exists: it says plainly that nothing is deployed. It needs **no environment variables and
no secrets**. Railway sets `PORT`; the server reads it.

```
cd ~/Documents/Etch
git status --short && git pull            # railway up ships the folder as it is: it must be clean
railway init --name etch                  # once: creates the project (or: railway link, to use an existing one)
railway add --service web                 # once: an empty service named web
railway up ./web --path-as-root --service web --ci
railway domain --service web              # once: gives it a public URL
```

When contracts exist on Robinhood Chain: `cd web && npm run sync` copies `deployments/4663.json` into the app, commit it, and
deploy the web again. The keeper is not deployed anywhere in Phase 2.
