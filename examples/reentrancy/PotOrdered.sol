// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;
/// The same pot, with the effect before the interaction.
///
/// Nothing else differs from `Pot.sol`. The bound is restored before control
/// leaves, so a reentrant call finds the contract in a state it is allowed to
/// be in and cannot leave it in one it is not. mulu says so: the violation
/// `Pot.sol` has under `--reentrancy` is not here.
contract Pot {
    uint256 public total;
    function add(uint256 amount) external {
        require(total + amount <= 100);
        total = total + amount;                       // effect before interaction
        (bool ok, ) = msg.sender.call("");
        require(ok);
    }
}
