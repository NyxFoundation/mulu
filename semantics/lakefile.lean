import Lake
open Lake DSL

/-!
mulu — the correspondence side.

This package exists to state, in Lean, what the analyser's findings would have
to satisfy to be about a program rather than about a model. Stating that needs
a semantics of the Yul solc emits, and one already exists: Nethermind's
`EvmYul`, which Paradigm's Solidus pins for its verified backend.

It is a **separate package on purpose**. `EvmYul` requires mathlib, so its
dependency closure is several gigabytes and needs the network. The analyser
that `mulu verify` runs must build offline from Lean core alone, because the
whole point of the kernel re-check is that a reader can reproduce it without
trusting a supply chain. Keeping the two apart means adopting a semantics
costs the correspondence proofs their build time and costs the checker
nothing.

Nothing here is on the path of `mulu analyze` or `mulu verify`.
-/

package «mulu-semantics» where
  leanOptions := #[⟨`autoImplicit, false⟩, ⟨`relaxedAutoImplicit, false⟩]

require mulu from ".." / "lean"

-- Pinned to the Solidus fork: it is the copy a verified compiler depends on,
-- so it is the copy most likely to be looked at. It is currently identical to
-- NethermindEth/EVMYulLean, from which it is forked.
require evmyul from git
  "https://github.com/paradigmxyz/EVMYulLean.git" @ "main"

@[default_target]
lean_lib «MuluSemantics» where
  globs := #[.andSubmodules `MuluSemantics]

/-- Built on demand by `mulu semantics-diff`, which writes `MuluDiff/Generated.lean`
first. Neither is a default target: without that file there is nothing to run,
and `make check` must not need this package at all. -/
lean_lib «MuluDiff» where
  globs := #[.andSubmodules `MuluDiff]

lean_exe «mulu-diff» where
  root := `MuluDiff.Main
