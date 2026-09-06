//! These need a real solc. They are skipped, loudly, when there is none.

use mulu_solc::{CompileOptions, Solc};

fn solc() -> Option<Solc> {
    match Solc::discover(None) {
        Ok(s) => Some(s),
        Err(_) => {
            eprintln!("skipped: no solc on PATH (set MULU_SOLC to run this test)");
            None
        }
    }
}

const LIMITS: &str = include_str!("../../../examples/limits/Limits.sol");

#[test]
fn compiles_limits_and_returns_the_artifacts_the_ir_needs() {
    let Some(solc) = solc() else { return };
    let b = solc
        .compile(&[("Limits.sol".into(), LIMITS.into())], &CompileOptions::default())
        .expect("Limits.sol must compile");

    assert!(b.compiler.starts_with("0.8."), "unexpected compiler {}", b.compiler);
    assert_eq!(b.sources.len(), 1);
    assert_eq!(b.sources[0].id, 0, "the Yul @src annotations refer to this id");
    assert_eq!(b.sources[0].content, LIMITS);

    let c = b.contract("Limits").expect("the Limits contract");
    assert!(c.ir.contains("object \"Limits"), "the unoptimized Yul must be present");
    assert!(c.storage_layout["storage"][0]["label"] == "limit");
    let mut eps = c.entrypoints();
    eps.sort();
    assert_eq!(eps, vec!["forceSet(uint256)", "limit()", "setLimit(uint256)"]);
    assert!(c.bytecode.is_none(), "bytecode is not requested by default");
}

#[test]
fn a_compile_error_is_an_error_even_when_solc_exits_zero() {
    let Some(solc) = solc() else { return };
    let bad = "pragma solidity ^0.8.0; contract B { function f() external { undefined_thing(); } }";
    let err = solc
        .compile(&[("B.sol".into(), bad.into())], &CompileOptions::default())
        .expect_err("an undeclared identifier must fail the build");
    let text = err.to_string();
    assert!(text.contains("compilation failed"), "{text}");
    assert!(text.contains("undefined_thing") || text.contains("Undeclared"), "{text}");
}

#[test]
fn warnings_do_not_fail_the_build() {
    let Some(solc) = solc() else { return };
    // Limits.sol has no SPDX identifier, which solc warns about.
    let b = solc
        .compile(&[("Limits.sol".into(), LIMITS.into())], &CompileOptions::default())
        .unwrap();
    assert!(!b.warnings.is_empty(), "expected the SPDX warning to be kept");
}

#[test]
fn the_input_hash_changes_with_the_settings() {
    let Some(solc) = solc() else { return };
    let src = [("Limits.sol".to_string(), LIMITS.to_string())];
    let a = solc.compile(&src, &CompileOptions::default()).unwrap();
    let b = solc
        .compile(&src, &CompileOptions { evm_version: "shanghai".into(), ..Default::default() })
        .unwrap();
    assert_ne!(
        a.input_sha256, b.input_sha256,
        "a different evmVersion must produce a different input hash"
    );
}
