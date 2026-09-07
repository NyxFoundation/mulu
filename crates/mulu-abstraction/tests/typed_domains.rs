//! The ABI and the storage layout decide the domains.
//!
//! docs/11 §5 admits type-correct calls only. Reading the argument's type is
//! what makes that real: a `uint8` argument ranges over 256 values, and a
//! bound it cannot break is not a bound the model should let it break.

use mulu_abstraction::interval::{IntervalSet, U256};
use mulu_abstraction::model::{signature_params, Abstraction, Builder};
use mulu_abstraction::spec::{storage_vars, Spec};
use mulu_abstraction::types::domain_of;
use mulu_yul::{lower_contract, ProgramIr};

fn build() -> (ProgramIr, Abstraction) {
    let ir = lower_contract(
        "Meter",
        "Meter.sol",
        "0.8.28+commit.7893614a.Linux.g++",
        include_str!("../../mulu-yul/tests/fixtures/Meter.yul"),
        &serde_json::from_str(include_str!("../../mulu-yul/tests/fixtures/Meter.abi.json")).unwrap(),
        serde_json::from_str(include_str!("../../mulu-yul/tests/fixtures/Meter.storage.json")).unwrap(),
    )
    .unwrap();
    let props = Spec::parse(include_str!("../../../examples/typed/Meter.spec.json"))
        .unwrap()
        .compile("Meter", &ir.storage_layout)
        .unwrap();
    let a = Builder::new(&ir, &props).build();
    (ir, a)
}

fn u(n: u64) -> U256 {
    U256::from(n)
}

#[test]
fn signature_types_are_read_off_the_abi() {
    assert_eq!(signature_params("record(uint8)"), vec!["uint8"]);
    assert_eq!(signature_params("reading()"), Vec::<String>::new());
    assert_eq!(signature_params("f(uint256,address)"), vec!["uint256", "address"]);
    // nested commas do not split
    assert_eq!(signature_params("g((uint8,bool),uint256[])"), vec!["(uint8,bool)", "uint256[]"]);
}

#[test]
fn a_uint8_argument_never_leaves_its_type() {
    let (_, a) = build();
    assert!(a.report.complete(), "unsupported: {:?}", a.report.unsupported);

    // every state record can be called in carries an argument region inside
    // [0, 255]; nothing above it exists for that entrypoint
    let byte = domain_of("uint8").unwrap();
    let calls: Vec<&str> = a
        .model
        .transitions
        .iter()
        .filter(|t| t.event.starts_with("call_record"))
        .map(|t| t.event.as_str())
        .collect();
    assert!(!calls.is_empty());
    for c in &calls {
        let idx: usize = c.rsplit("#X").next().unwrap().parse().unwrap();
        assert!(
            a.report.argument_regions[idx].set.subset_of(&byte),
            "{c} would receive a value no uint8 can hold"
        );
    }
}

#[test]
fn the_narrow_entrypoint_cannot_break_a_bound_it_has_no_room_for() {
    let (_, a) = build();
    // reading <= 1000 and a uint8 tops out at 255, so record cannot violate it
    let to_bad: Vec<&str> =
        a.model.transitions.iter().filter(|t| t.to == "bad").map(|t| t.from.as_str()).collect();
    assert!(!to_bad.is_empty(), "force can still violate it");
    assert!(
        !to_bad.iter().any(|s| s.starts_with("record")),
        "record reached bad: {to_bad:?}"
    );
    // and the interval arithmetic recorded why. The predicate is keyed by
    // the type rather than by the entrypoint: two entrypoints taking a
    // `uint8` constrain the partition the same way, and keying by signature
    // made two identical predicates out of that.
    assert!(
        a.report.discharged.iter().any(|d| d.contains("type:uint8")),
        "{:?}",
        a.report.discharged
    );
}

#[test]
fn the_wide_entrypoint_still_can() {
    let (_, a) = build();
    let goes = |from: &str, ev: &str| -> Option<String> {
        a.model.transitions.iter().find(|t| t.from == from && t.event == ev).map(|t| t.to.clone())
    };
    // the last region is the one above the bound
    let last = a.report.argument_regions.len() - 1;
    let s1 = goes("idle_REA0", &format!("call_force#X{last}")).expect("force on the top region");
    let s2 = goes(&s1, "store_reading").unwrap();
    let s3 = goes(&s2, "return").unwrap();
    assert_eq!(goes(&s3, "next_tx").as_deref(), Some("bad"));
}

#[test]
fn the_guard_survives_the_cleanup_solc_inserts() {
    let (_, a) = build();
    // `require(x <= 100)` on a uint8 reaches the model through and(x, 0xff);
    // the type is what lets that be dropped, so the guard is still A at 100
    let regions: Vec<&IntervalSet> = a.report.argument_regions.iter().map(|r| &r.set).collect();
    assert_eq!(*regions[0], IntervalSet::le(u(100)));
    assert_eq!(*regions[1], IntervalSet::range(u(101), u(255)));
    assert!(a.model.checks.iter().any(|c| c.id == "A"));
}

#[test]
fn storage_types_come_from_the_layout_table() {
    let (ir, _) = build();
    let vars = storage_vars(&ir.storage_layout);
    let reading = vars.iter().find(|v| v.label == "reading").unwrap();
    assert_eq!(reading.type_label.as_deref(), Some("uint256"));
    assert_eq!(reading.bytes, Some(32));
    assert!(reading.whole_slot());
}
