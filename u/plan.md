# Swarm Execution Plan

This document outlines the phased, parallel execution strategy for the MossyMesh AI Swarm. The goal is to distribute workstreams among 16 specialized agents to hit our constraints (10MB RAM cap, strict determinism) in record time.

## Phase 1: Parallel Foundation (Current)
- **Workstream A (Frontend/Captive Portal)**: Agents build the Vite React PWA and nginx landing page logic.
- **Workstream B (Mesh Transport)**: Agents build Kademlia DHT pathfinding in Rust.
- **Workstream C (Consensus/Ledger)**: Agents stub out `trie-db` and SNARK configurations.
- **Workstream D (Engine Logic)**: Agents wire up the `shakmaty` engine into WASM.
- **Workstream E (Sandbox)**: Agents set up the WAMR environment.
- **Workstream F (Interop)**: Agents mock AsyncAPI endpoints.

## Phase 2: Integration & Constraint Enforcement
- Ensure cross-boundary APIs are respected (e.g., Frontend calling WASM from Workstream D).
- Execute the RAM ceiling checks: Active ledger must not exceed 10 MB.
- Deploy testing framework to verify 836 Mnps evaluation limit.

## Phase 3: Final Polishing & Verification
- Test PWA offline capabilities.
- Validate STUN-less hole punching under heavy lines.
- Complete VDF integration for Sybil resistance.
