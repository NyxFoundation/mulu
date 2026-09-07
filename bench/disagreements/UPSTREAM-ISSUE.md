# Draft issue for `fsainas/contracts-verification-benchmark`

Not sent. This is the text to file if and when we decide to.

Checked against `01251cc` (`feat: add docker image`), which is the current
upstream HEAD as of 2026-09-08.

---

**Title:** Three `ground-truth.csv` rows contradict the contracts they describe

Hello, and thank you for the benchmark. We used it to evaluate a static
analyser and found three (property, version) rows where the ground truth and
the contract disagree. Each one is a counterexample that runs on an EVM, and
in one case the repository's own experiment table already implies the row is
wrong.

The counterexamples are three Foundry tests. Each passes, and each passing
test is a request the ground truth says cannot exist.

### 1. `vault/wd-fin-before`, v3

`ground-truth.csv` says `1`. The property is

> after a successful `withdraw`, `finalize` cannot be successfully called
> before `wait_time` blocks have elapsed, with no in-between calls

and v3 is described as *removed the time constraint on `finalize`*: it
comments out `require(block.number >= request_time + wait_time)`. So
`finalize` succeeds in the same block as the `withdraw` that requested it.

```solidity
Vault v = new Vault{value: 1 ether}(RECOVERY, 100);
uint256 requested = block.number;
v.withdraw(payable(address(this)), 1);
require(block.number == requested);  // none of the 100 blocks has elapsed
v.finalize();                        // succeeds
```

`contracts/vault/README.md` marks Certora `FN` on this cell, which is only
consistent with the property failing. The same README's ground-truth table
also disagrees with `ground-truth.csv` on `wd-fin-before` (v1, v2) and
`fin-canc-twice` (all three versions).

### 2. `zerotoken_bank/wd-not-revert`, v5

`ground-truth.csv` says `1`. The property is

> a `withdraw(amount)` call does not revert if `amount` is bigger than zero
> and less or equal to the balance entry of `msg.sender`

and v5 adds `require(amount <= 100)` to `withdraw`. A balance of 300 is
reachable, and `withdraw(150)` against it reverts.

```solidity
ZeroTokenBank b = new ZeroTokenBank();
b.deposit(150);
b.deposit(150);              // the deposit cap is `amount < 200`
// balanceOf(this) == 300, and 0 < 150 <= 300
(bool ok, ) = address(b).call(abi.encodeWithSignature("withdraw(uint256)", uint256(150)));
// ok == false
```

`certora/wd-not-revert.spec` omits the `amount > 0` half of the README's
wording, which may be how this was missed; the counterexample above does not
depend on that half either way.

### 3. `zerotoken_bank/dep-not-revert`, v7

`ground-truth.csv` says `0` for v1 through v6 and `1` for v7. The property is
"a `deposit(amount)` call never reverts", and the footnote for v1 gives the
reason as *reverts if overflow*. `deposit` is byte-identical in all seven
versions: v7's diff against v1 adds `created_block`, a constructor, and one
`require` in `withdraw`. It overflows in v7 too.

```solidity
ZeroTokenBank b = new ZeroTokenBank();
b.deposit(type(uint256).max);
(bool ok, ) = address(b).call(abi.encodeWithSignature("deposit(uint256)", uint256(1)));
// ok == false
```

### Reproducing

The three tests are in our repository, and they read the contracts from a
clone of this one rather than copying them:

    https://github.com/NyxFoundation/mulu -> bench/disagreements/

    ./bench/fetch.sh
    forge test --root bench/disagreements -vv

Happy to open a PR against `ground-truth.csv` and the affected READMEs if that
is useful, or to leave it here if you would rather check it yourselves first.
