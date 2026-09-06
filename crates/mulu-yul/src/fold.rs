//! Constant folding over the pure fragment of the EVM Yul dialect.
//!
//! solc writes whole-slot stores as a mask-and-merge sequence, so a helper
//! that is the identity on its argument only looks like one after
//! `and(v, not(0xff..ff))` and friends are evaluated. Every rule here is an
//! exact identity of 256-bit word semantics; operations whose semantics this
//! does not model (signed shifts and divisions, `exp`, `byte`, `signextend`)
//! are left untouched rather than approximated.

use crate::ast::Expr;

pub type U256 = ruint::aliases::U256;

fn parse(text: &str) -> Option<U256> {
    let t = text.trim();
    if let Some(h) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        U256::from_str_radix(h, 16).ok()
    } else {
        U256::from_str_radix(t, 10).ok()
    }
}

fn literal(e: &Expr) -> Option<U256> {
    match e {
        Expr::Literal { text, .. } => parse(text),
        _ => None,
    }
}

fn lit(v: U256, src: Option<crate::lex::SrcSpan>) -> Expr {
    Expr::Literal { text: format!("0x{v:x}"), src }
}

fn bool_lit(b: bool, src: Option<crate::lex::SrcSpan>) -> Expr {
    lit(if b { U256::from(1u8) } else { U256::ZERO }, src)
}

/// One pass of folding, bottom up.
pub fn fold(e: &Expr) -> Expr {
    let Expr::Call { name, args, src } = e else { return e.clone() };
    let args: Vec<Expr> = args.iter().map(fold).collect();
    let src = *src;
    let a = args.first().and_then(literal);
    let b = args.get(1).and_then(literal);

    // Both operands literal: evaluate exactly.
    if let (Some(x), Some(y)) = (a, b) {
        let v = match name.as_str() {
            "add" => Some(x.wrapping_add(y)),
            "sub" => Some(x.wrapping_sub(y)),
            "mul" => Some(x.wrapping_mul(y)),
            // EVM: division and remainder by zero yield zero.
            "div" => Some(if y.is_zero() { U256::ZERO } else { x / y }),
            "mod" => Some(if y.is_zero() { U256::ZERO } else { x % y }),
            "and" => Some(x & y),
            "or" => Some(x | y),
            "xor" => Some(x ^ y),
            // EVM: shl/shr take the shift amount first; >= 256 gives zero.
            "shl" => Some(if x >= U256::from(256u16) { U256::ZERO } else { y << x.to::<usize>() }),
            "shr" => Some(if x >= U256::from(256u16) { U256::ZERO } else { y >> x.to::<usize>() }),
            _ => None,
        };
        if let Some(v) = v {
            return lit(v, src);
        }
        let p = match name.as_str() {
            "lt" => Some(x < y),
            "gt" => Some(x > y),
            "eq" => Some(x == y),
            _ => None,
        };
        if let Some(p) = p {
            return bool_lit(p, src);
        }
    }

    // One operand literal, or none: the exact algebraic identities.
    let max = U256::MAX;
    let zero = U256::ZERO;
    match (name.as_str(), args.len()) {
        ("not", 1) => {
            if let Some(x) = a {
                return lit(!x, src);
            }
        }
        ("iszero", 1) => {
            if let Some(x) = a {
                return bool_lit(x.is_zero(), src);
            }
        }
        ("and", 2) => {
            if a == Some(max) {
                return args[1].clone();
            }
            if b == Some(max) {
                return args[0].clone();
            }
            if a == Some(zero) || b == Some(zero) {
                return lit(zero, src);
            }
        }
        ("or", 2) => {
            if a == Some(zero) {
                return args[1].clone();
            }
            if b == Some(zero) {
                return args[0].clone();
            }
            if a == Some(max) || b == Some(max) {
                return lit(max, src);
            }
        }
        ("xor", 2) => {
            if a == Some(zero) {
                return args[1].clone();
            }
            if b == Some(zero) {
                return args[0].clone();
            }
        }
        ("add", 2) | ("sub", 2) => {
            if b == Some(zero) {
                return args[0].clone();
            }
            if name == "add" && a == Some(zero) {
                return args[1].clone();
            }
        }
        ("mul", 2) => {
            if a == Some(U256::from(1u8)) {
                return args[1].clone();
            }
            if b == Some(U256::from(1u8)) {
                return args[0].clone();
            }
            if a == Some(zero) || b == Some(zero) {
                return lit(zero, src);
            }
        }
        // shl/shr take the shift amount first: a shift of zero is the identity.
        ("shl", 2) | ("shr", 2) => {
            if a == Some(zero) {
                return args[1].clone();
            }
        }
        _ => {}
    }
    Expr::Call { name: name.clone(), args, src }
}

/// Fold repeatedly until it stops changing, with a bound.
pub fn fold_fixpoint(e: &Expr) -> Expr {
    let mut cur = e.clone();
    for _ in 0..32 {
        let next = fold(&cur);
        if next.render() == cur.render() {
            return next;
        }
        cur = next;
    }
    cur
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_object;

    fn e(text: &str) -> Expr {
        let src = format!("object \"T\" {{ code {{ let c := {text} }} }}");
        let p = parse_object(&src).unwrap();
        let crate::Stmt::Let { value: Some(v), .. } = &p.object.code.stmts[0] else { panic!() };
        v.clone()
    }
    fn f(text: &str) -> String {
        fold_fixpoint(&e(text)).render()
    }

    #[test]
    fn the_whole_slot_store_pattern_collapses_to_the_value() {
        // exactly the body of solc's update_byte_slice_32_shift_0
        let mask = "0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";
        assert_eq!(f(&format!("or(and(value, not({mask})), and(toInsert, {mask}))")), "toInsert");
        assert_eq!(f("shl(0, toInsert)"), "toInsert");
    }

    #[test]
    fn literal_arithmetic_uses_evm_semantics() {
        assert_eq!(f("add(2, 3)"), "0x5");
        assert_eq!(f("sub(0, 1)"), "0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff");
        assert_eq!(f("div(7, 0)"), "0x0", "EVM division by zero is zero");
        assert_eq!(f("mod(7, 0)"), "0x0");
        assert_eq!(f("shl(256, 1)"), "0x0", "a shift of 256 or more clears the word");
        assert_eq!(f("shr(1, 8)"), "0x4");
        assert_eq!(f("lt(1, 2)"), "0x1");
        assert_eq!(f("gt(1, 2)"), "0x0");
        assert_eq!(f("iszero(0)"), "0x1");
        assert_eq!(f("iszero(5)"), "0x0");
    }

    #[test]
    fn identities_hold_without_knowing_the_variable() {
        assert_eq!(f("and(x, 0)"), "0x0");
        assert_eq!(f("or(x, 0)"), "x");
        assert_eq!(f("xor(x, 0)"), "x");
        assert_eq!(f("add(x, 0)"), "x");
        assert_eq!(f("sub(x, 0)"), "x");
        assert_eq!(f("mul(x, 1)"), "x");
        assert_eq!(f("mul(x, 0)"), "0x0");
    }

    #[test]
    fn operations_that_are_not_modelled_are_left_alone() {
        for expr in ["sdiv(a, b)", "smod(a, b)", "sar(1, x)", "exp(2, x)", "byte(0, x)", "signextend(0, x)"] {
            assert_eq!(f(expr), e(expr).render(), "{expr} must not be rewritten");
        }
        // a guard condition has nothing to fold and must survive untouched
        assert_eq!(f("iszero(gt(var_x_5, 0x64))"), "iszero(gt(var_x_5, 0x64))");
    }

    #[test]
    fn folding_agrees_with_direct_evaluation_on_small_values() {
        for x in [0u64, 1, 2, 7, 255, 256] {
            for y in [0u64, 1, 3, 8, 255] {
                let (a, b) = (U256::from(x), U256::from(y));
                assert_eq!(f(&format!("add({x}, {y})")), format!("0x{:x}", a.wrapping_add(b)));
                assert_eq!(f(&format!("and({x}, {y})")), format!("0x{:x}", a & b));
                assert_eq!(f(&format!("or({x}, {y})")), format!("0x{:x}", a | b));
                assert_eq!(f(&format!("eq({x}, {y})")), if x == y { "0x1" } else { "0x0" });
            }
        }
    }
}
