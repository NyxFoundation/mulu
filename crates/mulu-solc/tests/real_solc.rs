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

#[test]
fn imports_are_followed_from_the_entry_file() {
    let Some(solc) = solc() else { return };
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let b = solc
        .compile_files(
            &root.join("examples/access"),
            &[root.join("examples/access/Vault.sol")],
            &CompileOptions::default(),
        )
        .expect("Vault.sol imports Base.sol and must compile from the entry file alone");

    // both files are in the bundle, hashed, with the ids the Yul refers to
    let mut paths: Vec<&str> = b.sources.iter().map(|s| s.path.as_str()).collect();
    paths.sort();
    assert_eq!(paths, vec!["Base.sol", "Vault.sol"]);
    assert!(b.sources.iter().all(|s| s.sha256.len() == 64));
    assert!(b.unresolved_imports.is_empty());

    // an abstract contract has no code and is not an analysis target
    assert_eq!(b.contract_names(), vec!["Vault"]);
    assert_eq!(b.codeless_contracts, vec!["Bounded".to_string()]);
}

#[test]
fn selecting_an_abstract_contract_says_why_it_cannot_be_analysed() {
    let Some(solc) = solc() else { return };
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let b = solc
        .compile_files(
            &root.join("examples/access"),
            &[root.join("examples/access/Vault.sol")],
            &CompileOptions::default(),
        )
        .unwrap();
    let err = mulu_solc::driver_select(&b, Some("Bounded")).unwrap_err().to_string();
    assert!(err.contains("abstract"), "{err}");
    // a name that is not there at all reads differently
    let err = mulu_solc::driver_select(&b, Some("Nope")).unwrap_err().to_string();
    assert!(err.contains("no contract named"), "{err}");
}

#[test]
fn the_ast_index_locates_a_modifier_across_files() {
    let Some(solc) = solc() else { return };
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let b = solc
        .compile_files(
            &root.join("examples/access"),
            &[root.join("examples/access/Vault.sol")],
            &CompileOptions::default(),
        )
        .unwrap();
    assert!(!b.ast_index.is_empty());

    // find the `capped` modifier and confirm it is attributed to Bounded
    let m = b
        .ast_index
        .nodes
        .iter()
        .find(|n| n.name == "capped")
        .expect("the capped modifier");
    assert_eq!(m.kind, mulu_solc::AstKind::Modifier);
    assert_eq!(m.contract.as_deref(), Some("Bounded"));
    let base = b.source_by_id(m.file_id).unwrap();
    assert_eq!(base.path, "Base.sol", "the modifier lives in the imported file");
    // a span inside it resolves back to the modifier
    let inside = b.ast_index.modifier_at(m.file_id, m.start + 10, m.start + 20).unwrap();
    assert_eq!(inside.name, "capped");
}
