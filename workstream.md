# MossyMesh Parallel Workstreams

To allow multiple agents to work in parallel without merge conflicts, work is divided into independent workstreams.  
**Agent roster and file ownership:** [`AGENTS.md`](AGENTS.md)  
**Interface contracts:** [`docs/interface-contracts.md`](docs/interface-contracts.md)  
**SLA / DoD:** [`docs/sla-and-dod.md`](docs/sla-and-dod.md)

## Workstream A: Frontend Layer & Captive Portal
**Directory Scope**: `/frontend` and `/captive-portal`  
**Core Technologies**: React, TypeScript, Vite, vite-plugin-pwa, nginx  
**Agents**: 04 Portal, 15 Frontend UX  
**Focus & Deliverables**:
- Offline-first PWA for MessyMash Chess.
- Captive portal redirection and UI.
- nginx (`client_max_body_size 150M`) and docker compose for assets.
- Chessboard and network status UI consuming **host DTOs only** (see contracts §6–7).

## Workstream B: Mesh Transport & Network Layer
**Directory Scope**: `/mesh-transport`  
**Core Technologies**: Rust, Kademlia DHT, reticulum-rs / lxmf-rs direction  
**Agents**: 02 Transport, 03 Topology, 08 VDF (module), 13 Security  
**Focus & Deliverables**:
- Identity-based routing daemon surface.
- Kademlia DHT pathfinding.
- STUN-less hole punching; LoRa CSMA/CA & BLE wrappers.
- VRF assignment, VDF sybil gate, quarantine/honeypot/hash-chain.
- File ownership split documented in `AGENTS.md`.

## Workstream C: Consensus & Ledger (Data Layer)
**Directory Scope**: `/consensus`  
**Core Technologies**: Rust, trie-db path, nova-snark, yrs (YATA CRDT), ipld-core  
**Agents**: 06 Trie, 07 SNARK, 09 CRDT  
**Focus & Deliverables**:
- Incremental Merkle-Patricia datastore.
- Recursive ZK folding for constant-sized proofs.
- Deterministic island merge via CRDT/binary deltas.
- Active ledger ≤ **10 MB** (`MAX_LEDGER_SIZE`).

## Workstream D: Chess Engine Logic & WASM
**Directory Scope**: `/engine`  
**Core Technologies**: Rust, shakmaty, wasm32-wasip1  
**Agents**: 05 Logic  
**Focus & Deliverables**:
- shakmaty bitboard evaluation.
- Compile loop to wasm32-wasip1 for sandbox load.
- Syzygy mmap strategy; ~836 Mnps reference target.
- Pure deterministic APIs for sandbox FFI.

## Workstream E: Sandbox & Execution Environment
**Directory Scope**: `/sandbox`  
**Core Technologies**: WAMR, WASI, MinRoot VDF coordination  
**Agents**: 08 Cryptography (VDF gate), 14 AI Quantization  
**Focus & Deliverables**:
- WAMR environment with fixed-block pools.
- Symmetric static INT8 hooks.
- Enforce 10 MB via `MEM_LIMIT` / `wasm_runtime_full_init` path.
- Job admit only with VDF-backed Ephemeral Job DID.

## Workstream F: Interop, Credits & Bridges
**Directory Scope**: `/interop`  
**Core Technologies**: Rust, AsyncAPI/OpenAPI, HTLC, TWAMM  
**Agents**: 10 Escrow, 11 DeFi/TWAMM, 12 Governance  
**Focus & Deliverables**:
- AsyncAPI endpoints (`/api/v1/health`, `/api/v1/submit_job`, …).
- HTLC escrow with VDF-delayed cancellation.
- OpenAPI gateway on internet reconnect.
- TWAMM orchestration; **≤ 2%** max-spread cap.
- WoT / multi-sig decay / ZK vote hooks.

## Cross-Cutting (not a product workstream)
**Directory Scope**: `AGENTS.md`, `u/**`, `docs/**`, `workstream.md`, `integration/**`  
**Agents**: 01 Architect, 16 DevOps/Auditor  
**Focus**: contracts, SLA compliance, smoke harness, change control.

---

### Collaboration Rules for Agents
1. **Directory Isolation**: Modify only your assigned Directory Scope (and owned files if sharing a crate).
2. **Interface Contracts**: Agree on APIs in `docs/interface-contracts.md`; mock until the peer workstream is ready.
3. **No Centralized Assumptions**: No centralized IP/DNS-as-sole-locator or cloud databases.
4. **Testing**: Isolated unit tests per workstream; cross-crate smoke in `integration/`.
5. **Branches**: `agent/NN-*` from `origin/main`; never force-push `main`.
6. **Worktrees**: Prefer a dedicated git worktree per agent branch to avoid checkout thrash.
