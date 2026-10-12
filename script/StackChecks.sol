// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Strings} from "@openzeppelin/contracts/utils/Strings.sol";
import {DeployChecks} from "./DeployChecks.sol";
import {BtcAddress} from "../src/lib/BtcAddress.sol";
import {CurveFactory} from "../src/curve/CurveFactory.sol";
import {FloorModule} from "../src/floor/FloorModule.sol";

/// @title StackChecks
/// @notice docs/v6/deploy-stack.md §4. Every reason script/DeployStack.s.sol refuses, in one place, with no cheatcodes, so that a
/// test can run each of them. Same rule as DeployChecks: nothing proceeds unless the person running it has typed out which chain
/// and which sender they mean, and every input the chain will hold for good has been looked at before anything is broadcast.
library StackChecks {
    error UnknownChain(uint256 chainId);
    error ConfirmMismatch(string expected);
    error RegistryIsNotAContract();
    error RegistryNotWired();
    error ListingIsRune(uint256 listingId);
    error ListingNotAutonomous(uint256 listingId);
    error CustodianMissing();
    error CustodianHasCode();
    error CustodyNotTaproot();
    error PoolManagerMissing();
    error TextMissing();
    error StageNotZero(uint256 stage);
    error StageTooHigh(uint256 stage);
    error ParamsOutOfBounds(string which);
    error NftCollectionMissing();
    error NftChainIsThisChain();
    error NftCollectionTwice(address collection);
    error NftListsDiffer();
    error NftSlugBad(string slug);
    error NftSlugTwice(string slug);

    /// @notice `deploy-etch-stack:<chainid>:<deployer, lowercase hex>`. A sixth word beside the five that exist (deploy-etch,
    /// deploy-etch-test, deploy-etch-treasury, deploy-etch-wrapper, deploy-etch-nft): no step's confirm stands in for another's.
    function expectedConfirm(uint256 chainId, address deployer) internal pure returns (string memory) {
        return string.concat("deploy-etch-stack:", Strings.toString(chainId), ":", Strings.toHexString(deployer));
    }

    /// @notice docs/v5/launch-gates.md G7: the stage's per-collection numbers this stack can hold on chain. The table's other
    /// columns (custody items and ETH, outbound values) are the canister's install argument; they are written into the file
    /// beside these so the go-live check can compare the two.
    struct Stage {
        uint256 mintsPerDay;          // Wrapper.initMintLimit: mints a day a collection
        uint256 itemsPerDay;          // FloorModule.Params.maxItemsPerDay: item arrivals the module pays for in 24 h
        uint256 custodyItems;         // items in custody a collection (informational: the canister's)
        uint256 custodyEth;           // wei of custody a collection (informational: the canister's)
        uint256 timelockDelayHours;   // informational
    }

    function stageOf(uint256 stage) internal pure returns (Stage memory s) {
        if (stage == 0) return Stage(3, 3, 3, 10 ether, 72);
        if (stage == 1) return Stage(6, 6, 12, 40 ether, 72);
        if (stage == 2) return Stage(12, 12, 48, 160 ether, 72);
        if (stage == 3) return Stage(24, 24, 192, 640 ether, 24 * 7);
        revert StageTooHigh(stage);
    }

    struct Plan {
        uint256 chainId;
        address deployer;
        string confirm;               // ETCH_STACK_CONFIRM, as typed
        address registry;
        bool registryHasCode;
        address registryDistributor;  // zero: the registry is not wired, and the wrapper could not read its beacon
        uint256 listingId;
        bool listingIsRune;
        bool listingAutonomous;
        address custodian;            // the canister's evm_address()
        bool custodianHasCode;
        string custodyAddress;        // the canister's custody taproot address
        string name;
        string symbol;
        bool poolManagerHasCode;
        uint256 stage;                // G7: 0 on a public chain's first deployment
        bool nft;                     // an EVM collection (beside the Bitcoin one, or alone: nftOnly)
        bool nftOnly;                 // docs/v7/phase1-nft.md §3.4: the EVM collection alone
        uint64 nftChainId;
        address nftCollection;
    }

    /// docs/v7/phase1-nft.md §9 D1 (2026-09-27): several Ethereum collections in one stack, each named once, each with its name,
    /// symbol and slug (one wrapper, poster and module per collection; nothing shared between them but the stack's treasury,
    /// directory and registry).
    function checkNftSet(address[] memory cols, uint256 names, uint256 symbols, uint256 slugs) internal pure {
        if (cols.length == 0) revert NftCollectionMissing();
        if (names != cols.length || symbols != cols.length || slugs != cols.length) revert NftListsDiffer();
        for (uint256 i = 0; i < cols.length; ++i) {
            if (cols[i] == address(0)) revert NftCollectionMissing();
            for (uint256 j = 0; j < i; ++j) if (cols[j] == cols[i]) revert NftCollectionTwice(cols[i]);
        }
    }

    /// Audit 2026-09-27-nft D-L-7: the stack file keys each collection by its slug (`"col-" + slug`, and `cols.<slug>`), so a
    /// repeated slug would silently drop a deployed, treasury-registered collection from the file (and from golive-check, the keeper,
    /// the monitor and the canister's list). Each slug is non-empty, `[a-z0-9-]+`, and named once.
    function checkNftSlugs(string[] memory slugs) internal pure {
        for (uint256 i = 0; i < slugs.length; ++i) {
            bytes memory b = bytes(slugs[i]);
            if (b.length == 0 || b.length > 64) revert NftSlugBad(slugs[i]);
            for (uint256 k = 0; k < b.length; ++k) {
                bytes1 x = b[k];
                if (!((x >= "a" && x <= "z") || (x >= "0" && x <= "9") || x == "-")) revert NftSlugBad(slugs[i]);
            }
            for (uint256 j = 0; j < i; ++j) if (keccak256(b) == keccak256(bytes(slugs[j]))) revert NftSlugTwice(slugs[i]);
        }
    }

    function check(Plan memory p) internal pure {
        if (p.chainId != DeployChecks.ROBINHOOD && !DeployChecks.isOurs(p.chainId)) revert UnknownChain(p.chainId);
        string memory expected = expectedConfirm(p.chainId, p.deployer);
        if (keccak256(bytes(p.confirm)) != keccak256(bytes(expected))) revert ConfirmMismatch(expected);
        if (!p.registryHasCode) revert RegistryIsNotAContract();
        if (p.registryDistributor == address(0)) revert RegistryNotWired();
        if (p.listingIsRune) revert ListingIsRune(p.listingId);
        // docs/v3/wrapper.md: the custodian is the v2 buyer. On a public chain a wrapper over a bonded (v1) vault is refused; on
        // our own chains it is allowed so the stack can be exercised against the v1 demo listings.
        if (!DeployChecks.isOurs(p.chainId) && !p.listingAutonomous) revert ListingNotAutonomous(p.listingId);
        if (p.custodian == address(0)) revert CustodianMissing();
        if (p.custodianHasCode) revert CustodianHasCode();
        if (p.nftOnly && !p.nft) revert NftCollectionMissing();
        if (!p.nftOnly && !BtcAddress.isTaproot(p.custodyAddress)) revert CustodyNotTaproot();
        if (!p.nftOnly && (bytes(p.name).length == 0 || bytes(p.symbol).length == 0)) revert TextMissing();
        if (!p.poolManagerHasCode) revert PoolManagerMissing();
        // launch-gates G7: custody caps start at stage 0. A later stage is a timelocked raise of a RUNNING install, not a deployment.
        if (!DeployChecks.isOurs(p.chainId) && p.stage != 0) revert StageNotZero(p.stage);
        stageOf(p.stage);   // reverts StageTooHigh past the table
        if (p.nft) {
            if (p.nftCollection == address(0) || p.nftChainId == 0) revert NftCollectionMissing();
            if (p.nftChainId == p.chainId) revert NftChainIsThisChain();
        }
    }

    error CollectionOutOfBounds(string what);

    /// @notice docs/v7/phase1-nft-pooled.md §2: one EVM collection's own graduation targets and module conversion / reserve, each
    /// bounded here, before anything is broadcast. `CurveFactory` refuses an early raise above 10 ETH (`HARD_MAX_EARLY_RAISE`).
    function checkNftCollection(uint256 chainId, uint64 nftChainId, address collection, CurveFactory.Targets memory t, uint16 convBps,
        uint16 reserveBps) internal pure {
        if (collection == address(0) || nftChainId == 0) revert NftCollectionMissing();
        if (nftChainId == chainId) revert NftChainIsThisChain();
        if (t.normalRaise < 1 ether || t.normalRaise > 10 ether) revert CollectionOutOfBounds("normalRaise: 1 to 10 ETH");
        if (t.earlyRaise < t.normalRaise || t.earlyRaise > 10 ether) revert CollectionOutOfBounds("earlyRaise: normalRaise to 10 ETH");
        if (t.earlyUntilEth > 20 ether) revert CollectionOutOfBounds("earlyUntilEth: at most 20 ETH");
        if (t.maxRaise < t.earlyRaise || t.maxRaise > 30 ether) revert CollectionOutOfBounds("maxRaise: earlyRaise to 30 ETH");
        if (convBps < 300 || convBps > 1_600) revert CollectionOutOfBounds("convBps: 300 to 1,600");
        if (reserveBps > 2_500) revert CollectionOutOfBounds("reserveBps: at most 2,500");
    }

    /// @notice FloorModule's own constructor bounds, checked here so a bad constant is refused before anything is broadcast rather
    /// than reverting after the wrapper and the desk have been deployed.
    function checkParams(FloorModule.Params memory p) internal pure {
        if (p.reserveBps > 5_000) revert ParamsOutOfBounds("reserveBps");
        if (p.fillBandBps > 1_000) revert ParamsOutOfBounds("fillBandBps");
        if (p.maxSpreadBps > 5_000 || p.baseSpreadBps > p.maxSpreadBps) revert ParamsOutOfBounds("spread");
        if (p.maxMoveBps == 0 || p.maxMoveBps > 2_000) revert ParamsOutOfBounds("maxMoveBps");
        if (p.convBps > 2_000 || p.bufferBps > 2_000) revert ParamsOutOfBounds("convBps/bufferBps");
        if (p.staleSpreadBps > p.maxSpreadBps || p.stressSpreadBps > p.maxSpreadBps) revert ParamsOutOfBounds("stale/stress spread");
        if (p.volWeightBps > 50_000) revert ParamsOutOfBounds("volWeightBps");
        if (p.mintPerHour == 0 || p.burnPerHour == 0) revert ParamsOutOfBounds("per-hour caps");
        if (p.maxPublicEthBacked == 0 || p.maxPublicEthBacked > p.maxEthBacked) revert ParamsOutOfBounds("maxPublicEthBacked");
        if (p.rentTerm < 1 days || p.premiumBps > 5_000 || p.rentShareBps > 10_000) revert ParamsOutOfBounds("rental");
        if (p.lowConfidenceSpreadBps > p.maxSpreadBps || p.fullConfidenceBps == 0 || p.fullConfidenceBps > 10_000) revert ParamsOutOfBounds("confidence");
        if (p.minConfidenceBps > p.fullConfidenceBps || p.maxItemsPerDay == 0) revert ParamsOutOfBounds("minConfidence/maxItemsPerDay");
    }

    /// @notice The frog defaults of docs/v5/floor-module.md §6 and §13, in whole items (`unit` = 1e18 for frogs; a dearer
    /// collection scales the per-item counts down through `unit`), with the stage's `maxItemsPerDay`.
    function defaultParams(uint256 unit, uint256 stage) internal pure returns (FloorModule.Params memory p) {
        p.baseSpreadBps = 15;
        p.volWeightBps = 10_000;
        p.staleSpreadBps = 100;
        p.stressSpreadBps = 300;
        p.maxSpreadBps = 1_000;
        p.maxMoveBps = 500;
        p.convBps = 300;
        p.bufferBps = 500;
        p.mintPerHour = 5 * unit;
        p.burnPerHour = 5 * unit;
        p.maxPublicEthBacked = 10 * unit;
        p.maxEthBacked = 40 * unit;
        p.revenueTarget = 1 ether;
        p.rentTerm = 30 days;
        p.premiumBps = 1_000;
        p.rentShareBps = 5_000;
        p.maxRentals = 20;
        p.lowConfidenceSpreadBps = 100;
        p.fullConfidenceBps = 8_000;
        p.minConfidenceBps = 3_000;
        p.maxItemsPerDay = uint16(stageOf(stage).itemsPerDay);
        p.reserveBps = 2_500;
        p.fillBandBps = 300;
    }

    /// @notice Where the stack file is written. Never a real chain's existing file (the shell refuses first; the script again).
    function fileFor(uint256 chainId) internal pure returns (string memory) {
        return string.concat("deployments/stack-", Strings.toString(chainId), ".json");
    }
}
