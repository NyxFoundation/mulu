import Mulu.Core.Envelope

/-!
# Correctness — the public statements

Named corollaries of `checkEnvelope_sound` and `closed_sound`, in the form
the report generator cites. Each theorem is about the **finite model that
was checked**, not about any source program: lifting to Yul / Solidity is a
separate correspondence obligation (`scope: abstract-model`).
-/

namespace Mulu.Core

section Envelope
variable {p : Plant} {nb : Bool} {cert : List (List Nat)}

/-- **Safety**: the envelope contains no bad state (and only in-range states). -/
theorem envelope_safe (h : checkEnvelope p nb cert = true) :
    ∀ q, q ∈ finalW cert → q < p.numStates ∧ q ∉ p.bad :=
  (checkEnvelope_sound h).1.1

/-- **Controllability**: an uncontrollable transition never leaves the envelope. -/
theorem envelope_controllable (h : checkEnvelope p nb cert = true) :
    ∀ q e q', q ∈ finalW cert → (⟨q, e, q'⟩ : Edge) ∈ p.edges →
      p.uncontrollable e = true → q' ∈ finalW cert :=
  fun q e q' hq he hu => (checkEnvelope_sound h).1.2.1 q hq ⟨q, e, q'⟩ he rfl hu

/-- **Nonblocking**: in nonblocking mode every envelope state can reach a
marked state inside the envelope. -/
theorem envelope_nonblocking (h : checkEnvelope p true cert = true) :
    ∀ q, q ∈ finalW cert → CoreachIn p (finalW cert) q :=
  (checkEnvelope_sound h).1.2.2 rfl

/-- **Maximality**: every good set is contained in the envelope, so the
envelope is the greatest fixed point and the supervisor that disables exactly
`disabled p (finalW cert)` is maximally permissive on this model. -/
theorem envelope_maximal (h : checkEnvelope p nb cert = true) :
    ∀ G, Good p nb G → Sub G (finalW cert) :=
  (checkEnvelope_sound h).2

/-- **Unrealizability**: if no initial state survives, no good set contains an
initial state — no supervisor meets the objective from the initial states. -/
theorem envelope_unrealizable (h : checkEnvelope p nb cert = true)
    (hnone : ∀ q, q ∈ p.initial → q ∉ finalW cert) :
    ∀ G, Good p nb G → ∀ q, q ∈ p.initial → q ∉ G :=
  fun G hG q hq hqG => hnone q hq (envelope_maximal h G hG q hqG)

/-- A transition in `disabled` is controllable, starts inside and ends outside. -/
theorem mem_disabled {W : List Nat} {e : Edge} :
    e ∈ disabled p W ↔ e ∈ p.edges ∧ e.src ∈ W ∧ e.ev ∈ p.controllable ∧ e.dst ∉ W := by
  simp [disabled, List.mem_filter, and_assoc]

end Envelope

section Reach
variable {p : Plant} {R : List Nat}

/-- A reachability certificate that passes `isClosed` covers the reachable set. -/
theorem reach_cover (h : isClosed p R = true) : ∀ q, Reachable p q → q ∈ R :=
  closed_sound h

/-- **Model safety**: if a checked invariant avoids `bad`, no bad state is reachable. -/
theorem reach_safe (h : isClosed p R = true) (hb : (R.all fun q => !p.bad.contains q) = true) :
    ∀ q, Reachable p q → q ∉ p.bad := by
  intro q hq hbad
  have := List.all_eq_true.1 hb q (closed_sound h q hq)
  simp at this
  exact this hbad

end Reach

end Mulu.Core
