# Interface Contracts (Cross-Crate)

**Owner:** Architect (`agent/01-architect`)  
**Rule:** Implement behind these surfaces. Prefer mocks until the producer crate is ready. Breaking changes require Architect + peer agent sign-off.

All cross-crate messages SHOULD be `serde`-serializable (CBOR preferred on mesh; JSON allowed on browser↔local gateway).

---

## Layer Diagram

```
┌─────────────────────────────────────────────────────────────┐
│  frontend / captive-portal (TS)                             │
│    HTTP/WS ──► interop gateway (when online)                │
│    local RPC / postMessage ──► mesh node host API           │
└───────────────────────────┬─────────────────────────────────┘
                            │ MeshHostApi / JobSubmit
┌───────────────────────────▼─────────────────────────────────┐
│  interop                                                    │
│    AsyncAPI + HTLC/TWAMM + optional OpenAPI bridge          │
└───────┬───────────────────────────────┬─────────────────────┘
        │ JobEnvelope / EscrowEvent     │ StateQuery
┌───────▼──────────────┐       ┌────────▼─────────────────────┐
│  mesh-transport      │       │  consensus                   │
│  route, VRF, VDF     │       │  ledger, proofs, CRDT merge  │
└───────┬──────────────┘       └────────▲─────────────────────┘
        │ ExecuteRequest                │ CommitReceipt
┌───────▼──────────────┐                │
│  sandbox (WAMR)      ├────────────────┘
│  MEM_LIMIT 10MB      │
└───────┬──────────────┘
        │ FFI: evaluate_move / get_best_move / …
┌───────▼──────────────┐
│  engine (shakmaty)   │
│  deterministic WASM  │
└──────────────────────┘
```

---

## 1. Shared Types (logical)

```text
NodeId        = 32-byte identity hash (not IP, not DNS name)
JobId         = 32-byte Ephemeral Job DID (VDF-gated)
ContentId     = 32-byte content hash (IPLD-style)
TraceHash     = 32-byte hash-chain link of WASM execution trace
```

### `JobEnvelope` (interop → transport → sandbox)

| Field | Type | Notes |
| --- | --- | --- |
| `job_id` | `JobId` | Requires valid VDF proof attachment |
| `submitter` | `NodeId` | Identity of requester |
| `payload_cid` | `ContentId` | WASM module or chess FEN/job blob |
| `fn_name` | `string` | Exported WASM symbol (e.g. `evaluate_move`) |
| `args` | `bytes` | Opaque to transport |
| `replicas` | `u8` | Primaries default 3 + standbys default 2 |
| `max_wall_ms` | `u64` | Contributes to <5% timeout SLA |

### `VdfProof`

| Field | Type | Notes |
| --- | --- | --- |
| `start_x` | field element / u64 stub | MinRoot start |
| `steps` | `u64` | Sequential steps (~10 min wall in prod) |
| `final_x` | field element / u64 stub | Claimed output |
| `modulus_id` | `u32` / curve id | Parameter set |

**Invariant:** Creating a new `JobId` without a verifiable VDF burn is rejected.

### `VrfAssignment`

| Field | Type | Notes |
| --- | --- | --- |
| `job_id` | `JobId` | |
| `seed_commit` | `[u8;32]` | Commit-and-reveal |
| `primaries` | `[NodeId; 3]` | Dynamic triangulation |
| `standbys` | `[NodeId; 2]` | Instant failover |
| `weights` | map | Battery + thermal aware |

### `ExecutionResult` (sandbox → consensus / transport)

| Field | Type | Notes |
| --- | --- | --- |
| `job_id` | `JobId` | |
| `worker` | `NodeId` | |
| `output` | `bytes` | Deterministic guest output |
| `trace_hash` | `TraceHash` | Free-rider prevention |
| `mem_peak` | `u32` | Guest heap capped by `MEM_LIMIT` |
| `ok` | `bool` | |

### `CommitReceipt` (consensus)

| Field | Type | Notes |
| --- | --- | --- |
| `root_hash` | `[u8;32]` | Trie root after insert/merge |
| `proof` | Merkle and/or SNARK bytes | Prefer constant-size SNARK |
| `ledger_bytes` | `u32` | **must be ≤ 10_000_000** |

### `EscrowEvent` (interop)

| Field | Type | Notes |
| --- | --- | --- |
| `htlc_id` | `[u8;32]` | |
| `amount` | `u128` | Abstract credit units |
| `hash_lock` | `[u8;32]` | |
| `vdf_cancel_deadline` | VDF or step counter | VDF-delayed cancellation |
| `state` | `Open\|Claimed\|Refunded\|Slashed` | |

---

## 2. mesh-transport ↔ sandbox

| Op | Signature (logical) | Producer | Consumer |
| --- | --- | --- | --- |
| Gate job | `verify_vdf(proof) -> bool` | transport (`vdf_sybil`) | sandbox / transport admit |
| Assign | `assign_workers(job, topology) -> VrfAssignment` | transport (`vrf_assigner`) | transport + interop |
| Execute | `WamrInstance::invoke_wasm_function(fn, args) -> Result<bytes>` | sandbox | transport worker loop |
| Cap RAM | `allocate(size) -> Result<ptr>` enforces `MEM_LIMIT = 10 MiB` | sandbox | all guests |
| Prove work | `hash_chain::append(trace) -> TraceHash` | transport | consensus / security |

**Error contract:** OOM returns deterministic error so all verifiers agree.

**Current stubs:** `mesh_transport::init_mesh_transport()`, `sandbox::WamrInstance::{new, allocate, invoke_wasm_function}`, `sandbox::MEM_LIMIT`.

### 2a. Job messaging (Phase 4 — P4-MSG)

**Module:** `mesh_transport::messaging` (agent 02; topology route hooks agent 03).

In-memory island bus is OK for unit/smoke; later wire LXMF/libp2p. **No DNS/IP** in keys — only `NodeId = [u8; 32]`.

| Op | Signature (logical) | Notes |
| --- | --- | --- |
| Distribute | `distribute_job(envelope: JobEnvelope, workers: &[[u8;32]]) -> Result<(), MsgError>` | Queue one delivery per assigned worker inbox |
| Poll | `JobMeshBus::poll_inbox(worker) -> Option<JobDelivery>` | Worker pulls next job |
| Submit | `submit_result(result: ExecutionResult) -> Result<(), MsgError>` | Worker must be assigned; no double-submit |
| Collect one | `collect_result(job_id) -> Result<ExecutionResult, MsgError>` | First available result |
| Collect all | `collect_all_results(job_id) -> Result<Vec<ExecutionResult>, MsgError>` | Replica quorum path |

**Types (serde):** `JobEnvelope`, `ExecutionResult`, `JobDelivery { envelope, worker }`, `MsgError`.

**MsgError tokens (stable):** `NoWorkers`, `UnknownJob`, `NoResult`, `AlreadySubmitted`, `WorkerNotAssigned`, `EmptyPayload`, `LockPoisoned`.

**Pipeline:**

```text
interop submit → VDF admit → VRF assign → messaging::distribute_job
  → worker poll_inbox → sandbox Job::admit_and_load / invoke
  → messaging::submit_result → collect_result → consensus CommitReceipt
```

**Init:** `messaging::init_messaging()` called from `init_mesh_transport()`.

**Topology (03):** route workers via Kademlia / `topology` cost; messaging only needs `NodeId` list — must not require multiaddrs or public DNS.

### 2b. WASM load API (Phase 4 — P4-WASM)

**Sandbox (14):** load real engine module bytes (not only host-sim stub).

| Op | Signature | Notes |
| --- | --- | --- |
| Load (test) | `Job::load(module_bytes) -> Result<Job, JobError>` | No VDF gate |
| Admit+load | `Job::admit_and_load(receipt, verifier, module_bytes) -> Result<Job, JobError>` | Production path |
| Invoke | `Job::invoke` / `invoke_admitted(fn, args) -> Result<Vec<u8>, JobError>` | Admitted required for trusted path |

**Module validation (required for checkbox):**

1. Reject empty bytes → `JobError::InvalidModule` / `HostError::InvalidModule`.
2. Prefer magic `\0asm` (WASM binary preamble) check on load; invalid → same stable error string.
3. Guest heap still capped by `MEM_LIMIT = 10 MiB`.

**Engine (05):**

```text
cargo build -p engine --target wasm32-wasip1
# preferred: cdylib → target/wasm32-wasip1/release/engine.wasm (or libengine.so.wasm)
# today docs note rlib-only; P4-WASM needs loadable .wasm for sandbox Job::load
```

| WASM export | Args | Returns | Semantics |
| --- | --- | --- | --- |
| `evaluate_move` | position + move | score / validity bytes | Deterministic eval |
| `get_best_move` | position + depth | move bytes | Search within `MAX_DEPTH` |

Optional marker for host-sim discovery: ASCII `MOSSYMESH_EXPORTS:name1,name2` in module bytes (sandbox host sim only).

---

## 3. sandbox ↔ engine

| WASM export | Args | Returns | Semantics |
| --- | --- | --- | --- |
| `evaluate_move` | position + move | score / validity bytes | Deterministic eval |
| `get_best_move` | position + depth | move bytes | Search within `MAX_DEPTH` |

**Native crate API:** `engine::EngineState::{new, from_fen, get_moves, make_move, evaluate_position}`, `engine::benchmark_mnps()`, `engine::MAX_DEPTH = 64`.

**Determinism:** no wall-clock; no RNG without explicit seed in job args; integer scores preferred.

**Build:** default features WASM-safe (no `syzygy` / `syzygy-mmap` on wasip1). See `devops/engine-wasm.md`.

---

## 4. mesh-transport ↔ consensus

| Op | Logical API |
| --- | --- |
| Insert | `TrieNode::insert_node(key, value)` |
| Merge islands | `TrieNode::merge_state(remote)` |
| Verify | `verify_proof() -> bool` |
| Compress | `fold_snarks()` / `verify_snark()` |
| Bound | `MAX_LEDGER_SIZE = 10_000_000` |

Transport must not store full historical ledger on edge; only active roots + proofs + ring buffers.

---

## 5. consensus ↔ interop

| Op | Notes |
| --- | --- |
| Query root | `/api/v1/health` stub |
| Submit job credit | HTLC must be Open |
| Settlement | claim/refund + VDF-delayed cancel |
| Online bridge | OpenAPI when uplink exists; no cloud DB |

**Stubs:** `interop::AsyncApiRequest`, `handle_rest_call`, `handle_websocket`.

---

## 6. interop ↔ frontend

| Endpoint (v1) | Method | Response |
| --- | --- | --- |
| `/api/v1/health` | GET | island status |
| `/api/v1/submit_job` | POST | accept + future `job_id` |
| `/api/v1/game/state` | GET | FEN + moves (future) |
| `/api/v1/game/move` | POST | new FEN or error (future) |
| WS `/api/v1/sync` | WS | ledger/game updates |

- Offline-first PWA; chess usable with zero uplink.
- No Firebase/AWS SDK; no hardcoded cloud as sole backend.
- Captive portal nginx: `client_max_body_size 150M`.

---

## 7. mesh-transport ↔ frontend (indirect)

Frontend displays topology via host JSON DTOs (peer list, battery, thermal, quarantine). UI never imports Rust RF modules directly.

---

## 8. Versioning

- Workspace `0.1.x` until Phase 3.
- Additive fields OK; renames need `schema_ver: u16`.
- `integration/` smoke must pass before multi-crate merges to `main`.

## 9. Non-Goals

- Resolving `NodeId` via public DNS.
- Persisting ledger in third-party cloud SQL.
- Non-deterministic guest APIs inside WASM jobs.
