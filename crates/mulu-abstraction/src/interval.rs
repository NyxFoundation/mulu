//! Sets of `uint256` values as unions of closed intervals.
//!
//! docs/11 §7 works the Limits example by interval arithmetic over uint256,
//! and that is what this is. For the P1a fragment — comparisons of one
//! variable against literals, combined with and/or/not — interval sets are an
//! *exact* decision procedure: no solver, so no `trusted-solver` assumption
//! (docs/09 §5) and no incomplete answers.
//!
//! Representation is canonical: ranges are sorted, disjoint and never
//! adjacent, so two sets are equal exactly when they hold the same values.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

pub type U256 = ruint::aliases::U256;

pub const ZERO: U256 = U256::ZERO;

pub fn max_u256() -> U256 {
    U256::MAX
}

/// The smallest word whose two's-complement reading is negative, `2^255`.
pub fn sign_bit() -> U256 {
    U256::from(1u8) << 255
}

/// The words whose two's-complement reading is below that of `k`.
///
/// A signed comparison is a statement about the same 256-bit words, read
/// differently, so it still names a set of words. What it is not is an
/// interval: every negative value sits above every positive one as a word,
/// so `x < 0` is the single range `[2^255, 2^256-1]` and `x < 1` is two.
pub fn signed_lt(k: U256) -> IntervalSet {
    let half = sign_bit();
    if k >= half {
        // `k` is negative: below it are the negatives below it as words.
        IntervalSet::range(half, k).difference(&IntervalSet::point(k))
    } else {
        // `k` is at or above zero: every negative, and the smaller words.
        IntervalSet::lt(k).union(&IntervalSet::ge(half))
    }
}

/// The words whose two's-complement reading is above that of `k`.
pub fn signed_gt(k: U256) -> IntervalSet {
    signed_lt(k).union(&IntervalSet::point(k)).complement()
}

/// The words an `intN` admits: `[0, 2^(N-1)-1]` and `[2^256-2^(N-1), 2^256-1]`.
pub fn signed_bits(bits: u32) -> IntervalSet {
    if bits >= 256 {
        return IntervalSet::full();
    }
    let top = (U256::from(1u8) << (bits as usize - 1)) - U256::from(1u8);
    let low = max_u256() - top;
    IntervalSet::le(top).union(&IntervalSet::ge(low))
}

/// A canonical union of closed intervals `[lo, hi]` over `uint256`.
///
/// Serialised as pairs of **decimal strings**, matching the convention of
/// docs/09 §3: a uint256 never goes through a JSON number, which cannot hold
/// it exactly.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct IntervalSet {
    /// Sorted, disjoint, non-adjacent, each `lo <= hi`.
    ranges: Vec<(U256, U256)>,
}

impl IntervalSet {
    pub fn empty() -> Self {
        Self { ranges: vec![] }
    }

    pub fn full() -> Self {
        Self {
            ranges: vec![(ZERO, U256::MAX)],
        }
    }

    /// `[lo, hi]`, empty when `lo > hi`.
    pub fn range(lo: U256, hi: U256) -> Self {
        if lo > hi {
            Self::empty()
        } else {
            Self {
                ranges: vec![(lo, hi)],
            }
        }
    }

    pub fn point(v: U256) -> Self {
        Self::range(v, v)
    }

    /// `x <= v`
    pub fn le(v: U256) -> Self {
        Self::range(ZERO, v)
    }

    /// `x < v`
    pub fn lt(v: U256) -> Self {
        match v.checked_sub(U256::from(1u8)) {
            Some(u) => Self::range(ZERO, u),
            None => Self::empty(), // x < 0 is unsatisfiable
        }
    }

    /// `x >= v`
    pub fn ge(v: U256) -> Self {
        Self::range(v, U256::MAX)
    }

    /// `x > v`
    pub fn gt(v: U256) -> Self {
        match v.checked_add(U256::from(1u8)) {
            Some(u) => Self::range(u, U256::MAX),
            None => Self::empty(), // x > MAX is unsatisfiable
        }
    }

    pub fn eq_to(v: U256) -> Self {
        Self::point(v)
    }

    pub fn ne_to(v: U256) -> Self {
        Self::point(v).complement()
    }

    /// The smallest and largest value in the set, or `None` when it is empty.
    /// A comparison of two sets needs only these: everything between is
    /// covered by the order.
    pub fn bounds(&self) -> Option<(U256, U256)> {
        Some((self.ranges.first()?.0, self.ranges.last()?.1))
    }

    pub fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }

    pub fn is_full(&self) -> bool {
        self.ranges.len() == 1 && self.ranges[0] == (ZERO, U256::MAX)
    }

    pub fn contains(&self, v: U256) -> bool {
        self.ranges.iter().any(|(lo, hi)| *lo <= v && v <= *hi)
    }

    pub fn ranges(&self) -> &[(U256, U256)] {
        &self.ranges
    }

    /// Build from arbitrary ranges: sort, drop empties, merge overlapping and
    /// adjacent ones. The only constructor that establishes the invariant.
    fn normalize(mut raw: Vec<(U256, U256)>) -> Self {
        raw.retain(|(lo, hi)| lo <= hi);
        raw.sort();
        let mut out: Vec<(U256, U256)> = Vec::with_capacity(raw.len());
        for (lo, hi) in raw {
            match out.last_mut() {
                // overlapping, or adjacent with no gap: extend
                Some((_, prev_hi))
                    if lo <= *prev_hi || prev_hi.checked_add(U256::from(1u8)) == Some(lo) =>
                {
                    if hi > *prev_hi {
                        *prev_hi = hi;
                    }
                }
                _ => out.push((lo, hi)),
            }
        }
        Self { ranges: out }
    }

    pub fn union(&self, other: &Self) -> Self {
        let mut raw = self.ranges.clone();
        raw.extend_from_slice(&other.ranges);
        Self::normalize(raw)
    }

    pub fn intersect(&self, other: &Self) -> Self {
        let mut out = Vec::new();
        let (mut i, mut j) = (0usize, 0usize);
        while i < self.ranges.len() && j < other.ranges.len() {
            let (alo, ahi) = self.ranges[i];
            let (blo, bhi) = other.ranges[j];
            let lo = alo.max(blo);
            let hi = ahi.min(bhi);
            if lo <= hi {
                out.push((lo, hi));
            }
            if ahi < bhi {
                i += 1;
            } else {
                j += 1;
            }
        }
        Self { ranges: out }
    }

    pub fn complement(&self) -> Self {
        let mut out = Vec::new();
        let mut cursor = Some(ZERO);
        for (lo, hi) in &self.ranges {
            if let Some(c) = cursor {
                if c < *lo {
                    out.push((c, *lo - U256::from(1u8)));
                }
            }
            cursor = hi.checked_add(U256::from(1u8));
            if cursor.is_none() {
                break; // the range reached MAX; nothing above it
            }
        }
        if let Some(c) = cursor {
            out.push((c, U256::MAX));
        }
        Self { ranges: out }
    }

    pub fn difference(&self, other: &Self) -> Self {
        self.intersect(&other.complement())
    }

    /// Every value of `self` is a value of `other`.
    pub fn subset_of(&self, other: &Self) -> bool {
        self.difference(other).is_empty()
    }

    /// No value belongs to both.
    pub fn disjoint_from(&self, other: &Self) -> bool {
        self.intersect(other).is_empty()
    }

    /// A representative value, for building concrete counterexamples.
    pub fn witness(&self) -> Option<U256> {
        self.ranges.first().map(|(lo, _)| *lo)
    }

    /// How many values the set holds, when that fits in a u128.
    pub fn count(&self) -> Option<u128> {
        let mut total = 0u128;
        for (lo, hi) in &self.ranges {
            let span = hi.checked_sub(*lo)?.checked_add(U256::from(1u8))?;
            let span: u128 = span.try_into().ok()?;
            total = total.checked_add(span)?;
        }
        Some(total)
    }
}

impl Serialize for IntervalSet {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let as_text: Vec<[String; 2]> = self
            .ranges
            .iter()
            .map(|(lo, hi)| [lo.to_string(), hi.to_string()])
            .collect();
        as_text.serialize(s)
    }
}

impl<'de> Deserialize<'de> for IntervalSet {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let raw = Vec::<[String; 2]>::deserialize(d)?;
        let mut ranges = Vec::with_capacity(raw.len());
        for [lo, hi] in raw {
            let lo = parse_decimal(&lo).map_err(D::Error::custom)?;
            let hi = parse_decimal(&hi).map_err(D::Error::custom)?;
            ranges.push((lo, hi));
        }
        // Rebuild through the normaliser: a hand-edited artifact must not be
        // able to smuggle in a non-canonical or overlapping representation.
        Ok(IntervalSet::normalize(ranges))
    }
}

/// Parse a uint256 written in decimal, or in hex with an `0x` prefix.
pub fn parse_decimal(text: &str) -> Result<U256, String> {
    let t = text.trim();
    let parsed = if let Some(hex) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        U256::from_str_radix(hex, 16)
    } else {
        U256::from_str_radix(t, 10)
    };
    parsed.map_err(|e| format!("{text:?} is not a uint256: {e}"))
}

impl fmt::Debug for IntervalSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_empty() {
            return write!(f, "{{}}");
        }
        if self.is_full() {
            return write!(f, "uint256");
        }
        let parts: Vec<String> = self
            .ranges
            .iter()
            .map(|(lo, hi)| {
                if lo == hi {
                    format!("{lo}")
                } else {
                    format!("[{lo}, {hi}]")
                }
            })
            .collect();
        write!(f, "{{{}}}", parts.join(" ∪ "))
    }
}

impl fmt::Display for IntervalSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(n: u64) -> U256 {
        U256::from(n)
    }

    /// Every set built by the tests, for the algebraic laws below.
    fn samples() -> Vec<IntervalSet> {
        vec![
            IntervalSet::empty(),
            IntervalSet::full(),
            IntervalSet::le(u(100)),
            IntervalSet::gt(u(100)),
            IntervalSet::le(u(1000)),
            IntervalSet::point(ZERO),
            IntervalSet::point(U256::MAX),
            IntervalSet::lt(ZERO),
            IntervalSet::gt(U256::MAX),
            IntervalSet::range(u(10), u(20)).union(&IntervalSet::range(u(30), u(40))),
            IntervalSet::ne_to(u(7)),
        ]
    }

    #[test]
    fn boundaries_do_not_wrap() {
        assert!(IntervalSet::lt(ZERO).is_empty(), "x < 0 has no solution");
        assert!(
            IntervalSet::gt(U256::MAX).is_empty(),
            "x > MAX has no solution"
        );
        assert!(IntervalSet::ge(ZERO).is_full());
        assert!(IntervalSet::le(U256::MAX).is_full());
        assert_eq!(
            IntervalSet::point(U256::MAX).complement(),
            IntervalSet::le(U256::MAX - u(1))
        );
        assert_eq!(IntervalSet::point(ZERO).complement(), IntervalSet::ge(u(1)));
    }

    #[test]
    fn complement_is_an_involution_and_partitions() {
        for s in samples() {
            assert_eq!(s.complement().complement(), s, "double complement of {s:?}");
            assert!(
                s.intersect(&s.complement()).is_empty(),
                "{s:?} meets its complement"
            );
            assert!(
                s.union(&s.complement()).is_full(),
                "{s:?} plus complement is not everything"
            );
        }
    }

    #[test]
    fn de_morgan_and_absorption() {
        for a in samples() {
            for b in samples() {
                assert_eq!(
                    a.intersect(&b).complement(),
                    a.complement().union(&b.complement()),
                    "de Morgan on {a:?} and {b:?}"
                );
                assert_eq!(a.union(&b), b.union(&a));
                assert_eq!(a.intersect(&b), b.intersect(&a));
                assert!(a.intersect(&b).subset_of(&a));
                assert!(a.subset_of(&a.union(&b)));
            }
        }
    }

    #[test]
    fn representation_is_canonical() {
        // adjacent ranges merge, so equal sets are equal values
        let a = IntervalSet::range(u(0), u(9)).union(&IntervalSet::range(u(10), u(20)));
        assert_eq!(a, IntervalSet::range(u(0), u(20)));
        assert_eq!(a.ranges().len(), 1);
        // overlapping too
        let b = IntervalSet::range(u(0), u(15)).union(&IntervalSet::range(u(10), u(20)));
        assert_eq!(b, a);
    }

    #[test]
    fn agrees_with_brute_force_on_a_small_domain() {
        // Compare against explicit membership for every value in 0..64.
        let build = |i: usize| -> IntervalSet {
            match i % 6 {
                0 => IntervalSet::le(u((i as u64 * 7) % 64)),
                1 => IntervalSet::gt(u((i as u64 * 5) % 64)),
                2 => IntervalSet::point(u((i as u64 * 3) % 64)),
                3 => IntervalSet::ne_to(u((i as u64 * 11) % 64)),
                4 => IntervalSet::range(u((i as u64) % 64), u((i as u64 + 13) % 64)),
                _ => IntervalSet::lt(u((i as u64 * 2) % 64)),
            }
        };
        for i in 0..24 {
            for j in 0..24 {
                let (a, b) = (build(i), build(j));
                for v in 0..64u64 {
                    let x = u(v);
                    assert_eq!(a.union(&b).contains(x), a.contains(x) || b.contains(x));
                    assert_eq!(a.intersect(&b).contains(x), a.contains(x) && b.contains(x));
                    assert_eq!(a.complement().contains(x), !a.contains(x));
                    assert_eq!(
                        a.difference(&b).contains(x),
                        a.contains(x) && !b.contains(x)
                    );
                }
                assert_eq!(
                    a.subset_of(&b),
                    (0..64u64).all(|v| !a.contains(u(v)) || b.contains(u(v)))
                );
                assert_eq!(
                    a.disjoint_from(&b),
                    (0..64u64).all(|v| !(a.contains(u(v)) && b.contains(u(v))))
                );
            }
        }
    }

    #[test]
    fn the_limits_partition_of_docs_11() {
        // p1: x <= 100, p2: x <= 1000. The three regions of docs/11 §7.
        let p1 = IntervalSet::le(u(100));
        let p2 = IntervalSet::le(u(1000));
        let x0 = p1.intersect(&p2);
        let x1 = p1.complement().intersect(&p2);
        let x2 = p1.complement().intersect(&p2.complement());

        // p1 true and p2 false is infeasible: x <= 100 implies x <= 1000
        assert!(
            p1.intersect(&p2.complement()).is_empty(),
            "p1 and not p2 must be unsatisfiable"
        );
        assert!(p1.subset_of(&p2), "A implies B, which is why B never fails");

        // the three regions cover uint256 and are pairwise disjoint
        assert!(x0.union(&x1).union(&x2).is_full());
        for (a, b) in [(&x0, &x1), (&x0, &x2), (&x1, &x2)] {
            assert!(a.disjoint_from(b));
        }
        assert_eq!(x0.count(), Some(101));
        assert_eq!(x1.count(), Some(900));
        assert!(x2.count().is_none(), "the tail is larger than u128");
        assert_eq!(x2.witness(), Some(u(1001)));
    }
}

#[cfg(test)]
mod signed_tests {
    use super::*;

    fn u(v: u64) -> U256 {
        U256::from(v)
    }

    /// A signed comparison is about the same words, read differently. Zero is
    /// the seam: everything at or above `2^255` reads as negative.
    #[test]
    fn the_negatives_sit_at_the_top_of_the_word() {
        let half = sign_bit();
        assert_eq!(signed_lt(U256::ZERO), IntervalSet::ge(half));
        assert_eq!(signed_gt(U256::ZERO), IntervalSet::range(u(1), half - u(1)));
        // `-1` is the all-ones word, and nothing but the other negatives is
        // below it
        assert_eq!(
            signed_lt(max_u256()),
            IntervalSet::range(half, max_u256() - u(1))
        );
        assert!(signed_gt(max_u256()).contains(U256::ZERO));
        assert!(!signed_gt(max_u256()).contains(half));
        // the two halves and the point itself partition the word
        for k in [U256::ZERO, u(1), half, max_u256()] {
            let all = signed_lt(k)
                .union(&signed_gt(k))
                .union(&IntervalSet::point(k));
            assert!(all.is_full(), "{k} does not partition");
            assert!(signed_lt(k).disjoint_from(&signed_gt(k)));
        }
    }

    /// An `intN` admits `2^N` words, in two blocks.
    #[test]
    fn a_signed_width_admits_both_ends_of_the_word() {
        assert_eq!(signed_bits(8).count(), Some(256));
        assert!(signed_bits(8).contains(u(127)));
        assert!(!signed_bits(8).contains(u(128)));
        assert!(signed_bits(8).contains(max_u256()));
        assert!(signed_bits(256).is_full());
    }
}
