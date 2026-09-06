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

The chain runs end to end for the P1a subset: Solidity in, certified findings out.

| stage | command | what it does |
|---|---|---|
| **P1-01** front end | `ir` | Solidity to ProgramIR through solc's unoptimized Yul |
| **P1-02** abstraction | `analyze` | ProgramIR and a spec to a finite model, then analysed |
| **P0** analysis core | `analyze-model`, `verify` | finite models in, certified findings out |

* The P1a subset is one uint256 argument per entrypoint, pure comparison
  guards, whole-slot uint256 storage, no loops, no external calls. Anything
  outside it is reported and makes the unit incomplete (exit 2), never dropped.
* Every claim is about the generated model (`scope: abstract-model`). Nothing
  here proves anything about a source program until the correspondence layers
  of docs 09/11 exist. That is P1-04.

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
# P1-02: the whole chain, Solidity to certified findings
mulu analyze examples/limits/Limits.sol --contract Limits \
     --spec examples/limits/limits.spec.json --out analysis-limits
mulu verify analysis-limits

# P1-01 alone: stop at the ProgramIR
mulu ir examples/limits/Limits.sol --contract Limits --out ir-limits

# P0 alone: a hand-written finite model
mulu validate examples/limits/model.json
mulu analyze-model examples/limits/model.json --out a
mulu analyze-model examples/fixtures/blocking-cycle.json --out b --objective safety
```

### `mulu analyze`

```
abstraction (p1a-abi-single-v1)
  entrypoints modelled: setLimit(uint256), forceSet(uint256), limit()
  specification limit-bound: limit <= 1000
  argument regions
    X0   {[0, 100]}
    X1   {[101, 1000]}
    X2   {[1001, 1157920892373161954235709850086879078532699846656405640394575840079131296399
35]}
  discharged: argument: A and B and not spec:limit-bound is unsatisfiable
  model: 36 states, 47 transitions

WARNING  spec-violation   idle_LIM0 --call_forceSet_X2--> ... --next_tx--> bad
                          claim: bad-reachable  status: proven
INFO     check-A          can fail from setLimit#0_X1_LIM0, setLimit#0_X2_LIM0
HINT     check-B          never fails on any reachable model state
                          claim: never-fails  status: proven  depends on: A
```

The argument domain is split by the guard conditions **and** by the
specification pulled back through `limit = x`, which is what makes every guard
decidable on every region. The split is done by interval arithmetic over
uint256, so it is exact: no solver, and therefore no `trusted-solver`
assumption. Combinations that turn out to be unsatisfiable, such as `x <= 100`
together with `x > 1000`, are reported as discharged rather than dropped in
silence.

Without `--spec` there is no bad state and only redundancy is analysed
(docs/09 §3).

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

The IR and abstraction stages prove nothing. They record, per check, the
syntactic criterion matched and the semantic gap it leaves open. The
abstraction additionally records its environment profile, what it discharged by
interval arithmetic, and anything it refused to model, in `abstraction.json`.

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
│   ├── mulu-yul/       Yul lexer/parser, CFG, effects, folding, ProgramIR, checks
│   ├── mulu-abstraction/ uint256 intervals, guard predicates, spec DSL, model builder
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

P1-01 and P1-02 are done: `mulu analyze` builds by construction what
`examples/limits/model.json` says by hand, and reaches the same two findings.

Next is **P1-03**: solve a counterexample region for concrete arguments and
replay it on a local EVM, so `forceSet(1001)` is reproduced rather than only
derived. Then **P1-04**, the correspondence proofs that let a finding move from
`abstract-model` to `yul-semantics`, and **P1-05**, the reference control plant
that turns the envelope into overrestriction candidates on generated models.
P2: reentrancy, after checking the DFA-plant theory even applies.
P3: annotations, LSP, Yul hints. P4: benchmarks.

## License

MIT or Apache-2.0, at your option (see `LICENSE-MIT`, `LICENSE-APACHE`).
