# Round 6, part E: upgrade safety of the round-5 state, the H3 ask-site guard (E5-I-2), and the rest of the canister delta

Reviewer: independent AI re-review, 2026-09-28. Worktree `/Users/jlohayon/Documents/Rook-nft`, branch `nft-side` at `e04fa16`. The
delta read is `git diff 8ef82d6..HEAD -- autobuyer/canister-rs/` (fix round 5, `e811911`), with older code where a finding goes back
further. Scope:

- Upgrade safety: every new saved field; any new enum case in a saved type; the `.did` against the Rust types and against 8ef82d6's;
  the fixtures (`eth-state-round2.hex` / `eth-state-round4.hex` re-encoded, `eth-state-round5.hex`, `state/eth-state-1dc2a61.hex`);
  `post_upgrade`'s new traps and resets (`held_fits`, `chain_moved`); a large-state decode probe.
- The rewritten H3 ask-site guard (`tests/audit_nft_r5.rs` `r5_e5_i2_…`, E5-I-2).
- The rest of the delta not owned by another part: `evmsend.rs` (`our_txs_body`), `relayeth.rs` (`EthDeposit.request_id`),
  `ethnft.rs` (`SENT_TX`, `UNDELIVERABLE`, `ADVANCED`, `decode_ticket_tx`), `canister.rs` (`our_txs_view`, `nonce_at`),
  `ethnet.rs` (`eth_late_release`, `eth_home_clear`: access control only; their semantics are parts C and F).

Toolchain 1.98.1. `src/nft/EthNftWrapper.sol`, `src/nft/NftWrapper.sol` and `src/floor/FloorModule.sol` were read for the getters
the new selectors call.

**Nothing in the worktree was modified except this report and `review6/poc/E/`; nothing committed, pushed, deployed or signed.** Every
PoC ran in a private APFS clone at `/tmp/rook-r6-E/` (checked with `diff -rq` against the tree: only the appended PoCs differ), and the
old build in a `git archive` export of `1dc2a61` there. Both were removed at the end.

Severity scale: the README's.

## Verdict

**Upgrade safety holds. E5-I-2 is partly closed. No Critical, High, Medium or Low in this part.**

- **Upgrade safety:**
  - Every new saved field is `Option` with `#[serde(default)]`: `Watch.request_id` / `home_pin`, `LateOrder.released_at`,
    `HomeTx.pin`, `Inbox.home_clear`, `SaleScan.done_big` / `stuck` / `spent`.
  - No enum case was added to a saved type. A variant-by-variant comparison of every `pub enum` in the Ethereum modules, 8ef82d6
    against HEAD, finds one new enum, `ScanErr`. It is not saved (no `Deserialize`, used only inside `scan_read`).
  - `EthDeposit.request_id` has no `serde(default)`, but `EthDeposit` is not saved (no `Deserialize`; it is copied into `Watch`).
  - The `.did` is service-compatible with 8ef82d6's. Every field of the round-5, round-4 and round-2 fixtures is the same through the
    `.did` and through the Rust encoder, in both directions. A control run against 8ef82d6's `.did` does name the round-5 fields.
  - A whole `(Saved,)` written by the 1dc2a61 build, with every round-4 Ethereum field and `Inbox.payouts`, restores in HEAD:
    `lost_elements` and `lost_opts` are empty and the round-5 fields are absent. No such whole-state fixture from 1dc2a61 existed
    before; the fixer's test uses the `EthState` alone.
  - `state/eth-state-1dc2a61.hex` is genuine. 1dc2a61's own fixture test passes on a `git archive` export of 1dc2a61, and the file is
    byte-identical to that commit's `eth-state-round4.hex`.
  - The re-encoded round-2 and round-4 fixtures hold the same values as 8ef82d6's bytes, with nothing lost.
  - A 519 kB round-5 state at the new bounds decodes in 23 ms (debug). The bounds are 5 scans with 32 deferred ranges, 64 remembered
    blocks and 512 todo each, 400 buys with late orders, and 64 claims.
- **`post_upgrade`:** the new trap (`held_fits`) and the reset (`chain_moved`) block no upgrade the operator needs.
  - An upgrade that keeps the outflow limits always passes `held_fits`, since custody was admitted under caps that fit those limits.
    That includes a caps lowering, a security fix and a collection removal.
  - Only an outflow lowering is refused, and only while custody holds more than the lower limits can let out. That is the fixer's
    stated call 9, and H1 (the pause) covers the emergency.
  - One residual: E6-I-2.
- **E5-I-2:** the new guard catches review 5's four mutations and C3-H-1's shape, as claimed. It still accepts five other regressions
  of the same kind (E6-I-1). The code today is keyed correctly: every `outflow_record` site uses the same binding as its
  `outflow_check`, and there is no `OUTFLOW.with(... check/record ...)` outside `canister.rs`. So this is a guard gap, not a defect.
- **Selectors and access control:**
  - `SENT_TX` = `sentTx(uint256)` (`EthNftWrapper.sol:104`, `bytes32`).
  - `UNDELIVERABLE` = `undeliverable(uint256)` (`:101`, `bool`).
  - `ADVANCED` = `advanced()` (`FloorModule.sol:237`; set at `:705`, zeroed at `:722` / `:742`, so it is 0 when no advance is open).
  - `decode_ticket_tx`'s word 8 is `NftWrapper.Ticket.deliveryTx`: stage, owner, round, pinnedAt, pinnedAtBlock, deliverBy,
    deliverByBlock, itemId, then **deliveryTx**, then destination.
  - `eth_late_release` (`ethnet.rs:604`) and `eth_home_clear` (`:615`) are `only_operator()`, then `not_paused()`, then
    `not_ticking()`. The source test counts 10 guarded updates.
  - `our_txs_view` is a transform query like the others: public and pure.

## Counts (this part)

| | Critical | High | Medium | Low | Info |
| --- | ---: | ---: | ---: | ---: | ---: |
| New | 0 | 0 | 0 | 0 | 2 (E6-I-1, E6-I-2) |
| Carried, re-rated here | | | | E5-L-2 (unchanged) | E5-I-1 (unchanged) |

## Round-5 items in scope: verdicts

| ID | Verdict | Basis |
| --- | --- | --- |
| **E5-I-2** (H3 ask-site test a string count) | **Partly closed** | `r5_e5_i2_…` reads every `src/*.rs` at test time, strips comments, matches `outflow_check(` with any argument, resolves an identifier key to its nearest `let`, and pins `mint_key`'s body and the advance's `n`. It catches review 5's (a)–(d) and C3-H-1's shape. Residual: **E6-I-1** |
| **E5-I-3** (the round-4 fixture lacked `Inbox.payouts`) | **Closed** | `eth-state-round5.hex` holds a claim and every round-5 field (the `.did` check above); the 1dc2a61 whole-state dump here also carries one and restores |
| **F5-L-1** (the Ethereum floors survive a chain move), upgrade half | **Closed for a one-step move** | `post_upgrade` (`canister.rs:571-576`) resets `floors.eth` and `last_mined` when the saved and new `eth_nft.chain_id` differ. Residual: **E6-I-2** (a two-step move) |
| **A5-L-1** (`held_fits` at every upgrade), upgrade-safety half only | **Holds; bricks nothing needed** | See the verdict. Part A owns its semantics |
| **E5-L-2** (the exit rule ignores the tripwire) | **Unchanged, Low** | Not fixed (`fixes-round5.md` §7). The launch steps keep the margin: per collection 22 + 3 < 36, and in all 61 + 6 < 93 at step 3 (12 / 31, ×3). Only the stage-0 defaults with deposits on, or a trip factor of 2, reach it |
| **E5-I-1** (a reverted mint's record keeps its send time) | **Unchanged, Info** | `outflow.rs` `record` is unchanged in the delta |
| **E5-L-1** (a misread deposit mint wedges its collection) | **Closed** (spot-checked; part F owns the drop path) | `ethflow.rs:3437-3446`: a failing mint whose token is on the wrapper from its own transaction (`wrapper_holds(.., Some(&d.tx))`) is recorded under the same `mint_key` and set `Minted`. The fixer's re-run of review 5's PoC fails on its attack assertion |

## Findings

### E6-I-1 (Info): the semantic H3 guard still accepts five regressions of the kind it exists to catch

- **Where:** `autobuyer/canister-rs/tests/audit_nft_r5.rs`:
  - `:1866`: only the text `outflow_check(` is an ask;
  - `:1834-1859`: `binding` takes the nearest `let` and nothing after it;
  - `:1947-1954`: `src_files` reads `src/*.rs` only, not sub-directories;
  - `h3_check` `:1919-1945`: it never reads `outflow_record`, and resolves one level only.
- **What it accepts** (`poc/E/r6_e_h3_guard.append.rs`; each mutated source passes `h3_check`):
  1. **The deposit mint's `col` made per deposit** (`ethflow.rs:3425`: `let col = format!("{}:{}", key_of(&c, chain), h(&d.tx));`).
     The text `mint_key(st, &col, &d.collection)` is unchanged, but the key is per deposit (E4-H-1's shape). Worse, the
     per-collection mint scope is per deposit too, so `per_collection_mints` never binds. The guard does not resolve `col`, or any
     argument of a helper call.
  2. **The delivery key mutated after its `let`** (`ethflow.rs:2448`: `let mut key = …; key.push_str(&format!(":{}", io.now()));`):
     it is per tick, so every waiting tick is a new ask toward the tripwire. That is the E4-H-1 / C3-H-1 class.
  3. **The record side** (`ethflow.rs:2469`: `io.outflow_record("deliver", &col, Flow::Item, 1)`). `record` is idempotent per key
     (`outflow.rs`), so with one constant key only the window's first delivery counts as gone out, and the item limits never bind.
     This is the dangerous direction for H3, and the guard reads no `outflow_record` site.
  4. **A new ask through a path call**, `(I::outflow_check)(io, &k, …)` with a per-call key: `outflow_check)(` is not
     `outflow_check(`. A `use … as` alias passes the same way.
  5. **A new ask in `src/<dir>/mod.rs`:** only top-level files are read.
- **Why Info:** the code at `e04fa16` has none of these. I checked each `outflow_record` against its `outflow_check` binding. The
  only record without a check at its own site is `adopt_advance` (`ethflow.rs:3613`). It records what already went out, under the
  advance key, which is idempotent with the original. The only direct limiter access is in `canister.rs`. The weakness is in the
  guard that is meant to keep it so.
- **Fix:**
  - Read `outflow_record(` as well, and require each record's key to be the same expression or binding as the check it follows.
  - Resolve every identifier inside a key and inside a listed helper call's arguments (`col` for `mint_key`) to its binding, and
    refuse a `let mut` key binding.
  - Walk `src/` recursively.
  - Refuse any occurrence of the identifier `outflow_check` that is not a call or a declaration (paths, `as` aliases).
  - Or, better than any text guard, make the key a type: an `OutflowKey` built only by one constructor per logical outflow, with
    `outflow_check` and `outflow_record` taking it.

### E6-I-2 (Info): a chain move in two upgrades keeps the old chain's Ethereum floors

- **Where:** `ethflow.rs:4200-4209` (`chain_moved`), called at `canister.rs:572-573` with `from` = the saved `Init`'s `eth_nft`
  chain.
- **What:**
  1. One upgrade drops `eth_nft`. That is allowed when nothing is open (`stranded_by`, and the busy check at `:561`). The state,
     with `pin_floors` and `last_mined`, is kept: `from = Some(1)`, `to = None`, no reset.
  2. A later upgrade adds it back on another chain: `from = None`, `to = Some(b)`, no reset.
  3. Chain 1's pin floor (about 26 M) and nonce floor now apply to chain `b`. Every pin of a chain whose head is lower is refused,
     and so are its first nonces. The Ethereum side does nothing.
- **PoC:** `poc/E/r6_e_chain_moved.append.rs` (`r6_e6_i2_…`).
- **Why Info:**
  - It needs two timelocked upgrades that move the NFT side off Ethereum, which is not in any plan.
  - It fails closed: refused reads, no spend.
  - Another upgrade that moves the chain again recovers it.
- **Fix:** save the chain the floors belong to with them (for example `PinFloors.eth_chain`) and reset whenever it differs from the
  configured chain, or reset when `to` is `None`.

## What holds (checked, no finding)

- **Saved types:** see the verdict. The only new `pub enum` is `ScanErr`; `Got` is private and unsaved.
- **The `.did` (`poc/E/didcheck6.*`):**
  - `service_compatible(HEAD, 8ef82d6)` returns `Ok`.
  - The two new methods are `(nat64) -> (Result)` and `(nat64, bool) -> (Result)`.
  - `Inbox.home_clear : opt record { nat64; bool; nat64 }` matches `Option<(u64, bool, u64)>`.
- **Old states:** the whole 1dc2a61 `(Saved,)` and the pre-round-5 fixture bytes restore with nothing lost (`poc/E/r6_e_restore.*`,
  `r6_e_old_fixtures.*`). An old pending `HomeTx` restores with `pin: None`, which `home_nonce_tx` handles by searching
  `HOME_NONCE_SEARCH_BLOCKS` back. Part F owns whether that depth is enough.
- **`held_fits` at `post_upgrade`** (`canister.rs:331-347`, `:528-534`):
  - It is evaluated on the kept `Init`, after `check_cap_change`.
  - Held counts every book entry, reservations included, whatever its collection.
  - It never refuses an upgrade that keeps the outflow limits (the invariant in the verdict).
  - A refusal traps atomically: the running code and state stay.
- **`our_txs_body`** (`evmsend.rs:139-152`):
  - It filters `from` case-insensitively and projects only `[hash, nonce]` lower-cased.
  - `null` stays `null`, and an unusable reply becomes a fixed error body.
  - Providers differing in anything else of the block cannot disagree through it.
  - Not checked here: part F owns whether the 2,000,000-byte cap on a full `eth_getBlockByNumber(n, true)` of 4663 is enough in a
    busy block (`canister.rs:1068`). If it is not, `home_nonce_tx` errs and the step falls back to its effect read, or waits.

## PoCs (`review6/poc/E/`, see its README)

| PoC | Shows | Result |
| --- | --- | --- |
| `r6_e_h3_guard.append.rs` | E6-I-1: five mutations accepted | passes (weakness present) |
| `r6_e_chain_moved.append.rs` | E6-I-2 | passes (weakness present) |
| `dump_saved_1dc2a61.append.rs` → `saved-1dc2a61-rich.hex` | a whole state written by 1dc2a61; 1dc2a61's own fixture test | ok |
| `r6_e_restore.append.rs` | that state restores in HEAD; the large-state probe | ok |
| `r6_e_old_fixtures.append.rs` | the pre-round-5 fixture bytes decode losslessly | ok |
| `didcheck6.main.rs` | `.did` vs 8ef82d6 and vs the Rust encoder, with a control | ok |

The whole `audit_nft_r5` test file with these PoCs appended: 38 passed (the fixer's 33 and these 5).
