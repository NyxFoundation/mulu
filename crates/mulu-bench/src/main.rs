//! `mulu-bench` — how far mulu gets on a corpus it did not choose.
//!
//! The point is not a score. It is the histogram of *why* mulu stops, because
//! that is the list of things to implement, in the order the corpus says they
//! matter. A tool that reports three finding types on contracts nobody writes
//! has not achieved anything, and the only way to know which those are is to
//! run it on programs someone else picked.
//!
//! Stages, each of which a contract either reaches or stops before:
//!
//! | stage | what it means |
//! |---|---|
//! | `compiled` | solc produced Yul for it at all |
//! | `lowered` | mulu turned that Yul into a ProgramIR |
//! | `modelled` | the abstraction produced a model with nothing unsupported |
//!
//! A contract that stops carries the reason, and the reasons are counted.

mod corpus;

use anyhow::Result;
use clap::Parser;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[derive(Parser)]
#[command(name = "mulu-bench", about = "Measure mulu against a corpus of real Solidity")]
struct Args {
    /// The corpus root, e.g. a checkout of solc's `test/libsolidity/semanticTests`
    corpus: PathBuf,
    /// Where to write the per-case results
    #[arg(long)]
    out: PathBuf,
    /// Path to solc (default: $MULU_SOLC, then PATH)
    #[arg(long)]
    solc: Option<PathBuf>,
    #[arg(long, default_value = "8")]
    jobs: usize,
    /// Stop after this many cases, for a quick look
    #[arg(long)]
    limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
struct Outcome {
    name: String,
    /// `out-of-scope`, `compiled`, `lowered`, `modelled`
    stage: &'static str,
    /// Why it stopped, empty when it reached the last stage.
    reason: String,
    calls: usize,
    /// Entrypoints the abstraction modelled, when it got that far.
    entrypoints: usize,
    checks: usize,
    states: usize,
}

/// The reason strings carry contract and function names, which would make
/// every case its own category. This keeps the part that says what kind of
/// thing was not handled.
fn category(reason: &str) -> String {
    let r = reason.trim();
    for (needle, label) in [
        ("carries effects the model does not represent", "an instruction with effects the model cannot represent"),
        ("is not one of this function's parameters", "a guard over something that is not an argument"),
        ("no entrypoint", "no entrypoint the abstraction could model"),
        ("unsupported builtin", "an unsupported Yul builtin"),
        ("no deployed object", "no deployed object in the Yul"),
        ("abstract, an interface or a library", "no code (abstract, interface or library)"),
        ("no contract with code", "no contract with code"),
        ("storage layout", "a storage layout the abstraction cannot read"),
        ("parse", "the Yul did not parse"),
        ("Compile", "solc rejected it"),
        ("compiling", "solc rejected it"),
    ] {
        if r.contains(needle) {
            return label.to_string();
        }
    }
    // Fall back to the first line, trimmed. Splitting on punctuation picked
    // up fragments of a compiler diagnostic's source excerpt, which made a
    // category out of whatever the caret happened to sit under.
    r.lines().next().unwrap_or(r).trim().chars().take(90).collect()
}

fn measure(case: &corpus::Case, solc: Option<PathBuf>) -> Outcome {
    let mut o = Outcome {
        name: case.name.clone(),
        stage: "out-of-scope",
        reason: String::new(),
        calls: case.expectations.len(),
        entrypoints: 0,
        checks: 0,
        states: 0,
    };
    if let Some(why) = case.out_of_scope() {
        o.reason = why.to_string();
        return o;
    }
    // solc, then mulu's front end. `compile_and_lower` does both, so a
    // failure is attributed by looking at where the message came from.
    let dir = std::env::temp_dir().join(format!(
        "mulu-bench-{}-{}",
        std::process::id(),
        case.name.replace(['/', '.'], "_")
    ));
    let _ = std::fs::remove_dir_all(&dir);
    if std::fs::create_dir_all(&dir).is_err() {
        o.reason = "could not make a working directory".into();
        return o;
    }
    let mut written = vec![];
    for (name, body) in &case.sources {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if std::fs::write(&path, body).is_err() {
            o.reason = "could not write the source".into();
            let _ = std::fs::remove_dir_all(&dir);
            return o;
        }
        written.push(path);
    }

    let lowered = compile_and_lower(&written, solc);
    let (bundle, name, ir) = match lowered {
        Ok(v) => v,
        Err(e) => {
            let msg = format!("{e:#}");
            // solc rejecting the source is about solc, not about mulu: the
            // corpus tracks a newer compiler than the one pinned here, and a
            // feature it does not know is not a coverage gap.
            o.stage = if msg.contains("compiling with") || msg.contains("compilation failed") {
                "out-of-scope"
            } else {
                "compiled"
            };
            o.reason = if o.stage == "out-of-scope" {
                "the pinned solc rejected the source".to_string()
            } else {
                category(&msg)
            };
            let _ = std::fs::remove_dir_all(&dir);
            return o;
        }
    };
    let _ = bundle;
    let _ = name;
    o.stage = "lowered";
    o.checks = ir.checks.len();

    // The abstraction, with no specification: the redundancy half of the
    // tool, which is the half that needs nothing from the user.
    let abstraction = mulu_abstraction::model::Builder::new(&ir, &[]).build();
    o.entrypoints = abstraction.report.entrypoints_modelled.len();
    o.states = abstraction.model.states.len();
    if abstraction.report.complete() {
        o.stage = "modelled";
    } else {
        o.reason = category(abstraction.report.unsupported.first().map(|s| s.as_str()).unwrap_or(""));
    }
    let _ = std::fs::remove_dir_all(&dir);
    o
}

/// A copy of the CLI's front end, without the CLI's printing. Kept here
/// rather than shared so a change in the reporting cannot move a number.
fn compile_and_lower(
    sources: &[PathBuf],
    solc_path: Option<PathBuf>,
) -> Result<(mulu_solc::BuildBundle, String, mulu_yul::ProgramIr)> {
    let solc = mulu_solc::Solc::discover(solc_path)?;
    let root = sources[0].parent().unwrap_or(std::path::Path::new(".")).to_path_buf();
    let opts = mulu_solc::CompileOptions::default();
    let bundle = solc.compile_files(&root, sources, &opts)?;
    // solc's own test runner instantiates the *last* contract in the file;
    // the others are helpers. Asking for "the only one" instead made a
    // failure out of every test that defines a helper.
    let selected = match mulu_solc::driver_select(&bundle, None) {
        Ok(c) => c.name.clone(),
        Err(_) => bundle
            .contracts
            .iter()
            .filter(|c| c.bytecode.as_deref().is_some_and(|b| !b.is_empty()))
            .next_back()
            .ok_or_else(|| anyhow::anyhow!("this build defines no contract with code"))?
            .name
            .clone(),
    };
    let c = bundle.contract(&selected).expect("just selected");
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
    let ir = mulu_yul::lower_contract_with(
        &c.name,
        &c.source_path,
        &bundle.compiler,
        &c.ir,
        &c.abi,
        c.storage_layout.clone(),
        mulu_yul::SolcFacts { origins: Some(&lookup), selectors },
    )?;
    Ok((bundle, selected, ir))
}

fn main() -> Result<()> {
    let args = Args::parse();
    let mut cases = corpus::load(&args.corpus)?;
    if let Some(n) = args.limit {
        cases.truncate(n);
    }
    let total = cases.len();
    eprintln!("{total} case(s) from {}", args.corpus.display());

    let queue = Arc::new(Mutex::new(cases.into_iter()));
    let results = Arc::new(Mutex::new(Vec::<Outcome>::new()));
    let done = Arc::new(Mutex::new(0usize));
    std::thread::scope(|s| {
        for _ in 0..args.jobs.max(1) {
            let (queue, results, done, solc) =
                (queue.clone(), results.clone(), done.clone(), args.solc.clone());
            s.spawn(move || loop {
                let Some(case) = queue.lock().unwrap().next() else { return };
                let o = measure(&case, solc.clone());
                results.lock().unwrap().push(o);
                let mut d = done.lock().unwrap();
                *d += 1;
                if *d % 100 == 0 {
                    eprintln!("  {}/{total}", *d);
                }
            });
        }
    });

    let mut out = Arc::try_unwrap(results).unwrap().into_inner().unwrap();
    out.sort_by(|a, b| a.name.cmp(&b.name));

    let mut stages: BTreeMap<&str, usize> = BTreeMap::new();
    let mut reasons: BTreeMap<String, usize> = BTreeMap::new();
    for o in &out {
        *stages.entry(o.stage).or_default() += 1;
        if !o.reason.is_empty() {
            *reasons.entry(o.reason.clone()).or_default() += 1;
        }
    }
    let in_scope = out.iter().filter(|o| o.stage != "out-of-scope").count();
    let modelled = *stages.get("modelled").unwrap_or(&0);

    println!("\n{total} case(s), {in_scope} in scope");
    for (stage, n) in &stages {
        println!("  {stage:<14} {n:>5}");
    }
    if in_scope > 0 {
        println!("\n  modelled / in scope: {modelled}/{in_scope} = {:.1}%", 100.0 * modelled as f64 / in_scope as f64);
    }
    println!("\nwhy the rest stopped");
    let mut ranked: Vec<(&String, &usize)> = reasons.iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(a.1));
    for (reason, n) in ranked.iter().take(15) {
        println!("  {n:>5}  {reason}");
    }

    std::fs::create_dir_all(args.out.parent().unwrap_or(std::path::Path::new(".")))?;
    let doc = serde_json::json!({
        "corpus": args.corpus.display().to_string(),
        "total": total,
        "in_scope": in_scope,
        "stages": stages,
        "reasons": reasons,
        "cases": out,
    });
    std::fs::write(&args.out, serde_json::to_string_pretty(&doc)?)?;
    println!("\nwrote {}", args.out.display());
    Ok(())
}
