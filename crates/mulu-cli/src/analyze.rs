//! `mulu analyze` — P1-02: Solidity in, certified findings out.
//!
//! Chains the front end of P1-01 to the analysis core of P0 through predicate
//! abstraction: compile, lower to ProgramIR, build a finite model refined by
//! the guards and the specification, then hand it to the same worker and the
//! same certificate checks `analyze-model` uses.

use anyhow::{Context, Result};
use mulu_abstraction::model::Builder;
use mulu_abstraction::spec::Spec;
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};

pub struct AnalyzeArgs {
    pub sources: Vec<PathBuf>,
    /// A Foundry or Hardhat project directory, or one build-info file.
    pub project: Option<PathBuf>,
    pub build_info: Option<PathBuf>,
    pub contract: Option<String>,
    pub spec: Option<PathBuf>,
    pub out: PathBuf,
    pub solc: Option<PathBuf>,
    pub evm_version: String,
    pub objective: String,
    pub fail_on_candidate: bool,
    pub max_states: usize,
    pub max_edges: usize,
}

/// Where the sources and the compiler settings come from: the command line,
/// or the project's own build. With a build-info the settings are the
/// project's, so the analysis is of the code the project builds rather than of
/// a compilation mulu chose; every way the two still differ comes back as
/// `Drift` and is said out loud.
fn front_end(
    args: &AnalyzeArgs,
) -> Result<(PathBuf, mulu_solc::BuildBundle, String, mulu_yul::ir::ProgramIr, Option<mulu_solc::Drift>)>
{
    let named = match (&args.build_info, &args.project) {
        (Some(_), Some(_)) => {
            anyhow::bail!("pass --build-info or --project, not both")
        }
        (Some(f), None) => Some((f.clone(), f.parent().unwrap_or(Path::new(".")).to_path_buf())),
        (None, Some(dir)) => {
            let found = mulu_solc::project::discover(dir);
            let Some(first) = found.first() else {
                anyhow::bail!(
                    "no build-info under {}: build the project first (forge build / npx hardhat \
                     compile), or pass the source files directly",
                    dir.display()
                )
            };
            Some((first.clone(), dir.clone()))
        }
        (None, None) => None,
    };
    let Some((file, project_root)) = named else {
        if args.sources.is_empty() {
            anyhow::bail!("pass Solidity source files, or --project / --build-info");
        }
        let root = crate::build::source_root(&args.sources);
        let (bundle, name, ir) = crate::build::compile_and_lower(
            &args.sources,
            args.contract.as_deref(),
            args.solc.clone(),
            &args.evm_version,
        )?;
        return Ok((root, bundle, name, ir, None));
    };
    if !args.sources.is_empty() {
        anyhow::bail!("--project / --build-info supply the sources; do not also list files");
    }
    let bi = mulu_solc::project::read(&file)?;
    println!(
        "build   {} ({} source(s), solc {})",
        file.display(),
        bi.sources.len(),
        bi.solc_version
    );
    // `--evm-version` at its default means "whatever the project used"; given
    // explicitly it overrides, and that is itself a difference worth seeing.
    let evm = (args.evm_version != crate::DEFAULT_EVM_VERSION).then_some(args.evm_version.as_str());
    let (bundle, name, ir, drift) = crate::build::compile_project(
        &bi,
        args.contract.as_deref(),
        args.solc.clone(),
        evm,
        &project_root,
    )?;
    Ok((project_root, bundle, name, ir, Some(drift)))
}

pub fn run(args: &AnalyzeArgs, tools: &crate::ToolArgs) -> Result<i32> {
    let (root, bundle, name, ir, drift) = front_end(args)?;
    let contract = bundle.contract(&name).expect("selected contract");
    fs::create_dir_all(&args.out)?;
    crate::build::write_artifacts(&args.out, &bundle, contract, &ir)?;

    // --- specification
    let spec = match &args.spec {
        Some(p) => {
            let text = fs::read_to_string(p)
                .with_context(|| format!("reading the spec {}", p.display()))?;
            fs::write(args.out.join("spec.json"), &text)?;
            Spec::parse(&text).with_context(|| format!("parsing the spec {}", p.display()))?
        }
        None => Spec { schema_version: 1, properties: vec![] },
    };
    let props = spec
        .compile(&name, &ir.storage_layout)
        .with_context(|| "compiling the specification against the storage layout")?;
    if args.spec.is_some() && props.is_empty() {
        eprintln!("warning: the specification has no property for contract {name}");
    }

    // --- abstraction
    let abstraction = Builder::new(&ir, &props).build();
    let model_text = serde_json::to_string_pretty(&abstraction.model)?;
    fs::write(args.out.join("model.json"), &model_text)?;
    fs::write(
        args.out.join("abstraction.json"),
        serde_json::to_string_pretty(&abstraction.report)?,
    )?;

    print_abstraction(&abstraction, &props);

    // --- the P0 core, on the generated model
    let provenance = json!({
        "stage": "analyze",
        "solidity": crate::build::provenance(&bundle, contract, &ir),
        "specification": {
            "path": args.spec.as_ref().map(|p| p.display().to_string()),
            "properties": props.iter().map(|p| json!({"id": p.id, "text": p.text})).collect::<Vec<_>>(),
        },
        "abstraction": {
            "environment_profile": abstraction.report.environment_profile,
            "argument_regions": abstraction.report.argument_regions.len(),
            "storage_regions": abstraction.report.storage_regions.len(),
            "assumptions": abstraction.report.assumptions,
            "discharged": abstraction.report.discharged,
            "unsupported": abstraction.report.unsupported,
            "complete": abstraction.report.complete(),
        },
        // P1b: what the project builds, and how this differs from it. Absent
        // when the sources came from the command line, where there is no
        // project build to differ from.
        "project_build": drift.as_ref().map(|d| json!({
            "differences": d.lines(),
            "detail": d,
        })),
        "scope_note": "Every finding below is about the generated finite model. Carrying one \
                       back to the Solidity source needs the correspondence proofs of P1-04.",
    });

    // P1-03: turn each counterexample into concrete calls and run them on a
    // local EVM. The record sits beside the certificate; a model proof and an
    // execution are different evidence (docs/09 §5).
    let bytecode = contract.bytecode.clone();
    let layout = ir.storage_layout.clone();
    let report = abstraction.report.clone();
    let props_for_replay = props.clone();
    let out_dir = args.out.clone();
    let reproducer = move |d: &crate::report::Diagnostic| -> Option<serde_json::Value> {
        let mut rep = match d.kind {
            "spec-violation" if d.status == "proven" => {
                let path = d.detail.as_ref()?.get("path")?;
                match crate::reproduce::concretise(path, &report) {
                    Ok(calls) => {
                        let slots = crate::reproduce::spec_slots(&layout, &props_for_replay);
                        crate::reproduce::run(
                            bytecode.as_deref(),
                            calls,
                            &slots,
                            &props_for_replay,
                            &layout,
                        )
                    }
                    Err(why) => crate::reproduce::Reproduction {
                        status: "unsupported",
                        reason: Some(why),
                        calls: vec![],
                        run: None,
                        path: None,
                        assumptions: crate::reproduce::ASSUMPTIONS.to_vec(),
                    },
                }
            }
            "overrestriction" => {
                let state = d.detail.as_ref()?.get("impl_state")?.as_str()?;
                match crate::reproduce::concretise_rejected_call(state, &report) {
                    Ok(calls) => crate::reproduce::run_rejection(bytecode.as_deref(), calls),
                    Err(why) => crate::reproduce::Reproduction {
                        status: "unsupported",
                        reason: Some(why),
                        calls: vec![],
                        run: None,
                        path: None,
                        assumptions: crate::reproduce::ASSUMPTIONS.to_vec(),
                    },
                }
            }
            _ => return None,
        };
        if let Err(e) = crate::reproduce::write_record(&out_dir, &d.id, &mut rep) {
            eprintln!("warning: could not write the reproduction record: {e:#}");
        }
        serde_json::to_value(&rep).ok()
    };

    if let Some(d) = &drift {
        if d.is_empty() {
            println!("\n  this is the project's own build: same sources, same compiler");
        } else {
            println!("\n  this analysis is not the build the project ships:");
            for line in d.lines() {
                println!("    {line}");
            }
        }
    }

    // The contract in the adopted Yul semantics, beside the model built from
    // the same `ir`. This proves nothing; it is what the correspondence
    // conditions would be stated about, and the normalisations the rendering
    // needed are the assumptions that come with it.
    let (semantics_path, normalisations, hazards) =
        crate::build::write_semantics_module(&args.out, &contract.ir, &name)?;
    println!("\nsemantics");
    println!("  {} in EvmYul's notation", semantics_path.display());
    if normalisations.is_empty() {
        println!("  the rendering is a transcription: nothing was normalised");
    } else {
        for l in &normalisations {
            println!("  {l}");
        }
    }
    for l in &hazards {
        println!("  CANNOT BE READ THERE: {l}");
    }

    // P1-04: what stands between a model claim and a claim about the program.
    let ledger =
        crate::obligations::ledger(&ir, &abstraction.report, drift.as_ref(), &normalisations, &hazards);
    print_obligations(&ledger);

    println!("\n--- analysis of the generated model ---\n");
    let code = crate::analyze_model_at(
        &args.out.join("model.json"),
        &args.out,
        &args.objective,
        args.fail_on_candidate,
        crate::Limits { max_states: args.max_states, max_edges: args.max_edges },
        tools,
        crate::Frontend {
            provenance: Some(provenance),
            reproducer: Some(&reproducer),
            ledger: Some(&ledger),
            sites: crate::build::check_sites(&bundle, &ir, &root),
            // A finding about the contract as a whole is shown on the
            // contract, not on an arbitrary line of it.
            fallback_site: contract_site(&bundle, &ir, &root),
            unsupported: abstraction.report.unsupported.clone(),
        },
    )?;

    // An incomplete abstraction cannot be reported as a complete analysis.
    // The exit code already accounts for it, because the incompleteness is
    // recorded as an analysis status rather than patched on afterwards; two
    // places to compute one number is one place too many.
    if !abstraction.report.complete() {
        eprintln!(
            "\nthe abstraction left {} thing(s) unmodelled, so this unit is not complete:",
            abstraction.report.unsupported.len()
        );
        for u in &abstraction.report.unsupported {
            eprintln!("  {u}");
        }
    }
    Ok(code)
}

fn print_obligations(l: &crate::obligations::Ledger) {
    let open = l.open();
    println!("\ncorrespondence");
    println!("  findings are reported at: {}", l.scope);
    println!("  {} obligation(s), {} open", l.obligations.len(), open.len());
    // Two kinds of open, and printing them as one list makes an assumption on
    // a compiler nobody has verified look like an item on a to-do list.
    let ours: Vec<_> = open.iter().filter(|o| o.ours()).collect();
    let theirs: Vec<_> = open.iter().filter(|o| !o.ours()).collect();
    if !ours.is_empty() {
        println!("  {} to prove here:", ours.len());
        for o in &ours {
            println!("    {:<40} reaches {}", o.id, o.reaches);
        }
    }
    if !theirs.is_empty() {
        // Named by who bears them. One is a compiler nobody has proved
        // correct, another is a semantics nobody has proved matches the EVM;
        // calling both "a compiler" would be wrong about the second.
        println!("  {} assumption(s) on work this project did not do:", theirs.len());
        for o in &theirs {
            println!("    {:<40} {:<8} reaches {}", o.id, format!("({})", o.bearer), o.reaches);
        }
    }
    if open.is_empty() {
        println!("    none");
    }
}

fn print_abstraction(
    a: &mulu_abstraction::model::Abstraction,
    props: &[mulu_abstraction::spec::CompiledProperty],
) {
    println!("\nabstraction ({})", a.report.environment_profile);
    println!("  entrypoints modelled: {}", a.report.entrypoints_modelled.join(", "));
    if !a.report.entrypoints_skipped.is_empty() {
        println!("  entrypoints skipped:  {}", a.report.entrypoints_skipped.join(", "));
    }
    if props.is_empty() {
        println!("  specification: none, so only redundancy is analysed");
    } else {
        for p in props {
            println!("  specification {}: {}", p.id, p.text);
        }
    }
    println!("  argument regions");
    for r in &a.report.argument_regions {
        println!("    {:<4} {}", r.name, r.set);
    }
    if !a.report.storage_regions.is_empty() {
        println!("  storage regions");
        for r in &a.report.storage_regions {
            println!("    {:<6} {}", r.name, r.description);
        }
    }
    for d in &a.report.discharged {
        println!("  discharged: {d}");
    }
    println!("  model: {} states, {} transitions", a.model.states.len(), a.model.transitions.len());
    if a.report.complete() {
        println!("  unsupported: none");
    } else {
        println!("  unsupported ({}):", a.report.unsupported.len());
        for u in &a.report.unsupported {
            println!("    {u}");
        }
    }
}

/// The whole source file the contract is declared in, with no region: a
/// specification violation is about the contract, and inventing a line for it
/// would point the reader somewhere the finding is not.
fn contract_site(
    bundle: &mulu_solc::BuildBundle,
    ir: &mulu_yul::ir::ProgramIr,
    root: &std::path::Path,
) -> Option<crate::build::Site> {
    let src = bundle.sources.iter().find(|s| s.path == ir.source_path)?;
    Some(crate::build::Site {
        path: crate::build::source_uri(root, &src.path),
        sha256: src.sha256.clone(),
        region: None,
    })
}
