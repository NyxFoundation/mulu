//! P1-01 acceptance (docs/09 §7): for examples/limits/Limits.sol the IR must
//! preserve checks A and B, the storage update, the revert paths and the
//! source locations. The Yul fixture is solc's real output, committed so this
//! test does not need solc installed.

use mulu_yul::ir::{CheckEdge, FunctionKind, Op, Terminator};
use mulu_yul::{lower_contract, CheckOrigin, ProgramIr, Purity};

fn ir() -> ProgramIr {
    lower_contract(
        "Limits",
        "Limits.sol",
        "0.8.28+commit.7893614a.Linux.g++",
        include_str!("fixtures/Limits.yul"),
        &serde_json::from_str(include_str!("fixtures/Limits.abi.json")).unwrap(),
        serde_json::from_str(include_str!("fixtures/Limits.storage.json")).unwrap(),
    )
    .expect("lowering solc output must succeed")
}

/// Source text of the span a location points at. Read from the fixture, not
/// from examples/, so the Yul and the source it was generated from always
/// agree; the drift test below catches them going out of step.
fn span_text(l: mulu_yul::Location) -> &'static str {
    const SRC: &str = include_str!("fixtures/Limits.sol");
    &SRC[l.byte_start as usize..(l.byte_start + l.byte_length) as usize]
}

#[test]
fn access_fixtures_match_their_example_contracts() {
    assert_eq!(
        include_str!("fixtures/access-Base.sol"),
        include_str!("../../../examples/access/Base.sol"),
        "examples/access/Base.sol changed; rerun tools/regen-yul-fixtures.sh"
    );
    assert_eq!(
        include_str!("fixtures/access-Vault.sol"),
        include_str!("../../../examples/access/Vault.sol"),
        "examples/access/Vault.sol changed; rerun tools/regen-yul-fixtures.sh"
    );
}

#[test]
fn typed_fixture_matches_its_example_contract() {
    assert_eq!(
        include_str!("fixtures/typed-Meter.sol"),
        include_str!("../../../examples/typed/Meter.sol"),
        "examples/typed/Meter.sol changed; rerun tools/regen-yul-fixtures.sh"
    );
}

#[test]
fn fixture_matches_the_example_contract() {
    // The committed Yul was generated from this exact source. If the example
    // changes, every @src offset shifts, so the pair must be regenerated
    // together: tools/regen-yul-fixtures.sh.
    assert_eq!(
        include_str!("fixtures/Limits.sol"),
        include_str!("../../../examples/limits/Limits.sol"),
        "examples/limits/Limits.sol changed; rerun tools/regen-yul-fixtures.sh"
    );
}

#[test]
fn the_two_requires_become_checks_a_and_b() {
    let ir = ir();
    let a = ir.check("A").expect("check A");
    let b = ir.check("B").expect("check B");

    for c in [a, b] {
        assert_eq!(c.origin, CheckOrigin::Require);
        assert_eq!(c.function, "fun_setLimit_27");
        assert_eq!(c.purity, Purity::Pure, "P1a needs pure guards");
        assert!(matches!(c.pass_edge, CheckEdge::Continue));
        assert!(matches!(c.fail_edge, CheckEdge::Revert { .. }));
    }

    // The conditions resolve to the comparisons of docs/08 §3: A is x <= 100
    // and B is x <= 1000, both over setLimit's parameter.
    assert_eq!(a.condition_text, "iszero(gt(var_x_5, 0x64))");
    assert_eq!(b.condition_text, "iszero(gt(var_x_5, 0x03e8))");
    // and what the generated Yul actually said is kept alongside
    assert_eq!(a.condition_as_written, "expr_11");
    assert_eq!(b.condition_as_written, "expr_18");
}

#[test]
fn check_locations_point_at_the_require_statements() {
    let ir = ir();
    assert_eq!(span_text(ir.check("A").unwrap().source.unwrap()), r#"require(x <= 100, "cap")"#);
    assert_eq!(span_text(ir.check("B").unwrap().source.unwrap()), r#"require(x <= 1000, "bound")"#);
}

#[test]
fn the_storage_update_is_preserved_with_its_location() {
    let ir = ir();
    let writes = ir.storage_writes();

    // both setters write storage, at the `limit = x` assignments
    for f in ["fun_setLimit_27", "fun_forceSet_37"] {
        let (_, ins) = writes.iter().find(|(fun, _)| *fun == f).unwrap_or_else(|| panic!("no storage write in {f}"));
        assert_eq!(span_text(ins.source.unwrap()), "limit = x");
        let Op::Effect { call } = &ins.op else { panic!("expected a call") };
        assert!(call.render().starts_with("update_storage_value_offset_0"), "{}", call.render());
    }

    // and the sstore itself is still in the IR, not summarised away
    let sstore = writes
        .iter()
        .find(|(f, _)| *f == "update_storage_value_offset_0_t_uint256_to_t_uint256")
        .expect("the helper holding the sstore");
    let Op::Effect { call } = &sstore.1.op else { panic!() };
    assert!(call.render().starts_with("sstore("), "{}", call.render());

    // the getter reads but never writes
    let getter = ir.function("getter_fun_limit_3").unwrap();
    assert!(getter.effects.reads_storage && !getter.effects.writes_storage);
}

#[test]
fn whole_slot_writes_resolve_to_slot_and_parameter() {
    let ir = ir();
    // The abstraction needs "slot 0 receives the argument", not solc's
    // mask-and-merge over temporaries.
    let mut got: Vec<(String, String, String)> = ir
        .recognised_storage_writes()
        .into_iter()
        .filter(|(f, _, _)| f.starts_with("fun_"))
        .map(|(f, _, w)| (f.to_string(), w.slot_text.clone(), w.value_text.clone()))
        .collect();
    got.sort();
    assert_eq!(
        got,
        vec![
            ("fun_forceSet_37".to_string(), "0x00".to_string(), "var_x_29".to_string()),
            ("fun_setLimit_27".to_string(), "0x00".to_string(), "var_x_5".to_string()),
        ]
    );
    // the value named is each function's own parameter
    for (f, _, w) in ir.recognised_storage_writes() {
        let Some(func) = ir.function(f) else { continue };
        if func.kind != FunctionKind::Body {
            continue;
        }
        assert!(func.parameters.contains(&w.value_text), "{} writes {}", f, w.value_text);
    }
}

#[test]
fn revert_and_return_paths_are_terminators() {
    let ir = ir();
    let f = ir.function("external_fun_setLimit_27").unwrap();
    assert_eq!(f.kind, FunctionKind::External);
    // the non-payable guard sends one side to a block that reverts
    let branch = f.blocks.iter().find_map(|b| match &b.terminator {
        Terminator::Branch { then_block, else_block, .. } => Some((*then_block, *else_block)),
        _ => None,
    });
    let (then_b, else_b) = branch.expect("a branch for the callvalue guard");
    assert!(f.block(then_b).always_reverts, "callvalue() non-zero must revert");
    assert!(!f.block(else_b).always_reverts);
    assert!(f.blocks.iter().any(|b| matches!(b.terminator, Terminator::Return { .. })));

    // require_helper reverts on every path once entered
    let helper = ir
        .functions
        .iter()
        .find(|f| f.id.starts_with("require_helper_"))
        .expect("a require helper");
    assert!(helper.effects.can_revert);
}

#[test]
fn entrypoints_pair_selectors_with_abi_signatures() {
    let ir = ir();
    let mut got: Vec<(&str, &str)> =
        ir.entrypoints.iter().map(|e| (e.selector.as_str(), e.signature.as_str())).collect();
    got.sort();
    assert_eq!(
        got,
        vec![
            ("0x27ea6f2b", "setLimit(uint256)"),
            ("0x81a9dc5e", "forceSet(uint256)"),
            ("0xa4d66daf", "limit()"),
        ]
    );
}

#[test]
fn compiler_guards_are_separated_from_source_requires() {
    let ir = ir();
    // Only the two require() calls get letters; generated guards keep an id
    // tied to where they sit, so adding a require does not renumber them.
    let lettered: Vec<&str> = ir
        .checks
        .iter()
        .filter(|c| c.origin == CheckOrigin::Require)
        .map(|c| c.id.as_str())
        .collect();
    assert_eq!(lettered, vec!["A", "B"]);
    assert!(ir.checks.iter().filter(|c| c.origin != CheckOrigin::Require).all(|c| c.id.starts_with("gen:")));

    // The non-payable guard is found, and it is not pure: it reads the
    // environment, so P1a may not use it as a predicate over arguments.
    let nonpayable = ir
        .checks
        .iter()
        .find(|c| c.function == "external_fun_setLimit_27" && c.condition_text == "iszero(callvalue())")
        .expect("the non-payable guard");
    assert_eq!(nonpayable.purity, Purity::ReadsEnvironment);
    assert!(!ir.pure_checks().iter().any(|c| c.id == nonpayable.id));
}

#[test]
fn a_guard_helper_is_not_also_reported_as_its_own_check() {
    let ir = ir();
    // The `if iszero(condition) { revert }` inside require_helper_* is the
    // same check as the call site; counting both would double report it.
    assert!(
        !ir.checks.iter().any(|c| c.function.starts_with("require_helper_")),
        "guard helper bodies must not yield separate checks"
    );
    assert!(!ir.checks.iter().any(|c| c.function == "validator_revert_t_uint256"));
}

#[test]
fn nothing_in_this_contract_is_outside_the_p1a_subset() {
    let ir = ir();
    assert!(ir.fully_supported(), "unexpected unsupported items: {:?}", ir.unsupported);
    assert!(ir.functions.iter().all(|f| f.effects.supported()));
}

#[test]
fn the_ir_round_trips_through_json() {
    let ir = ir();
    let text = serde_json::to_string(&ir).unwrap();
    let back: ProgramIr = serde_json::from_str(&text).unwrap();
    assert_eq!(back.checks.len(), ir.checks.len());
    assert_eq!(back.check("A").unwrap().condition_text, ir.check("A").unwrap().condition_text);
}
