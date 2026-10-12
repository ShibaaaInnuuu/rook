// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Clones} from "@openzeppelin/contracts/proxy/Clones.sol";
import {ReentrancyGuard} from "@openzeppelin/contracts/utils/ReentrancyGuard.sol";
import {IPonsV2LaunchFactory, IPonsV2FeeEscrow, IPonsV2BondingCurve, IPonsV2MemeHook} from "./interfaces/IPons.sol";
import {ILauncher} from "./interfaces/ILauncher.sol";
import {ListingRegistry} from "./ListingRegistry.sol";
import {IEtchVault} from "./interfaces/IEtchVault.sol";
import {FeeRouter} from "./FeeRouter.sol";

/// @title Launcher
/// @notice Launches a coin on Pons V2 paired for good with one listing: it clones a FeeRouter, names the clone as the coin's
/// creator-fee recipient in the same transaction, and records who the creator is (Pons records this contract).
/// No owner, nothing to configure after deployment.
/// @dev What this does NOT protect against: a creator launching many coins on one listing (each simply has its own router and its
/// own ledger), a misleading coin name, or Pons changing its launch terms — `expectedEconomics` lets the creator pin those.
/// Native-ETH launches only: a coin quoted in an ERC-20 would pay its fees in that token, which the vault does not hold.
contract Launcher is ILauncher, ReentrancyGuard {
    struct CoinInfo { address creator; address router; uint256 listingId; bool exists; }

    struct LaunchParams {
        string name;
        string symbol;
        string logo;
        string description;
        IPonsV2LaunchFactory.Socials socials;
        uint16 creatorTaxBps;
        bytes32 expectedEconomics;   // from Pons's previewLaunchEconomics; zero waives the check
        bytes32 salt;
        uint256 launchConfigId;
        uint256 listingId;
    }

    IPonsV2LaunchFactory public immutable factory;
    IPonsV2FeeEscrow public immutable escrow;
    ListingRegistry public immutable registry;
    address public immutable routerImplementation;

    mapping(address coin => CoinInfo) private _coins;
    address[] public coins;

    event Launched(address indexed coin, address indexed creator, uint256 indexed listingId, address curve, address router);

    error ListingPaused();
    error WrongLaunchFee(uint256 sent, uint256 required);

    constructor(IPonsV2LaunchFactory factory_, ListingRegistry registry_) {
        factory = factory_;
        escrow = IPonsV2FeeEscrow(factory_.feeEscrow());
        registry = registry_;
        routerImplementation = address(new FeeRouter());
    }

    /// @notice Send exactly Pons's launch fee. The creator is exempted from the launch-second snipe tax, as Pons intends for a
    /// launcher; nobody else is.
    function launch(LaunchParams calldata p) external payable nonReentrant returns (address coin, address curve, address router) {
        if (registry.isPaused(p.listingId)) revert ListingPaused();     // also reverts for a listing that does not exist
        uint256 fee = factory.launchFee();
        if (msg.value != fee) revert WrongLaunchFee(msg.value, fee);

        router = Clones.cloneDeterministic(routerImplementation, keccak256(abi.encode(msg.sender, p.salt)));
        address[] memory exempt = new address[](1);
        exempt[0] = msg.sender;
        (coin, curve) = factory.launchToken{value: fee}(
            IPonsV2LaunchFactory.TokenParams({
                name: p.name, symbol: p.symbol, logo: p.logo, description: p.description, socials: p.socials,
                creatorFeeRecipient: router, creatorTaxBps: p.creatorTaxBps, buybackEnabled: false,
                expectedEconomics: p.expectedEconomics, salt: p.salt
            }),
            p.launchConfigId, address(0), exempt
        );
        FeeRouter(payable(router)).initialize(coin, registry, IEtchVault(registry.vaultOf(p.listingId)), escrow,
            IPonsV2BondingCurve(curve), IPonsV2MemeHook(factory.memeHook()));
        _coins[coin] = CoinInfo(msg.sender, router, p.listingId, true);
        coins.push(coin);
        emit Launched(coin, msg.sender, p.listingId, curve, router);
    }

    function routerOf(address coin) external view returns (address) { return _coins[coin].router; }
    function creatorOf(address coin) external view returns (address) { return _coins[coin].creator; }
    function listingOf(address coin) external view returns (uint256) { return _coins[coin].listingId; }
    function coinCount() external view returns (uint256) { return coins.length; }
    function predictRouter(address creator, bytes32 salt) external view returns (address) {
        return Clones.predictDeterministicAddress(routerImplementation, keccak256(abi.encode(creator, salt)), address(this));
    }
}
