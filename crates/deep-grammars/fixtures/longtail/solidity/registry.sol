// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {Ownable} from "@openzeppelin/contracts/access/Ownable.sol";

contract Registry is Ownable {
    struct Entry {
        address owner;
        uint256 score;
        bool active;
    }

    mapping(address => Entry) public entries;
    uint256 public total;
    event Registered(address indexed owner, uint256 score);

    error AlreadyRegistered(address owner);

    modifier onlyActive(address owner) {
        require(entries[owner].active, "inactive");
        _;
    }

    constructor() Ownable(msg.sender) {}

    function register(address owner, uint256 score) external onlyActive(owner) returns (uint256) {
        Entry storage entry = entries[owner];
        entry.owner = owner;
        entry.score = score;
        entry.active = true;
        total += 1;
        emit Registered(owner, score);
        return total;
    }

    function lookup(address owner) public view returns (Entry memory) {
        return entries[owner];
    }
}
