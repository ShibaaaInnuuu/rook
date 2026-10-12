# Review round 7, part F: PoCs

Ran in a private copy (`git archive a78a542 autobuyer/canister-rs` into `/tmp/rook-r7-F`, `diff -rq --exclude=target` identical to the
worktree before anything was added; removed afterwards). Toolchain 1.98.1. 4663 network use: read-only `eth_blockNumber`,
`eth_getBlockByNumber`, `eth_getTransactionCount` against `rpc.mainnet.chain.robinhood.com` and `rpc.ordofi.network`. Nothing sent.

| File | How | Result |
| --- | --- | --- |
| `probe4663.sh`, `depth_scan.py`, `depth_scan.output.txt` | run as is | 4663 makes 9.95 blocks/s; Robinhood's provider serves state 6,198-6,234 blocks under ITS head, a sliding window |
| `r7_f_reach.append.rs`, `r7_f_reach.output.txt` | appended to the copy's `tests/audit_nft_r6.rs` (its fake, `hidden_deposit_live`, `run_for`) | **F7-L-1**: with the pin's lag under the head modelled (the fake's pin is the head), the reach seen from `HomeTx.pin` is 4,169-5,192 blocks; a first resolving tick 420 s (lag 2,047) or 540 s (lag 1,024) after signing leaves a mined, hidden deposit UNIDENTIFIED |
