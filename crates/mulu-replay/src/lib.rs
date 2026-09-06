//! `mulu-replay` — run a concrete counterexample on a local EVM.
//!
//! docs/08 §5 replays only counterexamples, and docs/09 §5 keeps `proven` and
//! `reproduced` apart: a model proof and an execution are different evidence
//! and neither stands in for the other. This crate produces the second kind.
//!
//! The EVM runs in-process, so a replay is deterministic and needs no node.
//! It deploys the contract's own creation code and sends the calls; nothing
//! is stubbed, and the storage read back afterwards is what the EVM wrote.

use revm::context::result::{ExecutionResult, Output};
use revm::context::TxEnv;
use revm::database::InMemoryDB;
use revm::primitives::{keccak256, Address, Bytes, TxKind};
pub use revm::primitives::U256;
use revm::context_interface::JournalTr;
use revm::{Context, DatabaseRef, ExecuteCommitEvm, MainBuilder, MainContext};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

/// Generous for the contracts P1a models, and below the protocol's per
/// transaction cap, which a larger value trips before the code even runs.
const GAS_LIMIT: u64 = 10_000_000;

#[derive(Debug, Error)]
pub enum ReplayError {
    #[error("deploying the contract failed: {0}")]
    Deploy(String),
    #[error("the EVM rejected the transaction: {0}")]
    Evm(String),
    #[error("the creation code is empty; the contract may be abstract")]
    NoCode,
    #[error("{0}")]
    Encoding(String),
}

/// One concrete call: an entrypoint and, in the P1a subset, at most one
/// 32-byte argument.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Call {
    /// `forceSet(uint256)`, used both to derive the selector and to report.
    pub signature: String,
    /// Decimal, so a uint256 never passes through a JSON number.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub argument: Option<String>,
}

/// The four-byte selector of a signature.
pub fn selector_of(signature: &str) -> [u8; 4] {
    let h = keccak256(signature.as_bytes());
    [h[0], h[1], h[2], h[3]]
}

fn parse_u256(text: &str) -> Result<U256, ReplayError> {
    let t = text.trim();
    let r = if let Some(h) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        U256::from_str_radix(h, 16)
    } else {
        U256::from_str_radix(t, 10)
    };
    r.map_err(|e| ReplayError::Encoding(format!("{text:?} is not a uint256: {e}")))
}

impl Call {
    /// Selector followed by the argument, left-padded to a word. That is the
    /// ABI encoding for the value types P1a models; anything else is refused
    /// rather than encoded wrongly.
    pub fn calldata(&self) -> Result<Vec<u8>, ReplayError> {
        let mut out = selector_of(&self.signature).to_vec();
        if let Some(a) = &self.argument {
            let v = parse_u256(a)?;
            out.extend_from_slice(&v.to_be_bytes::<32>());
        }
        Ok(out)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallOutcome {
    pub signature: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub argument: Option<String>,
    pub success: bool,
    /// The revert reason, decoded from `Error(string)` where it is one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revert_reason: Option<String>,
    pub gas_used: u64,
    /// The requested slots as they stood when this call ended. A property is
    /// evaluated at every successful transaction end, not only the last, so
    /// a violation that a later call repairs still has to be visible.
    pub storage: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Replay {
    pub address: String,
    pub calls: Vec<CallOutcome>,
    /// Slot to value, both decimal.
    pub storage: BTreeMap<String, String>,
    /// The EVM this ran on, so a result can be tied to it.
    pub evm: String,
}

/// Decode `Error(string)` revert data, which is what `require` with a message
/// produces. Anything else is reported as raw bytes rather than guessed at.
fn revert_reason(data: &[u8]) -> Option<String> {
    if data.is_empty() {
        return None;
    }
    const ERROR_SELECTOR: [u8; 4] = [0x08, 0xc3, 0x79, 0xa0];
    if data.len() >= 4 && data[..4] == ERROR_SELECTOR && data.len() >= 68 {
        let len = U256::from_be_slice(&data[36..68]);
        let len: usize = len.try_into().ok()?;
        let start = 68usize;
        let end = start.checked_add(len)?;
        if end <= data.len() {
            return String::from_utf8(data[start..end].to_vec()).ok();
        }
    }
    Some(format!("0x{}", hex_of(data)))
}

fn hex_of(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Read the requested slots of `address` from the live database.
fn read_slots(
    db: &InMemoryDB,
    address: Address,
    slots: &[U256],
) -> Result<BTreeMap<String, String>, ReplayError> {
    let mut out = BTreeMap::new();
    for slot in slots {
        let v = db
            .storage_ref(address, *slot)
            .map_err(|e| ReplayError::Evm(format!("reading storage: {e:?}")))?;
        out.insert(slot.to_string(), v.to_string());
    }
    Ok(out)
}

/// Deploy `creation` and send `calls` in order, then read `slots`.
pub fn replay(creation: &[u8], calls: &[Call], slots: &[U256]) -> Result<Replay, ReplayError> {
    if creation.is_empty() {
        return Err(ReplayError::NoCode);
    }
    let mut evm = Context::mainnet().with_db(InMemoryDB::default()).build_mainnet();
    let caller = Address::from([0x11u8; 20]);

    let deploy = TxEnv {
        caller,
        kind: TxKind::Create,
        data: Bytes::copy_from_slice(creation),
        gas_limit: GAS_LIMIT,
        value: U256::ZERO,
        nonce: 0,
        ..Default::default()
    };
    let result = evm.transact_commit(deploy).map_err(|e| ReplayError::Evm(format!("{e:?}")))?;
    let address = match result {
        ExecutionResult::Success { output: Output::Create(_, Some(a)), .. } => a,
        ExecutionResult::Success { .. } => {
            return Err(ReplayError::Deploy("no address was created".into()))
        }
        ExecutionResult::Revert { output, .. } => {
            return Err(ReplayError::Deploy(
                revert_reason(&output).unwrap_or_else(|| "reverted".into()),
            ))
        }
        ExecutionResult::Halt { reason, .. } => {
            return Err(ReplayError::Deploy(format!("halted: {reason:?}")))
        }
    };

    let mut outcomes = Vec::new();
    for (i, c) in calls.iter().enumerate() {
        let tx = TxEnv {
            caller,
            kind: TxKind::Call(address),
            data: Bytes::from(c.calldata()?),
            gas_limit: GAS_LIMIT,
            value: U256::ZERO,
            nonce: (i + 1) as u64,
            ..Default::default()
        };
        let r = evm.transact_commit(tx).map_err(|e| ReplayError::Evm(format!("{e:?}")))?;
        let (success, reason, gas) = match r {
            ExecutionResult::Success { gas, .. } => (true, None, gas.spent()),
            ExecutionResult::Revert { output, gas, .. } => {
                (false, revert_reason(&output), gas.spent())
            }
            ExecutionResult::Halt { reason, gas, .. } => {
                (false, Some(format!("halted: {reason:?}")), gas.spent())
            }
        };
        let snapshot = read_slots(evm.ctx.journaled_state.db(), address, slots)?;
        outcomes.push(CallOutcome {
            signature: c.signature.clone(),
            argument: c.argument.clone(),
            success,
            revert_reason: reason,
            gas_used: gas,
            storage: snapshot,
        });
    }

    let storage = read_slots(evm.ctx.journaled_state.db(), address, slots)?;

    Ok(Replay {
        address: format!("0x{}", hex_of(address.as_slice())),
        calls: outcomes,
        storage,
        evm: format!("revm {}", env!("CARGO_PKG_VERSION")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selectors_match_the_known_values() {
        // solc reports these for examples/limits
        assert_eq!(hex_of(&selector_of("setLimit(uint256)")), "27ea6f2b");
        assert_eq!(hex_of(&selector_of("forceSet(uint256)")), "81a9dc5e");
        assert_eq!(hex_of(&selector_of("limit()")), "a4d66daf");
    }

    #[test]
    fn calldata_is_the_selector_then_a_padded_word() {
        let c = Call { signature: "forceSet(uint256)".into(), argument: Some("1001".into()) };
        let d = c.calldata().unwrap();
        assert_eq!(d.len(), 36);
        assert_eq!(hex_of(&d[..4]), "81a9dc5e");
        assert_eq!(U256::from_be_slice(&d[4..]), U256::from(1001u64));
        // a no-argument call is the selector alone
        let g = Call { signature: "limit()".into(), argument: None };
        assert_eq!(g.calldata().unwrap().len(), 4);
    }

    #[test]
    fn a_non_numeric_argument_is_refused() {
        let c = Call { signature: "f(uint256)".into(), argument: Some("banana".into()) };
        assert!(c.calldata().is_err());
    }

    #[test]
    fn an_error_string_revert_is_decoded() {
        // abi.encodeWithSignature("Error(string)", "cap")
        let mut d = vec![0x08, 0xc3, 0x79, 0xa0];
        d.extend_from_slice(&U256::from(32u64).to_be_bytes::<32>());
        d.extend_from_slice(&U256::from(3u64).to_be_bytes::<32>());
        let mut word = [0u8; 32];
        word[..3].copy_from_slice(b"cap");
        d.extend_from_slice(&word);
        assert_eq!(revert_reason(&d).as_deref(), Some("cap"));
        // a panic or a custom error is reported raw, not guessed at
        assert!(revert_reason(&[0x4e, 0x48, 0x7b, 0x71]).unwrap().starts_with("0x4e487b71"));
        assert_eq!(revert_reason(&[]), None);
    }

    #[test]
    fn empty_creation_code_is_refused() {
        assert!(matches!(replay(&[], &[], &[]), Err(ReplayError::NoCode)));
    }
}
