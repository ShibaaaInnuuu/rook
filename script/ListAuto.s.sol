// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Script, console2} from "forge-std/Script.sol";
import {ListingRegistry} from "../src/ListingRegistry.sol";
import {AutoBuyerVault} from "../src/AutoBuyerVault.sol";
import {DeployChecks} from "./DeployChecks.sol";
import {GoLiveChecks} from "./GoLiveChecks.sol";

/// @title ListAuto
/// @notice docs/deploy/contracts-mainnet.md, step 4. Lists a collection or a rune whose vault releases ETH to ONE autonomous buyer —
/// the canister — fixed for the life of the vault. The registry creates the AutoBuyerVault itself; nothing is deployed from here.
///
/// On a public chain this script SENDS NOTHING: listing is owner-only and the owner is the multisig, so it prints the target and the
/// calldata for the multisig to propose. On our own chains (31337, the staging fork) the owner is an unlocked node account and the
/// script makes the call itself, then reads the new vault back.
///
///   ETCH_REGISTRY=0x… ETCH_LIST_KIND=inscriptions|rune ETCH_LIST_NAME=… ETCH_LIST_REF=<slug | block:tx> \
///   [ETCH_LIST_ROOT=0x… | ETCH_LIST_DIVISIBILITY=n] ETCH_LIST_BTC_VAULT=bc1p… ETCH_BUYER=0x… \
///   ETCH_BUYER_CONFIRM=buyer-is:<chainid>:<buyer, lowercase> ETCH_MAX_IN_FLIGHT=<wei> \
///   forge script script/ListAuto.s.sol:ListAuto --rpc-url <rpc> [--broadcast: our chains only]
/// Nothing here reads a private key.
contract ListAuto is Script {
    function run() external {
        address registryAddr = vm.envAddress("ETCH_REGISTRY");
        bool rune = keccak256(bytes(vm.envOr("ETCH_LIST_KIND", string("")))) == keccak256("rune");
        GoLiveChecks.ListPlan memory p = GoLiveChecks.ListPlan({
            chainId: block.chainid,
            registry: registryAddr,
            registryHasCode: registryAddr.code.length != 0,
            rune: rune,
            name: vm.envOr("ETCH_LIST_NAME", string("")),
            ref: vm.envOr("ETCH_LIST_REF", string("")),
            root: vm.envOr("ETCH_LIST_ROOT", bytes32(0)),
            divisibility: uint8(vm.envOr("ETCH_LIST_DIVISIBILITY", uint256(0))),
            btcVault: vm.envOr("ETCH_LIST_BTC_VAULT", string("")),
            buyer: vm.envOr("ETCH_BUYER", address(0)),
            buyerHasCode: false,
            buyerConfirm: vm.envOr("ETCH_BUYER_CONFIRM", string("")),
            maxInFlight: vm.envOr("ETCH_MAX_IN_FLIGHT", uint256(0))
        });
        p.buyerHasCode = p.buyer.code.length != 0;
        GoLiveChecks.checkList(p);

        ListingRegistry registry = ListingRegistry(registryAddr);
        bytes memory data = rune
            ? abi.encodeCall(ListingRegistry.listRuneAuto, (p.name, p.ref, p.divisibility, p.btcVault, payable(p.buyer), p.maxInFlight))
            : abi.encodeCall(ListingRegistry.listInscriptionsAuto, (p.name, p.ref, p.root, p.btcVault, payable(p.buyer), p.maxInFlight));

        if (!DeployChecks.isOurs(block.chainid)) {
            console2.log("NOTHING WAS SENT. The owner (multisig) proposes this call:");
            console2.log("  to:     ", registryAddr);
            console2.log("  owner:  ", registry.owner());
            console2.log("  buyer:  ", p.buyer, "(immutable in this vault, forever)");
            console2.log("  listing id it will get:", registry.count());
            console2.log("  calldata:");
            console2.logBytes(data);
            return;
        }

        uint256 id = registry.count();
        vm.startBroadcast(registry.owner());
        (bool ok, bytes memory ret) = registryAddr.call(data);
        vm.stopBroadcast();
        if (!ok) {
            assembly { revert(add(ret, 32), mload(ret)) }
        }
        AutoBuyerVault v = AutoBuyerVault(payable(registry.vaultOf(id)));
        require(v.buyer() == p.buyer && v.maxInFlight() == p.maxInFlight && v.listingId() == id && v.isRune() == rune, "read-back");
        console2.log("listed", id, "vault", address(v));
    }
}
