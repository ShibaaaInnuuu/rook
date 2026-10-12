// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @title LocalPonsFactory
/// @notice scripts/golive-dryrun.sh ONLY: a bare local node has no Pons, and script/Deploy.s.sol refuses (PonsMissing) without it.
/// The dry-run plants this runtime code at Pons' real factory address with `anvil_setCode`, so the Launcher's constructor — which
/// asks the factory for its fee escrow and nothing else — can run. Nothing launches against it; the forked dry-run uses real Pons.
/// Never deployed anywhere but a throwaway local node.
contract LocalPonsFactory {
    address public constant ESCROW = address(0xE5C0);
    function feeEscrow() external pure returns (address) { return ESCROW; }
}
