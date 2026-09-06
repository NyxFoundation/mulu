//! Diagnostics (docs/09 §5) and the human-readable summary.

use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Serialize)]
pub struct Evidence {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub path: Option<String>,
    pub checked: bool,
    pub kernel_checked: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Diagnostic {
    pub id: String,
    pub kind: &'static str,
    pub claim: &'static str,
    /// proven / reproduced / candidate / unknown / not-requested
    pub status: &'static str,
    /// abstract-model / yul-semantics / solidity-source / evm-bytecode
    pub scope: &'static str,
    pub severity: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub check_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<String>,
    pub assumptions: Vec<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<Evidence>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<Value>,
}

pub const MODEL_ASSUMPTIONS: &[&str] = &[
    "finite-model-is-the-object", // claims are about model.json, not a source program
    "normalisation-by-mulu-model", // name→index mapping trusted (recorded in manifest)
];

pub fn print_human(diags: &[Diagnostic], out_dir: &std::path::Path) {
    for d in diags {
        println!("{:<8} {}", d.severity, d.id);
        for line in d.message.lines() {
            println!("         {line}");
        }
        let mut meta = format!("claim: {}  status: {}  scope: {}", d.claim, d.status, d.scope);
        if !d.depends_on.is_empty() {
            meta.push_str(&format!("  depends on: {}", d.depends_on.join(", ")));
        }
        println!("         {meta}");
        if let Some(e) = &d.evidence {
            let k = match e.kernel_checked {
                Some(true) => ", kernel-checked",
                Some(false) => ", kernel check FAILED",
                None => "",
            };
            println!(
                "         evidence: {}{}{}",
                e.path.as_deref().unwrap_or(e.kind),
                if e.checked { " (checked" } else { " (NOT checked" },
                format!("{k})")
            );
        }
        println!();
    }
    println!("artifacts: {}", out_dir.display());
}
