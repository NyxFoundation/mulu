//! Order facts, and what follows from them together.
//!
//! A walk collects facts of the shape `a <= b`, `a < b` and `a == b` between
//! terms, and the regions add bounds of the shape `term <= 100`. On their own
//! neither settles much. Closed under transitivity, and with the little
//! arithmetic solc's own checks put in reach, they settle a good deal: that a
//! path cannot be taken, or that a value written to a slot lands in one region
//! and not another.
//!
//! Everything here is over rendered terms, which is what makes it usable from
//! both sides: the walk keys its facts this way, and a specification's line
//! renders to the same string when it is the same question.

use crate::interval::{max_u256, parse_decimal, IntervalSet, U256};

/// `a, b` of a two-argument call's rendered arguments, at the top level.
pub fn split_top(inner: &str) -> Option<(String, String)> {
    let mut depth = 0i32;
    for (i, c) in inner.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                return Some((
                    inner[..i].trim().to_string(),
                    inner[i + 1..].trim().to_string(),
                ))
            }
            _ => {}
        }
    }
    None
}

/// Is this set of order facts contradictory?
///
/// `amount <= balances[caller]`, `balances[caller] <= contract_balance` and
/// `contract_balance < amount` cannot all hold. Deriving that is the whole of
/// what a supplied invariant does for a path: the path is one the walk
/// produced and the contract cannot be on.
pub fn contradictory(edges: &[(String, String, bool)]) -> bool {
    // `t + k` for a positive literal `k` is strictly above `t`, as long as
    // the addition cannot wrap. An upper bound on `t` that leaves room for
    // `k` is what says it cannot: solc's own `push` guard, and the slot's
    // own type, both bound an array's length below 2^64. Without this the
    // length after a push was unrelated to the length before it.
    //
    // This runs here rather than where the edges are gathered, because the
    // bound may come from the regions and the addition from the facts.
    let edges = {
        let mut out = edges.to_vec();
        let mut upper: std::collections::BTreeMap<String, crate::interval::U256> =
            std::collections::BTreeMap::new();
        for (a, b, _) in &out {
            if let Ok(v) = parse_decimal(b) {
                let e = upper.entry(a.clone()).or_insert(v);
                if v < *e {
                    *e = v;
                }
            }
        }
        let terms: std::collections::BTreeSet<String> = out
            .iter()
            .flat_map(|(a, b, _)| [a.clone(), b.clone()])
            .collect();
        let mut strict = vec![];
        for term in &terms {
            let Some(inner) = term.strip_prefix("add(").and_then(|r| r.strip_suffix(")")) else {
                continue;
            };
            let Some((x, k)) = split_top(inner) else {
                continue;
            };
            // Either side may be the literal.
            for (t, lit) in [(&x, &k), (&k, &x)] {
                let Ok(k) = parse_decimal(lit) else { continue };
                if k == crate::interval::U256::ZERO {
                    continue;
                }
                let Some(b) = upper.get(t) else { continue };
                if crate::interval::max_u256() - *b < k {
                    continue;
                }
                strict.push((t.clone(), term.clone(), true));
            }
        }

        out.extend(strict);
        out
    };
    let edges = &edges[..];

    // `a - b` is at most `a` when `b <= a`, which is when it does not
    // underflow. The walk knows `b <= a` from the check solc puts there, and
    // without this the difference is an unrelated term: `balances -= amount
    // - 1` produced a path where `amount <= balances` and `amount - 1 >
    // balances` both held.
    let mut edges = edges.to_vec();
    for _ in 0..2 {
        let known: std::collections::BTreeSet<(String, String)> = edges
            .iter()
            .map(|(a, b, _)| (a.clone(), b.clone()))
            .collect();
        let mut extra = vec![];
        for term in edges
            .iter()
            .flat_map(|(a, b, _)| [a.clone(), b.clone()])
            .collect::<std::collections::BTreeSet<_>>()
        {
            let Some(inner) = term.strip_prefix("sub(").and_then(|r| r.strip_suffix(")")) else {
                continue;
            };
            let Some((a, b)) = split_top(inner) else {
                continue;
            };
            if known.contains(&(b.clone(), a.clone()))
                && !known.contains(&(term.clone(), a.clone()))
            {
                extra.push((term.clone(), a, false));
            }
        }
        if extra.is_empty() {
            break;
        }
        edges.extend(extra);
    }

    let mut extra_bitwise = vec![];
    // Bitwise `or` only ever sets bits and `and` only ever clears them, so
    // `or(a, b)` is at or above both its arguments and `and(a, b)` at or
    // below both. That is the whole of what says a packed word is not zero:
    // `_initializing` sits in the ninth byte of the same word as
    // `_initialized`, the outer modifier writes `or(and(word, mask), 1)`, and
    // the inner one's guard reads that word as zero. Nothing about the mask
    // is needed to see the two cannot both hold.
    for term in edges
        .iter()
        .flat_map(|(a, b, _)| [a.clone(), b.clone()])
        .collect::<std::collections::BTreeSet<_>>()
    {
        // Only over words. A variable that shares its slot is named for the
        // variable and bounded by its type, while the read-modify-write
        // around it is over the whole 32-byte word: `_timeout_called` is one
        // byte in `{0, 1}` and the word written back is at least 256. Both
        // are true, of different things wearing one name, and putting them
        // on the same graph made a reachable path look contradictory.
        if term.contains("storage(") || term.contains("cell(") {
            continue;
        }
        for (head, above) in [("or(", true), ("and(", false)] {
            let Some(inner) = term.strip_prefix(head).and_then(|r| r.strip_suffix(")")) else {
                continue;
            };
            let Some((a, b)) = split_top(inner) else {
                continue;
            };
            for arg in [a, b] {
                extra_bitwise.push(if above {
                    (arg, term.clone(), false)
                } else {
                    (term.clone(), arg, false)
                });
            }
        }
    }
    edges.extend(extra_bitwise);

    // Floyd-Warshall over the terms, carrying whether some edge on the path
    // was strict. A term reachable from itself through a strict edge is a
    // value strictly less than itself.
    // Literals order among themselves, which is how a region's bound meets a
    // relation's: `2^160 <= amount` and `amount < 1` contradict only once
    // `1 < 2^160` is on the graph.
    let mut edges = edges;
    let lits: Vec<(String, crate::interval::U256)> = edges
        .iter()
        .flat_map(|(a, b, _)| [a.clone(), b.clone()])
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .filter_map(|t| parse_decimal(&t).ok().map(|v| (t, v)))
        .collect();
    for (ta, va) in &lits {
        for (tb, vb) in &lits {
            if va < vb {
                edges.push((ta.clone(), tb.clone(), true));
            }
        }
    }
    let mut nodes: Vec<&str> = edges
        .iter()
        .flat_map(|(a, b, _)| [a.as_str(), b.as_str()])
        .collect();
    nodes.sort_unstable();
    nodes.dedup();
    if nodes.len() > 96 {
        return false;
    }
    let idx = |x: &str| nodes.binary_search(&x).ok();
    let n = nodes.len();
    // `reach[i][j]`: None if no path, Some(strict) otherwise.
    let mut reach = vec![vec![None::<bool>; n]; n];
    for (a, b, strict) in &edges {
        let (Some(i), Some(j)) = (idx(a.as_str()), idx(b.as_str())) else {
            continue;
        };
        let cur = reach[i][j];
        reach[i][j] = Some(cur.unwrap_or(false) || *strict);
    }
    for k in 0..n {
        for i in 0..n {
            let Some(ik) = reach[i][k] else { continue };
            for j in 0..n {
                let Some(kj) = reach[k][j] else { continue };
                let s = ik || kj;
                reach[i][j] = Some(reach[i][j].unwrap_or(false) || s);
            }
        }
    }
    (0..n).any(|i| reach[i][i] == Some(true))
}


/// What the facts pin `term` between, as far as the literals reach.
///
/// `require(total + amount <= 100)` leaves `add(storage(total), var_amount) <=
/// 100` on the path, and the store that follows writes exactly that term. Its
/// region is then known, where interval arithmetic alone had to guess between
/// them.
pub fn bounds_of(edges: &[(String, String, bool)], term: &str) -> IntervalSet {
    let mut lo = U256::ZERO;
    let mut hi = max_u256();
    // One literal at a time: add the assertion that would contradict it, and
    // see whether it does. That reuses the closure rather than reimplementing
    // reachability, and it is exact for the literals already on the graph.
    let lits: Vec<U256> = edges
        .iter()
        .flat_map(|(a, b, _)| [a.clone(), b.clone()])
        .filter_map(|t| parse_decimal(&t).ok())
        .collect();
    for v in lits {
        // Strict first: if `term <= v` cannot hold, then `v < term`, and the
        // bound is one above. Testing only the non-strict form put the bound
        // one short of where the facts put it.
        let mut with = edges.to_vec();
        with.push((term.to_string(), v.to_string(), false));
        if contradictory(&with) {
            if v < max_u256() && v + U256::from(1u8) > lo {
                lo = v + U256::from(1u8);
            }
        } else {
            let mut with = edges.to_vec();
            with.push((term.to_string(), v.to_string(), true));
            if contradictory(&with) && v > lo {
                lo = v;
            }
        }
        let mut with = edges.to_vec();
        with.push((v.to_string(), term.to_string(), false));
        if contradictory(&with) {
            if v > U256::ZERO && v - U256::from(1u8) < hi {
                hi = v - U256::from(1u8);
            }
        } else {
            let mut with = edges.to_vec();
            with.push((v.to_string(), term.to_string(), true));
            if contradictory(&with) && v < hi {
                hi = v;
            }
        }
    }
    if lo > hi {
        return IntervalSet::empty();
    }
    IntervalSet::range(lo, hi)
}

/// The order facts a set of assumptions carries, as edges `left <= right`,
/// with `strict` marking `<`. A fact taken to be false is the reverse edge:
/// `not (a <= b)` is `b < a`. An equality that holds is two edges.
pub fn edges_of(facts: &std::collections::BTreeMap<String, bool>) -> Vec<(String, String, bool)> {
    let mut out = vec![];
    for (k, v) in facts {
        if let Some((a, b)) = k.split_once(" == ") {
            if *v {
                out.push((a.to_string(), b.to_string(), false));
                out.push((b.to_string(), a.to_string(), false));
            }
            continue;
        }
        let (a, strict, b) = if let Some((a, b)) = k.split_once(" <= ") {
            (a, false, b)
        } else if let Some((a, b)) = k.split_once(" < ") {
            (a, true, b)
        } else {
            continue;
        };
        out.push(match (strict, v) {
            (false, true) => (a.to_string(), b.to_string(), false),
            (false, false) => (b.to_string(), a.to_string(), true),
            (true, true) => (a.to_string(), b.to_string(), true),
            (true, false) => (b.to_string(), a.to_string(), false),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// The guard the source wrote bounds the value the store writes, and the
    /// two are the same term.
    #[test]
    fn a_guard_bounds_what_the_store_writes() {
        let facts: BTreeMap<String, bool> =
            [("add(storage(total), amt) <= 100".to_string(), true)].into_iter().collect();
        let e = edges_of(&facts);
        let b = bounds_of(&e, "add(storage(total), amt)");
        assert_eq!(b, IntervalSet::le(U256::from(100u8)));
        // and a term nothing says anything about is not bounded
        assert!(bounds_of(&e, "storage(other)").is_full());
    }

    /// A guard taken the other way bounds it from below.
    #[test]
    fn a_guard_that_failed_bounds_it_from_below() {
        let facts: BTreeMap<String, bool> =
            [("x <= 100".to_string(), false)].into_iter().collect();
        let e = edges_of(&facts);
        let b = bounds_of(&e, "x");
        assert_eq!(b, IntervalSet::ge(U256::from(101u8)));
    }
}
