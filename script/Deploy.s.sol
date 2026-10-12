// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Script, console2} from "forge-std/Script.sol";
import {IPonsV2LaunchFactory} from "../src/interfaces/IPons.sol";
import {ListingRegistry} from "../src/ListingRegistry.sol";
import {Launcher} from "../src/Launcher.sol";
import {Distributor} from "../src/Distributor.sol";
import {DrandEvmnet} from "../src/rand/DrandEvmnet.sol";
import {DeployChecks} from "./DeployChecks.sol";

/// @title Deploy
/// @notice docs/phase2.md §1. Deploys the registry (owned by the multisig from its first block), the drand verifier, the Launcher
/// (which deploys the router implementation it clones) and the Distributor, and writes deployments/<chainid>.json.
/// The deployer never owns anything. On a public chain the two owner-only wiring calls are PRINTED for the multisig to make; on the
/// local chain (31337) the "multisig" is an unlocked dev account and the script makes them, and lists two demo listings.
/// @dev Run through script/deploy.sh, which checks the RPC's chain id before forge is even started. Nothing here reads a private key:
/// the sender is whatever `--sender` names, unlocked at the node or held by a hardware wallet.
contract Deploy is Script {
    IPonsV2LaunchFactory constant PONS_FACTORY = IPonsV2LaunchFactory(0x7eD598BcEf8bd9Edd8C97A195C6d13f40801EC7e);

    /// @dev src/lib/Wait.sol: the chain's measured parent block time, from the table or the environment; never guessed.
    function parentBlockSeconds() internal view returns (uint256 s) {
        s = vm.envOr("ETCH_PARENT_BLOCK_SECONDS", DeployChecks.parentBlockSeconds(block.chainid));
        require(s != 0, "unknown chain: set ETCH_PARENT_BLOCK_SECONDS to its measured parent block time");
    }

    function run() external {
        address deployer = msg.sender;
        DeployChecks.Plan memory p = DeployChecks.Plan({
            chainId: block.chainid,
            deployer: deployer,
            confirm: vm.envOr("ETCH_DEPLOY_CONFIRM", string("")),
            owner: vm.envOr("ETCH_OWNER", address(0)),
            treasury: vm.envOr("ETCH_TREASURY", address(0)),
            keeper: vm.envOr("ETCH_KEEPER", address(0)),
            purchaser: vm.envOr("ETCH_PURCHASER", address(0)),
            attestor: vm.envOr("ETCH_ATTESTOR", address(0)),
            ownerHasCode: false,
            ponsHasCode: address(PONS_FACTORY).code.length != 0,
            testLabel: keccak256(bytes(vm.envOr("ETCH_DEPLOY_LABEL", string("")))) == keccak256("test")
        });
        // A multisig is a contract. An account with an EIP-7702 delegation also has code — 23 bytes, 0xef0100 ‖ delegate — and is
        // one key, not a multisig. EIP-7702 is live on Robinhood Chain (anvil's well-known dev accounts are delegated there, found by
        // scripts/golive-dryrun.sh on a mainnet fork), so "has code" alone would let a single-key owner pass as a multisig.
        p.ownerHasCode = p.owner.code.length != 0 && !isDelegatedAccount(p.owner.code);
        DeployChecks.check(p);      // every refusal happens here, before anything is broadcast

        vm.startBroadcast(deployer);
        ListingRegistry registry = new ListingRegistry(p.owner, p.treasury);
        DrandEvmnet beacon = new DrandEvmnet();
        Launcher launcher = new Launcher(PONS_FACTORY, registry);
        Distributor distributor = new Distributor(registry, beacon, parentBlockSeconds());
        vm.stopBroadcast();

        bool local = DeployChecks.isOurs(block.chainid);
        if (local) {
            // the owner is an unlocked dev account here, and only here
            vm.startBroadcast(p.owner);
            registry.wire(address(launcher), address(distributor));
            registry.proposeRoles(p.keeper, p.purchaser, p.attestor);
            // two demo listings so the app and the keeper have something to read; the membership root is of ONE demo inscription id
            // On the staging fork the collection has 1,024 fixture members (…i0 to …i1023): an inscription can be held once, so a
            // one-member collection would let a long-lived chain complete exactly one purchase, ever (docs/phase6.md, found by the smoke test).
            registry.listInscriptions("Demo Stones", "demo-stones", block.chainid == DeployChecks.STAGING ? stagingRoot() : registry.leafOf(DEMO_INSCRIPTION), DEMO_BTC_VAULT);
            registry.listRune("DEMO\xe2\x80\xa2RUNE", "840000:1", 2, DEMO_BTC_VAULT);
            vm.stopBroadcast();
        } else {
            console2.log("The deployer owns nothing. The owner (multisig) must now make these two calls on the registry:");
            console2.log("  registry:", address(registry));
            console2.log("  1. wire(launcher, distributor)");
            console2.logBytes(abi.encodeCall(ListingRegistry.wire, (address(launcher), address(distributor))));
            console2.log("  2. proposeRoles(keeper, purchaser, attestor)   (the first roles apply at once)");
            console2.logBytes(abi.encodeCall(ListingRegistry.proposeRoles, (p.keeper, p.purchaser, p.attestor)));
        }

        string memory k = "etch";
        vm.serializeUint(k, "chainId", block.chainid);
        vm.serializeUint(k, "deployedAtBlock", block.number);
        vm.serializeAddress(k, "registry", address(registry));
        vm.serializeAddress(k, "launcher", address(launcher));
        vm.serializeAddress(k, "routerImplementation", launcher.routerImplementation());
        vm.serializeAddress(k, "distributor", address(distributor));
        vm.serializeAddress(k, "beacon", address(beacon));
        vm.serializeAddress(k, "ponsFactory", address(PONS_FACTORY));
        vm.serializeAddress(k, "owner", p.owner);
        vm.serializeAddress(k, "treasury", p.treasury);
        vm.serializeAddress(k, "keeper", p.keeper);
        vm.serializeAddress(k, "purchaser", p.purchaser);
        vm.serializeAddress(k, "attestor", p.attestor);
        vm.serializeBool(k, "wired", local);
        vm.serializeString(k, "label", p.testLabel ? "test" : (block.chainid == DeployChecks.STAGING ? "staging" : (local ? "local" : "mainnet")));
        string memory json = vm.serializeBool(k, "demo", local);
        vm.writeJson(json, DeployChecks.fileFor(block.chainid, p.testLabel));
    }

    /// EIP-7702's delegation designator: exactly 23 bytes, 0xef0100 followed by the delegate's address.
    function isDelegatedAccount(bytes memory code) public pure returns (bool) {
        return code.length == 23 && code[0] == 0xef && code[1] == 0x01 && code[2] == 0x00;
    }

    // a one-leaf tree's root is its leaf: the demo collection has exactly one member, and says so
    uint256 constant STAGING_MEMBERS = 1024;

    /// The root of the staging fixture collection, built as OpenZeppelin's MerkleProof reads it (sorted pairs). keeper/staging/members.ts
    /// builds the same tree and the proofs; test/DeployChecks.t.sol pins the root so the two cannot drift apart.
    function stagingRoot() public pure returns (bytes32) {
        bytes32[] memory level = new bytes32[](STAGING_MEMBERS);
        for (uint256 i = 0; i < STAGING_MEMBERS; i++) level[i] = keccak256(bytes.concat(keccak256(bytes(string.concat("0000000000000000000000000000000000000000000000000000000000000000i", vm.toString(i))))));   // ListingRegistry.leafOf
        for (uint256 n = STAGING_MEMBERS; n > 1; n /= 2) {
            for (uint256 i = 0; i < n / 2; i++) {
                (bytes32 x, bytes32 y) = level[2 * i] < level[2 * i + 1] ? (level[2 * i], level[2 * i + 1]) : (level[2 * i + 1], level[2 * i]);
                level[i] = keccak256(bytes.concat(x, y));
            }
        }
        return level[0];
    }

    string constant DEMO_INSCRIPTION = "0000000000000000000000000000000000000000000000000000000000000000i0";
    string constant DEMO_BTC_VAULT = "bc1p-local-demo-not-a-real-address";
}
