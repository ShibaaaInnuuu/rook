# A5: canister flows on a Solana home

| | |
|---|---|
| Area | A5: the buying and delivery flows of `autobuyer/canister-rs` on a Solana home: `solhome`, `ethflow::home_sol` and every `sol_home()` branch (ethflow, ethnet, ethnft, dormancy), `frogsol` and the Solana branches of frogflow/convflow, the Solana paths of redeem/custodynet/delivery, `relaysol`, `relayeth::judge_to_solana`, rates/oracle (SOL/ETH, SOL/BTC), custodycap `sol:` keys, the upgrade guards, `assert_home`, late payouts, the cost cuts of f2c24e0 |
| Baseline | `f2c24e0` on branch `golive-2026-10-08` |
| Date | 2026-10-10 |
| Out of scope | The Solana primitives (solkey, soltx, solsend, solrpc, solread, solix: A4) are taken as given, and only their misuse is noted. The EVM home was audited earlier: findings that apply to it equally are flagged "inherited". |
| PoC | `autobuyer/canister-rs/tests/audit_a5_sol_flows.rs`. Each test asserts the correct behaviour, so it **fails** on the baseline. |
| Result | 0 Critical, 0 High, 4 Medium, 5 Low, 6 Informational |

## 1. Threat model

**Assets**
- The canister's SOL at its threshold-Ed25519 address. One address holds every flow's money: Ethereum-side advances, frog advances, refunds, reimbursements, gas keeps and the nonce rent.
- Its ETH on Ethereum: arrivals earmarked to buys, and leftovers.
- The BTC float, and the inscriptions in custody.
- The ERC-721s held for Ethereum NFT collections.
- Holders' redemptions: an Ethereum delivery, a taproot delivery, or a burn.

**Actors and what each controls**

| Actor | Controls | Bounded by |
|---|---|---|
| Relay quote / solver | The quote JSON, fill timing, refunds (plain transfers with an unauthenticated memo) | Judges S1–S12 / rules 1–12, the order-id recomputation, the canister's own instruction, the balance rules |
| Relay depository owner `6Bz8…` | Upgrades the depository program, rotates the allocator | S3 (ProgramData slot and authority, allocator), read at judgement only |
| Lagging or malicious RPC | Account reads, transaction reads, balances | ≥ 2 agreeing providers at finalized (A4), the genesis check |
| Ticker source / XRC | One SOL/USD, ETH/USD or BTC/USD leg | ≥ 3 sources, spread ≤ 100 bps, lower median |
| Holder | Ticket destination (Eth, taproot, burn, or custody), timing of redemptions | rook_core's checks (A2), `delivery::decide`, H4 |
| Seller (Seaport / ord) | Listings, PSBTs | Unchanged judges (EVM audit) |
| Operator / controller | Hand-ins (quotes, payouts), `sol_clear_incident`, `eth_give_back`, `eth_late_release`, upgrade arguments | `only_operator`, the timelock on upgrades |
| Keeper | Posts the Inscriptions floor when the registry names it | `live_poster` |
| Anyone | Sends SOL to the canister's address with any memo | Balance rule ≥ 50 % of the deposit |

## 2. Findings

| ID | Title | Severity | PoC |
|---|---|---|---|
| A5-01 | A written-off order's late payout is judged in the wrong currency on the Solana home: never creditable | Medium | `a5_01_*` (2 tests, failing) |
| A5-02 | One Solana address, several balance watches: the canister's own inflows read as another flow's refund (frog `item_arrived` during an Ethereum buy's bridge; the way back's ETH → SOL payout during a frog conversion; `BackFunded` does not hold the leg) | Medium | `a5_02_*` (failing) |
| A5-03 | `deposit_native` hands Relay's upgradeable program our signer privilege over the whole SOL float; S3 is checked only at judgement, and a durable-nonce deposit can land arbitrarily later | Medium | none (design) |
| A5-04 | `relayeth::judge_with` (ETH → SOL way back) truncates order deadlines with `as u64` (C-07 not carried over) | Medium | `a5_04_*` (failing) |
| A5-05 | The upgrade that first names a Solana home validates the argument, not the configuration it keeps: a saved EVM frog wrapper stays (custody silently reads EVM tickets without `assert_chain`); an argument omitting `home` validates eth_nft and holder deposits as the EVM home's; SALES, dormancy desk ids and open late orders are not checked | Low | none |
| A5-06 | A frog ticket redeemed to the listing's own custody address is a permanent `Relist` on the Solana home (sales are OFF): never delivered, never marked, the holder waits 37 days | Low | none |
| A5-07 | A Solana-origin quote is consumed (inbox removed, order id burned) before the line is checked: every busy-line tick wastes a judged quote and the S3 / rate outcalls | Low | none |
| A5-08 | Line turns: an owner queued while the line was busy and then failing before `ready()` holds the head of the queue for up to `TURN_SECS` (1 h) per occurrence | Low | none |
| A5-09 | The DORMANT watchdog reads at most `POOLS_READ` (1,000) unbacked pools in provider order: a graduation beyond them is never seen | Low | none |
| A5-I1 | The frog's S10 floor uses the lower median of sats/SOL (up to 1 % laxer); the SOL/ETH advance is sized at the lower lamports/ETH (up to 1 % short, only 25 bps FX margin): fail-safe | Info | |
| A5-I2 | A frog ticket whose destination does not parse for our network (`Destination::Eth` on an Inscriptions ticket, if rook_core ever allows it) loops in `owed` (an error each tick), never refused or marked | Info | |
| A5-I3 | Cosmetic units: "wei" in Solana-home logs (convflow GiveBack/Returned, `eth_give_back`, `Stuck` "REFUNDED LATE on 4663 … wei", payout texts) | Info | |
| A5-I4 | No per-order SOL ceiling on the SOL → ETH forward leg (only SOL → BTC has 250 SOL); `max_purchase_wei` up to 25 ETH ≈ 570 SOL may exceed Relay's SOL inventory: liveness only | Info | |
| A5-I5 | `TicketV.owner` and the desk requester are truncated to 20 bytes of a 32-byte Solana key; unused for authorization today; keep it that way | Info | |
| A5-I6 | Response caps of f2c24e0 (`TICKER_BYTES` 5,000, `SLICES_FIRST` 64): an answer over its cap is refused by the IC, never truncated, so every overflow is fail-closed (fewer sources, or the full-size retry); see A4 for the per-entry estimate of `program_slices` | Info | |

---

### A5-01: A written-off order's late payout is judged in the wrong currency (Medium)

**Where:** `src/ethflow.rs:4883-4888` (`judge_sol_payout`) and `src/ethflow.rs:5002-5007` (`judge_payout`), reached through `payout_pass` (`ethflow.rs:5034-5045`). The watches are written in `home_sol::fund` (`Watch.value = d.lamports`) and in the way back (`ethflow.rs:3251`, `value: d.value`, in wei).

**Description.** On the Solana home a forward order (SOL → ETH) sends **lamports** and pays out **wei**. The way back (ETH → SOL) sends **wei** and pays out **lamports**. Both payout judges compare the amount paid with `o.watch.value` with no conversion: "never more than the order sent" and "at least half of it".
- A forward order's late fill on Ethereum (`eth_credit_payout(buy, 1, …)`) goes to `judge_payout`. 0.87 ETH = 8.7e17 compared with 2e10 lamports is always refused as "more than the order sent".
- A way back's late fill on Solana (`eth_credit_payout(buy, 792703809, sig, …)`) goes to `judge_sol_payout`. 1.1e10 lamports × 10⁴ compared with 5e17 wei × 5,000 is always refused as "under half".
- The same-currency cases still work: a forward order's late refund on Solana, and a way back's late refund on Ethereum.

**Impact.** A written-off Relay order's real payout can never be credited to its buy or collection. Its `late` record keeps holding the Relay leg (`late_hold`, C5-M-1), so the Ethereum side starts no buy, no way back and no sweep. The only exit is `eth_late_release`. After that the paid ETH or SOL sits at our address unattributed, which is exactly the C5-M-1 situation (a later order's watch can read it as its own arrival or refund). Nothing is stolen, but the module's backing is not returned, and the stranded funds need a code change to recover.

**Exploit / trigger.** No attacker is needed: Relay fills or refunds after the deadline plus grace (the case C4-M-1 exists for), and the operator hands the payout in.

**PoC.** `a5_01_a_written_off_forward_orders_late_ethereum_fill_is_credited` and `a5_01_a_written_off_way_backs_late_solana_fill_is_credited`. They use the real `sol-eth-20` and `eth-sol-0.5` amounts. Both fail with the refusals above.

**Recommendation.**
- Bound the payout in its own currency: `wei_cap_of(value)` / `home_cap_of(value)` (rate plus `FX_TOLERANCE_BPS`), as `step_buy` already does for live watches.
- Better, store the order's value in both units when the watch is taken (`value_dest_max`, `min_out`), and judge "≤ value" and "≥ ½" against the payout chain's unit. The minimum can use the order's own `min_out`, which is already in the destination's unit.

---

### A5-02: The canister's own inflows read as another flow's refund on the shared Solana address (Medium; inherited in kind from the 4663 home)

**Where:**
- `ethflow.rs:2504`: the Bridging refund rule on `home_balance` = the SOL balance.
- `convflow.rs:167`: the frog refund rule on `our_balance` = the same SOL balance.
- `frogflow.rs:2453`: `item_arrived(cost)` pays `cost` lamports to the custodian = our address (rook_core `handle_item_arrived`, `pay_out(module_vault → custodian, cost)`).
- `ethflow.rs:306` (`in_relay` omits `BackFunded`) and `ethflow.rs:3205`: the way back's deposit checks only the Ethereum side's buys and `late_hold`, never `frog_relay_busy`.

**Description.** Q5 §6.3 accepts the balance rule because "paying us cannot hurt us" for third parties. On the Solana home, though, every flow's watch reads the one address that every flow is also paid into. The mutual exclusion (B-L-6, C-06) covers advances and the frog side's own reimbursements, but two inflows of our own escape it:
1. **Frog report during an Ethereum bridge.** A frog not advanced for (inventory, float-funded, or a stray reassigned to a sink) reports with `item_arrived(cost)`, which pays us `cost` (1–10 SOL by default, up to 250 SOL with a limit). Nothing makes this wait while an Ethereum buy is `Bridging` and watching the SOL balance. If cost ≥ ½ of the Ethereum deposit, the buy reads `REFUNDED`.
2. **Ethereum way back during a frog conversion.** `BackFunded` is not `in_relay`, so `relay_busy()` (the frog side's `eth_relay_busy`) is false. A frog conversion can therefore advance and deposit while the way back waits for its quote (up to `BACK_QUOTE_SECS` = 2 h). The way back's deposit is then signed without checking `frog_relay_busy`, and Relay's ETH → SOL payout lands while the frog watch (`Converting`, until the BTC is 3 confirmations deep) reads the same balance. Both the way back ("BACK ON SOLANA") and the frog ("REFUNDED") claim the same lamports.

**Impact.** One flow re-deposits or returns lamports that belong to another. The real order's fill (ETH on Ethereum, or BTC in the float) arrives unwatched and unattributed. The flow that loses the race finds its SOL gone: a `Returning` / `return_advance` whose simulation fails for lack of lamports until some other flow's advance lands and is spent in its place. Module books drift, and value is stranded across chains. Nothing reaches an outsider, and some timing coincidence is needed: case 1 has the bridge window (seconds to minutes); case 2 has the BTC confirmation window (about 30–60 min) inside a 2 h quote window.

**PoC.** `a5_02_a_buy_waiting_for_its_way_back_quote_holds_the_relay_leg` fails: `relay_busy()` is false for a `BackFunded` buy with its ask out. Case 1 needs no PoC beyond `relayconv::refunded(balance_after + cost, balance_after, deposit)` (see `tests/ethnft_sol.rs::sol_a_refund_without_a_memo_is_still_a_refund…`, which shows that any rise ≥ ½ is a refund).

**Recommendation.**
- Count `BackFunded` in `in_relay` (or check `frog_relay_busy()` at the way back's signature).
- Make `item_arrived`, and any other credit of ours, wait while `ethflow::relay_busy()`. Symmetrically, the Ethereum side should not open a watch while `convflow::reimbursement_owed`.
- Structurally: give each flow its own Solana deposit/refund address (another threshold derivation path), so a balance rule reads only its own leg.

---

### A5-03: Relay's upgradeable depository receives our signer privilege over the whole SOL float (Medium)

**Where:**
- `relaysol.rs:85-92`: `deposit_native_metas` passes `ours` as signer and writable.
- `relaysol.rs:197-202`: S3 is read once, in `judge_sol`.
- `solhome::sign`: the transaction is on the durable-nonce line and is re-sent until it lands.

**Description.** On the EVM home `depositNative{value}` can take at most `value`. On Solana, the program invoked with our account as a signer can CPI `system_program::transfer` for **any** amount from it: the whole SOL balance of every flow. The depository is upgradeable by `6Bz8…`. S3 pins the code at judgement time. But the signed transaction uses a durable nonce, does not expire, and is re-sent until it lands (minutes, or longer when providers fail), and S3 is never re-read. A program upgrade in that window, from a malicious or compromised owner key, executes with our signature.

**Impact.** Loss of the entire SOL float (every advance, refund and keep at the address), against a deposit that should risk only `amount`. The likelihood rests on Relay's owner key, a trusted third party. The impact is far larger than the EVM trust model it replaced.

**Recommendation.** Deposit from a dedicated, otherwise empty key: a second threshold Ed25519 derivation path, funded in the same transaction or the one before with exactly `amount` + fee, so the depository's signer privilege covers only the deposit. If that is not done:
- Re-read S3 before every resend.
- Sign deposits on a recent blockhash (bounded life) instead of the durable nonce.
- Keep the float at the address small.

---

### A5-04: ETH → SOL judge truncates deadlines (Medium)

**Where:** `relayeth.rs:208` (refund deadlines) and `relayeth.rs:250` (output deadline): `uint_of(..)? as u64`.

**Description.** Conversion review C-07 made `relayconv` and `relaysol` refuse any deadline over a uint32. `judge_with`, which the Solana home uses for the way back (`judge_to_solana`), still truncates. A quote whose output deadline is 2^64 + t is judged as t (inside the band), while the order id commits to 2^64 + t.

**Impact.**
- The canister's deadline-driven logic (`DEADLINE_GRACE_SECS`, `BACK_LATE_SECS`, the write-off) runs on a deadline the order does not carry.
- The order's real refund deadline is unbounded or meaningless to Relay's settlement, which can leave the way back's ETH in limbo past every watch.
- It needs a hostile or buggy quote, which the judge exists to refuse.

**PoC.** `a5_04_judge_to_solana_refuses_a_deadline_over_a_uint32` (the real `eth-sol-0.5` quote, deadline +2^64, id recomputed) is accepted. The test fails.

**Recommendation.** Use the same `deadline_of` (u32 check) in `relayeth` for refunds and output, as `relayconv` and `relaysol` do.

---

### A5-05: Upgrade guards check the argument, not the kept configuration (Low)

**Where:** `canister.rs:655` (`validate(&arg)`), `canister.rs:339` (`kept.wrapper = saved_wrapper…`), `canister.rs:182-185` (`restore_home`), and the guards at `canister.rs:~700-720`.

**Description.**
1. The frog wrapper is identity: the saved one is kept. An upgrade that first names a Solana home from an install with an EVM frog wrapper keeps the EVM wrapper, while `validate_frog_home` checks the argument's wrapper. As a result:
   - `CustodianIo::sol_wrapper()` is `None`, so custody silently reads and records tickets on the EVM wrapper.
   - On the Solana home `assert_home` no longer runs `assert_chain` (the eth_chainId check), so those EVM reads are unguarded.
   - The frog side refuses everything ("the frog wrapper is not a Solana address").
2. An argument that omits `home` keeps the saved Solana home (`restore_home`), but `validate` checks `eth_nft` with `validate_home(e, false)` and `holder_deposits` against an EVM home. `frognet` gates deposits by `self.sol.is_none()`, so frog deposits stay off, but the Ethereum side's `deposits` flag is accepted.
3. The "first names a Solana home" guard checks Ethereum buys, deliveries and pending work, and the frog side. It does not check:
   - SALES (sales are OFF on Solana);
   - custody redemptions in flight on EVM tickets;
   - dormancy desk request ids (EVM ids re-read on the Solana desk);
   - finished buys still holding an open `late` order.

**Impact.** Misconfiguration and DoS through an operator upgrade; fail-closed in most branches. The custody EVM fallback is the one silent EVM behaviour on a Solana home.

**Recommendation.**
- Validate the **kept** `Init` (wrapper against home, eth_nft and deposits against the kept home).
- Refuse a Solana home with a non-Solana kept wrapper.
- Extend the first-switch guard to custody redemptions, SALES, dormancy lives and open late orders.

---

### A5-06: A frog ticket to the listing's custody address is a permanent `Relist` on the Solana home (Low; cross-area lead from A2)

**Where:** `redeem.rs:407-424` (`discover`: a destination equal to our custody script is `RPhase::Relist`, finished, never delivered, never attested); rook_core accepts `listing.custody` as an Inscriptions destination (A2, wrap.rs:242-247).

**Questions from A2, answered for the Solana home:**
1. **Does the sale side ever sell or move a relisted item?** No. `set_sale_policy` is refused on the Solana home (`salenet.rs:41`). `saleflow::read` returns at once with no policies and no sales. Its ticket walk and `adopt_relist` read the EVM wrapper and vault (`frogflow::word` → `eth_read`) and cannot adopt a rook_core ticket. Nothing in the canister moves the inscription: it stays in custody, so the refund after the late window restores a unit that is still backed. **No phantom backing on the Solana home.** The exception is a SaleState carried over from an EVM install through an upgrade (A5-05 item 3); its `btc_pass` can still move listed outpoints of EVM-era sales, which is not reachable for rook_core tickets.
2. **Other paths that act on our own address as a destination:**
   - Ethereum NFT deliveries to our own EVM address or to nobody are marked undeliverable at once (`ethflow::mark_pass`).
   - Frog deliveries to the float or buyer address (`not_destinations`) are `Refused` and then attested (`mark_undeliverable`).
   - The custody address alone is the inert `Relist`, the one inconsistent case.

**Impact.** A holder who redeems to custody (by mistake, or a UI default) has the unit locked for 37 days with nothing happening, and the redemption record is kept for ever. There is no loss to the protocol.

**Recommendation.** On the Solana home (sales off), treat a custody destination like `not_destinations`: `Refused`, then `mark_undeliverable`, so the holder is refunded promptly. Or have rook_core refuse `listing.custody` while no relist mechanism exists on Solana (A2).

---

### A5-07: Quotes consumed before the line is checked (Low)

**Where:** `ethflow.rs:6509` (`home_sol::fund` removes the quote, reads S3, `sol_me`, the rate and balances, pushes `used_orders`, then `send` → `solhome::ready` may refuse because the line is busy); likewise `convflow::deposit_sol`.

**Description and impact.** On the one canister-wide line "busy" is common. Each time, the handed-in quote is lost, its order id is burned (harmless), and about four outcalls plus a full rate read (10 ticker legs + XRC) are paid for nothing. Under contention the flows can starve on quote freshness (TTL 300 s). The cost is liveness and cycles.

**Recommendation.** Check `SolLine::busy()` / `claim` before taking the quote out of the inbox and before the outcalls.

---

### A5-08: A queued turn held by a flow that no longer reaches `ready()` (Low)

**Where:** `solhome.rs:229-247` (`claim`); releases happen only on `Ok(None)` (`ethflow.rs:1247`, `frogflow.rs:690`, `redeem.rs:1270`).

**Description.** An owner is queued when the line is busy. If on later ticks its act errors before `ready()` (a refused quote, a read failure), its entry stays at the head and every other owner waits until the entry is `TURN_SECS` (1 h) old. The entry's `at` is never refreshed, which bounds the wait. But an owner that is repeatedly queued and then repeatedly errors can cut the line's throughput to about one step per hour for the others.

**Recommendation.** Release the turn on an `Err` before `ready()` as well, or keep the turn only while the owner's act is the one that queued it.

---

### A5-09: DORMANT watchdog bounded at 1,000 pools (Low)

**Where:** `ethflow.rs:6626` (`take(sh::POOLS_READ)` over `unbacked`, in provider order).

**Description.** More than 1,000 unbacked vaults per collection makes the pools beyond the first 1,000 invisible to `graduations`. A migrated pool among them never wakes the collection, and its coin waits for a fresh floor. Unlikely at launch volumes.

**Recommendation.** Rotate a cursor over the list across ticks, or refuse loudly over the bound as `VAULTS_LISTED` does.

---

### Informational

- **A5-I1.** `aggregate` takes the lower median.
  - SOL/BTC: fewer sats per SOL means a laxer S10 floor for frogs (≤ 1 % within the spread rule).
  - SOL/ETH: fewer lamports per ETH means a stricter S10 for the Ethereum side but a smaller advance (`advance_bound` grosses up with `FX_MARGIN_BPS` = 25 < `MAX_SPREAD_BPS` = 100). It can arrive slightly short of the buy's most, which then goes back, so it is fail-safe.
- **A5-I2.** `redeem::owed` (`redeem.rs:869`) propagates the `script_of` error, so an unparseable destination is retried every tick for ever (reads and cycles). Prefer `Refused { Malformed }` plus a mark.
- **A5-I3.** Several Solana-home log lines say "wei" for lamports (convflow `GiveBack`/`LateCredit` texts, `ethflow` `Stuck` "REFUNDED LATE on 4663", `eth_give_back`, payout-pass texts, the C2-L-2 return). The values are correct; only the wording is wrong.
- **A5-I4.** SOL → ETH has no capacity ceiling like `RELAY_SOL_BTC_MAX_LAMPORTS`. A large `max_purchase_wei` produces advances Relay may not quote; the buy then goes back.
- **A5-I5.** The truncated 20-byte owner and requester keys are used only for display. Never use them for authorization.
- **A5-I6.** The cost cuts are fail-closed, as described in the table.

## 3. Checked and found sound

- **`deposit_instruction`.**
  - The canister writes its own instruction from the judged `amount` and `order_id`: the five pinned metas, the pinned program, and no lookup table.
  - `judge_sol` S1–S12 matches `relayconv::judge` rule for rule, with the Solana additions: S3 ProgramData slot and authority with owner checks (`s3_reads`), S5 exact metas, S6 id = `hashStruct(orderData)` = `protocol.v2.orderId`, `paymentDetails` chain, depository, currency and amount.
  - S8 allows refunds only to our Solana address in SOL, or to the destination recipient in the destination currency, and requires at least one on Solana.
  - S9 requires one payment to our recipient in native currency, with no calls and no fees. S10 checks the floor from our own rate. S11 checks the u32 deadline band. S12 checks replay (per flow; orders differ by recipient across flows).
  - The recipient (our EVM address, or the float) and the refund (our Solana address) cannot be redirected.
- **Refund recognition.** The balance rule (≥ 50 % rise over the watch's start) decides, and the memo is a label only (`refund_label`). Third-party spoofing costs at least half the deposit and pays us. See A5-02 for the canister's own inflows.
- **Units.**
  - The advance is in lamports, the NFT in wei: `advance_covers` cuts the keep, fee and margin before converting, rounding down; `advance_bound` grosses up with ceiling rounding plus the FX margin and the keep, and is capped by the item cap.
  - `post_price` rounds down and clamps into `floor_bounds`.
  - `module_floor` and `gate` read wei at the tick's rate. `wei_cap_of` and `home_cap_of` bound live arrivals and way-back credits with the tolerance in the right direction.
  - `cost_of` is a unitless ratio.
  - The frog rate is sats per 10^18 lamports (`sats_per_home_unit`), so `min_sats_floor`, `wei_for_sats` and `cost_of` are consistent. `sats_to_lamports` rounds down.
  - The frog bounds are 1 / 10 / 250 SOL (`check_limits`, `read_sink`'s clamp, the advance and judge checks of `RELAY_SOL_BTC_MAX_LAMPORTS`), and `set_purchase_limit` uses the home's unit and ceiling.
- **Custody caps.** `sol:<wrapper>` keys are used consistently by frogflow, convflow, redeem and `caps_of`. Values are counted in wei at the tick's SOL/ETH rate (rounding down undercounts by under 1 wei per lamport step) and stored per frog (`values`) for later records.
- **Oracle.**
  - Both legs come from the same source.
  - ≥ 3 sources, spread ≤ 100 bps, lower median.
  - XRC staleness (600 s), future-dated refusal, ≥ 2 rates behind each quote.
  - Quantisation to 4 significant figures for dollar legs.
  - The OKX and crypto.com instrument is checked.
  - No cached rate (a rate is held only within one tick).
- **Delivery.**
  - Solana tickets are final state: `facts_final` and `ticket_read` take the nearer deadline (`chain_time = max(finalized, now)`).
  - The destination is bound to the read (`decide`). The network is checked by `network_ok` (registry and custody hrp) and `script_of(dest, hrp)`. A burn goes to the NUMS address per network.
  - Exactly once: `SolRecording` / `SolSent` resolve from the ticket account (`Delivered`, `Refunded` → `LostRace`, `undeliverable`) and re-sign only when the intent is no longer pending. `note_sent` goes out once (`noted`) and is best effort.
  - Ethereum deliveries: Eth destination as is, burn to `0x…dEaD`, taproot to nobody (marked); `record_delivery` names the item key read from rook_core.
- **Line.**
  - One step in flight canister-wide. Outcomes are kept per owner (16; with at most one step per owner in flight they cannot overflow).
  - A step missing from the line with no outcome is taken back as not landed. The trap window between `sign` and the flow saving `sol_step` is covered, because the outcome is processed by owner and not by `sol_step`.
  - A preflight refusal is abandoned unforwarded. `sol_clear_incident` is operator-only, refused while paused or mid-tick, and needs an open incident.
- **Upgrade.**
  - `migrate` (B1 → B2) moves B1's line once, keeps the pending step as the Ethereum side's, and refuses when both lines exist.
  - `restore_home` keeps the identity fields and takes only the providers and lifetime from the argument.
  - The currency switch is refused while Ethereum or frog work is open (but see A5-05).
- **`assert_home`.** It checks the genesis hash agreed by the configured providers against the pinned cluster, in both the paused and the unpaused tick.
- **Fallbacks.** The `home_sol` paths refuse `MintDeposit` / `Fill`. `ethnet`'s EVM home calls are never reached on the Solana home (`tests/ethnft_sol.rs` panics on any). The one EVM fallback found is the custody wrapper in A5-05.
- **`getProgramAccounts`.** The owner of every slice is checked (`unbacked`); over `VAULTS_LISTED` is refused loudly (paging itself is A4's).
- **`judge_sol_payout` / `put_sol_payout`.** They check the memo against the order or request id, a System transfer to us from another account, the delta cross-check, credited keys (keccak of the signature) and the inbox bound. The defect is only the units (A5-01).

## 4. Test results

- `cargo test --locked --no-fail-fast` in `autobuyer/canister-rs` at f2c24e0, plus the new file: **940 passed** (the existing suite, unchanged) and **4 failed**. The 4 failures are the PoCs, as intended:
  - `a5_01_a_written_off_forward_orders_late_ethereum_fill_is_credited`: `Err("870000000000000000 wei is more than the order sent (20000000000)")`
  - `a5_01_a_written_off_way_backs_late_solana_fill_is_credited`: `Err("11294422509 lamports is under half the order's 500000000000000000 …")`
  - `a5_02_a_buy_waiting_for_its_way_back_quote_holds_the_relay_leg`: `relay_busy()` is false for `BackFunded`
  - `a5_04_judge_to_solana_refuses_a_deadline_over_a_uint32`: the quote is accepted with deadline 2^64 + t judged as t
- No existing source file was modified.

## Verification (independent, 2026-10-10)

The verifier tried to disprove each finding. It re-read the cited code at `f2c24e0` and re-ran `cargo test --locked --test audit_a5_sol_flows`: 4 of 4 fail with the messages quoted in §4. It also read Relay's pinned constants and the Q5 spike (`relaysol.rs`, `docs/v9/spikes/q5-relay-solana-legs.md`) and rook_core's `advance` / `settle_advance` guards. No source file was modified.

| ID | Verdict | Final severity | Reason |
|---|---|---|---|
| A5-01 | CONFIRMED | Medium | The forward watch stores lamports (`ethflow.rs:6537`, `value: d.lamports`) and the way back stores wei (`:3251`). `judge_payout` and `judge_sol_payout` compare `paid` with `w.value` and convert nothing. No other path credits a written-off cross-currency payout, so only `eth_late_release` can free the leg |
| A5-02 | CONFIRMED | Medium | `in_relay` omits `BackFunded`, so `relay_busy()`, which the frog side reads as `eth_relay_busy`, is false while the way back's ask is out. `eth_act` step 3 signs the way back's deposit checking only Ethereum buys and `late_hold`, never `frog_relay_busy`. Separately, `report_one` signs `item_arrived(cost)` without consulting the Ethereum leg. Case 2 is a deterministic gap open for up to 2 h; case 1 needs a narrow bridge window |
| A5-03 | CONFIRMED | Medium (impact understated) | `deposit_native_metas` passes `ours` as signer and writable. A program invoked with a signer can CPI with that privilege, so an upgraded depository could drain the float. It could also act as **custodian** on rook_core (e.g. `advance` from a module vault to `me`, then transfer the lamports out in the same instruction) and as **nonce authority** (`AuthorizeNonceAccount`, `WithdrawNonceAccount`). Likelihood rests on Relay's upgrade key: Q5 itself says it cannot tell whether `6Bz8…` is a single key or a multisig (NEEDS-INFO for likelihood only) |
| A5-04 | CONFIRMED-ADJUSTED | **Low** | Confirmed: `uint_of(..)? as u64` at `relayeth.rs:208,250`, and the PoC passes with 2^64 + t. The identical defect in relayconv was rated **Info** when it was fixed (conversion review C-07, commit 3228c5f). It needs a hostile quote, the order is typed `uint32` so Relay cannot settle such an id normally, and the worst case is the way back's ETH waiting at Relay, not taken |
| A5-05 | CONFIRMED | Low | `validate(&arg)` runs before `restore_init`, and `kept.wrapper` keeps the saved wrapper. It is moot if the Solana home is a fresh install rather than an upgrade from an EVM install |
| A5-06 | CONFIRMED | Low | `redeem::discover` marks a destination equal to `our_script` as `RPhase::Relist` |
| A5-07 | CONFIRMED | Low | `home_sol::fund` removes the quote before the S3 reads and before `ready()` |
| A5-08 | CONFIRMED | Low | `release` is called only on `Ok(None)` (`ethflow.rs:1247`, `frogflow.rs:690`, `redeem.rs:1270`), and the owner's own queue entry survives `claim`'s retain |
| A5-09 | CONFIRMED | Low | `.take(sh::POOLS_READ)` over `unbacked`, in provider order |
| A5-I1…I6 | CONFIRMED | Info | Spot-checked; no disagreement |

**Notes**

- **A5-03, recommendation.**
  - Re-reading S3 before each resend, or signing on a recent blockhash, does not stop an *active* upgrade-key holder. Our signed deposit is readable from the public `sol_line` query before its first send, and a program upgraded at slot N is invocable from N+1, so the attacker can upgrade the program at least one slot ahead of our transaction.
  - Only the dedicated deposit key (a second Ed25519 derivation path holding exactly `amount` + fee) bounds the exposure. Make it the fix, not one option among several.
- **A5-02 and A4-01.** A Relay deposit is the one step with no on-chain idempotency guard (`home_effect` returns `None` for it). A4-01's abandon path and NEW-A4-01's incident clear can both produce a second deposit, which then collides with the balance watches described here.
- **"Checked and found sound" / Line.** The bullet "A preflight refusal is abandoned unforwarded" is not sound: see A4-01, whose verdict here is Medium.

**New issues**

None beyond NEW-A4-01, which is recorded in the A4 report.

## Fixes (2026-10-10)

Working tree on `golive-2026-10-08` (uncommitted). Paths are under `autobuyer/canister-rs/`. `cargo test --locked`: 967 passed, 0 failed (see A4's Fixes for the breakdown).

| ID | Status | Fix | Test |
|---|---|---|---|
| A5-01 (Medium) | Fixed | `src/ethflow.rs:4947` `payout_in_bounds`, called by `judge_sol_payout` and `judge_payout`. On the Solana home, a payout on the order's destination chain is judged in that chain's unit, against the order's own `min_out`: at least ½ `min_out`, at most `min_out · 10⁴/(10⁴ − relayeth::MAX_FEE_BPS) · (10⁴ + FX_TOLERANCE_BPS)/10⁴`. Every order was judged with `min_out ≥ value at our rate × (1 − fee)`, so the cap covers the order's full value there. Refunds on the origin chain, and everything on the EVM home, keep the `value` bounds. `judge_payout` takes a `sol_home` flag; `payout_pass` passes `io.sol_home().is_some()`. | both `a5_01_*` PoCs (only the call gained `, true`); `a5_01_payouts_are_bounded_in_their_own_unit_and_the_evm_home_is_unchanged`; `a5_01_a_way_backs_fill_on_solana_over_its_cap_or_under_half_its_min_out_is_refused` |
| A5-02 (Medium) | Fixed | (a) `BackFunded` is counted in `in_relay` (`src/ethflow.rs:309`). Step 3 ignores other `BackFunded` buys, so two waiting way backs cannot block each other. (b) The way back's deposit waits on `io.frog_relay_busy()` (`src/ethflow.rs:2606`). (c) `item_arrived` for a frog not advanced for waits while `eth_relay_busy()` (`src/frogflow.rs:2464`). Symmetric part: no new Ethereum buy (a new watch) opens on the Solana home while `EthIo::frog_reimbursement_owed()` (`src/ethflow.rs:2400`; the IC version reads FROGS/FLOORS and fails closed). | PoC `a5_02_*`; `ethnft_sol.rs` `a5_02_the_way_backs_deposit_waits_while_a_bitcoin_side_conversion_holds_the_relay_leg` and `a5_02_no_new_buy_opens_a_watch_while_a_frog_reimbursement_is_owed`; `frogs_sol.rs` `a5_02_a5_08_item_arrived_waits_for_the_ethereum_leg_and_an_unused_turn_is_released` |
| A5-03 (Medium) | Fixed | A dedicated threshold Ed25519 key on path `b"sol-relay"` (`src/solkey.rs:24`, `sol_relay_pubkey`). `sign_message` (`:101`) lets either key sign alone, never both. A Relay deposit is never signed by the main key. `solhome::sign` (`src/solhome.rs:503`) turns it into legs (`RelayLeg`, `:187`, kept in `SolLine.relay`): (1) the main key transfers exactly `amount + 5,000` to the Relay key, on the line's lifetime; (2) the Relay key alone signs and pays for `deposit_native`, on a recent blockhash (`solsend::payer_of` forces the fallback lifetime; the Relay key is no nonce authority and no custodian); (3) after anything but a successful deposit (it failed, was given up, or the simulation refused it), or when the key already held lamports, the key's balance is swept back to the main key BEFORE the step's outcome is reported (`relay_next`, `:711`), so no balance watch reads it as a refund. A successful deposit leaves the key at 0. Refunds go to the main key: the asks now carry `user` = Relay key and `refund_to` = main key (`src/ethflow.rs:2510`, `src/frogsol.rs:446`). The judge's S5 checks the metas against `SolWant.depositor` (the Relay key, `src/relaysol.rs:122`). S8 still requires the Solana refund to go to `our_sol` (the main key). If Relay did not honour `refundTo`, every quote would be refused (fail-closed). Ops must confirm on the first live quote. The balance checks now cover the deposit and its 2 fees. | `ethnft_sol.rs` `to_arrival` (deposit sender is the Relay key; nothing left at it; log `system:2, deposit_native`); `fix_a5_03_a_failed_deposit_is_swept_back_from_the_relay_key_before_its_flow_is_told`; `fix_a5_03_a_relay_key_holding_lamports_is_swept_back_before_any_deposit`; `frogs_sol.rs` conversion flow |
| A5-04 (Low) | Fixed | `src/relayeth.rs:94` `deadline_of`: a u32-checked deadline ("rule 11 … over a uint32"), used for refund and output deadlines. | PoC `a5_04_*`; `a5_04_a_refund_deadline_over_a_uint32_is_refused` |
| A5-05 | Not changed | Out of the requested scope (operator-upgrade misconfiguration; moot for a fresh Solana install). | |
| A5-06 (Low) | Fixed | `src/redeem.rs:423`: a custody destination becomes `Relist` only when `sol_wrapper()` is `None`. On the Solana home (`:882`) it is `Refused` with `attest = Due`, so it is marked undeliverable and refunded. The EVM home is unchanged. | `frogs_sol.rs` `a5_06_a5_i2_a_ticket_to_custody_or_to_no_bitcoin_address_is_refused_and_marked_undeliverable` |
| A5-07 (Low) | Fixed | `solhome::turn` (`src/solhome.rs:468`: incident and queue claim, no outcall) runs before the quote is consumed or any S3/rate read: `src/ethflow.rs:6590` (`home_sol::fund`, which keeps the quote in the inbox) and `src/convflow.rs:430` (which hands the quote back with the new `FrogIo::return_quote`). | `ethnft_sol.rs` `fix_a5_07_a_quote_waits_unconsumed_while_the_line_is_not_this_sides_turn` |
| A5-08 (Low) | Fixed | `src/ethflow.rs:1276` `release_untaken_turn`: an act that ended in `Err` gives up its queue entry when that entry heads a FREE line, which means its act never reached `ready()`. The queued wait behind a busy line keeps its place. Called from the Eth, frog (`src/frogflow.rs:698`) and custody (`src/redeem.rs:1296`) passes. | the `frogs_sol.rs` A5-02/A5-08 test (release and keep) |
| A5-09 | Not changed | Out of the requested scope. | |
| A5-I2 (Info) | Fixed | `src/redeem.rs:874`: an unparseable destination becomes `Refused { Malformed }` with `attest = Due` (marked undeliverable), instead of an error every tick. | the `frogs_sol.rs` A5-06/I2 test |
| A5-I3 (Info) | Fixed | Solana-home texts say "lamports": convflow GiveBack, LateCredit and `watch_late`; ethflow "REFUNDED LATE on Solana", the C2-L-2 return, the payout pass and `give_back_now_keeping`. The `*_keeping` helpers tell the home apart by the keep amount, which is a heuristic. | `a5_i3_the_give_back_names_the_homes_unit` |
| A5-I1, I4–I6 | Not changed | Informational, fail-safe. | |

**A6-13, cross-referenced here as the audit asked.**
- rook_backing's program id now comes from `Init.home`: `Home::Solana.rook_backing: opt text` (`src/canister.rs:115`), mapped to `SolHomeCfg.rook_backing`.
- `solread::partner_of(listing, program)` and the dormancy watchdog's vault listing use it.
- It is identity: set once, from nothing (`restore_home`).
- On mainnet-beta it must be named, and must not be the dev id (`src/canister.rs:138`). Elsewhere it defaults to the dev id.
- Updated with it: `.did`, the monitor's strict Init IDL and its argdiff (`monitor/src/candid.mjs`, `monitor/src/argdiff.mjs`), and `ops/rehearsal/sol-devnet/canister-init.mjs`.
- `init-dormant.hex` and `init-solana.hex` were regenerated.
- Tests: `tests/solana.rs` (validation, the default, restore as identity).
