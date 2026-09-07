import EvmYul.Yul.Interpreter
import EvmYul.Yul.YulNotation
import MuluDiff.Runner

/-!
# Two defects in the adopted semantics

`semantics:evmyul-matches-the-evm` is an assumption, not a proof, and this
file is what an assumption looks like when someone checks it. Each case below
is a Yul program whose meaning the specification fixes, run through EvmYul,
with what the specification says next to what came out.

Where they differ, **EvmYul is wrong**. These are not two defensible readings
of an ambiguous document. The Yul specification says a switch takes "the
branch corresponding to the matching constant", and that the default "is taken
if none of the literal constants matches". Both cases below contradict that in
the direction of the implementation, and the first is asymmetric in a way no
intended semantics would be: each case's error is recorded per case while the
default's escapes.

What follows from that is not that mulu works around them. It is that a
contract containing the first construct cannot be read in this semantics at
all, so `mulu-yul/src/lean.rs` refuses to compare one and records it against
the obligation it defeats. The second never arises from solc, which always
writes `default {}` itself.

Run with `make semantics-probe`. A case that starts agreeing is as
interesting as one that starts disagreeing, so the expected strings are
pinned and the probe fails if any of them changes.
-/

open EvmYul EvmYul.Yul EvmYul.Yul.Ast

namespace MuluDiff.Probe

private def noFunctions : Finmap (fun (_ : YulFunctionName) ↦ FunctionDefinition) := ∅

/-- **A matching case does not stop the default from running.**

Yul: "the case whose value matches is executed; if none matches, the default
case is executed." Here case 1 matches, so `sstore(0, 7)` runs and nothing
reverts.

EvmYul's `exec` (`Interpreter.lean`, the `.Switch` arm) runs the default
branch *before* selecting, and propagates its error rather than recording it
per case the way it records the cases' own errors. A default that reverts
therefore reverts the whole switch. -/
def matchedCaseWithRevertingDefault : YulContract where
  dispatcher :=
    <s {
      switch 1
      case 1 { sstore(0, 7) }
      default { revert(0, 0) }
    } >
  functions := noFunctions

/-- **A switch with no `default` is given `default { break }` by the notation.**

Yul: an unmatched switch with no default does nothing, so `sstore(1, 3)`
stands. The notation (`YulNotation.lean`, the `switch` elaborator) supplies
`[.Break]` when there is no default, and a `break` is not nothing: the state
that comes back is a `Checkpoint`, and the earlier write is lost. -/
def unmatchedNoDefault : YulContract where
  dispatcher :=
    <s {
      sstore(1, 3)
      switch 9
      case 1 { sstore(0, 7) }
    } >
  functions := noFunctions

/-- The same program with an explicit empty `default`, which is what solc
writes and therefore what mulu renders. It agrees with the specification, so
the second defect does not reach anything compiled from Solidity. -/
def unmatchedEmptyDefault : YulContract where
  dispatcher :=
    <s {
      sstore(1, 3)
      switch 9
      case 1 { sstore(0, 7) }
      default { }
    } >
  functions := noFunctions

/-- The same again, built from the AST constructors rather than the notation,
with an empty default list. It agrees, which locates the second divergence in
the notation's elaborator and not in `exec`. -/
def unmatchedAstDirect : YulContract where
  dispatcher :=
    .Block [ .ExprStmtCall (.Call (Sum.inl .SSTORE) [.Lit ⟨1⟩, .Lit ⟨3⟩])
           , .Switch (.Lit ⟨9⟩)
               [(⟨1⟩, [.ExprStmtCall (.Call (Sum.inl .SSTORE) [.Lit ⟨0⟩, .Lit ⟨7⟩])])]
               [] ]
  functions := noFunctions

/-- `(name, what the Yul specification says, the contract)`. -/
def cases : List (String × String × YulContract) :=
  [ ("matching case, reverting default", "ok 0=7", matchedCaseWithRevertingDefault)
  , ("unmatched, no default (notation)", "ok 1=3", unmatchedNoDefault)
  , ("unmatched, empty default (what solc writes)", "ok 1=3", unmatchedEmptyDefault)
  , ("unmatched, empty default list (AST)", "ok 1=3", unmatchedAstDirect) ]

/-- What EvmYul actually does, today. Pinned so that a change in either
direction is visible. -/
def observed : List String :=
  [ "revert ", "ok ", "ok 1=3", "ok 1=3" ]

def run : IO UInt32 := do
  let mut bad : UInt32 := 0
  for ((name, spec, c), expected) in cases.zip observed do
    let got := (MuluDiff.run c 0x11 [] [ByteArray.mk #[]]).head!
    -- drop the leading transaction index
    let got := (got.splitOn " ").drop 1 |> String.intercalate " "
    let agrees := got == spec
    IO.println s!"{if agrees then "  agrees" else "DIFFERS"}  {name}"
    IO.println s!"           Yul says: {spec}"
    IO.println s!"           EvmYul:   {got}"
    if got != expected then
      IO.println s!"           CHANGED: this probe expected {expected}"
      bad := bad + 1
  if bad == 0 then
    IO.println "\nEvery case behaves as this probe recorded. Two of them are still wrong."
  return bad

end MuluDiff.Probe
