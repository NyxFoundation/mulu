# Benchmarks

Two corpora, for two different questions. Neither is vendored: solc is
GPL-3.0 and mulu is MIT, so both are fetched at measurement time and read as
data.

## `semanticTests` — how much Solidity can mulu read?

solc's own `test/libsolidity/semanticTests/` is about 1700 contracts, each
with the calls to make and the results to expect written in a footer. It is
the largest body of small Solidity whose meaning someone else has already
pinned, which is what makes it worth measuring against: **mulu does not get to
choose the programs.**

```sh
./bench/fetch.sh
cargo run --release -p mulu-bench -- \
  bench/corpus/solidity/test/libsolidity/semanticTests --out bench/results.json
```

CI runs the same thing with `--min-modelled`, which fails the build below a
floor. The floor is the last measured number less a small margin, so a real
regression fails and normal noise does not. It also catches something a floor
would not obviously catch: the run has to *finish*, and the worst defect this
corpus found was a ten-line contract that never did.

**28.3% overstates it, and the harness now says so.** Of the 404 contracts
that reach a complete model, 56 contain a guard mulu would report on. The rest
model to three states and a straight line, because there is nothing in them to
say anything about. Coverage that produces no finding is not coverage.

Both numbers are about a corpus that is a catalogue of language features, not
a sample of contracts people write: only 13% of it contains a `require` or an
`assert` at all, because most of it is testing arithmetic and ABI encoding.
The question "how much real Solidity can mulu read" is a different
measurement, on a different corpus, and it has not been made.

The number to read is not the percentage. It is the histogram of *why* mulu
stops, because that is the list of things to implement, in the order the
corpus says they matter. A percentage on its own would only say whether the
fragment is small, which we already know.

A case is **out of scope** when it asks for something the measurement is not
about: a linked library, the via-IR pipeline, an EVM version this run is not,
a solc newer than the pinned one, or a test with no calls. Counting those
against mulu would damn it for something it was never asked to do; counting
them for it would flatter.

## `contracts-verification-benchmark` — is mulu right?

The other corpus has an answer key. `fsainas/contracts-verification-benchmark`
holds 47 contracts across 16 use cases (bank, vault, escrow, htlc, lottery,
crowdfund, tinyamm and so on). `v1` conforms to the use case's specification
and the later versions carry a seeded defect. Beside them, `ground-truth.csv`
records for each of 365 (property, version) pairs whether the property holds,
and `contracts/scores.csv` records what Certora and solc's own model checker
scored on it.

```sh
./bench/fetch.sh
cargo run --release -p mulu-bench -- --corpus-kind verification-benchmark \
  bench/corpus/contracts-verification-benchmark --out bench/verification.json
```

This is the corpus to drive to 100%, because it is the one where 100% means
something: a wrong answer is visible. `semanticTests` cannot be driven to 100%
by anyone, because it is a catalogue of language features and most of them are
outside any abstraction mulu will have.

**mulu does not yet score on it**, and the harness does not pretend to. What
it measures today is how far each contract gets — compiled, lowered, modelled
— because the properties are revert conditions over an entrypoint's arguments
and the storage it reads, and mulu's specification language holds only state
invariants at the end of a successful transaction. Scoring waits on that.

What the histogram says to implement, in the order the corpus asks for it:

42 of the 47 now reach a model, and 40 of those hold a guard mulu would
report on. Three loops and two contracts that run out of the time budget are
what is left.

### The score, 2026-09-08 (later the same day)

`bench/properties/` holds mulu's encodings, one file per use case, beside the
`certora/` and `solcmc/` encodings the benchmark ships for the other tools.
Each property names the file it was written from.

| | |
| --- | --- |
| (property, version) pairs asked | 62 |
| correct | 59 |
| wrong | 3 |
| no answer | 0 |

Every pair mulu and the key agree on the facts about is answered correctly.
The three that are not are all cases where mulu produces a concrete request
that the answer key says cannot exist. They are left in the score as wrong, because a
tool does not get to grade its own disagreements, but each is checkable in a
minute:

- **`vault/wd-fin-before` v3.** v3 removes `require(block.number >=
  request_time + wait_time)` from `finalize`, so `finalize` succeeds before
  the wait time has passed and the property does not hold. The key says it
  holds. The benchmark's own experiment table in `contracts/vault/README.md`
  marks Certora `FN` on this cell, which is only possible if the truth is
  that it fails.
- **`zerotoken_bank/wd-not-revert` v5.** v5 adds `require(amount <= 100)` to
  `withdraw`. The property is "does not revert if `amount` is bigger than
  zero and less or equal to the balance entry", and `withdraw(150)` against
  a balance of 200 reverts. The key says it holds.
- **`zerotoken_bank/dep-not-revert` v7.** `deposit` is byte-identical in v1
  through v7, and the key says the property fails in v1 through v6 (deposit
  reverts on overflow) and holds in v7.

`contracts/vault/README.md` and `contracts/vault/ground-truth.csv` also
disagree with each other on `wd-fin-before` and `fin-canc-twice`.

### What the encoding rests on

Two things, both declared and both printed with the answer.

`bench/properties/zerotoken_bank.json` supplies one invariant,
`cbal-ge-bal`: the contract's total is at least any one balance entry. mulu's
abstraction carries no inductive invariant over storage, so without it the
walk produces a path where `withdraw` underflows the total, which the
contract cannot be on. The benchmark lists `cbal-ge-bal` as a property of its
own and scores it 1 for every version. An answer that used an invariant says
which.

`zerotoken_bank/wd-not-revert` follows the README's wording, which says
`amount is bigger than zero and less or equal to the balance entry`. The
Certora file drops the first half, and with it the counterexample
`withdraw(0)`.

Nine of the sixteen use cases have no file yet, and the reasons are the
model's, not the encoding's. Their revert rules turn on the *sender's* ether
balance, on `msg.value` (the environment profile fixes it at zero), on
several transactions in sequence, or on signed arithmetic: `zerotoken_bet`
keeps its balances in `int`, and every comparison on one is an `slt` that
nothing here decides.

## What the corpus said, 2026-09-07

| run | modelled / in scope | what changed |
| --- | --- | --- |
| first | 0% | — |
| | 22.0% | `data` is not a reserved word inside Yul code |
| | 22.9% | a constructor writing constants is determined |
| | 24.5% | an environment, so a function may take several arguments |
| | 34.5% | a `let` binds what it defines, and `sload` resolves |
| | 34.7% | a guard may be over a local, and over two of them |
| | 35.0% | a switch, interval arithmetic, a condition evaluated |
| | **25.0%** | **a soundness regression found and reverted** |
| | 25.5% | a call that defines a value is entered, not skipped |
| | 25.6% | memory word 64, and a slot passed as a parameter |
| | 27.0% | the helpers solc writes everything through are evaluated |
| | 28.1% | **an exponential removed**: `and` evaluated each side twice |
| | 28.2% | a branch over a parameter refines the partition |
| | **28.3%** | a call made for its value is followed when finding what is reachable |
| | 28.4% | `call` is modelled, under a stated no-reentrancy assumption |
| | 31.9% | a constructor write the model cannot follow widens the initial state |
| | 49.9% | a choice the regions do not settle forks the walk instead of refusing |
| latest | 66.7% | a type is a slot's universe, a narrow write is a write, a panic forks |

The first run said 0%. mulu's Yul parser treated `data` as a reserved word,
and solc names a generated helper `array_dataslot_…(ptr) -> data` for every
array, struct and mapping. One line in the keyword list was the difference
between reading a third of the corpus and reading almost none of it. That is
the argument for measuring against programs someone else chose.

The largest single move, ten points, came from binding the targets of a `let`.
Nothing bound them, so every local was unknown, and a guard over a local was
"not the argument" even where the local *was* the argument one line later.

**Then ten points came back off, on purpose.** Binding a definition and moving
on skipped the safety net for an instruction that can revert, so a `let` whose
value reverts was passed over and the revert path was dropped from the model.
That is the exact hole the net exists to stop, and it was worth ten points of
score. Only definitions that compute are bound now, and a call that defines a
value is entered rather than skipped, so its checks are seen. A number that
goes down because the tool got more correct is the number to publish.

The corpus also found a ten-line contract that never finished being analysed.
`and` asked whether each side was a single value, which evaluates it, and then
evaluated the chosen side again; solc nests cleanups, so a chain of them cost
two to its length. Three wrong guesses at the cause were made before bisecting
the commit that introduced it. The bounds and the two quadratic dedups fixed
along the way were all real and are all kept, but none of them was this.

## What is left, and why it is not more of the same

1027 in-scope cases still stop. They are not spread evenly.

| count | directory |
| --- | --- |
| 200 | `array/` |
| 116 | `abicoder/` |
| 53 | `viaYul/` |
| 52 | `structs/` |

Two things account for most of it, and neither is a missing case.

**A relation between two regions.** An array access is guarded by
`index < length`. Both sides are regions, and independent interval regions over
two variables cannot decide a relation between them: with the index anywhere
in `uint256` and the length anywhere in `uint256`, neither `<` nor `>=` holds.
Deciding it needs the partition to be over the *pair*, which is a different
abstraction from the one that is here. An array access with a constant index
does model, and the module elaborates in the semantics; a symbolic one does
not.

**Loops.** Copying an array or a struct is a loop, and P1a refuses one rather
than unrolling it a fixed number of times and calling the result general.
Most of the constructor failures are this.

Both are fragment decisions rather than bugs, and both make
`simulation:step-covered` harder to prove in a way the incremental work so far
did not. That is the reason to decide them once rather than grow into them.
