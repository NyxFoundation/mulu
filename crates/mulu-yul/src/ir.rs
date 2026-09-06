//! ProgramIR — the contract of docs/09 §2.1, as produced from Yul.

use crate::ast::Expr;
use crate::builtins::{Effects, Purity};
use crate::lex::SrcSpan;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// `(file-id, byte-start, byte-length)`. Built from solc's `fileId:start:end`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Location {
    pub file_id: u32,
    pub byte_start: u32,
    pub byte_length: u32,
}

impl From<SrcSpan> for Location {
    fn from(s: SrcSpan) -> Self {
        Location { file_id: s.file_id, byte_start: s.start, byte_length: s.len() }
    }
}

pub type BlockId = usize;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
pub enum Op {
    /// `let a, b := expr`
    Let { targets: Vec<String>, value: Option<Expr> },
    /// `a, b := expr`
    Assign { targets: Vec<String>, value: Expr },
    /// A call evaluated for its effect, e.g. `sstore(...)` or a helper call.
    Effect { call: Expr },
}

/// A recognised whole-slot storage write, with the expressions that give the
/// slot and the value once alias helpers and single-assignment locals have
/// been folded in. Recorded when the callee's body holds exactly one `sstore`
/// whose slot and value both reduce to parameters of that callee, so the call
/// site's arguments determine both.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageWrite {
    pub slot: Expr,
    pub value: Expr,
    pub slot_text: String,
    pub value_text: String,
    /// The helper it was recognised through; `None` for a literal `sstore`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub via: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Instruction {
    pub op: Op,
    pub reads: Vec<String>,
    pub writes: Vec<String>,
    /// Transitive effects of everything this instruction calls.
    pub effects: Effects,
    /// Set when this instruction writes a whole storage slot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_write: Option<StorageWrite>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<Location>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Terminator {
    Jump { target: BlockId },
    Branch { cond: Expr, then_block: BlockId, else_block: BlockId },
    Switch { value: Expr, cases: Vec<(String, BlockId)>, default: Option<BlockId> },
    /// `return(offset, size)`: the transaction succeeds.
    Return { call: Expr },
    /// `revert(offset, size)` or `invalid()`: state is rolled back.
    Revert { call: Expr },
    /// `stop()`
    Stop,
    /// Fell off the end of a Yul function, or an explicit `leave`.
    Leave,
    /// A construct outside the subset. Never treated as a no-op.
    Unsupported { reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Block {
    pub id: BlockId,
    pub instructions: Vec<Instruction>,
    pub terminator: Terminator,
    /// Every path out of this block reverts the transaction.
    pub always_reverts: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FunctionKind {
    /// The object's top-level code: the dispatcher, or the creation code.
    ObjectCode,
    /// `external_fun_*`: ABI decode, call the body, ABI encode the result.
    External,
    /// `fun_*` / `getter_fun_*`: a Solidity function body.
    Body,
    /// `constructor_*`
    Constructor,
    /// Compiler-generated helper.
    Helper,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Function {
    pub id: String,
    pub kind: FunctionKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub solidity_name: Option<String>,
    pub parameters: Vec<String>,
    pub returns: Vec<String>,
    pub entry: BlockId,
    pub blocks: Vec<Block>,
    pub effects: Effects,
    /// Calling this function always reverts.
    pub always_reverts: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<Location>,
}

impl Function {
    pub fn block(&self, id: BlockId) -> &Block {
        &self.blocks[id]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CheckOrigin {
    /// A `require(...)` in the Solidity source.
    Require,
    /// Inserted by the compiler: non-payable guard, ABI validation, and such.
    Compiler,
    /// A guard written directly as `if ... { revert }` in Yul.
    Inline,
}

/// Where control goes on each side of a check.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "edge", rename_all = "kebab-case")]
pub enum CheckEdge {
    /// Execution continues after the check, in the same block.
    Continue,
    /// The transaction reverts, through the named helper.
    Revert {
        #[serde(skip_serializing_if = "Option::is_none")]
        via: Option<String>,
    },
    /// A real successor block in this function's CFG.
    Block { id: BlockId },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Check {
    /// Stable id within the contract, assigned in source order: A, B, C...
    pub id: String,
    /// The function whose body evaluates it.
    pub function: String,
    /// Passes when this expression is non-zero.
    pub condition: Expr,
    /// The condition rendered as Yul, for reports.
    pub condition_text: String,
    /// The condition before propagation, as it stands in the generated Yul.
    pub condition_as_written: String,
    pub origin: CheckOrigin,
    /// The guard helper this came from, if it was a call.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub helper: Option<String>,
    /// Block whose execution reaches the check.
    pub pre_location: BlockId,
    pub pass_edge: CheckEdge,
    pub fail_edge: CheckEdge,
    pub purity: Purity,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<Location>,
    /// Recognition criteria and semantic gaps this check depends on.
    pub assumptions: Vec<String>,
}

/// Something the lowering met and did not model. Kept with a location so a
/// later stage can refuse to call the analysis complete.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Unsupported {
    pub function: String,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<Location>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entrypoint {
    /// `setLimit(uint256)`
    pub signature: String,
    /// 4-byte selector as it appears in the dispatcher, e.g. `0x27ea6f2b`.
    pub selector: String,
    /// The `external_fun_*` wrapper.
    pub external_function: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgramIr {
    pub schema_version: u32,
    pub contract: String,
    pub source_path: String,
    /// The artifact this IR was built from. Claims derived from it are about
    /// this layer, not about the deployed bytecode.
    pub derived_from: String,
    pub compiler: String,
    /// solc file id to source path, from the Yul `@use-src` annotations.
    pub use_src: BTreeMap<u32, String>,
    pub entrypoints: Vec<Entrypoint>,
    pub functions: Vec<Function>,
    pub checks: Vec<Check>,
    pub storage_layout: serde_json::Value,
    pub unsupported: Vec<Unsupported>,
}

pub const SCHEMA_VERSION: u32 = 1;

impl ProgramIr {
    pub fn function(&self, id: &str) -> Option<&Function> {
        self.functions.iter().find(|f| f.id == id)
    }
    pub fn check(&self, id: &str) -> Option<&Check> {
        self.checks.iter().find(|c| c.id == id)
    }
    /// Checks the analysis may use as predicates: pure conditions only.
    pub fn pure_checks(&self) -> Vec<&Check> {
        self.checks.iter().filter(|c| c.purity == Purity::Pure).collect()
    }
    /// Recognised whole-slot writes, with the function they sit in.
    pub fn recognised_storage_writes(&self) -> Vec<(&str, &Instruction, &StorageWrite)> {
        self.functions
            .iter()
            .flat_map(|f| {
                f.blocks.iter().flat_map(move |b| {
                    b.instructions.iter().filter_map(move |i| {
                        i.storage_write.as_ref().map(|w| (f.id.as_str(), i, w))
                    })
                })
            })
            .collect()
    }

    /// Instructions that write storage, with the function they sit in.
    pub fn storage_writes(&self) -> Vec<(&str, &Instruction)> {
        self.functions
            .iter()
            .flat_map(|f| {
                f.blocks.iter().flat_map(move |b| {
                    b.instructions
                        .iter()
                        .filter(|i| i.effects.writes_storage)
                        .map(move |i| (f.id.as_str(), i))
                })
            })
            .collect()
    }
    /// True when every construct met was modelled.
    pub fn fully_supported(&self) -> bool {
        self.unsupported.is_empty()
    }
}
