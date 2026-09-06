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
#print axioms t1
#print axioms t2
#print axioms checkEnvelope_sound
#print axioms checkCertificate_sound
#print axioms checkRedundancy_sound
#print axioms checkPath_sound
