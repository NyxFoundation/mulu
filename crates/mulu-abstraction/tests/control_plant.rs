//! P1-05: the reference plant, its envelope, and overrestriction.
//!
//! docs/04 §5 keeps three things apart: the implementation's own behaviour,
//! the largest behaviour a supervisor could allow, and the difference between
//! them. The plant is what the second is computed on. docs/11 §4 fixes its
//! shape: continuing at a control site is controllable, rejecting there is
//! not, and the guard's own condition plays no part.

use mulu_abstraction::model::{Abstraction, Builder};
use mulu_abstraction::spec::Spec;
use mulu_model::schema::{Control, ControlPlant};
use mulu_yul::{lower_contract, ProgramIr};

fn build() -> (ProgramIr, Abstraction) {
    let ir = lower_contract(
        "Limits",
        "Limits.sol",
        "0.8.28+commit.7893614a.Linux.g++",
        include_str!("../../mulu-yul/tests/fixtures/Limits.yul"),
        &serde_json::from_str(include_str!(
            "../../mulu-yul/tests/fixtures/Limits.abi.json"
        ))
        .unwrap(),
        serde_json::from_str(include_str!(
            "../../mulu-yul/tests/fixtures/Limits.storage.json"
        ))
        .unwrap(),
    )
    .unwrap();
    let props = Spec::parse(include_str!("../../../examples/limits/Limits.spec.json"))
        .unwrap()
        .compile("Limits", &ir.storage_layout)
        .unwrap();
    let a = Builder::new(&ir, &props).build();
    (ir, a)
}

fn plant(a: &Abstraction) -> &ControlPlant {
    a.model.control_plant.as_ref().expect("a reference plant")
}

#[test]
fn continuing_is_controllable_and_rejecting_is_not() {
    let (_, a) = build();
    let p = plant(&a);
    for e in &p.events {
        let expected = if e.id.starts_with("cont_") {
            Control::Controllable
        } else {
            Control::Uncontrollable
        };
        assert_eq!(e.control, expected, "event {}", e.id);
    }
    // docs/11 §4: rejection stays available whatever the supervisor allows,
    // which is what makes this plant conservative.
    assert!(p.events.iter().any(|e| e.id.starts_with("rej_")));
    for site in &p.sites {
        let rej = site.continue_event.replacen("cont_", "rej_", 1);
        assert!(
            p.events.iter().any(|e| e.id == rej),
            "no rejection at site {}",
            site.id
        );
    }
}

#[test]
fn a_site_sits_at_every_guard_and_at_an_unguarded_writer() {
    let (_, a) = build();
    let p = plant(&a);
    let mut ids: Vec<&str> = p.sites.iter().map(|s| s.id.as_str()).collect();
    ids.sort();
    assert_eq!(ids, vec!["A", "B", "entry_forceSet"]);

    // the two guards are paired with the checks they parameterise
    for id in ["A", "B"] {
        let s = p.sites.iter().find(|s| s.id == id).unwrap();
        assert_eq!(s.check.as_deref(), Some(id));
        assert!(!s.pairs.is_empty());
    }
    // forceSet has no guard, so its site is unpaired: it asks whether one is
    // needed, it does not compare against an existing one
    let f = p.sites.iter().find(|s| s.id == "entry_forceSet").unwrap();
    assert!(f.check.is_none() && f.pairs.is_empty());
    // the getter writes nothing, so there is nothing to forbid there
    assert!(!ids.iter().any(|i| i.contains("limit")));
}

#[test]
fn every_paired_state_exists_in_both_models() {
    let (_, a) = build();
    let p = plant(&a);
    for site in &p.sites {
        for pair in &site.pairs {
            assert!(
                p.states.contains(&pair.plant_state),
                "{} is not a plant state",
                pair.plant_state
            );
            // The implementation stops at the first failing guard, so a pair
            // beyond that point would name a state that does not exist.
            assert!(
                a.model.states.contains(&pair.impl_state),
                "{} is not an implementation state",
                pair.impl_state
            );
        }
    }
}

#[test]
fn the_plant_carries_the_same_monitor_as_the_implementation() {
    let (_, a) = build();
    let p = plant(&a);
    // Without it nothing is unsafe, the envelope forbids nothing, and every
    // rejection the implementation makes looks like an overrestriction.
    assert_eq!(p.bad, vec!["bad".to_string()]);
    assert!(p.transitions.iter().any(|t| t.to == "bad"));
    assert!(p.accepting.as_ref().is_some_and(|acc| !acc.is_empty()));
    for s in p.accepting.as_ref().unwrap() {
        assert!(
            p.marked.contains(s),
            "an accepting state must be marked: {s}"
        );
    }
}

#[test]
fn the_plant_is_partially_deterministic_and_well_formed() {
    let (_, a) = build();
    // the same validator the supervisory-control core applies
    let text = serde_json::to_string(&a.model).unwrap();
    let parsed = mulu_model::validate::parse_and_validate(&text)
        .expect("the generated model and its plant must satisfy finite-product v1");
    assert!(parsed.control_plant.is_some());
}

#[test]
fn the_guard_condition_plays_no_part_in_the_plant() {
    let (_, a) = build();
    let p = plant(&a);
    // Both outcomes are available at a site whatever region we are in: that
    // is what parameterising the check out means (docs/04 §2).
    for site in p.sites.iter().filter(|s| s.check.is_some()) {
        for pair in &site.pairs {
            let out: Vec<&str> = p
                .transitions
                .iter()
                .filter(|t| t.from == pair.plant_state)
                .map(|t| t.event.as_str())
                .collect();
            assert!(out.contains(&site.continue_event.as_str()), "{:?}", out);
            let rej = site.continue_event.replacen("cont_", "rej_", 1);
            assert!(out.contains(&rej.as_str()), "{:?}", out);
        }
    }
}
