//! Value domains of Solidity types.
//!
//! docs/11 §5 fixes the environment as "type-correct calls to the listed ABI
//! entrypoints". Honouring that means the argument domain has to come from the
//! ABI type: a `uint8` argument ranges over 256 values, not 2^256. Modelling it
//! over the whole word contradicts the profile we claim to work under, and
//! invents regions no call can reach.
//!
//! The same applies to storage: a `uint8` slot cannot hold 300.
//!
//! A type whose values do not form an interval set is not refused: it is
//! given the whole word, which is sound and says less. Signed integers are
//! the exception. Reading a signed value as unsigned does not widen the
//! domain, it reorders it, and every comparison the model makes would be the
//! wrong one.

use crate::interval::{IntervalSet, U256};

/// The values a type can take, as an unsigned 256-bit word.
pub fn domain_of(ty: &str) -> Result<IntervalSet, String> {
    let t = ty.trim();
    if t == "bool" {
        // solc keeps a bool as 0 or 1 and cleans it on the way in.
        return Ok(IntervalSet::le(U256::from(1u8)));
    }
    if t == "address" || t == "address payable" {
        return Ok(unsigned_bits(160));
    }
    if let Some(rest) = t.strip_prefix("uint") {
        let bits: u32 = if rest.is_empty() { 256 } else { rest.parse().map_err(|_| bad(t))? };
        if bits == 0 || bits > 256 || bits % 8 != 0 {
            return Err(bad(t));
        }
        return Ok(unsigned_bits(bits));
    }
    if t.starts_with("int") {
        return Err(format!(
            "`{t}` is signed; P1a reasons over unsigned uint256 words only"
        ));
    }
    if t == "bytes32" {
        // A full word, left-aligned or not: every value is possible.
        return Ok(IntervalSet::full());
    }
    Err(bad(t))
}

/// The domain to reason over, and what was given up to get it.
///
/// A `bytes4` argument holds its value in the top four bytes, so its values
/// are the multiples of 2^224 below 2^256 — not an interval set. A `string`
/// argument reaches the body as a memory pointer. Neither is a domain this
/// can describe, and refusing them threw away the contract for a parameter
/// that often decides nothing. The whole word covers both, and a guard over
/// such an argument is then one the regions do not decide, which the walk
/// takes both ways.
pub fn domain_or_whole_word(ty: &str) -> Result<(IntervalSet, Option<String>), String> {
    match domain_of(ty) {
        Ok(d) => Ok((d, None)),
        Err(why) if why.contains("signed") => Err(why),
        Err(_) => Ok((
            IntervalSet::full(),
            Some(format!(
                "whole-word-argument: an argument of type `{ty}` ranges over the whole 256-bit \
                 word here, because its values are not an interval set. Wider than the type \
                 admits, so no guard is decided that the type alone would not decide"
            )),
        )),
    }
}

fn bad(t: &str) -> String {
    format!("`{t}` is outside the P1a type fragment (uint8..uint256, address, bool)")
}

/// `[0, 2^bits - 1]`.
fn unsigned_bits(bits: u32) -> IntervalSet {
    if bits >= 256 {
        return IntervalSet::full();
    }
    let max = (U256::from(1u8) << bits as usize) - U256::from(1u8);
    IntervalSet::le(max)
}

/// The type id solc uses in `storageLayout.types`, mapped to a Solidity type
/// name: `t_uint8` -> `uint8`.
pub fn label_of_type_id(type_id: &str, types: &serde_json::Value) -> Option<String> {
    types
        .get(type_id)
        .and_then(|t| t.get("label"))
        .and_then(|l| l.as_str())
        .map(|s| s.to_string())
        .or_else(|| type_id.strip_prefix("t_").map(|s| s.to_string()))
}

/// How many bytes a storage type occupies, from `storageLayout.types`.
pub fn bytes_of_type_id(type_id: &str, types: &serde_json::Value) -> Option<u64> {
    types
        .get(type_id)
        .and_then(|t| t.get("numberOfBytes"))
        .and_then(|n| n.as_str())
        .and_then(|s| s.parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(n: u64) -> U256 {
        U256::from(n)
    }

    #[test]
    fn integer_widths_give_the_right_number_of_values() {
        assert_eq!(domain_of("uint8").unwrap().count(), Some(256));
        assert_eq!(domain_of("uint16").unwrap().count(), Some(65536));
        assert_eq!(domain_of("uint8").unwrap(), IntervalSet::le(u(255)));
        assert!(domain_of("uint256").unwrap().is_full());
        assert!(domain_of("uint").unwrap().is_full());
        // 2^160 - 1 is the largest address
        let addr = domain_of("address").unwrap();
        assert!(addr.contains(U256::from_str_radix("ffffffffffffffffffffffffffffffffffffffff", 16).unwrap()));
        assert!(!addr.contains(U256::from_str_radix("10000000000000000000000000000000000000000", 16).unwrap()));
        assert_eq!(domain_of("address payable").unwrap(), addr);
    }

    #[test]
    fn a_bool_holds_two_values() {
        let b = domain_of("bool").unwrap();
        assert_eq!(b.count(), Some(2));
        assert!(b.contains(u(0)) && b.contains(u(1)) && !b.contains(u(2)));
    }

    #[test]
    fn what_p1a_cannot_model_is_refused() {
        for t in ["int256", "int8", "bytes", "string", "uint7", "uint0", "uint512", "mapping", "MyStruct"] {
            assert!(domain_of(t).is_err(), "{t} must be refused an exact domain");
        }
        // and the message says why, rather than being generic
        assert!(domain_of("int256").unwrap_err().contains("signed"));
        // a full word is a full word, whichever end the value sits at
        assert!(domain_of("bytes32").unwrap().is_full());
    }

    /// Everything but a signed integer has *some* domain to reason over.
    /// Refusing the contract for a `string` parameter that decides nothing
    /// was the wrong trade; a domain that says less is the right one.
    #[test]
    fn a_type_without_an_interval_domain_gets_the_whole_word_and_says_so() {
        for t in ["bytes", "string", "bytes4", "MyStruct", "uint256[]"] {
            let (d, note) = domain_or_whole_word(t).unwrap();
            assert!(d.is_full(), "{t}");
            assert!(note.unwrap().starts_with("whole-word-argument:"), "{t}");
        }
        // exact where it can be, and silent about it
        let (d, note) = domain_or_whole_word("uint8").unwrap();
        assert_eq!(d.count(), Some(256));
        assert!(note.is_none());
        // a signed integer is still refused: the whole word is not a wider
        // reading of it, it is a different one
        assert!(domain_or_whole_word("int256").is_err());
    }

    #[test]
    fn a_narrow_domain_removes_regions_a_wide_one_would_invent() {
        // a guard `x <= 100` over a uint8 argument leaves two regions, not three
        let dom = domain_of("uint8").unwrap();
        let guard = IntervalSet::le(u(100));
        let inside = dom.intersect(&guard);
        let outside = dom.difference(&guard);
        assert_eq!(inside.count(), Some(101));
        assert_eq!(outside, IntervalSet::range(u(101), u(255)));
        // over uint256 the same split leaves a tail no uint8 call can reach
        let wide = IntervalSet::full().difference(&guard);
        assert!(wide.contains(u(1000)));
        assert!(!outside.contains(u(1000)));
    }

    #[test]
    fn storage_type_table_is_read_not_guessed() {
        let types = serde_json::json!({
            "t_uint8": {"label": "uint8", "numberOfBytes": "1", "encoding": "inplace"},
            "t_address": {"label": "address", "numberOfBytes": "20", "encoding": "inplace"}
        });
        assert_eq!(label_of_type_id("t_uint8", &types).as_deref(), Some("uint8"));
        assert_eq!(bytes_of_type_id("t_address", &types), Some(20));
        // an id the table does not mention falls back to the id's own shape
        assert_eq!(label_of_type_id("t_uint256", &types).as_deref(), Some("uint256"));
        assert_eq!(bytes_of_type_id("t_uint256", &types), None);
    }
}
