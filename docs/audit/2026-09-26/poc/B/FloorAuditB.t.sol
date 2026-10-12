// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {FloorBase} from "../../floor/FloorBase.sol";
import {FloorModule} from "../../../src/floor/FloorModule.sol";
import {Wrapper} from "../../../src/Wrapper.sol";

/// Audit B (floor module, wrapping, custody bookkeeping): proof-of-concept tests. Frog defaults of FloorBase with the 25% reserve
/// share that DeployStack/StackChecks.defaultParams deploys (FloorBase itself runs reserveBps = 0).
contract FloorAuditB is FloorBase {
    address mallory = makeAddr("mallory");

    function _params() internal pure override returns (FloorModule.Params memory p) {
        p = super._params();
        p.reserveBps = 2_500;
        p.maxItemsPerDay = 3; // stage 0
    }

    // ------------------------------------------------------------------------------------------------------------------------
    // B-01: a 4-wei mint re-creates a 1-wei reserve tier; every burn then reverts (NotEnoughBacked above 1 wei, Slippage at 1 wei)
    // ------------------------------------------------------------------------------------------------------------------------
    function test_B01_dustMintFreezesEveryBurn() public {
        _mint(alice, 4 * module.mintPrice());                 // 4 units: 1 reserve unit + 3 convertible
        assertEq(module.reserveUnits(), 1e18);
        // Alice exits the reserve tier first (as the design requires)...
        _burn(alice, module.reserveUnits());
        assertEq(module.reserveUnits(), 0, "reserve tier drained");
        uint256 aliceLeft = wrapper.balanceOf(alice);
        assertGt(aliceLeft, 2.9e18, "3 convertible units left to exit");

        // ...Mallory spends 4 wei (plus gas) to re-open a 1-wei reserve tier
        vm.deal(mallory, 4);
        vm.prank(mallory);
        module.mint{value: 4}(mallory, 0);
        assertEq(module.reserveUnits(), 1, "a reserve tier of ONE WEI of units");

        (uint256 p, uint256 room) = module.burnCapacity();
        assertGt(p, 0.6 ether, "the price is fine");
        assertEq(room, 1, "but the module will take one wei of wX");
        assertEq(module.quoteBurn(1), 0, "worth zero ETH");

        vm.startPrank(alice);
        wrapper.approve(address(module), type(uint256).max);
        vm.expectRevert(FloorModule.NotEnoughBacked.selector);
        module.burn(1e18, 0, alice);
        vm.expectRevert(FloorModule.Slippage.selector);
        module.burn(1, 0, alice);
        vm.stopPrank();

        // it persists: nothing drains the 1-wei tier except a burn that pays >= 1 wei, which it cannot
        for (uint256 h = 0; h < 24; ++h) _calm();
        (, room) = module.burnCapacity();
        assertEq(room, 1, "a day later, still frozen");
        vm.prank(alice);
        vm.expectRevert(FloorModule.NotEnoughBacked.selector);
        module.burn(1e18, 0, alice);
        emit log_named_decimal_uint("Alice's ETH-backed wX with no module exit", aliceLeft, 18);
        emit log_named_decimal_uint("ETH backing them, locked until someone mints", module.backing(), 18);
        _books();
    }

    /// The same grief, re-armed every time a burn drains the reserve tier exactly: a bank run can be held off the module.
    function test_B01b_griefIsReArmableAfterEveryDrain() public {
        _mint(alice, 4 * module.mintPrice());                 // R = 1 unit
        _burn(alice, module.reserveUnits());                  // drains it
        vm.deal(mallory, 1 ether);
        vm.prank(mallory);
        module.mint{value: 4}(mallory, 0);
        assertEq(module.reserveUnits(), 1);
        // an honest minter arrives and unfreezes it for a moment...
        _calm();
        _mint(bob, module.mintPrice());
        uint256 r = module.reserveUnits();
        assertGt(r, 1);
        _burn(alice, r);                                      // ...the tier drains again
        assertEq(module.reserveUnits(), 0);
        vm.prank(mallory);
        module.mint{value: 4}(mallory, 0);                    // and is re-armed for 4 wei
        vm.prank(alice);
        wrapper.approve(address(module), type(uint256).max);
        vm.prank(alice);
        vm.expectRevert(FloorModule.NotEnoughBacked.selector);
        module.burn(1e18, 0, alice);
    }

    // ------------------------------------------------------------------------------------------------------------------------
    // B-02: a compromised (immutable) keeper walks the floor up and burners take the fee-/treasury-funded reserve surplus.
    // FloorModule.sol:38-39 and floor-module.md §6.3/§12 say a compromised keeper costs "nothing through burns".
    // ------------------------------------------------------------------------------------------------------------------------
    function test_B02_compromisedKeeperDrainsTheReserveSurplusThroughBurns() public {
        _mint(alice, 4 * module.mintPrice());                 // an honest minter: 1 reserve unit at the mint price
        // the collection's reserve slice of coin fees / the treasury's 90% reserves allocation (Phase 1: setShares(9000,0,0,1000))
        address curveLike = makeAddr("a curve paying its reserve slice");
        vm.deal(curveLike, 3 ether);
        vm.prank(curveLike);
        module.fundReserve{value: 3 ether}();
        uint256 s0 = module.reserve();
        uint256 basisPerReserveUnit = module.mintPrice();     // what that reserve unit was paid in at (calm, same floor)

        for (uint256 i = 0; i < 2; ++i) _deposit(mallory);   // Mallory's 2 wX, "bought in the pool" at ~F each

        // the owner pauses the listing: the module does not read the registry, so nothing stops
        vm.prank(owner);
        registry.setPaused(FROGS, true);

        // the keeper key is stolen; it posts a far-off target every hour, the accepted floor walks +5%/h
        for (uint256 h = 0; h < 15; ++h) { _skip(1 hours); _post(F * 100); }
        _calm();                                              // one flat re-post: the volatility spread has decayed
        assertGt(module.accepted(), F * 2, "accepted floor pushed past 2x in ~15 h");

        uint256 before = mallory.balance;
        _burn(mallory, module.reserveUnits());                // the reserve tier (1 unit) at the pushed floor
        uint256 got = mallory.balance - before;
        emit log_named_decimal_uint("Mallory paid (pool, ~F)", F, 18);
        emit log_named_decimal_uint("Mallory received for ONE unit", got, 18);
        emit log_named_decimal_uint("that unit's paid-in basis", basisPerReserveUnit, 18);
        emit log_named_decimal_uint("reserve before", s0, 18);
        emit log_named_decimal_uint("reserve after", module.reserve(), 18);
        assertGt(got, 2 * basisPerReserveUnit, "paid out > 2x what the unit paid in: the surplus is being drained");
        assertGt(got - F, 0.6 ether, "Mallory's profit per unit, taken from the protocol's reserve surplus");
        _books();
    }

    // ------------------------------------------------------------------------------------------------------------------------
    // B-03: the registry keeper (a hot key) can record a fake delivery for any resolved ticket: the redeemer's wX is gone, no
    // refund is possible, and the frog silently leaves the books while it is still in custody.
    // ------------------------------------------------------------------------------------------------------------------------
    function test_B03_keeperForgesADelivery_redeemerLosesTheTokenAndTheRefund() public {
        _deposit(alice);                                      // one frog in custody, Alice holds its wX
        vm.prank(alice);
        uint256 id = wrapper.redeem(TAPROOT);
        vm.warp(vm.getBlockTimestamp() + 2 minutes);
        _resolveTicket(id, keccak256("r"));
        assertEq(uint8(wrapper.ticket(id).stage), uint8(Wrapper.Stage.Resolved));

        vm.prank(keeper);                                     // not the custodian; nothing was sent on Bitcoin
        wrapper.recordDelivery(id, keccak256("made-up txid"));

        _skip(8 days);
        vm.expectRevert(Wrapper.WrongStage.selector);
        wrapper.refund(id);
        assertEq(wrapper.balanceOf(alice), 0, "Alice has neither the frog nor her token");
        assertEq(wrapper.held(), 0, "the frog left the books; it is still at the custody address, unaccounted");
    }

    // ------------------------------------------------------------------------------------------------------------------------
    // B-04: the keeper is immutable and the module has no owner: if the key is lost (or its sender stops), ETH-backed holders
    // can never exit at the module, and the backing + reserve are locked for ever.
    // ------------------------------------------------------------------------------------------------------------------------
    function test_B04_keeperLossLocksTheBackingForever() public {
        _mint(alice, 4 * module.mintPrice());
        uint256 locked = address(module).balance;
        _skip(365 days);
        vm.startPrank(alice);
        wrapper.approve(address(module), type(uint256).max);
        vm.expectRevert(FloorModule.Stale.selector);
        module.burn(1e18, 0, alice);
        vm.stopPrank();
        vm.prank(custodian);
        vm.expectRevert(FloorModule.CapStale.selector);
        module.itemArrived(F);
        assertEq(address(module).balance, locked, "a year on, every wei is still in the module with no path out");
        assertEq(module.revenue(), 0, "skim has nothing to take either");
    }

    // ------------------------------------------------------------------------------------------------------------------------
    // Fuzz: in a calm market with a fee-funded surplus in the reserve, a mint -> burn round trip never profits (honest keeper).
    // ------------------------------------------------------------------------------------------------------------------------
    function testFuzz_B_roundTripNeverProfitsWithSurplus(uint256 eth, uint256 surplus, uint256 prior) public {
        eth = bound(eth, 4, 1.5 ether);
        surplus = bound(surplus, 0, 10 ether);
        prior = bound(prior, 0, 2);
        if (prior != 0) _mint(bob, prior * module.mintPrice());
        if (surplus != 0) { vm.deal(address(this), surplus); module.fundReserve{value: surplus}(); }
        uint256 units = _mint(mallory, eth);
        (, uint256 room) = module.burnCapacity();
        uint256 out;
        uint256 bal = wrapper.balanceOf(mallory);
        while (bal != 0 && room != 0) {
            uint256 u = bal < room ? bal : room;
            if (module.quoteBurn(u) == 0) break;
            out += _burn(mallory, u);
            bal = wrapper.balanceOf(mallory);
            (, room) = module.burnCapacity();
        }
        assertLe(out, eth, "no round-trip profit");
        assertLe(units, eth * 1e18 / F + 1, "never more than a unit per floor");
        _books();
    }
}
