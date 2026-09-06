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
    assert!(trace.contains("call_forceSet_X2"), "{trace}");
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
