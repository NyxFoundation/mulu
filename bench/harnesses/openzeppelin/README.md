# OpenZeppelin harnesses

Flattened OpenZeppelin Contracts v5 sources, each with a small harness
contract that exposes what the specification is about. They are here so the
answers in `bench/properties/openzeppelin/` can be reproduced:

```
mulu analyze --out /tmp/out \
  bench/harnesses/openzeppelin/OwnableHarness.flat.sol \
  --contract OwnableHarness \
  --call-properties bench/properties/openzeppelin/ownable.json
```

The library code is OpenZeppelin's, MIT, copied rather than fetched: a
specification is about a particular text, and a version that moves under it
answers a different question. The harness at the bottom of each file is this
project's, also MIT.

Three of them are not a straight copy, and the file says so where it happens:

- `SafeCastHarness` writes `SafeCast` as an abstract contract rather than a
  library. solc emits a library as a separate deployed object and leaves a
  `linkersymbol` at the call site, which P1a does not model. The bodies under
  test are the same either way.
- `ReentrancyGuardHarness` gives its entrypoints empty bodies. A body that
  can revert on its own would make the rules about the body rather than about
  the modifier.
- `Ownable2StepHarness` and `OwnableHarness` add a `restricted()` that does
  nothing but carry the modifier, which is how the modifier itself is stated.

`InitializableHarness` follows `fv/harnesses/InitializableHarness.sol` from
OpenZeppelin's own Certora setup, including the nested initializers.
