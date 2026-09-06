//! `mulu ir` — P1-01: compile a Solidity source with solc and write the
//! ProgramIR the later stages read.
//!
//! This command performs no analysis. It produces the intermediate artifact
//! and says plainly what it could not model.

use anyhow::{Context, Result};
use mulu_solc::{CompileOptions, Solc};
use mulu_yul::ir::{FunctionKind, Op};
use mulu_yul::{CheckOrigin, ProgramIr, Purity};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};

pub struct IrArgs {
    pub sources: Vec<PathBuf>,
    pub contract: Option<String>,
    pub out: PathBuf,
    pub solc: Option<PathBuf>,
    pub evm_version: String,
}

/// Compile and lower, shared by `mulu ir` and `mulu analyze`.
pub fn compile_and_lower(
    sources: &[PathBuf],
    contract: Option<&str>,
    solc_path: Option<PathBuf>,
    evm_version: &str,
) -> Result<(mulu_solc::BuildBundle, String, ProgramIr)> {
    let solc = Solc::discover(solc_path)?;
    let root = sources
        .first()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    let opts = CompileOptions { evm_version: evm_version.to_string(), ..Default::default() };

    let bundle = solc
        .compile_files(&root, sources, &opts)
        .with_context(|| format!("compiling with {}", solc.path().display()))?;
    for w in &bundle.warnings {
        eprintln!("solc warning: {}", w.lines().next().unwrap_or(w));
    }
    let selected = mulu_solc::driver_select(&bundle, contract)?.name.clone();
    let c = bundle.contract(&selected).expect("just selected");

    if !bundle.unresolved_imports.is_empty() {
        eprintln!(
            "warning: {} import(s) need a remapping and were not followed: {}",
            bundle.unresolved_imports.len(),
            bundle.unresolved_imports.join(", ")
        );
    }

    // The AST says which contract and which modifier a check was written in;
    // the Yul only carries a byte span (docs/08 §2).
    let index = bundle.ast_index.clone();
    let lookup = move |file_id: u32, start: u32, end: u32| mulu_yul::SourceOrigin {
        contract: index.contract_at(file_id, start, end).map(|s| s.to_string()),
        member: index
            .enclosing(file_id, start, end)
            .into_iter()
            .find(|n| n.kind != mulu_solc::AstKind::Contract)
            .map(|n| n.name.clone()),
        in_modifier: index.modifier_at(file_id, start, end).is_some(),
    };

    let selectors = c.selectors();
    if selectors.is_empty() {
        eprintln!(
            "warning: solc reported no method identifiers for {}; entrypoint signatures fall \
             back to name matching, which cannot tell overloads apart",
            c.name
        );
    }
    let ir = mulu_yul::lower_contract_with(
        &c.name,
        &c.source_path,
        &bundle.compiler,
        &c.ir,
        &c.abi,
        c.storage_layout.clone(),
        mulu_yul::SolcFacts { origins: Some(&lookup), selectors },
    )
    .with_context(|| format!("lowering the Yul of {}", c.name))?;
    Ok((bundle, selected, ir))
}

pub fn run(args: &IrArgs) -> Result<i32> {
    let (bundle, name, ir) = compile_and_lower(
        &args.sources,
        args.contract.as_deref(),
        args.solc.clone(),
        &args.evm_version,
    )?;
    let contract = bundle.contract(&name).expect("selected contract");

    write_artifacts(&args.out, &bundle, contract, &ir)?;
    write_ir_manifest(&args.out, &bundle, contract, &ir)?;
    print_summary(&bundle, &ir, &args.out);

    // docs/09 §4: anything unsupported means the unit is not complete.
    Ok(if ir.fully_supported() { 0 } else { 2 })
}

pub fn write_artifacts(
    out: &Path,
    bundle: &mulu_solc::BuildBundle,
    contract: &mulu_solc::ContractArtifact,
    ir: &ProgramIr,
) -> Result<()> {
    fs::create_dir_all(out.join("build/sources"))?;
    for s in &bundle.sources {
        let p = out.join("build/sources").join(s.path.replace('/', "_"));
        fs::write(p, &s.content)?;
    }
    fs::write(out.join("build/settings.json"), serde_json::to_string_pretty(&bundle.settings)?)?;
    fs::write(out.join("build").join(format!("{}.yul", contract.name)), &contract.ir)?;
    fs::write(out.join("build/abi.json"), serde_json::to_string_pretty(&contract.abi)?)?;
    fs::write(
        out.join("build/storage-layout.json"),
        serde_json::to_string_pretty(&contract.storage_layout)?,
    )?;
    fs::write(out.join("program.json"), serde_json::to_string_pretty(ir)?)?;
    Ok(())
}

/// What `mulu ir` records; `mulu analyze` folds the same facts into its own
/// manifest instead.
pub fn provenance(
    bundle: &mulu_solc::BuildBundle,
    contract: &mulu_solc::ContractArtifact,
    ir: &ProgramIr,
) -> serde_json::Value {
    json!({
        "compiler": bundle.compiler,
        "settings": bundle.settings,
        "standard_json_input_sha256": bundle.input_sha256,
        "contract": contract.name,
        "sources": bundle.sources.iter().map(|s| json!({
            "id": s.id, "path": s.path, "sha256": s.sha256
        })).collect::<Vec<_>>(),
        "analysed_artifact": ir.derived_from,
        "scope_note": "Findings derived from this IR are about the unoptimized Yul, \
                       not the deployed bytecode. Promoting them needs the correspondence \
                       proofs of P1-04.",
        "entrypoints": ir.entrypoints,
        "unsupported": ir.unsupported,
        "fully_supported": ir.fully_supported(),
    })
}

fn write_ir_manifest(
    out: &Path,
    bundle: &mulu_solc::BuildBundle,
    contract: &mulu_solc::ContractArtifact,
    ir: &ProgramIr,
) -> Result<()> {
    let manifest = json!({
        "schema_version": 1,
        "tool": crate::TOOL,
        "stage": "ir",
        "provenance": provenance(bundle, contract, ir),
    });
    fs::write(out.join("manifest.json"), serde_json::to_string_pretty(&manifest)?)?;
    Ok(())
}

/// `path:line:col` for a location, resolved through the file id it carries.
/// With several sources a check can point into a file other than the one the
/// contract is declared in, which is exactly what a modifier does.
pub fn where_of(
    bundle: &mulu_solc::BuildBundle,
    ir: &ProgramIr,
    loc: Option<mulu_yul::Location>,
) -> String {
    let Some(l) = loc else { return "(generated)".into() };
    let Some(src) = bundle.source_by_id(l.file_id) else {
        return format!("file#{}:{}", l.file_id, l.byte_start);
    };
    let path = ir.use_src.get(&l.file_id).cloned().unwrap_or_else(|| src.path.clone());
    match src.line_col(l.byte_start as usize) {
        Some((line, col)) => format!("{path}:{line}:{}", col + 1),
        None => format!("{path}@{}", l.byte_start),
    }
}

pub fn print_summary(bundle: &mulu_solc::BuildBundle, ir: &ProgramIr, out: &Path) {
    println!("contract {} from {}", ir.contract, ir.source_path);
    println!("compiler {}", bundle.compiler);
    if bundle.sources.len() > 1 {
        let names: Vec<String> = bundle
            .sources
            .iter()
            .map(|s| format!("{} (#{})", s.path, s.id))
            .collect();
        println!("sources  {}", names.join(", "));
    }
    println!("artifact {}\n", ir.derived_from);

    println!("entrypoints");
    for e in &ir.entrypoints {
        println!("  {}  {}", e.selector, e.signature);
    }

    println!("\nchecks");
    for c in &ir.checks {
        let origin = match c.origin {
            CheckOrigin::Require => "require",
            CheckOrigin::Modifier => "modifier",
            CheckOrigin::Compiler => "compiler",
            CheckOrigin::Inline => "inline",
        };
        let where_ = where_of(bundle, ir, c.source);
        let fname = ir
            .function(&c.function)
            .and_then(|f| f.solidity_name.clone())
            .unwrap_or_else(|| c.function.clone());
        let written = match (&c.declared_in, &c.written_in) {
            (Some(ct), Some(m)) if c.origin == CheckOrigin::Modifier => {
                format!("{fname}  (modifier {ct}.{m})")
            }
            (Some(ct), Some(m)) => format!("{ct}.{m}"),
            _ => fname.clone(),
        };
        println!("  {:<28} {origin:<9} {written}", c.id);
        println!("      passes when  {}", c.condition_text);
        println!("      purity {:?}   at {where_}", c.purity);
    }
    let pure = ir.pure_checks().len();
    println!(
        "\n  {} check(s); {pure} usable as predicates over arguments, {} not",
        ir.checks.len(),
        ir.checks.len() - pure
    );

    println!("\nstorage writes");
    for (f, i) in ir.storage_writes() {
        // report at the Solidity level, skipping the generated helper frames
        let Some(func) = ir.function(f) else { continue };
        if func.kind != FunctionKind::Body {
            continue;
        }
        let text = match &i.op {
            Op::Effect { call } => call.render(),
            Op::Let { value: Some(v), .. } => v.render(),
            Op::Assign { value, .. } => value.render(),
            _ => "?".into(),
        };
        let where_ = where_of(bundle, ir, i.source);
        println!("  {}  {where_}", func.solidity_name.clone().unwrap_or_else(|| f.to_string()));
        println!("      {text}");
    }

    if ir.unsupported.is_empty() {
        println!("\nunsupported: none");
    } else {
        println!("\nunsupported ({}) — this unit is not complete", ir.unsupported.len());
        for u in &ir.unsupported {
            println!("  {}: {}", u.function, u.reason);
        }
    }
    let _ = Purity::Pure;
    println!("\nartifacts: {}", out.display());
}
