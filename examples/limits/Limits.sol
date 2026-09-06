// Design example from docs/08 §3. Not compiled by mulu yet (P1): the finite
// model in model.json was written by hand from this source.
pragma solidity ^0.8.0;

contract Limits {
    uint256 public limit;

    function setLimit(uint256 x) external {
        require(x <= 100, "cap");       // A
        require(x <= 1000, "bound");    // B
        limit = x;
    }

    function forceSet(uint256 x) external {
        limit = x;
    }
}
