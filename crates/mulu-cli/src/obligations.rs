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
//! The rule is executable. `promoted_scope` decides a finding's scope from the
//! ledger; `verify` recomputes what a finding rests on from its own kind,
//! rather than reading the list off the report, refuses a ledger claiming a
//! discharge it cannot check, and refuses a report whose ledger is missing.
//! What it does not do is verify a discharge, because nothing here can: an
//! obligation is discharged by a proof, and none exists.

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
    /// Who could ever discharge this. `mulu` is work on this project and the
    /// statement is ours to prove. `solc` is a correctness property of a
    /// compiler nobody has proved correct, so it is an assumption on a third
    /// party rather than an item on our list, and listing the two together
    /// makes the second look like the first. `caller` is neither: the
    /// analysis was handed something that stands for nothing.
    #[serde(default = "mulu_bearer")]
    pub bearer: String,
    /// The artifact that would discharge this, where one is known to exist.
    /// Naming it is not a claim that it has been used, or that it is correct.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub would_need: Option<String>,
    /// `None` while open. Nothing sets this yet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discharged_by: Option<String>,
}

fn mulu_bearer() -> String {
    "mulu".into()
}

impl Obligation {
    pub fn open(&self) -> bool {
        self.discharged_by.is_none()
    }

    /// Open, and ours to close. The rest are open because someone else has
    /// not proved their compiler correct, which no amount of work here fixes.
    pub fn ours(&self) -> bool {
        self.bearer == "mulu"
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ledger {
    /// `solidity` when the model was built from a contract, `model-only` when
    /// one was handed in directly. They rest on different things.
    #[serde(default = "solidity_kind")]
    pub kind: String,
    pub obligations: Vec<Obligation>,
    /// The layer every finding is reported at, given what is open.
    pub scope: String,
    pub note: String,
}

fn solidity_kind() -> String {
    "solidity".into()
}

/// The obligation a model handed in directly rests on: nothing above it is
/// even defined, because there is no artifact for it to correspond to.
pub const MODEL_ONLY: &str = "correspondence:no-artifact";

impl Ledger {
    /// A model given directly to the analyser. Its findings are about that
    /// model and there is no program they could be about, so the layers above
    /// are not merely undischarged, they are undefined.
    pub fn model_only() -> Ledger {
        let mut o = ob(
            MODEL_ONLY,
            "yul-semantics",
            "The analysis was handed a finite model, not a contract. Nothing says what this \
             model is a model of, so no claim about a program can be built on it. Discharging \
             this means supplying the artifact and its correspondence, which is what analysing \
             a contract with `mulu analyze` produces; this ledger would then be that one.",
            None,
            vec!["the model was given directly".into()],
        );
        o.bearer = "caller".into();
        Ledger {
            kind: "model-only".into(),
            obligations: vec![o],
            scope: "abstract-model".into(),
            note: "A model given directly stands for nothing but itself.".into(),
        }
    }

    /// What a finding of this kind rests on, in this ledger. An unrecognised
    /// ledger kind is treated as the stricter of the two rather than waved
    /// through as an ordinary analysis.
    pub fn depends_on(&self, kind: &str, check_id: Option<&str>) -> Vec<String> {
        match self.kind.as_str() {
            "solidity" => depends_on(kind, check_id),
            _ => vec![MODEL_ONLY.to_string()],
        }
    }

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
        bearer: mulu_bearer(),
        would_need: None,
    }
}

/// An obligation nobody here can close: a property of a compiler this project
/// did not write and nobody has proved correct.
fn on_solc(mut o: Obligation, would_need: &str) -> Obligation {
    o.bearer = "solc".into();
    o.would_need = Some(would_need.to_string());
    o
}

/// Build the ledger for one analysis.
pub fn ledger(
    ir: &ProgramIr,
    report: &AbstractionReport,
    drift: Option<&mulu_solc::Drift>,
    normalisations: &[String],
    hazards: &[String],
) -> Ledger {
    let mut out = Vec::new();

    // The root. Everything above needs a semantics to be stated against, and
    // there is none, so nothing above can be discharged either.
    // The root used to be "there is no semantics". There is one now:
    // `semantics/` instantiates `Mulu.Semantics.Concrete` at Nethermind's
    // EvmYul, the model Paradigm's Solidus pins. Adopting it did not make a
    // claim true, it split one obligation into two smaller and more honest
    // ones: this contract is not yet expressed in that semantics, and that
    // semantics is not proved to agree with the EVM.
    // The contract is in the semantics now: `semantics/<Name>.lean` in the
    // analysis directory renders the same `ir` the model was built from. What
    // is not proved is that the rendering is the same program. It is not a
    // transcription, and each way it is not is listed here rather than
    // described, so a contract that needed none of them carries none.
    let mut rendering = ob(
        "semantics:rendering-preserves-the-program",
        "yul-semantics",
        "The module rendered into `semantics/` denotes the same program as the Yul it was \
         rendered from. It is not a transcription: EvmYul's notation and AST differ from Yul's \
         grammar in a few places, and each difference is bridged by a rewrite that is faithful \
         to the Yul specification and is not proved to be.",
        Some("MuluSemantics.concrete"),
        {
            let mut v = vec![format!("the analysed artifact is {}", ir.derived_from)];
            v.extend(normalisations.iter().cloned());
            v
        },
    );
    rendering.would_need = Some(
        "a proof that each rewrite the renderer applies preserves the meaning of the Yul, \
         stated against the same semantics. Nothing in the toolchain proves it today; a \
         differential run of the rendered module against the EVM would be evidence and not a \
         proof."
            .into(),
    );
    out.push(rendering);
    let mut adopted = ob(
        "semantics:evmyul-matches-the-evm",
        "yul-semantics",
        "The adopted semantics is the semantics of Yul. EvmYul is executable and is run \
         against the Ethereum test suite, which is evidence and not a proof: a divergence \
         between it and a real client would make every claim above it wrong in the same way.",
        Some("EvmYul.Yul.exec"),
        vec!["the correspondence is stated against EvmYul".into()],
    );
    // Not ours. Adopting a semantics moves an assumption; it does not remove
    // one, and the ledger has to show where the assumption went.
    adopted.bearer = "evmyul".into();
    // Not a possibility any more, where a contract contains one of them: a
    // measured defect belongs on the obligation it defeats.
    adopted.raised_by.extend(hazards.iter().cloned());
    adopted.would_need = Some(
        "a proof that EvmYul agrees with the EVM. None exists for any EVM semantics. Its \
         conformance runs against ethereum/tests are the evidence there is."
            .into(),
    );
    out.push(adopted);

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
    // A behaviour the abstraction could not model is not a missing proof of
    // step-covered, it is a reason step-covered is false. It belongs in the
    // ledger next to the condition it defeats, not only in a warning.
    for u in &report.unsupported {
        step_raised.push(format!("not modelled: {u}"));
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
    out.push(on_solc(
        ob(
            "compilation:yul-corresponds-to-source",
            "solidity-source",
            "solc's lowering of this contract to Yul preserves the behaviours the claims are \
             about. Not attempted: the analysed artifact is the Yul, and a statement about the \
             Solidity source needs this link.",
            None,
            vec!["the front end reads solc's `ir` output".into()],
        ),
        "a proof that solc lowers Solidity to Yul faithfully. None exists. A verified compiler \
         (Paradigm's Solidus, Verity) replaces the question rather than answering it for solc: \
         it would be a different compiler, so the obligation would become a proof about that \
         one.",
    ));
    // P1b: when the project's own build was read, every way this compilation
    // differs from it is a reason the deployed bytecode is not this artifact,
    // and belongs on the obligation that says so rather than in a warning
    // that scrolls past.
    let mut deployed_raised = vec!["the analysis reads unoptimized Yul".to_string()];
    if let Some(d) = drift {
        deployed_raised.extend(d.lines());
    }
    out.push(on_solc(
        ob(
            "compilation:optimised-bytecode",
            "evm-bytecode",
            "The deployed bytecode behaves as the unoptimized Yul does. Not attempted: the \
             optimizer is off and the analysed artifact is not what would be deployed.",
            None,
            deployed_raised,
        ),
        "a proof that solc's Yul-to-bytecode compilation and its optimizer preserve behaviour. \
         None exists for solc. Solidus proves exactly this for its own backend, so compiling \
         with it would move this obligation to that proof, not discharge it here.",
    ));

    // A layer that is not one of `LAYERS` can neither block nor promote, so a
    // typo here would drop an obligation without a word.
    debug_assert!(
        out.iter().all(|o| LAYERS.contains(&o.reaches.as_str())),
        "every obligation must name a layer of docs/04 §7"
    );
    let unknown: Vec<&str> = out
        .iter()
        .map(|o| o.reaches.as_str())
        .filter(|r| !LAYERS.contains(r))
        .collect();
    assert!(unknown.is_empty(), "obligations name layers that do not exist: {unknown:?}");

    let scope = Ledger {
        kind: solidity_kind(),
        obligations: out.clone(),
        scope: String::new(),
        note: String::new(),
    }
    .promoted_scope(&[])
    .to_string();
    Ledger {
        kind: solidity_kind(),
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
        Ledger {
            kind: "solidity".into(),
            obligations: obs,
            scope: "abstract-model".into(),
            note: String::new(),
        }
    }

    #[test]
    fn a_model_given_directly_rests_on_one_thing_and_reaches_nothing() {
        let l = Ledger::model_only();
        assert_eq!(l.promoted_scope(&[]), "abstract-model");
        assert_eq!(l.depends_on("redundant-check", Some("A")), vec![MODEL_ONLY.to_string()]);
        assert_eq!(l.depends_on("envelope", None), vec![MODEL_ONLY.to_string()]);
        assert!(l.obligations.iter().all(|o| o.open()));
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
        "semantics:rendering-preserves-the-program".to_string(),
        "semantics:evmyul-matches-the-evm".to_string(),
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
