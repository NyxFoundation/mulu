//! `mulu analyze` end to end: solc, ProgramIR, predicate abstraction, the
//! Lean worker and the kernel re-check, on the real Limits contract.
//! Skipped when solc is not installed.

use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

fn ready() -> bool {
    let solc = std::env::var("MULU_SOLC").unwrap_or_else(|_| "solc".into());
    let has_solc = Command::new(solc).arg("--version").output().map(|o| o.status.success()).unwrap_or(false);
    let has_worker = root().join("lean/.lake/build/bin/mulu-worker").exists();
    if !has_solc || !has_worker {
        eprintln!("skipped: needs solc and a built lean/ worker");
    }
    has_solc && has_worker
}

fn mulu() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_mulu"));
    c.env("MULU_LEAN_DIR", root().join("lean"));
    c
}

fn analyze(out_name: &str, extra: &[&str]) -> (i32, PathBuf) {
    let out = std::env::temp_dir().join(format!("mulu-an-{out_name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&out);
    let code = mulu()
        .args(["analyze", root().join("examples/limits/Limits.sol").to_str().unwrap()])
        .args(["--contract", "Limits", "--out", out.to_str().unwrap()])
        .args(extra)
        .status()
        .unwrap()
        .code()
        .unwrap();
    (code, out)
}

fn json(p: &Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

fn diag<'a>(r: &'a serde_json::Value, id: &str) -> &'a serde_json::Value {
    r["diagnostics"].as_array().unwrap().iter().find(|d| d["id"] == id).unwrap_or_else(|| panic!("no diagnostic {id}"))
}

#[test]
fn from_solidity_to_certified_findings() {
    if !ready() {
        return;
    }
    let spec = root().join("examples/limits/Limits.spec.json");
    let (code, out) = analyze("spec", &["--spec", spec.to_str().unwrap()]);
    assert_eq!(code, 1, "a confirmed specification violation exits 1");

    let report = json(&out.join("report.json"));

    // B is redundant, proven on the generated model and re-checked by the kernel
    let b = diag(&report, "check-B");
    assert_eq!(b["claim"], "never-fails");
    assert_eq!(b["status"], "proven");
    assert_eq!(b["depends_on"][0], "A");
    assert_eq!(b["evidence"]["kernel_checked"], true);

    // A is not: it rejects arguments above 100
    assert_eq!(diag(&report, "check-A")["claim"], "may-fail");

    // and forceSet reaches the violating state
    let v = diag(&report, "spec-violation");
    assert_eq!(v["status"], "proven");
    let trace = v["message"].as_str().unwrap();
    assert!(trace.contains("call_forceSet#X2"), "{trace}");
    assert!(trace.contains("store_limit"), "{trace}");

    // the manifest carries where all of this came from
    let manifest = json(&out.join("manifest.json"));
    let p = &manifest["provenance"];
    assert_eq!(p["stage"], "analyze");
    assert!(p["solidity"]["compiler"].as_str().unwrap().starts_with("0.8."));
    assert_eq!(p["abstraction"]["environment_profile"], "p1a-abi-single-v1");
    assert_eq!(p["abstraction"]["complete"], true);
    assert_eq!(p["specification"]["properties"][0]["text"], "limit <= 1000");
    assert!(p["scope_note"].as_str().unwrap().contains("P1-04"));

    // every intermediate artifact is kept
    for f in ["program.json", "model.json", "abstraction.json", "spec.json", "core-model.json"] {
        assert!(out.join(f).exists(), "missing {f}");
    }

    // and the whole thing re-checks, certificates and axioms included
    let v = mulu().args(["verify", out.to_str().unwrap()]).status().unwrap();
    assert_eq!(v.code(), Some(0), "verify must accept what analyze produced");
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn without_a_specification_redundancy_still_works() {
    if !ready() {
        return;
    }
    let (code, out) = analyze("nospec", &[]);
    assert_eq!(code, 0, "no specification means nothing can be violated");
    let report = json(&out.join("report.json"));
    assert_eq!(diag(&report, "check-B")["status"], "proven");
    assert_eq!(diag(&report, "safety")["status"], "not-requested");
    let model = json(&out.join("model.json"));
    assert!(model["bad"].as_array().unwrap().is_empty());
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn a_specification_naming_an_unknown_variable_is_refused() {
    if !ready() {
        return;
    }
    let bad = std::env::temp_dir().join(format!("mulu-badspec-{}.json", std::process::id()));
    std::fs::write(
        &bad,
        r#"{"schema_version":1,"properties":[{"id":"p","contract":"Limits",
            "when":"successful-transaction-end",
            "assert":{"op":"ule","left":{"storage":"nosuchvar"},"right":{"uint256":"1"}}}]}"#,
    )
    .unwrap();
    let (code, _) = analyze("badspec", &["--spec", bad.to_str().unwrap()]);
    assert_eq!(code, 3, "an unresolved name is an input error, not a silent true");
    let _ = std::fs::remove_file(&bad);
}

#[test]
fn a_guard_in_an_imported_modifier_is_attributed_to_it() {
    if !ready() {
        return;
    }
    let out = std::env::temp_dir().join(format!("mulu-an-vault-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&out);
    let spec = root().join("examples/access/Vault.spec.json");
    let code = mulu()
        .args(["analyze", root().join("examples/access/Vault.sol").to_str().unwrap()])
        .args(["--contract", "Vault", "--spec", spec.to_str().unwrap()])
        .args(["--out", out.to_str().unwrap()])
        .status()
        .unwrap()
        .code()
        .unwrap();
    assert_eq!(code, 1);

    let ir = json(&out.join("program.json"));
    let checks = ir["checks"].as_array().unwrap();
    let a = checks.iter().find(|c| c["id"] == "A").expect("check A");

    // the AST is what says this guard was written in a modifier, and where
    assert_eq!(a["origin"], "modifier");
    assert_eq!(a["declared_in"], "Bounded");
    assert_eq!(a["written_in"], "capped");
    assert_eq!(a["source"]["file_id"], 0, "Base.sol is the imported file");

    // the span really is the require inside Base.sol
    let base = std::fs::read_to_string(root().join("examples/access/Base.sol")).unwrap();
    let s = a["source"]["byte_start"].as_u64().unwrap() as usize;
    let n = a["source"]["byte_length"].as_u64().unwrap() as usize;
    assert_eq!(&base[s..s + n], r#"require(x <= 100, "cap")"#);

    // B is in the importing file and depends on A across that boundary
    let b = checks.iter().find(|c| c["id"] == "B").unwrap();
    assert_eq!(b["origin"], "require");
    assert_eq!(b["declared_in"], "Vault");
    assert_eq!(b["source"]["file_id"], 1);

    let report = json(&out.join("report.json"));
    assert_eq!(diag(&report, "check-B")["claim"], "never-fails");
    assert_eq!(diag(&report, "check-B")["status"], "proven");
    assert_eq!(diag(&report, "check-B")["depends_on"][0], "A");

    let v = mulu().args(["verify", out.to_str().unwrap()]).status().unwrap();
    assert_eq!(v.code(), Some(0));
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn an_argument_the_abstraction_cannot_follow_is_refused_not_dropped() {
    if !ready() {
        return;
    }
    // The modifier receives a computed value, so the argument regions no
    // longer describe what it tests. Before the walk followed calls at all
    // this produced a model in which the function did nothing.
    let dir = std::env::temp_dir().join(format!("mulu-refuse-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("C.sol"),
        r#"pragma solidity ^0.8.0;
abstract contract B { modifier capped(uint256 y) { require(y <= 100, "cap"); _; } }
contract C is B {
    uint256 public v;
    function f(uint256 x) external capped(x + 1) { v = x; }
}"#,
    )
    .unwrap();
    let out = dir.join("out");
    let code = mulu()
        .args(["analyze", dir.join("C.sol").to_str().unwrap()])
        .args(["--contract", "C", "--out", out.to_str().unwrap()])
        .status()
        .unwrap()
        .code()
        .unwrap();
    assert_eq!(code, 2, "an incomplete abstraction is not a complete analysis");

    let a = json(&out.join("abstraction.json"));
    let unsupported = a["unsupported"].as_array().unwrap();
    assert!(!unsupported.is_empty());
    let joined = unsupported.iter().map(|u| u.as_str().unwrap()).collect::<Vec<_>>().join("\n");
    // The safety net that matters: an instruction whose effects the model
    // cannot express stops the walk instead of being skipped.
    assert!(
        joined.contains("carries effects the model does not represent"),
        "{joined}"
    );
    // and the model that was written out does not claim f is a no-op
    let model = json(&out.join("model.json"));
    let checks = model["checks"].as_array().unwrap();
    let has_store = model["transitions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["event"].as_str().unwrap().starts_with("store_"));
    assert!(
        checks.is_empty() || has_store,
        "a model with guards but no store would be the silent failure this guards against"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn all_three_findings_come_out_of_the_source() {
    if !ready() {
        return;
    }
    // docs/10: the point of the common model is that redundancy, spec
    // violation and overrestriction are answered on it together. Before the
    // reference plant was generated, analyze could only produce two of them.
    let spec = root().join("examples/limits/Limits.spec.json");
    let (code, out) = analyze("three", &["--spec", spec.to_str().unwrap()]);
    assert_eq!(code, 1);
    let report = json(&out.join("report.json"));
    let kinds: Vec<&str> =
        report["diagnostics"].as_array().unwrap().iter().map(|d| d["kind"].as_str().unwrap()).collect();
    for k in ["redundant-check", "spec-violation", "overrestriction", "envelope"] {
        assert!(kinds.contains(&k), "missing {k} in {kinds:?}");
    }

    // The candidate is the region docs/08 §3 names: setLimit(500) is allowed
    // by the specification and rejected by check A.
    let over: Vec<&serde_json::Value> = report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["kind"] == "overrestriction")
        .collect();
    assert_eq!(over.len(), 1, "only the region between the two bounds qualifies");
    let o = over[0];
    assert_eq!(o["check_id"], "A");
    assert_eq!(o["status"], "candidate", "an overrestriction is never proven here");
    assert_eq!(o["scope"], "abstract-model");
    // it is X1 = [101, 1000], which is where 500 lives
    let model = json(&out.join("model.json"));
    let abstraction = json(&out.join("abstraction.json"));
    let region_name = o["detail"]["impl_state"].as_str().unwrap();
    assert!(region_name.contains("X1"), "{region_name}");
    let x1 = &abstraction["argument_regions"][1];
    assert_eq!(x1["name"], "X1");
    assert_eq!(x1["set"][0][0], "101");
    assert_eq!(x1["set"][0][1], "1000");

    // the supervisor's own answer: stop setLimit storing past the bound, and
    // guard forceSet, which has no check at all
    let env = diag(&report, "envelope");
    assert_eq!(env["claim"], "maximal-permissive");
    let disabled: Vec<String> = env["detail"]["disabled"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["event"].as_str().unwrap().to_string())
        .collect();
    assert!(disabled.iter().any(|d| d == "cont_B"), "{disabled:?}");
    assert!(disabled.iter().any(|d| d == "cont_entry_forceSet"), "{disabled:?}");

    // the plant is analysed as its own model, with its own certificate
    let manifest = json(&out.join("manifest.json"));
    let certs: Vec<&str> = manifest["certificates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["model"].as_str().unwrap())
        .collect();
    assert!(certs.contains(&"model_plant"), "{certs:?}");
    assert!(model["control_plant"].is_object());

    let v = mulu().args(["verify", out.to_str().unwrap()]).status().unwrap();
    assert_eq!(v.code(), Some(0));
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn an_author_written_if_revert_guard_is_named_like_a_require() {
    if !ready() {
        return;
    }
    // solc puts `if (..) revert()` on a branch, so only the AST separates a
    // guard the author wrote from one the compiler inserted.
    let out = std::env::temp_dir().join(format!("mulu-an-gate-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&out);
    let spec = root().join("examples/guards/Gate.spec.json");
    let code = mulu()
        .args(["analyze", root().join("examples/guards/Gate.sol").to_str().unwrap()])
        .args(["--contract", "Gate", "--spec", spec.to_str().unwrap()])
        .args(["--out", out.to_str().unwrap()])
        .status()
        .unwrap()
        .code()
        .unwrap();
    assert_eq!(code, 1);

    let ir = json(&out.join("program.json"));
    let a = ir["checks"].as_array().unwrap().iter().find(|c| c["id"] == "A").expect("check A");
    assert_eq!(a["origin"], "inline", "the author wrote it, the compiler did not");
    assert_eq!(a["declared_in"], "Gate");
    assert_eq!(a["written_in"], "setLimit");
    // the compiler's own branch guards keep a generated id
    assert!(ir["checks"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["origin"] == "compiler" && c["id"].as_str().unwrap().starts_with("gen:")));

    // and it behaves like any other guard: a site, and an overrestriction
    let report = json(&out.join("report.json"));
    let over: Vec<&serde_json::Value> = report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["kind"] == "overrestriction")
        .collect();
    assert_eq!(over.len(), 1);
    assert_eq!(over[0]["check_id"], "A");

    let v = mulu().args(["verify", out.to_str().unwrap()]).status().unwrap();
    assert_eq!(v.code(), Some(0));
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn the_counterexample_is_reproduced_on_a_local_evm() {
    if !ready() {
        return;
    }
    // docs/09 §7, P1-03: forceSet(1001) must be reproduced, and with no
    // specification the tool must not assert a hole.
    let spec = root().join("examples/limits/Limits.spec.json");
    let (code, out) = analyze("replay", &["--spec", spec.to_str().unwrap()]);
    assert_eq!(code, 1);
    let report = json(&out.join("report.json"));

    let v = diag(&report, "spec-violation");
    // the model proof and the execution sit side by side; neither replaces
    // the other (docs/09 §5)
    assert_eq!(v["status"], "proven");
    assert_eq!(v["evidence"]["kernel_checked"], true);
    let rep = &v["reproduction"];
    assert_eq!(rep["status"], "reproduced");
    assert_eq!(rep["calls"][0]["signature"], "forceSet(uint256)");
    assert_eq!(rep["calls"][0]["argument"], "1001");
    assert_eq!(rep["reason"], "violated: limit-bound");
    // the EVM really left 1001 in the slot the specification is about
    assert_eq!(rep["run"]["storage"]["0"], "1001");
    assert!(rep["run"]["calls"][0]["success"].as_bool().unwrap());
    assert!(!rep["assumptions"].as_array().unwrap().is_empty());

    // and the record is on disk beside the certificates
    let path = rep["path"].as_str().unwrap();
    assert!(out.join(path).exists(), "missing {path}");

    // the overrestriction is reproduced from the other side: the contract
    // really rejects a call the specification permits
    let o = report
        .as_object()
        .unwrap()
        .get("diagnostics")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["kind"] == "overrestriction")
        .unwrap();
    assert_eq!(o["reproduction"]["status"], "reproduced");
    assert_eq!(o["reproduction"]["calls"][0]["signature"], "setLimit(uint256)");
    assert_eq!(o["reproduction"]["run"]["calls"][0]["success"], false);
    assert_eq!(o["reproduction"]["run"]["calls"][0]["revert_reason"], "cap");

    let v = mulu().args(["verify", out.to_str().unwrap()]).status().unwrap();
    assert_eq!(v.code(), Some(0));
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn without_a_specification_nothing_is_asserted_about_holes() {
    if !ready() {
        return;
    }
    let (code, out) = analyze("noreplay", &[]);
    assert_eq!(code, 0);
    let report = json(&out.join("report.json"));

    // no violation to reproduce, and none claimed
    assert_eq!(diag(&report, "safety")["status"], "not-requested");
    assert!(report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .all(|d| d["kind"] != "spec-violation" || d["status"] == "not-requested"));

    // and overrestriction is a comparison against a specification, so it is
    // not answered either (docs/04 §1)
    let o = diag(&report, "overrestriction");
    assert_eq!(o["status"], "not-requested");
    assert!(o["reproduction"].is_null());

    // redundancy needs no specification and is still answered
    assert_eq!(diag(&report, "check-B")["status"], "proven");
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn a_run_that_was_cut_off_never_reads_as_a_clean_one() {
    if !ready() {
        return;
    }
    // docs/09 §4: unsupported/partial must not be displayed as safe. The
    // limits used to be labels — the worker wrote "partial" on the analyses
    // and then reported `check-B: never-fails / proven` anyway, because the
    // reachability search had its own fuel and finished. A reader skimming
    // the findings could not tell the run had been cut off at all.
    let spec = root().join("examples/limits/Limits.spec.json");
    let (code, out) = analyze("cutoff", &["--spec", spec.to_str().unwrap(), "--max-states", "5"]);
    assert_eq!(code, 2, "a cut-off run is inconclusive, not a pass and not a violation");

    let report = json(&out.join("report.json"));
    let diags = report["diagnostics"].as_array().unwrap();
    assert!(!diags.is_empty(), "silence would read as nothing to report");

    for d in diags {
        let (id, status) = (d["id"].as_str().unwrap(), d["status"].as_str().unwrap());
        assert_eq!(status, "unknown", "{id} claims {status} on a model that was never analysed");
        // and it says why, so the reader can raise the limit
        let msg = d["message"].as_str().unwrap();
        assert!(msg.contains("limit is 5"), "{id} gives no reason: {msg}");
        // and the message never asserts what the status says was not decided
        assert!(!msg.contains("never fails"), "{id} asserts on an undecided run: {msg}");
    }

    // the same three questions are still asked, so nothing silently vanished
    let ids: Vec<&str> = diags.iter().map(|d| d["id"].as_str().unwrap()).collect();
    for want in ["safety", "check-A", "check-B", "envelope"] {
        assert!(ids.contains(&want), "missing {want} in {ids:?}");
    }

    // no certificate was emitted for a search that did not run
    let manifest = json(&out.join("manifest.json"));
    assert!(
        manifest["certificates"].as_array().unwrap().is_empty(),
        "a declined analysis has nothing to certify"
    );

    let statuses = &report["summary"]["analyses"];
    assert!(
        statuses.as_array().unwrap().iter().any(|s| s["status"] == "partial"),
        "the summary must carry the cut-off too: {statuses}"
    );

    // `verify` on this directory is honestly OK: nothing was claimed, so
    // nothing needs a certificate. It has to say that rather than let "OK"
    // read as "this analysis is fine".
    let o = mulu().args(["verify", out.to_str().unwrap()]).output().unwrap();
    assert_eq!(o.status.code(), Some(0));
    let text = String::from_utf8_lossy(&o.stdout);
    assert!(text.contains("decided nothing about"), "{text}");
    assert!(text.contains("0 certificates"), "{text}");
    let _ = std::fs::remove_dir_all(&out);
}

/// Every SARIF result must obey the rule from the standard that makes the
/// format safe to hand to a code-scanning viewer: a result whose `kind` is
/// anything other than `fail` has `level` `none`. That is what stops a
/// candidate or an undecided run from being drawn as an error.
fn sarif_results(out: &Path) -> Vec<serde_json::Value> {
    let doc = json(&out.join("results.sarif"));
    assert_eq!(doc["version"], "2.1.0");
    let run = &doc["runs"][0];
    let rules: Vec<&str> =
        run["tool"]["driver"]["rules"].as_array().unwrap().iter().map(|r| r["id"].as_str().unwrap()).collect();
    let results = run["results"].as_array().unwrap().clone();
    for r in &results {
        let (kind, level) = (r["kind"].as_str().unwrap(), r["level"].as_str().unwrap());
        if kind != "fail" {
            assert_eq!(level, "none", "SARIF forbids a level on a {kind} result: {r}");
        }
        let rule = r["ruleId"].as_str().unwrap();
        assert!(rules.contains(&rule), "{rule} has no rule metadata");
        assert_eq!(rules[r["ruleIndex"].as_u64().unwrap() as usize], rule, "ruleIndex disagrees with ruleId");
        assert!(!r["partialFingerprints"]["muluFinding/v1"].as_str().unwrap().is_empty());
    }
    results
}

#[test]
fn sarif_says_what_was_proven_and_what_was_only_suspected() {
    if !ready() {
        return;
    }
    let spec = root().join("examples/limits/Limits.spec.json");
    let (code, out) = analyze("sarif", &["--spec", spec.to_str().unwrap()]);
    assert_eq!(code, 1);
    let results = sarif_results(&out);

    let by_rule = |id: &str| -> serde_json::Value {
        results.iter().find(|r| r["ruleId"] == id).unwrap_or_else(|| panic!("no {id}")).clone()
    };

    // proven: a real finding, drawn at its severity
    let v = by_rule("spec-violation");
    assert_eq!(v["kind"], "fail");
    assert_eq!(v["properties"]["status"], "proven");

    // candidate: SARIF's word for it is `review`, and it carries no level
    let o = by_rule("overrestriction");
    assert_eq!(o["kind"], "review");
    assert_eq!(o["properties"]["status"], "candidate");
    assert!(o["message"]["text"].as_str().unwrap().contains("candidate"));

    // the envelope is an answer, not a defect
    assert_eq!(by_rule("envelope")["kind"], "informational");

    // the scope the whole tool rests on is in every message and in properties
    for r in &results {
        assert_eq!(r["properties"]["scope"], "abstract-model");
        assert!(
            r["message"]["text"].as_str().unwrap().contains("finite model"),
            "a viewer shows this line and nothing else: {}",
            r["message"]["text"]
        );
    }

    // the location is the line the check is written on, resolvable from the
    // working directory rather than from solc's own source key
    let b = by_rule("redundant-check");
    let loc = &b["locations"][0]["physicalLocation"];
    // Relative to the working directory when the source is under it, an
    // absolute `file:` URI when it is not, as it is when cargo runs this test
    // from the crate directory. Either way it names the file, not solc's key.
    let uri = loc["artifactLocation"]["uri"].as_str().unwrap();
    assert!(uri.ends_with("examples/limits/Limits.sol"), "{uri}");
    let on_disk = PathBuf::from(uri.strip_prefix("file://").unwrap_or(uri));
    let on_disk = if on_disk.is_absolute() { on_disk } else { std::env::current_dir().unwrap().join(on_disk) };
    assert!(on_disk.exists(), "{uri} does not resolve to a file");
    let line = loc["region"]["startLine"].as_u64().unwrap();
    let text = std::fs::read_to_string(&on_disk).unwrap();
    let src_line = text.lines().nth(line as usize - 1).unwrap();
    assert!(src_line.contains("require"), "line {line} is {src_line:?}, not check B");

    // and the artifact it points into carries the digest mulu read
    let artifacts = json(&out.join("results.sarif"))["runs"][0]["artifacts"].clone();
    assert!(artifacts.as_array().unwrap().iter().any(|a| a["location"]["uri"] == uri
        && a["hashes"]["sha-256"].as_str().is_some_and(|h| h.len() == 64)));
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn sarif_from_a_cut_off_run_decides_nothing() {
    if !ready() {
        return;
    }
    let spec = root().join("examples/limits/Limits.spec.json");
    let (code, out) = analyze("sarif-cut", &["--spec", spec.to_str().unwrap(), "--max-states", "5"]);
    assert_eq!(code, 2);
    for r in sarif_results(&out) {
        // `open` is SARIF's "evaluated, and could not decide". Not `pass`,
        // which is what a viewer shows as a clean file.
        assert_eq!(r["kind"], "open", "{} on a run that was cut off: {r}", r["ruleId"]);
        assert_eq!(r["level"], "none");
    }
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn the_same_input_produces_the_same_bytes() {
    if !ready() {
        return;
    }
    // docs/09 §4, the other half of P1-06: two runs of the same version on the
    // same source must be comparable, or a diff of two reports is noise.
    let spec = root().join("examples/limits/Limits.spec.json");
    let (a_code, a) = analyze("det-a", &["--spec", spec.to_str().unwrap()]);
    let (b_code, b) = analyze("det-b", &["--spec", spec.to_str().unwrap()]);
    assert_eq!(a_code, b_code);
    for f in ["model.json", "core-model.json", "abstraction.json", "report.json", "results.sarif"] {
        let (x, y) = (std::fs::read(a.join(f)).unwrap(), std::fs::read(b.join(f)).unwrap());
        assert_eq!(x, y, "{f} differs between two runs of the same input");
    }
    // manifest.json carries a timestamp, so only its analysis-bearing parts
    // are expected to match.
    let (ma, mb) = (json(&a.join("manifest.json")), json(&b.join("manifest.json")));
    for k in ["input", "provenance", "certificates", "semantics", "kernel"] {
        assert_eq!(ma[k], mb[k], "manifest.{k} differs between two runs");
    }
    let _ = std::fs::remove_dir_all(&a);
    let _ = std::fs::remove_dir_all(&b);
}
