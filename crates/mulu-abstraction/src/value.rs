//! What values an expression can take, given what its variable can take.
//!
//! Storing a narrower type into a wider slot makes solc insert a cleanup:
//! `reading = x` with `x : uint8` becomes `and(x, 0xff)`. That mask is the
//! identity only because `x` is a uint8, which is exactly what the ABI type
//! tells us. Without the type it cannot be justified, and with it, it can.
//!
//! Anything this cannot evaluate exactly is refused. A truncating mask, one
//! that really does change values in range, is not silently treated as a
//! no-op.

use crate::interval::{IntervalSet, U256};
use mulu_yul::Expr;

/// The set of values `e` can produce when `var` ranges over `region`.
/// What the walk knows about the variables in scope: each name maps to the
/// values it can take in the region being walked.
///
/// This replaced a single `(name, region)` pair. One entrypoint argument was
/// the only thing the abstraction could name, so a function of two arguments
/// had nothing to be a function of, and an internal call could pass the
/// argument to exactly one parameter. An environment is the same idea with
/// the arity taken out of it.
pub type Env = std::collections::BTreeMap<String, IntervalSet>;

/// One binding, for a caller that has only one thing to say.
pub fn env_of(name: &str, set: &IntervalSet) -> Env {
    Env::from([(name.to_string(), set.clone())])
}

pub fn value_set(e: &Expr, env: &Env) -> Result<IntervalSet, String> {
    match e {
        Expr::Literal { text, .. } => {
            let v = crate::interval::parse_decimal(text)?;
            Ok(IntervalSet::point(v))
        }
        Expr::Ident { name, .. } => match env.get(name) {
            Some(set) => Ok(set.clone()),
            None => Err(format!("`{name}` is not a value the walk knows here")),
        },
        Expr::Call { name, args, .. } => match (name.as_str(), args.len()) {
            // A mask that covers everything the inner expression can produce
            // leaves it alone. Solidity's narrowing cleanups are this shape.
            ("and", 2) => {
                // Each side is evaluated **once**. An earlier version asked
                // whether each side was a single value and then evaluated the
                // chosen side again, so every nested `and` evaluated its
                // child twice and a chain of cleanups cost two to the depth.
                // The mask may be written rather than given, since solc emits
                // `not(31)` for a round-down, so a literal check alone is not
                // enough.
                let (l, r) = (mask_of(&args[0]), mask_of(&args[1]));
                if let (Some(a), Some(b)) = (l, r) {
                    return Ok(IntervalSet::point(a & b));
                }
                let single = |set: &IntervalSet| -> Option<U256> {
                    let (lo, hi) = set.bounds()?;
                    (lo == hi).then_some(lo)
                };
                let (set, mask) = match (l, r) {
                    (None, Some(m)) => (value_set(&args[0], env)?, m),
                    (Some(m), None) => (value_set(&args[1], env)?, m),
                    (None, None) => {
                        let a = value_set(&args[0], env)?;
                        let b = value_set(&args[1], env)?;
                        match (single(&a), single(&b)) {
                            (Some(x), Some(y)) => return Ok(IntervalSet::point(x & y)),
                            (_, Some(m)) => (a, m),
                            (Some(m), _) => (b, m),
                            (None, None) => {
                                return Err("`and` of two ranges is outside the P1a fragment".into())
                            }
                        }
                    }
                    (Some(_), Some(_)) => unreachable!("handled above"),
                };
                // A high-bit mask clears the low bits, which is a round down
                // to a power of two. `and(x, not(31))` is how solc rounds an
                // allocation size, and the result is exact: the operation is
                // monotone, so the ends map to the ends.
                if let Some(low) = clears_low_bits(mask) {
                    let (lo, hi) = set.bounds().unwrap_or((U256::ZERO, U256::ZERO));
                    let _ = low;
                    return Ok(IntervalSet::range(lo & mask, hi & mask));
                }
                let covered = full_mask_set(mask).ok_or_else(|| {
                    format!("the mask 0x{mask:x} is not a low-bit mask; P1a does not model it")
                })?;
                if set.subset_of(&covered) {
                    Ok(set)
                } else {
                    Err(format!(
                        "the mask 0x{mask:x} truncates values the argument can take, \\
                         so the stored value is not the argument"
                    ))
                }
            }
            // Arithmetic on what is known. Exact where it cannot wrap, and
            // refused where it can: a wrapped result is a different value and
            // approximating it would put a number in a region it is not in.
            // `not(k)` for a literal is a constant. solc writes the
            // round-down mask that way: `and(x, not(31))`.
            ("not", 1) => {
                let v = mask_of(&args[0]).ok_or_else(|| {
                    "`not` of a non-literal is outside the P1a fragment".to_string()
                })?;
                Ok(IntervalSet::point(!v))
            }
            ("add", 2) | ("sub", 2) | ("mul", 2) => {
                let (l, r) = (value_set(&args[0], env)?, value_set(&args[1], env)?);
                arith(name, &l, &r)
            }
            ("or", 2) if mask_of(&args[0]) == Some(U256::ZERO) => value_set(&args[1], env),
            ("or", 2) if mask_of(&args[1]) == Some(U256::ZERO) => value_set(&args[0], env),
            (other, _) => Err(format!(
                "`{other}` in a stored value is outside the P1a fragment"
            )),
        },
    }
}

fn mask_of(e: &Expr) -> Option<U256> {
    match e {
        Expr::Literal { text, .. } => crate::interval::parse_decimal(text).ok(),
        _ => None,
    }
}

/// `n` when `mask` is `!(2^n - 1)`: a mask that keeps the high bits and
/// clears the low ones, which is a round down to a multiple of `2^n`.
fn clears_low_bits(mask: U256) -> Option<u32> {
    let complement = !mask;
    // The complement must be `2^n - 1`, and not everything.
    if complement == U256::ZERO || mask == U256::ZERO {
        return None;
    }
    let n = complement.checked_add(U256::from(1u8))?;
    (n & complement == U256::ZERO).then(|| complement.bit_len() as u32)
}

/// `[0, mask]` when `mask` is `2^n - 1`, else `None`.
fn full_mask_set(mask: U256) -> Option<IntervalSet> {
    // 2^n - 1 is exactly the values whose successor is a power of two.
    let next = mask.checked_add(U256::from(1u8));
    match next {
        Some(n) if n.count_ones() == 1 => Some(IntervalSet::le(mask)),
        // mask == U256::MAX: adding one overflows, and it covers everything
        None => Some(IntervalSet::full()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mulu_yul::parse_object;

    fn e(text: &str) -> Expr {
        let src = format!("object \"T\" {{ code {{ let c := {text} }} }}");
        let p = parse_object(&src).unwrap();
        let mulu_yul::Stmt::Let { value: Some(v), .. } = &p.object.code.stmts[0] else {
            panic!()
        };
        v.clone()
    }
    fn u(n: u64) -> U256 {
        U256::from(n)
    }

    #[test]
    fn the_argument_passes_through_a_cleanup_for_its_own_type() {
        // `reading = x` with x : uint8 lowers to and(x, 0xff)
        let dom = crate::types::domain_of("uint8").unwrap();
        let got = value_set(&e("and(var_x, 0xff)"), &env_of("var_x", &dom)).unwrap();
        assert_eq!(got, dom);
        // a narrower region survives too
        let narrow = IntervalSet::le(u(100));
        assert_eq!(
            value_set(&e("and(var_x, 0xff)"), &env_of("var_x", &narrow)).unwrap(),
            narrow
        );
    }

    #[test]
    fn a_mask_that_really_truncates_is_refused() {
        // a uint256 argument through a uint8 mask is not the argument
        let full = IntervalSet::full();
        let err = value_set(&e("and(var_x, 0xff)"), &env_of("var_x", &full)).unwrap_err();
        assert!(err.contains("truncates"), "{err}");
    }

    #[test]
    fn literals_and_the_bare_argument() {
        let r = IntervalSet::range(u(10), u(20));
        assert_eq!(value_set(&e("var_x"), &env_of("var_x", &r)).unwrap(), r);
        assert_eq!(
            value_set(&e("0x2a"), &env_of("var_x", &r)).unwrap(),
            IntervalSet::point(u(42))
        );
        assert_eq!(
            value_set(&e("42"), &Env::new()).unwrap(),
            IntervalSet::point(u(42))
        );
        assert!(value_set(&e("other"), &env_of("var_x", &r)).is_err());
    }

    #[test]
    fn only_low_bit_masks_count_as_cleanups() {
        let dom = IntervalSet::le(u(255));
        // 0xf0 is not 2^n - 1
        let err = value_set(&e("and(var_x, 0xf0)"), &env_of("var_x", &dom)).unwrap_err();
        assert!(err.contains("not a low-bit mask"), "{err}");
        // the full word mask covers everything
        let full = "0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";
        assert_eq!(
            value_set(
                &e(&format!("and(var_x, {full})")),
                &env_of("var_x", &IntervalSet::full())
            )
            .unwrap(),
            IntervalSet::full()
        );
    }

    #[test]
    fn an_address_cleanup_is_the_identity_on_addresses() {
        let dom = crate::types::domain_of("address").unwrap();
        let mask = "0xffffffffffffffffffffffffffffffffffffffff";
        let got = value_set(&e(&format!("and(var_a, {mask})")), &env_of("var_a", &dom)).unwrap();
        assert_eq!(got, dom);
    }

    #[test]
    fn what_this_cannot_evaluate_is_refused_not_guessed() {
        // `add` and `mul` are evaluated now, exactly; see `arith_tests`.
        // These are still outside: a storage read needs the walk's current
        // region, and a shift is not modelled at all.
        let r = IntervalSet::le(u(100));
        for expr in [
            "sload(0)",
            "shr(1, var_x)",
            "div(var_x, 2)",
            "keccak256(var_x, 32)",
        ] {
            assert!(value_set(&e(expr), &env_of("var_x", &r)).is_err(), "{expr}");
        }
    }
}

/// The value of an expression that is already a literal, after folding.
/// `None` for anything that still depends on something.
pub fn constant(e: &Expr) -> Option<U256> {
    match e {
        Expr::Literal { text, .. } => crate::interval::parse_decimal(text).ok(),
        _ => None,
    }
}

/// `add`, `sub` and `mul` on interval sets, exact or refused.
///
/// Only the ends are needed: each is monotone in both operands over the range
/// where it does not wrap. Where it can wrap the answer is not an interval at
/// all, and returning one anyway would be inventing a value.
fn arith(op: &str, l: &IntervalSet, r: &IntervalSet) -> Result<IntervalSet, String> {
    let (Some((llo, lhi)), Some((rlo, rhi))) = (l.bounds(), r.bounds()) else {
        return Ok(IntervalSet::empty());
    };
    let wrap = || format!("`{op}` of {l} and {r} can wrap, so the result is not an interval");
    match op {
        "add" => {
            let hi = lhi.checked_add(rhi).ok_or_else(wrap)?;
            Ok(IntervalSet::range(llo + rlo, hi))
        }
        "sub" => {
            // Unsigned: every value of the left must be at or above every
            // value of the right, or the subtraction wraps.
            let lo = llo.checked_sub(rhi).ok_or_else(wrap)?;
            Ok(IntervalSet::range(lo, lhi - rlo))
        }
        "mul" => {
            let hi = lhi.checked_mul(rhi).ok_or_else(wrap)?;
            Ok(IntervalSet::range(llo * rlo, hi))
        }
        _ => Err(format!("`{op}` is outside the P1a fragment")),
    }
}

#[cfg(test)]
mod arith_tests {
    use super::*;
    use crate::interval::U256;

    fn u(n: u64) -> U256 {
        U256::from(n)
    }

    #[test]
    fn arithmetic_is_exact_where_it_cannot_wrap() {
        let a = IntervalSet::range(u(10), u(20));
        let b = IntervalSet::range(u(1), u(2));
        assert_eq!(
            arith("add", &a, &b).unwrap(),
            IntervalSet::range(u(11), u(22))
        );
        assert_eq!(
            arith("sub", &a, &b).unwrap(),
            IntervalSet::range(u(8), u(19))
        );
        assert_eq!(
            arith("mul", &a, &b).unwrap(),
            IntervalSet::range(u(10), u(40))
        );
    }

    #[test]
    fn arithmetic_that_can_wrap_is_refused_not_approximated() {
        // A wrapped result is a different value, and putting it in an
        // interval anyway would place a number in a region it is not in.
        let big = IntervalSet::range(
            crate::interval::max_u256() - u(1),
            crate::interval::max_u256(),
        );
        let one = IntervalSet::point(u(2));
        assert!(arith("add", &big, &one).is_err());
        assert!(arith("mul", &big, &one).is_err());
        // and unsigned subtraction wraps when the right can exceed the left
        assert!(arith("sub", &IntervalSet::point(u(1)), &IntervalSet::point(u(2))).is_err());
    }

    #[test]
    fn a_computed_local_becomes_known() {
        // `capped(x + 1)`: the modifier's parameter is the argument plus one.
        let e = |src: &str| {
            let s = format!("object \"T\" {{ code {{ let c := {src} }} }}");
            let p = mulu_yul::parse_object(&s).unwrap();
            let mulu_yul::Stmt::Let { value: Some(v), .. } = &p.object.code.stmts[0] else {
                panic!()
            };
            v.clone()
        };
        let env = env_of("x", &IntervalSet::range(u(0), u(99)));
        assert_eq!(
            value_set(&e("add(x, 1)"), &env).unwrap(),
            IntervalSet::range(u(1), u(100))
        );
    }
}

#[cfg(test)]
mod mask_tests {
    use super::*;
    use crate::interval::U256;

    fn u(n: u64) -> U256 {
        U256::from(n)
    }

    #[test]
    fn a_high_bit_mask_rounds_an_interval_down() {
        // `and(x, not(31))` is how solc rounds an allocation size down to a
        // multiple of 32. The operation is monotone, so the ends map to the
        // ends and the result is exact rather than approximated.
        assert_eq!(clears_low_bits(!u(31)), Some(5));
        assert_eq!(
            clears_low_bits(u(31)),
            None,
            "a low-bit mask is the other case"
        );
        assert_eq!(clears_low_bits(U256::ZERO), None);

        let env = env_of("x", &IntervalSet::range(u(33), u(70)));
        let e = |src: &str| {
            let s = format!("object \"T\" {{ code {{ let c := {src} }} }}");
            let p = mulu_yul::parse_object(&s).unwrap();
            let mulu_yul::Stmt::Let { value: Some(v), .. } = &p.object.code.stmts[0] else {
                panic!()
            };
            v.clone()
        };
        assert_eq!(
            value_set(&e("and(x, not(31))"), &env).unwrap(),
            IntervalSet::range(u(32), u(64))
        );
    }
}

#[cfg(test)]
mod nesting_tests {
    use super::*;
    use crate::interval::U256;

    #[test]
    fn a_chain_of_cleanups_costs_its_length_not_two_to_it() {
        // solc nests cleanups: `and(and(and(x, m), m), m)`. Asking whether
        // each side was a single value and then evaluating the chosen side
        // again meant every node evaluated its child twice, so a chain of
        // twenty took longer than anyone would wait. This is not a timing
        // test; it simply would not finish before the fix.
        let mut e = "var_x".to_string();
        for _ in 0..24 {
            e = format!("and({e}, 0xffff)");
        }
        let src = format!("object \"T\" {{ code {{ let c := {e} }} }}");
        let p = mulu_yul::parse_object(&src).unwrap();
        let mulu_yul::Stmt::Let { value: Some(v), .. } = &p.object.code.stmts[0] else {
            panic!()
        };
        let dom = IntervalSet::le(U256::from(0xffffu32));
        assert_eq!(value_set(v, &env_of("var_x", &dom)).unwrap(), dom);
    }
}
