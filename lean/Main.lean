import Mulu.Worker.Protocol

/-!
`mulu-worker` — one JSON request on stdin, one JSON response on stdout.

    mulu-worker --dir <analysis-dir>
-/

open Lean (Json fromJson?)
open Mulu.Worker Mulu.Core Mulu.Analysis

def readJsonFile (path : System.FilePath) : IO (Except String Json) := do
  let s ← IO.FS.readFile path
  pure (Json.parse s)

def resolve (dir : System.FilePath) (rel : String) : Except String System.FilePath :=
  if rel.startsWith "/" || (rel.splitOn "/").contains ".." then
    .error s!"path must be relative to the analysis directory and must not escape it: {rel}"
  else .ok (dir / rel)

def run (dir : System.FilePath) (input : String) : IO Json := do
  let req ← match Json.parse input >>= fromJson? (α := Request) with
    | .ok r => pure r
    | .error e => return errorResponse "?" s!"bad request: {e}"
  if req.protocolVersion != 1 then
    return errorResponse req.requestId s!"unsupported protocol_version {req.protocolVersion}"
  let mpath ← match resolve dir req.modelPath with
    | .ok p => pure p
    | .error e => return errorResponse req.requestId e
  let model ← match ← readJsonFile mpath with
    | .ok j => match fromJson? (α := CoreModel) j with
      | .ok m => pure m
      | .error e => return errorResponse req.requestId s!"bad model: {e}"
    | .error e => return errorResponse req.requestId s!"bad model json: {e}"
  if !model.plant.wellFormed then
    return errorResponse req.requestId "model is not well-formed (index out of range)"
  match req.method with
  | "analyze" => pure (analyze model req)
  | "verify" =>
    let some cpath := req.certificatePath
      | return errorResponse req.requestId "verify needs certificate_path"
    let cp ← match resolve dir cpath with
      | .ok p => pure p
      | .error e => return errorResponse req.requestId e
    match ← readJsonFile cp with
    | .ok j => match fromJson? (α := Certificate) j with
      | .ok c => pure (verify model req c)
      | .error e => return errorResponse req.requestId s!"bad certificate: {e}"
    | .error e => return errorResponse req.requestId s!"bad certificate json: {e}"
  | m => pure (errorResponse req.requestId s!"unknown method {m}")

def main (args : List String) : IO UInt32 := do
  let dir : System.FilePath := match args with
    | ["--dir", d] => d
    | _ => "."
  let stdin ← IO.getStdin
  let input ← stdin.readToEnd
  let out ← run dir input
  IO.println out.compress
  pure 0
