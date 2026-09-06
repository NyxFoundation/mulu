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

    let ir = mulu_yul::lower_contract(
        &c.name,
        &c.source_path,
        &bundle.compiler,
        &c.ir,
        &c.abi,
        c.storage_layout.clone(),
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

pub fn print_summary(bundle: &mulu_solc::BuildBundle, ir: &ProgramIr, out: &Path) {
    println!("contract {} from {}", ir.contract, ir.source_path);
    println!("compiler {}", bundle.compiler);
    println!("artifact {}\n", ir.derived_from);

    println!("entrypoints");
    for e in &ir.entrypoints {
        println!("  {}  {}", e.selector, e.signature);
    }

    println!("\nchecks");
    let src = bundle.source_by_id(0);
    for c in &ir.checks {
        let origin = match c.origin {
            CheckOrigin::Require => "require",
            CheckOrigin::Compiler => "compiler",
            CheckOrigin::Inline => "inline",
        };
        let where_ = c
            .source
            .and_then(|l| src.and_then(|s| s.line_col(l.byte_start as usize)))
            .map(|(line, col)| format!("{}:{}:{}", ir.source_path, line, col + 1))
            .unwrap_or_else(|| "(generated)".into());
        let fname = ir
            .function(&c.function)
            .and_then(|f| f.solidity_name.clone())
            .unwrap_or_else(|| c.function.clone());
        println!("  {:<28} {origin:<9} {fname}", c.id);
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
        let where_ = i
            .source
            .and_then(|l| src.and_then(|s| s.line_col(l.byte_start as usize)))
            .map(|(line, col)| format!("{}:{}:{}", ir.source_path, line, col + 1))
            .unwrap_or_else(|| "(generated)".into());
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
