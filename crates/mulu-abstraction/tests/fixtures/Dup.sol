pragma solidity ^0.8.0;
contract Dup {
    uint256 public limit;
    function f(uint256 x) external {
        require(x < 10);
        require(x > 20);
        limit = x;
    }
}
