// The argument type decides the domain (P1-02 / docs/11 §5).
// `record` takes a uint8, so its argument can never exceed 255 and the bound
// on `reading` cannot be broken through it. `force` takes a uint256 and can.
// Reading that from the ABI is what tells the two apart.
pragma solidity ^0.8.0;

contract Meter {
    uint256 public reading;

    function record(uint8 x) external {
        require(x <= 100, "cap");
        reading = x;
    }

    function force(uint256 x) external {
        reading = x;
    }
}
