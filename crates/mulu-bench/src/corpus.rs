//! Reading solc's own semantic tests as a corpus.
//!
//! `test/libsolidity/semanticTests/` is about 1700 contracts, each with the
//! calls to make and the results to expect written in a footer that solc's
//! test runner parses. It is the largest body of small Solidity whose meaning
//! someone else has already pinned, which makes it the right thing to measure
//! against: mulu does not get to choose the programs.
//!
//! ```text
//! // ----
//! // div(uint256,uint256): 7, 2 -> 3
//! // div(uint256,uint256): 7, 0 -> FAILURE, hex"4e487b71", 0x12
//! ```
//!
//! The corpus is not vendored. solc is GPL-3.0 and mulu is MIT, so the tests
//! are fetched at measurement time and read as data.

use serde::Serialize;
use std::path::{Path, PathBuf};

/// The EVM version the measurement runs at, matching mulu's default.
pub const MEASURED_ON: &str = "cancun";

/// Solidity's EVM versions, oldest first. A test's `EVMVersion:` directive is
/// a constraint on this order.
const EVM_VERSIONS: &[&str] = &[
    "homestead",
    "tangerineWhistle",
    "spuriousDragon",
    "byzantium",
    "constantinople",
    "petersburg",
    "istanbul",
    "berlin",
    "london",
    "paris",
    "shanghai",
    "cancun",
    "prague",
    "osaka",
];

/// Does `spec` (`>=constantinople`, `<london`, `=paris`) admit `have`?
///
/// Most of the corpus's directives are lower bounds that any recent EVM
/// satisfies. Treating every directive as out of scope threw away a fifth of
/// the corpus for nothing.
pub fn evm_version_allows(spec: &str, have: &str) -> bool {
    let idx = |v: &str| EVM_VERSIONS.iter().position(|x| *x == v);
    let Some(h) = idx(have) else { return false };
    let spec = spec.trim();
    let (op, name) = if let Some(r) = spec.strip_prefix(">=") {
        (">=", r)
    } else if let Some(r) = spec.strip_prefix("<=") {
        ("<=", r)
    } else if let Some(r) = spec.strip_prefix('>') {
        (">", r)
    } else if let Some(r) = spec.strip_prefix('<') {
        ("<", r)
    } else if let Some(r) = spec.strip_prefix('=') {
        ("=", r)
    } else {
        ("=", spec)
    };
    let Some(n) = idx(name.trim()) else { return false };
    match op {
        ">=" => h >= n,
        "<=" => h <= n,
        ">" => h > n,
        "<" => h < n,
        _ => h == n,
    }
}

/// One expected call from a test's footer.
#[derive(Debug, Clone, Serialize)]
pub struct Expectation {
    /// `div(uint256,uint256)`.
    pub signature: String,
    /// Everything between `:` and `->`, verbatim. Not parsed into values:
    /// the encoding solc's runner uses is its own, and guessing at it would
    /// make a different call.
    pub args: String,
    /// Everything after `->`.
    pub result: String,
    /// The call is expected to revert.
    pub reverts: bool,
}

/// Which corpus a case came from. The two ask different questions, and the
/// rules for "out of scope" differ with the question.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    /// solc's `semanticTests`: how much Solidity can mulu read?
    SemanticTests,
    /// `contracts-verification-benchmark`: contracts with a published answer
    /// key. There is no call footer here; the expectations live in a
    /// `ground-truth.csv` beside the sources.
    VerificationBenchmark,
}

#[derive(Debug, Clone, Serialize)]
pub struct Case {
    pub kind: Kind,
    pub path: PathBuf,
    /// Path relative to the corpus root, which is what a report names.
    pub name: String,
    pub source: String,
    pub expectations: Vec<Expectation>,
    /// `// EVMVersion: >=cancun` and friends, verbatim.
    pub directives: Vec<String>,
    /// `(name, content)`. A test may hold several sources, split by
    /// `==== Source: NAME ====`; writing them as one file made solc reject
    /// the separator and counted it against mulu.
    pub sources: Vec<(String, String)>,
}

impl Case {
    /// A test mulu cannot be measured on for a reason that is the corpus's,
    /// not mulu's: it needs a linked library, a specific EVM version, or the
    /// via-IR pipeline. Counting these as failures would flatter or damn the
    /// tool for something it was never asked to do.
    pub fn out_of_scope(&self) -> Option<&'static str> {
        for d in &self.directives {
            if d.starts_with("library:") {
                return Some("needs a linked library");
            }
            if d.starts_with("compileViaYul:") {
                return Some("pins the via-IR pipeline");
            }
            if let Some(spec) = d.strip_prefix("EVMVersion:") {
                if !evm_version_allows(spec.trim(), MEASURED_ON) {
                    return Some("pins an EVM version this run is not");
                }
            }
        }
        // A semantic test with no calls pins nothing, so measuring against
        // it says nothing. The verification benchmark has no call footers at
        // all: what it pins is in `ground-truth.csv`.
        if self.kind == Kind::SemanticTests && self.expectations.is_empty() {
            return Some("no calls to make");
        }
        None
    }
}

fn parse_footer(source: &str) -> (Vec<Expectation>, Vec<String>) {
    let mut calls = vec![];
    let mut directives = vec![];
    let mut in_footer = false;
    for line in source.lines() {
        let t = line.trim();
        if t == "// ----" {
            in_footer = true;
            continue;
        }
        if t.starts_with("// ====") {
            continue;
        }
        let Some(body) = t.strip_prefix("//") else { continue };
        let body = body.trim();
        // Directives appear above the footer as `// EVMVersion: >=paris`.
        if !in_footer {
            if let Some((k, _)) = body.split_once(':') {
                if k.chars().all(|c| c.is_ascii_alphabetic()) && !k.is_empty() {
                    directives.push(body.to_string());
                }
            }
            continue;
        }
        // `~ emit …` is an event expectation attached to the call above it,
        // and `gas …` is a measurement. Neither is a call.
        if body.starts_with('~') || body.starts_with("gas ") || body.is_empty() {
            continue;
        }
        let Some((call, result)) = body.split_once("->") else { continue };
        let result = result.trim();
        let (sig_args, _) = (call.trim(), ());
        let (signature, args) = match sig_args.split_once(':') {
            Some((s, a)) => (s.trim().to_string(), a.trim().to_string()),
            None => (sig_args.to_string(), String::new()),
        };
        if !signature.contains('(') {
            continue;
        }
        calls.push(Expectation {
            signature,
            args,
            reverts: result.starts_with("FAILURE"),
            result: result.to_string(),
        });
    }
    (calls, directives)
}

/// Every `.sol` under `root`, in a stable order so two runs report the same
/// thing in the same place.
pub fn load(root: &Path) -> anyhow::Result<Vec<Case>> {
    let mut files = vec![];
    walk(root, &mut files)?;
    files.sort();
    let mut cases = vec![];
    for path in files {
        let source = std::fs::read_to_string(&path)?;
        let (expectations, directives) = parse_footer(&source);
        let name = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .to_string();
        let sources = split_sources(&source);
        cases.push(Case {
            kind: Kind::SemanticTests,
            path,
            name,
            source,
            expectations,
            directives,
            sources,
        });
    }
    Ok(cases)
}

/// `contracts-verification-benchmark`: `contracts/<use case>/versions/*.sol`,
/// one file per version, with `v1` conforming to the specification and the
/// rest carrying a seeded defect. `ground-truth.csv` beside them says, for
/// each (property, version), whether the property holds.
///
/// The versions import from a shared `lib/` two directories up, so every
/// case carries those sources with it under the path the import names.
pub fn load_verification_benchmark(root: &Path) -> anyhow::Result<Vec<Case>> {
    let lib_dir = root.join("lib");
    let mut lib: Vec<(String, String)> = vec![];
    if lib_dir.is_dir() {
        let mut names: Vec<PathBuf> = std::fs::read_dir(&lib_dir)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "sol"))
            .collect();
        names.sort();
        for p in names {
            let name = p.file_name().unwrap_or_default().to_string_lossy().to_string();
            lib.push((format!("lib/{name}"), std::fs::read_to_string(&p)?));
        }
    }

    let contracts = root.join("contracts");
    let mut use_cases: Vec<PathBuf> = match std::fs::read_dir(&contracts) {
        Ok(d) => d.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.is_dir()).collect(),
        Err(e) => return Err(anyhow::anyhow!("reading {}: {e}", contracts.display())),
    };
    use_cases.sort();

    let mut cases = vec![];
    for uc in use_cases {
        let versions = uc.join("versions");
        if !versions.is_dir() {
            continue;
        }
        let mut files: Vec<PathBuf> = std::fs::read_dir(&versions)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "sol"))
            .collect();
        files.sort();
        let use_case = uc.file_name().unwrap_or_default().to_string_lossy().to_string();
        for path in files {
            let source = std::fs::read_to_string(&path)?;
            let file = path.file_name().unwrap_or_default().to_string_lossy().to_string();
            let mut sources = vec![(file.clone(), source.clone())];
            sources.extend(lib.iter().cloned());
            cases.push(Case {
                kind: Kind::VerificationBenchmark,
                name: format!("{use_case}/{file}"),
                path,
                source,
                expectations: vec![],
                directives: vec![],
                sources,
            });
        }
    }
    Ok(cases)
}

/// `==== Source: A ====` splits a test into several files. Without a
/// separator the whole test is one source named `C.sol`.
pub fn split_sources(source: &str) -> Vec<(String, String)> {
    if !source.contains("==== Source:") {
        return vec![("C.sol".to_string(), source.to_string())];
    }
    let mut out: Vec<(String, String)> = vec![];
    let mut current: Option<(String, String)> = None;
    for line in source.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("==== Source:") {
            if let Some(c) = current.take() {
                out.push(c);
            }
            let name = rest.trim_end_matches('=').trim().to_string();
            // solc keys sources by the name as written; a bare name gets the
            // extension so an import of `A` still resolves.
            let name = if name.contains('.') { name } else { format!("{name}.sol") };
            current = Some((name, String::new()));
            continue;
        }
        if t == "// ----" {
            break;
        }
        if let Some((_, body)) = current.as_mut() {
            body.push_str(line);
            body.push('\n');
        }
    }
    if let Some(c) = current.take() {
        out.push(c);
    }
    if out.is_empty() {
        vec![("C.sol".to_string(), source.to_string())]
    } else {
        out
    }
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> anyhow::Result<()> {
    for e in std::fs::read_dir(dir)? {
        let p = e?.path();
        if p.is_dir() {
            walk(&p, out)?;
        } else if p.extension().is_some_and(|x| x == "sol") {
            out.push(p);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_footer_becomes_the_calls_it_names() {
        let (calls, _) = parse_footer(
            r#"contract C { }
// ----
// div(uint256,uint256): 7, 2 -> 3
// div(uint256,uint256): 7, 0 -> FAILURE, hex"4e487b71", 0x12 # throws #
// f() -> 1
"#,
        );
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[0].signature, "div(uint256,uint256)");
        assert_eq!(calls[0].args, "7, 2");
        assert_eq!(calls[0].result, "3");
        assert!(!calls[0].reverts);
        assert!(calls[1].reverts);
        assert_eq!(calls[2].args, "", "a call with no arguments has none");
    }

    #[test]
    fn an_event_line_and_a_gas_line_are_not_calls() {
        let (calls, _) = parse_footer(
            r#"// ----
// transfer(address,uint256): 2, 5 -> true
// ~ emit Transfer(address,address,uint256): #0x12, #0x02, 0x05
// gas irOptimized: 51054
"#,
        );
        assert_eq!(calls.len(), 1, "only the call is a call");
    }

    #[test]
    fn a_directive_is_read_and_puts_the_case_out_of_scope() {
        let (_, d) = parse_footer("// EVMVersion: >=cancun\ncontract C {}\n// ----\n// f() -> 1\n");
        assert_eq!(d, vec!["EVMVersion: >=cancun"]);
    }

    #[test]
    fn a_multi_source_test_becomes_several_files() {
        let v = split_sources(
            "==== Source: A ====\ncontract A {}\n==== Source: B ====\nimport \"A\";\ncontract B {}\n// ----\n// f() -> 1\n",
        );
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].0, "A.sol", "a bare name gets an extension so an import resolves");
        assert!(v[0].1.contains("contract A"));
        assert!(v[1].1.contains("contract B"));
        assert!(!v[1].1.contains("// ----"), "the footer is not source");
        // and a test with no separator is still one source
        assert_eq!(split_sources("contract C {}").len(), 1);
    }

    #[test]
    fn a_version_bound_is_compared_rather_than_refused() {
        // Most of the corpus's directives are lower bounds any recent EVM
        // meets. Refusing all of them threw away a fifth of it for nothing.
        assert!(evm_version_allows(">=constantinople", "cancun"));
        assert!(evm_version_allows(">=cancun", "cancun"));
        assert!(!evm_version_allows(">=prague", "cancun"));
        assert!(!evm_version_allows("<london", "cancun"));
        assert!(evm_version_allows("<=cancun", "cancun"));
        assert!(evm_version_allows("=cancun", "cancun"));
        assert!(!evm_version_allows("=paris", "cancun"));
        assert!(!evm_version_allows(">=nonesuch", "cancun"), "an unknown name is not admitted");
    }
}
