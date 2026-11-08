pragma solidity ^0.8.20;

contract Broken {
    uint256 public total;

    function bump() external {
        total += 1;
    }
