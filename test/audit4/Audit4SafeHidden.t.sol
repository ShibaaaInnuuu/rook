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
    function execTransactionFromModule(address to, uint256 value, bytes memory data, uint8 operation) external returns (bool);
}

/// Creation code that plants storage, then returns the CANONICAL SafeProxy runtime: the proxy's code hash is canonical, but its
/// storage holds a module that no linked list names (review4's PoC, unchanged).
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

/// R4-check L-S1, now a regression (docs/audit/2026-09-26/review4/fixes-golive-lows.md §3). Review4's fork PoC showed a Safe whose
/// storage was written by its creation code passing every read `script/golive-check.sh` made of the owner. golive-check now also
/// requires the Safe's creation by a canonical SafeProxyFactory with `setup()` in the same transaction: the SafeSetup's transaction
/// must hold exactly one `ProxyCreation(proxy, singleton)` naming the Safe, from a canonical factory address whose code on this chain
/// is the canonical factory's, on a canonical singleton. `_creationRead` is that rule, applied here to the logs of the creating
/// transaction (in a forge test the test body is that transaction); the script's own implementation is exercised against these
/// shapes in scripts/audit2_golive_check.test.sh.
///
/// Opt-in, a read-only fork of the public 4663 RPC (canonical factory, SafeL2 1.4.1 and handler, as deployed there):
///   AUDIT4_FORK=1 forge test --match-contract Audit4SafeHidden
contract Audit4SafeHidden is Test {
    address constant FACTORY_141 = 0x4e1DCf7AD4e460CfD30791CCC4F9c8a4f820ec67;
    address constant FACTORY_130 = 0xa6B71E26C5e0845f74c812102Ca7114b6a896AB2;
    address constant FACTORY_130_155 = 0xC22834581EbC8527d974F8a1c97E1bEA4EF910BC;
    bytes32 constant FACTORY_141_HASH = 0x50c3cdc4074750a7a974204a716c999edd37482f907608d960b2b025ee0b3317;
    bytes32 constant FACTORY_130_HASH = 0x337d7f54be11b6ed55fef7b667ea5488db53db8320a05d1146aa4bd169a39a9b;
    address constant SAFE_L2_141 = 0x29fcB43b46531BcA003ddC8FCB67FFE91900C762;
    address constant SAFE_141 = 0x41675C099F32341bf84BFc5382aF534df5C7461a;
    address constant HANDLER_141 = 0xfd0732Dc9E303f09fCEf3a7388Ad10A83459Ec99;
    bytes32 constant GUARD_SLOT = 0x4a204f620c8c5ccdca3fd54d003badd85ba500436a431f0cbda4f558c93c34c8;
    bytes32 constant FALLBACK_SLOT = 0x6c9a6c4a39284e37ed1cf53d337577d14212a4870fb976a4366c693b939918d5;
    bytes32 constant SAFE_SETUP = keccak256("SafeSetup(address,address[],uint256,address,address)");
    bytes32 constant PROXY_CREATION = keccak256("ProxyCreation(address,address)");
    // golive-check's SAFE_PROXY_HASHES
    bytes32 constant PROXY_HASH_A = 0xb89c1b3bdf2cf8827818646bce9a8f6e372885f8c55e5c07acbd307cb133b000;
    bytes32 constant PROXY_HASH_B = 0xd7d408ebcd99b2b70be43e20253d6d92a8ea8fab29bd3be7f55b10032331fb4c;

    address owner1 = makeAddr("device1");
    address owner2 = makeAddr("device2");
    address thief = makeAddr("thief");

    function _fork() internal returns (bool) {
        if (!vm.envOr("AUDIT4_FORK", false)) return false;
        vm.createSelectFork("robinhood");
        assertEq(block.chainid, 4663);
        return true;
    }

    function _owners() internal view returns (address[] memory o) {
        o = new address[](2);
        o[0] = owner1;
        o[1] = owner2;
    }

    /// golive-check's R4-check L-S1 read over the logs of the SafeSetup's transaction: exactly one ProxyCreation naming `safe`, from
    /// a canonical factory address carrying the canonical factory code, on a canonical singleton.
    function _creationRead(address safe, Vm.Log[] memory txLogs) internal view returns (bool ok, string memory why) {
        uint256 n;
        address factory;
        address singleton;
        for (uint256 i; i < txLogs.length; i++) {
            Vm.Log memory l = txLogs[i];
            if (l.topics.length == 0 || l.topics[0] != PROXY_CREATION) continue;
            address proxy;
            address sing;
            if (l.topics.length >= 2) {
                proxy = address(uint160(uint256(l.topics[1])));
                sing = abi.decode(l.data, (address));
            } else {
                (proxy, sing) = abi.decode(l.data, (address, address));
            }
            if (proxy != safe) continue;
            n++;
            factory = l.emitter;
            singleton = sing;
        }
        if (n != 1) return (false, n == 0 ? "no ProxyCreation naming the Safe in its SafeSetup's transaction" : "more than one ProxyCreation");
        bytes32 h = keccak256(factory.code);
        if (factory == FACTORY_141 ? h != FACTORY_141_HASH : (factory == FACTORY_130 || factory == FACTORY_130_155) ? h != FACTORY_130_HASH : true) {
            return (false, "not a canonical SafeProxyFactory");
        }
        if (singleton != SAFE_L2_141 && singleton != SAFE_141) return (false, "not a canonical singleton");
        return (true, "");
    }

    function _setupCount(address safe, Vm.Log[] memory logs) internal pure returns (uint256 n, address initializer, uint256 threshold,
        address[] memory owners)
    {
        for (uint256 i; i < logs.length; i++) if (logs[i].emitter == safe && logs[i].topics[0] == SAFE_SETUP) {
            n++;
            (owners, threshold, initializer, ) = abi.decode(logs[i].data, (address[], uint256, address, address));
        }
    }

    /// Review4's PoC, re-run: every read golive-check made in round 4 still passes, and the hidden module is really there, but the
    /// creation read FAILs the Safe, even with a decoy ProxyCreation from the canonical factory in the same transaction.
    function test_audit4_aPlantedSafeFailsGoliveChecksCreationRead() public {
        if (!_fork()) return;
        address honest = IFactory(FACTORY_141).createProxyWithNonce(SAFE_L2_141, "", 1);
        bytes memory runtime = honest.code;
        bytes32 h = keccak256(runtime);
        assertTrue(h == PROXY_HASH_A || h == PROXY_HASH_B, "the canonical proxy runtime hash is in golive-check's list");

        vm.recordLogs();                                                 // the attacker's creating transaction
        IFactory(FACTORY_141).createProxyWithNonce(SAFE_L2_141, "", 2); // a decoy: a canonical ProxyCreation of ANOTHER proxy
        address safe = address(new Planter(runtime, SAFE_L2_141, thief));
        ISafe(safe).setup(_owners(), 2, address(0), "", HANDLER_141, address(0), 0, payable(address(0)));
        Vm.Log[] memory logs = vm.getRecordedLogs();

        // the round-4 reads, all still passing
        assertEq(keccak256(safe.code), h, "runtime: a canonical Safe proxy");
        assertEq(address(uint160(uint256(vm.load(safe, 0)))), SAFE_L2_141, "slot 0: a canonical singleton");
        (address[] memory mods,) = ISafe(safe).getModulesPaginated(address(1), 10);
        assertEq(mods.length, 0, "getModulesPaginated: no module");
        assertEq(vm.load(safe, GUARD_SLOT), bytes32(0), "no guard");
        assertEq(address(uint160(uint256(vm.load(safe, FALLBACK_SLOT)))), HANDLER_141, "fallback handler: the canonical 1.4.1 one");
        (uint256 n, address initializer, uint256 threshold, address[] memory setupOwners) = _setupCount(safe, logs);
        assertEq(n, 1, "exactly one SafeSetup event");
        assertEq(initializer, address(0), "SafeSetup: initializer zero");
        assertEq(threshold, ISafe(safe).getThreshold(), "SafeSetup threshold = the chain's");
        assertEq(keccak256(abi.encode(setupOwners)), keccak256(abi.encode(ISafe(safe).getOwners())), "SafeSetup owners = the chain's");
        assertTrue(ISafe(safe).isModuleEnabled(thief), "the planted module is really there");

        // R4-check L-S1: the creation read FAILs it
        (bool ok, string memory why) = _creationRead(safe, logs);
        assertFalse(ok, "golive-check FAILs a Safe not created by a canonical factory with its setup()");
        assertEq(why, "no ProxyCreation naming the Safe in its SafeSetup's transaction");
    }

    /// The control: a Safe made the way the ceremony makes it (the canonical 1.4.1 factory, setup() as the initializer, one
    /// transaction) passes the creation read, and no module hides in it: the factory's proxy creation code writes slot 0 only.
    function test_audit4_aFactoryMadeSafePassesTheCreationReadAndHidesNoModule() public {
        if (!_fork()) return;
        vm.recordLogs();
        bytes memory init = abi.encodeCall(ISafe.setup, (_owners(), 2, address(0), "", HANDLER_141, address(0), 0, payable(address(0))));
        address safe = IFactory(FACTORY_141).createProxyWithNonce(SAFE_L2_141, init, uint256(keccak256("ceremony")));
        Vm.Log[] memory logs = vm.getRecordedLogs();
        (bool ok, string memory why) = _creationRead(safe, logs);
        assertTrue(ok, why);
        (uint256 n, address initializer, uint256 threshold, address[] memory setupOwners) = _setupCount(safe, logs);
        assertEq(n, 1, "its SafeSetup is in the same transaction");
        assertEq(initializer, address(0));
        assertEq(threshold, 2);
        assertEq(keccak256(abi.encode(setupOwners)), keccak256(abi.encode(ISafe(safe).getOwners())));
        assertFalse(ISafe(safe).isModuleEnabled(thief), "no hidden module");
        vm.deal(safe, 1 ether);
        vm.prank(thief);
        vm.expectRevert();                                              // GS104: the thief is not a module
        ISafe(safe).execTransactionFromModule(thief, 1 ether, "", 0);
        assertEq(safe.balance, 1 ether);
    }
}
