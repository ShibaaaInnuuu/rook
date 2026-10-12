// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {ILauncher} from "../../src/interfaces/ILauncher.sol";
import {IRandomnessBeacon} from "../../src/interfaces/IRandomnessBeacon.sol";

contract MockLauncher is ILauncher {
    mapping(address => address) public routerOf;
    mapping(address => address) public creatorOf;
    mapping(address => uint256) public listingOf;
    function set(address coin, address router, address creator, uint256 listingId) external {
        routerOf[coin] = router; creatorOf[coin] = creator; listingOf[coin] = listingId;
    }
}

/// A beacon whose rounds are 3 s apart from t = 0 and whose "signature" is the randomness itself. For the draw's own logic only;
/// the real verifier is tested against real drand beacons in DrandEvmnet.t.sol and once end to end in Distributor.t.sol.
contract MockBeacon is IRandomnessBeacon {
    mapping(uint64 => bytes32) public randomnessOf;
    function roundAt(uint256 timestamp) external pure returns (uint64) { return uint64(timestamp / 3) + 1; }
    function timeOf(uint64 round) external pure returns (uint256) { return (uint256(round) - 1) * 3; }
    function prove(uint64 round, bytes calldata signature) external returns (bytes32 r) {
        r = randomnessOf[round];
        if (r != bytes32(0)) return r;
        require(signature.length == 32, "bad mock signature");
        r = bytes32(signature);
        require(r != bytes32(0), "zero");
        randomnessOf[round] = r;
    }
}

contract MockEscrow {
    mapping(address => uint256) public balanceOf;
    function credit(address recipient) external payable { balanceOf[recipient] += msg.value; }
    function claim() external returns (uint256 amount) {
        amount = balanceOf[msg.sender];
        require(amount != 0, "NoBalance");
        balanceOf[msg.sender] = 0;
        (bool ok,) = msg.sender.call{value: amount}("");
        require(ok, "TransferFailed");
    }
}

/// A curve that holds fees until its recipient sweeps them into the escrow, as Pons's does.
contract MockCurve {
    MockEscrow public immutable escrow;
    address public recipient;
    bool public graduated;
    constructor(MockEscrow escrow_) { escrow = escrow_; }
    function setRecipient(address r) external { recipient = r; }
    function setGraduated(bool g) external { graduated = g; }
    receive() external payable {}
    function sweepFees(uint256) external {
        require(!graduated, "AlreadyGraduated");
        require(msg.sender == recipient, "NotFeeSweepOperator");
        uint256 b = address(this).balance;
        if (b != 0) escrow.credit{value: b}(recipient);
    }
}

contract RefusesEth {
    bool public open;
    function setOpen(bool o) external { open = o; }
    receive() external payable { require(open, "closed"); }
}

/// A plain ERC-20, enough for a snapshot to be taken of it. The protocol token is an ordinary coin; nothing in the giveaway path
/// reads anything from it that every coin does not have.
contract MockToken {
    string public name = "etched";
    string public symbol = "ETCHED";
    uint8 public constant decimals = 18;
    uint256 public totalSupply;
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;
    event Transfer(address indexed from, address indexed to, uint256 value);

    function mint(address to, uint256 amount) external {
        totalSupply += amount;
        balanceOf[to] += amount;
        emit Transfer(address(0), to, amount);
    }
    function transfer(address to, uint256 amount) external returns (bool) {
        balanceOf[msg.sender] -= amount;
        balanceOf[to] += amount;
        emit Transfer(msg.sender, to, amount);
        return true;
    }
}
