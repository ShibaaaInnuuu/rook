# Snapshot rules, version 1

Written 2026-09-21, before the tool. A snapshot is the one input to a draw or a rune distribution that no contract checks. These
are the rules the keeper's snapshot must follow, written so that a stranger with the public RPC can recompute the same root. A
root that these rules do not reproduce is a wrong root, whoever published it.

## Inputs (all public, all in the published file)

`coin`, `block` (the snapshot block), `kind` (`weighted` | `per-wallet` | `pro-rata`), and for `pro-rata` the `total` units being
distributed; for `per-wallet` the `minBalance` (default one whole coin). The keeper must name the block **before** it exists —
in the draw's or distribution's `uri` document or an earlier on-chain event. *The tool cannot check that promise*; see the end.

## Balances

Every `Transfer(from, to, value)` log of the coin from its first log up to and including `block`, applied in (block, log index)
order. A balance that would go negative is an error and stops the tool: it means logs are missing.

## Exclusions — addresses that hold the coin without being holders

Removed before anything is counted, and listed in the output so they can be argued with:
the zero address and `0x…dEaD`; the coin's own contract; its Pons curve; the Pons factory, pool hook, fee escrow, liquidity
locker and buyback vault; the Uniswap v4 pool manager and position manager; and ours — the coin's router, the listing's vault, the
Launcher, the Distributor. Pons's and Uniswap's addresses are read from the factory (`getLaunchedToken`, `memeHook`, `feeEscrow`…)
where it exposes them and otherwise come from `exclusions.json`, which is part of the published output. Other contracts (a
multisig, an exchange wallet) **are** holders: a rule that guessed at them would be a rule somebody could lean on.

## Tickets and shares

Holders are ordered by address, ascending, compared as 20-byte numbers. Ties cannot occur.

- **weighted**: one ticket per **whole coin** (`balance / 10^decimals`, rounded down). Holders with none are dropped. Ranges are
  contiguous from 0 in holder order: `[first, end)`. `totalTickets` is the last `end`.
- **per-wallet**: one ticket for each holder with `balance ≥ minBalance`, same ordering, ranges of width 1.
- **pro-rata**: `units = floor(total × balance / Σ balances)` over the non-excluded holders. Holders with 0 units are dropped.
  The rounding remainder is allocated to nobody: it stays unclaimed and rolls into the coin's next distribution, which is what the
  Distributor does with anything unclaimed.

## Leaves and tree

Exactly what `Distributor` verifies: `ticketLeaf = keccak256(keccak256(abi.encode(address holder, uint256 first, uint256 end)))`,
`shareLeaf = keccak256(keccak256(abi.encode(address holder, uint256 units)))`. Leaves are sorted ascending as 32-byte numbers. Each
layer pairs neighbours left to right and hashes a pair as `keccak256(min ‖ max)` (OpenZeppelin's commutative hash, which is what
`MerkleProof.verify` recomputes); an odd node at the end of a layer moves up unchanged. One leaf is its own root. An empty list has
no root and the tool refuses.

## Output

`root`, `totalTickets` or `allocated`/`remainder`, the inputs, the exclusions used, and every entry with its leaf and proof, as
JSON with keys in a fixed order and no timestamps — so that two runs give the same file, byte for byte, and `sha256` of the file
is a meaningful thing to publish.

## What this does not prove

That the keeper used the block it promised. A keeper who may choose the block after seeing balances can choose a favourable one.
The defence is procedural and visible: the block is named in advance, on chain or in the `uri`, and a snapshot at any other block
is grounds for the owner to cancel within the 15-minute window. Nor does the tool prove the RPC told the truth; run it against
any second RPC and compare roots.

## Version 2 for SweepVault draws (2026-09-26)

`docs/audit/2026-09-26/fixes-B-C-keeper-draws.md` §6. A SweepVault draw no longer takes a keeper-chosen block. The keeper
announces the draw, which pins a future drand round; that round's randomness fixes `snapshotAt`, a moment of the 24 hours before
the announcement (`SnapshotFixed`). The snapshot block is the last block of the chain whose timestamp is at most `snapshotAt`.
The tree is v1's plus one sentinel leaf, `keccak256(keccak256(abi.encode(address(0), totalTickets, snapshotAt)))`, which commits
the count and the time to the root (`claim` refuses holder zero). The v1 Distributor keeps v1.

**No eligible holder** (`docs/audit/2026-09-26/review2/fixes-R3-carry.md`): when nobody outside the exclusions has a whole coin at
the snapshot block, the tree is the sentinel alone, `leaf(address(0), 0, snapshotAt)`: the root **is** that leaf, its proof is empty,
`totalTickets` is `"0"` and `entries` is `[]`. `openDraw` accepts exactly that for `totalTickets = 0` (any other root, or any proof,
is refused), and after the challenge window `requestDraw` closes the draw with no winner (`Empty`). Anyone rebuilding the snapshot
who finds a holder has caught a false empty: the Safe or the guardian cancels it before the close, and the coin's next draw reuses the
same snapshot time. v1 (no `snapshotAt`) of nobody still has no root.

**What `openDraw` can and cannot check** (R4 check A4-I1, `docs/audit/2026-09-26/review4/fixes-golive-lows.md` §7): with
`totalTickets > 0` it refuses an empty sentinel proof (a real list always has a holder leaf beside the sentinel, R3 check A3-I2). It
cannot tell a real holder leaf from a junk node: the sentinel `leaf(address(0), N, snapshotAt)` beside one junk sibling opens, pins
and would use the coin's carried seed with nobody able to claim. No on-chain rule closes that without the holder list (a keeper that
forges roots can build a junk tree of any required shape), so, like every false list, it is caught by recomputing the tree: the
monitor's indexes reproduce the holders' root, judge the draw `bad`, and the guardian pauses the keeper and cancels it before the pin
(`monitor/test/holders.test.mjs`, "audit4 A4-I1").


**The tool, v2** (`tools/snapshot`, wave 2, `docs/audit/2026-09-26/fixes-W2-services.md` §9):

- `snapshot(…, { snapshotAt })` (weighted only) adds the sentinel and writes `version: 2`, `inputs.snapshotAt`, and
  `sentinel: { holder: 0x0, first: totalTickets, end: snapshotAt, leaf, proof }` (the proof `openDraw` takes). Without a time the
  output is v1, byte for byte. `test/SnapshotToolV2.t.sol` holds the vault's `leaf` and OpenZeppelin's `MerkleProof` to the tool's
  fixture.
- `--time T` resolves the block by the rule above (the last block stamped at or before T; refused while no later block exists).
- `--stack <file> --slug <slug>`: a coin on our own curve; the exclusions are the stack's (`src/stack.ts`, the same function the
  keeper builds with) plus `--exclusions`, which fails closed when unreadable.
- `--vault <raffle vault> --draw-id <n> --stack … --slug …`: reads the draw's coin and `snapshotAt` from the chain, rebuilds, and
  compares the root and the ticket count with the draw's. Exit 3 on a mismatch: the monitor's recompute, run on a second RPC.
- Log ranges a provider refuses as too large (4663's 10,000-log cap) are halved down to one block; throttles answered in HTTP 200
  are retried; any other failure stops the snapshot (no partial tree).
