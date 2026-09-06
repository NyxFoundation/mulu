//! Following `import` statements so a multi-file contract compiles from its
//! entry file alone.
//!
//! Every file reached is read and hashed into the bundle, so the analysis is
//! pinned to the exact bytes that produced it. Imports that need a remapping
//! (a bare `@scope/...` path) are reported rather than guessed at.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Remove comments so an `import` inside one is not mistaken for a statement.
/// String literals are kept, since that is where the path lives.
pub fn strip_comments(src: &str) -> String {
    let b: Vec<char> = src.chars().collect();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            '/' if i + 1 < b.len() && b[i + 1] == '/' => {
                while i < b.len() && b[i] != '\n' {
                    i += 1;
                }
            }
            '/' if i + 1 < b.len() && b[i + 1] == '*' => {
                i += 2;
                while i + 1 < b.len() && !(b[i] == '*' && b[i + 1] == '/') {
                    i += 1;
                }
                i = (i + 2).min(b.len());
                out.push(' ');
            }
            q @ ('"' | '\'') => {
                out.push(q);
                i += 1;
                while i < b.len() && b[i] != q {
                    if b[i] == '\\' && i + 1 < b.len() {
                        out.push(b[i]);
                        i += 1;
                    }
                    out.push(b[i]);
                    i += 1;
                }
                if i < b.len() {
                    out.push(q);
                    i += 1;
                }
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

/// The paths a source imports, in order.
pub fn imported_paths(src: &str) -> Vec<String> {
    let cleaned = strip_comments(src);
    let mut out = Vec::new();
    let bytes: Vec<char> = cleaned.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        // `import` must be a whole word
        if bytes[i] == 'i' && cleaned[char_byte(&bytes, i)..].starts_with("import") {
            let before_ok = i == 0 || !is_word(bytes[i - 1]);
            let after = i + 6;
            let after_ok = after >= bytes.len() || !is_word(bytes[after]);
            if before_ok && after_ok {
                // take the first string literal before the terminating `;`
                let mut j = after;
                let mut path: Option<String> = None;
                while j < bytes.len() && bytes[j] != ';' {
                    if bytes[j] == '"' || bytes[j] == '\'' {
                        let q = bytes[j];
                        j += 1;
                        let mut s = String::new();
                        while j < bytes.len() && bytes[j] != q {
                            s.push(bytes[j]);
                            j += 1;
                        }
                        path = Some(s);
                        break;
                    }
                    j += 1;
                }
                if let Some(p) = path {
                    if !p.is_empty() {
                        out.push(p);
                    }
                }
                i = after;
                continue;
            }
        }
        i += 1;
    }
    out
}

fn is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '$'
}

fn char_byte(chars: &[char], idx: usize) -> usize {
    chars[..idx].iter().map(|c| c.len_utf8()).sum()
}

/// Normalise `a/./b/../c` without touching the filesystem.
fn normalise(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[derive(Debug)]
pub struct Resolved {
    /// Source key as used in the Standard JSON, to file contents.
    pub sources: BTreeMap<String, String>,
    /// Imports that need a remapping and were not followed.
    pub unresolved: Vec<String>,
}

/// Read `entries` and everything they import, transitively.
///
/// Source keys are paths relative to `root`, so they match the `@use-src`
/// names in the generated Yul.
pub fn resolve(root: &Path, entries: &[PathBuf]) -> std::io::Result<Resolved> {
    let mut sources = BTreeMap::new();
    let mut unresolved = Vec::new();
    let mut seen = BTreeSet::new();
    let mut queue: Vec<PathBuf> = entries.iter().map(|p| normalise(p)).collect();

    while let Some(path) = queue.pop() {
        if !seen.insert(path.clone()) {
            continue;
        }
        let content = std::fs::read_to_string(&path)?;
        let key = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        for imported in imported_paths(&content) {
            if imported.starts_with('.') {
                let base = path.parent().unwrap_or(Path::new("."));
                queue.push(normalise(&base.join(&imported)));
            } else if !unresolved.contains(&imported) {
                // A bare path needs a remapping, which P1 does not read.
                unresolved.push(imported);
            }
        }
        sources.insert(key, content);
    }
    Ok(Resolved { sources, unresolved })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_import_form_yields_its_path() {
        let src = r#"
            pragma solidity ^0.8.0;
            import "./Base.sol";
            import "./Other.sol" as other;
            import * as lib from "../lib/Lib.sol";
            import {A, B as C} from "./Types.sol";
        "#;
        assert_eq!(
            imported_paths(src),
            vec!["./Base.sol", "./Other.sol", "../lib/Lib.sol", "./Types.sol"]
        );
    }

    #[test]
    fn an_import_inside_a_comment_or_a_word_is_not_one() {
        let src = r#"
            // import "./Commented.sol";
            /* import "./Block.sol"; */
            contract C { string public important = "./NotAnImport.sol"; }
            function reimport() external {}
        "#;
        assert!(imported_paths(src).is_empty(), "{:?}", imported_paths(src));
    }

    #[test]
    fn comment_stripping_keeps_string_contents() {
        let out = strip_comments(r#"a = "keep // this"; // drop this"#);
        assert!(out.contains("keep // this"));
        assert!(!out.contains("drop this"));
    }

    #[test]
    fn relative_paths_resolve_against_the_importing_file() {
        let dir = std::env::temp_dir().join(format!("mulu-imp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("lib")).unwrap();
        std::fs::write(dir.join("lib/Base.sol"), "pragma solidity ^0.8.0;\ncontract B {}").unwrap();
        std::fs::write(
            dir.join("Main.sol"),
            "pragma solidity ^0.8.0;\nimport \"./lib/Base.sol\";\ncontract M is B {}",
        )
        .unwrap();
        let r = resolve(&dir, &[dir.join("Main.sol")]).unwrap();
        let mut keys: Vec<&String> = r.sources.keys().collect();
        keys.sort();
        assert_eq!(keys, vec!["Main.sol", "lib/Base.sol"]);
        assert!(r.unresolved.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_bare_import_is_reported_not_guessed() {
        let dir = std::env::temp_dir().join(format!("mulu-imp2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("M.sol"),
            "import \"@openzeppelin/contracts/token/ERC20.sol\";\ncontract M {}",
        )
        .unwrap();
        let r = resolve(&dir, &[dir.join("M.sol")]).unwrap();
        assert_eq!(r.unresolved, vec!["@openzeppelin/contracts/token/ERC20.sol"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_cycle_terminates() {
        let dir = std::env::temp_dir().join(format!("mulu-imp3-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("A.sol"), "import \"./B.sol\";").unwrap();
        std::fs::write(dir.join("B.sol"), "import \"./A.sol\";").unwrap();
        let r = resolve(&dir, &[dir.join("A.sol")]).unwrap();
        assert_eq!(r.sources.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
