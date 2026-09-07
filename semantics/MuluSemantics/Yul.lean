import Mulu
import EvmYul.Yul.Interpreter

/-!
# The Yul semantics, as `Mulu.Semantics.Concrete`

`Mulu/Semantics/Simulation.lean` states the link from a model claim to a claim
about the thing modelled, over an arbitrary `Concrete S`. It supplies no `S`,
on purpose: the theorem holds for whatever semantics is eventually given, and
until one is given every finding stays at `abstract-model`.

This file gives one. `EvmYul` is Nethermind's executable model of Yul and the
EVM in Lean, and is the copy Paradigm's Solidus pins for its verified backend.
Adopting it does not make the semantics true; it moves the residual assumption
from "there is no semantics to state the conditions against" to "this
semantics agrees with the EVM", which its conformance suite tests rather than
proves. The obligation ledger says exactly that.

## What a step is

`EvmYul` gives a fuel-indexed big-step interpreter,
`exec : Nat → Stmt → Option YulContract → State → Except Exception State`.
`Concrete` wants a relation on states. The relation here executes one
statement of a list, which is the granularity the analyser's models are built
at: each `Step` of a `mulu` trace is one Yul instruction.

Three decisions, each of which the ledger has to record:

* **A revert is a step.** The outcome carries the `Except`, so a guard that
  fails moves to `.error .Revert` rather than getting stuck. The analyser's
  models have fail edges, and a relation with no successor there would make
  `step_covered` vacuously true exactly where it matters most.
* **Running out of fuel is not.** `.OutOfFuel` is a property of the
  interpreter, not a behaviour of the program, so it is excluded. The cost is
  that this relation cannot tell divergence from being stuck: a statement that
  never terminates simply has no successor, and `step_covered` says nothing
  about it. For the P1a fragment, which has no unbounded loops, that is a gap
  in the statement rather than in the coverage. Outside it, it is a gap.
* **The code comes from the state.** `codeOverride` is `none`, so calls
  resolve against the account the execution environment names, as they do in
  `EvmYul`'s own tests.
-/

namespace MuluSemantics

open EvmYul EvmYul.Yul

/-- A Yul execution in progress: the outcome so far, and the statements that
have not run yet. The outcome is an `Except` so that a revert is a state the
relation can reach rather than a place it stops. -/
abbrev Config := Except Yul.Exception Yul.State × List Ast.Stmt

/-- `a` steps to `b` when the first statement left in `a` runs, for some
amount of fuel, to the outcome in `b`.

`∃ fuel` rather than a fixed budget: the fuel is the interpreter's, and a
claim about the program must not depend on how much of it was supplied. What
it does mean is that a divergent statement has no successor at all, which is
the gap the module comment names. -/
def stepRel : Config → Config → Prop :=
  fun a b =>
    ∃ s stmt rest,
      a = (.ok s, stmt :: rest) ∧
      b.2 = rest ∧
      (∃ fuel, Yul.exec fuel stmt .none s = b.1) ∧
      -- Out of fuel is the interpreter giving up, not the program doing
      -- something. Admitting it as a step would let a model "cover" a
      -- behaviour the program does not have.
      b.1 ≠ .error .OutOfFuel

/-- The concrete system for one contract: start in `s₀` with `prog` to run. -/
def concrete (s₀ : Yul.State) (prog : List Ast.Stmt) : Mulu.Semantics.Concrete Config where
  initial := fun a => a = (.ok s₀, prog)
  step := stepRel

/-- Running out of fuel is never a step, whatever the statement. This is the
one property of the definition worth stating separately: it is what stops the
interpreter's budget from becoming a behaviour of the program. -/
theorem no_step_to_out_of_fuel (a b : Config) (h : stepRel a b) :
    b.1 ≠ .error .OutOfFuel := by
  obtain ⟨_, _, _, _, _, _, hne⟩ := h
  exact hne

/-- A step consumes exactly one statement, so the remaining program is a
suffix of what it was. An analyser that claims a model edge for a step is
claiming it for one Yul instruction, not for a block. -/
theorem step_consumes_one (a b : Config) (h : stepRel a b) :
    ∃ stmt, a.2 = stmt :: b.2 := by
  obtain ⟨_, stmt, rest, ha, hb, _, _⟩ := h
  exact ⟨stmt, by rw [ha, hb]⟩

/-- Only a live execution steps: once the outcome is an exception, nothing
follows it. A reverted transaction is a leaf, which is what the analyser's
models say too. -/
theorem no_step_from_error (a b : Config) (e : Yul.Exception) (h : stepRel a b)
    (he : a.1 = .error e) : False := by
  obtain ⟨s, _, _, ha, _, _, _⟩ := h
  rw [ha] at he
  exact Except.noConfusion he

end MuluSemantics
