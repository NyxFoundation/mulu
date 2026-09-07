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

The number to read is not the percentage. It is the histogram of *why* mulu
stops, because that is the list of things to implement, in the order the
corpus says they matter. A percentage on its own would only say whether the
fragment is small, which we already know.

A case is **out of scope** when it asks for something the measurement is not
about: a linked library, the via-IR pipeline, an EVM version this run is not,
a solc newer than the pinned one, or a test with no calls. Counting those
against mulu would damn it for something it was never asked to do; counting
them for it would flatter.

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
| | **25.6%** | memory word 64, and a slot passed as a parameter |

The first run said 0%. mulu's Yul parser treated `data` as a reserved word,
and solc names a generated helper `array_dataslot_…(ptr) -> data` for every
array, struct and mapping. One line in the keyword list was the difference
between reading a third of the corpus and reading almost none of it. That is
the argument for measuring against programs someone else chose.

The largest single move, ten points, came from binding the targets of a `let`.
Nothing bound them, so every local was unknown, and a guard over a local was
"not the argument" even where the local *was* the argument one line later. It
is not a feature anyone would have put on a roadmap; the corpus found it.

**Then ten points came back off, on purpose.** Binding a definition and moving
on skipped the safety net for an instruction that can revert, so a `let` whose
value reverts was passed over and the revert path was dropped from the model.
That is the exact hole the net exists to stop, and it was worth ten points of
score. Only definitions that compute are bound now. Some of those points came
back the right way: a call that defines a value is entered like one made as a
statement, so its checks are seen and its results are bound on the way out.
A number that goes down because the tool got more correct is the number to
publish.

Why the rest stop, ranked:

| count | reason | whose |
| --- | --- | --- |
| 252 | via-IR, a newer solc, an EVM version, or no calls | the corpus |
| 113 | the constructor's effect on storage is not determined | arrays and structs |
| 40 | an instruction with effects the model cannot represent | arrays and structs |
| 38 | a branch is not decided by the argument regions | refinement |
| 31 | reaches `call` or `staticcall` | out of the P1a subset |
| 12 | the ABI lists one parameter and the body takes two | a decoder that returns two |

An array **read** with a constant index models, and the module elaborates in
the semantics. An array read or write with a *symbolic* index does not, and
this is a structural limit rather than a missing case: the guard is
`index < length`, both sides are regions, and independent interval regions
over two variables cannot decide a relation between them. Deciding it needs
the partition to be over the pair, which is a different abstraction from the
one that is there. That, and the array as a storage fact whose length changes,
are what the top rows are.
