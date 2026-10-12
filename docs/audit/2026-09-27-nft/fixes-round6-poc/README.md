# The round-6 reviewers' PoCs, re-run on fix round 6

Each part's PoCs (`../review6/poc/`) were rebuilt as their READMEs say, in a private copy of `autobuyer/canister-rs` and `monitor/`
under `/tmp/rook-fix6/poc` (removed afterwards), on the fix-round-6 code. The harnesses are e04fa16's `tests/audit_nft_r5.rs` fake
world with the reviewers' diffs, as they ran them. The only edits to the PoC sources: part C's calls of `judge_payout` name the new
argument (the payout's receipt logs; none in these PoCs), and part C's file gets back the closing brace its `r6_c_l1_b_…` lacks; part
F's file keeps the harness and the five `r6_` tests (plus the `milady_deposits` / `step` helpers they call), not the 33 round-5 tests.
A PoC that FAILS shows its weakness is gone.

| File | Result |
| --- | --- |
| `B.review6-pocs-on-round6.output.txt` | **B6-M-1** a and b and **B6-L-1** fail on their weakness assertions: after a 12,000-block gap on a 10,000-block provider the first newer sale is read at pass 1 (was 385–395), and a day's are read; the transforms keep the refusal's class; 16 unreadable old receipts no longer hold the newer sales (144 of 144 read, was 0). `r6_b_m2_a_…` (B6-L-2) asserts nothing and prints: at 30 % and 50 % busy, 432 of 432 due sales counted, 0 lost for good, `SaleScan.bad` at most 1 (was 376, 20 lost, 529) |
| `C.review6-pocs-on-round6.output.txt` | All five C PoCs **fail**: the stranger's gas-only depository call is refused (no `RelayCallExecuted` paying us, C6-L-1), and so is the ERC-20 call through an address-like request id; a credit waits while the loss-closing `returnAdvance(0)` is out, so neither branch records a payout nobody is paid (C6-L-2, both chains); the written-off forward order keeps its custody reservation (C6-L-3) |
| `F.review6-pocs-on-round6.output.txt` | **F6-L-2 fails** (a receipt the providers disagree on no longer halts the line). `r6_f6_l1_at_the_runners_cadence_…` still passes by construction: it drives the canister at 600 s + 60 s, which is out of the provider's ≈ 10 minutes of state; the fix is the runner's cadence while a home transaction is out (`RUNNER_PENDING_EVERY_S`, 240 s: `audit_nft_r6.rs` `r6_f6_l1_…` at 300 s identifies at the first tick). F6-I-1 and F6-I-3 pass (Infos, not fixed: `../fixes-round6.md` §8) |
| `AD.review6-pocs-on-round6.output.txt` | **A6-L-1** (a, b), **D6-L-1**, **D6-L-3**, **D6-I-2** and **D6-I-3** fail. The A6-I-2 PoC passes: it calls `checkStagedProposal` at the JS level without `requireBaseline`; golive-check now passes it under `--phase1` and refuses without a baseline (`monitor/test/staged_r6.test.mjs`). The D6-L-2 PoC passes because its state is the round-5 type (no `HomeTx.sent_at`): the canister now stamps every home transaction and the monitor pages its age (`monitor/test/audit_nft_d_r6.test.mjs`, on the canister's own round-6 bytes). The IDL control passes |

## `P/`: this round's provider probes (read-only, 2026-09-28)

| File | What |
| --- | --- |
| `probe.py`, `probe.output.txt` | The recommended Ethereum pair: `eth_getLogs` depth to 130,000 blocks, receipts to 70,000, a 10,000-block range, the result caps, Seaport's global `OrderFulfilled` volume, 13-way bursts |
| `burst.py`, `burst.output.txt` | 20 rounds of 13 simultaneous `eth_getLogs` per provider from one IP, and a refusal's shape (Tenderly's JSON 429, MEV Blocker's Cloudflare page) |
| `send.py`, `send2.py`, `send.output.txt` | `eth_sendRawTransaction` reply shapes: a garbage payload, and the raw bytes of a stranger's already-mined transaction (rebuilt from its fields; MEV Blocker echoes its hash, so the bytes are right). Neither can be included. The first lines of the output are a first run whose raw read failed (`None` sent): noise, kept as run |
| `h4663.py`, `h4663.output.txt` | 4663 providers: which keep historical state how deep (F6-L-1) |
| `depository-events.txt` | Relay's depository's events, from its bytecode (C6-L-1) |
