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
    #[serde(default)]
    pub call_properties: Vec<CallProperty>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CallProperty {
    pub id: String,
    /// The benchmark's own encoding this one was written from, so a reader
    /// can put them side by side.
    #[serde(default)]
    pub from: String,
    /// The ABI signature, matching an entrypoint of the contract.
    pub entrypoint: String,
    /// The requests the property is about. Absent means all of them.
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
    Cell { var: String, key: Box<Term> },
    /// Transaction context: `caller`, `callvalue`, `number`, `timestamp`,
    /// `origin`, `selfbalance`.
    Env(String),
    /// A literal, in decimal or 0x hex.
    Uint256(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Condition {
    /// Unsigned <=, <, >=, >
    Ule { left: Term, right: Term },
    Ult { left: Term, right: Term },
    Uge { left: Term, right: Term },
    Ugt { left: Term, right: Term },
    Eq { left: Term, right: Term },
    Ne { left: Term, right: Term },
    And { args: Vec<Condition> },
    Or { args: Vec<Condition> },
    Not { arg: Box<Condition> },
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
        Term::Cell { var, key } => format!("cell({var}, {})", render(key, p)?),
        Term::Env(name) => format!("{name}()"),
        Term::Uint256(text) => parse_decimal(text).ok()?.to_string(),
    })
}

/// The values the term can take on this path, when the path says.
fn values(t: &Term, p: &PathSummary) -> Option<IntervalSet> {
    match t {
        Term::Argument(i) => p.arguments.get(*i).cloned(),
        Term::Storage(v) => p.storage.get(v).cloned(),
        Term::Uint256(text) => parse_decimal(text).ok().map(IntervalSet::point),
        // A mapping cell and the transaction context are not in the model's
        // state. What is known about them is what the path assumed, which is
        // read as a relation rather than as a set.
        Term::Cell { .. } | Term::Env(_) => None,
    }
}

/// Does the relation `left OP right` hold on this path?
fn holds(op: Op, left: &Term, right: &Term, p: &PathSummary) -> Tri {
    // Intervals first: exact where both sides are in the model's state.
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
        return Tri::Unknown;
    }
    // Otherwise the path has to have taken a side on it.
    let (Some(l), Some(r)) = (render(left, p), render(right, p)) else { return Tri::Unknown };
    let key = Relation { op, left: l.clone(), right: r.clone() }.key();
    if let Some(v) = p.assumed.get(&key) {
        return if *v { Tri::True } else { Tri::False };
    }
    // `a <= b` is settled by `b < a` too, and by `a < b`.
    let flip = |op: Op, a: &str, b: &str| {
        Relation { op, left: a.to_string(), right: b.to_string() }.key()
    };
    match op {
        Op::Le => {
            if let Some(v) = p.assumed.get(&flip(Op::Lt, &r, &l)) {
                return if *v { Tri::False } else { Tri::True };
            }
            if p.assumed.get(&flip(Op::Lt, &l, &r)) == Some(&true) {
                return Tri::True;
            }
        }
        Op::Lt => {
            if let Some(v) = p.assumed.get(&flip(Op::Le, &r, &l)) {
                return if *v { Tri::False } else { Tri::True };
            }
        }
        Op::Eq => {}
    }
    Tri::Unknown
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
        And { args } => args.iter().fold(Tri::True, |acc, a| acc.and(evaluate(a, p))),
        Or { args } => args.iter().fold(Tri::False, |acc, a| acc.or(evaluate(a, p))),
        Not { arg } => evaluate(arg, p).not(),
    }
}

/// Answer one property against the paths of one abstraction.
pub fn check(prop: &CallProperty, paths: &[PathSummary]) -> Answer {
    let mine: Vec<&PathSummary> =
        paths.iter().filter(|p| p.entrypoint == prop.entrypoint).collect();
    if mine.is_empty() {
        return Answer {
            id: prop.id.clone(),
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
            "every one of the {} path(s) a matching request can be on {}",
            possible.len(),
            if want_revert { "reverts" } else { "returns" }
        ),
        (Verdict::Fails, Some(p)) => format!(
            "a request on a path that {} matches: arguments {}, assuming {}",
            if p.reverts { "reverts" } else { "returns" },
            p.arguments.iter().map(|a| a.to_string()).collect::<Vec<_>>().join(", "),
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
        _ => "no path of this entrypoint can carry a matching request".to_string(),
    };
    Answer {
        id: prop.id.clone(),
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
            entrypoint: "withdraw(uint256)".into(),
            given: Some(Condition::Ule {
                left: Term::Argument(0),
                right: Term::Cell {
                    var: "balances".into(),
                    key: Box::new(Term::Env("caller".into())),
                },
            }),
            outcome: Outcome::DoesNotRevert,
        };
        let paths = vec![
            path(false, IntervalSet::ge(u(1)), &[(LE_BAL, true)]),
            // the `require(success)` on the external call
            path(true, IntervalSet::ge(u(1)), &[(LE_BAL, true), ("var_success_46", false)]),
        ];
        let a = check(&not_revert, &paths);
        assert_eq!(a.verdict, Verdict::Fails, "{}", a.because);
    }

    /// A property naming something no path mentions is not answered "holds"
    /// by default. Nothing is ruled out, so every path counts, and a
    /// disagreement among them is a failure rather than a pass.
    #[test]
    fn a_term_the_model_does_not_have_does_not_become_a_pass() {
        let p = CallProperty {
            id: "x".into(),
            from: String::new(),
            entrypoint: "withdraw(uint256)".into(),
            given: Some(Condition::Ugt {
                left: Term::Env("number".into()),
                right: Term::Storage("deadline".into()),
            }),
            outcome: Outcome::Reverts,
        };
        let paths = vec![path(false, IntervalSet::full(), &[])];
        let a = check(&p, &paths);
        assert_eq!(a.verdict, Verdict::Fails, "{}", a.because);
    }
}
