//! Validation (P0-02): out-of-range ids, duplicate ids, several targets for
//! the same `(state, event)`, unknown enum values, unknown schema versions.
//!
//! Unknown *fields* are rejected by serde (`deny_unknown_fields`) before
//! reaching here; unknown `control` values likewise.

use crate::schema::{ControlPlant, EventDecl, FiniteProduct, Initial, Transition};
use std::collections::{HashMap, HashSet};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ValidationError {
    #[error("unsupported schema_version {0} (expected {expected})", expected = crate::schema::SCHEMA_VERSION)]
    SchemaVersion(u32),
    #[error("unsupported kind {0:?} (expected \"finite-product\")")]
    Kind(String),
    #[error("{section}: duplicate id {id:?}")]
    DuplicateId { section: &'static str, id: String },
    #[error("{section}: unknown {what} {id:?}")]
    UnknownId { section: &'static str, what: &'static str, id: String },
    #[error("{section}: initial state list is empty")]
    NoInitial { section: &'static str },
    #[error("{section}: state {from:?} on event {event:?} has several targets ({a:?}, {b:?}); the supervisory-control core needs a partially deterministic plant")]
    Nondeterministic { section: &'static str, from: String, event: String, a: String, b: String },
    #[error("{section}: identical transition listed twice ({from:?} --{event}--> {to:?})")]
    DuplicateTransition { section: &'static str, from: String, event: String, to: String },
    #[error("checks: check {id:?} uses the same event {event:?} as pass and fail")]
    CheckSameEvent { id: String, event: String },
    #[error("control_plant.sites: site {id:?} continue_event {event:?} must be controllable")]
    SiteNotControllable { id: String, event: String },
}

fn check_unique(section: &'static str, ids: impl Iterator<Item = String>) -> Result<HashSet<String>, ValidationError> {
    let mut seen = HashSet::new();
    for id in ids {
        if !seen.insert(id.clone()) {
            return Err(ValidationError::DuplicateId { section, id });
        }
    }
    Ok(seen)
}

fn check_graph(
    section: &'static str,
    states: &[String],
    initial: &Initial,
    marked: &[String],
    bad: &[String],
    events: &[EventDecl],
    transitions: &[Transition],
) -> Result<(), ValidationError> {
    let state_set = check_unique(section, states.iter().cloned())?;
    let event_set = check_unique(section, events.iter().map(|e| e.id.clone()))?;
    let known = |what: &'static str, id: &str| -> Result<(), ValidationError> {
        if state_set.contains(id) {
            Ok(())
        } else {
            Err(ValidationError::UnknownId { section, what, id: id.to_string() })
        }
    };
    let init = initial.names();
    if init.is_empty() {
        return Err(ValidationError::NoInitial { section });
    }
    for s in &init {
        known("initial state", s)?;
    }
    for s in marked {
        known("marked state", s)?;
    }
    for s in bad {
        known("bad state", s)?;
    }
    let mut targets: HashMap<(&str, &str), &str> = HashMap::new();
    for t in transitions {
        known("state", &t.from)?;
        known("state", &t.to)?;
        if !event_set.contains(&t.event) {
            return Err(ValidationError::UnknownId { section, what: "event", id: t.event.clone() });
        }
        match targets.insert((&t.from, &t.event), &t.to) {
            None => {}
            Some(prev) if prev == t.to => {
                return Err(ValidationError::DuplicateTransition {
                    section,
                    from: t.from.clone(),
                    event: t.event.clone(),
                    to: t.to.clone(),
                })
            }
            Some(prev) => {
                return Err(ValidationError::Nondeterministic {
                    section,
                    from: t.from.clone(),
                    event: t.event.clone(),
                    a: prev.to_string(),
                    b: t.to.clone(),
                })
            }
        }
    }
    Ok(())
}

pub fn validate(m: &FiniteProduct) -> Result<(), ValidationError> {
    if m.schema_version != crate::schema::SCHEMA_VERSION {
        return Err(ValidationError::SchemaVersion(m.schema_version));
    }
    if m.kind != crate::schema::KIND_FINITE_PRODUCT {
        return Err(ValidationError::Kind(m.kind.clone()));
    }
    check_graph("model", &m.states, &m.initial, &m.marked, &m.bad, &m.events, &m.transitions)?;
    let event_set: HashSet<&str> = m.events.iter().map(|e| e.id.as_str()).collect();
    let check_set = check_unique("checks", m.checks.iter().map(|c| c.id.clone()))?;
    for c in &m.checks {
        for ev in [&c.pass_event, &c.fail_event] {
            if !event_set.contains(ev.as_str()) {
                return Err(ValidationError::UnknownId { section: "checks", what: "event", id: ev.clone() });
            }
        }
        if c.pass_event == c.fail_event {
            return Err(ValidationError::CheckSameEvent { id: c.id.clone(), event: c.pass_event.clone() });
        }
        for d in &c.depends_on {
            if !check_set.contains(d) {
                return Err(ValidationError::UnknownId { section: "checks", what: "check", id: d.clone() });
            }
        }
    }
    if let Some(cp) = &m.control_plant {
        validate_control_plant(cp, &m.states, &check_set)?;
    }
    Ok(())
}

fn validate_control_plant(cp: &ControlPlant, impl_states: &[String], checks: &HashSet<String>) -> Result<(), ValidationError> {
    let section = "control_plant";
    check_graph(section, &cp.states, &cp.initial, &cp.marked, &cp.bad, &cp.events, &cp.transitions)?;
    let plant_states: HashSet<&str> = cp.states.iter().map(|s| s.as_str()).collect();
    let impl_set: HashSet<&str> = impl_states.iter().map(|s| s.as_str()).collect();
    let controllable: HashSet<&str> = cp
        .events
        .iter()
        .filter(|e| e.control == crate::schema::Control::Controllable)
        .map(|e| e.id.as_str())
        .collect();
    let event_set: HashSet<&str> = cp.events.iter().map(|e| e.id.as_str()).collect();
    if let Some(acc) = &cp.accepting {
        for s in acc {
            if !plant_states.contains(s.as_str()) {
                return Err(ValidationError::UnknownId { section: "control_plant.accepting", what: "state", id: s.clone() });
            }
        }
    }
    check_unique("control_plant.sites", cp.sites.iter().map(|s| s.id.clone()))?;
    for s in &cp.sites {
        if !event_set.contains(s.continue_event.as_str()) {
            return Err(ValidationError::UnknownId { section: "control_plant.sites", what: "event", id: s.continue_event.clone() });
        }
        if !controllable.contains(s.continue_event.as_str()) {
            return Err(ValidationError::SiteNotControllable { id: s.id.clone(), event: s.continue_event.clone() });
        }
        if let Some(c) = &s.check {
            if !checks.contains(c) {
                return Err(ValidationError::UnknownId { section: "control_plant.sites", what: "check", id: c.clone() });
            }
        }
        for p in &s.pairs {
            if !plant_states.contains(p.plant_state.as_str()) {
                return Err(ValidationError::UnknownId { section: "control_plant.sites", what: "plant state", id: p.plant_state.clone() });
            }
            if !impl_set.contains(p.impl_state.as_str()) {
                return Err(ValidationError::UnknownId { section: "control_plant.sites", what: "impl state", id: p.impl_state.clone() });
            }
        }
    }
    Ok(())
}

/// Parse + validate from JSON text. Unknown fields / enum values surface as
/// serde errors, everything else as [`ValidationError`].
pub fn parse_and_validate(text: &str) -> Result<FiniteProduct, String> {
    let m: FiniteProduct = serde_json::from_str(text).map_err(|e| format!("schema: {e}"))?;
    validate(&m).map_err(|e| e.to_string())?;
    Ok(m)
}
