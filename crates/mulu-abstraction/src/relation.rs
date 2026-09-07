//! The canonical form of a condition.
//!
//! Two conditions that ask the same question should be recognised as the same
//! question, whether they are two guards in the same function or a guard and a
//! line of a specification. `require(amount <= balances[msg.sender])` reaches
//! the walk as
//!
//! ```text
//! iszero(gt(cleanup_t_uint256(expr_30), cleanup_t_uint256(expr_34)))
//! ```
//!
//! and the underflow check on `balances[msg.sender] -= amount` reaches it as
//! `iszero(gt(sub(cleanup(x), cleanup(y)), cleanup(x)))`. A specification
//! would write it as `argument 0 <= cell balances[caller]`. All three are the
//! same relation between the same two values, and this is where they meet.
//!
//! What is stripped are solc's own encoding wrappers: `cleanup_t_T`,
//! `convert_t_A_to_t_B`, `prepare_store_t_T` and `identity`. Each is the
//! identity on a value that already has the type it names, which is what the
//! `typed-domains` assumption already says holds.

use mulu_yul::Expr;
use std::collections::BTreeMap;

/// What a term needs from the contract to be named: the storage variable at a
/// literal slot.
pub trait Layout {
    fn label_at(&self, slot: crate::interval::U256) -> Option<String>;
}

impl<F: Fn(crate::interval::U256) -> Option<String>> Layout for F {
    fn label_at(&self, slot: crate::interval::U256) -> Option<String> {
        self(slot)
    }
}

fn slot_of(e: &Expr) -> Option<crate::interval::U256> {
    match e {
        Expr::Literal { text, .. } => crate::interval::parse_decimal(text).ok(),
        _ => None,
    }
}

/// A comparison in canonical form: `left OP right`, with the operator one of
/// four and the sides rendered as terms.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Relation {
    pub op: Op,
    pub left: String,
    pub right: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Op {
    /// Unsigned <
    Lt,
    /// Unsigned <=
    Le,
    Eq,
}

impl Relation {
    /// The key two equal relations share, and that a negated one does not.
    pub fn key(&self) -> String {
        let o = match self.op {
            Op::Lt => "<",
            Op::Le => "<=",
            Op::Eq => "==",
        };
        format!("{} {o} {}", self.left, self.right)
    }
}

/// `cleanup_t_uint256(x)` down to `x`, everywhere in the expression.
///
/// A wrapper is stripped by name, not by looking at its body: the names are
/// solc's own and their shape is fixed. Stripping one that is *not* the
/// identity on its argument would be unsound, which is why the list is
/// exactly the four encoding wrappers and nothing else.
pub fn strip(e: &Expr) -> Expr {
    match e {
        Expr::Call { name, args, src } => {
            if args.len() == 1 && is_encoding_wrapper(name) {
                return strip(&args[0]);
            }
            Expr::Call { name: name.clone(), args: args.iter().map(strip).collect(), src: *src }
        }
        other => other.clone(),
    }
}

fn is_encoding_wrapper(name: &str) -> bool {
    name == "identity"
        || name.starts_with("cleanup_t_")
        || name.starts_with("prepare_store_t_")
        || (name.starts_with("convert_t_") && name.contains("_to_t_"))
}

/// solc's storage idiom, rewritten to something a specification can write.
///
/// `read_from_storage_split_offset_0_t_uint256(mapping_index_access_t_mapping$
/// _t_address_$_t_uint256_$_of_t_address(0x00, caller()))` is what the code
/// says; `cell(balances, caller())` is what a specification would. The names
/// solc generates carry the types in them and change with the types, so
/// matching on them by prefix and rebuilding is the only stable form.
///
/// - `read_from_storage_*(x)` and `sload(x)` become `sload(x)`
/// - `mapping_index_access_*(slot, key)` becomes `mapping(slot, key)`
/// - `sload(<literal>)` becomes `storage(<the variable there>)`
/// - `sload(mapping(<literal>, k))` becomes `cell(<the variable there>, k)`
pub fn normalise(e: &Expr, layout: &impl Layout) -> Expr {
    let call = |name: &str, args: Vec<Expr>| Expr::Call {
        name: name.to_string(),
        args,
        src: None,
    };
    // A literal reads in decimal on both sides of the comparison, whichever
    // base solc wrote it in.
    if let Expr::Literal { text, src } = e {
        return match crate::interval::parse_decimal(text) {
            Ok(v) => Expr::Literal { text: v.to_string(), src: *src },
            Err(_) => e.clone(),
        };
    }
    let Expr::Call { name, args, src } = e else { return e.clone() };
    let args: Vec<Expr> = args.iter().map(|a| normalise(a, layout)).collect();

    if (name.starts_with("read_from_storage") || name == "sload") && args.len() == 1 {
        let named = match &args[0] {
            lit @ Expr::Literal { .. } => slot_of(lit)
                .and_then(|s| layout.label_at(s))
                .map(|l| call("storage", vec![Expr::Ident { name: l, src: None }])),
            Expr::Call { name: m, args: ma, .. } if m == "mapping" && ma.len() == 2 => {
                slot_of(&ma[0]).and_then(|s| layout.label_at(s)).map(|l| {
                    call("cell", vec![Expr::Ident { name: l, src: None }, ma[1].clone()])
                })
            }
            _ => None,
        };
        return named.unwrap_or_else(|| call("sload", args));
    }
    if name.starts_with("mapping_index_access") && args.len() == 2 {
        return call("mapping", args);
    }
    Expr::Call { name: name.clone(), args, src: *src }
}

/// The condition as a relation, with every local replaced by its term.
///
/// `None` when the condition is not a comparison this can name: a bare
/// boolean, a conjunction, arithmetic on both sides. Those still fork; they
/// just do not join up with anything.
pub fn of(cond: &Expr, terms: &BTreeMap<String, Expr>) -> Option<Relation> {
    of_in(cond, terms, &|_: crate::interval::U256| None)
}

/// The same, naming storage the way the contract's layout does.
pub fn of_in(
    cond: &Expr,
    terms: &BTreeMap<String, Expr>,
    layout: &impl Layout,
) -> Option<Relation> {
    let e = strip(&mulu_yul::fold::fold_fixpoint(&cond.substitute(terms)));
    rel(&normalise(&e, layout), false)
}

/// `negated` tracks an odd number of enclosing `iszero`, which is how solc
/// writes `<=` as `iszero(gt(..))`.
fn rel(e: &Expr, negated: bool) -> Option<Relation> {
    let Expr::Call { name, args, .. } = e else { return None };
    if name == "iszero" && args.len() == 1 {
        return rel(&args[0], !negated);
    }
    if args.len() != 2 {
        return None;
    }
    let a = strip(&args[0]).render();
    let b = strip(&args[1]).render();
    // `not(a < b)` is `b <= a`, and `not(a == b)` has no form here: it is a
    // disequality, which is two relations, so it is left alone.
    let (op, left, right) = match (name.as_str(), negated) {
        ("lt", false) => (Op::Lt, a, b),
        ("lt", true) => (Op::Le, b, a),
        ("gt", false) => (Op::Lt, b, a),
        ("gt", true) => (Op::Le, a, b),
        ("eq", false) => (Op::Eq, a, b),
        _ => return None,
    };
    Some(Relation { op, left, right })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `f(a, b)` and bare names, built rather than parsed: the crate has no
    /// expression parser and this needs no more than these two shapes.
    fn parse(src: &str) -> Expr {
        let src = src.trim();
        let Some(open) = src.find('(') else {
            return if crate::interval::parse_decimal(src).is_ok() {
                Expr::Literal { text: src.to_string(), src: None }
            } else {
                Expr::Ident { name: src.to_string(), src: None }
            };
        };
        let name = src[..open].to_string();
        let inner = &src[open + 1..src.rfind(')').expect("balanced")];
        let mut args = Vec::new();
        let (mut depth, mut start) = (0i32, 0usize);
        for (i, c) in inner.char_indices() {
            match c {
                '(' => depth += 1,
                ')' => depth -= 1,
                ',' if depth == 0 => {
                    args.push(parse(&inner[start..i]));
                    start = i + 1;
                }
                _ => {}
            }
        }
        if !inner.trim().is_empty() {
            args.push(parse(&inner[start..]));
        }
        Expr::Call { name, args, src: None }
    }

    #[test]
    fn the_encoding_wrappers_are_not_part_of_the_question() {
        let e = parse("gt(cleanup_t_uint256(a), convert_t_rational_0_by_1_to_t_uint256(b))");
        assert_eq!(strip(&e).render(), "gt(a, b)");
        // and only those: `sub` is arithmetic, `keccak256` is a function
        let e = parse("sub(a, keccak256(b, c))");
        assert_eq!(strip(&e).render(), e.render());
    }

    /// The two ways solc writes `a <= b` reach the same relation, and so does
    /// the way a specification would write it.
    #[test]
    fn the_same_question_asked_three_ways_has_one_key() {
        let t = BTreeMap::new();
        let from_guard = of(&parse("iszero(gt(cleanup_t_uint256(a), cleanup_t_uint256(b)))"), &t);
        let from_lt = of(&parse("iszero(lt(b, a))"), &t);
        let from_spec = of(&parse("iszero(gt(a, b))"), &t);
        assert_eq!(from_guard.as_ref().map(Relation::key).as_deref(), Some("a <= b"));
        assert_eq!(from_lt, from_guard);
        assert_eq!(from_spec, from_guard);
    }

    #[test]
    fn a_local_stands_for_what_defines_it() {
        let mut t = BTreeMap::new();
        t.insert("expr_30".to_string(), parse("var_amount_20"));
        t.insert("expr_34".to_string(), parse("sload(mapping_index_access(0x00, caller()))"));
        let r = of(&parse("iszero(gt(cleanup_t_uint256(expr_30), cleanup_t_uint256(expr_34)))"), &t)
            .expect("a relation");
        assert_eq!(r.key(), "var_amount_20 <= sload(mapping(0, caller()))");
    }

    /// The point of the whole module: the guard the compiler wrote and the
    /// line a specification would write reach the same key.
    #[test]
    fn a_mapping_cell_is_named_the_way_a_specification_would_name_it() {
        let layout = |slot: crate::interval::U256| {
            slot.is_zero().then(|| "balances".to_string())
        };
        let mut t = BTreeMap::new();
        t.insert("expr_30".to_string(), parse("var_amount_20"));
        t.insert(
            "expr_34".to_string(),
            parse(
                "read_from_storage_split_offset_0_t_uint256(\
                 mapping_index_access_t_mapping$_t_address_$_t_uint256_$_of_t_address(0x00, caller()))",
            ),
        );
        let from_code =
            of_in(&parse("iszero(gt(cleanup_t_uint256(expr_30), cleanup_t_uint256(expr_34)))"), &t, &layout)
                .expect("a relation");
        assert_eq!(from_code.key(), "var_amount_20 <= cell(balances, caller())");

        // and a plain slot is named by its variable
        t.insert("g".to_string(), parse("read_from_storage_split_offset_0_t_uint256(0x00)"));
        let r = of_in(&parse("lt(g, x)"), &t, &layout).expect("a relation");
        assert_eq!(r.key(), "storage(balances) < x");
    }

    #[test]
    fn what_is_not_a_comparison_has_no_relation() {
        let t = BTreeMap::new();
        assert!(of(&parse("var_success_46"), &t).is_none());
        assert!(of(&parse("and(a, b)"), &t).is_none());
        // a disequality is two relations, not one
        assert!(of(&parse("iszero(eq(a, b))"), &t).is_none());
    }
}
