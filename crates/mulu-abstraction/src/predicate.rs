//! Yul guard conditions as predicates over one `uint256` variable.
//!
//! The P1a fragment (docs/09 §1: "pure, total comparisons") is: a comparison
//! between one variable and one literal, combined with `and`, `or` and
//! `iszero`. Inside it the interval representation is exact. Outside it the
//! answer is `Unsupported` with a reason, never a guess and never `true`
//! (docs/09 §3: "未知の演算子や未解決の変数を true として扱わない").

use crate::interval::{parse_decimal, IntervalSet, U256};
use mulu_yul::Expr;
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Predicate {
    /// Holds everywhere.
    True,
    /// Holds nowhere.
    False,
    /// Holds exactly when `var` takes a value in `set`.
    Over { var: String, set: IntervalSet },
}

impl Predicate {
    pub fn negate(&self) -> Predicate {
        match self {
            Predicate::True => Predicate::False,
            Predicate::False => Predicate::True,
            Predicate::Over { var, set } => {
                Predicate::normalise(var.clone(), set.complement())
            }
        }
    }

    fn normalise(var: String, set: IntervalSet) -> Predicate {
        if set.is_empty() {
            Predicate::False
        } else if set.is_full() {
            Predicate::True
        } else {
            Predicate::Over { var, set }
        }
    }

    pub fn and(&self, other: &Predicate) -> Result<Predicate, String> {
        match (self, other) {
            (Predicate::False, _) | (_, Predicate::False) => Ok(Predicate::False),
            (Predicate::True, p) | (p, Predicate::True) => Ok(p.clone()),
            (Predicate::Over { var: a, set: s }, Predicate::Over { var: b, set: t }) => {
                if a != b {
                    return Err(format!(
                        "a guard over two variables ({a} and {b}) is outside the P1a fragment"
                    ));
                }
                Ok(Predicate::normalise(a.clone(), s.intersect(t)))
            }
        }
    }

    pub fn or(&self, other: &Predicate) -> Result<Predicate, String> {
        match (self, other) {
            (Predicate::True, _) | (_, Predicate::True) => Ok(Predicate::True),
            (Predicate::False, p) | (p, Predicate::False) => Ok(p.clone()),
            (Predicate::Over { var: a, set: s }, Predicate::Over { var: b, set: t }) => {
                if a != b {
                    return Err(format!(
                        "a guard over two variables ({a} and {b}) is outside the P1a fragment"
                    ));
                }
                Ok(Predicate::normalise(a.clone(), s.union(t)))
            }
        }
    }

    /// The variable this predicate constrains, if any.
    pub fn var(&self) -> Option<&str> {
        match self {
            Predicate::Over { var, .. } => Some(var),
            _ => None,
        }
    }

    /// The values of the variable that satisfy it (everything, for `True`).
    pub fn set(&self) -> IntervalSet {
        match self {
            Predicate::True => IntervalSet::full(),
            Predicate::False => IntervalSet::empty(),
            Predicate::Over { set, .. } => set.clone(),
        }
    }

    /// Decide the predicate on a region of the variable's domain.
    /// `None` means the region straddles the boundary, which cannot happen
    /// once the region partition has been refined by this predicate.
    pub fn decide(&self, env: &crate::value::Env) -> Option<bool> {
        match self {
            Predicate::True => Some(true),
            Predicate::False => Some(false),
            Predicate::Over { var: v, set } => {
                let Some(region) = env.get(v) else { return None };
                if region.subset_of(set) {
                    Some(true)
                } else if region.disjoint_from(set) {
                    Some(false)
                } else {
                    None
                }
            }
        }
    }
}

impl fmt::Display for Predicate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Predicate::True => write!(f, "true"),
            Predicate::False => write!(f, "false"),
            Predicate::Over { var, set } => write!(f, "{var} ∈ {set}"),
        }
    }
}

/// One side of a comparison, once resolved.
enum Operand {
    Var(String),
    Lit(U256),
}

/// Drop a cleanup that cannot change the value: `and(v, mask)` is `v` when the
/// variable's declared type keeps it inside the mask. A `uint8` argument
/// compared with `x <= 100` reaches here as `and(x, 0xff)`.
fn strip_cleanup(e: &Expr, domain: &IntervalSet) -> Expr {
    let Expr::Call { name, args, .. } = e else { return e.clone() };
    if name != "and" || args.len() != 2 {
        return e.clone();
    }
    let (inner, mask) = match (literal_of(&args[0]), literal_of(&args[1])) {
        (None, Some(m)) => (&args[0], m),
        (Some(m), None) => (&args[1], m),
        _ => return e.clone(),
    };
    if domain.subset_of(&IntervalSet::le(mask)) {
        strip_cleanup(inner, domain)
    } else {
        e.clone()
    }
}

fn literal_of(e: &Expr) -> Option<U256> {
    match e {
        Expr::Literal { text, .. } => parse_decimal(text).ok(),
        _ => None,
    }
}

fn operand(e: &Expr, vars: &[String]) -> Result<Operand, String> {
    match e {
        Expr::Ident { name, .. } => {
            if vars.iter().any(|v| v == name) {
                Ok(Operand::Var(name.clone()))
            } else {
                Err(format!("`{name}` is not one of this function's parameters"))
            }
        }
        Expr::Literal { text, .. } => {
            parse_decimal(text).map(Operand::Lit).map_err(|e| e.to_string())
        }
        Expr::Call { name, .. } => {
            Err(format!("`{name}(...)` in a comparison is outside the P1a fragment"))
        }
    }
}

/// `a op b` where exactly one side is a variable. `flip` gives the comparison
/// to use when the variable is on the right.
fn compare(
    a: &Expr,
    b: &Expr,
    vars: &[String],
    on_left: impl Fn(U256) -> IntervalSet,
    on_right: impl Fn(U256) -> IntervalSet,
) -> Result<Predicate, String> {
    match (operand(a, vars)?, operand(b, vars)?) {
        (Operand::Var(v), Operand::Lit(k)) => Ok(Predicate::normalise(v, on_left(k))),
        (Operand::Lit(k), Operand::Var(v)) => Ok(Predicate::normalise(v, on_right(k))),
        (Operand::Lit(_), Operand::Lit(_)) => {
            Err("a comparison of two literals should have been folded away".into())
        }
        (Operand::Var(_), Operand::Var(_)) => {
            Err("a comparison of two variables is outside the P1a fragment".into())
        }
    }
}

/// Translate a Yul condition. It holds when the expression is non-zero.
pub fn translate(e: &Expr, vars: &[String]) -> Result<Predicate, String> {
    translate_in(e, vars, &IntervalSet::full())
}

/// As `translate`, knowing what the variable's declared type admits. That is
/// what makes solc's narrowing cleanups droppable rather than opaque.
pub fn translate_in(e: &Expr, vars: &[String], domain: &IntervalSet) -> Result<Predicate, String> {
    let e = &strip_cleanup(e, domain);
    match e {
        Expr::Literal { text, .. } => {
            let v = parse_decimal(text).map_err(|e| e.to_string())?;
            Ok(if v.is_zero() { Predicate::False } else { Predicate::True })
        }
        // A bare variable used as a condition means "non-zero".
        Expr::Ident { name, .. } => {
            if vars.iter().any(|v| v == name) {
                Ok(Predicate::Over { var: name.clone(), set: IntervalSet::ne_to(U256::ZERO) })
            } else {
                Err(format!("`{name}` is not one of this function's parameters"))
            }
        }
        Expr::Call { name, args, .. } => {
            let arity = |n: usize| -> Result<(), String> {
                if args.len() == n {
                    Ok(())
                } else {
                    Err(format!("`{name}` with {} arguments", args.len()))
                }
            };
            match name.as_str() {
                "iszero" => {
                    arity(1)?;
                    Ok(translate_in(&args[0], vars, domain)?.negate())
                }
                // Unsigned comparisons.
                "gt" => {
                    arity(2)?;
                    let (l, r) = (strip_cleanup(&args[0], domain), strip_cleanup(&args[1], domain));
                    compare(&l, &r, vars, IntervalSet::gt, IntervalSet::lt)
                }
                "lt" => {
                    arity(2)?;
                    let (l, r) = (strip_cleanup(&args[0], domain), strip_cleanup(&args[1], domain));
                    compare(&l, &r, vars, IntervalSet::lt, IntervalSet::gt)
                }
                "eq" => {
                    arity(2)?;
                    // `eq(e, e)` is true whatever `e` is, which is how the
                    // uint256 ABI validator collapses.
                    let (l, r) = (strip_cleanup(&args[0], domain), strip_cleanup(&args[1], domain));
                    if l.render() == r.render() {
                        return Ok(Predicate::True);
                    }
                    compare(&l, &r, vars, IntervalSet::eq_to, IntervalSet::eq_to)
                }
                // Boolean combination. Yul's `and`/`or` are bitwise, so this is
                // only valid when both sides are 0/1, which holds for the
                // comparison results solc feeds them.
                "and" => {
                    arity(2)?;
                    translate_in(&args[0], vars, domain)?
                        .and(&translate_in(&args[1], vars, domain)?)
                }
                "or" => {
                    arity(2)?;
                    translate_in(&args[0], vars, domain)?
                        .or(&translate_in(&args[1], vars, domain)?)
                }
                "slt" | "sgt" => Err(format!(
                    "`{name}` is a signed comparison; P1a models unsigned uint256 only"
                )),
                other => Err(format!("`{other}` is outside the P1a guard fragment")),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mulu_yul::parse_object;

    /// Parse `let c := <expr>` and translate the expression.
    fn tr(yul_expr: &str, vars: &[&str]) -> Result<Predicate, String> {
        let src = format!("object \"T\" {{ code {{ let c := {yul_expr} }} }}");
        let p = parse_object(&src).unwrap();
        let mulu_yul::Stmt::Let { value: Some(e), .. } = &p.object.code.stmts[0] else {
            panic!("expected a let")
        };
        let vars: Vec<String> = vars.iter().map(|s| s.to_string()).collect();
        translate(e, &vars)
    }

    fn u(n: u64) -> U256 {
        U256::from(n)
    }

    #[test]
    fn the_two_limits_guards() {
        // A: iszero(gt(x, 100)) is x <= 100
        let a = tr("iszero(gt(x, 100))", &["x"]).unwrap();
        assert_eq!(a, Predicate::Over { var: "x".into(), set: IntervalSet::le(u(100)) });
        // B: iszero(gt(x, 1000)) is x <= 1000
        let b = tr("iszero(gt(x, 1000))", &["x"]).unwrap();
        assert_eq!(b, Predicate::Over { var: "x".into(), set: IntervalSet::le(u(1000)) });
        // and A implies B, which is exactly why B never fails after A
        assert!(a.set().subset_of(&b.set()));
    }

    #[test]
    fn hex_and_decimal_literals_agree() {
        assert_eq!(tr("gt(x, 0x64)", &["x"]).unwrap(), tr("gt(x, 100)", &["x"]).unwrap());
        assert_eq!(tr("gt(x, 0x03e8)", &["x"]).unwrap(), tr("gt(x, 1000)", &["x"]).unwrap());
    }

    #[test]
    fn the_variable_may_sit_on_either_side() {
        // 100 < x is the same as x > 100
        assert_eq!(tr("lt(100, x)", &["x"]).unwrap(), tr("gt(x, 100)", &["x"]).unwrap());
        assert_eq!(tr("gt(100, x)", &["x"]).unwrap(), tr("lt(x, 100)", &["x"]).unwrap());
    }

    #[test]
    fn boolean_combination() {
        let both = tr("and(iszero(lt(x, 10)), iszero(gt(x, 20)))", &["x"]).unwrap();
        assert_eq!(both, Predicate::Over { var: "x".into(), set: IntervalSet::range(u(10), u(20)) });
        let either = tr("or(lt(x, 10), gt(x, 20))", &["x"]).unwrap();
        assert_eq!(either, both.negate());
    }

    #[test]
    fn a_trivially_true_comparison_collapses() {
        // the uint256 ABI validator reduces to eq(v, v)
        assert_eq!(tr("eq(value, value)", &["value"]).unwrap(), Predicate::True);
        assert_eq!(tr("iszero(gt(x, 0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff))", &["x"]).unwrap(), Predicate::True);
        assert_eq!(tr("lt(x, 0)", &["x"]).unwrap(), Predicate::False);
    }

    #[test]
    fn what_is_outside_the_fragment_is_refused_not_guessed() {
        for (expr, vars) in [
            ("slt(x, 100)", &["x"][..]),          // signed
            ("gt(sub(a, b), 32)", &["a", "b"]),   // arithmetic
            ("gt(x, y)", &["x", "y"]),            // two variables
            ("gt(calldatasize(), 4)", &["x"]),    // environment read
            ("callvalue()", &["x"]),              // not a comparison
            ("gt(unknown_var, 1)", &["x"]),       // unresolved name
        ] {
            let vars: Vec<String> = vars.iter().map(|s| s.to_string()).collect();
            let src = format!("object \"T\" {{ code {{ let c := {expr} }} }}");
            let p = parse_object(&src).unwrap();
            let mulu_yul::Stmt::Let { value: Some(e), .. } = &p.object.code.stmts[0] else {
                panic!()
            };
            assert!(translate(e, &vars).is_err(), "{expr} must be refused, not guessed");
        }
    }

    #[test]
    fn a_narrowing_cleanup_is_dropped_when_the_type_justifies_it() {
        let byte = crate::types::domain_of("uint8").unwrap();
        // a uint8 argument reaches a comparison through and(x, 0xff)
        let src = "object \"T\" { code { let c := iszero(gt(and(x, 0xff), 100)) } }";
        let p = parse_object(src).unwrap();
        let mulu_yul::Stmt::Let { value: Some(e), .. } = &p.object.code.stmts[0] else { panic!() };
        let vars = vec!["x".to_string()];
        let got = translate_in(e, &vars, &byte).unwrap();
        assert_eq!(got, Predicate::Over { var: "x".into(), set: IntervalSet::le(u(100)) });

        // without the type the mask could truncate, so it is not dropped
        assert!(translate_in(e, &vars, &IntervalSet::full()).is_err());
    }

    #[test]
    fn deciding_a_predicate_on_a_region() {
        let a = tr("iszero(gt(x, 100))", &["x"]).unwrap();
        assert_eq!(a.decide(&crate::value::env_of("x", &IntervalSet::le(u(100)))), Some(true));
        assert_eq!(a.decide(&crate::value::env_of("x", &IntervalSet::range(u(101), u(1000)))), Some(false));
        // straddling the boundary is undecided, never a guess
        assert_eq!(a.decide(&crate::value::env_of("x", &IntervalSet::le(u(200)))), None);
        // a predicate over another variable says nothing about this one
        assert_eq!(a.decide(&crate::value::env_of("y", &IntervalSet::le(u(100)))), None);
    }

    #[test]
    fn negation_round_trips() {
        for e in ["iszero(gt(x, 100))", "lt(x, 7)", "eq(x, 3)", "or(lt(x, 2), gt(x, 9))"] {
            let p = tr(e, &["x"]).unwrap();
            assert_eq!(p.negate().negate(), p, "{e}");
            assert!(p.set().intersect(&p.negate().set()).is_empty());
            assert!(p.set().union(&p.negate().set()).is_full());
        }
    }
}
