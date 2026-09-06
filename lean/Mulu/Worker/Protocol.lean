import Lean.Data.Json
import Mulu.Analysis.Certificate

/-!
# Worker protocol (JSON)

One request per process. stdin: one JSON object. stdout: one JSON object.
stderr: logs. Paths are resolved relative to the analysis directory
(`--dir`), never above it.

The model read here is the *normalised* core model produced by the Rust
side (`core-model.json`): integer indices only. The mapping back to state
and event names lives in the analysis manifest.
-/

namespace Mulu.Worker
open Lean (Json FromJson ToJson toJson fromJson?)
open Mulu.Core Mulu.Analysis

/-! ## Core model -/

structure CoreModel where
  plant : Plant
  checks : List Check
deriving Inhabited

def parseEdge (j : Json) : Except String Edge := do
  let a ← fromJson? (α := List Nat) j
  match a with
  | [s, e, d] => pure ⟨s, e, d⟩
  | _ => throw s!"edge must be [src, event, dst], got {j.compress}"

instance : FromJson Check where
  fromJson? j := do
    let id ← j.getObjValAs? Nat "id"
    let pe ← j.getObjValAs? Nat "pass_event"
    let fe ← j.getObjValAs? Nat "fail_event"
    pure ⟨id, pe, fe⟩

instance : ToJson Check where
  toJson c := Json.mkObj [("id", toJson c.id), ("pass_event", toJson c.passEvent),
    ("fail_event", toJson c.failEvent)]

def edgeJson (e : Edge) : Json := toJson [e.src, e.ev, e.dst]

instance : FromJson CoreModel where
  fromJson? j := do
    let numStates ← j.getObjValAs? Nat "num_states"
    let numEvents ← j.getObjValAs? Nat "num_events"
    let controllable ← j.getObjValAs? (List Nat) "controllable"
    let initial ← j.getObjValAs? (List Nat) "initial"
    let marked ← j.getObjValAs? (List Nat) "marked"
    let bad ← j.getObjValAs? (List Nat) "bad"
    let edgesJ ← j.getObjValAs? (List Json) "edges"
    let edges ← edgesJ.mapM parseEdge
    let checks ← match j.getObjVal? "checks" with
      | .ok cj => fromJson? (α := List Check) cj
      | .error _ => pure []
    pure { plant := { numStates, numEvents, controllable, initial, marked, bad, edges }, checks }

/-! ## Certificates -/

instance : FromJson Certificate where
  fromJson? j := do
    let kind ← j.getObjValAs? String "kind"
    match kind with
    | "reachability" =>
      let R ← j.getObjValAs? (List Nat) "states"
      pure (.reachability R)
    | "redundancy" =>
      let R ← j.getObjValAs? (List Nat) "states"
      let c ← j.getObjValAs? Check "check"
      pure (.redundancy R c)
    | "violation" =>
      let pj ← j.getObjValAs? (List Json) "path"
      let path ← pj.mapM parseEdge
      pure (.violation path)
    | "envelope" =>
      let nb ← j.getObjValAs? Bool "nonblocking"
      let chain ← j.getObjValAs? (List (List Nat)) "chain"
      pure (.envelope nb chain)
    | k => throw s!"unknown certificate kind: {k}"

instance : ToJson Certificate where
  toJson
    | .reachability R => Json.mkObj [("kind", "reachability"), ("states", toJson R)]
    | .redundancy R c => Json.mkObj [("kind", "redundancy"), ("states", toJson R), ("check", toJson c)]
    | .violation path => Json.mkObj [("kind", "violation"), ("path", toJson (path.map edgeJson))]
    | .envelope nb chain => Json.mkObj [("kind", "envelope"), ("nonblocking", toJson nb),
        ("chain", toJson chain)]

/-! ## Algorithms that only *produce* candidates (never trusted) -/

/-- Iterate `F` from `Q \ B`, recording every distinct set. -/
def envelopeChain (p : Plant) (nb : Bool) : List (List Nat) :=
  go (p.numStates + 1) (initialW p)
where
  go : Nat → List Nat → List (List Nat)
    | 0, W => [W]
    | n + 1, W =>
      let W' := envStep p nb W
      if setEq W' W then [W] else W :: go n W'

/-- Breadth-first search for a path from an initial state to a bad state. -/
partial def findViolation (p : Plant) : Option (List Edge) :=
  let rec loop (frontier : List Nat) (parent : List (Nat × Edge)) (seen : List Nat) (fuel : Nat) :
      Option (List Edge) :=
    match fuel with
    | 0 => none
    | fuel + 1 =>
      match frontier with
      | [] => none
      | q :: rest =>
        if p.bad.contains q then some (rebuild parent q [])
        else
          let out := p.edges.filter (fun e => e.src == q && !seen.contains e.dst)
          let newSeen := seen ++ out.map (·.dst)
          let newParent := parent ++ out.map (fun e => (e.dst, e))
          loop (rest ++ out.map (·.dst)) newParent newSeen fuel
  if p.initial.any (p.bad.contains ·) then some []  -- initial state itself is bad
  else loop p.initial [] p.initial (p.numStates * p.edges.length + p.numStates + 1)
where
  rebuild (parent : List (Nat × Edge)) (q : Nat) (acc : List Edge) : List Edge :=
    match parent.find? (·.1 == q) with
    | none => acc
    | some (_, e) =>
      if acc.length > parent.length then acc else rebuild parent e.src (e :: acc)

/-! ## Request / response -/

structure Limits where
  maxStates : Nat := 100000
  maxEdges : Nat := 1000000
deriving Inhabited

structure Request where
  protocolVersion : Nat
  requestId : String
  method : String
  modelPath : String
  analyses : List String := ["reachability", "redundancy", "safety", "envelope"]
  objective : String := "safety-nonblocking"
  certificatePath : Option String := none
  limits : Limits := {}
deriving Inhabited

instance : FromJson Limits where
  fromJson? j := do
    let ms := (j.getObjValAs? Nat "max_states").toOption.getD 100000
    let me := (j.getObjValAs? Nat "max_edges").toOption.getD 1000000
    pure { maxStates := ms, maxEdges := me }

instance : FromJson Request where
  fromJson? j := do
    let protocolVersion ← j.getObjValAs? Nat "protocol_version"
    let requestId ← j.getObjValAs? String "request_id"
    let method ← j.getObjValAs? String "method"
    let modelPath ← j.getObjValAs? String "model_path"
    let analyses := (j.getObjValAs? (List String) "analyses").toOption.getD
      ["reachability", "redundancy", "safety", "envelope"]
    let objective := (j.getObjValAs? String "objective").toOption.getD "safety-nonblocking"
    let certificatePath := (j.getObjValAs? String "certificate_path").toOption
    let limits := (j.getObjValAs? Limits "limits").toOption.getD {}
    pure { protocolVersion, requestId, method, modelPath, analyses, objective, certificatePath, limits }

def statusJson (s : String) : (String × Json) := ("status", s)

/-- Over budget: the analyses are declined rather than run and labelled.
Labelling a completed run `partial` while still handing back certificates
invites a reader to take the certificate and leave the label, and running a
model larger than the caller allowed is not a smaller answer, it is a
different one. -/
def declined (req : Request) (reason : String) : Json :=
  let one := Json.mkObj [statusJson "partial", ("reason", Json.str reason)]
  Json.mkObj [("protocol_version", 1), ("request_id", req.requestId),
    ("status", "partial"),
    ("analyses", Json.mkObj (req.analyses.map (fun a => (a, one)))),
    ("statistics", Json.mkObj []),
    ("cutoff_reason", "limits")]

/-- Run the analyses. Every produced certificate is re-checked with
`checkCertificate` before it is reported as checked. -/
def analyze (m : CoreModel) (req : Request) : Json := Id.run do
  let p := m.plant
  let mut fields : List (String × Json) := []
  if p.numStates > req.limits.maxStates then
    return declined req
      s!"the model has {p.numStates} states and the limit is {req.limits.maxStates}; nothing was analysed"
  if p.edges.length > req.limits.maxEdges then
    return declined req
      s!"the model has {p.edges.length} edges and the limit is {req.limits.maxEdges}; nothing was analysed"
  -- Past this point the model is within budget, so no analysis below has a
  -- size reason to stop: a `partial` there means the check itself failed.
  let want (a : String) := req.analyses.contains a
  -- reachability
  let R := reachSet p
  let reachCert : Certificate := .reachability R
  let reachOk := checkCertificate p reachCert
  if want "reachability" then
    fields := fields ++ [("reachability", Json.mkObj [
      statusJson (if reachOk then "complete" else "partial"),
      ("states", toJson R),
      ("certificate", toJson reachCert),
      ("checked", toJson reachOk)])]
  -- safety: direct search for a bad state
  if want "safety" then
    if p.bad.isEmpty then
      fields := fields ++ [("safety", Json.mkObj [statusJson "not-requested",
        ("reason", "model has no bad states (no specification)")])]
    else
      match findViolation p with
      | some path =>
        let cert : Certificate := .violation path
        let ok := checkCertificate p cert
        fields := fields ++ [("safety", Json.mkObj [
          statusJson (if ok then "complete" else "error"),
          ("violation", Json.mkObj [("path", toJson (path.map edgeJson)),
            ("certificate", toJson cert), ("checked", toJson ok)])])]
      | none =>
        let safe := reachOk && (R.all fun q => !p.bad.contains q)
        fields := fields ++ [("safety", Json.mkObj [
          statusJson (if safe then "complete" else "partial"),
          ("violation", Json.null),
          ("certificate", toJson reachCert),
          ("checked", toJson safe)])]
  -- redundancy
  if want "redundancy" then
    let items := m.checks.map fun c =>
      let cert : Certificate := .redundancy R c
      let ok := checkCertificate p cert
      let reached := reachedOn p R c
      let witness := (p.edges.find? fun e => e.ev == c.failEvent && R.contains e.src).map (·.src)
      let st := if !reachOk then "unknown"
        else if ok && reached then "never-fails"
        else if ok then "unreachable"
        else "may-fail"
      Json.mkObj [("id", toJson c.id), ("status", st),
        ("certificate", if ok && reached then toJson cert else Json.null),
        ("checked", toJson (ok && reached)),
        ("fail_witness_state", match witness with | some q => toJson q | none => Json.null)]
    fields := fields ++ [("redundancy", Json.mkObj [
      statusJson (if reachOk then "complete" else "partial"),
      ("checks", toJson items)])]
  -- envelope
  if want "envelope" then
    if !p.deterministic then
      fields := fields ++ [("envelope", Json.mkObj [statusJson "unsupported",
        ("reason", "plant is not partially deterministic: same (state, event) has several targets")])]
    else if p.bad.isEmpty && p.marked.isEmpty then
      fields := fields ++ [("envelope", Json.mkObj [statusJson "not-requested",
        ("reason", "no bad and no marked states: nothing to supervise")])]
    else
      let nb := req.objective != "safety"
      let chain := envelopeChain p nb
      let cert : Certificate := .envelope nb chain
      let ok := checkCertificate p cert
      let W := finalW chain
      let realizable := p.initial.any (W.contains ·)
      let st := if !ok then "partial"
        else if realizable then "complete" else "unrealizable"
      fields := fields ++ [("envelope", Json.mkObj [
        statusJson st,
        ("objective", if nb then "safety-nonblocking" else "safety"),
        ("winning", toJson W),
        ("disabled", toJson ((disabled p W).map edgeJson)),
        ("pruning_rounds", toJson (chain.length - 1)),
        ("certificate", toJson cert),
        ("checked", toJson ok)])]
  let stats := Json.mkObj [("states", toJson p.numStates), ("edges", toJson p.edges.length),
    ("events", toJson p.numEvents), ("reachable_states", toJson R.length)]
  return Json.mkObj [("protocol_version", 1), ("request_id", req.requestId),
    ("status", "complete"),
    ("analyses", Json.mkObj fields), ("statistics", stats)]

def verify (m : CoreModel) (req : Request) (cert : Certificate) : Json :=
  let ok := checkCertificate m.plant cert
  Json.mkObj [("protocol_version", 1), ("request_id", req.requestId),
    ("status", if ok then "complete" else "error"),
    ("checked", toJson ok), ("certificate", toJson cert),
    ("note", "checked by the compiled worker; the kernel route is certificates/Check.lean")]

def errorResponse (id : String) (msg : String) : Json :=
  Json.mkObj [("protocol_version", 1), ("request_id", id), ("status", "error"), ("error", msg)]

end Mulu.Worker
