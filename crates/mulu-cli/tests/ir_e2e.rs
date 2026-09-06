//! `mulu ir` end to end: real solc, real Yul, real ProgramIR on disk.
//! Skipped when solc is not installed.

use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

fn have_solc() -> bool {
    let p = std::env::var("MULU_SOLC").unwrap_or_else(|_| "solc".into());
    match Command::new(p).arg("--version").output() {
        Ok(o) => o.status.success(),
        Err(_) => {
            eprintln!("skipped: no solc available");
            false
        }
    }
}

#[test]
fn builds_program_ir_for_limits() {
    if !have_solc() {
        return;
    }
    let out = std::env::temp_dir().join(format!("mulu-ir-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&out);
    let status = Command::new(env!("CARGO_BIN_EXE_mulu"))
        .args(["ir", root().join("examples/limits/Limits.sol").to_str().unwrap()])
        .args(["--contract", "Limits", "--out", out.to_str().unwrap()])
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(0), "nothing in Limits is outside the P1a subset");

    let ir: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(out.join("program.json")).unwrap()).unwrap();
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(out.join("manifest.json")).unwrap()).unwrap();

    // the artifact under analysis is named, and it is not the bytecode
    assert!(ir["derived_from"].as_str().unwrap().contains("unoptimized Yul"));
    let p = &manifest["provenance"];
    assert_eq!(manifest["stage"], "ir");
    assert_eq!(p["fully_supported"], true);
    assert!(p["standard_json_input_sha256"].as_str().unwrap().len() == 64);
    assert!(p["compiler"].as_str().unwrap().starts_with("0.8."));

    // the two requires, with conditions resolved to comparisons over x
    let checks = ir["checks"].as_array().unwrap();
    let a = checks.iter().find(|c| c["id"] == "A").expect("check A");
    let b = checks.iter().find(|c| c["id"] == "B").expect("check B");
    assert_eq!(a["condition_text"], "iszero(gt(var_x_5, 0x64))");
    assert_eq!(b["condition_text"], "iszero(gt(var_x_5, 0x03e8))");
    assert_eq!(a["origin"], "require");
    assert_eq!(a["purity"], "pure");

    // located at the require statements in the original source
    let src = std::fs::read_to_string(root().join("examples/limits/Limits.sol")).unwrap();
    for (c, expected) in [(a, r#"require(x <= 100, "cap")"#), (b, r#"require(x <= 1000, "bound")"#)] {
        let s = c["source"]["byte_start"].as_u64().unwrap() as usize;
        let n = c["source"]["byte_length"].as_u64().unwrap() as usize;
        assert_eq!(&src[s..s + n], expected);
    }

    // the build inputs are kept next to the IR
    for f in ["build/Limits.yul", "build/abi.json", "build/storage-layout.json", "build/settings.json"] {
        assert!(out.join(f).exists(), "missing {f}");
    }
    let _ = std::fs::remove_dir_all(&out);
}
