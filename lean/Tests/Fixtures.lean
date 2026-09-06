import Mulu
/-!
The three P0 fixtures of docs/09 §2, checked by the kernel (`decide`), plus
tampered certificates that must be rejected. Run with
`lake env lean Tests/Fixtures.lean` (also `make test`).
-/
open Mulu.Core Mulu.Analysis

-- fixture 1: q0 -c-> q1 -u-> bad ; c controllable
def f1 : Plant where
  numStates := 3
  numEvents := 2
  controllable := [0]
  initial := [0]
  marked := [0]
  bad := [2]
  edges := [⟨0,0,1⟩, ⟨1,1,2⟩]
-- fixture 2: same, c uncontrollable
def f2 : Plant := { f1 with controllable := [] }
-- fixture 3: bad edge removed; q1 has no path to marked
def f3 : Plant := { f1 with edges := [⟨0,0,1⟩], bad := [] }

def chain (p : Plant) (nb : Bool) : List (List Nat) :=
  let rec go : Nat → List Nat → List (List Nat)
    | 0, W => [W]
    | n+1, W => let W' := envStep p nb W; if setEq W' W then [W] else W :: go n W'
  go (p.numStates + 1) (initialW p)

#eval chain f1 true
#eval chain f2 true
#eval chain f3 true
#eval chain f3 false
#eval disabled f1 (finalW (chain f1 true))
#eval reachSet f1
-- expected values from docs/09 §2
theorem t1 : checkEnvelope f1 true [[0,1],[0]] = true := by decide
example : disabled f1 [0] = [⟨0,0,1⟩] := by decide
theorem t2 : checkEnvelope f2 true [[0,1],[0],[]] = true := by decide      -- unrealizable: initial not in []
theorem t3 : checkEnvelope f3 true [[0,1,2],[0]] = true := by decide      -- nonblocking prunes q1 (and unreachable q2)
theorem t4 : checkEnvelope f3 false [[0,1,2]] = true := by decide         -- safety mode keeps q1
theorem t5 : checkCertificate f1 (.violation [⟨0,0,1⟩, ⟨1,1,2⟩]) = true := by decide
theorem t6 : checkCertificate f1 (.reachability [0,1,2]) = true := by decide
-- tampered / lazy certificates: must be rejected
example : checkEnvelope f1 true [[0,1],[0,1]] = false := by decide   -- claims q1 survives
example : checkEnvelope f1 true [[0,1]] = false := by decide         -- stops before the fixed point
example : checkEnvelope f1 true [[0]] = false := by decide           -- wrong W₀
example : checkCertificate f1 (.reachability [0,1]) = false := by decide       -- not closed
example : checkCertificate f1 (.violation [⟨0,0,1⟩]) = false := by decide     -- does not end in bad
example : checkCertificate f1 (.redundancy [0,1,2] ⟨0, 0, 1⟩) = false := by decide -- event 1 fails from reachable q1

/-! ### `unreachable` is not `never-fails`

`f4` has a check whose two events leave only q3, which nothing reaches.
`never-fails` holds of it, and so does the stronger "never evaluated"; the
point of the separate certificate is that the first does not imply the second,
so the report may not show the second on the first's evidence. -/

-- q0 -0-> q1 -1-> q2 (marked); q3 -2-> q2 and q3 -3-> q2, with q3 unreachable.
-- Check ⟨0, 2, 3⟩ is the one nothing reaches.
def f4 : Plant where
  numStates := 4
  numEvents := 4
  controllable := []
  initial := [0]
  marked := [2]
  bad := []
  edges := [⟨0,0,1⟩, ⟨1,1,2⟩, ⟨3,2,2⟩, ⟨3,3,2⟩]

theorem t7 : checkCertificate f4 (.unreachableCheck [0,1,2] ⟨0, 2, 3⟩) = true := by decide
-- the same check is trivially never-fails, which is the weaker claim
theorem t8 : checkCertificate f4 (.redundancy [0,1,2] ⟨0, 2, 3⟩) = true := by decide
-- and a check that *is* evaluated is never-fails but not unreachable, so the
-- weaker certificate cannot stand in for the stronger one
theorem t9 : checkCertificate f4 (.redundancy [0,1,2] ⟨1, 0, 3⟩) = true := by decide
example : checkCertificate f4 (.unreachableCheck [0,1,2] ⟨1, 0, 3⟩) = false := by decide
-- R that is not closed is rejected for this kind too
example : checkCertificate f4 (.unreachableCheck [0,1] ⟨0, 2, 3⟩) = false := by decide
-- naming q3 as reachable makes the check evaluated, and the certificate fails
example : checkCertificate f4 (.unreachableCheck [0,1,2,3] ⟨0, 2, 3⟩) = false := by decide

#print axioms t7
#print axioms checkUnreachable_sound
#print axioms t1
#print axioms t2
#print axioms checkEnvelope_sound
#print axioms checkCertificate_sound
#print axioms checkRedundancy_sound
#print axioms checkPath_sound

/-! ## Simulation (P1-04)

The link that would carry a model claim to the program. It is proved here on a
toy concrete system, which shows the theorem is usable; supplying the same
conditions for solc's Yul is the open work, so nothing the analyser reports is
promoted past `abstract-model`.
-/
open Mulu.Semantics

/-- A concrete system with two states, mapped onto f1 by `abs`. -/
def toy : Concrete Bool where
  initial := fun s => s = false
  step := fun s s' => s = false ∧ s' = true

def toyAbs : Bool → Nat := fun s => if s then 1 else 0

def toySim : Simulation toy f1 where
  abs := toyAbs
  initial_covered := by
    intro s hs
    simp [toy] at hs
    subst hs
    simp [toyAbs, f1]
  step_covered := by
    intro s s' _ hstep
    obtain ⟨h, h'⟩ := hstep
    subst h; subst h'
    exact ⟨0, by simp [toyAbs, f1]⟩

-- Reachability carries across, so a checked invariant of the model covers the
-- concrete run.
example : ∀ s, ReachableC toy s → Reachable f1 (toySim.abs s) := toySim.reachable

example (h : isClosed f1 [0, 1, 2] = true) : ∀ s, ReachableC toy s → toySim.abs s ∈ [0, 1, 2] :=
  toySim.invariant h

#print axioms Mulu.Semantics.Simulation.reachable
#print axioms Mulu.Semantics.Simulation.never_fails
#print axioms Mulu.Semantics.Simulation.invariant
