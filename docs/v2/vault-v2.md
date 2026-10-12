# The v2 vault: what changed, and what it costs

Written 2026-09-22, after `src/AutoBuyerVault.sol`. It records what v2 removes from v1, what it keeps, what the removal buys,
and — the part a design document usually leaves out — what each change makes *worse*. Nothing here is deployed or audited.

## The one-line difference

v1 releases ETH to a **person the registry names**, against a **bond** they posted, and a **second person** vouches for the
Bitcoin that came back. v2 releases it to **one address fixed when the vault is built**, which is the EVM address of the
buyer canister in `autonomous-buyer.md`, and that program posts its own receipt.

## What v2 keeps, unchanged

Everything about the coins. `deposit`/`depositFrom` and who may call them, the MIN_ENTRY queue and why it exists, the
proportional charge list, the price cap and its hour, the public hour before money moves, the Merkle membership check on an
inscription receipt, `_requireUnitPrice`, the seven-day public write-off, and the absence of any withdraw function. If a
holder's question is "what happens to my coin's fees", the answer is the same sentence in both versions.

## What v2 removes, and what that buys

| Removed | Why it existed | What replaces it |
|---|---|---|
| the bond (`bond`, `bondOf`, `requiredBondOf`, slashing) | a person could keep a release | nothing can be slashed from a subnet key. `maxInFlight` bounds what one bug costs; `MAX_OPEN` bounds how many at once |
| the `purchaser` role | somebody had to receive the ETH | `buyer`, immutable per vault |
| the `attestor` role | whoever spends must not vouch for the spending | the buyer posts its own receipt, and the contract checks the two things it can: collection membership, and not-already-held |
| `reserved` | declarations promised wei without naming payers | `inFlight`, and the charge list is fixed at `announce` |

Three findings from `docs/security/internal-review-v1.md` close as a consequence, and are not patched twice:

- **H1** — v1 charged the coins at `release`, an hour after the declaration, so any caller could pick which of several matured
  declarations ate the head of the queue; the loser could need more than `MAX_PAYERS` coins, permanently, because the queue
  only moves forward. v2 charges at `announce`. Each purchase already owns the coins that will pay for it, so there is no
  ordering left to exploit, and `TooManyPayers` can only be hit at the moment the buyer could simply announce less.
- **M4, M6** — both are about topping up or reducing a bond that no longer exists.

Two more are fixed in the registry, because they were never about the vault:

- **H2** — `renounceOwnership()` reverts. The pause stays instant, which is right; what is gone is the second half of the
  one-way door, where a paused registry with no owner could never be unpaused and no vault has a withdraw.
- **M1** — the keeper may not be rotated onto the purchaser, and no role may be rotated onto an address some vault already
  treats as its buyer. A buyer can already pull ETH to itself, so it must never also hold the key that bounds the pulling or
  the key that stops it.

## What v2 makes worse, stated plainly

1. **The receipt has no second signature.** In v1 the attestor was a separate key whose job was to look at Bitcoin and say
   what it saw; a false receipt needed two parties or one compromised attestor. In v2 the buyer signs its own. The contract
   still refuses an inscription outside the committed collection, refuses one already held, and refuses to close a short
   delivery until the money for what never arrived is back — but *that the Bitcoin transaction is real, and paid the listing's
   public address*, is checkable by anyone against Bitcoin and by nothing on chain. The argument for accepting this is that
   the buyer's code is public and its key is nobody's; the argument against is that it is still one actor's word, and a bug
   is as good as a lie. **This is the open question for the audit.**

2. **A lost buyer strands the vault for ever.** `buyer` is immutable precisely so that no key can redirect the money — which
   means a canister that is deleted, or a subnet that stops, leaves whatever is in that vault unreachable, because there is no
   withdraw by design. v1 could rotate the purchaser. The trade is "no lever anyone can pull" against "no lever anyone can
   pull". It is a real cost and it is deliberate.

3. **The buyer can freeze its own vault by announcing and stopping.** An announcement holds a `MAX_OPEN` slot and its share of
   `maxInFlight` until somebody clears it. The owner could always clear it; now, after `PUBLIC_CANCEL_AFTER` (7 days past the
   ready hour), anyone can, because nothing has left the vault at that point. That closes the two-key version of the problem,
   not the seven-day version.

## What still trusts a person

- The **keeper** posts the price cap. It is a bound, never a destination, it expires in an hour, and the wrong number costs an
  overpay up to the cap — never custody.
- The **owner** may pause a listing and write a purchase off. It may never receive or redirect anything, and seven days after
  a deadline anyone may write off without it.
- The **owner** names the buyer, once per vault, in public, at listing time.

## Open, for the audit

- The unsigned receipt (1 above): is a program's self-receipt acceptable, and if not, what is the cheapest second opinion —
  a retained attestor with a dispute window, a challenge period, or an on-chain Bitcoin light-client read?
- `maxInFlight` is immutable. Correct for safety, awkward for growth: a listing that outgrows its ceiling needs a new vault.
- The keeper-posted cap is still the trust the whole spend bound rests on, and it belongs in `trust-model.md`.
