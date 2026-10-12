# Review round 7, 2026-09-28: the Ethereum NFT side after fix round 6 (branch `nft-side`)

Reviewer: independent AI re-review, 2026-09-28. Worktree `/Users/jlohayon/Documents/Rook-nft`, branch `nft-side` at `a78a542`.

**Read:**
- `../README.md`, `../review2/` .. `../review6/` (README, the part reports, `poc/`);
- `../fixes-round1.md` .. `../fixes-round6.md`, `../fixes-round6-poc/`;
- `docs/v7/phase1-nft.md` (with the §5.1 runbook), `docs/v7/phase1-nft-pooled.md`;
- the code of `git diff bb713fe..HEAD` (fix round 6: `2d747ba`, `7b9e8ba`, `b4c3f55`, `785b384`, `2841f90`, `a78a542`). Earlier
  code was read where a finding goes back further.

**Method:** as round 6. Six scoped parts, a provider probe (P) and the test bar (T). Every round-6 fix was attacked, and the fixer's
claims were checked against the code and against PoCs, not taken as given. One PoC per new finding where feasible, and a
recommended fix for each.

**No code was modified. The only files are these review documents and their PoCs.** Nothing was pushed, deployed or signed, and no
key was used.

- **Chains written to:** local anvils only (the dry run's fork of 4663, the Seaport fork proof's fork of Ethereum).
- **Network use (read-only):** the fork upstreams; JSON-RPC reads of public Ethereum and 4663 providers (`eth_blockNumber`,
  `eth_getLogs`, `eth_getBlockByNumber`, `eth_getTransactionReceipt`, `eth_getTransactionByHash`, `eth_getTransactionCount`,
  `eth_getCode`, `eth_call`); one read-only `golive-check --probe-depth` run of the committed pair; the reproducible build's crate
  fetch. **No `eth_sendRawTransaction` of any kind was made** (unlike round 6's probe).
- **PoCs:** every PoC ran in a private copy under `/tmp/rook-r7-<part>/`, one folder per part, checked with `diff -rq` against the
  worktree first and removed afterwards. The test bar ran in the worktree and changed no tracked file.

| Part | Report | Scope | PoCs |
| --- | --- | --- | --- |
| A | `R7-launch-plan.md` | A6-L-1 / A6-I-2 / A6-I-4 (`staged.mjs`, `argdiff.mjs`, `--proposal`), golive-check's provider allowlist and `--probe-depth`, the launch configuration as committed, the runbook | `poc/A/` (node, the golive CLI) |
| B | `R7-canister-scan.md` | B6-M-1 in the canister (classes, re-anchor, hole, `lost`, `behind`), B6-L-1, B6-L-2, B5-L-2; steering by provider words | `poc/B/` (cargo, on `audit_nft_r6.rs`'s fake world) |
| C | `R7-canister-wayback.md` | C6-L-1 (`RelayCallExecuted`), C6-L-2, C6-L-3, C6-I-1, D6-L-3's canister side | `poc/C/` (cargo; read-only depository logs on Ethereum and 4663) |
| D | `R7-services.md` | Monitor pages (D6-L-1/2/3, the scan pages), the runner's pending cadence (F6-L-1), the IDLs, the Trust page | `poc/D/` (node; a fake shallow RPC) |
| E | `R7-canister-upgrade.md` | Upgrade safety of `HomeTx.sent_at`, the new `SaleScan` fields, the fixtures, the `.did`; the H3 guard (E6-I-1) | `poc/E/` (cargo, `.did` check, 16 historical fixtures) |
| F | `R7-canister-home.md` | F6-L-1 (the search's reach), F6-L-2, D6-L-1 / D6-L-2 canister side | `poc/F/` (cargo; 4663 state-depth scan) |
| P | `R7-canister-scan.md` "Providers" | The committed pair re-probed | `poc/P/` |
| T | this file, "Test bar" | The full bar, the dry run and the reproducible build | `poc/T/test-bar.txt` |

Severity scale and threat model: as `../README.md`. A "passing" PoC shows the weakness is present; it turns red once fixed.

## Verdict

**Ready to launch by the bar (0 C / 0 H / 0 M), subject to the operator's pre-launch tasks below.** Round 7 finds no Critical, High
or Medium. **B6-M-1 is closed**: the scan no longer freezes after a gap, the committed pair is deep for logs and receipts (re-probed
today), and golive-check refuses an unmeasured chain-1 provider under `--phase1`. Every round-6 Low is closed except **F6-L-1, only
partly** (F7-L-1). Six new Lows, all bounded (a short or late sale sample, an operator wait, an unbooked refund of at most one order),
are recommended for a fix round but none blocks launch.

No path was found that makes the canister sign what its judges did not build, drains custody with nobody able to stop it,
double-counts a sale, or double-pays on a provider's word. Option (c)'s steps are recognised only when exact.

**Two preconditions are hard, not advisory:** the §5.1 IC-outcall rehearsal of the pair (Tenderly is IPv4-only, and every Ethereum
read of the canister needs both providers), and C5-I-1's real Relay fill and refund (see C7-L-1: a refund paid through the
depository cannot be credited as built).

## Counts of OPEN findings (distinct)

| Area | Critical | High | Medium | Low | Info |
| --- | ---: | ---: | ---: | ---: | ---: |
| A launch plan (new) | 0 | 0 | 0 | 0 | 3 |
| B sale scan (new) | 0 | 0 | 0 | 3 | 5 |
| C way back / payout judge (new) | 0 | 0 | 0 | 1 | 4 |
| D services (new) | 0 | 0 | 0 | 1 | 3 |
| E upgrade / H3 guard (new) | 0 | 0 | 0 | 0 | 1 |
| F home line (new) | 0 | 0 | 0 | 1 | 2 |
| Earlier rounds, carried open | 0 | 0 | 0 | 12 | (as their reports) |
| **Open** | **0** | **0** | **0** | **18** | **18** |

**Merged duplicates:** D7-I-1 is B7-L-3 (counted as B7-L-3, Low). B7-I-5 is D7-L-1 plus A7-I-3. E7-I-2 is D7-I-2. E7-I-3 is
B7-I-6. F6-L-1's residual is F7-L-1 (counted new).

**The 12 carried Lows:** A5-L-3, E5-L-2, C4-L-1 (now paged persistently), C3-L-1, E3-L-2, B3-L-1 (the block is now retried for the
window), B-L-6 (partly), A-L-2, B-L-5, C-L-5, D-L-3, D-L-6. B5-L-2 is closed. No contract changed (`git diff bb713fe..HEAD -- src/
test/` is empty).

## Round-6 findings: verdicts

| ID | Verdict | Basis, residuals |
| --- | --- | --- |
| **B6-M-1** | **Closed** | The pair (`rpc.mevblocker.io` + `gateway.tenderly.co/public/mainnet`) is configured only in the dry run's install argument (`scripts/golive-dryrun.sh:268`), golive-check's `DEEP` list (`script/golive-check.sh:891-892`) and `phase1-nft.md`; the monitor configuration holds a placeholder and `evm_chains` names no default. Re-probed: logs at 30 / 60 / 80 k blocks and non-null receipts at 30 / 60 k on both; both result caps classify `limit`; no honest answer classifies `deep`. The re-anchor removes the freeze (`r6_b6_m1_a` reproduces). The allowlist refuses look-alike hosts (`…mevblocker.io.evil.com`, trailing dot, IPv6 literal, other paths). Residuals: B7-L-1, B7-L-2, B7-L-3, D7-L-1, B7-I-2, A7-I-3 |
| **A6-L-1** | **Closed** | Eight near-steps refused and paged SEV1 by the monitor and `--proposal` (another Init field, a lowered mint limit, a trip-factor raise, a removed collection, a lowered one-item value, the global value 1 ETH off, replaced 4663 providers, the round-6 PoC); the exact steps 2 and 3 still EXPECTED. A6-I-2 closed under `--phase1` (residual A7-I-1) |
| **B6-L-1** | **Closed** for nulls and disagreements | Whole-hash key, 3 fast tries, 6–24 h back-off, ≤ 4 of 16 reads. `busy` bypasses it (B7-L-2) |
| **B6-L-2** | **Closed** as reported | `busy` never defers or splits; `done_big` only on a two-sided answer; the refinement within its bound; `raw` evicted last. B7-I-1: a rate limit that splits the replicas is the IC's divergence reject ("timeout"), still `TooBig`; measured, nothing lost (432/432 at 30 % and 50 %, `bad` ≤ 32) |
| **B5-L-2** | **Closed** | Never given up in the window; one due retry a pass inside the byte bounds |
| **C6-L-1** | **Closed** against the reported attacks; new path unreachable (C7-L-1) | Event confirmed: `RelayCallExecuted(bytes32 id, (address to, bytes data, uint256 value, bool allowFailure) call)`, topic0 `0x4be10945…c5ea`, `id` not indexed (21,065 real logs, one topic each), emitted only on a successful inner call. The stranger's gas-only call, a forged emitter, a record paying someone else, a failed `allowFailure` call and an address-like request id are refused |
| **C6-L-2** | **Closed** | No loop; the TTL bounds the wait (C7-I-2: the stale claim must be handed in again) |
| **C6-L-3** | **Closed** | No double count; no permanent H2 block (C7-I-4) |
| **C6-I-1** | **Closed** | `ethflow.rs:2803`, `:3601` |
| **D6-L-1** | **Closed** (canister and monitor) | `home_clear_applies` = exactly the steps with no readable effect; the monitor's wording matches it case for case |
| **D6-L-2** | **Closed** | `HomeTx` is built only in `send_home` (`ethflow.rs:3221`), `sent_at` in seconds, kept on resend; `eth-home-out` SEV2 30 min / SEV1 2 h, persistent. D7-I-2: a transaction signed by the round-5 build has none (transitional) |
| **D6-L-3** | **Closed** | "Held" = `!credited && !released` as `late_hold`; SEV1 72 h after the write-off, persistent |
| **F6-L-1** | **Partly** (F7-L-1) | The runner's clamps and IDL are right, and the new `near` branch cannot misidentify. But the reach is measured from the provider's head while the search starts at the fast pin, 1–2 k blocks under it: the margin is 90–240 s, not the ~320 s claimed |
| **F6-L-2** | **Closed** | Fake / hidden receipt, a nonce lie, lies during identification or about the effect, a wrong "another transaction took the nonce", a flapping pool entry: each ends in a wait, never a drop, redo or double pay |
| **E6-I-1** | **Closed as reported** | The five round-6 mutations plus a helper-fn key, a macro key, a struct-field key and a closure are caught. Five others pass (E7-I-1), none present |
| **Upgrade safety** | **Holds** | All 16 historical EthState fixtures in git (`24fba50`..`2d747ba`, the original bytes) decode under HEAD with the new fields absent and nothing lost; bb713fe's round-2/4/5 bytes decode to the same values as the re-encoded ones; `state/eth-state-e04fa16.hex` is byte for byte e04fa16's `eth-state-round5.hex`; the `.did` is `service_compatible` with bb713fe's and e04fa16's and matches the encoder both ways |

## New Lows

| ID | Where | What | Fix |
| --- | --- | --- | --- |
| **B7-L-1** | `autobuyer/canister-rs/src/ethflow.rs:1284-1289` (trim `:1305-1308`); `nftnet.rs:57` | The error class is computed per provider before the pair is compared, and `nftnet::ask` returns the first failing provider's error, so ONE provider's words pick the class (undoing audit-R3's "a provider's words never steer" for the scan). One `deep`-worded answer in the hole walk gives up the rest of the hole for good; the fixer's trim only runs in the walk back, below the hole. A lying provider can aim it (stall a range near the head ~7 h → re-anchor → one `deep` answer). PoC `r7_b_l1_a`: one `4444 "pruned history unavailable"` on a 12,000-block gap: 88 of 119 gap sales counted after two days (control 119). Paged (`eth-scan-lost`, SEV2); sales only uncounted, nothing false counted | Take `deep` only when both providers say it (or re-try the lost span at a slow cadence until it leaves the window); trim `lost` from the hole walk's own reads |
| **B7-L-2** | `ethflow.rs:1518-1522`; the walks `:1255`, `:1283`, `:1314`, `:1353` | `busy` has no bound anywhere: no `rfail` entry, no retry share, no back-off, no page. PoC `r7_b_l1_b`: 16 old receipts answered busy → 0 of 144 newer sales in two days, and no paged field set. An honest keyless quota ("quota exceeded" is in the busy list) does the same | Count `busy` like a failure for receipts (share, back-off); page consecutive busy passes |
| **B7-L-3** (= D7-I-1) | `ethflow.rs:1233` with `:1578`; `monitor/src/ethnft.mjs:18`, `:150` | `eth-scan-behind` practically never fires: the pass-start re-anchor keeps `behind` ≤ 7,200 at each pass end, the page needs ≥ 7,200, and a stopped scan never rewrites it. PoC `r7_b_l2`: three days all-busy, 0 of 216 sales, page true on 1 of 216 passes. It is the watch the fixer's call 1 relies on (the keeper's `floor-stale` catches a stopped scan only indirectly) | Page on the lag before the re-anchor, on re-anchor count, and on the age of the last completed pass |
| **C7-L-1** | `ethflow.rs:4296-4301`, `:4315-4325` | A real Relay payout from the depository can never be credited. `RelayCallExecuted.id` is the CallRequest's EIP-712 struct hash (recomputed from a real transaction), never our order id, and a real `execute` input carries only calls, a nonce, the expiry and the signature. PoC `r7_c_l1_a`: a real transaction retargeted to pay us is refused "does not name order". The fixer's tests credit only because their `relay_exec` puts the order id in a second topic, which the contract never emits. A depository-paid refund of a written-off order can only be released; its ETH sits unbooked (bound: one order's value; recovery needs a timelocked upgrade). Not a regression; in the sample, native depository payments go almost only to Relay's solver | Bind the credit through the CallRequest (Relay's nonce + the recomputed struct hash), or document that only solver transfers are creditable; fix `relay_exec` to the real encoding; confirm with C5-I-1 |
| **D7-L-1** | `script/golive-check.sh:913-915` `depth_probe`, `:1002-1006`; runbook `phase1-nft.md:576-580` | `--probe-depth` takes an empty `eth_getLogs` list 60,000 blocks back as proof of depth (the receipt half proves old transactions, not old logs). A provider answering `[]` past its log retention passes, and a passing probe admits an unlisted provider. In the canister, paired with an honest one, the deep ranges disagree and fail as `Other` (not `lost`, no page); if both truncate, old sales are silently uncounted. PoC `poc/D/fake_shallow_rpc.py` → `ok` | Probe a range known to hold logs (Milady's 10,000 blocks 60,000 back held 44), require ≥ 1 log and the same count on both; same in the runbook |
| **F7-L-1** | `canister.rs:1076-1083`; `ethflow.rs` `HomeTx.pin`; `ops/src/runner.ts` `sleepAfter` | Robinhood's provider keeps state 6,198–6,234 blocks under its own head (4663: 9.95 blocks/s, measured). `HomeTx.pin` is the fast pin, 1,024–2,047 (+ up to 1,024) blocks under the head, so the search works only if the first resolving tick lands within 417–519 s of signing; the real delay is 240 s + the runner's other steps + outcalls, a 90–240 s margin. The `near` branch helps only a transaction mined > ~400 s after signing and is not exercised by any test (the fake's pin is the head). PoC with the fixer's harness: 420 s and 540 s ticks → UNIDENTIFIED. Impact as F6-L-1, less often: a hidden Relay deposit or `fundReserve` waits for `eth_home_clear` | Record a start block near the head at signing, or read the nonce at `from` first, or re-tick right after a signing tick; a test of the real start-block arithmetic |

## Infos (see the part reports)

- **A7-I-1:** the runbook's step-2 / step-3 `--proposal` commands (`phase1-nft.md:560`, `:570`) omit `--phase1`; without it an
  extra Init change passes with a note (the monitor still pages SEV1). **A7-I-2:** `nft-pilot.json:12` `_custody` still says 61
  items / 75 ETH. **A7-I-3:** the allowlist ignores scheme and port (`http://`, `:8443` show "on the list"; the IC's https rule fails
  such a pair closed).
- **B7-I-1** (divergence reject is `TooBig`, harmless), **B7-I-2** (a hole's `lost` erased by the walk back's trim), **B7-I-3**
  (merged holes re-read; cycles only), **B7-I-4** (Tenderly's "beyond current head" is `limit`), **B7-I-6** (`skipped` has no cap;
  state stays tens of kB).
- **C7-I-1** (`to_us` sums every record paying us in the receipt, bounded by the order's value), **C7-I-2** (a claim waiting behind
  its buy's 4663 transaction goes stale after 600 s and must be handed in again: say it in the runbook), **C7-I-3** (a stranger's
  plain transfer ends the hold only at ≥ half the order's value), **C7-I-4** (a written-off, loss-closed order keeps one item of room
  and can refuse a deposit at the cap).
- **D7-I-2** (no `sent_at` on a round-5-signed transaction), **D7-I-3** (240 s is a sleep after the cycle, not a period; the variable
  accepts 301–600), **D7-I-4** (the Trust page's "a shorter sample pages" holds only for `deep`; `bad` / `raw` / `down` are not read).
- **E7-I-1** (five H3-guard mutations still pass: a decoy `let` in a closed block, a typed `let` after a decoy, a per-call `key_of`,
  `#[path]` / `include!` outside `src/`).
- **F7-I-1** (no test runs the canister's reach code), **F7-I-2** (the runner stays at 240 s while an unidentifiable transaction
  waits for the operator).
- **Outside the code:** Relay's keyless `/requests/v2` is throttled since 2026-09-01 and sunsets 2026-11-24; `/requests/v3` needs a
  key. Nothing in code calls it, but the runbook's release rule ("only on Relay's records") and C5-I-1 rely on the operator reading
  them.

## The fixer's judgment calls

| Call | Verdict |
| --- | --- |
| 1. MEV Blocker + Tenderly as the canister's pair | **Accept, with the IC rehearsal as a hard precondition.** Deep on both logs and receipts. Every Ethereum read needs both (`nftnet::one`); Tenderly is IPv4-only (the repo's own `docs/v4/eth-nft-canister.md:24-35` rejected it for that), so a failing IPv4 fallback on the subnet stops every Ethereum read. The send-side privacy is moot: the canister also sends to Tenderly, which broadcasts, and only the chain read-back decides. The named watch (`eth-scan-behind`) does not work yet (B7-L-3) |
| 2. Re-anchor at a day's lag, gap read newest first | **Accept**, with B7-L-1 and B7-L-3 fixed |
| 3. Credit a depository call only on `RelayCallExecuted` paying us | **Sound against spoofing, but the credit path is dead for a real Relay execution (C7-L-1).** C5-I-1's measurement decides whether it matters |
| 4. 240 s runner cadence while a 4663 transaction is out | **Accept** (evidence hand-ins stay at 1,200 s; only tick cycles). It does not by itself deliver the claimed margin (F7-L-1, D7-I-3) |
| 5. A disagreed receipt with its nonce used decided as "seen nowhere" | **Accept** (F6-L-2 closed) |
| 6. A credit waits while its buy has a 4663 transaction out; 600 s stale claims | **Accept** (C7-I-2: runbook note) |
| 7. Written-off orders keep custody room until credited or released | **Accept** (C7-I-4) |
| 8. Skipped blocks retried for the window | **Accept** (inside the day's byte bound; B7-I-6) |

## The launch configuration

| Setting | Verdict |
| --- | --- |
| Five collections, option (c) step 1 (48: 13 / 17 / 13 / 3 / 2 at 12 / 24 a day), steps 2 and 3 | Safe and enforced. Every step fits the exit rule (⌈22/12⌉, ⌈48/24⌉, ⌈61/31⌉ = 2) and ×4, and is recognised only when exact. The go/no-go read of the committed file passes (664 checks) |
| Deposits on (D5), H3 25 ETH/day trip ×3, D1–D9 | Unchanged; nothing unsafe found |
| `monitor-nft.json` placeholders | Unfilled placeholders make the check fail, not pass |
| The canister's Ethereum pair | Deep enough (B6-M-1 closed); rehearsal and `--probe-depth` at go-live (with D7-L-1's caveat) |
| C5-I-1 | Still a precondition, now also for C7-L-1 |

## Remaining operator pre-launch tasks

1. **The §5.1 rehearsal from a real IC outcall** on the launch subnet: a pinned block, a 500-block launch-collection `eth_getLogs`
   60,000 back **over a range known to hold logs** (D7-L-1), and a receipt there, through the canister's transforms on both
   providers: consensus, agreement, and Tenderly's IPv4-only host reached.
2. **At go-live:** `golive-check --phase1 --probe-depth --install-arg <arg>`, and the full go/no-go read (`--launch`, `--canister`,
   `--monitor-config`) at step 1. Check that the probe's log answer is non-empty.
3. **C5-I-1:** a real Relay fill and refund each way; confirm whether a refund is a solver transfer (creditable) or a depository
   `execute` (not creditable as built: C7-L-1). Use `/requests/v3` (key) for Relay's records.
4. **Fill `monitor-nft.json`** (timelock, buyer, proposers and guardians, the guardian, `install_arg_hex` from the ceremony log) and
   the monitor's Dwellir key.
5. **Runner env:** `RUNNER_PENDING_EVERY_S` ≤ 300 (default 240; lower is better given F7-L-1), `OPENSEA_API_KEY`, `ETH_NFT_SLUGS`,
   `ETH_LOGS_RPC`; measure the runner's cycle length (D7-I-3).
6. **Steps 2 and 3** only after step 1 executes and the handover is done: `--proposal … --stage N --phase1 --baseline <running arg>`
   (add `--phase1`, A7-I-1), announce (H6), no other buyer upgrade during its 72 h, update the file's `"stage"` afterwards.
7. **Cycles:** a top-up policy for ~52 T/day under attack and a burn-rate page.
8. Until B7-L-3 is fixed, watch the keeper's `floor-stale` and the scan note's `behind` by hand.

## Recommended order for round-7 fixes (none blocks launch)

1. B7-L-3 and D7-L-1 (the watches the launch relies on).
2. B7-L-1 and B7-L-2 (one provider's words; unbounded `busy`).
3. F7-L-1 (the search start near the head, with a test of the real arithmetic).
4. C7-L-1 after C5-I-1's measurement (and the test's `relay_exec` encoding).
5. Infos: A7-I-1 / A7-I-2 (runbook `--phase1`, `_custody` text), A7-I-3, C7-I-2 (runbook note), E7-I-1, D7-I-3.

## Test bar (re-run by this review at `a78a542`)

| Suite | Result |
| --- | --- |
| `forge test --no-match-path "test/fork/*"` | **1,115 passed**, 0 failed (126 suites) |
| Seaport fork proof (anvil fork of Ethereum at 26,055,690 via `eth.drpc.org`, read-only upstream) | **2/2 passed** |
| Canister `cargo test` (toolchain 1.98.1) | **699 passed**, 0 failed |
| Timelock `cargo test`, and `--features rehearsal` | 15 passed, and 15 passed |
| ops | 60 passed; typecheck clean |
| monitor | 214 passed |
| keeper | 150 passed; typecheck clean |
| indexer | 135 passed; typecheck clean |
| web-rook | 72 passed; typecheck clean |
| staging, tools/snapshot | 29 and 19 passed; typechecks clean |
| `scripts/golive-dryrun.sh stack` | **DRY RUN: ALL PASS** (146 on the v3–v6 stack; 536 on the five-collection NFT stack; **664 on the go/no-go read of the committed `nft-pilot.json`**, install argument on mevblocker + tenderly; 52 refusals `ok`, including the round-5 pair under `--phase1`; `--stage 2` and `--proposal` steps 2 and 3 pass, both-at-once and caps-first refused) |

Every number in `fixes-round6.md` §11 reproduced. `git diff 2d747ba..HEAD -- autobuyer/` is empty.

**Reproducible build:** `docker build --no-cache --platform linux/amd64` of `autobuyer/Dockerfile.build` on a `git archive` export
of HEAD. Hashes **identical** to `fixes-round6.md` §11:

```
00d2d63530fee23d98a762c5ce81657a3f8bda65e7dd9459deb9c110a28ed3f1  buyer.wasm   (7,076,857 bytes; 1,345,257 gzip -9 -n, < 2,000,000)
b23a190365d4c5b2bfc41bd9e9caf147bf2eba50ba3b3d477e2be372840edc41  timelock.wasm
2145099ea00214e8b521d456f6fc48145c7139f1ecd0c5643393c09c688e62f4  timelock-rehearsal.wasm
```

(The first attempt died when the Docker daemon exited under memory pressure; Docker Desktop was restarted and the build re-run from
scratch.)

**Removed afterwards:** the export, the build image and the extracted wasm; every private copy under `/tmp/rook-r7-*`; the dry
run's deployment files (removed by the dry run itself); the anvils. Docker's shared build cache was left alone, and nothing was
installed.
