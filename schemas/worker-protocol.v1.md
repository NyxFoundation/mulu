# mulu-worker protocol, version 1

One request per process. `mulu-worker --dir <analysis-dir>` reads **one JSON
object** from stdin and writes **one JSON object** to stdout. stderr is for
logs. All paths are relative to `--dir`; absolute paths and `..` are rejected.

## Core model (`core-model.json`)

Produced by `mulu-model` from a `finite-product` file; integer indices only.

```json
{"num_states": 3, "num_events": 2, "controllable": [0], "initial": [0],
 "marked": [0], "bad": [2], "edges": [[0, 0, 1], [1, 1, 2]],
 "checks": [{"id": 0, "pass_event": 1, "fail_event": 0}]}
```

## Request

```json
{"protocol_version": 1, "request_id": "impl", "method": "analyze",
 "model_path": "core-model.json",
 "analyses": ["reachability", "redundancy", "safety", "envelope"],
 "objective": "safety-nonblocking",
 "limits": {"max_states": 100000, "max_edges": 1000000}}
```

`method` is `analyze` or `verify`; `verify` adds `certificate_path`.

## Response (analyze)

Per analysis a `status` from docs/09 §4 (`complete`, `unrealizable`,
`partial`, `unsupported`, `not-requested`, `error`), the computed data, and
the **certificate** the worker produced *and re-checked* (`checked: true`).
`complete` never means "safe": a completed search that found a violation is
also `complete`.

## Certificates

```json
{"kind": "reachability", "states": [0, 1, 2]}
{"kind": "redundancy",   "states": [...], "check": {"id": 1, "pass_event": 8, "fail_event": 9}}
{"kind": "violation",    "path": [[0, 0, 1], [1, 1, 2]]}
{"kind": "envelope",     "nonblocking": true, "chain": [[0, 1], [0]]}
```

What each one proves when `Mulu.Analysis.checkCertificate` returns `true` is
`Mulu.Analysis.Claim` (lean/Mulu/Analysis/Certificate.lean), by the theorem
`checkCertificate_sound`. The Rust CLI additionally writes
`certificates/Check.lean`, which re-runs the checker under the Lean kernel
(`by decide`) and prints the axioms used.
