# The round-7 reviewers' PoCs, re-run on fix round 7

Each part's PoCs (`../review7/poc/`) were rebuilt as their READMEs say, on the fix-round-7 code (`3300c96`), in private copies under
`/tmp/rook-fix7/poc`. The copies were checked with `diff -rq` (canister) or made by `git archive` (monitor, ops, scripts), and removed
afterwards.

- **The canister PoCs** are `tests/audit_nft_r6.rs`'s fake world, as the reviewers ran them:
  - part B with `r7_b.vs_audit_nft_r6.diff` applied;
  - part C's `r7_c.rs.snippet` and part F's `r7_f_reach.append.rs` appended.
- **The node PoCs** (parts A and D) ran against an export of the tree.

A PoC that FAILS shows its weakness is gone.

| File | Result |
| --- | --- |
| `B.review7-pocs-on-round7.output.txt` | All four B PoCs **fail** on their weakness assertions. **B7-L-1:** after one `deep` answer in the hole, 119 of 119 gap sales are counted (was 88) and `lost` is None. **B7-L-2:** 16 busy receipts no longer hold the newer sales (144 of 144, was 0). **B7-L-3:** three days all busy give `behind` up to 21,598 (was at most 7,200), and 144 passes reach the old 7,200 condition (was 1). **B7-I-1:** the IC's divergence reject is now `Busy`, not `TooBig` |
| `CF.review7-pocs-on-round7.output.txt` | **C7-L-1 fails:** the real-shape depository refund with the amount stated is `Ok(4.9 ETH)` (was refused as "does not name"). Unstated it is still refused: nothing on chain names the order, so the operator names it. **C7-I-1 fails:** the reviewer's two-execution input is not the depository's `execute`, so it is refused; `audit_nft_r7.rs` `r7_c7_i1_…` shows a real two-call request is credited one record at a time. **C7-I-2 passes:** an Info, accepted; the runbook now tells the operator to hand the claim in again. **`r7_f_reach_…` passes** by construction: its fake's pin is the head and its `home_nonce_tx` ignores the canister's start choice (F7-I-1). `audit_nft_r7.rs` `r7_f7_l1_a_…` runs the canister's own start function on a fast-pin fake: identified at 420 s and 500 s |
| `AD.review7-pocs-on-round7.output.txt` | **Monitor, A7-I-1:** passes at the JS level. It calls golive-check's `--proposal` path without `--phase1`, which is unchanged; the runbook's commands now carry `--phase1` and the baseline. **Monitor, D7-I-2:** passes because it reads one poll; the age now counts from the monitor's first sighting, so a second poll pages (`monitor/test/audit_nft_d_r7.test.mjs`). **Monitor, D7-I-1:** passes because it calls `ethNftAlerts` without the Ethereum head; `checkEthNft` now passes the head, and the lag is measured against it (`audit_nft_d_r7`). **Ops, D7-I-3:** the cosmetic PoC **fails**: `0x3c`, `6e1` and 301–600 are refused |
| `A.allowlist-on-round7.output.txt` | **A7-I-3:** `http://rpc.mevblocker.io` and `https://rpc.mevblocker.io:8443` now FAIL under `--phase1` (were "on the measured list"). The other ten URL variants are as before |
| `D/probe_depth_pinned.py`, `D/probe_depth_pinned.output.txt` | **D7-L-1**, the reviewer's shallow provider turned around. That provider (`[]` past 10,000 blocks) FAILs, and so do a short answer and a null receipt. An honest fake passes, and so does the live pair (read-only) |
| `didcheck.main.rs`, `didcheck.output.txt` | Review 7's `didcheck7` with the round-7 fixture. HEAD's `.did` is `service_compatible` with `a78a542`'s. Every field of the round-7, -6, -5, -4 and -2 fixtures is the same through the `.did` and the Rust encoder. The control through `a78a542`'s `.did` names the round-7 fields |

## `P/`: this round's probe (read-only, 2026-09-28)

| File | What |
| --- | --- |
| `pinned_range.py`, `pinned_range.output.txt` | The fixed range `--probe-depth` and the runbook's rehearsal read: Milady `Transfer` over 26,012,410..26,012,909, 10 logs on both MEV Blocker and Tenderly (63,500 blocks deep), and the receipt of `0x17133e8b…c093` (block 26,012,500, status 1, one Milady `Transfer`) on both |
