// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {CurveBase, MockSeeder} from "../utils/CurveBase.sol";
import {MockVenue} from "../utils/SweepBase.sol";
import {Clock} from "../utils/Clock.sol";
import {MockArbSys, MockERC20, MockL2GatewayRouter, MockBuybackRouter, MockQuote} from "./HomeMocks.sol";
import {ListingRegistry} from "../../src/ListingRegistry.sol";
import {Wrapper} from "../../src/Wrapper.sol";
import {SweepVault} from "../../src/SweepVault.sol";
import {MintDesk} from "../../src/curve/MintDesk.sol";
import {ItemQueue} from "../../src/curve/ItemQueue.sol";
import {CurveFactory} from "../../src/curve/CurveFactory.sol";
import {FrogCurve} from "../../src/curve/FrogCurve.sol";
import {CurveToken} from "../../src/curve/CurveToken.sol";
import {HomeListing} from "../../src/homechain/HomeListing.sol";
import {HomeSweepVault} from "../../src/homechain/HomeSweepVault.sol";
import {HomeWrappedAdapter} from "../../src/homechain/HomeWrappedAdapter.sol";
import {FeeEscrow, IHomeVaultFund} from "../../src/homechain/FeeEscrow.sol";
import {HomeReceiver, IFeeEscrowTickets, IBuybackRouter, IEthQuote} from "../../src/homechain/HomeReceiver.sol";
import {HomePolicy, IArbSys, IL2GatewayRouter, AddressAlias} from "../../src/homechain/IHomeChain.sol";
import {INftLauncher, INftVault} from "../../src/nft/INft.sol";
import {IRandomnessBeacon} from "../../src/interfaces/IRandomnessBeacon.sol";

/// docs/v6/home-chain-vault.md §10: the whole 4663 side of a home-chain collection, over `CurveBase`'s registry and roles — the
/// adapter over a twin, a queue, a seeder, a factory, two `HomeSweepVault`s whose escrows name them, and the listing that
/// verifies the bundle. Then the road home: a coin's sweep share → the vault → `forward` → the escrow → `release` → the home
/// vault's `fund` through ArbSys; and the BURN road: units swept at the venue, retired through the adapter, refunded and
/// reclaimed when the home chain never acknowledges, dispatched to the desk when it does.
contract HomeListingTest is CurveBase {
    MockERC20 twin;
    MockL2GatewayRouter gateway;
    MockArbSys arbSys;
    HomeWrappedAdapter adapter;
    MockVenue homeVenue;
    ItemQueue homeQueue;
    MockSeeder homeSeeder;
    CurveFactory homeFactory;
    HomeListing home;
    FeeEscrow raffleEscrow;
    FeeEscrow burnEscrow;
    HomeSweepVault homeRaffle;
    HomeSweepVault homeBurn;
    HomeReceiver receiver;

    address homeWrapperL1 = makeAddr("HomeWrapper on Ethereum");
    address homeDeskL1 = makeAddr("HomeRedeemDesk on Ethereum");
    address homeVaultRaffle = makeAddr("HomeVaultRaffle on Ethereum");
    address homeVaultBurn = makeAddr("HomeVaultBurn on Ethereum");
    address punksL1 = makeAddr("CryptoPunksMarket");
    address buyer = makeAddr("buyer");
    address deskAlias;
    uint256 constant REWARD = 0.001 ether;
    uint256 constant MIN_RELEASE = 0.01 ether;
    uint256 constant UNIT_PRICE = 0.01 ether;      // the venue's price per whole adapter unit

    function setUp() public override {
        super.setUp();
        twin = new MockERC20();
        gateway = new MockL2GatewayRouter(twin);
        arbSys = new MockArbSys();
        adapter = new HomeWrappedAdapter(IERC20(address(twin)), homeWrapperL1, homeDeskL1, IL2GatewayRouter(address(gateway)),
            IArbSys(address(arbSys)), IRandomnessBeacon(address(beacon)), PBS, "Wrapped Punk (4663)", "wPUNK");
        deskAlias = AddressAlias.l2(homeDeskL1);
        homeVenue = new MockVenue(Wrapper(address(adapter)));
        homeQueue = new ItemQueue(adapter, payee, 500, 0.001 ether, 1_000, PBS);
        homeSeeder = new MockSeeder(IERC20(address(adapter)));
        homeFactory = new CurveFactory(adapter, homeSeeder, homeQueue, platform, _template(), _targets(), PBS);
        home = new HomeListing(registry);
        // §10.2: per policy, the escrow names the vault's address computed ahead, then the vault names the escrow
        (raffleEscrow, homeRaffle) = _pair(0, homeVaultRaffle, SweepVault.Policy.RAFFLE);
        (burnEscrow, homeBurn) = _pair(1, homeVaultBurn, SweepVault.Policy.BURN);
        homeFactory.wire(homeRaffle, homeBurn);
        homeQueue.wire(homeFactory);
        receiver = new HomeReceiver(homeVaultRaffle, IFeeEscrowTickets(address(raffleEscrow)), IERC20(address(twin)),
            IBuybackRouter(address(new MockBuybackRouter(1000))), IEthQuote(address(new MockQuote(1000))), REWARD, 1 ether, 1 hours);
        vm.startPrank(owner);
        assertEq(home.list(_params(HomePolicy.Raffle, homeVaultRaffle, address(raffleEscrow), address(receiver), address(homeRaffle))), 0);
        assertEq(home.list(_params(HomePolicy.Burn, homeVaultBurn, address(burnEscrow), address(0), address(homeBurn))), 1);
        vm.stopPrank();
        homeSeeder.setDepth(2 ether);
        vm.deal(buyer, 100 ether);
    }

    function _pair(uint256 id, address homeVault, SweepVault.Policy policy) internal returns (FeeEscrow e, HomeSweepVault v) {
        address vaultAt = vm.computeCreateAddress(address(this), vm.getNonce(address(this)) + 1);
        e = new FeeEscrow(IArbSys(address(arbSys)), homeVault, INftLauncher(vaultAt), id, owner, keeper, REWARD, MIN_RELEASE, PBS);
        v = new HomeSweepVault(ListingRegistry(address(home)), id, adapter, policy, address(homeVenue), homeFactory, MintDesk(address(0)),
            PBS, INftVault(address(e)));
        assertEq(address(v), vaultAt, "the escrow names the vault");
        _ceiling(v);                                           // C-01: no cap is taken without the owner's ceiling
    }

    function _params(HomePolicy policy, address homeVault, address escrow, address receiver_, address vault)
        internal view returns (HomeListing.Params memory p)
    {
        p.homeChainId = 1;
        p.collection = punksL1;
        p.policy = policy;
        p.name = "CryptoPunks";
        p.homeVault = homeVault;
        p.homeWrapper = homeWrapperL1;
        p.wx = address(twin);
        p.adapter = address(adapter);
        p.escrow = escrow;
        p.receiver = receiver_;
        p.sweepVault = vault;
    }

    function _launchHome(SweepVault.Policy policy) internal returns (FrogCurve c, CurveToken tok) {
        vm.prank(creator);
        (address coin, address curve) = homeFactory.launch("Punk Coin", "PUNKC", policy, true);   // SWEEP_ONLY: §10.1
        c = FrogCurve(payable(curve));
        tok = CurveToken(coin);
    }

    /// Stock the venue with `n` whole adapter units: twin minted here, deposited into the adapter for the venue.
    function _stock(uint256 n) internal {
        twin.mint(address(this), n * ONE);
        twin.approve(address(adapter), n * ONE);
        adapter.deposit(n * ONE, address(homeVenue));
    }

    function _assertAdapterLedger() internal view {
        assertEq(adapter.totalSupply() + adapter.openTickets() * ONE, adapter.escrowed());
        assertEq(adapter.escrowed() + adapter.stray(), twin.balanceOf(address(adapter)));
    }

    // ---- the listing ---------------------------------------------------------------------------------------------------------------

    function test_theListingRecordsAVerifiedBundle_andBorrowsTheRegistrysRoles() public view {
        assertEq(home.owner(), owner);
        assertEq(home.keeper(), keeper);
        assertEq(home.treasury(), treasury);
        assertEq(home.count(), 2);
        HomeListing.Listing memory l = home.listing(0);
        assertEq(l.homeChainId, 1);
        assertEq(l.collection, punksL1);
        assertEq(uint8(l.policy), uint8(HomePolicy.Raffle));
        assertEq(l.homeVault, homeVaultRaffle);
        assertEq(l.homeWrapper, homeWrapperL1);
        assertEq(l.wx, address(twin));
        assertEq(l.adapter, address(adapter));
        assertEq(l.escrow, address(raffleEscrow));
        assertEq(l.receiver, address(receiver));
        assertEq(l.sweepVault, address(homeRaffle));
        assertEq(l.factory, address(homeFactory), "read from the vault, not given");
        assertEq(home.vaultOf(1), address(homeBurn));
        assertEq(home.ref(0), string.concat("eip155:1:", vm.toLowercase(vm.toString(punksL1))));
        assertFalse(home.isPaused(0));
        assertEq(address(homeRaffle.registry()), address(home), "the vault reads roles and pause from the listing");
        assertEq(homeFactory.listingOf(address(0)), 0);
    }

    function test_listRefusals() public {
        HomeListing.Params memory p = _params(HomePolicy.Raffle, homeVaultRaffle, address(raffleEscrow), address(0), address(homeRaffle));
        vm.expectRevert(HomeListing.NotOwner.selector);
        home.list(p);
        vm.startPrank(owner);
        vm.expectRevert(HomeListing.AlreadyListed.selector);        // (1, punks, Raffle) is listed
        home.list(p);
        p.collection = makeAddr("another collection");
        vm.expectRevert(HomeListing.WrongListingId.selector);       // the vault says listing 0; this would be 2
        home.list(p);
        p.policy = HomePolicy.Wrap;
        vm.expectRevert(HomeListing.BadPolicy.selector);
        home.list(p);
        p.policy = HomePolicy.Raffle;
        p.homeChainId = uint64(block.chainid);
        vm.expectRevert(HomeListing.BadChain.selector);
        home.list(p);
        p.homeChainId = 1;
        p.name = "";
        vm.expectRevert(HomeListing.Empty.selector);
        home.list(p);
        p.name = "x";
        p.sweepVault = address(homeBurn);                             // a burn vault for a raffle listing: its escrow says 1 too
        vm.expectRevert(HomeListing.WrongListingId.selector);
        home.list(p);
        vm.stopPrank();
        // a fresh pair for id 2, then every cross-check in turn
        (FeeEscrow e2, HomeSweepVault v2) = _pair(2, homeVaultRaffle, SweepVault.Policy.RAFFLE);
        p = _params(HomePolicy.Raffle, homeVaultRaffle, address(e2), address(0), address(v2));
        p.collection = makeAddr("another collection");
        vm.startPrank(owner);
        p.policy = HomePolicy.Burn;
        vm.expectRevert(HomeListing.WrongPolicy.selector);
        home.list(p);
        p.policy = HomePolicy.Raffle;
        p.adapter = address(wrapper);
        vm.expectRevert(HomeListing.WrongAdapter.selector);
        home.list(p);
        p.adapter = address(adapter);
        p.escrow = address(raffleEscrow);
        vm.expectRevert(HomeListing.WrongEscrow.selector);
        home.list(p);
        p.escrow = address(e2);
        p.homeVault = homeVaultBurn;
        vm.expectRevert(HomeListing.WrongHomeVault.selector);
        home.list(p);
        p.homeVault = homeVaultRaffle;
        p.wx = address(wrapper);
        vm.expectRevert(HomeListing.WrongTwin.selector);
        home.list(p);
        p.wx = address(twin);
        p.homeWrapper = stranger;
        vm.expectRevert(HomeListing.WrongHomeWrapper.selector);
        home.list(p);
        p.homeWrapper = homeWrapperL1;
        p.receiver = address(receiver);                               // the raffle listing's receiver, whose escrow is not e2
        vm.expectRevert(HomeListing.WrongReceiver.selector);
        home.list(p);
        p.receiver = address(0);
        assertEq(home.list(p), 2);
        vm.stopPrank();
        // a vault built with another registry, or an escrow that does not name it, never gets this far
        vm.expectRevert(HomeSweepVault.EscrowNotMine.selector);
        new HomeSweepVault(ListingRegistry(address(home)), 3, adapter, SweepVault.Policy.RAFFLE, address(homeVenue), homeFactory,
            MintDesk(address(0)), PBS, INftVault(address(e2)));
        vm.expectRevert(HomeSweepVault.NoEscrow.selector);
        new HomeSweepVault(ListingRegistry(address(home)), 3, adapter, SweepVault.Policy.RAFFLE, address(homeVenue), homeFactory,
            MintDesk(address(0)), PBS, INftVault(address(0)));
    }

    function test_pauseStopsForwardsAndSweepsOnly() public {
        (FrogCurve c, CurveToken tok) = _launchHome(SweepVault.Policy.RAFFLE);
        _buy(c, buyer, 1 ether);
        c.flushVaultFees();
        assertGt(homeRaffle.credit(address(tok)), 0);
        vm.prank(owner);
        home.setPaused(0, true);
        vm.expectRevert(SweepVault.ListingPaused.selector);
        homeRaffle.forward(address(tok));
        _buy(c, buyer, 1 ether);
        c.flushVaultFees();                                           // money still comes in
        vm.prank(owner);
        home.setPaused(0, false);
        homeRaffle.forward(address(tok));
        vm.prank(owner);
        vm.expectRevert(abi.encodeWithSelector(HomeListing.NoSuchListing.selector, 5));
        home.setPaused(5, true);
        vm.expectRevert(HomeListing.NotOwner.selector);
        home.setPaused(0, true);
    }

    // ---- the sweep share's road home (§10.4, §5) ----------------------------------------------------------------------------------------

    function test_sweepShareReachesTheEscrowAndReleasesToTheHomeVault() public {
        (FrogCurve c, CurveToken tok) = _launchHome(SweepVault.Policy.RAFFLE);
        address coin = address(tok);
        assertTrue(c.sweepOnly());
        assertEq(homeFactory.listingOf(coin), 0, "the coin's listing is the raffle listing");
        _fill(c, buyer);
        c.flushVaultFees();
        uint256 credit = homeRaffle.credit(coin);
        assertGe(credit, MIN_RELEASE, "the raise's fees cover a release");
        // the escrow trusts the vault, for the vault's coins, and nobody else
        assertEq(homeRaffle.routerOf(coin), address(homeRaffle));
        assertEq(homeRaffle.listingOf(coin), 0);
        assertEq(homeRaffle.creatorOf(coin), creator);
        assertEq(homeBurn.routerOf(coin), address(0), "a raffle coin is not the burn vault's");
        assertEq(homeBurn.listingOf(coin), type(uint256).max);
        assertEq(homeRaffle.routerOf(stranger), address(0));
        vm.deal(stranger, 1 ether);
        vm.prank(stranger);
        vm.expectRevert(FeeEscrow.NotRouter.selector);
        raffleEscrow.deposit{value: 1 ether}(coin);
        vm.expectRevert(HomeSweepVault.NothingToForward.selector);
        homeBurn.forward(coin);
        // anyone forwards
        vm.prank(stranger);
        uint256 forwarded = homeRaffle.forward(coin);
        assertEq(forwarded, credit);
        assertEq(homeRaffle.credit(coin), 0);
        assertEq(homeRaffle.totalCredit(), 0);
        assertEq(address(homeRaffle).balance, 0);
        assertEq(raffleEscrow.unspentOf(coin), credit);
        assertEq(raffleEscrow.recipientOf(coin), address(homeRaffle));
        vm.expectRevert(HomeSweepVault.NothingToForward.selector);
        homeRaffle.forward(coin);
        // anyone releases, and is paid the reward; the message is the home vault's fund(ticket 1)
        uint256 before = stranger.balance;
        vm.prank(stranger);
        uint256 id = raffleEscrow.release(coin);
        assertEq(id, 1);
        assertEq(stranger.balance - before, REWARD);
        MockArbSys.Sent memory m = arbSys.last();
        assertEq(m.destination, homeVaultRaffle);
        assertEq(m.value, credit - REWARD);
        assertEq(m.data, abi.encodeCall(IHomeVaultFund.fund, (1)));
        assertEq(m.from, address(raffleEscrow));
        assertEq(raffleEscrow.coinOf(1), coin);
        assertEq(raffleEscrow.l2RecipientOf(1), address(homeRaffle));
    }

    // ---- the BURN road: units through the adapter (§10.3) ----------------------------------------------------------------------------------

    function test_burnPolicy_retiresThroughTheAdapter_refundsWhenNeverAcknowledged_dispatchesWhenItIs() public {
        _stock(10);
        (FrogCurve c, CurveToken tok) = _launchHome(SweepVault.Policy.BURN);
        address coin = address(tok);
        _fill(c, buyer);
        c.flushVaultFees();
        uint256 credit = homeBurn.credit(coin);
        assertGe(credit, 2 * UNIT_PRICE);
        _postCap(homeBurn, UNIT_PRICE);
        homeBurn.sweep(coin, 2 * UNIT_PRICE, abi.encodeCall(MockVenue.buy, (UNIT_PRICE)), 2 * ONE);
        assertEq(homeBurn.units(coin), 2 * ONE);
        _assertAdapterLedger();
        // retire: a burn intent, the unit's twin escrowed at the adapter
        uint256 t0 = homeBurn.retire(coin);
        assertEq(homeBurn.retiredBy(t0), coin);
        assertEq(homeBurn.units(coin), ONE);
        assertEq(adapter.openTickets(), 1);
        assertEq(adapter.ticket(t0).owner, address(homeBurn));
        assertEq(arbSys.last().destination, homeDeskL1, "the intent went up to the desk");
        _assertAdapterLedger();
        // the home chain never acknowledges: after 21 days on both clocks anyone refunds, and the vault reclaims the unit
        vm.expectRevert(SweepVault.WrongStage.selector);
        homeBurn.reclaim(t0);
        Clock.skip(21 days);
        adapter.refund(t0);
        assertTrue(adapter.refunded(t0));
        assertEq(adapter.balanceOf(address(homeBurn)), 2 * ONE);
        homeBurn.reclaim(t0);
        assertEq(homeBurn.units(coin), 2 * ONE, "the coin has its unit back");
        _assertAdapterLedger();
        // retire again: this time the desk acknowledges, and anyone dispatches the twin up to the desk
        uint256 t1 = homeBurn.retire(coin);
        vm.prank(deskAlias);
        adapter.acknowledge(t1);
        uint256 twinBefore = twin.balanceOf(address(adapter));
        adapter.dispatch(t1);
        assertEq(twinBefore - twin.balanceOf(address(adapter)), ONE, "one unit's twin left, burned by the gateway");
        assertEq(gateway.last().to, homeDeskL1);
        assertEq(gateway.last().l1Token, homeWrapperL1);
        assertEq(adapter.openTickets(), 0);
        vm.expectRevert(SweepVault.WrongStage.selector);               // not refunded: nothing to reclaim
        homeBurn.reclaim(t1);
        assertEq(homeBurn.units(coin), ONE);
        _assertAdapterLedger();
        // the escrow road is open to a burn coin too
        homeBurn.forward(coin);
        assertEq(burnEscrow.unspentOf(coin), credit - 2 * UNIT_PRICE);
    }

    /// A non-sweep-only coin on a home-chain collection enqueues at a queue nobody can fill (the adapter is nobody's custodian):
    /// the curve's own refund path is what remains (§10.6).
    function test_aNonSweepOnlyCoinCannotBeFilled_onlyRefunded() public {
        vm.prank(keeper);
        homeRaffle.postCap(UNIT_PRICE);
        vm.prank(creator);
        (address coin, address curve) = homeFactory.launch("Punk Coin", "PUNKC", SweepVault.Policy.RAFFLE, false);
        FrogCurve c = FrogCurve(payable(curve));
        _fill(c, buyer);
        assertEq(uint8(c.state()), uint8(FrogCurve.State.Graduating));
        assertFalse(homeQueue.isFilled(coin));
        vm.prank(address(adapter));                                     // the "custodian" is the adapter: it never calls
        vm.expectRevert();
        homeQueue.recordItem(1 ether);
        vm.expectRevert(FrogCurve.NotFilled.selector);
        c.graduate();
        Clock.skip(3 days);
        c.openRefunds();
        assertEq(uint8(c.state()), uint8(FrogCurve.State.Refunding));
    }
}
