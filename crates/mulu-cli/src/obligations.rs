//! P1-04: what stands between a model claim and a claim about the program.
//!
//! docs/04 §7 stacks the layers — source, chosen artifact, concrete
//! semantics, finite model — and says a claim crosses one at a time.
//! `lean/Mulu/Semantics/Simulation.lean` states the crossing: an abstraction
//! with `initial_covered` and `step_covered` carries a checked invariant to
//! the modelled system, and a per-check `FailStepMatched` carries a
//! `never-fails`.
//!
//! This module is the ledger of those conditions. Each recognition the
//! pipeline makes raises one; none is discharged, because no formal semantics
//! of solc's Yul exists to discharge them against. So nothing is promoted past
//! `abstract-model`, and the reason is enumerated rather than described.
//!
//! The rule is executable: `promoted_scope` decides a finding's scope from the
//! ledger, and `verify` recomputes it, so a scope cannot be raised by editing
//! a report.

use mulu_abstraction::model::AbstractionReport;
use mulu_yul::{CheckOrigin, ProgramIr};
use serde::{Deserialize, Serialize};

/// The layers of docs/04 §7, innermost first.
pub const LAYERS: &[&str] = &["abstract-model", "yul-semantics", "solidity-source", "evm-bytecode"];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Obligation {
    pub id: String,
    /// The layer a claim reaches once this is discharged.
    pub reaches: String,
    /// What has to hold, in the words of the Lean statement where there is one.
    pub statement: String,
    /// The Lean name that would consume it, when it has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lean: Option<String>,
    /// What the analyser did that made this necessary.
    pub raised_by: Vec<String>,
    /// `None` while open. Nothing sets this yet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discharged_by: Option<String>,
}

impl Obligation {
    pub fn open(&self) -> bool {
        self.discharged_by.is_none()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ledger {
    pub obligations: Vec<Obligation>,
    /// The layer every finding is reported at, given what is open.
    pub scope: String,
    pub note: String,
}

impl Ledger {
    pub fn open(&self) -> Vec<&Obligation> {
        self.obligations.iter().filter(|o| o.open()).collect()
    }

    pub fn find(&self, id: &str) -> Option<&Obligation> {
        self.obligations.iter().find(|o| o.id == id)
    }

    /// The scope a finding may be reported at. A claim reaches a layer only
    /// when every obligation up to it is discharged, so one open obligation at
    /// the first layer holds everything at `abstract-model`.
    pub fn promoted_scope(&self, depends_on: &[&str]) -> &'static str {
        let mut reached = "abstract-model";
        for layer in LAYERS.iter().skip(1) {
            let at_layer: Vec<&Obligation> =
                self.obligations.iter().filter(|o| o.reaches == **layer).collect();
            // A layer nothing is recorded for is not a layer we have shown how
            // to reach. Silence there means the conditions were never
            // enumerated, which is not the same as their being met.
            if at_layer.is_empty() {
                return reached;
            }
            let blocking = at_layer.iter().any(|o| {
                o.open() && (depends_on.is_empty() || depends_on.contains(&o.id.as_str()))
            });
            if blocking {
                return reached;
            }
            reached = layer;
        }
        reached
    }
}

fn ob(
    id: &str,
    reaches: &str,
    statement: &str,
    lean: Option<&str>,
    raised_by: Vec<String>,
) -> Obligation {
    Obligation {
        id: id.to_string(),
        reaches: reaches.to_string(),
        statement: statement.to_string(),
        lean: lean.map(|s| s.to_string()),
        raised_by,
        discharged_by: None,
    }
}

/// Build the ledger for one analysis.
pub fn ledger(ir: &ProgramIr, report: &AbstractionReport) -> Ledger {
    let mut out = Vec::new();

    // The root. Everything above needs a semantics to be stated against, and
    // there is none, so nothing above can be discharged either.
    out.push(ob(
        "semantics:yul-not-formalised",
        "yul-semantics",
        "A formal semantics of the Yul solc emits, in Lean, against which the conditions \
         below can be stated. Without it `Concrete` has no instance and the simulation \
         theorem has nothing to apply to.",
        Some("Mulu.Semantics.Concrete"),
        vec![format!("the analysed artifact is {}", ir.derived_from)],
    ));

    // The two conditions of the simulation.
    let mut initial_raised = vec![];
    let mut step_raised = vec![];
    for a in &report.assumptions {
        let short = a.split(':').next().unwrap_or(a).to_string();
        if a.starts_with("initial-state") {
            initial_raised.push(short);
        } else {
            step_raised.push(short);
        }
    }
    for c in ir.checks.iter().filter(|c| c.origin != CheckOrigin::Compiler) {
        for a in &c.assumptions {
            let short = a.split(':').next().unwrap_or(a).to_string();
            if !step_raised.contains(&short) {
                step_raised.push(short);
            }
        }
    }
    initial_raised.sort();
    step_raised.sort();

    out.push(ob(
        "simulation:initial-covered",
        "yul-semantics",
        "Every concrete initial state maps into the model's initial states. The model \
         starts where the constructor leaves the contract, and nowhere else.",
        Some("Mulu.Semantics.Simulation.initial_covered"),
        initial_raised,
    ));
    out.push(ob(
        "simulation:step-covered",
        "yul-semantics",
        "Every concrete step from a reachable state is matched by an edge of the model. A \
         step with no matching edge is a behaviour the model does not have, and every \
         claim built on it fails.",
        Some("Mulu.Semantics.Simulation.step_covered"),
        step_raised,
    ));

    // One per check the source declares: a `never-fails` says something about
    // the program only if the concrete failing branch is matched by *that*
    // check's fail event.
    for c in ir.checks.iter().filter(|c| c.origin != CheckOrigin::Compiler) {
        let where_ = match (&c.declared_in, &c.written_in) {
            (Some(ct), Some(m)) => format!("{ct}.{m}"),
            _ => c.function.clone(),
        };
        out.push(ob(
            &format!("check:fail-step-matched:{}", c.id),
            "yul-semantics",
            &format!(
                "When the concrete semantics takes the failing branch of check {} ({}), the \
                 model has that check's fail event on the corresponding edge. The simulation \
                 gives some edge; this says which.",
                c.id, where_
            ),
            Some("Mulu.Semantics.Simulation.FailStepMatched"),
            vec![format!("check {} recognised in {where_}", c.id)],
        ));
    }

    // The reference plant is a second model, and comparing it with the first
    // is only meaningful if its control sites correspond to what the
    // implementation can actually express (docs/11 E9).
    if ir.checks.iter().any(|c| c.origin != CheckOrigin::Compiler) {
        out.push(ob(
            "plant:policy-corresponds",
            "yul-semantics",
            "A supervisor allowed by the reference plant is realisable by a program of the              shape the implementation has: continuing at a site is a condition the code could              carry, and rejecting stays available. Without this the envelope bounds a plant              the implementation is not an instance of, and an overrestriction compares two              unrelated things.",
            None,
            vec!["the reference plant parameterises each guard out".into()],
        ));
    }

    // The layers above, which P1 does not attempt.
    out.push(ob(
        "compilation:yul-corresponds-to-source",
        "solidity-source",
        "solc's lowering of this contract to Yul preserves the behaviours the claims are \
         about. Not attempted: the analysed artifact is the Yul, and a statement about the \
         Solidity source needs this link.",
        None,
        vec!["the front end reads solc's `ir` output".into()],
    ));
    out.push(ob(
        "compilation:optimised-bytecode",
        "evm-bytecode",
        "The deployed bytecode behaves as the unoptimized Yul does. Not attempted: the \
         optimizer is off and the analysed artifact is not what would be deployed.",
        None,
        vec!["the analysis reads unoptimized Yul".into()],
    ));

    let scope = Ledger {
        obligations: out.clone(),
        scope: String::new(),
        note: String::new(),
    }
    .promoted_scope(&[])
    .to_string();
    Ledger {
        obligations: out,
        scope,
        note: "A finding is reported at the deepest layer whose obligations are all \
               discharged. None is, so every finding is a claim about the generated model."
            .into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn led(obs: Vec<Obligation>) -> Ledger {
        Ledger { obligations: obs, scope: "abstract-model".into(), note: String::new() }
    }

    fn discharged(mut o: Obligation) -> Obligation {
        o.discharged_by = Some("a proof".into());
        o
    }

    #[test]
    fn one_open_obligation_holds_everything_at_the_model() {
        let l = led(vec![ob("a", "yul-semantics", "", None, vec![])]);
        assert_eq!(l.promoted_scope(&[]), "abstract-model");
    }

    #[test]
    fn a_layer_is_reached_only_when_every_obligation_up_to_it_is_discharged() {
        let l = led(vec![
            discharged(ob("a", "yul-semantics", "", None, vec![])),
            ob("b", "solidity-source", "", None, vec![]),
        ]);
        assert_eq!(l.promoted_scope(&[]), "yul-semantics");

        let l = led(vec![
            discharged(ob("a", "yul-semantics", "", None, vec![])),
            discharged(ob("b", "solidity-source", "", None, vec![])),
            ob("c", "evm-bytecode", "", None, vec![]),
        ]);
        assert_eq!(l.promoted_scope(&[]), "solidity-source");

        // and an open one at the first layer holds everything back, however
        // much is proved above it
        let l = led(vec![
            ob("a", "yul-semantics", "", None, vec![]),
            discharged(ob("b", "solidity-source", "", None, vec![])),
            discharged(ob("c", "evm-bytecode", "", None, vec![])),
        ]);
        assert_eq!(l.promoted_scope(&[]), "abstract-model");
    }

    #[test]
    fn a_finding_is_held_back_by_its_own_obligations_only() {
        let l = led(vec![
            discharged(ob("shared", "yul-semantics", "", None, vec![])),
            ob("check:fail-step-matched:A", "yul-semantics", "", None, vec![]),
        ]);
        // a finding that does not rest on check A's obligation may still move,
        // but only to the layer the ledger actually describes
        assert_eq!(l.promoted_scope(&["shared"]), "yul-semantics");
        // one that does may not
        assert_eq!(
            l.promoted_scope(&["shared", "check:fail-step-matched:A"]),
            "abstract-model"
        );
    }

    #[test]
    fn a_layer_nothing_is_recorded_for_is_not_reached() {
        // Silence about a layer means its conditions were never enumerated.
        let l = led(vec![discharged(ob("a", "yul-semantics", "", None, vec![]))]);
        assert_eq!(l.promoted_scope(&[]), "yul-semantics", "and no further");
        assert_eq!(led(vec![]).promoted_scope(&[]), "abstract-model");
    }

    #[test]
    fn everything_discharged_reaches_the_bytecode() {
        let l = led(vec![
            discharged(ob("a", "yul-semantics", "", None, vec![])),
            discharged(ob("b", "solidity-source", "", None, vec![])),
            discharged(ob("c", "evm-bytecode", "", None, vec![])),
        ]);
        assert_eq!(l.promoted_scope(&[]), "evm-bytecode");
    }
}

/// Which obligations a finding rests on. A claim is reported at the deepest
/// layer all of these reach.
pub fn depends_on(kind: &str, check_id: Option<&str>) -> Vec<String> {
    let mut v = vec![
        "semantics:yul-not-formalised".to_string(),
        "simulation:initial-covered".to_string(),
        "simulation:step-covered".to_string(),
    ];
    match kind {
        // A redundancy claim also needs its own check's failing branch to be
        // the one the model names.
        "redundant-check" | "check-may-fail" | "unreachable-check" | "check-unknown" => {
            if let Some(id) = check_id {
                v.push(format!("check:fail-step-matched:{id}"));
            }
        }
        // Anything about the envelope compares two models, so the plant has to
        // correspond as well.
        "envelope" | "overrestriction" => v.push("plant:policy-corresponds".to_string()),
        _ => {}
    }
    v
}
