//! End-to-end: fixtures through the real worker + kernel. Skipped (with a
//! message) when the Lean side is not built.

use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

fn worker_available() -> bool {
    root().join("lean/.lake/build/bin/mulu-worker").exists()
}

fn mulu() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_mulu"));
    c.env("MULU_LEAN_DIR", root().join("lean"));
    c
}

fn analyze(fixture: &str, extra: &[&str]) -> (i32, serde_json::Value, PathBuf) {
    let out = std::env::temp_dir().join(format!("mulu-e2e-{}-{}-{}", fixture.replace('/', "_"), extra.len(), std::process::id()));
    let _ = std::fs::remove_dir_all(&out);
    let status = mulu()
        .args(["analyze-model", root().join(fixture).to_str().unwrap(), "--out", out.to_str().unwrap()])
        .args(extra)
        .status()
        .unwrap();
    let report: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(out.join("report.json")).unwrap()).unwrap();
    (status.code().unwrap(), report, out)
}

fn diag<'a>(report: &'a serde_json::Value, id: &str) -> &'a serde_json::Value {
    report["diagnostics"].as_array().unwrap().iter().find(|d| d["id"] == id).unwrap_or_else(|| panic!("no diagnostic {id}"))
}

fn verify(dir: &Path) -> i32 {
    mulu().args(["verify", dir.to_str().unwrap()]).status().unwrap().code().unwrap()
}

#[test]
fn fixture_controllable() {
    if !worker_available() {
        eprintln!("skipped: build lean/ first");
        return;
    }
    let (code, r, out) = analyze("examples/fixtures/product-controllable.json", &[]);
    assert_eq!(code, 1, "proven violation → exit 1");
    let env = diag(&r, "envelope");
    assert_eq!(env["status"], "proven");
    assert_eq!(env["detail"]["winning"], serde_json::json!(["q0"]));
    assert_eq!(env["detail"]["disabled"][0]["from"], "q0");
    assert_eq!(env["evidence"]["kernel_checked"], true);
    assert_eq!(verify(&out), 0);
}

#[test]
fn fixture_uncontrollable_is_unrealizable() {
    if !worker_available() {
        return;
    }
    let (_, r, out) = analyze("examples/fixtures/product-uncontrollable.json", &[]);
    assert_eq!(diag(&r, "envelope")["claim"], "unrealizable");
    assert_eq!(verify(&out), 0);
}

#[test]
fn fixture_blocking_cycle_modes() {
    if !worker_available() {
        return;
    }
    let (code, r, _) = analyze("examples/fixtures/blocking-cycle.json", &[]);
    assert_eq!(code, 0);
    assert_eq!(diag(&r, "envelope")["detail"]["winning"], serde_json::json!(["q0"]));
    let (_, r, _) = analyze("examples/fixtures/blocking-cycle.json", &["--objective", "safety"]);
    assert_eq!(diag(&r, "envelope")["detail"]["winning"], serde_json::json!(["q0", "q1", "q2"]));
}

#[test]
fn limits_three_findings() {
    if !worker_available() {
        return;
    }
    let (code, r, out) = analyze("examples/limits/model.json", &[]);
    assert_eq!(code, 1);
    assert_eq!(diag(&r, "check-B")["claim"], "never-fails");
    assert_eq!(diag(&r, "check-B")["status"], "proven");
    assert_eq!(diag(&r, "check-A")["claim"], "may-fail");
    assert_eq!(diag(&r, "spec-violation")["status"], "proven");
    assert_eq!(diag(&r, "overrestriction-A-sL_A_X1")["status"], "candidate");
    assert!(r["diagnostics"].as_array().unwrap().iter().all(|d| d["id"] != "overrestriction-A-sL_A_X2"));
    assert_eq!(verify(&out), 0);
    // tamper with a certificate → verify must fail
    let p = out.join("certificates/redundancy_B.json");
    let mut c: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
    c["states"] = serde_json::json!([0]);
    std::fs::write(&p, c.to_string()).unwrap();
    assert_eq!(verify(&out), 4);
}
