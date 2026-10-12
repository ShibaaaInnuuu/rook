// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Strings} from "@openzeppelin/contracts/utils/Strings.sol";
import {ListingRegistry} from "../ListingRegistry.sol";
import {SweepVault} from "../SweepVault.sol";
import {HomePolicy} from "./IHomeChain.sol";
import {HomeSweepVault} from "./HomeSweepVault.sol";
import {HomeWrappedAdapter} from "./HomeWrappedAdapter.sol";
import {FeeEscrow} from "./FeeEscrow.sol";
import {HomeReceiver} from "./HomeReceiver.sol";

/// @title HomeListing
/// @notice The 4663 registry of home-chain collections (docs/v6/home-chain-vault.md §10.2): one listing per (home chain,
/// collection, policy), recording the home chain's vault and wrapper, the twin on this chain, the adapter, the escrow, the
/// receiver and the sweep vault — every 4663 link verified against the chain before it is written down. Like `NftRegistry` it
/// keeps NO roles of its own: owner, keeper and treasury are `ListingRegistry`'s, so every rotation still waits its seven public
/// days and moves every system at once. A `HomeSweepVault` is built with this contract as its `registry` and reads exactly
/// `owner()`, `keeper()` and `isPaused(id)` from it.
contract HomeListing {
    ListingRegistry public immutable listingRegistry;

    struct Listing {
        uint64 homeChainId;
        address collection;
        HomePolicy policy;
        bool paused;
        address homeVault;      // on the home chain: the FeeEscrow's `homeVault`
        address homeWrapper;    // on the home chain: the adapter's `homeWrapper`
        address wx;             // the twin here
        address adapter;
        address escrow;
        address receiver;       // zero for a policy that sends nothing down (Burn)
        address sweepVault;
        address factory;        // the sweep vault's launcher
        string name;
    }

    struct Params {
        uint64 homeChainId; address collection; HomePolicy policy; string name; address homeVault; address homeWrapper;
        address wx; address adapter; address escrow; address receiver; address sweepVault;
    }

    Listing[] private _listings;
    mapping(bytes32 key => bool) public listed;               // keccak(homeChainId, collection, policy)

    event HomeListed(uint256 indexed id, uint64 indexed homeChainId, address indexed collection, HomePolicy policy, address sweepVault,
        address escrow, address adapter, string name);
    event Paused(uint256 indexed id, bool paused);

    error NotOwner();
    error ZeroAddress();
    error Empty();
    error BadChain();
    error BadPolicy();
    error AlreadyListed();
    error NoSuchListing(uint256 id);
    error NotThisRegistry();
    error WrongListingId();
    error WrongPolicy();
    error WrongAdapter();
    error WrongEscrow();
    error WrongHomeVault();
    error WrongLauncher();
    error WrongTwin();
    error WrongHomeWrapper();
    error WrongReceiver();

    constructor(ListingRegistry listingRegistry_) {
        if (address(listingRegistry_) == address(0)) revert ZeroAddress();
        listingRegistry = listingRegistry_;
    }

    modifier onlyOwner() { if (msg.sender != owner()) revert NotOwner(); _; }

    function owner() public view returns (address) { return listingRegistry.owner(); }
    function keeper() external view returns (address) { return listingRegistry.keeper(); }
    /// @notice The registry's keeper incident switch and guardian (docs/audit/2026-09-26/fixes-B-C-keeper-draws.md), which the
    /// sweep vaults read through this listing like `keeper()`.
    function keeperPaused() external view returns (bool) { return listingRegistry.keeperPaused(); }
    function guardian() external view returns (address) { return listingRegistry.guardian(); }
    function treasury() external view returns (address) { return listingRegistry.treasury(); }

    /// @notice The owner records one home-chain collection under one policy, after checking that the 4663 contracts given point
    /// at each other and at this listing's id. What is on the home chain is recorded as the escrow and the adapter were built with.
    function list(Params calldata p) external onlyOwner returns (uint256 id) {
        if (p.collection == address(0) || p.homeVault == address(0) || p.homeWrapper == address(0) || p.wx == address(0)
            || p.adapter == address(0) || p.escrow == address(0) || p.sweepVault == address(0)) revert ZeroAddress();
        if (p.homeChainId == 0 || p.homeChainId == block.chainid) revert BadChain();
        if (p.policy == HomePolicy.Wrap) revert BadPolicy();
        if (bytes(p.name).length == 0) revert Empty();
        bytes32 key = keccak256(abi.encode(p.homeChainId, p.collection, p.policy));
        if (listed[key]) revert AlreadyListed();
        id = _listings.length;
        HomeSweepVault v = HomeSweepVault(payable(p.sweepVault));
        if (address(v.registry()) != address(this)) revert NotThisRegistry();
        if (v.listingId() != id) revert WrongListingId();
        if (uint8(v.policy()) != uint8(p.policy)) revert WrongPolicy();
        if (address(v.wrapper()) != p.adapter) revert WrongAdapter();
        if (address(v.escrow()) != p.escrow) revert WrongEscrow();
        FeeEscrow e = FeeEscrow(p.escrow);
        if (e.listingId() != id) revert WrongListingId();
        if (e.homeVault() != p.homeVault) revert WrongHomeVault();
        if (address(e.launcher()) != p.sweepVault) revert WrongLauncher();
        HomeWrappedAdapter a = HomeWrappedAdapter(p.adapter);
        if (address(a.wx()) != p.wx) revert WrongTwin();
        if (a.homeWrapper() != p.homeWrapper) revert WrongHomeWrapper();
        if (p.receiver != address(0) && address(HomeReceiver(payable(p.receiver)).escrow()) != p.escrow) revert WrongReceiver();
        listed[key] = true;
        _listings.push(Listing(p.homeChainId, p.collection, p.policy, false, p.homeVault, p.homeWrapper, p.wx, p.adapter, p.escrow,
            p.receiver, p.sweepVault, address(v.launcher()), p.name));
        emit HomeListed(id, p.homeChainId, p.collection, p.policy, p.sweepVault, p.escrow, p.adapter, p.name);
    }

    /// @notice Instant, as in ListingRegistry: pausing stops sweeps and forwards only. Money still comes in; draws still settle.
    function setPaused(uint256 id, bool paused_) external onlyOwner {
        _at(id).paused = paused_;
        emit Paused(id, paused_);
    }

    // ---- views ----------------------------------------------------------------------------------------------------------------

    function count() external view returns (uint256) { return _listings.length; }
    function listing(uint256 id) external view returns (Listing memory) { return _at(id); }
    function vaultOf(uint256 id) external view returns (address) { return _at(id).sweepVault; }
    function isPaused(uint256 id) external view returns (bool) { return _at(id).paused; }

    /// @notice `eip155:<homeChainId>:<collection>` (CAIP-10 style, lower-case hex), as `NftRegistry.ref`.
    function ref(uint256 id) external view returns (string memory) {
        Listing storage l = _at(id);
        return string.concat("eip155:", Strings.toString(l.homeChainId), ":", Strings.toHexString(l.collection));
    }

    function _at(uint256 id) private view returns (Listing storage) {
        if (id >= _listings.length) revert NoSuchListing(id);
        return _listings[id];
    }
}
