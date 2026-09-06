//! `mulu-yul` — read the unoptimized Yul solc emits, build a control-flow
//! graph per Yul function, and extract the checks with their source locations.
//!
//! This crate does no semantic reasoning. It recognises syntactic shapes,
//! records the criterion it used for each recognition, and marks everything it
//! does not understand as `unsupported` with a location. Deciding what a
//! condition *means* is the next stage's job (P1-02).

pub mod ast;
pub mod builtins;
pub mod fold;
pub mod ir;
pub mod lex;
pub mod lower;
pub mod parse;

pub use ast::{Block, Expr, FunctionDef, Object, Stmt};
pub use builtins::{Effects, Purity};
pub use ir::{Check, CheckOrigin, Function, Location, ProgramIr, SourceOrigin, Terminator};
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
    lower_contract_with(contract, source_path, compiler, yul, abi, storage_layout, None)
}

/// As `lower_contract`, with an AST lookup so checks carry the contract and
/// modifier they were written in.
#[allow(clippy::too_many_arguments)]
pub fn lower_contract_with(
    contract: &str,
    source_path: &str,
    compiler: &str,
    yul: &str,
    abi: &serde_json::Value,
    storage_layout: serde_json::Value,
    origins: Option<lower::OriginLookup<'_>>,
) -> Result<ProgramIr, ParseError> {
    let parsed = parse_object(yul)?;
    let mut lowering = lower::Lowering::new(contract, source_path, compiler);
    if let Some(o) = origins {
        lowering = lowering.with_origins(o);
    }
    Ok(lowering.run(&parsed.object, parsed.use_src, storage_layout, abi))
}
