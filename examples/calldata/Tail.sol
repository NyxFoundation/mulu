// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/// A guard over the argument, beside a dynamic calldata array.
///
/// solc puts its own checks around the array: the tail of a dynamic array
/// has to lie inside the call, and an index has to be below the length that
/// came in with it. Neither is a function of the abstract argument P1a
/// carries, and refusing the contract for them threw away the guard that
/// *is* one. They are undecided, and the walk takes them both ways.
contract Tail {
    function keep(uint256 x, uint256[] calldata a) external pure returns (uint256) {
        require(x > 10, "small");
        a;
        return x;
    }

    /// Indexing can revert on its own: nothing says the array is not empty.
    function first(uint256 x, uint256[] calldata a) external pure returns (uint256) {
        require(x > 10, "small");
        return a[0];
    }
}
