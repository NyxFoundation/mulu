//! P1-02 acceptance (docs/09 §7): from the Limits ProgramIR and the
//! specification, build a model in which check B never fails and forceSet
//! reaches a violating state. Uses the committed solc output, so no compiler
//! is needed.

use mulu_abstraction::interval::{IntervalSet, U256};
use mulu_abstraction::model::{Abstraction, Builder, ENVIRONMENT_PROFILE};
use mulu_abstraction::spec::Spec;
use mulu_model::schema::Control;
use mulu_yul::{lower_contract, ProgramIr};

const SPEC: &str = include_str!("../../../examples/limits/Limits.spec.json");

fn ir() -> ProgramIr {
    lower_contract(
        "Limits",
        "Limits.sol",
        "0.8.28+commit.7893614a.Linux.g++",
        include_str!("../../mulu-yul/tests/fixtures/Limits.yul"),
        &serde_json::from_str(include_str!("../../mulu-yul/tests/fixtures/Limits.abi.json")).unwrap(),
        serde_json::from_str(include_str!("../../mulu-yul/tests/fixtures/Limits.storage.json")).unwrap(),
    )
    .unwrap()
}

fn build(with_spec: bool) -> (ProgramIr, Abstraction) {
    let ir = ir();
    let props = if with_spec {
        Spec::parse(SPEC).unwrap().compile("Limits", &ir.storage_layout).unwrap()
    } else {
        vec![]
    };
    let a = Builder::new(&ir, &props).build();
    (ir, a)
}

fn u(n: u64) -> U256 {
    U256::from(n)
}

fn goes<'a>(a: &'a Abstraction, from: &str, event: &str) -> Option<&'a str> {
    a.model
        .transitions
        .iter()
        .find(|t| t.from == from && t.event == event)
        .map(|t| t.to.as_str())
}

#[test]
fn the_argument_partition_is_the_one_docs_11_works_through() {
    let (_, a) = build(true);
    let regions: Vec<&IntervalSet> = a.report.argument_regions.iter().map(|r| &r.set).collect();
    assert_eq!(regions.len(), 3, "docs/11 §7 splits uint256 into three regions");
    assert_eq!(*regions[0], IntervalSet::le(u(100)));
    assert_eq!(*regions[1], IntervalSet::range(u(101), u(1000)));
    assert_eq!(*regions[2], IntervalSet::ge(u(1001)));

    // the regions cover uint256 and do not overlap
    let mut all = IntervalSet::empty();
    for r in &regions {
        assert!(all.disjoint_from(r));
        all = all.union(r);
    }
    assert!(all.is_full());

    // and the infeasible combination is reported as settled, not silently dropped
    assert!(
        a.report.discharged.iter().any(|d| d.contains("A and B and not spec:limit-bound")),
        "{:?}",
        a.report.discharged
    );
}

#[test]
fn the_specification_is_pulled_back_onto_the_argument() {
    let (_, a) = build(true);
    // `limit := x` means a bound on limit is a bound on x
    let pulled = a
        .report
        .argument_predicates
        .iter()
        .find(|p| p.id == "spec:limit-bound")
        .expect("the specification pulled back through the assignment");
    assert!(pulled.source.contains("pulled back"), "{}", pulled.source);
    assert_eq!(pulled.set, IntervalSet::le(u(1000)));
}

#[test]
fn check_b_has_no_failing_transition_and_depends_on_a() {
    let (_, a) = build(true);
    let b = a.model.checks.iter().find(|c| c.id == "B").expect("check B");
    assert_eq!(b.depends_on, vec!["A".to_string()], "B is only reached once A passed");

    // B never fails: nothing in the model is labelled with its fail event.
    assert!(
        !a.model.transitions.iter().any(|t| t.event == b.fail_event),
        "B must have no failing transition, which is what makes it redundant"
    );
    // A does fail, for arguments above 100.
    let ac = a.model.checks.iter().find(|c| c.id == "A").unwrap();
    assert!(a.model.transitions.iter().any(|t| t.event == ac.fail_event));
    assert!(ac.depends_on.is_empty());
}

#[test]
fn forceset_reaches_bad_and_setlimit_does_not() {
    let (_, a) = build(true);
    assert_eq!(a.model.bad, vec!["bad".to_string()]);

    // forceSet with an argument above the bound stores it and violates the spec
    let s1 = goes(&a, "idle_LIM0", "call_forceSet#X2").expect("forceSet on X2");
    let s2 = goes(&a, s1, "store_limit").expect("the store");
    let s3 = goes(&a, s2, "return").expect("a successful return");
    assert_eq!(goes(&a, s3, "next_tx"), Some("bad"), "the specification is violated");

    // setLimit cannot: the guard rejects X2 before any store
    let t1 = goes(&a, "idle_LIM0", "call_setLimit#X2").expect("setLimit on X2");
    let t2 = goes(&a, t1, "A_fail").expect("A rejects it");
    assert!(t2.contains("rev"), "the transaction reverts, got {t2}");
    assert_eq!(goes(&a, t2, "next_tx"), Some("idle_LIM0"), "revert restores the entry storage");
}

#[test]
fn every_event_is_uncontrollable_in_the_implementation_model() {
    let (_, a) = build(true);
    // docs/11 §4: a request arrives and a guard result follows from the code;
    // neither is something a supervisor can forbid. Everything controllable
    // lives in the reference plant instead.
    assert!(a.model.events.iter().all(|e| e.control == Control::Uncontrollable));
    let plant = a.model.control_plant.as_ref().expect("a reference plant");
    assert!(plant.events.iter().any(|e| e.control == Control::Controllable));
    assert!(plant
        .events
        .iter()
        .filter(|e| e.control == Control::Controllable)
        .all(|e| e.id.starts_with("cont_")));
}

#[test]
fn the_model_is_partially_deterministic() {
    let (_, a) = build(true);
    // the supervisory-control core rejects a model that is not
    let mut seen = std::collections::HashMap::new();
    for t in &a.model.transitions {
        if let Some(prev) = seen.insert((&t.from, &t.event), &t.to) {
            assert_eq!(prev, &t.to, "{} on {} has two targets", t.from, t.event);
        }
    }
}

#[test]
fn without_a_specification_only_redundancy_is_modelled() {
    let (_, a) = build(false);
    assert!(a.model.bad.is_empty(), "no specification means no bad state");
    // B is still redundant: that judgement never needed the specification
    let b = a.model.checks.iter().find(|c| c.id == "B").unwrap();
    assert!(!a.model.transitions.iter().any(|t| t.event == b.fail_event));
    // and the model is smaller, because storage is no longer partitioned
    let (_, with) = build(true);
    assert!(a.model.states.len() < with.model.states.len());
}

#[test]
fn the_environment_profile_and_its_assumptions_are_recorded() {
    let (_, a) = build(true);
    assert_eq!(a.report.environment_profile, ENVIRONMENT_PROFILE);
    let joined = a.report.assumptions.join("\n");
    for expected in ["abi-decoder-unverified", "initial-state", "argument-regions-are-exact"] {
        assert!(joined.contains(expected), "missing assumption {expected} in:\n{joined}");
    }
    assert!(a.report.complete(), "unsupported: {:?}", a.report.unsupported);
    assert_eq!(a.report.entrypoints_modelled.len(), 3);
    assert!(a.report.entrypoints_skipped.is_empty());
}

#[test]
fn the_generated_model_passes_the_schema_validator() {
    let (_, a) = build(true);
    let text = serde_json::to_string(&a.model).unwrap();
    let parsed = mulu_model::validate::parse_and_validate(&text)
        .expect("the generated model must satisfy finite-product v1");
    assert_eq!(parsed.states.len(), a.model.states.len());
}
