# deployments/

One file per chain, `<chainid>.json`, written by `script/deploy.sh`. `31337.json` (a local anvil fork) is git-ignored; a public
chain's file is committed when a public deployment exists — there is none yet.

What the indexer and the keeper read from it: `chainId`, `registry`, `launcher`, `distributor`, `ponsFactory`, and **`fromBlock`**.
`fromBlock` must be the chain's own height as `eth_getLogs` understands it. It is *not* `deployedAtBlock`: on Robinhood Chain (an
Arbitrum chain) `block.number` inside a contract is the L1 height, tens of millions away from the L2 height, and reading logs from
it finds nothing. A file without a usable `fromBlock`, or whose `chainId` is not the node's, is treated as no deployment at all.
