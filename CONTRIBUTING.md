# Contributing to mulu

## Building

Lean 4 v4.25.0 (no mathlib, no network needed) and a stable Rust toolchain.
`solc` is needed only for the Solidity front end and its tests, which skip
without it.

```sh
cd lean && lake build && cd ..   # the Mulu library and mulu-worker
cargo build --release
make check                       # tests, kernel-checked fixtures, examples end to end
```

`make check` is what CI runs. It builds both halves, runs every test, replays
the four Solidity examples and re-verifies their certificates.

## What a change has to keep true

This is a verification tool, so the bar is not "the tests pass" but "the tool
cannot say more than it knows".

- **A claim carries its scope.** Every finding is reported at the layer whose
  correspondence obligations are discharged, and `verify` recomputes that from
  the ledger. Do not widen a claim without adding what earns it.
- **Nothing is silently dropped.** A construct the analysis cannot model is
  recorded as `unsupported` and makes the unit incomplete. A silent skip is a
  bug even when the result happens to be right.
- **`proven` and `reproduced` are different.** A kernel-checked certificate and
  an EVM run are separate evidence; neither upgrades the other.
- **A recognition states its criterion.** When the pipeline recognises a shape
  in solc's output, record what it matched and what gap that leaves, so the
  assumption is visible in the report rather than buried in the code.
- **Lean proofs use `decide`, never `native_decide`.** The axiom audit rejects
  anything outside `propext`, `Quot.sound` and `Classical.choice`.

## Naming

One convention per kind of thing, so a path says what it holds.

| kind | case | example |
|---|---|---|
| directories | lower kebab | `crates/mulu-yul`, `examples/access` |
| Solidity contracts | Pascal, matching the contract | `examples/limits/Limits.sol` |
| a file *about* a contract | Pascal, matching it | `examples/limits/Limits.spec.json` |
| hand-written models, schemas, scripts | lower kebab | `examples/models/blocking-cycle.json` |
| solc output kept as a fixture | Pascal, by contract | `tests/fixtures/Limits.yul` |
| a source copy kept for drift checks | `<example>-<Contract>` | `tests/fixtures/limits-Limits.sol` |
| Rust | snake, as the language wants | `crates/mulu-yul/src/lower.rs` |
| Lean | Pascal, as the language wants | `lean/Mulu/Core/Envelope.lean` |

The last two rows are the languages' own conventions and are not ours to
change. The rest exist so a reader can tell a contract from a model from a
copy without opening it.

## Fixtures

`crates/*/tests/fixtures/` holds real solc output, committed alongside the
exact source it came from so the parser tests need no compiler. A test fails
if the two drift. Regenerate both with:

```sh
./tools/regen-yul-fixtures.sh
```

## Adding an example

An example is a contract, a specification, and an entry in the `analyze` target
of the Makefile. Keep it small enough that a reader can hold the expected
answer in their head, and say in a comment what it is meant to demonstrate.

## Reporting a defect in the analysis

An unsound answer — a finding stronger than the evidence supports, or a
behaviour the model loses — is the most valuable report. Include the contract,
the specification, and what the tool said. See `SECURITY.md`.
