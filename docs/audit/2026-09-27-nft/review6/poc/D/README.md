# Review round 6, part D: PoCs

`audit6_nft_d.test.mjs` runs in a private copy of `monitor/` at `e04fa16` (the repo's `monitor`, `ops`, `scripts`, `deployments`,
`tools/snapshot`, `autobuyer/canister-rs/tests/fixtures` cloned under `/tmp/rook-r6-D/`, removed afterwards), with one extra file:
`monitor/src/candid_r4.mjs` = `git show 8ef82d6:monitor/src/candid.mjs` (the round-4 monitor's IDL, for the old-monitor/new-canister
decode). Run: `node --test test/audit6_nft_d.test.mjs`. A test named WEAKNESS passes while the weakness is present.
Output: `audit6_nft_d.output.txt` (6/6 pass).
