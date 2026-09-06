// A guard written as `if (..) revert()` rather than `require(..)`.
// solc puts it on a branch instead of a helper call, so recognising it needs
// the AST to say the author wrote it. Without that it is neither judged for
// redundancy nor parameterised out of the reference plant, and the contract
// is analysed as though it had no guard at all.
pragma solidity ^0.8.0;

contract Gate {
    uint256 public limit;

    function setLimit(uint256 x) external {
        if (x > 100) {
            revert("cap");
        }
        limit = x;
    }

    function forceSet(uint256 x) external {
        limit = x;
    }
}
