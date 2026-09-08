//! What an external review of the P1-05 change found, pinned shut.
//!
//! Each of these was reproduced before it was fixed. They are grouped here
//! because they share a shape: the plant made a claim the implementation
//! model could not support.

use mulu_abstraction::model::{Abstraction, Builder};
use mulu_abstraction::spec::Spec;
use mulu_yul::{lower_contract, ProgramIr};

fn ir(name: &str, yul: &str, abi: &str, layout: &str) -> ProgramIr {
    lower_contract(
        name,
        &format!("{name}.sol"),
        "0.8.28",
        yul,
        &serde_json::from_str(abi).unwrap(),
        serde_json::from_str(layout).unwrap(),
    )
    .unwrap()
}

fn limits() -> ProgramIr {
    ir(
        "Limits",
        include_str!("../../mulu-yul/tests/fixtures/Limits.yul"),
        include_str!("../../mulu-yul/tests/fixtures/Limits.abi.json"),
        include_str!("../../mulu-yul/tests/fixtures/Limits.storage.json"),
    )
}

fn build(ir: &ProgramIr, spec: &str, contract: &str) -> Abstraction {
    let props = Spec::parse(spec)
        .unwrap()
        .compile(contract, &ir.storage_layout)
        .unwrap();
    Builder::new(ir, &props).build()
}

/// A specification that permits nothing must forbid every successful end.
/// It compiles to a constant predicate with no variable, and skipping those
/// read "permits nothing" as "permits everything", so the plant forbade
/// nothing and every rejection looked like an overrestriction.
#[test]
fn a_specification_that_permits_nothing_forbids_every_success() {
    let spec = r#"{"schema_version":1,"properties":[{"id":"impossible","contract":"Limits",
        "when":"successful-transaction-end",
        "assert":{"op":"eq","left":{"uint256":"0"},"right":{"uint256":"1"}}}]}"#;
    let ir = limits();
    let a = build(&ir, spec, "Limits");

    assert_eq!(a.model.bad, vec!["bad".to_string()], "nothing may succeed");
    let plant = a.model.control_plant.as_ref().unwrap();
    assert_eq!(
        plant.bad,
        vec!["bad".to_string()],
        "the plant carries the same monitor"
    );
    assert!(a.model.transitions.iter().any(|t| t.to == "bad"));
    assert!(plant.transitions.iter().any(|t| t.to == "bad"));
}

/// The mirror image: a specification that permits everything forbids nothing.
#[test]
fn a_specification_that_permits_everything_forbids_nothing() {
    let spec = r#"{"schema_version":1,"properties":[{"id":"trivial","contract":"Limits",
        "when":"successful-transaction-end",
        "assert":{"op":"eq","left":{"uint256":"1"},"right":{"uint256":"1"}}}]}"#;
    let ir = limits();
    let a = build(&ir, spec, "Limits");
    assert!(a.model.bad.is_empty());
    assert!(!a.model.transitions.iter().any(|t| t.to == "bad"));
}

/// `if (..) revert()` is a guard the author wrote, but solc puts it on a
/// branch rather than a helper call. Leaving it out of the trace meant the
/// contract was modelled as though it had no guard: no redundancy verdict and
/// no control site to compare against.
#[test]
fn a_guard_written_as_if_revert_is_still_a_guard() {
    let ir = ir(
        "Gate",
        include_str!("../../mulu-yul/tests/fixtures/Gate.yul"),
        include_str!("../../mulu-yul/tests/fixtures/Gate.abi.json"),
        include_str!("../../mulu-yul/tests/fixtures/Gate.storage.json"),
    );
    let a = build(
        &ir,
        include_str!("../../../examples/guards/Gate.spec.json"),
        "Gate",
    );
    assert!(
        a.report.complete(),
        "unsupported: {:?}",
        a.report.unsupported
    );

    // It is a first-class check and it splits the argument domain at 100.
    // The id is `gen:...` here because this lowering has no AST; giving an
    // author-written guard a letter needs one, and the CLI test covers that.
    assert_eq!(a.model.checks.len(), 1);
    let id = a.model.checks[0].id.clone();
    assert_eq!(
        a.report.argument_regions[0].set,
        mulu_abstraction::interval::IntervalSet::le(mulu_abstraction::interval::U256::from(100u64))
    );

    // and the plant has a site for it, so an overrestriction can be seen
    let plant = a.model.control_plant.as_ref().unwrap();
    let site = plant
        .sites
        .iter()
        .find(|s| s.check.as_deref() == Some(&id))
        .expect("a site for it");
    assert!(!site.pairs.is_empty());
    assert_eq!(site.continue_event, format!("cont_{id}"));

    // the guard rejects above 100 and lets the store through below it
    let goes = |from: &str, ev: &str| -> Option<String> {
        a.model
            .transitions
            .iter()
            .find(|t| t.from == from && t.event == ev)
            .map(|t| t.to.clone())
    };
    let s0 = goes("idle_LIM0", "call_setLimit#X0").unwrap();
    let s1 = goes(&s0, &format!("{id}_pass")).expect("the guard passes below 100");
    // The region the store lands in is part of the event, so match on the
    // prefix: what this test is about is that the store happens at all.
    assert!(
        a.model
            .transitions
            .iter()
            .any(|t| t.from == s1 && t.event.starts_with("store_limit")),
        "the store happens below the bound"
    );
    let r0 = goes("idle_LIM0", "call_setLimit#X1").unwrap();
    assert!(
        goes(&r0, &format!("{id}_fail")).is_some(),
        "and rejects above it"
    );
}

/// A guard no region reaches is dead code. The plant walks past it, so it
/// needs a declaration; without one the site pointed at a check the model
/// never declared and the validator rejected the whole output.
#[test]
fn a_guard_no_execution_reaches_is_declared_and_called_unreachable() {
    // A and B are contradictory, so nothing gets as far as C.
    let ir = ir(
        "Three",
        include_str!("fixtures/Three.yul"),
        include_str!("fixtures/Three.abi.json"),
        include_str!("fixtures/Three.storage.json"),
    );
    let spec = r#"{"schema_version":1,"properties":[{"id":"b","contract":"Three",
        "when":"successful-transaction-end",
        "assert":{"op":"ule","left":{"storage":"limit"},"right":{"uint256":"1000"}}}]}"#;
    let a = build(&ir, spec, "Three");

    let ids: Vec<&str> = a.model.checks.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["A", "B", "C"],
        "every guard on the path is declared"
    );

    // C is declared but nothing is labelled with either of its events: that
    // is what makes the core call it unreachable rather than never-failing.
    let c = a.model.checks.iter().find(|c| c.id == "C").unwrap();
    assert!(!a.model.transitions.iter().any(|t| t.event == c.pass_event));
    assert!(!a.model.transitions.iter().any(|t| t.event == c.fail_event));

    // and every site still names a check the model declares
    let plant = a.model.control_plant.as_ref().unwrap();
    for s in &plant.sites {
        if let Some(check) = &s.check {
            assert!(
                ids.contains(&check.as_str()),
                "site {} names an undeclared check",
                s.id
            );
        }
    }
    let text = serde_json::to_string(&a.model).unwrap();
    mulu_model::validate::parse_and_validate(&text).expect("the model must still validate");
}

/// Two `require`s with no message lower to calls of the *same* helper. When a
/// check was identified by its block and helper name alone, both instructions
/// resolved to the first check: the second guard's condition was silently
/// replaced by the first's, and an argument the contract rejects was modelled
/// as accepted.
#[test]
fn two_requires_sharing_a_helper_stay_two_checks() {
    let ir = ir(
        "Dup",
        include_str!("fixtures/Dup.yul"),
        include_str!("fixtures/Dup.abi.json"),
        include_str!("fixtures/Dup.storage.json"),
    );
    let spec = r#"{"schema_version":1,"properties":[{"id":"b","contract":"Dup",
        "when":"successful-transaction-end",
        "assert":{"op":"ule","left":{"storage":"limit"},"right":{"uint256":"1000"}}}]}"#;
    let a = build(&ir, spec, "Dup");
    assert!(
        a.report.complete(),
        "unsupported: {:?}",
        a.report.unsupported
    );

    let ids: Vec<&str> = a.model.checks.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["A", "B"],
        "one id for two guards substitutes one condition for the other"
    );

    // `require(x < 10)` then `require(x > 20)` cannot both hold, so nothing is
    // ever stored. Under the old matching, x = 5 passed both and stored.
    assert!(
        !a.model
            .transitions
            .iter()
            .any(|t| t.event.starts_with("store_")),
        "contradictory guards must leave no path to a store"
    );
    // the first region is below 10, where A passes and B must fail
    let goes = |from: &str, ev: &str| -> Option<String> {
        a.model
            .transitions
            .iter()
            .find(|t| t.from == from && t.event == ev)
            .map(|t| t.to.clone())
    };
    let s0 = goes("idle_LIM0", "call_f#X0").unwrap();
    let s1 = goes(&s0, "A_pass").expect("A passes below 10");
    assert!(
        goes(&s1, "B_fail").is_some(),
        "B must reject the same value"
    );
}

/// A `require` with no message uses a helper named exactly `require_helper`.
/// Matching only the `require_helper_` prefix classified it as compiler
/// inserted, so it got a generated id instead of a letter.
#[test]
fn a_require_without_a_message_is_still_a_require() {
    let ir = ir(
        "Dup",
        include_str!("fixtures/Dup.yul"),
        include_str!("fixtures/Dup.abi.json"),
        include_str!("fixtures/Dup.storage.json"),
    );
    let source: Vec<&mulu_yul::Check> = ir
        .checks
        .iter()
        .filter(|c| c.origin == mulu_yul::CheckOrigin::Require)
        .collect();
    assert_eq!(
        source.len(),
        2,
        "both requires, {:?}",
        ir.checks
            .iter()
            .map(|c| (&c.id, c.origin))
            .collect::<Vec<_>>()
    );
    assert!(source
        .iter()
        .all(|c| c.helper.as_deref() == Some("require_helper")));
    let ids: Vec<&str> = source.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, vec!["A", "B"]);
}

/// Injective entrypoint names are not enough on their own: the call event
/// joins the name to the argument region, so a function actually named `f_X0`
/// shared an event with `f` called on region X0. One event with two targets
/// is not a model the supervisory-control core accepts.
#[test]
fn a_function_named_like_a_region_does_not_share_its_call_event() {
    let ir = ir(
        "Clash",
        include_str!("fixtures/Clash.yul"),
        include_str!("fixtures/Clash.abi.json"),
        include_str!("fixtures/Clash.storage.json"),
    );
    let spec = r#"{"schema_version":1,"properties":[{"id":"b","contract":"Clash",
        "when":"successful-transaction-end",
        "assert":{"op":"ule","left":{"storage":"limit"},"right":{"uint256":"1000"}}}]}"#;
    let a = build(&ir, spec, "Clash");

    // both entrypoints are modelled, and their request events differ
    let calls: Vec<&str> = a
        .model
        .events
        .iter()
        .map(|e| e.id.as_str())
        .filter(|e| e.starts_with("call_"))
        .collect();
    let mut uniq = calls.clone();
    uniq.sort();
    uniq.dedup();
    assert_eq!(
        uniq.len(),
        calls.len(),
        "two entrypoints share a request event: {calls:?}"
    );

    // and both graphs stay partially deterministic
    let text = serde_json::to_string(&a.model).unwrap();
    let parsed = mulu_model::validate::parse_and_validate(&text)
        .expect("the model and its plant must satisfy finite-product v1");
    assert!(parsed.control_plant.is_some());
}
