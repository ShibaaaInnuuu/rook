// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Clones} from "@openzeppelin/contracts/proxy/Clones.sol";
import {Base} from "./utils/Base.sol";
import {FeeRouter} from "../src/FeeRouter.sol";
import {IPonsV2FeeEscrow, IPonsV2BondingCurve, IPonsV2MemeHook} from "../src/interfaces/IPons.sol";
import {MockEscrow, MockCurve, RefusesEth} from "./utils/Mocks.sol";
import {ERC20} from "@openzeppelin/contracts/token/ERC20/ERC20.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {Clock} from "./utils/Clock.sol";

contract FeeRouterTest is Base {
    MockEscrow escrow;
    MockCurve curve;
    FeeRouter r;
    address coin = makeAddr("coin");

    function setUp() public override {
        super.setUp();
        escrow = new MockEscrow();
        curve = new MockCurve(escrow);
        r = FeeRouter(payable(Clones.clone(address(new FeeRouter()))));
        r.initialize(coin, registry, ordVault, IPonsV2FeeEscrow(address(escrow)), IPonsV2BondingCurve(address(curve)), IPonsV2MemeHook(address(0xB00C)));
        curve.setRecipient(address(r));
        launcher.set(coin, address(r), creatorA, 0);
    }

    function test_harvestSweepsTheCurve_claims_andSplits80_20() public {
        vm.deal(address(curve), 1 ether);                       // fees waiting inside the curve
        escrow.credit{value: 0.5 ether}(address(r));            // and some already in escrow
        vm.prank(stranger);                                     // anyone
        (uint256 toVault, uint256 toTreasury) = r.harvest();
        assertEq(toVault, 1.2 ether);
        assertEq(toTreasury, 0.3 ether);
        assertEq(address(ordVault).balance, 1.2 ether);
        assertEq(treasury.balance, 0.3 ether);
        assertEq(ordVault.contributedBy(coin), 1.2 ether);
        assertEq(address(r).balance, 0);
    }

    function testFuzz_theSplitIsExact_andNothingIsLeftBehind(uint96 amount) public {
        vm.assume(amount > 0);
        vm.deal(address(this), amount);
        escrow.credit{value: amount}(address(r));
        (uint256 toVault, uint256 toTreasury) = r.harvest();
        assertEq(toVault, (uint256(amount) * 8000) / 10000);
        assertEq(toVault + toTreasury, amount);
        assertEq(address(r).balance, 0);
        if (toVault == 0) return;
        assertEq(address(ordVault).balance, toVault);
    }

    function test_ethSentStraightToTheRouterIsSplitTheSameWay() public {
        vm.deal(stranger, 1 ether);
        vm.prank(stranger);
        (bool ok,) = address(r).call{value: 1 ether}("");
        assertTrue(ok);
        (uint256 toVault, uint256 toTreasury) = r.harvest();
        assertEq(toVault, 0.8 ether);
        assertEq(toTreasury, 0.2 ether);
    }

    function test_aTreasuryThatRefusesEthNeverBlocksTheVault_andIsPaidLater() public {
        RefusesEth bad = new RefusesEth();
        vm.startPrank(owner);
        registry.proposeTreasury(address(bad));
        Clock.skip(7 days);
        registry.acceptTreasury();
        vm.stopPrank();

        escrow.credit{value: 1 ether}(address(r));
        (uint256 toVault, uint256 toTreasury) = r.harvest();
        assertEq(toVault, 0.8 ether);
        assertEq(toTreasury, 0);
        assertEq(r.treasuryOwed(), 0.2 ether);
        assertEq(address(ordVault).balance, 0.8 ether);

        // the owed fifth is never re-split: a second harvest gives the vault 80% of the NEW money only
        escrow.credit{value: 1 ether}(address(r));
        (toVault,) = r.harvest();
        assertEq(toVault, 0.8 ether);
        assertEq(r.treasuryOwed(), 0.4 ether);

        bad.setOpen(true);
        (, toTreasury) = r.harvest();
        assertEq(toTreasury, 0.4 ether);
        assertEq(r.treasuryOwed(), 0);
        assertEq(address(bad).balance, 0.4 ether);
    }

    function test_afterGraduationTheCurveRefusesTheSweep_andHarvestStillWorks() public {
        curve.setGraduated(true);
        escrow.credit{value: 1 ether}(address(r));
        (uint256 toVault,) = r.harvest();
        assertEq(toVault, 0.8 ether);
    }

    function test_initialisedOnce_andThereIsNobodyToAsk() public {
        vm.expectRevert(FeeRouter.AlreadyInitialised.selector);
        r.initialize(address(1), registry, ordVault, IPonsV2FeeEscrow(address(escrow)), IPonsV2BondingCurve(address(curve)), IPonsV2MemeHook(address(0)));
        // no owner, no withdraw, no way to move the role: none of these selectors exists
        bytes4[6] memory sels = [bytes4(keccak256("owner()")), bytes4(keccak256("transferOwnership(address)")), bytes4(keccak256("withdraw(uint256)")),
            bytes4(keccak256("withdraw()")), bytes4(keccak256("transferCreatorFeeRecipient(address,address)")), bytes4(keccak256("upgradeTo(address)"))];
        for (uint256 i = 0; i < sels.length; ++i) {
            (bool ok,) = address(r).call(abi.encodeWithSelector(sels[i], address(this), address(this)));
            assertFalse(ok);
        }
        assertEq(r.VAULT_BPS(), 8000);
    }

    function test_nothingToHarvestIsNotAnError() public {
        (uint256 a, uint256 b) = r.harvest();
        assertEq(a + b, 0);
    }

    // ---- docs/phase1.md F1e: tokens that reach the router by Pons's owner-only rescue ----

    function test_forwardToken_movesTheWholeBalanceToTheTreasury_andAnyoneMayCallIt() public {
        StrayToken t = new StrayToken();
        t.mint(address(r), 1_000e18);
        vm.prank(makeAddr("anyone"));
        uint256 moved = r.forwardToken(IERC20(address(t)));
        assertEq(moved, 1_000e18);
        assertEq(t.balanceOf(registry.treasury()), 1_000e18);
        assertEq(t.balanceOf(address(r)), 0);
        assertEq(r.forwardToken(IERC20(address(t))), 0, "nothing left is not an error");
    }

    function test_whatTheTreasuryReturnsIsSplitLikeAnyOtherEth() public {
        // the treasury sold the tokens and sends the ETH back: by a plain transfer, or through the escrow — both split 80/20
        vm.deal(address(this), 2 ether);
        (bool ok,) = address(r).call{value: 1 ether}("");
        assertTrue(ok);
        escrow.credit{value: 1 ether}(address(r));
        (uint256 toVault, uint256 toTreasury) = r.harvest();
        assertEq(toVault, 1.6 ether);
        assertEq(toTreasury, 0.4 ether);
    }
}

contract StrayToken is ERC20 {
    constructor() ERC20("Stray", "STRAY") {}
    function mint(address to, uint256 a) external { _mint(to, a); }
}
