import MuluDiff.Runner
import MuluDiff.Generated

/-- Prints one line per transaction of the generated scenario. Generated.lean
is written by `mulu semantics-diff` and is not checked in: it holds one
contract and one list of calls. -/
def main : IO Unit := do
  for line in MuluDiff.run MuluDiff.Generated.contract MuluDiff.Generated.caller
                MuluDiff.Generated.initialStorage MuluDiff.Generated.scenario do
    IO.println line
