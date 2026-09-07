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
    let spec = root().join("examples/limits/Limits.spec.json");
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

    // The root obligation names what is actually missing. It was "no
    // semantics exists"; one does now, and the contract is rendered into it,
    // so what is missing is that the rendering is the same program.
    let root_ob =
        obs.iter().find(|o| o["id"] == "semantics:rendering-preserves-the-program").unwrap();
    assert_eq!(root_ob["reaches"], "yul-semantics");
    assert_eq!(root_ob["lean"], "MuluSemantics.concrete");
    assert!(root_ob["statement"].as_str().unwrap().contains("same program"));

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
fn a_diagnostic_may_not_name_obligations_of_its_own_invention() {
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
    let err = String::from_utf8_lossy(&o.stderr);
    // the recomputation catches it: a redundancy claim does not rest on this
    assert!(err.contains("something:invented"), "{err}");
    assert!(err.contains("rests on"), "{err}");
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn the_obligation_list_is_recomputed_not_read_off_the_report() {
    if !ready() {
        return;
    }
    // Reading the list off the report would let a shortened one raise the
    // layer as soon as any obligation is discharged. `verify` derives what a
    // finding rests on from what the finding is.
    let out = analysed("forge-list");
    let p = out.join("report.json");
    let mut r = json(&p);
    for d in r["diagnostics"].as_array_mut().unwrap() {
        if d["id"] == "check-B" {
            d["obligations"] = serde_json::json!([]);
        }
    }
    std::fs::write(&p, r.to_string()).unwrap();

    let o = mulu().args(["verify", out.to_str().unwrap()]).output().unwrap();
    assert_eq!(o.status.code(), Some(4));
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains("check-B lists obligations"), "{err}");
    assert!(err.contains("check:fail-step-matched:B"), "{err}");
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn dropping_the_ledger_does_not_skip_the_check() {
    if !ready() {
        return;
    }
    // Skipping when the ledger is absent would let a report be verified at
    // whatever layer it names, by deleting one file.
    let out = analysed("no-ledger");
    std::fs::remove_file(out.join("obligations.json")).unwrap();

    let o = mulu().args(["verify", out.to_str().unwrap()]).output().unwrap();
    assert_eq!(o.status.code(), Some(4));
    assert!(String::from_utf8_lossy(&o.stderr).contains("obligations.json is missing"));
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn an_envelope_claim_must_name_the_plant_obligation() {
    if !ready() {
        return;
    }
    let out = analysed("forge-plant");
    let p = out.join("report.json");
    let mut r = json(&p);
    for d in r["diagnostics"].as_array_mut().unwrap() {
        if d["id"] == "envelope" {
            // drop the obligation that says the plant corresponds at all
            d["obligations"] = serde_json::json!([
                "semantics:rendering-preserves-the-program",
                "semantics:evmyul-matches-the-evm",
                "simulation:initial-covered",
                "simulation:step-covered"
            ]);
        }
    }
    std::fs::write(&p, r.to_string()).unwrap();

    let o = mulu().args(["verify", out.to_str().unwrap()]).output().unwrap();
    assert_eq!(o.status.code(), Some(4));
    assert!(String::from_utf8_lossy(&o.stderr).contains("plant:policy-corresponds"));
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn deleting_both_files_does_not_evade_the_check() {
    if !ready() {
        return;
    }
    // Requiring only the ledger let the pair be deleted together, and the
    // layer check would then pass by having nothing to check.
    let out = analysed("no-pair");
    std::fs::remove_file(out.join("obligations.json")).unwrap();
    std::fs::remove_file(out.join("report.json")).unwrap();

    let o = mulu().args(["verify", out.to_str().unwrap()]).output().unwrap();
    assert_eq!(o.status.code(), Some(4));
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains("report.json is missing"), "{err}");
    assert!(err.contains("obligations.json is missing"), "{err}");
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn the_ledger_kind_is_bound_to_how_the_analysis_was_run() {
    if !ready() {
        return;
    }
    // The kind decides what a finding rests on, so it is not the file's to
    // choose: a solidity analysis carrying a model-only ledger would drop the
    // simulation obligations its findings actually rest on.
    let out = analysed("forge-kind");
    let p = out.join("obligations.json");
    let mut l = json(&p);
    l["kind"] = serde_json::json!("model-only");
    std::fs::write(&p, l.to_string()).unwrap();

    let o = mulu().args(["verify", out.to_str().unwrap()]).output().unwrap();
    assert_eq!(o.status.code(), Some(4));
    assert!(String::from_utf8_lossy(&o.stderr).contains("the ledger says it is a"));
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn a_model_given_directly_says_it_stands_for_nothing() {
    if !ready() {
        return;
    }
    // `analyze-model` gets a ledger too, and it is a different one.
    let out = std::env::temp_dir().join(format!("mulu-ob-modelonly-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&out);
    let code = mulu()
        .args(["analyze-model", root().join("examples/limits/model.json").to_str().unwrap()])
        .args(["--out", out.to_str().unwrap()])
        .status()
        .unwrap()
        .code()
        .unwrap();
    assert!(code <= 1, "the example contains a violation, so 0 or 1");

    let l = json(&out.join("obligations.json"));
    assert_eq!(l["kind"], "model-only");
    assert_eq!(l["obligations"].as_array().unwrap().len(), 1);
    assert_eq!(l["obligations"][0]["id"], "correspondence:no-artifact");
    assert_eq!(l["scope"], "abstract-model");

    // and every finding rests on that one thing, whatever its kind
    let report = json(&out.join("report.json"));
    for d in report["diagnostics"].as_array().unwrap() {
        assert_eq!(d["scope"], "abstract-model");
        assert_eq!(d["obligations"], serde_json::json!(["correspondence:no-artifact"]));
    }
    assert_eq!(mulu().args(["verify", out.to_str().unwrap()]).status().unwrap().code(), Some(0));
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn the_sarif_may_not_disagree_with_the_report() {
    if !ready() {
        return;
    }
    // The SARIF is what a reviewer sees on a pull request. If verify trusted
    // it, turning a proven violation into a pass would take one edit and
    // leave the report it came from untouched.
    let out = analysed("sarif-edited");
    let path = out.join("results.sarif");
    let mut doc = json(&path);
    for r in doc["runs"][0]["results"].as_array_mut().unwrap() {
        if r["ruleId"] == "spec-violation" {
            r["kind"] = serde_json::json!("pass");
            r["level"] = serde_json::json!("none");
        }
    }
    std::fs::write(&path, serde_json::to_string_pretty(&doc).unwrap()).unwrap();

    let o = mulu().args(["verify", out.to_str().unwrap()]).output().unwrap();
    assert_eq!(o.status.code(), Some(4));
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains("spec-violation") && err.contains("\"pass\""), "{err}");
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn a_finding_cannot_be_dropped_from_the_sarif() {
    if !ready() {
        return;
    }
    let out = analysed("sarif-dropped");
    let path = out.join("results.sarif");
    let mut doc = json(&path);
    let results = doc["runs"][0]["results"].as_array_mut().unwrap();
    results.retain(|r| r["ruleId"] != "spec-violation");
    std::fs::write(&path, serde_json::to_string_pretty(&doc).unwrap()).unwrap();

    let o = mulu().args(["verify", out.to_str().unwrap()]).output().unwrap();
    assert_eq!(o.status.code(), Some(4));
    assert!(
        String::from_utf8_lossy(&o.stderr).contains("is in report.json and not in results.sarif"),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn deleting_the_sarif_does_not_skip_the_check() {
    if !ready() {
        return;
    }
    let out = analysed("no-sarif");
    std::fs::remove_file(out.join("results.sarif")).unwrap();

    let o = mulu().args(["verify", out.to_str().unwrap()]).output().unwrap();
    assert_eq!(o.status.code(), Some(4));
    assert!(String::from_utf8_lossy(&o.stderr).contains("results.sarif is missing"));
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn verify_notes_an_analysis_that_did_not_run_whatever_it_is_called() {
    if !ready() {
        return;
    }
    // The note existed so "verify: OK" would not read as "this analysis is
    // fine". It listed the status names partial and unsupported, which is the
    // pattern the exit-code rule was changed away from: a status nobody
    // enumerated went unmentioned.
    let out = analysed("odd-status");
    let path = out.join("report.json");
    let mut report = json(&path);
    report["summary"]["analyses"][0]["status"] = serde_json::json!("error");
    std::fs::write(&path, serde_json::to_string_pretty(&report).unwrap()).unwrap();

    let o = mulu().args(["verify", out.to_str().unwrap(), "--no-kernel"]).output().unwrap();
    let text = String::from_utf8_lossy(&o.stdout);
    assert!(text.contains("decided nothing about"), "{text}");
    assert!(text.contains("(error)"), "{text}");
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn an_assumption_on_solc_is_not_an_item_on_our_list() {
    if !ready() {
        return;
    }
    // Six of the eight are statements about mulu's own abstraction and are
    // ours to prove. Two are correctness properties of a compiler nobody has
    // proved correct. Printing them as one list of open obligations makes the
    // second look like work that is merely pending.
    let out = analysed("bearers");
    let ledger = json(&out.join("obligations.json"));
    let obs = ledger["obligations"].as_array().unwrap();

    let theirs: Vec<&str> = obs
        .iter()
        .filter(|o| o["bearer"] == "solc")
        .map(|o| o["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        theirs,
        vec!["compilation:yul-corresponds-to-source", "compilation:optimised-bytecode"]
    );
    // and each says what would discharge it, which is not "more work here"
    for o in obs.iter().filter(|o| o["bearer"] == "solc") {
        let need = o["would_need"].as_str().unwrap();
        assert!(need.contains("None exists"), "{need}");
    }
    // Adopting a semantics moved an assumption rather than removing one, and
    // the ledger has to show where it went: the semantics agreeing with the
    // EVM is now its own item, and it is not ours.
    let adopted = obs.iter().find(|o| o["id"] == "semantics:evmyul-matches-the-evm").unwrap();
    assert_eq!(adopted["bearer"], "evmyul");
    assert_eq!(adopted["reaches"], "yul-semantics");
    assert!(adopted["would_need"].as_str().unwrap().contains("None exists"));
    assert!(adopted["discharged_by"].is_null(), "having a semantics is not proving it right");

    // and what remains ours is naming this contract in it
    let root = obs.iter().find(|o| o["id"] == "semantics:rendering-preserves-the-program").unwrap();
    assert_eq!(root["bearer"], "mulu");
    assert!(root["discharged_by"].is_null());

    // every claim still sits at the model, because everything is still open
    let report = json(&out.join("report.json"));
    for d in report["diagnostics"].as_array().unwrap() {
        assert_eq!(d["scope"], "abstract-model");
    }
    assert_eq!(mulu().args(["verify", out.to_str().unwrap()]).status().unwrap().code(), Some(0));
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn assuming_a_compiler_correct_is_recorded_not_erased() {
    if !ready() {
        return;
    }
    // The project decided to take solc as correct. That settles the two
    // obligations nobody could ever discharge, and it is not a proof, so it
    // has to stay visible: in the ledger as `assumed_by`, and on every
    // finding that is carried past it.
    let out = analysed("assumed");
    let obs = json(&out.join("obligations.json"))["obligations"].as_array().unwrap().clone();

    let assumed: Vec<&str> = obs
        .iter()
        .filter(|o| o["assumed_by"].is_string())
        .map(|o| o["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        assumed,
        vec![
            "semantics:evmyul-matches-the-evm",
            "compilation:yul-corresponds-to-source",
            "compilation:optimised-bytecode"
        ]
    );
    for o in obs.iter().filter(|o| o["assumed_by"].is_string()) {
        assert_ne!(o["bearer"], "mulu", "only work this project did not do can be assumed");
        assert!(o["discharged_by"].is_null(), "assumed is not discharged");
        assert!(o["assumed_by"].as_str().unwrap().contains("not a proof"));
    }
    // and nothing of ours was quietly settled along with them
    assert!(obs
        .iter()
        .filter(|o| o["bearer"] == "mulu")
        .all(|o| o["assumed_by"].is_null() && o["discharged_by"].is_null()));

    assert_eq!(mulu().args(["verify", out.to_str().unwrap()]).status().unwrap().code(), Some(0));
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn an_obligation_of_ours_cannot_be_assumed_away() {
    if !ready() {
        return;
    }
    // Assuming a third party's compiler is a decision. Assuming the proofs we
    // owe are done is not a decision about anything.
    let out = analysed("assume-ours");
    let path = out.join("obligations.json");
    let mut l = json(&path);
    for o in l["obligations"].as_array_mut().unwrap() {
        if o["id"] == "simulation:step-covered" {
            o["assumed_by"] = serde_json::json!(
                "assumed: solc compiles Solidity as its documentation says. A project decision, \
                 not a proof; no proof of solc exists."
            );
        }
    }
    std::fs::write(&path, serde_json::to_string_pretty(&l).unwrap()).unwrap();

    let o = mulu().args(["verify", out.to_str().unwrap()]).output().unwrap();
    assert_eq!(o.status.code(), Some(4));
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains("ours to prove and cannot be assumed"), "{err}");
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn a_ledger_cannot_invent_an_assumption_of_its_own() {
    if !ready() {
        return;
    }
    // The decision is the project's, recorded once. A ledger that writes its
    // own sentence would be settling an obligation by asserting it.
    let out = analysed("assume-invented");
    let path = out.join("obligations.json");
    let mut l = json(&path);
    for o in l["obligations"].as_array_mut().unwrap() {
        if o["id"] == "semantics:evmyul-matches-the-evm" {
            o["assumed_by"] = serde_json::json!("assumed: it is probably fine");
        }
    }
    std::fs::write(&path, serde_json::to_string_pretty(&l).unwrap()).unwrap();

    let o = mulu().args(["verify", out.to_str().unwrap()]).output().unwrap();
    assert_eq!(o.status.code(), Some(4));
    assert!(
        String::from_utf8_lossy(&o.stderr).contains("not the decision this project recorded"),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    let _ = std::fs::remove_dir_all(&out);
}
