//! Finding solc, building the Standard JSON input, running it, reading it back.

use crate::bundle::{sha256_hex, BuildBundle, ContractArtifact, SourceFile};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SolcError {
    #[error("solc not found: pass --solc, set MULU_SOLC, or put solc on PATH")]
    NotFound,
    #[error("running solc at {path}: {source}")]
    Spawn { path: String, source: std::io::Error },
    #[error("solc exited with {status} and produced no usable JSON\n{stderr}")]
    Exit { status: String, stderr: String },
    #[error("solc returned invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("compilation failed:\n{0}")]
    Compile(String),
    #[error("no contract named {wanted:?}; this build defines: {available}")]
    NoSuchContract { wanted: String, available: String },
    #[error("{0} is abstract, an interface or a library: solc produced no code for it, so there is nothing to analyse")]
    CodelessContract(String),
    #[error("this build defines no contract with code (only: {0})")]
    NoContractWithCode(String),
    #[error("solc did not emit {what} for {contract}; it was requested in outputSelection")]
    MissingOutput { what: &'static str, contract: String },
    #[error("reading {path}: {source}")]
    Io { path: String, source: std::io::Error },
}

pub struct Solc {
    path: PathBuf,
    version: String,
}

/// Settings we pin. The optimizer is off because the analysed artifact is the
/// unoptimized Yul; turning it on would silently change what is being read
/// (docs/09 §8, S2).
#[derive(Debug, Clone)]
pub struct CompileOptions {
    pub evm_version: String,
    pub optimizer: bool,
    pub via_ir: bool,
    pub include_bytecode: bool,
}

impl Default for CompileOptions {
    fn default() -> Self {
        Self {
            evm_version: "cancun".into(),
            optimizer: false,
            via_ir: false,
            include_bytecode: false,
        }
    }
}

fn parse_version(stdout: &str) -> Option<String> {
    stdout
        .lines()
        .find_map(|l| l.trim().strip_prefix("Version:"))
        .map(|v| v.trim().to_string())
}

impl Solc {
    /// `explicit` beats `MULU_SOLC`, which beats `solc` on PATH.
    pub fn discover(explicit: Option<PathBuf>) -> Result<Self, SolcError> {
        let path = explicit
            .or_else(|| std::env::var_os("MULU_SOLC").map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("solc"));
        let out = Command::new(&path)
            .arg("--version")
            .output()
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => SolcError::NotFound,
                _ => SolcError::Spawn { path: path.display().to_string(), source: e },
            })?;
        let stdout = String::from_utf8_lossy(&out.stdout);
        let version = parse_version(&stdout).ok_or_else(|| SolcError::Exit {
            status: out.status.to_string(),
            stderr: format!("could not read a version from:\n{stdout}"),
        })?;
        Ok(Self { path, version })
    }

    pub fn version(&self) -> &str {
        &self.version
    }
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn settings(&self, opts: &CompileOptions) -> Value {
        // `evm.methodIdentifiers` is solc's own selector table. Deriving the
        // pairing from the dispatcher and the ABI instead gets overloaded
        // functions wrong, and the argument type follows from the pairing.
        let mut outputs = vec!["abi", "storageLayout", "ir", "evm.methodIdentifiers", "metadata"];
        if opts.include_bytecode {
            outputs.push("evm.bytecode.object");
        }
        json!({
            "optimizer": {"enabled": opts.optimizer},
            "evmVersion": opts.evm_version,
            "viaIR": opts.via_ir,
            "outputSelection": {"*": {"*": outputs, "": ["ast"]}}
        })
    }

    /// Compile in-memory sources. `sources` is `(path, content)`.
    pub fn compile(
        &self,
        sources: &[(String, String)],
        opts: &CompileOptions,
    ) -> Result<BuildBundle, SolcError> {
        let mut src_map = serde_json::Map::new();
        for (path, content) in sources {
            src_map.insert(path.clone(), json!({"content": content}));
        }
        let settings = self.settings(opts);
        let input = json!({
            "language": "Solidity",
            "sources": Value::Object(src_map),
            "settings": settings.clone(),
        });
        // serde_json::Value sorts object keys, so this text is canonical.
        let input_text = serde_json::to_string(&input)?;
        let input_sha256 = sha256_hex(input_text.as_bytes());

        let mut child = Command::new(&self.path)
            .arg("--standard-json")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| SolcError::Spawn { path: self.path.display().to_string(), source: e })?;
        child.stdin.take().expect("stdin piped").write_all(input_text.as_bytes()).map_err(|e| {
            SolcError::Spawn { path: self.path.display().to_string(), source: e }
        })?;
        let out = child.wait_with_output().map_err(|e| SolcError::Spawn {
            path: self.path.display().to_string(),
            source: e,
        })?;
        let stdout = String::from_utf8_lossy(&out.stdout);
        if stdout.trim().is_empty() {
            return Err(SolcError::Exit {
                status: out.status.to_string(),
                stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            });
        }
        let response: Value = serde_json::from_str(&stdout)?;
        self.into_bundle(response, sources, settings, input_sha256)
    }

    /// Compile files from disk, following their `import` statements. Source
    /// keys are paths relative to `root`, matching the `@use-src` names solc
    /// writes into the generated Yul.
    pub fn compile_files(
        &self,
        root: &Path,
        files: &[PathBuf],
        opts: &CompileOptions,
    ) -> Result<BuildBundle, SolcError> {
        let resolved = crate::imports::resolve(root, files).map_err(|e| SolcError::Io {
            path: files.first().map(|f| f.display().to_string()).unwrap_or_default(),
            source: e,
        })?;
        let sources: Vec<(String, String)> = resolved.sources.into_iter().collect();
        let mut bundle = self.compile(&sources, opts)?;
        bundle.unresolved_imports = resolved.unresolved;
        Ok(bundle)
    }

    fn into_bundle(
        &self,
        response: Value,
        inputs: &[(String, String)],
        settings: Value,
        input_sha256: String,
    ) -> Result<BuildBundle, SolcError> {
        // Errors first: solc can exit 0 and still have failed (docs/09 §5 of 08).
        let mut warnings = Vec::new();
        let mut fatal = Vec::new();
        for e in response["errors"].as_array().cloned().unwrap_or_default() {
            let severity = e["severity"].as_str().unwrap_or("error");
            let text = e["formattedMessage"]
                .as_str()
                .or_else(|| e["message"].as_str())
                .unwrap_or("(no message)")
                .trim_end()
                .to_string();
            if severity == "error" {
                fatal.push(text);
            } else {
                warnings.push(text);
            }
        }
        if !fatal.is_empty() {
            return Err(SolcError::Compile(fatal.join("\n")));
        }

        let mut sources = Vec::new();
        let mut ast = BTreeMap::new();
        for (path, entry) in response["sources"].as_object().cloned().unwrap_or_default() {
            let id = entry["id"].as_u64().unwrap_or(0) as u32;
            let content = inputs
                .iter()
                .find(|(p, _)| *p == path)
                .map(|(_, c)| c.clone())
                .unwrap_or_default();
            sources.push(SourceFile::new(id, path.clone(), content));
            if let Some(a) = entry.get("ast") {
                ast.insert(path, a.clone());
            }
        }
        sources.sort_by_key(|s| s.id);

        let mut contracts = Vec::new();
        let mut codeless = Vec::new();
        for (source_path, per_file) in response["contracts"].as_object().cloned().unwrap_or_default() {
            for (name, c) in per_file.as_object().cloned().unwrap_or_default() {
                let ir = c["ir"].as_str().unwrap_or("").to_string();
                if ir.is_empty() {
                    // An abstract contract, interface or library has no code.
                    // That is not a compilation failure.
                    codeless.push(name);
                    continue;
                }
                let method_identifiers = c["evm"]["methodIdentifiers"]
                    .as_object()
                    .map(|m| {
                        m.iter()
                            .filter_map(|(sig, sel)| {
                                Some((sig.clone(), sel.as_str()?.to_string()))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                contracts.push(ContractArtifact {
                    name,
                    source_path: source_path.clone(),
                    abi: c["abi"].clone(),
                    storage_layout: c["storageLayout"].clone(),
                    method_identifiers,
                    metadata: c["metadata"].as_str().map(|s| s.to_string()),
                    ir,
                    bytecode: c["evm"]["bytecode"]["object"].as_str().map(|s| s.to_string()),
                });
            }
        }
        contracts.sort_by(|a, b| a.name.cmp(&b.name));
        codeless.sort();

        let ast_index = crate::ast::AstIndex::build(&ast);
        Ok(BuildBundle {
            compiler: self.version.clone(),
            settings,
            input_sha256,
            sources,
            ast_index,
            ast,
            contracts,
            warnings,
            unresolved_imports: vec![],
            codeless_contracts: codeless,
        })
    }
}

/// Pick the contract to analyse: the named one, or the only one there is.
pub fn select_contract<'a>(
    bundle: &'a BuildBundle,
    wanted: Option<&str>,
) -> Result<&'a ContractArtifact, SolcError> {
    match wanted {
        Some(n) => bundle.contract(n).ok_or_else(|| {
            if bundle.codeless_contracts.iter().any(|c| c == n) {
                SolcError::CodelessContract(n.to_string())
            } else {
                SolcError::NoSuchContract {
                    wanted: n.to_string(),
                    available: bundle.contract_names().join(", "),
                }
            }
        }),
        None => match bundle.contracts.len() {
            1 => Ok(&bundle.contracts[0]),
            0 => Err(SolcError::NoContractWithCode(bundle.codeless_contracts.join(", "))),
            _ => Err(SolcError::NoSuchContract {
                wanted: "(unspecified)".into(),
                available: bundle.contract_names().join(", "),
            }),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_parsing() {
        let out = "solc, the solidity compiler commandline interface\nVersion: 0.8.28+commit.7893614a.Linux.g++\n";
        assert_eq!(parse_version(out).unwrap(), "0.8.28+commit.7893614a.Linux.g++");
        assert!(parse_version("no version here").is_none());
    }

    #[test]
    fn utf16_and_line_col_on_multibyte_source() {
        let f = SourceFile::new(0, "a.sol", "// 日本語\nrequire(x);\n");
        let nl = f.content.find('\n').unwrap();
        // "// 日本語" is 12 bytes but 6 UTF-16 units: 3 ASCII plus 3 BMP chars
        assert_eq!(nl, 12);
        assert_eq!(f.utf16_offset(nl), Some(6));
        assert_eq!(f.line_col(nl), Some((1, 6)));
        let r = f.content.find("require").unwrap();
        assert_eq!(f.line_col(r), Some((2, 0)));
        // not a char boundary
        assert_eq!(f.utf16_offset(4), None);
    }
}
