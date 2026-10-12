# snapshot

Rebuilds a coin's holder snapshot at a block from public `Transfer` logs and prints the Merkle root the Distributor verifies.
Rules: `docs/snapshot-v1.md` (and its "Version 2", for SweepVault draws: the snapshot time and the sentinel leaf). Read-only: there is no signer in this package, and a test fails if one appears.

```
npm install
npm run snapshot -- --coin 0x… --block 68790000 --kind weighted --from-block 68500000 --exclusions exclusions.json
npm run snapshot -- --coin 0x… --time 1790000000 --kind weighted --stack ../../deployments/stack-4663.json --slug frogs --exclusions exclusions.json
npm run snapshot -- --vault 0xRAFFLE --draw-id 3 --stack ../../deployments/stack-4663.json --slug frogs --exclusions exclusions.json --rpc <a second RPC>
npm test            # unit tests, determinism, and that the committed Solidity fixtures are what the tool writes today
npm run fixtures    # rewrites ../../test/fixtures/snapshot-*.json, which `forge test --match-path test/SnapshotTool*` consumes
```

Checking a SweepVault draw (v2): `--vault/--draw-id` reads the draw's coin and snapshot time from the chain, rebuilds, and exits 3
when the root or the ticket count differs (the owner or the guardian can cancel until the pin).

Checking a v1 keeper's root during the 15-minute window: take `coin`, `block`, `kind` (and `total` / `minBalance`) from the published
file's `inputs`, run the command above with your own RPC, compare `root`. Same inputs give the same file, byte for byte; publish or
compare its `sha256`. If the roots differ, say so publicly and to the owner, who can cancel the draw or distribution in the window.
