//! An index over solc's AST.
//!
//! docs/08: "AST は関数・modifier・require の出所を追うため" — the AST is what
//! says where a check was written. The Yul only carries a byte span; whether
//! that span sits in a `require` inside a modifier, and which contract
//! declared that modifier, is in the AST and nowhere else.
//!
//! Note the two location formats do not agree: the AST writes
//! `start:length:fileId`, the Yul writes `fileId:start:end`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AstKind {
    Contract,
    Function,
    Modifier,
    Constructor,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AstNode {
    pub kind: AstKind,
    pub name: String,
    /// The contract that declares it; `None` for a contract itself.
    pub contract: Option<String>,
    pub file_id: u32,
    pub start: u32,
    pub length: u32,
}

impl AstNode {
    pub fn end(&self) -> u32 {
        self.start.saturating_add(self.length)
    }
    pub fn covers(&self, file_id: u32, start: u32, end: u32) -> bool {
        self.file_id == file_id && self.start <= start && end <= self.end()
    }
}

/// `start:length:fileId`, as solc writes an AST node's `src`.
fn parse_ast_src(text: &str) -> Option<(u32, u32, u32)> {
    let mut it = text.split(':');
    let start: i64 = it.next()?.parse().ok()?;
    let length: i64 = it.next()?.parse().ok()?;
    let file: i64 = it.next()?.parse().ok()?;
    if start < 0 || length < 0 || file < 0 {
        return None;
    }
    Some((start as u32, length as u32, file as u32))
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AstIndex {
    pub nodes: Vec<AstNode>,
    /// AST id to name, for every `immutable` variable declared anywhere in
    /// the build. solc compiles a read of one to `loadimmutable("13")`, where
    /// 13 is this id, and a guard on it is unreadable without the name.
    #[serde(default)]
    pub immutables: std::collections::BTreeMap<String, String>,
    /// Enum canonical name to how many members it has. A storage slot of an
    /// enum type holds one of them, and solc reverts on any other value, so
    /// the count is the slot's universe.
    #[serde(default)]
    pub enums: std::collections::BTreeMap<String, u64>,
}

impl AstIndex {
    /// Walk every source AST and record contracts, functions and modifiers.
    pub fn build(asts: &std::collections::BTreeMap<String, serde_json::Value>) -> Self {
        let mut nodes = Vec::new();
        for ast in asts.values() {
            collect(ast, None, &mut nodes);
        }
        // Innermost first when spans nest, so a plain scan finds the tightest.
        nodes.sort_by_key(|n| (n.file_id, n.start, n.length));
        let mut immutables = std::collections::BTreeMap::new();
        let mut enums = std::collections::BTreeMap::new();
        for ast in asts.values() {
            collect_immutables(ast, &mut immutables);
            collect_enums(ast, &mut enums);
        }
        Self { nodes, immutables, enums }
    }

    /// The tightest node containing the span, if any.
    pub fn innermost(&self, file_id: u32, start: u32, end: u32) -> Option<&AstNode> {
        self.nodes
            .iter()
            .filter(|n| n.covers(file_id, start, end))
            .min_by_key(|n| n.length)
    }

    /// Every node containing the span, widest first.
    pub fn enclosing(&self, file_id: u32, start: u32, end: u32) -> Vec<&AstNode> {
        let mut v: Vec<&AstNode> =
            self.nodes.iter().filter(|n| n.covers(file_id, start, end)).collect();
        v.sort_by_key(|n| std::cmp::Reverse(n.length));
        v
    }

    /// Is this span inside a modifier body? Returns the modifier node.
    pub fn modifier_at(&self, file_id: u32, start: u32, end: u32) -> Option<&AstNode> {
        self.enclosing(file_id, start, end).into_iter().find(|n| n.kind == AstKind::Modifier)
    }

    /// The contract declaring whatever contains this span.
    pub fn contract_at(&self, file_id: u32, start: u32, end: u32) -> Option<&str> {
        self.enclosing(file_id, start, end)
            .into_iter()
            .find(|n| n.kind == AstKind::Contract)
            .map(|n| n.name.as_str())
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

/// Every `VariableDeclaration` marked `immutable`, by its AST id.
fn collect_immutables(
    node: &serde_json::Value,
    out: &mut std::collections::BTreeMap<String, String>,
) {
    if node.get("nodeType").and_then(|v| v.as_str()) == Some("VariableDeclaration")
        && node.get("mutability").and_then(|v| v.as_str()) == Some("immutable")
    {
        if let (Some(id), Some(name)) = (
            node.get("id").and_then(|v| v.as_u64()),
            node.get("name").and_then(|v| v.as_str()),
        ) {
            out.insert(id.to_string(), name.to_string());
        }
    }
    match node {
        serde_json::Value::Object(m) => {
            for v in m.values() {
                collect_immutables(v, out);
            }
        }
        serde_json::Value::Array(a) => {
            for v in a {
                collect_immutables(v, out);
            }
        }
        _ => {}
    }
}

/// Every `EnumDefinition`, by canonical name, with its member count.
fn collect_enums(node: &serde_json::Value, out: &mut std::collections::BTreeMap<String, u64>) {
    if node.get("nodeType").and_then(|v| v.as_str()) == Some("EnumDefinition") {
        if let (Some(name), Some(members)) = (
            node.get("canonicalName").and_then(|v| v.as_str()),
            node.get("members").and_then(|v| v.as_array()),
        ) {
            out.insert(name.to_string(), members.len() as u64);
        }
    }
    match node {
        serde_json::Value::Object(m) => {
            for v in m.values() {
                collect_enums(v, out);
            }
        }
        serde_json::Value::Array(a) => {
            for v in a {
                collect_enums(v, out);
            }
        }
        _ => {}
    }
}

fn collect(node: &serde_json::Value, contract: Option<&str>, out: &mut Vec<AstNode>) {
    let mut current = contract.map(|s| s.to_string());
    if let Some(t) = node.get("nodeType").and_then(|v| v.as_str()) {
        let kind = match t {
            "ContractDefinition" => Some(AstKind::Contract),
            "ModifierDefinition" => Some(AstKind::Modifier),
            "FunctionDefinition" => Some(
                if node.get("kind").and_then(|v| v.as_str()) == Some("constructor") {
                    AstKind::Constructor
                } else {
                    AstKind::Function
                },
            ),
            _ => None,
        };
        if let Some(kind) = kind {
            let name = node
                .get("name")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .unwrap_or(if kind == AstKind::Constructor { "constructor" } else { "?" })
                .to_string();
            if let Some((start, length, file_id)) =
                node.get("src").and_then(|v| v.as_str()).and_then(parse_ast_src)
            {
                out.push(AstNode {
                    kind,
                    name: name.clone(),
                    contract: if kind == AstKind::Contract { None } else { current.clone() },
                    file_id,
                    start,
                    length,
                });
            }
            if kind == AstKind::Contract {
                current = Some(name);
            }
        }
    }
    match node {
        serde_json::Value::Object(map) => {
            for v in map.values() {
                collect(v, current.as_deref(), out);
            }
        }
        serde_json::Value::Array(items) => {
            for v in items {
                collect(v, current.as_deref(), out);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn index() -> AstIndex {
        let mut m = std::collections::BTreeMap::new();
        m.insert(
            "Base.sol".to_string(),
            json!({"nodeType": "SourceUnit", "nodes": [
                {"nodeType": "ContractDefinition", "name": "Owned", "src": "25:234:0", "nodes": [
                    {"nodeType": "ModifierDefinition", "name": "onlyOwner", "src": "82:90:0"},
                    {"nodeType": "ModifierDefinition", "name": "capped", "src": "178:79:0"}
                ]}
            ]}),
        );
        m.insert(
            "Vault.sol".to_string(),
            json!({"nodeType": "SourceUnit", "nodes": [
                {"nodeType": "ContractDefinition", "name": "Vault", "src": "47:170:1", "nodes": [
                    {"nodeType": "FunctionDefinition", "name": "setLimit", "kind": "function", "src": "104:111:1"},
                    {"nodeType": "FunctionDefinition", "name": "", "kind": "constructor", "src": "60:20:1"}
                ]}
            ]}),
        );
        AstIndex::build(&m)
    }

    #[test]
    fn the_ast_span_format_is_start_length_file() {
        assert_eq!(parse_ast_src("25:234:0"), Some((25, 234, 0)));
        // solc writes -1 for generated nodes with no source
        assert_eq!(parse_ast_src("-1:-1:-1"), None);
        assert_eq!(parse_ast_src("nonsense"), None);
    }

    #[test]
    fn a_require_inside_a_modifier_is_attributed_to_it() {
        let idx = index();
        // the require of `capped` sits inside 178..257 of file 0
        let m = idx.modifier_at(0, 200, 220).expect("inside the capped modifier");
        assert_eq!(m.name, "capped");
        assert_eq!(m.contract.as_deref(), Some("Owned"));
        assert_eq!(idx.contract_at(0, 200, 220), Some("Owned"));
    }

    #[test]
    fn a_require_in_a_function_body_is_not_a_modifier() {
        let idx = index();
        assert!(idx.modifier_at(1, 150, 160).is_none());
        let n = idx.innermost(1, 150, 160).expect("inside setLimit");
        assert_eq!((n.kind, n.name.as_str()), (AstKind::Function, "setLimit"));
        assert_eq!(idx.contract_at(1, 150, 160), Some("Vault"));
    }

    #[test]
    fn spans_are_matched_per_file() {
        let idx = index();
        // the same offsets in another file must not match
        assert!(idx.modifier_at(1, 200, 220).is_none());
        assert_eq!(idx.contract_at(0, 200, 220), Some("Owned"));
        assert_eq!(idx.contract_at(1, 200, 210), Some("Vault"));
    }

    #[test]
    fn a_constructor_is_recorded_as_one() {
        let idx = index();
        let n = idx.innermost(1, 65, 70).unwrap();
        assert_eq!(n.kind, AstKind::Constructor);
        assert_eq!(n.contract.as_deref(), Some("Vault"));
    }

    #[test]
    fn an_uncovered_span_yields_nothing_rather_than_a_guess() {
        let idx = index();
        assert!(idx.innermost(0, 0, 5).is_none(), "before the contract starts");
        assert!(idx.innermost(7, 100, 110).is_none(), "an unknown file");
        assert!(idx.enclosing(0, 0, 5).is_empty());
    }
}
