//! Yul syntax tree, faithful to what solc emits.

use crate::lex::SrcSpan;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Expr {
    Ident { name: String, src: Option<SrcSpan> },
    Literal { text: String, src: Option<SrcSpan> },
    Call { name: String, args: Vec<Expr>, src: Option<SrcSpan> },
}

impl Expr {
    pub fn src(&self) -> Option<SrcSpan> {
        match self {
            Expr::Ident { src, .. } | Expr::Literal { src, .. } | Expr::Call { src, .. } => *src,
        }
    }

    /// Every identifier read by this expression.
    pub fn idents(&self, out: &mut Vec<String>) {
        match self {
            Expr::Ident { name, .. } => out.push(name.clone()),
            Expr::Literal { .. } => {}
            Expr::Call { args, .. } => args.iter().for_each(|a| a.idents(out)),
        }
    }

    /// Every function name called, including nested.
    pub fn calls(&self, out: &mut Vec<String>) {
        if let Expr::Call { name, args, .. } = self {
            out.push(name.clone());
            args.iter().for_each(|a| a.calls(out));
        }
    }

    /// Substitute identifiers, used to move a guard condition from a helper's
    /// parameters to the arguments at its call site.
    pub fn substitute(&self, map: &std::collections::BTreeMap<String, Expr>) -> Expr {
        match self {
            Expr::Ident { name, src } => match map.get(name) {
                Some(replacement) => replacement.clone(),
                None => Expr::Ident { name: name.clone(), src: *src },
            },
            Expr::Literal { text, src } => Expr::Literal { text: text.clone(), src: *src },
            Expr::Call { name, args, src } => Expr::Call {
                name: name.clone(),
                args: args.iter().map(|a| a.substitute(map)).collect(),
                src: *src,
            },
        }
    }

    /// Compact Yul-like rendering, for messages and reports.
    pub fn render(&self) -> String {
        match self {
            Expr::Ident { name, .. } => name.clone(),
            Expr::Literal { text, .. } => text.clone(),
            Expr::Call { name, args, .. } => {
                let inner: Vec<String> = args.iter().map(|a| a.render()).collect();
                format!("{name}({})", inner.join(", "))
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Case {
    /// `None` is the `default` case.
    pub value: Option<String>,
    pub body: Block,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Stmt {
    Block(Block),
    Function(FunctionDef),
    Let { names: Vec<String>, value: Option<Expr>, src: Option<SrcSpan> },
    Assign { names: Vec<String>, value: Expr, src: Option<SrcSpan> },
    Expr { value: Expr, src: Option<SrcSpan> },
    If { cond: Expr, body: Block, src: Option<SrcSpan> },
    Switch { value: Expr, cases: Vec<Case>, src: Option<SrcSpan> },
    For { pre: Block, cond: Expr, post: Block, body: Block, src: Option<SrcSpan> },
    Break { src: Option<SrcSpan> },
    Continue { src: Option<SrcSpan> },
    Leave { src: Option<SrcSpan> },
}

impl Stmt {
    pub fn src(&self) -> Option<SrcSpan> {
        match self {
            Stmt::Block(b) => b.src,
            Stmt::Function(f) => f.src,
            Stmt::Let { src, .. }
            | Stmt::Assign { src, .. }
            | Stmt::Expr { src, .. }
            | Stmt::If { src, .. }
            | Stmt::Switch { src, .. }
            | Stmt::For { src, .. }
            | Stmt::Break { src }
            | Stmt::Continue { src }
            | Stmt::Leave { src } => *src,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub src: Option<SrcSpan>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionDef {
    pub name: String,
    pub params: Vec<String>,
    pub returns: Vec<String>,
    pub body: Block,
    pub src: Option<SrcSpan>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ObjectItem {
    Object(Object),
    Data { name: String, value: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Object {
    pub name: String,
    pub code: Block,
    pub items: Vec<ObjectItem>,
}

impl Object {
    /// The nested object holding the deployed (runtime) code, by solc's
    /// `<Name>_deployed` convention. Falls back to any single child object.
    pub fn deployed(&self) -> Option<&Object> {
        let children: Vec<&Object> = self
            .items
            .iter()
            .filter_map(|i| match i {
                ObjectItem::Object(o) => Some(o),
                _ => None,
            })
            .collect();
        children
            .iter()
            .find(|o| o.name.ends_with("_deployed"))
            .or_else(|| children.first())
            .copied()
    }

    /// Top-level function definitions in this object's code block.
    pub fn functions(&self) -> Vec<&FunctionDef> {
        self.code
            .stmts
            .iter()
            .filter_map(|s| match s {
                Stmt::Function(f) => Some(f),
                _ => None,
            })
            .collect()
    }
}
