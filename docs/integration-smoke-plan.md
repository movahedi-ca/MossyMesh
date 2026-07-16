# Integration Smoke Test Plan

**Owner:** Architect (01) + Determinism Auditor (16)  
**Harness:** `integration/` workspace member

## Goals

1. Prove crates **link** and expose expected init/FFI surfaces.
2. Exercise **happy-path contracts** without real radios or full WAMR.
3. Fail CI early on broken cross-crate wiring.

Non-goals: full RF fidelity, 836 Mnps benches, real Nova proofs.

## Prerequisites

```bash
rustup toolchain install stable
rustup target add wasm32-wasip1   # Phase 4+
cargo test -p integration
cargo test -p integration --features transport   # optional
```

If Rust is unavailable, treat this plan + harness source as the contract; run when toolchain exists.

## Test Matrix

| ID | Name | Crates | Asserts |
| --- | --- | --- | --- |
| SMK-01 | `smoke_inits` | consensus, engine, sandbox, interop (+ transport feature) | `init_*` panic-free |
| SMK-02 | `smoke_sandbox_mem_cap` | sandbox | over `MEM_LIMIT` → `Err` |
| SMK-03 | `smoke_engine_startpos` | engine | startpos 20 legal moves |
| SMK-04 | `smoke_consensus_insert_merge` | consensus | insert + merge child present |
| SMK-05 | `smoke_interop_health` | interop | `/api/v1/health` Ok |
| SMK-06 | `smoke_job_pipeline_stub` | interop → sandbox | `get_best_move` non-empty |
| SMK-07 | `smoke_ledger_bound_constant` | consensus | `MAX_LEDGER_SIZE == 10_000_000` |
| SMK-08 | `smoke_sandbox_mem_constant` | sandbox | `MEM_LIMIT == 10 * 1024 * 1024` |

## Pipeline Stub (SMK-06)

```text
AsyncApiRequest{ "/api/v1/submit_job", payload }
  → interop::handle_rest_call
  → sandbox::WamrInstance::new
  → invoke_wasm_function("get_best_move")
  → non-empty bytes
```

## Pass / Fail

- **Pass:** all `#[test]` in `integration` green on stable Rust.
- **Fail:** panic, link error, or SLA constant regression without Architect approval.

## Phase Hooks (future)

| Phase | Additional smoke |
| --- | --- |
| 1 | Packet translate sim |
| 2 | VDF gate before sandbox admit |
| 3 | Merkle proof round-trip |
| 4 | Real engine WASM in WAMR |
| 5 | TWAMM spread cap + gateway flag |

## Ownership

- Architect maintains plan + minimal harness.
- DevOps (16) wires CI when runners exist.
- Crate agents fix module failures; do not disable smoke without Architect note.
