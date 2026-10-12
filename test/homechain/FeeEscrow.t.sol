// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {Clock} from "../utils/Clock.sol";
import {MockLauncher} from "../utils/Mocks.sol";
import {MockArbSys, MockBuybackRouter, MockQuote, MockERC20} from "./HomeMocks.sol";
import {FeeEscrow, IHomeVaultFund} from "../../src/homechain/FeeEscrow.sol";
import {HomeReceiver, IFeeEscrowTickets, IBuybackRouter, IEthQuote} from "../../src/homechain/HomeReceiver.sol";
import {INftLauncher} from "../../src/nft/INft.sol";
import {IArbSys, AddressAlias} from "../../src/homechain/IHomeChain.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";

/// A bridge depository that keeps what it is sent, for the fast path.
contract MockDepository {
    uint256 public received;
    function depositNative(address, bytes32) external payable { received += msg.value; }
}

/// docs/v6/home-chain-vault.md §5: the 4663 side. Deposits from the coin's router only; anyone releases through ArbSys; the keeper
/// through an allow-listed bridge, bounded by the vault's confirmation; the confirmation only from the vault's alias; proceeds and
/// wX credited only from that alias, bought back in tranches and released to the ticket's recipient.
contract FeeEscrowTest is Test {
    MockArbSys arbSys;
    MockLauncher launcher;
    FeeEscrow e;
    HomeReceiver r;
    MockBuybackRouter router;
    MockQuote quote;
    MockERC20 wx;
    MockDepository depo;

    address homeVault = makeAddr("HomeVault on Ethereum");
    address vaultAlias;
    address owner = makeAddr("timelock");
    address keeper = makeAddr("keeper");
    address coinRouter = makeAddr("the coin's NftFeeRouter");
    address coin = makeAddr("coin");
    address coin2 = makeAddr("coin 2");
    address curve = makeAddr("the coin's curve (wX recipient)");
    address anyone = makeAddr("anyone");
    uint256 constant REWARD = 0.001 ether;
    uint256 constant MIN_RELEASE = 0.05 ether;

    function setUp() public {
        vm.warp(1_790_000_000);
        vm.roll(26_000_000);
        arbSys = new MockArbSys();
        launcher = new MockLauncher();
        launcher.set(coin, coinRouter, makeAddr("creator"), 7);
        launcher.set(coin2, coinRouter, makeAddr("creator 2"), 7);
        e = new FeeEscrow(IArbSys(address(arbSys)), homeVault, INftLauncher(address(launcher)), 7, owner, keeper, REWARD, MIN_RELEASE, 12);
        vaultAlias = AddressAlias.l2(homeVault);
        router = new MockBuybackRouter(1000);
        quote = new MockQuote(1000);
        wx = new MockERC20();
        r = new HomeReceiver(homeVault, IFeeEscrowTickets(address(e)), IERC20(address(wx)), IBuybackRouter(address(router)),
            IEthQuote(address(quote)), REWARD, 1 ether, 1 hours);
        depo = new MockDepository();
        vm.deal(coinRouter, 100 ether);
        vm.deal(vaultAlias, 100 ether);
    }

    receive() external payable {}      // the tests call `release` themselves and take the reward

    function _deposit(address c, uint256 amount) internal {
        vm.prank(coinRouter);
        e.deposit{value: amount}(c);
    }

    // ---- deposits ----------------------------------------------------------------------------------------------------------------

    function test_depositsFromTheRouterOnly() public {
        vm.expectRevert(FeeEscrow.NotRouter.selector);
        e.deposit{value: 1 ether}(coin);
        launcher.set(makeAddr("other coin"), coinRouter, makeAddr("c"), 8);      // another listing
        vm.prank(coinRouter);
        vm.expectRevert(FeeEscrow.NotRouter.selector);
        e.deposit{value: 1 ether}(makeAddr("other coin"));
        vm.prank(coinRouter);
        vm.expectRevert(FeeEscrow.ZeroAmount.selector);
        e.deposit{value: 0}(coin);
        _deposit(coin, 1 ether);
        assertEq(e.unspentOf(coin), 1 ether);
        assertEq(e.recipientOf(coin), coinRouter);
        vm.prank(coinRouter);
        e.depositFor{value: 0.5 ether}(coin, curve);
        assertEq(e.unspentOf(coin), 1.5 ether);
        assertEq(e.recipientOf(coin), curve);
        assertEq(e.accountedTotal(), 1.5 ether);
        // a plain payment is refused: no receive
        (bool ok,) = address(e).call{value: 1 ether}("");
        assertFalse(ok);
    }

    // ---- native release ------------------------------------------------------------------------------------------------------------

    function test_anyoneReleasesThroughArbSysAndGetsTheReward() public {
        _deposit(coin, 0.04 ether);
        vm.expectRevert(abi.encodeWithSelector(FeeEscrow.TooSmall.selector, 0.04 ether, MIN_RELEASE));
        e.release(coin);
        _deposit(coin, 0.06 ether);
        uint256 before = anyone.balance;
        vm.prank(anyone);
        uint256 id = e.release(coin);
        assertEq(id, 1, "tickets start at 1: 0 is unattributed on the home chain");
        assertEq(anyone.balance - before, REWARD);
        MockArbSys.Sent memory s = arbSys.last();
        assertEq(s.destination, homeVault);
        assertEq(s.value, 0.1 ether - REWARD);
        assertEq(s.data, abi.encodeCall(IHomeVaultFund.fund, (1)));
        assertEq(e.unspentOf(coin), 0);
        assertEq(e.accountedTotal(), 0);
        assertEq(address(e).balance, 0, "nothing stays");
        FeeEscrow.Ticket memory t = e.ticket(1);
        assertEq(t.coin, coin);
        assertEq(t.amount, 0.1 ether);
        assertEq(uint8(t.mode), uint8(FeeEscrow.Mode.Native));
        assertEq(uint8(t.status), uint8(FeeEscrow.Status.Sent));
        assertEq(e.coinOf(1), coin);
        assertEq(e.l2RecipientOf(1), coinRouter);
    }

    // ---- confirmation: the alias only ---------------------------------------------------------------------------------------------

    function test_confirmOnlyFromTheVaultsAlias() public {
        _deposit(coin, 0.1 ether);
        e.release(coin);
        vm.expectRevert(FeeEscrow.NotHomeVault.selector);
        e.confirm(1, 0.099 ether);
        vm.prank(homeVault);                                 // the un-aliased L1 address: not it either
        vm.expectRevert(FeeEscrow.NotHomeVault.selector);
        e.confirm(1, 0.099 ether);
        vm.prank(vaultAlias);
        e.confirm(1, 0.099 ether);
        FeeEscrow.Ticket memory t = e.ticket(1);
        assertEq(uint8(t.status), uint8(FeeEscrow.Status.Confirmed));
        assertEq(t.amountSeen, 0.099 ether);
        vm.prank(vaultAlias);
        vm.expectRevert(FeeEscrow.WrongStage.selector);
        e.confirm(1, 0.099 ether);
        vm.prank(vaultAlias);
        vm.expectRevert(FeeEscrow.BadInput.selector);
        e.confirm(9, 1);
        assertEq(vaultAlias, address(uint160(homeVault) + uint160(0x1111000000000000000000000000000000001111)));
    }

    // ---- fast release: the keeper, allow-listed, bounded by confirmation -------------------------------------------------------------

    function test_fastReleaseNeedsAnAllowListedBridgeAfterADelay() public {
        _deposit(coin, 0.1 ether);
        bytes memory call_ = abi.encodeCall(MockDepository.depositNative, (homeVault, bytes32("quote")));
        vm.prank(keeper);
        vm.expectRevert(FeeEscrow.BridgeNotAllowed.selector);
        e.releaseFast(coin, address(depo), call_);
        vm.expectRevert(FeeEscrow.NotOwner.selector);
        e.proposeBridge(address(depo));
        vm.prank(owner);
        e.proposeBridge(address(depo));
        vm.expectRevert(FeeEscrow.TooEarly.selector);
        e.acceptBridge(address(depo));
        Clock.skip(7 days);
        e.acceptBridge(address(depo));
        assertTrue(e.isBridge(address(depo)));
        vm.expectRevert(FeeEscrow.NotKeeper.selector);
        e.releaseFast(coin, address(depo), call_);
        vm.prank(keeper);
        uint256 id = e.releaseFast(coin, address(depo), call_);
        assertEq(depo.received(), 0.1 ether, "the whole amount: no reward on the keeper's path");
        assertEq(uint8(e.ticket(id).mode), uint8(FeeEscrow.Mode.Fast));
        assertEq(e.fastOpen(), 1);
        vm.prank(owner);
        e.removeBridge(address(depo));
        assertFalse(e.isBridge(address(depo)));
    }

    function test_anOverdueFastTicketBlocksFastReleasesUntilConfirmed() public {
        vm.prank(owner);
        e.proposeBridge(address(depo));
        Clock.skip(7 days);
        e.acceptBridge(address(depo));
        _deposit(coin, 0.1 ether);
        bytes memory call_ = abi.encodeCall(MockDepository.depositNative, (homeVault, bytes32("q")));
        vm.prank(keeper);
        uint256 id = e.releaseFast(coin, address(depo), call_);
        vm.expectRevert(FeeEscrow.TooEarly.selector);
        e.flagOverdue(id);
        Clock.skipTimeOnly(3 days);
        vm.expectRevert(FeeEscrow.TooEarly.selector);
        e.flagOverdue(id);
        vm.roll(block.number + 3 days / 12 + 1);
        e.flagOverdue(id);
        assertEq(e.fastOverdue(), 1);
        _deposit(coin2, 0.1 ether);
        vm.prank(keeper);
        vm.expectRevert(FeeEscrow.FastBlocked.selector);
        e.releaseFast(coin2, address(depo), call_);
        // the native path is never blocked
        e.release(coin2);
        // the vault's confirmation clears it
        vm.prank(vaultAlias);
        e.confirm(id, 0.1 ether);
        assertEq(e.fastOverdue(), 0);
        assertEq(e.fastOpen(), 0);
        _deposit(coin2, 0.1 ether);
        vm.prank(keeper);
        e.releaseFast(coin2, address(depo), call_);
    }

    // ---- the receiver: proceeds and buyback ----------------------------------------------------------------------------------------

    function test_proceedsCreditedOnlyFromTheAlias_andBoughtBackInTranches() public {
        _deposit(coin, 0.1 ether);
        e.release(coin);                                     // ticket 1 = coin
        vm.expectRevert(HomeReceiver.NotHomeVault.selector);
        r.creditProceeds{value: 1 ether}(1);
        vm.prank(vaultAlias);
        r.creditProceeds{value: 2.5 ether}(1);
        assertEq(r.proceedsOf(coin), 2.5 ether);
        uint256 before = anyone.balance;
        vm.prank(anyone);
        uint256 burned = r.buyback(coin);
        assertEq(anyone.balance - before, REWARD);
        assertEq(router.lastEth(), 1 ether - REWARD, "one tranche less the reward");
        assertEq(router.lastTo(), r.DEAD());
        assertEq(burned, (1 ether - REWARD) * 1000);
        assertEq(r.proceedsOf(coin), 1.5 ether);
        vm.prank(anyone);
        vm.expectRevert(abi.encodeWithSelector(HomeReceiver.Cooldown.selector, uint64(block.timestamp + 1 hours)));
        r.buyback(coin);
        Clock.skip(1 hours);
        r.buyback(coin);
        Clock.skip(1 hours);
        r.buyback(coin);                                     // the last 0.5
        assertEq(r.proceedsOf(coin), 0);
        vm.expectRevert(HomeReceiver.ZeroAmount.selector);
        r.buyback(coin);
    }

    function test_buybackRefusesWorseThanSpotLessFivePercent() public {
        _deposit(coin, 0.1 ether);
        e.release(coin);
        vm.prank(vaultAlias);
        r.creditProceeds{value: 1 ether}(1);
        router.setRate(940);                                 // 6% under the quote's 1000
        vm.expectRevert("TooLittle");
        r.buyback(coin);
        router.setRate(951);
        r.buyback(coin);
    }

    // ---- the receiver: wX for a ticket -------------------------------------------------------------------------------------------------

    function test_wrappedReleasedOnlyToTheTicketsRecipient() public {
        vm.prank(coinRouter);
        e.depositFor{value: 0.1 ether}(coin, curve);
        e.release(coin);                                     // ticket 1, recipient = curve
        vm.expectRevert(HomeReceiver.NotHomeVault.selector);
        r.creditWrapped(1, 1e18);
        vm.prank(vaultAlias);
        r.creditWrapped(1, 1e18);
        vm.expectRevert(HomeReceiver.NothingOwed.selector);  // the report is here, the token is not yet
        r.claimWrapped(1);
        wx.mint(address(r), 1e18);                           // the gateway's deposit lands
        vm.prank(anyone);
        r.claimWrapped(1);
        assertEq(wx.balanceOf(curve), 1e18);
        assertEq(r.wrappedOwed(1), 0);
        vm.expectRevert(HomeReceiver.NothingOwed.selector);
        r.claimWrapped(1);
    }

    function test_absorbAttributesUnaccountedEthToATicketsCoin() public {
        _deposit(coin, 0.1 ether);
        e.release(coin);
        vm.deal(address(r), 0.3 ether);                     // a dead retryable's refund, credited without a call
        r.absorb(1);
        assertEq(r.proceedsOf(coin), 0.3 ether);
        vm.expectRevert(HomeReceiver.ZeroAmount.selector);
        r.absorb(1);
    }

    function testFuzz_escrowNeverKeepsWhatItReleased(uint96 a, uint96 b) public {
        uint256 x = bound(a, MIN_RELEASE, 10 ether);
        uint256 y = bound(b, 1, 10 ether);
        _deposit(coin, x);
        _deposit(coin2, y);
        e.release(coin);
        assertEq(address(e).balance, y);
        assertEq(e.accountedTotal(), y);
        assertEq(arbSys.last().value + REWARD, x);
    }
}
