import Mulu.Core.Reachability

/-!
# Maximal permissive envelope — greatest fixed point

Given a plant already multiplied with a total specification monitor
(`bad` states), the envelope is the largest set `W ⊆ Q \ B` such that

* `W` is closed under uncontrollable transitions (controllability), and
* (nonblocking mode) every state of `W` can reach a marked state without
  leaving `W`.

It is the greatest fixed point of the monotone operator

    F(W) = { q ∈ W | uc-successors(q) ⊆ W ∧ (nb → CoreachIn (U(W)) q) }

started from `W₀ = Q \ B`.

The executable side does not *prove* anything by itself. It produces a
certificate `[W₀, W₁, …, Wₙ]`; `checkEnvelope` verifies `W₀ = Q \ B`,
`Wᵢ₊₁ = F(Wᵢ)` for each step and `Wₙ = F(Wₙ)`. From that success the
theorems below derive: safety, controllability, nonblocking and — via
monotonicity — **maximality** of `Wₙ`.
-/

namespace Mulu.Core

/-! ## Uncontrollable closure -/

/-- Mathematical: all uncontrollable successors of `q` lie in `W`. -/
def UcClosedAt (p : Plant) (W : List Nat) (q : Nat) : Prop :=
  ∀ e, e ∈ p.edges → e.src = q → p.uncontrollable e.ev = true → e.dst ∈ W

/-- Executable counterpart. -/
def ucClosedAt (p : Plant) (W : List Nat) (q : Nat) : Bool :=
  p.edges.all fun e => !(e.src == q && p.uncontrollable e.ev) || W.contains e.dst

theorem ucClosedAt_iff {p : Plant} {W : List Nat} {q : Nat} :
    ucClosedAt p W q = true ↔ UcClosedAt p W q := by
  simp only [ucClosedAt, UcClosedAt, List.all_eq_true]
  constructor
  · intro h e he hsrc huc
    have := h e he
    rw [hsrc] at this
    simp [huc] at this
    exact this
  · intro h e he
    by_cases hsrc : e.src = q
    · by_cases huc : p.uncontrollable e.ev = true
      · have := h e he hsrc huc
        simp [this]
      · simp [huc]
    · simp [hsrc]

theorem UcClosedAt.mono {p : Plant} {W W' : List Nat} {q : Nat} (hs : Sub W W')
    (h : UcClosedAt p W q) : UcClosedAt p W' q :=
  fun e he hsrc huc => hs _ (h e he hsrc huc)

/-- `U(W)`: the states of `W` whose uncontrollable successors stay in `W`. -/
def ucFilter (p : Plant) (W : List Nat) : List Nat := W.filter (ucClosedAt p W)

theorem mem_ucFilter {p : Plant} {W : List Nat} {q : Nat} :
    q ∈ ucFilter p W ↔ q ∈ W ∧ UcClosedAt p W q := by
  simp [ucFilter, List.mem_filter, ucClosedAt_iff]

theorem ucFilter_sub {p : Plant} {W : List Nat} : Sub (ucFilter p W) W :=
  fun _ h => (mem_ucFilter.1 h).1

theorem ucFilter_mono {p : Plant} {W W' : List Nat} (hs : Sub W W') :
    Sub (ucFilter p W) (ucFilter p W') := by
  intro q hq
  obtain ⟨hW, hc⟩ := mem_ucFilter.1 hq
  exact mem_ucFilter.2 ⟨hs _ hW, hc.mono hs⟩

/-! ## Coreachability inside a set -/

/-- Mathematical: `q` reaches a marked state along a path that stays in `U`. -/
inductive CoreachIn (p : Plant) (U : List Nat) : Nat → Prop
  | base {q : Nat} : q ∈ U → q ∈ p.marked → CoreachIn p U q
  | step {q e q' : Nat} : q ∈ U → (⟨q, e, q'⟩ : Edge) ∈ p.edges →
      CoreachIn p U q' → CoreachIn p U q

theorem CoreachIn.mem {p : Plant} {U : List Nat} {q : Nat} (h : CoreachIn p U q) : q ∈ U := by
  cases h <;> assumption

theorem CoreachIn.mono {p : Plant} {U U' : List Nat} {q : Nat} (hs : Sub U U')
    (h : CoreachIn p U q) : CoreachIn p U' q := by
  induction h with
  | base hq hm => exact .base (hs _ hq) hm
  | step hq he _ ih => exact .step (hs _ hq) he ih

/-- States of `U` with a successor (any event) in `S`. -/
def preIn (p : Plant) (U S : List Nat) : List Nat :=
  U.filter fun q => p.edges.any fun e => e.src == q && S.contains e.dst

theorem mem_preIn {p : Plant} {U S : List Nat} {q : Nat} :
    q ∈ preIn p U S ↔ q ∈ U ∧ ∃ e q', (⟨q, e, q'⟩ : Edge) ∈ p.edges ∧ q' ∈ S := by
  simp only [preIn, List.mem_filter, List.any_eq_true, Bool.and_eq_true, beq_iff_eq,
    contains_iff]
  constructor
  · rintro ⟨hU, ⟨⟨s, e, d⟩, he, hsrc, hd⟩⟩
    simp only at hsrc hd
    subst hsrc
    exact ⟨hU, e, d, he, hd⟩
  · rintro ⟨hU, e, q', he, hq'⟩
    exact ⟨hU, ⟨q, e, q'⟩, he, rfl, hq'⟩

/-- Marked states inside `U` (the seed of the backward iteration). -/
def markedIn (p : Plant) (U : List Nat) : List Nat := U.filter (fun q => p.marked.contains q)

theorem mem_markedIn {p : Plant} {U : List Nat} {q : Nat} :
    q ∈ markedIn p U ↔ q ∈ U ∧ q ∈ p.marked := by
  simp [markedIn, List.mem_filter]

def coreachStep (p : Plant) (U S : List Nat) : List Nat :=
  S ++ (preIn p U S).filter (fun q => !S.contains q)

def coreachIter (p : Plant) (U : List Nat) : Nat → List Nat → List Nat
  | 0, S => S
  | n + 1, S => if subB (preIn p U S) S then S else coreachIter p U n (coreachStep p U S)

/-- Candidate coreachable set (checked afterwards by `coreachClosed`). -/
def coreachSet (p : Plant) (U : List Nat) : List Nat :=
  coreachIter p U (p.numStates + 1) (markedIn p U)

/-- Certificate check: `S ⊇ marked ∩ U` and `S` is closed under predecessors in `U`. -/
def coreachClosed (p : Plant) (U S : List Nat) : Bool :=
  subB (markedIn p U) S && subB (preIn p U S) S

/-- A closed set contains every coreachable state (completeness). -/
theorem coreachClosed_complete {p : Plant} {U S : List Nat}
    (h : coreachClosed p U S = true) : ∀ q, CoreachIn p U q → q ∈ S := by
  simp only [coreachClosed, Bool.and_eq_true, subB_iff] at h
  intro q hc
  induction hc with
  | base hq hm => exact h.1 _ (mem_markedIn.2 ⟨hq, hm⟩)
  | step hq he _ ih => exact h.2 _ (mem_preIn.2 ⟨hq, _, _, he, ih⟩)

theorem markedIn_sound {p : Plant} {U : List Nat} : ∀ q, q ∈ markedIn p U → CoreachIn p U q :=
  fun _ h => let ⟨hU, hm⟩ := mem_markedIn.1 h; .base hU hm

theorem coreachStep_sound {p : Plant} {U S : List Nat}
    (hS : ∀ q, q ∈ S → CoreachIn p U q) : ∀ q, q ∈ coreachStep p U S → CoreachIn p U q := by
  intro q hq
  simp only [coreachStep, List.mem_append, List.mem_filter] at hq
  rcases hq with hq | ⟨hq, _⟩
  · exact hS q hq
  · obtain ⟨hU, e, q', he, hq'⟩ := mem_preIn.1 hq
    exact .step hU he (hS q' hq')

theorem coreachIter_sound {p : Plant} {U : List Nat} (n : Nat) {S : List Nat}
    (hS : ∀ q, q ∈ S → CoreachIn p U q) : ∀ q, q ∈ coreachIter p U n S → CoreachIn p U q := by
  induction n generalizing S with
  | zero => exact hS
  | succ n ih =>
    intro q hq
    simp only [coreachIter] at hq
    split at hq
    · exact hS q hq
    · exact ih (coreachStep_sound hS) q hq

/-- Everything the iteration returns is coreachable (soundness). -/
theorem coreachSet_sound {p : Plant} {U : List Nat} :
    ∀ q, q ∈ coreachSet p U → CoreachIn p U q :=
  coreachIter_sound _ markedIn_sound

/-! ## The envelope operator -/

/-- Mathematical membership in `F(W)`. -/
def FMem (p : Plant) (nb : Bool) (W : List Nat) (q : Nat) : Prop :=
  q ∈ W ∧ UcClosedAt p W q ∧ (nb = true → CoreachIn p (ucFilter p W) q)

theorem FMem.mono {p : Plant} {nb : Bool} {W W' : List Nat} {q : Nat} (hs : Sub W W')
    (h : FMem p nb W q) : FMem p nb W' q :=
  ⟨hs _ h.1, h.2.1.mono hs, fun hnb => (h.2.2 hnb).mono (ucFilter_mono hs)⟩

/-- Executable `F`. -/
def envStep (p : Plant) (nb : Bool) (W : List Nat) : List Nat :=
  let U := ucFilter p W
  if nb then coreachSet p U else U

/-- One certified step: `W' = F(W)` as sets. -/
def stepOk (p : Plant) (nb : Bool) (W W' : List Nat) : Bool :=
  let U := ucFilter p W
  let V := if nb then coreachSet p U else U
  (!nb || coreachClosed p U V) && setEq V W'

theorem stepOk_iff {p : Plant} {nb : Bool} {W W' : List Nat} (h : stepOk p nb W W' = true) :
    ∀ q, q ∈ W' ↔ FMem p nb W q := by
  intro q
  cases nb with
  | false =>
    simp only [stepOk, Bool.not_false, Bool.true_or, Bool.true_and, setEq_iff] at h
    constructor
    · intro hq
      obtain ⟨hW, hc⟩ := mem_ucFilter.1 (h.2 _ hq)
      exact ⟨hW, hc, fun hf => by cases hf⟩
    · intro hf
      exact h.1 _ (mem_ucFilter.2 ⟨hf.1, hf.2.1⟩)
  | true =>
    simp only [stepOk, Bool.not_true, Bool.false_or, if_true, Bool.and_eq_true, setEq_iff] at h
    obtain ⟨hcl, hVW, hWV⟩ := h
    constructor
    · intro hq
      have hc := coreachSet_sound _ (hWV _ hq)
      obtain ⟨hW, hu⟩ := mem_ucFilter.1 hc.mem
      exact ⟨hW, hu, fun _ => hc⟩
    · intro hf
      exact hVW _ (coreachClosed_complete hcl _ (hf.2.2 rfl))

/-- `Q \ B`, the starting set. -/
def initialW (p : Plant) : List Nat := p.allStates.filter (fun q => !p.bad.contains q)

theorem mem_initialW {p : Plant} {q : Nat} :
    q ∈ initialW p ↔ q < p.numStates ∧ q ∉ p.bad := by
  simp [initialW, Plant.allStates, List.mem_filter, List.mem_range]

/-- Check `W₀ → W₁ → … → Wₙ` and `Wₙ = F(Wₙ)`. -/
def checkChain (p : Plant) (nb : Bool) : List Nat → List (List Nat) → Bool
  | W, [] => stepOk p nb W W
  | W, W' :: rest => stepOk p nb W W' && checkChain p nb W' rest

/-- The last element of a non-empty chain (`[]` for an empty one). -/
def finalW : List (List Nat) → List Nat
  | [] => []
  | [W] => W
  | _ :: W' :: rest => finalW (W' :: rest)

/-- **Envelope certificate check.** -/
def checkEnvelope (p : Plant) (nb : Bool) : List (List Nat) → Bool
  | [] => false
  | W₀ :: rest => setEq W₀ (initialW p) && checkChain p nb W₀ rest

/-- Controllable transitions the supervisor must disable to stay in `W`. -/
def disabled (p : Plant) (W : List Nat) : List Edge :=
  p.edges.filter fun e => W.contains e.src && p.controllable.contains e.ev && !W.contains e.dst

/-- A *good* set: safe, uncontrollably closed and (nonblocking mode) coreachable
within itself. Every supervisor that satisfies the objective has a good set
of winning states, so maximality among good sets is the right notion. -/
def Good (p : Plant) (nb : Bool) (W : List Nat) : Prop :=
  (∀ q, q ∈ W → q < p.numStates ∧ q ∉ p.bad) ∧
  (∀ q, q ∈ W → UcClosedAt p W q) ∧
  (nb = true → ∀ q, q ∈ W → CoreachIn p W q)

/-- A good set is a post-fixed point of `F` relative to any superset. -/
theorem Good.fmem {p : Plant} {nb : Bool} {W' W : List Nat} (hG : Good p nb W')
    (hs : Sub W' W) : ∀ q, q ∈ W' → FMem p nb W q := by
  intro q hq
  refine ⟨hs _ hq, (hG.2.1 q hq).mono hs, fun hnb => ?_⟩
  refine (hG.2.2 hnb q hq).mono ?_
  intro x hx
  exact mem_ucFilter.2 ⟨hs _ hx, (hG.2.1 x hx).mono hs⟩

theorem chain_fix {p : Plant} {nb : Bool} : ∀ (W : List Nat) (rest : List (List Nat)),
    checkChain p nb W rest = true →
    ∀ q, q ∈ finalW (W :: rest) ↔ FMem p nb (finalW (W :: rest)) q
  | W, [], h => by
    simp only [finalW]
    exact stepOk_iff h
  | W, W' :: rest, h => by
    simp only [checkChain, Bool.and_eq_true] at h
    simp only [finalW]
    exact chain_fix W' rest h.2

theorem chain_sub {p : Plant} {nb : Bool} : ∀ (W : List Nat) (rest : List (List Nat)),
    checkChain p nb W rest = true → Sub (finalW (W :: rest)) W
  | W, [], _ => by
    simp only [finalW]
    exact Sub.refl W
  | W, W' :: rest, h => by
    simp only [checkChain, Bool.and_eq_true] at h
    simp only [finalW]
    have h1 : Sub W' W := fun q hq => ((stepOk_iff h.1 q).1 hq).1
    exact (chain_sub W' rest h.2).trans h1

theorem chain_maximal {p : Plant} {nb : Bool} {G : List Nat} (hG : Good p nb G) :
    ∀ (W : List Nat) (rest : List (List Nat)),
    checkChain p nb W rest = true → Sub G W → Sub G (finalW (W :: rest))
  | W, [], _, hs => by
    simp only [finalW]
    exact hs
  | W, W' :: rest, h, hs => by
    simp only [checkChain, Bool.and_eq_true] at h
    simp only [finalW]
    have hs' : Sub G W' := fun q hq => (stepOk_iff h.1 q).2 (hG.fmem hs q hq)
    exact chain_maximal hG W' rest h.2 hs'

/-- **Main theorem.** A checked envelope certificate yields a good final set
that is maximal among all good sets. -/
theorem checkEnvelope_sound {p : Plant} {nb : Bool} {cert : List (List Nat)}
    (h : checkEnvelope p nb cert = true) :
    Good p nb (finalW cert) ∧ ∀ G, Good p nb G → Sub G (finalW cert) := by
  cases cert with
  | nil => simp [checkEnvelope] at h
  | cons W₀ rest =>
    simp only [checkEnvelope, Bool.and_eq_true, setEq_iff] at h
    obtain ⟨⟨hW₀I, hIW₀⟩, hchain⟩ := h
    have hfix := chain_fix W₀ rest hchain
    have hsub := chain_sub W₀ rest hchain
    refine ⟨⟨?_, ?_, ?_⟩, ?_⟩
    · intro q hq
      exact mem_initialW.1 (hW₀I _ (hsub _ hq))
    · intro q hq
      exact ((hfix q).1 hq).2.1
    · intro hnb q hq
      exact (((hfix q).1 hq).2.2 hnb).mono ucFilter_sub
    · intro G hG
      have hGW₀ : Sub G W₀ := fun q hq =>
        hIW₀ _ (mem_initialW.2 (hG.1 q hq))
      exact chain_maximal hG W₀ rest hchain hGW₀

end Mulu.Core
