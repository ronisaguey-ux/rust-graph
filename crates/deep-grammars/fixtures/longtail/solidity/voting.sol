pragma solidity ^0.8.20;

interface IVoter {
    function vote(uint256 proposal) external returns (bool);
}

abstract contract Base {
    enum Phase { Open, Closed }

    Phase public phase;

    function close() internal virtual {
        phase = Phase.Closed;
    }
}

contract Voting is Base, IVoter {
    struct Proposal {
        uint256 id;
        uint256 votes;
    }

    Proposal[] public proposals;
    mapping(address => bool) public hasVoted;

    function vote(uint256 proposal) external returns (bool) {
        require(!hasVoted[msg.sender], "already voted");
        hasVoted[msg.sender] = true;
        proposals[proposal].votes += 1;
        return true;
    }
}
