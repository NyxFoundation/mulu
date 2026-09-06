//! A guard written in a modifier in an imported file must reach the model.
//!
//! Before the walk followed internal calls, solc's lowering of a modifier
//! (`fun_f` -> `modifier_m` -> `fun_f_inner`) hid both the guard and the
//! storage write, and the abstraction reported a model in which `setLimit`
//! did nothing at all, with `unsupported: none`. This pins that shut.

use mulu_abstraction::interval::{IntervalSet, U256};
use mulu_abstraction::model::{Abstraction, Builder};
use mulu_abstraction::spec::Spec;
use mulu_yul::{lower_contract, ProgramIr};

const SPEC: &str = include_str!("../../../examples/access/vault.spec.json");

fn build() -> (ProgramIr, Abstraction) {
    let ir = lower_contract(
        "Vault",
        "Vault.sol",
        "0.8.28+commit.7893614a.Linux.g++",
        include_str!("../../mulu-yul/tests/fixtures/Vault.yul"),
        &serde_json::from_str(include_str!("../../mulu-yul/tests/fixtures/Vault.abi.json")).unwrap(),
        serde_json::from_str(include_str!("../../mulu-yul/tests/fixtures/Vault.storage.json")).unwrap(),
    )
    .unwrap();
    let props = Spec::parse(SPEC).unwrap().compile("Vault", &ir.storage_layout).unwrap();
    let a = Builder::new(&ir, &props).build();
    (ir, a)
}

fn goes<'a>(a: &'a Abstraction, from: &str, event: &str) -> Option<&'a str> {
    a.model.transitions.iter().find(|t| t.from == from && t.event == event).map(|t| t.to.as_str())
}

#[test]
fn the_modifier_guard_reaches_the_model() {
    let (_, a) = build();
    assert!(a.report.complete(), "unsupported: {:?}", a.report.unsupported);

    // Two source-level checks, and B is only reached once A passed, even
    // though A is written in another contract in another file.
    let ids: Vec<&str> = a.model.checks.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, vec!["A", "B"]);
    let b = a.model.checks.iter().find(|c| c.id == "B").unwrap();
    assert_eq!(b.depends_on, vec!["A".to_string()]);
    assert!(!a.model.transitions.iter().any(|t| t.event == b.fail_event), "B must never fail");
}

#[test]
fn setlimit_is_not_a_no_op() {
    let (_, a) = build();
    // the regression: guard, guard, store, return, all present on the X0 path
    let s0 = goes(&a, "idle_LIM0", "call_setLimit#X0").expect("the call");
    let s1 = goes(&a, s0, "A_pass").expect("the modifier's guard");
    let s2 = goes(&a, s1, "B_pass").expect("the function's own guard");
    let s3 = goes(&a, s2, "store_limit").expect("the storage write inside the inner body");
    let s4 = goes(&a, s3, "return").expect("a successful return");
    assert!(s4.contains("ret"));

    // and an argument the modifier rejects reverts before any store
    let r0 = goes(&a, "idle_LIM0", "call_setLimit#X1").expect("the call");
    let r1 = goes(&a, r0, "A_fail").expect("the modifier rejects it");
    assert!(r1.contains("rev"), "got {r1}");
    assert_eq!(goes(&a, r1, "next_tx"), Some("idle_LIM0"));
}

#[test]
fn the_partition_is_refined_by_the_guard_in_the_imported_modifier() {
    let (_, a) = build();
    // A lives in Base.sol yet still splits the argument domain at 100
    let sets: Vec<&IntervalSet> = a.report.argument_regions.iter().map(|r| &r.set).collect();
    assert_eq!(*sets[0], IntervalSet::le(U256::from(100u64)));
    assert_eq!(sets.len(), 3);
    assert!(
        a.report.argument_predicates.iter().any(|p| p.id == "A" && p.source.contains("reached from")),
        "{:?}",
        a.report.argument_predicates
    );
}

#[test]
fn forceset_still_violates_the_specification() {
    let (_, a) = build();
    let s1 = goes(&a, "idle_LIM0", "call_forceSet#X2").unwrap();
    let s2 = goes(&a, s1, "store_limit").unwrap();
    let s3 = goes(&a, s2, "return").unwrap();
    assert_eq!(goes(&a, s3, "next_tx"), Some("bad"));
}

#[test]
fn the_two_examples_agree_on_the_shape_of_the_finding() {
    // Limits writes both guards in the function; Vault writes the first in an
    // imported modifier. The conclusion must not depend on that difference.
    let (_, vault) = build();
    let limits_ir = lower_contract(
        "Limits",
        "Limits.sol",
        "0.8.28",
        include_str!("../../mulu-yul/tests/fixtures/Limits.yul"),
        &serde_json::from_str(include_str!("../../mulu-yul/tests/fixtures/Limits.abi.json")).unwrap(),
        serde_json::from_str(include_str!("../../mulu-yul/tests/fixtures/Limits.storage.json")).unwrap(),
    )
    .unwrap();
    let props = Spec::parse(include_str!("../../../examples/limits/limits.spec.json"))
        .unwrap()
        .compile("Limits", &limits_ir.storage_layout)
        .unwrap();
    let limits = Builder::new(&limits_ir, &props).build();

    let regions = |a: &Abstraction| -> Vec<IntervalSet> {
        a.report.argument_regions.iter().map(|r| r.set.clone()).collect()
    };
    assert_eq!(regions(&vault), regions(&limits));
    assert_eq!(vault.model.states.len(), limits.model.states.len());
    assert_eq!(vault.model.transitions.len(), limits.model.transitions.len());
}
