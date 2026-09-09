//! Properties about a call: "`withdraw` reverts whenever the amount is zero
//! or above the caller's balance."
//!
//! The specification of docs/09 §3 says what must hold of *storage* when a
//! transaction ends. That cannot say what a benchmark's answer key says,
//! which is almost always about whether a particular request is accepted:
//!
//! ```text
//! rule withdraw_revert {
//!     require (amount == 0 || amount > currentContract.balances[e.msg.sender]);
//!     withdraw@withrevert(e, amount);
//!     assert lastReverted;
//! }
//! ```
//!
//! That is mulu's own question written by someone else. "Accepts a request it
//! should reject" is a spec violation and "rejects one it should accept" is
//! overrestriction, and both are decided here against the paths the walk took.
//!
//! A property is answered from `Abstraction::paths`. Each path carries the
//! interval each argument was in, the region each slot was in, the conditions
//! the walk could not decide and the side it took, and how it ended. A
//! property holds when *every* path a request satisfying `given` could be on
//! ends the way the property says. A path that cannot be ruled out counts,
//! which is why the answer is only ever "holds" when it is established.

use crate::interval::{parse_decimal, IntervalSet};
use crate::model::PathSummary;
use crate::order::contradictory;
use crate::relation::{Op, Relation};
use serde::{Deserialize, Serialize};

/// What the call is asserted to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    Reverts,
    DoesNotRevert,
}

/// A file of them, as the benchmark harness reads it. One per use case,
/// beside the encodings the benchmark ships for the other tools.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PropertyFile {
    pub schema_version: u32,
    #[serde(default)]
    pub use_case: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub note: String,
    /// Facts about the contract's storage that hold in every reachable state,
    /// and that the model does not derive: `contract_balance` is at least any
    /// one balance entry, say. mulu's abstraction has no inductive invariant
    /// over storage, so without these it reports paths the contract cannot be
    /// on. Each one is a claim about the contract, and an answer that used
    /// one says so.
    #[serde(default)]
    pub invariants: Vec<Invariant>,
    #[serde(default)]
    pub call_properties: Vec<CallProperty>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Invariant {
    pub id: String,
    /// Where the claim comes from: the corpus's own property list, ideally,
    /// so it can be checked against the same answer key.
    #[serde(default)]
    pub from: String,
    pub holds: Condition,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CallProperty {
    pub id: String,
    /// The benchmark's own encoding this one was written from, so a reader
    /// can put them side by side.
    #[serde(default)]
    pub from: String,
    /// What this property is expected to come out as, when that is not
    /// "holds". A rule mulu cannot answer is worth keeping in the file and
    /// worth saying so about, rather than deleting until the file agrees
    /// with the tool.
    #[serde(default)]
    pub expected: String,
    /// Why, when `expected` says something.
    #[serde(default)]
    pub why: String,
    /// One rule per entrypoint the property is about. A file of the
    /// benchmark's may hold several rules under one name, and the answer key
    /// has one row for the name, so the property holds when all of them do.
    pub rules: Vec<Rule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    /// The ABI signature, matching an entrypoint of the contract.
    pub entrypoint: String,
    /// The requests the rule is about. Absent means all of them.
    #[serde(default)]
    pub given: Option<Condition>,
    #[serde(rename = "assert")]
    pub outcome: Outcome,
}

/// A term the property can compare.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", deny_unknown_fields)]
pub enum Term {
    /// The entrypoint's nth argument, counting from zero.
    Argument(usize),
    /// A storage variable, by the label solc records in `storageLayout`.
    Storage(String),
    /// A mapping cell: the variable, and the key.
    Cell {
        var: String,
        key: Box<Term>,
    },
    /// A cell reached through more than one mapping.
    /// `_roles[role].members[account]` is `{ var: "_roles", keys: [role,
    /// account] }`, which is how every role check in OpenZeppelin's access
    /// control is written.
    Nested {
        var: String,
        keys: Vec<Term>,
    },
    /// Transaction or block context: `caller`, `callvalue`, `number`,
    /// `timestamp`, `origin`. `balance` is the contract's own ether balance,
    /// which solc reads as `balance(address())`.
    Env(String),
    /// An `immutable` variable, by the name the source gave it.
    Immutable(String),
    /// A struct field within a cell: `_roles[role].adminRole` is the second
    /// word of `_roles[role]`, which is `{ of: <that cell>, index: 1 }`.
    Field {
        of: Box<Term>,
        index: u64,
    },
    /// A storage slot by number, for storage the layout does not name.
    /// ERC-7201 namespaced storage is written through assembly at a fixed
    /// slot, so `storageLayout` is empty and there is no label to use;
    /// every upgradeable OpenZeppelin contract is this shape.
    Slot(String),
    /// A literal, in decimal or 0x hex.
    Uint256(String),
    /// Wrapping addition and subtraction, as the EVM does them. `request_time
    /// + wait_time` is a term a rule about a deadline needs.
    Add {
        left: Box<Term>,
        right: Box<Term>,
    },
    Sub {
        left: Box<Term>,
        right: Box<Term>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Condition {
    /// Unsigned <=, <, >=, >
    Ule {
        left: Term,
        right: Term,
    },
    Ult {
        left: Term,
        right: Term,
    },
    Uge {
        left: Term,
        right: Term,
    },
    Ugt {
        left: Term,
        right: Term,
    },
    Eq {
        left: Term,
        right: Term,
    },
    Ne {
        left: Term,
        right: Term,
    },
    And {
        args: Vec<Condition>,
    },
    Or {
        args: Vec<Condition>,
    },
    Not {
        arg: Box<Condition>,
    },
}

/// What a property came out as.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Verdict {
    /// Every path a request satisfying `given` could be on ends as asserted.
    Holds,
    /// One does not, and the witness says which.
    Fails,
    /// The model does not decide it: a term the property names is one no path
    /// mentions, so no path can be ruled in or out.
    Undecided,
}

#[derive(Debug, Clone, Serialize)]
pub struct Answer {
    pub id: String,
    /// Invariants the answer rests on, by id. Empty when it rests on none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assuming: Vec<String>,
    pub entrypoint: String,
    pub verdict: Verdict,
    /// Why, in one line.
    pub because: String,
    /// How many paths of the entrypoint the request could be on.
    pub paths_considered: usize,
}

/// Three-valued: the model says yes, says no, or does not say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tri {
    True,
    False,
    Unknown,
}

impl Tri {
    fn and(self, other: Tri) -> Tri {
        match (self, other) {
            (Tri::False, _) | (_, Tri::False) => Tri::False,
            (Tri::True, Tri::True) => Tri::True,
            _ => Tri::Unknown,
        }
    }
    fn or(self, other: Tri) -> Tri {
        match (self, other) {
            (Tri::True, _) | (_, Tri::True) => Tri::True,
            (Tri::False, Tri::False) => Tri::False,
            _ => Tri::Unknown,
        }
    }
    fn not(self) -> Tri {
        match self {
            Tri::True => Tri::False,
            Tri::False => Tri::True,
            Tri::Unknown => Tri::Unknown,
        }
    }
}

/// How a term reads in the language the paths' facts are written in, and what
/// values it can take on this path.
fn render(t: &Term, p: &PathSummary) -> Option<String> {
    Some(match t {
        Term::Argument(i) => p.parameters.get(*i)?.clone(),
        Term::Storage(v) => format!("storage({v})"),
        Term::Immutable(v) => format!("immutable({v})"),
        Term::Slot(n) => format!("sload({})", parse_decimal(n).ok()?),
        Term::Cell { var, key } => format!("cell({var}, {})", render(key, p)?),
        Term::Field { of, index } => format!("field({}, {index})", render(of, p)?),
        Term::Nested { var, keys } => {
            let mut out = String::from("cell(");
            out.push_str(var);
            for k in keys {
                out.push_str(", ");
                out.push_str(&render(k, p)?);
            }
            out.push(')');
            out
        }
        Term::Env(name) => match name.as_str() {
            "balance" | "selfbalance" => "balance(address())".to_string(),
            // The contract's own code size. Zero exactly while its
            // constructor runs, which several of OpenZeppelin's rules turn
            // on without saying so: they are stated of a deployed contract.
            "codesize" | "extcodesize" => "extcodesize(address())".to_string(),
            other => format!("{other}()"),
        },
        Term::Uint256(text) => parse_decimal(text).ok()?.to_string(),
        Term::Add { left, right } => format!("add({}, {})", render(left, p)?, render(right, p)?),
        Term::Sub { left, right } => format!("sub({}, {})", render(left, p)?, render(right, p)?),
    })
}

/// The values the term can take on this path, when the path says.
fn values(t: &Term, p: &PathSummary) -> Option<IntervalSet> {
    match t {
        Term::Argument(i) => p.arguments.get(*i).cloned(),
        Term::Storage(v) => p.storage.get(v).cloned(),
        Term::Uint256(text) => parse_decimal(text).ok().map(IntervalSet::point),
        // A mapping cell, the transaction context and arithmetic over them
        // are not in the model's state. What is known about them is what the
        // path assumed, which is read as a relation rather than as a set.
        Term::Cell { .. }
        | Term::Nested { .. }
        | Term::Field { .. }
        | Term::Env(_)
        | Term::Immutable(_)
        | Term::Slot(_)
        | Term::Add { .. }
        | Term::Sub { .. } => None,
    }
}

/// Does the relation `left OP right` hold on this path?
fn holds(op: Op, left: &Term, right: &Term, p: &PathSummary) -> Tri {
    // Intervals first: exact where both sides are in the model's state. A
    // region that does not settle it is not the end of the matter, though:
    // a slot with one region settles nothing by its interval and everything
    // by what the path assumed about it.
    if let (Some(a), Some(b)) = (values(left, p), values(right, p)) {
        if let (Some((alo, ahi)), Some((blo, bhi))) = (a.bounds(), b.bounds()) {
            let (yes, no) = match op {
                Op::Lt => (ahi < blo, alo >= bhi),
                Op::Le => (ahi <= blo, alo > bhi),
                Op::Eq => (a == b && a.count() == Some(1), a.disjoint_from(&b)),
            };
            if yes {
                return Tri::True;
            }
            if no {
                return Tri::False;
            }
        }
    }
    // Otherwise the path has to have taken a side on it, or the order facts
    // it carries have to settle it. Asking the closure both ways is what
    // makes `a < b` and `b < a` two sides of one question rather than two
    // unrelated keys.
    let (Some(l), Some(r)) = (render(left, p), render(right, p)) else {
        return Tri::Unknown;
    };
    let key = Relation {
        op,
        left: l.clone(),
        right: r.clone(),
    }
    .key();
    if let Some(v) = p.assumed.get(&key) {
        return if *v { Tri::True } else { Tri::False };
    }
    // Two literals cannot both be it: if the path assumed `state == 0`, then
    // `state == 1` is settled, and it is settled false. This is what an enum
    // in a guard needs, because every state is one equality.
    if op == Op::Eq {
        if let Term::Uint256(_) = right {
            for (k, v) in &p.assumed {
                if !*v {
                    continue;
                }
                let Some(rest) = k.strip_prefix(&format!("{l} == ")) else {
                    continue;
                };
                if rest != r && parse_decimal(rest).is_ok() {
                    return Tri::False;
                }
            }
        }
        return Tri::Unknown;
    }
    // `a <= b` holds when adding `b < a` makes the path impossible, and fails
    // when adding `a <= b` does.
    let base: Vec<(String, String, bool)> = order_edges(p)
        .into_iter()
        .chain(interval_edges(p))
        .collect();
    let with = |e: (String, String, bool)| {
        let mut v = base.clone();
        v.push(e);
        contradictory(&v)
    };
    let (assert_yes, assert_no) = match op {
        Op::Lt => ((l.clone(), r.clone(), true), (r.clone(), l.clone(), false)),
        Op::Le => ((l.clone(), r.clone(), false), (r.clone(), l.clone(), true)),
        Op::Eq => unreachable!("handled above"),
    };
    if with(assert_no) {
        return Tri::True;
    }
    if with(assert_yes) {
        return Tri::False;
    }
    Tri::Unknown
}

/// The order facts a path carries, as edges `left <= right`, with `strict`
/// marking `<`. A fact taken to be false is the reverse edge: `not (a <= b)`
/// is `b < a`.
fn order_edges(p: &PathSummary) -> Vec<(String, String, bool)> {
    let mut out = vec![];
    for (k, v) in &p.assumed {
        let (a, op, b) = if let Some((a, b)) = k.split_once(" <= ") {
            (a, Op::Le, b)
        } else if let Some((a, b)) = k.split_once(" < ") {
            (a, Op::Lt, b)
        } else {
            continue;
        };
        out.push(match (op, v) {
            (Op::Le, true) => (a.to_string(), b.to_string(), false),
            (Op::Le, false) => (b.to_string(), a.to_string(), true),
            (Op::Lt, true) => (a.to_string(), b.to_string(), true),
            (Op::Lt, false) => (b.to_string(), a.to_string(), false),
            _ => continue,
        });
    }
    // An equality is two orderings. A store records one, relating the slot
    // after the write to the value written, and that is how a length after a
    // push meets the length before it.
    for (k, v) in &p.assumed {
        if !*v {
            continue;
        }
        if let Some((a, b)) = k.split_once(" == ") {
            out.push((a.to_string(), b.to_string(), false));
            out.push((b.to_string(), a.to_string(), false));
        }
    }
    out
}

/// What the regions say, as order facts: an argument in `[1, 2^160)` is a
/// term with `1` below it. Interval arithmetic cannot decide
/// `sub(amount, 1) <= amount` and so the walk forks on it, and one side of
/// that fork is a path where the argument is both at least one and less than
/// one. Only the regions and the relations together rule it out.
fn interval_edges(p: &PathSummary) -> Vec<(String, String, bool)> {
    let mut out = vec![];
    let mut bound = |term: String, set: &IntervalSet| {
        if let Some((lo, hi)) = set.bounds() {
            if lo > crate::interval::U256::ZERO {
                out.push((lo.to_string(), term.clone(), false));
            }
            if hi < crate::interval::max_u256() {
                out.push((term, hi.to_string(), false));
            }
        }
    };
    for (name, set) in p.parameters.iter().zip(p.arguments.iter()) {
        bound(name.clone(), set);
    }
    for (label, set) in &p.storage {
        bound(format!("storage({label})"), set);
    }
    out
}

/// The same for a condition the caller asserts, when it is a comparison this
/// can read as an order fact.
fn order_edges_of(c: &Condition, p: &PathSummary) -> Vec<(String, String, bool)> {
    use Condition::*;
    let two = |l: &Term, r: &Term, strict: bool, swap: bool| {
        let (Some(a), Some(b)) = (render(l, p), render(r, p)) else {
            return vec![];
        };
        vec![if swap { (b, a, strict) } else { (a, b, strict) }]
    };
    match c {
        Ule { left, right } => two(left, right, false, false),
        Ult { left, right } => two(left, right, true, false),
        Uge { left, right } => two(left, right, false, true),
        Ugt { left, right } => two(left, right, true, true),
        And { args } => args.iter().flat_map(|a| order_edges_of(a, p)).collect(),
        _ => vec![],
    }
}

fn evaluate(c: &Condition, p: &PathSummary) -> Tri {
    use Condition::*;
    match c {
        Ule { left, right } => holds(Op::Le, left, right, p),
        Ult { left, right } => holds(Op::Lt, left, right, p),
        Uge { left, right } => holds(Op::Le, right, left, p),
        Ugt { left, right } => holds(Op::Lt, right, left, p),
        Eq { left, right } => holds(Op::Eq, left, right, p),
        Ne { left, right } => holds(Op::Eq, left, right, p).not(),
        And { args } => args
            .iter()
            .fold(Tri::True, |acc, a| acc.and(evaluate(a, p))),
        Or { args } => args
            .iter()
            .fold(Tri::False, |acc, a| acc.or(evaluate(a, p))),
        Not { arg } => evaluate(arg, p).not(),
    }
}

/// Answer one property against the paths of one abstraction.
///
/// A property with several rules holds when all of them do, is undecided when
/// any is undecided and none fails, and fails as soon as one does.
pub fn check(prop: &CallProperty, paths: &[PathSummary]) -> Answer {
    check_with(prop, &[], paths)
}

/// The same, with invariants the contract is claimed to satisfy. A path that
/// contradicts one is not a path the contract can be on.
pub fn check_with(prop: &CallProperty, invariants: &[Invariant], paths: &[PathSummary]) -> Answer {
    let mut considered = 0;
    let mut worst: Option<Answer> = None;
    for r in &prop.rules {
        let a = check_rule(&prop.id, r, invariants, paths);
        considered += a.paths_considered;
        let rank = |v: Verdict| match v {
            Verdict::Fails => 2,
            Verdict::Undecided => 1,
            Verdict::Holds => 0,
        };
        if worst
            .as_ref()
            .is_none_or(|w| rank(a.verdict) > rank(w.verdict))
        {
            worst = Some(a);
        }
    }
    match worst {
        Some(mut a) => {
            a.paths_considered = considered;
            a.assuming = invariants.iter().map(|i| i.id.clone()).collect();
            a
        }
        None => Answer {
            id: prop.id.clone(),
            assuming: vec![],
            entrypoint: String::new(),
            verdict: Verdict::Undecided,
            because: "the property has no rules".into(),
            paths_considered: 0,
        },
    }
}

fn check_rule(id: &str, prop: &Rule, invariants: &[Invariant], paths: &[PathSummary]) -> Answer {
    let mine: Vec<&PathSummary> = paths
        .iter()
        .filter(|p| p.entrypoint == prop.entrypoint)
        .collect();
    if mine.is_empty() {
        return Answer {
            id: id.to_string(),
            assuming: vec![],
            entrypoint: prop.entrypoint.clone(),
            verdict: Verdict::Undecided,
            because: format!("the model has no path through {}", prop.entrypoint),
            paths_considered: 0,
        };
    }
    // Paths a request satisfying `given` could be on: those where the
    // condition is true, and those where the model does not say. Ruling one
    // out needs the model to say `false`, which is the only direction that
    // may not be guessed.
    let possible: Vec<&&PathSummary> = mine
        .iter()
        .filter(|p| {
            // An invariant rules a path out either by being false on it, or
            // by contradicting what it assumed.
            if invariants
                .iter()
                .any(|i| evaluate(&i.holds, p) == Tri::False)
            {
                return false;
            }
            let mut edges = order_edges(p);
            edges.extend(interval_edges(p));
            for i in invariants {
                edges.extend(order_edges_of(&i.holds, p));
            }
            !contradictory(&edges)
        })
        .filter(|p| match &prop.given {
            None => true,
            Some(c) => evaluate(c, p) != Tri::False,
        })
        .collect();
    let want_revert = prop.outcome == Outcome::Reverts;
    let counterexample = possible.iter().find(|p| p.reverts != want_revert);
    let verdict = match counterexample {
        None if possible.is_empty() => Verdict::Undecided,
        None => Verdict::Holds,
        Some(_) => Verdict::Fails,
    };
    let because = match (&verdict, counterexample) {
        (Verdict::Holds, _) => format!(
            "{}: every one of the {} path(s) a matching request can be on {}",
            prop.entrypoint,
            possible.len(),
            if want_revert { "reverts" } else { "returns" }
        ),
        (Verdict::Fails, Some(p)) => format!(
            "{}: a request on a path that {} matches: arguments {}, assuming {}",
            prop.entrypoint,
            if p.reverts { "reverts" } else { "returns" },
            p.arguments
                .iter()
                .map(|a| a.to_string())
                .collect::<Vec<_>>()
                .join(", "),
            if p.assumed.is_empty() {
                "nothing".to_string()
            } else {
                p.assumed
                    .iter()
                    .map(|(k, v)| if *v { k.clone() } else { format!("not ({k})") })
                    .collect::<Vec<_>>()
                    .join(" and ")
            }
        ),
        _ => format!(
            "no path of {} can carry a matching request",
            prop.entrypoint
        ),
    };
    Answer {
        id: id.to_string(),
        assuming: vec![],
        entrypoint: prop.entrypoint.clone(),
        verdict,
        because,
        paths_considered: possible.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interval::U256;
    use std::collections::BTreeMap;

    fn path(reverts: bool, amount: IntervalSet, assumed: &[(&str, bool)]) -> PathSummary {
        PathSummary {
            entrypoint: "withdraw(uint256)".into(),
            parameters: vec!["var_amount_20".into()],
            arguments: vec![amount],
            storage: BTreeMap::new(),
            assumed: assumed.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
            reverts,
        }
    }

    fn u(n: u64) -> U256 {
        U256::from(n)
    }

    /// `withdraw` reverts when the amount is zero or above the balance. This
    /// is `bank/certora/withdraw-revert.spec`, in mulu's terms, against the
    /// paths a conforming `withdraw` produces.
    fn withdraw_revert() -> CallProperty {
        CallProperty {
            id: "withdraw-revert".into(),
            from: String::new(),
            expected: String::new(),
            why: String::new(),
            rules: vec![Rule {
                entrypoint: "withdraw(uint256)".into(),
                given: Some(Condition::Or {
                    args: vec![
                        Condition::Eq {
                            left: Term::Argument(0),
                            right: Term::Uint256("0".into()),
                        },
                        Condition::Ugt {
                            left: Term::Argument(0),
                            right: Term::Cell {
                                var: "balances".into(),
                                key: Box::new(Term::Env("caller".into())),
                            },
                        },
                    ],
                }),
                outcome: Outcome::Reverts,
            }],
        }
    }

    const LE_BAL: &str = "var_amount_20 <= cell(balances, caller())";

    #[test]
    fn a_conforming_withdraw_reverts_exactly_where_the_property_says() {
        // amount = 0: the first guard fails, so it reverts.
        // amount > 0 and above the balance: the second guard fails.
        // amount > 0 and within it: it returns.
        let paths = vec![
            path(true, IntervalSet::point(U256::ZERO), &[]),
            path(true, IntervalSet::ge(u(1)), &[(LE_BAL, false)]),
            path(false, IntervalSet::ge(u(1)), &[(LE_BAL, true)]),
        ];
        let a = check(&withdraw_revert(), &paths);
        assert_eq!(a.verdict, Verdict::Holds, "{}", a.because);
        // the returning path is ruled out, so only two were considered
        assert_eq!(a.paths_considered, 2);
    }

    /// v2 of the same contract drops `require(amount <= balances[msg.sender])`,
    /// and the benchmark's answer key says the property no longer holds.
    #[test]
    fn a_withdraw_missing_its_balance_check_fails_the_property() {
        let paths = vec![
            path(true, IntervalSet::point(U256::ZERO), &[]),
            // nothing was assumed about the balance, because nothing tested it
            path(false, IntervalSet::ge(u(1)), &[]),
        ];
        let a = check(&withdraw_revert(), &paths);
        assert_eq!(a.verdict, Verdict::Fails, "{}", a.because);
        assert!(a.because.contains("returns"), "{}", a.because);
    }

    /// The other half: `withdraw` does not revert when the amount is within
    /// the balance. On the conforming contract the external call can still
    /// fail, so this is the shape the answer key marks 0 for v1 as well.
    #[test]
    fn the_converse_property_sees_the_path_that_can_still_revert() {
        let not_revert = CallProperty {
            id: "withdraw-not-revert".into(),
            from: String::new(),
            expected: String::new(),
            why: String::new(),
            rules: vec![Rule {
                entrypoint: "withdraw(uint256)".into(),
                given: Some(Condition::Ule {
                    left: Term::Argument(0),
                    right: Term::Cell {
                        var: "balances".into(),
                        key: Box::new(Term::Env("caller".into())),
                    },
                }),
                outcome: Outcome::DoesNotRevert,
            }],
        };
        let paths = vec![
            path(false, IntervalSet::ge(u(1)), &[(LE_BAL, true)]),
            // the `require(success)` on the external call
            path(
                true,
                IntervalSet::ge(u(1)),
                &[(LE_BAL, true), ("var_success_46", false)],
            ),
        ];
        let a = check(&not_revert, &paths);
        assert_eq!(a.verdict, Verdict::Fails, "{}", a.because);
    }

    /// The walk forks on `sub(amount, 1) <= amount`, because interval
    /// arithmetic cannot decide it, and one side of that fork is a path where
    /// the argument is at least one and less than one at the same time. The
    /// regions and the relations together rule it out; neither does alone.
    #[test]
    fn a_path_that_contradicts_itself_is_not_a_path() {
        let p = PathSummary {
            entrypoint: "withdraw(uint256)".into(),
            parameters: vec!["amt".into()],
            arguments: vec![IntervalSet::ge(u(1))],
            storage: BTreeMap::new(),
            assumed: [("1 <= amt".to_string(), false)].into_iter().collect(),
            reverts: true,
        };
        let mut edges = order_edges(&p);
        edges.extend(interval_edges(&p));
        assert!(contradictory(&edges));
        // and without the regions it is a path like any other
        assert!(!contradictory(&order_edges(&p)));
    }

    /// An invariant the contract satisfies rules out a path by contradicting
    /// what it assumed, which needs the order facts closed under transitivity:
    /// `amt <= bal` and `bal <= total` cannot sit with `total < amt`.
    #[test]
    fn an_invariant_rules_out_a_path_only_through_transitivity() {
        let p = PathSummary {
            entrypoint: "withdraw(uint256)".into(),
            parameters: vec!["amt".into()],
            arguments: vec![IntervalSet::full()],
            storage: BTreeMap::new(),
            assumed: [
                ("amt <= cell(balances, caller())".to_string(), true),
                ("amt <= storage(total)".to_string(), false),
            ]
            .into_iter()
            .collect(),
            reverts: true,
        };
        let inv = Invariant {
            id: "cbal-ge-bal".into(),
            from: String::new(),
            holds: Condition::Ule {
                left: Term::Cell {
                    var: "balances".into(),
                    key: Box::new(Term::Env("caller".into())),
                },
                right: Term::Storage("total".into()),
            },
        };
        let mut edges = order_edges(&p);
        assert!(!contradictory(&edges), "the path stands on its own");
        edges.extend(order_edges_of(&inv.holds, &p));
        assert!(contradictory(&edges), "with the invariant it does not");
    }

    /// A `push` stores `length + 1` at the array's slot and then checks that
    /// the old length is below the new one. Deciding that needs three things
    /// together: the equality the store records, the bound the slot's type
    /// gives, and the closure over the two.
    #[test]
    fn a_length_after_a_push_is_above_the_length_before_it() {
        let p = PathSummary {
            entrypoint: "enter()".into(),
            parameters: vec![],
            arguments: vec![],
            storage: [("players".to_string(), IntervalSet::le(u(18446744073709551614)))]
                .into_iter()
                .collect(),
            assumed: [
                ("storage(players@1) == add(storage(players), 1)".to_string(), true),
                // the side of the fork that says the push did not grow it
                ("storage(players) < storage(players@1)".to_string(), false),
            ]
            .into_iter()
            .collect(),
            reverts: true,
        };
        let mut edges = order_edges(&p);
        edges.extend(interval_edges(&p));
        assert!(contradictory(&edges), "a length cannot fail to grow by one");

        // and without the bound it is not settled: `t + 1` could wrap
        let unbounded = PathSummary { storage: BTreeMap::new(), ..p };
        assert!(!contradictory(&order_edges(&unbounded)));
    }

    /// A property naming something no path mentions is not answered "holds"
    /// by default. Nothing is ruled out, so every path counts, and a
    /// disagreement among them is a failure rather than a pass.
    #[test]
    fn a_term_the_model_does_not_have_does_not_become_a_pass() {
        let p = CallProperty {
            id: "x".into(),
            from: String::new(),
            expected: String::new(),
            why: String::new(),
            rules: vec![Rule {
                entrypoint: "withdraw(uint256)".into(),
                given: Some(Condition::Ugt {
                    left: Term::Env("number".into()),
                    right: Term::Storage("deadline".into()),
                }),
                outcome: Outcome::Reverts,
            }],
        };
        let paths = vec![path(false, IntervalSet::full(), &[])];
        let a = check(&p, &paths);
        assert_eq!(a.verdict, Verdict::Fails, "{}", a.because);
    }
}
