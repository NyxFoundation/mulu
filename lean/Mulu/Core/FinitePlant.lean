/-!
# Finite plant

The Solidity-independent object of study: a finite, partially defined
transition structure with an event partition (controllable /
uncontrollable), initial states, marked states and bad states.

States and events are `Nat` indices into `[0, numStates)` / `[0, numEvents)`.
Sets are represented as `List Nat`; the mathematical statements use list
membership `∈`, the executable code uses `List.contains`.

Nothing here knows about Yul, solc, SMT or SARIF.
-/

namespace Mulu.Core

/-- One transition `src --ev--> dst`. -/
structure Edge where
  src : Nat
  ev : Nat
  dst : Nat
deriving Repr, DecidableEq, Inhabited

/-- A finite plant, possibly already multiplied with a specification monitor
(then `bad` is non-empty). Events not listed in `controllable` are
uncontrollable. -/
structure Plant where
  numStates : Nat
  numEvents : Nat
  controllable : List Nat
  initial : List Nat
  marked : List Nat
  bad : List Nat
  edges : List Edge
deriving Repr, Inhabited

namespace Plant

/-- Executable set membership. -/
abbrev mem (l : List Nat) (x : Nat) : Bool := l.contains x

/-- An event is uncontrollable iff it is not declared controllable. -/
def uncontrollable (p : Plant) (e : Nat) : Bool := !(p.controllable.contains e)

/-- All state indices. -/
def allStates (p : Plant) : List Nat := List.range p.numStates

/-- Every index is in range. Unknown indices are rejected before any analysis. -/
def wellFormed (p : Plant) : Bool :=
  p.initial.all (· < p.numStates) &&
  p.marked.all (· < p.numStates) &&
  p.bad.all (· < p.numStates) &&
  p.controllable.all (· < p.numEvents) &&
  p.edges.all (fun e => e.src < p.numStates && e.dst < p.numStates && e.ev < p.numEvents)

/-- Partial determinism: the same `(state, event)` never leads to two
different targets. The supervisory-control API requires this; the plain
reachability API does not. -/
def deterministic (p : Plant) : Bool :=
  p.edges.all fun e => p.edges.all fun e' =>
    !(e.src == e'.src && e.ev == e'.ev) || e.dst == e'.dst

/-- Mathematical one-step relation. -/
def Step (p : Plant) (q e q' : Nat) : Prop := (⟨q, e, q'⟩ : Edge) ∈ p.edges

end Plant

/-- `Sub A B`: every element of `A` is an element of `B` (sets as lists). -/
def Sub (A B : List Nat) : Prop := ∀ x, x ∈ A → x ∈ B

theorem Sub.refl (A : List Nat) : Sub A A := fun _ h => h

theorem Sub.trans {A B C : List Nat} (h₁ : Sub A B) (h₂ : Sub B C) : Sub A C :=
  fun x hx => h₂ x (h₁ x hx)

/-- Executable containment check and its meaning. -/
def subB (A B : List Nat) : Bool := A.all (fun x => B.contains x)

theorem contains_iff {l : List Nat} {x : Nat} : l.contains x = true ↔ x ∈ l :=
  List.contains_iff_mem

theorem subB_iff {A B : List Nat} : subB A B = true ↔ Sub A B := by
  simp [subB, Sub, List.all_eq_true]

/-- Executable set equality (mutual containment). -/
def setEq (A B : List Nat) : Bool := subB A B && subB B A

theorem setEq_iff {A B : List Nat} : setEq A B = true ↔ Sub A B ∧ Sub B A := by
  simp [setEq, subB_iff]

end Mulu.Core
