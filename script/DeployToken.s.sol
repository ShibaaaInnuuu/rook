// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Script, console2} from "forge-std/Script.sol";
import {ListingRegistry} from "../src/ListingRegistry.sol";
import {Treasury} from "../src/Treasury.sol";
import {DeployChecks} from "./DeployChecks.sol";

/// @title DeployToken
/// @notice docs/token/protocol-token-v1.md §7. Deploys the Treasury against an EXISTING registry and prints the calls the multisig
/// must make itself. It does NOT launch the protocol token: how that token is issued, at what supply and with what allocations is
/// the operator's decision (§6 (a)–(d)) and is made on the Launcher like any other coin, not here.
///
/// The deployer owns nothing and this script sends nothing but the one CREATE. Everything afterwards is owner-only and waits seven
/// public days, so the sequence is printed rather than performed:
///   1. registry.proposeTreasury(treasury)   — the owner. Seven days in public before it can be accepted.
///   2. registry.acceptTreasury()            — the owner, once those seven days have passed.
///   3. treasury.setToken(token)             — the owner, once, after the token has been launched through the Launcher.
/// Until (2), every router still pays the old treasury address; until (3), nothing here can be funded at all.
///
/// @dev NOT RUN. docs/token/protocol-token-v1.md §5: counsel must answer the questions there before this reaches a real chain.
///   ETCH_TREASURY_CONFIRM=deploy-etch-treasury:<chainid>:<deployer, lowercase> ETCH_REGISTRY=0x… [ETCH_TOKEN=0x…] \
///   forge script script/DeployToken.s.sol:DeployToken --rpc-url <rpc> --sender <deployer> --unlocked|--ledger --broadcast
/// Nothing here reads a private key.
contract DeployToken is Script {
    function run() external {
        address deployer = msg.sender;
        address registryAddr = vm.envOr("ETCH_REGISTRY", address(0));
        address token = vm.envOr("ETCH_TOKEN", address(0));

        DeployChecks.checkTreasury(DeployChecks.TreasuryPlan({
            chainId: block.chainid,
            deployer: deployer,
            confirm: vm.envOr("ETCH_TREASURY_CONFIRM", string("")),
            registry: registryAddr,
            token: token,
            registryHasCode: registryAddr.code.length != 0,
            tokenHasCode: token.code.length != 0
        }));

        ListingRegistry registry = ListingRegistry(registryAddr);
        address owner = registry.owner();

        vm.startBroadcast(deployer);
        Treasury treasury = new Treasury(registry);
        vm.stopBroadcast();

        console2.log("Treasury:", address(treasury));
        console2.log("  registry:", registryAddr);
        console2.log("  owner (it obeys the registry's owner, and owns nothing itself):", owner);
        console2.log("  the twenty per cent reaches it only after step 2 below, and can be spent only after step 3.");
        console2.log("The owner must now make these calls. Nobody else can, and nothing here can be undone by anyone else:");
        console2.log("  1. registry.proposeTreasury(treasury)   [then seven public days]");
        console2.logBytes(abi.encodeCall(ListingRegistry.proposeTreasury, (address(treasury))));
        console2.log("  2. registry.acceptTreasury()");
        console2.logBytes(abi.encodeCall(ListingRegistry.acceptTreasury, ()));
        if (token != address(0)) {
            console2.log("  3. treasury.setToken(token)   [once, and never again]", token);
            console2.logBytes(abi.encodeCall(Treasury.setToken, (token)));
        } else {
            console2.log("  3. treasury.setToken(token) once the protocol token has been launched through the Launcher.");
            console2.log("     Set ETCH_TOKEN and run this script again to print that call; it does not need a second deployment.");
        }

        string memory k = "treasury";
        vm.serializeUint(k, "chainId", block.chainid);
        vm.serializeAddress(k, "registry", registryAddr);
        vm.serializeAddress(k, "token", token);
        string memory out = vm.serializeAddress(k, "treasury", address(treasury));
        vm.writeJson(out, string.concat("deployments/treasury-", vm.toString(block.chainid), ".json"));
    }
}
