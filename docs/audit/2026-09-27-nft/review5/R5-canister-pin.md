# Round 5, part F: the pin floors on both chains and the home line's exit (C4-H-1's fix)

Reviewer: independent AI re-review, 2026-09-28. Worktree `/Users/jlohayon/Documents/Rook-nft`, branch `nft-side` at `1dc2a61`
(delta `2735dac..1dc2a61`, fix round 4). Scope: C4-H-1's fix and E4-M-1, that is `pin.rs` (`PinFloor`, `PinFloors`, the unit test),
`canister.rs` (`PIN_FLOORS`, `pin`, `CanisterIo::pin` / `evm_nonce` / `evm_balance` / `evm_seen`, `home_payout_tx`, `broadcast_evm`,
`evm_tx_status`, `pinned_read`, `pre_upgrade`, `post_upgrade`, `restore_init`), `ethnet.rs` (`EthNet::pin`, `eth_nonce`, `fees`,
`home_pin`, `home_sign`, `payout_tx`, every other read), `ethflow.rs` (`HomeTx.used_since`, `EthState.home_floor` / `pin_floors`,
`home_floor_ok`, `send_home`, `resolve_home`, `mined_home` step by step, `home_act`, `step_buy`'s Relay-leg reads, `give_back_now`),
`evmsend.rs`, and for the shared 4663 line `looper.rs`, `redeem.rs`, `frogflow.rs`, `nftnet.rs` (`pin_on`, the operator endpoints),
`evmread.rs` (`fast_pin`). `src/floor/FloorModule.sol` and `src/nft/EthNftWrapper.sol` (unchanged) were read for `advance`,
`returnAdvance`, `settleAdvance`, `fundReserve`, `noteSent` and `markUndeliverable`. Tests read: `tests/audit_nft_r4.rs`
`r4_c4_h1_*`, `r4_e4_m1_*`, `r4_states_saved_by_the_old_builds_*`, and `pin.rs` `c4_h1_*`. Toolchain 1.98.1.

**Nothing in the worktree was modified except this report and `review5/poc/F/`; nothing committed, pushed, deployed or signed.**
Every PoC ran in a private APFS clone of `autobuyer/` under `/tmp/rook-r5/F/`, checked identical to the worktree's `src/` and `tests/`
with `diff -rq` before the PoC was added, and removed afterwards. The harness is the fixer's `tests/audit_nft_r4.rs` copied whole
(its switches included: `home_back`, `home_pin_known`, `real_outflow`, ...), with three additions (`poc/F/r5_f.harness.diff`):

- `hide`: home transactions that MINED but that every 4663 provider fails to show (no receipt at the pin, no transaction by hash);
- `unsure`: the receipt read is `Unsure` (the providers disagree, one shows the transaction);
- `real_advance`: the module as `FloorModule.sol` is: `maxItemCost()` stays above 0 while an advance is open (only the unit in
  transit leaves `netUnits`), a second `advance` reverts `AdvanceOpen` and a `returnAdvance` with none open reverts `NoAdvance` at the
  estimate.

The additions are off by default. `cargo test --test r5_f` passes 30 of 30: the fixer's 22 with the switches off, plus the 8 PoCs.
`pin.rs` `c4_h1_*` passes. A concurrent `rm -rf /tmp/rook-r5` by another reviewer (about 05:15) deleted the first copy. The recorded
output was produced again on a fresh clone, checked with `diff -rq` against the worktree's `src/` and `tests/` before the PoC was
rebuilt from `poc/F/`, and it matched line for line.

Severity scale and threat model: the README's.

## Verdict

**C4-H-1 is closed as reported.** No 4663 read of the canister is taken at a pin below one already read, and no home transaction is
signed at a nonce below one already read:

- `canister::pin` (`canister.rs:899-903`) admits a pin only after `fast_pin` has agreed it on every provider, and refuses one below
  the canister-wide floor. Every 4663 state read of every part goes through it (the survey below).
- `CanisterIo::evm_nonce` (`:1260-1266`) refuses a nonce below the highest read.
- The NFT side also keeps its own floor (`home_floor_ok`, `ethflow.rs:2672-2694`; `send_home`, `:2697-2720`).
- The round-4 PoCs fail on their attack assertions (`fixes-round4-poc/`), and the E4-M-1 PoC's harness raises the balance with the
  nonce and pin unchanged, which is not what a lower pin shows. On the IC a lower pin is refused before the balance is read.

**The exit the fix added opens a Medium (F5-M-1).** `resolve_home` drops a pending home transaction that is "seen nowhere" for
`HOME_NONCE_TAKEN_SECS` while its nonce is used, and takes its step back as a revert (`:3147-3162`).

- With the floors in place, nothing of ours can take the NFT side's nonce any more: `send_home` signs only at the pin's nonce, and
  the other signers wait on `evm_pending`. So after this build the drop fires only when **our own transaction mined and every provider
  fails to show it**.
- In that case the drop is always wrong. For the steps that carry value, nothing guards the redo, contrary to `fixes-round4.md` §1
  and §9 call 1:
  - a Relay deposit is paid again from any other ETH at the shared address;
  - `fundReserve` is paid again;
  - an `advance` is orphaned: the module's advance stays open, its ETH is nobody's, and the re-sign holds the Relay leg for every
    collection with no operator exit;
  - a `returnAdvance` holds the Relay leg with no operator exit.
- "Seen nowhere for 1 h" is two samples an hour apart. Reads in between that a provider answered do not restart the clock.

**One Low (F5-L-1):** the Ethereum floor is one per canister, not per chain. It survives an upgrade that moves `eth_nft` to another
chain, which the upgrade allows while nothing is open, and then refuses that chain's pins or nonces for good. Five Infos.

## Counts (this part)

| Critical | High | Medium | Low | Info |
| --- | --- | --- | --- | --- |
| 0 | 0 | 1 | 1 | 5 |

## Round-4 items in scope

| ID | Sev (R4) | Verdict | How it was re-attacked |
| --- | --- | --- | --- |
| **C4-H-1**: a 4663 pin one bucket back signs at a used nonce; the one 4663 line waits for ever; our deposit read as refunded | High | **Closed** (its exit: **F5-M-1**; its floor: **F5-L-1**) | Every 4663 read surveyed (below). The floor is raised only by a pin agreed on every provider, and only after it is agreed. A provider far ahead cannot raise it: `choose_pin` refuses heads more than a bucket apart and pins one bucket under the LOWEST. Nonces only grow at monotone pins. Within a tick every part's pin is taken fresh and is at least the previous part's, so one part's higher read cannot stall another beyond that tick (a cached older pin can only fail a read, which is a wait). A chronically lagging provider lowers the pin, but the chain passes a bucket in about 100 s, so a floor set at a good moment is overtaken within a tick or two: refused ticks, never a permanent refusal. Restored across upgrades; an old state starts at 0 (F5-I-2). The Relay leg's 4663 reads (`Bridging` refund, `Stuck` late refund, `BackBridging`) are at the floored pin, and a refund returns `home − w.balance_after` capped at the value, never the raw balance (`ethflow.rs:1627-1640`, `:1664-1673`). |
| **E4-M-1** (= C4-H-1): a home pin that moves back reads our own Relay deposit as a refund | Med | **Closed** on the canister | The fixer's reading is right. The E PoC's `home_pin_back` raises the balance and leaves the nonce and pin as they were. A real pin behind our mined deposit also shows the nonce before it, and `canister::pin` refuses it before any balance is read. `r4_e4_m1_*` runs the path with the pin said. It would fail on the old code once adapted (no `home_pin`, so the phantom refund is read). |
| §9 call 1: a home transaction seen nowhere for 1 h with its nonce used is dropped and redone | (call) | **Not safe as written: F5-M-1** | See F5-M-1. The claim "every re-signed step is guarded by its estimate, the contracts and the balance check" does not hold for `Deposit`, `GiveBack` (`fundReserve` and `returnAdvance`) or `Advance`. |
| §10: `nftnet`'s `nft_*` endpoints are the only unfloored reads | (open) | **Confirmed** | `nftnet.rs:82` `pin_on` (no floor) is used by `nft_preview` / `nft_sign` / `nft_status` on the operator's own line. `nft_sign` refuses the NFT side's chain (`:305-306`). `EthNet::pin` calls the same `pin_on` and then admits the pin (`ethnet.rs:142`). The other unfloored reads are not pin reads, and none of them decides a spend on its own: the finality reads (`eth_finalized`, `home_finalized_time`, the custodian's `finalized` logs, H9 design) and `eth_getTransactionByHash` for Pending/NotYet. |
| §10: `looper` / `redeem` / `frogflow` "nonce used, not visible" waits have no exit | (open) | **Remaining trigger: F5-M-1's (F5-I-1)** | With the floor, a lower pin cannot cause them. They still fire, and hold the shared line, whenever a mined transaction of theirs is shown by no provider. That is the same trigger on which the NFT side now drops and redoes. The waits end when an index catches up, so they are not permanent unless the omission is. |
| Upgrade safety of `PIN_FLOORS` / `pin_floors` / `home_floor` / `used_since` | — | **Holds** (F5-L-1, F5-I-2) | Every new field is `opt` with `#[serde(default)]`. `pre_upgrade` only copies a heap value (no new trap). `post_upgrade` restores after its refusals and before `ETH` (`canister.rs:541`). An old state decodes with `None`: floors at 0, set by the first tick. A downgrade drops the fields, and `lost_opts` ignores fields the older type lacks. `r4_states_saved_by_the_old_builds_*` restores the three old builds' states. |

### Every chain read, surveyed for a floor

- **4663.** `canister.rs` reads 4663 state only at `pin()` or `CanisterIo::pin()`, which calls `pin()`:
  - `evm_balance`, `evm_nonce`, `estimate_gas` (pin by number), `eth_call`, `evm_seen` (receipt at the pin);
  - `home_payout_tx`, `broadcast_evm`'s read-back, `evm_tx_status`, `pinned_read`;
  - `purchase_count`, `vault_purchase`, `eth_read`, `base_fee`.

  `frognet::home_clock` reads the pinned block by hash. `custodynet` (`:201`, `:209`) and `frognet` (`:155`) take `io.pin()` of a
  `CanisterIo`. `ethnet` reads 4663 only through its `CanisterIo` (`home_read`, `home_balance`, `home_nonce`, `home_sign`,
  `home_seen`, `home_pin`) and `home_payout_tx`. Not pin reads: `assert_chain`, `rpc_view(eth_getTransactionByHash)` (Pending
  versus NotYet) and the finality reads.
- **Ethereum (NFT side).** `EthNet` reads only at `self.pin()`, which admits every pin (`ethnet.rs:137-146`):
  - `call`, `call_small`, `fees` (the `eth_call` simulation, `eth_estimateGas` at the pin number, the nonce);
  - `eth_pin`, `eth_read`, `eth_balance`, `eth_nonce`, `eth_receipt`;
  - `eth_plan_transfer`, `eth_plan_call`, `eth_listing`, `eth_sale_receipt`, `eth_transfer_moves`, `eth_sale_moves`, `payout_tx`.

  `eth_deposit_tx` reads the transaction by hash, then its receipt at the pin. Not pin reads: `eth_finalized` and the deposit
  scan's finality.

The only nonce read that does not pass `admit_nonce` is the one in `EthNet::fees` (`ethnet.rs:200`). That is the nonce Ethereum
transactions are signed at, and it is covered by the pin floor and `last_mined` (F5-I-3).

## Findings

### F5-M-1 (Medium): the home line's exit drops a transaction that MINED but that no provider shows for an hour, and redoes its step; for the value-bearing steps nothing guards the redo (a Relay deposit and `fundReserve` paid twice, an `advance` orphaned, the Relay leg held with no exit)

**Where.**

- `ethflow.rs:3138-3162` (`resolve_home`: `NotYet` with `chain > p.nonce` starts `used_since` (`:3152`); `HOME_NONCE_TAKEN_SECS`
  later it drops the transaction, raises the floor to the chain's nonce (`:3158`) and calls `mined_home(.., false)`).
- `:3163-3164`: `Pending` and `Unsure` leave `used_since` as it is.
- `mined_home`:
  - `:3171-3186` (`Advance`: back to `NeedAdvance`, `advanced = 0`, the custody reservation settled);
  - `:3187-3200` (`Deposit`: back to `Funded`, `last = None` (`:3195`), a new quote asked);
  - `:3201-3205` (`GiveBack`: "sent again next tick");
  - `:3235-3245` (`Settle`), `:3246-3258` (`Record`).
- `home_act`:
  - `:3030` (the redeposit's only check: `bal < d.value` on the raw balance);
  - `:2938-2946` (`Returning`: `bal < wei`, then `fundReserve` or `returnAdvance`);
  - `:2948-2997` (`NeedAdvance`: an `Err` from `send_home` leaves the buy in `NeedAdvance`, which `in_relay` counts, `:230-236`);
  - `:2875-2878` (a `Record` whose ticket is already `Delivered` goes `Gone` without `custodycap::release`).
- `give_back_now` `:3464` (no exit for `NeedAdvance` or `Returning`).
- `src/floor/FloorModule.sol:1040-1045` (`fundReserve`: no idempotence, any value above 0 is added to `surplus`), `:684-704`
  (`advance`: `AdvanceOpen` while `transitUnits != 0`).

**What happens.**

1. With the floors, the NFT side signs only at the pin's nonce (`send_home` at `:2705-2712`, `home_sign` reading `evm_nonce` at the
   same pin). The other signers of the address wait on `evm_pending`. So the only way this build reaches "nonce used, ours seen
   nowhere" is **our own transaction mined, and every provider returning neither its receipt at the pin nor the transaction by hash**:
   - a transaction index that lags, is rebuilt or is pruned on every 4663 endpoint (for instance several public endpoints fronting
     the same operator's nodes);
   - or a unanimous omission.

   The drop's designed case, a transaction signed at a used nonce, is now only the state an earlier build left wedged.
2. After the first unseen read, `used_since` is set. Reads that are `Unsure` (one provider shows the receipt and another does not) or
   `Pending` do not reset it. The first unseen read an hour or more later drops the transaction (PoC d: 3,660 s after the first
   unseen read, the next unseen read drops it, although for the hour between a provider showed it).
3. The step is taken back as if it had reverted, and redone:

| Step (mined, then dropped) | What the redo does | Guard claimed in §1 | Holds? |
| --- | --- | --- | --- |
| `Deposit` (Relay, 4663 → Ethereum) | `Funded` again with its watch dropped. The next quote is deposited again when the raw balance covers it. The first order fills on Ethereum, watched by nobody. | the balance check at a floored pin | **No.** The balance check is not idempotence. Any other ETH at the shared address pays the second deposit: the frog side's (B-L-6), a late payout, a stranger's. PoC a: 2 deposits, 1.0 ETH spent for one 0.5 ETH buy, one H3 value record. Without other ETH (control): the buy waits `Funded` for good, holding the Relay leg, and the first fill (0.498 ETH) is earmarked to no one. The operator's `eth_give_back` on `Funded` frees the leg but not the orphaned fill. |
| `GiveBack` of a sweep (`fundReserve`) | sent again | the contracts | **No.** `fundReserve` accepts any value above 0 again. PoC b: 0.6 ETH paid in for a 0.3 ETH sweep, the second 0.3 ETH out of other ETH at the address. |
| `GiveBack` of a buy (`returnAdvance`) | sent again | the contracts | **Stall.** The module reverts `NoAdvance` at the estimate, or the balance no longer covers it. The buy waits in `Returning` (in the Relay leg) with no operator exit. PoC e: after a day, Azuki never starts a buy, and `eth_give_back` refuses. |
| `Advance` | `NeedAdvance`, `advanced = 0`, the H2 reservation released | "a second `advance` reverts `AdvanceOpen`" | **No: orphaned.** The module's advance stays open and its ETH (up to the item cap: BAYC 7.35 ETH) sits at our address, tracked by no buy. On the real module the buy is re-advanced each tick and reverts at the estimate. It waits in `NeedAdvance`, which `in_relay` counts, so no collection buys again, and `eth_give_back` has no case for it (PoC c2). In the fake's own module (`maxItemCost` 0 in transit) the buy is dropped instead, and the advance is orphaned the same way (PoC c). Only a timelocked upgrade recovers either. |
| `Settle` | `Settling` re-signed | the estimate | Stall only: `settleAdvance` reverts `NoAdvance`, the buy never reaches `Settled`, and one estimate is spent a tick. It holds nothing. |
| `Record` | back to `Record` | "a second record `WrongStage`" | The ticket is `Delivered`, so it goes `Gone` (`:2875-2878`) and the held item is never released from the H2 book: one item of the collection's custody room lost for good (code reading, no PoC). |
| `Mark`, `Sent` (`noteSent`) | re-signed | the contracts | Harmless: both are idempotent in `EthNftWrapper.sol:408-425`. |
| `Post` | due again | — | Harmless. |
| `Mint`, `MintDeposit` | — | `itemOf` read in `mined_home` | **Holds**: the read turns a drop into `Minted`. |

**Impact.** A bounded loss per event: one deposit's value, one sweep's value, or one advance. Plus a stall of every collection's new
buys (not of burns, redemptions or floor posts) with no operator exit, until a timelocked upgrade. H3 counts the advance once and
nothing for the second deposit or `fundReserve` (the "outflow" of these steps was counted at the advance). The monitor sees a
`Funded` / `NeedAdvance` / `Returning` buy that does not move, not the cause.

**Likelihood.** Low. It needs every 4663 provider (and 2f+1 replicas each) to show neither the receipt nor the transaction, at two
reads an hour apart. One honest provider that shows the transaction on the drop tick prevents it. It is rated Medium because the
consequence, unlike the round-4 wait, is irreversible spending. It is also the only situation in which this build's drop fires: the
exit is correct only for the legacy wedged state.

**PoC** (`poc/F/r5_f.tests.rs`, output `poc/F/r5_f.output.txt`, all pass):

- `f5_m1_a_a_mined_relay_deposit_seen_nowhere_for_an_hour_is_deposited_again`: "Relay deposits 2, 4663 balance 1.5022 → 0.5022 ETH
  (spent 1.0 ETH for ONE buy of 0.5 ETH)"; one H3 value record.
- `f5_m1_a_control_without_other_eth_the_buy_holds_the_relay_leg_and_the_first_fill_is_nobodys`: after 3 days the buy is `Funded`
  (in the Relay leg), earmarked on Ethereum 0, and the first fill is unattributed.
- `f5_m1_b_a_mined_fund_reserve_seen_nowhere_for_an_hour_is_paid_again`: 2 `fundReserve` calls, 0.6 ETH paid in for a 0.3 ETH
  sweep.
- `f5_m1_c_a_mined_advance_seen_nowhere_for_an_hour_orphans_the_advance`: module transit open, 1 advance sent, no open buy, 0 tracked,
  0.5002 ETH at our address.
- `f5_m1_c2_on_the_real_module_the_taken_back_advance_holds_the_relay_leg_for_every_collection`: after a day Milady is `NeedAdvance`
  (in the Relay leg, "would not succeed now: execution reverted: AdvanceOpen()"), and Azuki never advances.
- `f5_m1_d_the_hour_is_not_reset_by_a_provider_that_shows_the_transaction`: dropped on the first unseen read 3,660 s after the first
  one, the hour between `Unsure`.
- `f5_m1_e_a_mined_return_advance_seen_nowhere_holds_the_relay_leg_for_good`: after a day Milady is `Returning` (in the Relay leg),
  Azuki has no buy, and `eth_give_back` is refused.

**Fix.**

- **Never take a step back on absence of evidence.** Before dropping, identify positively what used the nonce:
  - bisect `eth_getTransactionCount(us, block)` between the last pin at which the nonce was unused and the current pin (state is
    kept about 6,200 blocks back, so do it on the first unseen tick: about 12 reads);
  - read that block with its transactions;
  - compare the hash of our transaction at that nonce.
  - If it is ours, it is `Mined`: read its receipt at that block, or treat it as mined and let the step's own read-back decide. If it
    is another, drop.
- **Failing that, drop automatically only steps whose redo the chain makes idempotent** (`Post`, `Mark`, `Sent`, `Mint`,
  `MintDeposit`). For the others, read the state first:
  - `Advance`: the module's `transitUnits`;
  - `Settle`: the same, and whether the buy's item is wrapped;
  - `Record`: the ticket's stage, which releases the H2 book when `Delivered`;
  - `Deposit`: Relay's depository `Deposit` log for the order id at the pin;
  - `fundReserve`: the module's `ReserveFunded(us, ..)` logs since the signature;
  - `returnAdvance`: `transitUnits == 0`.
- **Where nothing can be read, wait and page**, with an operator endpoint that clears the pending transaction after looking (the home
  twin of `eth_line_clear`), as the round-4 fix proposed.
- **Reset `used_since` on every view other than "unseen with the nonce used"**, and require the unseen view on consecutive ticks
  across the whole window.
- **Give `NeedAdvance` and `Returning` an operator exit** that does not spend.

### F5-L-1 (Low): the Ethereum pin floor is one per canister, not per chain, and survives an upgrade that moves `eth_nft` to another chain; that chain's pins or nonces are then refused for good

**Where.** `pin.rs:99-104` (`PinFloors { home, eth }`: no chain id). `canister.rs:541` (restored whatever the configuration),
`:252` (`restore_init` keeps the argument's `eth_nft`, its `chain_id` included), `:533-538` (a chain move is refused only while a
line, a pending home transaction or an open buy exists). `ethnet.rs:142` (`admit_pin("Ethereum", ..)`), `:310`
(`admit_nonce("Ethereum", ..)`). `ethnft.rs:162-166` (any `evm_chains` entry but the home chain is a valid `eth_nft.chain_id`).

**What happens.** Moving the NFT side is an operator decision taken through a timelocked upgrade (for example to Base, or a rehearsal
install going between chains). After the move:

- **On a chain whose block numbers are lower** than the old floor (26,068,928 on Ethereum), every `EthNet::pin` is refused ("the pin
  9300000 is below block 26068928 ... C4-H-1"). The whole Ethereum side then reads nothing: no scan, no floor evidence, no delivery,
  no buy.
- **On a higher-numbered chain,** our address's nonce there is below the old one (40 → 0). `eth_nonce` is refused, and `resolve_eth`
  (`ethflow.rs:2394`) returns before its bump, cancel and supersede logic whenever a line transaction is not yet mined. An Ethereum
  transaction that does not mine promptly then waits for ever.

Only another code upgrade resets it: `PIN_FLOORS` has no endpoint. `EthState.last_mined` (`nonce_floor`, round 1) has the same
chain-blindness and would sign above the new chain's nonce (a gap). That half predates this round.

**Why Low.** It needs a privileged, unusual configuration change. The effect is a stall of the NFT side (holders' burns and
redemptions on 4663 are unaffected), recoverable by an upgrade.

**PoC.** `f5_l1_the_ethereum_pin_floor_is_not_keyed_by_chain_and_survives_a_chain_move` (a unit test over `PinFloors` plus source
checks of `post_upgrade` and `restore_init`). Both refusals are printed in `poc/F/r5_f.output.txt`.

**Fix.** Key each floor by chain id, and in `post_upgrade` reset any floor whose chain is not the configured one (with it
`last_mined` and `home_floor`, or refuse a chain move while `last_mined` is set). Alternatively, store the floors in the per-chain
state (`EthState` for the configured `eth_nft.chain_id`).

### F5-I-1 (Info): the other signers of the 4663 line still wait on F5-M-1's trigger, the opposite of what the NFT side now does

`looper.rs:320-328`, `redeem.rs:610-611` and `:1065-1066`, `frogflow.rs:2012-2016`: "nonce used, not visible: waiting rather than
guessing", with no exit (`fixes-round4.md` §10).

- With the floor, a lower pin cannot trigger them. The remaining trigger is a mined transaction of theirs that no provider shows: the
  condition on which the NFT side drops and redoes (F5-M-1).
- While they wait, `evm_pending` holds the whole line: no floor post, record or note for any collection.
- The wait ends when an index shows the transaction, so it is permanent only if the omission is.
- The two policies should be one, preferably the positive identification recommended in F5-M-1, applied to all four signers.
- `looper.rs:322-323`'s comment ("Only this loop signs for this key") is stale: the address is shared.

### F5-I-2 (Info): the first tick after upgrading from a pre-round-4 build has no floor

A state saved by `1eb5788` or earlier has no `pin_floors` and no `home_floor`, so both start at 0 (`canister.rs:541`: "the first tick
sets them"). On that one tick, a pin one bucket back is not refused.

- The NFT side now has its drop exit.
- The vault loop, the custodian and the frog side would wait for ever on a nonce they reused, which is the C4-H-1 wedge.
- It needs the fallback on exactly that tick, and a transaction of ours mined within about 100 to 200 s before the upgrade's stop.
  Negligible, but cheap to close: seed `PIN_FLOORS.home.nonce` at `post_upgrade` from the highest nonce the saved state knows
  (`EthState.pending`, the loop's `pending_evm`, the custodian's and the frog side's pending transactions) plus one.

### F5-I-3 (Info): the Ethereum signing nonce does not pass the nonce floor, contrary to §1's wording

- §1 says "a nonce read below the highest read is an error on both". Every Ethereum plan is signed at the nonce `EthNet::fees` reads
  directly (`ethnet.rs:200`), not through `eth_nonce` / `admit_nonce`.
- It is safe: the pin floor keeps that read at or above every pin already read, and the flow signs at `max(plan nonce,
  st.nonce_floor())` (`ethflow.rs:2150`).
- The second line exists only for `resolve_eth`'s check (`:2394`). Wording, or route `fees` through `admit_nonce`.

### F5-I-4 (Info): a refused pin is not cached, so every read of that tick asks for the pin again

- `CanisterIo::pin` caches only a successful pin (`canister.rs:1223-1231`). `EthNet::pin` likewise (`ethnet.rs:137-146`).
- `home_floor_ok` treats an `Err` from `home_pin` as a pass (`ethflow.rs:2691`, `_ => {}`).
- In a tick whose pin is below the floor, each later 4663 read re-runs `fast_pin`, and each Ethereum read re-runs `pin_on`. That is
  2 to 3 outcalls per provider per read, all refused. It is safe (nothing is read below the floor) but costs cycles on those ticks.
- Cache the refusal for the tick, and have `home_floor_ok` return it (the documented "a pin below it is a wait for the whole tick").

### F5-I-5 (Info): the tests hold the floor's arithmetic and the harmless drop, not the wiring or the dangerous drops

- `pin.rs` `c4_h1_*` tests `PinFloor` alone.
- Nothing holds that every 4663 read goes through `canister::pin`. A source-reading test would, as round 4 did for the H3 ask sites:
  no `rpc(` / `rpc_agreed(` with `pin::at` or `receipt_at_pin` outside `pin()`-taking functions. Nor that `evm_nonce` admits.
- `r4_c4_h1c_*` covers the drop for a `Post` only.
- No test drops a `Deposit`, `GiveBack` or `Advance`: the cases of F5-M-1.
- The r4 tests are meaningful for what they cover. `r4_c4_h1_a`, `h1b` (both with and without the world saying its pin) and `e4_m1`
  run the reviewers' attack shapes, and they would fail on the old code once adapted (it has no `home_pin`). I did not re-run them on
  the old code. `fixes-round4-poc/` shows the reviewers' own PoCs failing on this build.

## What holds

- **The pin floors.** Raised only by pins every provider agreed by number and hash (`fast_pin` / `pin_on`), after agreement. Never
  lowered; refused below.
  - A provider reporting a far-future head cannot raise the floor: `choose_pin` refuses a spread above one bucket and pins under the
    lowest.
  - A provider one bucket ahead changes nothing.
  - A lagging or flapping provider causes refused ticks, never a permanent refusal: the chain overtakes any agreed pin within about a
    bucket.
  - A changed provider set, or a larger bucket after an upgrade, costs at most a few ticks.
- **The nonce floors.** Monotone at monotone pins. The home one is shared correctly: one address, one line, every part's `evm_nonce`
  through `CanisterIo`.
- **One floor shared by the parts.** A later part's fresher pin can only make an earlier part's cached pin fail a read within the same
  tick, never a later tick.
- **The NFT side's own `home_floor`.**
  - It refuses a lower home pin at the tick's start.
  - `send_home` refuses below the next nonce past our last mined home transaction (`:2705-2712`).
  - It is raised on `Mined` (`:3135`) and on the drop (`:3158`) only.
  - The home line signs one raw transaction and looks up that one hash (no re-pricing, so no version mismatch).
- **The Relay leg on 4663.** Its reads are floored, and a refund returns the rise over the watch's start, capped at the value and the
  advance.
- **Mint and deposit mints.** A dropped mint is read back through `itemOf` and never minted twice.
- **Upgrades.** New fields decode from old states. `pre_upgrade` gained no trap. `post_upgrade` restores the floors after its
  refusals. A downgrade loses nothing it can read.
- **`nftnet`'s operator endpoints.** The only unfloored pin reads, as §10 says, and they never sign on the NFT side's chain.

## PoCs

`poc/F/`:

- `r5_f.harness.diff`: the three harness switches against `tests/audit_nft_r4.rs`.
- `r5_f.tests.rs`: the eight tests.
- `r5_f.output.txt`: the run.

To rebuild: copy `tests/audit_nft_r4.rs` to `tests/r5_f.rs`, apply the diff, append the tests, then
`cargo test --test r5_f f5_ -- --nocapture --test-threads=1`.

On `1dc2a61`: **8 passed** (each passing test shows the weakness; F5-M-1 a, a-control, b, c, c2, d, e; F5-L-1). The private copy
under `/tmp/rook-r5/F/` was removed.
