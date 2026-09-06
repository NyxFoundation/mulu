pragma solidity ^0.8.0;

import "./Base.sol";

contract Vault is Bounded {
    uint256 public limit;

    // `capped` rejects anything above 100, so the require below never fails:
    // the same relation between checks A and B as in examples/limits, but with
    // the first check written in an imported modifier.
    function setLimit(uint256 x) external capped(x) {
        require(x <= 1000, "bound");
        limit = x;
    }

    function forceSet(uint256 x) external {
        limit = x;
    }
}
