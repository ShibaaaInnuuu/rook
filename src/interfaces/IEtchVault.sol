// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @title IEtchVault
/// @notice What the rest of the system needs from a vault, whichever kind it is.
/// @dev Deliberately small. A vault's purchase machinery — who may spend, against what bound, with what receipt — differs
/// between v1 (`AssetVault`: a bonded, named purchaser) and v2 (`AutoBuyerVault`: a program whose key nobody holds), and
/// nothing outside the vault should have to know which it is talking to. Before this interface existed the Treasury and the
/// Distributor each cast a registry-supplied address to `AssetVault` and called it, which worked only because the two vaults
/// happened to share a field order — a type confusion waiting for the first divergence. Everything they actually need is
/// here, so the compiler checks it now.
interface IEtchVault {
    /// @notice One thing the vault bought, and which coin's fees paid for it.
    struct Acquisition {
        uint256 purchaseId;
        address coin;              // the coin whose fees paid for it
        uint256 amount;            // 1 for an inscription; rune units otherwise
        bytes32 btcTxid;
        uint64 satsPaid;
        string assetRef;           // the inscription id, or the rune id
    }

    /// @notice A coin's fees arrive. The vault checks that `msg.sender` really is that coin's router.
    function deposit(address coin) external payable;

    /// @notice The treasury's share arrives, attributed to the protocol token. Only the registry's treasury may call it.
    function depositFrom(address coin) external payable;

    /// @notice One receipted purchase. Reverts if there is no such acquisition.
    function acquisition(uint256 id) external view returns (Acquisition memory);

    function acquisitionCount() external view returns (uint256);

    /// @notice Whether this vault's listing is a rune (so a quantity is divisible) or a set of inscriptions.
    function isRune() external view returns (bool);
}
