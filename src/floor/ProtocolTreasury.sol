// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Math} from "@openzeppelin/contracts/utils/math/Math.sol";
import {ReentrancyGuard} from "@openzeppelin/contracts/utils/ReentrancyGuard.sol";
import {ListingRegistry} from "../ListingRegistry.sol";
import {FloorModule} from "./FloorModule.sol";

/// @title ProtocolTreasury
/// @notice Where every protocol income stream lands (docs/v5/floor-module.md §13.3): the platform's trade fee and the capped
/// graduation fee (every curve's `platform` is this contract), the modules' spread revenue (`skim`), the protocol liquidity's
/// LP fees, and whatever else is sent here. It lets ETH out four ways only, by percentages that sum to 100% and that only the
/// registry's owner (the Safe) may change, at once: into registered collections' reserves and item floats, weighted by
/// their trailing seven-day volume; into bootstrap loans, one per new collection, repaid by that collection's fee flow before
/// any other route; and to an operations address the owner names. It starts at Phase 1's split (90% reserves, 10% operations,
/// no floats, no loans; docs/v6/phase1-scope.md §1); stage 1 restores 40/30/20/10 and a 2 ETH loan by `setShares` and
/// `setBootstrapLoan`. There is no withdraw and no pause.
/// @dev What this does NOT protect against: THE OWNER TAKING EVERYTHING HERE AT ONCE (C-06). Every owner setter is instant, and
/// `addCollection` accepts any contract that names this treasury, so the owner can register its own "module", point every share
/// and the operations address at itself, and `allocate`, `distribute` and `bootstrap` the whole balance — the pots already
/// allocated included — in one sitting. The Trust page says so. Also not: a registered module whose `fundReserve` reverts (its
/// share stays in the pot; the others are paid, C-12); ETH forced in without a call (it is income like any other). Loans:
/// `repay` refuses more than is outstanding, so a loan is never repaid twice; nothing but `forgive` (the owner, an event) lowers
/// what is owed except a repayment.
contract ProtocolTreasury is ReentrancyGuard {
    struct Loan {
        uint256 principal;
        uint256 repaid;
        bool forgiven;
    }

    uint256 public constant BPS = 10_000;

    ListingRegistry public immutable registry;

    // ---- the shares: sum to BPS at every step. Phase 1's from construction (C-07): nothing to forget at go-live ----
    uint16 public reservesBps = 9_000;
    uint16 public floatsBps = 0;
    uint16 public bootstrapBps = 0;
    uint16 public opsBps = 1_000;
    /// @notice Operations, paid its share at every allocation; unset, that share joins the reserves pot.
    address public ops;
    /// @notice What one bootstrap loan is, at most (the pot may hold less). Zero in Phase 1.
    uint256 public bootstrapLoan = 0;

    // ---- the pots: allocated and not yet delivered; balance ≥ their sum ----
    uint256 public reservesPot;
    uint256 public floatsPot;
    uint256 public bootstrapPot;

    uint256 public totalReceived;
    uint256 public totalAllocated;
    uint256 public totalToOps;
    uint256 public totalToReserves;
    uint256 public totalToFloats;
    uint256 public totalLent;
    uint256 public totalRepaid;

    FloorModule[] private _collections;
    mapping(address module => bool) public isCollection;
    mapping(address module => Loan) private _loans;

    event Received(address indexed from, uint256 amount);
    event SharesSet(uint16 reservesBps, uint16 floatsBps, uint16 bootstrapBps, uint16 opsBps);
    event OpsSet(address ops);
    event BootstrapLoanSet(uint256 amount);
    event CollectionAdded(address indexed module);
    event Allocated(uint256 amount, uint256 toReserves, uint256 toFloats, uint256 toBootstrap, uint256 toOps);
    event ReserveDelivered(address indexed module, uint256 amount);
    event FloatDelivered(address indexed module, uint256 amount);
    event FloatDeferred(address indexed module, uint256 amount);
    event ReserveDeferred(address indexed module, uint256 amount);
    event Bootstrapped(address indexed module, uint256 principal, uint256 toReserve, uint256 toFloat);
    event Repaid(address indexed module, uint256 amount, uint256 outstanding);
    event Forgiven(address indexed module, uint256 outstanding);

    error NotOwner();
    error NotCollection();
    error AlreadyCollection();
    error BadShares();
    error BadInput();
    error NothingToDo();
    error AlreadyBootstrapped();
    error OverRepaid(uint256 amount, uint256 outstanding);
    error TransferFailed();

    constructor(ListingRegistry registry_) {
        if (address(registry_) == address(0)) revert BadInput();
        registry = registry_;
    }

    modifier onlyOwner() { if (msg.sender != registry.owner()) revert NotOwner(); _; }

    receive() external payable {
        totalReceived += msg.value;
        emit Received(msg.sender, msg.value);
    }

    // ---- the timelock ------------------------------------------------------------------------------------------------------

    /// @notice The four shares, which must sum to 100%. A pot whose share goes to zero is moved into the reserves pot, so a route
    /// switched off is never paid from income that arrived before the switch (C-07).
    function setShares(uint16 reserves_, uint16 floats_, uint16 bootstrap_, uint16 ops_) external onlyOwner {
        if (uint256(reserves_) + floats_ + bootstrap_ + ops_ != BPS) revert BadShares();
        if (floats_ == 0 && floatsPot != 0) { reservesPot += floatsPot; floatsPot = 0; }
        if (bootstrap_ == 0 && bootstrapPot != 0) { reservesPot += bootstrapPot; bootstrapPot = 0; }
        reservesBps = reserves_;
        floatsBps = floats_;
        bootstrapBps = bootstrap_;
        opsBps = ops_;
        emit SharesSet(reserves_, floats_, bootstrap_, ops_);
    }

    function setOps(address ops_) external onlyOwner {
        ops = ops_;
        emit OpsSet(ops_);
    }

    function setBootstrapLoan(uint256 amount) external onlyOwner {
        bootstrapLoan = amount;
        emit BootstrapLoanSet(amount);
    }

    /// @notice A collection's floor module, which must name this treasury as its own.
    function addCollection(FloorModule module) external onlyOwner {
        if (isCollection[address(module)]) revert AlreadyCollection();
        if (module.treasury() != address(this)) revert BadInput();
        isCollection[address(module)] = true;
        _collections.push(module);
        emit CollectionAdded(address(module));
    }

    /// @notice Forgive what a collection still owes of its bootstrap loan. The one way a loan falls without a repayment.
    function forgive(address module) external onlyOwner {
        Loan storage l = _loans[module];
        uint256 due = _outstanding(l);
        if (due == 0) revert NothingToDo();
        l.forgiven = true;
        emit Forgiven(module, due);
    }

    // ---- anyone -----------------------------------------------------------------------------------------------------------

    /// @notice ETH that arrived since the last allocation, not yet in a pot.
    function unallocated() public view returns (uint256) {
        return address(this).balance - reservesPot - floatsPot - bootstrapPot;
    }

    /// @notice Move everything that arrived since the last call into the pots by the shares, and pay operations its share.
    function allocate() external nonReentrant {
        uint256 amount = unallocated();
        if (amount == 0) revert NothingToDo();
        uint256 toReserves = (amount * reservesBps) / BPS;
        uint256 toFloats = (amount * floatsBps) / BPS;
        uint256 toBootstrap = (amount * bootstrapBps) / BPS;
        uint256 toOps = amount - toReserves - toFloats - toBootstrap;
        address o = ops;
        if (o == address(0)) { toReserves += toOps; toOps = 0; }
        reservesPot += toReserves;
        floatsPot += toFloats;
        bootstrapPot += toBootstrap;
        totalAllocated += amount;
        totalToOps += toOps;
        emit Allocated(amount, toReserves, toFloats, toBootstrap, toOps);
        _send(o, toOps);
    }

    /// @notice Pay the reserves pot and the floats pot out to the registered collections, each by its share of the trailing
    /// seven-day volume. A float a module cannot take now (a stale floor, no room) stays in the pot for the next call.
    function distribute() external nonReentrant {
        uint256 n = _collections.length;
        uint256[] memory w = new uint256[](n);
        uint256 total;
        for (uint256 i = 0; i < n; ++i) {
            w[i] = _collections[i].volume7d();
            total += w[i];
        }
        if (total == 0) revert NothingToDo();
        uint256 rPot = reservesPot;
        uint256 fPot = floatsPot;
        for (uint256 i = 0; i < n; ++i) {
            if (w[i] == 0) continue;
            FloorModule m = _collections[i];
            uint256 r = Math.mulDiv(rPot, w[i], total);
            if (r != 0) {
                reservesPot -= r;
                // C-12: a module that refuses keeps its share in the pot; the others are still paid
                try m.fundReserve{value: r}() {
                    totalToReserves += r;
                    emit ReserveDelivered(address(m), r);
                } catch {
                    reservesPot += r;
                    emit ReserveDeferred(address(m), r);
                }
            }
            uint256 f = Math.mulDiv(fPot, w[i], total);
            if (f != 0) {
                floatsPot -= f;
                try m.fundFloat{value: f}() returns (uint256) {
                    totalToFloats += f;
                    emit FloatDelivered(address(m), f);
                } catch {
                    floatsPot += f;
                    emit FloatDeferred(address(m), f);
                }
            }
        }
    }

    /// @notice A registered collection's one bootstrap loan, from the bootstrap pot: `bootstrapLoan` or what the pot holds, half
    /// to a float (all to the reserve while its floor is stale), half to its reserve. Repaid by the module's fee flow first.
    function bootstrap(FloorModule module) external nonReentrant {
        if (!isCollection[address(module)]) revert NotCollection();
        Loan storage l = _loans[address(module)];
        if (l.principal != 0) revert AlreadyBootstrapped();
        uint256 amount = Math.min(bootstrapLoan, bootstrapPot);
        if (amount == 0) revert NothingToDo();
        bootstrapPot -= amount;
        uint256 toFloat = amount / 2;
        uint256 toReserve = amount - toFloat;
        if (toFloat != 0) {
            try module.fundFloat{value: toFloat}() returns (uint256) {} catch { toReserve += toFloat; toFloat = 0; }
        }
        module.fundReserve{value: toReserve}();                 // before the loan is on the books: nothing to repay yet
        l.principal = amount;
        totalLent += amount;
        emit Bootstrapped(address(module), amount, toReserve, toFloat);
    }

    /// @notice A registered collection's module, repaying its loan: never more than is outstanding. The ETH is income again.
    /// @dev Not `nonReentrant`: a module funded by `distribute` repays from that very funding, inside the same call (its loan
    /// comes before any other route); this function reads and writes its own loan and takes `msg.value`, and calls nothing.
    function repay() external payable {
        if (!isCollection[msg.sender]) revert NotCollection();
        Loan storage l = _loans[msg.sender];
        uint256 due = _outstanding(l);
        if (msg.value == 0 || msg.value > due) revert OverRepaid(msg.value, due);
        l.repaid += msg.value;
        totalRepaid += msg.value;
        totalReceived += msg.value;
        emit Repaid(msg.sender, msg.value, due - msg.value);
    }

    // ---- views ------------------------------------------------------------------------------------------------------------

    function outstanding(address module) external view returns (uint256) { return _outstanding(_loans[module]); }
    function loan(address module) external view returns (Loan memory) { return _loans[module]; }
    function collectionCount() external view returns (uint256) { return _collections.length; }
    function collection(uint256 i) external view returns (FloorModule) { return _collections[i]; }

    /// @notice Each registered collection's weight now (its trailing seven-day volume) and their sum.
    function weights() external view returns (uint256[] memory w, uint256 total) {
        uint256 n = _collections.length;
        w = new uint256[](n);
        for (uint256 i = 0; i < n; ++i) {
            w[i] = _collections[i].volume7d();
            total += w[i];
        }
    }

    function _outstanding(Loan storage l) private view returns (uint256) {
        return l.forgiven ? 0 : l.principal - l.repaid;
    }

    function _send(address to, uint256 amount) private {
        if (amount == 0) return;
        (bool ok,) = to.call{value: amount}("");
        if (!ok) revert TransferFailed();
    }
}
