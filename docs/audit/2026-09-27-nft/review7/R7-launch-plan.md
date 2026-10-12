# Round 7, part A: the launch plan, the exact staged step and the provider gate

Reviewer: independent AI re-review, 2026-09-28. Worktree `/Users/jlohayon/Documents/Rook-nft`, branch `nft-side` at `a78a542`
(delta `bb713fe..HEAD`: `7b9e8ba` monitor, `b4c3f55` and `2841f90` go-live, the launch file).

**Scope:**
- A6-L-1, A6-I-2, A6-I-4: `monitor/src/staged.mjs` (`offStep` `:56`, `overFour` `:76`, `kindProblem` `:104`, `checkStagedProposal`,
  `stagedStep` `:163`, `stepTags`), `monitor/src/argdiff.mjs` (`diffInit` tags, `proposalAlert` `:269`, `own` `:294`),
  `monitor/src/watcher.mjs`, `script/golive-check.sh --proposal` (`:90-127`);
- B6-M-1's gate: `script/golive-check.sh` `DEEP` (`:890`), `depth_probe` (`:896`), the chain-1 provider loop (`:988-1012`);
  `scripts/golive-dryrun.sh` (the new pair, the old pair refused);
- the launch configuration as committed: `deployments/launch/nft-pilot.json`, `deployments/launch/monitor-nft.json`,
  `docs/v7/phase1-nft.md` §4.1 and §5.1, `docs/v7/phase1-nft-pooled.md`.

**Nothing in the worktree was modified except this report and `poc/A/`. Nothing was committed, pushed, deployed or signed.**

- **Where the PoCs ran:** a private copy at `/tmp/rook-r7-D` (`monitor/`, `ops/`, `script/`, `scripts/`, `deployments/`, the
  canister's fixtures and `.did`). `diff -rq` showed it identical to the worktree before anything was added. It was removed
  afterwards.
- **Network use:** one read-only `--probe-depth` run of the committed pair (2026-09-28 13:21 UTC; `poc/D/probe_depth_live.output.txt`).

Severity scale and threat model: `../README.md`'s. A passing "WEAKNESS" assertion shows the weakness is present.

## Verdict

- **A6-L-1 is closed.** A staged step is recognised only when it is exactly the step, by both the monitor and
  `golive-check --proposal`. Every near-step I built was refused and paged SEV1. That covers another Init field, a lowered mint
  limit, a trip-factor raise, a removed launch collection, a lowered one-item value, a global value one ETH off the sum, and replaced
  4663 providers (`poc/A/r7_a.test.mjs` test 2).
- **Things that do not bypass it:**
  - **Cap-key casing.** The canister accepts only lower-case canonical keys (`custodycap.rs` `canonical_key`) and refuses duplicates.
  - **`evm_chains` `None` versus `Some([])`.** Both mean "no chains" in the canister (`unwrap_or_default`), so treating them alike
    is correct.
  - **The ×4 boundary.** It matches `validate_change` (`raise_ok`: `new <= old × 4`; a new key is held to `first_of`, which
    `offStep` pins to the file's numbers anyway).
  - **Large wei amounts in the launch file.** They parse exactly (15 and 25 ETH are representable as doubles). A value that is not
    would make the step's numbers differ from the Init's, which fails closed.
- **A6-I-4 is closed** (`overFour`; own lines are never downgraded past ×4).
- **A6-I-2 is closed for `--phase1`**, but the runbook's documented `--proposal` commands do not pass `--phase1` (**A7-I-1**).
- **The provider gate (B6-M-1 fix 4) holds against look-alike hosts.**
  - `rpc.mevblocker.io.evil.com`, a trailing dot, an IPv6 literal, and MEV Blocker's other paths (`/fullprivacy`) all FAIL under
    `--phase1`.
  - Userinfo and a query are caught by the keyless rule.
  - It ignores the scheme and the port (**A7-I-3**).
  - `--probe-depth` fails closed on errors, a `null` receipt and a missing block. It accepts an empty log list, so a provider that
    silently truncates its log history passes (**D7-L-1**, reported in `R7-services.md` because the fix is the probe's).
- **The launch configuration as committed is safe.** No new finding above Info. One stale sentence (**A7-I-2**).

## Counts (part A, open)

| Critical | High | Medium | Low | Info |
| ---: | ---: | ---: | ---: | ---: |
| 0 | 0 | 0 | 0 (carried: A5-L-3, E5-L-2) | 3 |

## Round-6 items: verdicts

| ID | Verdict | Basis |
| --- | --- | --- |
| **A6-L-1** | **Closed** | `offStep(…, exact)` holds the proposal's global caps to the sums of its caps (`staged.mjs:66-70`). `kindProblem` refuses any non-launch cap key moving (`:113-114`), `overFour` any raise past ×4, and `proposalAlert` downgrades only the tagged lines the step names (`argdiff.mjs:294-299`; `watcher.mjs` passes `stepTags`). The reviewer's PoC (a ×4 Bitcoin cap, global caps above the sum, a new key) is refused, and so is every near-step in `poc/A/r7_a.test.mjs` test 2. The exact steps 2 and 3 remain EXPECTED (SEV2) and pass `--proposal` (test 1) |
| **A6-I-2** | **Closed under `--phase1`** | `checkStagedProposal` `:148` FAILs without a baseline when `requireBaseline`. `golive-check` (`:104-108`) falls back to the monitor configuration's `install_arg_hex` when it is real Candid. The committed `monitor-nft.json` still has a placeholder there, so it fails closed. Residual: **A7-I-1** |
| **A6-I-4** | **Closed** | `overFour` (`staged.mjs:76-88`) mirrors `custodycap::validate_change` and outflow's ×4. `trip_factor` is excluded, but any trip-factor move already fails `kindProblem` (test 2, "trip factor 3->12") |
| **D6-I-3** | Closed (= A6-L-1) | |
| **The `_outflow` text** | **Closed**; a sibling sentence is stale | **A7-I-2** |

## The launch configuration as committed

| Setting | Verdict |
| --- | --- |
| Five collections (`nft-pilot.json`), `maxPurchaseWei`, BAYC's J1 numbers | Unchanged since round 6; held by `golive-check --launch` (the dry run, part T) |
| Option (c): step 1 = 48 (13 / 17 / 13 / 3 / 2) at 12 / 24 a day; step 2 = `global_items` 31 alone; step 3 = J7's 61 alone | **Safe.** It fits the exit rule at every step: ⌈22/12⌉ = 2, ⌈48/24⌉ = 2 and ⌈61/31⌉ = 2, each within the two usable days. Every step is ≤ ×4 (48 → 61). Each step is recognised exactly, and nothing else can ride along (above) |
| Deposits ON (D5), H3 25 ETH/day, trip ×3 | Unchanged; the operator's decisions. A-M-1's gate, C-H-1's shared limit and their residuals are as earlier rounds |
| D1–D9 | Unchanged. D6 (the canister's providers) now names MEV Blocker + Tenderly (`phase1-nft.md` §4.1), and golive-check holds it |
| The canister's Ethereum pair | On the measured list. `--probe-depth` passes both today (13:21 UTC, read-only). Still needs the operator's IC-outcall rehearsal (Tenderly's IPv4-only host; consensus through the transforms) |
| `monitor-nft.json` | Placeholders (`timelock`, `buyer`, guardians, `install_arg_hex`, the guardian address) are the operator's to fill at launch. Under `--phase1 --proposal`, an unfilled `install_arg_hex` fails closed |
| `phase1-nft-pooled.md` | No change this round and nothing inconsistent with the file |

## Infos

- **A7-I-1 (the runbook's `--proposal` commands omit `--phase1`).**
  - `docs/v7/phase1-nft.md:560-562` (step 2) and `:570` (step 3) give
    `script/golive-check.sh --proposal <hex> --launch … --stage 2 --canister <buyer>`, with the baseline in brackets.
  - Without `--phase1`, `requireBaseline` is false, so a proposal that is step 2 plus a new `operator` and new 4663 `rpc_urls` passes
    with a note (`poc/A/r7_a.test.mjs` test 3).
  - The monitor still pages it SEV1, and with `--phase1` it FAILs.
  - **Fix:** put `--phase1` in both runbook commands, or require the baseline whenever the launch file is at a staged step.
- **A7-I-2 (stale text).** `nft-pilot.json:12` `_custody` still says the install's global caps are "61 items, 75 ETH". At step 1 the
  install's global item cap is the sum, 48. The data and `_outflow` are right. **Fix:** say 48 at step 1, and 61 after step 3.
- **A7-I-3 (the measured list ignores the scheme and the port).**
  - `golive-check.sh:995-1001` matches on the host and path only, so `http://rpc.mevblocker.io` and `https://rpc.mevblocker.io:8443`
    print "on the measured list" (`poc/A/allowlist.output.txt`).
  - The canister does not require `https://` for `evm_chains` URLs. The IC's outcalls do, so such a pair would fail every Ethereum
    read after install.
  - This is an operator-mistake gate, not an attack.
  - **Fix:** require `scheme == "https"` and no explicit port (or 443) for an entry to count as listed.

## PoCs (`poc/A/`)

- `r7_a.test.mjs` / `r7_a.output.txt`: the exact steps (control), eight near-steps refused, and A7-I-1 (3/3 pass).
- `allowlist.sh` / `allowlist.output.txt` / `mkhex.sh`: golive-check's `INSTALL_PY`, extracted verbatim, under `--phase1` on 12 URL
  variants of the pair.
