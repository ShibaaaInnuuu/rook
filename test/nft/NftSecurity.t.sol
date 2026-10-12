// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Clones} from "@openzeppelin/contracts/proxy/Clones.sol";
import {ReentrancyGuard} from "@openzeppelin/contracts/utils/ReentrancyGuard.sol";
import {NftBase} from "./NftBase.sol";
import {MockMarket, ReentrantSeller} from "./NftMocks.sol";
import {NftSweepVault} from "../../src/nft/NftSweepVault.sol";
import {SeaportGuard} from "../../src/nft/SeaportGuard.sol";
import {
    ItemType, ConsiderationItem, AdvancedOrder, CriteriaResolver, FulfillmentComponent, ISeaport16Sweep
} from "../../src/nft/Seaport16.sol";
import {NftVaultBase} from "../../src/nft/NftVaultBase.sol";
import {NftFeeRouter} from "../../src/nft/NftFeeRouter.sol";
import {INftRegistry, INftVault} from "../../src/nft/INft.sol";
import {IPonsV2FeeEscrow, IPonsV2BondingCurve, IPonsV2MemeHook} from "../../src/interfaces/IPons.sol";
import {MockEscrow, MockCurve} from "../utils/Mocks.sol";
import {Clock} from "../utils/Clock.sol";
import {Merkle} from "../utils/Merkle.sol";

/// docs/v4/nft.md §5: every way the brief named to get money or NFTs out of a sweep vault, or to buy the wrong thing with it.
contract NftSecurityTest is NftBase {
    address attacker = makeAddr("attacker");

    function _ready() internal {
        _fund(raffleVault, coinA, 1 ether);
        _cap(raffleVault, CAP);
    }

    // ---- drains ----------------------------------------------------------------------------------------------------------------

    function test_aTargetThatTakesTheEthAndDeliversNothingReverts() public {
        _ready();
        _ask(_ids(1), 0.05 ether);
        market.setMode(MockMarket.Mode.Swallow, address(0), 0);     // it passes the decoder, then misbehaves
        vm.prank(attacker);
        vm.expectRevert(abi.encodeWithSelector(NftSweepVault.NotEnoughBought.selector, 0, 1));
        raffleVault.sweep(address(market), _buyCall(_ids(1), address(raffleVault)), _ids(1), 1);
        assertEq(address(raffleVault).balance, 1 ether);
    }

    function test_aTargetThatRelaysTheEthToTheAttackerReverts() public {
        _ready();
        _ask(_ids(1), 0.05 ether);
        market.setMode(MockMarket.Mode.Relay, attacker, 0);
        vm.prank(attacker);
        vm.expectRevert(abi.encodeWithSelector(NftSweepVault.NotEnoughBought.selector, 0, 1));
        raffleVault.sweep(address(market), _buyCall(_ids(1), address(raffleVault)), _ids(1), 1);
        assertEq(attacker.balance, 0);
    }

    function test_aFakeSeaportIsRefused() public {
        _ready();
        MockMarket fake = new MockMarket(nft);
        vm.expectRevert(abi.encodeWithSelector(NftSweepVault.TargetNotAllowed.selector, address(fake)));
        raffleVault.sweep(address(fake), "", _ids(1), 1);
        // the collection itself as a target would let calldata approve the vault's NFTs away: refused the same way
        vm.expectRevert(abi.encodeWithSelector(NftSweepVault.TargetNotAllowed.selector, address(nft)));
        raffleVault.sweep(address(nft), abi.encodeWithSignature("setApprovalForAll(address,bool)", attacker, true), _ids(1), 1);
        // and so is the vault itself, or the registry
        vm.expectRevert(abi.encodeWithSelector(NftSweepVault.TargetNotAllowed.selector, address(raffleVault)));
        raffleVault.sweep(address(raffleVault), "", _ids(1), 1);
    }

    function test_noWayOutByAnyPlausibleName() public {
        _ready();
        uint256 a = _sweep(raffleVault, _ids(1), 0.05 ether);
        string[10] memory sigs = ["withdraw()", "withdraw(uint256)", "release(uint256,address)", "rescue(address,uint256)",
            "approve(address,uint256)", "setApprovalForAll(address,bool)", "transferNft(uint256,address)", "setPolicy(uint8)",
            "execute(address,bytes)", "multicall(bytes[])"];
        for (uint256 i = 0; i < sigs.length; ++i) {
            vm.prank(owner);
            (bool ok,) = address(raffleVault).call(abi.encodeWithSignature(sigs[i], attacker, 1));
            assertFalse(ok, sigs[i]);
        }
        assertEq(nft.ownerOf(1), address(raffleVault));
        assertFalse(nft.isApprovedForAll(address(raffleVault), address(market)));
        assertEq(nft.getApproved(1), address(0));
        a;
    }

    // ---- over the cap ------------------------------------------------------------------------------------------------------

    /// The balance check still stands on its own: a market that fills one of two orders and keeps the change is caught after the call.
    function test_spendOverCapTimesBoughtReverts() public {
        _ready();
        _ask(_ids(1, 2), 0.1 ether);
        AdvancedOrder[] memory orders = new AdvancedOrder[](2);
        orders[0] = _order(seller, 1, 0.1 ether);
        orders[1] = _order(seller, 2, 0.1 ether);
        bytes memory data = abi.encodeCall(ISeaport16Sweep.fulfillAvailableAdvancedOrders, (orders, new CriteriaResolver[](0),
            new FulfillmentComponent[][](0), new FulfillmentComponent[][](0), bytes32(0), address(raffleVault), 1));
        market.setMode(MockMarket.Mode.KeepChange, address(0), 0);
        vm.prank(attacker);
        vm.expectRevert(abi.encodeWithSelector(NftSweepVault.OverCap.selector, 0.2 ether, 0.1 ether));
        raffleVault.sweep(address(market), data, _ids(1, 2), 1);
    }

    function test_anAskAboveTheCapCannotBeFilled() public {
        _ready();
        _ask(_ids(1), 0.15 ether);
        vm.expectRevert(abi.encodeWithSelector(SeaportGuard.OrderOverCap.selector, 0, 0.15 ether, 0.1 ether));
        raffleVault.sweep(address(market), _buyCall(_ids(1), address(raffleVault)), _ids(1), 1);
    }

    /// docs/v4/nft.md §10.1, closed. This was `test_knownOpenProblem_aTipWithinTheCapIsPaid`: the sweeper appended a tip to itself
    /// within the cap and was paid. It fails against the old vault and passes now: the tip is refused before any ETH moves.
    function test_aTipWithinTheCapIsRefused() public {
        _ready();
        _ask(_ids(1), 0.05 ether);
        AdvancedOrder[] memory orders = new AdvancedOrder[](1);
        orders[0] = _withTip(_order(seller, 1, 0.05 ether), attacker, 0.05 ether);
        vm.prank(attacker);
        vm.expectRevert(abi.encodeWithSelector(SeaportGuard.Tip.selector, 0));
        raffleVault.sweep(address(market), _available(orders, address(raffleVault)), _ids(1), 1);
        assertEq(attacker.balance, 0);
        assertEq(address(raffleVault).balance, 1 ether);
    }

    /// Appends a consideration item past the signed count: what Seaport lets a fulfiller do.
    function _withTip(AdvancedOrder memory a, address to, uint256 amount) internal pure returns (AdvancedOrder memory) {
        ConsiderationItem[] memory c = new ConsiderationItem[](a.parameters.consideration.length + 1);
        for (uint256 i = 0; i < a.parameters.consideration.length; ++i) c[i] = a.parameters.consideration[i];
        c[c.length - 1] = ConsiderationItem(ItemType.NATIVE, address(0), 0, amount, amount, payable(to));
        a.parameters.consideration = c;
        return a;
    }

    // ---- what may arrive ---------------------------------------------------------------------------------------------------

    function test_anUnlistedIdArrivingReverts() public {
        _ready();
        _ask(_ids(1, 99), 0.05 ether);
        market.setMode(MockMarket.Mode.PlusExtra, address(0), 99);
        vm.expectRevert(abi.encodeWithSelector(NftSweepVault.UnlistedArrival.selector, 2, 1));
        raffleVault.sweep(address(market), _buyCall(_ids(1), address(raffleVault)), _ids(1), 1);
    }

    function test_nothingCanBeSafeTransferredInOutsideASweep() public {
        nft.mint(attacker, 5);
        vm.prank(attacker);
        vm.expectRevert(NftSweepVault.NotDuringSweep.selector);
        nft.safeTransferFrom(attacker, address(raffleVault), 5);
    }

    function test_anNftPushedInIsNoAcquisitionAndCannotBeTakenOut() public {
        nft.mint(attacker, 5);
        vm.prank(attacker);
        nft.transferFrom(attacker, address(raffleVault), 5);
        assertEq(raffleVault.acquisitionCount(), 0);
        // and a later sweep still balances: the stray is not a candidate and does not move
        _ready();
        _sweep(raffleVault, _ids(6), 0.05 ether);
        assertEq(raffleVault.acquisitionCount(), 1);
    }

    // ---- stale caps --------------------------------------------------------------------------------------------------------

    function test_noCapNoSweep() public {
        _fund(raffleVault, coinA, 1 ether);
        vm.expectRevert(NftVaultBase.CapExpired.selector);
        raffleVault.sweep(address(market), "", _ids(1), 1);
    }

    function test_aCapExpiresOnTheSequencersClock() public {
        _ready();
        Clock.skipTimeOnly(1 hours + 1);
        vm.expectRevert(NftVaultBase.CapExpired.selector);
        raffleVault.sweep(address(market), "", _ids(1), 1);
    }

    function test_aCapExpiresOnTheParentChainsClock() public {
        _ready();
        vm.roll(block.number + 301);          // a sequencer holding its timestamp back cannot keep the cap alive either
        vm.expectRevert(NftVaultBase.CapExpired.selector);
        raffleVault.sweep(address(market), "", _ids(1), 1);
    }

    function test_onlyTheKeeperPostsACap() public {
        vm.prank(attacker);
        vm.expectRevert(NftVaultBase.NotKeeper.selector);
        raffleVault.postCap(100 ether, "mine");
    }

    // ---- reentrancy --------------------------------------------------------------------------------------------------------

    /// A seller paid mid-sweep tries to sweep again.
    function test_aSellerCannotReenterTheSweep() public {
        _ready();
        ReentrantSeller evil = new ReentrantSeller();
        nft.mint(address(evil), 1);
        evil.approve(nft, address(market));
        evil.list(market, 1, 0.05 ether);
        _note(1, address(evil), 0.05 ether);
        nft.mint(seller, 2);
        evil.arm(address(raffleVault), abi.encodeCall(NftSweepVault.sweep, (address(market), "", _ids(2), 1)));
        vm.expectRevert();
        raffleVault.sweep(address(market), _buyCall(_ids(1), address(raffleVault)), _ids(1), 1);
        assertEq(address(raffleVault).balance, 1 ether);
    }

    /// A seller that is also a coin's router tries to pay money INTO the vault mid-sweep, to make the ETH that left look smaller.
    function test_aDepositCannotLandMidSweep() public {
        _ready();
        ReentrantSeller evil = new ReentrantSeller();
        address coinX = makeAddr("coinX");
        launcher.set(coinX, address(evil), attacker, 0);
        nft.mint(address(evil), 1);
        evil.approve(nft, address(market));
        evil.list(market, 1, 0.05 ether);
        _note(1, address(evil), 0.05 ether);
        evil.arm(address(raffleVault), abi.encodeCall(INftVault.deposit, (coinX)));
        vm.expectRevert();
        raffleVault.sweep(address(market), _buyCall(_ids(1), address(raffleVault)), _ids(1), 1);
    }

    /// The collection calls back into the vault while the vault receives through onERC721Received (a safe transfer).
    function test_aCollectionCannotReenterThroughTheReceiveHook() public {
        _ready();
        _ask(_ids(1), 0.05 ether);
        nft.mint(seller, 2);
        nft.setHook(address(raffleVault), abi.encodeCall(NftSweepVault.sweep, (address(market), "", _ids(2), 1)), true);
        vm.expectRevert();
        raffleVault.sweep(address(market), _buyCall(_ids(1), address(raffleVault)), _ids(1), 1);
    }

    /// The collection calls back into claimWin / burn while the won NFT is being transferred out.
    function test_aCollectionCannotReenterWhileAWinIsDelivered() public {
        _ready();
        uint256 a = _sweep(raffleVault, _ids(1), 0.05 ether);
        bytes32[] memory leaves = _tickets();
        vm.prank(keeper);
        raffleVault.openDraw(a, Merkle.root(leaves), 100, "ipfs://t");
        Clock.skip(15 minutes);
        raffleVault.requestDraw(a);
        raffleVault.resolve(a, abi.encode(keccak256("r")));
        (address who, uint256 first, uint256 end, uint256 idx) = _holderOf(raffleVault.lot(a).winningTicket);
        bytes32[] memory proof = Merkle.proof(leaves, idx);
        nft.setHook(address(raffleVault), abi.encodeCall(NftVaultBase.claimWin, (a, attacker, first, end, proof)), true);
        vm.expectRevert(ReentrancyGuard.ReentrancyGuardReentrantCall.selector);
        raffleVault.claimWin(a, who, first, end, proof);
    }

    // ---- attribution -------------------------------------------------------------------------------------------------------

    function test_aDirectPaymentToAVaultIsRefused() public {
        vm.deal(attacker, 1 ether);
        vm.prank(attacker);
        (bool ok,) = address(raffleVault).call{value: 1 ether}("");
        assertFalse(ok);
        vm.prank(attacker);
        (ok,) = address(remoteRaffle).call{value: 1 ether}("");
        assertFalse(ok);
    }

    function test_forcedEthIsNeverCountedOrSpent() public {
        _ready();
        vm.deal(address(raffleVault), 101 ether);            // as a selfdestruct would
        assertEq(raffleVault.attributed(), 1 ether);
        _cap(raffleVault, 60 ether);
        _ask(_ids(1), 50 ether);
        vm.expectRevert();                                   // value is capped at what is attributed, so the market is underpaid
        raffleVault.sweep(address(market), _buyCall(_ids(1), address(raffleVault)), _ids(1), 1);
    }

    /// The router attributes what the ESCROW paid it, not its balance: a direct payment goes to the treasury, whole.
    function test_aDirectPaymentToARouterIsNotAttributed() public {
        MockEscrow escrow = new MockEscrow();
        MockCurve curve = new MockCurve(escrow);
        NftFeeRouter impl = new NftFeeRouter();
        NftFeeRouter r = NftFeeRouter(payable(Clones.clone(address(impl))));
        address coinR = makeAddr("coinR");
        r.initialize(coinR, INftRegistry(address(nreg)), INftVault(address(raffleVault)), IPonsV2FeeEscrow(address(escrow)),
            IPonsV2BondingCurve(address(curve)), IPonsV2MemeHook(address(0)));
        curve.setRecipient(address(r));
        launcher.set(coinR, address(r), attacker, 0);
        vm.deal(address(curve), 1 ether);                    // real fees, in the curve
        vm.deal(attacker, 5 ether);
        vm.prank(attacker);
        (bool ok,) = address(r).call{value: 5 ether}("");    // a direct payment, hoping to buy raffle weight
        assertTrue(ok);
        r.harvest();
        assertEq(raffleVault.contributedBy(coinR), 0.8 ether, "80% of the escrow's 1 ETH, and nothing of the 5");
        assertEq(treasury.balance, 0.2 ether + 5 ether);
        assertEq(r.earned(), 1 ether);
    }

    function test_aRouterCannotBeInitialisedByAnyoneButItsLauncher() public {
        NftFeeRouter impl = new NftFeeRouter();
        NftFeeRouter r = NftFeeRouter(payable(Clones.clone(address(impl))));
        vm.prank(attacker);
        vm.expectRevert(NftFeeRouter.NotLauncher.selector);
        r.initialize(attacker, INftRegistry(address(nreg)), INftVault(address(raffleVault)), IPonsV2FeeEscrow(address(0)),
            IPonsV2BondingCurve(address(0)), IPonsV2MemeHook(address(0)));
        vm.expectRevert(NftFeeRouter.AlreadyInitialised.selector);
        impl.initialize(attacker, INftRegistry(address(nreg)), INftVault(address(raffleVault)), IPonsV2FeeEscrow(address(0)),
            IPonsV2BondingCurve(address(0)), IPonsV2MemeHook(address(0)));
    }

    // ---- policy is for ever ------------------------------------------------------------------------------------------------

    /// Burn money and raffle money never meet: a coin of the burn listing cannot pay the raffle vault, nor the reverse, and no
    /// function moves an acquisition from one fate to the other.
    function test_burnAndRaffleMoneyNeverMix() public {
        vm.prank(router);
        vm.expectRevert(NftVaultBase.NotRouter.selector);
        raffleVault.deposit{value: 1 ether}(coinC);
        vm.prank(router);
        vm.expectRevert(NftVaultBase.NotRouter.selector);
        burnVault.deposit{value: 1 ether}(coinA);
        assertTrue(address(raffleVault) != address(burnVault));
    }
}
