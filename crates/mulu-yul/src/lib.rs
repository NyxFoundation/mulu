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
    lower_contract_with(
        contract,
        source_path,
        compiler,
        yul,
        abi,
        storage_layout,
        SolcFacts::default(),
    )
}

/// What the compiler knows that the Yul alone does not say.
#[derive(Default)]
pub struct SolcFacts<'a> {
    /// Where a source span sits, from the AST.
    pub origins: Option<lower::OriginLookup<'a>>,
    /// Selector to signature, from `evm.methodIdentifiers`. Without it the
    /// pairing has to be guessed from the dispatcher and the ABI, which is
    /// wrong as soon as a function is overloaded.
    pub selectors: std::collections::BTreeMap<String, String>,
}

/// As `lower_contract`, using what the compiler reported alongside the Yul.
#[allow(clippy::too_many_arguments)]
pub fn lower_contract_with(
    contract: &str,
    source_path: &str,
    compiler: &str,
    yul: &str,
    abi: &serde_json::Value,
    storage_layout: serde_json::Value,
    facts: SolcFacts<'_>,
) -> Result<ProgramIr, ParseError> {
    let parsed = parse_object(yul)?;
    let mut lowering = lower::Lowering::new(contract, source_path, compiler);
    if let Some(o) = facts.origins {
        lowering = lowering.with_origins(o);
    }
    lowering = lowering.with_selectors(facts.selectors);
    Ok(lowering.run(&parsed.object, parsed.use_src, storage_layout, abi))
}
