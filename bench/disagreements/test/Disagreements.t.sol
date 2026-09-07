// SPDX-License-Identifier: MIT
pragma solidity >= 0.8.2;

import {Vault} from
    "corpus/contracts-verification-benchmark/contracts/vault/versions/Vault_v3.sol";
import {ZeroTokenBank as ZTB5} from
    "corpus/contracts-verification-benchmark/contracts/zerotoken_bank/versions/ZeroTokenBank_v5.sol";
import {ZeroTokenBank as ZTB7} from
    "corpus/contracts-verification-benchmark/contracts/zerotoken_bank/versions/ZeroTokenBank_v7.sol";

/// The three (property, version) pairs where mulu and the corpus's
/// `ground-truth.csv` disagree, run on an EVM.
///
/// Each test is the counterexample mulu produced, written out. A test that
/// passes is a counterexample that exists, which is what the answer key says
/// cannot happen. Reading the code is an argument; this is not.
///
///     ./bench/fetch.sh && forge test --root bench/disagreements -vv
contract Disagreements {
    address payable constant RECOVERY = payable(address(0xBEEF));

    /// `vault/wd-fin-before` v3. The key says the property holds.
    ///
    /// "after a successful `withdraw`, `finalize` cannot be successfully
    /// called before `wait_time` blocks have elapsed, with no in-between
    /// calls." v3 is the version that removed the time constraint, so it can.
    ///
    /// The corpus's own experiment table for this cell marks Certora `FN`,
    /// which is only possible if the property fails.
    function test_vault_finalizes_before_the_wait_time() external {
        Vault v = new Vault{value: 1 ether}(RECOVERY, 100);
        uint256 requested = block.number;
        v.withdraw(payable(address(this)), 1);
        require(block.number == requested, "no block has elapsed of the 100 required");
        v.finalize();
    }

    /// `zerotoken_bank/wd-not-revert` v5. The key says the property holds.
    ///
    /// "a `withdraw(amount)` call does not revert if `amount` is bigger than
    /// zero and less or equal to the balance entry of `msg.sender`." v5 is
    /// the version that caps a withdrawal at 100.
    function test_ztb5_withdraw_reverts_within_the_balance() external {
        ZTB5 b = new ZTB5();
        b.deposit(150);
        b.deposit(150);
        uint256 bal = b.balanceOf(address(this));
        require(bal == 300, "balance");
        require(150 > 0 && 150 <= bal, "the rule's own precondition holds");
        (bool ok, ) = address(b).call(abi.encodeWithSignature("withdraw(uint256)", uint256(150)));
        require(!ok, "withdraw did not revert");
    }

    /// `zerotoken_bank/dep-not-revert` v7. The key says the property holds in
    /// v7 and fails in v1 through v6, where `deposit` is the same code.
    ///
    /// "a `deposit(amount)` call never reverts."
    function test_ztb7_deposit_reverts_on_overflow() external {
        ZTB7 b = new ZTB7();
        b.deposit(type(uint256).max);
        (bool ok, ) = address(b).call(abi.encodeWithSignature("deposit(uint256)", uint256(1)));
        require(!ok, "deposit did not revert");
    }

    receive() external payable {}
}
