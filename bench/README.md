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
| | **35.0%** | a switch, interval arithmetic, a condition evaluated |

The first run said 0%. mulu's Yul parser treated `data` as a reserved word,
and solc names a generated helper `array_dataslot_…(ptr) -> data` for every
array, struct and mapping. One line in the keyword list was the difference
between reading a third of the corpus and reading almost none of it. That is
the argument for measuring against programs someone else chose.

The largest single move, ten points, came from binding the targets of a `let`.
Nothing bound them, so every local was unknown, and a guard over a local was
"not the argument" even where the local *was* the argument one line later. It
is not a feature anyone would have put on a roadmap; the corpus found it.

Why the rest stop, ranked:

| count | reason | whose |
| --- | --- | --- |
| 252 | via-IR, a newer solc, an EVM version, or no calls | the corpus |
| 113 | the constructor's effect on storage is not determined | arrays and structs |
| 40 | an instruction with effects the model cannot represent | arrays and structs |
| 38 | a branch is not decided by the argument regions | refinement |
| 31 | reaches `call` or `staticcall` | out of the P1a subset |
| 12 | the ABI lists one parameter and the body takes two | a decoder that returns two |

An array **read** models now: the bounds check is decided by comparing the
index region against the length read from storage, and the module elaborates
in the semantics. What is still missing is the array as a *storage fact*: a
length that changes when something is pushed, and a write to an element whose
slot is computed. That is what the top two rows are, and it is the next thing
to decide about, because it adds a kind of fact rather than generalising one
that is there.
