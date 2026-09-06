//! P1-04: the layer a claim is made at is a checked property, not a note.
//!
//! docs/04 §7 stacks source, artifact, semantics and model, and says a claim
//! crosses one layer at a time. `lean/Mulu/Semantics/Simulation.lean` states
//! the crossing; this checks that the analyser reports at the layer it has
//! earned and that the record cannot be edited into claiming more.

use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

fn ready() -> bool {
    let solc = std::env::var("MULU_SOLC").unwrap_or_else(|_| "solc".into());
    Command::new(solc).arg("--version").output().map(|o| o.status.success()).unwrap_or(false)
        && root().join("lean/.lake/build/bin/mulu-worker").exists()
}

fn mulu() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_mulu"));
    c.env("MULU_LEAN_DIR", root().join("lean"));
    c
}

fn json(p: &Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

fn analysed(name: &str) -> PathBuf {
    let out = std::env::temp_dir().join(format!("mulu-ob-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&out);
    let spec = root().join("examples/limits/limits.spec.json");
    mulu()
        .args(["analyze", root().join("examples/limits/Limits.sol").to_str().unwrap()])
        .args(["--contract", "Limits", "--spec", spec.to_str().unwrap()])
        .args(["--out", out.to_str().unwrap()])
        .status()
        .unwrap();
    out
}

#[test]
fn every_finding_stays_at_the_model_and_says_why() {
    if !ready() {
        return;
    }
    let out = analysed("scope");
    let ledger = json(&out.join("obligations.json"));
    let report = json(&out.join("report.json"));

    // nothing is discharged, so nothing is promoted
    let obs = ledger["obligations"].as_array().unwrap();
    assert!(!obs.is_empty());
    assert!(obs.iter().all(|o| o["discharged_by"].is_null()), "the tool discharges nothing");
    assert_eq!(ledger["scope"], "abstract-model");

    // the root obligation names what is actually missing
    let root_ob = obs.iter().find(|o| o["id"] == "semantics:yul-not-formalised").unwrap();
    assert_eq!(root_ob["reaches"], "yul-semantics");
    assert!(root_ob["statement"].as_str().unwrap().contains("formal semantics"));

    // the two simulation conditions are named after the Lean fields that
    // would consume them
    for (id, lean) in [
        ("simulation:initial-covered", "Mulu.Semantics.Simulation.initial_covered"),
        ("simulation:step-covered", "Mulu.Semantics.Simulation.step_covered"),
    ] {
        let o = obs.iter().find(|o| o["id"] == id).unwrap_or_else(|| panic!("missing {id}"));
        assert_eq!(o["lean"], lean);
        assert!(!o["raised_by"].as_array().unwrap().is_empty(), "{id} says what raised it");
    }

    // a redundancy claim rests on its own check's obligation as well
    let b = report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["id"] == "check-B")
        .unwrap();
    assert_eq!(b["scope"], "abstract-model");
    let deps: Vec<&str> =
        b["obligations"].as_array().unwrap().iter().map(|x| x.as_str().unwrap()).collect();
    assert!(deps.contains(&"check:fail-step-matched:B"), "{deps:?}");
    assert!(deps.contains(&"simulation:step-covered"));

    // and one about the envelope rests on the plant corresponding too
    let e = report["diagnostics"].as_array().unwrap().iter().find(|d| d["id"] == "envelope").unwrap();
    let deps: Vec<&str> =
        e["obligations"].as_array().unwrap().iter().map(|x| x.as_str().unwrap()).collect();
    assert!(deps.contains(&"plant:policy-corresponds"), "{deps:?}");

    assert_eq!(mulu().args(["verify", out.to_str().unwrap()]).status().unwrap().code(), Some(0));
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn a_scope_cannot_be_raised_by_editing_the_report() {
    if !ready() {
        return;
    }
    let out = analysed("forge-scope");
    let p = out.join("report.json");
    let mut r = json(&p);
    for d in r["diagnostics"].as_array_mut().unwrap() {
        if d["id"] == "check-B" {
            d["scope"] = serde_json::json!("yul-semantics");
        }
    }
    std::fs::write(&p, r.to_string()).unwrap();

    let o = mulu().args(["verify", out.to_str().unwrap()]).output().unwrap();
    assert_eq!(o.status.code(), Some(4));
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains("check-B is reported at"), "{err}");
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn a_discharge_nothing_checked_is_refused() {
    if !ready() {
        return;
    }
    let out = analysed("forge-ledger");
    let p = out.join("obligations.json");
    let mut l = json(&p);
    for o in l["obligations"].as_array_mut().unwrap() {
        o["discharged_by"] = serde_json::json!("trust me");
    }
    std::fs::write(&p, l.to_string()).unwrap();

    let o = mulu().args(["verify", out.to_str().unwrap()]).output().unwrap();
    assert_eq!(o.status.code(), Some(4));
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains("nothing here can check that"), "{err}");
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn a_diagnostic_may_not_name_an_obligation_the_ledger_lacks() {
    if !ready() {
        return;
    }
    let out = analysed("forge-dep");
    let p = out.join("report.json");
    let mut r = json(&p);
    for d in r["diagnostics"].as_array_mut().unwrap() {
        if d["id"] == "check-B" {
            d["obligations"] = serde_json::json!(["something:invented"]);
        }
    }
    std::fs::write(&p, r.to_string()).unwrap();

    let o = mulu().args(["verify", out.to_str().unwrap()]).output().unwrap();
    assert_eq!(o.status.code(), Some(4));
    assert!(String::from_utf8_lossy(&o.stderr).contains("names an obligation the ledger does not"));
    let _ = std::fs::remove_dir_all(&out);
}
