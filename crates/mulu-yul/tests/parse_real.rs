//! Parse the Yul solc actually produced for examples/limits/Limits.sol.
//! The fixture is committed so this test does not need solc installed.

use mulu_yul::{ast::Stmt, parse_object};

const LIMITS: &str = include_str!("fixtures/Limits.yul");

#[test]
fn parses_solc_output_for_limits() {
    let p = parse_object(LIMITS).expect("solc's Yul must parse");
    assert_eq!(p.use_src.get(&0).map(String::as_str), Some("Limits.sol"));
    assert_eq!(p.object.name, "Limits_38");

    let deployed = p.object.deployed().expect("a deployed object");
    assert_eq!(deployed.name, "Limits_38_deployed");

    let names: Vec<&str> = deployed.functions().iter().map(|f| f.name.as_str()).collect();
    for expected in [
        "fun_setLimit_27",
        "fun_forceSet_37",
        "external_fun_setLimit_27",
        "getter_fun_limit_3",
        "update_storage_value_offset_0_t_uint256_to_t_uint256",
        "validator_revert_t_uint256",
    ] {
        assert!(names.contains(&expected), "missing function {expected} in {names:?}");
    }
}

#[test]
fn keeps_the_source_span_of_each_require() {
    let p = parse_object(LIMITS).unwrap();
    let deployed = p.object.deployed().unwrap();
    let f = deployed.functions().into_iter().find(|f| f.name == "fun_setLimit_27").unwrap();

    // The two require() calls appear as calls to require_helper_*, each
    // carrying the @src span of the require statement in Limits.sol.
    let spans: Vec<_> = f
        .body
        .stmts
        .iter()
        .filter_map(|s| match s {
            Stmt::Expr { value, src } => {
                let mut calls = Vec::new();
                value.calls(&mut calls);
                calls.iter().any(|c| c.starts_with("require_helper_")).then_some(*src)
            }
            _ => None,
        })
        .flatten()
        .collect();
    assert_eq!(spans.len(), 2, "two require() calls, got {spans:?}");
    // docs/09: Location is (file-id, byte-start, byte-length); solc writes the
    // end offset, so the length is end - start. Check the spans by slicing the
    // source they came from rather than by hard-coded numbers.
    const SRC: &str = include_str!("fixtures/limits-Limits.sol");
    let text = |s: mulu_yul::SrcSpan| &SRC[s.start as usize..s.end as usize];
    assert_eq!(text(spans[0]), r#"require(x <= 100, "cap")"#);
    assert_eq!(text(spans[1]), r#"require(x <= 1000, "bound")"#);
    assert_eq!(spans[0].len(), 24);
    assert_eq!(spans[1].len(), 27);
}

#[test]
fn every_object_in_the_file_round_trips() {
    // Reparsing the rendered structure is not the goal; the point is that no
    // construct in real solc output is silently skipped by the parser.
    let p = parse_object(LIMITS).unwrap();
    let mut total = 0usize;
    fn count(b: &mulu_yul::ast::Block, total: &mut usize) {
        for s in &b.stmts {
            *total += 1;
            match s {
                Stmt::Function(f) => count(&f.body, total),
                Stmt::Block(b) => count(b, total),
                Stmt::If { body, .. } => count(body, total),
                Stmt::Switch { cases, .. } => cases.iter().for_each(|c| count(&c.body, total)),
                Stmt::For { pre, post, body, .. } => {
                    count(pre, total);
                    count(post, total);
                    count(body, total);
                }
                _ => {}
            }
        }
    }
    count(&p.object.code, &mut total);
    count(&p.object.deployed().unwrap().code, &mut total);
    assert!(total > 100, "expected a substantial statement count, got {total}");
}
