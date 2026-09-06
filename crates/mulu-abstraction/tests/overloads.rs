//! Two entrypoints with the same name must stay two entrypoints.
//!
//! The selector-to-signature pairing used to be guessed from the dispatcher
//! and the ABI by matching names. With an overload that gives one selector the
//! other's argument type, and the argument type decides the domain the model
//! reasons over, so calls were dropped. It also merged both control flows onto
//! one set of state names.

use mulu_abstraction::interval::{IntervalSet, U256};
use mulu_abstraction::model::{Abstraction, Builder};
use mulu_abstraction::spec::Spec;
use mulu_abstraction::types::domain_of;
use mulu_yul::{lower_contract, lower_contract_with, ProgramIr, SolcFacts};
use std::collections::BTreeMap;

const YUL: &str = include_str!("../../mulu-yul/tests/fixtures/Over.yul");
const ABI: &str = include_str!("../../mulu-yul/tests/fixtures/Over.abi.json");
const LAYOUT: &str = include_str!("../../mulu-yul/tests/fixtures/Over.storage.json");
const SELECTORS: &str = include_str!("../../mulu-yul/tests/fixtures/Over.selectors.json");

/// solc's table, inverted to selector -> signature.
fn selectors() -> BTreeMap<String, String> {
    let raw: BTreeMap<String, String> = serde_json::from_str(SELECTORS).unwrap();
    raw.into_iter().map(|(sig, sel)| (sel, sig)).collect()
}

fn ir_with_table() -> ProgramIr {
    lower_contract_with(
        "Over",
        "Over.sol",
        "0.8.28",
        YUL,
        &serde_json::from_str(ABI).unwrap(),
        serde_json::from_str(LAYOUT).unwrap(),
        SolcFacts { origins: None, selectors: selectors() },
    )
    .unwrap()
}

fn build(ir: &ProgramIr) -> Abstraction {
    let props = Spec::parse(include_str!("../../../examples/overload/over.spec.json"))
        .unwrap()
        .compile("Over", &ir.storage_layout)
        .unwrap();
    Builder::new(ir, &props).build()
}

#[test]
fn the_compilers_table_decides_which_overload_a_selector_is() {
    let ir = ir_with_table();
    let mut got: Vec<(&str, &str)> =
        ir.entrypoints.iter().map(|e| (e.selector.as_str(), e.signature.as_str())).collect();
    got.sort();
    assert_eq!(
        got,
        vec![
            ("0x24b8ba5f", "set(uint8)"),
            ("0x60fe47b1", "set(uint256)"),
            ("0xa4d66daf", "limit()"),
        ]
    );
}

#[test]
fn without_the_table_an_overload_is_reported_as_ambiguous_not_guessed() {
    let ir = lower_contract(
        "Over",
        "Over.sol",
        "0.8.28",
        YUL,
        &serde_json::from_str(ABI).unwrap(),
        serde_json::from_str(LAYOUT).unwrap(),
    )
    .unwrap();
    let ambiguous: Vec<&str> = ir
        .entrypoints
        .iter()
        .filter(|e| e.signature.contains("?overloaded"))
        .map(|e| e.selector.as_str())
        .collect();
    assert_eq!(ambiguous.len(), 2, "both `set` selectors are ambiguous by name alone");
    // the unambiguous one still resolves
    assert!(ir.entrypoints.iter().any(|e| e.signature == "limit()"));
}

#[test]
fn each_overload_gets_the_domain_of_its_own_argument() {
    let ir = ir_with_table();
    let a = build(&ir);
    assert!(a.report.complete(), "unsupported: {:?}", a.report.unsupported);

    let byte = domain_of("uint8").unwrap();
    let calls: Vec<&str> = a
        .model
        .events
        .iter()
        .map(|e| e.id.as_str())
        .filter(|e| e.starts_with("call_set"))
        .collect();
    let narrow: Vec<&&str> = calls.iter().filter(|c| c.starts_with("call_set_uint8#")).collect();
    let wide: Vec<&&str> = calls.iter().filter(|c| c.starts_with("call_set_uint256#")).collect();
    assert!(!narrow.is_empty() && !wide.is_empty());
    assert!(wide.len() > narrow.len(), "the uint256 overload reaches more regions");

    for c in &narrow {
        let idx: usize = c.rsplit("#X").next().unwrap().parse().unwrap();
        assert!(
            a.report.argument_regions[idx].set.subset_of(&byte),
            "{c} would receive a value no uint8 can hold"
        );
    }
}

#[test]
fn the_two_overloads_do_not_share_states() {
    let ir = ir_with_table();
    let a = build(&ir);
    // Before disambiguation both were `set#0_...`, merging two control flows.
    assert!(a.model.states.iter().any(|s| s.starts_with("set_uint8#")));
    assert!(a.model.states.iter().any(|s| s.starts_with("set_uint256#")));
    assert!(
        !a.model.states.iter().any(|s| s.starts_with("set#")),
        "a shared prefix means the two flows were merged"
    );

    // and the model stays partially deterministic
    let mut seen = BTreeMap::new();
    for t in &a.model.transitions {
        if let Some(prev) = seen.insert((&t.from, &t.event), &t.to) {
            assert_eq!(prev, &t.to, "{} on {} has two targets", t.from, t.event);
        }
    }
}

#[test]
fn the_guard_belongs_only_to_the_overload_that_declares_it() {
    let ir = ir_with_table();
    let a = build(&ir);
    // `require(x <= 100)` is in set(uint256); set(uint8) has no guard
    let guarded: Vec<&str> = a
        .model
        .transitions
        .iter()
        .filter(|t| t.event == "A_pass" || t.event == "A_fail")
        .map(|t| t.from.as_str())
        .collect();
    assert!(!guarded.is_empty());
    assert!(
        guarded.iter().all(|s| s.starts_with("set_uint256#")),
        "the guard leaked to the other overload: {guarded:?}"
    );
    // Neither overload can break the bound: one is capped at 100, the other
    // tops out at 255. Any transition into bad comes from the storage region
    // above the bound, which is only walked because the builder covers every
    // region; the core decides whether it is reachable.
    let from_a_setter: Vec<&str> = a
        .model
        .transitions
        .iter()
        .filter(|t| t.to == "bad" && t.from.starts_with("set"))
        .map(|t| t.from.as_str())
        .collect();
    assert!(from_a_setter.is_empty(), "a setter reached bad: {from_a_setter:?}");
    assert_eq!(
        a.report.argument_regions[0].set,
        IntervalSet::le(U256::from(100u64))
    );
}
