pragma solidity ^0.8.20;

library Math {
    function min(uint256 a, uint256 b) internal pure returns (uint256) {
        return a < b ? a : b;
    }
}

contract Staking {
    uint256 private constant RATE = 100;
    mapping(address => uint256) private balances;

    function stake() external payable {
        require(msg.value > 0, "empty");
        balances[msg.sender] += msg.value;
    }

    function unstake(uint256 amount) external {
        uint256 held = balances[msg.sender];
        if (amount > held) {
            revert("insufficient");
        }
        balances[msg.sender] = held - amount;
        payable(msg.sender).transfer(amount);
    }
}
