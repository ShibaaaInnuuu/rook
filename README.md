# Rook

**Memecoins backed by blue-chip NFTs.** Launch a coin on any of 38 NFT collections across Ethereum, Bitcoin (Ordinals) and Solana.
Coins trade on Meteora's bonding curves on Solana. When a coin graduates, half of what it raised goes into a vault that belongs to
the coin and holds that collection's NFTs. Holders can burn coins for their share of the vault, and one whole wrapped token
redeems a real NFT.

Site: https://userook.fun · Docs: https://userook.fun/#/docs

## How it works

- **Coins** launch on Meteora's Dynamic Bonding Curve, priced in their collection's **wrapped token** (wX: one whole token is one
  NFT in custody, or SOL set aside to buy one at the floor). A creator's first buy, inside the launch transaction, pays the
  normal 1 % fee; everyone else pays an anti-sniper fee that falls from 50 % to 1 % over the first 10 minutes.
- **Graduation** puts half the raise into the coin's vault and the rest into a Meteora pool whose liquidity is locked for good.
- **The custodian** is a canister on the Internet Computer. It mints and burns every wrapped token, keeps the books, buys the NFTs
  on their own chains (OpenSea, Ordinals marketplaces, Tensor and Magic Eden) and holds them with threshold keys no single person
  holds. Rook deploys no programs of its own on Solana.
- **Changes** to the custodian's code wait 72 hours in public behind a timelock. Every book is a public query, and every
  wrapped-token mint or burn carries a memo naming its book entry.

## Repository

| Folder | What |
|---|---|
| `autobuyer/canister-rs` | the custodian canister (Rust) |
| `autobuyer/timelock-rs` | the 72-hour timelock that controls it |
| `web-rook` | the site |
| `indexer` | the Solana indexer and its read-only API |
| `ops` | the runner, the reconciler, the fueler, and the launch ceremony tools (`ops/ceremony`) |
| `monitor` | the monitor and its alerts |
| `deployments/launch` | the launch files: collections, thresholds, pins |
| `docs/v9` | the current design, the audit record (`docs/v9/audit-2026-10`) and the runbooks |

Older folders (`src`, `script`, `keeper`, `solana`, `web`) hold the earlier Robinhood Chain and Solana-program designs this code
grew out of; they are not part of the launch.

## Verify the canister

The build is reproducible. Anyone can rebuild the exact module the IC runs and compare fingerprints:

```sh
cd autobuyer && scripts/reproducible-build.sh                       # Docker, linux/amd64
scripts/reproducible-build.sh verify <canister id> buyer.wasm.gz ic # MATCH or not
```

## Status

New code. It has been through internal audit rounds with proof-of-concept tests, every finding fixed, and has run end to end on
Solana's test network. New code can still hold bugs nobody has found. Nothing here is financial advice.
