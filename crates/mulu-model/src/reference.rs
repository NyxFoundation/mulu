//! Reference algorithms in plain Rust.
//!
//! Two flavours:
//! * `reach` / `envelope_iter` — straightforward fixpoint iterations, used to
//!   cross-check the Lean worker's *numbers* (a mismatch is a bug somewhere);
//! * `envelope_bruteforce` — enumerates **every** subset of `Q \ B`, keeps the
//!   good ones and returns their union. Exponential; only for tiny fixtures.
//!
//! Neither is a proof. The proof is the Lean certificate check.

use crate::core::CoreModel;
use std::collections::BTreeSet;

pub fn reach(m: &CoreModel) -> BTreeSet<usize> {
    let mut r: BTreeSet<usize> = m.initial.iter().copied().collect();
    loop {
        let mut added = false;
        for e in &m.edges {
            if r.contains(&e[0]) && r.insert(e[2]) {
                added = true;
            }
        }
        if !added {
            return r;
        }
    }
}

fn uc_closed(m: &CoreModel, w: &BTreeSet<usize>, q: usize) -> bool {
    m.edges.iter().all(|e| e[0] != q || m.is_controllable(e[1]) || w.contains(&e[2]))
}

fn coreach_in(m: &CoreModel, u: &BTreeSet<usize>) -> BTreeSet<usize> {
    let mut s: BTreeSet<usize> = u.iter().copied().filter(|q| m.marked.contains(q)).collect();
    loop {
        let mut added = false;
        for e in &m.edges {
            if u.contains(&e[0]) && s.contains(&e[2]) && s.insert(e[0]) {
                added = true;
            }
        }
        if !added {
            return s;
        }
    }
}

fn step(m: &CoreModel, w: &BTreeSet<usize>, nonblocking: bool) -> BTreeSet<usize> {
    let u: BTreeSet<usize> = w.iter().copied().filter(|&q| uc_closed(m, w, q)).collect();
    if nonblocking {
        coreach_in(m, &u)
    } else {
        u
    }
}

/// The chain `W₀ ⊋ W₁ ⊋ … ⊋ Wₙ = F(Wₙ)`.
pub fn envelope_chain(m: &CoreModel, nonblocking: bool) -> Vec<BTreeSet<usize>> {
    let mut w: BTreeSet<usize> = (0..m.num_states).filter(|q| !m.bad.contains(q)).collect();
    let mut chain = vec![w.clone()];
    loop {
        let w2 = step(m, &w, nonblocking);
        if w2 == w {
            return chain;
        }
        chain.push(w2.clone());
        w = w2;
    }
}

pub fn envelope_iter(m: &CoreModel, nonblocking: bool) -> BTreeSet<usize> {
    envelope_chain(m, nonblocking).pop().unwrap()
}

fn is_good(m: &CoreModel, w: &BTreeSet<usize>, nonblocking: bool) -> bool {
    w.iter().all(|&q| !m.bad.contains(&q) && uc_closed(m, w, q))
        && (!nonblocking || coreach_in(m, w).len() == w.len())
}

/// Union of all good subsets (the greatest good set). Panics above 16 states.
pub fn envelope_bruteforce(m: &CoreModel, nonblocking: bool) -> BTreeSet<usize> {
    assert!(m.num_states <= 16, "brute force is only for tiny fixtures");
    let candidates: Vec<usize> = (0..m.num_states).filter(|q| !m.bad.contains(q)).collect();
    let mut best = BTreeSet::new();
    for mask in 0u32..(1u32 << candidates.len()) {
        let w: BTreeSet<usize> = candidates.iter().enumerate().filter(|(i, _)| mask & (1 << i) != 0).map(|(_, &q)| q).collect();
        if is_good(m, &w, nonblocking) {
            best.extend(w);
        }
    }
    best
}

/// States of `w` from which some state of `targets` is reachable inside `w`.
pub fn coreach_within(m: &CoreModel, w: &BTreeSet<usize>, targets: &BTreeSet<usize>) -> BTreeSet<usize> {
    let mut s: BTreeSet<usize> = w.intersection(targets).copied().collect();
    loop {
        let mut added = false;
        for e in &m.edges {
            if w.contains(&e[0]) && s.contains(&e[2]) && s.insert(e[0]) {
                added = true;
            }
        }
        if !added {
            return s;
        }
    }
}

pub fn disabled(m: &CoreModel, w: &BTreeSet<usize>) -> Vec<[usize; 3]> {
    m.edges.iter().copied().filter(|e| w.contains(&e[0]) && m.is_controllable(e[1]) && !w.contains(&e[2])).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::CoreModel;

    fn fixture(controllable: Vec<usize>, edges: Vec<[usize; 3]>, bad: Vec<usize>) -> CoreModel {
        CoreModel { num_states: 3, num_events: 2, controllable, initial: vec![0], marked: vec![0], bad, edges, checks: vec![] }
    }

    #[test]
    fn docs_09_fixture_one() {
        let m = fixture(vec![0], vec![[0, 0, 1], [1, 1, 2]], vec![2]);
        let w = envelope_iter(&m, true);
        assert_eq!(w, BTreeSet::from([0]));
        assert_eq!(disabled(&m, &w), vec![[0, 0, 1]]);
        assert_eq!(envelope_bruteforce(&m, true), w);
    }

    #[test]
    fn docs_09_fixture_two_unrealizable() {
        let m = fixture(vec![], vec![[0, 0, 1], [1, 1, 2]], vec![2]);
        let w = envelope_iter(&m, true);
        assert!(w.is_empty());
        assert_eq!(envelope_bruteforce(&m, true), w);
    }

    #[test]
    fn docs_09_fixture_three_nonblocking_vs_safety() {
        let m = fixture(vec![0], vec![[0, 0, 1]], vec![]);
        assert_eq!(envelope_iter(&m, true), BTreeSet::from([0]));
        assert_eq!(envelope_iter(&m, false), BTreeSet::from([0, 1, 2]));
        assert_eq!(envelope_bruteforce(&m, true), BTreeSet::from([0]));
        assert_eq!(envelope_bruteforce(&m, false), BTreeSet::from([0, 1, 2]));
    }

    #[test]
    fn reach_is_minimal() {
        let m = fixture(vec![0], vec![[1, 1, 2]], vec![]);
        assert_eq!(reach(&m), BTreeSet::from([0]));
    }
}
