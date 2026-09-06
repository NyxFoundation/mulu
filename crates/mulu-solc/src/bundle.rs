//! What a compilation produced, with the hashes that pin it.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

/// One input source. `id` is the file id solc assigns, which is what the
/// `@use-src` / `@src` annotations in the generated Yul refer to.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceFile {
    pub id: u32,
    pub path: String,
    pub content: String,
    pub sha256: String,
}

impl SourceFile {
    pub fn new(id: u32, path: impl Into<String>, content: impl Into<String>) -> Self {
        let content = content.into();
        let sha256 = sha256_hex(content.as_bytes());
        Self { id, path: path.into(), content, sha256 }
    }

    /// Byte offset to a UTF-16 code-unit offset, for LSP positions
    /// (docs/09 §2.1). Returns `None` if the offset is not a char boundary.
    pub fn utf16_offset(&self, byte_offset: usize) -> Option<usize> {
        if byte_offset > self.content.len() || !self.content.is_char_boundary(byte_offset) {
            return None;
        }
        Some(self.content[..byte_offset].encode_utf16().count())
    }

    /// 1-based line and 0-based UTF-16 column, for human-readable output.
    pub fn line_col(&self, byte_offset: usize) -> Option<(usize, usize)> {
        if byte_offset > self.content.len() || !self.content.is_char_boundary(byte_offset) {
            return None;
        }
        let head = &self.content[..byte_offset];
        let line = head.matches('\n').count() + 1;
        let line_start = head.rfind('\n').map(|i| i + 1).unwrap_or(0);
        Some((line, self.content[line_start..byte_offset].encode_utf16().count()))
    }
}

/// Per-contract outputs. `ir` is the **unoptimized** Yul: the artifact the
/// analysis actually reads. `bytecode` is kept only for reproduction and is
/// never the subject of a claim at this stage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractArtifact {
    pub name: String,
    pub source_path: String,
    pub abi: serde_json::Value,
    pub storage_layout: serde_json::Value,
    pub ir: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytecode: Option<String>,
}

impl ContractArtifact {
    /// `(name, selector)` for every externally callable entry in the ABI,
    /// compiler-generated getters included.
    pub fn entrypoints(&self) -> Vec<String> {
        self.abi
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter(|i| i["type"] == "function")
                    .filter_map(|i| {
                        let name = i["name"].as_str()?;
                        let args: Vec<&str> =
                            i["inputs"].as_array()?.iter().filter_map(|a| a["type"].as_str()).collect();
                        Some(format!("{name}({})", args.join(",")))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildBundle {
    /// Where each contract, function and modifier sits, from the AST. This is
    /// the only place that says a `require` came from a modifier.
    #[serde(default)]
    pub ast_index: crate::ast::AstIndex,
    /// Full version string, e.g. `0.8.28+commit.7893614a.Linux.g++`.
    pub compiler: String,
    /// The exact `settings` object sent to solc.
    pub settings: serde_json::Value,
    /// sha256 of the canonical Standard JSON input.
    pub input_sha256: String,
    pub sources: Vec<SourceFile>,
    /// Per-source AST, keyed by source path.
    pub ast: BTreeMap<String, serde_json::Value>,
    pub contracts: Vec<ContractArtifact>,
    /// Warnings solc reported. Errors are never stored here: they abort.
    pub warnings: Vec<String>,
    /// Imports that needed a remapping and were not followed.
    #[serde(default)]
    pub unresolved_imports: Vec<String>,
    /// Contracts solc produced no code for: abstract contracts, interfaces and
    /// libraries. They are not analysis targets, but naming one should say why.
    #[serde(default)]
    pub codeless_contracts: Vec<String>,
}

impl BuildBundle {
    pub fn source_by_id(&self, id: u32) -> Option<&SourceFile> {
        self.sources.iter().find(|s| s.id == id)
    }
    pub fn contract(&self, name: &str) -> Option<&ContractArtifact> {
        self.contracts.iter().find(|c| c.name == name)
    }
    pub fn contract_names(&self) -> Vec<&str> {
        self.contracts.iter().map(|c| c.name.as_str()).collect()
    }
}
