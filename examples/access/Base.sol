// Design example for modifiers and imports (P1-01 / P1-02).
// The guard lives in a modifier declared here and is applied in Vault.sol,
// so the check's source span points into this file while the function that
// carries it is in another.
pragma solidity ^0.8.0;

abstract contract Bounded {
    modifier capped(uint256 x) {
        require(x <= 100, "cap");
        _;
    }
}
