// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {PoolBase} from "../utils/PoolBase.sol";
import {SweepVault} from "../../src/SweepVault.sol";
import {FrogCurve} from "../../src/curve/FrogCurve.sol";
import {CurveFactory} from "../../src/curve/CurveFactory.sol";
import {CurveToken} from "../../src/curve/CurveToken.sol";
import {ItemQueue} from "../../src/curve/ItemQueue.sol";
import {MintDesk} from "../../src/curve/MintDesk.sol";
import {CollectionDirectory} from "../../src/curve/CollectionDirectory.sol";
import {LiquidityLock} from "../../src/pools/LiquidityLock.sol";
import {EtchRouter} from "../../src/pools/EtchRouter.sol";
import {FloorModule} from "../../src/floor/FloorModule.sol";
import {NftWrapper} from "../../src/nft/NftWrapper.sol";
import {IWrappedCollection} from "../../src/interfaces/IWrappedCollection.sol";

/// The curve, the queue, the lock and the router are generic over the wrapped collection: the same graduation path runs over
/// wFROG (Bitcoin inscriptions, `Wrapper`) and wAPE (an ERC-721 on Ethereum, `NftWrapper`, at a 5 ETH floor). Each collection
/// has its own queue, lock, shared pool and bootstrap state (docs/v3/pools.md §5).
contract CollectionsTest is PoolBase {
    struct Stack {
        IWrappedCollection w;
        CurveFactory factory;
        ItemQueue queue;
        LiquidityLock lock;
        SweepVault raffle;
        EtchRouter router;
    }

    NftWrapper apes;
    Stack ape;
    uint256 nextApe = 1;
    uint256 constant APE_CAP = 5.1 ether;

    function setUp() public override {
        super.setUp();
        apes = new NftWrapper(beacon, 1, makeAddr("BAYC on Ethereum"), custodian, "Wrapped Bored Ape", "wAPE");
        apes.initMintLimit(1_000);
        ape = _stack(apes, APE_CAP);
    }

    function _stack(IWrappedCollection w, uint256 cap) internal returns (Stack memory s) {
        vm.startPrank(owner);
        uint256 rl = registry.listInscriptions("Apes raffle", "apes-raffle", bytes32(uint256(1)), "bc1p-unused");
        uint256 bl = registry.listInscriptions("Apes burn", "apes-burn", bytes32(uint256(1)), "bc1p-unused");
        vm.stopPrank();
        s.w = w;
        s.lock = new LiquidityLock(PM, w, _layout(), PBS);
        s.queue = new ItemQueue(w, payee, 500, 0.001 ether, 1_000, PBS);
        s.factory = new CurveFactory(w, s.lock, s.queue, platform, _template(), _targets(), PBS);
        MintDesk d = new MintDesk(w, payee, 6 hours, PBS);
        s.raffle = new SweepVault(registry, rl, w, SweepVault.Policy.RAFFLE, address(0), s.factory, d, PBS);
        SweepVault b = new SweepVault(registry, bl, w, SweepVault.Policy.BURN, address(0), s.factory, d, PBS);
        _ceiling(s.raffle);
        _ceiling(b);
        s.factory.wire(s.raffle, b);
        s.queue.wire(s.factory);
        s.lock.wire(s.factory);
        s.router = new EtchRouter(PM, s.lock, FloorModule(address(0)));
        vm.prank(keeper);
        s.raffle.postCap(cap);
    }

    function _frogs() internal view returns (Stack memory s) {
        s = Stack(wrapper, factory, queue, lock, curveRaffle, etchRouter);
    }

    function _mintItem(Stack memory s) internal {
        if (address(s.w) == address(apes)) {
            vm.prank(custodian);
            apes.mint(nextApe, keccak256(abi.encode("ape purchase", nextApe)), address(s.queue));
            nextApe++;
        } else {
            _deposit(address(s.queue));
        }
    }

    function _launchFill(Stack memory s) internal returns (FrogCurve c, CurveToken tok) {
        vm.prank(creator);
        (address coin, address curve) = s.factory.launch("Coin", "COIN", SweepVault.Policy.RAFFLE, false);
        c = FrogCurve(payable(curve));
        tok = CurveToken(coin);
        _fillCurve(c);
    }

    function _record(Stack memory s, uint256 cost) internal {
        _mintItem(s);
        if (!s.raffle.capFresh()) { vm.prank(keeper); s.raffle.postCap(s.queue.itemCap() == 0 ? cost : s.queue.itemCap()); }
        vm.prank(custodian);
        s.queue.recordItem(cost);
    }

    /// The same path for any collection: two coins close; items are bought FIFO until the first is filled; it graduates into
    /// its collection's pools; a trader buys it through the router.
    function _thePath(Stack memory s, uint256 itemCost) internal returns (FrogCurve c1, FrogCurve c2) {
        CurveToken t1;
        (c1, t1) = _launchFill(s);
        (c2,) = _launchFill(s);
        while (!s.queue.isFilled(address(t1))) _record(s, itemCost);
        if (!s.raffle.capFresh()) { vm.prank(keeper); s.raffle.postCap(itemCost); }
        c1.graduate();
        assertEq(uint8(c1.state()), uint8(FrogCurve.State.Graduated));
        (,, uint128 liq) = s.lock.coins(address(t1));
        assertGt(liq, 0, "the coin's pool is open");
        assertGt(s.lock.positionCount(), 0, "the collection's shared pool is seeded");
        assertEq(c1.unitsBought() + s.queue.entry(address(c2.token())).units, s.queue.itemsRecorded() * 1e18,
            "every unit bought is assigned, FIFO");
        address t = makeAddr("trader");
        vm.deal(t, 1 ether);
        vm.prank(t);
        uint256 got = s.router.buy{value: 0.05 ether}(address(t1), 1, t, block.timestamp);
        assertGt(got, 0);
    }

    function test_theSameGraduationPathOverFrogsAndApes() public {
        (FrogCurve f1,) = _thePath(_frogs(), 0.64 ether);
        (FrogCurve a1, FrogCurve a2) = _thePath(ape, 5.0 ether);
        // frogs: 6.742 ETH of budget at 0.64 → 10 frogs, 0.34 tail; apes: 5.0 then 1.742 of the second ape → 1.348 wAPE
        assertEq(f1.unitsBought(), 10e18);
        assertEq(a1.unitsBought(), 1e18 + uint256(1e18) * (a1.itemBudget() - 5 ether) / 5 ether);
        ItemQueue.Entry memory e2 = ape.queue.entry(address(a2.token()));
        assertEq(e2.spent, 5 ether - (a1.itemBudget() - 5 ether), "the second coin paid the rest of the second ape");
        assertEq(uint8(e2.status), uint8(ItemQueue.Status.Queued), "and waits, its remainder still its own");
    }

    function test_aSecondCollectionStartsEarlyWhileTheFirstIsNormal() public {
        _graduate(SweepVault.Policy.RAFFLE);
        _graduate(SweepVault.Policy.BURN);
        assertGe(lock.sharedEthIn(), 2 ether, "two early frog coins deepened the frog pool past the early stage");
        (uint256 frogRaise, bool frogEarly,) = factory.raiseFor(false);
        assertEq(frogRaise, 5 ether);
        assertFalse(frogEarly);
        (uint256 apeRaise, bool apeEarly, bool apeOpening) = ape.factory.raiseFor(false);
        assertEq(apeRaise, 8 ether, "the ape collection's own pool is empty: early, and opening");
        assertTrue(apeEarly && apeOpening);
        assertEq(ape.lock.sharedEthIn(), 0, "seeder shares and depth are per collection");
    }

    function test_theDirectoryListsEveryCollectionOnce() public {
        CollectionDirectory dir = new CollectionDirectory(owner);
        vm.startPrank(owner);
        dir.add(factory);
        dir.add(ape.factory);
        vm.expectRevert(CollectionDirectory.AlreadyListed.selector);
        dir.add(ape.factory);
        vm.stopPrank();
        vm.prank(stranger);
        vm.expectRevert(CollectionDirectory.NotAdmin.selector);
        dir.add(factory);
        address[] memory qs = dir.queues();
        assertEq(qs.length, 2);
        assertEq(qs[0], address(queue));
        assertEq(qs[1], address(ape.queue));
        CollectionDirectory.Collection memory c = dir.at(1);
        assertEq(c.wrapper, address(apes));
        assertEq(c.lock, address(ape.lock));
    }

    function test_aDeskUnitNobodyRequestedGoesBackToTheCustodian() public {
        _deposit(address(desk));
        vm.prank(stranger);
        vm.expectRevert(MintDesk.NotCustodian.selector);
        desk.reclaimStray();
        vm.prank(custodian);
        assertEq(desk.reclaimStray(), 1e18);
        assertEq(IERC20(address(wrapper)).balanceOf(custodian), 1e18);
    }
}
