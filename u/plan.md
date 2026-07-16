# Swarm Execution Plan

Phased parallel execution for the MossyMesh AI Swarm (16 agents).  
Hard constraints: **≤10 MB active ledger**, **<1% unverifiable outputs**, **no centralized DNS/IP/cloud DBs**.

| Doc | Path |
| --- | --- |
| Roster | [`AGENTS.md`](../AGENTS.md) |
| Contracts | [`docs/interface-contracts.md`](../docs/interface-contracts.md) |
| SLA / DoD | [`docs/sla-and-dod.md`](../docs/sla-and-dod.md) |
| Smoke | [`docs/integration-smoke-plan.md`](../docs/integration-smoke-plan.md) |

---

## Agent → Workstream Snapshot

| Agents | Focus |
| --- | --- |
| 01 Architect, 16 DevOps/Auditor | Contracts, SLA gates, `integration/` smoke |
| 02 Transport, 03 Topology, 08 VDF, 13 Security | `mesh-transport` |
| 04 Portal, 15 Frontend UX | `frontend`, `captive-portal` |
| 05 Engine | `engine` |
| 06 Trie, 07 SNARK, 09 CRDT | `consensus` |
| 08 + 14 Sandbox/AI | `sandbox` |
| 10 HTLC, 11 TWAMM, 12 Governance | `interop` |

---

## Phase 1: Parallel Foundation (Transport-first)

| Workstream | Agents | Exit criteria (summary) |
| --- | --- | --- |
| A Frontend/Portal | 04, 15 | Offline PWA + captive portal shell |
| B Mesh Transport | 02, 03, 13 | Phone packet → LoRa + Kademlia to offline node (sim OK) |
| C Consensus | 06, 07, 09 | Trie/SNARK/CRDT stubs with 10 MB constant |
| D Engine | 05 | shakmaty startpos + move gen tests |
| E Sandbox | 08, 14 | MEM_LIMIT enforce + VDF stub |
| F Interop | 10, 11, 12 | AsyncAPI health/submit mocks |

**Integration gate:** `cargo test -p integration` (SMK-01…08).

## Phase 2: Sandbox & Constraint Enforcement

- WAMR hardens 10 MB guest heap; fixed-block pools; INT8 policy hooks.
- Ephemeral Job DID = VDF burn (~10 min MinRoot in prod params).
- Frontend reaches engine only through sandbox FFI (`evaluate_move`, `get_best_move`).
- **transport admits job only after `verify_vdf`.**

## Phase 3: Consensus Hardening

- Incremental Merkle-Patricia + edge-verifiable proofs (fold toward constant size).
- Island merge via CRDT/binary deltas → identical roots.
- Active ledger accounting vs `MAX_LEDGER_SIZE`.

## Phase 4: Engine Logic & Credits

- Engine → `wasm32-wasip1`; Mnps bench ~836 on reference profile.
- HTLC escrow + VDF-delayed cancellation in interop.
- Hash-chain traces on results (free-rider prevention).

## Phase 5: Interop & Online Bridge

- OpenAPI gateway on uplink; island autonomy preserved offline.
- TWAMM max-spread ≤ 2%.
- E2E captive-portal chess against local host API.

Full phase checklists: [`docs/sla-and-dod.md`](../docs/sla-and-dod.md).

---

## Interface Contracts (Summary)

Normative fields: [`docs/interface-contracts.md`](../docs/interface-contracts.md).

| Boundary | Primary messages / APIs |
| --- | --- |
| **frontend ↔ interop** | HTTP/WS: `/api/v1/health`, `/api/v1/submit_job`, future game state/move |
| **interop ↔ mesh-transport** | `JobEnvelope`, `VrfAssignment`, escrow-gated admit |
| **mesh-transport ↔ sandbox** | VDF gate, `WamrInstance::invoke_wasm_function`, `TraceHash` |
| **sandbox ↔ engine** | WASM exports / `EngineState` pure functions; no host RNG/clock |
| **mesh-transport ↔ consensus** | `ExecutionResult` → `insert_node` / proofs; `merge_state` |
| **consensus ↔ interop** | roots, receipts, HTLC settlement |
| **mesh-transport ↔ frontend** | topology DTOs **via host**, never raw radio |

### Mocking policy

If a dependency crate is not ready, implement against logical contract types and ship a local mock. Keep `integration` green when replacing mocks.

---

## Execution Cadence

1. Branch `agent/NN-*` from `origin/main`.
2. Stay in directory / file ownership scope ([`AGENTS.md`](../AGENTS.md)).
3. Unit tests in-crate; smoke in `integration/`.
4. Push branch; open PR when contracts and smoke hold.
5. **Never force-push `main`.**

## Current Priority Order

1. Stabilize interface surfaces (contracts + this plan).
2. Phase 1 transport + portal acceptance paths.
3. Sandbox MEM_LIMIT + VDF gate.
4. Consensus merge + proof verify.
5. WASM engine + HTLC + TWAMM.
