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
    /// Which corpus `corpus` is. `semantic-tests` reads solc's `// ----`
    /// footers; `verification-benchmark` reads
    /// `contracts/<use case>/versions/` and the shared `lib/`.
    #[arg(long, value_enum, default_value = "semantic-tests")]
    corpus_kind: CorpusKind,
    /// A directory of mulu property files, one per use case, scored against
    /// the corpus's own `ground-truth.csv`.
    #[arg(long)]
    properties: Option<PathBuf>,
    /// Exit non-zero below this many modelled cases. What CI asserts: the
    /// floor catches a coverage regression, and the run finishing at all
    /// catches a return of the blowup that made a ten-line contract hang.
    #[arg(long)]
    min_modelled: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum CorpusKind {
    SemanticTests,
    VerificationBenchmark,
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
    /// Checks in the ProgramIR, which includes the ones solc inserts.
    checks: usize,
    /// Checks in the *model*: the guards mulu would report on. A model with
    /// none is a model with nothing to say, and counting those as coverage
    /// would flatter the number.
    model_checks: usize,
    states: usize,
    transitions: usize,
    /// One entry per property answered on this contract.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    answers: Vec<Scored>,
}

/// A property, what mulu said, and what the answer key says.
#[derive(Debug, Clone, Serialize)]
struct Scored {
    property: String,
    version: String,
    /// `holds`, `fails` or `undecided`.
    said: String,
    /// The answer key: true when the property holds of this version.
    truth: bool,
    /// `correct`, `wrong`, or `no-answer`.
    outcome: &'static str,
    /// The same, against the key with this project's corrections applied.
    /// Equal to `outcome` for every row no correction touches.
    corrected_outcome: &'static str,
    because: String,
}

/// A row of the corpus's `ground-truth.csv` this project believes is wrong.
///
/// The score against the key *as shipped* is the number this harness reports
/// first, and it is the one that counts: a tool does not grade its own
/// disagreements. These only add a second number beside it, so a reader can
/// see the difference and check it. Each one names a test in
/// `bench/disagreements/` that runs the counterexample on an EVM.
#[derive(Debug, Clone, serde::Deserialize)]
struct Correction {
    use_case: String,
    property: String,
    version: String,
    corrected: bool,
    #[serde(default)]
    test: String,
    #[serde(default)]
    why: String,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
struct CorrectionFile {
    #[serde(default)]
    corrections: Vec<Correction>,
}

/// The corpus's own `ground-truth.csv`, and the properties written against it.
///
/// The benchmark ships one encoding per tool: `certora/*.spec` for Certora,
/// `solcmc/*.sol` for solc's model checker. `bench/properties/*.json` is
/// mulu's, and this is where the two meet: for each (property, version) the
/// key holds, ask mulu, and compare.
struct AnswerKey {
    /// use case -> the properties written for it
    properties: BTreeMap<String, Vec<mulu_abstraction::call_property::CallProperty>>,
    /// use case -> the invariants its properties may rest on
    invariants: BTreeMap<String, Vec<mulu_abstraction::call_property::Invariant>>,
    /// (use case, property, version) -> does it hold
    truth: BTreeMap<(String, String, String), bool>,
    /// The same, where this project believes the shipped key is wrong.
    corrections: BTreeMap<(String, String, String), Correction>,
}

impl AnswerKey {
    fn load(dir: &std::path::Path, corpus: &std::path::Path) -> Result<Self> {
        let mut properties = BTreeMap::new();
        let mut invariants = BTreeMap::new();
        let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
            .map_err(|e| anyhow::anyhow!("reading {}: {e}", dir.display()))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "json"))
            .collect();
        files.sort();
        let mut corrections = BTreeMap::new();
        for f in files {
            let text = std::fs::read_to_string(&f)?;
            if f.file_name().is_some_and(|n| n == "corrections.json") {
                let file: CorrectionFile = serde_json::from_str(&text)
                    .map_err(|e| anyhow::anyhow!("{}: {e}", f.display()))?;
                for c in file.corrections {
                    corrections.insert(
                        (c.use_case.clone(), c.property.clone(), c.version.clone()),
                        c,
                    );
                }
                continue;
            }
            let file: mulu_abstraction::call_property::PropertyFile = serde_json::from_str(&text)
                .map_err(|e| anyhow::anyhow!("{}: {e}", f.display()))?;
            let use_case = if file.use_case.is_empty() {
                f.file_stem().unwrap_or_default().to_string_lossy().to_string()
            } else {
                file.use_case.clone()
            };
            invariants.insert(use_case.clone(), file.invariants);
            properties.insert(use_case, file.call_properties);
        }

        let mut truth = BTreeMap::new();
        let contracts = corpus.join("contracts");
        let mut dirs: Vec<PathBuf> = std::fs::read_dir(&contracts)
            .map_err(|e| anyhow::anyhow!("reading {}: {e}", contracts.display()))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_dir())
            .collect();
        dirs.sort();
        for d in dirs {
            let use_case = d.file_name().unwrap_or_default().to_string_lossy().to_string();
            let Ok(csv) = std::fs::read_to_string(d.join("ground-truth.csv")) else { continue };
            for line in csv.lines().skip(1) {
                let mut f = line.split(',');
                let (Some(prop), Some(version), Some(t)) = (f.next(), f.next(), f.next()) else {
                    continue;
                };
                truth.insert(
                    (use_case.clone(), prop.trim().to_string(), version.trim().to_string()),
                    t.trim() == "1",
                );
            }
        }
        Ok(Self { properties, invariants, truth, corrections })
    }

    /// `bank/Bank_v1.sol` -> the use case and the version.
    fn split(name: &str) -> Option<(String, String)> {
        let (use_case, file) = name.split_once('/')?;
        let stem = file.strip_suffix(".sol")?;
        let version = stem.rsplit_once('_')?.1.to_string();
        Some((use_case.to_string(), version))
    }

    fn score(
        &self,
        name: &str,
        paths: &[mulu_abstraction::model::PathSummary],
    ) -> Vec<Scored> {
        use mulu_abstraction::call_property::{check_with, Verdict};
        let Some((use_case, version)) = Self::split(name) else { return vec![] };
        let Some(props) = self.properties.get(&use_case) else { return vec![] };
        let mut out = vec![];
        for p in props {
            let Some(truth) =
                self.truth.get(&(use_case.clone(), p.id.clone(), version.clone())).copied()
            else {
                continue;
            };
            let inv = self.invariants.get(&use_case).map(|v| v.as_slice()).unwrap_or(&[]);
            let a = check_with(p, inv, paths);
            let key = (use_case.clone(), p.id.clone(), version.clone());
            let corrected_truth =
                self.corrections.get(&key).map(|c| c.corrected).unwrap_or(truth);
            let judge = |t: bool| match a.verdict {
                Verdict::Holds if t => ("holds", "correct"),
                Verdict::Holds => ("holds", "wrong"),
                Verdict::Fails if !t => ("fails", "correct"),
                Verdict::Fails => ("fails", "wrong"),
                Verdict::Undecided => ("undecided", "no-answer"),
            };
            let (said, outcome) = judge(truth);
            let (_, corrected_outcome) = judge(corrected_truth);
            out.push(Scored {
                property: p.id.clone(),
                version: version.clone(),
                said: said.to_string(),
                truth,
                outcome,
                corrected_outcome,
                because: if a.assuming.is_empty() {
                    a.because
                } else {
                    format!("{} [assuming {}]", a.because, a.assuming.join(", "))
                },
            });
        }
        out
    }
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

fn measure(case: &corpus::Case, solc: Option<PathBuf>, key: Option<&AnswerKey>) -> Outcome {
    let mut o = Outcome {
        name: case.name.clone(),
        stage: "out-of-scope",
        reason: String::new(),
        calls: case.expectations.len(),
        entrypoints: 0,
        checks: 0,
        model_checks: 0,
        states: 0,
        transitions: 0,
        answers: Vec::new(),
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
    for (i, (name, body)) in case.sources.iter().enumerate() {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if std::fs::write(&path, body).is_err() {
            o.reason = "could not write the source".into();
            let _ = std::fs::remove_dir_all(&dir);
            return o;
        }
        // A semantic test's `==== Source:` parts are all part of the test.
        // The verification benchmark's extra sources are the shared library
        // its imports name, and compiling those as top-level contracts would
        // let `driver_select` pick one of them instead of the contract under
        // measurement.
        if i == 0 || case.kind == corpus::Kind::SemanticTests {
            written.push(path);
        }
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
    o.transitions = abstraction.model.transitions.len();
    o.model_checks = abstraction.model.checks.len();
    if abstraction.report.complete() {
        o.stage = "modelled";
    } else {
        o.reason = category(abstraction.report.unsupported.first().map(|s| s.as_str()).unwrap_or(""));
    }
    // Score the properties written for this use case against the corpus's
    // own answer key. A contract whose model is incomplete is still asked:
    // an incomplete model can still settle a property, and saying nothing
    // would hide that it did.
    if let Some(key) = key {
        o.answers = key.score(&case.name, &abstraction.paths);
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
        mulu_yul::SolcFacts {
            origins: Some(&lookup),
            selectors,
            immutables: bundle.ast_index.immutables.clone(),
            enums: bundle.ast_index.enums.clone(),
        },
    )?;
    Ok((bundle, selected, ir))
}

fn main() -> Result<()> {
    let args = Args::parse();
    let key = match &args.properties {
        Some(dir) => Some(AnswerKey::load(dir, &args.corpus)?),
        None => None,
    };
    let mut cases = match args.corpus_kind {
        CorpusKind::SemanticTests => corpus::load(&args.corpus)?,
        CorpusKind::VerificationBenchmark => corpus::load_verification_benchmark(&args.corpus)?,
    };
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
            let key = key.as_ref();
            s.spawn(move || loop {
                let Some(case) = queue.lock().unwrap().next() else { return };
                let o = measure(&case, solc.clone(), key);
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
        println!(
            "\n  modelled / in scope: {modelled}/{in_scope} = {:.1}%",
            100.0 * modelled as f64 / in_scope as f64
        );
        // The number that says whether the coverage is worth anything: a
        // model with no guard in it is a model mulu has nothing to say about.
        let speaking = out.iter().filter(|o| o.stage == "modelled" && o.model_checks > 0).count();
        println!(
            "  of those, with a guard to report on: {speaking} = {:.1}% of in scope",
            100.0 * speaking as f64 / in_scope as f64
        );
    }
    // The answer key, when there is one. This is the number that means
    // something: a property mulu got wrong is visible here, and nowhere else.
    let scored: Vec<&Scored> = out.iter().flat_map(|o| o.answers.iter()).collect();
    if !scored.is_empty() {
        let correct = scored.iter().filter(|s| s.outcome == "correct").count();
        let wrong = scored.iter().filter(|s| s.outcome == "wrong").count();
        let none = scored.iter().filter(|s| s.outcome == "no-answer").count();
        println!("\nagainst the answer key");
        println!("  {} (property, version) pair(s) asked", scored.len());
        println!("  correct    {correct:>5}");
        println!("  wrong      {wrong:>5}");
        println!("  no answer  {none:>5}");
        println!(
            "  correct / asked: {:.1}%",
            100.0 * correct as f64 / scored.len() as f64
        );
        // The same score against the key with this project's corrections
        // applied. Second, and always beside the first: a tool does not get
        // to grade its own disagreements, and the point of printing both is
        // that the difference is visible rather than folded in.
        let corrected = scored.iter().filter(|s| s.corrected_outcome == "correct").count();
        if corrected != correct {
            println!(
                "  with this project's corrections to the key: {corrected} correct ({:.1}%), \
                 each one a test in bench/disagreements/",
                100.0 * corrected as f64 / scored.len() as f64
            );
        }
        if wrong > 0 {
            println!("\n  wrong:");
            for s in scored.iter().filter(|s| s.outcome == "wrong") {
                println!(
                    "    {}/{} said {}, key says {}: {}",
                    s.property,
                    s.version,
                    s.said,
                    if s.truth { "holds" } else { "fails" },
                    s.because
                );
            }
        }
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

    if let Some(floor) = args.min_modelled {
        if modelled < floor {
            eprintln!(
                "\n{modelled} case(s) reached a complete model and the floor is {floor}. \
                 Either something regressed, or the floor is stale and this run is the new \
                 number; both are worth looking at before it is moved."
            );
            std::process::exit(1);
        }
        println!("floor: {modelled} >= {floor}");
    }
    Ok(())
}
