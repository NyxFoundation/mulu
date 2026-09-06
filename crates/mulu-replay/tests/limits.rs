//! Deploy the real contract and reproduce the counterexample.
//!
//! docs/09 §7 gives P1-03 its condition: `forceSet(1001)` must be reproduced,
//! and with no specification the tool must not assert a hole. The first half
//! is here; the second is in the CLI, which never builds a call sequence when
//! there is no violation to reproduce.

use mulu_replay::{replay, Call};
use revm::primitives::U256;

/// solc's creation bytecode for examples/limits/Limits.sol.
fn creation() -> Vec<u8> {
    let hex = include_str!("fixtures/Limits.bin").trim();
    (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect()
}

fn call(sig: &str, arg: Option<&str>) -> Call {
    Call { signature: sig.into(), argument: arg.map(|s| s.to_string()) }
}

#[test]
fn forceset_1001_really_breaks_the_bound() {
    let r = replay(
        &creation(),
        &[call("forceSet(uint256)", Some("1001"))],
        &[U256::ZERO],
    )
    .expect("the contract must deploy and run");

    assert_eq!(r.calls.len(), 1);
    assert!(r.calls[0].success, "forceSet has no guard, so it succeeds");
    assert!(r.calls[0].gas_used > 0);
    // and the storage the specification is about now holds 1001
    assert_eq!(r.storage.get("0").map(String::as_str), Some("1001"));
    assert!(r.address.starts_with("0x"));
}

#[test]
fn setlimit_rejects_the_same_value_with_its_reason() {
    let r = replay(
        &creation(),
        &[call("setLimit(uint256)", Some("1001"))],
        &[U256::ZERO],
    )
    .unwrap();
    assert!(!r.calls[0].success, "the guard rejects it");
    assert_eq!(r.calls[0].revert_reason.as_deref(), Some("cap"));
    // a reverted transaction leaves storage as it was
    assert_eq!(r.storage.get("0").map(String::as_str), Some("0"));
}

#[test]
fn the_region_between_the_bounds_is_rejected_too() {
    // 500 is what the overrestriction candidate is about: the specification
    // permits it, and the contract still rejects it.
    let r = replay(&creation(), &[call("setLimit(uint256)", Some("500"))], &[U256::ZERO]).unwrap();
    assert!(!r.calls[0].success);
    assert_eq!(r.calls[0].revert_reason.as_deref(), Some("cap"));
    assert_eq!(r.storage.get("0").map(String::as_str), Some("0"));

    // while forceSet accepts it and stays inside the bound
    let r = replay(&creation(), &[call("forceSet(uint256)", Some("500"))], &[U256::ZERO]).unwrap();
    assert!(r.calls[0].success);
    assert_eq!(r.storage.get("0").map(String::as_str), Some("500"));
}

#[test]
fn a_sequence_runs_in_order_and_the_last_write_stands() {
    let r = replay(
        &creation(),
        &[
            call("setLimit(uint256)", Some("50")),
            call("forceSet(uint256)", Some("2000")),
            call("setLimit(uint256)", Some("2000")),
        ],
        &[U256::ZERO],
    )
    .unwrap();
    assert_eq!(
        r.calls.iter().map(|c| c.success).collect::<Vec<_>>(),
        vec![true, true, false],
        "the guarded call rejects 2000, the unguarded one does not"
    );
    assert_eq!(r.storage.get("0").map(String::as_str), Some("2000"));
}

#[test]
fn the_replay_is_deterministic() {
    let once = replay(&creation(), &[call("forceSet(uint256)", Some("7"))], &[U256::ZERO]).unwrap();
    let twice = replay(&creation(), &[call("forceSet(uint256)", Some("7"))], &[U256::ZERO]).unwrap();
    assert_eq!(once.address, twice.address);
    assert_eq!(once.storage, twice.storage);
    assert_eq!(once.calls[0].gas_used, twice.calls[0].gas_used);
}

#[test]
fn a_violation_a_later_call_repairs_is_still_visible() {
    // The specification is evaluated at every successful transaction end, so
    // a run that breaks the bound and then restores it has still broken it.
    // Reading only the final storage would report nothing.
    let r = replay(
        &creation(),
        &[
            call("forceSet(uint256)", Some("2000")),
            call("forceSet(uint256)", Some("10")),
        ],
        &[U256::ZERO],
    )
    .unwrap();
    assert!(r.calls.iter().all(|c| c.success));
    assert_eq!(r.calls[0].storage.get("0").map(String::as_str), Some("2000"));
    assert_eq!(r.calls[1].storage.get("0").map(String::as_str), Some("10"));
    // the final state is innocent, the run is not
    assert_eq!(r.storage.get("0").map(String::as_str), Some("10"));
}

#[test]
fn a_reverted_call_snapshots_the_restored_state() {
    let r = replay(
        &creation(),
        &[call("forceSet(uint256)", Some("42")), call("setLimit(uint256)", Some("9999"))],
        &[U256::ZERO],
    )
    .unwrap();
    assert!(r.calls[0].success && !r.calls[1].success);
    // the rejected transaction leaves the earlier value in place
    assert_eq!(r.calls[1].storage.get("0").map(String::as_str), Some("42"));
}
