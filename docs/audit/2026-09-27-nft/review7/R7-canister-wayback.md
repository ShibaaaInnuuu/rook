# Round 7, part C: the way back, the late hold and the payout judge after fix round 6

Reviewer: independent AI re-review, 2026-09-28. Worktree `/Users/jlohayon/Documents/Rook-nft`, branch `nft-side` at `a78a542`
(delta `bb713fe..HEAD`, canister commit `2d747ba`).

**Scope:**
- C6-L-1: `relayconv.rs` `RELAY_CALL_EXECUTED`, `call_executed` (`:31-52`); `ethflow.rs` `judge_payout` (`:4283-4337`);
  `EthIo::payout_logs` (`ethflow.rs:937`, `ethnet.rs:487-490`); `canister.rs` `home_receipt_logs` (`:1040-1050`); judgment call 3.
- C6-L-2: `payout_pass` (`ethflow.rs:4344-4480`), the WAIT branch (`:4356-4359`, `:4462-4467`), `buy_of_step`; judgment call 6.
- C6-L-3: `buy_pass`'s reservation sweep (`:2003-2019`), the Ethereum credit (`:4411-4435`); judgment call 7.
- C6-I-1: the deposit sites (`:2803`, `:3601`).
- D6-L-3, the canister side: `late_hold` / `relay_busy` (`:685-692`), `buy_pass`'s early return (`:2024-2028`).

Toolchain 1.98.1.

**Nothing in the worktree was modified except this report and `review7/poc/C/`. Nothing was committed, pushed, deployed or
signed.**

- **Where the PoCs ran:** a private copy of `autobuyer/canister-rs` at `/tmp/rook-r7-C`. `diff -rq` (excluding `target/`) showed it
  identical to the worktree before anything was added. It was removed afterwards.
- **The harness:** `tests/audit_nft_r6.rs`'s fake world, unchanged. The three PoCs are appended (`poc/C/r7_c.rs.snippet`), and their
  output is in `poc/C/r7_c.output.txt`.
- **Read-only network use** (`poc/C/depository_probe.output.txt`, `poc/C/rpc.sh`):
  - `eth_getCode` of Relay's depository on Ethereum and on 4663;
  - `eth_getLogs` of its `RelayCallExecuted` over 50,000 Ethereum blocks (17,730 logs) and 1,000,000 blocks of 4663 (3,335 logs);
  - `eth_getTransactionByHash` of real `execute` calls;
  - the depository's public source (`relayprotocol/relay-depository`, `RelayDepository.sol`);
  - Relay's public `/requests/v2` API (GET).

Severity scale and threat model: `../README.md`'s. A passing PoC shows the weakness is present.

## Verdict

- **C6-L-1 is closed against the reviewer's attack, but its credit path for a depository payout is unreachable in reality (C7-L-1,
  Low).**
  - **Signature confirmed.** The event is `RelayCallExecuted(bytes32 id, (address to, bytes data, uint256 value, bool allowFailure)
    call)`, topic0 `0x4be109453ef7e895dc7215c929fff9b76b51483d56a4d04548b4866e9aa7c5ea`.
    - `id` is **not indexed**: every one of 21,065 real logs has one topic.
    - The data is `[id, 0x40, to, 0x80, value, allowFailure, len(data), data…]`.
    - `call_executed` decodes the real bytes correctly (PoC step 1).
  - **Spoofing.** Every attempt failed:
    - another emitter: filtered by address;
    - a forged `to` or value by offset tricks: the depository encodes the event itself;
    - a failed `allowFailure` call: the source emits only `if (success)`, `RelayDepository.sol:219-220`;
    - a record for another order's id;
    - a view function with trailing bytes, or a deposit: no event.

    A stranger cannot make the depository emit a record paying us: that needs the allocator's signature.
  - **But the event's `id` is the CallRequest's EIP-712 struct hash** (`_executeCalls(structHash, …)`, `RelayDepository.sol:180`;
    reproduced on a real transaction in PoC step 2). It is never an order id or Relay's request id, so the judge's "the record's id
    names the order" half (`ethflow.rs:4301`) is dead code.
    - A real `execute` input carries only the calls, a random allocator nonce, the expiry and the signature. For a native payment to
      an EOA the call data is empty, so the input does not name the order either.
    - Result: a real depository payout to us is refused "does not name the order", even with the operator stating the exact amount.
    - The fixer's positive tests (`r6_c6_l1_a/b`) credit only through a shape the contract never emits: `topics: [sig, order_id]`.
- **C6-L-2 is closed.** No credit is written while the buy's 4663 transaction is out. The kept claim cannot loop for ever: its hand-in
  time is kept, and after 600 s it is dropped as stale (C7-I-2).
- **C6-L-3 is closed.** The reservation is kept exactly while a forward write-off is open (`!back && !credited && !released`). It is
  re-taken before `arrive` on a released order, and settled back if `arrive` refuses. With no room the ETH is the collection's
  leftover. No double count and no permanent block was found (below).
- **C6-I-1 is closed.** Both deposit sites read the 4663 pin with `?` before `used_orders.push`, so a failed read writes nothing and
  retries the next tick. `EthNet::home_pin` never answers `Ok(None)`.
- **D6-L-3's canister side is unchanged and correct.** `late_hold` is `!credited && !released` whatever the buy's phase, and gates new
  buys (`:2024`), ways back and sweeps (`relay_busy`). The paging is part D's.

## Counts (part C, open)

| Critical | High | Medium | Low | Info |
| ---: | ---: | ---: | ---: | ---: |
| 0 | 0 | 0 | 1 (C7-L-1) | 4 (C7-I-1..4) |

## C7-L-1 (Low): the depository credit path cannot credit a real Relay depository payout; one refunded that way is stranded

**Code:**
- `ethflow.rs:4296-4301`: `execs` and `names`; `:4315-4325`: the depository branch;
- `relayconv.rs:31-52`;
- the fixer's tests `tests/audit_nft_r6.rs:1345-1349` (`relay_exec`: the id as a second topic), `:1351-1406`.

**What happens:**
- `judge_payout` requires the payout to **name** the order before looking at the record: either a 32-byte window of the transaction's
  input equals the order id or the request id, or a `RelayCallExecuted` id does.
- **The record's id never names it.** The depository sets that id to `keccak(abi.encode(CALL_REQUEST_TYPEHASH, keccak(callHashes),
  nonce, expiration))`. PoC step 2 recomputes it from the real transaction `0x86eee61f…d84d` (Ethereum block 0x18dd4e1) and matches
  its log's id `0x84334077…6659`.
- **The input does not name it either.** A real `execute(CallRequest, signature)` input holds the calls (`to` = the payee, empty
  data, value), a random nonce (`0x8b63e873…01ed`; Relay's `/requests/v2?id=` finds no request under it), the expiry and the
  signature. The order id appears nowhere unless Relay chose to put it there.
- **The PoC.** `r7_c_l1_a_…` takes that real transaction re-targeted to pay our address (refund 4.9 ETH of a 5 ETH order). Relay's own
  record paying us is present and decodes. The result is `Err("the transaction does not name order …")`, with and without the
  operator's stated amount. The fixer's test shape (`topics [sig, order_id]`) is what credits (`Ok(4.9 ETH)`).

**Effect:**
- **If Relay pays a late payout of a written-off order out of its depository,** the operator cannot credit it (judgment call 3's
  "confirm a real refund carries it" is answered: it carries the record, but not the order).
  - The order can only be released (`eth_late_release`). The ETH then sits at our address unbooked: on 4663 in the gas pot, beside
    `others`; on Ethereum outside every earmark and leftover.
  - No path returns unbooked ETH to its module; recovering it takes a timelocked upgrade.
  - The module has already booked the loss (`returnAdvance(0)`).
  - The bound is one order's value per such event: at most the largest buy, e.g. BAYC's advance.
- **Not a regression.** Round 5's statement path needed the same naming, and a real depository call does not name the order, so it
  could not credit this either.
- **How likely.** Relay's depository executions in the sample are almost all the solver's own sweeps. On Ethereum over 50,000 blocks:
  - 2,592 native payments to the solver `0xf70d…dbef`;
  - 6 to others;
  - the rest ERC-20.

  On 4663 over 1,000,000 blocks, the only native payments are the 1,658 to the solver. So late refunds most likely come from the
  solver as plain transfers, which the judge credits when the input names the order.

  The non-solver executions look like depositor-claimed withdrawals: `0x86eee6…` is submitted by its own payee. If that is Relay's
  refund route for an unfilled deposit, the canister (the depositor) would have to submit the `execute` itself. It cannot, and the
  judge refuses `t.from == us` anyway (rule 1). The ETH would stay in the depository.
- **Low:** conditional on a written-off order (Relay failing past deadline + grace + a day) and a depository-route refund. The loss is
  bounded, operator-visible (the hold pages) and recoverable by upgrade.

**Fix:**
1. **Drop the dead id comparison, or name the order through Relay's own binding.** The record is `(structHash, call)`. What ties it to
   our order is Relay's API: the operator's hand-in could carry the CallRequest's nonce, from Relay's status for the order. Credit
   when the transaction's `execute` input decodes to that nonce and a call paying us, and the record's id equals the recomputed struct
   hash.
2. **Measure before launch (C5-I-1, already a precondition).** Do a real Relay deposit that is refunded, each way. Record whether the
   refund is a solver transfer (creditable now if its input names the order) or a depository `execute` (not creditable, needs 1). If
   it is a depositor-claimed withdrawal, the runbook must say that such a refund cannot reach the canister without a new signer path.
3. **Correct `r6_c6_l1_a/b`'s `relay_exec`** to the real encoding (one topic, id = struct hash), so that the tests show what is and is
   not creditable.

## Infos

- **C7-I-1 (the record amount is summed across ids).** `to_us` (`ethflow.rs:4317`) adds every `RelayCallExecuted` paying us in the
  receipt, whatever its id. With the order named in the input, a Relay batch paying us for two orders credits both to this one.
  - PoC `r7_c_i1_…`: 3 + 2 ETH, credited 5 ETH.
  - It is bounded by the order's value, and the balance check requires the ETH to be there. The other order's payout is then never
    creditable (`credited` holds the tx).
  - Fix: sum only the records whose call is the one the input names (see C7-L-1 fix 1).
- **C7-I-2 (judgment call 6: 600 s stale claims).** A claim waiting behind its buy's 4663 transaction is dropped as stale 600 s after
  hand-in. Nothing hands it in again.
  - PoC `r7_c_i2_…`: two WAITs, then "handed in too long ago: stale", the queue empty, the hold standing.
  - While it waits it is first in line, so any other claim waits too, at most 600 s.
  - An unidentified home transaction (F6 paths) can be out for an hour or more, so the operator may have to hand the claim in several
    times.
  - No loop, no starvation beyond the TTL, nothing credited wrongly. Acceptable. The runbook's `eth_credit_payout` line should say "if
    the note says `waits` and then `stale`, hand it in again after the 4663 transaction resolves".
- **C7-I-3 (judgment call 3's plain-transfer half: what a stranger's transfer costs).** A plain transfer to us whose input names the
  order is credited at its value (`:4309-4314`). To end the hold, a stranger must pay at least half the order's value (the
  `REFUND_SHARE_BPS` floor), and after the order's deposit block. We keep all of it.
  - The real payout, landing later, is then unbooked at our address: stranded as in C7-L-1, never lost to anyone.
  - The stranger gains nothing and pays at least ½ the order value. The hand-in is operator-only (`ethnet.rs:608-620`
    `only_operator`), so it needs the operator to pick the stranger's transaction over Relay's.
  - Acceptable as the fixer states.
- **C7-I-4 (judgment call 7: room held by a closed forward write-off).** The reservation is also kept after `eth_give_back` closes the
  buy as a loss (`finished()` but `late` open, `:2012-2013`). While the hold stands (possibly days), that collection's H2 room is one
  item short, so a holder deposit of that collection can be refused at its cap. New Relay buys are stopped by `late_hold` anyway, and
  release or credit frees the room at the next pass. It is conservative and bounded to one item.
  - Checked: no double count (`had` is looked up before `reserve`). `arrive`'s refusal settles only a reservation it made. A released
    order credited with no room goes to leftover and stays written off. A way back (`back`) keeps no room, and its Ethereum credit
    reserves as a fresh arrival.

## Round-6 items: verdicts

| ID | Verdict | Basis, residuals |
| --- | --- | --- |
| **C6-L-1** | **Closed against the attack; the replacement credit path is unreachable (C7-L-1)** | The stranger's gas-only depository call, the ERC-20 call through an address-like request id, forged emitters, records paying others, and failed `allowFailure` calls are all refused (source: emitted only on success). 4663 records are read through `home_receipt_logs` at the pin, every provider's projection equal. Ethereum's go through `eth_sale_receipt` at the pin, after `payout_tx`'s finality check. But the record's id is a struct hash, so a real depository payout is never creditable: C7-L-1, C7-I-1 |
| **C6-L-2** | **Closed** | `buy_of_step` covers every step of the buy (advance, deposit, give back, mint, settle). The claim is re-queued with its original `at`, TTL first. After the loss-closing `returnAdvance(0)` resolves, the credit goes to a new sweep's `fundReserve` (the fixer's `r6_c6_l2_a` reproduces). C7-I-2 |
| **C6-L-3** | **Closed** | The fixer's `r6_c6_l3_a/b` reproduce. The item is in the book, with no "not in flight". C7-I-4 |
| **C6-I-1** | **Closed** | `:2803`, `:3601` |
| **D6-L-3** (canister side) | **Closed** | `late_hold` unchanged; the page is part D's |
| **Judgment call 3** | **Needs the measurement, and C7-L-1's fix if the measurement shows a depository refund** | See C7-L-1 and C7-I-3 |
| **Judgment calls 6, 7** | **Acceptable** | C7-I-2, C7-I-4 |

**Outside this part, noted:**
- Relay's `/requests/v2` API now answers with a deprecation: throttled from 2026-09-01, retired 2026-11-24. The operator's "Relay's
  status names it" lookups, and any runner code on v2, need `/requests/v3`.
- `rpc.mevblocker.io` answered a 50,000-block `eth_getLogs` with `-32603 "service temporarily unavailable"`: for part B's
  classification.

## Recommended fixes, in order

1. **C7-L-1:** bind a depository payout to the order through the CallRequest (nonce from Relay's status + the recomputed struct hash),
   or document that only solver transfers are creditable. Run the C5-I-1 measurement first, and fix `relay_exec` in the tests to the
   real encoding.
2. **C7-I-1:** credit only the records of the named call.
3. **C7-I-2:** a runbook line for a claim that goes stale while waiting.
