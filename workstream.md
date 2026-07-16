# MossyMesh Parallel Workstreams

To allow multiple agents to work in parallel without merge conflicts, the work has been divided into independent workstreams based on the project phases outlined in the README. Each workstream is restricted to specific directories and technology stacks.

## Workstream A: Frontend Layer & Captive Portal
**Directory Scope**: `/frontend` and `/captive-portal`
**Core Technologies**: React, TypeScript, Vite, vite-plugin-pwa, nginx
**Agent Allocation**: 2-3 Agents
**Focus & Deliverables**:
- Develop the offline-first PWA for the MessyMash Chess application.
- Implement the Captive Portal redirection logic and UI.
- Configure `nginx` (with `client_max_body_size 150M`) and docker compose setups for serving assets.
- Build the React components for the chessboard and UI layout.

## Workstream B: Mesh Transport & Network Layer
**Directory Scope**: `/mesh-transport`
**Core Technologies**: Rust, Kademlia DHT, `reticulum-rs`, `lxmf-rs`
**Agent Allocation**: 3-4 Agents
**Focus & Deliverables**:
- Build the reticulum-rs daemon for identity-based routing.
- Implement Kademlia DHT pathfinding.
- Develop STUN-less hole punching for heavy lines and CSMA/CA / BLE wrappers for lightweight LoRa links.
- Set up the offline Wi-Fi domain logic.

## Workstream C: Consensus & Ledger (Data Layer)
**Directory Scope**: `/consensus` (needs to be created)
**Core Technologies**: Rust, `trie-db`, `nova-snark`, `yrs` (YATA CRDT), `ipld-core`
**Agent Allocation**: 3-4 Agents
**Focus & Deliverables**:
- Implement the Incremental Merkle-Patricia Trie datastore.
- Integrate `nova-snark` for recursive ZK-SNARK folding schemes to keep proofs constant-sized.
- Implement the `yrs` CRDT-based merging architecture for deterministic data sync across disconnected islands.
- Ensure the active ledger fits within the strict 10 MB RAM overhead constraint.

## Workstream D: Chess Engine Logic & WASM
**Directory Scope**: `/engine` (needs to be created)
**Core Technologies**: Rust, `shakmaty`, `wasm32-wasip1`
**Agent Allocation**: 2-3 Agents
**Focus & Deliverables**:
- Integrate the `shakmaty` engine for core bitboard evaluation.
- Compile the chess logic loop to the `wasm32-wasip1` target.
- Integrate `shakmaty-syzygy` for memory-mapped endgame tablebases.
- Ensure the engine can benchmark at ~836 Mnps in a WASM environment.

## Workstream E: Sandbox & Execution Environment
**Directory Scope**: `/sandbox` (needs to be created)
**Core Technologies**: WAMR, WASI, `minroot-vdf-rs`
**Agent Allocation**: 2-3 Agents
**Focus & Deliverables**:
- Deploy the WAMR (WebAssembly Micro Runtime) environment.
- Enforce the Symmetric Static INT8 Quantization and Fixed-Block Memory Pools.
- Enforce the 10 MB RAM cap via `wasm_runtime_full_init` and bounded aux stack (`-z stack-size=N`).
- Integrate `minroot-vdf-rs` to require burning a 10-minute sequential VDF for Ephemeral Job DIDs to prevent Sybil attacks.

## Workstream F: Interop, AI Processing & Smart Contracts
**Directory Scope**: `/interop` (needs to be created)
**Core Technologies**: Rust, Reticulum_AsyncAPI_rs, Vulkan Compute
**Agent Allocation**: 2-3 Agents
**Focus & Deliverables**:
- Build the `Reticulum_AsyncAPI_rs` endpoints.
- Implement Hashed Timelock Contracts (HTLCs) protected by VDF-Delayed Cancellation for escrowed credits.
- Spin up an OpenAPI gateway when internet reconnects.
- Prepare the TWAMM orchestration for bridging local liquidity to a global AMM.
- Standardize tensor formats and deterministic GPU computing.

---

### Collaboration Rules for Agents
1. **Directory Isolation**: Agents MUST ONLY modify files within their assigned `Directory Scope`.
2. **Interface Contracts**: If a workstream needs to interact with another (e.g., Frontend calling Mesh Transport), agents must agree on the API/Trait interfaces first and mock the responses until the dependent workstream is ready.
3. **No Centralized Assumptions**: Ensure all code strictly adheres to the "Out-of-Scope" rules in the README (no centralized IP addresses, DNS routing, or cloud databases).
4. **Testing**: Each workstream must include its own isolated unit tests before integrating.
