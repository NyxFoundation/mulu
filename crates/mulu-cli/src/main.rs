//! `mulu` — command line.
//!
//! Exit codes (docs/09 §4): 0 complete & no confirmed violation; 1 confirmed
//! violation; 2 partial / unsupported; 3 input or execution error;
//! 4 certificate check failed. Mixed results use the priority 4 > 3 > 2 > 1 > 0.

mod analyze;
mod build;
mod obligations;
mod reproduce;
mod lean;
mod report;
mod worker;

use anyhow::{anyhow, bail, Context, Result};
use clap::{Parser, Subcommand};
use mulu_model::core::{normalize, normalize_plant, Normalized};
use mulu_model::hash::{canonical_json, sha256_hex};
use mulu_model::{reference, validate::parse_and_validate, FiniteProduct};
use report::{Diagnostic, Evidence, MODEL_ASSUMPTIONS};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;
use worker::{Toolchain, WorkerRequest};

pub const TOOL: &str = concat!("mulu ", env!("CARGO_PKG_VERSION"));

#[derive(Parser)]
#[command(name = "mulu", version, about = "A Supervisory Control-based static analyzer for code redundancy and gap detection")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(clap::Args, Clone)]
pub struct ToolArgs {
    /// Path to the Lean project (default: auto-discover `lean/` or $MULU_LEAN_DIR)
    #[arg(long, global = true)]
    pub lean_dir: Option<PathBuf>,
    /// Path to the mulu-worker binary (default: <lean-dir>/.lake/build/bin/mulu-worker or $MULU_WORKER)
    #[arg(long, global = true)]
    pub worker: Option<PathBuf>,
    /// Skip the kernel re-check (`lake env lean certificates/Check.lean`)
    #[arg(long, global = true)]
    pub no_kernel: bool,
    /// Worker timeout in milliseconds
    #[arg(long, default_value = "60000", global = true)]
    pub timeout_ms: u64,
}

#[derive(Subcommand)]
enum Cmd {
    /// Analyse Solidity: compile, abstract, and check the model (P1-02)
    Analyze {
        /// Solidity source files to compile
        #[arg(required = true)]
        sources: Vec<PathBuf>,
        /// Which contract to analyse; required when the build defines several
        #[arg(long)]
        contract: Option<String>,
        /// Specification (docs/09 §3). Without one, only redundancy is analysed
        #[arg(long)]
        spec: Option<PathBuf>,
        #[arg(long)]
        out: PathBuf,
        /// Path to solc (default: $MULU_SOLC, then PATH)
        #[arg(long)]
        solc: Option<PathBuf>,
        #[arg(long, default_value = "cancun")]
        evm_version: String,
        /// safety-nonblocking (default) or safety
        #[arg(long, default_value = "safety-nonblocking")]
        objective: String,
        /// Exit 1 also for unconfirmed candidates
        #[arg(long)]
        fail_on_candidate: bool,
        #[arg(long, default_value = "100000")]
        max_states: usize,
        #[command(flatten)]
        tools: ToolArgs,
    },
    /// Compile Solidity with solc and write the ProgramIR (P1-01). No analysis.
    Ir {
        /// Solidity source files to compile
        #[arg(required = true)]
        sources: Vec<PathBuf>,
        /// Which contract to lower; required when the build defines several
        #[arg(long)]
        contract: Option<String>,
        #[arg(long)]
        out: PathBuf,
        /// Path to solc (default: $MULU_SOLC, then PATH)
        #[arg(long)]
        solc: Option<PathBuf>,
        #[arg(long, default_value = "cancun")]
        evm_version: String,
    },
    /// Analyse a finite-product model (schema v1) and write an analysis directory
    AnalyzeModel {
        model: PathBuf,
        #[arg(long)]
        out: PathBuf,
        /// safety-nonblocking (default) or safety
        #[arg(long, default_value = "safety-nonblocking")]
        objective: String,
        /// Exit 1 also for unconfirmed candidates (overrestriction, may-fail)
        #[arg(long)]
        fail_on_candidate: bool,
        #[arg(long, default_value = "100000")]
        max_states: usize,
        #[command(flatten)]
        tools: ToolArgs,
    },
    /// Re-check an analysis directory: hashes, certificates (worker + kernel), axioms
    Verify {
        dir: PathBuf,
        #[command(flatten)]
        tools: ToolArgs,
    },
    /// Validate a model file against schema v1 and print its normalised core
    Validate { model: PathBuf },
}

fn main() {
    let code = match run() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e:#}");
            3
        }
    };
    std::process::exit(code);
}

fn run() -> Result<i32> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Analyze {
            sources,
            contract,
            spec,
            out,
            solc,
            evm_version,
            objective,
            fail_on_candidate,
            max_states,
            tools,
        } => analyze::run(
            &analyze::AnalyzeArgs {
                sources,
                contract,
                spec,
                out,
                solc,
                evm_version,
                objective,
                fail_on_candidate,
                max_states,
            },
            &tools,
        ),
        Cmd::Ir { sources, contract, out, solc, evm_version } => build::run(&build::IrArgs {
            sources,
            contract,
            out,
            solc,
            evm_version,
        }),
        Cmd::Validate { model } => {
            let text = fs::read_to_string(&model).with_context(|| format!("reading {}", model.display()))?;
            let m = parse_and_validate(&text).map_err(|e| anyhow!("{}: {e}", model.display()))?;
            let n = normalize(&m);
            println!("{}", serde_json::to_string_pretty(&n.core)?);
            Ok(0)
        }
        Cmd::AnalyzeModel { model, out, objective, fail_on_candidate, max_states, tools } => {
            analyze_model_at(
                &model,
                &out,
                &objective,
                fail_on_candidate,
                max_states,
                &tools,
                None,
                None,
                None,
            )
        }
        Cmd::Verify { dir, tools } => verify(&dir, &tools),
    }
}

// ---------------------------------------------------------------------------
// analyze-model

struct Ctx {
    out: PathBuf,
    tc: Toolchain,
    timeout: Duration,
    objective: String,
    certs: Vec<(String, String, Value)>, // (id, lean model name, certificate)
    diags: Vec<Diagnostic>,
    statuses: Vec<(String, String)>,
    cross_check_errors: Vec<String>,
}

impl Ctx {
    fn add_cert(&mut self, id: &str, model: &str, cert: &Value) -> Result<String> {
        let rel = format!("certificates/{}.json", lean::ident(id));
        fs::write(self.out.join(&rel), serde_json::to_string_pretty(cert)?)?;
        self.certs.push((id.to_string(), model.to_string(), cert.clone()));
        Ok(rel)
    }
    fn status(&mut self, analysis: &str, status: &str) {
        self.statuses.push((analysis.to_string(), status.to_string()));
    }
}

fn names(n: &Normalized, v: &Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().filter_map(|x| x.as_u64()).map(|i| n.state_name(i as usize).to_string()).collect()).unwrap_or_default()
}

fn edge_names(n: &Normalized, v: &Value) -> Vec<Value> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|e| e.as_array())
                .filter(|e| e.len() == 3)
                .map(|e| {
                    let i = |k: usize| e[k].as_u64().unwrap_or(0) as usize;
                    json!({"from": n.state_name(i(0)), "event": n.event_name(i(1)), "to": n.state_name(i(2))})
                })
                .collect()
        })
        .unwrap_or_default()
}

fn nat_set(v: &Value) -> BTreeSet<usize> {
    v.as_array().map(|a| a.iter().filter_map(|x| x.as_u64().map(|n| n as usize)).collect()).unwrap_or_default()
}

/// Given a finished diagnostic, produce its concrete reproduction, if any.
pub type Reproducer<'a> = &'a dyn Fn(&Diagnostic) -> Option<Value>;

#[allow(clippy::too_many_arguments)]
pub fn analyze_model_at(
    model: &Path,
    out: &Path,
    objective: &str,
    fail_on_candidate: bool,
    max_states: usize,
    tools: &ToolArgs,
    provenance: Option<Value>,
    reproducer: Option<Reproducer<'_>>,
    ledger: Option<&obligations::Ledger>,
) -> Result<i32> {
    if objective != "safety-nonblocking" && objective != "safety" {
        bail!("--objective must be safety-nonblocking or safety");
    }
    let text = fs::read_to_string(model).with_context(|| format!("reading {}", model.display()))?;
    let fp = parse_and_validate(&text).map_err(|e| anyhow!("{}: {e}", model.display()))?;
    let impl_n = normalize(&fp);
    let plant_n = fp.control_plant.as_ref().map(normalize_plant);

    fs::create_dir_all(out.join("certificates"))?;
    fs::write(out.join("model.json"), &text)?;
    let core_json = canonical_json(&impl_n.core.to_json());
    fs::write(out.join("core-model.json"), &core_json)?;
    if let Some(p) = &plant_n {
        fs::write(out.join("core-plant.json"), canonical_json(&p.core.to_json()))?;
    }
    let tc = Toolchain::discover(tools.lean_dir.clone(), tools.worker.clone());
    let mut ctx = Ctx {
        out: out.to_path_buf(),
        tc,
        timeout: Duration::from_millis(tools.timeout_ms),
        objective: objective.to_string(),
        certs: vec![],
        diags: vec![],
        statuses: vec![],
        cross_check_errors: vec![],
    };

    // --- implementation model
    let impl_analyses: Vec<&str> = if plant_n.is_some() {
        vec!["reachability", "redundancy", "safety"]
    } else {
        vec!["reachability", "redundancy", "safety", "envelope"]
    };
    let resp_impl = worker::call(
        &ctx.tc,
        out,
        &WorkerRequest { request_id: "impl", method: "analyze", model_path: "core-model.json", analyses: &impl_analyses, objective, certificate_path: None, max_states },
        ctx.timeout,
    )?;
    fs::write(out.join("worker-impl.json"), serde_json::to_string_pretty(&resp_impl)?)?;
    if resp_impl["status"] == "error" {
        bail!("worker: {}", resp_impl["error"]);
    }
    let impl_reach = handle_reachability(&mut ctx, &impl_n, "model_impl", &resp_impl["analyses"]["reachability"], "impl")?;
    handle_safety(&mut ctx, &impl_n, &resp_impl["analyses"]["safety"])?;
    let impl_fail_states = handle_redundancy(&mut ctx, &fp, &impl_n, &resp_impl["analyses"]["redundancy"])?;

    // --- envelope: on the control plant if present, else on the model itself
    let mut plant_reach: Option<BTreeSet<usize>> = None;
    let winning: Option<BTreeSet<usize>>;
    if let Some(pn) = &plant_n {
        let resp_plant = worker::call(
            &ctx.tc,
            out,
            &WorkerRequest { request_id: "plant", method: "analyze", model_path: "core-plant.json", analyses: &["reachability", "envelope"], objective, certificate_path: None, max_states },
            ctx.timeout,
        )?;
        fs::write(out.join("worker-plant.json"), serde_json::to_string_pretty(&resp_plant)?)?;
        if resp_plant["status"] == "error" {
            bail!("worker (plant): {}", resp_plant["error"]);
        }
        plant_reach = Some(handle_reachability(&mut ctx, pn, "model_plant", &resp_plant["analyses"]["reachability"], "plant")?);
        winning = handle_envelope(&mut ctx, pn, "model_plant", &resp_plant["analyses"]["envelope"], "control-plant")?;
    } else {
        winning = handle_envelope(&mut ctx, &impl_n, "model_impl", &resp_impl["analyses"]["envelope"], "model")?;
    }
    if let (Some(pn), Some(pr), Some(w)) = (&plant_n, &plant_reach, &winning) {
        overrestriction(&mut ctx, &fp, &impl_n, pn, &impl_reach, &impl_fail_states, pr, w);
    }

    // --- kernel route
    let mut models: Vec<(&str, &mulu_model::CoreModel)> = vec![("model_impl", &impl_n.core)];
    if let Some(p) = &plant_n {
        models.push(("model_plant", &p.core));
    }
    let model_hash = sha256_hex(text.as_bytes());
    let entries: Vec<lean::CertEntry> = ctx.certs.iter().map(|(id, m, c)| lean::CertEntry { id, model: m, cert: c }).collect();
    let check_src = lean::generate(&models, &entries, &model_hash)?;
    let check_path = out.join("certificates/Check.lean");
    fs::write(&check_path, &check_src)?;
    let kernel = if tools.no_kernel {
        None
    } else {
        match ctx.tc.lean_dir.as_deref() {
            Some(ld) => {
                let r = lean::run_kernel(ld, &check_path)?;
                fs::write(out.join("certificates/kernel.log"), &r.log)?;
                fs::write(out.join("certificates/axioms.txt"), r.axioms.iter().map(|(n, a)| format!("{n}: [{}]\n", a.join(", "))).collect::<String>())?;
                Some(r)
            }
            None => {
                eprintln!("warning: Lean project not found, kernel check skipped (pass --lean-dir)");
                None
            }
        }
    };
    let kernel_ok = kernel.as_ref().map(|k| k.ok);
    for d in &mut ctx.diags {
        if let Some(e) = &mut d.evidence {
            if e.kind == "lean-certificate" {
                e.kernel_checked = kernel_ok;
            }
        }
    }
    if let Some(k) = &kernel {
        if !k.ok {
            eprintln!("kernel check FAILED:\n{}", k.log);
            if !k.bad_axioms.is_empty() {
                eprintln!("disallowed axioms: {:?}", k.bad_axioms);
            }
        }
    }

    // --- correspondence (P1-04): a finding is reported at the deepest layer
    //     whose obligations are discharged. None is, so nothing moves past
    //     the model, and the ledger says exactly what is missing.
    // A model handed in directly still gets a ledger: it says there is nothing
    // for the model to correspond to, which is why its findings stop there.
    let owned = ledger.cloned().unwrap_or_else(obligations::Ledger::model_only);
    for d in &mut ctx.diags {
        let deps = owned.depends_on(d.kind, d.check_id.as_deref());
        let refs: Vec<&str> = deps.iter().map(|s| s.as_str()).collect();
        d.scope = owned.promoted_scope(&refs);
        d.obligations = deps;
    }
    fs::write(out.join("obligations.json"), serde_json::to_string_pretty(&owned)?)?;

    // --- concrete reproduction (P1-03), beside the certificate rather than
    //     in place of it
    if let Some(rep) = reproducer {
        for d in &mut ctx.diags {
            if let Some(v) = rep(d) {
                d.reproduction = Some(v);
            }
        }
    }

    // --- exit code
    let any_unchecked = ctx.diags.iter().any(|d| d.evidence.as_ref().map(|e| !e.checked).unwrap_or(false));
    let code = if any_unchecked || kernel_ok == Some(false) || !ctx.cross_check_errors.is_empty() {
        4
    } else if ctx
        .diags
        .iter()
        .any(|d| d.reproduction.as_ref().is_some_and(|r| r["status"] == "not-reproduced"))
    {
        // The model and the EVM disagree. docs/08 §5: a replay that does not
        // reproduce is not evidence the counterexample was spurious, so this
        // is a gap to look at, not a finding to dismiss.
        2
    } else if ctx.statuses.iter().any(|(_, s)| s == "partial" || s == "unsupported") {
        2
    } else if ctx.diags.iter().any(|d| d.kind == "spec-violation" && d.status == "proven")
        || (fail_on_candidate && ctx.diags.iter().any(|d| d.status == "candidate"))
    {
        1
    } else {
        0
    };
    for e in &ctx.cross_check_errors {
        eprintln!("cross-check mismatch (tool bug): {e}");
    }

    // --- manifest + report
    let manifest = json!({
        "schema_version": 1,
        "tool": TOOL,
        "created_unix": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs(),
        "input": {"model": model.display().to_string(), "model_sha256": model_hash,
                  "core_model_sha256": sha256_hex(core_json.as_bytes())},
        "objective": objective,
        "semantics": {"finite-product": 1, "lean": "Mulu (lean/), kernel tactic: decide", "allowed_axioms": lean::ALLOWED_AXIOMS},
        "provenance": provenance,
        "state_names": impl_n.state_names, "event_names": impl_n.event_names, "check_names": impl_n.check_names,
        "plant_state_names": plant_n.as_ref().map(|p| p.state_names.clone()),
        "plant_event_names": plant_n.as_ref().map(|p| p.event_names.clone()),
        "certificates": ctx.certs.iter().map(|(id, m, _)| json!({"id": id, "model": m, "path": format!("certificates/{}.json", lean::ident(id))})).collect::<Vec<_>>(),
        "kernel": {"ran": kernel.is_some(), "ok": kernel_ok, "check_file": "certificates/Check.lean"},
    });
    fs::write(out.join("manifest.json"), serde_json::to_string_pretty(&manifest)?)?;
    let report = json!({
        "schema_version": 1,
        "tool": TOOL,
        "manifest": "manifest.json",
        "summary": {"exit_code": code, "analyses": ctx.statuses.iter().map(|(a, s)| json!({"analysis": a, "status": s})).collect::<Vec<_>>(),
                    "kernel_checked": kernel_ok, "cross_check_errors": ctx.cross_check_errors},
        "diagnostics": ctx.diags,
        "statistics": {"impl": resp_impl["statistics"].clone()},
    });
    fs::write(out.join("report.json"), serde_json::to_string_pretty(&report)?)?;
    report::print_human(&ctx.diags, out);
    println!("exit code: {code}");
    Ok(code)
}

fn evidence(path: Option<String>, checked: bool) -> Option<Evidence> {
    Some(Evidence { kind: "lean-certificate", path, checked, kernel_checked: None })
}

fn handle_reachability(ctx: &mut Ctx, n: &Normalized, model: &str, a: &Value, tag: &str) -> Result<BTreeSet<usize>> {
    let status = a["status"].as_str().unwrap_or("error").to_string();
    ctx.status(&format!("reachability-{tag}"), &status);
    let r = nat_set(&a["states"]);
    let reference = reference::reach(&n.core);
    if status == "complete" && r != reference {
        ctx.cross_check_errors.push(format!("reachability-{tag}: worker {:?} vs reference {:?}", r, reference));
    }
    if a["checked"].as_bool() == Some(true) {
        ctx.add_cert(&format!("reach-{tag}"), model, &a["certificate"])?;
    }
    Ok(r)
}

fn handle_safety(ctx: &mut Ctx, n: &Normalized, a: &Value) -> Result<()> {
    let status = a["status"].as_str().unwrap_or("error").to_string();
    ctx.status("safety", &status);
    if status == "not-requested" {
        ctx.diags.push(Diagnostic {
            id: "safety".into(),
            kind: "spec-violation",
            claim: "bad-reachable",
            status: "not-requested",
            scope: "abstract-model",
            severity: "INFO",
            message: "no specification (no bad states): violation search not requested".into(),
            check_id: None,
            depends_on: vec![],
            assumptions: MODEL_ASSUMPTIONS.to_vec(),
            evidence: None,
            detail: None,
            reproduction: None,
            obligations: vec![],
        });
        return Ok(());
    }
    if let Some(v) = a["violation"].as_object() {
        let checked = v["checked"].as_bool() == Some(true);
        let path = ctx.add_cert("safety-violation", "model_impl", &v["certificate"])?;
        let steps = edge_names(n, &v["path"]);
        let trace: Vec<String> = steps.iter().map(|e| format!("{} --{}--> {}", e["from"].as_str().unwrap(), e["event"].as_str().unwrap(), e["to"].as_str().unwrap())).collect();
        ctx.diags.push(Diagnostic {
            id: "spec-violation".into(),
            kind: "spec-violation",
            claim: "bad-reachable",
            status: if checked { "proven" } else { "candidate" },
            scope: "abstract-model",
            severity: "WARNING",
            message: format!("a bad state is reachable:\n{}", trace.join("\n")),
            check_id: None,
            depends_on: vec![],
            assumptions: MODEL_ASSUMPTIONS.to_vec(),
            evidence: evidence(Some(path), checked),
            detail: Some(json!({"path": steps})),
            reproduction: None,
            obligations: vec![],
        });
    } else {
        let checked = a["checked"].as_bool() == Some(true);
        let path = if checked { Some(ctx.add_cert("safety-invariant", "model_impl", &a["certificate"])?) } else { None };
        ctx.diags.push(Diagnostic {
            id: "model-safe".into(),
            kind: "model-safe",
            claim: "bad-unreachable",
            status: if checked && status == "complete" { "proven" } else { "unknown" },
            scope: "abstract-model",
            severity: "INFO",
            message: "no bad state is reachable (checked invariant avoids all bad states)".into(),
            check_id: None,
            depends_on: vec![],
            assumptions: MODEL_ASSUMPTIONS.to_vec(),
            evidence: if checked { evidence(path, true) } else { None },
            detail: None,
            reproduction: None,
            obligations: vec![],
        });
    }
    Ok(())
}

/// Returns, per check id, the impl states where its fail event is enabled from the reachable set.
fn handle_redundancy(ctx: &mut Ctx, fp: &FiniteProduct, n: &Normalized, a: &Value) -> Result<Vec<(String, Vec<usize>)>> {
    let status = a["status"].as_str().unwrap_or("error").to_string();
    ctx.status("redundancy", &status);
    let mut fails = vec![];
    let reach = reference::reach(&n.core);
    for item in a["checks"].as_array().cloned().unwrap_or_default() {
        let idx = item["id"].as_u64().unwrap_or(0) as usize;
        let decl = &fp.checks[idx];
        let st = item["status"].as_str().unwrap_or("unknown");
        let fail_ev = n.core.checks[idx].fail_event;
        let fail_states: Vec<usize> = n.core.edges.iter().filter(|e| e[1] == fail_ev && reach.contains(&e[0])).map(|e| e[0]).collect();
        fails.push((decl.id.clone(), fail_states.clone()));
        let (kind, claim, dstatus, severity, message): (&str, &str, &str, &str, String) = match st {
            "never-fails" => ("redundant-check", "never-fails", "proven", "HINT", format!("check {} never fails on any reachable model state.", decl.id)),
            "unreachable" => ("unreachable-check", "unreachable", "proven", "INFO", format!("check {} is never evaluated from the initial states (dead code on the model).", decl.id)),
            "may-fail" => ("check-may-fail", "may-fail", "candidate", "INFO", format!("check {} can fail from reachable state(s): {}", decl.id, fail_states.iter().map(|&q| n.state_name(q)).collect::<Vec<_>>().join(", "))),
            _ => ("check-unknown", "never-fails", "unknown", "INFO", format!("check {}: reachability incomplete", decl.id)),
        };
        let ev = if item["checked"].as_bool() == Some(true) {
            let p = ctx.add_cert(&format!("redundancy-{}", decl.id), "model_impl", &item["certificate"])?;
            evidence(Some(p), true)
        } else {
            None
        };
        ctx.diags.push(Diagnostic {
            id: format!("check-{}", decl.id),
            kind: Box::leak(kind.to_string().into_boxed_str()),
            claim: Box::leak(claim.to_string().into_boxed_str()),
            status: Box::leak(dstatus.to_string().into_boxed_str()),
            scope: "abstract-model",
            severity: Box::leak(severity.to_string().into_boxed_str()),
            message,
            check_id: Some(decl.id.clone()),
            depends_on: decl.depends_on.clone(),
            assumptions: [MODEL_ASSUMPTIONS, &["pure-guard", "never-fails-not-removal-equivalent"]].concat(),
            evidence: ev,
            detail: None,
            reproduction: None,
            obligations: vec![],
        });
    }
    Ok(fails)
}

fn handle_envelope(ctx: &mut Ctx, n: &Normalized, model: &str, a: &Value, on: &str) -> Result<Option<BTreeSet<usize>>> {
    let status = a["status"].as_str().unwrap_or("error").to_string();
    ctx.status("envelope", &status);
    let nb = ctx.objective != "safety";
    match status.as_str() {
        "complete" | "unrealizable" => {
            let w = nat_set(&a["winning"]);
            let reference = reference::envelope_iter(&n.core, nb);
            if w != reference {
                ctx.cross_check_errors.push(format!("envelope: worker {:?} vs reference {:?}", w, reference));
            }
            if n.core.num_states <= 14 {
                let bf = reference::envelope_bruteforce(&n.core, nb);
                if w != bf {
                    ctx.cross_check_errors.push(format!("envelope: worker {:?} vs brute force {:?}", w, bf));
                }
            }
            let checked = a["checked"].as_bool() == Some(true);
            let path = ctx.add_cert("envelope", model, &a["certificate"])?;
            let disabled = edge_names(n, &a["disabled"]);
            let winning_names = names(n, &a["winning"]);
            let (claim, msg, sev): (&str, String, &str) = if status == "complete" {
                let dis: Vec<String> = disabled.iter().map(|e| format!("({}, {})", e["from"].as_str().unwrap(), e["event"].as_str().unwrap())).collect();
                ("maximal-permissive", format!("maximal permissive envelope on the {on}: winning = {{{}}}\ndisable = {{{}}}", winning_names.join(", "), dis.join(", ")), "INFO")
            } else {
                ("unrealizable", format!("no supervisor can enforce the objective from the initial state(s) on the {on}: an uncontrollable path to bad (or a blocking region) cannot be avoided"), "WARNING")
            };
            ctx.diags.push(Diagnostic {
                id: "envelope".into(),
                kind: "envelope",
                claim: Box::leak(claim.to_string().into_boxed_str()),
                status: if checked { "proven" } else { "candidate" },
                scope: "abstract-model",
                severity: sev,
                message: msg,
                check_id: None,
                depends_on: vec![],
                assumptions: [MODEL_ASSUMPTIONS, &["conservative-reference-plant", "partial-determinism"]].concat(),
                evidence: evidence(Some(path), checked),
                detail: Some(json!({"objective": a["objective"], "winning": winning_names, "disabled": disabled, "pruning_rounds": a["pruning_rounds"]})),
                reproduction: None,
                obligations: vec![],
            });
            Ok(Some(w))
        }
        other => {
            ctx.diags.push(Diagnostic {
                id: "envelope".into(),
                kind: "envelope",
                claim: "maximal-permissive",
                status: if other == "not-requested" { "not-requested" } else { "unknown" },
                scope: "abstract-model",
                severity: "INFO",
                message: format!("envelope {other}: {}", a["reason"].as_str().unwrap_or("")),
                check_id: None,
                depends_on: vec![],
                assumptions: MODEL_ASSUMPTIONS.to_vec(),
                evidence: None,
                detail: None,
                reproduction: None,
                obligations: vec![],
            });
            Ok(None)
        }
    }
}

/// Overrestriction candidates (docs/04 §5, docs/11 §4): same request, the
/// implementation rejects (fail edge from a reachable impl state) while the
/// envelope allows continuing (continue edge stays inside W from a reachable
/// plant state). Reported as *candidate*, never as proven.
#[allow(clippy::too_many_arguments)]
fn overrestriction(ctx: &mut Ctx, fp: &FiniteProduct, impl_n: &Normalized, plant_n: &Normalized, impl_reach: &BTreeSet<usize>, impl_fails: &[(String, Vec<usize>)], plant_reach: &BTreeSet<usize>, w: &BTreeSet<usize>) {
    let cp = fp.control_plant.as_ref().unwrap();
    // docs/04 §1: overrestriction is a comparison against a specification.
    // Without one the envelope constrains nothing, so every rejection would
    // qualify and the answer would say nothing.
    if cp.bad.is_empty() && fp.bad.is_empty() {
        ctx.status("overrestriction", "not-requested");
        ctx.diags.push(Diagnostic {
            id: "overrestriction".into(),
            kind: "overrestriction",
            claim: "spec-permits-rejected-request",
            status: "not-requested",
            scope: "abstract-model",
            severity: "INFO",
            message: "no specification: what a check is too strict *for* is undefined, so \
                      overrestriction is not analysed"
                .into(),
            check_id: None,
            depends_on: vec![],
            assumptions: MODEL_ASSUMPTIONS.to_vec(),
            evidence: None,
            detail: None,
            reproduction: None,
            obligations: vec![],
        });
        return;
    }
    let accepting: BTreeSet<usize> = cp
        .accepting
        .as_ref()
        .map(|a| a.iter().filter_map(|s| plant_n.state_index(s)).collect())
        .unwrap_or_else(|| plant_n.core.marked.iter().copied().collect());
    // States of W from which the *same request* can still be accepted inside the
    // envelope: paths may not pass through a transaction boundary (a marked,
    // non-accepting state such as `rev` or `idle`) before reaching an accepting state.
    let marked: BTreeSet<usize> = plant_n.core.marked.iter().copied().collect();
    let within: BTreeSet<usize> = w.iter().copied().filter(|q| !marked.contains(q) || accepting.contains(q)).collect();
    let can_accept = reference::coreach_within(&plant_n.core, &within, &accepting);
    for site in &cp.sites {
        let Some(check) = &site.check else { continue };
        let cont = plant_n.event_index(&site.continue_event).unwrap();
        let fail_states = impl_fails.iter().find(|(c, _)| c == check).map(|(_, s)| s.clone()).unwrap_or_default();
        for pair in &site.pairs {
            let qp = plant_n.state_index(&pair.plant_state).unwrap();
            let qi = impl_n.state_index(&pair.impl_state).unwrap();
            let allowed = plant_reach.contains(&qp)
                && w.contains(&qp)
                && plant_n.core.edges.iter().any(|e| e[0] == qp && e[1] == cont && w.contains(&e[2]) && can_accept.contains(&e[2]));
            let rejected = impl_reach.contains(&qi) && fail_states.contains(&qi);
            if allowed && rejected {
                ctx.diags.push(Diagnostic {
                    id: format!("overrestriction-{}-{}", check, pair.impl_state),
                    kind: "overrestriction",
                    claim: "spec-permits-rejected-request",
                    status: "candidate",
                    scope: "abstract-model",
                    severity: "INFO",
                    message: format!(
                        "check {check} rejects at {} but the envelope allows continuing at the corresponding plant state {} (site {}) and the request can still complete successfully inside the envelope.\nCandidate for relaxation; this does not say the check can be removed.",
                        pair.impl_state, pair.plant_state, site.id
                    ),
                    check_id: Some(check.clone()),
                    depends_on: vec![],
                    assumptions: [MODEL_ASSUMPTIONS, &["conservative-reference-plant", "site-pair-correspondence-unproven"]].concat(),
                    evidence: None,
                    detail: Some(json!({"impl_state": pair.impl_state, "plant_state": pair.plant_state, "site": site.id, "continue_event": site.continue_event})),
                    reproduction: None,
                    obligations: vec![],
                });
            }
        }
    }
}

// ---------------------------------------------------------------------------
// verify

fn verify(dir: &Path, tools: &ToolArgs) -> Result<i32> {
    let manifest: Value = serde_json::from_str(&fs::read_to_string(dir.join("manifest.json")).context("reading manifest.json")?)?;
    let text = fs::read_to_string(dir.join("model.json")).context("reading model.json")?;
    let mut failures: Vec<String> = vec![];
    let model_hash = sha256_hex(text.as_bytes());
    if manifest["input"]["model_sha256"] != model_hash {
        failures.push("model.json hash differs from manifest".into());
    }
    let fp = parse_and_validate(&text).map_err(|e| anyhow!("model.json: {e}"))?;
    let impl_n = normalize(&fp);
    let plant_n = fp.control_plant.as_ref().map(normalize_plant);
    let core_json = canonical_json(&impl_n.core.to_json());
    if fs::read_to_string(dir.join("core-model.json"))? != core_json {
        failures.push("core-model.json does not match the normalisation of model.json".into());
    }
    if let Some(p) = &plant_n {
        if fs::read_to_string(dir.join("core-plant.json")).unwrap_or_default() != canonical_json(&p.core.to_json()) {
            failures.push("core-plant.json does not match the normalisation of model.json".into());
        }
    }
    let tc = Toolchain::discover(tools.lean_dir.clone(), tools.worker.clone());
    let certs: Vec<Value> = manifest["certificates"].as_array().cloned().unwrap_or_default();
    let mut loaded: Vec<(String, String, Value)> = vec![];
    for c in &certs {
        let id = c["id"].as_str().unwrap_or("?").to_string();
        let model = c["model"].as_str().unwrap_or("model_impl").to_string();
        let rel = c["path"].as_str().unwrap_or("").to_string();
        let cert: Value = match fs::read_to_string(dir.join(&rel)) {
            Ok(s) => serde_json::from_str(&s)?,
            Err(e) => {
                failures.push(format!("{rel}: {e}"));
                continue;
            }
        };
        let model_path = if model == "model_plant" { "core-plant.json" } else { "core-model.json" };
        let resp = worker::call(
            &tc,
            dir,
            &WorkerRequest { request_id: &id, method: "verify", model_path, analyses: &[], objective: "safety-nonblocking", certificate_path: Some(&rel), max_states: usize::MAX },
            Duration::from_millis(tools.timeout_ms),
        )?;
        let ok = resp["checked"].as_bool() == Some(true);
        println!("{:<9} {id}  ({model}, worker)", if ok { "OK" } else { "FAIL" });
        if !ok {
            failures.push(format!("certificate {id} rejected by the worker: {}", resp["error"].as_str().unwrap_or("check returned false")));
        }
        loaded.push((id, model, cert));
    }
    // A reported scope must be one the ledger allows. Without this the layer
    // a claim is made at would be a note in a file, not a checked property.
    let ledger_path = dir.join("obligations.json");
    // Both files must be here. Requiring only one lets the pair be deleted
    // together, and the layer check would then pass by having nothing to check.
    if !dir.join("report.json").exists() {
        failures.push("report.json is missing, so there are no findings to check".into());
    }
    if !ledger_path.exists() {
        failures.push(
            "obligations.json is missing, so no diagnostic's layer could be checked".into(),
        );
    }
    if ledger_path.exists() {
        let ledger_text = fs::read_to_string(&ledger_path).context("reading obligations.json")?;
        let l: obligations::Ledger =
            serde_json::from_str(&ledger_text).context("parsing obligations.json")?;
        let report: Value = serde_json::from_str(
            &fs::read_to_string(dir.join("report.json")).context("reading report.json")?,
        )?;
        // The ledger's kind decides what a finding rests on, so it is not the
        // report's to choose. Bind it to how the analysis was actually run.
        let from_solidity = !manifest["provenance"].is_null();
        let expected_kind = if from_solidity { "solidity" } else { "model-only" };
        if l.kind != expected_kind {
            failures.push(format!(
                "the ledger says it is a {:?} analysis but the manifest says {expected_kind:?}",
                l.kind
            ));
        }

        // The tool discharges nothing, so a discharge in the ledger is a claim
        // no one checked. Refuse it rather than let it raise a scope.
        for o in l.obligations.iter().filter(|o| !o.open()) {
            failures.push(format!(
                "obligation {} claims to be discharged by {:?}, and nothing here can check \
                 that; P1-04 discharges none",
                o.id,
                o.discharged_by.as_deref().unwrap_or("?")
            ));
        }
        let mut checked = 0usize;
        for d in report["diagnostics"].as_array().cloned().unwrap_or_default() {
            let id = d["id"].as_str().unwrap_or("?").to_string();
            let claimed = d["scope"].as_str().unwrap_or("");
            let listed: Vec<String> = d["obligations"]
                .as_array()
                .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
                .unwrap_or_default();

            // Recompute what this finding rests on from what it *is*. Reading
            // the list off the report would let a shortened list raise the
            // layer as soon as any obligation is discharged.
            let kind = d["kind"].as_str().unwrap_or("");
            let check_id = d["check_id"].as_str();
            let mut expected = l.depends_on(kind, check_id);
            expected.sort();
            let mut listed_sorted = listed.clone();
            listed_sorted.sort();
            if listed_sorted != expected {
                failures.push(format!(
                    "{id} lists obligations {listed_sorted:?} but a {kind:?} finding rests on                      {expected:?}"
                ));
            }
            for o in &expected {
                if l.find(o).is_none() {
                    failures.push(format!("{id} rests on {o}, which the ledger does not record"));
                }
            }

            let refs: Vec<&str> = expected.iter().map(|s| s.as_str()).collect();
            let allowed = l.promoted_scope(&refs);
            if claimed != allowed {
                failures.push(format!(
                    "{id} is reported at {claimed:?} but its obligations reach {allowed:?}"
                ));
            }
            checked += 1;
        }
        println!(
            "{:<9} {checked} diagnostic scope(s) against {} obligation(s), {} open",
            "OK",
            l.obligations.len(),
            l.open().len()
        );
    }

    // kernel route: regenerate and compare byte-for-byte, then run
    let mut models: Vec<(&str, &mulu_model::CoreModel)> = vec![("model_impl", &impl_n.core)];
    if let Some(p) = &plant_n {
        models.push(("model_plant", &p.core));
    }
    let entries: Vec<lean::CertEntry> = loaded.iter().map(|(id, m, c)| lean::CertEntry { id, model: m, cert: c }).collect();
    let regenerated = lean::generate(&models, &entries, &model_hash)?;
    let stored = fs::read_to_string(dir.join("certificates/Check.lean")).unwrap_or_default();
    if stored != regenerated {
        failures.push("certificates/Check.lean differs from the regenerated file (model or certificates changed)".into());
    }
    if !tools.no_kernel {
        match tc.lean_dir.as_deref() {
            Some(ld) => {
                let r = lean::run_kernel(ld, &dir.join("certificates/Check.lean"))?;
                for (n, a) in &r.axioms {
                    println!("{:<9} {n}  axioms: [{}]", "KERNEL", a.join(", "));
                }
                if !r.ok {
                    failures.push(format!("kernel check failed:\n{}", r.log));
                }
            }
            None => failures.push("Lean project not found: cannot run the kernel check (pass --lean-dir or --no-kernel)".into()),
        }
    }
    if failures.is_empty() {
        println!("verify: OK ({} certificates, model sha256 {})", loaded.len(), &model_hash[..12]);
        Ok(0)
    } else {
        for f in &failures {
            eprintln!("verify: FAIL: {f}");
        }
        Ok(4)
    }
}
