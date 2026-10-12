// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Script, console2} from "forge-std/Script.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {Strings} from "@openzeppelin/contracts/utils/Strings.sol";
import {SweepVault} from "../src/SweepVault.sol";
import {FrogCurve} from "../src/curve/FrogCurve.sol";
import {CurveFactory} from "../src/curve/CurveFactory.sol";
import {EtchRouter} from "../src/pools/EtchRouter.sol";
import {FloorModule} from "../src/floor/FloorModule.sol";
import {Wrapper} from "../src/Wrapper.sol";
import {DeployChecks} from "./DeployChecks.sol";

/// @title StackRehearsal
/// @notice docs/v6/deploy-stack.md §7. What scripts/golive-dryrun.sh drives on a LOCAL fork after script/deploy-stack.sh, from
/// the stack file: one launch → its graduation through the floor module into the real PoolManager → a public mint at the module →
/// a sweep at the router → a raffle draw of one whole wX, in four phases separated by the shell's time moves and the waits for
/// the drand rounds: the draw is announced (a seed round pinned), opened with the seed round's real signature (which fixes the
/// snapshot time, committed in the root's sentinel leaf), pinned, then resolved and claimed (docs/audit/2026-09-26/
/// fixes-B-C-keeper-draws.md). Refused on any chain that is not ours (31337 or the staging chain id): a rehearsal, never a deployment.
///
/// Phase `advance` (docs/v6/relay-conversion.md §2), run after `launch`: the custodian takes one unit's item budget before its item
/// (`advance(maxItemCost())`), the unit is in transit, then the ETH comes back (`returnAdvance`) and the module is as it was.
///
///   REHEARSAL_PHASE=launch|advance|open|draw|resolve [DRAND_SEED_SIGNATURE=0x…] [DRAND_SIGNATURE=0x…64 bytes] forge script …
/// Every sender is an anvil account or an impersonated code-less address; nothing here reads a key.
contract StackRehearsal is Script {
    uint256 constant FLOOR = 0.60 ether;     // the keeper's floor at the module
    uint256 constant CAP = 0.66 ether;       // the keeper's cap at the vaults: the all-in price a sweep may pay (floor + conversion)
    uint256 constant TICKETS = 1_000;

    // code-less addresses (the fork's dev accounts carry EIP-7702 code, and a delegated account may refuse plain ETH)
    address constant CREATOR = 0x00000000000000000000000000000000C0FFEE01;
    address constant BUYER = 0x00000000000000000000000000000000C0ffee02;
    address constant TRADER = 0x00000000000000000000000000000000C0FfEE03;

    error NotOurs(uint256 chainId);

    struct Stack { address wrapper; address module; address factory; address router; address raffle; address keeper; }

    function _stack() internal view returns (Stack memory s, string memory file) {
        file = string.concat("deployments/stack-", Strings.toString(block.chainid), ".json");
        string memory j = vm.readFile(file);
        s.wrapper = vm.parseJsonAddress(j, ".collections[0].wrapper");
        s.module = vm.parseJsonAddress(j, ".collections[0].floorModule");
        s.factory = vm.parseJsonAddress(j, ".collections[0].factory");
        s.router = vm.parseJsonAddress(j, ".collections[0].router");
        s.raffle = vm.parseJsonAddress(j, ".collections[0].raffleVault");
        s.keeper = vm.parseJsonAddress(j, ".keeper");
    }

    function _stateFile() internal view returns (string memory) {
        return string.concat("deployments/rehearsal-", Strings.toString(block.chainid), ".json");
    }

    function run() external {
        if (!DeployChecks.isOurs(block.chainid)) revert NotOurs(block.chainid);
        bytes32 phase = keccak256(bytes(vm.envString("REHEARSAL_PHASE")));
        if (phase == keccak256("launch")) _launch();
        else if (phase == keccak256("advance")) _advance();
        else if (phase == keccak256("open")) _open();
        else if (phase == keccak256("draw")) _draw();
        else if (phase == keccak256("resolve")) _resolve();
        else revert("REHEARSAL_PHASE must be launch, advance, open, draw or resolve");
    }

    // ---- phase 1: launch → graduation → module mint → sweep → the draw opened -------------------------------------------------

    function _launch() internal {
        (Stack memory s,) = _stack();
        FloorModule module = FloorModule(payable(s.module));
        CurveFactory factory = CurveFactory(s.factory);
        SweepVault raffle = SweepVault(payable(s.raffle));
        EtchRouter router = EtchRouter(payable(s.router));

        // the keeper: the floor at the module, the cap at the vault (the factory serves the freshest of the vaults' caps)
        vm.startBroadcast(s.keeper);
        module.postFloor(FLOOR, 10_000, 50);
        raffle.postCap(CAP);
        vm.stopBroadcast();
        require(module.fresh(), "the module is not fresh after the post");
        (uint256 cap, bool fresh) = factory.freshCap();
        require(fresh && cap == CAP, "the factory does not serve the cap");

        // the creator launches a RAFFLE coin at the 1% tier
        vm.startBroadcast(CREATOR);
        (address coin, address curveAddr) = factory.launch("Rook Rehearsal", "REH", SweepVault.Policy.RAFFLE, false, 100);
        vm.stopBroadcast();
        FrogCurve curve = FrogCurve(payable(curveAddr));
        require(factory.feeOf(coin) == 100 && factory.policyOf(coin) == SweepVault.Policy.RAFFLE, "launch read-back");

        // the first buy, seconds after launch: it pays the decaying launch fee, all of it to the vault
        uint256 extra = curve.launchExtraBps();
        require(extra > 4_000, "the launch fee should still be near its opening 50%");
        vm.startBroadcast(BUYER);
        curve.buy{value: 3 ether}(0);
        uint256 raise = curve.raise();
        curve.buy{value: raise * 2}(0);                 // fills the raise; the change comes back in the same transaction
        vm.stopBroadcast();
        require(curve.state() == FrogCurve.State.Graduating, "the raise was not reached");
        require(module.isFilled(coin), "the module did not fill the graduation at once (stale floor, or no room)");

        // graduation: the item budget was converted through the module; the lock seeds both pools in the real PoolManager
        vm.startBroadcast(BUYER);
        curve.graduate();
        vm.stopBroadcast();
        require(curve.state() == FrogCurve.State.Graduated, "not graduated");
        uint256 units = module.ethBackedUnits();
        require(units > 1e18, "no ETH-backed units after the graduation");

        // a public mint at the module: what the quote said is what came out
        uint256 quoted = module.quoteMint(0.2 ether);
        vm.startBroadcast(TRADER);
        uint256 minted = module.mint{value: 0.2 ether}(TRADER, 0);
        vm.stopBroadcast();
        require(minted == quoted && minted > 0, "the mint did not match its quote");
        require(module.ethBackedUnits() == units + minted, "the wrapper's ethBacked did not move by the mint");

        // the sweep: the coin's vault share of the launch fee, spent at the router for at least one whole wX under the cap
        vm.startBroadcast(TRADER);
        curve.flushVaultFees();
        uint256 credit = raffle.credit(coin);
        require(credit >= 0.7 ether, "the launch fee did not reach the vault");
        uint256 spend = credit < 0.8 ether ? credit : 0.8 ether;
        bytes memory route = abi.encodeCall(EtchRouter.buyWrapped, (1, address(raffle), block.timestamp + 1 hours));
        (uint256 spent, uint256 bought) = raffle.sweep(coin, spend, route, 1e18);
        vm.stopBroadcast();
        require(bought >= 1e18 && spent <= spend, "the sweep did not buy a whole wX");
        require(raffle.units(coin) >= 1e18, "the vault does not credit the coin a whole unit");

        // the keeper announces a draw of one whole wX among the coin's holders: the seed round that will pick the snapshot time
        vm.startBroadcast(s.keeper);
        uint256 id = raffle.announceDraw(coin);
        vm.stopBroadcast();
        require(raffle.draw(id).stage == SweepVault.Stage.Announced, "the draw is not announced");

        string memory k = "rehearsal";
        vm.serializeAddress(k, "coin", coin);
        vm.serializeAddress(k, "curve", curveAddr);
        vm.serializeUint(k, "launchExtraBps", extra);
        vm.serializeUint(k, "gradUnits", units);
        vm.serializeUint(k, "minted", minted);
        vm.serializeUint(k, "swept", spent);
        vm.serializeUint(k, "bought", bought);
        vm.serializeUint(k, "drawId", id);
        vm.serializeUint(k, "seedRound", raffle.draw(id).seedRound);
        vm.serializeUint(k, "tickets", TICKETS);
        vm.serializeAddress(k, "raffle", address(raffle));
        vm.serializeAddress(k, "wrapper", s.wrapper);
        vm.serializeAddress(k, "router", address(router));
        string memory json = vm.serializeAddress(k, "winner", BUYER);
        vm.writeJson(json, _stateFile());
        console2.log("coin", coin, "curve", curveAddr);
        console2.log("launch fee at the first buy (bps)", extra);
        console2.log("ETH-backed units after graduation", units);
        console2.log("minted at the module", minted, "quoted", quoted);
        console2.log("swept", spent, "bought", bought);
        console2.log("draw", id, "announced; seed round", raffle.draw(id).seedRound);
    }

    // ---- phase 1b: the advance (docs/v6/relay-conversion.md §2): the graduation's own ETH leaves before its item, then returns ----

    function _owed(FloorModule m) internal view returns (uint256) {
        return m.backing() + m.reserve() + m.surplus() + m.revenue() + m.gradEth() + m.rentalEscrow() + m.renterPool() + m.owedTotal();
    }

    function _advance() internal {
        (Stack memory s,) = _stack();
        FloorModule module = FloorModule(payable(s.module));
        address custodian = Wrapper(s.wrapper).custodian();
        address payee = module.payee();
        uint256 amount = module.maxItemCost();
        require(amount > 0, "no item budget to advance: the graduation left no whole convertible unit, or the floor is stale");
        uint256 net0 = module.netUnits();
        uint256 bal0 = address(module).balance;
        uint256 backing0 = module.backing();
        uint256 payee0 = payee.balance;
        uint256 e0 = module.ethBackedUnits();

        vm.startBroadcast(custodian);
        module.advance(amount);
        vm.stopBroadcast();
        require(module.transitUnits() == 1e18 && module.advanced() == amount, "the unit is not in transit");
        require(module.netUnits() == net0 - 1e18, "the transit unit is still counted convertible");
        require(module.ethBackedUnits() == e0, "the wrapper's ethBacked moved");
        require(address(module).balance == bal0 - amount, "the module did not pay exactly the advance");
        require(payee == custodian ? payee.balance + 0.01 ether >= payee0 + amount : payee.balance == payee0 + amount, "the payee was not paid");
        require(address(module).balance >= _owed(module), "the books do not hold with a unit in transit");
        console2.log("advanced", amount, "to the payee", payee);
        console2.log("in transit (units)", module.transitUnits(), "convertible", module.netUnits());

        vm.startBroadcast(custodian);
        module.returnAdvance{value: amount}();
        vm.stopBroadcast();
        require(module.transitUnits() == 0 && module.advanced() == 0, "the advance is still open");
        require(module.netUnits() == net0, "the unit is not convertible again");
        require(module.backing() == backing0, "the unit's backing is not whole again");
        require(address(module).balance == bal0, "the module does not hold what it held");
        require(address(module).balance >= _owed(module), "the books do not hold after the return");
        console2.log("returned", amount, "; in transit (units)", module.transitUnits());
    }

    // ---- phase 2: the seed round exists: prove it, which fixes the snapshot time, and open with the sentinel in the root --------

    function _open() internal {
        (Stack memory s,) = _stack();
        SweepVault raffle = SweepVault(payable(s.raffle));
        string memory j = vm.readFile(_stateFile());
        uint256 id = vm.parseJsonUint(j, ".drawId");
        SweepVault.Draw memory d = raffle.draw(id);
        bytes32 seed = raffle.beacon().randomnessOf(d.seedRound);
        if (seed == bytes32(0)) {
            vm.startBroadcast(TRADER);                       // anyone may prove a round; the beacon verifies the BN254 signature
            seed = raffle.beacon().prove(d.seedRound, vm.envBytes("DRAND_SEED_SIGNATURE"));
            vm.stopBroadcast();
        }
        uint64 at = raffle.snapshotTimeOf(id, seed);
        // the list: one leaf, the buyer holds every ticket; plus the sentinel committing the count and the snapshot time
        bytes32 holder = raffle.leaf(BUYER, 0, TICKETS);
        bytes32 sentinel = raffle.leaf(address(0), TICKETS, at);
        bytes32 root = holder < sentinel ? keccak256(abi.encodePacked(holder, sentinel)) : keccak256(abi.encodePacked(sentinel, holder));
        bytes32[] memory proof = new bytes32[](1);
        proof[0] = holder;
        vm.startBroadcast(s.keeper);
        raffle.openDraw(id, root, TICKETS, "rehearsal://one-holder", proof, "");
        vm.stopBroadcast();
        require(raffle.draw(id).stage == SweepVault.Stage.Open && raffle.draw(id).snapshotAt == at, "the draw is not open");
        string memory k = "open";
        vm.serializeUint(k, "drawId", id);
        vm.serializeUint(k, "snapshotAt", at);
        string memory json = vm.serializeBytes32(k, "sentinel", sentinel);
        vm.writeJson(json, string.concat("deployments/rehearsal-open-", Strings.toString(block.chainid), ".json"));
        console2.log("draw", id, "opened; snapshot time", at);
    }

    // ---- phase 3: after 15 minutes on both clocks, pin a drand round ------------------------------------------------------------

    function _draw() internal {
        (Stack memory s,) = _stack();
        SweepVault raffle = SweepVault(payable(s.raffle));
        string memory j = vm.readFile(_stateFile());
        uint256 id = vm.parseJsonUint(j, ".drawId");
        vm.startBroadcast(TRADER);
        raffle.requestDraw(id);
        vm.stopBroadcast();
        SweepVault.Draw memory d = raffle.draw(id);
        require(d.stage == SweepVault.Stage.Pinned && d.round != 0, "not pinned");
        string memory k = "pin";
        vm.serializeUint(k, "drawId", id);
        string memory json = vm.serializeUint(k, "round", d.round);
        vm.writeJson(json, string.concat("deployments/rehearsal-pin-", Strings.toString(block.chainid), ".json"));
        console2.log("draw", id, "pinned to drand evmnet round", d.round);
    }

    // ---- phase 4: the round exists: resolve with its real signature, claim, and check the wX moved ---------------------------------

    function _resolve() internal {
        (Stack memory s,) = _stack();
        SweepVault raffle = SweepVault(payable(s.raffle));
        IERC20 w = IERC20(s.wrapper);
        string memory j = vm.readFile(_stateFile());
        uint256 id = vm.parseJsonUint(j, ".drawId");
        uint256 tickets = vm.parseJsonUint(j, ".tickets");
        bytes memory sig = vm.envBytes("DRAND_SIGNATURE");
        uint256 before = w.balanceOf(BUYER);
        vm.startBroadcast(TRADER);
        raffle.resolve(id, sig);                        // the beacon verifies the BN254 signature; a wrong one reverts here
        bytes32[] memory proof = new bytes32[](1);
        proof[0] = vm.parseJsonBytes32(vm.readFile(string.concat("deployments/rehearsal-open-", Strings.toString(block.chainid), ".json")), ".sentinel");
        raffle.claim(id, BUYER, 0, tickets, proof);
        vm.stopBroadcast();
        SweepVault.Draw memory d = raffle.draw(id);
        require(d.stage == SweepVault.Stage.Claimed && d.winner == BUYER, "not claimed by the holder");
        require(w.balanceOf(BUYER) == before + 1e18, "one whole wX did not reach the winner");
        console2.log("draw", id, "resolved: winning ticket", d.winningTicket);
        console2.log("one whole wX to the winner", BUYER);
    }
}
