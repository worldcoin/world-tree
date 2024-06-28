// SPDX-License-Identifier: MIT
pragma solidity ^0.8.19;

contract BridgedWorldID {
    event RootAdded(uint256 root, uint128 timestamp);

    uint256 _latestRoot = 0x2134e76ac5d21aab186c2be1dd8f84ee880a1e46eaf712f9d371b6df22191f3e;

    function latestRoot() public view virtual returns (uint256) {
        return _latestRoot;
    }

    function receiveRoot(uint256 newRoot) external {
        _latestRoot = newRoot;

        emit RootAdded(newRoot, uint128(block.timestamp));
    }
}
