import Mulu.Analysis.Certificate

/-!
# Simulation — carrying a model claim to the thing being modelled

Everything the analyser reports is a claim about a finite model it built
itself. docs/04 §7 draws the chain: a claim moves from the model to the
program only along a correspondence, and each link is an obligation, not a
convention.

This file states the link. A concrete system, an abstraction map, and two
conditions on it:

* every concrete initial state maps into the model's initial states;
* every concrete step from a reachable state is matched by a model edge.

From those, a checked invariant on the model covers every reachable concrete
state, and a checked `never-fails` becomes a statement about the program.

Nothing here supplies those conditions. `Concrete` is a parameter, so this
holds for whatever semantics is eventually given; discharging the conditions
for solc's Yul is the work the analyser's obligation ledger enumerates and
which is not done. Until it is, a finding stays `abstract-model`.
-/

namespace Mulu.Semantics

open Mulu.Core Mulu.Analysis

/-- A transition system over an arbitrary state type: the thing being
modelled, whatever its semantics turns out to be. -/
structure Concrete (S : Type) where
  initial : S → Prop
  step : S → S → Prop

variable {S : Type}

/-- Reachability in the concrete system. -/
inductive ReachableC (c : Concrete S) : S → Prop
  | init {s : S} : c.initial s → ReachableC c s
  | step {s s' : S} : ReachableC c s → c.step s s' → ReachableC c s'

/-- An abstraction of `c` by `p`: a map on states and the two conditions that
make the model an over-approximation. `step_covered` is only required on
reachable states, which is what an analyser can hope to establish. -/
structure Simulation (c : Concrete S) (p : Plant) where
  abs : S → Nat
  /-- docs/04 §3: the initial states must be covered, or the model starts
  somewhere the program does not. -/
  initial_covered : ∀ s, c.initial s → abs s ∈ p.initial
  /-- docs/11 §6: every concrete step is represented. A step with no matching
  edge is a behaviour the model does not have, and every claim below fails. -/
  step_covered :
    ∀ s s', ReachableC c s → c.step s s' → ∃ e, (⟨abs s, e, abs s'⟩ : Edge) ∈ p.edges

namespace Simulation

variable {c : Concrete S} {p : Plant}

/-- **The link.** Every reachable concrete state maps to a reachable model
state. -/
theorem reachable (sim : Simulation c p) : ∀ s, ReachableC c s → Reachable p (sim.abs s) := by
  intro s h
  induction h with
  | init hi => exact Reachable.init (sim.initial_covered _ hi)
  | step hr hstep ih =>
    obtain ⟨e, he⟩ := sim.step_covered _ _ hr hstep
    exact Reachable.step ih he

/-- A checked invariant of the model covers every reachable concrete state.
The certificate is checked once, on the model; this is what makes it say
something about the program. -/
theorem invariant (sim : Simulation c p) {R : List Nat} (h : isClosed p R = true) :
    ∀ s, ReachableC c s → sim.abs s ∈ R :=
  fun s hs => closed_sound h _ (sim.reachable s hs)

/-- If no model state in a checked invariant is bad, no reachable concrete
state maps to a bad one. -/
theorem safe (sim : Simulation c p) {R : List Nat} (h : isClosed p R = true)
    (hb : (R.all fun q => !p.bad.contains q) = true) :
    ∀ s, ReachableC c s → sim.abs s ∉ p.bad := by
  intro s hs
  exact reach_safe h hb _ (sim.reachable s hs)

/-- What it takes for a model check to speak about a concrete guard.
`failStep s s'` is the concrete semantics taking the guard's failing branch.
The obligation is that such a step is matched by *that check's* fail event,
not merely by some event: `step_covered` gives an edge, and this says which.

It cannot be read off the model. A model with no fail edge satisfies it only
if the concrete guard never fails, which is the conclusion, so discharging it
needs the concrete semantics. That is the work the analyser's ledger lists
and which is not done. -/
def FailStepMatched (sim : Simulation c p) (chk : Check) (failStep : S → S → Prop) : Prop :=
  ∀ s s', ReachableC c s → failStep s s' →
    (⟨sim.abs s, chk.failEvent, sim.abs s'⟩ : Edge) ∈ p.edges

/-- **`never-fails`, carried across.** A checked redundancy certificate plus
the matching obligation says the guard's failing branch is unreachable in the
program, not merely in the model.

This is still not `removal-equivalent` (docs/04 §3): a branch that cannot be
taken may still be observable through gas or a side effect. -/
theorem never_fails (sim : Simulation c p) {R : List Nat} {chk : Check}
    {failStep : S → S → Prop} (hc : checkRedundancy p R chk = true)
    (m : FailStepMatched sim chk failStep) :
    ∀ s s', ReachableC c s → ¬ failStep s s' := by
  intro s s' hs hf
  exact checkRedundancy_sound hc _ (sim.abs s') (sim.reachable s hs) (m s s' hs hf)

/-- The obligation is about the concrete semantics, and this says why: a model
that has no fail edge for the check gives it no help at all. Anything matched
would have to be an edge the model does not contain. -/
theorem fail_step_matched_needs_the_semantics (sim : Simulation c p) {chk : Check}
    {failStep : S → S → Prop}
    (hnone : ∀ q q', (⟨q, chk.failEvent, q'⟩ : Edge) ∉ p.edges)
    (m : FailStepMatched sim chk failStep) :
    ∀ s s', ReachableC c s → ¬ failStep s s' := by
  intro s s' hs hf
  exact hnone _ _ (m s s' hs hf)

end Simulation

/-! ## Composing the chain

docs/04 §7 stacks the layers: source, chosen artifact, concrete semantics,
finite model. A claim crosses one layer at a time, and simulations compose, so
a proof for each link gives the whole chain. -/

/-- Two abstractions compose when the middle system is the same. -/
def compose {S T : Type} {c : Concrete S} {d : Concrete T} {p : Plant}
    (f : S → T)
    (hinit : ∀ s, c.initial s → d.initial (f s))
    (hstep : ∀ s s', ReachableC c s → c.step s s' → d.step (f s) (f s'))
    (hreach : ∀ s, ReachableC c s → ReachableC d (f s))
    (sim : Simulation d p) : Simulation c p where
  abs := fun s => sim.abs (f s)
  initial_covered := fun s hs => sim.initial_covered _ (hinit s hs)
  step_covered := fun s s' hr hs =>
    sim.step_covered _ _ (hreach s hr) (hstep s s' hr hs)

/-- The middle system's reachability, which `compose` asks for, follows from
the two conditions it already has. -/
theorem reachable_of_maps {S T : Type} {c : Concrete S} {d : Concrete T}
    (f : S → T)
    (hinit : ∀ s, c.initial s → d.initial (f s))
    (hstep : ∀ s s', ReachableC c s → c.step s s' → d.step (f s) (f s')) :
    ∀ s, ReachableC c s → ReachableC d (f s) := by
  intro s h
  induction h with
  | init hi => exact ReachableC.init (hinit _ hi)
  | step hr hs ih => exact ReachableC.step ih (hstep _ _ hr hs)

end Mulu.Semantics
