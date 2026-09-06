// Two entrypoints share a name, so only the compiler's selector table says
// which is which. Pairing them by name gives one of them the other's argument
// type, and the argument type is what fixes the domain the model reasons over.
pragma solidity ^0.8.0;

contract Over {
    uint256 public limit;

    function set(uint256 x) external {
        require(x <= 100, "cap");
        limit = x;
    }

    function set(uint8 y) external {
        limit = y;
    }
}
