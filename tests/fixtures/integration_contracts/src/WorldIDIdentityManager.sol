// SPDX-License-Identifier: MIT
pragma solidity ^0.8.19;

contract WorldIDIdentityManager {
    uint256 _latestRoot = 0x2134e76ac5d21aab186c2be1dd8f84ee880a1e46eaf712f9d371b6df22191f3e;

    event TreeChanged(
        uint256 indexed preRoot,
        uint8 indexed kind,
        uint256 indexed postRoot
    );

    function latestRoot() external view returns (uint256) {
        return _latestRoot;
    }

    function registerIdentities(
        uint256[8] calldata /* insertionProof */,
        uint256 preRoot,
        uint32 /* startIndex */,
        uint256[] calldata /* identityCommitments */,
        uint256 postRoot
    ) external {
        emit TreeChanged(preRoot, 0, postRoot);
    }

    function deleteIdentities(
        uint256[8] calldata /* deletionProof */,
        bytes calldata /* packedDeletionIndices */,
        uint256 preRoot,
        uint256 postRoot
    ) external {
        emit TreeChanged(preRoot, 1, postRoot);
    }
}
