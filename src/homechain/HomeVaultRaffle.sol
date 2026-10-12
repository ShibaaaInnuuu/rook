// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {MerkleProof} from "@openzeppelin/contracts/utils/cryptography/MerkleProof.sol";
import {Wait} from "../lib/Wait.sol";
import {HomePolicy} from "./IHomeChain.sol";
import {HomeVaultBase} from "./HomeVaultBase.sol";

/// @title HomeVaultRaffle
/// @notice A home-chain vault whose every purchase is raffled to the holders of the coin that paid for it: `NftVaultBase`'s draw
/// (a keeper snapshot, a 15-minute challenge the owner may cancel, a drand round pinned before it exists, a proof of the winning
/// ticket range) with the NFT handed to the proven winner IN THE CLAIM, on this chain. docs/v6/home-chain-vault.md §3.3.
/// @dev The keeper reads which coin a ticket was for on 4663 (`FeeEscrow.coinOf`) and snapshots that coin's holders there; the
/// winner's address is the same 20 bytes here. The snapshot should exclude contracts, which have no twin on this chain.
contract HomeVaultRaffle is HomeVaultBase {
    uint256 public constant CHALLENGE = 15 minutes;
    uint64 public constant PIN_AHEAD = 20;
    uint256 public constant REDRAW_AFTER = 24 hours;
    uint256 public constant VOID_AFTER = 7 days;

    struct Draw {
        uint64 challengeEnds;
        uint64 challengeEndsBlock;
        uint64 round;
        address winner;
        bytes32 root;
        uint256 totalTickets;
        uint256 winningTicket;
        string uri;
    }
    mapping(uint256 acquisitionId => Draw) private _draws;

    event DrawOpened(uint256 indexed acquisitionId, bytes32 root, uint256 totalTickets, string uri, uint64 challengeEnds);
    event DrawCancelled(uint256 indexed acquisitionId);
    event DrawPinned(uint256 indexed acquisitionId, uint64 round);
    event DrawResolved(uint256 indexed acquisitionId, uint256 winningTicket);
    event DrawVoided(uint256 indexed acquisitionId, uint256 unclaimedTicket);
    event Won(uint256 indexed acquisitionId, address indexed winner);

    error BadProof();
    error ChallengeWindow(uint64 until);
    error WindowOver();

    constructor(Config memory c) HomeVaultBase(c, HomePolicy.Raffle) {}

    function openDraw(uint256 acquisitionId, bytes32 root, uint256 totalTickets, string calldata uri) external onlyKeeper {
        Acquisition storage a = _acquisitions[acquisitionId];
        if (a.stage != Stage.Held) revert WrongStage();
        if (root == bytes32(0) || totalTickets == 0 || bytes(uri).length == 0) revert BadInput();
        Draw storage d = _draws[acquisitionId];
        _stage(a, Stage.Open);
        d.root = root;
        d.totalTickets = totalTickets;
        d.uri = uri;
        d.challengeEnds = uint64(block.timestamp + CHALLENGE);
        d.challengeEndsBlock = uint64(block.number + Wait.blocksFor(CHALLENGE, parentBlockSeconds));
        emit DrawOpened(acquisitionId, root, totalTickets, uri, d.challengeEnds);
    }

    /// @notice The owner's only lever, and only while the snapshot is under challenge.
    function cancelDraw(uint256 acquisitionId) external onlyOwner {
        Acquisition storage a = _acquisitions[acquisitionId];
        Draw storage d = _draws[acquisitionId];
        if (a.stage != Stage.Open) revert WrongStage();
        if (Wait.elapsed(d.challengeEnds, d.challengeEndsBlock)) revert WindowOver();
        _reopen(a, d);
        emit DrawCancelled(acquisitionId);
    }

    function requestDraw(uint256 acquisitionId) external {
        Acquisition storage a = _acquisitions[acquisitionId];
        Draw storage d = _draws[acquisitionId];
        if (a.stage != Stage.Open) revert WrongStage();
        if (!Wait.elapsed(d.challengeEnds, d.challengeEndsBlock)) revert ChallengeWindow(d.challengeEnds);
        _pin(acquisitionId, a, d);
    }

    function redraw(uint256 acquisitionId) external {
        Acquisition storage a = _acquisitions[acquisitionId];
        Draw storage d = _draws[acquisitionId];
        if (a.stage != Stage.Pinned) revert WrongStage();
        if (!Wait.elapsed(uint64(a.at + REDRAW_AFTER), uint64(a.atBlock + Wait.blocksFor(REDRAW_AFTER, parentBlockSeconds)))
            || beacon.randomnessOf(d.round) != bytes32(0)) revert TooEarly();
        _pin(acquisitionId, a, d);
    }

    function _pin(uint256 acquisitionId, Acquisition storage a, Draw storage d) private {
        _stage(a, Stage.Pinned);
        d.round = beacon.roundAt(block.timestamp) + PIN_AHEAD;
        emit DrawPinned(acquisitionId, d.round);
    }

    function resolve(uint256 acquisitionId, bytes calldata signature) external {
        Acquisition storage a = _acquisitions[acquisitionId];
        Draw storage d = _draws[acquisitionId];
        if (a.stage != Stage.Pinned) revert WrongStage();
        bytes32 randomness = beacon.randomnessOf(d.round);
        if (randomness == bytes32(0)) randomness = beacon.prove(d.round, signature);
        d.winningTicket = uint256(keccak256(abi.encode(randomness, address(this), acquisitionId))) % d.totalTickets;
        _stage(a, Stage.Resolved);
        emit DrawResolved(acquisitionId, d.winningTicket);
    }

    function voidDraw(uint256 acquisitionId) external onlyOwner {
        Acquisition storage a = _acquisitions[acquisitionId];
        Draw storage d = _draws[acquisitionId];
        if (a.stage != Stage.Resolved) revert WrongStage();
        if (!Wait.elapsed(uint64(a.at + VOID_AFTER), uint64(a.atBlock + Wait.blocksFor(VOID_AFTER, parentBlockSeconds)))) revert TooEarly();
        emit DrawVoided(acquisitionId, d.winningTicket);
        _reopen(a, d);
    }

    /// @notice Anyone proves whose ticket range holds the winning ticket; the NFT goes to that winner in this call, and to nobody else.
    function claimWin(uint256 acquisitionId, address winner, uint256 firstTicket, uint256 endTicket, bytes32[] calldata proof)
        external nonReentrant
    {
        Acquisition storage a = _acquisitions[acquisitionId];
        Draw storage d = _draws[acquisitionId];
        if (a.stage != Stage.Resolved) revert WrongStage();
        if (winner == address(0) || d.winningTicket < firstTicket || d.winningTicket >= endTicket) revert BadProof();
        if (!MerkleProof.verifyCalldata(proof, d.root, ticketLeaf(winner, firstTicket, endTicket))) revert BadProof();
        d.winner = winner;
        _stage(a, Stage.Delivered);
        guard.release(collection, 1, a.pricePaid);
        guard.out(collection, 1, 0);
        emit Won(acquisitionId, winner);
        _send(winner, a.tokenId);
    }

    /// @notice The same leaf as `Distributor.ticketLeaf`, so the keeper's snapshot tool serves both.
    function ticketLeaf(address holder, uint256 firstTicket, uint256 endTicket) public pure returns (bytes32) {
        return keccak256(bytes.concat(keccak256(abi.encode(holder, firstTicket, endTicket))));
    }

    function _reopen(Acquisition storage a, Draw storage d) private {
        _stage(a, Stage.Held);
        d.root = bytes32(0);
        d.totalTickets = 0;
        d.winningTicket = 0;
        d.round = 0;
        d.uri = "";
    }

    function draw(uint256 acquisitionId) external view returns (Draw memory) { return _draws[acquisitionId]; }
}
