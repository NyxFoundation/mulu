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
    /// Every path the walk took, with what it assumed on the way. This is
    /// what a property about *calls* is answered from: "does `withdraw`
    /// revert whenever the amount is zero or above the balance" is a question
    /// about which of these paths a request can be on, and how they end.
    pub paths: Vec<PathSummary>,
}

/// One walk of one entrypoint, from one region of each argument and one
/// region of storage, down one side of each choice the regions did not settle.
#[derive(Debug, Clone, Serialize)]
pub struct PathSummary {
    pub entrypoint: String,
    /// The body's parameter names, in order, so a property can say "argument
    /// 0" and mean the same word the facts are written in.
    pub parameters: Vec<String>,
    /// The values each argument could take on this path, as intervals.
    pub arguments: Vec<IntervalSet>,
    /// Which region each storage slot was in, by label.
    pub storage: BTreeMap<String, IntervalSet>,
    /// The conditions the walk could not decide, and the side it took. Keys
    /// are canonical relations: `var_amount_20 <= cell(balances, caller())`.
    pub assumed: BTreeMap<String, bool>,
    /// Whether the transaction ended by returning or by reverting.
    pub reverts: bool,
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

/// What a constructor leaves in storage.
#[derive(Debug, Default)]
struct ConstructorEffect {
    /// Slots whose final value the model followed to a constant.
    determined: BTreeMap<usize, U256>,
    /// Slots written in a way the model could not follow.
    unknown: BTreeSet<usize>,
    /// A write whose slot the model could not follow: any slot may have moved.
    every_slot_unknown: bool,
    /// Why, in the order the writes appear.
    reasons: Vec<String>,
}

/// One initial state or several, as the schema distinguishes them.
fn initial_of(names: &[String]) -> Initial {
    match names {
        [one] => Initial::One(one.clone()),
        _ => Initial::Many(names.to_vec()),
    }
}

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
                    // A call that defines a value is followed too. The walk
                    // enters one, so a guard inside it is reached; leaving it
                    // out here meant that guard never refined the partition,
                    // and the walk then found it undecided in a region that
                    // was only coarse because of this.
                    if let Some((_, callee, _)) = defining_call(ins) {
                        if self.ir.function(&callee).is_some() {
                            queue.push(callee);
                        }
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
            // An external call is modelled as "returns 0 or 1, and nothing is
            // known about memory afterwards". What that leaves out is the
            // callee calling back in: a reentrant call runs the contract's
            // own entrypoints again and can move storage under the caller.
            // The model does not represent that, so it says so.
            if f.effects.external_call {
                self.note(format!(
                    "no-reentrancy: {} makes an external call, and the model assumes the \
                     callee does not call back into this contract",
                    e.signature
                ));
            }
            // The argument's domain comes from its ABI type, not from the
            // machine word. A uint8 argument has 256 values; treating it as a
            // full word invents regions no type-correct call can reach.
            //
            // A dynamic type reaches the body as more words than the ABI
            // lists it as: `f(bytes)` takes a pointer and a length. Then the
            // types line up with nothing, and every body parameter takes the
            // whole word.
            let abi_params = signature_params(&e.signature);
            let arities_match = abi_params.len() == f.parameters.len();
            if !arities_match {
                self.note(format!(
                    "whole-word-argument: the ABI lists {} parameter(s) of {} and the body takes \
                     {}, which is how solc passes a dynamic type. Each body parameter ranges \
                     over the whole 256-bit word",
                    abi_params.len(),
                    e.signature,
                    f.parameters.len()
                ));
            }
            let mut params = Vec::new();
            let mut bad = false;
            for (i, name) in f.parameters.iter().enumerate() {
                let ty = if arities_match {
                    abi_params[i].clone()
                } else {
                    "uint256".to_string()
                };
                match crate::types::domain_or_whole_word(&ty) {
                    Ok((domain, note)) => {
                        if let Some(n) = note {
                            self.note(n);
                        }
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
            // A guard that has no predicate form is not yet a refusal. The
            // predicate is what refines the partition, and a condition can
            // fail to be one and still be decidable where both sides are
            // values: `gt(add(memPtr, size), 0xffffffff)` is a comparison
            // whose left is arithmetic. Refusing here meant the walk never
            // got to look. It refuses instead, if it also cannot decide.
            if let Ok(p) = crate::predicate::translate_in(&c.condition, &f.parameters, domain) {
                out.insert(c.id.clone(), p);
            }
        }
        out
    }

    /// Partition `uint256` by a family of sets: the non-empty atoms of the
    /// boolean algebra they generate, plus the combinations that turned out
    /// to be unsatisfiable.
    fn partition(sets: &[(String, IntervalSet)]) -> (Vec<IntervalSet>, Vec<String>) {
        Self::partition_within(&IntervalSet::full(), sets)
    }

    /// The same, over a universe smaller than the whole word.
    ///
    /// A slot's type is not a boundary to split on, it is the universe the
    /// slot lives in. Splitting on it produced a second region holding the
    /// values the type cannot take, and the region product over the slots is
    /// what the walk enumerates: a contract with seven `address` and `bool`
    /// slots asked for 128 storage regions, 127 of which no deployment can
    /// reach.
    fn partition_within(
        universe: &IntervalSet,
        sets: &[(String, IntervalSet)],
    ) -> (Vec<IntervalSet>, Vec<String>) {
        let mut atoms = vec![(universe.clone(), Vec::<(String, bool)>::new())];
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
            // A slot cannot hold values its type has no room for. That is the
            // universe it lives in, not a line to cut it along: cutting gave
            // every `address` slot a region holding the values an address
            // cannot be, and the walk enumerates the product over the slots.
            let type_label = v.type_label.clone().unwrap_or_else(|| v.type_id.clone());
            let universe = match crate::types::domain_of(&type_label) {
                Ok(d) => {
                    if !d.is_full() {
                        storage_preds.push(PredicateInfo {
                            id: format!("type:{}", v.label),
                            source: format!("the declared type of {}", v.label),
                            text: format!("{} is {type_label}", v.label),
                            set: d.clone(),
                        });
                    }
                    d
                }
                Err(_) => IntervalSet::full(),
            };
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
            let (regions, infeasible) = Self::partition_within(&universe, &sets);
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
                    // Only a guard over one of this function's own parameters
                    // refines the *argument* partition. Letting a predicate
                    // over a local in was what made `partition` exponential
                    // in practice: it is exponential in how many sets it is
                    // given, and every local appearing in a condition added
                    // one. A guard over a local is still decided, at the
                    // walk, where the local has a value.
                    if !p.var().is_some_and(|v| f.parameters.iter().any(|x| x == v)) {
                        continue;
                    }
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

                // A plain branch is a boundary the regions have to respect
                // too. Only guards refined the partition, so a region could
                // straddle a branch condition and the walk had no region fine
                // enough to take one side of it. Same restriction as a guard:
                // only a condition over one of this function's own parameters
                // partitions the argument space.
                for b in &f.blocks {
                    let Terminator::Branch { cond, .. } = &b.terminator else { continue };
                    let Ok(pred) = crate::predicate::translate_in(cond, &f.parameters, &e.widest())
                    else {
                        continue;
                    };
                    let Some(var) = pred.var() else { continue };
                    if !f.parameters.iter().any(|x| x == var) {
                        continue;
                    }
                    let set = pred.set();
                    let id = format!("branch:{fname}#{}", b.id);
                    if !set.is_full() && !set.is_empty() && !arg_sets.iter().any(|(n, _)| *n == id) {
                        arg_preds.push(PredicateInfo {
                            id: id.clone(),
                            source: format!("a branch in {fname} reached from {}", e.solidity_name),
                            text: format!("{pred}"),
                            set: set.clone(),
                        });
                        arg_sets.push((id, set));
                    }
                }

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
        // `partition` builds the atoms of the boolean algebra these generate,
        // which is exponential in how many there are. A contract needing more
        // boundaries than this is refused rather than waited for.
        const MAX_ARG_PREDICATES: usize = 12;
        if arg_sets.len() > MAX_ARG_PREDICATES {
            self.refuse(format!(
                "this contract's guards need {} argument boundaries and the limit is \
                 {MAX_ARG_PREDICATES}; partitioning that many would cost more than the answer \
                 is worth",
                arg_sets.len()
            ));
            arg_sets.clear();
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
            "free-memory-pointer-at-0x80: the walk begins inside the entrypoint, so it starts \
             where solc's dispatcher leaves memory word 64. Only constant addresses are tracked, \
             and a store to a computed one drops all of them",
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
    /// Helpers that only compute, worked out once. `eval_call` used to
    /// establish this per call by scanning every function and every check of
    /// the contract, on every expression `value_set` could not read, in every
    /// walk. That scan, not the evaluation, was the cost.
    pure_helpers: BTreeSet<String>,
    /// Steps taken across every walk of this contract.
    ///
    /// `MAX_STEPS` bounds one walk and `MAX_WALKS` bounds how many there are,
    /// and neither bounds their product: a contract can ask for a few
    /// thousand walks that are each a few thousand steps and take longer than
    /// anyone will wait. The budget is spent across all of them, and running
    /// out refuses the contract rather than returning a model built from the
    /// part that fit.
    budget: usize,
    /// What `transitions` already holds, so adding one is a lookup.
    transition_keys: BTreeSet<(String, String, String)>,
    plant_transition_keys: BTreeSet<(String, String, String)>,
    /// When this contract's walks must be over.
    ///
    /// The step count and the walk count each bound one thing, and a
    /// pathology can hide between them or inside a single step. A wall clock
    /// bounds all of them at once, and it is the only bound that holds for a
    /// cost nobody has thought of yet. Running out refuses the contract; it
    /// never returns a model built from the part that fit.
    deadline: std::time::Instant,
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
    /// Every path the walk took, in the order it took them.
    paths: Vec<PathSummary>,
}

/// One position of an abstract execution along the path where guards pass.
enum Step {
    Check { id: String, text: String, passes: bool, depends_on: Vec<String> },
    Store { slot: usize, label: String, to: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ending {
    Return,
    Revert,
}

struct Trace {
    steps: Vec<Step>,
    ending: Ending,
    /// Conditions the regions did not decide, and the side this path took, as
    /// far as the transaction got. A check that fails ends the transaction,
    /// and the walk carries on past it only to record what the *other* side
    /// of that check would meet; nothing after it is on this path.
    assumed: BTreeMap<String, bool>,
    /// The transaction ends by reverting: a check on it fails, or the walk
    /// reached a `revert`.
    reverts: bool,
}

/// Why a walk did not produce a trace.
enum TraceStop {
    /// A choice the regions do not settle, reached with nothing left to take
    /// from the caller's list: how many ways it goes, and what it was. What
    /// it turns on is a value the model does not have, such as a word read
    /// from a mapping cell. The caller re-runs the walk once per way, so the
    /// contract keeps every path instead of being refused, and records `what`
    /// so a reader knows the model is coarse there rather than exact.
    Undecided { ways: usize, what: String },
    /// Something the model does not represent. The walk cannot continue down
    /// either side, so the contract is refused.
    Refused(String),
}

impl From<String> for TraceStop {
    fn from(s: String) -> Self {
        TraceStop::Refused(s)
    }
}

impl From<&str> for TraceStop {
    fn from(s: &str) -> Self {
        TraceStop::Refused(s.to_string())
    }
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
        // Worked out once, from the whole contract, before `b` is moved.
        const MAX_BODY: usize = 32;
        let with_checks: BTreeSet<&str> = b.ir.checks.iter().map(|c| c.function.as_str()).collect();
        let pure_helpers: BTreeSet<String> = b
            .ir
            .functions
            .iter()
            .filter(|f| {
                !f.effects.writes_storage
                    && !f.effects.can_revert
                    && !with_checks.contains(f.id.as_str())
                    && f.blocks.len() == 1
                    && f.blocks[0].instructions.len() <= MAX_BODY
                    && !f.returns.is_empty()
            })
            .map(|f| f.id.clone())
            .collect();
        drop(with_checks);
        Self {
            b,
            entries,
            guards,
            arg_regions,
            arg_preds,
            storage_preds,
            per_slot,
            pure_helpers,
            budget: 4_000_000,
            transition_keys: BTreeSet::new(),
            plant_transition_keys: BTreeSet::new(),
            deadline: std::time::Instant::now() + std::time::Duration::from_secs(20),
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
            paths: Vec::new(),
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

    /// Add a transition once. The duplicate check was a scan of everything
    /// added so far, which is quadratic in the number of transitions and was
    /// fine while a walk was a dozen steps. It stopped being fine when the
    /// walk began entering the calls it used to skip: a few thousand steps
    /// turned a scan into minutes.
    fn add(&mut self, from: &str, event: &str, to: &str) {
        self.states.insert(from.to_string());
        self.states.insert(to.to_string());
        let key = (from.to_string(), event.to_string(), to.to_string());
        if self.transition_keys.insert(key) {
            self.transitions.push(Transition {
                from: from.to_string(),
                event: event.to_string(),
                to: to.to_string(),
            });
        }
    }

    /// The storage the deployment leaves, when every constructor write
    /// resolves to a constant slot and a constant value. `Err` names the
    /// first write that does not, because "the constructor writes storage"
    /// told a reader nothing about which write or why.
    /// What the constructor leaves in storage, slot by slot.
    ///
    /// A write whose value is a constant determines its slot. A write the
    /// model cannot follow does not: it leaves that slot at whatever the
    /// deployment put there, which the model represents by starting in every
    /// region of it at once. That is weaker than knowing the value and
    /// stronger than refusing the contract, and refusing was what happened
    /// before: `owner = msg.sender` in a constructor threw away the whole
    /// analysis of every function, including the ones that never read
    /// `owner`.
    fn constructor_storage(&self, writes: &[&mulu_yul::ir::Instruction]) -> ConstructorEffect {
        let mut out = ConstructorEffect::default();
        for i in writes {
            let Some(w) = &i.storage_write else {
                // The write shape is not one the front end reduced to a slot
                // and a value, so it could have been to any slot.
                out.reasons.push(
                    "an instruction writes storage in a way the front end did not recognise as \
                     a slot and a value"
                        .to_string(),
                );
                out.every_slot_unknown = true;
                continue;
            };
            let slot = mulu_yul::fold::fold_fixpoint(&w.slot);
            let value = mulu_yul::fold::fold_fixpoint(&w.value);
            let Some(slot) = crate::value::constant(&slot) else {
                out.reasons.push(format!("the slot in `{}` is not a constant", w.slot_text));
                out.every_slot_unknown = true;
                continue;
            };
            let Ok(slot) = usize::try_from(slot) else {
                out.reasons.push(format!("slot {slot} is beyond the layout this models"));
                out.every_slot_unknown = true;
                continue;
            };
            if slot >= self.per_slot.len() {
                // Not a declared variable: a mapping cell, or padding. It
                // moves nothing the model tracks.
                continue;
            }
            match crate::value::constant(&value) {
                Some(v) => {
                    // A later write wins, which is the order the constructor
                    // runs in.
                    out.determined.insert(slot, v);
                    out.unknown.remove(&slot);
                }
                None => {
                    out.reasons.push(format!(
                        "the value written to slot {slot} is `{}`, which is not a constant",
                        w.value_text
                    ));
                    out.determined.remove(&slot);
                    out.unknown.insert(slot);
                }
            }
        }
        out
    }

    /// Decide a Yul condition by evaluating it, when the predicate built for
    /// it could not decide.
    ///
    /// The predicate form is a partition refiner: it has to be a set over one
    /// variable, so `gt(add(memPtr, size), 0xffffffff)` has no form to take.
    /// Evaluating the condition needs no such form, because both sides are
    /// already values here. It refines nothing and is therefore not a
    /// substitute for the predicate; it is what to try when the predicate has
    /// nothing to say.
    fn decide_cond(
        &self,
        cond: &mulu_yul::Expr,
        env: &crate::value::Env,
        storage: &StorageRegion,
        memory: &BTreeMap<U256, IntervalSet>,
    ) -> Option<bool> {
        let order = |a: &mulu_yul::Expr, b: &mulu_yul::Expr, strict: bool| -> Option<bool> {
            let (l, r) = (
                self.eval(a, env, storage, memory).ok()?,
                self.eval(b, env, storage, memory).ok()?,
            );
            let ((llo, lhi), (rlo, rhi)) = (l.bounds()?, r.bounds()?);
            if if strict { lhi < rlo } else { lhi <= rlo } {
                Some(true)
            } else if if strict { llo >= rhi } else { llo > rhi } {
                Some(false)
            } else {
                None
            }
        };
        let mulu_yul::Expr::Call { name, args, .. } = cond else {
            // A bare value is the condition: non-zero is true.
            let set = self.eval(cond, env, storage, memory).ok()?;
            let zero = IntervalSet::point(U256::ZERO);
            return if set.disjoint_from(&zero) {
                Some(true)
            } else if set.subset_of(&zero) {
                Some(false)
            } else {
                None
            };
        };
        match (name.as_str(), args.len()) {
            ("lt", 2) => order(&args[0], &args[1], true),
            ("gt", 2) => order(&args[1], &args[0], true),
            ("iszero", 1) => self.decide_cond(&args[0], env, storage, memory).map(|b| !b),
            ("eq", 2) => {
                let (l, r) = (
                    self.eval(&args[0], env, storage, memory).ok()?,
                    self.eval(&args[1], env, storage, memory).ok()?,
                );
                if l.disjoint_from(&r) {
                    Some(false)
                } else if l == r && l.bounds().map(|(a, b)| a == b).unwrap_or(false) {
                    // Both are the same single value.
                    Some(true)
                } else {
                    None
                }
            }
            ("and", 2) => match (
                self.decide_cond(&args[0], env, storage, memory),
                self.decide_cond(&args[1], env, storage, memory),
            ) {
                (Some(false), _) | (_, Some(false)) => Some(false),
                (Some(true), Some(true)) => Some(true),
                _ => None,
            },
            ("or", 2) => match (
                self.decide_cond(&args[0], env, storage, memory),
                self.decide_cond(&args[1], env, storage, memory),
            ) {
                (Some(true), _) | (_, Some(true)) => Some(true),
                (Some(false), Some(false)) => Some(false),
                _ => None,
            },
            _ => None,
        }
    }

    /// Evaluate a call to a helper that only computes.
    ///
    /// solc's IR is mostly tiny pure converters: `cleanup_t_uint256`,
    /// `convert_t_rational_2_by_1_to_t_uint256`, `identity`. A value passing
    /// through one of them became unknown, and everything downstream of it
    /// with it, which is why a guard on an allocation size saw an empty
    /// environment for a size that was the literal 2.
    ///
    /// Only straight-line bodies, and only functions with no effect and no
    /// check: a helper that can revert is a helper the walk must *enter*, so
    /// that the revert path reaches the model, and evaluating it here instead
    /// would lose exactly that.
    fn eval_call(
        &self,
        callee: &str,
        args: &[mulu_yul::Expr],
        env: &crate::value::Env,
        storage: &StorageRegion,
        memory: &BTreeMap<U256, IntervalSet>,
        depth: usize,
    ) -> Result<IntervalSet, String> {
        // Nesting is bounded because the walk enumerates a product of regions
        // and evaluates the same helper many times over.
        const MAX_DEPTH: usize = 4;
        if depth >= MAX_DEPTH {
            return Err(format!("evaluating {callee} nests deeper than {MAX_DEPTH} calls"));
        }
        // A helper with an effect or a check is *not* evaluated: the walk
        // must enter one of those so its revert path reaches the model, and
        // evaluating it here would lose exactly that. Membership is decided
        // once per contract rather than by scanning it per call.
        if !self.pure_helpers.contains(callee) {
            return Err(format!("`{callee}` is not a helper that only computes"));
        }
        let g = self
            .b
            .ir
            .function(callee)
            .ok_or_else(|| format!("`{callee}` is not a function this build defines"))?;
        let ret = g.returns.first().expect("a pure helper returns something");
        let mut inner = crate::value::Env::new();
        for (p, a) in g.parameters.iter().zip(args.iter()) {
            inner.insert(p.clone(), self.eval_at(a, env, storage, memory, depth + 1)?);
        }
        for i in &g.blocks[0].instructions {
            let (targets, value) = match &i.op {
                mulu_yul::ir::Op::Let { targets, value: Some(v) } => (targets, v),
                mulu_yul::ir::Op::Assign { targets, value } => (targets, value),
                mulu_yul::ir::Op::Let { targets, value: None } => {
                    for t in targets {
                        inner.insert(t.clone(), IntervalSet::point(U256::ZERO));
                    }
                    continue;
                }
                mulu_yul::ir::Op::Effect { .. } => continue,
            };
            if targets.len() != 1 {
                for t in targets {
                    inner.remove(t);
                }
                continue;
            }
            match self.eval_at(value, &inner, storage, memory, depth + 1) {
                Ok(v) => {
                    inner.insert(targets[0].clone(), v);
                }
                Err(_) => {
                    inner.remove(&targets[0]);
                }
            }
        }
        inner
            .get(ret)
            .cloned()
            .ok_or_else(|| format!("`{callee}` did not determine its result"))
    }

    /// The single value an address expression denotes, if it denotes one.
    ///
    /// A slot is usually a literal, but solc passes it as a parameter to its
    /// generated helpers: `array_length_T(array)` is `sload(array)` where
    /// `array` is bound to the slot. Folding alone cannot see that; the
    /// environment can.
    fn address(
        &self,
        e: &mulu_yul::Expr,
        env: &crate::value::Env,
        storage: &StorageRegion,
        memory: &BTreeMap<U256, IntervalSet>,
        depth: usize,
    ) -> Option<U256> {
        let folded = mulu_yul::fold::fold_fixpoint(e);
        if let Some(v) = crate::value::constant(&folded) {
            return Some(v);
        }
        // Carrying the depth is what stops this: `eval` resolves an address
        // through `address`, and `address` used to start over at zero, so
        // `mload(mload(64))` recursed for ever between them. Optimised, that
        // is a loop rather than a crash, which is the worst of both.
        let set = self.eval_at(e, env, storage, memory, depth + 1).ok()?;
        let (lo, hi) = set.bounds()?;
        (lo == hi).then_some(lo)
    }

    /// `value_set`, plus the one thing it cannot know on its own: what a
    /// storage slot holds in the region being walked. `sload` of a constant
    /// slot is the whole reason a length or a bound is ever in scope.
    fn eval(
        &self,
        e: &mulu_yul::Expr,
        env: &crate::value::Env,
        storage: &StorageRegion,
        memory: &BTreeMap<U256, IntervalSet>,
    ) -> Result<IntervalSet, String> {
        self.eval_at(e, env, storage, memory, 0)
    }

    fn eval_at(
        &self,
        e: &mulu_yul::Expr,
        env: &crate::value::Env,
        storage: &StorageRegion,
        memory: &BTreeMap<U256, IntervalSet>,
        depth: usize,
    ) -> Result<IntervalSet, String> {
        // The same bound as `eval_call`, here too: `eval` and `address` call
        // each other, so neither can be the only one that counts.
        if depth >= 8 {
            return Err("evaluating this expression nests deeper than 8 levels".into());
        }
        if let mulu_yul::Expr::Call { name, args, .. } = e {
            if name == "mload" && args.len() == 1 {
                let Some(k) = self.address(&args[0], env, storage, memory, depth) else {
                    return Err("`mload` of a computed address is outside the P1a fragment".into());
                };
                return memory
                    .get(&k)
                    .cloned()
                    .ok_or_else(|| format!("nothing is known about memory at {k}"));
            }
            // `call` and `staticcall` push 1 on success and 0 on failure.
            // Two values, both reachable: the callee is not modelled, so
            // neither outcome can be ruled out.
            if (name == "call" && args.len() == 7) || (name == "staticcall" && args.len() == 6) {
                return Ok(IntervalSet::point(U256::ZERO).union(&IntervalSet::point(U256::from(1))));
            }
            if name == "sload" && args.len() == 1 {
                let Some(v) = self.address(&args[0], env, storage, memory, depth) else {
                    return Err("`sload` of a computed slot is outside the P1a fragment".into());
                };
                let Ok(i) = usize::try_from(v) else {
                    return Err(format!("slot {v} is beyond the layout this models"));
                };
                let (label, regions) = self
                    .per_slot
                    .get(i)
                    .ok_or_else(|| format!("slot {i} is not in the storage layout"))?;
                let _ = label;
                return regions
                    .get(storage[i])
                    .cloned()
                    .ok_or_else(|| format!("slot {i} has no region {}", storage[i]));
            }
        }
        match crate::value::value_set(e, env) {
            Ok(v) => Ok(v),
            // A call `value_set` does not know may still be a helper that
            // only computes.
            Err(why) => match e {
                mulu_yul::Expr::Call { name, args, .. } => {
                    self.eval_call(name, args, env, storage, memory, depth)
                }
                _ => Err(why),
            },
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
        // A constructor that writes constants is as determined as one that
        // writes nothing: `x = 5` leaves slot 0 at 5, and refusing it threw
        // away every contract with an initialiser. What is refused is a
        // constructor whose writes depend on something the model does not
        // have, such as `owner = msg.sender`.
        let effect = self.constructor_storage(&ctor_writes);
        if ctor_writes.is_empty() {
            self.b.note(
                "initial-state: the constructor writes no storage, so every slot starts at zero",
            );
        } else if effect.reasons.is_empty() {
            self.b.note(
                "initial-state: every constructor write resolved to a constant, so the model \
                 starts where the deployment leaves the contract",
            );
        }
        // Each slot contributes the regions the deployment could have left it
        // in: one, when the constructor's write is a constant or there is no
        // write; all of them, when the model could not follow the write.
        let per_slot_initial: Vec<Vec<usize>> = (0..self.per_slot.len())
            .map(|i| {
                let unknown = effect.every_slot_unknown || effect.unknown.contains(&i);
                if unknown {
                    return (0..self.per_slot[i].1.len()).collect();
                }
                let v = effect.determined.get(&i).copied().unwrap_or(U256::ZERO);
                vec![self.region_of(i, &IntervalSet::point(v)).unwrap_or(0)]
            })
            .collect();
        if !effect.reasons.is_empty() {
            let mut reasons = effect.reasons.clone();
            reasons.sort();
            reasons.dedup();
            let where_ = if effect.every_slot_unknown {
                "every slot".to_string()
            } else {
                let mut names: Vec<&str> = effect
                    .unknown
                    .iter()
                    .map(|i| self.per_slot[*i].0.as_str())
                    .collect();
                names.sort();
                names.join(", ")
            };
            self.b.note(format!(
                "initial-state-widened: the constructor's effect on {where_} is not determined \
                 ({}), so the model starts in every region of it at once. Findings hold for any \
                 deployment, and none of them rests on a value the constructor set",
                reasons.join("; ")
            ));
        }
        let initial_storage: Vec<StorageRegion> = per_slot_initial
            .iter()
            .fold(vec![Vec::new()], |acc: Vec<StorageRegion>, choices| {
                acc.iter()
                    .flat_map(|prefix| {
                        choices.iter().map(move |c| {
                            let mut v = prefix.clone();
                            v.push(*c);
                            v
                        })
                    })
                    .collect()
            });

        // idle states, one per storage region
        for s in &storage_regions {
            let n = format!("idle_{}", self.storage_name(s));
            self.states.insert(n.clone());
            self.marked.insert(n);
        }

        let mut entries = std::mem::take(&mut self.entries);

        // One walk per entrypoint, per choice of region for each argument,
        // per storage region. That product is a power of the arity, so a
        // function of three arguments over a partition refined by a few
        // guards can ask for more walks than the answer is worth. Refusing
        // says so, and refusing the *contract* rather than walking a fraction
        // of it: a model built from part of the product would be a search
        // that did not finish, reported as one that found nothing.
        const MAX_WALKS: usize = 4096;
        // A path may fork this many times, and one (entrypoint, arguments,
        // storage) triple may take this many paths in all. Both are there
        // because the fork count is exponential in the first: a function
        // guarded by eight mapping reads has 256 paths, and past that the
        // model costs more than it says.
        const MAX_SPLITS: usize = 24;
        const MAX_FORKS: usize = 4096;
        let planned: usize = entries
            .iter()
            .map(|e| {
                let per: usize = e
                    .params
                    .iter()
                    .map(|p| {
                        (0..self.arg_regions.len())
                            .filter(|i| self.arg_regions[*i].subset_of(&p.domain))
                            .count()
                            .max(1)
                    })
                    .product();
                per.saturating_mul(storage_regions.len())
            })
            .sum();
        if planned > MAX_WALKS {
            self.b.refuse(format!(
                "the arguments and storage of this contract need {planned} walks and the limit \
                 is {MAX_WALKS}; the model is not built rather than built from part of it"
            ));
            entries.clear();
        }

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
                    // A guard the regions do not decide is not a dead end: it
                    // is a fork, and the walk is run again down each side. The
                    // decisions are a list the walk consumes in the order it
                    // meets them, so a path with n such guards is n+1 runs of
                    // the walk rather than one walk carrying a solver.
                    //
                    // Depth-first with an explicit stack, so a contract that
                    // forks more than the budget allows is refused with the
                    // paths it did take already emitted.
                    let mut stack: Vec<Vec<usize>> = vec![Vec::new()];
                    let mut forks = 0usize;
                    while let Some(sp) = stack.pop() {
                        // One walk feeds both models, so their states line up
                        // and the pairing in `sites` means what it says.
                        match self.trace(e, &a, s, &sp) {
                            Ok(t) => {
                                self.paths.push(PathSummary {
                                    entrypoint: e.signature.clone(),
                                    parameters: e.params.iter().map(|p| p.name.clone()).collect(),
                                    arguments: a
                                        .iter()
                                        .map(|i| self.arg_regions[*i].clone())
                                        .collect(),
                                    storage: s
                                        .iter()
                                        .enumerate()
                                        .map(|(i, r)| {
                                            (self.per_slot[i].0.clone(), self.per_slot[i].1[*r].clone())
                                        })
                                        .collect(),
                                    assumed: t.assumed.clone(),
                                    reverts: t.reverts,
                                });
                                self.emit_impl(e, &a, s, &t);
                                self.emit_plant(e, &a, s, &t);
                            }
                            Err(TraceStop::Refused(why)) => {
                                self.b.refuse(format!("{}: {why}", e.signature));
                                break;
                            }
                            Err(TraceStop::Undecided { ways, what }) => {
                                forks += 1;
                                self.b.note(format!(
                                    "split-on-an-unknown-value: {what} is not decided by the \
                                     regions, so the model takes every way it goes. Every path \
                                     the program has is in the model and some that it may not \
                                     have; a check that fails only on such a path is reported \
                                     as one that can fail, never as one that cannot"
                                ));
                                if sp.len() >= MAX_SPLITS || forks > MAX_FORKS {
                                    self.b.refuse(format!(
                                        "{}: more of this function turns on values the model \
                                         does not have than the limit allows ({} on one path, \
                                         limit {MAX_SPLITS}; {forks} paths, limit {MAX_FORKS}); \
                                         the last was {what}. The model is not built rather \
                                         than built from part of it",
                                        e.signature,
                                        sp.len()
                                    ));
                                    break;
                                }
                                for side in 0..ways {
                                    let mut next = sp.clone();
                                    next.push(side);
                                    stack.push(next);
                                }
                            }
                        }
                    }
                }
            }
        }

        let bad = if self.bad_used { vec!["bad".to_string()] } else { vec![] };
        if self.bad_used {
            self.states.insert("bad".into());
        }

        let mut initial_names: Vec<String> = initial_storage
            .iter()
            .map(|s| format!("idle_{}", self.storage_name(s)))
            .collect();
        initial_names.sort();
        initial_names.dedup();
        for n in &initial_names {
            self.states.insert(n.clone());
        }

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
                initial: initial_of(&initial_names),
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
            initial: initial_of(&initial_names),
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
        Abstraction { model, report, paths: self.paths }
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
        splits: &[usize],
    ) -> Result<Trace, TraceStop> {
        // Choices the caller already made at the forks, in the order this
        // walk meets them. Running out means the walk found one more than the
        // caller knew about, which is what `Undecided` says.
        let mut splits = splits.iter().copied();
        const MAX_DEPTH: usize = 32;
        const MAX_STEPS: usize = 10_000;

        let mut storage = entry_storage.clone();
        // Memory at constant addresses. solc's allocator lives at word 64:
        // the dispatcher writes `mstore(64, 0x80)`, every allocation reads it
        // and writes it back, and the guard on the result is what stopped the
        // walk in every contract with a memory array or struct. Only constant
        // addresses are tracked, and a write to a computed one clears the map,
        // because it could have landed anywhere.
        let mut memory: BTreeMap<U256, IntervalSet> = BTreeMap::new();
        // The walk starts inside the entrypoint, not at the dispatcher that
        // called it, so the free pointer the dispatcher sets is not otherwise
        // in scope. Every solc-generated dispatcher begins `mstore(64, 0x80)`,
        // and the allocator's guard is on a value derived from it, so the
        // walk starts where the dispatcher leaves it. Recorded as an
        // assumption rather than assumed quietly.
        memory.insert(U256::from(64u8), IntervalSet::point(U256::from(0x80u8)));
        let mut steps: Vec<Step> = Vec::new();
        let mut seen_checks: Vec<String> = Vec::new();
        // Conditions this path has already taken a side on, by canonical
        // term. `require(amount <= balances[msg.sender])` and the underflow
        // check on `balances[msg.sender] -= amount` are two conditions over
        // the same two values, and without this the walk forked on each and
        // produced a path where the first passed and the second failed. That
        // path is not one the program has, and the paths multiply.
        let mut facts: BTreeMap<String, bool> = BTreeMap::new();
        // What was assumed when the first check on this path failed. After
        // that the transaction is over, so anything the walk goes on to
        // assume is about a path this is not.
        let mut reverted_at: Option<BTreeMap<String, bool>> = None;
        // Storage named the way the contract names it, so a fact key reads
        // `cell(balances, caller())` rather than carrying solc's generated
        // helper names, which change with the types they encode.
        let slots: BTreeMap<U256, String> = self
            .per_slot
            .iter()
            .enumerate()
            .filter_map(|(i, (label, _))| {
                let _ = i;
                let v = self.b.storage.iter().find(|v| v.label == *label)?;
                let slot = crate::interval::parse_decimal(&v.slot).ok()?;
                Some((slot, label.clone()))
            })
            .collect();
        let layout = move |s: U256| slots.get(&s).cloned();
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
            terms: e
                .params
                .iter()
                .map(|p| {
                    (
                        p.name.clone(),
                        mulu_yul::Expr::Ident { name: p.name.clone(), src: None },
                    )
                })
                .collect(),
            returns_to: vec![],
            visited: BTreeSet::new(),
        }];
        let mut n = 0usize;

        loop {
            n += 1;
            if n > MAX_STEPS {
                return Err("the abstract execution did not terminate within the step limit".into());
            }
            // Checked every 256 steps: often enough to stop, rarely enough
            // not to be the cost itself.
            if n % 256 == 0 && std::time::Instant::now() > self.deadline {
                return Err("this contract exceeded the abstraction's time budget".into());
            }
            match self.budget.checked_sub(1) {
                Some(left) => self.budget = left,
                None => {
                    return Err(
                        "this contract's walks exhausted the abstraction's step budget".into()
                    )
                }
            }
            let depth = frames.len();
            let (func, block, index, env, terms) = {
                let fr = frames.last().expect("a frame");
                (fr.func.clone(), fr.block, fr.index, fr.env.clone(), fr.terms.clone())
            };
            let f = self.b.ir.function(&func).ok_or("no such function")?.clone();
            let blk = f.block(block).clone();

            if index < blk.instructions.len() {
                frames.last_mut().unwrap().index += 1;
                let ins = &blk.instructions[index];

                if let Some(c) = self.check_at(&f, block, index, ins) {
                    let guards = self.guards_of(&f, &e.widest());
                    // The predicate first, because it is what refines the
                    // partition. Where it has nothing to say, the condition
                    // is evaluated directly: both sides are values here even
                    // when neither is a set over one variable.
                    let passes = match guards
                        .get(&c.id)
                        .and_then(|p| p.decide(&env))
                        .or_else(|| self.decide_cond(&c.condition, &env, &storage, &memory))
                    {
                        Some(v) => v,
                        None => decide_or_split(
                            &c.condition,
                            &terms,
                            &layout,
                            &mut facts,
                            &mut splits,
                            || format!("check {} in {func}", c.id),
                        )?,
                    };
                    if !passes && reverted_at.is_none() {
                        reverted_at = Some(facts.clone());
                    }
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
                    // A write whose slot is not a declared variable is a
                    // mapping or array cell, which the model does not track.
                    // It moves nothing in the state, under the assumption
                    // recorded once for the contract: a keccak-derived slot
                    // does not collide with a small declared one.
                    let Ok((slot_idx, slot_label)) = self.slot_of(&w.slot_text) else {
                        self.b.note(
                            "cells-do-not-alias: a write to a computed slot is a mapping or \
                             array cell and moves no declared variable. keccak256 not \
                             colliding with a small constant slot is an assumption, the same \
                             one solc's own storage layout rests on",
                        );
                        continue;
                    };
                    // What is written may not be a value the walk knows: it
                    // can come from a mapping cell, or from a call the model
                    // does not follow. Then the slot could end in any of its
                    // regions, and the walk takes each in turn rather than
                    // refusing. Same for a value that straddles a boundary.
                    let to = match crate::value::value_set(&w.value, &env)
                        .ok()
                        .and_then(|values| self.region_of(slot_idx, &values))
                    {
                        Some(to) => to,
                        None => {
                            let ways = self.per_slot[slot_idx].1.len();
                            // A slot with one region has one place the write
                            // can land, so there is nothing to choose. Asking
                            // for a decision anyway doubled the path count
                            // for every store of an unknown value, which is
                            // most stores.
                            if ways <= 1 {
                                0
                            } else {
                                splits.next().ok_or_else(|| TraceStop::Undecided {
                                    ways,
                                    what: format!("the value written to {slot_label}"),
                                })?
                            }
                        }
                    };
                    storage[slot_idx] = to;
                    steps.push(Step::Store { slot: slot_idx, label: slot_label, to });
                    continue;
                }

                // A local defined here is a value the rest of the walk can
                // use. Nothing bound them before, so every guard over a local
                // was "not the argument" even when the local *was* the
                // argument, one `let` later. This is where a bounds check
                // stops being unreachable: `let arrayLength := sload(slot)`
                // then `if iszero(lt(index, arrayLength))`.
                //
                // The safety net comes first. A definition whose value can
                // revert or write storage is not a definition the walk may
                // pass over: binding the target and moving on would drop a
                // revert path the program has, which is exactly the silent
                // hole the net below exists to stop. Binding is for the
                // instructions that only compute.
                let only_computes = !ins.effects.writes_storage && !ins.effects.can_revert;

                // `mstore` at a constant address is the one memory fact worth
                // keeping, because the allocator's free pointer lives at 64
                // and every guard on an allocation reads it. A store to a
                // computed address could land anywhere, so it clears what is
                // known rather than being ignored.
                // An external call may write anywhere in the output region,
                // whose address the model does not track, so afterwards it
                // knows nothing about memory. The result itself is bound by
                // the arms below, from `eval`.
                if ins.effects.external_call {
                    memory.clear();
                }

                if let mulu_yul::ir::Op::Effect { call: mulu_yul::Expr::Call { name, args, .. } } =
                    &ins.op
                {
                    if name == "mstore" && args.len() == 2 {
                        let at = mulu_yul::fold::fold_fixpoint(&args[0]);
                        match crate::value::constant(&at) {
                            Some(k) => match self.eval(&args[1], &env, &storage, &memory) {
                                Ok(v) => {
                                    memory.insert(k, v);
                                }
                                Err(_) => {
                                    memory.remove(&k);
                                }
                            },
                            None => memory.clear(),
                        }
                        continue;
                    }
                }

                match &ins.op {
                    mulu_yul::ir::Op::Let { targets, value: Some(v) }
                    | mulu_yul::ir::Op::Assign { targets, value: v }
                        if only_computes && targets.len() == 1 =>
                    {
                        if let Ok(set) = self.eval(v, &env, &storage, &memory) {
                            frames.last_mut().unwrap().env.insert(targets[0].clone(), set);
                        } else {
                            // Not knowing a local's *value* is not an error.
                            // A guard that needs it forks later, by name.
                            frames.last_mut().unwrap().env.remove(&targets[0]);
                        }
                        // Its term is known whether or not its value is, and
                        // the term is what says two locals hold the same
                        // thing.
                        let fr = frames.last_mut().unwrap();
                        match canon(v, &terms) {
                            Some(t) => {
                                fr.terms.insert(targets[0].clone(), t);
                            }
                            None => {
                                fr.terms.remove(&targets[0]);
                            }
                        }
                        continue;
                    }
                    mulu_yul::ir::Op::Let { targets, value: None } if only_computes => {
                        // `let a, b` is zero until assigned.
                        for t in targets {
                            let fr = frames.last_mut().unwrap();
                            fr.env.insert(t.clone(), IntervalSet::point(U256::ZERO));
                            fr.terms.insert(
                                t.clone(),
                                mulu_yul::Expr::Literal { text: "0".into(), src: None },
                            );
                        }
                        continue;
                    }
                    mulu_yul::ir::Op::Let { targets, .. }
                    | mulu_yul::ir::Op::Assign { targets, .. }
                        if only_computes =>
                    {
                        // Several targets from one call: nothing here can say
                        // which value went where, so they stay unknown.
                        for t in targets {
                            let fr = frames.last_mut().unwrap();
                            fr.env.remove(t);
                            fr.terms.remove(t);
                        }
                        continue;
                    }
                    // Anything with an effect falls through to the storage
                    // write, the call and the net below, as it did before.
                    _ => {}
                }

                // A definition whose value is a call that reverts, writes
                // storage or holds a check has to be entered, not skipped and
                // not refused. Skipping drops the revert path; refusing loses
                // every array index access, whose bounds check lives inside
                // exactly such a call. The results are bound on the way out.
                if let Some((targets, callee, args)) = defining_call(ins) {
                    if let Some(g) = self.b.ir.function(&callee) {
                        let matters = g.effects.writes_storage
                            || g.effects.can_revert
                            || self.b.ir.checks.iter().any(|c| c.function == callee);
                        if matters {
                            if depth >= MAX_DEPTH {
                                return Err(format!("call depth limit reached at {callee}").into());
                            }
                            let inner = bind_arguments(g, &args, &env)?;
                            let inner_terms = bind_terms(g, &args, &terms);
                            frames.push(Frame {
                                func: callee,
                                block: 0,
                                index: 0,
                                env: inner,
                                terms: inner_terms,
                                returns_to: targets,
                                visited: BTreeSet::new(),
                            });
                            continue;
                        }
                    }
                }

                if let Some((callee, args)) = statement_call(ins) {
                    if let Some(g) = self.b.ir.function(&callee) {
                        let matters = g.effects.writes_storage
                            || g.effects.can_revert
                            || self.b.ir.checks.iter().any(|c| c.function == callee);
                        if matters {
                            if depth >= MAX_DEPTH {
                                return Err(format!("call depth limit reached at {callee}").into());
                            }
                            let inner = bind_arguments(g, &args, &env)?;
                            let inner_terms = bind_terms(g, &args, &terms);
                            frames.push(Frame {
                                func: callee,
                                block: 0,
                                index: 0,
                                env: inner,
                                terms: inner_terms,
                                returns_to: vec![],
                                visited: BTreeSet::new(),
                            });
                            continue;
                        }
                    }
                }

                // An instruction that can revert but writes nothing is a
                // cleanup with a panic in it: solc's `cleanup_t_enum` reverts
                // on a value outside the enum, and it sits *inside* the
                // condition rather than being a call the walk can enter. Two
                // ways: it does not panic and the instruction is the value it
                // computes, or it does and the transaction ends there.
                // Skipping it would drop the revert path, which is the hole
                // this net exists to stop.
                if ins.effects.can_revert && !ins.effects.writes_storage {
                    let reverts = decide_or_split(
                        &mulu_yul::Expr::Ident {
                            name: format!("panic-in:{func}#{block}#{index}"),
                            src: None,
                        },
                        &terms,
                        &layout,
                        &mut facts,
                        &mut splits,
                        || format!("a panic inside an instruction in {func}"),
                    )?;
                    if reverts {
                        return Ok(Trace { steps, ending: Ending::Revert, assumed: reverted_at.unwrap_or(facts), reverts: true });
                    }
                    // It computes; whatever it defines stays unknown, which
                    // is what the arms above would have left had it not been
                    // able to revert.
                    if let mulu_yul::ir::Op::Let { targets, .. }
                    | mulu_yul::ir::Op::Assign { targets, .. } = &ins.op
                    {
                        for t in targets {
                            let fr = frames.last_mut().unwrap();
                            fr.env.remove(t);
                            fr.terms.remove(t);
                        }
                    }
                    continue;
                }

                if ins.effects.writes_storage || ins.effects.can_revert {
                    return Err(format!(
                        "an instruction in {func} carries effects the model does not represent \
                         ({}{}); P1a cannot skip it",
                        if ins.effects.writes_storage { "writes storage" } else { "" },
                        if ins.effects.can_revert { " can revert" } else { "" },
                    ).into());
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
                        let passes = match guards
                            .get(&c.id)
                            .and_then(|p| p.decide(&env))
                            .or_else(|| self.decide_cond(&c.condition, &env, &storage, &memory))
                        {
                            Some(v) => v,
                            None => decide_or_split(
                                &c.condition,
                                &terms,
                                &layout,
                                &mut facts,
                                &mut splits,
                                || format!("check {} in {func}", c.id),
                            )?,
                        };
                        if !passes && reverted_at.is_none() {
                            reverted_at = Some(facts.clone());
                        }
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
                            _ => return Err(format!("check {} has no block edges", c.id).into()),
                        };
                        let fr = frames.last_mut().unwrap();
                        if !fr.visited.insert((target, 0)) {
                            return Err("the abstract execution revisits a block; P1a does not model loops".into());
                        }
                        fr.block = target;
                        fr.index = 0;
                        continue;
                    }
                    // A branch on a mapping cell is the same situation as a
                    // guard on one, and takes the same two-sided treatment.
                    let taken = match crate::predicate::translate_in(cond, &f.parameters, &e.widest())
                        .ok()
                        .and_then(|p| p.decide(&env))
                        .or_else(|| self.decide_cond(cond, &env, &storage, &memory))
                    {
                        Some(v) => v,
                        None => decide_or_split(cond, &terms, &layout, &mut facts, &mut splits, || {
                            format!("a branch in {func}")
                        })?,
                    };
                    let target = if taken { *then_block } else { *else_block };
                    let fr = frames.last_mut().unwrap();
                    if !fr.visited.insert((target, 0)) {
                        return Err("the abstract execution revisits a block; P1a does not model loops".into());
                    }
                    fr.block = target;
                    fr.index = 0;
                }
                Terminator::Revert { .. } => {
                    return Ok(Trace { steps, ending: Ending::Revert, assumed: reverted_at.unwrap_or(facts), reverts: true })
                }
                Terminator::Return { .. } | Terminator::Stop => {
                    return Ok(Trace { steps, ending: Ending::Return, assumed: reverted_at.clone().unwrap_or_else(|| facts.clone()), reverts: reverted_at.is_some() })
                }
                Terminator::Leave => {
                    // Bind what the call produced, by the callee's own return
                    // names, before the frame that knew them goes away.
                    let done = frames.pop().expect("a frame");
                    if frames.is_empty() {
                        return Ok(Trace { steps, ending: Ending::Return, assumed: reverted_at.clone().unwrap_or_else(|| facts.clone()), reverts: reverted_at.is_some() });
                    }
                    if !done.returns_to.is_empty() {
                        let rets = self
                            .b
                            .ir
                            .function(&done.func)
                            .map(|g| g.returns.clone())
                            .unwrap_or_default();
                        let caller = &mut frames.last_mut().unwrap().env;
                        for (target, ret) in done.returns_to.iter().zip(rets.iter()) {
                            match done.env.get(ret) {
                                Some(set) => {
                                    caller.insert(target.clone(), set.clone());
                                }
                                // Unknown is not zero. Leaving a stale
                                // binding would be worse than none.
                                None => {
                                    caller.remove(target);
                                }
                            }
                        }
                        // More targets than the callee returns is a shape
                        // this cannot describe; leave the rest unknown.
                        for target in done.returns_to.iter().skip(rets.len()) {
                            caller.remove(target);
                        }
                    }
                }
                // A switch is a chain of equality tests on one value, which
                // is what the walk already does for a branch. Refusing it
                // left out the dispatcher's own shape and any hand-written
                // `switch` in a body.
                Terminator::Switch { value, cases, default } => {
                    // A switch on a value the walk does not know goes every
                    // way the switch has: one per case, plus one for the
                    // default. `returndatasize` after an external call is the
                    // one that matters, and refusing it lost the whole
                    // contract for a value nothing later reads.
                    let set = match self.eval(value, &env, &storage, &memory) {
                        Ok(v) => Some(v),
                        Err(_) => None,
                    };
                    #[allow(unused_assignments)]
                    let mut target: Option<usize> = None;
                    let Some(set) = set else {
                        let ways = cases.len() + 1;
                        let pick = splits.next().ok_or_else(|| TraceStop::Undecided {
                            ways,
                            what: format!("a switch in {func}"),
                        })?;
                        let chosen = match cases.get(pick) {
                            Some((_, b)) => Some(*b),
                            None => *default,
                        };
                        match chosen {
                            Some(t) => {
                                let fr = frames.last_mut().unwrap();
                                if !fr.visited.insert((t, 0)) {
                                    return Err(TraceStop::Refused(
                                        "the abstract execution revisits a block; P1a does not \
                                         model loops"
                                            .into(),
                                    ));
                                }
                                fr.block = t;
                                fr.index = 0;
                                continue;
                            }
                            // Nothing matched and there is no default: the
                            // switch does nothing and the function ends.
                            None => {
                                frames.pop();
                                if frames.is_empty() {
                                    return Ok(Trace { steps, ending: Ending::Return, assumed: reverted_at.clone().unwrap_or_else(|| facts.clone()), reverts: reverted_at.is_some() });
                                }
                                continue;
                            }
                        }
                    };
                    // Which cases the value could match. One means the switch
                    // is decided; more than one, or one plus the chance of
                    // matching none, means it is not, and the walk goes each
                    // way. A `bool` reaching a `switch` is the common shape:
                    // the value is in {0, 1} and case 0 is one of two ways.
                    let mut possible: Vec<Option<usize>> = Vec::new();
                    let mut certain = false;
                    for (lit, block) in cases {
                        let v = crate::interval::parse_decimal(lit)
                            .map_err(|e| format!("a switch case label is not a literal: {e}"))?;
                        let case = IntervalSet::point(v);
                        if set.subset_of(&case) {
                            possible.clear();
                            possible.push(Some(*block));
                            certain = true;
                            break;
                        }
                        if !set.disjoint_from(&case) {
                            possible.push(Some(*block));
                        }
                    }
                    if !certain {
                        // The value can also match no case at all, unless the
                        // cases cover it between them.
                        let covered = cases
                            .iter()
                            .filter_map(|(lit, _)| crate::interval::parse_decimal(lit).ok())
                            .fold(IntervalSet::empty(), |acc, v| {
                                acc.union(&IntervalSet::point(v))
                            });
                        if !set.subset_of(&covered) {
                            possible.push(*default);
                        }
                    }
                    if possible.len() > 1 {
                        let ways = possible.len();
                        let pick = splits.next().ok_or_else(|| TraceStop::Undecided {
                            ways,
                            what: format!("a switch in {func}"),
                        })?;
                        target = possible[pick.min(ways - 1)];
                        if target.is_none() {
                            // Nothing matched and there is no default.
                            frames.pop();
                            if frames.is_empty() {
                                return Ok(Trace { steps, ending: Ending::Return, assumed: reverted_at.clone().unwrap_or_else(|| facts.clone()), reverts: reverted_at.is_some() });
                            }
                            continue;
                        }
                    } else {
                        target = possible.first().copied().flatten();
                    }
                    let target = match (target, default) {
                        (Some(t), _) => t,
                        // Yul: an unmatched switch with no default does
                        // nothing, which here is falling out of the function.
                        (None, Some(d)) => *d,
                        (None, None) => {
                            // Nothing matched and there is no default, so
                            // the switch does nothing and the function ends.
                            frames.pop();
                            if frames.is_empty() {
                                return Ok(Trace { steps, ending: Ending::Return, assumed: reverted_at.clone().unwrap_or_else(|| facts.clone()), reverts: reverted_at.is_some() });
                            }
                            continue;
                        }
                    };
                    let fr = frames.last_mut().unwrap();
                    if !fr.visited.insert((target, 0)) {
                        return Err(
                            "the abstract execution revisits a block; P1a does not model loops"
                                .into(),
                        );
                    }
                    fr.block = target;
                    fr.index = 0;
                }
                Terminator::Unsupported { reason } => return Err(reason.clone().into()),
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

    /// The plant's half of [`Self::add`], and quadratic for the same reason
    /// until it was not.
    fn plant_add(&mut self, from: &str, event: &str, to: &str) {
        self.plant_states.insert(from.to_string());
        self.plant_states.insert(to.to_string());
        let key = (from.to_string(), event.to_string(), to.to_string());
        if self.plant_transition_keys.insert(key) {
            self.plant_transitions.push(Transition {
                from: from.to_string(),
                event: event.to_string(),
                to: to.to_string(),
            });
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
    /// What this frame knows: parameter name to the values it can take.
    env: crate::value::Env,
    /// The same names, as expressions in terms of the entrypoint's arguments
    /// and the reads the model cannot resolve. Two locals holding the same
    /// value render the same here, which is how the walk recognises a
    /// condition it has already taken a side on.
    terms: BTreeMap<String, mulu_yul::Expr>,
    /// The caller's names for this call's results. Empty for a call made as
    /// a statement, which has none.
    returns_to: Vec<String>,
    visited: BTreeSet<(usize, usize)>,
}

/// A definition whose value is one call, with its arguments: `let x := f(a)`
/// or `x, y := f(a)`. The call is what the walk has to enter, and the targets
/// are where its results go.
fn defining_call(
    ins: &mulu_yul::ir::Instruction,
) -> Option<(Vec<String>, String, Vec<mulu_yul::Expr>)> {
    let (targets, value) = match &ins.op {
        mulu_yul::ir::Op::Let { targets, value: Some(v) } => (targets, v),
        mulu_yul::ir::Op::Assign { targets, value } => (targets, value),
        _ => return None,
    };
    match value {
        mulu_yul::Expr::Call { name, args, .. } => {
            Some((targets.clone(), name.clone(), args.clone()))
        }
        _ => None,
    }
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

/// Take a side on a condition the regions do not decide.
///
/// If this path already took a side on a condition with the same canonical
/// term, take the same side: `require(amount <= bal)` followed by the
/// underflow check on `bal -= amount` is one relation asked twice, and
/// forking on each produced a path where the first held and the second did
/// not. Otherwise fork, and remember the side for the rest of the path.
fn decide_or_split(
    cond: &mulu_yul::Expr,
    terms: &BTreeMap<String, mulu_yul::Expr>,
    layout: &impl crate::relation::Layout,
    facts: &mut BTreeMap<String, bool>,
    splits: &mut impl Iterator<Item = usize>,
    what: impl Fn() -> String,
) -> Result<bool, TraceStop> {
    // The relation first, because it recognises the same question asked two
    // ways: `iszero(gt(a, b))` and `iszero(lt(b, a))` are one fact. Where the
    // condition is not a comparison, its canonical term still keys it, which
    // catches the same condition asked twice.
    let key = crate::relation::of_in(cond, terms, layout)
        .map(|r| r.key())
        .or_else(|| canon(cond, terms).map(|t| t.render()));
    if let Some(k) = &key {
        if let Some(known) = facts.get(k) {
            return Ok(*known);
        }
    }
    let side = splits.next().ok_or_else(|| TraceStop::Undecided { ways: 2, what: what() })? == 1;
    if let Some(k) = key {
        facts.insert(k, side);
    }
    Ok(side)
}

/// The callee's parameters, as terms in the caller's names.
fn bind_terms(
    callee: &Function,
    args: &[mulu_yul::Expr],
    caller: &BTreeMap<String, mulu_yul::Expr>,
) -> BTreeMap<String, mulu_yul::Expr> {
    let mut out = BTreeMap::new();
    for (i, a) in args.iter().enumerate() {
        let Some(param) = callee.parameters.get(i) else { break };
        if let Some(t) = canon(a, caller) {
            out.insert(param.clone(), t);
        }
    }
    out
}

/// The canonical term of an expression: every local replaced by what defines
/// it, folded. Two expressions with the same term denote the same value, and
/// that is all the fact store needs — it never has to decide the value, only
/// notice that a condition is one it has already taken a side on.
///
/// Bounded, because substitution can grow a term faster than it is worth:
/// past the cap there is no term, and a condition over it forks as before.
fn canon(e: &mulu_yul::Expr, terms: &BTreeMap<String, mulu_yul::Expr>) -> Option<mulu_yul::Expr> {
    const MAX_TERM: usize = 4096;
    let out = mulu_yul::fold::fold_fixpoint(&e.substitute(terms));
    if out.render().len() > MAX_TERM {
        return None;
    }
    Some(out)
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
