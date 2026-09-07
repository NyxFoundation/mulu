import EvmYul.Yul.Interpreter

/-!
# Running a rendered contract, to compare it with an EVM

`semantics:rendering-preserves-the-program` says the module `mulu` renders is
the same program as the Yul it was rendered from, and nothing proves it. This
is the cheap half of the answer: run the rendered module on concrete calls and
compare the storage it produces with what revm produces for the same calls on
the same contract. Agreement is not a proof and never becomes one. Divergence
is a defect, in the renderer, in the abstraction, or in the semantics, and it
is found here rather than inside a claim.

A transaction is the dispatcher run on fresh calldata. Reverting discards the
storage it wrote, which is what `execTopLevel` does with `.Revert` and what an
EVM does with a failed transaction; `.YulHalt` is a `return` or `stop`, so it
keeps its state.
-/

namespace MuluDiff

open EvmYul EvmYul.Yul EvmYul.Yul.Ast

/-- Where the contract under test lives. Any address does; this one is only
ever compared against itself. -/
def addr : AccountAddress := AccountAddress.ofUInt256 ⟨1⟩

def bytes (l : List Nat) : ByteArray := ByteArray.mk (l.map UInt8.ofNat).toArray

/-- A four-byte selector followed by 32-byte big-endian words, which is the
ABI encoding of the P1a subset: at most one static argument per call. -/
def calldataOf (sel : List Nat) (args : List Nat) : ByteArray :=
  let word (n : Nat) : List Nat := (List.range 32).map fun i => n / 256 ^ (31 - i) % 256
  bytes (sel ++ args.flatMap word)

/-- The state a first transaction starts from: the contract deployed at
`addr`, with the storage the deployment left. -/
def start (code : YulContract) (caller : Nat) (storage : List (Nat × Nat)) : Yul.State :=
  let acct : Account .Yul :=
    { code := code
    , balance := ⟨0⟩
    , nonce := ⟨0⟩
    , storage := Batteries.RBMap.ofList (storage.map fun (k, v) => (UInt256.ofNat k, UInt256.ofNat v)) compare
    , tstorage := ∅ }
  Yul.State.Ok
    { accountMap := Batteries.RBMap.insert ∅ addr acct
    , σ₀ := ∅, totalGasUsedInBlock := 0, transactionReceipts := #[]
    , substate := Inhabited.default
    , executionEnv :=
        { calldata := ByteArray.mk #[], code := Inhabited.default, codeOwner := addr
        , source := Inhabited.default, weiValue := ⟨0⟩
        -- `caller()` reads this. It has to be the address the EVM side sends
        -- from, or a contract with access control diverges for a reason that
        -- has nothing to do with the rendering.
        , sender := AccountAddress.ofUInt256 (UInt256.ofNat caller)
        , gasPrice := 0, header := (Inhabited.default : BlockHeader), depth := 0
        , perm := true, blobVersionedHashes := [] }
    , blocks := ∅, genesisBlockHeader := Inhabited.default, createdAccounts := ∅
    , gasAvailable := ⟨0⟩, activeWords := ⟨0⟩
    , memory := ByteArray.mk #[], returnData := ByteArray.mk #[], H_return := ByteArray.mk #[] }
    ∅

/-- Fresh calldata and a fresh variable store: one transaction does not see
another's locals. -/
def withCalldata (s : Yul.State) (cd : ByteArray) : Yul.State :=
  match s with
  | .Ok ss _ => .Ok { ss with executionEnv := { ss.executionEnv with calldata := cd } } ∅
  | other => other

def storageOf (s : Yul.State) : List (Nat × Nat) :=
  match s.sharedState.accountMap.find? addr with
  | .none => []
  | .some a => a.storage.toList.map fun (k, v) => (k.toNat, v.toNat)

/-- One transaction. The state that comes back is the one the next
transaction starts from, so a revert really does undo its writes. -/
def step (code : YulContract) (s : Yul.State) (cd : ByteArray) : Yul.State × String :=
  match exec 5000 code.dispatcher .none (withCalldata s cd) with
  | .ok s₂ => (s₂, "ok")
  | .error (.YulHalt s₂ _) => (s₂, "ok")
  | .error .Revert => (s, "revert")
  | .error e => (s, s!"error({repr e})")

/-- One line per transaction: `<index> <status> <slot>=<value>,…`, which is
what the Rust side compares against revm. -/
def run (code : YulContract) (caller : Nat) (storage : List (Nat × Nat))
    (cds : List ByteArray) : List String :=
  let rec go (i : Nat) (s : Yul.State) : List ByteArray → List String
    | [] => []
    | cd :: rest =>
      let (s', status) := step code s cd
      let cells := (storageOf s').map (fun (k, v) => s!"{k}={v}") |> String.intercalate ","
      s!"{i} {status} {cells}" :: go (i + 1) s' rest
  go 0 (start code caller storage) cds

end MuluDiff
