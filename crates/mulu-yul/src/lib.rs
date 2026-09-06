//! `mulu-yul` — read the unoptimized Yul solc emits, build a control-flow
//! graph per Yul function, and extract the checks with their source locations.
//!
//! This crate does no semantic reasoning. It recognises syntactic shapes,
//! records the criterion it used for each recognition, and marks everything it
//! does not understand as `unsupported` with a location. Deciding what a
//! condition *means* is the next stage's job (P1-02).

pub mod ast;
pub mod builtins;
pub mod ir;
pub mod lex;
pub mod lower;
pub mod parse;

pub use ast::{Block, Expr, FunctionDef, Object, Stmt};
pub use builtins::{Effects, Purity};
pub use ir::{Check, CheckOrigin, Function, Location, ProgramIr, Terminator};
pub use lex::SrcSpan;
pub use parse::{parse_object, ParseError, Parsed};

/// Build ProgramIR from a contract's Yul, ABI and storage layout.
pub fn lower_contract(
    contract: &str,
    source_path: &str,
    compiler: &str,
    yul: &str,
    abi: &serde_json::Value,
    storage_layout: serde_json::Value,
) -> Result<ProgramIr, ParseError> {
    let parsed = parse_object(yul)?;
    let lowering = lower::Lowering::new(contract, source_path, compiler);
    Ok(lowering.run(&parsed.object, parsed.use_src, storage_layout, abi))
}
