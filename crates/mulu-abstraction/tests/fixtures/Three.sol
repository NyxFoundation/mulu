pragma solidity ^0.8.0;
contract Three {
    uint256 public limit;
    function f(uint256 x) external {
        require(x < 10, "A");
        require(x > 20, "B");
        require(x < 30, "C");
        limit = x;
    }
}
