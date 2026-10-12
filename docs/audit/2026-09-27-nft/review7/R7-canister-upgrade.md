# Round 7, part E: upgrade safety of the round-6 state, the fixtures, the `.did`, and the H3 guard (E6-I-1)

Reviewer: independent AI re-review, 2026-09-28. Worktree `/Users/jlohayon/Documents/Rook-nft`, branch `nft-side` at `a78a542`
(delta `bb713fe..HEAD`, canister commit `2d747ba`).

**Scope:**
- the new saved fields:
  - `HomeTx.sent_at` (`ethflow.rs:492-495`);
  - `SaleScan.hole` / `lost` / `behind` / `rfail` / `raw` (`ethflow.rs:587-611`), their pruning (`:1194-1210`, `:1478-1551`,
    `:1587-1592`, `:1712-1730`);
- `post_upgrade`'s decode and `lost_opts` (`canister.rs:472-…`, `:507-520`);
- the fixtures `tests/fixtures/eth-state-round{2,4,5,6}.hex` and `state/eth-state-e04fa16.hex`;
- `etch_autobuyer.did` (`HomeTx`, `SaleScan`);
- the H3 guard `tests/audit_nft_r5.rs` `binding` / `h3_sites` / `h3_check` / `src_files` (`:1868-2038`) and `r5_e5_i2_…`
  (`:2041-2118`).

Toolchain 1.98.1. **Nothing in the worktree was modified except this report and `review7/poc/E/`.** PoCs ran in the part-F copy
`/tmp/rook-r7-F` (`git archive a78a542 autobuyer/canister-rs`; `diff -rq --exclude=target` identical before anything was added;
removed afterwards).

Severity scale and threat model: `../README.md`'s. **No canister is installed:** every upgrade question below matters for rehearsal
canisters and for the first real upgrade after launch.

## Verdict

- **Upgrade safety of the round-6 state: holds.**
  - Every new field is `opt` with `#[serde(default)]`, and no enum case was added.
  - Every historical `EthState` fixture byte string in git (16 of them, `24fba50` .. `2d747ba`, read with `git show`, not the
    re-encoded HEAD copies) decodes under HEAD. Each re-encodes with `lost_opts` empty, and the round-6 fields are absent in all but
    round 6's own (`poc/E/r7_e_old_fixtures.output.txt`).
- **The re-encoded fixtures hid nothing.** bb713fe's round-2 / -4 / -5 bytes decode to values EQUAL to HEAD's re-encoded fixtures. The
  bytes grew only by the new `opt` fields' type-table entries.
- **The e04fa16 fixture's provenance holds.** `tests/fixtures/state/eth-state-e04fa16.hex` is byte for byte `e04fa16`'s (and
  `bb713fe`'s) `eth-state-round5.hex` (sha1 `7d5ff674…`). e04fa16's own `r5_the_round_5_state_fixture_is_what_the_canister_encodes`
  held that file equal to its encoder, so it is what the round-5 build writes.
- **The `.did`:**
  - HEAD's `.did` is `service_compatible` with bb713fe's (identical to e04fa16's).
  - Every field of the round-6 / -5 / -4 / -2 fixtures is the same through the `.did` and through the Rust encoder, both ways
    (`poc/E/didcheck7.output.txt`). The control through the old `.did` names exactly the round-6 fields.
  - The new record fields are `opt`, so old clients (candid subtyping) ignore them and new clients read old answers as `null`.
  - The monitor's `candid.mjs` `HomeTx` reads `sent_at` as `Opt`.
- **E6-I-1 is closed as reported.** All five round-6 mutations are asserted caught in `r5_e5_i2_…`. My new ones:
  - caught: a helper-fn key, a macro key, a struct-field key, a closure;
  - **accepted: five others (E7-I-1, Info)**. None is present in the code.
- **No Critical, High, Medium or Low.**

## Counts (part E, open)

| Critical | High | Medium | Low | Info |
| ---: | ---: | ---: | ---: | ---: |
| 0 | 0 | 0 | 0 | 3 (E7-I-1, E7-I-2, E7-I-3) |

## What was checked

| Check | How | Result |
| --- | --- | --- |
| Old bytes → HEAD types | `r7_e_every_historical_eth_state_fixture_restores_under_head`: every commit's version of `eth-state-round{2,4,5,6}.hex` (`git log` of each file) | 16 / 16 decode; `lost_opts` empty; `pending.sent_at` and every `SaleScan` round-6 field absent (round 6's own: all present) |
| Re-encoding changed nothing | `r7_e_the_re_encoded_fixtures_hold_the_same_values_as_their_bb713fe_bytes` | round 2, 4, 5: `bb713fe` value == HEAD value (2,601→2,656; 2,840→2,896; 2,857→2,913 bytes) |
| Whole `(Saved,)` tuples of older builds | the fixer's `r4_states_saved_by_the_old_builds_…` (e04f49b, 4949da0, a37f54b, 1eb5788), `r5_…1dc2a61…`, `r6_the_state_the_round_5_build_wrote_…` re-run | pass (audit_nft_r4 22, r5 34, r6 16 + my 1) |
| The e04fa16 fixture | sha1 of `git show e04fa16:…/eth-state-round5.hex`, `bb713fe:` the same, and `state/eth-state-e04fa16.hex` | identical |
| `.did` vs encoder, `service_compatible` | `didcheck7` (review6's method, the round-6 fixture added) | ok on 4 fixtures; compatible with bb713fe's and e04fa16's `.did` |
| Growth of the new saved fields | reading | `hole`, `lost`: one range each, clipped to the window (`:1205-1206`). `behind`: a scalar. `rfail`: kept only for moves still queued in `pend` (`:1546`), so it is bounded by `SALE_TODO_MAX` (512). `raw`: evicted past `2 × SALE_BAD_MAX` (`:1719-1728`). `skipped` has no count cap now that it is never given up (E7-I-3) |

## E6-I-1: the H3 guard (closed as reported)

The round-6 guard (`h3_sites`, `:1907-1960`):
- reads every `outflow_check(` and `outflow_record(`;
- resolves a bare key to its nearest textual `let`;
- refuses `let mut` keys and any non-call use of the names;
- resolves `mint_key`'s scope;
- walks `src/` recursively.

`r5_e5_i2_…` asserts review 6's five mutations (1)-(5), and the test passes.

**New mutations tried** (`poc/E/r7_e_h3_guard.append.rs`, run on the real `src/` in memory; `r7_e_h3_guard.output.txt`):

| Mutation | Guard |
| --- | --- |
| a delivery key built by a helper fn `deliver_key(&d, io.now())` | caught ("not reviewed") |
| a key from a macro `deliver_key!(d)` | caught |
| a key read from a struct field `&k.key` | caught |
| an ask inside a closure `\|io, s\| io.outflow_check(s, …)` | caught (no `let s`) |
| **(A)** the real key per-time, and the reviewed `let key = format!("deliver:…")` repeated inside a closed `{ … }` block just before the call | **accepted**: `binding` takes the textually nearest `let`, not the one in scope |
| **(B)** a decoy reviewed `let key = …`, then `let key: String = format!(…, io.now())` | **accepted**: a type-annotated `let` is not a binding the guard reads |
| **(C)** `key_of`'s body (the scope of both mint keys and the advance key) made per-call | **accepted**: only `mint_key`'s body and the advance count are pinned |
| **(D)** `#[path = "../extra/ask.rs"] mod …;` in `lib.rs` | **accepted**: files outside `src/` are never read |
| **(E)** `include!("../extra/ask2.rs")` | **accepted** |

**E7-I-1 (Info):**
- The guard is a regression aid over text, not a boundary.
- (A) and (B) need a deliberately misleading edit. (C)-(E) are plausible refactors: a helper whose body changes, and generated or
  included code.
- None is in the tree today.
- **Recommended:**
  - pin `key_of` / `custodycap::evm_key` bodies like `mint_key`'s;
  - refuse `#[path` and `include!` in `src/`;
  - have `binding` refuse a key whose name has another `let` (typed or in a block) between the reviewed one and the call;
  - or, better, make the key a type (`H3Key`) whose constructors are the reviewed ones.

## Infos

- **E7-I-1:** the H3 guard's five residual mutations (above).
- **E7-I-2:** a `HomeTx` pending across the first upgrade to this build has `sent_at: None`, and the monitor's `eth-home-out`
  (`monitor/src/ethnft.mjs:142-143`: `sentAt > 0`) never pages it. Only `eth-home-unseen` does, and only once its nonce is used.
  - Relevant only to a rehearsal canister, or to an install that already has one out. The first install starts empty.
  - Fix: page from `used_since`, or from the first time the monitor saw that hash, when `sent_at` is absent. Or have `post_upgrade`
    stamp an absent `sent_at` with the upgrade time.
- **E7-I-3:** `SaleScan.skipped` is no longer given up (B5-L-2's fix), is pruned only by the window, and has no count cap, unlike
  `done_big` (64) and `bad` / `raw` (32 / 64).
  - Each entry is about 20 bytes. An attacker's oversized blocks add at most a few a pass, so a week is about 10^3 entries, tens of
    kB a collection: harmless for stable memory and `eth_nft_state`'s size.
  - But at most ONE skipped block is retried a pass (`:1412-1438`). With more due entries than passes left in the window, some are
    never retried before they leave it. Part B's question, recorded here for the bound.

## Round-6 items: verdicts (part E)

| ID | Verdict |
| --- | --- |
| E6-I-1 | **Closed as reported** (residuals E7-I-1) |
| Upgrade safety of `HomeTx.sent_at`, `SaleScan.hole/lost/behind/rfail/raw` | **Holds** (E7-I-2 is a monitor gap for a pre-upgrade pending transaction) |
| The re-encoded fixtures (round 2 / 4 / 5) | **Honest**: equal values to their bb713fe bytes |
| `state/eth-state-e04fa16.hex` | **Is** e04fa16's encoder output |
| `.did` | **`service_compatible`**, encoder-equal on all four fixtures |
