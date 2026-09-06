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
use std::path::PathBuf;

pub struct AnalyzeArgs {
    pub sources: Vec<PathBuf>,
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

pub fn run(args: &AnalyzeArgs, tools: &crate::ToolArgs) -> Result<i32> {
    let root = crate::build::source_root(&args.sources);
    let (bundle, name, ir) = crate::build::compile_and_lower(
        &args.sources,
        args.contract.as_deref(),
        args.solc.clone(),
        &args.evm_version,
    )?;
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

    // P1-04: what stands between a model claim and a claim about the program.
    let ledger = crate::obligations::ledger(&ir, &abstraction.report);
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
    for o in &open {
        println!("    {:<40} reaches {}", o.id, o.reaches);
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
