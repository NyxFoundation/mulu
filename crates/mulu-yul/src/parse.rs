//! Recursive-descent parser for the Yul solc emits.

use crate::ast::*;
use crate::lex::{lex, SrcSpan, Tok, Token};
use std::collections::BTreeMap;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ParseError {
    #[error(transparent)]
    Lex(#[from] crate::lex::LexError),
    #[error("Yul parse error at line {line}: expected {expected}, found {found}")]
    Unexpected { line: usize, expected: String, found: String },
    #[error("Yul parse error at line {line}: {message}")]
    Message { line: usize, message: String },
}

const KEYWORDS: &[&str] = &[
    "object", "code", "data", "function", "let", "if", "switch", "case", "default", "for", "break",
    "continue", "leave", "true", "false",
];

#[derive(Debug)]
pub struct Parsed {
    pub object: Object,
    pub use_src: BTreeMap<u32, String>,
}

struct Parser {
    toks: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> &Tok {
        &self.toks[self.pos].tok
    }
    fn peek_at(&self, n: usize) -> &Tok {
        &self.toks[(self.pos + n).min(self.toks.len() - 1)].tok
    }
    fn src(&self) -> Option<SrcSpan> {
        self.toks[self.pos].src
    }
    fn line(&self) -> usize {
        self.toks[self.pos].line
    }
    fn bump(&mut self) -> Tok {
        let t = self.toks[self.pos].tok.clone();
        if self.pos + 1 < self.toks.len() {
            self.pos += 1;
        }
        t
    }
    fn err<T>(&self, expected: &str) -> Result<T, ParseError> {
        Err(ParseError::Unexpected {
            line: self.line(),
            expected: expected.to_string(),
            found: self.peek().describe(),
        })
    }
    fn eat(&mut self, t: &Tok, what: &str) -> Result<(), ParseError> {
        if self.peek() == t {
            self.bump();
            Ok(())
        } else {
            self.err(what)
        }
    }
    fn is_kw(&self, kw: &str) -> bool {
        matches!(self.peek(), Tok::Ident(s) if s == kw)
    }
    fn eat_kw(&mut self, kw: &str) -> Result<(), ParseError> {
        if self.is_kw(kw) {
            self.bump();
            Ok(())
        } else {
            self.err(&format!("keyword `{kw}`"))
        }
    }
    fn ident(&mut self) -> Result<String, ParseError> {
        match self.peek().clone() {
            Tok::Ident(s) if !KEYWORDS.contains(&s.as_str()) => {
                self.bump();
                Ok(s)
            }
            _ => self.err("an identifier"),
        }
    }

    fn object(&mut self) -> Result<Object, ParseError> {
        self.eat_kw("object")?;
        let name = match self.bump() {
            Tok::StringLit(s) => s,
            other => {
                return Err(ParseError::Unexpected {
                    line: self.line(),
                    expected: "an object name string".into(),
                    found: other.describe(),
                })
            }
        };
        self.eat(&Tok::LBrace, "`{` after the object name")?;
        self.eat_kw("code")?;
        let code = self.block()?;
        let mut items = Vec::new();
        loop {
            if self.is_kw("object") {
                items.push(ObjectItem::Object(self.object()?));
            } else if self.is_kw("data") {
                self.bump();
                let dname = match self.bump() {
                    Tok::StringLit(s) => s,
                    other => {
                        return Err(ParseError::Unexpected {
                            line: self.line(),
                            expected: "a data name string".into(),
                            found: other.describe(),
                        })
                    }
                };
                let value = match self.bump() {
                    Tok::StringLit(s) | Tok::HexLit(s) => s,
                    other => {
                        return Err(ParseError::Unexpected {
                            line: self.line(),
                            expected: "a string or hex literal".into(),
                            found: other.describe(),
                        })
                    }
                };
                items.push(ObjectItem::Data { name: dname, value });
            } else {
                break;
            }
        }
        self.eat(&Tok::RBrace, "`}` closing the object")?;
        Ok(Object { name, code, items })
    }

    fn block(&mut self) -> Result<Block, ParseError> {
        let src = self.src();
        self.eat(&Tok::LBrace, "`{`")?;
        let mut stmts = Vec::new();
        while self.peek() != &Tok::RBrace {
            if self.peek() == &Tok::Eof {
                return Err(ParseError::Message {
                    line: self.line(),
                    message: "unclosed block at end of input".into(),
                });
            }
            stmts.push(self.stmt()?);
        }
        self.bump();
        Ok(Block { stmts, src })
    }

    fn stmt(&mut self) -> Result<Stmt, ParseError> {
        let src = self.src();
        if self.peek() == &Tok::LBrace {
            return Ok(Stmt::Block(self.block()?));
        }
        if self.is_kw("function") {
            self.bump();
            let name = self.ident()?;
            self.eat(&Tok::LParen, "`(` after the function name")?;
            let mut params = Vec::new();
            while self.peek() != &Tok::RParen {
                params.push(self.ident()?);
                if self.peek() == &Tok::Comma {
                    self.bump();
                }
            }
            self.bump();
            let mut returns = Vec::new();
            if self.peek() == &Tok::Arrow {
                self.bump();
                loop {
                    returns.push(self.ident()?);
                    if self.peek() == &Tok::Comma {
                        self.bump();
                    } else {
                        break;
                    }
                }
            }
            let body = self.block()?;
            return Ok(Stmt::Function(FunctionDef { name, params, returns, body, src }));
        }
        if self.is_kw("let") {
            self.bump();
            let mut names = vec![self.ident()?];
            while self.peek() == &Tok::Comma {
                self.bump();
                names.push(self.ident()?);
            }
            let value = if self.peek() == &Tok::Assign {
                self.bump();
                Some(self.expr()?)
            } else {
                None
            };
            return Ok(Stmt::Let { names, value, src });
        }
        if self.is_kw("if") {
            self.bump();
            let cond = self.expr()?;
            let body = self.block()?;
            return Ok(Stmt::If { cond, body, src });
        }
        if self.is_kw("switch") {
            self.bump();
            let value = self.expr()?;
            let mut cases = Vec::new();
            while self.is_kw("case") || self.is_kw("default") {
                if self.is_kw("case") {
                    self.bump();
                    let lit = match self.bump() {
                        Tok::Number(n) => n,
                        Tok::StringLit(s) => format!("{s:?}"),
                        Tok::HexLit(h) => format!("hex\"{h}\""),
                        Tok::Ident(i) if i == "true" || i == "false" => i,
                        other => {
                            return Err(ParseError::Unexpected {
                                line: self.line(),
                                expected: "a case literal".into(),
                                found: other.describe(),
                            })
                        }
                    };
                    let body = self.block()?;
                    cases.push(Case { value: Some(lit), body });
                } else {
                    self.bump();
                    let body = self.block()?;
                    cases.push(Case { value: None, body });
                }
            }
            return Ok(Stmt::Switch { value, cases, src });
        }
        if self.is_kw("for") {
            self.bump();
            let pre = self.block()?;
            let cond = self.expr()?;
            let post = self.block()?;
            let body = self.block()?;
            return Ok(Stmt::For { pre, cond, post, body, src });
        }
        for (kw, make) in [
            ("break", (|s| Stmt::Break { src: s }) as fn(Option<SrcSpan>) -> Stmt),
            ("continue", |s| Stmt::Continue { src: s }),
            ("leave", |s| Stmt::Leave { src: s }),
        ] {
            if self.is_kw(kw) {
                self.bump();
                return Ok(make(src));
            }
        }
        // assignment: Ident (, Ident)* := Expr
        if matches!(self.peek(), Tok::Ident(_))
            && (self.peek_at(1) == &Tok::Assign || self.peek_at(1) == &Tok::Comma)
        {
            let save = self.pos;
            let mut names = Vec::new();
            let ok = loop {
                match self.ident() {
                    Ok(n) => names.push(n),
                    Err(_) => break false,
                }
                if self.peek() == &Tok::Comma {
                    self.bump();
                } else {
                    break self.peek() == &Tok::Assign;
                }
            };
            if ok {
                self.bump();
                let value = self.expr()?;
                return Ok(Stmt::Assign { names, value, src });
            }
            self.pos = save;
        }
        let value = self.expr()?;
        Ok(Stmt::Expr { value, src })
    }

    fn expr(&mut self) -> Result<Expr, ParseError> {
        let src = self.src();
        match self.peek().clone() {
            Tok::Number(n) => {
                self.bump();
                self.skip_type_suffix();
                Ok(Expr::Literal { text: n, src })
            }
            Tok::StringLit(s) => {
                self.bump();
                self.skip_type_suffix();
                Ok(Expr::Literal { text: format!("{s:?}"), src })
            }
            Tok::HexLit(h) => {
                self.bump();
                self.skip_type_suffix();
                Ok(Expr::Literal { text: format!("hex\"{h}\""), src })
            }
            Tok::Ident(name) if name == "true" || name == "false" => {
                self.bump();
                self.skip_type_suffix();
                Ok(Expr::Literal { text: name, src })
            }
            Tok::Ident(name) if !KEYWORDS.contains(&name.as_str()) => {
                self.bump();
                if self.peek() == &Tok::LParen {
                    self.bump();
                    let mut args = Vec::new();
                    while self.peek() != &Tok::RParen {
                        args.push(self.expr()?);
                        if self.peek() == &Tok::Comma {
                            self.bump();
                        } else {
                            break;
                        }
                    }
                    self.eat(&Tok::RParen, "`)` closing the argument list")?;
                    Ok(Expr::Call { name, args, src })
                } else {
                    Ok(Expr::Ident { name, src })
                }
            }
            _ => self.err("an expression"),
        }
    }

    /// Typed literals (`1:u256`) exist in the Yul grammar; the EVM dialect
    /// solc emits does not use them, but accept and drop them.
    fn skip_type_suffix(&mut self) {
        if self.peek() == &Tok::Colon {
            self.bump();
            let _ = self.ident();
        }
    }
}

/// Parse a whole Yul object as produced by `solc --standard-json` `ir` output.
pub fn parse_object(input: &str) -> Result<Parsed, ParseError> {
    let lexed = lex(input)?;
    let mut p = Parser { toks: lexed.tokens, pos: 0 };
    let object = p.object()?;
    if p.peek() != &Tok::Eof {
        return Err(ParseError::Message {
            line: p.line(),
            message: format!("trailing input after the top-level object: {}", p.peek().describe()),
        });
    }
    Ok(Parsed { object, use_src: lexed.use_src })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_nested_object_with_data() {
        let src = r#"
        object "A" {
            code { let x := 1 }
            object "A_deployed" {
                code { mstore(64, 128) }
                data ".metadata" hex"ff00"
            }
        }"#;
        let p = parse_object(src).unwrap();
        assert_eq!(p.object.name, "A");
        let dep = p.object.deployed().unwrap();
        assert_eq!(dep.name, "A_deployed");
        assert_eq!(dep.items.len(), 1);
    }

    #[test]
    fn parses_every_statement_form() {
        let src = r#"
        object "A" { code {
            function f(a, b) -> r { r := add(a, b) leave }
            let x, y := f(1, 2)
            x := 3
            if iszero(x) { revert(0, 0) }
            switch x
            case 0 { let q := 1 }
            case 1 { let q := 2 }
            default { stop() }
            for { let i := 0 } lt(i, 10) { i := add(i, 1) } { break continue }
            sstore(0, x)
        } }"#;
        let p = parse_object(src).unwrap();
        let stmts = &p.object.code.stmts;
        assert!(matches!(stmts[0], Stmt::Function(_)));
        assert!(matches!(&stmts[1], Stmt::Let { names, .. } if names.len() == 2));
        assert!(matches!(stmts[2], Stmt::Assign { .. }));
        assert!(matches!(stmts[3], Stmt::If { .. }));
        assert!(matches!(&stmts[4], Stmt::Switch { cases, .. } if cases.len() == 3));
        assert!(matches!(stmts[5], Stmt::For { .. }));
        assert!(matches!(stmts[6], Stmt::Expr { .. }));
    }

    #[test]
    fn substitution_moves_a_condition_to_the_call_site() {
        let p = parse_object(r#"object "A" { code { let c := iszero(gt(v, 100)) } }"#).unwrap();
        let Stmt::Let { value: Some(e), .. } = &p.object.code.stmts[0] else { panic!() };
        let mut map = BTreeMap::new();
        map.insert("v".to_string(), Expr::Ident { name: "x".into(), src: None });
        assert_eq!(e.substitute(&map).render(), "iszero(gt(x, 100))");
    }

    #[test]
    fn reports_position_on_bad_input() {
        let e = parse_object("object \"A\" { code { let := 1 } }").unwrap_err();
        assert!(matches!(e, ParseError::Unexpected { .. }), "got {e}");
    }
}
