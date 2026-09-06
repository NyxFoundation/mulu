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
    let spec = root().join("examples/limits/limits.spec.json");
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
    let spec = root().join("examples/access/vault.spec.json");
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
    let spec = root().join("examples/limits/limits.spec.json");
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
    let spec = root().join("examples/guards/gate.spec.json");
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
