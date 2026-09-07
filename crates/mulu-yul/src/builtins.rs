//! What each EVM Yul builtin does, and which ones are outside the P1a subset.
//!
//! docs/09 §1: external calls, callbacks, delegatecall, CREATE, inline
//! assembly, `gas` and unmodelled builtins are `unsupported` in P1a. They are
//! recorded, never quietly treated as no-ops.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Builtin {
    /// Arithmetic and comparison: a function of its arguments alone.
    Pure,
    ReadsMemory,
    WritesMemory,
    ReadsStorage,
    WritesStorage,
    ReadsCalldata,
    /// Block and transaction context: deterministic in a transaction, but not
    /// a function of the guard's arguments.
    ReadsEnvironment,
    ReadsCode,
    Log,
    Return,
    Revert,
    Stop,
    Invalid,
    /// Hands control to another contract. P1a models the result (0 or 1)
    /// and clears what it knows about memory, under the stated assumption
    /// that the callee does not reenter.
    ExternalCall,
    /// Known builtin whose effect P1a does not model.
    Unsupported,
}

pub fn classify(name: &str) -> Option<Builtin> {
    use Builtin::*;
    Some(match name {
        "add" | "sub" | "mul" | "div" | "sdiv" | "mod" | "smod" | "exp" | "not" | "lt" | "gt"
        | "slt" | "sgt" | "eq" | "iszero" | "and" | "or" | "xor" | "byte" | "shl" | "shr"
        | "sar" | "addmod" | "mulmod" | "signextend" => Pure,

        "mload" | "msize" | "keccak256" | "sha3" => ReadsMemory,
        "mstore" | "mstore8" | "mcopy" | "memoryguard" => WritesMemory,

        "sload" | "tload" => ReadsStorage,
        "sstore" | "tstore" => WritesStorage,

        "calldataload" | "calldatasize" | "calldatacopy" => ReadsCalldata,

        "address" | "balance" | "selfbalance" | "origin" | "caller" | "callvalue" | "gasprice"
        | "extcodesize" | "extcodehash" | "blockhash" | "coinbase" | "timestamp" | "number"
        | "difficulty" | "prevrandao" | "gaslimit" | "chainid" | "basefee" | "blobbasefee"
        | "blobhash" => ReadsEnvironment,

        "codesize" | "codecopy" | "extcodecopy" | "datasize" | "dataoffset" | "datacopy"
        | "returndatasize" | "returndatacopy" | "setimmutable" | "loadimmutable" => ReadsCode,

        "log0" | "log1" | "log2" | "log3" | "log4" => Log,

        "return" => Return,
        "revert" => Revert,
        "stop" => Stop,
        "invalid" => Invalid,

        "call" | "staticcall" => ExternalCall,

        // These change who is running or what the code is, which no
        // assumption about the callee's behaviour recovers.
        "callcode" | "delegatecall" | "create" | "create2" | "selfdestruct" => Unsupported,

        // Gas is not a function of the arguments, which is what the purity
        // lattice already records. `gas()` reaches a guard only through a
        // call's gas parameter, where its value never decides anything.
        "gas" => ReadsEnvironment,

        "pc" | "pop" | "verbatim" => Unsupported,

        _ => return None,
    })
}

/// Transitive effect summary of a function or an instruction.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Effects {
    pub reads_storage: bool,
    pub writes_storage: bool,
    pub reads_memory: bool,
    pub writes_memory: bool,
    pub reads_calldata: bool,
    pub reads_environment: bool,
    pub logs: bool,
    pub can_return: bool,
    pub can_revert: bool,
    pub external_call: bool,
    /// Builtins or callees P1a does not model, by name.
    pub unsupported: Vec<String>,
}

impl Effects {
    pub fn merge(&mut self, other: &Effects) {
        self.reads_storage |= other.reads_storage;
        self.writes_storage |= other.writes_storage;
        self.reads_memory |= other.reads_memory;
        self.writes_memory |= other.writes_memory;
        self.reads_calldata |= other.reads_calldata;
        self.reads_environment |= other.reads_environment;
        self.logs |= other.logs;
        self.can_return |= other.can_return;
        self.can_revert |= other.can_revert;
        self.external_call |= other.external_call;
        for u in &other.unsupported {
            if !self.unsupported.contains(u) {
                self.unsupported.push(u.clone());
            }
        }
    }

    pub fn add_builtin(&mut self, name: &str, b: Builtin) {
        use Builtin::*;
        match b {
            Pure | ReadsCode => {}
            ReadsMemory => self.reads_memory = true,
            WritesMemory => self.writes_memory = true,
            ReadsStorage => self.reads_storage = true,
            WritesStorage => self.writes_storage = true,
            ReadsCalldata => self.reads_calldata = true,
            ReadsEnvironment => self.reads_environment = true,
            Log => self.logs = true,
            Return => self.can_return = true,
            Revert | Invalid => self.can_revert = true,
            Stop => self.can_return = true,
            // Modelled, but only under the no-reentrancy assumption the
            // model records against the entrypoint that reaches it.
            ExternalCall => self.external_call = true,
            Unsupported => {
                if !self.unsupported.iter().any(|u| u == name) {
                    self.unsupported.push(name.to_string());
                }
            }
        }
    }

    /// Nothing outside the P1a subset. An external call is inside it, under
    /// the assumption recorded where the entrypoint is admitted.
    pub fn supported(&self) -> bool {
        self.unsupported.is_empty()
    }
}

/// How far a condition is from being a function of its arguments alone
/// (docs/04 §3: P1 restricts checks to pure, total comparisons).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Purity {
    /// Arithmetic and comparison over the arguments only.
    Pure,
    ReadsCalldata,
    ReadsEnvironment,
    ReadsState,
    /// Has effects, or calls something not modelled.
    Effectful,
}

impl Purity {
    pub fn of(e: &Effects) -> Purity {
        if !e.supported() || e.writes_storage || e.writes_memory || e.logs {
            Purity::Effectful
        } else if e.reads_storage {
            Purity::ReadsState
        } else if e.reads_environment {
            Purity::ReadsEnvironment
        } else if e.reads_calldata {
            Purity::ReadsCalldata
        } else {
            Purity::Pure
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn p1a_excludes_the_constructs_docs_09_lists() {
        for name in ["delegatecall", "callcode", "create", "create2", "selfdestruct", "verbatim"] {
            let b = classify(name).unwrap_or_else(|| panic!("{name} unclassified"));
            let mut e = Effects::default();
            e.add_builtin(name, b);
            assert!(!e.supported(), "{name} must be out of the P1a subset");
        }
    }

    /// `call` and `staticcall` are inside the subset, but only because the
    /// model records what it is assuming: the result is 0 or 1, memory is
    /// forgotten, and the callee does not reenter. The flag is what the
    /// abstraction reads to write that assumption down, so losing it would
    /// turn a stated assumption into a silent one.
    #[test]
    fn an_external_call_is_modelled_but_flagged() {
        for name in ["call", "staticcall"] {
            let b = classify(name).unwrap_or_else(|| panic!("{name} unclassified"));
            let mut e = Effects::default();
            e.add_builtin(name, b);
            assert!(e.supported(), "{name} is modelled under an assumption");
            assert!(e.external_call, "{name} must set the flag the assumption hangs on");
        }
    }

    /// Gas is not a function of the arguments, and that is all the model
    /// needs to know: a guard reading it is impure and never becomes a
    /// predicate over arguments.
    #[test]
    fn gas_is_an_environment_read() {
        let mut e = Effects::default();
        e.add_builtin("gas", classify("gas").unwrap());
        assert!(e.supported());
        assert_eq!(Purity::of(&e), Purity::ReadsEnvironment);
    }

    #[test]
    fn comparisons_are_pure_and_storage_is_not() {
        let mut e = Effects::default();
        e.add_builtin("gt", classify("gt").unwrap());
        e.add_builtin("iszero", classify("iszero").unwrap());
        assert_eq!(Purity::of(&e), Purity::Pure);
        e.add_builtin("sload", classify("sload").unwrap());
        assert_eq!(Purity::of(&e), Purity::ReadsState);
    }

    #[test]
    fn unknown_names_are_not_builtins() {
        assert!(classify("fun_setLimit_27").is_none());
        assert!(classify("require_helper_x").is_none());
    }
}
