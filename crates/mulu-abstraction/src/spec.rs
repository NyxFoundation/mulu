//! The specification input of docs/09 §3.
//!
//! Names, integers, comparisons and boolean combinations, nothing else. Names
//! are resolved against solc's `storageLayout` and type-checked. An unknown
//! operator or an unresolved name is an error: it is never taken to be true.
//! uint256 literals are decimal strings, because a JSON number cannot hold
//! one exactly.

use crate::interval::{parse_decimal, IntervalSet};
use crate::predicate::Predicate;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Spec {
    pub schema_version: u32,
    #[serde(default)]
    pub properties: Vec<Property>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum When {
    /// Evaluated when a transaction ends successfully, not at every step.
    SuccessfulTransactionEnd,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Property {
    pub id: String,
    pub contract: String,
    pub when: When,
    #[serde(rename = "assert")]
    pub assertion: Assertion,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Assertion {
    /// Unsigned <=
    Ule { left: Operand, right: Operand },
    /// Unsigned <
    Ult { left: Operand, right: Operand },
    /// Unsigned >=
    Uge { left: Operand, right: Operand },
    /// Unsigned >
    Ugt { left: Operand, right: Operand },
    Eq { left: Operand, right: Operand },
    Ne { left: Operand, right: Operand },
    And { args: Vec<Assertion> },
    Or { args: Vec<Assertion> },
    Not { arg: Box<Assertion> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", deny_unknown_fields)]
pub enum Operand {
    /// A storage variable, by the label solc records in storageLayout.
    Storage(String),
    /// A literal, written in decimal (or 0x-prefixed hex).
    Uint256(String),
}

#[derive(Debug, Error)]
pub enum SpecError {
    #[error("unsupported spec schema_version {0} (expected {SCHEMA_VERSION})")]
    SchemaVersion(u32),
    #[error("property {id:?}: {message}")]
    Property { id: String, message: String },
    #[error("spec json: {0}")]
    Json(#[from] serde_json::Error),
}

/// A storage variable as solc describes it, with its type resolved through
/// the layout's `types` table.
#[derive(Debug, Clone)]
pub struct StorageVar {
    pub label: String,
    pub slot: String,
    pub offset: u64,
    pub type_id: String,
    /// The Solidity type name, e.g. `uint8`.
    pub type_label: Option<String>,
    /// How many of the slot's 32 bytes it occupies.
    pub bytes: Option<u64>,
}

impl StorageVar {
    /// True when the variable is the only thing in its slot and fills it.
    pub fn whole_slot(&self) -> bool {
        self.offset == 0 && self.bytes == Some(32)
    }
}

/// Read solc's `storageLayout` output.
pub fn storage_vars(layout: &serde_json::Value) -> Vec<StorageVar> {
    let types = &layout["types"];
    layout["storage"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|v| {
                    let type_id = v["type"].as_str()?.to_string();
                    Some(StorageVar {
                        label: v["label"].as_str()?.to_string(),
                        slot: v["slot"].as_str()?.to_string(),
                        offset: v["offset"].as_u64().unwrap_or(0),
                        type_label: crate::types::label_of_type_id(&type_id, types),
                        bytes: crate::types::bytes_of_type_id(&type_id, types),
                        type_id,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// A property reduced to a predicate over one storage variable.
#[derive(Debug, Clone)]
pub struct CompiledProperty {
    pub id: String,
    pub when: When,
    /// Holds exactly when the property holds.
    pub predicate: Predicate,
    /// The storage variable it constrains; `None` for a constant property.
    pub var: Option<String>,
    /// How it reads, for reports.
    pub text: String,
}

fn operand_side(o: &Operand, vars: &[StorageVar]) -> Result<Side, String> {
    match o {
        Operand::Storage(name) => {
            let v = vars
                .iter()
                .find(|v| &v.label == name)
                .ok_or_else(|| format!("no storage variable named {name:?} in this contract"))?;
            // The type must be one P1a can give a value domain to, and the
            // variable must own its slot: a packed one is written by a masked
            // merge, not by a whole-slot store.
            let label = v.type_label.clone().unwrap_or_else(|| v.type_id.clone());
            crate::types::domain_of(&label)
                .map_err(|e| format!("storage variable {name:?}: {e}"))?;
            if !v.whole_slot() {
                return Err(format!(
                    "storage variable {name:?} ({label}) shares slot {} at offset {}; \
                     P1a models variables that own a whole slot",
                    v.slot, v.offset
                ));
            }
            Ok(Side::Var(name.clone()))
        }
        Operand::Uint256(text) => parse_decimal(text).map(Side::Lit).map_err(|e| e.to_string()),
    }
}

enum Side {
    Var(String),
    Lit(crate::interval::U256),
}

fn compile_assertion(a: &Assertion, vars: &[StorageVar]) -> Result<Predicate, String> {
    use Assertion::*;
    let cmp = |left: &Operand,
               right: &Operand,
               on_left: fn(crate::interval::U256) -> IntervalSet,
               on_right: fn(crate::interval::U256) -> IntervalSet|
     -> Result<Predicate, String> {
        match (operand_side(left, vars)?, operand_side(right, vars)?) {
            (Side::Var(v), Side::Lit(k)) => Ok(predicate_over(v, on_left(k))),
            (Side::Lit(k), Side::Var(v)) => Ok(predicate_over(v, on_right(k))),
            (Side::Lit(a), Side::Lit(b)) => {
                Ok(if on_left(b).contains(a) { Predicate::True } else { Predicate::False })
            }
            (Side::Var(_), Side::Var(_)) => {
                Err("a comparison of two storage variables is outside the P1a fragment".into())
            }
        }
    };
    match a {
        Ule { left, right } => cmp(left, right, IntervalSet::le, IntervalSet::ge),
        Ult { left, right } => cmp(left, right, IntervalSet::lt, IntervalSet::gt),
        Uge { left, right } => cmp(left, right, IntervalSet::ge, IntervalSet::le),
        Ugt { left, right } => cmp(left, right, IntervalSet::gt, IntervalSet::lt),
        Eq { left, right } => cmp(left, right, IntervalSet::eq_to, IntervalSet::eq_to),
        Ne { left, right } => cmp(left, right, IntervalSet::ne_to, IntervalSet::ne_to),
        And { args } => {
            let mut acc = Predicate::True;
            for a in args {
                acc = acc.and(&compile_assertion(a, vars)?)?;
            }
            Ok(acc)
        }
        Or { args } => {
            let mut acc = Predicate::False;
            for a in args {
                acc = acc.or(&compile_assertion(a, vars)?)?;
            }
            Ok(acc)
        }
        Not { arg } => Ok(compile_assertion(arg, vars)?.negate()),
    }
}

fn predicate_over(var: String, set: IntervalSet) -> Predicate {
    if set.is_empty() {
        Predicate::False
    } else if set.is_full() {
        Predicate::True
    } else {
        Predicate::Over { var, set }
    }
}

fn render(a: &Assertion) -> String {
    use Assertion::*;
    let side = |o: &Operand| match o {
        Operand::Storage(s) => s.clone(),
        Operand::Uint256(v) => v.clone(),
    };
    match a {
        Ule { left, right } => format!("{} <= {}", side(left), side(right)),
        Ult { left, right } => format!("{} < {}", side(left), side(right)),
        Uge { left, right } => format!("{} >= {}", side(left), side(right)),
        Ugt { left, right } => format!("{} > {}", side(left), side(right)),
        Eq { left, right } => format!("{} == {}", side(left), side(right)),
        Ne { left, right } => format!("{} != {}", side(left), side(right)),
        And { args } => format!("({})", args.iter().map(render).collect::<Vec<_>>().join(" and ")),
        Or { args } => format!("({})", args.iter().map(render).collect::<Vec<_>>().join(" or ")),
        Not { arg } => format!("not {}", render(arg)),
    }
}

impl Spec {
    pub fn parse(text: &str) -> Result<Spec, SpecError> {
        let s: Spec = serde_json::from_str(text)?;
        if s.schema_version != SCHEMA_VERSION {
            return Err(SpecError::SchemaVersion(s.schema_version));
        }
        Ok(s)
    }

    /// Compile the properties that apply to `contract`.
    pub fn compile(
        &self,
        contract: &str,
        layout: &serde_json::Value,
    ) -> Result<Vec<CompiledProperty>, SpecError> {
        let vars = storage_vars(layout);
        let mut out = Vec::new();
        for p in self.properties.iter().filter(|p| p.contract == contract) {
            let predicate = compile_assertion(&p.assertion, &vars)
                .map_err(|message| SpecError::Property { id: p.id.clone(), message })?;
            out.push(CompiledProperty {
                id: p.id.clone(),
                when: p.when,
                var: predicate.var().map(|s| s.to_string()),
                text: render(&p.assertion),
                predicate,
            });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shaped like real solc output, `numberOfBytes` included: without it
    /// nothing can tell a packed variable from one that owns its slot.
    fn layout() -> serde_json::Value {
        serde_json::json!({
            "storage": [
                {"astId": 3, "contract": "Limits.sol:Limits", "label": "limit",
                 "offset": 0, "slot": "0", "type": "t_uint256"},
                {"astId": 5, "contract": "Limits.sol:Limits", "label": "owner",
                 "offset": 0, "slot": "1", "type": "t_address"},
                {"astId": 7, "contract": "Limits.sol:Limits", "label": "packed",
                 "offset": 20, "slot": "1", "type": "t_uint8"}
            ],
            "types": {
                "t_uint256": {"label": "uint256", "numberOfBytes": "32", "encoding": "inplace"},
                "t_address": {"label": "address", "numberOfBytes": "20", "encoding": "inplace"},
                "t_uint8": {"label": "uint8", "numberOfBytes": "1", "encoding": "inplace"}
            }
        })
    }

    const LIMITS_SPEC: &str = include_str!("../../../examples/limits/Limits.spec.json");

    #[test]
    fn the_limits_property_compiles_to_an_interval() {
        let spec = Spec::parse(LIMITS_SPEC).unwrap();
        let c = spec.compile("Limits", &layout()).unwrap();
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].id, "limit-bound");
        assert_eq!(c[0].when, When::SuccessfulTransactionEnd);
        assert_eq!(c[0].var.as_deref(), Some("limit"));
        assert_eq!(c[0].text, "limit <= 1000");
        assert_eq!(
            c[0].predicate.set(),
            IntervalSet::le(crate::interval::U256::from(1000u64))
        );
    }

    #[test]
    fn properties_for_another_contract_are_skipped() {
        let spec = Spec::parse(LIMITS_SPEC).unwrap();
        assert!(spec.compile("SomethingElse", &layout()).unwrap().is_empty());
    }

    #[test]
    fn an_empty_spec_is_valid_and_means_redundancy_only() {
        let s = Spec::parse(r#"{"schema_version": 1, "properties": []}"#).unwrap();
        assert!(s.compile("Limits", &layout()).unwrap().is_empty());
        let s = Spec::parse(r#"{"schema_version": 1}"#).unwrap();
        assert!(s.properties.is_empty());
    }

    #[test]
    fn unknown_names_types_and_operators_are_errors_not_true() {
        let cases = [
            // unresolved storage name
            r#"{"schema_version":1,"properties":[{"id":"p","contract":"Limits","when":"successful-transaction-end","assert":{"op":"ule","left":{"storage":"nope"},"right":{"uint256":"1"}}}]}"#,
            // a variable that does not own its slot
            r#"{"schema_version":1,"properties":[{"id":"p","contract":"Limits","when":"successful-transaction-end","assert":{"op":"ule","left":{"storage":"packed"},"right":{"uint256":"1"}}}]}"#,
            // two storage variables
            r#"{"schema_version":1,"properties":[{"id":"p","contract":"Limits","when":"successful-transaction-end","assert":{"op":"ule","left":{"storage":"limit"},"right":{"storage":"limit"}}}]}"#,
        ];
        for c in cases {
            let spec = Spec::parse(c).unwrap();
            assert!(spec.compile("Limits", &layout()).is_err(), "{c}");
        }
        // an unknown operator does not even parse
        assert!(Spec::parse(
            r#"{"schema_version":1,"properties":[{"id":"p","contract":"Limits","when":"successful-transaction-end","assert":{"op":"approximately","left":{"storage":"limit"},"right":{"uint256":"1"}}}]}"#
        )
        .is_err());
        // and neither does an unknown schema version or a stray field
        assert!(Spec::parse(r#"{"schema_version": 2, "properties": []}"#).is_err());
        assert!(Spec::parse(r#"{"schema_version": 1, "properties": [], "extra": 1}"#).is_err());
    }

    #[test]
    fn a_variable_that_owns_its_slot_may_be_narrower_than_a_word() {
        // `owner` is an address: 20 bytes at offset 0 of slot 1, so it does
        // not own the slot and P1a refuses it, while `limit` is fine.
        let vars = storage_vars(&layout());
        let limit = vars.iter().find(|v| v.label == "limit").unwrap();
        assert_eq!(limit.type_label.as_deref(), Some("uint256"));
        assert_eq!(limit.bytes, Some(32));
        assert!(limit.whole_slot());

        let owner = vars.iter().find(|v| v.label == "owner").unwrap();
        assert_eq!(owner.bytes, Some(20));
        assert!(!owner.whole_slot(), "20 bytes leaves room for something else");

        let packed = vars.iter().find(|v| v.label == "packed").unwrap();
        assert_eq!(packed.offset, 20);
        assert!(!packed.whole_slot());
    }

    #[test]
    fn literals_keep_full_uint256_precision() {
        let big = "115792089237316195423570985008687907853269984665640564039457584007913129639935";
        let json = format!(
            r#"{{"schema_version":1,"properties":[{{"id":"p","contract":"Limits","when":"successful-transaction-end","assert":{{"op":"ult","left":{{"storage":"limit"}},"right":{{"uint256":"{big}"}}}}}}]}}"#
        );
        let c = Spec::parse(&json).unwrap().compile("Limits", &layout()).unwrap();
        // limit < MAX excludes exactly one value
        assert_eq!(c[0].predicate.negate().set().count(), Some(1));
    }

    #[test]
    fn boolean_combinations_compile() {
        let json = r#"{"schema_version":1,"properties":[{"id":"band","contract":"Limits","when":"successful-transaction-end",
            "assert":{"op":"and","args":[
                {"op":"uge","left":{"storage":"limit"},"right":{"uint256":"10"}},
                {"op":"ule","left":{"storage":"limit"},"right":{"uint256":"20"}}]}}]}"#;
        let c = Spec::parse(json).unwrap().compile("Limits", &layout()).unwrap();
        assert_eq!(c[0].predicate.set().count(), Some(11));
        assert_eq!(c[0].text, "(limit >= 10 and limit <= 20)");
    }
}
