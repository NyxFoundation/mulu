import Lake
open Lake DSL

/-!
mulu — Lean side.

`Mulu` is the Solidity-independent finite supervisory-control core:
definitions, executable algorithms, certificate checkers and the theorems
that connect checker success to the mathematical claims.
It deliberately has **no external dependencies** (no mathlib), so that the
trusted base is Lean core only and `lake build` works offline.

`mulu-worker` is the JSON subprocess that the Rust CLI drives.
-/

package «mulu» where
  leanOptions := #[
    ⟨`autoImplicit, false⟩,
    ⟨`relaxedAutoImplicit, false⟩
  ]

@[default_target]
lean_lib «Mulu» where
  globs := #[.andSubmodules `Mulu]

@[default_target]
lean_exe «mulu-worker» where
  root := `Main
