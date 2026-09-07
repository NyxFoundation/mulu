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
    /// The name of the `immutable` variable solc gave this AST id.
    fn immutable(&self, _id: &str) -> Option<String> {
        None
    }
    /// How many writes the model could not place have happened so far.
    fn cell_generation(&self) -> u32 {
        0
    }
}

impl<F: Fn(crate::interval::U256) -> Option<String>> Layout for F {
    fn label_at(&self, slot: crate::interval::U256) -> Option<String> {
        self(slot)
    }
}

/// A slot table, an immutable table, and how many times each slot has been
/// written on the path so far.
///
/// The version is what keeps a term honest about *when* it was read.
/// `storage(x)` before a write and after it are two values, and rendering
/// both the same let a fact recorded before the write be used after it. A
/// specification talks about the state the call started in, which is
/// version 0, so that one keeps the plain name.
pub struct Names {
    pub slots: BTreeMap<crate::interval::U256, String>,
    pub immutables: BTreeMap<String, String>,
    /// Declared slot label to how many times it has been written.
    pub versions: BTreeMap<String, u32>,
    /// How many times a slot the model does not track has been written. Any
    /// of them could have been a cell of any mapping, so one counter covers
    /// them all.
    pub cell_version: u32,
}

impl Names {
    fn stamp(&self, label: &str) -> String {
        match self.versions.get(label).copied().unwrap_or(0) {
            0 => label.to_string(),
            n => format!("{label}@{n}"),
        }
    }
}

impl Layout for Names {
    fn label_at(&self, slot: crate::interval::U256) -> Option<String> {
        self.slots.get(&slot).map(|l| self.stamp(l))
    }
    fn immutable(&self, id: &str) -> Option<String> {
        // An immutable is written once, at deployment, and never again.
        self.immutables.get(id).cloned()
    }
    fn cell_generation(&self) -> u32 {
        self.cell_version
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
            // `and(x, 2^k - 1)` is what `cleanup_t_address` and its kin
            // become once the lowerer has inlined them, and it is the
            // identity on a value that already fits in k bits. solc emits it
            // exactly where the value does, which is the `typed-domains`
            // assumption the model already records. Only masks of that shape:
            // `and(add(size, 31), not(31))` clears low bits and is arithmetic.
            if name == "and" && args.len() == 2 {
                if let Some(k) = low_bit_mask(&args[1]) {
                    if k > 0 && k <= 256 && k % 8 == 0 {
                        return strip(&args[0]);
                    }
                }
            }
            Expr::Call {
                name: name.clone(),
                args: args.iter().map(strip).collect(),
                src: *src,
            }
        }
        other => other.clone(),
    }
}

/// `2^k - 1` as a width in bits, when the literal is one.
fn low_bit_mask(e: &Expr) -> Option<u32> {
    let Expr::Literal { text, .. } = e else {
        return None;
    };
    let v = crate::interval::parse_decimal(text).ok()?;
    let bits = 256 - v.leading_zeros() as u32;
    let all_ones = if bits >= 256 {
        crate::interval::max_u256()
    } else {
        (crate::interval::U256::from(1u8) << bits as usize) - crate::interval::U256::from(1u8)
    };
    (v == all_ones).then_some(bits)
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
            Ok(v) => Expr::Literal {
                text: v.to_string(),
                src: *src,
            },
            Err(_) => e.clone(),
        };
    }
    let Expr::Call { name, args, src } = e else {
        return e.clone();
    };
    let args: Vec<Expr> = args.iter().map(|a| normalise(a, layout)).collect();

    if (name.starts_with("read_from_storage") || name == "sload") && args.len() == 1 {
        let named = match &args[0] {
            lit @ Expr::Literal { .. } => slot_of(lit)
                .and_then(|s| layout.label_at(s))
                .map(|l| call("storage", vec![Expr::Ident { name: l, src: None }])),
            Expr::Call {
                name: m, args: ma, ..
            } if m == "mapping" && ma.len() == 2 => {
                slot_of(&ma[0]).and_then(|s| layout.label_at(s)).map(|l| {
                    let l = match layout.cell_generation() {
                        0 => l,
                        n => format!("{l}@{n}"),
                    };
                    call(
                        "cell",
                        vec![Expr::Ident { name: l, src: None }, ma[1].clone()],
                    )
                })
            }
            _ => None,
        };
        return named.unwrap_or_else(|| call("sload", args));
    }
    if name.starts_with("mapping_index_access") && args.len() == 2 {
        return call("mapping", args);
    }
    // A dynamic array's length is the word at its slot, which is also what
    // `sload` of that slot reads. Two names for one value meant the walk
    // could not see that a length read after a push is the length read
    // before it, plus one.
    if name.starts_with("array_length") && args.len() == 1 {
        return normalise(&call("sload", args), layout);
    }
    if name.starts_with("convert_array") && args.len() == 1 {
        return args.into_iter().next().expect("one argument");
    }
    // `loadimmutable("13")` is a read of the immutable declared at AST node
    // 13. The id is solc's and changes with the source; the name does not.
    if name == "loadimmutable" && args.len() == 1 {
        if let Expr::Literal { text, .. } = &args[0] {
            let id = text.trim_matches('"');
            if let Some(n) = layout.immutable(id) {
                return call("immutable", vec![Expr::Ident { name: n, src: None }]);
            }
        }
    }
    Expr::Call {
        name: name.clone(),
        args,
        src: *src,
    }
}

/// The condition as a relation, with every local replaced by its term.
///
/// `None` when the condition is not a comparison this can name: a bare
/// boolean, a conjunction, arithmetic on both sides. Those still fork; they
/// just do not join up with anything.
pub fn of(cond: &Expr, terms: &BTreeMap<String, Expr>) -> Option<Relation> {
    of_in(cond, terms, &|_: crate::interval::U256| None).map(|(r, _)| r)
}

/// The same, naming storage the way the contract's layout does.
/// The relation, and whether the condition is that relation or its negation.
/// `iszero(eq(a, b))` is the relation `a == b` with the sense `false`: one
/// key, two sides, which is what lets a `require(a == b)` and the `if
/// iszero(a == b)` solc writes for the other half of an `||` meet.
pub fn of_in(
    cond: &Expr,
    terms: &BTreeMap<String, Expr>,
    layout: &impl Layout,
) -> Option<(Relation, bool)> {
    let e = strip(&mulu_yul::fold::fold_fixpoint(&cond.substitute(terms)));
    rel(&normalise(&e, layout), false)
}

/// `negated` tracks an odd number of enclosing `iszero`, which is how solc
/// writes `<=` as `iszero(gt(..))`.
fn rel(e: &Expr, negated: bool) -> Option<(Relation, bool)> {
    // A Yul condition is "not zero", so any expression used as one is the
    // relation `e == 0` read the other way round. `require(isCommitted)`
    // becomes `storage(isCommitted) == 0` with the sense reversed, which is
    // the same key a specification writing `isCommitted == false` produces.
    let as_nonzero = |e: &Expr| {
        Some((
            Relation {
                op: Op::Eq,
                left: e.render(),
                right: "0".to_string(),
            },
            negated,
        ))
    };
    let Expr::Call { name, args, .. } = e else {
        return as_nonzero(e);
    };
    if name == "iszero" && args.len() == 1 {
        return rel(&args[0], !negated);
    }
    if args.len() != 2 {
        return as_nonzero(e);
    }
    let a = strip(&args[0]).render();
    let b = strip(&args[1]).render();
    // `not(a < b)` is `b <= a`, and `not(a == b)` has no form here: it is a
    // disequality, which is two relations, so it is left alone.
    // `sub(a, b) <= a` is solc's underflow check, and it says exactly
    // `b <= a`. Leaving it in the arithmetic hid a relation the rest of the
    // walk already had, so the same fact was assumed twice, once each way.
    if let (
        Expr::Call {
            name: sub,
            args: sa,
            ..
        },
        b,
    ) = (&strip(&args[0]), &strip(&args[1]))
    {
        if sub == "sub" && sa.len() == 2 && strip(&sa[0]).render() == b.render() {
            let (x, y) = (strip(&sa[0]).render(), strip(&sa[1]).render());
            match (name.as_str(), negated) {
                // gt(sub(a, b), a) negated is sub(a, b) <= a, i.e. b <= a
                ("gt", true) => {
                    return Some((
                        Relation {
                            op: Op::Le,
                            left: y,
                            right: x,
                        },
                        true,
                    ))
                }
                ("gt", false) => {
                    return Some((
                        Relation {
                            op: Op::Lt,
                            left: x,
                            right: y,
                        },
                        true,
                    ))
                }
                _ => {}
            }
        }
    }
    let (op, left, right, sense) = match (name.as_str(), negated) {
        ("lt", false) => (Op::Lt, a, b, true),
        ("lt", true) => (Op::Le, b, a, true),
        ("gt", false) => (Op::Lt, b, a, true),
        ("gt", true) => (Op::Le, a, b, true),
        ("eq", false) => (Op::Eq, a, b, true),
        // A disequality is one relation read the other way round, not two.
        ("eq", true) => (Op::Eq, a, b, false),
        _ => return as_nonzero(e),
    };
    Some((Relation { op, left, right }, sense))
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
                Expr::Literal {
                    text: src.to_string(),
                    src: None,
                }
            } else {
                Expr::Ident {
                    name: src.to_string(),
                    src: None,
                }
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
        Expr::Call {
            name,
            args,
            src: None,
        }
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
        let from_guard = of(
            &parse("iszero(gt(cleanup_t_uint256(a), cleanup_t_uint256(b)))"),
            &t,
        );
        let from_lt = of(&parse("iszero(lt(b, a))"), &t);
        let from_spec = of(&parse("iszero(gt(a, b))"), &t);
        assert_eq!(
            from_guard.as_ref().map(Relation::key).as_deref(),
            Some("a <= b")
        );
        assert_eq!(from_lt, from_guard);
        assert_eq!(from_spec, from_guard);
    }

    #[test]
    fn a_local_stands_for_what_defines_it() {
        let mut t = BTreeMap::new();
        t.insert("expr_30".to_string(), parse("var_amount_20"));
        t.insert(
            "expr_34".to_string(),
            parse("sload(mapping_index_access(0x00, caller()))"),
        );
        let r = of(
            &parse("iszero(gt(cleanup_t_uint256(expr_30), cleanup_t_uint256(expr_34)))"),
            &t,
        )
        .expect("a relation");
        assert_eq!(r.key(), "var_amount_20 <= sload(mapping(0, caller()))");
    }

    /// solc compares two addresses by masking both to 160 bits. The mask is
    /// the identity on an address, which is what the values being compared
    /// are, so the question is about the two addresses.
    #[test]
    fn the_mask_solc_puts_on_a_typed_value_is_not_part_of_the_question() {
        let m = "1461501637330902918203684832716283019655932542975";
        let t = BTreeMap::new();
        let r =
            of(&parse(&format!("eq(and(caller(), {m}), and(o, {m}))")), &t).expect("a relation");
        assert_eq!(r.key(), "caller() == o");
        // and a mask that is not `2^k - 1` is arithmetic, and stays
        let keep = parse("and(add(size, 31), 115792089237316195423570985008687907853269984665640564039457584007913129639904)");
        assert_eq!(strip(&keep).render(), keep.render());
    }

    /// The point of the whole module: the guard the compiler wrote and the
    /// line a specification would write reach the same key.
    #[test]
    fn a_mapping_cell_is_named_the_way_a_specification_would_name_it() {
        let layout = |slot: crate::interval::U256| slot.is_zero().then(|| "balances".to_string());
        let mut t = BTreeMap::new();
        t.insert("expr_30".to_string(), parse("var_amount_20"));
        t.insert(
            "expr_34".to_string(),
            parse(
                "read_from_storage_split_offset_0_t_uint256(\
                 mapping_index_access_t_mapping$_t_address_$_t_uint256_$_of_t_address(0x00, caller()))",
            ),
        );
        let from_code = of_in(
            &parse("iszero(gt(cleanup_t_uint256(expr_30), cleanup_t_uint256(expr_34)))"),
            &t,
            &layout,
        )
        .expect("a relation");
        assert_eq!(
            from_code.0.key(),
            "var_amount_20 <= cell(balances, caller())"
        );

        // and a plain slot is named by its variable
        t.insert(
            "g".to_string(),
            parse("read_from_storage_split_offset_0_t_uint256(0x00)"),
        );
        let r = of_in(&parse("lt(g, x)"), &t, &layout).expect("a relation");
        assert_eq!(r.0.key(), "storage(balances) < x");
    }

    /// solc checks `a - b` for underflow by asking whether the difference
    /// came out bigger than `a`. That is `b <= a`, and saying so joins the
    /// check up with the guard the source wrote.
    #[test]
    fn the_underflow_check_is_the_relation_it_stands_for() {
        let t = BTreeMap::new();
        let r = of(&parse("iszero(gt(sub(cleanup_t_uint256(bal), cleanup_t_uint256(amt)), cleanup_t_uint256(bal)))"), &t)
            .expect("a relation");
        assert_eq!(r.key(), "amt <= bal");
        // and the other side of it
        let r = of(&parse("gt(sub(bal, amt), bal)"), &t).expect("a relation");
        assert_eq!(r.key(), "bal < amt");
    }

    /// A condition that is not a comparison is still a question: Yul reads
    /// it as "not zero", and that is one relation with the sense reversed.
    /// `require(isCommitted)` and a specification's `isCommitted == false`
    /// then meet on one key.
    #[test]
    fn a_condition_that_is_not_a_comparison_is_the_relation_against_zero() {
        let t = BTreeMap::new();
        let (r, sense) =
            of_in(&parse("flag"), &t, &|_: crate::interval::U256| None).expect("a relation");
        assert_eq!((r.key().as_str(), sense), ("flag == 0", false));
        // and `iszero` of it is the same key, the other way
        let (r, sense) =
            of_in(&parse("iszero(flag)"), &t, &|_: crate::interval::U256| None).expect("one");
        assert_eq!((r.key().as_str(), sense), ("flag == 0", true));
    }

    /// A disequality is the equality read the other way round, not two
    /// relations, and a conjunction is neither.
    #[test]
    fn a_disequality_is_the_equality_with_the_sense_reversed() {
        let t = BTreeMap::new();
        let no = |_: crate::interval::U256| None;
        let (r, sense) = of_in(&parse("iszero(eq(a, b))"), &t, &no).expect("a relation");
        assert_eq!((r.key().as_str(), sense), ("a == b", false));
        // a conjunction has no single relation, so it is keyed as a whole
        let (r, sense) = of_in(&parse("and(a, b)"), &t, &no).expect("a key");
        assert_eq!((r.key().as_str(), sense), ("and(a, b) == 0", false));
    }
}
