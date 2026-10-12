# Rook security review, area D: the buyer canister (`autobuyer/canister-rs`) and the timelock (`autobuyer/timelock-rs`)

Reviewer D of five. Written 2026-09-25. Nothing was committed, pushed, deployed or signed, and no key, wallet or identity
was touched. The only changes to the checkout are two new PoC test files: `autobuyer/canister-rs/tests/audit_d_frogs.rs` and
`autobuyer/canister-rs/tests/audit_d_redeem.rs`. The network calls I made were public, read-only GETs and POSTs
(`eth_blockNumber`, `eth_getBlockByNumber`, `eth_getBalance`, `eth_call`, `eth_getLogs`) to the two C8 RPCs, plus mempool.space and
blockstream.info.

## Scope and commit

- **Checkout:** `/Users/jlohayon/Documents/Rook-audit-D`, detached at `e04f49b2bb94597e0107a607c5bc7ac3d3e3bbce`
  ("Merge branch 'p1-keeper'", 2026-09-25 01:27 -0400).
- **Canister:** `autobuyer/canister-rs`, about 23k lines of Rust in `src/` and `tests/`. Line-by-line, I read the
  Phase-1-reachable paths:
  - `canister.rs`
  - `frognet.rs`, `frogflow.rs` (buys, deposits and returns), `frogbuy.rs` (the judge)
  - `psbt.rs`, `sighash.rs`
  - `custody.rs`, `custodynet.rs`
  - `redeem.rs`, `conflict.rs` (H8/K1), `outbound.rs` (H4), `outflow.rs` (H3), `custodycap.rs` (H2)
  - `pin.rs`, `ordapi.rs`, `ordnet.rs`, `btcnet.rs`, `btcsend.rs` (checkpoint, K2)
  - `deposits.rs`, `handin.rs` and `handinnet.rs` (`submit_offer`)
  - the guards of every `#[ic_cdk::update]` (52 of them)

  I skimmed `oracle.rs`, `rates.rs` and `delivery.rs`. The v2 loop, NFT, sale and runes paths (all OFF in Phase 1) I read only
  for their guards.
- **Timelock:** `autobuyer/timelock-rs/src/{rules.rs, canister.rs}`, all of it.
- **Tests at this commit:** canister **432 pass, 0 fail**, which matches `phase1-scope.md`. With my 4 PoCs it is 436 pass, 0 fail.
  Timelock **8 pass, 0 fail**.

## Methodology (what I used)

1. **Design docs first:**
   - `docs/v6/phase1-scope.md`
   - `docs/deploy/canister-mainnet.md`
   - `docs/v5/custody-hardening.md` (H1–H14, including H8's "resolved before code" note)
   - `docs/v5/custody-threat-model.md` (skimmed)
   - `docs/trust-model.md`
   - `docs/v6/offer-handins.md` §7
   - `docs/v2/pinned-reads.md` (by reference)
   - the C-/K-/G- items that these name
2. **Invariants** written down (below), then line-by-line review, then PoCs against the repository's own fake-world harnesses
   (copied, not modified), then **live read-only measurement** of the providers the Phase-1 install argument names. The tests
   do not reach those providers.
3. **DFINITY Internet Computer security best practices:**
   - [Inter-canister calls](https://docs.internetcomputer.org/guides/security/inter-canister-calls/): state commits at every
     await; a trap after an await rolls back only the callback; journaling and locks via `Drop`; stop before upgrading; outstanding
     callbacks.
   - [DoS prevention](https://docs.internetcomputer.org/guides/security/dos-prevention/): `canister_inspect_message`; "attach
     what the call is expected to need rather than a generous margin, especially where the outcall can be triggered by a user";
     expensive operations triggered by untrusted input.
   - [Security best practices: Denial of service](https://internetcomputer.org/docs/building-apps/security/dos)
   - [Data storage / upgrades](https://docs.internetcomputer.org/building-apps/security/data-storage): pre/post-upgrade hooks,
     instruction limits.
   - [Canister control](https://docs.internetcomputer.org/guides/security/canister-control/)
   - Cycle prices: the formula quoted in `canister-mainnet.md` (DFINITY cycle-costs reference). Each HTTPS outcall costs
     `(3,000,000 + 60,000·n)·n + 400·n` per request byte + `800·n` per **reserved** response byte, where n is the subnet size.
4. **Rust / dependencies:**
   - [RustSec advisory DB](https://rustsec.org) via `cargo audit` 0.x, installed into the scratchpad; 1,271 advisories loaded.
   - The ANSSI Rust secure-coding rules I actually applied: integer overflow in release builds (no `overflow-checks` in
     `[profile.release]`), panics and traps in consensus code, unbounded collections.
5. **Bitcoin:**
   - BIP341 sighash, including which types are signed.
   - BIP125 rules 3, 4 and 6 for the conflict-out.
   - Mempool policy (eviction, pinning).
   - Esplora endpoint limits.
   - Fee sniping and locktime; RBF signalling.
   - Ordinals first-in-first-out sat assignment, inscriptions lost to fees, pointer and reinscription, runes on outputs.

## Invariants the code must keep (checked)

| # | Invariant | Status |
| --- | --- | --- |
| I1 | Custody's key signs only (a) a delivery of exactly one item output, whole, to a finalized ticket's destination; (b) a return of a refused deposit, whole, to its sender; (c) a conflict-out paying only custody. | **Holds.** `outbound::verify_btc` re-checks each against a separately stated purpose (`redeem.rs:436-444`, `frogflow.rs:1326-1334`, `conflict.rs:252-257`). |
| I2 | No sat carrying an inscription or rune is ever spent as a fee: fee inputs, conflict extras and float inputs are ord-clean. | **Holds**, given two verifying providers (`custody.rs fee_eligible`, `conflict.rs pick_extra`, `frogflow.rs:811-813`). |
| I3 | A frog buy lands exactly the named, collection-member inscription on one custody output, with no foreign sats in that range, at most `cap_sats` from the float and at most `fee_bound` to miners. | **Holds** (`frogbuy::judge`). BIP341's commitment to all amounts and scripts closes the "lie in witness_utxo" trick, and only `SIGHASH_DEFAULT` is signed. Seller-side re-witnessing is L-6. |
| I4 | Save the signed bytes, then send; never sign twice for one record. | **Violated** by `give_back` (L-1). Holds for buys, deliveries and conflicts. |
| I5 | Redemption (which cannot be switched off) delivers within the ticket window, or refunds with the item never leaving. | **Violated in practice**: H-1 (never delivers on the C8 pair), H-4 (a dust flood blocks it), H-2 (anyone freezes it), M-1 (the item can leave after a refund). |
| I6 | Only the operator (or a controller) acts, and the pause stops every signing path. | **Holds** (source-scanning tests; I checked all 52 updates). `submit_offer` is intentionally open (L-3). |
| I7 | An upgrade is atomic, keeps state, and cannot jump the caps (×4). | **Caps hold.** State retention fails open (M-2). The limits have no ×4 bound (I-5). |
| I8 | The timelock: no action on the target before `delay`; the argument executed is the one published; veto k-of-g; resume k-of-g. | **Holds** (`rules.rs`). Stale resume votes are I-4. |
| I9 | Cycles: untrusted parties cannot make the canister spend cycles faster than the operator can see and top up. | **Violated** (H-3, L-3). |

## Findings

| ID | Severity | Title | Where | Phase-1 reachable? | PoC |
| --- | --- | --- | --- | --- | --- |
| H-1 | **High** | No delivery can ever be built with the Phase-1 RPC pair: the finalized-pin ticket read and the 8M-block `eth_getLogs` are refused | `custodynet.rs:332-341`, `redeem.rs:94`, `canister.rs:664-682`, `pin.rs:140` | **Yes, always** | Live measurement (below). **Confirmed** |
| H-2 | **High** | Anyone can freeze (pause) all of custody with ~9 junk deposits, and it re-freezes one tick after the guardians resume | `frogflow.rs:1139-1147`, `1281-1315`; `outflow.rs` `check`; `canister.rs:1348-1361` | Yes (deposit handling runs whenever a wrapper is set) | `tests/audit_d_frogs.rs::audit_d_anyone_trips_the_custody_freeze_…` **passes** |
| H-3 | **High** | Permissionless cycles drain: every unfinished deposit is re-read in full every tick (~1.1 MB of reserved outcalls), with no bound on count | `frogflow.rs:978-1037`, `1039-1092`; `custodynet.rs:229-240`; `frognet.rs:269-276` | Yes | `tests/audit_d_frogs.rs::audit_d_pending_deposits_are_re_read_…` **passes**; cycles computed |
| H-4 | **High** | Sending more than 500 dust outputs to the custody address permanently breaks UTXO listing, so no delivery or return can find a fee input | `custodynet.rs:241-255`, `redeem.rs:449-458`, `frogflow.rs:1302` | Yes | Live measurement of both explorers (below). **Confirmed** |
| M-1 | **Medium** | A delivery sitting in a mempool is never re-priced or conflicted; past `deliverBy` the redeemer refunds and then the delivery confirms (`LostRace`, one unbacked unit) | `redeem.rs:376-389`, `custody.rs` `bound_rate` (100 sat/vB clamp) | Yes (needs a fee spike, or a delivery sent late) | `tests/audit_d_redeem.rs` **passes** |
| M-2 | **Medium** | `post_upgrade` treats a stable-memory decode failure as a fresh install: it wipes redemptions and signed bytes, **unpauses**, and resets the H2 book and the H3 limiter | `canister.rs:355-360` | Via any upgrade whose `Saved` type is incompatible | Code-level (no PoC) |
| L-1 | Low | A deposit return is sent before its `Returning` record is saved (breaks save-then-send) | `frogflow.rs:1335-1338` with the caller at `1143`, `1030-1031` | Yes | `tests/audit_d_frogs.rs::audit_d_a_return_is_sent_before_it_is_saved` **passes** |
| L-2 | Low | The timelock upgrades a *running* buyer: no `stop_canister`, and `pre_upgrade` does not refuse mid-tick. Lost continuations leak a cap reservation (`frog:{id}` Duplicate forever) and unsaved return bytes | `timelock-rs/src/canister.rs:165-171`; `frogflow.rs:803-822`; `custodycap.rs:242-245` | On any upgrade | Reasoned |
| L-3 | Low | No `canister_inspect_message` on either canister. `submit_offer` is open in Phase 1 with no board configured. The per-caller rate table never forgets other callers. Ingress spam drains cycles (the timelock has only 2 T) | `handinnet.rs:46-65`, `handin.rs:395-411`, timelock `canister.rs` | Yes | Reasoned |
| L-4 | Low | Lost updates: the tick clones `FROGS`/`FLOORS`/`HANDINS`/`SALES` and writes them back after many awaits, so `refuse_deposit`, `frog_proof` and the controller setters called mid-tick are silently discarded (they return `Ok`) | `frognet.rs:367-375`, `461-488` | Yes | Reasoned |
| L-5 | Low | A `Signed` frog buy has no timeout or abandon: a seller who keeps an unconfirmed conflicting spend alive holds the float inputs and blocks all buying (one purchase at a time) | `frogflow.rs:630-647`, `assign` `started` | Yes | Reasoned |
| L-6 | Low | The seller can re-witness their input after we sign (the same txid, a script-path reveal with an inscription `pointer` into custody's output). Custody then holds more than one inscription, and the frog becomes `Stranded` (paid, not minted) | `frogbuy.rs:270-282`, `frogflow.rs:705-727` | Yes (needs a seller UTXO with a script tree) | Reasoned |
| L-7 | Low | Provider-chosen parameters are taken as max/min with no bound: one explorer sets every custody fee to the 100 sat/vB clamp (`fee_rate` takes the max); one RPC can push the finalized pin arbitrarily deep (`choose_finalized` takes the min, with no check against the head) | `custodynet.rs:256-265`, `pin.rs:140-146` | Yes (a malicious or buggy provider) | Reasoned |
| I-1 | Info | H4's `names_this_item` compares the ticket with itself (fast-pin against finalized acquisition id), not with the outpoint or inscription being spent; `outbound.rs` says otherwise | `redeem.rs:116-117`, `outbound.rs` `TicketFacts` doc | — | — |
| I-2 | Info | `set_unisat_key`, `clear_unisat_key` and `add_ord_provider` are operator-callable and pause-exempt. The operator can drop the only second contents provider (a DoS of buys and deliveries); an added ord can only refuse | `ordnet.rs:204-247` | — | — |
| I-3 | Info | `btc_tx_seen` and every custody read skip the network checkpoint (only `broadcast_btc` checks it); `canister-mainnet.md` says both check it | `btcnet.rs:107-146` | — | — |
| I-4 | Info | Timelock resume votes are cleared only by `pause_target`. After a tripwire or operator pause, one stale vote plus one new vote resumes | `rules.rs:348-372` | — | — |
| I-5 | Info | `outflow_limits` can be raised without limit by one upgrade argument; only the caps have the ×4 rule | `canister.rs:226-238` | — | — |
| I-6 | Info | A redeemer may name the canister's own float or buyer address as the destination (only custody is refused). The frog then becomes unspendable by any path; this harms only the redeemer | `custody.rs` `build`, `outbound.rs` `whole_to` | — | — |
| I-7 | Info | `paste 1.0.15` unmaintained (RUSTSEC-2024-0436), transitive through candid/ic-cdk. No vulnerabilities in either lockfile | `Cargo.lock` | — | — |

Counts: **0 Critical, 4 High, 2 Medium, 7 Low, 7 Info.**

No path I found lets an outsider, the operator, or a malicious offer take an item or the float beyond the per-frog cap. The
custody signing core is well built. The Highs are all about availability of the path that cannot be switched off, and they are
cheap and permissionless.

---

### H-1 (High): no delivery can ever be built with the Phase-1 RPC pair

**Description.** Before any delivery is built, `redeem::owed` calls `ticket_facts` (`custodynet.rs:332-341`). That function
makes two reads, and every provider must answer each one:

1. `eth_call ticket(id)` at `pin_finalized()`: the `finalized` block, rounded down to 1,024, read by its hash;
2. `eth_getLogs` over `[fin − 8,000,000, fin]` (`REDEEMED_LOOKBACK_BLOCKS`, `redeem.rs:94`).

The Phase-1 install argument (`phase1-scope.md` §3.2 C8) names `rpc.mainnet.chain.robinhood.com` and `rpc.ordofi.network`. On
2026-09-25, read-only:

- **Robinhood does not serve state at the finalized block.**
  - Over five minutes, head − finalized was **8,428 at its smallest, right after a finality step, and ~12,000 at its largest**.
    The pin is one bucket lower still.
  - Robinhood serves state about 6,000–7,000 blocks back: `eth_getBalance` at head−6000 answered; at head−7000 it failed.
  - `eth_call` and `eth_getBalance` with `{"blockHash": <finalized bucket>}` returned
    `-32000 "historical state cd9b8f… is not available"`.
  - ordofi answered the same read.
- **ordofi refuses any `eth_getLogs` wider than 10,000 blocks** (bisected: 10,000 blocks answer `[]`; 10,001 blocks, 20k, 100k,
  1M and 8M answer `-32005 "the network is busy, please try again in a moment"`). Robinhood accepted the 8M range.

Either failure makes `ticket_facts` return `Err`, so the redemption waits every tick. Every Phase-1 ticket therefore reaches
`deliverBy` and refunds.

The fast pin (≈1–2k blocks back) is unaffected, which is why the dry runs looked green. The fake worlds answer any read at any
block (`tests/redeem.rs::ticket_facts`), and the doc's measured "Robinhood keeps ≈6,220 blocks" was never compared with the
finalized lag.

**Exploit / impact.** No attacker is needed. From the first redemption, the backing promise (redeem one wX for a frog) is
broken. Holders get wX back after 7 days, so no value is lost, but no frog ever leaves custody. On mainnet this would look
like an outage of the one path the scope calls unstoppable.

**PoC (reproducible, read-only):**

```
u=https://rpc.mainnet.chain.robinhood.com
curl -s -X POST -H 'content-type: application/json' $u --data '[{"jsonrpc":"2.0","id":1,"method":"eth_blockNumber","params":[]},{"jsonrpc":"2.0","id":2,"method":"eth_getBlockByNumber","params":["finalized",false]}]'
# take h = hash of eth_getBlockByNumber(<finalized rounded down to 1024>)
curl … --data '{"jsonrpc":"2.0","id":1,"method":"eth_call","params":[{"to":"0x…01","data":"0x"},{"blockHash":"<h>"}]}'
#  -> {"error":{"code":-32000,"message":"historical state … is not available"}}
curl … https://rpc.ordofi.network --data '{…"eth_getLogs","params":[{"address":"0x…01","fromBlock":"<fin-10000>","toBlock":"<fin>"}]}'
#  -> {"error":{"code":-32005,"message":"the network is busy, please try again in a moment"}}   (fin-9999 -> [])
```

Measured: finalized 72,720,721 and 72,724,502; head 72,731,546–72,735,324; the finalized bucket 72,720,384 was unavailable on
Robinhood and available on ordofi.

**Fix.**
- Read the ticket at the fast pin, and **use finality for the event only**.
- Or use a second archive-class provider: a Nitro node (H10), or two archive endpoints.
- Page `eth_getLogs` in ≤ 9,999-block windows. Better, drop the 9-day lookback: read the ticket's own `redeemedAt` block or emit
  its block in the ticket, and ask for logs in one small window.
- Add a replica or mainnet-fork test that runs `ticket_facts` against the real C8 providers, and make it a go-live gate.
- Re-measure providers against `finalized`, not `latest`.

### H-2 (High): anyone can freeze (pause) all of custody with junk deposits

**Description.** `deposits_pass` runs every tick whenever a wrapper is set. Phase 1 "turns deposits off" only by never
supplying `frog_proof` (`phase1-scope.md` §1). It still does four things:

- it classifies any custody output whose transaction carries an `EWD1`-tagged `OP_RETURN`;
- it refuses a malformed marker (among other reasons) automatically: `Reason::Malformed` is returnable;
- it returns each refused output through `give_back`;
- `give_back` asks the H3 limiter with a distinct key `return:<outpoint>` (`frogflow.rs:1313-1315`).

At stage 0 the per-collection item limit is 3 a day with a trip factor of 3. The **ninth distinct ask in 24 h** therefore trips
`Cause::Tripwire`, and the wiring maps that onto `PAUSED` (`canister.rs:1354-1358`). Deliveries share the same collection key.
While paused, the tick does nothing: no deliveries, conflict-outs, returns or buys.

After k guardians call `resume_target`, `unfreeze` forgets the asks that did not go out. But the refused deposits are still
`Refused` and ask again on the very next tick. 3 done + 7 waiting ≥ 9, so it re-freezes at once. This repeats every day for
as long as refused deposits exist, and the attacker's outputs are even returned to them at custody's expense.

**Exploit.** One Bitcoin transaction with 10 custody outputs, e.g. 2,000 sats each (a plain P2TR output 0 per transaction in
the PoC; a single multi-output transaction works the same way, since each output is its own `DepositRec`), and one `OP_RETURN`
containing `EWD1` with a truncated payload. The input must be a taproot key the attacker owns, so the returns go back to them.
Cost: ~20–25k sats plus fee, most of it returned. Six confirmations later, custody is frozen within 2–3 ticks. It re-freezes
after every resume until an upgrade (≥ 72 h) changes the code.

**PoC.** `tests/audit_d_frogs.rs::audit_d_anyone_trips_the_custody_freeze_with_refused_junk_deposits_and_it_re_trips_after_resume`:

```
AUDIT-D: frozen after 3 ticks; 3 returns sent, 7 refused deposits still asking; cause Some(Tripwire { … flow: Item, demanded: 9, limit: 3 })
AUDIT-D: re-frozen one tick after the guardians' resume: Some(Tripwire { … demanded: 9, limit: 3 })
test … ok
```

**Fix.**
- While deposits are OFF, gate the whole of `deposits_pass` behind a controller-set switch that defaults to off. Note that this
  needs the post-handover setter path §4 already asks for.
- Give returns their own limiter scope (collection key `return:…`), separate from deliveries, so junk can never starve or
  freeze redemptions.
- Do not count a return's asks toward the tripwire, or count them in their own trip.
- Require a minimum deposit value, or an inscription, before a marked output is tracked at all.

### H-3 (High): permissionless cycles drain through deposits

**Description.** A deposit that is not finished is re-checked **in full every tick**. That includes one with a well-formed
marker and any single inscription, which waits forever for the membership proof Phase 1 never gives. `check_deposit` costs,
per deposit per tick:

| Read | Outcalls | Reserved per call |
| --- | --- | --- |
| `tx_view` | 2 | 200 kB |
| `block_hash` | 2 | 2 kB |
| `tip` | 2 | 2 kB |
| `contents` (ord + UniSat, heights before and after, UniSat runes) | ~7 | up to 64 kB |
| `locate` (3 providers × 3 reads) | 9 | up to 16 kB |
| `output` | 4 | 200 kB + 8 kB |
| **Total** | **~26** | **≈ 1.10 MB** |

At the documented prices that is **≈ 12.8 B cycles per deposit per tick on a 13-node subnet** (≈ 34.5 B on the 34-node
fiduciary subnet). The runner ticks every 10 minutes (144 ticks a day), so each deposit costs **≈ 1.8 T a day (≈ 5 T on
fiduciary)**. There is no bound on the number of such deposits: classification adds 8 a tick, 1,152 a day.

- **The cheapest drain:** ~11 junk-inscription deposits (≈ 4 on fiduciary) burn the whole 20 T budget in about a day, at an
  attacker cost of a few dollars per inscription. The operator's only lever in Phase 1 is `refuse_deposit`. That turns each one
  into a return, which then feeds H-2 at 9.
- **Refused deposits** waiting on the limiter cost more: `give_back` reads the transaction, the output, fee rates and
  **custody's whole UTXO list (2 × 400 kB)** before it asks the limiter. That is ≈ 20 B per deposit per tick, capped at 8 by H-2's
  trip.
- **A deposit transaction padded above 200 kB** (padding outputs going back to the attacker) makes `tx_view` fail for its 8
  custody outputs. They are never classified, so they are retried every tick forever: ≈ 34 B per tick.

This goes against DFINITY's DoS guidance: user-triggerable outcalls with generous `max_response_bytes`, and no rate or cost
bound per untrusted input.

**PoC.** `tests/audit_d_frogs.rs::audit_d_pending_deposits_are_re_read_in_full_every_tick_forever`:

```
world reads per tick with 1 pending deposit: [5, 5, 5]
world reads per tick with 8 pending deposits: [35, 33, 33]
4 world reads per pending deposit per tick, never ending
```

Each world read is 2–9 replicated outcalls on the IC; the count above excludes `block_hash` and `tip`. Converting reads to
cycles uses the price formula above (reservation-priced replicated outcalls, as `canister-mainnet.md` assumes).

**Fix.**
- Deposits OFF means not read (the H-2 switch).
- Otherwise:
  - back off exponentially per deposit;
  - check the cheap, local facts first and stop there (marker, proof present) before any outcall; a deposit awaiting a proof
    needs no re-read until a proof arrives;
  - cap the number of tracked deposits;
  - size `max_response_bytes` to the answer. A transform-reduced `/tx/<id>` does not need 200 kB reserved;
    `/tx/<id>/outspend` and `/status` need ≤ 1 kB.
- Add an in-canister cycles floor that skips optional work below, say, 8 T.

### H-4 (High): a dust flood to custody permanently breaks fee selection

**Description.** Every delivery (`redeem::choose_fee`), every return (`give_back`) and every conflict-out that needs a second
input lists custody's UTXOs with `/scripthash/<h>/utxo` on **both** explorers, which must agree (`custodynet.rs:241-255`). Read
live on 2026-09-25, for an address with 5,830 UTXOs:

- **mempool.space** answers `HTTP 400 "Too many unspent transaction outputs (>500). Contact support to raise limits."`;
- **blockstream.info** answers 1.46 MB, about 251 B per UTXO. That exceeds the canister's 400 kB `max_response_bytes` at
  about 1,600 UTXOs.

Anyone can send outputs to the custody address, which is public (it is the listing's `btcVault`). No code path can consolidate
UTXOs without first listing them.

**Exploit.**
1. One transaction with 501 × 330-sat P2TR outputs to custody: ≈ 165k sats of dust plus ≈ 21.5k vB of fee, ≈ 45–110k sats at
   2–5 sat/vB.
2. Every delivery then waits with "…: HTTP 400"; every ticket refunds.
3. This lasts until an upgrade (≥ 72 h) adds a different UTXO source, such as pagination through `/txs/chain` or the
   canister's own index.

(If fewer than 500 UTXOs already exist, fewer are needed.)

**PoC.** Live read-only probe: `curl https://mempool.space/api/address/1BitcoinEaterAddressDontSendf59kuE/utxo` returns
HTTP 400 with the message above. `blockstream.info` answers HTTP 200 with 1,464,318 bytes.

**Fix.**
- Keep custody's spendable fee UTXOs **in the canister's own state**: record the change outputs it creates and plain sats it
  was sent on purpose to a separate fee-float key.
- Pick fee inputs from that set, and verify one outpoint with `output()` and ord, instead of listing the address.
- Pay delivery fees from a **separate fee key (`b"fee"`)**, whose address is not published on chain, not from the public custody
  address.
- Add a sweep of dust.

### M-1 (Medium): a pending delivery can cross its deadline, then confirm after a refund

**Description.**
- `Sent` + `BtcView::Mempool` is "waiting for a block" (`redeem.rs:382`). The rule is not re-run, and there is no RBF or CPFP.
- H8's conflict-out runs only when *every* explorer stops showing the transaction (`NotYet`). An explorer disagreement is
  `Unsure`, which also only waits.
- The fee rate is the 6-block estimate, clamped to ≤ 100 sat/vB (`custody.rs bound_rate`); the send happens > 24 h before
  `deliverBy`.
- `Wrapper.refund` is anyone's once `deliverBy` has passed (`Wrapper.sol:548-551`), and it mints one wX back.

So when fees stay above the delivery's feerate past `deliverBy`, the redeemer refunds and the delivery confirms later:
`LostRace`. The redeemer has the frog **and** the token, and the unit is unbacked forever. The refund also re-queues the
acquisition, so a later ticket on it gets `Stuck`.

Fee spikes above 100 sat/vB lasting days have happened (for example, Dec 2023 and the Apr 2024 halving). Sends late in the
window (the outflow limiter, a UniSat key missing after an upgrade, H-1 fixed late) shorten the needed spike to a little over
24 h.

**PoC.** `tests/audit_d_redeem.rs::audit_d_a_delivery_pending_past_its_deadline_is_never_bumped_or_conflicted_and_ends_a_lost_race`
passes. It runs 1,014 ticks of watching with zero re-signs, then a refund, then a confirmation, ending in `LostRace`.

**Fix.**
- In `Sent`, re-run `ruling` every tick.
- When the time left falls below a threshold (for example 48 h), fee-bump by RBF with the same inputs, same destination and a
  higher fee paid from the change output. Verify the new bytes with H4, save, then send.
- Past the margin, if still unconfirmed, conflict out the fee input even while the transaction is in a mempool. RBF with a higher
  fee does that, since custody's inputs signal replacement.
- Raise or remove the 100 sat/vB clamp for deliveries, bounded by `MAX_DELIVERY_FEE_SATS`.
- Contract side: `refund` could require an extra grace period after `deliverBy`.

### M-2 (Medium): `post_upgrade` fails open into a fresh install

**Description.** `canister.rs:355-360`: if `stable_restore::<(Saved,)>()` returns `Err`, the code runs `init(arg)` and notes
"starts from nothing". The case this was written for (the first upgrade from a build with no `pre_upgrade`) cannot happen on a
fresh mainnet install. What it does now is turn **any** candid incompatibility in an upgrade into a silent wipe. For example, a
new non-`opt` field in `Redemption`, `DepositRec` or `Conflict`, or a changed enum variant payload. The wipe:

- drops `CUSTODY` (redemptions, including `Signed` / `Conflicting` raw bytes) and `FROGS` (buys in flight, their saved bytes,
  deposits);
- **resets `PAUSED` to false**, so a guardian's pause is lifted by an upgrade that fixes something;
- empties the H2 book, so up to 3 more items can be acquired while 3 are held;
- empties the H3 limiter and its freeze;
- resets the ticket cursor to 0, so deliveries are rebuilt with new fee inputs while old signed ones may still be live.

The IC's install is atomic, so trapping would refuse the upgrade and keep everything. DFINITY's upgrade guidance treats the
hooks as the most dangerous code in a canister.

**Fix.**
- `ic_cdk::trap` on a decode error.
- Add a CI test that decodes a saved state from the previous released build (a golden `stable_save` fixture) with the new
  `Saved` type.
- In the longer term, move to `ic-stable-structures`, so there is no `pre_upgrade` serialisation.

### L-1 (Low): a return is sent before it is saved

`give_back` sets `d.phase = Returning{raw,…}` on a local copy, calls `outflow_record`, then awaits `btc_send`
(`frogflow.rs:1335-1338`). The caller puts `d` back into `st` and saves only after `give_back` returns (`frogflow.rs:1143`,
`1030-1031`). The comment "the caller saves the record before the next await" is wrong: the send is that await.

At the send's commit point the durable record still says `Refused`. A trap later in the callback, or an upgrade while the
outcall is outstanding (L-2), loses the bytes, and the next tick signs a different return. It goes to the same destination,
so no item is lost, but the design's never-sign-twice rule (H8 point 6) is broken.

**PoC** `audit_d_a_return_is_sent_before_it_is_saved` passes: "saved phase … at the moment its return was sent: Some("Refused")".

**Fix:** write `d` into `st` and `io.save(st)` inside `give_back` before `btc_send`, as `accept` and `owed` do.

### L-2 (Low): upgrades run against a running buyer

`execute` calls `install_code` in Upgrade mode directly (`timelock-rs/src/canister.rs:165-171`). The buyer's tick awaits dozens
of outcalls. An upgrade landing mid-tick drops the callbacks. DFINITY advises stopping a canister before upgrading when it has
outstanding calls. The concrete leaks:

- `accept` reserves `frog:{id}` in the H2 book before awaiting `sign_float`. If that continuation never runs, the reservation
  stays `in_flight`. Every later `reserve` for that frog returns `Duplicate`, and `assign` eventually marks it `Failed` without
  `settle`. **One of the 3 stage-0 item slots is consumed forever**, with no admin path to clear it.
- L-1's unsaved return bytes.

**Fix:**
- Have the timelock call `stop_canister`, then `install_code`, then `start_canister`. Handle a stop that does not finish (the
  outcalls are bounded, so it will).
- Alternatively, have the buyer's `pre_upgrade` trap while `TICKING` is true.
- Settle stale `in_flight` reservations whose frog is no longer `Wanted`/`Signed`/`Sent`.

### L-3 (Low): ingress cycles drain; `submit_offer` open in Phase 1

- Neither canister implements `canister_inspect_message`. Every ingress update, including those that trap on `only_operator`,
  is paid by the canister. The timelock is blackholed with 2 T cycles, and every one of its updates accepts any caller and
  returns `Err` (still billed). If it freezes, no pause, veto or execute goes through it (the operator can still pause directly).
- `submit_offer` is open to any non-anonymous principal even though no board is set (OFF in Phase 1).
- Its per-caller `Rate` table only prunes the *calling* entry's timestamps. `entries.retain(|(_, ts)| !ts.is_empty())` never
  removes another caller's stale entry (`handin.rs:403-409`), so the table grows by one entry per fresh principal (they are
  free) and every call does a linear scan.

**Fix:**
- Add `inspect_message` on both canisters that accepts only known principals for guarded methods and refuses `submit_offer`
  while no board exists.
- Prune all stale entries.
- Automate top-ups (H13).

### L-4 (Low): lost updates across the tick's awaits

`frognet::tick` clones `FROGS`, `FLOORS`, `HANDINS` and `SALES` (`frognet.rs:367-370`), runs a long async pass that saves the
*clone* at checkpoints, and finally overwrites the globals (`:372-375`). Any change made in between is overwritten:

- operator methods: `refuse_deposit`, `frog_proof` (`:461-488`);
- controller methods: `set_item_config`, `set_floor_sink`, `set_purchase_limit`, `set_offer_board`, `set_sale_policy`.

Each returns `Ok` and is lost. `begin_tick` guards only `tick` and `open_purchase`.

**Fix:** have these methods refuse with "a tick is running" (the `TICKING` check), or queue them and apply them at the tick's
end.

### L-5 (Low): a signed buy can be held forever

`FPhase::Signed` with the transaction not seen re-sends forever. It fails only when the seller's outpoint is spent by a
**confirmed** transaction (`frogflow.rs:630-647`). A seller who double-spends with a low-fee conflict kept in mempools pins the
buy. The float inputs stay committed, and `assign` starts no new purchase while any frog is `acquiring`. That halts graduation
buying, and curves time out into refunds.

**Fix:** a deadline for `Signed`/`Sent`. After it, conflict out a float input back to the float (the same H8 machinery, float
key), settle the reservation, and fail the frog.

### L-6 (Low): the seller can re-witness after we sign

`judge` step 9 checks the seller's witness *in the PSBT*. Once we broadcast, the seller can broadcast the same txid with a
different valid witness for their own input, for example a script-path spend that reveals an inscription whose `pointer` lands
in custody's output range. Only one version of the txid can win, but the seller controls which one reaches miners first.

`custody_holds` then sees two inscriptions and the frog becomes `Stranded`: the price is paid, nothing is minted, and the item
is counted in the book forever.

**Fix:** require the seller's input to be spent through a key path the canister can check, by knowing the output is key-path
only (with BIP86-style proof that there is no script tree, which ordinary wallets cannot give). Or accept the risk and add a
`Stranded` recovery path that is admitted to inventory once ord shows the original frog alone, or that moves the extra
inscription out.

### L-7 (Low): provider-chosen extremes

- `CustodianIo::fee_rate` takes the **maximum** across explorers (`custodynet.rs:256-265`). One lying explorer makes every custody
  spend pay the 100 sat/vB clamp (≈ 21k sats per delivery, under the 25k cap).
- `choose_finalized` takes the **minimum** finalized block with no bound against the agreed head (`pin.rs:140-146`). One lying
  RPC can make the H9 read use an arbitrarily old block. Against a pruned honest peer that is a denial of service. Against two
  archive peers it undoes H9's point (the ticket is read "Resolved" at an old block); the fast-pin rule is then the only check.

**Fix:**
- Use the median, or require agreement within a ratio.
- Require `head − finalized ≤` a sane bound, and require the providers' finalized numbers to be within one bucket of each other,
  as the fast pin already does.

### Info

- **I-1:** `TicketFacts.names_this_item` is `t.acquisition_id == r.acquisition_id`, where both come from the same ticket at two
  pins (`redeem.rs:116`). H4 therefore does not independently tie the spent outpoint to the ticket's item. Either say so in
  `outbound.rs`, or pass the inscription id and have the verifier compare `leafOf`.
- **I-2:** the operator can `clear_unisat_key`, which starves contents verification. Operator-added ords can only refuse
  (every provider must agree, and ordinals.com is always included on mainnet). Acceptable; document it.
- **I-3:** the checkpoint is enforced only on `broadcast_btc`. Reads (`btc_tx_seen`, UTXOs, outputs) trust whatever
  `btc_api_urls` an upgrade names. Low risk (the upgrade is timelocked), but the runbook says otherwise.
- **I-4:** stale resume votes (`rules.rs:358-372`). Clear `resume_votes` whenever a new vote finds the target paused for a new
  reason, or expire votes after N hours. Moot while all guardians sit on device B.
- **I-5:** give outflow limits the same ×4 rule as caps.
- **I-6:** refuse destinations equal to the float or buyer scripts, as custody is refused.
- **I-7 (dependency audit):** `cargo audit` (RustSec DB, 1,271 advisories) on both lockfiles found **0 vulnerabilities**. The
  one warning is RUSTSEC-2024-0436 (`paste` 1.0.15, unmaintained), transitive through `ic-cdk`/`candid`. Versions:
  - canister: `ic-cdk 0.19.0`, `candid 0.10.35`, `k256 0.13.4`, `ecdsa 0.16.9`, `serde_json 1.0.151`, `sha2`/`sha3 0.10.9`;
  - timelock: `ic-cdk 0.19.0`, `candid 0.10.36`.

  `[profile.release]` has no `overflow-checks`. The unchecked sums I found are safe: `judge` steps 7–8 (`values[..s].iter().sum()`,
  `start += o.value`) only ever see amounts that BIP341 forces to be the real ones (< 2.1e15), and `psbt::verify` uses
  `saturating_*`.

## What I tried that held (for the record)

- **`frog_offer`:**
  - asking to sign a foreign input;
  - lying about our own or a foreign input's `witness_utxo` amount or script (BIP341 makes our signature invalid);
  - spending custody;
  - a second seller input;
  - an `OP_RETURN` (runes onto custody);
  - the inscribed sat routed to the fee or to change;
  - foreign sats in custody's range;
  - custody below dust;
  - a spend over the cap or a fee over the bound;
  - a non-member inscription (Merkle proof against the on-chain root);
  - a seller output carrying extra inscriptions or runes.

  All are refused. Only `SIGHASH_DEFAULT` is ever signed for buys and custody (`0x83` only for the OFF listing path).
- **Deliveries:**
  - swapped destinations, custody as destination, a third output, change to a stranger;
  - a fee input carrying an inscription;
  - two deliveries sharing a fee input;
  - the conflict spending the item.

  All are refused by `custody::build`, H4 and `reserved`. H8's fee math satisfies BIP125 rules 3, 4 and 6, and stays under
  25k sats in the worst change-less case at 100 sat/vB.
- **Access control:**
  - every one of the 52 update methods has a caller guard (except the intended `submit_offer`);
  - the rehearsal signers are dead on 4663 and on mainnet;
  - `KNOWN_NETWORKS` + checkpoint (K2) are correct, and the header's proof of work is checked in a test;
  - `restore_init` cannot move the chain, key, network or wrapper;
  - caps cannot jump more than ×4.
- **Timelock:**
  - the delay cannot be bypassed;
  - the argument executed is byte-identical to the published `arg_hex`;
  - only one pending proposal per proposer;
  - epochs stop two upgrades from landing back to back;
  - withdraw is own-only; veto is k-of-g; resume is k-of-g;
  - a proposal is removed before the await (no double execute);
  - a roles change cannot drop below the floors;
  - upgrade only, never reinstall.

## Test gaps

1. **No test touches a real 4663 provider on the delivery path.** `ticket_facts` is always faked, which is how H-1 shipped.
   Add a fork or replica test with the real C8 URLs, and a provider-capability check at install (the finalized state depth, the
   `eth_getLogs` range).
2. No adversarial deposit tests with the H3 limiter on (H-2), and no test counting outcalls or cycles per tick against a hostile
   custody address (H-3, H-4).
3. No test that `Sent` deliveries near `deliverBy` are bumped or conflicted (M-1).
4. No golden-state upgrade test: decode a `Saved` from the last release (M-2). The G8 "10× stage-3 `pre_upgrade`" drill is not
   done, and H13's "bounded state" is not implemented: `redemptions`, `frogs`, `deposits`, `seen` and `unmarked` never prune.
5. No test that a save precedes every `btc_send` / `send_evm`. The frogs fake world asserts it for EVM sends only (L-1).
6. No test of a mid-tick upgrade or lost continuation (L-2), and none of setters racing a tick (L-4).
7. No timeout test for `FPhase::Signed` (L-5).
8. UniSat's parsers are tested only against swagger-shaped fixtures ("UNVERIFIED against a keyed call", `ordapi.rs`). Every
   Phase-1 buy and delivery depends on them agreeing with ord byte for byte.

## Not covered

- The Solidity contracts (areas A–C). I read only `Wrapper.sol` `recordDelivery`/`refund`, for M-1.
- The v2 vault loop, bridge, Seaport/NFT, sale/YOYO and runes purchase paths, beyond their guards and pause checks (all OFF in
  Phase 1).
- `oracle.rs`/`rates.rs` in depth (the rate bounds the float's buy cap, and the sink's `maxItemCost` bounds reimbursement).
- The reproducible build, and actual replica or mainnet behaviour of HTTPS-outcall consensus (for example, how often 13 nodes
  straddle a 1,024-block bucket), cycle numbers under the newer pay-per-use outcall pricing, and the threshold-key subnet itself.
- The ops runner (K4) and offer tool, the keeper (K3) and the monitor (K6).
