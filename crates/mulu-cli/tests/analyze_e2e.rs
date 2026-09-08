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
fn a_loop_is_over_approximated_rather_than_unrolled_or_refused() {
    if !ready() {
        return;
    }
    // Unrolling a loop a finite number of times and using the result for an
    // unbounded claim is what docs/09 forbids, and refusing it threw away
    // every contract that copies an array. The loop's *effect* is
    // over-approximated instead: what it writes is unknown afterwards, what
    // it defines is forgotten, and if it can revert then so can the function.
    let dir = std::env::temp_dir().join(format!("mulu-loop-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("C.sol"),
        r#"pragma solidity ^0.8.0;
contract C {
    uint256 public total;
    uint256 public untouched;
    function sum(uint256 n) external {
        require(n > 0);
        uint256 acc = 0;
        for (uint256 i = 0; i < n; i++) { acc += i; }
        total = acc;
    }
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
    assert_eq!(code, 0, "a loop is a coarser model, not an incomplete one");

    let a = json(&out.join("abstraction.json"));
    assert!(
        a["unsupported"].as_array().unwrap().is_empty(),
        "nothing should be unmodelled: {:?}",
        a["unsupported"]
    );
    let assumptions: Vec<String> = a["assumptions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap().to_string())
        .collect();
    assert!(
        assumptions.iter().any(|x| x.starts_with("loop-over-approximated:")),
        "the coarseness has to be on the record: {assumptions:?}"
    );

    // The guard before the loop is still decided, because the loop cannot
    // have changed the argument. That is the point of over-approximating
    // rather than giving up on the function.
    let model = json(&out.join("model.json"));
    let events: Vec<String> = model["transitions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["event"].as_str().unwrap().to_string())
        .collect();
    assert!(events.iter().any(|e| e == "A_pass"), "{events:?}");
    assert!(events.iter().any(|e| e == "A_fail"), "{events:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_guard_the_regions_do_not_decide_splits_the_walk_and_says_so() {
    if !ready() {
        return;
    }
    // `balances[msg.sender]` is a mapping cell, and no partition of the
    // argument space decides `amount <= cell`: the guard is about the
    // relation between the two, and the model has one of them. Refusing threw
    // away the contract. The walk now goes both ways, which keeps every path
    // the program has and adds some it may not, so a check reached only on a
    // split path is reported as one that *can* fail, never as one that
    // cannot. The report has to say the model is coarse there.
    let dir = std::env::temp_dir().join(format!("mulu-split-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("C.sol"),
        r#"pragma solidity ^0.8.0;
contract C {
    mapping(address => uint256) balances;
    uint256 public total;
    function withdraw(uint256 amount) external {
        require(amount > 0);
        require(amount <= balances[msg.sender]);
        balances[msg.sender] -= amount;
        total -= amount;
    }
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
    assert_eq!(code, 0, "a split is a coarser model, not an incomplete one");

    let a = json(&out.join("abstraction.json"));
    assert!(
        a["unsupported"].as_array().unwrap().is_empty(),
        "nothing should be unmodelled: {:?}",
        a["unsupported"]
    );
    let assumptions: Vec<String> = a["assumptions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap().to_string())
        .collect();
    assert!(
        assumptions.iter().any(|x| x.starts_with("split-on-an-unknown-value:")),
        "the coarseness has to be on the record: {assumptions:?}"
    );
    assert!(
        assumptions.iter().any(|x| x.starts_with("cells-do-not-alias:")),
        "writing a mapping cell rests on keccak not colliding: {assumptions:?}"
    );

    // Both sides are in the model: the guard can fail, and the walk reaches
    // past it. A model where it only ever passed would be the silent hole.
    let model = json(&out.join("model.json"));
    let events: Vec<String> = model["transitions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["event"].as_str().unwrap().to_string())
        .collect();
    assert!(events.iter().any(|e| e == "B_fail"), "{events:?}");
    assert!(events.iter().any(|e| e == "B_pass"), "{events:?}");
    assert!(events.iter().any(|e| e.starts_with("store_")), "{events:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_argument_the_abstraction_cannot_follow_is_refused_not_dropped() {
    if !ready() {
        return;
    }
    // `delegatecall` runs another contract's code with this contract's
    // storage, so no assumption about the callee recovers what it does here.
    // The entrypoint is refused and named, and the whole chain says so: the
    // process exit code, the report, the SARIF invocation, the ledger.
    let dir = std::env::temp_dir().join(format!("mulu-refuse-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("C.sol"),
        r#"pragma solidity ^0.8.0;
contract C {
    uint256 public v;
    function f(uint256 x) external {
        require(x <= 100, "cap");
        (bool ok, ) = address(this).delegatecall(abi.encodeWithSignature("g(uint256)", x));
        require(ok);
        v = x;
    }
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
    // The property, not the sentence. Which net catches this has moved
    // three times as the abstraction learned to follow more, and each move
    // was an improvement; what must not change is that the entrypoint is
    // refused and named, rather than modelled as a no-op.
    assert!(joined.contains("f(uint256)"), "the refusal must name the entrypoint: {joined}");
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

    // The report and the SARIF have to carry it too. A reader of either sees
    // findings proven on a model that is not the whole contract, and the run
    // is the only place that can say so.
    let report = json(&out.join("report.json"));
    assert_eq!(report["summary"]["exit_code"], 2, "the report must agree with the process");
    assert!(!report["summary"]["unsupported"].as_array().unwrap().is_empty());
    assert!(report["summary"]["analyses"]
        .as_array()
        .unwrap()
        .iter()
        .any(|a| a["analysis"] == "abstraction" && a["status"] == "unsupported"));

    let inv = json(&out.join("results.sarif"))["runs"][0]["invocations"][0].clone();
    assert_eq!(inv["exitCode"], 2);
    let notes: Vec<String> = inv["toolExecutionNotifications"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["message"]["text"].as_str().unwrap().to_string())
        .collect();
    assert!(notes.iter().any(|n| n.contains("unmodelled")), "{notes:?}");

    // and the ledger records it against the condition it defeats, rather than
    // leaving step-covered looking merely unproven
    let ledger = json(&out.join("obligations.json"));
    let step = ledger["obligations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["id"] == "simulation:step-covered")
        .unwrap()
        .clone();
    let raised: Vec<&str> =
        step["raised_by"].as_array().unwrap().iter().map(|r| r.as_str().unwrap()).collect();
    assert!(raised.iter().any(|r| r.starts_with("not modelled:")), "{raised:?}");
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

    // what the EVM said travels with the finding, since the SARIF message is
    // the only line a reviewer sees
    assert!(
        v["message"]["text"].as_str().unwrap().contains("replayed on a local EVM and reproduced"),
        "{}",
        v["message"]["text"]
    );

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

#[test]
fn a_worker_that_was_stopped_never_exits_zero() {
    if !ready() {
        return;
    }
    // The parent's timeout used to return a response with no `analyses`, so
    // every analysis defaulted to the status "error", and the exit code rule
    // listed only partial and unsupported. A timed-out run therefore exited
    // 0: green in CI, having decided nothing.
    let spec = root().join("examples/limits/Limits.spec.json");
    let (code, out) = analyze("timeout", &["--spec", spec.to_str().unwrap(), "--timeout-ms", "1"]);
    assert_eq!(code, 2, "a run the worker never finished is not a pass");

    let report = json(&out.join("report.json"));
    for d in report["diagnostics"].as_array().unwrap() {
        assert_eq!(d["status"], "unknown", "{}", d["id"]);
        assert!(
            d["message"].as_str().unwrap().contains("stopped after"),
            "{} does not say the worker was stopped: {}",
            d["id"],
            d["message"]
        );
    }
    assert!(report["summary"]["analyses"].as_array().unwrap().iter().all(|a| a["status"] == "partial"));
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn the_edge_count_is_a_limit_the_caller_can_set() {
    if !ready() {
        return;
    }
    // The CLI sent only max_states, so the worker always used its own default
    // edge limit and --max-states was the only budget a caller could express.
    let spec = root().join("examples/limits/Limits.spec.json");
    let (code, out) = analyze("edges", &["--spec", spec.to_str().unwrap(), "--max-edges", "3"]);
    assert_eq!(code, 2);
    let report = json(&out.join("report.json"));
    let msg = report["diagnostics"][0]["message"].as_str().unwrap().to_string();
    assert!(msg.contains("edges and the limit is 3"), "{msg}");
    let _ = std::fs::remove_dir_all(&out);
}

/// Write a Foundry/Hardhat-shaped project: two sources behind a remapping,
/// and the build-info a real build would have left.
fn project(name: &str, settings: serde_json::Value, solc_version: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mulu-proj-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::create_dir_all(dir.join("lib/oz/token")).unwrap();
    std::fs::create_dir_all(dir.join("out/build-info")).unwrap();
    let limits = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;
import "@oz/token/Cap.sol";
contract Limits is Cap {
    uint256 public limit;
    function setLimit(uint256 x) external capped(x) {
        require(x <= 1000, "hard");
        limit = x;
    }
    function forceSet(uint256 x) external { limit = x; }
}
"#;
    let cap = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;
abstract contract Cap {
    modifier capped(uint256 x) {
        require(x <= 100, "cap");
        _;
    }
}
"#;
    std::fs::write(dir.join("src/Limits.sol"), limits).unwrap();
    std::fs::write(dir.join("lib/oz/token/Cap.sol"), cap).unwrap();
    let bi = serde_json::json!({
        "_format": "hh-sol-build-info-1",
        "id": "test",
        "solcVersion": solc_version,
        "input": {
            "language": "Solidity",
            "sources": {
                "src/Limits.sol": {"content": limits},
                "lib/oz/token/Cap.sol": {"content": cap},
            },
            "settings": settings,
        },
        "output": {},
    });
    std::fs::write(dir.join("out/build-info/test.json"), serde_json::to_string_pretty(&bi).unwrap())
        .unwrap();
    std::fs::copy(root().join("examples/limits/Limits.spec.json"), dir.join("spec.json")).unwrap();
    dir
}

fn analyze_project(dir: &Path) -> (i32, PathBuf, String) {
    let out = dir.join("analysis");
    let o = mulu()
        .args(["analyze", "--project", dir.to_str().unwrap()])
        .args(["--contract", "Limits", "--spec", dir.join("spec.json").to_str().unwrap()])
        .args(["--out", out.to_str().unwrap()])
        .output()
        .unwrap();
    (o.status.code().unwrap(), out, String::from_utf8_lossy(&o.stdout).into_owned())
}

fn local_solc_version() -> String {
    let solc = std::env::var("MULU_SOLC").unwrap_or_else(|_| "solc".into());
    let out = Command::new(solc).arg("--version").output().unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|l| l.trim().strip_prefix("Version:"))
        .map(|v| v.trim().to_string())
        .unwrap_or_default()
}

#[test]
fn a_project_build_supplies_the_sources_and_the_settings() {
    if !ready() {
        return;
    }
    // docs/08 §4: read the build-info rather than write an evaluator for
    // foundry.toml or hardhat.config.js. The remapping is the point: without
    // settings.remappings the import does not resolve, and the guard in the
    // imported modifier would not be found at all.
    let settings = serde_json::json!({
        "optimizer": {"enabled": false},
        "evmVersion": "paris",
        "remappings": ["@oz/=lib/oz/"],
    });
    let dir = project("clean", settings, &local_solc_version());
    let (code, out, stdout) = analyze_project(&dir);
    assert_eq!(code, 1, "the example contains a violation");
    assert!(stdout.contains("this is the project's own build"), "{stdout}");

    // the project's own EVM version was used, not mulu's default
    let manifest = json(&out.join("manifest.json"));
    assert_eq!(manifest["provenance"]["solidity"]["settings"]["evmVersion"], "paris");
    assert_eq!(
        manifest["provenance"]["solidity"]["settings"]["remappings"][0],
        "@oz/=lib/oz/"
    );

    // and the guard written in the imported modifier is attributed to it
    let sarif = json(&out.join("results.sarif"));
    let a = sarif["runs"][0]["results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["properties"]["id"] == "check-A")
        .unwrap()
        .clone();
    let uri = a["locations"][0]["physicalLocation"]["artifactLocation"]["uri"].as_str().unwrap();
    assert!(uri.ends_with("lib/oz/token/Cap.sol"), "{uri}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_build_this_is_not_says_so() {
    if !ready() {
        return;
    }
    // Analysing a project's sources is not analysing the project's build. The
    // optimizer, a different solc and a file edited since are each a reason
    // the deployed bytecode is not this artifact, so each goes on the
    // obligation that says so rather than into a warning that scrolls past.
    let settings = serde_json::json!({
        "optimizer": {"enabled": true, "runs": 200},
        "viaIR": true,
        "remappings": ["@oz/=lib/oz/"],
    });
    let dir = project("drifted", settings, "0.4.11");
    // edit one source after the build
    let cap = dir.join("lib/oz/token/Cap.sol");
    let text = std::fs::read_to_string(&cap).unwrap();
    std::fs::write(&cap, text.replace("// SPDX", "// edited\n// SPDX")).unwrap();

    let (code, out, stdout) = analyze_project(&dir);
    assert_eq!(code, 1);
    assert!(stdout.contains("not the build the project ships"), "{stdout}");

    let raised: Vec<String> = json(&out.join("obligations.json"))["obligations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["id"] == "compilation:optimised-bytecode")
        .unwrap()["raised_by"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r.as_str().unwrap().to_string())
        .collect();
    let joined = raised.join("\n");
    assert!(joined.contains("solc 0.4.11"), "{joined}");
    assert!(joined.contains("optimizer on"), "{joined}");
    assert!(joined.contains("IR pipeline"), "{joined}");
    assert!(joined.contains("Cap.sol has been edited"), "{joined}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_project_with_no_build_is_told_to_build_it() {
    if !ready() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("mulu-nobuild-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let o = mulu()
        .args(["analyze", "--project", dir.to_str().unwrap()])
        .args(["--out", dir.join("out").to_str().unwrap()])
        .output()
        .unwrap();
    assert_ne!(o.status.code(), Some(0));
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains("build the project first"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_contract_is_written_into_the_semantics_it_will_be_proved_against() {
    if !ready() {
        return;
    }
    // `Mulu.Semantics.Simulation` states the correspondence over an arbitrary
    // concrete system. Stating it about *this* contract needs the contract in
    // the semantics, so the analysis writes it there, from the same `ir` the
    // model was built from.
    let spec = root().join("examples/limits/Limits.spec.json");
    let (_code, out) = analyze("semantics", &["--spec", spec.to_str().unwrap()]);
    let module = out.join("semantics/Limits.lean");
    let text = std::fs::read_to_string(&module).expect("semantics module");

    assert!(text.contains("import EvmYul.Yul.Interpreter"), "{text:.200}");
    assert!(text.contains("def contract : YulContract"), "{text:.200}");
    assert!(text.contains("dispatcher :="));
    // the guards the analysis reports on are in it
    assert!(text.contains("external_fun_setLimit"), "the entrypoints must be there");
    // `memoryguard` is a hint to solc's optimizer, and EvmYul has no such call
    assert!(!text.contains("memoryguard("), "memoryguard must be unwrapped, not passed through");
    // a require message is a word, not a quoted string: the notation has no
    // string literal, and Yul says a string literal *is* that word
    assert!(!text.contains("\"cap\""), "a string literal must become the word it denotes");

    // and what the rendering did that a transcription would not is recorded
    // against the obligation it raises, not left in a warning
    let raised: Vec<String> = json(&out.join("obligations.json"))["obligations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["id"] == "semantics:rendering-preserves-the-program")
        .expect("the rendering obligation")["raised_by"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r.as_str().unwrap().to_string())
        .collect();
    let joined = raised.join("\n");
    assert!(joined.contains("memoryguard"), "{joined}");
    assert!(joined.contains("32-byte word"), "{joined}");
    // this contract has no loop, and solc writes `default {}` itself, so it
    // must not be made to carry either assumption
    assert!(!joined.contains("for loop"), "{joined}");
    assert!(!joined.contains("no default"), "{joined}");

    // and solc's dispatcher has an empty default, so the contract does not
    // trigger the defect that would make it unreadable in the semantics
    let evmyul: Vec<String> = json(&out.join("obligations.json"))["obligations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["id"] == "semantics:evmyul-matches-the-evm")
        .unwrap()["raised_by"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r.as_str().unwrap().to_string())
        .collect();
    assert!(
        !evmyul.iter().any(|l| l.contains("non-empty `default`")),
        "{evmyul:?}"
    );
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn every_example_renders_into_the_semantics() {
    if !ready() {
        return;
    }
    // Refusing is the design: a half-rendered contract would give a semantics
    // for a program that is not the one analysed. So every example has to go
    // through, or the renderer is not usable on real solc output.
    for (path, name) in [
        ("examples/limits/Limits.sol", "Limits"),
        ("examples/access/Vault.sol", "Vault"),
        ("examples/typed/Meter.sol", "Meter"),
        ("examples/overload/Over.sol", "Over"),
        ("examples/guards/Gate.sol", "Gate"),
    ] {
        let out = std::env::temp_dir().join(format!("mulu-yl-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&out);
        let o = mulu()
            .args(["yul-lean", root().join(path).to_str().unwrap()])
            .args(["--contract", name, "--out", out.to_str().unwrap()])
            .output()
            .unwrap();
        assert_eq!(o.status.code(), Some(0), "{name}: {}", String::from_utf8_lossy(&o.stderr));
        let text = std::fs::read_to_string(out.join(format!("{name}.lean"))).unwrap();
        assert!(text.contains("def contract : YulContract"), "{name}");
        assert!(!text.contains("memoryguard("), "{name}");
        let _ = std::fs::remove_dir_all(&out);
    }
}

#[test]
fn a_contract_the_semantics_mis_executes_is_not_compared() {
    if !ready() {
        return;
    }
    // EvmYul runs a switch's default branch even when a case matches, and
    // propagates its error. That is a defect in the semantics, not a rewrite
    // mulu could make: the program is ordinary Yul. So a contract containing
    // one is refused rather than measured, because agreement or disagreement
    // on a path the semantics gets wrong says nothing about the rendering.
    let dir = std::env::temp_dir().join(format!("mulu-sw-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("Sw.sol"),
        r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;
contract Sw {
    uint256 public v;
    function pick(uint256 x) external {
        assembly {
            switch x
            case 1 { sstore(0, 7) }
            default { revert(0, 0) }
        }
    }
}"#,
    )
    .unwrap();

    let o = mulu()
        .args(["yul-lean", dir.join("Sw.sol").to_str().unwrap()])
        .args(["--contract", "Sw", "--out", dir.join("out").to_str().unwrap()])
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains("cannot be read in the adopted semantics"), "{err}");
    assert!(err.contains("non-empty `default`"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_semantics_is_assumed_right_except_where_it_was_measured_wrong() {
    if !ready() {
        return;
    }
    // Assuming an unproved thing is a decision. Assuming a thing you have a
    // counterexample to is not, so the decision is withdrawn per contract:
    // one that reaches a place EvmYul gets wrong leaves the obligation open.
    let spec = root().join("examples/limits/Limits.spec.json");
    let (_, ok_out) = analyze("assumed-ok", &["--spec", spec.to_str().unwrap()]);
    let assumed = |dir: &Path| -> serde_json::Value {
        json(&dir.join("obligations.json"))["obligations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|o| o["id"] == "semantics:evmyul-matches-the-evm")
            .unwrap()["assumed_by"]
            .clone()
    };
    assert!(assumed(&ok_out).is_string(), "nothing here reaches a place it gets wrong");

    // the same analysis of a contract with a non-empty `default`, which does
    let dir = std::env::temp_dir().join(format!("mulu-hz-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("Sw.sol"),
        r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;
contract Sw {
    uint256 public v;
    function pick(uint256 x) external {
        assembly {
            switch x
            case 1 { sstore(0, 7) }
            default { revert(0, 0) }
        }
    }
}"#,
    )
    .unwrap();
    let out = dir.join("out");
    mulu()
        .args(["analyze", dir.join("Sw.sol").to_str().unwrap()])
        .args(["--contract", "Sw", "--out", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(assumed(&out).is_null(), "a measured counterexample is not settled by a decision");
    let _ = std::fs::remove_dir_all(&ok_out);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_guard_inside_a_call_made_for_its_value_is_reached_and_refines() {
    if !ready() {
        return;
    }
    // A `require` inside a function called for its value is a revert path the
    // model must have, and a guard whose boundary the regions must respect.
    //
    // Both halves were missing and were fixed separately, which is why this
    // pins them together. The walk skipped a definition whose value can
    // revert, dropping the path; when it began entering them, the reachable
    // set still followed only calls made as statements, so the guard never
    // refined the partition and the walk then found it undecided in a region
    // that was coarse only for that reason.
    let dir = std::env::temp_dir().join(format!("mulu-valuecall-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("C.sol"),
        r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;
contract C {
    uint256 public v;
    function f(uint256 x) public { v = g(x); }
    function g(uint256 x) internal pure returns (uint256) {
        require(x <= 10, "g");
        return x;
    }
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
    assert_eq!(code, 0, "nothing here is unsupported and there is no specification");

    let a = json(&out.join("abstraction.json"));
    assert!(a["unsupported"].as_array().unwrap().is_empty(), "{}", a["unsupported"]);

    // the guard's boundary is in the partition
    let sets: Vec<String> = a["argument_regions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["set"].to_string())
        .collect();
    assert!(sets.iter().any(|s| s.contains("10")), "{sets:?}");

    // and the model has the check with both outcomes reachable, the failing
    // one being the revert path the walk used to skip
    let model = json(&out.join("model.json"));
    let checks = model["checks"].as_array().unwrap();
    assert_eq!(checks.len(), 1, "one guard, one check");
    let (pass, fail) = (
        checks[0]["pass_event"].as_str().unwrap(),
        checks[0]["fail_event"].as_str().unwrap(),
    );
    let has = |ev: &str| {
        model["transitions"].as_array().unwrap().iter().any(|t| t["event"] == ev)
    };
    assert!(has(pass), "the passing branch");
    assert!(has(fail), "the reverting branch, which is the point");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn reentry_is_the_difference_between_safe_and_not() {
    if !ready() {
        return;
    }
    // `add` checks the bound before it leaves the contract and restores it
    // after, so each transaction on its own keeps `total <= 100`. Two of them
    // interleaved do not: both read `total` before either writes it, both
    // pass the guard, and both add.
    //
    // The contract cannot forbid the callee from calling back, which is why
    // `reenter` is an uncontrollable event of the plant. The whole difference
    // between the two runs below is whether the model has it.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/reentrancy");
    let sol = root.join("Pot.sol");
    let spec = root.join("Pot.spec.json");
    let verdict = |sol: &std::path::Path, reentrancy: bool| -> String {
        let out = std::env::temp_dir().join(format!(
            "mulu-reentry-{}-{}-{reentrancy}",
            std::process::id(),
            sol.file_stem().unwrap().to_string_lossy()
        ));
        let _ = std::fs::remove_dir_all(&out);
        let mut cmd = mulu();
        cmd.args(["analyze", sol.to_str().unwrap()])
            .args(["--spec", spec.to_str().unwrap()])
            .args(["--out", out.to_str().unwrap()]);
        if reentrancy {
            cmd.arg("--reentrancy");
        }
        assert!(cmd.status().unwrap().code().is_some());
        let report = json(&out.join("report.json"));
        let d = report["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["id"] == "model-safe" || d["id"] == "spec-violation")
            .cloned()
            .expect("a safety diagnostic");
        assert_eq!(d["status"], "proven", "either way the kernel checks it");
        assert!(report["summary"]["kernel_checked"].as_bool().unwrap_or(false));
        let s = d["claim"].as_str().unwrap().to_string();
        let _ = std::fs::remove_dir_all(&out);
        s
    };
    // Assuming the callee away, the bound holds, and the kernel checks the
    // invariant that says so.
    assert_eq!(verdict(&sol, false), "bad-unreachable", "no reentry: the bound holds");
    // Modelling it, the violation is reachable, and the kernel checks the
    // path that reaches it. Nothing else about the run differs.
    assert_eq!(verdict(&sol, true), "bad-reachable", "with reentry: the bound does not");

    // The same contract with the effect before the interaction. The bound is
    // restored before control leaves, so a reentrant call finds the contract
    // in a state it is allowed to be in. This is the half that says the
    // analysis is not just reporting every external call: it is the ordering
    // that decides, and mulu works it out rather than matching a pattern.
    let ordered = root.join("PotOrdered.sol");
    assert_eq!(verdict(&ordered, true), "bad-unreachable", "effect before interaction is safe");
}

#[test]
fn a_getter_in_a_guard_reads_as_the_variable_it_returns() {
    if !ready() {
        return;
    }
    // `require(!paused())` reads storage through a function, and the walk
    // does not enter a helper with no effect and no check. The term stayed
    // as the call, where a specification says `_paused`, and the two could
    // not meet. This is the shape OpenZeppelin's `Pausable` has, and the
    // shape of most of its access control.
    let dir = std::env::temp_dir().join(format!("mulu-getter-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("C.sol"),
        r#"pragma solidity ^0.8.0;
contract C {
    bool private _paused;
    function paused() public view returns (bool) { return _paused; }
    function pause() external { require(!paused()); _paused = true; }
}"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("p.json"),
        r#"{"schema_version":1,"call_properties":[
          {"id":"pause-reverts-when-paused","rules":[{"entrypoint":"pause()",
            "given":{"op":"ne","left":{"storage":"_paused"},"right":{"uint256":"0"}},
            "assert":"reverts"}]},
          {"id":"pause-succeeds-when-not","rules":[{"entrypoint":"pause()",
            "given":{"op":"eq","left":{"storage":"_paused"},"right":{"uint256":"0"}},
            "assert":"does-not-revert"}]}]}"#,
    )
    .unwrap();
    let out = dir.join("out");
    let code = mulu()
        .args(["analyze", dir.join("C.sol").to_str().unwrap()])
        .args(["--contract", "C", "--out", out.to_str().unwrap()])
        .args(["--call-properties", dir.join("p.json").to_str().unwrap()])
        .status()
        .unwrap()
        .code()
        .unwrap();
    assert_eq!(code, 0);
    let answers = json(&out.join("call-properties.json"));
    let verdicts: Vec<String> = answers
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["verdict"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(verdicts, vec!["holds", "holds"], "both directions of `success <=> !paused`");
    let _ = std::fs::remove_dir_all(&dir);
}
