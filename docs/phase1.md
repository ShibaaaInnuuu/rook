# Phase 1: plan, then findings

Plan written 2026-09-21, committed before any Phase 1 code. Findings are appended below it as they are made, each marked
**verified** (and how), **read from source**, or **unverified**. Nothing is deployed, signed or sent in this phase either.

## The plan, in priority order

### 1. What reaches the router after graduation — the question that decides whether this is a business

Phase 0 proved the curve side and left the pool side read from source only. From that reading (Pons V2's pool hook,
`PonsV2MemeHook`): the hook takes its fee in the swap's *unspecified* currency, so an ordinary exact-input **buy** pays its fee in
the **coin**, and an exact-input **sell** pays in ETH. Coin-denominated fees are converted to ETH by a swap inside
`sweepPoolFees`, and the hook lets **only Pons's sweep operator** run a sweep while *any* coin-denominated fee is pending
(`_requiresTrustedOperator`). If that reading is right, then after graduation our router cannot sweep at all on a normal day — not
even the ETH half — and everything it earns arrives when, and only when, Pons's operator sweeps that pool.

To establish, on a fork of 4663 and from live history:
- a. launch through our Launcher, graduate, swap in the v4 pool in both directions; read the hook's pending buckets per currency;
- b. who can sweep what: the router's `sweepPool` with coin fees pending (expected: refused), with only ETH fees pending
  (expected: allowed), and the operator's sweep (impersonated on the fork — a fork can pretend to be anyone; nothing is sent);
- c. the router's take per 1 ETH of pool volume at a 0%, 2% and 10% creator tax, after the operator's conversion and the 30/70
  split, and what the 80/20 then leaves the vault;
- d. from live history: does the operator actually sweep, how often, and how long fees wait — `PoolFeesSwept` events on the
  live hook, across pools; and, if one exists, a real graduated coin whose recipient is a contract.
- e. tokens in the router: by the source reading the router never receives the coin through the ordinary path (everything is
  converted and credited to the escrow in ETH). The one path that does hand it coins is Pons's owner-only `rescuePoolFees`. The
  smallest safe handling is decided after (a)–(d), written here before it is built.

### 2. A snapshot tool anyone can re-run (`tools/snapshot/`, TypeScript)

Coin + block + kind in; balances at that block rebuilt from `Transfer` logs at the public RPC; the documented exclusions applied;
the Merkle tree built in exactly the leaf format `Distributor` verifies (ticket ranges for a draw; holder/amount for a rune
distribution); root printed, full list written as JSON. Deterministic, byte for byte. A test feeds the tool's root and proofs to
the Solidity contracts. `docs/trust-model.md` gains the third party's procedure for the 15-minute window, and what the tool
cannot prove.

### 3. A keeper dry-run (`keeper/`, TypeScript), read-only

Watches our contracts' events on a local node, reads prices from a pluggable source (a fixture source; one real public source
only if its terms allow automated reads, else none), computes what it *would* post — caps with their TTL, a declaration, the
purchase, the ETH→BTC quote from LI.FI's keyless public endpoint at a low rate — and writes a journal. **Structurally unable to
send**: no signer is constructed anywhere in the package, and a test fails if a wallet or signing import appears.

### 4. Can the purchaser's trust be reduced cheaply?

Two candidates from Phase 0's trust model: a per-declaration sub-address for the released ETH, and a bond the purchaser posts that
a write-off slashes. Each is evaluated for whether it *reduces* the trust or only *moves* it; built only if it reduces it.

### 5. README and this document updated with the results.

Not in Phase 1: a web app, real buying, bridging, any script that could deploy.

---

## Findings

### F1. After graduation the router earns 0.7% + the creator tax of pool volume, in ETH — and it arrives on Pons's operator's schedule, not ours. **Verified**, two ways.

**The headline number.** Per 1 ETH of pool volume the router receives **0.70% + the creator tax**; the vault's 80% of that is
0.56% + 0.8 × tax. Same as on the curve. Graduation does not thin the economics.

| Creator tax | Router take per 1 ETH of pool volume (fork) | Vault's 80% | Theory (70% of the 1% hook fee + tax) |
|---|---|---|---|
| 0% | **0.699%** | 0.560% | 0.700% |
| 2% | **2.72%** | 2.18% | 2.70% |
| 10% | **11.2%** | 8.98% | 10.7% |

*How (fork):* `test/fork/PoolFees.fork.t.sol`, 4663 at the public RPC, 2026-09-21: launch through our Launcher, buy the curve
through its threshold, call Pons's permissionless `createGraduatedPool` (graduation is two steps; seeding the v4 pool is the
second), make ten 0.05 ETH round trips straight against the pool manager, let the sweep operator sweep (impersonated — a fork can
pretend to be anyone; nothing was sent), then `harvest()`. Volume is what the trader paid in plus what it got back; a seller's
proceeds are already net of tax, which is why the 10% row reads a little above theory. With one 1 ETH trade against the ~4 ETH
pool the 0% figure was 0.636% — that is our own price impact on the conversion, not a fee; real pools are deeper.

*How (live history), which matters more:* eight coins launched by the product this design follows have graduated, each with a
**contract** as creator-fee recipient. For each, every `Swap` of its pool (pool manager `0x8366a39C…0951`) and every
`PoolFeesSwept` of the hook (`0xE5e70264…e044`) since launch were read (`docs/data/graduated-contract-recipients-2026-09-21.json`):

| Coin | Tax | Pool volume (ETH) | Credited to the recipient (ETH) | Per ETH of volume | Theory |
|---|---|---|---|---|---|
| `0x1f88…1de9` | 1% | 520.66 | 8.871 | 1.704% | 1.70% |
| `0x430a…a9b0` | 1% | 219.80 | 3.784 | 1.722% | 1.70% |
| `0xdb48…9105` | 1% | 112.72 | 1.964 | 1.742% | 1.70% |
| `0x05da…69eb` | 1% | 90.05 | 1.541 | 1.712% | 1.70% |
| `0xafd7…b165` | 1% | 74.49 | 1.264 | 1.697% | 1.70% |
| `0xd050…7b38` | 1.1% | 57.68 | 1.034 | 1.793% | 1.80% |
| `0x0d9d…3215` | 3% | 32.06 | 1.154 | 3.600% | 3.70% |
| `0x354c…12ad` | 2% | 25.37 | 0.690 | 2.718% | 2.70% |

1,132.8 ETH of real pool volume, 20.30 ETH credited to contract recipients: **1.79% on a volume-weighted average tax of ~1.1%.**
Volume here includes the hook's own conversion swaps, which pay no fee, so it slightly understates the rate. 82 of the same
launcher's 90 coins have not graduated; a coin that never graduates earns on the curve only, at the same rate.

**Which asset, and who may sweep.** **Verified** on the fork (`test_whoMaySweepWhat`), as read from source:
- The hook charges the swap's *unspecified* side. An exact-input **buy pays in the coin**; an exact-input **sell pays in ETH**. Live:
  of 27,230 fee events on those eight pools, 14,739 were coin-side and 12,491 ETH-side.
- While **any** coin-denominated fee is pending, only Pons's sweep operator may sweep that pool. Our router's `sweepPool` is
  refused — for the ETH half too. A stranger is always refused.
- The operator's sweep converts the coin half to ETH inside the pool (at most a 3% price move per sweep; the rest waits) and
  credits ETH to our router's escrow balance. **The router is never handed the coin by a sweep.** `harvest()` then splits 80/20.
- In the narrow state where only ETH-side fees are pending (just after an operator sweep, sells only since), the router's own
  `sweepPool` works. It is kept, and it is a convenience, not a guarantee.

**So post-graduation income depends on Pons's operator. Does it sweep? Verified from live logs:** in the 5.59 hours before block
68,797,305 the hook emitted **18,115 `PoolFeesSwept`** across **903 pools**; 25 of 25 sampled sweep transactions were sent by the
operator `0x49BbF2b7…73d2`. The gap between consecutive sweeps of the same pool: **median 84 s, p90 16 min, p99 1.7 h, max 4.9 h**.
It is a busy automated sweeper today. It is also one key at one company: if it stops, post-graduation fees sit in the hook, and
the only other exit is Pons's owner-only `rescuePoolFees`.

**Unverified:** that the operator will keep sweeping small pools when gas or its policy changes; whether it skips pools below some
size (342 of the 903 pools were swept once in the window, which is as consistent with low volume as with a threshold).

### F1e. Tokens in the router: decided before it is built

By the ordinary path the router never holds the coin. One path does hand it coins: Pons's **owner-only** `rescuePoolFees`, an
escape hatch that pays pending fees *directly* to the recipient, coin-denominated fees included. A router with no token function
would strand them for ever.

Considered and rejected: a permissionless `sellTokens(minOut)` that swaps through the pool. Whoever calls it chooses `minOut`, and
a caller who chooses 1 wei and sandwiches the swap takes the difference — the exact reason Pons gates its own conversion to a
trusted operator. A posted floor price would need an oracle this project does not have. A swap function on an ownerless contract
is a standing invitation, for a path that may never be used.

**Built instead: `forwardToken(token)`** — permissionless, moves the router's whole balance of any ERC-20 to the registry's
treasury and says so in an event. The treasury is expected to sell it and return the ETH by `escrow.credit(router)` or a plain
transfer to the router — both of which `harvest()` splits 80/20 like any other ETH, so the vault is made whole without any new
accounting. That expectation is **trust, stated as trust** (trust-model.md): the contract cannot force the treasury to sell or to
return anything. It is the smallest function that turns "stuck for ever" into "visible and recoverable", and it adds no swap.

### F2. The snapshot tool. **Built; verified against the contracts and against a real coin.**

`tools/snapshot/` (rules in `docs/snapshot-v1.md`, written first). 7 tool tests; `test/SnapshotTool.t.sol` (3 tests) hands the
tool's roots, ranges, amounts and proofs to the real `Distributor`: a weighted draw and a per-wallet draw settle on the tool's
proof for whichever entry holds the winning ticket, its ranges tile `[0, totalTickets)` exactly, every holder claims a pro-rata
share with the tool's proof, and `allocated + remainder` is the whole acquisition.

Run twice against a real graduated coin (`0x0d9d…3215`, block 68,790,000, 2,222 transfers) at the public RPC: the two files are
**byte-identical** (`sha256 4b77d5bb…`). Two things the real run taught:
- **A public RPC keeps no historical state** (`eth_call` at an old block fails). Balances never needed it — they come from logs —
  and the few facts read by `eth_call` (the coin's curve, its decimals) do not change, so they are read at `latest`.
- **The first real snapshot's largest "holder" was Pons's liquidity locker**, 8.2% of supply, which graduation parks there. It was
  not on the exclusion list; it is now, with the buyback vault, the position manager and the graduation executor, each read from
  the live factory. With it excluded the same coin gives 48 holders and 129,182,845 tickets. This is exactly the kind of error
  the 15-minute window exists for, and exactly why the exclusions are part of the published file.

**Unverified / not provable by the tool:** that the keeper used the block it promised; that the RPC told the truth (run a second).

### F3. Venues: what can be bought by API, and on whose permission. **Verified** by the parent session on 2026-09-21, by the methods stated; our words, their clauses cited.

**UniSat — first choice.** Read from their public OpenAPI file (`open-api.unisat.io/swagger.yaml`, 306 paths) and their developer
legal page. Both kinds can be **bought by API as PSBTs**: for inscriptions `…/market/collection/auction/create_bid_prepare` →
`create_bid` (returns the PSBT and what to sign) → `confirm_bid`; the same three for runes, with batch variants. Floor data:
`collection_statistic(_list)`, `runes/auction/list`. **Independent checks exist:** an address's rune balances, a rune's holders,
and a collection indexer that answers "is this inscription in this collection" and lists a collection's items — an outside opinion
on the membership our registry otherwise takes on the owner's word (one indexer's opinion, still). Their spec marks the buy
endpoints as needing explicit confirmation before a call; our declare → public hour → release is such a step. Keys are
self-serve (free tier 5 calls/s, 2,000 a day). **Terms:** §3.1 — any commercial use of the API "requires the prior written
permission from UniSat"; §2.3 they may suspend at their discretion; §2.4 no circumventing limits or redistributing data; §1.4 the
API may change or end. No clause banning automated trading was found in that text. *Not read:* paid plan prices, whether market
endpoints are in the free tier, marketplace fee rates.

**Magic Eden — second, and for runes only.** Read in a real browser (their docs answer fetchers with 403). The documented API has a
complete **rune** buy path ("get an unsigned PSBT for buying a rune" → "submit a signed PSBT"), rune balances by address, and
read-only collection data. It documents **no endpoint that buys an inscription.** Their Terms (PDF dated 01/29/2024; may not be
the newest) send anyone automating to the API and forbid driving the website with a bot (§4(c), §9(f)). Keyless calls share one
30-requests-a-minute demo key; the endpoints that return PSBTs need a key issued after a form. Clauses that need their written
answer before a key is used to buy: §9(h) (no use "for any commercial purpose or for the benefit of any third party"), §9(p)
(no manipulative trading — buying listed floors at the ask is ordinary buying, but no copy of ours may claim price impact), §9(q)
(no fundraising use; aimed at the assets bought being financial instruments, which inscriptions and runes are not, but close
enough to ask). §5(k)/§12: access ends at their discretion. The separate "API Terms of Service" their docs link to is a deleted
Google Doc today — unreadable, and one more question for the letter.

**OKX — none.** Their developer docs are now "Onchain OS" (wallet, payments, DEX trade, market data); the old Ordinals marketplace
pages 404 or redirect, and nothing in the market section mentions inscriptions or runes. Absence of a page is weaker evidence than
a statement; it is what was found.

**What this does to the design.**
1. Venue order: UniSat (both kinds), Magic Eden (runes), a direct PSBT from a seller, or a human at a website. **The contracts do
   not care how the purchaser buys** — they bound the ETH, time it, and demand a receipt — so a venue closing to us changes the
   keeper, not the protocol. That was not an accident, and it is the design's best property on this side.
2. **No key is used to buy until that venue has answered in writing.** One letter each, disclosing what this is: an automated buyer
   for a protocol's vault, buying at the ask under a posted cap, holding at a public address, delivering to draw winners and
   holders. UniSat: §3.1. Magic Eden: §9(h), §9(p), §9(q), and where the API terms now live.
3. A venue can cut us off at will (both say so). The trust model says so, and the design must never depend on one.
4. **The dry-run's price source — our reading, not theirs:** a keyless, read-only, low-rate read of a public price endpoint, by a
   program that buys nothing, is the use their free public APIs are published for. It is still their API under their terms, so
   the keeper ships with a **fixture source as the default** and one real source (Magic Eden's public collection stats and rune
   market info, far under the shared 30 a minute) that runs only when asked for by name. UniSat's needs a key and is left as an
   interface until §3.1 is answered.

### F3b. The keeper dry-run. **Built.**

`keeper/` (design in `docs/keeper-dryrun-v1.md`, written first). 9 tests. It reads listings and vaults by `eth_call`, a price from a
source, a bridge quote, and journals what it would post. **It cannot send:** its one RPC function refuses every method off a
five-name read list before any request is made; a test fails if any source file imports an account, names a key, a signing or
sending call, or a venue's buy endpoint. One keyless LI.FI quote was taken to check the quoter reads the live response:
0.1 ETH on 4663 → BTC via LayerSwap, ~26 min; 250,000 sats of inscription costs about **0.0806 ETH** through it. The Magic Eden
source is implemented against response shapes written from general knowledge — **unverified live** — is off unless named, and an
answer it does not understand is an error, never a price. Its reads of our contracts were tested against a stub node, **not** a
deployed instance: there is none.

### F4. Can the purchaser's trust be reduced cheaply? One candidate moves it, one reduces it. Decided here, before the code.

**A per-declaration sub-address — not built.** Releasing each declaration's ETH to a fresh address makes each one traceable to its
own bridge transaction. But whoever holds those keys is still the purchaser; the contract cannot tell a sub-address the purchaser
controls from one it does not, and nothing about theft gets harder. It relabels the trust. The traceability it offers is already
there: `Released(id, purchaser, amount)` and a receipt carrying the Bitcoin txid.

**A bond — built, because it changes the arithmetic of theft.** Today a purchaser who takes a released amount and never buys costs
the paying coins that amount; the vault stops, and the loss is recorded for ever. With a bond:

- The purchaser (anyone, in fact) posts ETH into the vault as a bond. `release` refuses unless the bond covers
  `bondBps` of everything released and not yet settled, **including** the purchase being released.
- When a purchase is written off, the bond is slashed by what was lost (up to the whole bond) and the slashed ETH is credited
  back to the coins that were charged, in proportion — exactly as `returnUnspent` credits them. What is recorded as written off is
  what the bond did not cover.
- The bond can be withdrawn by the purchaser only down to what the outstanding purchases still require, and not at all while the
  vault is blocked by an overdue receipt.
- **The owner is not the only one who can slash.** `writeOff` stays the owner's after the deadline; but once a receipt is 7 days
  past its deadline **anyone** may write the purchase off. Otherwise an owner friendly to the purchaser could simply never do it.
- `bondBps` lives in the registry: set at deployment, **raised at once, lowered only after the 7-day public delay** every other
  change waits. The default is **10,000 (100%)**.

*Does it reduce trust or move it?* It reduces it. At 100% a purchaser who steals `X` forfeits `X`: there is nothing to gain, and
the coins are made whole. At `b`% the worst case for the coins is `(1 − b)` of what is outstanding, a number set in public. What
it **costs** is the operator's capital — up to three open purchases' worth, locked while they are open. That is the honest price
of not being trusted, and at the start the amounts are small. What it **does not fix:** a purchaser who buys the wrong thing at an
inflated price from themselves (the cap and the receipt bound that, not the bond); an attestor who lies (a different key, a public
Bitcoin address to check against); and at under 100%, the remainder.

Considered and left for later: **pay on delivery** — the purchaser buys with their own money and the vault reimburses against the
attestor's receipt after a challenge window. It needs the same operator capital as a 100% bond and trusts the attestor's receipt
more; it is a bigger change to the flow than Phase 1 should make untested.

### F4 (built). The bond

`AssetVault.postBond / withdrawBond / requiredBond`, slashing inside `writeOff`, `ListingRegistry.setBondBps / acceptBondBps`.
`test/Bond.t.sol`, 8 tests, each trying to break a rule: no bond, no release; the bond must cover what is already out too; a
write-off pays the charged coins from the bond and records only the remainder; at 100% the coins are made whole; the bond cannot
be withdrawn below what is outstanding, nor while a receipt is overdue, nor by anyone but the purchaser; a settled purchase frees
it; an owner who does nothing cannot shelter it (anyone writes off 7 days past the deadline); the requirement rises at once and
falls only after 7 public days. The vault's invariants now read `attributed + bond == balance` with a third party posting bonds
mid-campaign. The ordinary tests lower the requirement to zero the legitimate slow way in their setup, so they still test
everything else exactly as before.

## Where Phase 1 leaves things

| | Count | |
|---|---|---|
| Unit, fuzz and invariant tests | 88 | `forge test` |
| Fork tests on 4663 (public RPC) | 8 | 6 from Phase 0 + who-may-sweep-what + the take at 0/2/10% |
| Snapshot tool | 7 | + 3 Solidity tests consuming its fixtures (in the 88) |
| Keeper dry-run | 9 | |

**Do the economics work after graduation?** Yes, as far as the chain can show. The rate is the curve's rate (0.7% + tax of volume),
it arrives in ETH, and eight real coins with contract recipients received what the arithmetic says (1,133 ETH of pool volume →
20.3 ETH). Two honest qualifications. **First, the income is Pons's operator's to deliver**, not ours to collect: on a normal day
our router cannot sweep a graduated pool at all. **Second, a rate is not a business**: 82 of that launcher's 90 coins never
graduated, and the vault's 80% of 1.7% needs about **74 ETH of volume to buy a 1 ETH floor** (at a 1% tax; about 46 ETH at 2%).
A coin that trades 500 ETH, like the best of those eight, buys six or seven such floors; most coins buy none. That is the same
arithmetic the product this follows lives with — it is a statement about memecoins, not about the contracts.

**Still unverified:** that the operator keeps sweeping small pools; the Magic Eden response shapes in the dry-run; the dry-run
against deployed contracts (there are none); every venue's written answer; UniSat's fee rates and which tier includes its market
endpoints; a fill, a bridge transfer, or a delivery — nothing was bought, bridged or sent. Unaudited.

**Phase 2, in order:** the two letters (UniSat §3.1; Magic Eden §9(h)/(p)/(q) and the missing API terms); a local anvil
deployment script that refuses any other chain id, so the dry-run and the snapshot tool run against real instances; a
UniSat price source once permitted; pay-on-delivery evaluated against the bond with the same tests; an audit before any key.
