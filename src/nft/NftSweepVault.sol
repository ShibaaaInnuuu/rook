// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IERC721} from "@openzeppelin/contracts/token/ERC721/IERC721.sol";
import {IRandomnessBeacon} from "../interfaces/IRandomnessBeacon.sol";
import {Policy, INftRegistry} from "./INft.sol";
import {NftVaultBase} from "./NftVaultBase.sol";
import {SeaportGuard} from "./SeaportGuard.sol";

/// @title NftSweepVault
/// @notice One NFT listing on Robinhood Chain itself: its coins' fees buy the collection's floor through an allow-listed Seaport, by
/// ANYONE's call, and the contract — not a keeper, not the caller — decides whether the purchase stands. docs/v4/nft.md §5.
///
/// The vault forwards only Seaport 1.6 calldata it has decoded, checked order by order and re-encoded (`SeaportGuard`: one listed
/// NFT per order, native ETH at most the cap per NFT, no fulfiller tip, nobody paid who is the caller). It then measures its own
/// NFT balance and its own ETH before and after, and keeps the purchase only if:
///   - every NFT that arrived is one of the caller's listed candidate ids (the balance rose by exactly the candidates it now owns),
///   - at least `minBought` of them arrived,
///   - what left is at most the fresh cap times what arrived.
/// Anything else reverts the whole transaction, Seaport's transfers with it.
///
/// NFTs leave only by a won raffle (`claimWin`) or a burn to 0x…dEaD (`burn`), whichever the listing fixed. There is no withdraw,
/// no owner path, and the vault never approves anything to anyone — so an order naming the vault as offerer cannot move what it holds.
///
/// @dev What this does NOT protect against:
///  - a sweeper who lists an NFT it owns, from another address, at up to the cap and fills that listing. The vault then buys a
///    real NFT of the collection, but at up to (cap − floor) over the floor per NFT. No chain can tell that seller from an honest
///    one asking the same price. docs/v4/nft.md §10.1. (A fulfiller TIP, the old form of this, is refused.)
///  - a cap posted too high: the keeper's word about a market, expiring after an hour on both clocks.
///  - a collection contract that lies in `ownerOf`/`balanceOf`. Collections are listed by the owner.
///  - an NFT sent here outside a sweep (a plain transferFrom): it is no acquisition, and stays for ever.
///  - ETH forced in (selfdestruct): never counted, never spendable.
contract NftSweepVault is NftVaultBase {
    uint256 public constant MAX_CANDIDATES = 50;

    uint256 public sweepCount;
    /// @dev The target of the sweep in progress, and the only address `receive` accepts ETH from. Zero outside a sweep.
    address private _sweeping;

    event Swept(uint256 indexed sweepId, address indexed sweeper, address target, uint256 bought, uint256 spent);

    error TargetNotAllowed(address target);
    error BadCandidates();
    error DuplicateCandidate(uint256 tokenId);
    error AlreadyHeld(uint256 tokenId);
    error SweepFailed(bytes reason);
    error UnlistedArrival(uint256 balanceRise, uint256 candidatesArrived);
    error NotEnoughBought(uint256 bought, uint256 promised);
    error NothingSpent();
    error OverCap(uint256 spent, uint256 bound);
    error NotDuringSweep();

    constructor(INftRegistry registry_, uint256 listingId_, address collection_, Policy policy_, IRandomnessBeacon beacon_)
        NftVaultBase(registry_, listingId_, uint64(block.chainid), collection_, policy_, beacon_)
    {}

    /// @notice Seaport's refund of what an order did not use, during a sweep, from the sweep's target. Nothing else.
    receive() external payable {
        if (_sweeping == address(0) || msg.sender != _sweeping) revert NoDirectPayments();
    }

    /// @notice Accepted only from the listing's collection and only during a sweep: nobody can push other NFTs in through the safe path.
    function onERC721Received(address, address, uint256, bytes calldata) external view returns (bytes4) {
        if (_sweeping == address(0) || msg.sender != collection) revert NotDuringSweep();
        return this.onERC721Received.selector;
    }

    /// @notice Permissionless. Decodes `data` as Seaport 1.6 calldata and checks every order in it (`SeaportGuard`), forwards the
    /// re-encoded call to an allow-listed `target` with at most what those orders can take, then checks the outcome and charges the
    /// coins that paid, first in first out. Each NFT is attributed to the coin that paid most of its price.
    /// @param data fulfillBasicOrder, fulfillBasicOrder_efficient_6GL6yc or fulfillAvailableAdvancedOrders calldata; anything else reverts
    /// @param candidates the token ids this sweep may bring in, 1–50, distinct, none already held
    /// @param minBought the fewest of them that must arrive
    function sweep(address target, bytes calldata data, uint256[] calldata candidates, uint256 minBought)
        external nonReentrant returns (uint256 bought, uint256 spent)
    {
        if (registry.isPaused(listingId)) revert ListingPaused();
        if (!registry.isSweepTarget(target)) revert TargetNotAllowed(target);
        if (!capFresh()) revert CapExpired();
        uint256 n = candidates.length;
        if (n == 0 || n > MAX_CANDIDATES || minBought == 0 || minBought > n) revert BadCandidates();
        IERC721 c = IERC721(collection);
        for (uint256 i = 0; i < n; ++i) {
            for (uint256 j = 0; j < i; ++j) if (candidates[j] == candidates[i]) revert DuplicateCandidate(candidates[i]);
            if (_ownerOf(c, candidates[i]) == address(this)) revert AlreadyHeld(candidates[i]);
        }

        (bytes memory call_, uint256 value) = SeaportGuard.check(data, collection, candidates, capWei);
        if (value > attributed) value = attributed;
        uint256 nftBefore = c.balanceOf(address(this));
        uint256 ethBefore = address(this).balance;
        _sweeping = target;
        (bool ok, bytes memory ret) = target.call{value: value}(call_);
        _sweeping = address(0);
        if (!ok) revert SweepFailed(ret);

        uint256[] memory got = new uint256[](n);
        for (uint256 i = 0; i < n; ++i) if (_ownerOf(c, candidates[i]) == address(this)) got[bought++] = candidates[i];
        uint256 nftAfter = c.balanceOf(address(this));
        if (nftAfter < nftBefore || nftAfter - nftBefore != bought) revert UnlistedArrival(nftAfter - nftBefore, bought);
        if (bought < minBought) revert NotEnoughBought(bought, minBought);
        uint256 ethAfter = address(this).balance;
        if (ethAfter >= ethBefore) revert NothingSpent();
        spent = ethBefore - ethAfter;
        if (spent > capWei * bought) revert OverCap(spent, capWei * bought);
        // spent ≤ value ≤ attributed: the only ETH that can leave during the call is `value`

        uint256 sweepId = sweepCount++;
        _chargeFifo(sweepId, spent);
        uint256 price = spent / bought;
        for (uint256 i = 0; i < bought; ++i) {
            uint256 p = i == bought - 1 ? spent - price * (bought - 1) : price;
            _record(got[i], _attribute(sweepId, p), p, sweepId, bytes32(block.number));
        }
        emit Swept(sweepId, msg.sender, target, bought, spent);
    }

    /// @notice Permissionless, for a Burn listing: sends one bought NFT to 0x…dEaD.
    function burn(uint256 acquisitionId) external nonReentrant {
        Acquisition memory a = _markBurned(acquisitionId, bytes32(0));
        IERC721(collection).transferFrom(address(this), DEAD, a.tokenId);
    }

    /// @dev A won lot's NFT goes straight to the winner. `transferFrom`, not the safe variant: a winner's contract cannot refuse
    /// it and block the lot. The state is already final when this runs, and the whole call is under the reentrancy lock.
    function _onWin(uint256 acquisitionId, Lot storage, address winner) internal override {
        IERC721(collection).transferFrom(address(this), winner, _acq(acquisitionId).tokenId);
        _lot(acquisitionId).stage = Stage.Delivered;
    }

    function _ownerOf(IERC721 c, uint256 tokenId) private view returns (address) {
        try c.ownerOf(tokenId) returns (address o) { return o; } catch { return address(0); }
    }
}
