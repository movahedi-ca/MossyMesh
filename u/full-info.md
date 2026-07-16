# MossyMesh Master Documentation

Single source of truth for the swarm: charter, stack, agent map, and **crate interface contracts**.

| Companion | Path |
| --- | --- |
| Agent grid | [`AGENTS.md`](../AGENTS.md) |
| Workstreams | [`workstream.md`](../workstream.md) |
| Plan | [`u/plan.md`](plan.md) |
| Constraints card | [`u/info.md`](info.md) |
| Contracts | [`docs/interface-contracts.md`](../docs/interface-contracts.md) |
| SLA / DoD | [`docs/sla-and-dod.md`](../docs/sla-and-dod.md) |
| Smoke plan | [`docs/integration-smoke-plan.md`](../docs/integration-smoke-plan.md) |

---

## 1. Mission

Build a self-healing mesh that turns phones, Raspberry Pis, PCs, and LoRa radios into a unified, decentralized compute grid **independent of traditional ISPs, DNS servers, and fiat rails**.

**PoC:** MessyMash offline-capable Chess (`shakmaty`) stressing perfect state-transition determinism across heterogeneous hardware.

---

## 2. Strict SLAs

| Constraint | Value |
| --- | --- |
| Active ledger RAM (edge) | ≤ **10 MB** |
| Unverifiable outputs | < **1%** |
| Job timeout rate | < **5%** |
| Capacity target | 100 RVCH/day / 20-node island, zero upstream Internet |
| Forbidden | Centralized public DNS as sole locator, fixed IP control planes, cloud DBs |

Data availability: RAM-disk ring buffers (ephemeral DA + LRU) and regional SSD hubs with append-only logs + Reed-Solomon (charter).

---

## 3. Core Technology Stack

| Layer | Technologies |
| --- | --- |
| Frontend | React, TypeScript, Vite, vite-plugin-pwa, nginx (`client_max_body_size 150M`) |
| Engine | Rust, shakmaty, shakmaty-syzygy, wasm32-wasip1 |
| Sandbox | WAMR, WASI, symmetric static INT8, fixed-block pools |
| Transport | reticulum-rs / lxmf-rs direction, Kademlia DHT, LoRa, BLE, Wi-Fi Direct |
| Consensus | trie-db path, ipld/DAG-CBOR, nova-snark folding, yrs/YATA CRDT |
| Interop | AsyncAPI/OpenAPI gateway, HTLC, TWAMM (≤2% spread) |
| AI | SITF, Edge PagedAttention, Vulkan compute (tiered) |

---

## 4. Repository Crate Map

| Path | Role | Primary agents |
| --- | --- | --- |
| `mesh-transport/` | Identity routing, RF adapters, VRF, VDF, security | 02, 03, 08, 13 |
| `sandbox/` | WAMR isolation, MEM_LIMIT | 08, 14 |
| `engine/` | Chess bitboards / WASM guest | 05 |
| `consensus/` | Ledger, proofs, CRDT merge | 06, 07, 09 |
| `interop/` | Gateway, HTLC, TWAMM, governance hooks | 10, 11, 12 |
| `frontend/`, `captive-portal/` | PWA + portal | 04, 15 |
| `integration/` | Cross-crate smoke | 01, 16 |
| `swarm/` | CrewAI orchestration (meta) | ops |

---

## 5. Interface Contracts (Concrete)

Normative detail: [`docs/interface-contracts.md`](../docs/interface-contracts.md).

### 5.1 End-to-end data path

```
Frontend (PWA)
  --HTTP/WS-->  interop (AsyncAPI)
  --JobEnvelope-->  mesh-transport (VDF admit, VRF assign, route)
  --ExecuteRequest-->  sandbox (WAMR, MEM_LIMIT)
  --FFI-->  engine (deterministic eval)
  --ExecutionResult+TraceHash-->  consensus (trie insert / SNARK / CRDT)
  --CommitReceipt / EscrowEvent-->  interop → Frontend
```

### 5.2 mesh-transport ↔ sandbox

| Contract | Detail |
| --- | --- |
| Admit | Job requires verifiable MinRoot **VDF** before schedule |
| Assign | VRF sortition: **3 primary + 2 standby**; battery/thermal weights |
| Execute | `sandbox::WamrInstance::invoke_wasm_function(name, args) -> Result<Vec<u8>>` |
| Memory | `sandbox::MEM_LIMIT = 10 MiB`; deterministic `Err` on overflow |
| Integrity | Execution **hash chain** link required for credit |

**Symbols today:** `mesh_transport::init_mesh_transport()`, `sandbox::{WamrInstance, MEM_LIMIT, init_sandbox}`.

### 5.3 sandbox ↔ engine

| Export / API | Behavior |
| --- | --- |
| `EngineState::from_fen` / startpos | Host or guest load |
| `get_moves` / `make_move` | Legal move gen; illegal → error |
| `evaluate_position` | Deterministic integer score |
| WASM names | `evaluate_move`, `get_best_move` (sandbox FFI) |
| Limits | `MAX_DEPTH = 64`; no unseeded RNG; no wall-clock in guest |

**Symbols today:** `engine::{EngineState, benchmark_mnps, MAX_DEPTH, init_engine}`.

### 5.4 mesh-transport ↔ consensus

| Op | API |
| --- | --- |
| Commit result | `TrieNode::insert_node(key, value)` |
| Island reconvergence | `TrieNode::merge_state(&remote)` |
| Verify | `verify_proof` / `verify_snark` |
| Compress | `fold_snarks` |
| Bound | `MAX_LEDGER_SIZE = 10_000_000` |

Transport gossips **roots + proofs + results**, not unbounded history, on edge devices.

### 5.5 consensus ↔ interop

- Health/job submit stubs: `/api/v1/health`, `/api/v1/submit_job`.
- HTLC states `Open|Claimed|Refunded|Slashed` with VDF-delayed cancel.
- OpenAPI bridge is optional uplink; **island ledger remains authoritative offline**.

### 5.6 interop ↔ frontend

| Endpoint | Purpose |
| --- | --- |
| `GET /api/v1/health` | Island alive |
| `POST /api/v1/submit_job` | Enqueue compute/chess job |
| `GET/POST /api/v1/game/*` | Future FEN/move (reserved) |
| `WS /api/v1/sync` | Live deltas |

Frontend remains offline-capable; captive portal is the on-ramp, not cloud SaaS.

### 5.7 mesh-transport ↔ frontend (DTO only)

UI reads peer/link/battery/quarantine **JSON DTOs** from the local host API.

---

## 6. Collaboration Rules

1. **Directory isolation** — only assigned scope ([`AGENTS.md`](../AGENTS.md)).
2. **Contracts first** — mock peers; do not invent conflicting field names.
3. **No centralized assumptions** — charter out-of-scope list is absolute.
4. **Tests** — unit tests per crate; smoke in `integration/`.
5. **Change control** — crypto/memory changes need SLA proof before `main`.

---

## 7. Phase DoD Index

| Phase | One-line DoD |
| --- | --- |
| 1 Transport | Phone test packet → LoRa → offline node via Kademlia |
| 2 Sandbox | WAMR 10 MB cap + 10 min VDF for Job DID |
| 3 Consensus | Sub-MB edge proof + deterministic island merge |
| 4 Logic | WASM chess ~836 Mnps + HTLC VDF-cancel |
| 5 Interop | Uplink OpenAPI + TWAMM ≤2% spread |

Full checklists: [`docs/sla-and-dod.md`](../docs/sla-and-dod.md).

---

## 8. Integration Smoke

```bash
cargo test -p integration
cargo test -p integration --features transport   # when mesh-transport builds cleanly
```

Covers inits, MEM_LIMIT, startpos moves, trie merge, interop health, stub job pipeline.

---

## 9. Risk Register (Architect Watch)

| Risk | Mitigation owner |
| --- | --- |
| Shared worktree collisions | Strict scopes + small PRs; prefer dedicated worktrees per agent |
| SLA-RAM regression | Constants in smoke tests; review on ledger PRs |
| Stub drift vs contracts | Architect updates contracts when real types land |
| Upstream crate abandonment | Fork/vendor strategy (charter) |
| iOS Wi-Fi captive flakiness | Portal/devops runbooks |
