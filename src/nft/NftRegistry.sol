// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Strings} from "@openzeppelin/contracts/utils/Strings.sol";
import {ListingRegistry} from "../ListingRegistry.sol";
import {IRandomnessBeacon} from "../interfaces/IRandomnessBeacon.sol";
import {Wait} from "../lib/Wait.sol";
import {Policy, INftRegistry} from "./INft.sol";
import {NftSweepVault} from "./NftSweepVault.sol";
import {NftRemoteVault} from "./NftRemoteVault.sol";

/// @title NftRegistry
/// @notice NFT listings on ANY EVM chain: a chain id, a collection contract, and one fate for everything bought — raffle or burn.
/// A collection on this chain gets an `NftSweepVault` (bought through Seaport, checked by the vault); a collection anywhere else gets
/// an `NftRemoteVault` (bought by the canister, accounted here). Both are the same kind of listing. docs/v4/nft.md §2.
/// @dev A separate contract, not a new `Kind` in `ListingRegistry`: that struct is decoded by the web app, the canister and
/// golive-check, and its fields are Bitcoin-shaped; and that registry is ~41 KB already. This one keeps NO roles of its own. Owner,
/// keeper and treasury are `ListingRegistry`'s, so every rotation still waits its seven public days and moves both systems at once.
contract NftRegistry is INftRegistry {
    ListingRegistry public immutable listingRegistry;
    IRandomnessBeacon public immutable beacon;
    uint256 public constant CHANGE_DELAY = 7 days;

    struct NftListing {
        uint64 chainId;
        Policy policy;
        bool paused;
        address collection;
        address vault;
        address buyer;          // the canister's EVM address, for a remote listing; zero for a listing on this chain
        string name;
    }

    address public launcher;
    NftListing[] private _listings;
    mapping(bytes32 key => bool) public listed;               // keccak(chainId, collection, policy)
    mapping(address target => bool) public isSweepTarget;
    mapping(address target => uint64) public targetReadyAt;
    mapping(address target => uint64) public targetReadyAtBlock;

    event Wired(address launcher);
    event NftListed(uint256 indexed id, uint64 indexed chainId, address indexed collection, Policy policy, address vault, address buyer, string name);
    event Paused(uint256 indexed id, bool paused);
    event SweepTargetProposed(address indexed target, uint64 readyAt);
    event SweepTargetSet(address indexed target, bool allowed);

    error NotOwner();
    error AlreadyWired();
    error ZeroAddress();
    error Empty();
    error AlreadyListed();
    error NoCode(address collection);
    error BadBuyer(address buyer);
    error NoSuchListing(uint256 id);
    error NotProposed();
    error TooEarly(uint64 readyAt);

    constructor(ListingRegistry listingRegistry_, IRandomnessBeacon beacon_, address seaport) {
        listingRegistry = listingRegistry_;
        beacon = beacon_;
        if (seaport != address(0)) {
            isSweepTarget[seaport] = true;
            emit SweepTargetSet(seaport, true);
        }
    }

    modifier onlyOwner() { if (msg.sender != owner()) revert NotOwner(); _; }

    function owner() public view returns (address) { return listingRegistry.owner(); }
    function keeper() external view returns (address) { return listingRegistry.keeper(); }
    function treasury() external view returns (address) { return listingRegistry.treasury(); }

    /// @notice Once. The NFT launcher, whose routers are the only addresses a vault accepts fees from.
    function wire(address launcher_) external onlyOwner {
        if (launcher != address(0)) revert AlreadyWired();
        if (launcher_ == address(0)) revert ZeroAddress();
        launcher = launcher_;
        emit Wired(launcher_);
    }

    // ---- listings ------------------------------------------------------------------------------------------------------------

    /// @notice A collection on THIS chain (`chainId == block.chainid`): swept through Seaport by anyone, checked by its vault.
    /// Pass `buyer = 0` and `maxInFlight = 0`. A collection on any other chain: bought by `buyer`, the canister's code-less EVM
    /// address, at most `maxInFlight` at once. The same collection may be listed once per policy, so the two fates never share money.
    function listNft(uint64 chainId, address collection, Policy policy, string calldata name, address payable buyer,
        uint256 maxInFlight) external onlyOwner returns (uint256 id)
    {
        if (collection == address(0) || chainId == 0) revert ZeroAddress();
        if (bytes(name).length == 0) revert Empty();
        bytes32 key = keccak256(abi.encode(chainId, collection, policy));
        if (listed[key]) revert AlreadyListed();
        listed[key] = true;
        id = _listings.length;
        address vault;
        if (chainId == block.chainid) {
            if (collection.code.length == 0) revert NoCode(collection);
            if (buyer != address(0) || maxInFlight != 0) revert BadBuyer(buyer);
            vault = address(new NftSweepVault(this, id, collection, policy, beacon));
        } else {
            _checkBuyer(buyer);
            if (maxInFlight == 0) revert Empty();
            vault = address(new NftRemoteVault(this, id, chainId, collection, policy, beacon, buyer, maxInFlight));
        }
        _listings.push(NftListing(chainId, policy, false, collection, vault, buyer, name));
        emit NftListed(id, chainId, collection, policy, vault, buyer, name);
    }

    /// @dev The canister's address may pull ETH to itself, so it must never also be a key that bounds or pauses the pulling
    /// (ListingRegistry's rule for v2 buyers), and it must be an account, not a contract. The keeper as named, paused or not
    /// (`keeper()` is zero while paused; review2 N-6). Then ListingRegistry reserves it
    /// (docs/v4/nft.md §2.1): it re-checks pending roles too, and refuses every later rotation onto it.
    function _checkBuyer(address buyer) private {
        ListingRegistry r = listingRegistry;
        if (buyer == address(0) || buyer.code.length != 0 || buyer == r.owner() || buyer == r.namedKeeper() || buyer == r.purchaser()
            || buyer == r.attestor() || buyer == r.treasury()) revert BadBuyer(buyer);
        r.reserveBuyer(buyer);
    }

    /// @notice Instant, as in ListingRegistry: pausing stops buying only. Money still comes in; raffles and burns still settle.
    function setPaused(uint256 id, bool paused_) external onlyOwner {
        _at(id).paused = paused_;
        emit Paused(id, paused_);
    }

    // ---- the Seaport allow-list ---------------------------------------------------------------------------------------------

    /// @notice A new sweep target waits CHANGE_DELAY in public, on both clocks: a sweep target is handed the vault's ETH.
    function proposeSweepTarget(address target) external onlyOwner {
        if (target == address(0)) revert ZeroAddress();
        targetReadyAt[target] = uint64(block.timestamp + CHANGE_DELAY);
        targetReadyAtBlock[target] = uint64(block.number + Wait.blocksFor(CHANGE_DELAY));
        emit SweepTargetProposed(target, targetReadyAt[target]);
    }

    function acceptSweepTarget(address target) external onlyOwner {
        uint64 at = targetReadyAt[target];
        if (at == 0) revert NotProposed();
        if (!Wait.elapsed(at, targetReadyAtBlock[target])) revert TooEarly(at);
        delete targetReadyAt[target];
        delete targetReadyAtBlock[target];
        isSweepTarget[target] = true;
        emit SweepTargetSet(target, true);
    }

    /// @notice Instant: removing a target (or a pending proposal) can only stop sweeps.
    function removeSweepTarget(address target) external onlyOwner {
        delete targetReadyAt[target];
        delete targetReadyAtBlock[target];
        isSweepTarget[target] = false;
        emit SweepTargetSet(target, false);
    }

    // ---- views ----------------------------------------------------------------------------------------------------------------

    function count() external view returns (uint256) { return _listings.length; }
    function listing(uint256 id) external view returns (NftListing memory) { return _at(id); }
    function vaultOf(uint256 id) external view returns (address) { return _at(id).vault; }
    function isPaused(uint256 id) external view returns (bool) { return _at(id).paused; }
    function isNative(uint256 id) external view returns (bool) { return _at(id).chainId == block.chainid; }

    /// @notice `eip155:<chainId>:<collection>` (CAIP-10 style, lower-case hex), for the app and the canister.
    function ref(uint256 id) external view returns (string memory) {
        NftListing storage l = _at(id);
        return string.concat("eip155:", Strings.toString(l.chainId), ":", Strings.toHexString(l.collection));
    }

    function _at(uint256 id) private view returns (NftListing storage) {
        if (id >= _listings.length) revert NoSuchListing(id);
        return _listings[id];
    }
}
