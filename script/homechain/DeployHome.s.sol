// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Script} from "forge-std/Script.sol";
import {DrandEvmnet} from "../../src/rand/DrandEvmnet.sol";
import {HomeGuard} from "../../src/homechain/HomeGuard.sol";
import {HomeWrapper} from "../../src/homechain/HomeWrapper.sol";
import {HomeVaultBase} from "../../src/homechain/HomeVaultBase.sol";
import {HomeVaultYoyo} from "../../src/homechain/HomeVaultYoyo.sol";
import {IRandomnessBeacon} from "../../src/interfaces/IRandomnessBeacon.sol";
import {IArbInbox, IArbOutbox, IHomeGuard} from "../../src/homechain/IHomeChain.sol";

/// @notice The Ethereum-side deployment of docs/v6/home-chain-vault.md, as a dry run on a local fork ONLY. Never broadcast by this
/// branch: no deploy, no public-chain transaction. The addresses are the ones verified read-only in §2 of the design; the 4663
/// side (FeeEscrow, HomeReceiver) must exist first, because the vault names both at construction and the escrow names the vault
/// (both addresses are computable ahead from their deployers' nonces).
///
///   forge script script/homechain/DeployHome.s.sol --rpc-url http://127.0.0.1:8641      (a local anvil fork; no --broadcast)
contract DeployHome is Script {
    address constant INBOX = 0x1A07cc4BD17E0118BdB54D70990D2158AbAD7a2D;        // Robinhood Chain's Inbox on Ethereum
    address constant OUTBOX = 0xf0ce991ea4A0d2400A4AB49b20ae333f6Dce3DE9;       // its Outbox
    address constant L1_GATEWAY_ROUTER = 0x6a2E3a1e16FC29f27Ce61429746D558d656975bB;
    address constant SEAPORT = 0x0000000000000068F116a894984e2DB1123eB395;
    address constant PUNKS = 0xb47e3cd837dDF8e4c57F05d70Ab865de6e193BBB;
    address constant BAYC = 0xBC4CA0EdA7647A8aB7C2061c2E118A18a936f13D;

    function run() external {
        require(block.chainid == 1, "the home chain of these collections is Ethereum");
        address timelock = vm.envAddress("HOME_TIMELOCK");        // an OZ TimelockController, delay >= 7 days
        address keeper = vm.envAddress("HOME_KEEPER");
        address escrow = vm.envAddress("HOME_ESCROW_4663");        // FeeEscrow on 4663
        address receiver = vm.envAddress("HOME_RECEIVER_4663");    // HomeReceiver on 4663
        vm.startBroadcast();
        DrandEvmnet drand = new DrandEvmnet();
        HomeGuard guard = new HomeGuard(timelock, 7 days, 12);
        HomeVaultBase.Config memory c;
        c.collection = PUNKS;
        c.isPunks = true;
        c.beacon = IRandomnessBeacon(address(drand));
        c.guard = IHomeGuard(address(guard));
        c.inbox = IArbInbox(INBOX);
        c.outbox = IArbOutbox(OUTBOX);
        c.escrow = escrow;
        c.receiver = receiver;
        c.owner = timelock;
        c.keeper = keeper;
        c.reward = 0.01 ether;
        c.buffer = 0.05 ether;
        c.parentBlockSeconds = 12;
        HomeVaultYoyo punkYoyo = new HomeVaultYoyo(c, 12_000, 30 days);
        vm.stopBroadcast();
        // registration with the guard is the timelock's act, proposed through it after the deploy: guard.register(PUNKS, punkYoyo)
        punkYoyo;
    }
}
