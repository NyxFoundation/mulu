<h1 align="center">mulu</h1>

<p align="center">
  <em>A Supervisory Control-based static analyzer for code redundancy and gap detection.</em>
</p>

<p align="center">
  <a href="https://github.com/NyxFoundation/mulu/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/NyxFoundation/mulu/actions/workflows/ci.yml/badge.svg"></a>
  <a href="LICENSE"><img alt="License: MIT" src="https://img.shields.io/badge/license-MIT-blue.svg"></a>
  <img alt="Lean 4" src="https://img.shields.io/badge/Lean-4.22.0-4B0082.svg">
  <img alt="Rust" src="https://img.shields.io/badge/Rust-stable-orange.svg">
  <img alt="mathlib free" src="https://img.shields.io/badge/mathlib-not%20required-lightgrey.svg">
</p>

---

Most analysers tell you what might go wrong. mulu also tells you what your
checks are doing that they need not, and where they are stricter than your
specification asks. It answers all three on **one model**, and every answer
it calls proven is backed by a certificate the **Lean kernel** re-checks.

```console
$ mulu analyze examples/limits/Limits.sol --contract Limits \
      --spec examples/limits/Limits.spec.json --out analysis

HINT     check-B          never fails on any reachable model state
                          claim: never-fails  status: proven  depends on: A
WARNING  spec-violation   forceSet(1001) leaves limit = 1001, breaking limit <= 1000
                          reproduced on a local EVM
INFO     overrestriction-A
                          setLimit(101) is permitted by the specification and rejected
                          claim: spec-permits-rejected-request  status: candidate
```

| question | finding | needs a spec |
|---|---|---|
| Does this check always succeed when reached? | `redundant-check` | no |
| Can the contract reach a state the spec forbids? | `spec-violation` | yes |
| Does it reject requests the spec would allow? | `overrestriction` | yes |

The envelope behind the third is the Ramadge–Wonham **maximal permissive
supervisor**: the largest behaviour that stays safe, is closed under events
the code cannot refuse, and can always complete.

## Install

Lean 4 v4.22.0 (no mathlib, no network) and a stable Rust toolchain. `solc` is
needed only for the Solidity front end.

```sh
git clone https://github.com/NyxFoundation/mulu && cd mulu
cd lean && lake build && cd ..     # the Mulu library and mulu-worker
cargo build --release
make check                         # tests, kernel checks, examples end to end
```

## Honest limits, up front

- **Every finding is a claim about a model mulu built**, not about your
  contract. `obligations.json` lists what would have to be proved to change
  that, and none of it is proved yet. See [What a finding is a claim
  about](#what-a-finding-is-a-claim-about).
- The analysable subset is small: one argument per entrypoint typed `uintN`,
  `address` or `bool`, pure comparison guards, storage variables that own their
  slot, no loops, no external calls.
- Anything outside it is **reported and makes the run incomplete**, never
  quietly skipped.
- `never-fails` is not `removal-equivalent`. It does not say you may delete the
  check.

## Status (2026-09-06)

The chain runs end to end for the P1a subset: Solidity in, certified findings out.

| stage | command | what it does |
|---|---|---|
| **P1-01** front end | `ir` | Solidity to ProgramIR through solc's unoptimized Yul |
| **P1-02** abstraction | `analyze` | ProgramIR and a spec to a finite model, then analysed |
| **P1-05** reference plant | `analyze` | the same walk again with the guards parameterised out |
| **P1-03** replay | `analyze` | each counterexample run on an in-process EVM |
| **P1-04** correspondence | `analyze`, `verify` | what stands between a model claim and the program |
| **P1-06** output | `analyze`, `verify` | SARIF, deterministic artifacts, cut-off runs that decide nothing |
| **P0** analysis core | `analyze-model`, `verify` | finite models in, certified findings out |

Several source files, `import` statements and guards written in modifiers all
work. Abstract contracts, interfaces and libraries are recognised as having no
code rather than treated as a compilation failure.

## Use

```sh
# P1-02: the whole chain, Solidity to certified findings
mulu analyze examples/limits/Limits.sol --contract Limits \
     --spec examples/limits/Limits.spec.json --out analysis-limits
mulu verify analysis-limits

# the same finding, with the first guard in a modifier in an imported file
mulu analyze examples/access/Vault.sol --contract Vault \
     --spec examples/access/Vault.spec.json --out analysis-vault

# a narrow argument type rules a violation out rather than inventing one
mulu analyze examples/typed/Meter.sol --contract Meter \
     --spec examples/typed/Meter.spec.json --out analysis-meter

# two entrypoints sharing a name stay two entrypoints
mulu analyze examples/overload/Over.sol --contract Over \
     --spec examples/overload/Over.spec.json --out analysis-over

# a guard written `if (..) revert()` rather than `require(..)`
mulu analyze examples/guards/Gate.sol --contract Gate \
     --spec examples/guards/Gate.spec.json --out analysis-gate

# P1b: a Foundry or Hardhat project, using the settings it built with
mulu analyze --project ./my-project --contract Limits --spec ./limits.spec.json --out ./analysis

# P1-01 alone: stop at the ProgramIR
mulu ir examples/limits/Limits.sol --contract Limits --out ir-limits

# P0 alone: a hand-written finite model
mulu validate examples/limits/model.json
mulu analyze-model examples/limits/model.json --out a
mulu analyze-model examples/models/blocking-cycle.json --out b --objective safety
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
INFO     envelope         disable = {(setLimit@1_X2_LIM0, cont_B),
                                     (forceSet@0_X2_LIM0, cont_entry_forceSet)}
INFO     overrestriction-A-setLimit#0_X1_LIM0
                          claim: spec-permits-rejected-request  status: candidate
```

### Reading a project's own build

`--project <dir>` reads the newest **build-info** file Foundry and Hardhat
write under `out/build-info` or `artifacts/build-info`: the exact Standard JSON
input and output of a real build. `--build-info <file>` names one directly.
The settings a project built with are a fact recorded there, so mulu takes
them from the file rather than evaluating `foundry.toml` or, worse,
`hardhat.config.js`. Remappings come from it too, which is what makes an
imported guard resolve at all.

mulu does not analyse the build-info's own output: a normal build does not
request `ir`, and it usually has the optimizer on, which is not the artifact
any claim here is about. The build supplies the sources and the settings; mulu
compiles them again for the artifact it reads, and says every way the two
still differ:

```
this analysis is not the build the project ships:
  the project built with solc 0.8.26 and mulu ran 0.8.28, so this is not the compilation the project ships
  the project builds with the optimizer on; the analysed Yul is unoptimized
  lib/oz/token/Cap.sol has been edited since the build
```

Each of those lines is also recorded against `compilation:optimised-bytecode`
in the obligation ledger, because each is a reason the deployed bytecode is
not this artifact. A warning scrolls past; an obligation does not.

### SARIF, and what a viewer is allowed to draw

Every run also writes `results.sarif` (SARIF 2.1.0), and `--sarif <path>` puts
a copy where a CI step wants it. It is a second rendering of `report.json`,
never a second analysis, and `verify` compares the two finding by finding.

SARIF separates *what the tool concluded* (`kind`) from *how bad it is*
(`level`), and requires `level` to be `none` whenever `kind` is not `fail`. So
a candidate cannot be drawn as an error even by a viewer that ignores
everything mulu says about scope:

| mulu status | SARIF kind | what a viewer shows |
| --- | --- | --- |
| `proven`, `reproduced` | `fail` | a finding, at its severity |
| `candidate` | `review` | needs a human |
| `unknown` | `open` | undecided |
| `not-requested` | `notApplicable` | not run |

A run cut off by `--max-states`, `--max-edges` or `--timeout-ms` decides
nothing: the analyses are declined rather than run and labelled, every finding
comes back `unknown`, and the exit code is 2. Nothing from a cut-off run is
ever shown as `pass`, and a single undecided finding is enough to keep the exit
code off 0.

### What a finding is a claim about

Everything reported is a claim about the model the tool built. Moving one to
the program means crossing the layers of docs/04 §7 — model, Yul semantics,
Solidity source, deployed bytecode — and each crossing is an obligation.

`lean/Mulu/Semantics/Simulation.lean` states the first crossing. Given an
abstraction with `initial_covered` and `step_covered`, a checked invariant of
the model covers every reachable concrete state, and a checked `never-fails`
becomes a statement about the program, provided the concrete guard's failing
branch is matched by that check's fail event.

`obligations.json` is the ledger of those conditions for one analysis:

```
correspondence
  findings are reported at: abstract-model
  9 obligation(s), 9 open
  6 to prove here:
    semantics:rendering-preserves-the-program  reaches yul-semantics
    simulation:initial-covered               reaches yul-semantics
    simulation:step-covered                  reaches yul-semantics
    check:fail-step-matched:A                reaches yul-semantics
    check:fail-step-matched:B                reaches yul-semantics
    plant:policy-corresponds                 reaches yul-semantics
  3 assumption(s) on work this project did not do:
    semantics:evmyul-matches-the-evm         (evmyul) reaches yul-semantics
    compilation:yul-corresponds-to-source    (solc)   reaches solidity-source
    compilation:optimised-bytecode           (solc)   reaches evm-bytecode
```

**None of them is discharged**, so every finding stays at `abstract-model` and
the reason is enumerated rather than described. The two groups are not the
same kind of thing. The first six are statements about mulu's own abstraction
and the proofs are ours to write. The last three are properties of artifacts
this project did not write: that solc lowers Solidity faithfully, that its
optimizer preserves behaviour, and that the adopted Yul semantics agrees with
the EVM. No amount of work here closes those.

### The Yul semantics

`semantics/` instantiates `Mulu.Semantics.Concrete` at
[EvmYul](https://github.com/NethermindEth/EVMYulLean), Nethermind's executable
model of Yul and the EVM in Lean, pinned through the fork Paradigm's
[Solidus](https://www.paradigm.xyz/writing/solidus) uses for its verified
backend. Writing a Yul semantics from scratch would be the wrong work when one
exists in the same proof assistant.

Adopting it did not discharge anything. It split one obligation into two
smaller and more honest ones: this contract is not yet expressed in that
semantics, and that semantics is not proved to agree with the EVM. The second
is now visible in the ledger under its own name rather than hiding inside "no
semantics exists".

It is a **separate Lake package, opt-in, built by `make semantics`**. EvmYul
requires mathlib, so its dependency closure is several gigabytes and needs the
network. `mulu analyze` and `mulu verify` import none of it, and `make check`
still builds from Lean core alone, because the point of the kernel re-check is
that a reader can reproduce it without trusting a supply chain.

Every analysis writes the contract into that semantics, at
`semantics/<Name>.lean` in the output directory, from the same `ir` the model
was built from. `mulu yul-lean` produces the same module on its own. The
rendering is not a transcription, and each way it is not is reported and
recorded against the obligation it raises:

```
semantics
  analysis-limits/semantics/Limits.lean in EvmYul's notation
  a string literal was rendered as the 32-byte word Yul says it denotes, left-aligned and zero-padded
  memoryguard(x) was rendered as x: it is a hint to solc's optimizer with no run-time meaning
```

A contract that needed no rewrite carries no such assumption. `make
semantics-check` renders every example and checks Lean accepts it, because a
rendering only mulu can read would prove nothing about anything.

`mulu semantics-diff` goes further: it runs the rendered contract in EvmYul's
interpreter and the same calls on revm, and compares the storage. Both start
from the storage the deployment left, so a constructor this does not model is
not mistaken for a step disagreement.

```
call                         EvmYul                 revm
setLimit(50)                 ok {0=50}              ok {0=50}
setLimit(101)                revert {0=50}          revert {0=50}
forceSet(1001)               ok {0=1001}            ok {0=1001}
```

All five examples agree, across reverts, modifiers, `uint8` truncation and
overload dispatch (`make semantics-diff`). **This is evidence and not a
proof**, and the obligation stays open either way. What it catches is a
divergence, which is a defect in one of three places: the rendering is a
different program, EvmYul and revm disagree about the EVM, or the analysis is
reading the wrong artifact.

The rule is executable, not documentary. `verify` recomputes each finding's
scope from the ledger and refuses a report that claims more; it also refuses a
ledger that claims a discharge, since the tool discharges nothing and cannot
check one. A layer nothing is recorded for is not treated as reached: silence
means the conditions were never enumerated.

### Replay

A region is not a counterexample anyone can act on. Each finding is turned
into concrete calls and run on an in-process EVM, deploying the contract's own
creation code:

```
WARNING  spec-violation   idle_LIM0 --call_forceSet#X2--> ... --next_tx--> bad
                          claim: bad-reachable  status: proven
                          reproduced on a local EVM: reproduced
                            forceSet(1001)
                            violated: limit-bound
INFO     overrestriction-A-setLimit#0_X1_LIM0
                          reproduced on a local EVM: reproduced
                            setLimit(101)
                            the contract rejects it: cap
```

The argument is the region's smallest member, so `X2 = [1001, MAX]` gives
`forceSet(1001)`. The record, including the storage the EVM left behind, goes
in `witnesses/`.

A replay is **different evidence**, not a stronger version of the certificate:
the model claim stays `proven` by the kernel and the run is recorded beside it.
The specification is evaluated at every successful transaction end, so a
violation a later call repairs is still seen, and a property that could not be
evaluated is reported as such rather than counted as satisfied.

A replay that does *not* reproduce means the model and the EVM disagree. That
is a gap to look at, not a finding to dismiss, so it makes the unit incomplete.

### The reference plant

The third finding needs a second model. The same walk runs again with each
guard replaced by a control site: continuing is **controllable**, rejecting is
**uncontrollable**, and the guard's own condition plays no part. That is the
conservative plant of docs/11 §4, and its maximal permissive envelope is the
largest behaviour a supervisor could allow.

An overrestriction is where the two disagree: the implementation rejects a
request the envelope would accept, and the request could still complete. For
`examples/limits` that is exactly the region between the two bounds, so
`setLimit(500)` is a candidate while `setLimit(2000)` is not.

The plant carries the same specification monitor as the implementation. Without
it nothing is unsafe, the envelope forbids nothing, and every rejection looks
like an overrestriction. It also gets a site at the entry of an entrypoint that
writes storage with no guard at all, which is how `forceSet` is reported as
needing one.

Both models come from one walk, so their states correspond and the pairing in
`sites` means what it says. A candidate is always `candidate`, never proven: it
says the specification permits the request, not that the check can be removed.

The argument domain is split by the guard conditions **and** by the
specification pulled back through `limit = x`, which is what makes every guard
decidable on every region. The split is done by interval arithmetic over
uint256, so it is exact: no solver, and therefore no `trusted-solver`
assumption. Combinations that turn out to be unsatisfiable, such as `x <= 100`
together with `x > 1000`, are reported as discharged rather than dropped in
silence.

Without `--spec` there is no bad state and only redundancy is analysed
(docs/09 §3).

### Modifiers, imports and where a check was written

solc lowers a modifier into a separate Yul function, so `setLimit` becomes
`fun_setLimit` calling `modifier_capped` calling `fun_setLimit_inner`. The walk
follows those calls; a guard inside a modifier refines the argument partition
exactly like one written in the function body. `examples/access` is
`examples/limits` with the first guard moved into a modifier in an imported
file, and it reaches the same conclusion.

The Yul only carries a byte span. Which contract and which modifier that span
belongs to is in the AST, which the adapter indexes:

```
checks
  A       modifier  modifier_capped_28  (modifier Bounded.capped)
      passes when  iszero(gt(var_x_24, 0x64))
      purity Pure   at Base.sol:9:9
  B       require   Vault.setLimit
      passes when  iszero(gt(var_x_24, 0x03e8))
      purity Pure   at Vault.sol:12:9
```

Locations resolve through the file id they carry, so a check reported for
`Vault` can point into `Base.sol`.

An instruction whose effects the model cannot express stops the walk rather
than being skipped. Without that, a modifier the walk did not follow produced a
model in which the function did nothing, reported as complete.

### Types decide the domains

The environment profile admits type-correct calls, so the argument's domain is
what its ABI type admits, not the whole machine word. `examples/typed` bounds
`reading <= 1000` and offers two ways to set it:

| entrypoint | argument domain | can it break the bound |
|---|---|---|
| `record(uint8)` | `[0, 255]` | no, and the model says so |
| `force(uint256)` | the whole word | yes, with a path to `bad` |

Treating both as uint256 would invent a region above 255 that no call to
`record` can reach. Storage domains come from the layout's type table the same
way, and a variable that shares its slot with another is refused rather than
written whole.

Which entry a selector is comes from solc's `evm.methodIdentifiers`, not from
matching the dispatcher against ABI names. With an overload, name matching
hands one selector the other's argument type, and the type is what fixes the
domain, so calls disappear from the model. `examples/overload` has two `set`
functions, one `uint256` and one `uint8`; they get separate states, separate
call events and separate domains. Without the table the pairing is reported as
ambiguous rather than guessed.

Reading the type also settles the cleanups solc inserts. Storing a `uint8`
into a `uint256` slot lowers to `and(x, 0xff)`, and comparing one lowers to
`gt(and(x, 0xff), 100)`. That mask is the identity exactly when the type keeps
the value inside it, which is checked; a mask that would really truncate is
refused.

`analyze-model` writes:

```
analysis-limits/
├── manifest.json          input hashes, name↔index maps, certificate list, kernel result
├── model.json             copy of the input
├── core-model.json        normalised integer model the worker reads (+ core-plant.json)
├── worker-impl.json       raw worker responses (+ worker-plant.json)
├── report.json            machine-readable diagnostics (the source of truth)
├── results.sarif          the same diagnostics as SARIF 2.1.0, for a code-scanning viewer
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
the kernel with an axiom audit. It also checks that `results.sarif` says what
`report.json` says, and that neither file is missing. Changing an edge, a
certificate, the proof tactic or a line of the SARIF makes it fail (exit 4).

Two runs of the same version on the same input produce byte-identical
`model.json`, `core-model.json`, `abstraction.json`, `report.json` and
`results.sarif`, so a diff of two runs is signal.

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
| `Semantics.Simulation.reachable` | an abstraction covering initial states and steps carries reachability |
| `Semantics.Simulation.invariant` | so a checked invariant covers every reachable concrete state |
| `Semantics.Simulation.never_fails` | and a checked redundancy says the guard's failing branch is unreachable |

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
│   ├── mulu-solc/      solc Standard JSON driver, imports, AST index, hashes, build-info
│   ├── mulu-yul/       Yul lexer/parser, CFG, effects, folding, ProgramIR, checks
│   ├── mulu-abstraction/ uint256 intervals, guard predicates, spec DSL, model builder
│   ├── mulu-replay/    concrete calls on an in-process EVM (revm)
│   └── mulu-cli/       `mulu` — worker driver, Check.lean generator, report, exit codes
├── semantics/          opt-in: `Mulu.Semantics.Concrete` at EvmYul (needs mathlib)
├── lean/
│   ├── Mulu/Core/      FinitePlant, Reachability (lfp), Envelope (gfp), Correctness
│   ├── Mulu/Analysis/  Redundancy, Certificate (checkCertificate + soundness)
│   ├── Mulu/Semantics/ Simulation: carrying a model claim to what it models
│   ├── Mulu/Worker/    JSON protocol
│   ├── Main.lean       mulu-worker
│   └── Tests/          kernel-checked fixture theorems and rejected tampers
├── schemas/            finite-product.v1.json, worker-protocol.v1.md
├── tools/              regen-yul-fixtures.sh
└── examples/
    ├── models/         hand-written finite models, for the core alone
    │                   (including a check nothing reaches)
    ├── limits/         the worked example: two guards, one redundant
    ├── access/         the first guard in a modifier in an imported file
    ├── guards/         a guard written `if (..) revert()`
    ├── overload/       two entrypoints sharing a name
    └── typed/          a narrow argument type rules a violation out
```

`crates/mulu-yul/tests/fixtures/` holds solc's real output for `Limits.sol` so
the parser tests need no compiler. It is committed together with the exact
source it came from, and a test fails if the two drift apart. Regenerate both
with `tools/regen-yul-fixtures.sh`.

The design documents (theory, event model, implementation plan, evaluation)
live in the NyxFoundation `projects/mulu/docs` directory.

## Roadmap

P1-01, P1-02, P1-05 and P1-03 are done: `mulu analyze` builds by construction
what `examples/limits/model.json` says by hand, reaches all three findings, and
runs each one on an EVM.

P1-04 states the crossing and enumerates what it needs; it does not discharge
it. The one obligation everything else waits on is a formal semantics of solc's
Yul in Lean, which is docs/09 §9's fourth open question and a piece of work in
its own right.

After that: **P1b** (Foundry and
Hardhat projects, incremental caching), **P2** reentrancy, after checking the
DFA-plant theory even applies, **P3** annotations and LSP, **P4** benchmarks.
P2: reentrancy, after checking the DFA-plant theory even applies.
P3: annotations, LSP, Yul hints. P4: benchmarks.

## License

MIT. See [`LICENSE`](LICENSE).
