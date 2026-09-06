//! P1-03: turn an abstract counterexample into concrete calls and run them.
//!
//! docs/09 §5 keeps `proven` and `reproduced` apart. The model path is proven
//! by a kernel-checked certificate; running it on an EVM is a different kind
//! of evidence, and neither stands in for the other. So a replay is recorded
//! beside the certificate, never in place of it.
//!
//! docs/08 §5 also warns that a replay which does not reproduce is not
//! evidence the counterexample was spurious. Here it means the model and the
//! EVM disagree, which is reported loudly and makes the unit incomplete.

use anyhow::{Context, Result};
use mulu_abstraction::interval::IntervalSet;
use mulu_abstraction::model::AbstractionReport;
use mulu_replay::{replay, Call, Replay};
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Serialize)]
pub struct Reproduction {
    /// reproduced / not-reproduced / unsupported
    pub status: &'static str,
    /// Why, when the answer is not `reproduced`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub calls: Vec<Call>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run: Option<Replay>,
    /// Where the record was written.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub assumptions: Vec<&'static str>,
}

pub const ASSUMPTIONS: &[&str] = &[
    "replay-caller-fixed: one caller sends every call, which the environment profile permits \
     because a guard that reads the caller is not in the P1a subset",
    "replay-value-zero: calls carry no value, as the environment profile states",
    "replay-witness-is-one-value: the region's smallest member stands for the region; another \
     member could behave differently only outside the modelled subset",
];

/// Read the call events off an abstract path and pick a concrete value for
/// each. The region's smallest member is the witness.
pub fn concretise(path: &Value, report: &AbstractionReport) -> Result<Vec<Call>, String> {
    let steps = path.as_array().ok_or("the counterexample has no path")?;
    let mut calls = Vec::new();
    for s in steps {
        let event = s["event"].as_str().unwrap_or_default();
        let Some(rest) = event.strip_prefix("call_") else { continue };
        // `call_<model name>#X<k>`, or `call_<model name>` with no argument.
        let (name, region) = match rest.split_once('#') {
            Some((n, r)) => (n, Some(r)),
            None => (rest, None),
        };
        let e = report
            .entrypoints
            .iter()
            .find(|e| e.model_name == name)
            .ok_or_else(|| format!("no entrypoint is modelled as {name:?}"))?;
        check_encodable(e)?;
        let argument = match region {
            None => None,
            Some(r) => {
                let set = region_set(report, r)
                    .ok_or_else(|| format!("the model names a region {r:?} the report does not"))?;
                let w = set
                    .witness()
                    .ok_or_else(|| format!("region {r} is empty, so it has no witness"))?;
                Some(w.to_string())
            }
        };
        calls.push(Call { signature: e.signature.clone(), argument });
    }
    if calls.is_empty() {
        return Err("the path contains no call, so there is nothing to replay".into());
    }
    Ok(calls)
}

/// Refuse an entrypoint whose argument is not one ABI word. The abstraction
/// already restricts the types it models, but the encoder here would pad any
/// value into a word regardless, and wrong calldata makes the run say nothing
/// in either direction.
fn check_encodable(e: &mulu_abstraction::model::EntrypointInfo) -> Result<(), String> {
    match &e.param_type {
        None => Ok(()),
        Some(t) if mulu_replay::is_static_word(t) => Ok(()),
        Some(t) => Err(format!(
            "{}: an argument of type {t} is not one ABI word, and this encoder would pad it \
             into one",
            e.signature
        )),
    }
}

fn region_set(report: &AbstractionReport, name: &str) -> Option<IntervalSet> {
    report.argument_regions.iter().find(|r| r.name == name).map(|r| r.set.clone())
}

/// Which storage slots the specification talks about, so the replay reads
/// exactly what the claim is about.
pub fn spec_slots(layout: &Value, props: &[mulu_abstraction::spec::CompiledProperty]) -> Vec<mulu_replay::U256> {
    let vars = mulu_abstraction::spec::storage_vars(layout);
    let mut out = Vec::new();
    for p in props {
        let Some(name) = &p.var else { continue };
        let Some(v) = vars.iter().find(|v| &v.label == name) else { continue };
        if let Ok(slot) = mulu_abstraction::interval::parse_decimal(&v.slot) {
            let bytes: [u8; 32] = slot.to_be_bytes();
            let s = mulu_replay::U256::from_be_bytes(bytes);
            if !out.contains(&s) {
                out.push(s);
            }
        }
    }
    out
}

/// Run the calls and say whether the specification really broke.
#[allow(clippy::too_many_arguments)]
pub fn run(
    creation_hex: Option<&str>,
    calls: Vec<Call>,
    slots: &[mulu_replay::U256],
    props: &[mulu_abstraction::spec::CompiledProperty],
    layout: &Value,
) -> Reproduction {
    let base = Reproduction {
        status: "unsupported",
        reason: None,
        calls: calls.clone(),
        run: None,
        path: None,
        assumptions: ASSUMPTIONS.to_vec(),
    };
    let Some(hex) = creation_hex.filter(|h| !h.is_empty()) else {
        return Reproduction {
            reason: Some("solc produced no creation bytecode for this contract".into()),
            ..base
        };
    };
    let bytes = match decode_hex(hex) {
        Ok(b) => b,
        Err(e) => return Reproduction { reason: Some(e), ..base },
    };
    let r = match replay(&bytes, &calls, slots) {
        Ok(r) => r,
        Err(e) => return Reproduction { reason: Some(e.to_string()), ..base },
    };

    // Did the specification actually break? A property is evaluated at every
    // successful transaction end, and one that could not be evaluated is
    // reported as such rather than counted as satisfied.
    let outcome = evaluate_spec(&r, props, layout);
    if !outcome.broken.is_empty() {
        return Reproduction {
            status: "reproduced",
            reason: Some(format!("violated: {}", outcome.broken.join(", "))),
            run: Some(r),
            ..base
        };
    }
    if !outcome.unevaluable.is_empty() {
        return Reproduction {
            status: "unsupported",
            reason: Some(format!(
                "the calls ran but the specification could not be evaluated on the result: {}",
                outcome.unevaluable.join("; ")
            )),
            run: Some(r),
            ..base
        };
    }
    Reproduction {
        status: "not-reproduced",
        reason: Some(
            "the calls ran and no property was violated; the model and the EVM disagree".into(),
        ),
        run: Some(r),
        ..base
    }
}

/// What evaluating the specification against a run found.
struct SpecOutcome {
    /// Property ids a successful transaction end violated.
    broken: Vec<String>,
    /// Property ids that could not be evaluated, with why. These are never
    /// counted as satisfied: not knowing is not the same as holding.
    unevaluable: Vec<String>,
}

/// Evaluate every property at every successful transaction end, which is when
/// docs/09 §3 says it is evaluated. Checking only the final state would miss
/// a violation a later call repairs.
fn evaluate_spec(
    r: &Replay,
    props: &[mulu_abstraction::spec::CompiledProperty],
    layout: &Value,
) -> SpecOutcome {
    let vars = mulu_abstraction::spec::storage_vars(layout);
    let mut broken: Vec<String> = Vec::new();
    let mut unevaluable: Vec<String> = Vec::new();

    for p in props {
        // Only one timing exists today, but matching it means a new one is a
        // compile error rather than a property checked at the wrong moment.
        match p.when {
            mulu_abstraction::spec::When::SuccessfulTransactionEnd => {}
        }
        match &p.var {
            None => {
                // A constant property. `false` forbids every successful end.
                if p.predicate == mulu_abstraction::predicate::Predicate::False
                    && r.calls.iter().any(|c| c.success)
                {
                    broken.push(p.id.clone());
                }
            }
            Some(name) => {
                let Some(v) = vars.iter().find(|v| &v.label == name) else {
                    unevaluable.push(format!(
                        "{}: {name} is not in the storage layout of the deployed contract",
                        p.id
                    ));
                    continue;
                };
                let mut violated = false;
                let mut read_any = false;
                let mut unreadable = false;
                for c in r.calls.iter().filter(|c| c.success) {
                    let Some(read) = c.storage.get(&v.slot) else { continue };
                    read_any = true;
                    match mulu_abstraction::interval::parse_decimal(read) {
                        Ok(value) => {
                            if !p.predicate.set().contains(value) {
                                violated = true;
                            }
                        }
                        Err(e) => {
                            unevaluable
                                .push(format!("{}: slot {} read back as {read:?}: {e}", p.id, v.slot));
                            unreadable = true;
                            break;
                        }
                    }
                }
                if unreadable {
                    // already reported; do not report the same property twice
                } else if violated {
                    broken.push(p.id.clone());
                } else if !read_any && r.calls.iter().any(|c| c.success) {
                    unevaluable.push(format!(
                        "{}: slot {} was not read back, so the property was never checked",
                        p.id, v.slot
                    ));
                }
            }
        }
    }
    SpecOutcome { broken, unevaluable }
}

fn decode_hex(text: &str) -> Result<Vec<u8>, String> {
    let t = text.trim().trim_start_matches("0x");
    if t.len() % 2 != 0 {
        return Err("the creation bytecode has an odd number of hex digits".into());
    }
    (0..t.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&t[i..i + 2], 16).map_err(|e| format!("bad bytecode: {e}")))
        .collect()
}

/// Write the record beside the other evidence.
pub fn write_record(out: &std::path::Path, id: &str, rep: &mut Reproduction) -> Result<()> {
    std::fs::create_dir_all(out.join("witnesses"))?;
    let rel = format!("witnesses/{}.json", crate::lean::ident(id));
    std::fs::write(out.join(&rel), serde_json::to_string_pretty(rep)?)
        .with_context(|| format!("writing {rel}"))?;
    rep.path = Some(rel);
    Ok(())
}

/// An implementation state name, `<entrypoint>#<step>_<region>_<storage>`,
/// back to the entrypoint and the argument region. The format is this tool's
/// own; anything that does not match yields `None` rather than a guess.
pub fn split_impl_state(state: &str) -> Option<(&str, Option<&str>)> {
    let (name, rest) = state.split_once('#')?;
    let mut parts = rest.split('_');
    let _step = parts.next()?;
    let region = parts.next().filter(|p| p.starts_with('X'));
    Some((name, region))
}

/// The single call an overrestriction is about: the implementation rejects it
/// and the specification would have permitted it.
pub fn concretise_rejected_call(
    impl_state: &str,
    report: &AbstractionReport,
) -> Result<Vec<Call>, String> {
    let (name, region) = split_impl_state(impl_state)
        .ok_or_else(|| format!("{impl_state:?} is not a state this tool named"))?;
    let e = report
        .entrypoints
        .iter()
        .find(|e| e.model_name == name)
        .ok_or_else(|| format!("no entrypoint is modelled as {name:?}"))?;
    check_encodable(e)?;
    let argument = match region {
        None => None,
        Some(r) => {
            let set = region_set(report, r)
                .ok_or_else(|| format!("the model names a region {r:?} the report does not"))?;
            Some(set.witness().ok_or("the region is empty")?.to_string())
        }
    };
    Ok(vec![Call { signature: e.signature.clone(), argument }])
}

/// Run a call the implementation is expected to reject.
pub fn run_rejection(creation_hex: Option<&str>, calls: Vec<Call>) -> Reproduction {
    let base = Reproduction {
        status: "unsupported",
        reason: None,
        calls: calls.clone(),
        run: None,
        path: None,
        assumptions: ASSUMPTIONS.to_vec(),
    };
    let Some(hex) = creation_hex.filter(|h| !h.is_empty()) else {
        return Reproduction { reason: Some("no creation bytecode".into()), ..base };
    };
    let bytes = match decode_hex(hex) {
        Ok(b) => b,
        Err(e) => return Reproduction { reason: Some(e), ..base },
    };
    match replay(&bytes, &calls, &[]) {
        Ok(r) => {
            let rejected = !r.calls.is_empty() && r.calls.iter().all(|c| !c.success);
            if rejected {
                let why = r.calls[0].revert_reason.clone().unwrap_or_else(|| "reverted".into());
                Reproduction {
                    status: "reproduced",
                    reason: Some(format!("the contract rejects it: {why}")),
                    run: Some(r),
                    ..base
                }
            } else {
                Reproduction {
                    status: "not-reproduced",
                    reason: Some(
                        "the contract accepted the call the model says it rejects; the model \
                         and the EVM disagree"
                            .into(),
                    ),
                    run: Some(r),
                    ..base
                }
            }
        }
        Err(e) => Reproduction { reason: Some(e.to_string()), ..base },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mulu_abstraction::interval::U256;
    use mulu_abstraction::model::{EntrypointInfo, RegionInfo};

    pub(super) fn report() -> AbstractionReport {
        AbstractionReport {
            environment_profile: "p1a-abi-single-v1",
            entrypoints: vec![
                EntrypointInfo {
                    model_name: "forceSet".into(),
                    signature: "forceSet(uint256)".into(),
                    selector: "0x81a9dc5e".into(),
                    param_type: Some("uint256".into()),
                },
                EntrypointInfo {
                    model_name: "limit".into(),
                    signature: "limit()".into(),
                    selector: "0xa4d66daf".into(),
                    param_type: None,
                },
            ],
            entrypoints_modelled: vec![],
            entrypoints_skipped: vec![],
            argument_predicates: vec![],
            storage_predicates: vec![],
            argument_regions: vec![
                RegionInfo {
                    name: "X0".into(),
                    set: IntervalSet::le(U256::from(100u64)),
                    description: String::new(),
                },
                RegionInfo {
                    name: "X2".into(),
                    set: IntervalSet::ge(U256::from(1001u64)),
                    description: String::new(),
                },
            ],
            storage_regions: vec![],
            discharged: vec![],
            assumptions: vec![],
            unsupported: vec![],
        }
    }

    fn path(events: &[&str]) -> Value {
        Value::Array(
            events.iter().map(|e| serde_json::json!({"from": "a", "event": e, "to": "b"})).collect(),
        )
    }

    #[test]
    fn a_call_event_becomes_a_call_with_the_regions_witness() {
        let calls = concretise(&path(&["call_forceSet#X2", "store_limit", "return"]), &report())
            .unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].signature, "forceSet(uint256)");
        // the smallest member of [1001, MAX]
        assert_eq!(calls[0].argument.as_deref(), Some("1001"));
    }

    #[test]
    fn an_entrypoint_with_no_argument_gets_none() {
        let calls = concretise(&path(&["call_limit", "return"]), &report()).unwrap();
        assert_eq!(calls[0].argument, None);
        assert_eq!(calls[0].calldata().unwrap().len(), 4);
    }

    #[test]
    fn several_calls_keep_their_order() {
        let calls =
            concretise(&path(&["call_forceSet#X0", "return", "call_forceSet#X2"]), &report())
                .unwrap();
        assert_eq!(
            calls.iter().map(|c| c.argument.clone().unwrap()).collect::<Vec<_>>(),
            vec!["0", "1001"]
        );
    }

    #[test]
    fn a_path_the_report_cannot_explain_is_refused() {
        // an entrypoint the report does not list
        assert!(concretise(&path(&["call_nothing#X0"]), &report()).is_err());
        // a region the report does not list
        assert!(concretise(&path(&["call_forceSet#X9"]), &report()).is_err());
        // and a path with no call at all
        assert!(concretise(&path(&["store_limit", "return"]), &report()).is_err());
    }

    #[test]
    fn an_implementation_state_splits_into_entrypoint_and_region() {
        assert_eq!(split_impl_state("setLimit#0_X1_LIM0"), Some(("setLimit", Some("X1"))));
        assert_eq!(split_impl_state("limit#2_LIM0"), Some(("limit", None)));
        // a name with an underscore in it survives, because `#` separates
        assert_eq!(split_impl_state("set_uint256#0_X3_LIM1"), Some(("set_uint256", Some("X3"))));
        assert_eq!(split_impl_state("not a state"), None);
    }

    #[test]
    fn a_rejected_call_is_concretised_from_its_state() {
        let mut r = report();
        r.entrypoints[0].model_name = "setLimit".into();
        r.entrypoints[0].signature = "setLimit(uint256)".into();
        let calls = concretise_rejected_call("setLimit#0_X2_LIM0", &r).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].argument.as_deref(), Some("1001"));
        assert!(concretise_rejected_call("setLimit#0_X9_LIM0", &r).is_err());
    }
}

#[cfg(test)]
mod spec_tests {
    use super::*;
    use mulu_abstraction::spec::Spec;
    use mulu_replay::{CallOutcome, Replay};
    use std::collections::BTreeMap;

    fn layout() -> Value {
        serde_json::json!({
            "storage": [{"label": "limit", "offset": 0, "slot": "0", "type": "t_uint256"}],
            "types": {"t_uint256": {"label": "uint256", "numberOfBytes": "32", "encoding": "inplace"}}
        })
    }

    fn props(json: &str) -> Vec<mulu_abstraction::spec::CompiledProperty> {
        Spec::parse(json).unwrap().compile("C", &layout()).unwrap()
    }

    fn bound() -> Vec<mulu_abstraction::spec::CompiledProperty> {
        props(
            r#"{"schema_version":1,"properties":[{"id":"b","contract":"C",
            "when":"successful-transaction-end",
            "assert":{"op":"ule","left":{"storage":"limit"},"right":{"uint256":"1000"}}}]}"#,
        )
    }

    fn run(calls: &[(bool, &str)]) -> Replay {
        Replay {
            address: "0x00".into(),
            evm: "test".into(),
            storage: calls
                .last()
                .map(|(_, v)| BTreeMap::from([("0".to_string(), v.to_string())]))
                .unwrap_or_default(),
            calls: calls
                .iter()
                .map(|(ok, v)| CallOutcome {
                    signature: "f(uint256)".into(),
                    argument: None,
                    success: *ok,
                    revert_reason: None,
                    gas_used: 1,
                    storage: BTreeMap::from([("0".to_string(), v.to_string())]),
                })
                .collect(),
        }
    }

    #[test]
    fn a_violation_at_any_successful_end_counts() {
        // broken, then repaired: the final state is innocent, the run is not
        let o = evaluate_spec(&run(&[(true, "2000"), (true, "10")]), &bound(), &layout());
        assert_eq!(o.broken, vec!["b".to_string()]);
        assert!(o.unevaluable.is_empty());
    }

    #[test]
    fn a_run_that_stays_inside_the_bound_breaks_nothing() {
        let o = evaluate_spec(&run(&[(true, "10"), (true, "1000")]), &bound(), &layout());
        assert!(o.broken.is_empty() && o.unevaluable.is_empty());
    }

    #[test]
    fn a_reverted_call_is_not_a_transaction_end_the_property_is_about() {
        // 2000 sits in storage only in a call that failed
        let o = evaluate_spec(&run(&[(false, "2000")]), &bound(), &layout());
        assert!(o.broken.is_empty(), "a reverted call is not a successful end");
    }

    #[test]
    fn a_property_that_could_not_be_evaluated_is_never_counted_as_satisfied() {
        // the slot was not read back
        let mut r = run(&[(true, "10")]);
        r.calls[0].storage.clear();
        let o = evaluate_spec(&r, &bound(), &layout());
        assert!(o.broken.is_empty());
        assert_eq!(o.unevaluable.len(), 1, "{:?}", o.unevaluable);
        assert!(o.unevaluable[0].contains("never checked"));

        // the variable is not in the layout at all
        let empty = serde_json::json!({"storage": [], "types": {}});
        let o = evaluate_spec(&run(&[(true, "10")]), &bound(), &empty);
        assert_eq!(o.unevaluable.len(), 1);
        assert!(o.unevaluable[0].contains("not in the storage layout"));
    }

    #[test]
    fn a_constant_false_property_is_broken_by_any_success() {
        let p = props(
            r#"{"schema_version":1,"properties":[{"id":"never","contract":"C",
            "when":"successful-transaction-end",
            "assert":{"op":"eq","left":{"uint256":"0"},"right":{"uint256":"1"}}}]}"#,
        );
        assert_eq!(evaluate_spec(&run(&[(true, "0")]), &p, &layout()).broken, vec!["never"]);
        // but a run where nothing succeeds breaks nothing
        assert!(evaluate_spec(&run(&[(false, "0")]), &p, &layout()).broken.is_empty());
    }
}

#[cfg(test)]
mod review_tests {
    use super::*;
    use mulu_abstraction::model::EntrypointInfo;

    fn entry(sig: &str, ty: Option<&str>) -> EntrypointInfo {
        EntrypointInfo {
            model_name: "f".into(),
            signature: sig.into(),
            selector: "0x00000000".into(),
            param_type: ty.map(|s| s.to_string()),
        }
    }

    /// The encoder pads any value into one word. An argument that is not one
    /// word would get calldata that means something else, and the run would
    /// then say nothing in either direction.
    #[test]
    fn an_argument_that_is_not_one_abi_word_is_refused() {
        assert!(check_encodable(&entry("f(uint256)", Some("uint256"))).is_ok());
        assert!(check_encodable(&entry("f(uint8)", Some("uint8"))).is_ok());
        assert!(check_encodable(&entry("f(address)", Some("address"))).is_ok());
        assert!(check_encodable(&entry("f(bool)", Some("bool"))).is_ok());
        assert!(check_encodable(&entry("f()", None)).is_ok());
        for t in ["string", "bytes", "uint256[]", "(uint8,bool)", "int256"] {
            let e = check_encodable(&entry("f(x)", Some(t))).unwrap_err();
            assert!(e.contains("not one ABI word"), "{e}");
        }
    }

    #[test]
    fn concretising_refuses_before_encoding_a_type_it_cannot() {
        let mut r = super::tests::report();
        r.entrypoints[0].param_type = Some("bytes".into());
        let path = serde_json::json!([{"from": "a", "event": "call_forceSet#X2", "to": "b"}]);
        assert!(concretise(&path, &r).is_err());
        assert!(concretise_rejected_call("forceSet#0_X2_S", &r).is_err());
    }

    /// `all()` is vacuously true on an empty sequence, so a run with no calls
    /// took the "rejected" branch and then indexed into nothing.
    #[test]
    fn a_run_with_no_calls_does_not_claim_a_rejection() {
        // no creation code, so the replay never runs: the point is that this
        // returns rather than panicking
        let rep = run_rejection(None, vec![]);
        assert_eq!(rep.status, "unsupported");
        assert!(rep.run.is_none());
    }

    #[test]
    fn a_property_whose_value_will_not_parse_is_reported_once() {
        use mulu_abstraction::spec::Spec;
        use mulu_replay::{CallOutcome, Replay};
        use std::collections::BTreeMap;
        let layout = serde_json::json!({
            "storage": [{"label": "limit", "offset": 0, "slot": "0", "type": "t_uint256"}],
            "types": {"t_uint256": {"label": "uint256", "numberOfBytes": "32", "encoding": "inplace"}}
        });
        let props = Spec::parse(
            r#"{"schema_version":1,"properties":[{"id":"b","contract":"C",
            "when":"successful-transaction-end",
            "assert":{"op":"ule","left":{"storage":"limit"},"right":{"uint256":"1000"}}}]}"#,
        )
        .unwrap()
        .compile("C", &layout)
        .unwrap();
        let r = Replay {
            address: "0x00".into(),
            evm: "test".into(),
            storage: BTreeMap::new(),
            calls: vec![CallOutcome {
                signature: "f()".into(),
                argument: None,
                success: true,
                revert_reason: None,
                gas_used: 1,
                storage: BTreeMap::from([("0".to_string(), "not a number".to_string())]),
            }],
        };
        let o = evaluate_spec(&r, &props, &layout);
        assert!(o.broken.is_empty());
        assert_eq!(o.unevaluable.len(), 1, "reported once, not twice: {:?}", o.unevaluable);
    }
}
