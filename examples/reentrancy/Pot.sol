// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

/// A pot that must never hold more than 100.
///
/// `add` checks the invariant before it leaves the contract and restores it
/// after. Each transaction on its own keeps `total <= 100`. Two of them
/// interleaved do not: both read `total` before either writes it, both pass
/// the guard, and both add.
///
/// The interleaving is only possible because control leaves the contract
/// between the check and the effect, and the callee may call back in. That
/// is not something the contract can forbid, which is what makes it an
/// uncontrollable event of the plant.
contract Pot {
    uint256 public total;

    function add(uint256 amount) external {
        require(total + amount <= 100);
        (bool ok, ) = msg.sender.call("");
        require(ok);
        total = total + amount;
    }
}
