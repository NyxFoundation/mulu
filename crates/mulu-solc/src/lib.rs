//! `mulu-solc` — drive `solc` through its Standard JSON interface and capture
//! everything later stages need, with the inputs hashed so a result can be
//! tied to the exact bytes it came from.
//!
//! Two rules this module exists to enforce (docs/09 §2.1, docs/08 §5):
//!
//! * the compiler's **exit code is not the success criterion**. solc can exit
//!   0 while reporting `severity: "error"` inside the JSON, so the output is
//!   scanned for errors independently;
//! * the **analysed artifact is always named**. We request unoptimized Yul
//!   (`ir`) and record the settings that produced it, so a result derived from
//!   it is never confused with a statement about the deployed bytecode.

pub mod ast;
mod bundle;
mod driver;
pub mod imports;

pub use ast::{AstIndex, AstKind, AstNode};
pub use bundle::{BuildBundle, ContractArtifact, SourceFile};
pub use driver::{select_contract as driver_select, CompileOptions, Solc, SolcError};
