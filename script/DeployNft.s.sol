// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Script, console2} from "forge-std/Script.sol";
import {Strings} from "@openzeppelin/contracts/utils/Strings.sol";
import {IPonsV2LaunchFactory} from "../src/interfaces/IPons.sol";
import {IRandomnessBeacon} from "../src/interfaces/IRandomnessBeacon.sol";
import {ListingRegistry} from "../src/ListingRegistry.sol";
import {Distributor} from "../src/Distributor.sol";
import {NftRegistry} from "../src/nft/NftRegistry.sol";
import {NftLauncher} from "../src/nft/NftLauncher.sol";
import {NftFeeRouter} from "../src/nft/NftFeeRouter.sol";
import {Policy} from "../src/nft/INft.sol";
import {DeployChecks} from "./DeployChecks.sol";

/// @title DeployNft
/// @notice docs/v4/nft.md. Deploys the NFT side against the EXISTING ListingRegistry (whose owner, keeper and treasury it borrows)
/// and the existing drand beacon: NftRegistry (allow-listing Seaport 1.6 where it has code) and NftLauncher (which deploys the
/// router implementation it clones). The deployer owns nothing. On a public chain the two owner calls — `wire(launcher)` on the
/// NFT registry and `wireNft(nftRegistry)` on the ListingRegistry — are PRINTED for the multisig; on our own chains the unlocked
/// owner makes both and lists two demo listings (one per case: a collection on this chain, if StonkBrokers is here, and one on
/// Ethereum bought by ETCH_NFT_BUYER). Writes deployments/nft-<chainid>.json.
///
///   ETCH_NFT_CONFIRM=deploy-etch-nft:<chainid>:<deployer, lowercase> ETCH_REGISTRY=0x… script/deploy-nft.sh [<rpc>] <deployer> [--ledger|--trezor]
/// Nothing here reads a private key.
contract DeployNft is Script {
    IPonsV2LaunchFactory constant PONS_FACTORY = IPonsV2LaunchFactory(0x7eD598BcEf8bd9Edd8C97A195C6d13f40801EC7e);
    address constant SEAPORT_16 = 0x0000000000000068F116a894984e2DB1123eB395;
    address constant STONKBROKERS = 0x539CdD042c2f3d93EbC5BE7DfFf0c79F3B4fAbF0;   // a 4663 collection, for the demo listing
    address constant DEMO_ETH_COLLECTION = 0xBC4CA0EdA7647A8aB7C2061c2E118A18a936f13D;  // named on chain 1 only; nothing here reads it

    error ConfirmMismatch(string expected);
    error RegistryIsNotAContract();
    error NotWired();
    error PonsMissing();

    function run() external {
        address deployer = msg.sender;
        string memory expected = string.concat("deploy-etch-nft:", Strings.toString(block.chainid), ":", Strings.toHexString(deployer));
        if (block.chainid != DeployChecks.ROBINHOOD && !DeployChecks.isOurs(block.chainid)) revert DeployChecks.UnknownChain(block.chainid);
        if (keccak256(bytes(vm.envOr("ETCH_NFT_CONFIRM", string("")))) != keccak256(bytes(expected))) revert ConfirmMismatch(expected);
        ListingRegistry lr = ListingRegistry(vm.envAddress("ETCH_REGISTRY"));
        if (address(lr).code.length == 0) revert RegistryIsNotAContract();
        if (address(PONS_FACTORY).code.length == 0) revert PonsMissing();
        address distributor = lr.distributor();
        if (distributor == address(0)) revert NotWired();         // the beacon is read from the Distributor the registry is wired to
        IRandomnessBeacon beacon = Distributor(distributor).beacon();
        address seaport = SEAPORT_16.code.length != 0 ? SEAPORT_16 : address(0);
        if (seaport == address(0)) console2.log("note: Seaport 1.6 has no code on this chain; no sweep target is allow-listed");
        string memory file = string.concat("deployments/nft-", vm.toString(block.chainid), ".json");
        if (!DeployChecks.isOurs(block.chainid)) require(!vm.exists(file), "an NFT deployment file is never overwritten on a real chain");

        vm.startBroadcast(deployer);
        NftRegistry nreg = new NftRegistry(lr, beacon, seaport);
        NftLauncher launcher = new NftLauncher(PONS_FACTORY, nreg);
        vm.stopBroadcast();

        // read back before anything is written down
        NftFeeRouter impl = NftFeeRouter(payable(launcher.routerImplementation()));
        require(address(nreg.listingRegistry()) == address(lr) && address(nreg.beacon()) == address(beacon), "read-back: registry");
        require(seaport == address(0) || nreg.isSweepTarget(seaport), "read-back: seaport");
        require(address(launcher.registry()) == address(nreg) && address(launcher.factory()) == address(PONS_FACTORY)
            && address(launcher.escrow()) == PONS_FACTORY.feeEscrow(), "read-back: launcher");
        require(impl.launcher() == address(launcher) && impl.coin() == address(1), "read-back: router implementation");

        bool local = DeployChecks.isOurs(block.chainid);
        if (local) {
            address owner = lr.owner();
            vm.startBroadcast(owner);
            nreg.wire(address(launcher));
            lr.wireNft(address(nreg));                    // docs/v4/nft.md §2.1: remote buyers are reserved in the registry
            if (STONKBROKERS.code.length != 0) {
                nreg.listNft(uint64(block.chainid), STONKBROKERS, Policy.Raffle, "StonkBrokers (demo)", payable(0), 0);
            }
            address payable buyer = payable(vm.envOr("ETCH_NFT_BUYER", address(0)));
            if (buyer != address(0)) nreg.listNft(1, DEMO_ETH_COLLECTION, Policy.Burn, "Ethereum floor, burned (demo)", buyer, 1 ether);
            vm.stopBroadcast();
        } else {
            console2.log("The deployer owns nothing. The owner (multisig) must now make these two calls:");
            console2.log("  on the NFT registry:", address(nreg));
            console2.log("  wire(nftLauncher)");
            console2.logBytes(abi.encodeCall(NftRegistry.wire, (address(launcher))));
            console2.log("  on the ListingRegistry:", address(lr));
            console2.log("  wireNft(nftRegistry)   (docs/v4/nft.md 2.1: without it no remote listing can be made)");
            console2.logBytes(abi.encodeCall(ListingRegistry.wireNft, (address(nreg))));
        }

        string memory k = "nft";
        vm.serializeUint(k, "chainId", block.chainid);
        vm.serializeAddress(k, "nftRegistry", address(nreg));
        vm.serializeAddress(k, "nftLauncher", address(launcher));
        vm.serializeAddress(k, "nftRouterImplementation", address(impl));
        vm.serializeAddress(k, "listingRegistry", address(lr));
        vm.serializeAddress(k, "beacon", address(beacon));
        vm.serializeAddress(k, "seaport", seaport);
        vm.serializeAddress(k, "ponsFactory", address(PONS_FACTORY));
        string memory json = vm.serializeBool(k, "wired", local);
        vm.writeJson(json, file);
        console2.log("nftRegistry", address(nreg));
        console2.log("nftLauncher", address(launcher), "->", file);
    }
}
