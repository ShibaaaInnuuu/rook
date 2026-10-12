// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {Clones} from "@openzeppelin/contracts/proxy/Clones.sol";
import {Base} from "./utils/Base.sol";
import {FeeRouter} from "../src/FeeRouter.sol";
import {IPonsV2FeeEscrow, IPonsV2BondingCurve, IPonsV2MemeHook} from "../src/interfaces/IPons.sol";
import {MockEscrow, MockCurve} from "./utils/Mocks.sol";

contract RouterHandler is Test {
    FeeRouter r; MockEscrow escrow; MockCurve curve;
    uint256 public ghostIn; uint256 public ghostVault; uint256 public ghostTreasury;
    constructor(FeeRouter r_, MockEscrow e, MockCurve c) { r = r_; escrow = e; curve = c; }
    function feesInEscrow(uint88 a) external { a = uint88(bound(a, 1, 100 ether)); vm.deal(address(this), a); escrow.credit{value: a}(address(r)); ghostIn += a; }
    function feesInTheCurve(uint88 a) external { a = uint88(bound(a, 1, 100 ether)); vm.deal(address(this), a); (bool ok,) = address(curve).call{value: a}(""); require(ok); ghostIn += a; }
    function sentStraight(uint88 a) external { a = uint88(bound(a, 1, 100 ether)); vm.deal(address(this), a); (bool ok,) = address(r).call{value: a}(""); require(ok); ghostIn += a; }
    function harvest() external { (uint256 v, uint256 t) = r.harvest(); ghostVault += v; ghostTreasury += t; }
}

contract FeeRouterInvariantTest is Base {
    RouterHandler handler; FeeRouter r; MockEscrow escrow; MockCurve curve;
    address coin = makeAddr("coin");

    function setUp() public override {
        super.setUp();
        escrow = new MockEscrow();
        curve = new MockCurve(escrow);
        r = FeeRouter(payable(Clones.clone(address(new FeeRouter()))));
        r.initialize(coin, registry, ordVault, IPonsV2FeeEscrow(address(escrow)), IPonsV2BondingCurve(address(curve)), IPonsV2MemeHook(address(0)));
        curve.setRecipient(address(r));
        launcher.set(coin, address(r), creatorA, 0);
        handler = new RouterHandler(r, escrow, curve);
        targetContract(address(handler));
    }

    /// Whatever arrives by whatever door, the vault has had 80% of every harvest (each rounded down once) and the treasury the rest.
    function invariant_eightyTwenty_andNothingElseGoesAnywhere() public view {
        uint256 out = handler.ghostVault() + handler.ghostTreasury();
        assertEq(address(ordVault).balance, handler.ghostVault());
        assertEq(treasury.balance, handler.ghostTreasury());
        assertEq(out + address(r).balance + address(curve).balance + address(escrow).balance, handler.ghostIn());
        // 80% exactly, less at most one wei of rounding a harvest
        assertLe(handler.ghostVault() * 10000, out * 8000);
        assertGe(handler.ghostVault() * 10000 + 10000 * 300, out * 8000);
    }
}
