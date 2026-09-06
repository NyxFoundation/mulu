//! Driving `mulu-worker` (one JSON request per process).

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub struct Toolchain {
    pub lean_dir: Option<PathBuf>,
    pub worker: Option<PathBuf>,
}

fn walk_up_for(start: &Path, rel: &str) -> Option<PathBuf> {
    let mut cur = Some(start.to_path_buf());
    while let Some(dir) = cur {
        let cand = dir.join(rel);
        if cand.exists() {
            return Some(dir);
        }
        cur = dir.parent().map(|p| p.to_path_buf());
    }
    None
}

impl Toolchain {
    pub fn discover(lean_dir: Option<PathBuf>, worker: Option<PathBuf>) -> Self {
        let lean_dir = lean_dir
            .or_else(|| std::env::var_os("MULU_LEAN_DIR").map(PathBuf::from))
            .or_else(|| {
                let from_exe = std::env::current_exe().ok().and_then(|e| walk_up_for(&e, "lean/lakefile.lean"));
                from_exe
                    .or_else(|| std::env::current_dir().ok().and_then(|d| walk_up_for(&d, "lean/lakefile.lean")))
                    .map(|root| root.join("lean"))
            });
        let worker = worker
            .or_else(|| std::env::var_os("MULU_WORKER").map(PathBuf::from))
            .or_else(|| lean_dir.as_ref().map(|d| d.join(".lake/build/bin/mulu-worker")))
            .filter(|p| p.exists());
        Self { lean_dir, worker }
    }

    pub fn worker_path(&self) -> Result<&Path> {
        self.worker.as_deref().ok_or_else(|| {
            anyhow!("mulu-worker not found: build it with `cd lean && lake build`, or pass --worker / MULU_WORKER")
        })
    }
}

pub struct WorkerRequest<'a> {
    pub request_id: &'a str,
    pub method: &'a str,
    pub model_path: &'a str,
    pub analyses: &'a [&'a str],
    pub objective: &'a str,
    pub certificate_path: Option<&'a str>,
    pub max_states: usize,
}

/// Run the worker in `dir`. A timeout yields `status: partial` per docs/09 §4.
pub fn call(tc: &Toolchain, dir: &Path, req: &WorkerRequest, timeout: Duration) -> Result<Value> {
    let worker = tc.worker_path()?;
    let mut body = json!({
        "protocol_version": 1,
        "request_id": req.request_id,
        "method": req.method,
        "model_path": req.model_path,
        "analyses": req.analyses,
        "objective": req.objective,
        "limits": {"max_states": req.max_states, "timeout_ms": timeout.as_millis() as u64},
    });
    if let Some(c) = req.certificate_path {
        body["certificate_path"] = json!(c);
    }
    let mut child = Command::new(worker)
        .arg("--dir")
        .arg(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .with_context(|| format!("spawning {}", worker.display()))?;
    {
        let mut stdin = child.stdin.take().unwrap();
        stdin.write_all(body.to_string().as_bytes())?;
    }
    let start = Instant::now();
    let mut stdout = child.stdout.take().unwrap();
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        std::io::Read::read_to_string(&mut stdout, &mut s).map(|_| s)
    });
    loop {
        if let Some(status) = child.try_wait()? {
            let out = reader.join().map_err(|_| anyhow!("worker reader thread panicked"))??;
            if !status.success() {
                bail!("worker exited with {status}");
            }
            let v: Value = serde_json::from_str(out.trim()).with_context(|| format!("worker returned invalid JSON: {out}"))?;
            return Ok(v);
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(json!({"protocol_version": 1, "request_id": req.request_id, "status": "partial",
                "error": format!("worker timeout after {} ms", timeout.as_millis())}));
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}
