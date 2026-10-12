// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Strings} from "@openzeppelin/contracts/utils/Strings.sol";
import {DeployChecks} from "./DeployChecks.sol";
import {BtcAddress} from "../src/lib/BtcAddress.sol";

/// @title GoLiveChecks
/// @notice docs/deploy/contracts-mainnet.md. The refusals for the two v2/v3 steps that come AFTER script/Deploy.s.sol: naming an
/// autonomous buyer in a listing (script/ListAuto.s.sol) and deploying the wrapper pair (script/DeployWrapClaim.s.sol). No cheatcodes, so a
/// test can run every one of them. Same rule as DeployChecks: nothing proceeds unless the person running it has typed out which
/// chain, which sender and — for the one address that can never be changed afterwards — which buyer they mean.
library GoLiveChecks {
    error UnknownChain(uint256 chainId);
    error ConfirmMismatch(string expected);
    error RegistryIsNotAContract();
    error BuyerMissing();
    error BuyerHasCode();
    error BuyerConfirmMismatch(string expected);
    error MaxInFlightZero();
    error TextMissing();
    error RootMissing();
    error DivisibilityTooLarge();
    error WrongDistributor(address registryHas, address given);
    error WrongBeacon(address distributorHas, address given);
    error ListingNotAutonomous(uint256 listingId);
    error ListingIsRune(uint256 listingId);
    error CustodianMissing();
    error CustodyNotTaproot();
    error CustodyIsNotTheListingsAddress(string listingHas, string given);

    // ---- naming a buyer: registry.listInscriptionsAuto / listRuneAuto ---------------------------------------------------------

    /// @notice `buyer-is:<chainid>:<buyer, lowercase hex>`. The buyer is immutable in every vault it is named in — there is no key
    /// anybody can rotate to point the money elsewhere — so the person listing types it out, for this chain, before anything is sent
    /// or printed. It must be the canister's own `evm_address()`, read from the IC (docs/deploy/canister-mainnet.md, step 6).
    function expectedBuyerConfirm(uint256 chainId, address buyer) internal pure returns (string memory) {
        return string.concat("buyer-is:", Strings.toString(chainId), ":", Strings.toHexString(buyer));
    }

    struct ListPlan {
        uint256 chainId;
        address registry;
        bool registryHasCode;
        bool rune;                // listRuneAuto rather than listInscriptionsAuto
        string name;
        string ref;               // a collection's slug, or a rune's id "block:tx"
        bytes32 root;             // inscriptions only
        uint8 divisibility;       // runes only
        string btcVault;
        address buyer;
        bool buyerHasCode;
        string buyerConfirm;      // ETCH_BUYER_CONFIRM, as typed
        uint256 maxInFlight;
    }

    function checkList(ListPlan memory p) internal pure {
        if (p.chainId != DeployChecks.ROBINHOOD && !DeployChecks.isOurs(p.chainId)) revert UnknownChain(p.chainId);
        if (!p.registryHasCode) revert RegistryIsNotAContract();
        if (bytes(p.name).length == 0 || bytes(p.ref).length == 0 || bytes(p.btcVault).length == 0) revert TextMissing();
        if (p.rune) {
            if (p.divisibility > 38) revert DivisibilityTooLarge();   // the runes protocol's own maximum, as the registry checks
        } else if (p.root == bytes32(0)) {
            revert RootMissing();
        }
        if (p.buyer == address(0)) revert BuyerMissing();
        // the buyer is the canister's threshold-key address: an externally owned account with no code. A contract here is a
        // mistake (a multisig, a router, the vault itself), and the vault would release ETH to it forever.
        if (p.buyerHasCode) revert BuyerHasCode();
        string memory expected = expectedBuyerConfirm(p.chainId, p.buyer);
        if (keccak256(bytes(p.buyerConfirm)) != keccak256(bytes(expected))) revert BuyerConfirmMismatch(expected);
        if (p.maxInFlight == 0) revert MaxInFlightZero();
    }

    // ---- deploying the wrapper pair: WrapClaim, which builds its own Wrapper ------------------------------------------------

    /// @notice `deploy-etch-wrapper:<chainid>:<deployer, lowercase hex>`. A fourth word beside DeployChecks' three, so no confirm
    /// string for one step can stand in for another's. One confirm deploys the pair: src/WrapClaim.sol creates its Wrapper in its
    /// constructor and names itself the Wrapper's `distribution`, so there is no separate Wrapper deployment to confirm.
    function expectedWrapperConfirm(uint256 chainId, address deployer) internal pure returns (string memory) {
        return string.concat("deploy-etch-wrapper:", Strings.toString(chainId), ":", Strings.toHexString(deployer));
    }

    struct WrapperPlan {
        uint256 chainId;
        address deployer;
        string confirm;           // ETCH_WRAPPER_CONFIRM, as typed
        address registry;
        bool registryHasCode;
        address registryDistributor;   // what the registry was wired with
        address distributor;           // what the wrapper would be built with
        address distributorBeacon;     // the distributor's own beacon
        address beacon;                // what the wrapper would be built with
        uint256 listingId;
        bool listingAutonomous;
        bool listingIsRune;
        string name;
        string symbol;
        address custodian;             // ETCH_WRAPPER_CUSTODIAN: the buyer canister's EVM address, allowed to record deliveries
        string custodyAddress;         // ETCH_WRAPPER_CUSTODY_ADDRESS: the canister's CUSTODY taproot address (not the buyer's)
        string listingBtcVault;        // where the listing delivers purchased inscriptions
    }

    function checkWrapper(WrapperPlan memory p) internal pure {
        if (p.chainId != DeployChecks.ROBINHOOD && !DeployChecks.isOurs(p.chainId)) revert UnknownChain(p.chainId);
        string memory expected = expectedWrapperConfirm(p.chainId, p.deployer);
        if (keccak256(bytes(p.confirm)) != keccak256(bytes(expected))) revert ConfirmMismatch(expected);
        if (!p.registryHasCode) revert RegistryIsNotAContract();
        // the wrapper resolves tickets through the Distributor's lots and the beacon's rounds; a wrapper built against another pair
        // would read a different draw's state than the one the registry's Distributor keeps
        if (p.distributor != p.registryDistributor) revert WrongDistributor(p.registryDistributor, p.distributor);
        if (p.beacon != p.distributorBeacon) revert WrongBeacon(p.distributorBeacon, p.beacon);
        if (p.listingIsRune) revert ListingIsRune(p.listingId);
        // docs/v3/wrapper.md: the custodian is the v2 buyer. On a public chain a wrapper over a bonded (v1) vault is refused; on our
        // own chains it is allowed so the contract can be exercised against the v1 demo listings.
        if (!DeployChecks.isOurs(p.chainId) && !p.listingAutonomous) revert ListingNotAutonomous(p.listingId);
        // No distribution input: docs/v3/wrapper.md decision 2 (tokens minted to step 2's claim contract) holds by construction,
        // because WrapClaim is that contract and passes itself. script/golive-check.sh reads the pairing back from the chain.
        if (bytes(p.name).length == 0 || bytes(p.symbol).length == 0) revert TextMissing();
        // docs/v3/wrapper.md, "Custody on its own key": the custodian records its own deliveries, and frogs are held at custody.
        // The wrapper would refuse every wrap while the listing delivers elsewhere, so a pair like that is refused here instead.
        if (p.custodian == address(0)) revert CustodianMissing();
        if (!BtcAddress.isTaproot(p.custodyAddress)) revert CustodyNotTaproot();
        if (keccak256(bytes(p.listingBtcVault)) != keccak256(bytes(p.custodyAddress))) {
            revert CustodyIsNotTheListingsAddress(p.listingBtcVault, p.custodyAddress);
        }
    }
}
