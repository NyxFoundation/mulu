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
pub fn value_set(e: &Expr, var: Option<&str>, region: &IntervalSet) -> Result<IntervalSet, String> {
    match e {
        Expr::Literal { text, .. } => {
            let v = crate::interval::parse_decimal(text)?;
            Ok(IntervalSet::point(v))
        }
        Expr::Ident { name, .. } => {
            if Some(name.as_str()) == var {
                Ok(region.clone())
            } else {
                Err(format!("`{name}` is not the argument, so its value is unknown here"))
            }
        }
        Expr::Call { name, args, .. } => match (name.as_str(), args.len()) {
            // A mask that covers everything the inner expression can produce
            // leaves it alone. Solidity's narrowing cleanups are this shape.
            ("and", 2) => {
                let (inner, mask) = match (mask_of(&args[0]), mask_of(&args[1])) {
                    (None, Some(m)) => (&args[0], m),
                    (Some(m), None) => (&args[1], m),
                    (Some(a), Some(b)) => return Ok(IntervalSet::point(a & b)),
                    (None, None) => {
                        return Err("`and` of two non-literals is outside the P1a fragment".into())
                    }
                };
                let set = value_set(inner, var, region)?;
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
            ("or", 2) if mask_of(&args[0]) == Some(U256::ZERO) => value_set(&args[1], var, region),
            ("or", 2) if mask_of(&args[1]) == Some(U256::ZERO) => value_set(&args[0], var, region),
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
        let mulu_yul::Stmt::Let { value: Some(v), .. } = &p.object.code.stmts[0] else { panic!() };
        v.clone()
    }
    fn u(n: u64) -> U256 {
        U256::from(n)
    }

    #[test]
    fn the_argument_passes_through_a_cleanup_for_its_own_type() {
        // `reading = x` with x : uint8 lowers to and(x, 0xff)
        let dom = crate::types::domain_of("uint8").unwrap();
        let got = value_set(&e("and(var_x, 0xff)"), Some("var_x"), &dom).unwrap();
        assert_eq!(got, dom);
        // a narrower region survives too
        let narrow = IntervalSet::le(u(100));
        assert_eq!(value_set(&e("and(var_x, 0xff)"), Some("var_x"), &narrow).unwrap(), narrow);
    }

    #[test]
    fn a_mask_that_really_truncates_is_refused() {
        // a uint256 argument through a uint8 mask is not the argument
        let full = IntervalSet::full();
        let err = value_set(&e("and(var_x, 0xff)"), Some("var_x"), &full).unwrap_err();
        assert!(err.contains("truncates"), "{err}");
    }

    #[test]
    fn literals_and_the_bare_argument() {
        let r = IntervalSet::range(u(10), u(20));
        assert_eq!(value_set(&e("var_x"), Some("var_x"), &r).unwrap(), r);
        assert_eq!(value_set(&e("0x2a"), Some("var_x"), &r).unwrap(), IntervalSet::point(u(42)));
        assert_eq!(value_set(&e("42"), None, &r).unwrap(), IntervalSet::point(u(42)));
        assert!(value_set(&e("other"), Some("var_x"), &r).is_err());
    }

    #[test]
    fn only_low_bit_masks_count_as_cleanups() {
        let dom = IntervalSet::le(u(255));
        // 0xf0 is not 2^n - 1
        let err = value_set(&e("and(var_x, 0xf0)"), Some("var_x"), &dom).unwrap_err();
        assert!(err.contains("not a low-bit mask"), "{err}");
        // the full word mask covers everything
        let full = "0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";
        assert_eq!(
            value_set(&e(&format!("and(var_x, {full})")), Some("var_x"), &IntervalSet::full()).unwrap(),
            IntervalSet::full()
        );
    }

    #[test]
    fn an_address_cleanup_is_the_identity_on_addresses() {
        let dom = crate::types::domain_of("address").unwrap();
        let mask = "0xffffffffffffffffffffffffffffffffffffffff";
        let got = value_set(&e(&format!("and(var_a, {mask})")), Some("var_a"), &dom).unwrap();
        assert_eq!(got, dom);
    }

    #[test]
    fn arithmetic_is_not_evaluated_it_is_refused() {
        let r = IntervalSet::le(u(100));
        for expr in ["add(var_x, 1)", "mul(var_x, 2)", "sload(0)", "shr(1, var_x)"] {
            assert!(value_set(&e(expr), Some("var_x"), &r).is_err(), "{expr}");
        }
    }
}
