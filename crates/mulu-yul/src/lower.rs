//! Yul AST -> ProgramIR: CFG per Yul function, transitive effects, and check
//! extraction.
//!
//! Every recognition here is syntactic and its criterion is recorded on the
//! result. Nothing is simplified away: a construct that is not understood
//! becomes an `Unsupported` entry with a location.

use crate::ast::{self, Expr, FunctionDef, Stmt};
use crate::builtins::{classify, Effects, Purity};
use crate::ir::*;
use crate::lex::SrcSpan;
use std::collections::{BTreeMap, BTreeSet};

/// A helper whose whole body is one guard: `if <cond> { ...always revert... }`.
#[derive(Debug, Clone)]
struct GuardHelper {
    /// Passes when this is non-zero, written over the function's parameters.
    pass_condition: Expr,
    params: Vec<String>,
}

pub struct Lowering<'a> {
    contract: &'a str,
    source_path: &'a str,
    compiler: &'a str,
    functions: Vec<Function>,
    checks: Vec<Check>,
    unsupported: Vec<Unsupported>,
    /// Yul function name -> its definition, across the objects we lower.
    defs: BTreeMap<String, &'a FunctionDef>,
    effects: BTreeMap<String, Effects>,
    always_reverts: BTreeSet<String>,
    guards: BTreeMap<String, GuardHelper>,
    aliases: BTreeMap<String, (Vec<String>, Expr)>,
}

fn loc(s: Option<SrcSpan>) -> Option<Location> {
    s.map(Location::from)
}

fn kind_of(name: &str) -> FunctionKind {
    if name.starts_with("external_fun_") {
        FunctionKind::External
    } else if name.starts_with("constructor_") {
        FunctionKind::Constructor
    } else if name.starts_with("fun_") || name.starts_with("getter_fun_") {
        FunctionKind::Body
    } else {
        FunctionKind::Helper
    }
}

/// `fun_setLimit_27` -> `setLimit`, `getter_fun_limit_3` -> `limit`.
fn solidity_name(name: &str) -> Option<String> {
    let rest = name
        .strip_prefix("external_fun_")
        .or_else(|| name.strip_prefix("getter_fun_"))
        .or_else(|| name.strip_prefix("fun_"))?;
    let base = match rest.rsplit_once('_') {
        Some((head, tail)) if tail.chars().all(|c| c.is_ascii_digit()) && !head.is_empty() => head,
        _ => rest,
    };
    Some(base.to_string())
}

// ---------------------------------------------------------------- CFG builder

struct Builder {
    blocks: Vec<Block>,
    current: BlockId,
    terminated: bool,
    loops: Vec<(BlockId, BlockId)>, // (break target, continue target)
}

impl Builder {
    fn new() -> Self {
        let entry = Block {
            id: 0,
            instructions: vec![],
            terminator: Terminator::Leave,
            always_reverts: false,
        };
        Self { blocks: vec![entry], current: 0, terminated: false, loops: vec![] }
    }
    fn new_block(&mut self) -> BlockId {
        let id = self.blocks.len();
        self.blocks.push(Block {
            id,
            instructions: vec![],
            terminator: Terminator::Leave,
            always_reverts: false,
        });
        id
    }
    fn emit(&mut self, i: Instruction) {
        if !self.terminated {
            self.blocks[self.current].instructions.push(i);
        }
    }
    fn terminate(&mut self, t: Terminator) {
        if !self.terminated {
            self.blocks[self.current].terminator = t;
            self.terminated = true;
        }
    }
    fn switch_to(&mut self, b: BlockId) {
        self.current = b;
        self.terminated = false;
    }
}

impl<'a> Lowering<'a> {
    pub fn new(contract: &'a str, source_path: &'a str, compiler: &'a str) -> Self {
        Self {
            contract,
            source_path,
            compiler,
            functions: vec![],
            checks: vec![],
            unsupported: vec![],
            defs: BTreeMap::new(),
            effects: BTreeMap::new(),
            always_reverts: BTreeSet::new(),
            guards: BTreeMap::new(),
            aliases: BTreeMap::new(),
        }
    }

    // ------------------------------------------------------------- effects

    /// Direct effects of an expression, treating callees as opaque for now.
    fn expr_effects_shallow(&self, e: &Expr, out: &mut Effects, callees: &mut Vec<String>) {
        if let Expr::Call { name, args, .. } = e {
            match classify(name) {
                Some(b) => out.add_builtin(name, b),
                None => callees.push(name.clone()),
            }
            for a in args {
                self.expr_effects_shallow(a, out, callees);
            }
        }
    }

    fn block_effects_shallow(&self, b: &ast::Block, out: &mut Effects, callees: &mut Vec<String>) {
        for s in &b.stmts {
            match s {
                // A nested function definition has its own summary.
                Stmt::Function(_) => {}
                Stmt::Let { value: Some(v), .. } => self.expr_effects_shallow(v, out, callees),
                Stmt::Let { value: None, .. } => {}
                Stmt::Assign { value, .. } | Stmt::Expr { value, .. } => {
                    self.expr_effects_shallow(value, out, callees)
                }
                Stmt::If { cond, body, .. } => {
                    self.expr_effects_shallow(cond, out, callees);
                    self.block_effects_shallow(body, out, callees);
                }
                Stmt::Switch { value, cases, .. } => {
                    self.expr_effects_shallow(value, out, callees);
                    for c in cases {
                        self.block_effects_shallow(&c.body, out, callees);
                    }
                }
                Stmt::For { pre, cond, post, body, .. } => {
                    self.block_effects_shallow(pre, out, callees);
                    self.expr_effects_shallow(cond, out, callees);
                    self.block_effects_shallow(post, out, callees);
                    self.block_effects_shallow(body, out, callees);
                }
                Stmt::Block(b) => self.block_effects_shallow(b, out, callees),
                Stmt::Break { .. } | Stmt::Continue { .. } | Stmt::Leave { .. } => {}
            }
        }
    }

    /// Fixpoint over the call graph: effects and "always reverts".
    fn analyse_functions(&mut self) {
        let mut shallow: BTreeMap<String, (Effects, Vec<String>)> = BTreeMap::new();
        for (name, def) in &self.defs {
            let mut e = Effects::default();
            let mut callees = Vec::new();
            self.block_effects_shallow(&def.body, &mut e, &mut callees);
            // A callee we have no definition for is not modelled.
            for c in &callees {
                if !self.defs.contains_key(c) {
                    if !e.unsupported.contains(c) {
                        e.unsupported.push(c.clone());
                    }
                }
            }
            shallow.insert(name.clone(), (e, callees));
        }
        // effects
        let mut summaries: BTreeMap<String, Effects> =
            shallow.iter().map(|(k, (e, _))| (k.clone(), e.clone())).collect();
        loop {
            let mut changed = false;
            for (name, (_, callees)) in &shallow {
                let mut merged = summaries[name].clone();
                for c in callees {
                    if let Some(ce) = summaries.get(c) {
                        merged.merge(ce);
                    }
                }
                if merged != summaries[name] {
                    summaries.insert(name.clone(), merged);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        self.effects = summaries;

        // always-reverts, by structure rather than by effect flags
        loop {
            let mut changed = false;
            for (name, def) in &self.defs {
                if self.always_reverts.contains(name.as_str()) {
                    continue;
                }
                if self.block_always_reverts(&def.body) {
                    self.always_reverts.insert(name.clone());
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
    }

    /// Every path through this Yul block ends in `revert`/`invalid`.
    fn block_always_reverts(&self, b: &ast::Block) -> bool {
        for s in &b.stmts {
            match s {
                Stmt::Expr { value: Expr::Call { name, .. }, .. } => {
                    if name == "revert" || name == "invalid" {
                        return true;
                    }
                    if self.always_reverts.contains(name.as_str()) {
                        return true;
                    }
                }
                Stmt::Switch { cases, .. } => {
                    let has_default = cases.iter().any(|c| c.value.is_none());
                    if has_default && cases.iter().all(|c| self.block_always_reverts(&c.body)) {
                        return true;
                    }
                }
                Stmt::Block(inner) => {
                    if self.block_always_reverts(inner) {
                        return true;
                    }
                }
                _ => {}
            }
        }
        false
    }

    /// Recognise guard helpers: zero results, body is exactly one
    /// `if <cond> { <always reverts> }`.
    fn find_guards(&mut self) {
        let names: Vec<String> = self.defs.keys().cloned().collect();
        for name in names {
            let def = self.defs[&name];
            if !def.returns.is_empty() || def.body.stmts.len() != 1 {
                continue;
            }
            let Stmt::If { cond, body, .. } = &def.body.stmts[0] else { continue };
            if !self.block_always_reverts(body) {
                continue;
            }
            // The condition itself must not have effects: the guard is a test.
            let mut e = Effects::default();
            let mut callees = Vec::new();
            self.expr_effects_shallow(cond, &mut e, &mut callees);
            for c in &callees {
                if let Some(ce) = self.effects.get(c) {
                    e.merge(ce);
                }
            }
            if e.writes_storage || e.writes_memory || e.can_revert || !e.supported() {
                continue;
            }
            // `if iszero(P) { revert }` passes on P; otherwise it passes on iszero(C).
            let pass_condition = match cond {
                Expr::Call { name, args, .. } if name == "iszero" && args.len() == 1 => {
                    args[0].clone()
                }
                other => Expr::Call {
                    name: "iszero".into(),
                    args: vec![other.clone()],
                    src: other.src(),
                },
            };
            self.guards
                .insert(name.clone(), GuardHelper { pass_condition, params: def.params.clone() });
        }
    }

    fn purity_of_expr(&self, e: &Expr) -> Purity {
        let mut eff = Effects::default();
        let mut callees = Vec::new();
        self.expr_effects_shallow(e, &mut eff, &mut callees);
        for c in &callees {
            match self.effects.get(c) {
                Some(ce) => eff.merge(ce),
                None => eff.unsupported.push(c.clone()),
            }
        }
        Purity::of(&eff)
    }

    fn instruction_effects(&self, e: &Expr) -> Effects {
        let mut eff = Effects::default();
        let mut callees = Vec::new();
        self.expr_effects_shallow(e, &mut eff, &mut callees);
        for c in &callees {
            match self.effects.get(c) {
                Some(ce) => eff.merge(ce),
                None => eff.unsupported.push(c.clone()),
            }
        }
        eff
    }

    // -------------------------------------------------------------- lowering

    fn instruction(&self, op: Op, value: Option<&Expr>, src: Option<SrcSpan>) -> Instruction {
        let mut reads = Vec::new();
        let mut effects = Effects::default();
        if let Some(v) = value {
            v.idents(&mut reads);
            effects = self.instruction_effects(v);
        }
        reads.sort();
        reads.dedup();
        let writes = match &op {
            Op::Let { targets, .. } | Op::Assign { targets, .. } => targets.clone(),
            Op::Effect { .. } => vec![],
        };
        Instruction { op, reads, writes, effects, source: loc(src) }
    }

    fn lower_stmts(&mut self, b: &mut Builder, block: &ast::Block, fname: &str) {
        for s in &block.stmts {
            if b.terminated {
                // Yul has no fallthrough after revert/return; anything after is
                // dead code that solc does emit, so just stop.
                break;
            }
            match s {
                Stmt::Function(_) => {} // lowered separately
                Stmt::Let { names, value, src } => {
                    let op = Op::Let { targets: names.clone(), value: value.clone() };
                    let i = self.instruction(op, value.as_ref(), *src);
                    b.emit(i);
                }
                Stmt::Assign { names, value, src } => {
                    let op = Op::Assign { targets: names.clone(), value: value.clone() };
                    let i = self.instruction(op, Some(value), *src);
                    b.emit(i);
                }
                Stmt::Expr { value, src } => {
                    if let Expr::Call { name, .. } = value {
                        match name.as_str() {
                            "revert" | "invalid" => {
                                b.terminate(Terminator::Revert { call: value.clone() });
                                continue;
                            }
                            "return" => {
                                b.terminate(Terminator::Return { call: value.clone() });
                                continue;
                            }
                            "stop" => {
                                b.terminate(Terminator::Stop);
                                continue;
                            }
                            _ => {}
                        }
                        if self.always_reverts.contains(name.as_str()) {
                            let i =
                                self.instruction(Op::Effect { call: value.clone() }, Some(value), *src);
                            b.emit(i);
                            b.terminate(Terminator::Revert { call: value.clone() });
                            continue;
                        }
                    }
                    let i = self.instruction(Op::Effect { call: value.clone() }, Some(value), *src);
                    b.emit(i);
                }
                Stmt::If { cond, body, src } => {
                    let then_b = b.new_block();
                    let cont_b = b.new_block();
                    b.terminate(Terminator::Branch {
                        cond: cond.clone(),
                        then_block: then_b,
                        else_block: cont_b,
                    });
                    let _ = src;
                    b.switch_to(then_b);
                    self.lower_stmts(b, body, fname);
                    if !b.terminated {
                        b.terminate(Terminator::Jump { target: cont_b });
                    }
                    b.switch_to(cont_b);
                }
                Stmt::Switch { value, cases, src } => {
                    let cont_b = b.new_block();
                    let mut arms = Vec::new();
                    let mut default = None;
                    for c in cases {
                        let blk = b.new_block();
                        match &c.value {
                            Some(v) => arms.push((v.clone(), blk)),
                            None => default = Some(blk),
                        }
                    }
                    b.terminate(Terminator::Switch {
                        value: value.clone(),
                        cases: arms.clone(),
                        default,
                    });
                    let bodies: Vec<(BlockId, &ast::Block)> = cases
                        .iter()
                        .map(|c| match &c.value {
                            Some(v) => {
                                (arms.iter().find(|(lit, _)| lit == v).unwrap().1, &c.body)
                            }
                            None => (default.unwrap(), &c.body),
                        })
                        .collect();
                    for (blk, body) in bodies {
                        b.switch_to(blk);
                        self.lower_stmts(b, body, fname);
                        if !b.terminated {
                            b.terminate(Terminator::Jump { target: cont_b });
                        }
                    }
                    if default.is_none() {
                        // no default arm: the switch can fall through
                    }
                    let _ = src;
                    b.switch_to(cont_b);
                }
                Stmt::For { pre, cond, post, body, src } => {
                    self.lower_stmts(b, pre, fname);
                    let head = b.new_block();
                    let body_b = b.new_block();
                    let post_b = b.new_block();
                    let exit_b = b.new_block();
                    if !b.terminated {
                        b.terminate(Terminator::Jump { target: head });
                    }
                    b.switch_to(head);
                    b.terminate(Terminator::Branch {
                        cond: cond.clone(),
                        then_block: body_b,
                        else_block: exit_b,
                    });
                    b.loops.push((exit_b, post_b));
                    b.switch_to(body_b);
                    self.lower_stmts(b, body, fname);
                    if !b.terminated {
                        b.terminate(Terminator::Jump { target: post_b });
                    }
                    b.loops.pop();
                    b.switch_to(post_b);
                    self.lower_stmts(b, post, fname);
                    if !b.terminated {
                        b.terminate(Terminator::Jump { target: head });
                    }
                    let _ = src;
                    b.switch_to(exit_b);
                }
                Stmt::Break { src } => match b.loops.last() {
                    Some((brk, _)) => b.terminate(Terminator::Jump { target: *brk }),
                    None => {
                        self.unsupported.push(Unsupported {
                            function: fname.to_string(),
                            reason: "`break` outside a loop".into(),
                            source: loc(*src),
                        });
                        b.terminate(Terminator::Unsupported {
                            reason: "break outside a loop".into(),
                        });
                    }
                },
                Stmt::Continue { src } => match b.loops.last() {
                    Some((_, cont)) => b.terminate(Terminator::Jump { target: *cont }),
                    None => {
                        self.unsupported.push(Unsupported {
                            function: fname.to_string(),
                            reason: "`continue` outside a loop".into(),
                            source: loc(*src),
                        });
                        b.terminate(Terminator::Unsupported {
                            reason: "continue outside a loop".into(),
                        });
                    }
                },
                Stmt::Leave { .. } => b.terminate(Terminator::Leave),
                Stmt::Block(inner) => self.lower_stmts(b, inner, fname),
            }
        }
    }

    /// Backward fixpoint: which blocks revert on every path.
    fn mark_always_reverts(&self, blocks: &mut [Block]) {
        loop {
            let mut changed = false;
            for i in 0..blocks.len() {
                if blocks[i].always_reverts {
                    continue;
                }
                let via_instruction = blocks[i]
                    .instructions
                    .iter()
                    .any(|ins| matches!(&ins.op, Op::Effect { call: Expr::Call { name, .. } }
                        if self.always_reverts.contains(name.as_str())));
                let via_terminator = match &blocks[i].terminator {
                    Terminator::Revert { .. } => true,
                    Terminator::Jump { target } => blocks[*target].always_reverts,
                    Terminator::Branch { then_block, else_block, .. } => {
                        blocks[*then_block].always_reverts && blocks[*else_block].always_reverts
                    }
                    Terminator::Switch { cases, default, .. } => {
                        default.is_some()
                            && cases.iter().all(|(_, b)| blocks[*b].always_reverts)
                            && blocks[default.unwrap()].always_reverts
                    }
                    _ => false,
                };
                if via_instruction || via_terminator {
                    blocks[i].always_reverts = true;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
    }

    fn lower_function(&mut self, id: &str, params: &[String], returns: &[String],
                      body: &ast::Block, kind: FunctionKind, src: Option<SrcSpan>) -> Function {
        let mut b = Builder::new();
        self.lower_stmts(&mut b, body, id);
        if !b.terminated {
            b.terminate(if kind == FunctionKind::ObjectCode {
                Terminator::Stop
            } else {
                Terminator::Leave
            });
        }
        let mut blocks = b.blocks;
        self.mark_always_reverts(&mut blocks);
        let effects = self.effects.get(id).cloned().unwrap_or_else(|| {
            let mut e = Effects::default();
            let mut callees = Vec::new();
            self.block_effects_shallow(body, &mut e, &mut callees);
            for c in &callees {
                if let Some(ce) = self.effects.get(c) {
                    e.merge(ce);
                }
            }
            e
        });
        Function {
            id: id.to_string(),
            kind,
            solidity_name: solidity_name(id),
            parameters: params.to_vec(),
            returns: returns.to_vec(),
            entry: 0,
            always_reverts: blocks[0].always_reverts,
            blocks,
            effects,
            source: loc(src),
        }
    }

    // --------------------------------------------------- expression cleanup

    /// Functions that are a pure alias for one expression over their
    /// parameters: one return value, straight-line `let`/`:=` body, no
    /// control flow, no effects. solc emits many (`cleanup_*`, `convert_*`,
    /// `identity`) and leaving them in makes every condition unreadable.
    fn find_pure_aliases(&mut self) {
        let names: Vec<String> = self.defs.keys().cloned().collect();
        for name in names {
            let def = self.defs[&name];
            if def.returns.len() != 1 {
                continue;
            }
            let eff = self.effects.get(&name).cloned().unwrap_or_default();
            if Purity::of(&eff) != Purity::Pure {
                continue;
            }
            // Straight-line body only.
            let mut env: BTreeMap<String, Expr> = BTreeMap::new();
            let mut ok = true;
            for st in &def.body.stmts {
                match st {
                    Stmt::Let { names, value: Some(v), .. } if names.len() == 1 => {
                        env.insert(names[0].clone(), v.substitute(&env));
                    }
                    Stmt::Assign { names, value, .. } if names.len() == 1 => {
                        let resolved = value.substitute(&env);
                        env.insert(names[0].clone(), resolved);
                    }
                    _ => {
                        ok = false;
                        break;
                    }
                }
            }
            if !ok {
                continue;
            }
            if let Some(ret) = env.get(&def.returns[0]) {
                self.aliases.insert(name.clone(), (def.params.clone(), ret.clone()));
            }
        }
    }

    /// Immediate dominators by the usual iterative fixpoint, so a definition
    /// is only propagated to uses it actually dominates.
    fn dominators(blocks: &[Block]) -> Vec<BTreeSet<BlockId>> {
        let n = blocks.len();
        let mut preds: Vec<Vec<BlockId>> = vec![vec![]; n];
        for b in blocks {
            for succ in successors(&b.terminator) {
                preds[succ].push(b.id);
            }
        }
        let all: BTreeSet<BlockId> = (0..n).collect();
        let mut dom: Vec<BTreeSet<BlockId>> = (0..n).map(|_| all.clone()).collect();
        dom[0] = [0].into_iter().collect();
        loop {
            let mut changed = false;
            for i in 1..n {
                let mut new: Option<BTreeSet<BlockId>> = None;
                for p in &preds[i] {
                    new = Some(match new {
                        None => dom[*p].clone(),
                        Some(acc) => acc.intersection(&dom[*p]).copied().collect(),
                    });
                }
                let mut new = new.unwrap_or_default();
                new.insert(i);
                if new != dom[i] {
                    dom[i] = new;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        dom
    }

    /// Single-assignment, pure local definitions, with the block they are in.
    fn local_defs(&self, f: &Function) -> BTreeMap<String, (BlockId, usize, Expr)> {
        let mut count: BTreeMap<String, usize> = BTreeMap::new();
        let mut defs: BTreeMap<String, (BlockId, usize, Expr)> = BTreeMap::new();
        for b in &f.blocks {
            for (idx, ins) in b.instructions.iter().enumerate() {
                let (targets, value) = match &ins.op {
                    Op::Let { targets, value: Some(v) } => (targets, Some(v)),
                    Op::Let { targets, value: None } => (targets, None),
                    Op::Assign { targets, value } => (targets, Some(value)),
                    Op::Effect { .. } => continue,
                };
                for t in targets {
                    *count.entry(t.clone()).or_default() += 1;
                }
                if targets.len() == 1 {
                    if let Some(v) = value {
                        if Purity::of(&ins.effects) == Purity::Pure {
                            defs.insert(targets[0].clone(), (b.id, idx, v.clone()));
                        }
                    }
                }
            }
        }
        defs.retain(|k, _| count.get(k) == Some(&1));
        defs
    }

    /// Rewrite an expression into one over parameters and literals: inline
    /// pure alias helpers and propagate dominating single-assignment locals.
    fn simplify(&self, e: &Expr, f: &Function, at_block: BlockId, at_index: usize) -> Expr {
        let dom = Self::dominators(&f.blocks);
        let defs = self.local_defs(f);
        let mut cur = e.clone();
        // Bounded: solc's expression chains are shallow, and a bound keeps a
        // pathological input from looping.
        for _ in 0..64 {
            let next = self.simplify_once(&cur, &defs, &dom, at_block, at_index);
            if next.render() == cur.render() {
                return next;
            }
            cur = next;
        }
        cur
    }

    fn simplify_once(
        &self,
        e: &Expr,
        defs: &BTreeMap<String, (BlockId, usize, Expr)>,
        dom: &[BTreeSet<BlockId>],
        at_block: BlockId,
        at_index: usize,
    ) -> Expr {
        match e {
            Expr::Literal { .. } => e.clone(),
            Expr::Ident { name, src } => match defs.get(name) {
                Some((db, di, v))
                    if (*db == at_block && *di < at_index)
                        || (*db != at_block && dom.get(at_block).is_some_and(|d| d.contains(db))) =>
                {
                    let _ = src;
                    v.clone()
                }
                _ => e.clone(),
            },
            Expr::Call { name, args, src } => {
                let args: Vec<Expr> = args
                    .iter()
                    .map(|a| self.simplify_once(a, defs, dom, at_block, at_index))
                    .collect();
                if let Some((params, body)) = self.aliases.get(name) {
                    if params.len() == args.len() {
                        let map: BTreeMap<String, Expr> =
                            params.iter().cloned().zip(args.iter().cloned()).collect();
                        return body.substitute(&map);
                    }
                }
                Expr::Call { name: name.clone(), args, src: *src }
            }
        }
    }

    // ------------------------------------------------------------- checks

    fn extract_checks(&mut self) {
        let mut found: Vec<Check> = Vec::new();
        for f in &self.functions {
            // A guard helper *is* a check; reporting its internal branch as a
            // second one would double count it. It is reported at call sites.
            let is_guard = self.guards.contains_key(&f.id);
            for blk in &f.blocks {
                for (idx, ins) in blk.instructions.iter().enumerate() {
                    let Op::Effect { call: Expr::Call { name, args, src } } = &ins.op else {
                        continue;
                    };
                    let Some(g) = self.guards.get(name) else { continue };
                    if g.params.len() != args.len() {
                        continue;
                    }
                    let map: BTreeMap<String, Expr> =
                        g.params.iter().cloned().zip(args.iter().cloned()).collect();
                    let raw = g.pass_condition.substitute(&map);
                    let condition = self.simplify(&raw, f, blk.id, idx);
                    let origin = if name.starts_with("require_helper_") {
                        CheckOrigin::Require
                    } else {
                        CheckOrigin::Compiler
                    };
                    found.push(Check {
                        id: String::new(),
                        function: f.id.clone(),
                        condition_text: condition.render(),
                        condition_as_written: raw.render(),
                        purity: self.purity_of_expr(&condition),
                        condition,
                        origin,
                        helper: Some(name.clone()),
                        pre_location: blk.id,
                        pass_edge: CheckEdge::Continue,
                        fail_edge: CheckEdge::Revert { via: Some(name.clone()) },
                        source: loc(*src),
                        assumptions: guard_assumptions(true),
                    });
                }
                if is_guard {
                    continue;
                }
                if let Terminator::Branch { cond, then_block, else_block } = &blk.terminator {
                    let then_rev = f.blocks[*then_block].always_reverts;
                    let else_rev = f.blocks[*else_block].always_reverts;
                    if then_rev == else_rev {
                        continue; // both or neither revert: a branch, not a guard
                    }
                    let (pass_block, fail_block, raw) = if then_rev {
                        (
                            *else_block,
                            *then_block,
                            Expr::Call {
                                name: "iszero".into(),
                                args: vec![cond.clone()],
                                src: cond.src(),
                            },
                        )
                    } else {
                        (*then_block, *else_block, cond.clone())
                    };
                    let at = blk.instructions.len();
                    let condition = self.simplify(&raw, f, blk.id, at);
                    let purity = self.purity_of_expr(&condition);
                    if purity == Purity::Effectful {
                        continue;
                    }
                    found.push(Check {
                        id: String::new(),
                        function: f.id.clone(),
                        condition_text: condition.render(),
                        condition_as_written: raw.render(),
                        purity,
                        condition,
                        origin: CheckOrigin::Compiler,
                        helper: None,
                        pre_location: blk.id,
                        pass_edge: CheckEdge::Block { id: pass_block },
                        fail_edge: CheckEdge::Block { id: fail_block },
                        source: loc(cond.src()),
                        assumptions: guard_assumptions(false),
                    });
                }
            }
        }
        // `require`s carry letters in source order: they are what a reader of
        // the contract refers to. Compiler-inserted guards get an id derived
        // from where they sit, so adding a require does not renumber them.
        found.sort_by_key(|c| {
            (
                c.origin != CheckOrigin::Require,
                c.source.map(|l| (l.file_id, l.byte_start, l.byte_length)).unwrap_or((u32::MAX, u32::MAX, u32::MAX)),
                c.function.clone(),
                c.pre_location,
            )
        });
        let mut nth = 0usize;
        for c in found.iter_mut() {
            if c.origin == CheckOrigin::Require {
                c.id = check_id(nth);
                nth += 1;
            } else {
                c.id = format!("gen:{}#{}", c.function, c.pre_location);
            }
        }
        self.checks = found;
    }

    // --------------------------------------------------------------- driver

    pub fn run(
        mut self,
        object: &'a ast::Object,
        use_src: BTreeMap<u32, String>,
        storage_layout: serde_json::Value,
        abi: &serde_json::Value,
    ) -> ProgramIr {
        // Lower the creation object's code and the deployed object.
        let mut objects: Vec<(&ast::Object, String)> = vec![(object, object.name.clone())];
        if let Some(dep) = object.deployed() {
            objects.push((dep, dep.name.clone()));
        }
        for (o, _) in &objects {
            for f in o.functions() {
                self.defs.insert(f.name.clone(), f);
            }
        }
        self.analyse_functions();
        self.find_guards();
        self.find_pure_aliases();

        for (o, oname) in &objects {
            let code_id = format!("{oname}#code");
            let f = self.lower_function(
                &code_id,
                &[],
                &[],
                &o.code,
                FunctionKind::ObjectCode,
                o.code.src,
            );
            self.functions.push(f);
            for def in o.functions() {
                let f = self.lower_function(
                    &def.name,
                    &def.params,
                    &def.returns,
                    &def.body,
                    kind_of(&def.name),
                    def.src,
                );
                self.functions.push(f);
            }
        }
        self.functions.sort_by(|a, b| a.id.cmp(&b.id));
        self.extract_checks();

        let entrypoints = self.entrypoints(object, abi);

        // Anything not modelled, gathered from the function summaries.
        let mut unsupported = std::mem::take(&mut self.unsupported);
        for f in &self.functions {
            for u in &f.effects.unsupported {
                unsupported.push(Unsupported {
                    function: f.id.clone(),
                    reason: format!("calls `{u}`, which P1a does not model"),
                    source: f.source,
                });
            }
        }
        unsupported.sort_by(|a, b| (a.function.clone(), a.reason.clone()).cmp(&(b.function.clone(), b.reason.clone())));
        unsupported.dedup_by(|a, b| a.function == b.function && a.reason == b.reason);

        ProgramIr {
            schema_version: SCHEMA_VERSION,
            contract: self.contract.to_string(),
            source_path: self.source_path.to_string(),
            derived_from: "solc standard-json `ir` output (unoptimized Yul)".into(),
            compiler: self.compiler.to_string(),
            use_src,
            entrypoints,
            functions: self.functions,
            checks: self.checks,
            storage_layout,
            unsupported,
        }
    }

    /// Read the dispatcher's `switch selector case 0x... { external_fun_X() }`
    /// and pair each selector with the ABI signature of the same name.
    fn entrypoints(&self, object: &ast::Object, abi: &serde_json::Value) -> Vec<Entrypoint> {
        let Some(dep) = object.deployed() else { return vec![] };
        let mut out = Vec::new();
        collect_selectors(&dep.code, &mut out);
        let sigs: Vec<(String, String)> = abi
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter(|i| i["type"] == "function")
                    .filter_map(|i| {
                        let name = i["name"].as_str()?.to_string();
                        let args: Vec<&str> = i["inputs"]
                            .as_array()?
                            .iter()
                            .filter_map(|a| a["type"].as_str())
                            .collect();
                        Some((name.clone(), format!("{name}({})", args.join(","))))
                    })
                    .collect()
            })
            .unwrap_or_default();
        out.into_iter()
            .map(|(selector, external_function)| {
                let sol = solidity_name(&external_function).unwrap_or_default();
                let signature = sigs
                    .iter()
                    .find(|(n, _)| *n == sol)
                    .map(|(_, s)| s.clone())
                    .unwrap_or_else(|| format!("{sol}(?)"));
                Entrypoint { signature, selector, external_function }
            })
            .collect()
    }
}

fn collect_selectors(b: &ast::Block, out: &mut Vec<(String, String)>) {
    for s in &b.stmts {
        match s {
            Stmt::Switch { cases, .. } => {
                for c in cases {
                    let Some(lit) = &c.value else { continue };
                    // the arm body is a single call to external_fun_*
                    for st in &c.body.stmts {
                        if let Stmt::Expr { value: Expr::Call { name, .. }, .. } = st {
                            if name.starts_with("external_fun_") {
                                out.push((lit.clone(), name.clone()));
                            }
                        }
                    }
                }
            }
            Stmt::If { body, .. } => collect_selectors(body, out),
            Stmt::Block(inner) => collect_selectors(inner, out),
            _ => {}
        }
    }
}

/// What a recognised guard depends on, so the gap is visible in the report.
fn guard_assumptions(via_helper: bool) -> Vec<String> {
    let mut v = vec![
        "expression-propagation: single-assignment pure locals and pure alias helpers were substituted into the condition".to_string(),
    ];
    if via_helper {
        v.push("guard-helper-shape: the callee's body is one `if c { .. revert }` and nothing else".into());
        v.push("revert-path-memory-unobservable: memory written to build the revert reason is discarded by the rollback".into());
    } else {
        v.push("inline-guard-shape: exactly one side of the branch reverts on every path".into());
    }
    v
}

/// Blocks a terminator can transfer control to.
fn successors(t: &Terminator) -> Vec<BlockId> {
    match t {
        Terminator::Jump { target } => vec![*target],
        Terminator::Branch { then_block, else_block, .. } => vec![*then_block, *else_block],
        Terminator::Switch { cases, default, .. } => {
            let mut v: Vec<BlockId> = cases.iter().map(|(_, b)| *b).collect();
            v.extend(default.iter().copied());
            v
        }
        _ => vec![],
    }
}

/// A, B, ... Z, AA, AB, ...
fn check_id(i: usize) -> String {
    let mut n = i;
    let mut s = Vec::new();
    loop {
        s.push(b'A' + (n % 26) as u8);
        if n < 26 {
            break;
        }
        n = n / 26 - 1;
    }
    s.reverse();
    String::from_utf8(s).expect("ascii")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_ids_are_stable_and_ordered() {
        assert_eq!(check_id(0), "A");
        assert_eq!(check_id(1), "B");
        assert_eq!(check_id(25), "Z");
        assert_eq!(check_id(26), "AA");
    }

    #[test]
    fn solidity_names_drop_the_ast_id_suffix() {
        assert_eq!(solidity_name("fun_setLimit_27").as_deref(), Some("setLimit"));
        assert_eq!(solidity_name("external_fun_forceSet_37").as_deref(), Some("forceSet"));
        assert_eq!(solidity_name("getter_fun_limit_3").as_deref(), Some("limit"));
        assert_eq!(solidity_name("allocate_unbounded"), None);
    }

    #[test]
    fn function_kinds_follow_solc_naming() {
        assert_eq!(kind_of("external_fun_setLimit_27"), FunctionKind::External);
        assert_eq!(kind_of("fun_setLimit_27"), FunctionKind::Body);
        assert_eq!(kind_of("constructor_Limits_38"), FunctionKind::Constructor);
        assert_eq!(kind_of("allocate_unbounded"), FunctionKind::Helper);
    }
}
