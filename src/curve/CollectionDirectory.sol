// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {CurveFactory} from "./CurveFactory.sol";
import {IItemQueue} from "../interfaces/IItemQueue.sol";

/// @title CollectionDirectory
/// @notice Where the canister (and the page) finds every collection's curve stack on-chain: its wrapped token, factory, buy
/// queue and liquidity lock — so the canister reads one address instead of keeping its own append-only list. docs/v3/pools.md.
/// @dev Append-only: an entry is never removed or changed. `add` is the admin's (the multisig), and checks that the four
/// contracts name each other, so a wrong entry cannot point the canister at a queue that is not the factory's. What this does
/// NOT protect against: the admin listing a collection nobody should buy; the canister still applies its own checks.
contract CollectionDirectory {
    struct Collection {
        address wrapper;
        CurveFactory factory;
        IItemQueue queue;          // the buy queue or the floor module: the canister drives either through the same selectors
        address lock;
    }

    address public immutable admin;
    Collection[] private _collections;
    mapping(address wrapper => uint256) public indexOf;   // 1-based; 0 = not listed

    event Added(uint256 indexed index, address indexed wrapper, address factory, address queue, address lock);

    error NotAdmin();
    error AlreadyListed();
    error Inconsistent();

    constructor(address admin_) {
        if (admin_ == address(0)) revert NotAdmin();
        admin = admin_;
    }

    function add(CurveFactory factory) external returns (uint256 index) {
        if (msg.sender != admin) revert NotAdmin();
        address w = factory.wrapped();
        IItemQueue q = factory.queue();
        address l = address(factory.seeder());
        if (indexOf[w] != 0) revert AlreadyListed();
        if (address(q.factory()) != address(factory) || address(q.wrapper()) != w) revert Inconsistent();
        if (address(factory.raffleVault()) == address(0)) revert Inconsistent();
        index = _collections.length;
        _collections.push(Collection(w, factory, q, l));
        indexOf[w] = index + 1;
        emit Added(index, w, address(factory), address(q), l);
    }

    function count() external view returns (uint256) { return _collections.length; }
    function at(uint256 i) external view returns (Collection memory) { return _collections[i]; }
    /// @notice Every buy queue, for the canister: one read.
    function queues() external view returns (address[] memory qs) {
        qs = new address[](_collections.length);
        for (uint256 i = 0; i < qs.length; ++i) qs[i] = address(_collections[i].queue);
    }
}
