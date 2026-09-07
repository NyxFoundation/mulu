//! `mulu semantics-diff` — run the rendered contract in Lean and in an EVM.
//!
//! `semantics:rendering-preserves-the-program` says the module mulu renders is
//! the same program as the Yul it came from, and nothing proves it. This is
//! the cheap half of the answer. The same calls go to EvmYul's interpreter and
//! to revm, and the storage they leave is compared.
//!
//! Agreement is not a proof and never becomes one; the ledger keeps the
//! obligation open either way. Divergence is a defect, and it is one of three:
//! the renderer wrote a different program, revm and EvmYul disagree about the
//! EVM, or the analysis is reading the wrong artifact. All three are worth
//! finding before they are inside a claim.
//!
//! The Lean side starts from the storage the deployment left, taken from the
//! EVM. That is deliberate: it isolates the step correspondence from
//! `simulation:initial-covered`, which is a separate obligation, so a
//! constructor this cannot model does not show up as a step disagreement.

use anyhow::{anyhow, bail, Context, Result};
use mulu_replay::Call;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::fmt::Write as _;
use std::process::Command;

/// revm sends from an address of twenty `0x11` bytes (`mulu_replay::replay`).
/// The Lean side has to send from the same one.
const CALLER_HEX: &str = "1111111111111111111111111111111111111111";

pub struct DiffArgs {
    pub sources: Vec<PathBuf>,
    pub contract: Option<String>,
    pub calls: Vec<String>,
    pub solc: Option<PathBuf>,
    pub evm_version: String,
    /// The `semantics/` package. `mulu-diff` is built there.
    pub semantics_dir: PathBuf,
}

/// `setLimit(uint256)=101`, or `limit()` for a call with no argument.
fn parse_call(s: &str) -> Result<Call> {
    let (sig, arg) = match s.split_once('=') {
        Some((sig, arg)) => (sig.trim(), Some(arg.trim().to_string())),
        None => (s.trim(), None),
    };
    if !sig.contains('(') || !sig.ends_with(')') {
        bail!("{s:?} is not a call: write it as `setLimit(uint256)=101`");
    }
    if let Some(a) = &arg {
        a.parse::<u128>().with_context(|| format!("the argument of {sig} must be a number"))?;
    }
    Ok(Call { signature: sig.to_string(), argument: arg })
}

/// Slot numbers the contract's storage layout names, so the comparison covers
/// what the contract actually has rather than a guess at what it uses.
fn slots(layout: &serde_json::Value) -> Vec<u64> {
    layout["storage"]
        .as_array()
        .map(|a| a.iter().filter_map(|e| e["slot"].as_str()?.parse().ok()).collect())
        .unwrap_or_default()
}

fn generated_module(
    contract_def: &str,
    initial: &BTreeMap<u64, String>,
    calls: &[Call],
) -> Result<String> {
    let mut out = String::from(mulu_yul::lean::HEADER);
    out.push_str(
        "import EvmYul.Yul.Interpreter\nimport EvmYul.Yul.YulNotation\nimport MuluDiff.Runner\n\n\
         namespace MuluDiff.Generated\n\nopen EvmYul EvmYul.Yul EvmYul.Yul.Ast\n\n",
    );
    out.push_str(contract_def);
    let _ = writeln!(
        out,
        "\n/-- The address the EVM side sends from. `caller()` reads it, so a contract with\n\
         access control would otherwise diverge for a reason that is not the rendering. -/\n\
         def caller : Nat := 0x{}\n",
        CALLER_HEX
    );
    out.push_str("\n/-- The storage the deployment left, read from the EVM. Starting the two\nrunners from the same place is what makes a later difference mean something. -/\ndef initialStorage : List (Nat × Nat) :=\n  [");
    let cells: Vec<String> = initial.iter().map(|(k, v)| format!("({k}, {v})")).collect();
    out.push_str(&cells.join(", "));
    out.push_str("]\n\n/-- One calldata per transaction, in order. -/\ndef scenario : List ByteArray :=\n  [");
    let mut items = vec![];
    for c in calls {
        let sel = mulu_replay::selector_of(&c.signature);
        let sel = sel.iter().map(|b| format!("0x{b:02x}")).collect::<Vec<_>>().join(", ");
        let arg = match &c.argument {
            Some(a) => format!("[{a}]"),
            None => "[]".to_string(),
        };
        items.push(format!("calldataOf [{sel}] {arg}"));
    }
    out.push_str(&items.join("\n  , "));
    out.push_str("]\n\nend MuluDiff.Generated\n");
    Ok(out)
}

/// `0 ok 0=50` -> `(0, "ok", {0: "50"})`
fn parse_line(line: &str) -> Option<(usize, String, BTreeMap<u64, String>)> {
    let mut it = line.splitn(3, ' ');
    let i: usize = it.next()?.parse().ok()?;
    let status = it.next()?.to_string();
    let mut cells = BTreeMap::new();
    for cell in it.next().unwrap_or("").split(',').filter(|c| !c.is_empty()) {
        let (k, v) = cell.split_once('=')?;
        cells.insert(k.trim().parse().ok()?, v.trim().to_string());
    }
    Some((i, status, cells))
}

pub fn run(args: &DiffArgs) -> Result<i32> {
    let calls: Vec<Call> = args.calls.iter().map(|c| parse_call(c)).collect::<Result<_>>()?;
    if calls.is_empty() {
        bail!("pass at least one --call, as `setLimit(uint256)=101`");
    }
    let (bundle, name, ir) = crate::build::compile_and_lower(
        &args.sources,
        args.contract.as_deref(),
        args.solc.clone(),
        &args.evm_version,
    )?;
    let contract = bundle.contract(&name).expect("selected contract");
    let creation = contract
        .bytecode
        .as_deref()
        .filter(|h| !h.is_empty())
        .ok_or_else(|| anyhow!("solc produced no creation bytecode for {name}"))?;
    let creation = hex_bytes(creation)?;
    let want: Vec<mulu_replay::U256> =
        slots(&ir.storage_layout).into_iter().map(mulu_replay::U256::from).collect();

    // The EVM first: its post-deployment storage is where the Lean run starts.
    let deployed = mulu_replay::replay(&creation, &[], &want)
        .map_err(|e| anyhow!("deploying {name} on the EVM: {e}"))?;
    let initial: BTreeMap<u64, String> =
        deployed.storage.iter().filter_map(|(k, v)| Some((k.parse().ok()?, v.clone()))).collect();
    let evm = mulu_replay::replay(&creation, &calls, &want)
        .map_err(|e| anyhow!("replaying {name} on the EVM: {e}"))?;

    let parsed = mulu_yul::parse::parse_object(&contract.ir)
        .map_err(|e| anyhow!("parsing the Yul of {name}: {e}"))?;
    let object = parsed
        .object
        .deployed()
        .ok_or_else(|| anyhow!("{name}: the Yul has no deployed object"))?;
    let (def, norm, hazards) = mulu_yul::lean::contract_def(object)
        .map_err(|e| anyhow!("rendering {name} in EvmYul's notation: {e}"))?;
    if !hazards.is_empty() {
        // Comparing the two here would be measuring a path the semantics is
        // known to get wrong, and reporting agreement or disagreement on it
        // would say nothing about the rendering.
        eprintln!("this contract cannot be read in the adopted semantics as it stands:");
        for l in hazards.lines() {
            eprintln!("  {l}");
        }
        bail!("refusing to compare a contract the semantics is known to mis-execute");
    }
    let module = generated_module(&def, &initial, &calls)?;

    let dir = args.semantics_dir.join("MuluDiff");
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("Generated.lean"), &module)?;
    println!("contract {name}, {} call(s)", calls.len());
    println!("evm      {}", evm.evm);
    for l in norm.lines() {
        println!("rendered {l}");
    }

    println!("\nbuilding the Lean runner (this needs the semantics package)");
    let built = Command::new("lake")
        .arg("build")
        .arg("mulu-diff")
        .current_dir(&args.semantics_dir)
        .status()
        .with_context(|| format!("running lake in {}", args.semantics_dir.display()))?;
    if !built.success() {
        bail!("lake build mulu-diff failed in {}", args.semantics_dir.display());
    }
    let exe = args.semantics_dir.join(".lake/build/bin/mulu-diff");
    let out = Command::new(&exe)
        .output()
        .with_context(|| format!("running {}", exe.display()))?;
    if !out.status.success() {
        bail!("{} exited with {}", exe.display(), out.status);
    }
    let lean_lines = String::from_utf8_lossy(&out.stdout);

    // --- compare
    println!("\n{:<28} {:<22} {}", "call", "EvmYul", "revm");
    let mut differences = vec![];
    for (i, c) in calls.iter().enumerate() {
        let label = match &c.argument {
            Some(a) => format!("{}({a})", c.signature.split('(').next().unwrap_or("?")),
            None => c.signature.clone(),
        };
        let Some((_, lean_status, lean_storage)) =
            lean_lines.lines().filter_map(parse_line).find(|(j, _, _)| *j == i)
        else {
            differences.push(format!("{label}: the Lean run reported nothing for this call"));
            continue;
        };
        let evm_call = &evm.calls[i];
        let evm_status = if evm_call.success { "ok" } else { "revert" };
        let evm_storage: BTreeMap<u64, String> = evm_call
            .storage
            .iter()
            .filter_map(|(k, v)| Some((k.parse().ok()?, v.clone())))
            .filter(|(_, v)| v != "0")
            .collect();
        let lean_storage: BTreeMap<u64, String> =
            lean_storage.into_iter().filter(|(_, v)| v != "0").collect();
        println!(
            "{label:<28} {:<22} {}",
            format!("{lean_status} {}", cells(&lean_storage)),
            format!("{evm_status} {}", cells(&evm_storage))
        );
        if lean_status != evm_status {
            differences.push(format!(
                "{label}: EvmYul says {lean_status} and the EVM says {evm_status}"
            ));
        }
        if lean_storage != evm_storage {
            differences.push(format!(
                "{label}: storage differs, EvmYul {} and the EVM {}",
                cells(&lean_storage),
                cells(&evm_storage)
            ));
        }
    }

    if differences.is_empty() {
        println!(
            "\nthe two agree on {} call(s). This is evidence and not a proof: \
             semantics:rendering-preserves-the-program stays open.",
            calls.len()
        );
        Ok(0)
    } else {
        eprintln!("\nthe rendered module and the EVM disagree:");
        for d in &differences {
            eprintln!("  {d}");
        }
        eprintln!(
            "\nOne of three things is wrong: the rendering is a different program, EvmYul and \
             revm disagree about the EVM, or the analysis reads the wrong artifact."
        );
        Ok(1)
    }
}

fn cells(m: &BTreeMap<u64, String>) -> String {
    if m.is_empty() {
        return "{}".into();
    }
    let v: Vec<String> = m.iter().map(|(k, val)| format!("{k}={val}")).collect();
    format!("{{{}}}", v.join(","))
}

fn hex_bytes(h: &str) -> Result<Vec<u8>> {
    let h = h.strip_prefix("0x").unwrap_or(h);
    if h.len() % 2 != 0 {
        bail!("the creation bytecode has an odd number of hex digits");
    }
    (0..h.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&h[i..i + 2], 16).map_err(|e| anyhow!("bad hex: {e}")))
        .collect()
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_call_is_a_signature_and_at_most_one_argument() {
        let c = parse_call("setLimit(uint256)=101").unwrap();
        assert_eq!(c.signature, "setLimit(uint256)");
        assert_eq!(c.argument.as_deref(), Some("101"));
        assert!(parse_call("limit()").unwrap().argument.is_none());
        // a name is not a call: the selector comes from the signature, and
        // guessing the types would send different calldata
        assert!(parse_call("setLimit=1").is_err());
        assert!(parse_call("setLimit(uint256)=twelve").is_err());
    }

    #[test]
    fn the_generated_scenario_encodes_the_calls_it_was_given() {
        let calls = vec![
            Call { signature: "setLimit(uint256)".into(), argument: Some("101".into()) },
            Call { signature: "limit()".into(), argument: None },
        ];
        let initial = BTreeMap::from([(0u64, "7".to_string())]);
        let m = generated_module("def contract : YulContract := sorry\n", &initial, &calls).unwrap();
        assert!(m.contains("def initialStorage : List (Nat × Nat) :=\n  [(0, 7)]"), "{m}");
        // solc's selector for setLimit(uint256) is 0x27ea6f2b
        assert!(m.contains("calldataOf [0x27, 0xea, 0x6f, 0x2b] [101]"), "{m}");
        assert!(m.contains("[]"), "a call with no argument sends only its selector");
        // and the caller must be the address revm sends from
        assert!(m.contains(CALLER_HEX), "{m}");
    }

    #[test]
    fn a_line_from_the_lean_runner_is_read_back_as_it_was_written() {
        let (i, status, cells) = parse_line("2 revert 0=50,1=7").unwrap();
        assert_eq!(i, 2);
        assert_eq!(status, "revert");
        assert_eq!(cells[&0], "50");
        assert_eq!(cells[&1], "7");
        assert!(parse_line("0 ok ").unwrap().2.is_empty());
    }
}
