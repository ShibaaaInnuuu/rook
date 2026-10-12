# Three trust points, closed: the operator's custody, the cycles nobody could read, and the keys that could freeze the buyer

Written 2026-09-23, before its code. It answers three rows of `docs/audit/README.md`'s known issues and the "Blocked today" 3 of
`docs/deploy/canister-mainnet.md`, and it changes `docs/deploy/key-ceremony.md`'s table of lost and stolen keys.

## 1. The operator key was a custody key

**What was true.** Three inputs that decide where a purchase's Bitcoin goes came from the operator alone:

- `open_purchase(amount, rune, units, recipient_script, cap_ref)` — the order, including the script the asset lands on;
- `record_quote(id, quote)` — the quote a PSBT is checked against, including `recipient_script`, `our_script` and
  `max_spend_sats`;
- `sign_purchase(id, psbt, input)` — signs any PSBT that satisfies that quote.

So a stolen operator key could open a purchase, record a quote naming its own script and a spend as large as the buyer's balance,
and have the canister sign it. `psbt.rs` checked the PSBT against the quote faithfully; the quote was the hole.

**What the chain already records.** The buyer does not decide what a purchase is for; the chain does:

- the buyer's vault (`Init.vault`) is an `AutoBuyerVault` with an immutable `registry` and `listingId`;
- `registry.listing(listingId)` gives the listing's `kind` (inscriptions or rune), `ref` (for a rune, its id `block:tx`), `vault`
  (which must be ours) and **`btcVault`** — the Bitcoin address the listing's assets land on and receipts are checked against,
  which the owner can change only through a 7-day public delay (`CHANGE_DELAY`);
- the vault's `purchase(id)` gives what was announced: `maxEth` (what the vault released) and `quantity` (the units, in the rune's
  smallest unit).

**The change.** Every quote, however it arrives, is checked against those facts, read replicated from the chain at the moment it is
recorded, and again at the moment it is signed:

| Field | Must equal / be bounded by | Source |
| --- | --- | --- |
| listing | a **rune** listing, not paused, whose `vault` is this buyer's vault | `registry.listing(vault.listingId())` |
| `rune` | the listing's `ref`, parsed | the listing |
| `recipient_script` | the scriptPubKey of the listing's `btcVault`, decoded for the installed network (`tb1…` on signet/testnet4, `bc1…` on mainnet) | the listing |
| `our_script` | the canister's own taproot script | the canister's key |
| `units` | at least the vault purchase's `quantity` | `vault.purchase(id)` |
| `max_spend_sats` | at most **the reference value of the ETH the vault released for this purchase** (`maxEth − returned`), plus `rates::TOLERANCE_BPS` = 300 (3%) for fees and spread | the reference rate (below) |
| `max_fee_sats` | at most `max_spend_sats` | — |
| vault purchase | status `Pulled`: the ETH has left the vault for this purchase, and nothing else | `vault.purchase(id)` |

The order at `open_purchase` is checked the same way before anything is announced: its `rune` and `recipient_script` must be the
listing's. The operator no longer types a recipient at all in effect — a recipient that is not the listing's is refused.

**One listing read, one rate path.** The listing is read through `looper::read_listing` — the same read the purchase policy's
snapshot makes, decoded by `policy::ret_listing`, its address by `policy::script_of` — so the address a purchase is opened for and
the address a quote must pay come from the same place. The cap is `Io::reference_cap_sats`, implemented by `oracle::reference_cap`:
the same live `read_rate` (a median of at least three keyless tickers and XRC, at most 1% apart) and the same 3% as the bridge
judge's `reference_floor`, the other way (`rates::cap_sats` beside `rates::floor_sats`). No rate, no quote: fail closed.

**Rehearsal purchases.** `rehearse_open_purchase` exists only on a canister installed for signet (an upgrade cannot change the
network), has no order and no vault behind it, and `sighash-e2e.mjs` records a hand-written quote on one. Those keep the unchecked
path, and only on signet: on a canister installed for any other network there are no rehearsal purchases to record a quote for.

**What power the operator still has, exactly:**

- **when** to open a purchase and **how much** to buy — bounded by the keeper's price cap, `maxInFlight` and `MAX_OPEN`, which the
  vault enforces at `announce`;
- **which honest quote**: any quote paying the listing's address, for at least the announced units, costing at most the reference
  value of the released ETH plus 3%. The worst an operator colluding with a venue can take is that 3%, plus overpaying within the
  cap, which `psbt.rs` already names as the residual risk;
- **refusing to act**, and **abandoning** a purchase before any BTC is spent (its ETH goes back to the vault);
- **pausing** the buyer (not resuming it).

It can no longer send a purchase's Bitcoin anywhere but the listing's published address.

**What this does not close.** The listing's `btcVault` is the owner's (the Safe's) choice, behind a 7-day delay — the operator
cannot change it, but the Safe can, publicly. A reference rate that is wrong by more than 3% moves the cap with it; that is the rate
module's problem to bound, and it must never be the venue's number.

## 2. Nobody could read the buyer's cycles after the handover

`canister_status` answers only controllers, and after the handover the buyer's only controller is the timelock. So:

- the buyer gets a public query `cycles() -> { balance, low_threshold }`; the timelock gets the same;
- the thresholds are constants in the code, so the watcher reads them rather than guessing: **buyer 5T** (about two days of a
  one-read-a-minute loop on a 13-node subnet, `docs/deploy/canister-mainnet.md` "Cycles"), **timelock 0.5T** (it spends almost
  nothing; it must only never freeze);
- `scripts/timelock-watch.mjs` asks both on every poll and posts once when either falls below its threshold (and once when it
  recovers), beside its proposal alerts.

Anyone can top up either canister, blackholed or not.

## 3. A lost or stolen timelock key could freeze the buyer for good

**What was true.** One proposer, one guardian, fixed forever (the timelock is blackholed). A stolen proposer could propose forever
(the guardian vetoing forever, every honest upgrade blocked); a stolen guardian could veto forever; a lost proposer meant no upgrade
ever again.

**The change: sets, a veto threshold, and roles that change like code.**

- `proposers`: a set (1–5). Any one of them may propose. Each has **at most one pending proposal**, so one stolen proposer cannot
  occupy the only slot, and may **withdraw only its own**.
- `guardians`: a set (1–5), with `veto_threshold` = k. A proposal is cancelled when **k distinct guardians** have voted to veto it.
  Any single guardian may **pause** the buyer (stopping is always safe); **resuming** takes k guardians' votes.
- A third kind of proposal, **`Roles { proposers, guardians, veto_threshold }`**, replaces the sets. It waits the same delay (72-hour
  floor) and can be vetoed by k guardians like any other. The target is untouched by it.
- **Stale proposals fail.** Every proposal records the target epoch (how many target actions have executed) and the roles epoch
  when made. An upgrade or controller change executes only if no other target action has executed since it was proposed; a roles
  change only if the roles have not changed since. Otherwise it fails, is recorded, and is cleared — so two pending upgrades cannot
  land one after the other and silently undo each other, and a roles change cannot be made on an out-of-date picture.
- **Production rules** (refused at install and in any `Roles` proposal; the `rehearsal` build relaxes them as it relaxes the floor):
  at least 2 proposers; at least 3 guardians; `2 ≤ k ≤ guardians − 1`; no principal is both a proposer and a guardian; nobody is
  anonymous. The 72-hour floor is unchanged.

**What each key's loss or theft now means:**

| Key | Stolen | Lost |
| --- | --- | --- |
| one proposer | It can propose; each of its proposals needs k guardians to veto (the watcher posts it). It cannot withdraw anyone else's proposal, and its single slot cannot block the others'. An honest proposer proposes `Roles` without it; the stolen key cannot cancel that; it lands after the delay. **Bounded by one delay.** | The other proposers carry on; one of them proposes `Roles` replacing it. |
| one guardian | It cannot veto alone (k ≥ 2). It can pause the buyer, repeatedly — a denial of service, not a theft — until a `Roles` proposal removes it, which it cannot veto alone. **Bounded by one delay.** | The veto still needs only k of the remaining guardians (k ≤ guardians − 1). A `Roles` proposal replaces it. |
| k guardians together | They can veto every proposal, including the `Roles` proposal that would remove them, and keep the buyer paused: the buyer's code is frozen as it is and it may be stopped. They still cannot move funds or change code. | — |
| every proposer | — | No upgrade and no roles change, ever: the buyer runs as it is. Hence at least 2 proposers, stored apart. |
| a proposer and k guardians together | The design's limit: they can change anything, with a public delay. | — |

**Why not smaller.** A single guardian with a veto (today) is one key that can freeze the buyer. A 2-of-3 veto costs the honest side
one more person reachable within the delay, and buys: no single key, lost or stolen, can freeze or redirect anything for longer than
one delay. **Why not guardians proposing.** Letting guardians propose a roles change would recover from losing every proposer, but it
would make k guardians able to install a proposer of their choosing — a veto set that can become a proposer set is no longer a veto.
The recovery for "every proposer lost" is custody: two or more, stored apart.

## Tests, and the rehearsal

- The quote and order checks are pure (`src/listing.rs`) and tested against a listing encoded by `cast abi-encode`, the BIP-173/350
  address vectors, and the rehearsal's own funded address; the loop's fakes serve listing facts and a fixture rate.
- The timelock's rules are pure (`rules.rs`) and tested for every row of the table above.
- On the local replica, with two NEW canisters of this branch's own (`autobuyer_trust`, `timelock_trust`) and three local test
  identities (never a real key): install with sets; a single guardian's veto is not enough and two are; one proposer cannot withdraw
  another's proposal; a `Roles` proposal removes a proposer after the delay and the removed one can no longer propose; a stale upgrade
  fails; `cycles()` answers on both.

## Results (2026-09-23)

**§1, the operator.** `src/listing.rs` holds the checks; `looper::check_quote_on_chain` gathers the facts and runs them where a
quote is recorded (the loop's `GetQuote` step and `record_quote`) and again before signing (the loop's `RequestAndVerifyPsbt` step
and `sign_purchase`); `open_purchase` checks the order. Only those two places record a quote at all; the relay handoffs
(`relay_venue_quote`, `relay_venue_psbt`) pass the venue's quote to the loop, which checks it. Tested: the listing decoded from a
`cast abi-encode` fixture; the BIP-173/350 vectors and the rehearsal's funded signet address through `policy::script_of` (which had
no tests of its own); each of the operator's old powers refused one field at a time; and in `tests/loop.rs`, a quote paying anything
but the listing's address is never recorded, a spend over the reference cap is never recorded, and a listing paused after the quote
signs nothing (and signs once unpaused). Canister: 147 unit, 2 end-to-end, 18 loop and 3 policy tests pass; the wasm builds clean.

**§2, cycles.** `cycles()` on both canisters; the watcher alerts once below each canister's own threshold and once on recovery.
Its live test found `dfx`'s JSON writing a `nat` with digit separators (`3_500_000_000_000`), which `BigInt` refuses — fixed. On
the local replica the watcher reported the rehearsal buyer at 3.50T, under its 5T threshold, once, and not again on the next poll.

**§3, the timelock.** `rules.rs` rewritten: 8 tests, in both builds. `rehearse-roles.sh` on two new canisters (buyer
`i3ojt-f7777-77777-aaaka-cai`, timelock `i4pph-ih777-77777-aaakq-cai`) with six local test identities and production-shaped roles —
**27 checks pass**: an install with a veto threshold above its guardians refused; the handover; `cycles()` for anyone on both; a
guardian cannot propose; one proposer cannot withdraw another's proposal; one guardian's veto is not enough, the same guardian twice
is one vote, a second vetoes it; an upgrade executed by the anonymous principal moves the module hash, and the controller change
proposed beside it before it ran fails as stale; a roles change removing a proposer cannot be withdrawn by it nor vetoed by one
guardian, runs after the delay, and the removed key can then propose nothing while the new one can; a proposer cannot pause; one
guardian pauses and cannot resume alone; two resume. `rehearse.sh` (#24) ported to the new interface passes every one of its
checks again, including the failed upgrade's atomicity and the pause surviving the real one. The watcher's tests, live on the
replica, pass (5).

**Left open.** The rate is read live on every check: a quote step and a signing each make the six outcalls and one XRC call of
`read_rate` — a cycles cost, and a check that fails closed if fewer than three sources answer. `k` guardians colluding can still
freeze the code, and losing every proposer ends upgrades; both are stated, not solved.
