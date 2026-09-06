//! `finite-product` schema, version 1.
//!
//! This is the P0 core fixture format: a finite graph that is *already* the
//! product of a plant and a specification monitor (`bad` states), not a
//! Solidity input. See `schemas/finite-product.v1.json` for the JSON Schema.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Control {
    Controllable,
    Uncontrollable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventDecl {
    pub id: String,
    pub control: Control,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Transition {
    pub from: String,
    pub event: String,
    pub to: String,
}

/// `initial` may be a single state name or a list.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Initial {
    One(String),
    Many(Vec<String>),
}

impl Initial {
    pub fn names(&self) -> Vec<String> {
        match self {
            Initial::One(s) => vec![s.clone()],
            Initial::Many(v) => v.clone(),
        }
    }
}

/// A check of the implementation model: its guard result is modelled by two
/// events (`pass_event` / `fail_event`). `depends_on` is the sound (not
/// necessarily minimal) dependency list reported with a redundancy finding.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckDecl {
    pub id: String,
    pub pass_event: String,
    pub fail_event: String,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// One state of the control plant paired with the implementation state it
/// abstracts the same concrete situation as.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SitePair {
    pub plant_state: String,
    pub impl_state: String,
}

/// A control site of the reference plant: a location where the supervisor
/// may forbid `continue_event`. If `check` names an implementation check, the
/// site corresponds to that check and overrestriction candidates are derived
/// from the pairs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlSite {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check: Option<String>,
    pub continue_event: String,
    #[serde(default)]
    pub pairs: Vec<SitePair>,
}

/// The conservative reference plant (docs/11 §4): controllable `Continue`
/// events at declared sites, everything else uncontrollable.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlPlant {
    pub states: Vec<String>,
    pub initial: Initial,
    pub marked: Vec<String>,
    pub bad: Vec<String>,
    pub events: Vec<EventDecl>,
    pub transitions: Vec<Transition>,
    /// Marked states that mean *successful* completion (default: all marked).
    /// Used to decide whether the envelope could actually *accept* a request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepting: Option<Vec<String>>,
    #[serde(default)]
    pub sites: Vec<ControlSite>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FiniteProduct {
    pub schema_version: u32,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub states: Vec<String>,
    pub initial: Initial,
    pub marked: Vec<String>,
    pub bad: Vec<String>,
    pub events: Vec<EventDecl>,
    pub transitions: Vec<Transition>,
    #[serde(default)]
    pub checks: Vec<CheckDecl>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control_plant: Option<ControlPlant>,
}

pub const SCHEMA_VERSION: u32 = 1;
pub const KIND_FINITE_PRODUCT: &str = "finite-product";
