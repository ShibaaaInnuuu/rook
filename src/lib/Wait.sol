// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @title Wait
/// @notice A delay that the sequencer cannot shorten.
///
/// @dev Every waiting period in this system exists so that a person can see something and stop it: the hour between
/// announcing a purchase and the money moving, the fifteen minutes in which a draw may be challenged, the hour a winner has
/// to correct a Bitcoin address. All of them were measured in `block.timestamp` alone, and on this chain that is the wrong
/// clock to trust alone.
///
/// Chain 4663 is an Arbitrum Nitro L2. Nitro lets the sequencer choose each block's timestamp, subject to two rules: it may
/// never go backwards, and it must lie between 24 hours behind wall-clock time and **one hour ahead of it**
/// (https://docs.arbitrum.io/build-decentralized-apps/arbitrum-vs-ethereum/block-numbers-and-time). One hour ahead is not a
/// margin against an hour-long wait; it is exactly equal to it. A sequencer that stamps the next block an hour in the future
/// makes `block.timestamp >= readyAt` true in the block after the announcement, and the public hour becomes one block. The
/// same jump closes a fifteen-minute challenge window before anybody can challenge. Arbitrum's own guidance says as much:
/// timing is "reliable in the longer term (i.e., on the order of at least several hours) but unreliable in the shorter term".
///
/// `block.number` is the second clock, and it is the sequencer-independent one. On Nitro it is NOT the L2 block number: it is
/// the parent chain's, which the sequencer reports and cannot run ahead of. Measured on 4663 on 2026-09-22 with an
/// `eth_call` state override returning the NUMBER opcode: the EVM saw 26,034,810 while the L2 head was 69,869,490 and the
/// RPC's `l1BlockNumber` was 26,034,810 — so `block.number` is the parent chain's, confirmed rather than assumed. Its rate,
/// over three spans ending at that block:
///
///   |   L2 blocks |   elapsed |  parent blocks | s/parent-block | parent blocks/hour |
///   |------------:|----------:|---------------:|---------------:|-------------------:|
///   |     100,000 |    2.79 h |            827 |          12.14 |              296.6 |
///   |   1,000,000 |   27.97 h |          8,323 |          12.10 |              297.6 |
///   |   5,000,000 |  139.90 h |         41,758 |          12.06 |              298.5 |
///
/// So on 4663 the parent block time is 12 s, and a one-hour wait is 300 parent blocks ≈ 60.4 minutes at the measured rate.
///
/// **That number is deployment config, not a constant.** Each contract that waits takes `parentBlockSeconds` in its
/// constructor, keeps it immutable, and passes it to `blocksFor`; 4663 deploys with 12 (rounded DOWN from 12.06–12.14 s, so the
/// block count for a period is an over-estimate and the wait errs long). A chain whose `block.number` is its own and faster
/// deploys with its own measured value, and the block half of each wait stays a real guard there. `checkBlockSeconds` bounds
/// it to 1–60 s: zero would divide by zero, and anything longer than a minute is not a block time anyone has measured.
///
/// What this does NOT fix: a sequencer that stalls `block.number` makes waits LONGER, which is the safe direction but is
/// still a liveness lever it holds. A parent chain that permanently speeds up would weaken the block half of the guard, and
/// the timestamp half would become the binding one again — the rate above should be re-measured before mainnet and whenever
/// the parent chain changes its block time.
library Wait {
    /// @dev Only for the contracts this branch may not edit — ListingRegistry, src/nft/*, and the AssetVault / AutoBuyerVault the
    /// registry deploys (their constructor arguments are the registry's). They keep 4663's value until the merge threads the
    /// config through them; every other waiting contract takes its own.
    uint256 internal constant LEGACY_PARENT_BLOCK_SECONDS = 12;
    uint256 internal constant MAX_PARENT_BLOCK_SECONDS = 60;

    error BadParentBlockSeconds(uint256 secondsPerBlock);

    /// @notice Refuse a block time nobody could have measured.
    function checkBlockSeconds(uint256 parentBlockSeconds) internal pure {
        if (parentBlockSeconds == 0 || parentBlockSeconds > MAX_PARENT_BLOCK_SECONDS) revert BadParentBlockSeconds(parentBlockSeconds);
    }

    /// @notice The number of parent-chain blocks in `period`, for a chain whose parent makes a block every `parentBlockSeconds`.
    function blocksFor(uint256 period, uint256 parentBlockSeconds) internal pure returns (uint256) {
        return period / parentBlockSeconds;
    }

    /// @notice 4663's value, for the legacy callers named above only.
    function blocksFor(uint256 period) internal pure returns (uint256) {
        return period / LEGACY_PARENT_BLOCK_SECONDS;
    }

    /// @notice Has the wait really elapsed? Both clocks must say so: the sequencer's, and the parent chain's.
    function elapsed(uint64 readyAtTime, uint64 readyAtBlock) internal view returns (bool) {
        return block.timestamp >= readyAtTime && block.number >= readyAtBlock;
    }
}
