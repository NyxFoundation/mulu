//! `mulu ir` — P1-01: compile a Solidity source with solc and write the
//! ProgramIR the later stages read.
//!
//! This command performs no analysis. It produces the intermediate artifact
//! and says plainly what it could not model.

use anyhow::{anyhow, Context, Result};
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
    let root = source_root(sources);
    let opts = CompileOptions { evm_version: evm_version.to_string(), ..Default::default() };

    let bundle = solc
        .compile_files(&root, sources, &opts)
        .with_context(|| format!("compiling with {}", solc.path().display()))?;
    lower_bundle(bundle, contract)
}

/// Compile the sources a project's own build recorded, with the settings it
/// recorded, except the two mulu cannot follow: the optimizer stays off and
/// the IR pipeline stays off, because the analysed artifact is the
/// unoptimized `ir`. Both differences come back in the [`Drift`] so the run
/// can say them out loud instead of quietly analysing something else.
pub fn compile_project(
    bi: &mulu_solc::BuildInfo,
    contract: Option<&str>,
    solc_path: Option<PathBuf>,
    evm_version: Option<&str>,
    project_root: &Path,
) -> Result<(mulu_solc::BuildBundle, String, ProgramIr, mulu_solc::Drift)> {
    let solc = Solc::discover(solc_path)?;
    let drift = bi.drift(solc.version(), project_root);
    let opts = CompileOptions {
        evm_version: evm_version
            .map(String::from)
            .or_else(|| bi.evm_version())
            .unwrap_or_else(|| "cancun".into()),
        remappings: bi.remappings(),
        ..Default::default()
    };
    let bundle = solc
        .compile(&bi.sources, &opts)
        .with_context(|| format!("recompiling the build of {} with {}", bi.path.display(), solc.path().display()))?;
    let (bundle, name, ir) = lower_bundle(bundle, contract)?;
    Ok((bundle, name, ir, drift))
}

fn lower_bundle(
    bundle: mulu_solc::BuildBundle,
    contract: Option<&str>,
) -> Result<(mulu_solc::BuildBundle, String, ProgramIr)> {
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

/// Where a finding is written. `region` is absent when the finding is about
/// the file as a whole: inventing a line for it would point the reader at
/// somewhere the finding is not.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Site {
    /// Relative to the working directory when the file is under it, which is
    /// what a code-scanning viewer resolves against. solc keys sources
    /// relative to the first input's directory, so this is not that key.
    pub path: String,
    pub sha256: String,
    pub region: Option<Region>,
}

/// 1-based lines, and 1-based columns in UTF-16 code units, as SARIF counts.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Region {
    pub start_line: usize,
    pub start_column: usize,
    pub end_line: usize,
    pub end_column: usize,
}

/// The directory solc source keys are relative to: the first input's parent.
pub fn source_root(sources: &[PathBuf]) -> PathBuf {
    sources.first().and_then(|p| p.parent()).map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from("."))
}

/// A solc source key turned into a URI a reader can open. solc keys sources
/// relative to the first input's directory, which locates nothing on its own.
///
/// A code-scanning viewer resolves a relative URI against the repository, so
/// a file under the working directory gets a path relative to it, which is
/// the same string on every machine. A file outside it gets an absolute
/// `file:` URI, which is machine-specific but at least points at the file;
/// the alternative is a path that resolves to the wrong file or to none.
pub fn source_uri(root: &Path, key: &str) -> String {
    let full = root.join(key);
    let (Ok(full), Ok(cwd)) = (full.canonicalize(), std::env::current_dir()) else {
        return key.to_string();
    };
    match full.strip_prefix(&cwd) {
        Ok(rel) => join_uri(rel),
        Err(_) => format!("file://{}", join_uri(&full)),
    }
}

/// Path components as URI segments, escaping the characters that would end
/// the path or start a query.
fn join_uri(p: &Path) -> String {
    let escape = |s: std::borrow::Cow<str>| {
        s.chars()
            .map(|c| match c {
                '%' => "%25".to_string(),
                ' ' => "%20".to_string(),
                '#' => "%23".to_string(),
                '?' => "%3F".to_string(),
                _ => c.to_string(),
            })
            .collect::<String>()
    };
    let parts: Vec<String> =
        p.components().map(|c| escape(c.as_os_str().to_string_lossy())).collect();
    // An absolute path's first component is the root, which already is "/".
    parts.join("/").replace("//", "/")
}

/// The structured form of [`where_of`]. `None` for a generated location:
/// pointing a reader at a line that is not in their source is worse than
/// pointing them at nothing.
pub fn site_of(
    bundle: &mulu_solc::BuildBundle,
    ir: &ProgramIr,
    root: &Path,
    loc: Option<mulu_yul::Location>,
) -> Option<Site> {
    let l = loc?;
    let src = bundle.source_by_id(l.file_id)?;
    let start = l.byte_start as usize;
    let (start_line, start_col) = src.line_col(start)?;
    // A span that runs past the end of the file is a bug upstream, not a
    // reason to drop the location: fall back to the start.
    let (end_line, end_col) =
        src.line_col(start + l.byte_length as usize).unwrap_or((start_line, start_col));
    // `@use-src` names the file as the Yul refers to it, which is the name the
    // user typed; `sources` names it as solc resolved it. Either way it is a
    // key relative to the build root, so it goes through `source_uri`.
    let key = ir.use_src.get(&l.file_id).cloned().unwrap_or_else(|| src.path.clone());
    Some(Site {
        path: source_uri(root, &key),
        sha256: src.sha256.clone(),
        region: Some(Region {
            start_line,
            start_column: start_col + 1,
            end_line,
            end_column: end_col + 1,
        }),
    })
}

/// Where each check of the model is written, keyed by the check id the model
/// uses. The model itself carries no source location, on purpose: it is the
/// front end's job to say where a finding about check `A` should be shown.
pub fn check_sites(
    bundle: &mulu_solc::BuildBundle,
    ir: &ProgramIr,
    root: &Path,
) -> std::collections::BTreeMap<String, Site> {
    ir.checks.iter().filter_map(|c| Some((c.id.clone(), site_of(bundle, ir, root, c.source)?))).collect()
}

/// `mulu yul-lean` — the contract's Yul as an `EvmYul` `YulContract`, so the
/// correspondence conditions can be stated about this contract rather than
/// about an arbitrary program. `analyze` writes the same module into its
/// output directory; this command is for looking at one on its own.
///
/// It reads the same unoptimized `ir` the analysis reads, from the same
/// build, so the Lean module and the model are about the same bytes.
pub fn yul_lean(args: &IrArgs) -> Result<i32> {
    let (bundle, name, _ir) =
        compile_and_lower(&args.sources, args.contract.as_deref(), args.solc.clone(), &args.evm_version)?;
    let c = bundle.contract(&name).expect("selected contract");
    let parsed = mulu_yul::parse::parse_object(&c.ir)
        .map_err(|e| anyhow!("parsing the Yul of {name}: {e}"))?;
    let deployed = parsed
        .object
        .deployed()
        .ok_or_else(|| anyhow!("{name}: the Yul has no deployed object to render"))?;
    let (module, norm) = mulu_yul::lean::contract_module(deployed, &name)
        .map_err(|e| anyhow!("rendering {name} in EvmYul's notation: {e}"))?;
    fs::create_dir_all(&args.out)?;
    let path = args.out.join(format!("{name}.lean"));
    fs::write(&path, &module)?;
    println!("contract {name} from {}", c.source_path);
    println!("compiler {}", bundle.compiler);
    println!("object   {}", deployed.name);
    println!("functions {}", deployed.functions().len());
    if norm.is_empty() {
        println!("\nthe rendering is a transcription: nothing was normalised");
    } else {
        println!("\nthe rendering is not a transcription:");
        for l in norm.lines() {
            println!("  {l}");
        }
    }
    println!("\nwrote {}", path.display());
    println!(
        "\nThis states nothing. It puts the contract where the correspondence conditions of\n\
         Mulu.Semantics.Simulation can be stated about it. That the rendering is the same\n\
         program is itself unproved: semantics:rendering-preserves-the-program."
    );
    Ok(0)
}

/// The same rendering, for the analysis directory. `analyze` writes it beside
/// the model so that the artifact the correspondence would be stated about
/// sits next to the artifact the claims are about, and both came from the
/// same `ir` bytes. Writing it needs no Lean and no mathlib: it is text.
pub fn write_semantics_module(
    out: &Path,
    ir_text: &str,
    name: &str,
) -> Result<(PathBuf, Vec<String>)> {
    let parsed = mulu_yul::parse::parse_object(ir_text)
        .map_err(|e| anyhow!("parsing the Yul of {name}: {e}"))?;
    let deployed = parsed
        .object
        .deployed()
        .ok_or_else(|| anyhow!("{name}: the Yul has no deployed object to render"))?;
    let (module, norm) = mulu_yul::lean::contract_module(deployed, name)
        .map_err(|e| anyhow!("rendering {name} in EvmYul's notation: {e}"))?;
    let dir = out.join("semantics");
    fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{name}.lean"));
    fs::write(&path, module)?;
    Ok((path, norm.lines()))
}
