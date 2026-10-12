# The protocol token, version 1

Frozen 2026-09-21, before its code, as every document in this repository has been. It was written after the
operator decided "and we will have protocol token", having seen that the comparable product gives NFTs to holders
of its own token, weighted by balance, on a cadence, with nothing to enter.

It settles **mechanism**. It settles **nothing about supply, distribution or allocations**: those are the
operator's, and §6 lists them as open questions with a recommendation and no more. Nothing in this document is
legal advice, and §5 says plainly why a lawyer must read it before anything is deployed.

## 1. What the token is, and what it is not

Every coin launched here splits its creator fees 80 / 20: eighty to the vault of the collection or rune it is
paired with, twenty to a treasury address. Until now the treasury has been an address that does nothing. This
document gives it a purpose: **the treasury's twenty per cent buys Bitcoin assets through exactly the machinery a
vault already uses, and those assets are distributed to holders of the protocol token.**

It is, precisely:

- a token whose holders are entered in periodic distributions of assets the treasury **has already bought**;
- entered automatically, by holding it at a published block. There is nothing to enter and no action to take
  until you have won something and must say where it should go.

It is **not**, and no page of ours may imply otherwise:

- **not a claim on the treasury.** No holder can demand the treasury's ETH, and no function anywhere pays a
  holder ETH. The treasury has no withdraw function, exactly as a vault has none.
- **not a promise of returns, or of any distribution at all.** If no coin trades, no fees arrive, nothing is
  bought and nothing is distributed. A holder is owed nothing at any point.
- **not equity, not a share of revenue, not a debt.** Holding it conveys no ownership of anything and no right
  against anyone.
- **not a lottery ticket sold by us.** We sell nothing to enter: holding is the entry, and the draw is over
  assets already owned by a contract.

The distinction that matters, and the one our whole brand rests on: **the ledger is published.** Every wei the
treasury receives, every purchase it declares, every asset it acquires, every snapshot and every draw is on chain
or in a file anyone can rebuild. What is trusted is named in `docs/trust-model.md` and is not reduced by this
document.

## 2. The mechanism, which is the machinery we already have

Nothing here is new machinery. The parts are the ones already built, tested and documented.

1. **Fees arrive.** Every coin's `FeeRouter` sends 20% of what it claims to `registry.treasury()`. That address
   becomes a `Treasury` contract, set through the registry's existing seven-day public delay
   (`proposeTreasury` / `acceptTreasury`). The router is unchanged; it does not know or care what the treasury is.
2. **The treasury funds a vault.** `Treasury.fund(vault, amount)` moves ETH into any **registry** vault,
   attributed to the protocol token as the paying coin. The treasury has **no withdraw function**: ETH leaves it
   only into a vault, and a vault's ETH can only ever buy that listing's own asset. The keeper chooses which
   vault, which is a choice of *collection*, never a choice of *recipient*.
3. **The purchase is the ordinary one.** `AssetVault` then behaves exactly as it does for any coin: a keeper-posted
   cap with a one-hour life, a declaration published an hour before any ETH moves, a release anyone may call, a
   bond that covers it, a receipt from a different key inside a deadline, and the whole machine stopping at the
   first overdue receipt. Every safety property of `docs/phase1.md` and `docs/phase3.md` holds unchanged, because
   the code is unchanged.
4. **The distribution is the ordinary one.** The acquisition is charged to the protocol token, so `Distributor`
   treats it as that coin's: a snapshot of the token's holders published as a Merkle tree, a fifteen-minute public
   challenge window in which no draw may be requested, a draw pinned to a drand round that did not exist when it
   was pinned, requested by anyone and delivered by anyone, and a winner who posts their own Bitcoin address.

The only thing this document adds to the contracts is **the treasury's ability to put its ETH into a vault at
all**. Everything after that already exists.

### 2.1 The snapshot, and every exclusion

The snapshot is of protocol-token holders, weighted by balance, at a named block. Excluded, and named here so
that a third party rebuilding the root can check the list rather than take it on trust:

| Excluded | Why |
|---|---|
| the zero address and `0x…dead` | not holders |
| the token's own contract | not a holder |
| the token's bonding curve | Pons's, holding unsold supply |
| Pons's factory, meme hook and fee escrow | protocol plumbing |
| the token's Uniswap v4 pool and position manager | liquidity, not a holder |
| the `Treasury` contract | it would otherwise win its own giveaways |
| the token's own `FeeRouter` | in transit |
| every `AssetVault` and the `Distributor` | in transit |
| the `Launcher` and the `ListingRegistry` | protocol plumbing |
| any address the operator names as the team's | §6 open question (d); whatever is held must be excluded |

`tools/snapshot` derives the first nine itself from the chain and the registry, so that a checker needs to supply
only the team's list. A snapshot with no eligible holder produces no root and no draw: nothing is published and
the asset waits for the next one.

### 2.2 Inscriptions are drawn; runes are shared

Unchanged from a coin's own acquisitions, and for the same reason: an inscription cannot be divided, so it goes to
one holder by a draw weighted by balance; a rune is fungible, so it is paid **pro rata** by Merkle claim with the
posted minimum and the roll-over that already exist. A giveaway of runes is not a draw at all and has no element
of chance, which §5 notes is legally the safer of the two.

### 2.3 Cadence

The cadence is the operator's (§6 (e)). Whatever it is, it is **not** a promise: a distribution happens when the
treasury has bought something, and the treasury buys when fees have arrived, a cap is posted and a purchase has
been declared, waited out and receipted. The site must describe the cadence as the keeper's intention and show
what has actually happened, never as a schedule holders are owed.

## 3. What this gives the owner, stated as a power

Two new powers, both worth naming in `docs/trust-model.md`:

- **The keeper chooses which collection the treasury buys.** It cannot choose a recipient, cannot take ETH, and
  cannot spend above the posted cap. It can choose badly, or choose a collection nobody wants.
- **The token's creator chooses each giveaway's mode**, because `Distributor` gives that choice to the creator of
  the coin that paid, and the paying coin is the protocol token. So the owner may hold an asset rather than
  distribute it. That is a real discretion and the site must show it: a lot in `Hold` is visible as held.

## 4. What is deliberately not built

- **No buy-back and no burn.** Both are plausible uses of the treasury and both change the token's economics; they
  are the operator's decision, not a mechanism this document should presume.
- **No staking, no lock-up, no tiers.** Holding is the entry. Anything else is a new surface to audit.
- **No treasury spending on anything but assets.** The treasury cannot pay a person, a service or a bill. If the
  operator wants an operating share, it must be a separate, visible split, decided in §6 (f) and amended in.

## 5. Legal, and why counsel must read this before anything is deployed

`docs/legal/prize-draw-memo.md` already sets out why a random draw of a valuable asset among token holders can be
read as a lottery or a prize promotion, and §5 of that memo raises what a fee-funded distribution invites. **A
protocol token makes every one of those questions harder, not easier**, for three reasons that are specific to it:

1. **Recurring distributions funded by protocol revenue, to holders of a token, is the classic shape of an
   investment contract** in jurisdictions that use one. A memecoin whose fees buy an asset for its own holders is
   a closed loop between one coin and its buyers; a protocol token collects from *every* coin on the platform and
   pays its holders. That is a far stronger argument that holders expect profits from the efforts of others.
2. **Consideration becomes much clearer.** The prize-draw memo notes that buying a memecoin may or may not be
   "consideration" for entry. Buying a token whose stated purpose includes receiving distributions is a weaker
   defence.
3. **We would be the promoter.** With a coin's own draw we are infrastructure; with the protocol's giveaway we
   choose the asset, publish the snapshot, run the draw and benefit from the token. Nobody else is in the frame.

Questions to put to counsel, in this order, before any deployment:

1. In our jurisdiction and our expected users', is a token whose holders receive recurring distributions funded by
   protocol revenue a security, a collective investment scheme, or neither — and does the answer change if the
   distributions are discretionary rather than scheduled?
2. Does the pro-rata rune path (no chance) sit differently from the weighted draw (chance), and would running
   only the pro-rata path materially reduce the risk?
3. Does "nothing to enter, holding is the entry" avoid the consideration element where we are likely to be read,
   or does the purchase of the token supply it?
4. What must the site say, and what must it never say, for the disclosure to be adequate — specifically about
   returns, cadence and the treasury?
5. Which jurisdictions should be geo-blocked, accepting that a front-end block is cosmetic and the contracts are
   open to anyone?
6. Does our holding any of the supply, or selling any of it, change the answers to 1–5?
7. If the token is launched on a public bonding curve by anyone rather than issued by us, does that change who the
   promoter is?

Until those are answered, the mechanism may exist in code and on staging, and **must not be deployed to a real
chain**. That is the same rule the rest of the repository already follows.

## 6. Open questions — the operator's, not this document's

Each carries a recommendation and nothing more. None is implemented; the code reads whatever the deployment
provides.

**(a) Total supply.** *Recommendation:* 1,000,000,000 with 18 decimals, the same shape as a Pons launch, so the
token is ordinary and the snapshot's `balance / 10**18` ticket arithmetic is the one already tested.

**(b) How it is distributed.** *Recommendation:* launch it on Pons through our own Launcher, exactly like any
other coin, with no pre-sale and no allocation. It is the only option that requires no new code, makes the
token's own fees flow 80 / 20 like everything else, gives it a public price from the first block, and lets us say
truthfully that we did not sell it. It also makes the promoter question in §5 (7) easier.

**(c) Its own pairing.** A coin launched here must be paired with a listing. *Recommendation:* pair the protocol
token with a single, permanent, well-known collection, so its own 80% buys assets for its own holders like any
other coin, and its 20% joins the treasury with everyone else's.

**(d) Whether the team holds any, and under what lock.** *Recommendation:* if any is held, hold it in one named
address, publish that address before launch, exclude it from every snapshot (this document already requires the
exclusion), and lock it publicly. If the answer is "none", say so and prove it with the launch transaction; it is
the strongest answer available.

**(e) The cadence.** *Recommendation:* no fixed cadence at all at first. Buy when there is enough to buy with, and
show on the site what has happened rather than what is scheduled. A promised cadence is a promise, and §5 (1)
notes that discretionary distributions may sit better than scheduled ones.

**(f) Whether the treasury keeps an operating share.** *Recommendation:* decide it before launch rather than
after, and if there is one, make it a visible constant, not a withdrawal. This document as frozen assumes **no**
operating share: everything the treasury receives goes into vaults. Changing that needs an amendment here, and
would give the treasury a power it currently does not have.

**(g) A creator tax of its own.** Pons lets a launch set 0–10% on top of the base creator share.
*Recommendation:* zero. Any tax on the protocol token's own trading is a cost its holders pay to fund their own
giveaways, which is circular and hard to describe honestly.

## 7. What is built under this document

- `src/Treasury.sol`: receives ETH from every router, funds registry vaults attributed to the protocol token,
  has no withdraw function, and can be paused by the owner.
- `AssetVault.depositFrom(address coin)`: one new external function, callable only by `registry.treasury()`,
  taking the same accounting path as a router's `deposit`. It is the whole of the contract change.
- `tools/snapshot --giveaway`: derives the etched-side exclusions from the registry so that anyone can rebuild a
  giveaway's root, exactly as they can a coin's draw.
- `script/DeployToken.s.sol`: behind the same confirm-string guard as every other script here, and never run.

## 8. How this could mislead

- **A giveaway is not a yield.** A holder who reads the receipts page and annualises what the treasury has bought
  is inventing a number the protocol has not promised. The site must not help them.
- **One winner is not a distribution.** A weighted draw of a single inscription pays exactly one holder. Most
  holders of most giveaways receive nothing. The pro-rata rune path is the only one that pays everybody, and even
  then the posted minimum means small holders accrue rather than receive.
- **The treasury's balance is not the token's backing.** It is ETH that may only ever become assets for the
  holders of the coins that paid it, which after this document includes the protocol token — but no holder can
  ever withdraw it, and it is not a floor under any price.
