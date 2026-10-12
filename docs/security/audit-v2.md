# Third pass: the chain's own specification, and what it cost

2026-09-22. Scope: all of `src/`, including the new `AutoBuyerVault`. Method: `audit-method-v1.md`. This pass was run
against published professional practice rather than intuition, because the two earlier passes had exhausted what intuition
could find. Reproductions are in `test/security/SequencerClock.t.sol` and `test/AutoBuyerVault.t.sol`.

**Read this first.** The most severe finding here was *already in the previous report, rated Low*. The facts had not
changed; the reasoning had. The earlier pass compared the one-hour public wait against sequencer drift it had **measured**
(1.3 s) and concluded the margin was ample. The right comparison is against the drift the platform **permits**, which
Arbitrum documents as one hour — exactly the length of the window. Measuring what a system does is not the same as reading
what it may do, and this codebase had been doing the first and calling it the second.

## Summary

**1 high, 2 medium, 1 low, 3 informational, all new; 4 closed from the earlier pass; 6 Slither detections triaged to zero.**

| # | Sev | Where | One line | Status |
|---|---|---|---|---|
| **S1** | high | every waiting period in the system | The sequencer may stamp a block an hour ahead, and every window a human uses to intervene was an hour or less, on that clock alone | **fixed** |
| **S2** | medium | `AutoBuyerVault.returnFunds` | A stranger could close a purchase by repaying it, erasing a receipt that was about to land | **fixed** |
| **S3** | medium | v2, by design | The buyer signs its own receipt: there is no second opinion on what Bitcoin did | **open — the decision for an auditor** |
| **S4** | medium | `Treasury.fund`, `Distributor.choose` | A registry-supplied vault address was cast to one concrete vault type; it worked only because two structs happened to agree | **fixed** |
| **S5** | low | `AutoBuyerVault.writeOff` | Money that came home after a write-off had nowhere to go | **fixed** |
| **S6** | info | deployment | No monitoring, no published security contact, no incident-response plan | open |
| **S7** | info | `AutoBuyerVault` | `maxInFlight` is immutable, and a lost buyer strands the vault for ever | open, deliberate |
| **S8** | info | `_giveBack` → `_credit` | A refunded coin re-enters the queue at the tail, losing its place | open, cosmetic |

---

## S1 — a delay the sequencer can delete

**Severity: high.** Not theft. The removal of every window in which a person can stop something, available to one party, at
no cost, with no trace beyond the timestamps themselves.

**Where.** `AssetVault.release` / `AutoBuyerVault.pull` (the public hour), `Distributor.cancelDraw`, `requestDraw`,
`cancelDistribution`, `roll` (the fifteen-minute challenge), `Distributor.setDestination` / `recordDelivery` (the hour a
winner has to correct a Bitcoin address).

**The defect.** Chain 4663 is an Arbitrum Nitro L2. Nitro lets the sequencer choose each block's timestamp under two rules:
it may never go backwards, and it must lie between 24 hours behind wall-clock and **one hour ahead of it**. One hour ahead
is not a margin against an hour-long wait; it is the entire wait. Announce a purchase at chain time `T`, and `readyAt` is
`T + 3600`. The sequencer may legally stamp the **very next block** `T + 3600`. The public hour becomes one block.

The fifteen-minute challenge window is worse: it is well inside the tolerance, so it can be closed before anybody has seen
the draw at all. The destination-correction hour fails in the other direction — it *locks early*, and the winner loses the
chance to fix a wrong Bitcoin address before the asset is sent to it.

Arbitrum's own guidance says the quiet part: timing is "reliable in the longer term (i.e., on the order of at least several
hours) but unreliable in the shorter term (minutes)." Every window in this system was in the unreliable band. Every window
longer than the tolerance — the 3-day settle deadline, the 7-day write-off, the registry's 7-day change delay — was always
fine, and still is.

**What an attacker does.** Operates the sequencer, or compromises it. On this chain that is one party. This is not an
unprivileged attack; it is the blast radius of an entity the trust model had not been asked about.

**What it gains them.** With a compromised keeper, the public hour is the only thing standing between a bad cap and the
money. With a compromised buyer, the same. The hour is the design's answer to "what if a key goes wrong", and it was
deletable by a different party than the one the answer was written about.

**The fix, and why `block.number` is the right second clock.** On Nitro `block.number` is **not** the L2 block number: it is
the parent chain's, which the sequencer reports and cannot run ahead of. Verified on 4663 rather than assumed, with an
`eth_call` state override returning the `NUMBER` opcode:

```
block.number (EVM)     = 26,034,810
l1BlockNumber (RPC)    = 26,034,810
L2 head                = 69,869,490
```

Its rate, measured over three spans ending at that block:

| L2 blocks | elapsed | parent blocks | s/parent-block | parent blocks/hour |
|---:|---:|---:|---:|---:|
| 100,000 | 2.79 h | 827 | 12.14 | 296.6 |
| 1,000,000 | 27.97 h | 8,323 | 12.10 | 297.6 |
| 5,000,000 | 139.90 h | 41,758 | 12.06 | 298.5 |

`src/lib/Wait.sol` takes 12 s per parent block — rounded **down** from the measurement, so a period's block count is an
over-estimate and every wait errs long — and every window at or under the tolerance now requires both clocks to agree.

A sequencer that *stalls* `block.number` makes waits longer, which is the safe direction but is still a liveness lever it
holds; force-inclusion through the delayed inbox bounds it at 24 hours. A parent chain that permanently speeds up would
weaken the block half and hand the binding constraint back to the timestamp: the rate must be re-measured before mainnet and
whenever the parent chain changes its block time. Both are written into the library.

**What this also exposed.** Every test in the repository advanced time with `vm.warp` alone — one of the two clocks the
contracts now read, which is a chain that does not exist. `test/utils/Clock.sol` advances both, and
`Clock.skipTimeOnly` exists to move only the sequencer's, which is exactly what a sequencer can do, and is what the S1
reproductions use.

---

## S2 — a stranger could close a purchase by paying for it

**Severity: medium.** No theft; the attacker pays. Permanent denial of one acquisition, for a price.

**Where.** `AutoBuyerVault.returnFunds`, as first written in the v2 rework.

**The defect.** The permissionless refund is deliberate and correct: making the coins whole must need nobody's key. But the
first version also *closed* the purchase when the returned total reached `maxEth`, whoever sent it. So anyone could spend
`maxEth` to race a receipt — the buyer really bought the asset on Bitcoin, a stranger repays the vault, the purchase closes
as `Returned`, `settleInscription` then reverts with `WrongStatus`, and the acquisition is never recorded. An unrecorded
acquisition can never be distributed, so the asset sits at the listing's Bitcoin address belonging to nobody in particular.

Note the shape: this is a defect the *rework introduced*. v1's `returnUnspent` never closed a purchase, so v1 never had it.
A finding against new code, found by auditing the new code rather than assuming the rework inherited v1's properties.

**The fix.** Anyone may still repay in full; only the buyer or the owner closes a purchase by doing so. A fully-repaid
purchase left open settles as normal, or goes overdue and writes off the zero still outstanding.

---

## S3 — the buyer signs its own receipt

**Severity: medium, and it is a design decision, not a bug.** Recorded here because it is the question an external auditor
should be handed first.

**Where.** `AutoBuyerVault.settleInscription` / `settleRune` / `settleShort`, all `onlyBuyer`.

**The position.** v1 separated the key that spends from the key that vouches: the purchaser received ETH, a different
attestor signed the receipt. v2 collapses them, on the argument that the buyer is a program whose code is public and whose
key is held by a subnet rather than a person.

What the contract still checks is real, and it is what this chain can check: an inscription must be in the listing's
committed Merkle collection, must not already be held, and a short delivery cannot close until the money for what never
arrived is back in the vault. What it cannot check is the part that matters most — that the Bitcoin transaction exists and
paid the listing's public address. That is checkable by any person against Bitcoin and by nothing on chain.

The honest sentence is that a bug in the buyer is as good as a lie from it, and the bound is `maxInFlight`.

**The options, for whoever decides.** Retain the attestor with a dispute window that reopens a purchase; a challenge period
before an acquisition becomes distributable; or an on-chain read of Bitcoin (ICP's Bitcoin canister gives this on mainnet,
and is consensus-grade — `docs/v2/state-machine.md` already records that the testnet branch's HTTPS-outcall read is weaker).
None is implemented. The cheapest is the first.

---

## S4 — a vault address cast to a vault type

**Severity: medium.** Latent. It worked, and the reason it worked is not a reason to keep it.

**Where.** `Treasury.fund` (`AssetVault(vault).depositFrom{value}(token_)`) and `Distributor.choose`
(`vault = AssetVault(registry.vaultOf(listingId))`).

**The defect.** Both take an address from the registry and cast it to one concrete vault type. Once a listing can be v2,
that address may be an `AutoBuyerVault`, and the calls resolved only because the two contracts happened to declare
`depositFrom(address)` with the same selector and `Acquisition` with the same field order. Nothing enforced either. The
first time a field is added to one struct and not the other, the Distributor decodes a delivery record from the wrong
offsets and nothing reverts.

Found by building the access-control matrix that `audit-method-v1.md` category 3 asks for — enumerating who may call every
external function meant enumerating the calls, which is where the cast was sitting.

**The fix.** `IEtchVault` now declares the whole shared surface — `deposit`, `depositFrom`, `acquisition`,
`acquisitionCount`, `isRune` — and owns the `Acquisition` struct, so there is one type rather than two that agree.
`test_theTreasuryAndTheDistributorBothWorkAgainstAV2Vault` drives both, end to end, against v2 vaults of each kind.

---

## S5 — money that came home after a write-off had nowhere to go

**Severity: low.** Recoverable-looking funds that were not recoverable.

Neither vault has a `receive()`, by design: ETH enters only through `deposit`, `depositFrom` and a purchase's own refund.
After a write-off, `returnFunds` reverted with `WrongStatus`, so a bridge that paid out late, or a venue that refunded after
the deadline, left the buyer holding ETH it had no way to give back. Now a late refund is accepted and un-writes exactly
that much of the recorded loss — `totalWrittenOff` should say what was actually lost, not what was feared.

---

## Informational

- **S6. No monitoring, no security contact, no incident response.** Trail of Bits lists all three as deployment
  prerequisites: contract monitoring, an entry in the blockchain-security-contacts registry, and a written plan that assumes
  a privileged key is compromised. This codebase emits events for every critical operation — the raw material is there — and
  nothing consumes them. This is the weakest category in the maturity rubric and the cheapest to fix.
- **S7. `maxInFlight` is immutable and `buyer` is immutable.** Both are the safe side of their trade and both have a cost:
  a listing that outgrows its ceiling needs a new vault, and a buyer that is lost — a canister deleted, a subnet gone —
  strands whatever the vault holds, for ever, because there is no withdraw. Stated rather than hedged, in `docs/v2/vault-v2.md`.
- **S8. A refunded coin re-enters the queue at the tail.** `_giveBack` credits through `_credit`, so a coin that was drained
  to zero and dequeued loses its FIFO position when its money comes back. Cosmetic; v1 behaves the same way.

## Slither 0.11.6 — six detectors, zero findings

Run over `src/` with dependencies, tests and scripts filtered out. Every detection was inspected; all six are false
positives, which is the expected result for a codebase with no assembly, no `delegatecall`, no upgrade path and no external
token flows. Recorded because a clean static-analysis run is routinely mistaken for a clean audit.

| Detector | What it flagged | Why it is not a finding |
|---|---|---|
| `arbitrary-send-eth` | `FeeRouter.harvest` sending to `vault` and `registry.treasury()` | Both addresses come from the registry, neither from the caller. That is the property, not the bug. |
| `reentrancy-eth` | `harvest` writes `treasuryOwed`/`_entered` after `vault.deposit{value}` | `_entered` is set before any call and blocks re-entry; `treasuryOwed` is zeroed before the treasury call, and only `harvest` reads it. |
| `reentrancy-no-eth` | `Distributor.resolve` writes after `beacon.prove` | `beacon` is immutable and is `DrandEvmnet`, which makes no external calls — only pairing precompiles. If the beacon were ever a different contract this would be real; `DeployChecks` is what keeps it from being. |
| `incorrect-equality` | `b == take`, `_open[i] == id`, `amount == 0` | Exact equality on values the same function computed. Intended in each case. |
| `uninitialized-local` | `given`, `payers`, `best`, `bestGap`, `claimed` | Solidity zero-initialises. Each is an accumulator that must start at zero. |
| `uninitialized-state` | `_charges` mapping | A mapping needs no initialiser. |

## Still open from the second pass

Closed since: **C1** (critical), **H1**, **H2**, **M1**; **M4** and **M6** do not exist in v2, which has no bond.
Still open and unchanged: **M2** (the keeper can block rune payouts), **M3** (change below `MIN_ENTRY` is stranded),
**M5** (a lot pinned to a dead beacon), **M7** (a declaration never expires), **M8** (a queue slot costs a refundable
donation), **M9** (`redraw` is a free selective abort), **L1–L9**, **I1–I5**. **L6 is superseded by S1 above** — it was the
same defect, correctly located and wrongly rated.

And one that is not a contract finding but blocks the whole v2 flow, from the parallel rehearsal work: **the buyer canister
cannot broadcast an EVM transaction at all.** Its `rpc()` demands identical responses from every provider, and an ICP
replicated outcall runs on every node, so `eth_sendRawTransaction` returns a hash to one node and "already known" to the
rest — consensus fails. `announce`, `pull`, `settle` and `returnFunds` therefore have no path to chain 4663 yet, no matter
what this contract accepts. See `docs/v2/2026-09-22-evm-and-runes-rehearsals.md`.
