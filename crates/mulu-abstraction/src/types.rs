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
//! Types P1a does not model are refused. Signed integers are refused because
//! the interval arithmetic here is unsigned; `bytesN` because its value sits
//! left-aligned in the word, which is a different encoding than the numeric
//! types share.

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
    if t.starts_with("bytes") || t == "string" {
        return Err(format!(
            "`{t}` is not a numeric word; P1a models uint, address and bool"
        ));
    }
    Err(bad(t))
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
        for t in ["int256", "int8", "bytes32", "bytes", "string", "uint7", "uint0", "uint512", "mapping", "MyStruct"] {
            assert!(domain_of(t).is_err(), "{t} must be refused");
        }
        // and the message says why, rather than being generic
        assert!(domain_of("int256").unwrap_err().contains("signed"));
        assert!(domain_of("bytes32").unwrap_err().contains("not a numeric word"));
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
