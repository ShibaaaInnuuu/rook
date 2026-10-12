# The round-5 reviewers' PoCs, re-run on fix round 5

Each part's PoCs (`../review5/poc/`) were rebuilt exactly as their READMEs say, in a private copy of `autobuyer/canister-rs` and
`monitor/` under `/tmp` (removed afterwards), on the fix-round-5 canister commit. The only edits to the PoC sources are the new `opt`
fields their struct literals must now name (`Watch.request_id` / `home_pin`, `LateOrder.released_at`), set to `None`.

| File | Result |
| --- | --- |
| `B.review5-pocs-on-round5.output.txt` | All six B PoCs (B5-M-1 a/b/c, B5-M-2, B5-L-1, B4-L-2's edge) **fail on their weakness assertions**; the 22 round-4 tests of the copied harness pass |
| `C.review5-pocs-on-round5.output.txt` | All five C PoCs (C5-M-1, C5-L-1 a/b/c, C5-L-3) **fail**: no order starts after the write-off, the judge refuses what does not name the order, a failed read credits nothing |
| `F.review5-pocs-on-round5.output.txt` | All eight F PoCs **fail**: nothing is dropped on absence of evidence (their `run_for(..).expect("dropped")`), Unsure restarts the clock, the Ethereum floors reset on a chain move (the PoC's `PinFloors` unit lines still print: the reset is in `post_upgrade`, `ethflow::chain_moved`) |
| `E.review5-pocs-on-round5.output.txt` | E5-L-1 **fails** (the mint is no longer taken back on a failed read). E5-I-1 and E5-L-2 pass (not fixed: `fixes-round5.md` §7, §8). E5-I-2 passes because it re-runs round 4's string test, which is kept; the new semantic test `r5_e5_i2_…` catches all four of its mutations |
| `D.review5-pocs-on-round5.output.txt` | D5-L-2 **fails**. D5-L-1's PoC still passes: its fake wrapper answers `deliveryTx` 0, where the real `recordDelivery` sets it to the delivery's transaction; `monitor/test/audit_nft_d_r5.test.mjs` holds the exemption with it (and that a refund or another record still pages). The Infos' PoCs (D5-I-1, D5-I-2) pass: not fixed |
