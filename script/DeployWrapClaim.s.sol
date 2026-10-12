// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Script, console2} from "forge-std/Script.sol";
import {ListingRegistry} from "../src/ListingRegistry.sol";
import {Distributor} from "../src/Distributor.sol";
import {Wrapper} from "../src/Wrapper.sol";
import {WrapClaim} from "../src/WrapClaim.sol";
import {IRandomnessBeacon} from "../src/interfaces/IRandomnessBeacon.sol";
import {DeployChecks} from "./DeployChecks.sol";
import {GoLiveChecks} from "./GoLiveChecks.sol";

/// @title DeployWrapClaim
/// @notice docs/deploy/contracts-mainnet.md, step 6. Deploys ONE WrapClaim (docs/v3/wrapper.md, step 2) over one inscriptions
/// listing, against the EXISTING registry, Distributor and beacon. WrapClaim builds its own Wrapper in its constructor and names
/// itself that Wrapper's `distribution`, so this is one CREATE and two contracts; the Wrapper's address is read back from
/// `wrapClaim.wrapper()` and both are written to deployments/wrapper-<chainid>-<listing>.json. The deployer owns nothing: neither
/// contract has an owner of its own; WrapClaim's `cancel` is the registry owner's, for 15 minutes after a root.
///
///   ETCH_WRAPPER_CONFIRM=deploy-etch-wrapper:<chainid>:<deployer, lowercase> ETCH_REGISTRY=0x… ETCH_WRAPPER_LISTING=<id> \
///   ETCH_WRAPPER_NAME="Wrapped Bitcoin Frog" ETCH_WRAPPER_SYMBOL=wFROG \
///   ETCH_WRAPPER_CUSTODIAN=<the canister's evm_address()> ETCH_WRAPPER_CUSTODY_ADDRESS=<its custody_address()> \
///   script/deploy-wrapclaim.sh [<rpc>] <deployer> [--ledger|--trezor]
/// Nothing here reads a private key.
contract DeployWrapClaim is Script {
    function run() external {
        address deployer = msg.sender;
        ListingRegistry registry = ListingRegistry(vm.envAddress("ETCH_REGISTRY"));
        uint256 listingId = vm.envUint("ETCH_WRAPPER_LISTING");
        bool registryHasCode = address(registry).code.length != 0;

        GoLiveChecks.WrapperPlan memory p;
        p.chainId = block.chainid;
        p.deployer = deployer;
        p.confirm = vm.envOr("ETCH_WRAPPER_CONFIRM", string(""));
        p.registry = address(registry);
        p.registryHasCode = registryHasCode;
        p.name = vm.envOr("ETCH_WRAPPER_NAME", string(""));
        p.symbol = vm.envOr("ETCH_WRAPPER_SYMBOL", string(""));
        p.listingId = listingId;
        p.custodian = vm.envOr("ETCH_WRAPPER_CUSTODIAN", address(0));
        p.custodyAddress = vm.envOr("ETCH_WRAPPER_CUSTODY_ADDRESS", string(""));
        if (registryHasCode) {   // read the rest only from a registry that exists; the check refuses otherwise
            p.registryDistributor = registry.distributor();
            p.distributor = p.registryDistributor;
            p.distributorBeacon = p.distributor == address(0) ? address(0) : address(Distributor(p.distributor).beacon());
            p.beacon = p.distributorBeacon;
            ListingRegistry.Listing memory l = registry.listing(listingId);
            p.listingAutonomous = l.autonomous;
            p.listingIsRune = l.kind == ListingRegistry.Kind.Rune;
            p.listingBtcVault = l.btcVault;
        }
        GoLiveChecks.checkWrapper(p);

        bool test = keccak256(bytes(vm.envOr("ETCH_DEPLOY_LABEL", string("")))) == keccak256("test");
        string memory file = string.concat("deployments/", test ? "test-" : "", "wrapper-", vm.toString(block.chainid), "-",
            vm.toString(listingId), ".json");
        if (!DeployChecks.isOurs(block.chainid)) require(!vm.exists(file), "a wrapper deployment file is never overwritten on a real chain");

        uint256 parentBlockSeconds = vm.envOr("ETCH_PARENT_BLOCK_SECONDS", DeployChecks.parentBlockSeconds(block.chainid));
        require(parentBlockSeconds != 0, "unknown chain: set ETCH_PARENT_BLOCK_SECONDS to its measured parent block time");
        vm.startBroadcast(deployer);
        WrapClaim c = new WrapClaim(registry, Distributor(p.distributor), IRandomnessBeacon(p.beacon), listingId, p.custodian, p.custodyAddress,
            p.name, p.symbol, parentBlockSeconds);
        vm.stopBroadcast();
        Wrapper w = c.wrapper();

        // read back the pair, before anything is written down: each points at the other, and both at this registry's draw
        require(address(c.registry()) == address(registry) && c.listingId() == listingId, "read-back: claim");
        require(address(w.registry()) == address(registry) && address(w.distributor()) == p.distributor
            && address(w.beacon()) == p.beacon && w.listingId() == listingId && w.distribution() == address(c)
            && w.totalSupply() == 0 && w.custodian() == p.custodian
            && w.parentBlockSeconds() == parentBlockSeconds && c.parentBlockSeconds() == parentBlockSeconds
            && keccak256(bytes(w.custodyAddress())) == keccak256(bytes(p.custodyAddress)), "read-back: wrapper");

        string memory k = "wrapper";
        vm.serializeUint(k, "chainId", block.chainid);
        vm.serializeAddress(k, "wrapClaim", address(c));
        vm.serializeAddress(k, "wrapper", address(w));
        vm.serializeAddress(k, "registry", address(registry));
        vm.serializeAddress(k, "distributor", p.distributor);
        vm.serializeAddress(k, "beacon", p.beacon);
        vm.serializeUint(k, "listingId", listingId);
        vm.serializeAddress(k, "custodian", p.custodian);
        vm.serializeString(k, "custodyAddress", p.custodyAddress);
        vm.serializeString(k, "name", p.name);
        string memory json = vm.serializeString(k, "symbol", p.symbol);
        vm.writeJson(json, file);
        console2.log("wrapClaim", address(c));
        console2.log("wrapper  ", address(w), "->", file);
    }
}
