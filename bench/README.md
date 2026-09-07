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

| | |
| --- | --- |
| cases | 1682 |
| in scope | 1430 |
| reaching a complete model | 328 (22.9%) |

The first run said 0%. mulu's Yul parser treated `data` as a reserved word,
and solc names a generated helper `array_dataslot_…(ptr) -> data` for every
array, struct and mapping. One line in the keyword list was the difference
between reading a third of the corpus and reading almost none of it. That is
the argument for measuring against programs someone else chose.

Why the other 1102 stop, ranked:

| count | reason |
| --- | --- |
| 179 | an instruction with effects the model cannot represent |
| 133 | a guard over something that is not an argument |
| 103 | the constructor's effect on storage is not determined |
| 37 | more than one argument |
| 38 | reaches `call` or `staticcall` |

**These are mostly one thing.** Of the 1102, 260 are in `array/` or `structs/`,
and the array helpers account for most of the top three rows as well: a bounds
check is a guard over a length rather than over an argument, and an allocation
is an instruction whose effect the model has no room for. Arrays are the next
fragment to decide about, and the decision is not only about coverage: every
extension makes `simulation:step-covered` harder to prove, so the fragment
should be chosen once and proved once rather than grown and re-proved.
