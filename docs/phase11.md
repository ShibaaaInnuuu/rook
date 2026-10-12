# Phase 11 — a real inventory: the catalogue listed as vaults

2026-09-21.

The staging chain has carried two listings since Phase 6: `Demo Stones`, a collection of 1,024 inscription ids that name nothing,
and `DEMO•RUNE`, a rune id that was never etched. Everything the product does — fees splitting into a vault, a keeper posting a
cap, a draw, a pro-rata payout — it does against those two. A visitor meets a working machine with nothing in it.

Phase 9 committed a catalogue of **40 real Bitcoin collections and 19 real runes** (`web/public/catalogue/catalogue.json`, fetched
2026-09-21). This phase lists them. **58 vaults**: 39 collections and 19 runes, each one a real asset with a real name, a real
slug or rune id, and a real dated floor — and each one carrying a membership commitment that is honestly a placeholder, a
Bitcoin address nobody can be paid at, and, for runes, no cap at all.

Nothing here makes staging more real than it is. It makes the *inventory* real and marks every fixture as a fixture.

## 1. What the catalogue actually holds, and what the root therefore covers

A listing's `membershipRoot` is the registry's promise about **which inscriptions this vault may deliver**: `AssetVault.settleInscription`
takes a Merkle proof against it, and a leaf is `keccak256(bytes.concat(keccak256(bytes(inscriptionId))))` (`ListingRegistry.leafOf`).
A real listing's root must commit to the collection's **whole verified item list** — 1,563 ids for Pixel Pepes, 10,000 for Bitcoin
Punks, as the catalogue's own supply figures say — so that the attestor cannot settle something that was never in the collection.

**The catalogue does not hold item lists.** It holds, per collection, an identity (name, slug, source URL, supply, floor, an
image) and **three or four inscription ids** — one `sampleInscription` and up to three `items[]` entries kept for the picture
strip. That is what Ordinals Wallet's keyless index gave up; the full membership of a collection needs either a paid index or a
full `ord` node walking every inscription, and Phase 9 recorded that we have neither.

So the root committed here **is a placeholder, and is committed as one**:

| The root of a staging collection covers | Count | What it is |
| --- | --- | --- |
| the inscription ids the catalogue actually holds | 3 or 4 per collection | **real** ids of real inscriptions, from Ordinals Wallet |
| the staging fixture members `0000…0000i0` … `i1023` | 1,024 | ids that name nothing, already used by `Demo Stones` since Phase 6 |

It does **not** cover the other ~1,559 Pixel Pepes. It is not a claim that we know them. It is a commitment over the handful we
have, padded with the fixture ids the operator needs to keep demonstrating delivery on a chain that is never reset.

Two guards keep that honest:

- The fixture members are **refused off our own chains.** `plannedListings()` takes a chain id; asking for fixture members on
  anything but 9466301 or 31337 throws. A real listing is built from real ids or it is not built.
- Every listing this phase creates is recorded in `${STAGING_DATA}/listings.json` with `"rootCovers": {"realIds": 4,
  "fixtureMembers": 1024, "placeholder": true}` and the sentence *"a placeholder: the catalogue holds a few of this collection's
  inscription ids, not its membership"*. That file is what the keeper reads to build proofs, so the thing that makes proofs and
  the thing that admits what the root means are the same file.

**Nothing may say we have verified membership.** Not this document, not the operator's logs, not the manifest, and not the
chain's own `/info`, which now carries the claim in one sentence so that anything reading the staging chain meets it without
having to find this file. The site is another agent's ground and is not edited here; what it reads says what the root is.

## 2. Caps come from the catalogue's dated floor; runes get no cap

The keeper already derives a cap from a floor: `plan()` converts sats to wei through the bridge quote and adds 500 bps
(`keeper/src/plan.ts`), and `AssetVault.postCap` stamps it with `CAP_TTL = 1 hour`. The only thing missing was a floor per
collection. The listing creator now writes `${STAGING_DATA}/prices.json` from the catalogue: `floorSats` per slug, for all 39
collections, replacing the hand-written `{"demo-stones": "150000"}`.

At the fixture quote (3,200,000 sats/ETH, 1,500 sats fixed, 60 bps):

| Collection floor | Posted cap, one item |
| --- | --- |
| 7,000 sats (Space Pepes) | 0.0023 ETH |
| 411,099 sats (Pixel Pepes) | 0.1370 ETH |
| 200,000,000 sats (Bitcoin Bandits) | 66.65 ETH |

The floors are **Ordinals Wallet's, dated 2026-09-21, and not refreshed**. A cap posted from a three-day-old floor is a cap
posted from a stale floor; on a real chain the keeper reads a live source (`MagicEdenSource`) and the TTL is the whole point. The
`capRef` recorded on every staging declaration already says `staging-fixture:fixture:<slug>`.

The expensive end is left as it falls: a vault holding 0.4 ETH of fees cannot declare one Bitcoin Bandit at 66 ETH, so it posts its
cap and `plan()` returns `skip: under-one-item`. That is the correct behaviour and it is now visible across the inventory rather
than hidden behind one demo collection.

**Runes get no cap, deliberately.** The catalogue has no rune floor — Phase 9: *"a floor needs a marketplace, and every keyless
one refuses unauthenticated reads"* — so `prices.json` carries `"runes": {}` and the keeper posts nothing. What the contracts do
then was checked rather than assumed:

- `AssetVault.capWei` is 0 and `capExpiry` is 0.
- `declarePurchase` reverts **`CapExpired()`** at `if (block.timestamp > capExpiry)`, because every timestamp is past zero.
- So a rune vault **refuses to buy**. It accepts fees, attributes them, and will not declare a purchase. Nothing worse happens:
  `boundFor()` is never reached, so there is no path where a zero cap becomes an unbounded spend.

One consequence has to be named: **no real rune can buy anything on staging**, so the rune half of the product — a pro-rata
distribution, a holder's claim, a payout — would never appear. The DEMO rune the deploy lists (`DEMO•RUNE`, rune id `840000:1`)
therefore keeps its fixture price of 12 sats and goes on demonstrating that path. It is labelled a demo in its own name, and it
is the only rune vault that ever declares a purchase. The 19 real ones post nothing and say so.

That demo's rune id, `840000:1`, is also a real rune in the catalogue (`Z•Z•Z•Z•Z•FEHU•Z•Z•Z•Z•Z`). The registry does not
require a listing's `ref` to be unique and the indexer keys listings by id, so both exist: listing 1 is the demo, and the real
one gets its own vault with no cap. The manifest is keyed by **listing id**, not by ref, so the two are never confused — which
is why the keeper decides "no cap" from the manifest and the listing's own floor rather than from the rune id.

One honest wart: the error is named `CapExpired()` when the truthful name for this case is *no cap was ever posted*. It refuses
correctly and it refuses safely; only the reason it gives is imprecise. It is a contract change, so it belongs with the next
revision of the contracts before an audit, not to a listing phase. Recorded here so it is not lost. Until then the operator logs
`no-cap-posted` with `why: "the catalogue has no floor for a rune; nothing may be bought without one"` and the vault's own state
says the same.

## 3. The Bitcoin address: nobody can be paid at it

Every listing needs a `btcVault` string. On staging it is, per listing:

```
bc1p-staging-fixture-not-a-real-address-<slug>
```

This is **not a valid bech32m address** — it carries hyphens, which the charset does not allow — so no wallet will send to it and
no exchange will accept it. That is the point, and it is the same shape Phase 2 used for the local demo
(`bc1p-local-demo-not-a-real-address`). A BIP-350 test vector would also be unheld, but a *valid* address invites a UI to render
a QR code that a person could scan. An invalid one cannot be scanned, cannot be sent to, and reads as a fixture to anyone who
looks at it. The slug is on the end only so that the inventory shows 58 distinct strings, as a real deployment would.

**A real deployment needs one address per vault, generated by whoever holds the custody keys, and the address is the single most
dangerous string in the system.** It is where the purchaser's bridged ETH arrives as BTC. Nothing in this repository generates
one, holds a key to one, or checks one against a key, and nothing here should.

## 4. What a staging vault fakes that a real one must not

This is the list of work between here and a mainnet listing. Every row is something this phase writes as a fixture.

| # | Staging | A real listing needs |
| --- | --- | --- |
| 1 | membership root over 3–4 real ids + 1,024 fixture ids | a root over the collection's **complete, verified** item list, from a paid index or a full `ord` walk, with the list published so anyone can rebuild the root |
| 2 | `btcVault` is an unsendable marker string | a real taproot address per vault, generated by the custody signer, checked against its key, and test-received before a single fee lands |
| 3 | floors dated 2026-09-21, never refreshed | a live floor source per asset, with staleness refusing the cap rather than posting an old one |
| 4 | no rune floors at all, so runes cannot buy | a rune market source (authenticated), or runes are not listed |
| 5 | listings created by an unlocked owner the node impersonates | `listInscriptions` / `listRune` calldata **signed by the multisig**, one transaction per listing, reviewed before sending |
| 6 | every Bitcoin receipt, txid and delivery is a fixture hash | a real purchaser, real transactions, and an attestor that has seen them |
| 7 | the 7-day `CHANGE_DELAY` and the hour-long waits are shortened by rewriting storage | the waits run |
| 8 | 58 listings appear in one boot | listings added deliberately, one at a time, each with its own review |

Rows 1 and 2 are the ones that could cost someone money. Rows 3–4 cost an overpay. Rows 5–8 are process.

## 5. One place, two chains

All of it lives in `staging/catalogue/listings.ts`, which is pure: catalogue in, planned listings out, with no node, no keys and
no chain.

- `plannedListings(catalogue, { chainId, fixtureMembers })` → the 58 listings, each with kind, name, ref, root or divisibility,
  `btcVault`, the ids its root covers, its floor and its weight; plus `skipped[]`, each with a reason.
- `listingCalls(planned)` → `{ functionName, args, calldata }` per listing, encoded against `ListingRegistry`.
- `cd staging && npm run listings:calldata -- --chain <id>` prints those calls for the chain named on the command line. On a real chain it refuses
  fixture members (§1) and prints **calldata for the multisig to sign** — the same function, the same arguments, the same file.

Staging is the only place that *sends* them: `staging/bootstrap/listings.ts` takes the same planned listings and the node's own
impersonation of the owner. A real chain gets the calldata and a human.

## 6. Boot: 58 creations without lying about readiness

Each `listInscriptions` deploys an `AssetVault` (the registry's `_list` does `new AssetVault(...)`), so 58 listings are 58
contract deployments, not 58 cheap writes. The container's healthcheck has a 180-second start period
(`staging/chain/Dockerfile`), and today `bootstrap()` runs **before** the proxy listens, so everything it does is inside that
window.

Creation is therefore moved **out of the boot path**:

1. `bootstrap()` deploys the contracts and the two demo listings exactly as before, and the proxy starts.
2. Listing creation then runs beside the proxy, in batches of 8, with the batch's transactions sent together and awaited as one.
3. After every batch it rewrites `${STAGING_DATA}/listings.json`, and `/healthz` reports progress from that file:
   `"listings": {"created": 42, "total": 58, "done": false}`.
4. `/healthz` stays **200 while creating**. The container *is* ready: the node answers, the RPC serves, the demo listings work.
   Failing the healthcheck would restart the container mid-inventory, which is the opposite of what is wanted. The honest signal
   is the count, not a red light, and `done: false` is on the same endpoint for anything that wants to wait.
5. It is idempotent. It reads the registry and matches on **kind, ref and name together** — a rune id is not unique, since the
   demo rune's `840000:1` is also a real one in the catalogue (§2) — and creates only what is missing, so a restart in the
   middle of the inventory finishes it instead of duplicating it.

## 7. Keeping 58 vaults busy

Phase 8's simulator launched coins round-robin across two listings and, every twenty seconds, read every purchase and every
acquisition of every listing. Both break at 58. So does the thing the whole product is about: with 58 vaults and one simulator,
a vault's own trading fees reach its cap in days, and the pages that show what a vault has *done* stay empty on all but two.

**Launches reach every vault, in the catalogue's own order of popularity.** The first 58 coins go to 58 different vaults, the
busiest collections first (the top five carry 69% of the recorded all-time volume), with the runes spread evenly through them;
after that a rune every fourth coin, and the popular collections come round again sooner. A coin launched into a top collection
is given a "runner" or "momentum" mood, so the fee ranking a visitor sees is the catalogue's ranking and not a coin flip.
The cadence follows the inventory: the first two dozen coins arrive in the first minutes, then one every three to six minutes
until every vault has one, and only then the slow cadence Phase 8 chose for a chain that must look like weeks of use.

**Fees are topped up, and that is the one fixture in it.** The simulator finds the vault nearest its posted cap that cannot yet
afford one item, and sends it the difference through `AssetVault.depositFrom`, **as the treasury** — the same call the
treasury's twenty per cent arrives by, from an address nobody holds a key to, crediting the coin exactly as a real fee is
credited. Every one is logged `topped-up` with `staging-fixture` and the reason. Nothing about the cap, the queue, the public
wait, the bond, the draw or the settlement is faked by it; only the fee income, which on a chain nobody really trades is the
one thing that cannot be real. Runes are never topped up: they have no cap to reach (§2).

**Caps are re-posted before they expire.** A cap lives an hour. The operator used to post one only when it was also declaring a
purchase, so a vault that could not yet afford an item showed an expired price until it could — on two demo listings that was
invisible, on 58 it would be most of the inventory. The keeper now posts whenever its own policy says to: no cap, under fifteen
minutes left, or more than 200 bps away from the new one. That is what a real keeper does, and the policy that decides it is
unchanged (`keeper/src/plan.ts`).

**The spread pass is bounded.** A purchase that is settled, cancelled or written off, and a lot that is delivered, can never
change again: each is read once and remembered. The counts themselves are re-read for a rotating window of vaults, because a new
purchase only appears when the operator declares one and noticing it a minute later costs nothing. **The operator's pass is
bounded the same way**: it reads every listing that has work — an open purchase, a blocked vault, a cap inside its last quarter
hour, enough to declare with, or an unfinished lot — plus a rotating window of the rest. Nothing waiting is ever missed, however
many vaults there are. The numbers are in §8.

A held lot is **parked** rather than forgotten: a creator may choose again out of Hold, so it is re-read when its listing comes
round in the window instead of keeping that listing busy on every pass. Without that, one held lot per vault would have made
every vault permanently busy and undone the window entirely.

The listing set is re-read every thirty seconds instead of once at start, because listings now appear over the first minute
rather than all at boot, and **how far down the launch order the chain has got is kept on the volume**, so a restarted
simulator carries on into the vaults that have no coin instead of starting again at the top of the list.

**What is NOT done: trading after graduation — and why it cannot be, yet.** A graduated coin's curve is closed, so its chart
stops where it graduated. The obvious fix is to swap in its pool instead. **There is no pool.** Read from the fork, on a coin
the simulator graduated:

- the factory reports `phase = 1`, `sweptQuote = 4.2 ETH`, `sweptTokens = 2.857e26` — graduation happened;
- the factory's own `poolManager()` is `0x8366a39CC670B4001A1121B8F6A443A643e40951`, and it emitted **no `Initialize`** for any
  pool in the entire run;
- the swept tokens went **to the factory itself** (`0x7eD5…EC7e`), not to the PoolManager.

So on our fork the sweep moves the liquidity into the factory and whatever creates the pool afterwards never runs. Until it
does there is nothing to swap against, and a swap path in the simulator would have no counterparty. That is the gap, and it is
upstream of the simulator: the next step is finding what completes a graduation on this chain, not writing swap calldata.
Meanwhile most coins never graduate, so most charts keep growing; the graduated ones stop.

## 8. Measurements

Recorded from real runs on the machine that wrote this, not estimated. Two chains were used: a **fork of Robinhood Chain**
(`anvil --fork-url` at head − 20, chain id 9466301 — what the container actually boots), and a **local anvil at the same chain
id with no fork**, to separate our own cost from the cost of asking the real chain about an address it has never seen. The code
is the code the container runs: `staging/chain/start.ts`, its bootstrap, its listing creator, the operator and the simulator.

### First boot, cold

| | On a fork of Robinhood Chain | Local anvil, no fork |
| --- | --- | --- |
| node up, clock brought to now | 7 s | 1 s |
| deploy (`bootstrap()`: roles, `script/deploy.sh`, four contracts, two demo listings) | 5.0 s | 3.4 s |
| **the proxy answers from here** | **~7 s in** | ~4 s in |
| 58 listings, batches of 8 | 11.6 s | 1.5 s |
| **total to a full inventory** | **16.6 s** | 4.8 s |
| a second boot (nothing to create) | 1.2 s | 1.0 s |

The healthcheck's start period is **180 seconds** (`staging/chain/Dockerfile`). A cold first boot with the whole catalogue uses
**9% of it**, and the proxy is answering after the first seven. Of the 11.6 s, 10.6 s is transactions and about 0.7 s is
building 39 Merkle trees of 1,028 leaves each in the creator's own process.

Creation is still kept out of the boot path and still reports its count, because the margin is not the same everywhere: the fork
costs **200 ms per listing against 25 ms local**, and all of that difference is anvil asking the real chain about each new
vault address. A slower upstream moves that number and nothing else.

**Batching earns 20%, not an order of magnitude**: 58 listings took 10.6 s in batches of eight against 13.3 s one at a time,
because each vault is still its own block. Eight is kept — it costs nothing and it is what makes the progress count move in
steps a person can see.

### Per tick, with 60 vaults (58 from the catalogue and the 2 demo listings)

| | Before | Now, cold | Now, at ten minutes of full activity |
| --- | --- | --- | --- |
| operator pass (every 10 s) | 60 listings × 7 reads + every lot of every vault, for ever | 80 reads | **~225 reads**: the listings with work, plus a rotating window of 12 |
| simulator spread (every 20 s) | every purchase and every lot of every vault | 24 reads | **~80 reads**: 12 vaults' counts, plus what is still open |

That is 22 reads a second at ten minutes, against a node on the same machine, and it does not follow the size of the chain: a
settled purchase, a cancelled one, a written-off one, a delivered lot and a lot a creator chose to hold are each read once and
then not again. What is left is what is **open**, and a vault may hold at most three open purchases by policy, so the ceiling
is the work in flight rather than everything that has ever happened. On a chain that is never reset, that is the difference
between a tick that settles and one that grows all day.

### A second finding: a bond that was right once and wrong afterwards

`AssetVault.release` checks the purchaser's bond against **everything in their hands, this release included**. The operator
posted that bond when it *declared*, from the outstanding balance at that moment — correct on a chain where a vault has one
purchase at a time, wrong as soon as a vault has two: declare A, declare B, release A, and releasing B then needs a bond for
A and B while only B's was posted. It surfaced the moment vaults started reaching their caps repeatedly, as
`BondTooSmall(239746861908086836, 136998206804621049)` on a real staging vault.

The bond is now topped up **before every release as well**, from what the vault itself reports (`outstandingOf`,
`registry.bondBps()`) rather than from a running total the operator keeps. Nothing in the contracts changed: the contract was
right and the operator was optimistic.

### A third finding: an address the contract was always going to refuse

The simulator posts a winner's Bitcoin address for them, from three BIP-350 test vectors. One of them,
`bc1pw508d6qejxtdg4y5r3zarvary0c5xw7k…`, is BIP-350's **forty-byte** version-1 vector: valid bech32m, and not a taproot
output. `Distributor.setDestination` runs every address through `BtcAddress.isTaproot`, which takes the 62-character `bc1p`
form and nothing else, so every winner handed that one stalled at `BadAddress()`. On two listings it came up rarely enough to
look like noise; on 58 it was **thirty stalled deliveries in ten minutes**.

It is replaced with a third valid vector, and both sides are now pinned: `test/StagingFixtureAddresses.t.sol` asserts each
string against the library the Distributor uses, and the simulator's own test asserts the shape that library requires. The
existing test had checked the prefix and a minimum length, which the bad vector passed.

### A finding worth keeping: what "Insufficient funds" really meant

The fork carries Robinhood Chain's block gas limit, **2^50**. When anvil cannot estimate a transaction — which is exactly when
the call would revert — it checks the sender's balance against `gas_limit × gas_price`, and at one gwei that asks for about
**1,125,900 ETH**. The staging roles held 100. So **every reverting call came back as "Insufficient funds for gas * price +
value"** instead of its own error, and a genuine refusal was indistinguishable from an empty account. It was measured directly:
the same reverting call is refused for funds at 100 ETH and accepted (to revert properly, on chain) at 10,000,000.

The roles are therefore funded to **2,000,000 staging ETH** at bootstrap and kept there by the operator. The number is
meaningless — this chain's ETH is its own and nobody holds a key to any of these addresses — and it makes the node report the
real reason. Nothing about it reaches a real chain: `anvil_setBalance` is our node's own method and the public proxy refuses it.

## 9. What a visitor meets on a cold chain

Measured on the fork run of §8 — 60 vaults (58 from the catalogue, 2 demo), one simulator, one operator, the container's own
settings except that the public waits were cut to 60 s rather than the deployed 120 s, which moves the settlement times and
nothing else. Counts are cumulative from boot.

| At | 5 minutes (measured) | 10 minutes (measured) | 30 minutes (projected) | 120 minutes (projected) |
| --- | --- | --- | --- | --- |
| vaults | 60, all listed by second 16 | 60 | 60 | 60 |
| caps posted | 42 (every collection, twice over) | 42 | re-posted as they expire | re-posted as they expire |
| rune vaults saying "no cap posted" | 18 | 18 | 18 | 18 |
| coins, and vaults holding one | 21 / 21 | 25 / 25 | ~31 / ~31 | ~55 / ~55 |
| curve trades | 61 | 170 | ~570 | ~2,400 |
| purchases announced / released / settled | 17 / 7 / 2 | 46 / 31 / 46 | ~130 / ~120 / ~130 | ~500 / ~490 / ~500 |
| returned unspent, written off | 1, 1 | 1, 1 | 1, 1 held open by design | 1, 1 held open by design |
| draws opened / resolved / delivered | 1 / 0 / 0 | 27 / 2 / 1 | ~80 / ~70 / ~60 | ~330 / ~320 / ~300 |
| top-ups | 20 | 44 | ~150 | ~600 |
| failures | 0 | 0 | 0 | 0 |

The five- and ten-minute columns are counted from the log of that run. The thirty- and hundred-and-twenty-minute columns are
**projected** from the rates measured between them — about 20 curve trades, 9 settlements, 5 draws and 5 top-ups a minute — and
from the launch cadence (the first 24 coins in the first four minutes, then one every three to six until every vault has one).
They are arithmetic, not observations, and they assume nothing else changes.

One number in the table is deliberately flat: *returned unspent* and *written off* stay at one each, because the simulator
holds exactly one purchase in each of those states so the page that shows them is never empty (Phase 8's spreader, unchanged).

The shape of it: **the inventory is complete before a visitor could finish reading the front page**, coins arrive fast enough
that the grid is never empty, and the middle of the product — a cap posted, a purchase announced, released, settled, one
returned unspent, one written off, a draw opened, a winner, a delivery — fills in over the first ten minutes and then keeps
turning. Every rune vault but the demo one shows "no cap posted", which is the honest state and says why.

## 10. What could not be listed

**One collection of the 40: `Sub 100k`.** Its `sampleInscription` is `null`, its `first` is `null` and its `items[]` is empty —
the catalogue holds its name, its supply of 99,866 and its floor of 43,165 sats, and **not one inscription id**. A membership
root over nothing is either `bytes32(0)`, which `listInscriptions` rejects with `Empty()`, or a root over fixture ids alone,
which would be a vault claiming a real collection while committing to nothing but ids that name nothing. Neither is acceptable,
so it is skipped, and `listings.json` records it as skipped with that reason.

All 19 runes are listed. Their ids, symbols, divisibilities and supplies are the reference indexer's own and are exact
(Phase 9), and divisibility is carried through to the vault, which is what `boundFor()` divides by.

**39 + 19 = 58 vaults.**
