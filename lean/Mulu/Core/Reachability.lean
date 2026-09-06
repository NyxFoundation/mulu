import Mulu.Core.FinitePlant

/-!
# Reachability — least fixed point

`Reachable p q` is the mathematical reachable set
`μX. initial ∪ Post(X)`.

The executable side has two parts:

* `reachSet` — a worklist-free saturation that *computes* a candidate
  (sound: everything it returns is reachable);
* `isClosed` — the *certificate check* `initial ⊆ R ∧ Post(R) ⊆ R`, whose
  success implies that `R` covers the reachable set (`closed_sound`).

A report never relies on the numbers alone: it relies on `isClosed R = true`
plus `closed_sound`, and on `reachIter_sound` for the minimality direction.
-/

namespace Mulu.Core

/-- Mathematical reachability. -/
inductive Reachable (p : Plant) : Nat → Prop
  | init {q : Nat} : q ∈ p.initial → Reachable p q
  | step {q e q' : Nat} : Reachable p q → (⟨q, e, q'⟩ : Edge) ∈ p.edges → Reachable p q'

/-- Successors of one state. -/
def succs (p : Plant) (q : Nat) : List Nat :=
  (p.edges.filter (fun e => e.src == q)).map (·.dst)

/-- `Post(R)`. -/
def post (p : Plant) (R : List Nat) : List Nat := R.flatMap (succs p)

theorem mem_succs {p : Plant} {q e q' : Nat} (h : (⟨q, e, q'⟩ : Edge) ∈ p.edges) :
    q' ∈ succs p q := by
  simp only [succs, List.mem_map, List.mem_filter]
  exact ⟨⟨q, e, q'⟩, ⟨h, by simp⟩, rfl⟩

theorem succs_mem {p : Plant} {q q' : Nat} (h : q' ∈ succs p q) :
    ∃ e, (⟨q, e, q'⟩ : Edge) ∈ p.edges := by
  simp only [succs, List.mem_map, List.mem_filter] at h
  obtain ⟨⟨s, e, d⟩, ⟨hmem, hsrc⟩, hdst⟩ := h
  simp at hsrc hdst
  subst hsrc; subst hdst
  exact ⟨e, hmem⟩

theorem mem_post {p : Plant} {R : List Nat} {q e q' : Nat}
    (hq : q ∈ R) (h : (⟨q, e, q'⟩ : Edge) ∈ p.edges) : q' ∈ post p R := by
  simp only [post, List.mem_flatMap]
  exact ⟨q, hq, mem_succs h⟩

theorem post_mem {p : Plant} {R : List Nat} {q' : Nat} (h : q' ∈ post p R) :
    ∃ q e, q ∈ R ∧ (⟨q, e, q'⟩ : Edge) ∈ p.edges := by
  simp only [post, List.mem_flatMap] at h
  obtain ⟨q, hq, hs⟩ := h
  obtain ⟨e, he⟩ := succs_mem hs
  exact ⟨q, e, hq, he⟩

/-- Certificate check for an invariant set: `initial ⊆ R` and `Post(R) ⊆ R`. -/
def isClosed (p : Plant) (R : List Nat) : Bool :=
  subB p.initial R && subB (post p R) R

/-- **Soundness of the invariant certificate**: a closed set covers every
reachable state. -/
theorem closed_sound {p : Plant} {R : List Nat} (h : isClosed p R = true) :
    ∀ q, Reachable p q → q ∈ R := by
  simp only [isClosed, Bool.and_eq_true, subB_iff] at h
  obtain ⟨hI, hP⟩ := h
  intro q hq
  induction hq with
  | init hi => exact hI _ hi
  | step _ he ih => exact hP _ (mem_post ih he)

/-- One saturation step: add the successors not yet present. -/
def reachStep (p : Plant) (R : List Nat) : List Nat :=
  R ++ (post p R).filter (fun x => !R.contains x)

/-- Saturate with fuel; stops early once `Post(R) ⊆ R`. -/
def reachIter (p : Plant) : Nat → List Nat → List Nat
  | 0, R => R
  | n + 1, R => if subB (post p R) R then R else reachIter p n (reachStep p R)

/-- Candidate reachable set. Its closure is *checked* by `isClosed`, not assumed. -/
def reachSet (p : Plant) : List Nat := reachIter p (p.numStates + 1) p.initial

theorem reachStep_sound {p : Plant} {R : List Nat}
    (hR : ∀ q, q ∈ R → Reachable p q) : ∀ q, q ∈ reachStep p R → Reachable p q := by
  intro q hq
  simp only [reachStep, List.mem_append, List.mem_filter] at hq
  rcases hq with hq | ⟨hq, _⟩
  · exact hR q hq
  · obtain ⟨q₀, e, hq₀, he⟩ := post_mem hq
    exact Reachable.step (hR q₀ hq₀) he

/-- **Minimality**: iteration from reachable seeds only produces reachable states. -/
theorem reachIter_sound {p : Plant} (n : Nat) {R : List Nat}
    (hR : ∀ q, q ∈ R → Reachable p q) : ∀ q, q ∈ reachIter p n R → Reachable p q := by
  induction n generalizing R with
  | zero => exact hR
  | succ n ih =>
    intro q hq
    simp only [reachIter] at hq
    split at hq
    · exact hR q hq
    · exact ih (reachStep_sound hR) q hq

theorem reachSet_sound {p : Plant} : ∀ q, q ∈ reachSet p → Reachable p q :=
  reachIter_sound _ (fun _ h => Reachable.init h)

/-- Everything a checked reachable set contains is reachable, and it contains
everything reachable: `R` *is* the reachable set. -/
theorem reachSet_exact {p : Plant} (h : isClosed p (reachSet p) = true) :
    ∀ q, q ∈ reachSet p ↔ Reachable p q :=
  fun q => ⟨reachSet_sound q, closed_sound h q⟩

/-- **Spec-violation witness**: a path from an initial state to a bad state. -/
def checkPath (p : Plant) : List Edge → Bool
  | [] => false
  | e :: rest =>
    p.initial.contains e.src && chain e rest
where
  chain : Edge → List Edge → Bool
    | e, [] => p.edges.contains e && p.bad.contains e.dst
    | e, e' :: rest => p.edges.contains e && e.dst == e'.src && chain e' rest

theorem edge_contains_iff {l : List Edge} {e : Edge} : l.contains e = true ↔ e ∈ l :=
  List.contains_iff_mem

theorem chain_sound {p : Plant} : ∀ (e : Edge) (rest : List Edge),
    checkPath.chain p e rest = true → Reachable p e.src →
    ∃ b, Reachable p b ∧ b ∈ p.bad
  | e, [], h, hr => by
    simp only [checkPath.chain, Bool.and_eq_true, edge_contains_iff, contains_iff] at h
    exact ⟨e.dst, Reachable.step hr h.1, h.2⟩
  | e, e' :: rest, h, hr => by
    simp only [checkPath.chain, Bool.and_eq_true, edge_contains_iff, beq_iff_eq] at h
    obtain ⟨⟨he, hd⟩, hc⟩ := h
    have : Reachable p e'.src := hd ▸ Reachable.step hr he
    exact chain_sound e' rest hc this

/-- A checked path proves that some bad state is reachable. -/
theorem checkPath_sound {p : Plant} {path : List Edge} (h : checkPath p path = true) :
    ∃ b, Reachable p b ∧ b ∈ p.bad := by
  cases path with
  | nil => simp [checkPath] at h
  | cons e rest =>
    simp only [checkPath, Bool.and_eq_true, contains_iff] at h
    exact chain_sound e rest h.2 (Reachable.init h.1)

end Mulu.Core
