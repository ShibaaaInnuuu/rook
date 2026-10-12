// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {console2} from "forge-std/console2.sol";
import {AuditNftABase} from "./AuditNftABase.sol";
import {FloorModule} from "../../src/floor/FloorModule.sol";
import {PosterSource} from "../../src/floor/PosterSource.sol";
import {EthNftWrapper} from "../../src/nft/EthNftWrapper.sol";
import {IKeeperSource} from "../../src/interfaces/IKeeperSource.sol";
import {IFloorWrapped} from "../../src/interfaces/IFloorWrapped.sol";
import {IRandomnessBeacon} from "../../src/interfaces/IRandomnessBeacon.sol";
import {IListingPause} from "../../src/nft/EthNftWrapper.sol";

/// A-M-1: holder deposits (ON at launch, D5) turn a floor posted above the market, within the module's limits, into ETH taken
/// from the module's backing: buy the NFT at the market floor on Ethereum, deposit it (one unit, fungible with the ETH-backed
/// ones), burn the unit at the module at the posted floor. PASS = the attack works (profit logged).
contract AuditNftA_M1_DepositBurn is AuditNftABase {
    /// The runner withholds cheap listings, the canister clamps at its sale band (+10 %), the module walks there in two hours.
    function _inflateTo(uint256 bps) internal {
        uint256 goal = F * (10_000 + bps) / 10_000;
        while (module.accepted() < goal) { _skip(1 hours); _post(goal); }
        assertEq(module.accepted(), goal, "the module accepted the inflated floor within its own limits");
    }

    function _depositAndBurn(uint256 n) internal returns (uint256 paidOnEthereum, uint256 got) {
        for (uint256 i = 0; i < n; ++i) _deposit(9000 + i, mallory);        // bought on OpenSea at the real floor F, each
        paidOnEthereum = n * F;
        got = _burn(mallory, n * ONE);
        _books();
    }

    function test_A_M1a_graduationDuringTheInflation_depositsAndBurnsTakeTheGap() public {
        _inflateTo(1_000);                                                  // +10 %: the canister's own up_cap
        uint256 gp = module.gradPrice();
        uint256 units = _graduate(4.12 ether);                              // one ordinary 5 ETH raise's item budget (docs §1.2)
        console2.log("gradPrice at the +10% floor (wei per unit)", gp);
        console2.log("graduation units for 4.12 ETH", units);
        console2.log("  at the true floor it would have been", uint256(4.12 ether) * ONE / (F * 10_300 / 10_000));
        _holdFor(module.accepted(), 6);                                     // three calm hours of re-posts: the vol term decays
        // what the canister's advance takes per item at this floor (maxItemCost = accepted x 1.03): 13.3 % over the market
        console2.log("maxItemCost at the +10% floor", module.maxItemCost(), "vs F", F);
        (uint256 unitPrice,) = module.burnCapacity();
        console2.log("burn price per unit", unitPrice, "beta", module.beta());
        (uint256 paid, uint256 got) = _depositAndBurn(STAGE0_MINTS);        // the day's whole mint room (wrapper H11 = 3)
        console2.log("mallory paid on Ethereum (3 x F)", paid);
        console2.log("mallory got from the module", got);
        console2.log("profit in one day (wei)", got - paid);
        assertGt(got, paid * 1_080 / 1_000, "more than 8 % over the market price for the same NFTs");
    }

    function test_A_M1b_graduationAtTheTrueFloor_theBetaCapLimitsTheGap() public {
        uint256 units = _graduate(4.12 ether);                              // before any inflation: beta ~ F x 1.03
        console2.log("graduation units at the true floor", units);
        _inflateTo(1_000);
        _holdFor(module.accepted(), 6);
        (uint256 paid, uint256 got) = _depositAndBurn(STAGE0_MINTS);
        console2.log("mallory paid on Ethereum (3 x F)", paid);
        console2.log("mallory got from the module", got);
        console2.log("profit in one day (wei)", got - paid);
        assertGt(got, paid, "still a profit: the burn pays up to beta = the graduation price (F x 1.03)");
    }

    function test_A_M1c_atTheGuardiansThreshold_plus19pct() public {
        _inflateTo(1_900);                                                  // just under D8's 20 % pause, one day's band
        _graduate(4.12 ether);
        _holdFor(module.accepted(), 6);
        (uint256 paid, uint256 got) = _depositAndBurn(STAGE0_MINTS);
        console2.log("+19%: mallory paid (3 x F)", paid);
        console2.log("+19%: mallory got", got);
        console2.log("+19%: profit in one day (wei)", got - paid);
        assertGt(got, paid * 1_150 / 1_000);
    }

    /// Control: at an honest floor, an NFT bought AT the posted floor loses the round trip (the burn spread).
    function test_A_M1_control_anHonestFloorLoses() public {
        _graduate(4.12 ether);
        _holdFor(F, 6);
        (uint256 paid, uint256 got) = _depositAndBurn(STAGE0_MINTS);
        console2.log("control: paid (3 x F)", paid, "got", got);
        assertLt(got, paid, "no profit at the true floor");
    }

    /// The honest floor is the rank-2 listing (docs §4.2). An NFT bought at the rank-1 listing, 2 % under it, deposited and burned
    /// before any conversion has put a fill in the window, still profits: nothing needs to be mis-posted.
    function test_A_M1d_honestRank2Floor_rank1ListingTwoPctUnder_stillProfits() public {
        _graduate(4.12 ether);
        _holdFor(F, 6);
        for (uint256 i = 0; i < STAGE0_MINTS; ++i) _deposit(7000 + i, mallory);
        uint256 paid = STAGE0_MINTS * F * 98 / 100;
        uint256 got = _burn(mallory, STAGE0_MINTS * ONE);
        console2.log("rank-1 at -2%: paid", paid, "got", got);
        console2.log("rank-1 at -2%: profit in one day (wei)", got - paid);
        assertGt(got, paid);
        _books();
    }

    /// After one conversion the fills price (cost / 1.03, clamped to 3 % under the accepted floor) enters the 24 h time-weighted
    /// average: three hours after it the honest-floor gap is still open; about a day after it, it is closed.
    function test_A_M1e_aFillClosesTheHonestGapOnlyOverItsWindow() public {
        _graduate(4.12 ether);
        uint256 amount = module.maxItemCost();
        vm.prank(custodian); module.advance(amount);
        _mintPurchase(9180, address(module));
        vm.prank(custodian); module.settleAdvance(F);                       // bought at the market floor
        _holdFor(F, 6);                                                     // +3 h
        _deposit(7000, mallory);
        uint256 got3h = _burn(mallory, ONE);
        console2.log("3 h after a fill: price()", module.price(), "burn got", got3h);
        assertGt(got3h, F * 98 / 100, "still a profit 3 h after the fill");
        _holdFor(F, 40);                                                    // +23 h
        _deposit(7001, mallory);
        uint256 got23h = _burn(mallory, ONE);
        console2.log("23 h after a fill: price()", module.price(), "burn got", got23h);
        assertLt(got23h, F * 98 / 100, "closed once the fill dominates the window");
    }
}

/// A-M-2: `redeemToRelist` is a free option on EthNftWrapper. The canister never delivers a relist ticket and marks it
/// undeliverable at once (ethnft::judge_transfer, "a relist ticket: never delivered"), so it refunds at deliverBy + 72 h. Whoever
/// holds `held − openTickets` units can keep every NFT promised to its own tickets, refund and re-burn in one transaction, for gas.
/// Redemption, which "cannot be switched off", is closed to everybody else indefinitely. PASS = the freeze works.
contract AuditNftA_M2_RelistFreeze is AuditNftABase {
    function test_A_M2_threeUnitsFreezeEveryRedemptionIndefinitely_atNoCost() public {
        for (uint256 i = 0; i < 3; ++i) _mintPurchase(100 + i, alice);      // stage 0: three Miladys in custody
        vm.prank(alice);
        w.transfer(mallory, 3 * ONE);                                       // mallory bought 3 wX in the pool
        uint256[] memory ids = new uint256[](3);
        vm.startPrank(mallory);
        for (uint256 i = 0; i < 3; ++i) ids[i] = w.redeemToRelist();
        vm.stopPrank();
        vm.deal(bob, 0);
        _skip(2 minutes);
        for (uint256 cycle = 0; cycle < 3; ++cycle) {
            for (uint256 i = 0; i < 3; ++i) _resolve(ids[i]);               // the keyless resolver, as always
            for (uint256 i = 0; i < 3; ++i) { vm.prank(custodian); w.markUndeliverable(ids[i]); }   // as built: at once
            // bob, holding a unit from anywhere, cannot redeem for the whole cycle
            deal(address(w), bob, ONE, true);
            vm.prank(bob);
            vm.expectRevert(EthNftWrapper.NoItem.selector);
            w.redeem(bob);
            deal(address(w), bob, 0, true);
            _skip(7 days + 72 hours + 1);
            // refund and re-burn in one transaction (a helper contract would do it atomically; here: the same block)
            vm.startPrank(mallory);
            for (uint256 i = 0; i < 3; ++i) { w.refund(ids[i]); ids[i] = w.redeemToRelist(); }
            vm.stopPrank();
            assertEq(w.openTickets(), 3);
            assertEq(w.balanceOf(mallory), 0);
            _skip(2 minutes);
        }
        console2.log("days every redemption was refused", uint256(30));
        // mallory ends where she began: refund her last three and she has her 3 units back
        for (uint256 i = 0; i < 3; ++i) _resolve(ids[i]);
        for (uint256 i = 0; i < 3; ++i) { vm.prank(custodian); w.markUndeliverable(ids[i]); }
        _skip(10 days + 1);
        for (uint256 i = 0; i < 3; ++i) w.refund(ids[i]);
        assertEq(w.balanceOf(mallory), 3 * ONE, "the freeze cost nothing but gas");
        _books();
    }

    /// Without the canister's mark the same ticket refunds at deliverBy + 30 days: the same freeze, 37-day cycles.
    function test_A_M2b_withoutTheMarkTheCycleIsLonger_stillFree() public {
        _mintPurchase(1, alice);
        vm.prank(alice);
        uint256 id = w.redeemToRelist();
        _skip(2 minutes);
        _resolve(id);
        _skip(7 days + 30 days + 1);
        w.refund(id);
        assertEq(w.balanceOf(alice), ONE);
    }
}

/// A-L-1: the wrapper's H11 mint room is one pool for purchases and deposits. Deposits minted while a purchase is between its
/// advance and its fill (the canister skips deposit mints only from `Buying` to `Minting`) take the day's room; the purchase's
/// mint then reverts `MintRateLimited`, its advance stays open, and the module refuses every other advance (`AdvanceOpen`) for up
/// to 24 h. PASS = the starvation works on chain.
contract AuditNftA_L1_MintRoomStarvation is AuditNftABase {
    function test_A_L1_threeDepositsHoldAnOpenAdvanceForADay() public {
        _graduate(4.12 ether);
        uint256 amount = module.maxItemCost();
        vm.prank(custodian);
        module.advance(amount);                                             // EthBuy{NeedAdvance → Funded → Bridging → Arrived}
        for (uint256 i = 0; i < 3; ++i) _deposit(500 + i, mallory);         // three claimed deposits, final, minted meanwhile
        vm.prank(custodian);
        vm.expectRevert(EthNftWrapper.MintRateLimited.selector);
        w.mint(9180, _tx(9180, "seaport fill"), address(module));           // the bought NFT cannot be wrapped
        vm.prank(custodian);
        vm.expectRevert(FloorModule.AdvanceOpen.selector);
        module.advance(1);
        _skip(23 hours);
        vm.prank(custodian);
        vm.expectRevert(EthNftWrapper.MintRateLimited.selector);
        w.mint(9180, _tx(9180, "seaport fill"), address(module));
        assertEq(module.transitUnits(), ONE, "the advance is still open after 23 h");
        _skip(1 hours + 60);
        _post(F);
        _mintPurchase(9180, address(module));
        vm.prank(custodian);
        module.settleAdvance(amount);
        _books();
    }
}

/// A-L-2: the EthNftWrapper's `deployer` is the deploy EOA for good (DeployStack deploys it directly and wires it itself): after
/// the deploy that key alone can cut the collection's mint rate at once or propose a raise anyone accepts in 7 days, and a
/// stage-1 raise needs it. PASS = the power exists and is used after launch.
contract AuditNftA_L2_DeployerKeepsMintLimit is AuditNftABase {
    function test_A_L2_theDeployKeyCutsMintsAfterLaunch() public {
        assertEq(w.deployer(), address(this), "the deploy key (this test stands in for it)");
        _mintPurchase(1, alice);                                            // launched: an item is minted
        w.proposeMintLimit(1);                                              // the deploy key, instantly
        assertEq(w.maxMintsPerDay(), 1);
        vm.prank(custodian);
        vm.expectRevert(EthNftWrapper.MintRateLimited.selector);
        w.mint(3, _tx(3, "seaport fill"), address(module));              // the day's second purchase is refused
        w.proposeMintLimit(1_000);                                          // or a raise: anyone accepts after 7 days
        _skip(7 days + 1);
        vm.prank(stranger);
        w.acceptMintLimit();
        assertEq(w.maxMintsPerDay(), 1_000);
    }
}

/// A-L-3: the guardian's floor pause (D8) is the registry's one keeper switch. With several NFT collections on one registry (the
/// operator's multi-collection launch), a pause for ONE collection's floor zeroes EVERY PosterSource and the registry's keeper EOA:
/// every module's mints, advances and conversions stop, every curve's closing buy is refused (`hasRoomFor` false), sweeps and draws
/// stop. The listing pause, the only other switch, gates the wrappers' mints only. PASS = one pause halts the other collection.
contract AuditNftA_L3_PauseIsGlobal is AuditNftABase {
    function test_A_L3_oneCollectionsPauseHaltsAnotherCollection() public {
        // a second collection (Azuki) on the same registry, same canister, as a second DeployStack NFT run would build it
        EthNftWrapper w2 = new EthNftWrapper(IListingPause(address(registry)), 0, IRandomnessBeacon(address(beacon)), ETHEREUM,
            makeAddr("Azuki on chain 1"), custodian, "Wrapped Azuki", "wAZUKI", PBS);
        PosterSource p2 = new PosterSource(IKeeperSource(address(registry)), custodian);
        FloorModule m2 = new FloorModule(IFloorWrapped(address(w2)), IKeeperSource(address(p2)), custodian, platform, address(0), _params(), PBS);
        w2.wireFloorModule(address(m2));
        w2.initMintLimit(3);
        vm.prank(custodian); m2.postFloor(0.66 ether, 10_000, 50);
        (uint256 price2,) = m2.mintCapacity();
        assertGt(price2, 0, "Azuki mints");
        assertTrue(m2.hasRoomFor(4 ether), "an Azuki curve may close");
        // Milady's posted floor is > 20 % off OpenSea and CoinGecko: the monitor's guardian pauses (D8)
        vm.prank(guardian);
        registry.pauseKeeper();
        (price2,) = m2.mintCapacity();
        assertEq(price2, 0, "Azuki's module no longer mints");
        assertFalse(m2.hasRoomFor(4 ether), "an Azuki curve can no longer close");
        assertEq(m2.maxItemCost(), 0, "no Azuki conversion");
        assertTrue(m2.holdsRefunds(), "every Azuki curve's refunds are held");
        vm.prank(custodian);
        vm.expectRevert(FloorModule.NotKeeper.selector);
        m2.postFloor(0.66 ether, 10_000, 50);                              // the canister cannot post Azuki's (good) floor
        assertEq(registry.keeper(), address(0), "and the registry's keeper EOA (caps, draws) is zero too");
        vm.prank(guardian);
        vm.expectRevert();
        registry.unpauseKeeper();                                           // only the Safe resumes
        // there is no per-collection floor switch: PosterSource has no setter, the listing pause gates wrapper mints only
        vm.prank(owner);
        registry.unpauseKeeper();
        vm.prank(owner);
        registry.setPaused(0, true);
        vm.prank(custodian);
        m2.postFloor(0.66 ether, 10_000, 50);
        (price2,) = m2.mintCapacity();
        assertGt(price2, 0, "the listing pause does not stop the module");
    }
}

/// Invariant spot-check with deposits ON: deposits, purchases (advance → mint → settle), public mints, burns, redemptions of all
/// three kinds, resolves, deliveries, marks and refunds (resolved and pinned), in a random order. The ledger and the books hold,
/// every open ticket has an item, and a deposit minted after a ticket's burn is never drawn by it. PASS = holds.
contract AuditNftA_Holds_LedgerWithDeposits is AuditNftABase {
    uint256[11] hits;

    function testFuzz_A_ledgerHoldsWithDepositsRelistAndAdvances(uint256 seed) public {
        _run(seed);
    }

    /// Non-vacuity: over 20 seeds every action (deposit, advance, settle, return, mint, burn, the three redeems, resolve,
    /// deliver/mark, refund) is taken at least once.
    function test_A_ledgerHolds_theRunReachesEveryAction() public {
        for (uint256 s = 1; s <= 20; ++s) { if (s > 1) setUp(); _run(s); }
        for (uint256 i = 0; i < 11; ++i) { console2.log("action", i, "taken", hits[i]); assertGt(hits[i], 0); }
    }

    function _run(uint256 seed) internal {
        uint256 token = 1;
        address[3] memory hs = [alice, bob, mallory];
        _graduate(4.12 ether);
        for (uint256 step = 0; step < 60; ++step) {
            seed = uint256(keccak256(abi.encode(seed, step)));
            uint256 a = seed % 11;
            address h = hs[(seed >> 8) % 3];
            if (a == 0 && w.mintRoom()) {
                uint256 tickets = w.ticketCount();
                uint256 itemId = _deposit(token++, h); hits[0]++;
                // a deposit's stamp is the ticket count at its mint: no ticket burned before it can draw it
                (uint256 pid, uint256 stamp) = w.pendingAt(w.pendingCount() - 1);
                assertEq(pid, itemId);
                assertEq(stamp, tickets);
            } else if (a == 1 && module.transitUnits() == 0 && module.maxItemCost() != 0) {
                uint256 amt = module.maxItemCost();
                vm.prank(custodian); module.advance(amt); hits[1]++;
            } else if (a == 2 && module.transitUnits() != 0 && w.mintRoom()) {
                _mintPurchase(token++, address(module));
                uint256 cost = module.advanced() * 97 / 100;
                vm.prank(custodian); module.settleAdvance(cost); hits[2]++;
            } else if (a == 3 && module.transitUnits() != 0 && (seed >> 16) % 4 == 0) {
                uint256 adv = module.advanced();
                vm.deal(custodian, adv); vm.prank(custodian); module.returnAdvance{value: adv * 99 / 100}(); hits[3]++;
            } else if (a == 4) {
                (, uint256 maxIn) = module.mintCapacity();
                if (maxIn > 0.1 ether) { vm.deal(h, 0.1 ether); vm.prank(h); module.mint{value: 0.1 ether}(h, 0); hits[4]++; }
            } else if (a == 5) {
                (uint256 p, uint256 mu) = module.burnCapacity();
                uint256 u = w.balanceOf(h) < mu ? w.balanceOf(h) : mu;
                if (u > 0 && p > 0 && u * p / ONE > 0) { vm.startPrank(h); w.approve(address(module), u); module.burn(u, 0, h); vm.stopPrank(); hits[5]++; }
            } else if (a == 6 && w.balanceOf(h) >= ONE && w.held() >= w.openTickets() + 1) {
                uint256 k = (seed >> 24) % 3;
                vm.prank(h);
                if (k == 0) w.redeem(h); else if (k == 1) w.redeemToBurn(); else { w.redeemToRelist(); hits[6]++; }
            } else if (a == 7 && w.nextToResolve() < w.ticketCount()) {
                _skip(2 minutes); _resolve(w.nextToResolve()); hits[7]++;
            } else if (a == 8) {
                uint256 n = w.ticketCount();
                for (uint256 i = 0; i < n; ++i) {
                    if (w.ticket(i).stage == EthNftWrapper.Stage.Resolved) {
                        bool deliver = (seed >> 32) % 2 == 0 && w.ticket(i).destination != custodian;
                        vm.prank(custodian);
                        if (deliver) w.recordDelivery(i, keccak256(abi.encode(i)));
                        else w.markUndeliverable(i);
                        hits[8]++;
                        break;
                    }
                }
            } else if (a == 9) {
                _skip(11 days);
                _post(F);
                uint256 n = w.ticketCount();
                for (uint256 i = 0; i < n; ++i) {
                    EthNftWrapper.Stage s = w.ticket(i).stage;
                    if ((s == EthNftWrapper.Stage.Resolved && w.undeliverable(i)) || s == EthNftWrapper.Stage.Pinned) { w.refund(i); hits[9]++; }
                }
            } else {
                _skip(1 hours); _post(F); hits[10]++;
            }
            _books();
            assertGe(w.held(), w.openTickets(), "every open ticket has an item");
        }
    }
}
