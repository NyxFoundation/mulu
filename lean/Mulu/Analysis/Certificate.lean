import Mulu.Analysis.Redundancy

/-!
# Certificates

One sum type for everything the worker can be asked to check, a single
`checkCertificate` entry point, and the theorem that turns its success into
the corresponding claim. The Rust side stores certificates as JSON and also
emits a self-contained `Check.lean` that re-runs `checkCertificate` under the
kernel (`by decide`), so `verify` does not have to trust the compiled worker.
-/

namespace Mulu.Analysis
open Mulu.Core

inductive Certificate where
  /-- `R` is an invariant covering the reachable states. -/
  | reachability (R : List Nat)
  /-- `R` invariant, and check `c` never fails on it. -/
  | redundancy (R : List Nat) (c : Check)
  /-- Bad state reachable along `path`. -/
  | violation (path : List Edge)
  /-- Envelope chain `[W₀, …, Wₙ]` for objective `nonblocking`. -/
  | envelope (nonblocking : Bool) (chain : List (List Nat))
deriving Repr, Inhabited

def checkCertificate (p : Plant) : Certificate → Bool
  | .reachability R => isClosed p R
  | .redundancy R c => checkRedundancy p R c
  | .violation path => checkPath p path
  | .envelope nb chain => checkEnvelope p nb chain

/-- What a successful check means, per certificate kind. -/
def Claim (p : Plant) : Certificate → Prop
  | .reachability R => ∀ q, Reachable p q → q ∈ R
  | .redundancy _ c => ∀ q q', Reachable p q → (⟨q, c.failEvent, q'⟩ : Edge) ∈ p.edges → False
  | .violation _ => ∃ b, Reachable p b ∧ b ∈ p.bad
  | .envelope nb chain => Good p nb (finalW chain) ∧ ∀ G, Good p nb G → Sub G (finalW chain)

/-- **checker_sound**: check success implies the claim. -/
theorem checkCertificate_sound {p : Plant} (c : Certificate)
    (h : checkCertificate p c = true) : Claim p c := by
  cases c with
  | reachability R => exact closed_sound h
  | redundancy R c => exact checkRedundancy_sound h
  | violation path => exact checkPath_sound h
  | envelope nb chain => exact checkEnvelope_sound h

end Mulu.Analysis
