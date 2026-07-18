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
| SMK-06 | `smoke_job_pipeline` | interop → sandbox (test MinRoot admit) → engine → consensus | admitted `get_best_move` + startpos eval + Merkle proof |
| SMK-07 | `smoke_ledger_bound_constant` | consensus | `MAX_LEDGER_SIZE == 10_000_000` |
| SMK-08 | `smoke_sandbox_mem_constant` | sandbox | `MEM_LIMIT == 10 * 1024 * 1024` |
| SMK-09 | `smoke_engine_wasm_load` | engine + sandbox | wasip1 artifact loads via `Job::load` / `admit_and_load` |
| SMK-10 | `smoke_job_messaging` | mesh-transport | distribute → poll → submit_result → collect_result |

## Job Pipeline (SMK-06)

```text
AsyncApiRequest{ "/api/v1/submit_job", payload }
  → interop::handle_rest_call
  → sandbox::MinRootVdfVerifier::for_tests + Job::admit_and_load
  → Job::invoke_admitted("get_best_move")
  → engine::EngineState startpos (legal moves + evaluate_position)
  → consensus::MerklePatriciaTrie insert + prove + verify_proof
  → JobPipelineResult { job_did, sandbox_output, eval, root, proof_ok }
```

Offline / fast: test MinRoot uses small iteration counts (never production 50M).

## Phase 4 paths (open SLA boxes)

### SMK-09 — Engine WASM load (P4-WASM)

**Owners:** 05 (engine wasip1/cdylib), 14 (sandbox load path), 16 (wire smoke).

```bash
rustup target add wasm32-wasip1
cargo build -p engine --release --target wasm32-wasip1
# Prefer loadable .wasm (cdylib). If only rlib exists, box stays open.
```

| Step | Assert |
| --- | --- |
| 1 | Artifact exists under `target/wasm32-wasip1/release/` with WASM magic `\0asm` **or** documented host path that still validates magic |
| 2 | `sandbox::Job::load(bytes)` Ok; empty / bad magic → `InvalidModule` |
| 3 | `Job::admit_and_load` + `invoke_admitted("get_best_move" \| "evaluate_move")` returns non-empty deterministic bytes |
| 4 | Peak guest mem ≤ `MEM_LIMIT` |

**Pass for SLA box:** steps 1–3 green in `cargo test -p integration` (or crate tests + noted smoke). Host-sim-only `\0asm` stub **does not** close the box alone — need engine wasip1 build artifact in the load path.

### SMK-10 — Job messaging (P4-MSG)

**Owners:** 02 (bus), 03 (worker route via topology/Kademlia), 16 (smoke).

```text
JobEnvelope → messaging::distribute_job(env, workers)
  → worker poll_inbox → (optional) sandbox invoke
  → messaging::submit_result(ExecutionResult)
  → messaging::collect_result(job_id)
```

| Step | Assert |
| --- | --- |
| 1 | Empty workers → `MsgError::NoWorkers` |
| 2 | Distribute to ≥1 `NodeId`; each inbox depth ≥ 1 |
| 3 | Unassigned worker submit → `WorkerNotAssigned` |
| 4 | Double submit same worker → `AlreadySubmitted` |
| 5 | One ok submit → `collect_result` returns same `output` / `trace_hash` |
| 6 | Serde round-trip `JobEnvelope` + `ExecutionResult` |

**Pass for SLA box:** unit tests in `mesh-transport` green **and** integration (or `cargo test -p mesh-transport` under transport feature) covers distribute/collect. Prefer identity-only keys (no DNS).

### Combined P4 happy path (optional SMK-11 later)

```text
VRF assign workers → distribute_job → admit_and_load(engine.wasm)
  → invoke → submit_result → collect_result → trie insert proof
```

## Pass / Fail

- **Pass:** all `#[test]` in `integration` green on stable Rust.
- **Fail:** panic, link error, or SLA constant regression without Architect approval.
- **P4 boxes:** Architect (01) checks `docs/sla-and-dod.md` only when SMK-09 and SMK-10 criteria met on merged/peer-proven code; then updates `CHANGELOG.md` Still open.

## Phase Hooks

| Phase | Additional smoke |
| --- | --- |
| 1 | Packet translate sim |
| 2 | VDF gate before sandbox admit |
| 3 | Merkle proof round-trip |
| 4 | SMK-09 engine WASM load + SMK-10 job messaging |
| 5 | TWAMM spread cap + gateway flag |

## Ownership

- Architect maintains plan + minimal harness.
- DevOps (16) wires CI when runners exist.
- Crate agents fix module failures; do not disable smoke without Architect note.
