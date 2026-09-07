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
