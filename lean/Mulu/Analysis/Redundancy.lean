import Mulu.Core.Correctness

/-!
# Redundant checks — `never-fails`

A check `c` of the implementation model is represented by two events:
`passEvent` (the guard evaluated to true) and `failEvent` (it evaluated to
false and the transaction reverts). The guard result is *fixed* by the
semantics, so a state either enables the pass event or the fail event.

`c` never fails iff **no reachable state enables its fail event**.
The certificate is a checked invariant set `R` (`isClosed`) together with the
observation that no edge labelled `failEvent` leaves a state of `R`.

This proves `never-fails` on the model. It does not prove
`removal-equivalent` (gas, side effects, observation) — that is a separate
claim the report never conflates with this one.

The theorem does not look at the pass edges of `c` itself, so a "circular"
certificate that assumes `c`'s own success cannot be produced: the only
hypotheses are the closure of `R` from the initial states.
-/

namespace Mulu.Analysis
open Mulu.Core

/-- A check as seen by the finite model. -/
structure Check where
  id : Nat
  passEvent : Nat
  failEvent : Nat
deriving Repr, DecidableEq, Inhabited

/-- No state of `R` enables the fail event of `c`. -/
def neverFailsOn (p : Plant) (R : List Nat) (c : Check) : Bool :=
  p.edges.all fun e => !(e.ev == c.failEvent && R.contains e.src)

/-- Full certificate check for `redundant-check / never-fails`. -/
def checkRedundancy (p : Plant) (R : List Nat) (c : Check) : Bool :=
  isClosed p R && neverFailsOn p R c

/-- **Soundness**: after a successful check, the fail event of `c` is not
enabled in any reachable state of the model. -/
theorem checkRedundancy_sound {p : Plant} {R : List Nat} {c : Check}
    (h : checkRedundancy p R c = true) :
    ∀ q q', Reachable p q → (⟨q, c.failEvent, q'⟩ : Edge) ∈ p.edges → False := by
  simp only [checkRedundancy, Bool.and_eq_true] at h
  obtain ⟨hcl, hnf⟩ := h
  intro q q' hq he
  have hR := closed_sound hcl q hq
  have := List.all_eq_true.1 hnf _ he
  simp [hR] at this

/-- Some reachable state enables the pass event: the check is actually
exercised (otherwise it is *unreachable*, which the report shows separately). -/
def reachedOn (p : Plant) (R : List Nat) (c : Check) : Bool :=
  p.edges.any fun e => e.ev == c.passEvent && R.contains e.src

end Mulu.Analysis
