// Design example from docs/08 §3.
// `mulu ir` compiles this file and lowers solc's Yul to ProgramIR (P1-01).
// model.json next to it is the hand-written finite model used by
// `mulu analyze-model`; connecting the two is P1-02.
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
