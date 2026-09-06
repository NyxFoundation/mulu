# mulu

**mulu — A Supervisory Control-based static analyzer for code redundancy and gap detection.**

mulu takes a finite transition model of a program together with a specification
monitor and answers three questions on one common model, each with a
machine-checked certificate:

| question | finding | needs a spec | how |
|---|---|---|---|
| Does this check always succeed when reached? | `redundant-check` / `never-fails` | no | reachable set, least fixed point |
| Can the implementation reach a bad state? | `spec-violation` | yes | direct search + path certificate |
| Does the implementation reject requests the spec would allow? | `overrestriction` (candidate) | yes | maximal permissive envelope, greatest fixed point |

The envelope is the Ramadge–Wonham *maximal permissive supervisor* of the plant
under the specification: the largest set of states that is safe, closed under
uncontrollable events and (in nonblocking mode) can always complete.
Rust does I/O, normalisation and reporting; **Lean 4 computes and, more
importantly, checks** — every reported `proven` finding is backed by a
certificate whose meaning is a Lean theorem, re-checked by the Lean kernel.

## Status (2026-09-06)

Two stages exist. They do not meet yet.

| stage | command | what it does |
|---|---|---|
| **P0** analysis core | `analyze-model`, `verify` | finite models in, certified findings out |
| **P1-01** front end | `ir` | Solidity in, ProgramIR out. No analysis |

* `mulu analyze`, which would run one into the other, needs the predicate
  abstraction of P1-02 and returns `unsupported` (exit 2).
* Every claim is about the model or the IR (`scope: abstract-model`). Nothing
  here proves anything about a source program until the correspondence layers
  of docs 09/11 exist.

## Build

Lean 4 (v4.25.0, no mathlib, no network needed) and a stable Rust toolchain.
`solc` is needed only for `mulu ir` and its tests, which skip without it.

```sh
cd lean && lake build && cd ..      # Mulu library + lean/.lake/build/bin/mulu-worker
cargo build --release               # target/release/mulu
make test                           # cargo test + kernel-checked fixture theorems
```

## Use

```sh
# P1-01: Solidity -> ProgramIR
mulu ir examples/limits/Limits.sol --contract Limits --out ir-limits

# P0: finite model -> certified findings
mulu validate examples/limits/model.json
mulu analyze-model examples/limits/model.json --out analysis-limits
mulu verify analysis-limits
mulu analyze-model examples/fixtures/blocking-cycle.json --out a --objective safety
```

`analyze-model` writes:

```
analysis-limits/
├── manifest.json          input hashes, name↔index maps, certificate list, kernel result
├── model.json             copy of the input
├── core-model.json        normalised integer model the worker reads (+ core-plant.json)
├── worker-impl.json       raw worker responses (+ worker-plant.json)
├── report.json            machine-readable diagnostics (the source of truth)
└── certificates/
    ├── *.json             one certificate per finding (checked by the worker)
    ├── Check.lean         self-contained kernel re-check: `theorem … := by decide`
    ├── kernel.log         output of `lake env lean Check.lean`
    └── axioms.txt         `#print axioms` per theorem (must be ⊆ propext, Quot.sound, Classical.choice)
```

### `mulu ir`

Compiles with solc's Standard JSON, reads the **unoptimized Yul**, and lowers it
to a CFG per Yul function with the checks pulled out. For `examples/limits`:

```
checks
  A                            require   setLimit
      passes when  iszero(gt(var_x_5, 0x64))
      purity Pure   at Limits.sol:11:9
  B                            require   setLimit
      passes when  iszero(gt(var_x_5, 0x03e8))
      purity Pure   at Limits.sol:12:9
  gen:external_fun_setLimit_27#0 compiler  setLimit
      passes when  iszero(callvalue())
      purity ReadsEnvironment   at Limits.sol:7:1
storage writes
  setLimit  Limits.sol:13:9
      update_storage_value_offset_0_t_uint256_to_t_uint256(0x00, expr_23)
unsupported: none
```

`require`s get letters in source order, because that is how a reader of the
contract refers to them. Compiler-inserted guards get an id tied to where they
sit, so adding a `require` does not renumber them. A condition is reported both
as written in the Yul (`expr_11`) and after propagating single-assignment pure
locals and pure alias helpers, which is what makes `x <= 100` visible again.
`purity` says whether a condition is a function of the call's arguments alone:
only `Pure` ones can become predicates in P1a. Anything outside the P1a subset
(external calls, `delegatecall`, `create`, `gas`) is listed under `unsupported`
and makes the exit code 2, never silently dropped.

### `mulu analyze-model`

Output for `examples/limits` (the design example of docs 08 §3):

```
WARNING  spec-violation      idle --call_forceSet_X2--> fS_W_X2 --store--> fS_end_L1 --ret--> bad
                             claim: bad-reachable  status: proven  scope: abstract-model
INFO     check-A             can fail from sL_A_X1, sL_A_X2          claim: may-fail  status: candidate
HINT     check-B             never fails on any reachable state       claim: never-fails  status: proven  depends on: A
INFO     envelope            winning = {…}  disable = {(B_X2, cont_B), (F_X2, cont_F)}   status: proven
INFO     overrestriction-A-sL_A_X1   A rejects, the envelope would accept the same request   status: candidate
```

Exit codes: `0` complete, no confirmed violation · `1` confirmed violation
(or a candidate with `--fail-on-candidate`) · `2` partial / unsupported ·
`3` input or execution error · `4` a certificate failed to check.
Mixed results use the priority 4 > 3 > 2 > 1 > 0.

`verify` re-hashes `model.json`, re-normalises it, re-checks every certificate
through the worker, regenerates `Check.lean` byte-for-byte and re-runs it under
the kernel with an axiom audit. Changing an edge, a certificate or the proof
tactic makes it fail (exit 4).

## What is proved, and by what

| Lean theorem (`lean/Mulu/`) | statement |
|---|---|
| `Core.closed_sound` | `isClosed p R = true → ∀ q, Reachable p q → q ∈ R` |
| `Core.reachSet_sound` | everything the iteration returns is reachable (minimality) |
| `Core.checkPath_sound` | a checked path proves a bad state is reachable |
| `Core.checkEnvelope_sound` | a checked chain `[W₀,…,Wₙ]` yields `Wₙ` **safe, uncontrollably closed, nonblocking, and maximal** among all such sets |
| `Core.envelope_unrealizable` | if no initial state survives, no supervisor meets the objective |
| `Analysis.checkRedundancy_sound` | no reachable state enables the fail event of the check |
| `Analysis.checkCertificate_sound` | `checkCertificate p c = true → Claim p c` for all of the above |

The IR stage proves nothing. It records, per check, the syntactic criterion it
matched and the semantic gap that leaves open, under `assumptions`.

Trusted base: the Lean kernel, `Mulu.Core`'s definitions, the normalisation
`FiniteProduct → CoreModel` in `crates/mulu-model` (recorded in the manifest),
and the hand-written model itself. The compiled worker is *not* trusted for
`proven`: `Check.lean` re-does the check under the kernel with `decide`
(`native_decide` is not used). `Mulu.Core` imports nothing but Lean core.

Deliberately **not** claimed: `never-fails` is not `removal-equivalent`
(gas, side effects); an unrealizable envelope is not a source-level
vulnerability; an overrestriction is a *candidate* derived from a conservative
reference plant whose correspondence to the implementation is an open
obligation (docs 11 E9); nothing is said about inputs outside the model.

## Layout

```
mulu/
├── crates/
│   ├── mulu-model/     schema v1, validator, normalisation, hashes, reference algorithms
│   ├── mulu-solc/      solc Standard JSON driver, BuildBundle, source hashes
│   ├── mulu-yul/       Yul lexer/parser, CFG, effects, ProgramIR, check extraction
│   └── mulu-cli/       `mulu` — worker driver, Check.lean generator, report, exit codes
├── lean/
│   ├── Mulu/Core/      FinitePlant, Reachability (lfp), Envelope (gfp), Correctness
│   ├── Mulu/Analysis/  Redundancy, Certificate (checkCertificate + soundness)
│   ├── Mulu/Worker/    JSON protocol
│   ├── Main.lean       mulu-worker
│   └── Tests/          kernel-checked fixture theorems and rejected tampers
├── schemas/            finite-product.v1.json, worker-protocol.v1.md
├── tools/              regen-yul-fixtures.sh
└── examples/           fixtures/ (docs 09 §2), limits/ (docs 08 §3: source + hand-written model)
```

`crates/mulu-yul/tests/fixtures/` holds solc's real output for `Limits.sol` so
the parser tests need no compiler. It is committed together with the exact
source it came from, and a test fails if the two drift apart. Regenerate both
with `tools/regen-yul-fixtures.sh`.

The design documents (theory, event model, implementation plan, evaluation)
live in the NyxFoundation `projects/mulu/docs` directory.

## Roadmap

P1-01 (solc adapter and ProgramIR) is done. Next is **P1-02**: turn the pure
checks and storage writes of the IR into predicates and a finite model, so
`mulu analyze` can run the two halves together and reproduce by construction
what `examples/limits/model.json` says by hand. Then P1-03 replays
counterexamples on a local EVM and P1-04 proves the correspondence that lets a
finding move from `abstract-model` to `yul-semantics`.
P2: reentrancy, after checking the DFA-plant theory even applies.
P3: annotations, LSP, Yul hints. P4: benchmarks.

## License

MIT or Apache-2.0, at your option (see `LICENSE-MIT`, `LICENSE-APACHE`).
