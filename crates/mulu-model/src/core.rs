//! Normalisation to the integer-indexed core model read by `mulu-worker`.

use crate::schema::{Control, ControlPlant, EventDecl, FiniteProduct, Initial, Transition};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CoreCheck {
    pub id: usize,
    pub pass_event: usize,
    pub fail_event: usize,
}

/// Exactly what `Mulu.Worker.CoreModel` parses. Edges are `[src, ev, dst]`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CoreModel {
    pub num_states: usize,
    pub num_events: usize,
    pub controllable: Vec<usize>,
    pub initial: Vec<usize>,
    pub marked: Vec<usize>,
    pub bad: Vec<usize>,
    pub edges: Vec<[usize; 3]>,
    pub checks: Vec<CoreCheck>,
}

/// A core model together with its name maps.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Normalized {
    pub core: CoreModel,
    pub state_names: Vec<String>,
    pub event_names: Vec<String>,
    pub check_names: Vec<String>,
}

impl Normalized {
    pub fn state_index(&self, name: &str) -> Option<usize> {
        self.state_names.iter().position(|s| s == name)
    }
    pub fn event_index(&self, name: &str) -> Option<usize> {
        self.event_names.iter().position(|s| s == name)
    }
    pub fn state_name(&self, i: usize) -> &str {
        &self.state_names[i]
    }
    pub fn event_name(&self, i: usize) -> &str {
        &self.event_names[i]
    }
    pub fn edge_names(&self, e: &[usize; 3]) -> (String, String, String) {
        (self.state_names[e[0]].clone(), self.event_names[e[1]].clone(), self.state_names[e[2]].clone())
    }
}

fn normalize_graph(
    states: &[String],
    initial: &Initial,
    marked: &[String],
    bad: &[String],
    events: &[EventDecl],
    transitions: &[Transition],
) -> (CoreModel, Vec<String>, Vec<String>) {
    let sidx: HashMap<&str, usize> = states.iter().enumerate().map(|(i, s)| (s.as_str(), i)).collect();
    let eidx: HashMap<&str, usize> = events.iter().enumerate().map(|(i, e)| (e.id.as_str(), i)).collect();
    let idx = |s: &str| sidx[s];
    let core = CoreModel {
        num_states: states.len(),
        num_events: events.len(),
        controllable: events
            .iter()
            .enumerate()
            .filter(|(_, e)| e.control == Control::Controllable)
            .map(|(i, _)| i)
            .collect(),
        initial: initial.names().iter().map(|s| idx(s)).collect(),
        marked: marked.iter().map(|s| idx(s)).collect(),
        bad: bad.iter().map(|s| idx(s)).collect(),
        edges: transitions.iter().map(|t| [idx(&t.from), eidx[t.event.as_str()], idx(&t.to)]).collect(),
        checks: vec![],
    };
    (core, states.to_vec(), events.iter().map(|e| e.id.clone()).collect())
}

/// Normalise the implementation model (validated input assumed).
pub fn normalize(m: &FiniteProduct) -> Normalized {
    let (mut core, state_names, event_names) =
        normalize_graph(&m.states, &m.initial, &m.marked, &m.bad, &m.events, &m.transitions);
    let eidx: HashMap<&str, usize> = event_names.iter().enumerate().map(|(i, e)| (e.as_str(), i)).collect();
    core.checks = m
        .checks
        .iter()
        .enumerate()
        .map(|(i, c)| CoreCheck { id: i, pass_event: eidx[c.pass_event.as_str()], fail_event: eidx[c.fail_event.as_str()] })
        .collect();
    Normalized { core, state_names, event_names, check_names: m.checks.iter().map(|c| c.id.clone()).collect() }
}

/// Normalise the control plant (validated input assumed).
pub fn normalize_plant(cp: &ControlPlant) -> Normalized {
    let (core, state_names, event_names) =
        normalize_graph(&cp.states, &cp.initial, &cp.marked, &cp.bad, &cp.events, &cp.transitions);
    Normalized { core, state_names, event_names, check_names: vec![] }
}

impl CoreModel {
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("CoreModel serialises")
    }
    pub fn is_controllable(&self, ev: usize) -> bool {
        self.controllable.contains(&ev)
    }
}
