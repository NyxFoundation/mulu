//! Yul lexer.
//!
//! Beyond tokens it carries solc's location annotations, which are written as
//! doc comments and are the only link back to the Solidity source:
//!
//! ```text
//! /// @use-src 0:"Limits.sol"
//! /// @src 0:261:285  "require(x <= 100, \"cap\")"
//! ```
//!
//! An `@src` applies to everything after it until the next one, so each token
//! records the annotation in force when it was read.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

/// `fileId:start:end` as solc writes it. `end` is exclusive; a negative file
/// id means "generated code with no source", which we keep as `None` rather
/// than inventing a location.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SrcSpan {
    pub file_id: u32,
    pub start: u32,
    pub end: u32,
}

impl SrcSpan {
    pub fn len(&self) -> u32 {
        self.end.saturating_sub(self.start)
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tok {
    Ident(String),
    Number(String),
    StringLit(String),
    HexLit(String),
    LBrace,
    RBrace,
    LParen,
    RParen,
    Comma,
    Colon,
    Assign,
    Arrow,
    Eof,
}

impl Tok {
    pub fn describe(&self) -> String {
        match self {
            Tok::Ident(s) => format!("identifier {s:?}"),
            Tok::Number(s) => format!("number {s}"),
            Tok::StringLit(_) => "string".into(),
            Tok::HexLit(_) => "hex literal".into(),
            Tok::LBrace => "{".into(),
            Tok::RBrace => "}".into(),
            Tok::LParen => "(".into(),
            Tok::RParen => ")".into(),
            Tok::Comma => ",".into(),
            Tok::Colon => ":".into(),
            Tok::Assign => ":=".into(),
            Tok::Arrow => "->".into(),
            Tok::Eof => "end of input".into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Token {
    pub tok: Tok,
    /// The `@src` annotation in force at this token.
    pub src: Option<SrcSpan>,
    /// Byte offset into the Yul text, for parser diagnostics.
    pub offset: usize,
    pub line: usize,
}

#[derive(Debug, Error)]
#[error("Yul lex error at line {line}: {message}")]
pub struct LexError {
    pub line: usize,
    pub message: String,
}

pub struct Lexed {
    pub tokens: Vec<Token>,
    /// file id -> source path, from `@use-src`.
    pub use_src: BTreeMap<u32, String>,
}

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_' || c == '$'
}
fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '$' || c == '.'
}

/// `@src 0:261:285` -> span. Any negative component yields `None`.
fn parse_src(rest: &str) -> Option<SrcSpan> {
    let mut it = rest.trim_start().split_whitespace().next()?.split(':');
    let f: i64 = it.next()?.parse().ok()?;
    let s: i64 = it.next()?.parse().ok()?;
    let e: i64 = it.next()?.parse().ok()?;
    if f < 0 || s < 0 || e < 0 {
        return None;
    }
    Some(SrcSpan { file_id: f as u32, start: s as u32, end: e as u32 })
}

/// `@use-src 0:"A.sol", 1:"B.sol"`.
fn parse_use_src(rest: &str, out: &mut BTreeMap<u32, String>) {
    for part in rest.split(',') {
        let part = part.trim();
        let Some((id, path)) = part.split_once(':') else { continue };
        let Ok(id) = id.trim().parse::<u32>() else { continue };
        let path = path.trim().trim_matches('"');
        if !path.is_empty() {
            out.insert(id, path.to_string());
        }
    }
}

pub fn lex(input: &str) -> Result<Lexed, LexError> {
    let b: Vec<char> = input.chars().collect();
    // byte offset of each char index, so spans stay in bytes
    let mut byte_at = Vec::with_capacity(b.len() + 1);
    {
        let mut o = 0usize;
        for c in &b {
            byte_at.push(o);
            o += c.len_utf8();
        }
        byte_at.push(o);
    }

    let mut tokens = Vec::new();
    let mut use_src = BTreeMap::new();
    let mut cur_src: Option<SrcSpan> = None;
    let mut i = 0usize;
    let mut line = 1usize;

    macro_rules! push {
        ($t:expr, $start:expr) => {
            tokens.push(Token { tok: $t, src: cur_src, offset: byte_at[$start], line })
        };
    }

    while i < b.len() {
        let c = b[i];
        if c == '\n' {
            line += 1;
            i += 1;
            continue;
        }
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        // comments
        if c == '/' && i + 1 < b.len() && b[i + 1] == '/' {
            let start = i;
            while i < b.len() && b[i] != '\n' {
                i += 1;
            }
            let text: String = b[start..i].iter().collect();
            let body = text.trim_start_matches('/').trim();
            if let Some(rest) = body.strip_prefix("@use-src") {
                parse_use_src(rest, &mut use_src);
            } else if let Some(rest) = body.strip_prefix("@src") {
                // `@src -1:-1:-1` clears the annotation
                cur_src = parse_src(rest);
            }
            continue;
        }
        if c == '/' && i + 1 < b.len() && b[i + 1] == '*' {
            i += 2;
            loop {
                if i + 1 >= b.len() {
                    return Err(LexError { line, message: "unterminated block comment".into() });
                }
                if b[i] == '*' && b[i + 1] == '/' {
                    i += 2;
                    break;
                }
                if b[i] == '\n' {
                    line += 1;
                }
                i += 1;
            }
            continue;
        }
        let start = i;
        match c {
            '{' => {
                push!(Tok::LBrace, start);
                i += 1;
            }
            '}' => {
                push!(Tok::RBrace, start);
                i += 1;
            }
            '(' => {
                push!(Tok::LParen, start);
                i += 1;
            }
            ')' => {
                push!(Tok::RParen, start);
                i += 1;
            }
            ',' => {
                push!(Tok::Comma, start);
                i += 1;
            }
            '-' if i + 1 < b.len() && b[i + 1] == '>' => {
                push!(Tok::Arrow, start);
                i += 2;
            }
            ':' if i + 1 < b.len() && b[i + 1] == '=' => {
                push!(Tok::Assign, start);
                i += 2;
            }
            ':' => {
                push!(Tok::Colon, start);
                i += 1;
            }
            '"' => {
                i += 1;
                let mut s = String::new();
                loop {
                    if i >= b.len() {
                        return Err(LexError { line, message: "unterminated string".into() });
                    }
                    match b[i] {
                        '"' => {
                            i += 1;
                            break;
                        }
                        '\\' if i + 1 < b.len() => {
                            let e = b[i + 1];
                            s.push(match e {
                                'n' => '\n',
                                't' => '\t',
                                'r' => '\r',
                                other => other,
                            });
                            i += 2;
                        }
                        ch => {
                            if ch == '\n' {
                                return Err(LexError { line, message: "newline in string".into() });
                            }
                            s.push(ch);
                            i += 1;
                        }
                    }
                }
                push!(Tok::StringLit(s), start);
            }
            _ if c.is_ascii_digit() => {
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == 'x' || b[i] == 'X') {
                    i += 1;
                }
                let text: String = b[start..i].iter().collect();
                push!(Tok::Number(text), start);
            }
            _ if is_ident_start(c) => {
                while i < b.len() && is_ident_char(b[i]) {
                    i += 1;
                }
                let text: String = b[start..i].iter().collect();
                // hex"..." literal
                if text == "hex" && i < b.len() && b[i] == '"' {
                    i += 1;
                    let hstart = i;
                    while i < b.len() && b[i] != '"' {
                        i += 1;
                    }
                    if i >= b.len() {
                        return Err(LexError { line, message: "unterminated hex literal".into() });
                    }
                    let h: String = b[hstart..i].iter().collect();
                    i += 1;
                    push!(Tok::HexLit(h), start);
                } else {
                    push!(Tok::Ident(text), start);
                }
            }
            other => {
                return Err(LexError { line, message: format!("unexpected character {other:?}") })
            }
        }
    }
    tokens.push(Token { tok: Tok::Eof, src: cur_src, offset: byte_at[b.len()], line });
    Ok(Lexed { tokens, use_src })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracks_src_annotations_and_use_src() {
        let src = r#"
/// @use-src 0:"Limits.sol"
object "X" {
    code {
        /// @src 0:261:285  "require(x <= 100, \"cap\")"
        let a := 1
        /// @src -1:-1:-1
        let b := 2
    }
}"#;
        let out = lex(src).unwrap();
        assert_eq!(out.use_src.get(&0).map(String::as_str), Some("Limits.sol"));
        let a = out.tokens.iter().find(|t| t.tok == Tok::Ident("a".into())).unwrap();
        assert_eq!(a.src, Some(SrcSpan { file_id: 0, start: 261, end: 285 }));
        assert_eq!(a.src.unwrap().len(), 24);
        // the -1 annotation clears it rather than inventing a location
        let b = out.tokens.iter().find(|t| t.tok == Tok::Ident("b".into())).unwrap();
        assert_eq!(b.src, None);
    }

    #[test]
    fn lexes_the_shapes_solc_emits() {
        let out = lex(r#"{ let x := add(0x64, 100) if iszero(x) { revert(0, 0) } }"#).unwrap();
        let kinds: Vec<&Tok> = out.tokens.iter().map(|t| &t.tok).collect();
        assert!(kinds.contains(&&Tok::Assign));
        assert!(kinds.contains(&&Tok::Number("0x64".into())));
        assert!(kinds.contains(&&Tok::Ident("iszero".into())));
    }

    #[test]
    fn rejects_unterminated_constructs() {
        assert!(lex(r#"{ let s := "oops }"#).is_err());
        assert!(lex("{ /* nope }").is_err());
    }

    #[test]
    fn offsets_are_bytes_not_chars() {
        let out = lex("// 日本語\nlet x := 1").unwrap();
        let x = out.tokens.iter().find(|t| t.tok == Tok::Ident("x".into())).unwrap();
        assert_eq!(&"// 日本語\nlet x := 1"[x.offset..x.offset + 1], "x");
    }
}
