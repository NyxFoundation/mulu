//! SARIF 2.1.0 output (P1-06).
//!
//! The reason this file is more than a field rename: SARIF has a vocabulary
//! for "I did not decide", and mulu needs it. A result carries both a `kind`
//! (what the tool concluded) and a `level` (how bad it is), and the standard
//! says that when `kind` is anything other than `fail`, `level` must be
//! `none`. So `candidate` and `unknown` cannot be rendered as errors by a
//! conforming viewer even if the viewer ignores everything mulu writes about
//! scope. That is the same rule docs/09 §4 states for the console output, and
//! here it is enforced by the format rather than by us.
//!
//! | mulu status    | SARIF kind      | shown as        |
//! |----------------|-----------------|-----------------|
//! | proven         | fail            | the severity    |
//! | reproduced     | fail            | the severity    |
//! | candidate      | review          | needs a human   |
//! | unknown        | open            | undecided       |
//! | not-requested  | notApplicable   | not run         |
//!
//! `model-safe / proven` is the one proven result that is not a defect, so it
//! goes out as `pass`.
//!
//! Findings stay at `abstract-model` until the correspondence obligations of
//! P1-04 are discharged. A SARIF consumer will still draw them on a line of
//! Solidity, so every message says which model the claim is about; the
//! machine-readable form is in `properties.scope` and `properties.obligations`.

use crate::build::Site;
use crate::report::Diagnostic;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

pub const SCHEMA: &str = "https://json.schemastore.org/sarif-2.1.0.json";
pub const VERSION: &str = "2.1.0";
const INFO_URI: &str = "https://github.com/NyxFoundation/mulu";

/// `(kind, SARIF name, one-line description)`.
const RULES: &[(&str, &str, &str)] = &[
    (
        "spec-violation",
        "SpecificationViolation",
        "A state the specification forbids is reachable. The trace is the shortest path to it.",
    ),
    (
        "model-safe",
        "SpecificationHolds",
        "No state the specification forbids is reachable.",
    ),
    (
        "redundant-check",
        "RedundantCheck",
        "The check never fails on any reachable state, so it never rejects anything. Removing it \
         is a separate question: see the assumptions on the result.",
    ),
    (
        "unreachable-check",
        "UnreachableCheck",
        "The check is never evaluated: nothing reaches it.",
    ),
    (
        "check-may-fail",
        "CheckMayFail",
        "The check rejects some reachable state. This is the ordinary case and not a defect.",
    ),
    (
        "check-unknown",
        "CheckNotExamined",
        "The analysis did not run far enough to say anything about this check.",
    ),
    (
        "overrestriction",
        "Overrestriction",
        "The check rejects a call the specification allows: the guard is stricter than the \
         property it protects.",
    ),
    (
        "envelope",
        "SupervisoryEnvelope",
        "The maximal permissive supervisor for the objective: the largest behaviour that stays \
         safe and nonblocking.",
    ),
];

/// SARIF's own words for the five things mulu can conclude. `level` is
/// separate and only meaningful for `fail`.
fn kind_of(d: &Diagnostic) -> &'static str {
    kind_for(d.kind, d.claim, d.status)
}

/// Taken apart from [`Diagnostic`] so `verify` can apply the same mapping to
/// what is on disk. One rule, one place.
fn kind_for(kind: &str, claim: &str, status: &str) -> &'static str {
    match (kind, claim, status) {
        (_, _, "not-requested") => "notApplicable",
        // Undecided comes before every other rule: an envelope that was never
        // computed is not a description of anything.
        (_, _, "unknown") => "open",
        ("model-safe", _, "proven") => "pass",
        // The envelope is what the supervisor would allow. It is the answer to
        // a question, not a defect; only an unrealizable objective is one.
        ("envelope", "maximal-permissive", _) => "informational",
        // A guard that can reject something is the ordinary case.
        ("check-may-fail", _, _) => "informational",
        (_, _, "proven" | "reproduced") => "fail",
        (_, _, "candidate") => "review",
        _ => "open",
    }
}

fn level_of(d: &Diagnostic, kind: &str) -> &'static str {
    level_for(d.severity, kind)
}

fn level_for(severity: &str, kind: &str) -> &'static str {
    if kind != "fail" {
        // Required by the standard, and it is the behaviour we want anyway.
        return "none";
    }
    match severity {
        "ERROR" => "error",
        "WARNING" => "warning",
        _ => "note",
    }
}

/// Stable across runs and across machines: it names the finding, not the run.
/// Two runs of the same version on the same source produce the same value, so
/// a consumer can tell a finding that persists from one that is new.
fn fingerprint(d: &Diagnostic, site: Option<&Site>) -> String {
    let where_ = match site {
        Some(s) => match &s.region {
            Some(r) => format!("{}:{}:{}", s.path, r.start_line, r.start_column),
            None => s.path.clone(),
        },
        None => "(no source location)".into(),
    };
    let seed = format!("{}\u{1}{}\u{1}{}\u{1}{}\u{1}{}", d.kind, d.claim, d.id, where_, d.check_id.as_deref().unwrap_or(""));
    mulu_model::hash::sha256_hex(seed.as_bytes())[..16].to_string()
}

fn location(site: &Site) -> Value {
    let mut phys = Map::new();
    phys.insert("artifactLocation".into(), json!({"uri": site.path}));
    if let Some(r) = &site.region {
        phys.insert(
            "region".into(),
            json!({"startLine": r.start_line, "startColumn": r.start_column,
                   "endLine": r.end_line, "endColumn": r.end_column}),
        );
    }
    json!({"physicalLocation": Value::Object(phys)})
}

/// The message a human reads in a code-scanning annotation. It has to stand
/// on its own: the reader sees this line and nothing else from the run.
fn message(d: &Diagnostic) -> String {
    let mut s = d.message.replace('\n', " ");
    // Only where the status changes what the reader should do with it: on an
    // informational result "mulu did not prove it" reads as a hedge on a
    // statement nobody took for a defect.
    match (kind_of(d), d.status) {
        ("review", _) => s.push_str("  This is a candidate: mulu did not prove it."),
        ("open", _) => s.push_str("  mulu could not decide this."),
        _ => {}
    }
    // What the EVM said. docs/08 section 5: a replay that does not reproduce
    // is not evidence the counterexample was spurious, so the finding keeps
    // its status; the disagreement is a gap the reviewer has to see.
    match d.reproduction.as_ref().and_then(|r| r["status"].as_str()) {
        Some("reproduced") => s.push_str("  The trace was replayed on a local EVM and reproduced."),
        Some("not-reproduced") => s.push_str(
            "  The replay on a local EVM did not reproduce this, which is a disagreement between \
             the model and the EVM rather than a reason to dismiss the finding.",
        ),
        Some("unsupported") => {
            s.push_str("  mulu could not turn this into concrete calls, so it was not replayed.")
        }
        _ => {}
    }
    if d.scope == "abstract-model" && d.status != "not-requested" {
        s.push_str(
            "  The claim is about the finite model mulu generated, not yet about the Solidity \
             source; see the correspondence obligations in the report.",
        );
    }
    s
}

/// A viewer shows a message in a panel, and an envelope over a large model
/// names every winning state. Past this the text stops being read and starts
/// being scrolled, and report.json has all of it anyway.
const MAX_MESSAGE: usize = 1200;

fn truncate(mut s: String) -> String {
    if s.chars().count() <= MAX_MESSAGE {
        return s;
    }
    let cut = s.char_indices().nth(MAX_MESSAGE).map(|(i, _)| i).unwrap_or(s.len());
    s.truncate(cut);
    s.push_str(" … (truncated; the full text is in report.json)");
    s
}

/// Build the SARIF document for one run.
pub fn build(
    diags: &[Diagnostic],
    sites: &BTreeMap<String, Site>,
    exit_code: i32,
    fallback: Option<&Site>,
    invocation: Value,
) -> Value {
    let used: Vec<&(&str, &str, &str)> =
        RULES.iter().filter(|(k, _, _)| diags.iter().any(|d| d.kind == *k)).collect();
    let rules: Vec<Value> = used
        .iter()
        .map(|(id, name, text)| {
            json!({
                "id": id,
                "name": name,
                "shortDescription": {"text": *text},
                "fullDescription": {"text": *text},
                "defaultConfiguration": {"level": if *id == "spec-violation" { "warning" } else { "note" }},
                "helpUri": format!("{INFO_URI}#{id}"),
            })
        })
        .collect();

    // Only files a result points into, each with the digest mulu read.
    let mut artifacts: BTreeMap<(String, String), ()> = BTreeMap::new();
    let mut results = vec![];
    for d in diags {
        let site = d.check_id.as_ref().and_then(|c| sites.get(c)).or(fallback);
        let kind = kind_of(d);
        let mut r = Map::new();
        r.insert("ruleId".into(), json!(d.kind));
        if let Some(i) = used.iter().position(|(k, _, _)| *k == d.kind) {
            r.insert("ruleIndex".into(), json!(i));
        }
        r.insert("kind".into(), json!(kind));
        r.insert("level".into(), json!(level_of(d, kind)));
        r.insert("message".into(), json!({"text": truncate(message(d))}));
        if let Some(s) = site {
            artifacts.insert((s.path.clone(), s.sha256.clone()), ());
            r.insert("locations".into(), json!([location(s)]));
        }
        r.insert("partialFingerprints".into(), json!({"muluFinding/v1": fingerprint(d, site)}));
        let mut props = Map::new();
        props.insert("id".into(), json!(d.id));
        props.insert("claim".into(), json!(d.claim));
        props.insert("status".into(), json!(d.status));
        props.insert("scope".into(), json!(d.scope));
        props.insert("assumptions".into(), json!(d.assumptions));
        if !d.depends_on.is_empty() {
            props.insert("depends_on".into(), json!(d.depends_on));
        }
        if !d.obligations.is_empty() {
            props.insert("obligations".into(), json!(d.obligations));
        }
        if let Some(e) = &d.evidence {
            props.insert(
                "evidence".into(),
                json!({"type": e.kind, "path": e.path, "checked": e.checked, "kernel_checked": e.kernel_checked}),
            );
        }
        if let Some(rep) = &d.reproduction {
            props.insert("reproduction".into(), json!({"status": rep["status"], "reason": rep["reason"]}));
        }
        r.insert("properties".into(), Value::Object(props));
        results.push(Value::Object(r));
    }

    let artifacts: Vec<Value> = artifacts
        .into_keys()
        .map(|(uri, sha)| {
            json!({"location": {"uri": uri, "uriBaseId": "%SRCROOT%"}, "hashes": {"sha-256": sha}})
        })
        .collect();

    json!({
        "$schema": SCHEMA,
        "version": VERSION,
        "runs": [{
            "tool": {"driver": {
                "name": "mulu",
                "version": env!("CARGO_PKG_VERSION"),
                "semanticVersion": env!("CARGO_PKG_VERSION"),
                "informationUri": INFO_URI,
                "rules": rules,
            }},
            "automationDetails": {"id": "mulu/analyze"},
            "artifacts": artifacts,
            "invocations": [invocation],
            "results": results,
            // Repeated here because a SARIF consumer often shows the run and
            // not the report, and the scope is the thing not to lose.
            "properties": {"exitCode": exit_code, "scopeNote":
                "Findings are claims about the finite model mulu generated. Each result names its \
                 own scope in properties.scope."},
        }],
    })
}

/// `executionSuccessful` is about mulu, not about the code under analysis: a
/// run that finished and found a violation succeeded.
pub fn invocation(
    exit_code: i32,
    statuses: &[(String, String)],
    errors: &[String],
    unsupported: &[String],
) -> Value {
    let mut notifications: Vec<Value> = errors
        .iter()
        .map(|e| json!({"level": "error", "message": {"text": format!("cross-check mismatch (tool bug): {e}")}}))
        .collect();
    // A finding proven on a model that omits part of the contract is a fact
    // about that model. This is where the run says the model is not the whole
    // contract, so the results below are not a statement about what was left
    // out. It is a property of the run, not a finding, which is why it is a
    // notification rather than a result.
    if !unsupported.is_empty() {
        notifications.push(json!({
            "level": "warning",
            "message": {"text": format!(
                "the abstraction left {} thing(s) unmodelled, so the model is not the whole \
                 contract and nothing here says anything about them: {}",
                unsupported.len(),
                unsupported.join("; ")
            )},
        }));
    }
    json!({
        "executionSuccessful": exit_code != 4 && errors.is_empty(),
        "exitCode": exit_code,
        "toolExecutionNotifications": notifications,
        "properties": {
            "analyses": statuses.iter().map(|(a, s)| json!({"analysis": a, "status": s})).collect::<Vec<_>>(),
            "unsupported": unsupported,
        },
    })
}

/// `verify`: the SARIF must say what the report says. A hand-edited results
/// file is the easy way to make a finding disappear from a pull request while
/// the report it was derived from still records it, so the two are compared
/// finding by finding rather than trusted to agree.
pub fn check_against(report: &Value, doc: &Value) -> Vec<String> {
    let mut bad = vec![];
    if doc["version"] != VERSION {
        bad.push(format!("results.sarif is version {}, not {VERSION}", doc["version"]));
    }
    let runs = doc["runs"].as_array().cloned().unwrap_or_default();
    if runs.len() != 1 {
        bad.push(format!("results.sarif has {} runs, not 1", runs.len()));
        return bad;
    }
    let results = runs[0]["results"].as_array().cloned().unwrap_or_default();
    let diags = report["diagnostics"].as_array().cloned().unwrap_or_default();
    if results.len() != diags.len() {
        bad.push(format!(
            "results.sarif has {} result(s) and report.json has {} finding(s)",
            results.len(),
            diags.len()
        ));
    }
    // Findings are matched by id, so two findings sharing one would let a
    // second, different result pass as the first.
    let mut seen: std::collections::BTreeSet<&str> = Default::default();
    for d in &diags {
        if let Some(id) = d["id"].as_str() {
            if !seen.insert(id) {
                bad.push(format!("report.json has two findings called {id}"));
            }
        }
    }
    for d in &diags {
        let id = d["id"].as_str().unwrap_or("?");
        let Some(r) = results.iter().find(|r| r["properties"]["id"] == d["id"]) else {
            bad.push(format!("{id} is in report.json and not in results.sarif"));
            continue;
        };
        let (kind, claim, status, severity) = (
            d["kind"].as_str().unwrap_or(""),
            d["claim"].as_str().unwrap_or(""),
            d["status"].as_str().unwrap_or(""),
            d["severity"].as_str().unwrap_or(""),
        );
        let want_kind = kind_for(kind, claim, status);
        if r["ruleId"] != kind {
            bad.push(format!("{id} is a {kind:?} finding and results.sarif calls it {}", r["ruleId"]));
        }
        if r["kind"] != want_kind {
            bad.push(format!(
                "{id} is {status:?} in report.json, which is SARIF {want_kind:?}, and results.sarif says {}",
                r["kind"]
            ));
        }
        let want_level = level_for(severity, want_kind);
        if r["level"] != want_level {
            bad.push(format!("{id} should be level {want_level:?} and results.sarif says {}", r["level"]));
        }
        for field in ["status", "scope", "claim"] {
            if r["properties"][field] != d[field] {
                bad.push(format!(
                    "{id} has {field} {} in report.json and {} in results.sarif",
                    d[field], r["properties"][field]
                ));
            }
        }
    }
    for r in &results {
        let rid = &r["properties"]["id"];
        if !diags.iter().any(|d| d["id"] == *rid) {
            bad.push(format!("results.sarif reports {rid}, which is not in report.json"));
        }
    }
    bad
}
