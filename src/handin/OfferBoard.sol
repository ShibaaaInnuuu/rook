// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {ReentrancyGuard} from "@openzeppelin/contracts/utils/ReentrancyGuard.sol";
import {IWrappedCollection} from "../interfaces/IWrappedCollection.sol";
import {Wait} from "../lib/Wait.sol";

/// @title OfferBoard
/// @notice Permissionless offer hand-ins for one wrapped collection (docs/v6/offer-handins.md). Anyone commits to a
/// marketplace offer — the sha256 of a seller-signed PSBT, the sats it asks of the canister's float, the inscription — with a
/// bond attached, and delivers the bytes to the canister off chain. The canister (the wrapper's custodian) judges the bytes,
/// buys the cheapest valid open hand-in when a sink wants an item, and once the purchase is six blocks deep `claim`s: the
/// submitter is credited its bond and a fixed reward from the pool. A hand-in the canister judged invalid is `slash`ed once
/// `slashDelay` has passed on both clocks; a valid one nobody bought is `refund`ed by its submitter after `expiry`.
/// @dev The books, at all times: `address(this).balance == bonded + rewardPool + owedTotal`. A slashed bond moves to the
/// pool; a claim or refund moves the bond (and the reward) to `owed`, pulled with `withdraw` so that a submitter that refuses
/// ETH cannot make the custodian's claim revert. `claim` pays `min(reward, rewardPool, rewardDayCap − paidToday)`: the pool
/// is never overdrawn and a dry pool never blocks a claim. What this does NOT decide: whether a hand-in is valid. That is the
/// custodian's word, bounded in time by `slashDelay` (a wrong slash is visible for that long before any bond moves) and in
/// money by the bond.
contract OfferBoard is ReentrancyGuard {
    enum Status { None, Open, Claimed, Slashed, Refunded }

    struct Submission {
        address submitter;
        uint64 at;
        uint64 atBlock;
        Status status;
        uint256 bond;
        uint256 askSats;
        bytes32 offerHash;
        string inscriptionId;
    }

    IWrappedCollection public immutable wrapper;
    uint256 public immutable bond;
    uint256 public immutable reward;
    uint256 public immutable slashDelay;
    uint256 public immutable expiry;
    uint256 public immutable rewardDayCap;
    uint256 public immutable maxOpen;
    uint256 public immutable maxOpenPerSubmitter;
    uint256 public immutable parentBlockSeconds;

    Submission[] private _subs;
    /// @dev ids currently open, unordered; `_openIndex[id]` is its index plus one.
    uint256[] private _open;
    mapping(uint256 id => uint256 indexPlusOne) private _openIndex;
    /// @notice The open submission with this hash, as id + 1; 0 when none.
    mapping(bytes32 offerHash => uint256 idPlusOne) public openIdOf;
    /// @notice Open submissions per address.
    mapping(address submitter => uint256) public openBy;
    /// @notice What a submitter may `withdraw`.
    mapping(address submitter => uint256) public owed;

    /// @notice Bonds of open submissions.
    uint256 public bonded;
    /// @notice ETH for rewards: funded, plus slashed bonds, less rewards paid.
    uint256 public rewardPool;
    /// @notice Σ `owed`.
    uint256 public owedTotal;
    uint64 public dayStartAt;
    uint64 public dayStartAtBlock;
    uint256 public paidToday;
    // the books, for the invariants
    uint256 public totalFunded;
    uint256 public totalSlashed;
    uint256 public totalRewardsPaid;

    event Submitted(uint256 indexed id, address indexed submitter, bytes32 indexed offerHash, uint256 askSats, string inscriptionId);
    event Claimed(uint256 indexed id, address indexed submitter, uint256 reward);
    event Slashed(uint256 indexed id, address indexed submitter, uint256 bond);
    event Refunded(uint256 indexed id, address indexed submitter, uint256 bond);
    event Funded(address indexed from, uint256 amount);
    event Withdrawn(address indexed to, uint256 amount);

    error NotCustodian();
    error NotSubmitter();
    error BadInput();
    error WrongBond(uint256 sent, uint256 required);
    error Duplicate(uint256 openId);
    error BoardFull();
    error TooManyOpen(address submitter);
    error WrongStatus();
    error TooEarly();
    error TransferFailed();

    constructor(
        IWrappedCollection wrapper_,
        uint256 bond_,
        uint256 reward_,
        uint256 slashDelay_,
        uint256 expiry_,
        uint256 rewardDayCap_,
        uint256 maxOpen_,
        uint256 maxOpenPerSubmitter_,
        uint256 parentBlockSeconds_
    ) {
        Wait.checkBlockSeconds(parentBlockSeconds_);
        if (address(wrapper_) == address(0) || bond_ == 0 || maxOpen_ == 0 || maxOpenPerSubmitter_ == 0) revert BadInput();
        if (maxOpenPerSubmitter_ > maxOpen_) revert BadInput();
        // a slash must be possible before the submitter can take the bond back, with the canister's own latency in between
        if (slashDelay_ == 0 || expiry_ <= slashDelay_ + 1 hours) revert BadInput();
        wrapper = wrapper_;
        bond = bond_;
        reward = reward_;
        slashDelay = slashDelay_;
        expiry = expiry_;
        rewardDayCap = rewardDayCap_;
        maxOpen = maxOpen_;
        maxOpenPerSubmitter = maxOpenPerSubmitter_;
        parentBlockSeconds = parentBlockSeconds_;
        dayStartAt = uint64(block.timestamp);
        dayStartAtBlock = uint64(block.number);
    }

    /// @notice The canister: the only address that may claim or slash.
    function custodian() public view returns (address) {
        return wrapper.custodian();
    }

    modifier onlyCustodian() {
        if (msg.sender != wrapper.custodian()) revert NotCustodian();
        _;
    }

    // ---- submitters ------------------------------------------------------------------------------------------------------------

    /// @notice Commit to an offer: `offerHash = sha256(psbt)`, `askSats` what the canister's float will spend (fee included),
    /// `inscriptionId` the frog. Exactly `bond` wei attached. The bytes go to the canister's `submit_offer`.
    function submit(bytes32 offerHash, uint256 askSats, string calldata inscriptionId) external payable nonReentrant returns (uint256 id) {
        if (msg.value != bond) revert WrongBond(msg.value, bond);
        if (offerHash == bytes32(0) || askSats == 0 || bytes(inscriptionId).length == 0 || bytes(inscriptionId).length > 80) revert BadInput();
        if (openIdOf[offerHash] != 0) revert Duplicate(openIdOf[offerHash] - 1);
        if (_open.length >= maxOpen) revert BoardFull();
        if (openBy[msg.sender] >= maxOpenPerSubmitter) revert TooManyOpen(msg.sender);
        id = _subs.length;
        _subs.push(Submission(msg.sender, uint64(block.timestamp), uint64(block.number), Status.Open, msg.value, askSats, offerHash, inscriptionId));
        _open.push(id);
        _openIndex[id] = _open.length;
        openIdOf[offerHash] = id + 1;
        openBy[msg.sender] += 1;
        bonded += msg.value;
        emit Submitted(id, msg.sender, offerHash, askSats, inscriptionId);
    }

    /// @notice The submitter, once `expiry` has passed on both clocks (a valid hand-in nobody bought); or the custodian at any
    /// time (a hand-in it will never buy). The bond is credited, no reward.
    function refund(uint256 id) external nonReentrant {
        Submission storage s = _sub(id);
        if (s.status != Status.Open) revert WrongStatus();
        if (msg.sender == wrapper.custodian()) {
            // the custodian frees a slot it will never fill
        } else {
            if (msg.sender != s.submitter) revert NotSubmitter();
            if (!_expired(s)) revert TooEarly();
        }
        _close(id, s, Status.Refunded);
        _credit(s.submitter, s.bond);
        emit Refunded(id, s.submitter, s.bond);
    }

    /// @notice Pull what `claim` and `refund` credited.
    function withdraw() external nonReentrant returns (uint256 amount) {
        amount = owed[msg.sender];
        if (amount == 0) return 0;
        owed[msg.sender] = 0;
        owedTotal -= amount;
        emit Withdrawn(msg.sender, amount);
        (bool ok,) = msg.sender.call{value: amount}("");
        if (!ok) revert TransferFailed();
    }

    /// @notice Anyone: the reward pool (the collection's fees, typically).
    function fund() external payable {
        if (msg.value == 0) revert BadInput();
        rewardPool += msg.value;
        totalFunded += msg.value;
        emit Funded(msg.sender, msg.value);
    }

    // ---- the custodian ---------------------------------------------------------------------------------------------------------

    /// @notice The canister, once the purchase this hand-in described is six blocks deep in custody: credits the submitter its
    /// bond and `min(reward, rewardPool, rewardDayCap − paidToday)`.
    function claim(uint256 id) external nonReentrant onlyCustodian returns (uint256 paid) {
        Submission storage s = _sub(id);
        if (s.status != Status.Open) revert WrongStatus();
        _rollDay();
        paid = reward;
        if (paid > rewardPool) paid = rewardPool;
        uint256 left = rewardDayCap - paidToday;
        if (paid > left) paid = left;
        _close(id, s, Status.Claimed);
        rewardPool -= paid;
        paidToday += paid;
        totalRewardsPaid += paid;
        _credit(s.submitter, s.bond + paid);
        emit Claimed(id, s.submitter, paid);
    }

    /// @notice The canister, for a hand-in it judged invalid, once `slashDelay` has passed on both clocks since the submission:
    /// the bond goes to the reward pool.
    function slash(uint256 id) external nonReentrant onlyCustodian {
        Submission storage s = _sub(id);
        if (s.status != Status.Open) revert WrongStatus();
        if (!_slashable(s)) revert TooEarly();
        _close(id, s, Status.Slashed);
        rewardPool += s.bond;
        totalSlashed += s.bond;
        emit Slashed(id, s.submitter, s.bond);
    }

    // ---- views ----------------------------------------------------------------------------------------------------------------------

    function submissionOf(uint256 id) external view returns (Submission memory) {
        return _subs[id];
    }

    function submissionCount() external view returns (uint256) {
        return _subs.length;
    }

    function openCount() external view returns (uint256) {
        return _open.length;
    }

    /// @notice Up to `max` open ids from position `start` of the open list (unordered; the canister orders by `askSats`).
    function openIds(uint256 start, uint256 max) external view returns (uint256[] memory ids) {
        uint256 n = _open.length;
        if (start >= n) return ids;
        uint256 end = start + max;
        if (end > n) end = n;
        ids = new uint256[](end - start);
        for (uint256 i = start; i < end; i++) {
            ids[i - start] = _open[i];
        }
    }

    function params() external view returns (uint256, uint256, uint256, uint256, uint256, uint256) {
        return (bond, reward, slashDelay, expiry, rewardDayCap, maxOpen);
    }

    /// @notice Open, and `slashDelay` past on both clocks.
    function slashable(uint256 id) external view returns (bool) {
        Submission storage s = _subs[id];
        return s.status == Status.Open && _slashable(s);
    }

    /// @notice `expiry` past on both clocks since the submission.
    function expired(uint256 id) external view returns (bool) {
        return _expired(_subs[id]);
    }

    // ---- internals --------------------------------------------------------------------------------------------------------------------

    function _sub(uint256 id) private view returns (Submission storage s) {
        if (id >= _subs.length) revert BadInput();
        s = _subs[id];
    }

    function _slashable(Submission storage s) private view returns (bool) {
        return Wait.elapsed(s.at + uint64(slashDelay), s.atBlock + uint64(Wait.blocksFor(slashDelay, parentBlockSeconds)));
    }

    function _expired(Submission storage s) private view returns (bool) {
        return Wait.elapsed(s.at + uint64(expiry), s.atBlock + uint64(Wait.blocksFor(expiry, parentBlockSeconds)));
    }

    /// @dev Open → `to`: off the open list, the hash and the submitter's count freed, the bond no longer counted as bonded.
    function _close(uint256 id, Submission storage s, Status to) private {
        s.status = to;
        uint256 idx = _openIndex[id] - 1;
        uint256 last = _open[_open.length - 1];
        _open[idx] = last;
        _openIndex[last] = idx + 1;
        _open.pop();
        delete _openIndex[id];
        delete openIdOf[s.offerHash];
        openBy[s.submitter] -= 1;
        bonded -= s.bond;
    }

    function _credit(address to, uint256 amount) private {
        owed[to] += amount;
        owedTotal += amount;
    }

    /// @dev A new day once both clocks have passed the last day's start plus a day.
    function _rollDay() private {
        if (Wait.elapsed(dayStartAt + 1 days, dayStartAtBlock + uint64(Wait.blocksFor(1 days, parentBlockSeconds)))) {
            dayStartAt = uint64(block.timestamp);
            dayStartAtBlock = uint64(block.number);
            paidToday = 0;
        }
    }
}
