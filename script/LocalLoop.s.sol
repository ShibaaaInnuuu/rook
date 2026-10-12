// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Script, console2} from "forge-std/Script.sol";
import {ListingRegistry} from "../src/ListingRegistry.sol";
import {AutoBuyerVault} from "../src/AutoBuyerVault.sol";
import {Distributor} from "../src/Distributor.sol";
import {MockLauncher, MockBeacon} from "../test/utils/Mocks.sol";

/// @title LocalLoop
/// @notice docs/v2/buyer-loop.md's integration run: the real `AutoBuyerVault`, on a LOCAL anvil chain only, with the buyer
/// canister's own EVM address (`BUYER`) as its buyer, fees deposited and a cap posted, so the canister's loop can announce, pull
/// and return against it. Refuses any chain but 31337. Senders are anvil's unlocked dev accounts (`--unlocked`): no key is read.
contract LocalLoop is Script {
    function run() external {
        require(block.chainid == 31337, "LocalLoop runs on a local anvil chain (31337) and nowhere else");
        address payable buyer = payable(vm.envAddress("BUYER"));
        address owner = vm.envAddress("OWNER");
        address keeper = vm.envAddress("KEEPER");
        address router = vm.envAddress("ROUTER");
        address coin = address(0xC014);

        vm.startBroadcast(owner);
        ListingRegistry registry = new ListingRegistry(owner, owner);
        MockLauncher launcher = new MockLauncher();
        Distributor distributor = new Distributor(registry, new MockBeacon(), 12);
        registry.wire(address(launcher), address(distributor));
        registry.proposeRoles(keeper, address(0xB0B), address(0xA77));
        uint256 id = registry.listRuneAuto("LOOP.RUNE", "840000:3", 0, "bc1p-loop", buyer, 20 ether);
        launcher.set(coin, router, owner, id);
        vm.stopBroadcast();

        AutoBuyerVault vault = AutoBuyerVault(payable(registry.vaultOf(id)));
        vm.startBroadcast(router);
        vault.deposit{value: 5 ether}(coin);
        vm.stopBroadcast();
        vm.startBroadcast(keeper);
        vault.postCap(0.01 ether); // wei per whole rune unit, divisibility 0: 1,000 units may cost up to 10 ETH
        vm.stopBroadcast();

        console2.log("VAULT", address(vault));
        console2.log("LISTING", id);
    }
}
