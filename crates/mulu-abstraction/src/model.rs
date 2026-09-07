//! Predicate abstraction: ProgramIR plus a specification, out comes the
//! `finite-product` model the P0 core already analyses.
//!
//! The construction follows docs/11: event schemas (§2), the state and what
//! each event preserves (§3), the environment profile (§5) and the interval
//! partition worked through in §7. Every step that could quietly lose a
//! behaviour instead records an assumption or refuses.

use crate::interval::{IntervalSet, U256};
use crate::predicate::Predicate;
use crate::spec::{storage_vars, CompiledProperty, StorageVar};
use mulu_model::schema::{
    CheckDecl, Control, ControlPlant, ControlSite, EventDecl, FiniteProduct, Initial, SitePair,
    Transition,
};
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

/// What the model's name for an entrypoint stands for. Turning an abstract
/// counterexample into calls needs this: the path names entrypoints the way
/// the model does, and a replay needs the signature and the selector.
#[derive(Debug, Clone, Serialize)]
pub struct EntrypointInfo {
    /// The name the model's states and events are built from.
    pub model_name: String,
    pub signature: String,
    pub selector: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub param_type: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AbstractionReport {
    pub environment_profile: &'static str,
    pub entrypoints: Vec<EntrypointInfo>,
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
    /// Every parameter, in order. This was one `Option<String>`: a function
    /// of two arguments had nothing for the abstraction to be a function of.
    params: Vec<Param>,
    /// The 4-byte selector, which is unique where a name need not be.
    selector: String,
}

#[derive(Debug, Clone)]
struct Param {
    /// The Yul variable the body reads it through.
    name: String,
    /// The ABI type, which fixes the domain.
    ty: String,
    /// The values it can take. docs/11 §5 admits type-correct calls only, so
    /// this is the domain the abstraction may reason over.
    domain: IntervalSet,
}

impl Entry {
    /// The values every argument can take at once, which is what a guard is
    /// decided against before any region is chosen.
    fn domains(&self) -> crate::value::Env {
        self.params.iter().map(|p| (p.name.clone(), p.domain.clone())).collect()
    }

    /// The widest single domain, for the places that still summarise the
    /// call by one set (predicate translation over the body's parameters).
    fn widest(&self) -> IntervalSet {
        self.params
            .iter()
            .map(|p| p.domain.clone())
            .reduce(|a, b| if a.subset_of(&b) { b } else { a })
            .unwrap_or_else(IntervalSet::full)
    }
}

/// The ABI parameter types of a signature: `setLimit(uint256)` -> `[uint256]`.
pub fn signature_params(sig: &str) -> Vec<String> {
    let Some(open) = sig.find('(') else { return vec![] };
    let Some(close) = sig.rfind(')') else { return vec![] };
    if close <= open + 1 {
        return vec![];
    }
    let inner = &sig[open + 1..close];
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut cur = String::new();
    for c in inner.chars() {
        match c {
            '(' | '[' => {
                depth += 1;
                cur.push(c);
            }
            ')' | ']' => {
                depth = depth.saturating_sub(1);
                cur.push(c);
            }
            ',' if depth == 0 => {
                out.push(cur.trim().to_string());
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }
    out
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

    /// The functions an entrypoint body actually executes: itself plus every
    /// function it calls as a statement, transitively. A modifier and the
    /// `_inner` body solc splits out both live here, and their guards and
    /// storage writes must refine the partition exactly like the entry's own.
    fn reachable(&self, entry: &str) -> Vec<String> {
        let mut seen = BTreeSet::new();
        let mut queue = vec![entry.to_string()];
        while let Some(name) = queue.pop() {
            if !seen.insert(name.clone()) {
                continue;
            }
            let Some(f) = self.ir.function(&name) else { continue };
            for b in &f.blocks {
                for ins in &b.instructions {
                    // Only statement calls; a guard helper is a check, and a
                    // store helper is already a recognised write.
                    if ins.storage_write.is_some() {
                        continue;
                    }
                    if let Some((callee, _)) = statement_call(ins) {
                        if self.ir.checks.iter().any(|c| c.helper.as_deref() == Some(callee.as_str())) {
                            continue;
                        }
                        if self.ir.function(&callee).is_some() {
                            queue.push(callee);
                        }
                    }
                }
            }
        }
        seen.into_iter().collect()
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
            if !f.effects.supported() {
                self.refuse(format!(
                    "entrypoint {} reaches {:?}, which P1a does not model",
                    e.signature, f.effects.unsupported
                ));
                continue;
            }
            // The argument's domain comes from its ABI type, not from the
            // machine word. A uint8 argument has 256 values; treating it as a
            // full word invents regions no type-correct call can reach.
            let abi_params = signature_params(&e.signature);
            if abi_params.len() != f.parameters.len() {
                self.refuse(format!(
                    "entrypoint {}: the ABI lists {} parameter(s) but the body takes {}",
                    e.signature,
                    abi_params.len(),
                    f.parameters.len()
                ));
                continue;
            }
            let mut params = Vec::new();
            let mut bad = false;
            for (ty, name) in abi_params.iter().zip(f.parameters.iter()) {
                match crate::types::domain_of(ty) {
                    Ok(domain) => {
                        params.push(Param { name: name.clone(), ty: ty.clone(), domain })
                    }
                    Err(why) => {
                        self.refuse(format!("entrypoint {}: {why}", e.signature));
                        bad = true;
                        break;
                    }
                }
            }
            if bad {
                continue;
            }
            out.push(Entry {
                solidity_name: f
                    .solidity_name
                    .clone()
                    .unwrap_or_else(|| f.id.clone()),
                signature: e.signature.clone(),
                func: f.id.clone(),
                params,
                selector: e.selector.clone(),
            });
        }
        out
    }

    /// Guard predicates of a function, keyed by check id.
    pub(crate) fn guards(
        &mut self,
        f: &Function,
        domain: &IntervalSet,
    ) -> BTreeMap<String, Predicate> {
        let mut out = BTreeMap::new();
        for c in self.ir.checks.iter().filter(|c| c.function == f.id) {
            if c.purity != Purity::Pure {
                self.refuse(format!(
                    "check {} in {} reads {:?}; P1a guards must be a function of the argument alone",
                    c.id, f.id, c.purity
                ));
                continue;
            }
            match crate::predicate::translate_in(&c.condition, &f.parameters, domain) {
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
        let mut entries = self.entries();
        disambiguate(&mut entries);

        // --- storage partition, one per slot, refined by the specification
        let mut storage_preds: Vec<PredicateInfo> = Vec::new();
        let mut per_slot: Vec<(String, Vec<IntervalSet>)> = Vec::new();
        for v in self.storage.clone() {
            let mut sets: Vec<(String, IntervalSet)> = Vec::new();
            // A slot cannot hold values its type has no room for.
            let type_label = v.type_label.clone().unwrap_or_else(|| v.type_id.clone());
            if let Ok(d) = crate::types::domain_of(&type_label) {
                if !d.is_full() {
                    storage_preds.push(PredicateInfo {
                        id: format!("type:{}", v.label),
                        source: format!("the declared type of {}", v.label),
                        text: format!("{} is {type_label}", v.label),
                        set: d.clone(),
                    });
                    sets.push((format!("type:{}", v.label), d));
                }
            }
            sets.extend(
                self.props
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
                    }),
            );
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
            for fname in self.reachable(&e.func) {
                let Some(f) = self.ir.function(&fname) else { continue };
                let f = f.clone();
                let g = self.guards(&f, &e.widest());
                for (id, p) in &g {
                    let set = p.set();
                    if !set.is_full() && !set.is_empty() && !arg_sets.iter().any(|(n, _)| n == id) {
                        arg_preds.push(PredicateInfo {
                            id: id.clone(),
                            source: format!("check {id} reached from {}", e.solidity_name),
                            text: format!("{p}"),
                            set: set.clone(),
                        });
                        arg_sets.push((id.clone(), set));
                    }
                }
                guards_by_func.insert(fname.clone(), g);

                // pull the spec back through `slot := argument`
                for b in &f.blocks {
                    for ins in &b.instructions {
                        let Some(w) = &ins.storage_write else { continue };
                        // the write carries this function's own parameter,
                        // directly or through a cleanup
                        let mut reads = Vec::new();
                        w.value.idents(&mut reads);
                        if !reads.iter().any(|r| f.parameters.contains(r)) {
                            continue;
                        }
                        let Some(label) = self.slot_label(&w.slot_text) else { continue };
                        for p in self.props.iter().filter(|p| p.var.as_deref() == Some(label.as_str())) {
                            let id = format!("spec:{}", p.id);
                            if !arg_sets.iter().any(|(n, _)| *n == id) {
                                arg_preds.push(PredicateInfo {
                                    id: id.clone(),
                                    source: format!(
                                        "spec property {} pulled back through {}: {} := the argument",
                                        p.id, e.solidity_name, label
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
        }
        // Each entrypoint's type domain refines the partition too, so every
        // region is wholly inside or wholly outside the values a given
        // entrypoint can receive.
        for e in &entries {
            for p in &e.params {
                if p.domain.is_full() {
                    continue;
                }
                let id = format!("type:{}", p.ty);
                if !arg_sets.iter().any(|(n, _)| *n == id) {
                    arg_preds.push(PredicateInfo {
                        id: id.clone(),
                        source: format!("the ABI type {} of an argument of {}", p.ty, e.signature),
                        text: format!("the argument is {}", p.ty),
                        set: p.domain.clone(),
                    });
                    arg_sets.push((id, p.domain.clone()));
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
        self.note(
            "typed-domains: each argument ranges over the values its ABI type admits and each \
             slot over the values its declared type admits, matching the profile's type-correct \
             calls rather than the whole machine word",
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
    plant_states: BTreeSet<String>,
    plant_marked: BTreeSet<String>,
    plant_accepting: BTreeSet<String>,
    plant_events: BTreeMap<String, EventDecl>,
    plant_transitions: Vec<Transition>,
    plant_sites: BTreeMap<String, ControlSite>,
    plant_bad_used: bool,
}

/// One position of an abstract execution along the path where guards pass.
enum Step {
    Check { id: String, text: String, passes: bool, depends_on: Vec<String> },
    Store { slot: usize, label: String, to: usize },
}

enum Ending {
    Return,
    Revert,
}

struct Trace {
    steps: Vec<Step>,
    ending: Ending,
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
            plant_states: BTreeSet::new(),
            plant_marked: BTreeSet::new(),
            plant_accepting: BTreeSet::new(),
            plant_events: BTreeMap::new(),
            plant_transitions: Vec::new(),
            plant_sites: BTreeMap::new(),
            plant_bad_used: false,
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

    /// The storage the deployment leaves, when every constructor write
    /// resolves to a constant slot and a constant value. `Err` names the
    /// first write that does not, because "the constructor writes storage"
    /// told a reader nothing about which write or why.
    fn constructor_storage(
        &self,
        writes: &[&mulu_yul::ir::Instruction],
    ) -> Result<BTreeMap<usize, U256>, String> {
        let mut out = BTreeMap::new();
        for i in writes {
            let Some(w) = &i.storage_write else {
                return Err(
                    "an instruction writes storage in a way the front end did not recognise as \
                     a slot and a value"
                        .to_string(),
                );
            };
            let slot = mulu_yul::fold::fold_fixpoint(&w.slot);
            let value = mulu_yul::fold::fold_fixpoint(&w.value);
            let Some(slot) = crate::value::constant(&slot) else {
                return Err(format!("the slot in `{}` is not a constant", w.slot_text));
            };
            let Some(value) = crate::value::constant(&value) else {
                return Err(format!(
                    "the value written to slot {slot} is `{}`, which is not a constant",
                    w.value_text
                ));
            };
            let Ok(slot) = usize::try_from(slot) else {
                return Err(format!("slot {slot} is beyond the layout this models"));
            };
            if slot >= self.per_slot.len() {
                return Err(format!("slot {slot} is not in the contract's storage layout"));
            }
            // A later write wins, which is the order the constructor runs in.
            out.insert(slot, value);
        }
        Ok(out)
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
        // A constructor that writes constants is as determined as one that
        // writes nothing: `x = 5` leaves slot 0 at 5, and refusing it threw
        // away every contract with an initialiser. What is refused is a
        // constructor whose writes depend on something the model does not
        // have, such as `owner = msg.sender`.
        let initial_storage: StorageRegion = match self.constructor_storage(&ctor_writes) {
            Ok(values) => {
                if ctor_writes.is_empty() {
                    self.b.note(
                        "initial-state: the constructor writes no storage, so every slot starts \
                         at zero",
                    );
                } else {
                    self.b.note(
                        "initial-state: every constructor write resolved to a constant, so the \
                         model starts where the deployment leaves the contract",
                    );
                }
                (0..self.per_slot.len())
                    .map(|i| {
                        let v = values.get(&i).copied().unwrap_or(U256::ZERO);
                        self.region_of(i, &IntervalSet::point(v)).unwrap_or(0)
                    })
                    .collect()
            }
            Err(why) => {
                self.b.refuse(&format!(
                    "the constructor's effect on storage is not determined: {why}"
                ));
                (0..self.per_slot.len()).map(|_| 0).collect()
            }
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
                // One choice of region per parameter, so the walk covers
                // the product. A parameter admits only the regions its type
                // can receive, which is what keeps the product from being
                // the whole partition raised to the arity.
                let per_param: Vec<Vec<usize>> = e
                    .params
                    .iter()
                    .map(|p| {
                        (0..self.arg_regions.len())
                            .filter(|i| self.arg_regions[*i].subset_of(&p.domain))
                            .collect()
                    })
                    .collect();
                for a in product(&per_param) {
                    // One walk feeds both models, so their states line up and
                    // the pairing in `sites` means what it says.
                    match self.trace(e, &a, s) {
                        Ok(t) => {
                            self.emit_impl(e, &a, s, &t);
                            self.emit_plant(e, &a, s, &t);
                        }
                        Err(why) => self.b.refuse(format!("{}: {why}", e.signature)),
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

        // Names the plant shares with the implementation, taken before any
        // field of `self` is moved out below.
        let idle_names: Vec<String> =
            storage_regions.iter().map(|s| format!("idle_{}", self.storage_name(s))).collect();

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

        // The reference plant, when the walk produced one. Its envelope is
        // what an overrestriction candidate is measured against.
        let plant_bad = self.plant_bad_used;
        if plant_bad {
            self.plant_states.insert("bad".into());
        }
        let control_plant = if self.plant_transitions.is_empty() {
            None
        } else {
            let mut pstates: Vec<String> = self.plant_states.into_iter().collect();
            pstates.sort();
            let mut pmarked: Vec<String> = self.plant_marked.into_iter().collect();
            pmarked.sort();
            let mut paccepting: Vec<String> = self.plant_accepting.into_iter().collect();
            paccepting.sort();
            let mut pevents: Vec<EventDecl> = self.plant_events.into_values().collect();
            pevents.sort_by(|a, b| a.id.cmp(&b.id));
            let mut sites: Vec<ControlSite> = self.plant_sites.into_values().collect();
            sites.sort_by(|a, b| a.id.cmp(&b.id));
            // idle states belong to the plant too: it shares the boundary.
            let mut all = pstates;
            for n in &idle_names {
                if !all.contains(n) {
                    all.push(n.clone());
                }
                if !pmarked.contains(n) {
                    pmarked.push(n.clone());
                }
            }
            all.sort();
            pmarked.sort();
            Some(ControlPlant {
                states: all,
                initial: Initial::One(initial.clone()),
                marked: pmarked,
                accepting: Some(paccepting),
                bad: if plant_bad { vec!["bad".to_string()] } else { vec![] },
                events: pevents,
                transitions: self.plant_transitions,
                sites,
            })
        };

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
            control_plant,
        };

        let report = AbstractionReport {
            environment_profile: ENVIRONMENT_PROFILE,
            entrypoints: entries
                .iter()
                .map(|e| EntrypointInfo {
                    model_name: e.solidity_name.clone(),
                    signature: e.signature.clone(),
                    selector: e.selector.clone(),
                    param_type: e.params.first().map(|p| p.ty.clone()),
                })
                .collect(),
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

    /// One abstract execution along the path where every guard passes.
    ///
    /// Both models are built from this: the implementation stops at the first
    /// guard the region makes fail, the reference plant keeps a controllable
    /// continue and an uncontrollable reject at each site instead. Walking
    /// once keeps them in step, which is what makes the state pairing in
    /// `sites` meaningful.
    fn trace(
        &mut self,
        e: &Entry,
        args: &[usize],
        entry_storage: &StorageRegion,
    ) -> Result<Trace, String> {
        const MAX_DEPTH: usize = 32;
        const MAX_STEPS: usize = 10_000;

        let mut storage = entry_storage.clone();
        let mut steps: Vec<Step> = Vec::new();
        let mut seen_checks: Vec<String> = Vec::new();
        let mut frames: Vec<Frame> = vec![Frame {
            func: e.func.clone(),
            block: 0,
            index: 0,
            env: e
                .params
                .iter()
                .zip(args.iter())
                .map(|(p, a)| (p.name.clone(), self.arg_regions[*a].clone()))
                .collect(),
            visited: BTreeSet::new(),
        }];
        let mut n = 0usize;

        loop {
            n += 1;
            if n > MAX_STEPS {
                return Err("the abstract execution did not terminate within the step limit".into());
            }
            let depth = frames.len();
            let (func, block, index, env) = {
                let fr = frames.last().expect("a frame");
                (fr.func.clone(), fr.block, fr.index, fr.env.clone())
            };
            let f = self.b.ir.function(&func).ok_or("no such function")?.clone();
            let blk = f.block(block).clone();

            if index < blk.instructions.len() {
                frames.last_mut().unwrap().index += 1;
                let ins = &blk.instructions[index];

                if let Some(c) = self.check_at(&f, block, index, ins) {
                    let guards = self.guards_of(&f, &e.widest());
                    let Some(p) = guards.get(&c.id) else {
                        return Err(format!("check {} could not be turned into a predicate", c.id));
                    };
                    let passes = p.decide(&env).ok_or_else(|| {
                        format!("check {} is not decided by the argument regions {}", c.id, show(&env))
                    })?;
                    steps.push(Step::Check {
                        id: c.id.clone(),
                        text: c.condition_text.clone(),
                        passes,
                        depends_on: seen_checks.clone(),
                    });
                    seen_checks.push(c.id.clone());
                    continue;
                }

                if let Some(w) = ins.storage_write.clone() {
                    let (slot_idx, slot_label) = self.slot_of(&w.slot_text)?;
                    let values = crate::value::value_set(&w.value, &env)
                        .map_err(|why| {
                            format!("the value written to {slot_label} is {}: {why}", w.value_text)
                        })?;
                    let to = self.region_of(slot_idx, &values).ok_or_else(|| {
                        format!("the value written to {slot_label} straddles a specification boundary")
                    })?;
                    storage[slot_idx] = to;
                    steps.push(Step::Store { slot: slot_idx, label: slot_label, to });
                    continue;
                }

                if let Some((callee, args)) = statement_call(ins) {
                    if let Some(g) = self.b.ir.function(&callee) {
                        let matters = g.effects.writes_storage
                            || g.effects.can_revert
                            || self.b.ir.checks.iter().any(|c| c.function == callee);
                        if matters {
                            if depth >= MAX_DEPTH {
                                return Err(format!("call depth limit reached at {callee}"));
                            }
                            let inner = bind_arguments(g, &args, &env)?;
                            frames.push(Frame {
                                func: callee,
                                block: 0,
                                index: 0,
                                env: inner,
                                visited: BTreeSet::new(),
                            });
                            continue;
                        }
                    }
                }

                if ins.effects.writes_storage || ins.effects.can_revert {
                    return Err(format!(
                        "an instruction in {func} carries effects the model does not represent \
                         ({}{}); P1a cannot skip it",
                        if ins.effects.writes_storage { "writes storage" } else { "" },
                        if ins.effects.can_revert { " can revert" } else { "" },
                    ));
                }
                continue;
            }

            match &blk.terminator {
                Terminator::Jump { target } => {
                    let fr = frames.last_mut().unwrap();
                    if !fr.visited.insert((*target, 0)) {
                        return Err("the abstract execution revisits a block; P1a does not model loops".into());
                    }
                    fr.block = *target;
                    fr.index = 0;
                }
                Terminator::Branch { cond, then_block, else_block } => {
                    // A guard written as `if (..) revert()` sits on the
                    // terminator rather than on a helper call. Treating it as
                    // an ordinary branch would leave it out of both models:
                    // no redundancy verdict, and no control site to
                    // parameterise it out of the plant.
                    if let Some(c) = self.branch_check(&f, block) {
                        let guards = self.guards_of(&f, &e.widest());
                        let Some(p) = guards.get(&c.id) else {
                            return Err(format!(
                                "check {} could not be turned into a predicate",
                                c.id
                            ));
                        };
                        let passes = p.decide(&env).ok_or_else(|| {
                            format!(
                                "check {} is not decided by the argument regions {}",
                                c.id,
                                show(&env)
                            )
                        })?;
                        steps.push(Step::Check {
                            id: c.id.clone(),
                            text: c.condition_text.clone(),
                            passes,
                            depends_on: seen_checks.clone(),
                        });
                        seen_checks.push(c.id.clone());
                        let target = match (&c.pass_edge, &c.fail_edge) {
                            (
                                mulu_yul::ir::CheckEdge::Block { id: pass },
                                mulu_yul::ir::CheckEdge::Block { .. },
                            ) => *pass,
                            _ => return Err(format!("check {} has no block edges", c.id)),
                        };
                        let fr = frames.last_mut().unwrap();
                        if !fr.visited.insert((target, 0)) {
                            return Err("the abstract execution revisits a block; P1a does not model loops".into());
                        }
                        fr.block = target;
                        fr.index = 0;
                        continue;
                    }
                    let p = crate::predicate::translate_in(cond, &f.parameters, &e.widest())
                        .map_err(|w| {
                        format!("a branch condition is outside the P1a fragment: {w}")
                    })?;
                    let taken = p
                        .decide(&env)
                        .ok_or("a branch is not decided by the argument regions")?;
                    let target = if taken { *then_block } else { *else_block };
                    let fr = frames.last_mut().unwrap();
                    if !fr.visited.insert((target, 0)) {
                        return Err("the abstract execution revisits a block; P1a does not model loops".into());
                    }
                    fr.block = target;
                    fr.index = 0;
                }
                Terminator::Revert { .. } => {
                    return Ok(Trace { steps, ending: Ending::Revert })
                }
                Terminator::Return { .. } | Terminator::Stop => {
                    return Ok(Trace { steps, ending: Ending::Return })
                }
                Terminator::Leave => {
                    frames.pop();
                    if frames.is_empty() {
                        return Ok(Trace { steps, ending: Ending::Return });
                    }
                }
                Terminator::Switch { .. } => {
                    return Err("a switch in a function body is outside P1a".into())
                }
                Terminator::Unsupported { reason } => return Err(reason.clone()),
            }
        }
    }

    /// The implementation model: guard results follow from the code, so the
    /// walk stops at the first one the region makes fail.
    fn emit_impl(
        &mut self,
        e: &Entry,
        arg: &[usize],
        entry_storage: &StorageRegion,
        t: &Trace,
    ) {
        let suffix = self.suffix(e, arg, entry_storage);
        let call_event = self.call_event(e, arg);
        let from = format!("idle_{}", self.storage_name(entry_storage));
        let mut cur = format!("{}#0_{suffix}", e.solidity_name);
        self.add(&from, &call_event.clone(), &cur);

        // Declare every check on the path, including those beyond the point
        // the implementation stops at. A check no region reaches is dead code,
        // and the core reports it as unreachable; leaving it undeclared would
        // instead leave the plant's site pointing at nothing.
        for step in &t.steps {
            if let Step::Check { id, text, depends_on, .. } = step {
                let pass = self.event(&format!("{id}_pass"), &format!("GuardResult({id}, true)"));
                let fail = self.event(&format!("{id}_fail"), &format!("GuardResult({id}, false)"));
                self.checks.entry(id.clone()).or_insert_with(|| CheckDecl {
                    id: id.clone(),
                    pass_event: pass,
                    fail_event: fail,
                    depends_on: depends_on.clone(),
                    description: Some(format!("{text} ({})", e.signature)),
                });
            }
        }

        let mut storage = entry_storage.clone();
        for (i, step) in t.steps.iter().enumerate() {
            let next = format!("{}#{}_{suffix}", e.solidity_name, i + 1);
            match step {
                Step::Check { id, passes, .. } => {
                    let pass = format!("{id}_pass");
                    let fail = format!("{id}_fail");
                    if *passes {
                        self.add(&cur.clone(), &pass, &next);
                    } else {
                        let rev = format!(
                            "{}#rev_{}",
                            e.solidity_name,
                            self.storage_name(entry_storage)
                        );
                        self.add(&cur.clone(), &fail, &rev);
                        self.finish_revert(&rev, entry_storage);
                        return;
                    }
                }
                Step::Store { slot, label, to } => {
                    storage[*slot] = *to;
                    let ev = self
                        .event(&format!("store_{label}"), &format!("InternalStep(store {label})"));
                    self.add(&cur.clone(), &ev, &next);
                }
            }
            cur = next;
        }
        match t.ending {
            Ending::Return => {
                let ret = format!("{}#ret_{}", e.solidity_name, self.storage_name(&storage));
                let ev = self.event("return", "TxReturn");
                self.add(&cur.clone(), &ev, &ret);
                self.finish_return(&ret, &storage);
            }
            Ending::Revert => {
                let rev = format!("{}#rev_{}", e.solidity_name, self.storage_name(entry_storage));
                let ev = self.event("revert", "TxRevert");
                self.add(&cur.clone(), &ev, &rev);
                self.finish_revert(&rev, entry_storage);
            }
        }
    }

    /// The conservative reference plant of docs/11 §4: at each control site
    /// the supervisor may forbid continuing, and rejecting stays possible
    /// whatever it decides. The guard's own condition plays no part here,
    /// which is exactly what "parameterised out" means.
    fn emit_plant(
        &mut self,
        e: &Entry,
        arg: &[usize],
        entry_storage: &StorageRegion,
        t: &Trace,
    ) {
        let suffix = self.suffix(e, arg, entry_storage);
        let call_event = self.call_event(e, arg);
        self.plant_event(&call_event, "CallRequest", Control::Uncontrollable);
        let from = format!("idle_{}", self.storage_name(entry_storage));
        let rev = format!("{}@rev_{}", e.solidity_name, self.storage_name(entry_storage));
        let mut cur = format!("{}@0_{suffix}", e.solidity_name);
        self.plant_add(&from, &call_event, &cur);

        // An entrypoint that changes storage without a guard still has a place
        // a guard could go: the entry. Without it the plant cannot express
        // "this needs a check", which is the whole question for forceSet.
        // A read-only entrypoint gets none: there is nothing there to forbid.
        let writes = t.steps.iter().any(|s| matches!(s, Step::Store { .. }));
        let guarded = t.steps.iter().any(|s| matches!(s, Step::Check { .. }));
        if writes && !guarded {
            let site = format!("entry_{}", e.solidity_name);
            let cont = format!("cont_{site}");
            let rej = format!("rej_{site}");
            self.plant_event(&cont, "Continue(entry site)", Control::Controllable);
            self.plant_event(&rej, "Reject(entry site)", Control::Uncontrollable);
            let next = format!("{}@e_{suffix}", e.solidity_name);
            self.plant_add(&cur.clone(), &cont, &next);
            self.plant_add(&cur.clone(), &rej, &rev);
            self.plant_sites.entry(site.clone()).or_insert(ControlSite {
                id: site,
                check: None,
                continue_event: cont,
                pairs: vec![],
            });
            cur = next;
        }

        let mut storage = entry_storage.clone();
        // The implementation stops at the first guard the region makes fail,
        // so beyond that point it has no state to pair a site with.
        let mut impl_reaches = true;
        for (i, step) in t.steps.iter().enumerate() {
            let next = format!("{}@{}_{suffix}", e.solidity_name, i + 1);
            match step {
                Step::Check { id, passes, .. } => {
                    let cont = format!("cont_{id}");
                    let rej = format!("rej_{id}");
                    self.plant_event(&cont, &format!("Continue(site {id})"), Control::Controllable);
                    // docs/11 §4: rejection stays available whatever the
                    // supervisor allows, which is what makes this plant
                    // conservative rather than the exact implementation.
                    self.plant_event(&rej, &format!("Reject(site {id})"), Control::Uncontrollable);
                    self.plant_add(&cur.clone(), &cont, &next);
                    self.plant_add(&cur.clone(), &rej, &rev);
                    let impl_state = format!("{}#{}_{suffix}", e.solidity_name, i);
                    let reached = impl_reaches;
                    let entry = self.plant_sites.entry(id.clone()).or_insert(ControlSite {
                        id: id.clone(),
                        check: Some(id.clone()),
                        continue_event: cont,
                        pairs: vec![],
                    });
                    if reached && !entry.pairs.iter().any(|p| p.plant_state == cur) {
                        entry.pairs.push(SitePair { plant_state: cur.clone(), impl_state });
                    }
                    if !*passes {
                        impl_reaches = false;
                    }
                }
                Step::Store { slot, label, to } => {
                    storage[*slot] = *to;
                    let ev = format!("store_{label}");
                    self.plant_event(&ev, &format!("InternalStep(store {label})"), Control::Uncontrollable);
                    self.plant_add(&cur.clone(), &ev, &next);
                }
            }
            cur = next;
        }
        match t.ending {
            Ending::Return => {
                let ret = format!("{}@ret_{}", e.solidity_name, self.storage_name(&storage));
                self.plant_event("return", "TxReturn", Control::Uncontrollable);
                self.plant_add(&cur.clone(), "return", &ret);
                self.plant_finish(&ret, &storage, true);
            }
            Ending::Revert => {
                self.plant_event("revert", "TxRevert", Control::Uncontrollable);
                self.plant_add(&cur.clone(), "revert", &rev);
            }
        }
        self.plant_finish(&rev, entry_storage, false);
    }

    /// The part of a state's name that says which call it is: one region
    /// name per argument, then the storage region. With no arguments it is
    /// just the storage, as before.
    fn suffix(&self, _e: &Entry, arg: &[usize], s: &StorageRegion) -> String {
        if arg.is_empty() {
            return self.storage_name(s);
        }
        let a: Vec<String> = arg.iter().map(|i| self.arg_name(*i)).collect();
        format!("{}_{}", a.join("."), self.storage_name(s))
    }

    fn call_event(&mut self, e: &Entry, arg: &[usize]) -> String {
        // `#` cannot occur in a Solidity identifier, so the entrypoint and the
        // region it is called with stay separable. Joining them with `_` let a
        // function actually named `f_X0` share an event with `f` called on
        // region X0, and one event with two targets is not a model the
        // supervisory-control core accepts.
        let id = if arg.is_empty() {
            format!("call_{}", e.solidity_name)
        } else {
            let a: Vec<String> = arg.iter().map(|i| self.arg_name(*i)).collect();
            format!("call_{}#{}", e.solidity_name, a.join("."))
        };
        let desc = if arg.is_empty() {
            format!("CallRequest({})", e.signature)
        } else {
            let parts: Vec<String> = e
                .params
                .iter()
                .zip(arg.iter())
                .map(|(p, i)| format!("{} in {}", p.name, self.arg_regions[*i]))
                .collect();
            format!("CallRequest({}, {})", e.signature, parts.join(", "))
        };
        self.event(&id, &desc)
    }

    fn plant_event(&mut self, id: &str, description: &str, control: Control) {
        self.plant_events.entry(id.to_string()).or_insert_with(|| EventDecl {
            id: id.to_string(),
            control,
            description: Some(description.to_string()),
        });
    }

    fn plant_add(&mut self, from: &str, event: &str, to: &str) {
        self.plant_states.insert(from.to_string());
        self.plant_states.insert(to.to_string());
        let t = Transition { from: from.to_string(), event: event.to_string(), to: to.to_string() };
        if !self
            .plant_transitions
            .iter()
            .any(|x| x.from == t.from && x.event == t.event && x.to == t.to)
        {
            self.plant_transitions.push(t);
        }
    }

    /// A terminal phase of the plant, and the boundary transition after it.
    /// `accepting` marks the ends that completed the request, which is what
    /// tells an overrestriction candidate from a rejection nobody wanted.
    fn plant_finish(&mut self, state: &str, storage: &StorageRegion, accepting: bool) {
        self.plant_marked.insert(state.to_string());
        if accepting {
            self.plant_accepting.insert(state.to_string());
        }
        self.plant_event("next_tx", "the transaction boundary", Control::Uncontrollable);
        // The plant carries the same monitor as the implementation. Without
        // it nothing is unsafe, the envelope forbids nothing, and every
        // rejection looks like an overrestriction.
        if accepting && self.violates_spec(storage) {
            self.plant_bad_used = true;
            self.plant_add(state, "next_tx", "bad");
        } else {
            let idle = format!("idle_{}", self.storage_name(storage));
            self.plant_add(state, "next_tx", &idle);
        }
    }

    fn slot_of(&self, slot_text: &str) -> Result<(usize, String), String> {
        let label = self
            .b
            .slot_label(slot_text)
            .ok_or_else(|| format!("a write to slot {slot_text} is not a declared variable"))?;
        let idx = self
            .per_slot
            .iter()
            .position(|(l, _)| *l == label)
            .ok_or_else(|| format!("slot {label} is not partitioned"))?;
        Ok((idx, label))
    }

    /// Guards of a function, computed on demand as the walk enters it.
    fn guards_of(&mut self, f: &Function, domain: &IntervalSet) -> BTreeMap<String, Predicate> {
        if let Some(g) = self.guards.get(&f.id) {
            return g.clone();
        }
        let g = self.b.guards(f, domain);
        self.guards.insert(f.id.clone(), g.clone());
        g
    }

    /// The check this block's branch terminator evaluates, if any.
    fn branch_check(&self, f: &Function, block: usize) -> Option<mulu_yul::Check> {
        self.b
            .ir
            .checks
            .iter()
            .find(|c| {
                c.function == f.id
                    && c.pre_location == block
                    && c.pre_instruction.is_none()
                    && matches!(c.pass_edge, mulu_yul::ir::CheckEdge::Block { .. })
            })
            .cloned()
    }

    /// The check evaluated by this instruction, if it is a guard-helper call.
    fn check_at(
        &self,
        f: &Function,
        block: usize,
        index: usize,
        ins: &mulu_yul::ir::Instruction,
    ) -> Option<mulu_yul::Check> {
        self.b
            .ir
            .checks
            .iter()
            .find(|c| {
                c.function == f.id
                    // Position, not just the block: two guards can share one,
                    // and matching on the helper name alone substitutes the
                    // first guard's condition for the second's when both
                    // `require`s lower to the same helper.
                    && c.pre_location == block
                    && c.pre_instruction == Some(index)
                    && matches!(c.pass_edge, mulu_yul::ir::CheckEdge::Continue)
                    && ins.storage_write.is_none()
                    && instruction_calls(ins, c.helper.as_deref())
            })
            .cloned()
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
            let Some(var) = &p.var else {
                // A property with no variable is constant. `false` forbids
                // every successful end; skipping it would read a
                // specification that permits nothing as one that permits
                // everything.
                if p.predicate == Predicate::False {
                    return true;
                }
                continue;
            };
            let Some(slot) = self.per_slot.iter().position(|(l, _)| l == var) else { continue };
            let region = &self.per_slot[slot].1[storage[slot]];
            if region.disjoint_from(&p.predicate.set()) {
                return true;
            }
        }
        false
    }
}

/// Overloaded functions share a Solidity name, so the states and events built
/// from it would merge two different control flows into one. Give each the
/// parameter types that tell them apart.
fn disambiguate(entries: &mut [Entry]) {
    // Two entrypoints sharing a name share their states, merging two control
    // flows into one, so the result has to be injective. A readable name is
    // preferred, but correctness is not traded for it: each candidate is
    // taken only if nothing has claimed it.
    let mut taken: BTreeSet<String> = BTreeSet::new();
    let shared: BTreeSet<String> = {
        let mut seen = BTreeSet::new();
        let mut dup = BTreeSet::new();
        for e in entries.iter() {
            if !seen.insert(e.solidity_name.clone()) {
                dup.insert(e.solidity_name.clone());
            }
        }
        dup
    };

    for e in entries.iter_mut() {
        let base = e.solidity_name.clone();
        let params = signature_params(&e.signature).join("_");
        let sel = e.selector.trim_start_matches("0x").to_string();
        // In order of preference; the last is unique because a selector is.
        let mut candidates = Vec::new();
        if !shared.contains(&base) {
            candidates.push(base.clone());
        }
        if !params.is_empty() {
            candidates.push(format!("{base}_{params}"));
        }
        candidates.push(format!("{base}_{sel}"));
        candidates.push(format!("fn_{sel}"));

        let chosen = candidates
            .into_iter()
            .find(|c| !taken.contains(c))
            .unwrap_or_else(|| {
                // Only reachable if a source name already reads like every
                // fallback. Number it rather than collide.
                let mut n = 0usize;
                loop {
                    let c = format!("fn_{sel}_{n}");
                    if !taken.contains(&c) {
                        break c;
                    }
                    n += 1;
                }
            });
        taken.insert(chosen.clone());
        e.solidity_name = chosen;
    }

    debug_assert_eq!(
        taken.len(),
        entries.len(),
        "entrypoint names must be injective"
    );
}

/// One activation on the walk's call stack.
struct Frame {
    func: String,
    block: usize,
    index: usize,
    /// The name denoting the abstract argument inside this activation.
    /// What this frame knows: parameter name to the values it can take.
    env: crate::value::Env,
    visited: BTreeSet<(usize, usize)>,
}

/// A call written as a statement, with its arguments.
fn statement_call(ins: &mulu_yul::ir::Instruction) -> Option<(String, Vec<mulu_yul::Expr>)> {
    match &ins.op {
        mulu_yul::ir::Op::Effect { call: mulu_yul::Expr::Call { name, args, .. } } => {
            Some((name.clone(), args.clone()))
        }
        _ => None,
    }
}

/// Which of the callee's parameters denotes the abstract argument.
///
/// P1a carries a single uint256 argument, so a call may pass it along
/// unchanged or pass none of it. Anything else, such as a computed value,
/// would need the argument partition to be re-derived and is refused.
/// What the callee knows, from what the caller knows.
///
/// Each argument expression is evaluated in the caller's environment and
/// bound to the callee's parameter of the same position. This replaced a rule
/// that could bind exactly one parameter, to exactly the entrypoint argument,
/// passed by name and nothing else. A call like `capped(x + 1)` had no way to
/// be described; now it is described whenever `value_set` can evaluate it.
///
/// A parameter whose argument does not evaluate is simply not bound. The walk
/// then refuses any guard that needs it, by name, rather than here.
fn bind_arguments(
    callee: &Function,
    args: &[mulu_yul::Expr],
    caller: &crate::value::Env,
) -> Result<crate::value::Env, String> {
    let mut env = crate::value::Env::new();
    for (i, a) in args.iter().enumerate() {
        let Some(param) = callee.parameters.get(i) else { break };
        if let Ok(set) = crate::value::value_set(a, caller) {
            env.insert(param.clone(), set);
        }
    }
    Ok(env)
}

/// Every way of picking one item from each list, in a stable order. With no
/// lists there is one choice: the empty one, which is a call with no
/// arguments.
fn product(per: &[Vec<usize>]) -> Vec<Vec<usize>> {
    let mut out = vec![vec![]];
    for choices in per {
        let mut next = Vec::new();
        for prefix in &out {
            for c in choices {
                let mut v = prefix.clone();
                v.push(*c);
                next.push(v);
            }
        }
        out = next;
    }
    out
}

/// Environments in a message, so a refusal names what was known.
fn show(env: &crate::value::Env) -> String {
    if env.is_empty() {
        return "{}".into();
    }
    let v: Vec<String> = env.iter().map(|(k, s)| format!("{k} in {s}")).collect();
    format!("{{{}}}", v.join(", "))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, sig: &str, selector: &str) -> Entry {
        Entry {
            solidity_name: name.into(),
            signature: sig.into(),
            func: format!("fun_{name}"),
            params: signature_params(sig)
                .iter()
                .map(|t| Param {
                    name: "x".into(),
                    ty: t.clone(),
                    domain: IntervalSet::full(),
                })
                .collect(),
            selector: selector.into(),
        }
    }

    fn names(v: &[Entry]) -> Vec<String> {
        v.iter().map(|e| e.solidity_name.clone()).collect()
    }

    #[test]
    fn overloads_are_told_apart_by_their_parameter_types() {
        let mut v = vec![
            entry("set", "set(uint256)", "0x11111111"),
            entry("set", "set(uint8)", "0x22222222"),
            entry("limit", "limit()", "0x33333333"),
        ];
        disambiguate(&mut v);
        assert_eq!(names(&v), vec!["set_uint256", "set_uint8", "limit"]);
    }

    #[test]
    fn names_stay_injective_however_adversarial_the_source_is() {
        // Every fallback form is also a real function name here.
        let mut v = vec![
            entry("set", "set(uint256)", "0x60fe47b1"),
            entry("set", "set(uint8)", "0x24b8ba5f"),
            entry("set_uint256", "set_uint256(uint256)", "0xcccccccc"),
            entry("set_uint256_60fe47b1", "set_uint256_60fe47b1(uint256)", "0xdddddddd"),
            entry("fn_60fe47b1", "fn_60fe47b1(uint256)", "0xeeeeeeee"),
        ];
        disambiguate(&mut v);
        let got = names(&v);
        let mut sorted = got.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), got.len(), "names must be injective, got {got:?}");
    }

    #[test]
    fn a_name_a_real_function_already_uses_is_not_reused() {
        // `set(uint256)` wants `set_uint256`, which is also the plain name of
        // the third entry. Whoever asks second takes another form.
        let mut v = vec![
            entry("set", "set(uint256)", "0xaaaaaaaa"),
            entry("set", "set(uint8)", "0xbbbbbbbb"),
            entry("set_uint256", "set_uint256(uint256)", "0xcccccccc"),
        ];
        disambiguate(&mut v);
        let got = names(&v);
        let mut sorted = got.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), got.len(), "names must be injective, got {got:?}");
        assert_eq!(got[1], "set_uint8", "the readable form is kept where it is free");
    }

    #[test]
    fn distinct_names_are_left_alone() {
        let mut v = vec![
            entry("setLimit", "setLimit(uint256)", "0x11111111"),
            entry("forceSet", "forceSet(uint256)", "0x22222222"),
        ];
        disambiguate(&mut v);
        assert_eq!(names(&v), vec!["setLimit", "forceSet"]);
    }
}
