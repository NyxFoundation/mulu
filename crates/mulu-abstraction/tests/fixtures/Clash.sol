pragma solidity ^0.8.0;
contract Clash {
    uint256 public limit;
    function f(uint256 x) external { require(x <= 100, "cap"); limit = x; }
    function f_X0() external { limit = 0; }
}
