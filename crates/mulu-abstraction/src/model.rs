//! Predicate abstraction: ProgramIR plus a specification, out comes the
//! `finite-product` model the P0 core already analyses.
//!
//! The construction follows docs/11: event schemas (§2), the state and what
//! each event preserves (§3), the environment profile (§5) and the interval
//! partition worked through in §7. Every step that could quietly lose a
//! behaviour instead records an assumption or refuses.

use crate::interval::{IntervalSet, U256};
use crate::predicate::{translate, Predicate};
use crate::spec::{storage_vars, CompiledProperty, StorageVar};
use mulu_model::schema::{CheckDecl, Control, EventDecl, FiniteProduct, Initial, Transition};
use mulu_yul::ir::{FunctionKind, Terminator};
use mulu_yul::{Function, ProgramIr, Purity};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// The environment the model is built under (docs/11 §5).
pub const ENVIRONMENT_PROFILE: &str = "p1a-abi-single-v1";

#[derive(Debug, Clone, Serialize)]
pub struct RegionInfo {
    pub name: String,
    pub set: IntervalSet,
    pub description: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PredicateInfo {
    pub id: String,
    pub source: String,
    pub text: String,
    pub set: IntervalSet,
}

#[derive(Debug, Clone, Serialize)]
pub struct AbstractionReport {
    pub environment_profile: &'static str,
    pub entrypoints_modelled: Vec<String>,
    pub entrypoints_skipped: Vec<String>,
    pub argument_predicates: Vec<PredicateInfo>,
    pub storage_predicates: Vec<PredicateInfo>,
    pub argument_regions: Vec<RegionInfo>,
    pub storage_regions: Vec<RegionInfo>,
    /// Facts the interval arithmetic settled, such as an infeasible combination.
    pub discharged: Vec<String>,
    pub assumptions: Vec<String>,
    /// Anything that stops this from being a complete model of the contract.
    pub unsupported: Vec<String>,
}

impl AbstractionReport {
    pub fn complete(&self) -> bool {
        self.unsupported.is_empty()
    }
}

pub struct Abstraction {
    pub model: FiniteProduct,
    pub report: AbstractionReport,
}

/// One entrypoint reduced to what the walk needs.
struct Entry {
    solidity_name: String,
    signature: String,
    func: String,
    param: Option<String>,
}

pub struct Builder<'a> {
    ir: &'a ProgramIr,
    props: &'a [CompiledProperty],
    storage: Vec<StorageVar>,
    unsupported: Vec<String>,
    assumptions: Vec<String>,
    discharged: Vec<String>,
}

/// Values a storage slot can be in, as an index into the slot's partition.
type StorageRegion = Vec<usize>;

impl<'a> Builder<'a> {
    pub fn new(ir: &'a ProgramIr, props: &'a [CompiledProperty]) -> Self {
        Self {
            ir,
            props,
            storage: storage_vars(&ir.storage_layout),
            unsupported: vec![],
            assumptions: vec![],
            discharged: vec![],
        }
    }

    fn note(&mut self, s: impl Into<String>) {
        let s = s.into();
        if !self.assumptions.contains(&s) {
            self.assumptions.push(s);
        }
    }
    fn refuse(&mut self, s: impl Into<String>) {
        let s = s.into();
        if !self.unsupported.contains(&s) {
            self.unsupported.push(s);
        }
    }

    /// The Solidity function body an `external_fun_*` wrapper calls.
    fn body_of(&self, external: &str) -> Option<&'a Function> {
        let ext = self.ir.function(external)?;
        for b in &ext.blocks {
            for ins in &b.instructions {
                let mut calls = Vec::new();
                match &ins.op {
                    mulu_yul::ir::Op::Effect { call } => call.calls(&mut calls),
                    mulu_yul::ir::Op::Let { value: Some(v), .. } => v.calls(&mut calls),
                    mulu_yul::ir::Op::Assign { value, .. } => value.calls(&mut calls),
                    _ => {}
                }
                for c in calls {
                    if let Some(f) = self.ir.function(&c) {
                        if f.kind == FunctionKind::Body {
                            return Some(f);
                        }
                    }
                }
            }
        }
        None
    }

    fn entries(&mut self) -> Vec<Entry> {
        let mut out = Vec::new();
        for e in &self.ir.entrypoints {
            let Some(f) = self.body_of(&e.external_function) else {
                self.refuse(format!(
                    "entrypoint {}: could not find the Solidity function body behind {}",
                    e.signature, e.external_function
                ));
                continue;
            };
            if f.parameters.len() > 1 {
                self.refuse(format!(
                    "entrypoint {} takes {} parameters; P1a models at most one uint256 argument",
                    e.signature,
                    f.parameters.len()
                ));
                continue;
            }
            if !f.effects.supported() {
                self.refuse(format!(
                    "entrypoint {} reaches {:?}, which P1a does not model",
                    e.signature, f.effects.unsupported
                ));
                continue;
            }
            out.push(Entry {
                solidity_name: f
                    .solidity_name
                    .clone()
                    .unwrap_or_else(|| f.id.clone()),
                signature: e.signature.clone(),
                func: f.id.clone(),
                param: f.parameters.first().cloned(),
            });
        }
        out
    }

    /// Guard predicates of a function, keyed by check id.
    fn guards(&mut self, f: &Function) -> BTreeMap<String, Predicate> {
        let mut out = BTreeMap::new();
        for c in self.ir.checks.iter().filter(|c| c.function == f.id) {
            if c.purity != Purity::Pure {
                self.refuse(format!(
                    "check {} in {} reads {:?}; P1a guards must be a function of the argument alone",
                    c.id, f.id, c.purity
                ));
                continue;
            }
            match translate(&c.condition, &f.parameters) {
                Ok(p) => {
                    out.insert(c.id.clone(), p);
                }
                Err(why) => self.refuse(format!("check {} in {}: {why}", c.id, f.id)),
            }
        }
        out
    }

    /// Partition `uint256` by a family of sets: the non-empty atoms of the
    /// boolean algebra they generate, plus the combinations that turned out
    /// to be unsatisfiable.
    fn partition(sets: &[(String, IntervalSet)]) -> (Vec<IntervalSet>, Vec<String>) {
        let mut atoms = vec![(IntervalSet::full(), Vec::<(String, bool)>::new())];
        let mut infeasible = Vec::new();
        for (name, s) in sets {
            let mut next = Vec::new();
            for (atom, trail) in atoms {
                for (part, sign) in [(atom.intersect(s), true), (atom.difference(s), false)] {
                    let mut t = trail.clone();
                    t.push((name.clone(), sign));
                    if part.is_empty() {
                        // Only report a combination that mentions every set,
                        // otherwise every refinement of it is reported too.
                        if t.len() == sets.len() {
                            infeasible.push(
                                t.iter()
                                    .map(|(n, s)| if *s { n.clone() } else { format!("not {n}") })
                                    .collect::<Vec<_>>()
                                    .join(" and "),
                            );
                        }
                    } else {
                        next.push((part, t));
                    }
                }
            }
            atoms = next;
        }
        let mut out: Vec<IntervalSet> = atoms.into_iter().map(|(a, _)| a).collect();
        out.sort_by_key(|s| s.ranges().first().map(|(lo, _)| *lo).unwrap_or(U256::ZERO));
        (out, infeasible)
    }

    pub fn build(mut self) -> Abstraction {
        let entries = self.entries();

        // --- storage partition, one per slot, refined by the specification
        let mut storage_preds: Vec<PredicateInfo> = Vec::new();
        let mut per_slot: Vec<(String, Vec<IntervalSet>)> = Vec::new();
        for v in self.storage.clone() {
            let sets: Vec<(String, IntervalSet)> = self
                .props
                .iter()
                .filter(|p| p.var.as_deref() == Some(v.label.as_str()))
                .map(|p| {
                    storage_preds.push(PredicateInfo {
                        id: p.id.clone(),
                        source: format!("spec property {}", p.id),
                        text: p.text.clone(),
                        set: p.predicate.set(),
                    });
                    (p.id.clone(), p.predicate.set())
                })
                .collect();
            let (regions, infeasible) = Self::partition(&sets);
            for i in infeasible {
                self.discharged.push(format!("storage {}: {i} is unsatisfiable", v.label));
            }
            per_slot.push((v.label.clone(), regions));
        }

        // --- argument predicates: guards, plus the specification pulled back
        //     through direct assignments of the argument to a slot
        let mut arg_sets: Vec<(String, IntervalSet)> = Vec::new();
        let mut arg_preds: Vec<PredicateInfo> = Vec::new();
        let mut guards_by_func: BTreeMap<String, BTreeMap<String, Predicate>> = BTreeMap::new();
        for e in &entries {
            let Some(f) = self.ir.function(&e.func) else { continue };
            let f = f.clone();
            let g = self.guards(&f);
            for (id, p) in &g {
                let set = p.set();
                if !set.is_full() && !set.is_empty() && !arg_sets.iter().any(|(n, _)| n == id) {
                    arg_preds.push(PredicateInfo {
                        id: id.clone(),
                        source: format!("check {id} in {}", e.solidity_name),
                        text: format!("{p}"),
                        set: set.clone(),
                    });
                    arg_sets.push((id.clone(), set));
                }
            }
            guards_by_func.insert(e.func.clone(), g);

            // pull the spec back through `slot := argument`
            for b in &f.blocks {
                for ins in &b.instructions {
                    let Some(w) = &ins.storage_write else { continue };
                    if Some(&w.value_text) != e.param.as_ref() {
                        continue;
                    }
                    let Some(label) = self.slot_label(&w.slot_text) else { continue };
                    for p in self.props.iter().filter(|p| p.var.as_deref() == Some(label.as_str())) {
                        let id = format!("spec:{}", p.id);
                        if !arg_sets.iter().any(|(n, _)| *n == id) {
                            arg_preds.push(PredicateInfo {
                                id: id.clone(),
                                source: format!(
                                    "spec property {} pulled back through {}: {} := {}",
                                    p.id, e.solidity_name, label, w.value_text
                                ),
                                text: p.text.clone(),
                                set: p.predicate.set(),
                            });
                            arg_sets.push((id, p.predicate.set()));
                        }
                    }
                }
            }
        }
        let (arg_regions, infeasible) = Self::partition(&arg_sets);
        for i in infeasible {
            self.discharged.push(format!("argument: {i} is unsatisfiable"));
        }

        self.note(format!(
            "environment profile {ENVIRONMENT_PROFILE}: well-formed ABI calls to the listed \
             entrypoints, value 0, any caller, any order, any finite number of transactions"
        ));
        self.note(
            "abi-decoder-unverified: calldata decoding and the non-payable guard are taken as \
             given by the environment profile and are not modelled",
        );
        self.note(
            "argument-regions-are-exact: guards are decided by interval arithmetic over uint256, \
             not by a solver, so no trusted-solver assumption is carried",
        );

        Walk::new(self, entries, guards_by_func, arg_regions, arg_preds, storage_preds, per_slot)
            .run()
    }

    fn slot_label(&self, slot_text: &str) -> Option<String> {
        let want = crate::interval::parse_decimal(slot_text).ok()?;
        self.storage
            .iter()
            .find(|v| crate::interval::parse_decimal(&v.slot).ok() == Some(want))
            .map(|v| v.label.clone())
    }
}

/// The symbolic walk that turns each entrypoint into transitions.
struct Walk<'a> {
    b: Builder<'a>,
    entries: Vec<Entry>,
    guards: BTreeMap<String, BTreeMap<String, Predicate>>,
    arg_regions: Vec<IntervalSet>,
    arg_preds: Vec<PredicateInfo>,
    storage_preds: Vec<PredicateInfo>,
    /// (slot label, regions of that slot)
    per_slot: Vec<(String, Vec<IntervalSet>)>,
    states: BTreeSet<String>,
    marked: BTreeSet<String>,
    events: BTreeMap<String, EventDecl>,
    transitions: Vec<Transition>,
    checks: BTreeMap<String, CheckDecl>,
    bad_used: bool,
}

impl<'a> Walk<'a> {
    #[allow(clippy::too_many_arguments)]
    fn new(
        b: Builder<'a>,
        entries: Vec<Entry>,
        guards: BTreeMap<String, BTreeMap<String, Predicate>>,
        arg_regions: Vec<IntervalSet>,
        arg_preds: Vec<PredicateInfo>,
        storage_preds: Vec<PredicateInfo>,
        per_slot: Vec<(String, Vec<IntervalSet>)>,
    ) -> Self {
        Self {
            b,
            entries,
            guards,
            arg_regions,
            arg_preds,
            storage_preds,
            per_slot,
            states: BTreeSet::new(),
            marked: BTreeSet::new(),
            events: BTreeMap::new(),
            transitions: Vec::new(),
            checks: BTreeMap::new(),
            bad_used: false,
        }
    }

    fn storage_regions(&self) -> Vec<StorageRegion> {
        let mut out: Vec<StorageRegion> = vec![vec![]];
        for (_, regions) in &self.per_slot {
            let mut next = Vec::new();
            for r in &out {
                for i in 0..regions.len() {
                    let mut v = r.clone();
                    v.push(i);
                    next.push(v);
                }
            }
            out = next;
        }
        out
    }

    fn storage_name(&self, s: &StorageRegion) -> String {
        if self.per_slot.len() == 1 && self.per_slot[0].1.len() == 1 {
            return "S".into();
        }
        let parts: Vec<String> = s
            .iter()
            .enumerate()
            .map(|(i, r)| format!("{}{}", short(&self.per_slot[i].0), r))
            .collect();
        parts.join("_")
    }

    fn arg_name(&self, a: usize) -> String {
        format!("X{a}")
    }

    fn event(&mut self, id: &str, description: &str) -> String {
        self.events.entry(id.to_string()).or_insert_with(|| EventDecl {
            id: id.to_string(),
            // Nothing here can be forbidden by a supervisor: requests arrive,
            // and guard results follow from the code (docs/11 §4).
            control: Control::Uncontrollable,
            description: Some(description.to_string()),
        });
        id.to_string()
    }

    fn add(&mut self, from: &str, event: &str, to: &str) {
        self.states.insert(from.to_string());
        self.states.insert(to.to_string());
        let t = Transition { from: from.to_string(), event: event.to_string(), to: to.to_string() };
        if !self
            .transitions
            .iter()
            .any(|x| x.from == t.from && x.event == t.event && x.to == t.to)
        {
            self.transitions.push(t);
        }
    }

    /// Which region of `slot` a value known to lie in `values` falls into.
    fn region_of(&self, slot: usize, values: &IntervalSet) -> Option<usize> {
        self.per_slot[slot].1.iter().position(|r| values.subset_of(r))
    }

    fn run(mut self) -> Abstraction {
        let storage_regions = self.storage_regions();

        // Initial storage: the state a constructor within the modelled subset
        // leaves behind. P1a handles the case where it writes nothing, so
        // every slot is zero.
        let ctor_writes: Vec<_> = self
            .b
            .ir
            .functions
            .iter()
            .filter(|f| f.kind == FunctionKind::Constructor)
            .flat_map(|f| f.blocks.iter().flat_map(|bl| bl.instructions.iter()))
            .filter(|i| i.effects.writes_storage)
            .collect();
        let initial_storage: StorageRegion = if ctor_writes.is_empty() {
            self.b.note(
                "initial-state: the constructor writes no storage, so every slot starts at zero",
            );
            (0..self.per_slot.len())
                .map(|i| {
                    self.region_of(i, &IntervalSet::point(U256::ZERO)).unwrap_or(0)
                })
                .collect()
        } else {
            self.b.refuse(
                "the constructor writes storage; P1a resolves only a constructor that leaves \
                 every slot at zero",
            );
            (0..self.per_slot.len()).map(|_| 0).collect()
        };

        // idle states, one per storage region
        for s in &storage_regions {
            let n = format!("idle_{}", self.storage_name(s));
            self.states.insert(n.clone());
            self.marked.insert(n);
        }

        let entries = std::mem::take(&mut self.entries);
        for e in &entries {
            for s in &storage_regions {
                let regions: Vec<Option<usize>> = match e.param {
                    Some(_) => (0..self.arg_regions.len()).map(Some).collect(),
                    None => vec![None],
                };
                for a in regions {
                    if let Err(why) = self.walk_one(e, a, s) {
                        self.b.refuse(format!("{}: {why}", e.signature));
                    }
                }
            }
        }

        let bad = if self.bad_used { vec!["bad".to_string()] } else { vec![] };
        if self.bad_used {
            self.states.insert("bad".into());
        }

        let initial = format!("idle_{}", self.storage_name(&initial_storage));
        self.states.insert(initial.clone());

        let mut states: Vec<String> = self.states.into_iter().collect();
        states.sort();
        let mut marked: Vec<String> = self.marked.into_iter().collect();
        marked.sort();
        let mut events: Vec<EventDecl> = self.events.into_values().collect();
        events.sort_by(|a, b| a.id.cmp(&b.id));
        let mut checks: Vec<CheckDecl> = self.checks.into_values().collect();
        checks.sort_by(|a, b| a.id.cmp(&b.id));

        let description = format!(
            "Generated by mulu from {} ({}). Environment profile {ENVIRONMENT_PROFILE}. \
             Argument regions partition uint256 by the guard and specification predicates; \
             storage regions partition each slot by the specification. \
             Claims about this model are claims about the abstraction, not about the source.",
            self.b.ir.contract, self.b.ir.compiler
        );

        let model = FiniteProduct {
            schema_version: 1,
            kind: "finite-product".into(),
            description: Some(description),
            states,
            initial: Initial::One(initial),
            marked,
            bad,
            events,
            transitions: self.transitions,
            checks,
            control_plant: None,
        };

        let report = AbstractionReport {
            environment_profile: ENVIRONMENT_PROFILE,
            entrypoints_modelled: entries.iter().map(|e| e.signature.clone()).collect(),
            entrypoints_skipped: self
                .b
                .ir
                .entrypoints
                .iter()
                .filter(|e| !entries.iter().any(|m| m.signature == e.signature))
                .map(|e| e.signature.clone())
                .collect(),
            argument_predicates: self.arg_preds,
            storage_predicates: self.storage_preds,
            argument_regions: self
                .arg_regions
                .iter()
                .enumerate()
                .map(|(i, s)| RegionInfo {
                    name: format!("X{i}"),
                    description: format!("argument in {s}"),
                    set: s.clone(),
                })
                .collect(),
            storage_regions: self
                .per_slot
                .iter()
                .flat_map(|(label, rs)| {
                    rs.iter().enumerate().map(move |(i, s)| RegionInfo {
                        name: format!("{}{i}", short(label)),
                        description: format!("{label} in {s}"),
                        set: s.clone(),
                    })
                })
                .collect(),
            discharged: self.b.discharged,
            assumptions: self.b.assumptions,
            unsupported: self.b.unsupported,
        };
        Abstraction { model, report }
    }

    /// One straight-line abstract execution. Every guard is decided by the
    /// argument region, so there is nothing to branch on.
    fn walk_one(
        &mut self,
        e: &Entry,
        arg: Option<usize>,
        entry_storage: &StorageRegion,
    ) -> Result<(), String> {
        let f = self.b.ir.function(&e.func).ok_or("no such function")?.clone();
        let arg_set = arg.map(|a| self.arg_regions[a].clone());
        let call_event = match arg {
            Some(a) => format!("call_{}_{}", e.solidity_name, self.arg_name(a)),
            None => format!("call_{}", e.solidity_name),
        };
        let desc = match &arg_set {
            Some(s) => format!("CallRequest({}, argument in {s})", e.signature),
            None => format!("CallRequest({})", e.signature),
        };
        self.event(&call_event, &desc);

        let from = format!("idle_{}", self.storage_name(entry_storage));
        let suffix = match arg {
            Some(a) => format!("{}_{}", self.arg_name(a), self.storage_name(entry_storage)),
            None => self.storage_name(entry_storage),
        };
        let mut cur = format!("{}#0_{suffix}", e.solidity_name);
        self.add(&from, &call_event, &cur);

        let mut storage = entry_storage.clone();
        let mut step = 0usize;
        let mut seen_checks: Vec<String> = Vec::new();
        let mut block = f.entry;
        let mut index = 0usize;
        let mut visited: BTreeSet<(usize, usize)> = BTreeSet::new();
        let empty = BTreeMap::new();
        let guards = self.guards.get(&e.func).unwrap_or(&empty).clone();

        loop {
            if !visited.insert((block, index)) {
                return Err("the abstract execution revisits a position; P1a does not model loops".into());
            }
            let blk = f.block(block);
            // instructions
            let mut advanced = false;
            while index < blk.instructions.len() {
                let ins = &blk.instructions[index];
                // a guard evaluated here?
                let check = self
                    .b
                    .ir
                    .checks
                    .iter()
                    .find(|c| {
                        c.function == f.id
                            && c.pre_location == block
                            && matches!(c.pass_edge, mulu_yul::ir::CheckEdge::Continue)
                            && ins
                                .storage_write
                                .is_none()
                                && guards.contains_key(&c.id)
                                && instruction_calls(ins, c.helper.as_deref())
                    })
                    .cloned();
                if let Some(c) = check {
                    let p = &guards[&c.id];
                    let var = e.param.clone().unwrap_or_default();
                    let region = arg_set.clone().unwrap_or_else(IntervalSet::full);
                    let outcome = p.decide(&var, &region).ok_or_else(|| {
                        format!(
                            "check {} is not decided by the argument region {region}",
                            c.id
                        )
                    })?;
                    let pass = self.event(&format!("{}_pass", c.id), &format!("GuardResult({}, true)", c.id));
                    let fail = self.event(&format!("{}_fail", c.id), &format!("GuardResult({}, false)", c.id));
                    self.checks.entry(c.id.clone()).or_insert_with(|| CheckDecl {
                        id: c.id.clone(),
                        pass_event: pass.clone(),
                        fail_event: fail.clone(),
                        depends_on: seen_checks.clone(),
                        description: Some(format!("{} ({})", c.condition_text, e.signature)),
                    });
                    if outcome {
                        step += 1;
                        let next = format!("{}#{step}_{suffix}", e.solidity_name);
                        self.add(&cur.clone(), &pass, &next);
                        cur = next;
                        seen_checks.push(c.id.clone());
                    } else {
                        // TxRevert restores the storage of the entry snapshot.
                        let rev = format!(
                            "{}#rev_{}",
                            e.solidity_name,
                            self.storage_name(entry_storage)
                        );
                        self.add(&cur.clone(), &fail, &rev);
                        self.finish_revert(&rev, entry_storage);
                        return Ok(());
                    }
                    index += 1;
                    advanced = true;
                    continue;
                }
                // a storage write?
                if let Some(w) = &ins.storage_write.clone() {
                    let slot_label = self
                        .b
                        .slot_label(&w.slot_text)
                        .ok_or_else(|| format!("a write to slot {} is not a declared variable", w.slot_text))?;
                    let slot_idx = self
                        .per_slot
                        .iter()
                        .position(|(l, _)| *l == slot_label)
                        .ok_or("unknown slot")?;
                    // the value: this function's argument, or a literal
                    let values = if Some(&w.value_text) == e.param.as_ref() {
                        arg_set.clone().ok_or("the argument is written but the function takes none")?
                    } else if let Ok(v) = crate::interval::parse_decimal(&w.value_text) {
                        IntervalSet::point(v)
                    } else {
                        return Err(format!(
                            "the value written to {slot_label} is {}, which P1a cannot resolve to \
                             the argument or a literal",
                            w.value_text
                        ));
                    };
                    let new_region = self.region_of(slot_idx, &values).ok_or_else(|| {
                        format!(
                            "the value written to {slot_label} straddles a specification boundary"
                        )
                    })?;
                    storage[slot_idx] = new_region;
                    let ev = self.event(
                        &format!("store_{slot_label}"),
                        &format!("InternalStep(store {slot_label})"),
                    );
                    step += 1;
                    let next = format!("{}#{step}_{suffix}", e.solidity_name);
                    self.add(&cur.clone(), &ev, &next);
                    cur = next;
                    index += 1;
                    advanced = true;
                    continue;
                }
                index += 1;
            }
            let _ = advanced;
            // terminator
            match &blk.terminator {
                Terminator::Jump { target } => {
                    block = *target;
                    index = 0;
                }
                Terminator::Branch { cond, then_block, else_block } => {
                    let p = translate(cond, &f.parameters).map_err(|w| {
                        format!("a branch condition is outside the P1a fragment: {w}")
                    })?;
                    let var = e.param.clone().unwrap_or_default();
                    let region = arg_set.clone().unwrap_or_else(IntervalSet::full);
                    let taken = p
                        .decide(&var, &region)
                        .ok_or("a branch is not decided by the argument region")?;
                    block = if taken { *then_block } else { *else_block };
                    index = 0;
                }
                Terminator::Revert { .. } => {
                    let rev =
                        format!("{}#rev_{}", e.solidity_name, self.storage_name(entry_storage));
                    let ev = self.event("revert", "TxRevert");
                    self.add(&cur.clone(), &ev, &rev);
                    self.finish_revert(&rev, entry_storage);
                    return Ok(());
                }
                Terminator::Return { .. } | Terminator::Stop | Terminator::Leave => {
                    let ret = format!("{}#ret_{}", e.solidity_name, self.storage_name(&storage));
                    let ev = self.event("return", "TxReturn");
                    self.add(&cur.clone(), &ev, &ret);
                    self.finish_return(&ret, &storage);
                    return Ok(());
                }
                Terminator::Switch { .. } => {
                    return Err("a switch in a function body is outside P1a".into())
                }
                Terminator::Unsupported { reason } => return Err(reason.clone()),
            }
        }
    }

    /// After a revert the transaction boundary is reached with the storage the
    /// call started from.
    fn finish_revert(&mut self, rev: &str, entry_storage: &StorageRegion) {
        self.marked.insert(rev.to_string());
        let ev = self.event("next_tx", "the transaction boundary: begin the next transaction");
        let idle = format!("idle_{}", self.storage_name(entry_storage));
        self.add(rev, &ev, &idle);
    }

    /// After a successful return the specification is evaluated. A violation
    /// goes to `bad`, which is absorbing, so a later transaction cannot undo it.
    fn finish_return(&mut self, ret: &str, storage: &StorageRegion) {
        self.marked.insert(ret.to_string());
        let violated = self.violates_spec(storage);
        let ev = self.event("next_tx", "the transaction boundary: begin the next transaction");
        if violated {
            self.bad_used = true;
            self.add(ret, &ev, "bad");
        } else {
            let idle = format!("idle_{}", self.storage_name(storage));
            self.add(ret, &ev, &idle);
        }
    }

    fn violates_spec(&self, storage: &StorageRegion) -> bool {
        for p in self.b.props {
            let Some(var) = &p.var else { continue };
            let Some(slot) = self.per_slot.iter().position(|(l, _)| l == var) else { continue };
            let region = &self.per_slot[slot].1[storage[slot]];
            if region.disjoint_from(&p.predicate.set()) {
                return true;
            }
        }
        false
    }
}

fn instruction_calls(ins: &mulu_yul::ir::Instruction, name: Option<&str>) -> bool {
    let Some(name) = name else { return false };
    let expr = match &ins.op {
        mulu_yul::ir::Op::Effect { call } => call,
        _ => return false,
    };
    let mut calls = Vec::new();
    expr.calls(&mut calls);
    calls.iter().any(|c| c == name)
}

fn short(label: &str) -> String {
    label.chars().take(3).collect::<String>().to_uppercase()
}
