# Sweeps: a coin's fees buy wFROG, and a policy fixed at launch decides where it goes

Written 2026-09-23, **before its code**, on branch `wrap-sweep` (from `f24e234`). This is the design for Nosh-style sweeps
over Etchwrap's wrapped model (docs/v3/wrapper.md). Nothing here is deployed or audited.

## In one paragraph

A coin launched on a **sweep listing** routes its fee share (80%, `FeeRouter.VAULT_BPS`) to a `SweepVault`. That vault has one
**policy, fixed when it is constructed**: `RAFFLE` (each whole wFROG it holds goes to one holder of the coin that paid for it,
chosen by balance and a drand round) or `BURN` (each whole wFROG is redeemed to a provably unspendable Bitcoin output, so the
frog leaves circulation for good). The vault buys wFROG with the coin's ETH through **one immutable venue**, never above a
keeper-posted cap that expires in an hour, or receives wFROG directly once coins pay their fees in it. It has no withdraw, no
owner setter, no policy setter. Every wei and every wFROG unit inside it is charged to exactly one coin.

## Rollout in stages

> **Superseded on 2026-09-23 by docs/v3/curve.md**: we no longer depend on Pons approving wFROG. Coins launch on our own
> ETH-quoted curve whose graduation buys frogs; supply comes from graduations, the public mint desk and holder deposits. What
> survives from this section: holder deposits (1a, now the arbitrage route), the protocol-owned wFROG/ETH pool (2), and the
> wFROG-fee path (3), which now carries the ALIEN/wFROG pool's fees. The Pons stages are kept below as the record of the
> alternative, and because the vault still serves a Pons coin unchanged: its `launcher` is whichever launcher names the coin.

Measured by the coordinator on chain 4663: Pons V2 pair tokens are **curated** — the factory has approved 69 (stocks/ETFs,
USDG, cbBTC, TAO, a few community tokens), each with Pons-set economics — and a coin's pair is fixed from its first curve trade.
So wFROG cannot quote a coin until Pons approves it, and the design must stand without that.

| stage | what exists | where a coin's fees go | built here |
|---|---|---|---|
| 1a | holder deposits: a frog sent to custody mints 1 wFROG | — | `Wrapper.wrapDeposit` (contract side); canister spec below |
| 1b | a genesis coin quoted in ETH on the existing AutoBuyer listing | its buyer sweeps floor frogs on Bitcoin, they are wrapped | nothing new (today's `wrap` path) |
| 2 | a wFROG/ETH Uniswap v4 pool on 4663, depositors + protocol-owned liquidity | — | spec only |
| 3 | Pons approves wFROG: coins launch X/wFROG | fees **are** wFROG → `SweepVault.depositWrapped` | the vault's entry point; the router is spec |
| 4 | fallback, Pons declines: coins launch X/ETH | ETH → `SweepVault.deposit` → `sweep` buys wFROG in the stage-2 pool | `SweepVault` |

Stages 3 and 4 feed **the same settlement**: both land as `units[coin]` in the same vault, and the raffle and the burn read only
`units[coin]`. Which way the wFROG arrived is invisible to them.

### 1a. Holder deposits (supply without Pons)

Today the only way a wFROG is minted is `Wrapper.wrap`: a frog the vault bought, receipted, ruled `Never` by the paying coin's
creator. That needs fees, which need coins, which need a market. A holder deposit breaks the loop: a frog owner sends an
inscription to `custodyAddress`, the canister confirms it, and one wFROG is minted to the EVM address the depositor named.

**How the depositor names the address — in the Bitcoin transaction itself.** The deposit transaction carries an
`OP_RETURN` output whose payload is `"EWD1" ‖ chainId(4663, 4 bytes BE) ‖ wrapper address (20) ‖ recipient (20)`. The canister
mints to the recipient in that output and to nobody else. There is **no EVM-side claim**, so there is nothing to front-run: the
binding between frog and recipient is signed by whoever owned the frog, inside the transaction that moves it.

**The canister's checks** (built on branch `canister-curve`; the canister side, including which custody outputs count as
deposits, the proof, the rate limit and the return of a refused frog, is in docs/v3/curve.md, "Canister: graduation buys, the
mint desk and holder deposits"). A deposit is attested only if every one of these holds:

1. **Confirmation depth ≥ 6** Bitcoin blocks, re-checked immediately before the EVM call. A reorg deeper than that would leave a
   wFROG minted against a frog that went elsewhere; six is Bitcoin's customary settlement depth and the residual risk is stated.
2. **The custody output is new**: no input of the deposit transaction is a custody UTXO. (A buggy or compromised attestation of a
   frog custody already holds would otherwise mint twice. The contract also refuses an inscription it already counts as held.)
3. **Exactly one inscription on the output's sats, and it is the declared one**, per ord *and* at least one independent explorer
   (the custody module's existing two-source rule). An output that also carries **runes, another inscription, or a
   parent/child reinscription** is refused and NOT minted: a frog carrying extras would make one wFROG worth more than one frog,
   and redemption would hand the extras to a random redeemer. Refused deposits are returned by the custodian: whole, to the
   taproot script of the deposit's input 0, with custody's delivery builder; a non-taproot sender's frog is held for a person
   (docs/v3/curve.md, "The return of a refused frog").
4. **The frog is in the collection**: the contract checks this itself, against the listing's Merkle root.
5. **All inputs signed with `SIGHASH_ALL`/`DEFAULT`**, so nobody else could have rewritten the `OP_RETURN` after signing.
6. **Exactly one `OP_RETURN`** of the format above, naming this chain and this wrapper; the recipient is not zero.

**The contract side (built): `Wrapper.wrapDeposit(inscriptionId, proof, txid, vout, to)`**, callable by the `custodian` only.
It checks the listing is not paused, the inscription is a member (`registry.isMember`), the outpoint `(txid, vout)` has never
minted, and the inscription is not already held by this wrapper; then it counts the frog as held, queues it like any wrapped
frog, and mints `ONE_ITEM` to `to`. A deposited frog's id in the wrapper is `DEPOSIT_FLAG | n`, disjoint from acquisition ids.

**Why this edits `Wrapper.sol` (unavoidable):** the wrapper is the only minter of wFROG and keeps the ledger
`totalSupply == (held − openTickets) × 1e18`. A deposit is a new frog entering `held` and a new token entering supply; anything
outside the wrapper doing either breaks the ledger. The edit is additive: one function, three mappings, one event, and
`recordDelivery` clearing the held-inscription flag. Nothing existing changes behaviour.

**Attack surface of deposits, stated:**
- *Front-running a claim*: none — no EVM claim exists (above).
- *Double-mint on a reorg*: the outpoint is minted once, ever. A reorg that *re-includes* the same deposit re-uses the same
  outpoint (refused). A reorg that *drops* it after minting is an unbacked wFROG: bounded by the six-block depth, not by code.
- *A frog with extras* (runes, a second inscription): the canister refuses (check 3). The contract cannot see sats.
- *A custodian that lies*: it can mint wFROG against nothing. That is the same trust the wrapper already places in custody
  ("the custodian is the whole question", Wrapper.sol); deposits do not widen it beyond the key that already holds every frog.
  They do widen *what a stolen key can do quietly* — mint rather than only steal — so the canister should rate-limit deposits
  (e.g. 50 a day) and the monitor should reconcile `held` against the custody UTXO set daily.

### 1b. A genesis coin quoted in ETH

Needs nothing new: a coin launched on the existing AutoBuyer inscriptions listing pays its fees in ETH to the AutoBuyer vault,
the buyer sweeps floor frogs on Bitcoin, and each receipted frog is ruled `Never` and wrapped, minting to the pro-rata claim
contract (WrapClaim) for the coin's holders. This *is* the canister route: slow (a Bitcoin purchase per frog), but it pulls
supply onto the chain before any pool exists.

### 2. The wFROG/ETH market (spec)

> **Built differently on 2026-09-23 (docs/v3/pools.md)**: one shared, hookless wX/ETH pool per collection at 0.30%, seeded by
> graduations (not the treasury) in layers; its fees go to the seeding coins' vaults as seeder shares, not to the treasury.

A Uniswap v4 pool on 4663, wFROG/native ETH, no hook, 1% fee tier (NFT-backed tokens are thin; a low tier invites
sandwiching the vault's own buys). Seeded by depositors (anyone) and by **protocol-owned liquidity** funded from the
**treasury's 20%**, never from a sweep vault (that would mix money the vaults promise to keep separate).

**Who owns the POL, and can it be withdrawn: never.** The v4 position NFT is minted to a `LiquidityLock` contract that has
`addLiquidity` (treasury-funded) and `collectFees` (fees go to the treasury) and **no** `decreaseLiquidity`, no NFT transfer,
no owner. The liquidity is permanent, like the vaults. (Not built here: it needs the v4 PositionManager address on 4663, which
has not been measured in this repo.)

### 3. Pons approves wFROG: fees are wFROG

A coin quoted in wFROG pays its creator fees in wFROG, so the sweep is automatic. What is needed:
- a **WrappedFeeRouter** (a sibling of FeeRouter, spec only): claims the wFROG fees from escrow, sends 80% to the vault with
  `wrapper.approve(vault, x); vault.depositWrapped(coin, x)` and 20% to the treasury. Same one-shot `initialize`, no owner.
- `SweepVault.depositWrapped(coin, amount)` (built): only that coin's router, only for a coin on this listing; pulls the
  wFROG with `transferFrom` and credits `units[coin]` by the balance actually received.
- `Launcher` must pass wFROG as `pairToken` for coins on this listing. That is a Launcher change for the merge; it is not
  made here.

### 4. Fallback: coins quoted in ETH, fees swept into wFROG

The coin's 80% arrives as ETH via `FeeRouter.harvest → vault.deposit{value}(coin)`, credited to `credit[coin]`. Anybody may
then call `sweep`, which buys wFROG in the stage-2 pool (details below). After graduation, a protocol-seeded X/wFROG v4 pool
can sit beside Pons's official X/ETH pool, funded from the treasury and locked the same way as stage 2's POL (spec only).

## The policy, fixed at launch

`SweepVault` takes `Policy policy` in its constructor and stores it `immutable`. There is no setter, no owner-only switch, and
the test suite reads the compiled ABI to assert that the vault's external surface is exactly the functions listed in this
document. A coin's policy is therefore **its listing's vault's policy**: `Launcher` initialises each FeeRouter with
`registry.vaultOf(listingId)` exactly once and FeeRouter has no setter. So a sweep collection has **two listings**, one per
policy (`<name> · raffle`, `<name> · burn`), and a creator picks the policy by picking the listing. Raffle money and burn
money are in different contracts; they cannot mix.

Retired from Nosh's earlier drafts and not offered: hold-in-vault, an equal-odds (per-wallet) lottery, a creator-named
recipient. **The creator gets nothing directly** — no function in the vault pays or names the creator.

## The split: 80/20, the existing constant

`FeeRouter.VAULT_BPS = 8_000` already splits every harvest 80% vault / 20% treasury, and routers are clones of one
implementation. A different split for sweep coins would need a second router implementation for a number nobody has a
reason to change: the 20% is what funds stage 2's POL and stage 4's X/wFROG pools. Kept at 80/20.

## Attribution: per coin, from the coin's own router only

- `deposit(coin)` (ETH) and `depositWrapped(coin, amount)` accept only `msg.sender == launcher.routerOf(coin)` and
  `launcher.listingOf(coin) == listingId`, where `launcher` is fixed at construction (the Pons Launcher, or our CurveFactory,
  which names each coin's curve as its router). Any other sender reverts. `depositFrom` (the Treasury's entry) always reverts:
  treasury money is not a coin's fees.
- `receive()` accepts ETH only from the venue, only during a sweep (a refund of unspent input). Everything else reverts.
- Every sweep is charged to one coin: `credit[coin] −= spent`, `units[coin] += bought`.
- wFROG transferred to the vault by anyone else is **stray**: credited to no coin, drawn by no raffle, burned by no one. It
  stays. That is the point: a direct payment buys no say.

**What this does NOT distinguish, and cannot:** fees earned from trades vs. ETH somebody paid into the coin's escrow
(`escrow.credit(router)`) or sent to the router. FeeRouter splits all of it alike, and the escrow does not record why ETH
arrived. The consequence is bounded: money paid in that way funds **that coin's own** raffle or burn, exactly as its trade
fees would, and cannot move which holder wins (the snapshot is by coin balance, the round by drand) or anything about another
coin. Paying a coin's fees for it is a gift to its holders, not a lever.

## The sweep route: the pool, permissionless, bounded by a cap

Two routes were weighed:

| | pool route (built) | canister route |
|---|---|---|
| how | `sweep` swaps the coin's ETH for wFROG in the wFROG/ETH pool | the buyer spends the ETH on floor frogs on Bitcoin, then wraps them to the vault |
| trust | the venue address (immutable) and the keeper's cap | the buyer's program, the custodian, and a bridge for the ETH |
| latency | one transaction | a Bitcoin purchase and six confirmations per frog |
| supply effect | when vault buying lifts wFROG over the floor, arbitrageurs buy floor frogs, deposit them (1a) and sell the wFROG to the pool: frogs still leave Bitcoin's market, one hop later | frogs leave Bitcoin's market directly |
| attribution | exact: one coin per call, measured deltas | per purchase, as AutoBuyerVault does |

The pool route is built because it is atomic, needs no new canister code, and its supply effect converges to the canister
route's through arbitrage once deposits (1a) exist. The canister route stays as stage 1b.

**The canister route is built too, through the public mint desk** (docs/v3/curve.md): `sweepViaDesk(coin, amount)` escrows
at most one cap's worth of the coin's ETH for one frog, and `collectDesk(id)` brings the wFROG (and the change, or the whole
amount if the request expired) home, measured and charged to the coin. `spend ≤ cap × bought` holds for it as for the pool.

**`sweep(coin, spend, swapData, minOut)`** — anyone may call it:
1. the listing is not paused, `0 < spend ≤ credit[coin]`, `minOut > 0`, and the cap is fresh on **both** clocks;
2. the vault calls **only** `venue` (immutable, set at construction, has code, not the wrapper, not the vault) with
   `value: spend` and the caller's `swapData`. The vault never approves anyone for anything, so no call can pull its wFROG;
3. it measures `spent` = its ETH balance before − after, and `bought` = its wFROG balance after − before;
4. it requires `bought ≥ minOut` and **`spent × 1e18 ≤ cap × bought`** (cap is wei per whole wFROG);
5. `credit[coin] −= spent`, `units[coin] += bought`. Reentrant calls into the vault revert.

**The cap** is posted by the keeper (`postCap(weiPerWrapped)`) and refused once **either** clock has passed an hour since
posting (`lib/Wait.sol`: the sequencer can run its clock an hour ahead, which alone would make a stale cap look fresh).
The keeper sets it from a reference price (the frog floor on Bitcoin in ETH, plus a margin); **the bound is the keeper's
word**, as in Nosh. What the contract guarantees is only that no sweep pays above the posted cap, and that an hour-old cap
buys nothing.

A permissionless caller can route badly (sandwich themselves, take a worse price), but never worse than the cap: the loss
from a malicious caller is bounded by `cap − fair price` per wFROG, the same bound Nosh accepts for its floor sweeps.

## Settlement

### RAFFLE — each whole wFROG to one holder, balance-weighted

Reuses the Distributor's draw machinery, over wFROG units instead of lots:

1. `openDraw(coin, root, totalTickets, uri, snapshotAt, snapshotBlock)` — keeper. Requires `units[coin] ≥ 1e18` and moves
   one `1e18` into the draw's reservation. The snapshot must be **at most an hour old on both clocks** and not in the future
   (stale snapshots are refused). The ticket list is the coin's holders, one ticket per whole coin, published at `uri` with
   leaves `keccak256(bytes.concat(keccak256(abi.encode(holder, firstTicket, endTicket))))` (the Distributor's leaf). The
   keeper excludes the vault, pools, the router and the burn address.
2. 15-minute challenge window on both clocks. The owner may `cancelDraw`; the reservation returns to `units[coin]`.
3. `requestDraw` — anyone, after the window: pins `beacon.roundAt(block.timestamp) + 20`, a round nobody has seen.
4. `redraw` — anyone, if the pinned round is still unproven 24 hours later on both clocks.
5. `resolve` — anyone: reads or proves the round; `winningTicket = keccak256(randomness, vault, drawId) % totalTickets`.
6. `claim(drawId, holder, first, end, proof)` — anyone, for the holder in the leaf: pays `1e18` wFROG to that holder.
7. `voidDraw` — anyone, 7 days after resolution (both clocks) if unclaimed: the `1e18` returns to `units[coin]` for a later draw.

**The raffle cannot be aimed**: the root and the ticket count are fixed before the round is pinned, the round is in the
future when pinned, and nobody can re-pin before 24 hours. **The snapshot is still the keeper's word** (as in the
Distributor): 15 minutes for anyone to recompute it, and the owner to cancel it.

### BURN — each whole wFROG leaves circulation for good

`retire(coin)` — anyone, when `units[coin] ≥ 1e18`: the vault calls `wrapper.redeem(BURN_ADDRESS)`, which burns the vault's
`1e18` and opens a redemption ticket like any holder's. The drand round picks the frog; the custodian delivers it to
`BURN_ADDRESS`; the keeper or custodian records the delivery.

**`BURN_ADDRESS = bc1p2zffkaxp5py4fdutfdsrt6t6tcrc5ks09rkfd428hlhf4n5q8tqqwv7ahk`**: a taproot output whose key is BIP341's
NUMS point `H`, x = `50929b74c1a04954b78b4b6035e97a5e078a5a0f28ec96d547bfee9ace803ac0` = SHA-256 of the uncompressed
secp256k1 generator (verified while writing this). Nobody knows its discrete log, and with no script tree committed
there is no script path: an inscription sent there is provably unspendable, and anybody can check that by recomputing `H`.

**The ledger is unchanged, retirements included.** `redeem` burns `1e18` and increments `openTickets`; delivery decrements
`held` and `openTickets`. `totalSupply == (held − openTickets) × 1e18` holds at every step, exactly as for a holder's
redemption; the invariant suite runs it with retirements mixed in.

**If the custodian does not deliver** within 7 days, `Wrapper.refund` mints `1e18` back to the ticket's owner — the vault.
`reclaim(ticketId)` (anyone) then credits it back to the coin that retired it, so it can be retired again. Until reclaimed it
is stray (above), never lost.

**The canister (spec for the merge):** deliver a burn ticket like any ticket, to its destination, with the same
check-before-send rule; it is not special. It should additionally refuse to treat `BURN_ADDRESS` as a custody input (it
can't be — nobody can sign for it) and publish the delivery txid, so a watcher can confirm the frog sits at `H` forever.

## What the vault can never do

- **Be drained.** The only outflows are: ETH to `venue` inside `sweep` (bounded by `credit[coin]` and by the cap against
  wFROG received); ETH escrowed at the mint desk by `sweepViaDesk` (at most one cap per frog, and only the vault can collect it); `1e18` wFROG to a proven winner of a resolved draw that reserved it; `1e18` burned by `redeem`. There is no
  withdraw, no approve, no arbitrary call, no owner function that moves assets.
- **Call anything but the venue** (and the wrapper, the beacon, the registry and launcher it reads).
- **Change its policy** (immutable; ABI surface asserted by test).

## What this does NOT protect against

- The keeper's cap and the keeper's snapshot. Both are its word, bounded in time and challengeable only in the ways above.
- A venue that misbehaves. It is immutable; choose it well (a v4 router, audited). The vault still gets `bought ≥ minOut`
  at `≤ cap`, or reverts.
- A custodian that does not deliver a burn ticket: refund and `reclaim` restore the books, not the frog's exit.
- Deposits (1a): a lying or compromised custodian can mint unbacked wFROG. Same key, same trust as custody itself.

## What agent B's registry must provide (for the merge)

The registry deploys vaults inside `_list`/`_listAuto`, and `ListingRegistry.sol` is not edited on this branch. A sweep
listing needs, as stated in `src/interfaces/ISweepListing.sol`:

- a listing kind for sweeps, e.g. `Kind.Sweep`, and `listSweep(name, slug, wrapper, policy, venue, launcher, desk) returns (uint256 id)`
  (owner only), which deploys `new SweepVault(this, id, wrapper, policy, venue, launcher, desk)` (the beacon is the wrapper's) (or via a factory, to keep the
  registry under the size limit) and stores it as the listing's `vault`, so that `Launcher` routes the coin's fees to it;
- `vaultOf(id)` returning it; `isPaused(id)`, `keeper()`, `owner()`, `launcher()` as today (the vault reads them);
- no `btcVault` or membership root for a sweep listing (it holds wFROG, not inscriptions); Distributor and Treasury calls
  against it revert by design (`acquisition`, `depositFrom`).
