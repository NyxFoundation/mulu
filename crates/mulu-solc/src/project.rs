//! Reading a project's own build (P1b).
//!
//! Foundry and Hardhat both write a **build-info** file for every compilation:
//! the exact Standard JSON input they sent solc and the output they got back.
//! docs/08 §4 says to take that as the entry point rather than write an
//! evaluator for `foundry.toml` or, worse, for `hardhat.config.js`. The
//! settings a project actually built with are a fact recorded in that file;
//! anything mulu inferred instead would be a guess that happened to agree.
//!
//! What mulu does **not** do is analyse the build-info's own output. A normal
//! project build does not request `ir`, so the Yul mulu reads is not in there,
//! and the optimizer is usually on, which is not the artifact any claim here
//! is about. So the build-info supplies the *configuration and the sources*,
//! mulu compiles them again for the artifact it analyses, and every way in
//! which that compilation differs from the project's is reported rather than
//! smoothed over: a different solc, an optimizer that was on, a source file
//! edited since the build.

use crate::bundle::sha256_hex;
use crate::driver::SolcError;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// One `build-info` file, as Foundry and Hardhat write it. The two formats
/// differ only in fields mulu does not need; both carry `input`, `output` and
/// a solc version.
#[derive(Debug, Clone)]
pub struct BuildInfo {
    pub path: PathBuf,
    /// `0.8.26`, as the project recorded it.
    pub solc_version: String,
    /// `0.8.26+commit.8a97fa7a.Linux.g++`, when the file has it.
    pub solc_long_version: Option<String>,
    /// Source path to content, in the order solc was given them.
    pub sources: Vec<(String, String)>,
    /// The project's own `settings` object, verbatim.
    pub settings: Value,
}

/// Where Foundry and Hardhat put their build-info files.
const BUILD_INFO_DIRS: &[&str] = &["out/build-info", "artifacts/build-info"];

/// Build-info files under `root`, newest first. Empty when the project has
/// not been built, which is not an error: it means there is nothing to read.
pub fn discover(root: &Path) -> Vec<PathBuf> {
    let mut found: Vec<(std::time::SystemTime, PathBuf)> = vec![];
    for dir in BUILD_INFO_DIRS {
        let Ok(entries) = std::fs::read_dir(root.join(dir)) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "json") {
                let t = e.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
                found.push((t, p));
            }
        }
    }
    found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    found.into_iter().map(|(_, p)| p).collect()
}

fn err(path: &Path, msg: impl Into<String>) -> SolcError {
    SolcError::Compile(format!("{}: {}", path.display(), msg.into()))
}

/// Parse a build-info file. Rejects one that does not carry the sources: the
/// point of reading it is to compile the same bytes the project compiled, and
/// a `urls` entry names a file that may since have changed.
pub fn read(path: &Path) -> Result<BuildInfo, SolcError> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| SolcError::Io { path: path.display().to_string(), source: e })?;
    let v: Value = serde_json::from_str(&text)?;
    let input = v.get("input").ok_or_else(|| {
        err(path, "no `input` field: this is not a Foundry or Hardhat build-info file")
    })?;
    if let Some(lang) = input["language"].as_str() {
        if lang != "Solidity" {
            return Err(err(path, format!("the build compiled {lang}, and mulu reads Solidity")));
        }
    }
    let solc_version = v["solcVersion"]
        .as_str()
        .or_else(|| v["solc_version"].as_str())
        .ok_or_else(|| err(path, "no `solcVersion`"))?
        .to_string();
    let solc_long_version =
        v["solcLongVersion"].as_str().or_else(|| v["solc_long_version"].as_str()).map(String::from);

    let obj = input["sources"]
        .as_object()
        .ok_or_else(|| err(path, "`input.sources` is not an object"))?;
    let mut sources = vec![];
    let mut without_content = vec![];
    for (p, entry) in obj {
        match entry["content"].as_str() {
            Some(c) => sources.push((p.clone(), c.to_string())),
            None => without_content.push(p.clone()),
        }
    }
    if !without_content.is_empty() {
        return Err(err(
            path,
            format!(
                "{} source(s) are given as `urls` rather than `content`, so the build cannot be \
                 reproduced from this file: {}",
                without_content.len(),
                without_content.join(", ")
            ),
        ));
    }
    if sources.is_empty() {
        return Err(err(path, "the build compiled no sources"));
    }
    sources.sort();
    Ok(BuildInfo {
        path: path.to_path_buf(),
        solc_version,
        solc_long_version,
        sources,
        settings: input["settings"].clone(),
    })
}

impl BuildInfo {
    /// `settings.remappings`, which import statements in the sources are
    /// written against. Without them a re-compilation of the same bytes fails
    /// to resolve imports the project resolved.
    pub fn remappings(&self) -> Vec<String> {
        self.settings["remappings"]
            .as_array()
            .map(|a| a.iter().filter_map(|r| r.as_str().map(String::from)).collect())
            .unwrap_or_default()
    }

    pub fn evm_version(&self) -> Option<String> {
        self.settings["evmVersion"].as_str().map(String::from)
    }

    pub fn optimizer_enabled(&self) -> bool {
        self.settings["optimizer"]["enabled"].as_bool().unwrap_or(false)
    }

    pub fn via_ir(&self) -> bool {
        self.settings["viaIR"].as_bool().unwrap_or(false)
    }

    /// `0.8.26` from `0.8.26+commit.…`, so two spellings of one version compare equal.
    fn short(v: &str) -> &str {
        v.split(['+', '-']).next().unwrap_or(v)
    }

    /// Every way this analysis will differ from the build the project ships.
    /// `root` is the directory the source paths are relative to.
    pub fn drift(&self, local_solc: &str, root: &Path) -> Drift {
        let mut edited = vec![];
        let mut absent = vec![];
        for (p, content) in &self.sources {
            match std::fs::read_to_string(root.join(p)) {
                Ok(on_disk) if on_disk != *content => edited.push(p.clone()),
                Ok(_) => {}
                Err(_) => absent.push(p.clone()),
            }
        }
        let (a, b) = (Self::short(&self.solc_version), Self::short(local_solc));
        Drift {
            solc: (a != b).then(|| (self.solc_version.clone(), local_solc.to_string())),
            optimizer: self.optimizer_enabled(),
            via_ir: self.via_ir(),
            edited,
            absent,
        }
    }

    /// The identity of the build these sources came from, for the manifest.
    pub fn digest(&self) -> String {
        let mut h = String::new();
        for (p, c) in &self.sources {
            h.push_str(p);
            h.push('\u{1}');
            h.push_str(&sha256_hex(c.as_bytes()));
            h.push('\u{2}');
        }
        sha256_hex(h.as_bytes())
    }
}

/// How the analysed compilation differs from the project's own. Empty means
/// mulu compiled the same sources with the same compiler; it never means the
/// analysed artifact *is* the deployed one, which is a separate obligation.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Drift {
    /// `(the project's solc, the one mulu ran)`, when they differ.
    pub solc: Option<(String, String)>,
    /// The project builds with the optimizer on; mulu reads unoptimized Yul.
    pub optimizer: bool,
    /// The project builds through the IR pipeline, which is a different
    /// lowering from the `ir` output mulu reads.
    pub via_ir: bool,
    /// Sources edited since the build.
    pub edited: Vec<String>,
    /// Sources the build recorded that are not on disk now.
    pub absent: Vec<String>,
}

impl Drift {
    pub fn is_empty(&self) -> bool {
        self.solc.is_none()
            && !self.optimizer
            && !self.via_ir
            && self.edited.is_empty()
            && self.absent.is_empty()
    }

    /// One line per difference, for the report and for the obligation that
    /// each of these makes harder to discharge.
    pub fn lines(&self) -> Vec<String> {
        let mut v = vec![];
        if let Some((project, local)) = &self.solc {
            v.push(format!(
                "the project built with solc {project} and mulu ran {local}, so this is not the \
                 compilation the project ships"
            ));
        }
        if self.optimizer {
            v.push(
                "the project builds with the optimizer on; the analysed Yul is unoptimized"
                    .to_string(),
            );
        }
        if self.via_ir {
            v.push(
                "the project builds through the IR pipeline, which lowers differently from the \
                 `ir` output read here"
                    .to_string(),
            );
        }
        for p in &self.edited {
            v.push(format!("{p} has been edited since the build"));
        }
        for p in &self.absent {
            v.push(format!("{p} was in the build and is not on disk now"));
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One file per case: the tests run in parallel and shared one path.
    fn info(name: &str, json: &str) -> Result<BuildInfo, SolcError> {
        let dir = std::env::temp_dir().join(format!("mulu-bi-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(format!("{name}.json"));
        std::fs::write(&p, json).unwrap();
        read(&p)
    }

    #[test]
    fn reads_the_shape_both_tools_write() {
        let bi = info(
            "both",
            r#"{"_format":"hh-sol-build-info-1","id":"x","solcVersion":"0.8.26",
                "solcLongVersion":"0.8.26+commit.8a97fa7a",
                "input":{"language":"Solidity",
                  "sources":{"src/A.sol":{"content":"contract A {}"}},
                  "settings":{"optimizer":{"enabled":true},"evmVersion":"paris",
                              "remappings":["@oz/=lib/oz/"]}},
                "output":{}}"#,
        )
        .unwrap();
        assert_eq!(bi.solc_version, "0.8.26");
        assert_eq!(bi.sources, vec![("src/A.sol".to_string(), "contract A {}".to_string())]);
        assert!(bi.optimizer_enabled());
        assert_eq!(bi.evm_version().as_deref(), Some("paris"));
        assert_eq!(bi.remappings(), vec!["@oz/=lib/oz/"]);
    }

    #[test]
    fn a_build_that_only_names_files_is_refused() {
        // `urls` points at files that may have changed since. Compiling those
        // is not reproducing the build, and saying it is would be the lie.
        let e = info(
            "urls",
            r#"{"solcVersion":"0.8.26","input":{"language":"Solidity",
                "sources":{"src/A.sol":{"urls":["/tmp/A.sol"]}},"settings":{}},"output":{}}"#,
        )
        .unwrap_err();
        assert!(e.to_string().contains("urls"), "{e}");
    }

    #[test]
    fn a_vyper_build_is_refused_by_name() {
        let e = info(
            "vyper",
            r#"{"solcVersion":"0.3.10","input":{"language":"Vyper",
                "sources":{"a.vy":{"content":"x"}},"settings":{}},"output":{}}"#,
        )
        .unwrap_err();
        assert!(e.to_string().contains("Vyper"), "{e}");
    }

    #[test]
    fn a_patch_version_difference_is_drift_and_a_commit_hash_is_not() {
        let bi = info(
            "drift",
            r#"{"solcVersion":"0.8.26","input":{"language":"Solidity",
                "sources":{"a.sol":{"content":"x"}},"settings":{}},"output":{}}"#,
        )
        .unwrap();
        let same = bi.drift("0.8.26+commit.8a97fa7a.Linux.g++", Path::new("/nonexistent"));
        assert!(same.solc.is_none(), "{:?}", same.solc);
        let other = bi.drift("0.8.30", Path::new("/nonexistent"));
        assert!(other.solc.is_some());
        // the source is not on disk there, which is its own difference
        assert_eq!(other.absent, vec!["a.sol"]);
        assert!(!other.is_empty());
    }
}
