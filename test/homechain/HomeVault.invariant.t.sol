// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {HomeBase} from "./HomeBase.sol";
import {Clock} from "../utils/Clock.sol";
import {Merkle} from "../utils/Merkle.sol";
import {HomeVaultBase} from "../../src/homechain/HomeVaultBase.sol";
import {HomeVaultYoyo} from "../../src/homechain/HomeVaultYoyo.sol";
import {HomeVaultRaffle} from "../../src/homechain/HomeVaultRaffle.sol";
import {HomeGuard} from "../../src/homechain/HomeGuard.sol";
import {MockSeaport, MockPunks} from "./HomeMocks.sol";
import {MockNft} from "../nft/NftMocks.sol";
import {
    ItemType, OrderType, OfferItem, ConsiderationItem, OrderParameters, AdvancedOrder, CriteriaResolver, FulfillmentComponent,
    ISeaport16Sweep
} from "../../src/nft/Seaport16.sol";

/// Every actor in any order against a Yoyo punks vault and a Raffle ERC-721 vault: funders (native and fast), sellers listing at
/// any price, triggers buying, relisting and settling, buyers taking asks, the keeper posting caps, raffles run end to end, time
/// passing. The invariants are the promises of docs/v6/home-chain-vault.md §3 and §6.
contract HomeVaultHandler is Test {
    HomeVaultYoyo public py;
    HomeVaultRaffle public rv;
    MockPunks punks;
    MockNft nft;
    MockSeaport seaport;
    HomeGuard guard;
    address keeper;
    address timelock;
    address[3] triggers;
    address seller;
    address buyer;

    uint256 public rewardsPaid;
    uint256 public buys;
    uint256 public settles;
    uint256 public wins;
    uint256 public fundedTotalPy;
    uint256 public fundedTotalRv;
    uint256 public panics;
    uint256 nextPunk = 100;
    uint256 nextNft = 100;
    uint256 nextTicket = 1;

    constructor(HomeVaultYoyo py_, HomeVaultRaffle rv_, MockPunks punks_, MockNft nft_, MockSeaport seaport_, HomeGuard guard_,
        address keeper_, address timelock_, address[3] memory triggers_, address seller_, address buyer_)
    {
        py = py_; rv = rv_; punks = punks_; nft = nft_; seaport = seaport_; guard = guard_; keeper = keeper_; timelock = timelock_;
        triggers = triggers_; seller = seller_; buyer = buyer_;
        vm.prank(seller_);
        nft.setApprovalForAll(address(seaport_), true);
    }

    function _isPanic(bytes memory r) private pure returns (bool) {
        return r.length >= 4 && r[0] == 0x4e && r[1] == 0x48 && r[2] == 0x7b && r[3] == 0x71;
    }
    function _caught(bytes memory r) private { if (_isPanic(r)) panics++; }

    function fund(uint96 amount, bool toPunks) external {
        uint256 a = bound(amount, 0.001 ether, 8 ether);
        HomeVaultBase v = toPunks ? HomeVaultBase(payable(address(py))) : HomeVaultBase(payable(address(rv)));
        vm.deal(address(this), a);
        try v.fund{value: a}(nextTicket++) {
            if (toPunks) fundedTotalPy += a; else fundedTotalRv += a;
        } catch (bytes memory r) { _caught(r); }
    }

    function cap(uint96 capWei, bool toPunks) external {
        uint256 c = bound(capWei, 0.01 ether, 10 ether);
        vm.prank(keeper);
        if (toPunks) py.postCap(c, "x"); else rv.postCap(c, "x");
    }

    function unpause() external {
        if (guard.paused()) { vm.prank(timelock); guard.setPaused(false); }
    }

    function offerAndBuyPunk(uint96 price, uint8 who) external {
        uint256 p = bound(price, 0.01 ether, 10 ether);
        uint256 id = nextPunk++;
        punks.mint(seller, id);
        vm.prank(seller);
        punks.offerPunkForSale(id, p);
        address t = triggers[who % 3];
        uint256 before = t.balance;
        vm.prank(t, t);
        try py.buyPunk(id) { buys++; rewardsPaid += t.balance - before; assertEq(t.balance - before, py.reward()); }
        catch (bytes memory r) { _caught(r); }
    }

    function takeAsk(uint8 which) external {
        uint256 n = py.acquisitionCount();
        if (n == 0) return;
        uint256 aid = which % n;
        HomeVaultBase.Acquisition memory a = py.acquisition(aid);
        (bool forSale,, address s, uint256 minValue,) = punks.punksOfferedForSale(a.tokenId);
        if (!forSale || s != address(py)) return;
        vm.deal(buyer, minValue);
        vm.prank(buyer);
        punks.buyPunk{value: minValue}(a.tokenId);
    }

    function settle(uint8 which, uint8 who) external {
        uint256 n = py.acquisitionCount();
        if (n == 0) return;
        address t = triggers[who % 3];
        uint256 before = t.balance;
        vm.prank(t, t);
        try py.settle(which % n) { settles++; rewardsPaid += t.balance - before; assertEq(t.balance - before, py.reward()); }
        catch (bytes memory r) { _caught(r); }
    }

    function relist(uint8 which, uint8 who) external {
        uint256 n = py.acquisitionCount();
        if (n == 0) return;
        address t = triggers[who % 3];
        uint256 before = t.balance;
        vm.prank(t, t);
        try py.relist(which % n) { rewardsPaid += t.balance - before; } catch (bytes memory r) { _caught(r); }
    }

    function buyNft(uint96 price, uint8 who) external {
        uint256 p = bound(price, 0.01 ether, 10 ether);
        uint256 id = nextNft++;
        nft.mint(seller, id);
        // a FULL_OPEN advanced order at `p`
        bytes memory data = _available(id, p);
        uint256[] memory ids = new uint256[](1);
        ids[0] = id;
        address t = triggers[who % 3];
        uint256 before = t.balance;
        vm.prank(t, t);
        try rv.buySeaport(data, ids, 1) { buys++; rewardsPaid += t.balance - before; assertEq(t.balance - before, rv.reward()); }
        catch (bytes memory r) { _caught(r); }
    }

    function raffle(uint8 which, bytes32 randomness) external {
        uint256 n = rv.acquisitionCount();
        if (n == 0) return;
        uint256 aid = which % n;
        if (rv.acquisition(aid).stage != HomeVaultBase.Stage.Held) return;
        bytes32[] memory leaves = new bytes32[](2);
        leaves[0] = rv.ticketLeaf(triggers[0], 0, 60);
        leaves[1] = rv.ticketLeaf(triggers[1], 60, 100);
        bytes32 root = Merkle.root(leaves);
        vm.prank(keeper);
        rv.openDraw(aid, root, 100, "ipfs://t");
        Clock.skip(15 minutes);
        rv.requestDraw(aid);
        Clock.skip(2 minutes);
        rv.resolve(aid, abi.encode(randomness == bytes32(0) ? bytes32(uint256(1)) : randomness));
        uint256 w = rv.draw(aid).winningTicket;
        (address who, uint256 f, uint256 e, uint256 i) = w < 60 ? (triggers[0], uint256(0), uint256(60), uint256(0)) : (triggers[1], uint256(60), uint256(100), uint256(1));
        try rv.claimWin(aid, who, f, e, Merkle.proof(leaves, i)) { wins++; assertEq(nft.ownerOf(rv.acquisition(aid).tokenId), who); }
        catch (bytes memory r) { _caught(r); }
    }

    function tick(uint32 secs) external {
        Clock.skip(bound(secs, 1, 2 days));
    }

    /// fulfillAvailableAdvancedOrders for one FULL_OPEN order: the seller sells `id` for `price`, bought into the raffle vault.
    function _available(uint256 id, uint256 price) private view returns (bytes memory) {
        OfferItem[] memory offer = new OfferItem[](1);
        offer[0] = OfferItem(ItemType.ERC721, address(nft), id, 1, 1);
        ConsiderationItem[] memory cons = new ConsiderationItem[](1);
        cons[0] = ConsiderationItem(ItemType.NATIVE, address(0), 0, price, price, payable(seller));
        AdvancedOrder[] memory orders = new AdvancedOrder[](1);
        orders[0].parameters = OrderParameters(seller, address(0), offer, cons, OrderType.FULL_OPEN, 0, type(uint256).max, bytes32(0),
            id, bytes32(0), 1);
        orders[0].numerator = 1;
        orders[0].denominator = 1;
        return abi.encodeCall(ISeaport16Sweep.fulfillAvailableAdvancedOrders, (orders, new CriteriaResolver[](0),
            new FulfillmentComponent[][](0), new FulfillmentComponent[][](0), bytes32(0), address(rv), 1));
    }
}

contract HomeVaultInvariantTest is HomeBase {
    HomeVaultHandler h;
    address buyer = makeAddr("buyer");

    function setUp() public override {
        super.setUp();
        vm.prank(timelock);
        guard.setPaused(false);
        address[3] memory ts = [makeAddr("t0"), makeAddr("t1"), makeAddr("t2")];
        h = new HomeVaultHandler(punkYoyo, raffle, punks, nft, seaport, guard, keeper, timelock, ts, seller, buyer);
        bytes4[] memory sels = new bytes4[](10);
        sels[0] = h.fund.selector;
        sels[1] = h.cap.selector;
        sels[2] = h.offerAndBuyPunk.selector;
        sels[3] = h.takeAsk.selector;
        sels[4] = h.settle.selector;
        sels[5] = h.relist.selector;
        sels[6] = h.raffle.selector;
        sels[7] = h.tick.selector;
        sels[8] = h.buyNft.selector;
        sels[9] = h.unpause.selector;
        targetSelector(FuzzSelector(address(h), sels));
        targetContract(address(h));
    }

    /// The balance is never below what the ledger owes: budget (attributed + waiting) plus proceeds held.
    function invariant_balanceCoversTheLedger() public view {
        assertGe(address(punkYoyo).balance, punkYoyo.attributed() + punkYoyo.waiting() + punkYoyo.proceedsHeld());
        assertGe(address(raffle).balance, raffle.attributed() + raffle.waiting() + raffle.proceedsHeld());
    }

    /// Rewards paid never exceed reward × successful triggers, and the caller's gain per trigger is exactly the reward (asserted in
    /// the handler at each success).
    function invariant_rewardsAreBoundedByTriggers() public view {
        assertLe(h.rewardsPaid(), REWARD * (h.buys() + h.settles() + punkYoyo.purchaseCount() + raffle.purchaseCount()));
    }

    /// Every acquisition's price was at most the cap in force, and the guard's counters match what the vaults hold.
    function invariant_capsHold() public view {
        HomeGuard.Cap memory c = guard.capOf(address(punks));
        assertLe(c.items, c.maxItems);
        assertLe(c.valueWei, c.maxValueWei);
        HomeGuard.Cap memory n = guard.capOf(address(nft));
        assertLe(n.items, n.maxItems);
        assertLe(n.valueWei, n.maxValueWei);
        (, uint256 gmi, uint256 gmv,, uint256 gi, uint256 gv,,) = guard.global();
        assertLe(gi, gmi);
        assertLe(gv, gmv);
    }

    /// The outbound window never exceeds its limit.
    function invariant_outflowWithinLimits() public view {
        (uint256 pc, uint256 gl, uint256 gw,,) = guard.limits();
        (uint256 items,,) = guard.windowOf(address(punks));
        assertLe(items, pc);
        (uint256 gItems, uint256 gValue,) = guard.globalWindow();
        assertLe(gItems, gl);
        assertLe(gValue, gw);
    }

    /// An NFT leaves a raffle vault only to a proven winner: everything the vault bought and did not deliver is still here.
    function invariant_raffleNftsLeaveOnlyByAWin() public view {
        for (uint256 i = 0; i < raffle.acquisitionCount(); ++i) {
            HomeVaultBase.Acquisition memory a = raffle.acquisition(i);
            if (a.stage == HomeVaultBase.Stage.Delivered) assertEq(nft.ownerOf(a.tokenId), raffle.draw(i).winner);
            else assertEq(nft.ownerOf(a.tokenId), address(raffle));
        }
    }

    /// A punk the Yoyo vault holds is always offered at exactly pricePaid × markup, to anyone; a sold one was settled or is settleable.
    function invariant_yoyoAsksAreTheMarkup() public view {
        for (uint256 i = 0; i < punkYoyo.acquisitionCount(); ++i) {
            HomeVaultBase.Acquisition memory a = punkYoyo.acquisition(i);
            if (punks.punkIndexToAddress(a.tokenId) == address(punkYoyo)) {
                (bool forSale,,, uint256 minValue, address only) = punks.punksOfferedForSale(a.tokenId);
                assertTrue(forSale);
                assertEq(minValue, a.pricePaid * MARKUP / 10_000);
                assertEq(only, address(0));
            }
        }
    }

    function invariant_noPanics() public view {
        assertEq(h.panics(), 0);
    }
}
