//! Rendering a parsed Yul object in `EvmYul`'s Lean notation.
//!
//! `semantics/` states mulu's correspondence conditions against EvmYul,
//! Nethermind's executable model of Yul in Lean. To state them about *this*
//! contract, the contract has to be in that semantics, and EvmYul's own
//! instructions for that are manual: split the dispatcher from the named
//! functions, drop comments, unwrap `memoryguard`. This module does it from
//! the parse mulu already has.
//!
//! Three places the rendering is not a transcription, each of which is a
//! difference a reader has to be told about rather than one to smooth over:
//!
//! * **`memoryguard(x)` becomes `x`.** It is a hint to solc's optimizer with
//!   no run-time meaning, and EvmYul does not model it.
//! * **A `switch` with no `default` gets an empty one.** Yul says an unmatched
//!   switch with no default does nothing. EvmYul's notation supplies
//!   `default { break }` when one is absent, and a `break` is not nothing:
//!   inside a loop it leaves the loop. Writing `default { }` says what Yul
//!   means and does not depend on that fallback.
//! * **A `for` loop's initialiser is hoisted before the loop.** EvmYul's
//!   `Stmt.For` has no initialiser, following solc's own ForLoopInitRewriter,
//!   and its notation only accepts `for { }`. Hoisting is the same rewrite,
//!   and it is sound only because Yul scopes the initialiser to the loop and
//!   nothing after it may refer to those names. mulu records that as an
//!   assumption rather than relying on the reader knowing it.

use crate::ast::{Block, Expr, FunctionDef, Object, Stmt};
use std::collections::BTreeSet;
use std::fmt::Write as _;

/// What the rendering did that a transcription would not. Collected while
/// rendering rather than listed in advance, so a contract that needed none of
/// them is not made to carry the assumption anyway.
#[derive(Debug, Default, Clone)]
pub struct Normalisations(BTreeSet<&'static str>);

impl Normalisations {
    fn note(&mut self, what: &'static str) {
        self.0.insert(what);
    }

    pub fn lines(&self) -> Vec<String> {
        self.0.iter().map(|s| s.to_string()).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

const MEMORYGUARD: &str = "memoryguard(x) was rendered as x: it is a hint to solc's optimizer \
                           with no run-time meaning, and EvmYul does not model it";
const SWITCH_DEFAULT: &str = "a switch with no default was given an empty one: Yul says an \
                              unmatched switch does nothing, and EvmYul's notation otherwise \
                              supplies `default { break }`";
const FOR_INIT: &str = "a for loop's initialiser was hoisted before the loop: EvmYul's `Stmt.For` \
                        has none, following solc's own ForLoopInitRewriter, and this is sound \
                        only because Yul scopes those names to the loop";
const STRING_WORD: &str = "a string literal was rendered as the 32-byte word Yul says it denotes, \
                           left-aligned and zero-padded";

/// What could not be rendered. Refusing is the point: a contract that is
/// silently half-translated would give a semantics for a program that is not
/// the one analysed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeanEmitError {
    pub reason: String,
}

impl std::fmt::Display for LeanEmitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.reason)
    }
}

impl std::error::Error for LeanEmitError {}

fn err<T>(reason: impl Into<String>) -> Result<T, LeanEmitError> {
    Err(LeanEmitError { reason: reason.into() })
}

/// Yul identifiers can contain `$` and `.`, which Lean's `ident` cannot.
/// solc emits neither in the unoptimized `ir`, so hitting one is a refusal
/// rather than a mangling: two names that mangled to one would silently make
/// a different program.
fn ident(name: &str) -> Result<&str, LeanEmitError> {
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return err(format!("the identifier {name:?} is not one Lean can parse as an `ident`"));
    }
    Ok(name)
}

fn expr(e: &Expr, out: &mut String, n: &mut Normalisations) -> Result<(), LeanEmitError> {
    match e {
        Expr::Ident { name, .. } => {
            out.push_str(ident(name)?);
        }
        Expr::Literal { text, .. } => {
            // The notation's `expr` accepts a numeral only. A Yul string
            // literal *is* a numeral: the spec says it denotes its bytes,
            // left-aligned in a 32-byte word and zero-padded on the right.
            // Writing that word is saying the same thing, not choosing a
            // number for it. solc emits these for `require` messages, so
            // refusing them would refuse most real contracts.
            let t = text.trim();
            if t.starts_with('"') || t.starts_with('\'') {
                n.note(STRING_WORD);
                out.push_str(&string_word(t)?);
            } else {
                out.push_str(t);
            }
        }
        Expr::Call { name, args, .. } => {
            // `memoryguard` is a hint to solc's optimizer, not an operation.
            if name == "memoryguard" {
                let [inner] = args.as_slice() else {
                    return err("memoryguard takes exactly one argument");
                };
                n.note(MEMORYGUARD);
                return expr(inner, out, n);
            }
            out.push_str(ident(name)?);
            out.push('(');
            for (i, a) in args.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                expr(a, out, n)?;
            }
            out.push(')');
        }
    }
    Ok(())
}

/// A Yul string literal as the 32-byte word it denotes: the bytes
/// left-aligned, zero-padded on the right. Over 32 bytes is not a Yul string
/// literal at all, and an escape this does not know is a refusal rather than
/// a guess, because guessing here changes the program.
fn string_word(lit: &str) -> Result<String, LeanEmitError> {
    let quote = lit.chars().next().unwrap_or('"');
    let inner = lit
        .strip_prefix(quote)
        .and_then(|s| s.strip_suffix(quote))
        .ok_or_else(|| LeanEmitError { reason: format!("unterminated string literal {lit}") })?;
    let mut bytes: Vec<u8> = vec![];
    let mut it = inner.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            let mut buf = [0u8; 4];
            bytes.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            continue;
        }
        match it.next() {
            Some('n') => bytes.push(b'\n'),
            Some('r') => bytes.push(b'\r'),
            Some('t') => bytes.push(b'\t'),
            Some('\\') => bytes.push(b'\\'),
            Some('"') => bytes.push(b'"'),
            Some('\'') => bytes.push(b'\''),
            Some('x') => {
                let hi = it.next();
                let lo = it.next();
                let (Some(hi), Some(lo)) = (hi, lo) else {
                    return err(format!("truncated \\x escape in {lit}"));
                };
                let v = u8::from_str_radix(&format!("{hi}{lo}"), 16)
                    .map_err(|_| LeanEmitError { reason: format!("bad \\x escape in {lit}") })?;
                bytes.push(v);
            }
            other => {
                return err(format!("escape \\{} in {lit} is not one this renders", other.unwrap_or(' ')))
            }
        }
    }
    if bytes.len() > 32 {
        return err(format!("the string literal {lit} is longer than one word"));
    }
    let mut word = [0u8; 32];
    word[..bytes.len()].copy_from_slice(&bytes);
    let mut hex = String::from("0x");
    for b in word {
        hex.push_str(&format!("{b:02x}"));
    }
    Ok(hex)
}

fn rendered(e: &Expr, n: &mut Normalisations) -> Result<String, LeanEmitError> {
    let mut s = String::new();
    expr(e, &mut s, n)?;
    Ok(s)
}

fn names(v: &[String]) -> Result<String, LeanEmitError> {
    let mut parts = vec![];
    for n in v {
        parts.push(ident(n)?.to_string());
    }
    Ok(parts.join(", "))
}

fn pad(indent: usize) -> String {
    "  ".repeat(indent)
}

fn stmt(s: &Stmt, indent: usize, out: &mut String, n: &mut Normalisations) -> Result<(), LeanEmitError> {
    let p = pad(indent);
    match s {
        Stmt::Block(b) => {
            let _ = writeln!(out, "{p}{{");
            block_body(b, indent + 1, out, n)?;
            let _ = writeln!(out, "{p}}}");
        }
        Stmt::Function(_) => {
            // Named functions live in the contract's function map, not in the
            // statement stream. A nested one would have nowhere to go.
            return err("a function definition inside a block cannot be rendered here");
        }
        Stmt::Let { names: vars, value, .. } => match value {
            Some(v) => {
                let _ = writeln!(out, "{p}let {} := {}", names(vars)?, rendered(v, n)?);
            }
            None => {
                let _ = writeln!(out, "{p}let {}", names(vars)?);
            }
        },
        Stmt::Assign { names: vars, value, .. } => {
            let _ = writeln!(out, "{p}{} := {}", names(vars)?, rendered(value, n)?);
        }
        Stmt::Expr { value, .. } => {
            let Expr::Call { .. } = value else {
                return err("a bare expression statement must be a call");
            };
            let _ = writeln!(out, "{p}{}", rendered(value, n)?);
        }
        Stmt::If { cond, body, .. } => {
            let _ = writeln!(out, "{p}if {} {{", rendered(cond, n)?);
            block_body(body, indent + 1, out, n)?;
            let _ = writeln!(out, "{p}}}");
        }
        Stmt::Switch { value, cases, .. } => {
            let _ = writeln!(out, "{p}switch {}", rendered(value, n)?);
            let mut has_default = false;
            for c in cases {
                match &c.value {
                    Some(lit) => {
                        let _ = writeln!(out, "{p}case {lit} {{");
                    }
                    None => {
                        has_default = true;
                        let _ = writeln!(out, "{p}default {{");
                    }
                }
                block_body(&c.body, indent + 1, out, n)?;
                let _ = writeln!(out, "{p}}}");
            }
            if !has_default {
                // Yul: nothing happens. Said explicitly, because the
                // notation's own fallback is `break`.
                n.note(SWITCH_DEFAULT);
                let _ = writeln!(out, "{p}default {{\n{p}}}");
            }
        }
        Stmt::For { pre, cond, post, body, .. } => {
            // Hoisted, because `Stmt.For` has no initialiser.
            if !pre.stmts.is_empty() {
                n.note(FOR_INIT);
            }
            for s in &pre.stmts {
                stmt(s, indent, out, n)?;
            }
            let _ = writeln!(out, "{p}for {{}} {} {{", rendered(cond, n)?);
            block_body(post, indent + 1, out, n)?;
            let _ = writeln!(out, "{p}}} {{");
            block_body(body, indent + 1, out, n)?;
            let _ = writeln!(out, "{p}}}");
        }
        Stmt::Break { .. } => {
            let _ = writeln!(out, "{p}break");
        }
        Stmt::Continue { .. } => {
            let _ = writeln!(out, "{p}continue");
        }
        Stmt::Leave { .. } => {
            let _ = writeln!(out, "{p}leave");
        }
    }
    Ok(())
}

fn block_body(b: &Block, indent: usize, out: &mut String, n: &mut Normalisations) -> Result<(), LeanEmitError> {
    for s in &b.stmts {
        stmt(s, indent, out, n)?;
    }
    Ok(())
}

/// The dispatcher: everything in the object's code block that is not a named
/// function definition.
fn dispatcher(o: &Object, n: &mut Normalisations) -> Result<String, LeanEmitError> {
    let mut out = String::new();
    out.push_str("{\n");
    for s in &o.code.stmts {
        if matches!(s, Stmt::Function(_)) {
            continue;
        }
        stmt(s, 1, &mut out, n)?;
    }
    out.push_str("}\n");
    Ok(out)
}

/// One named function, in the form `<f … >` takes.
fn function(f: &FunctionDef, n: &mut Normalisations) -> Result<String, LeanEmitError> {
    let mut out = String::new();
    let _ = write!(out, "function {}({})", ident(&f.name)?, names(&f.params)?);
    if !f.returns.is_empty() {
        let _ = write!(out, " -> {}", names(&f.returns)?);
    }
    out.push_str(" {\n");
    block_body(&f.body, 1, &mut out, n)?;
    out.push_str("}\n");
    Ok(out)
}

/// A complete Lean module defining the contract as an `EvmYul` `YulContract`.
/// `module` is the Lean namespace segment; the definition is `contract`.
pub fn contract_module(
    o: &Object,
    module: &str,
) -> Result<(String, Normalisations), LeanEmitError> {
    let mut n = Normalisations::default();
    let mut out = String::new();
    out.push_str(
        "-- Generated by `mulu yul-lean`. Do not edit.\n\
         --\n\
         -- The Yul of one contract, in EvmYul's notation, so the correspondence\n\
         -- conditions of `Mulu.Semantics.Simulation` can be stated about it.\n\
         -- `memoryguard` is unwrapped, a `switch` with no `default` is given an\n\
         -- empty one, and `for` initialisers are hoisted; see `mulu-yul/src/lean.rs`.\n\n",
    );
    out.push_str("import EvmYul.Yul.Interpreter\nimport EvmYul.Yul.YulNotation\n\n");
    let _ = writeln!(out, "namespace MuluGenerated.{}\n", ident(module)?);
    out.push_str("open EvmYul EvmYul.Yul EvmYul.Yul.Ast\n\n");
    let _ = writeln!(out, "/-- Derived from the Yul object `{}`. -/", o.name);
    out.push_str("def contract : YulContract where\n  dispatcher :=\n    <s ");
    out.push_str(&dispatcher(o, &mut n)?);
    out.push_str("    >\n  functions :=\n    (∅ : Finmap (fun (_ : YulFunctionName) ↦ FunctionDefinition))\n");
    for f in o.functions() {
        let _ = writeln!(out, "    |>.insert \"{}\"\n      <f {}      >", f.name, function(f, &mut n)?);
    }
    let _ = writeln!(out, "\nend MuluGenerated.{}", ident(module)?);
    Ok((out, n))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_string_literal_is_the_word_yul_says_it_is() {
        // "cap" is 0x636170 left-aligned: the same value solc stores for a
        // require message. Anything else would be a different program.
        let w = string_word("\"cap\"").unwrap();
        assert_eq!(&w[..8], "0x636170");
        assert_eq!(w.len(), 66);
        assert!(w.ends_with("00"));
        assert_eq!(string_word("\"\"").unwrap(), format!("0x{}", "00".repeat(32)));
    }

    #[test]
    fn escapes_this_does_not_know_are_refused_not_guessed() {
        assert!(string_word("\"a\\qb\"").is_err());
        assert!(string_word(&format!("\"{}\"", "x".repeat(33))).is_err());
        assert_eq!(&string_word("\"\\x41\"").unwrap()[..4], "0x41");
    }
}
