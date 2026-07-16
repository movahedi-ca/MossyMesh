# MossyMesh Agent Grid (16 Agents)

Authoritative roster for parallel work. Directory isolation is mandatory.
Interface contracts: [`docs/interface-contracts.md`](docs/interface-contracts.md).
SLAs and phase DoD: [`docs/sla-and-dod.md`](docs/sla-and-dod.md).

## Global Constraints (All Agents)

| SLA | Threshold |
| --- | --- |
| Active ledger RAM (edge) | ≤ **10 MB** |
| Unverifiable AI/compute outputs | < **1%** |
| Job timeout rate (unstable RF/env) | < **5%** |
| Centralized DNS / IP / cloud DBs | **Forbidden** |

Change control: any crypto-stack or memory-layout change must be proven not to violate the 10 MB ledger cap or determinism SLA before merging to `main`.

---

## Agent Matrix

| # | Branch | Role | Primary Scope | Workstream |
| ---: | --- | --- | --- | --- |
| 01 | `agent/01-architect` | Lead System Architect | `AGENTS.md`, `u/**`, `workstream.md`, `docs/**`, `integration/**` | Cross-cutting |
| 02 | `agent/02-transport` | LoRa & Reticulum Engineer | `mesh-transport/` (LoRa MAC, packet translate, identity, Wi-Fi Direct) | B Transport |
| 03 | `agent/03-network-topology` | Kademlia DHT Pathfinding Specialist | `mesh-transport/` (Kademlia, topology, STUN-less hole punch, BLE) | B Transport |
| 04 | `agent/04-portal` | Captive Portal Developer | `frontend/`, `captive-portal/` | A Frontend |
| 05 | `agent/05-logic` | Shakmaty Bitboard Optimizer | `engine/` | D Engine |
| 06 | `agent/06-consensus` | Merkle-Patricia Trie Engineer | `consensus/` (trie-db, IPLD/DAG-CBOR) | C Consensus |
| 07 | `agent/07-state-compression` | ZK-SNARK Folding Expert | `consensus/` (nova-snark folding, constant proofs) | C Consensus |
| 08 | `agent/08-cryptography` | VDF Cryptographer | `mesh-transport/` VDF + `sandbox/` job-DID gates | E / B |
| 09 | `agent/09-database` | CRDT Conflict Resolution Specialist | `consensus/` (yrs/YATA merge) | C Consensus |
| 10 | `agent/10-escrow` | Smart Contract / HTLC Developer | `interop/` (HTLC, VDF-delayed cancel) | F Interop |
| 11 | `agent/11-defi` | TWAMM Liquidity Architect | `interop/` (OpenAPI gateway, TWAMM, 2% max-spread) | F Interop |
| 12 | `agent/12-governance` | Web-of-Trust / DAO Governance | `interop/` + consensus hooks (vouching, multi-sig decay, ZK vote) | F / C |
| 13 | `agent/13-security` | Hardware Anomaly Detector | `mesh-transport/` (quarantine, honeypot, hash-chain, VRF weights) | B Transport |
| 14 | `agent/14-ai` | Edge AI Quantization Specialist | `sandbox/` + interop AI paths (INT8, SITF, PagedAttention) | E / F |
| 15 | `agent/15-frontend` | Frontend Chess UX / PWA | `frontend/` (chessboard, offline PWA, network UI) | A Frontend |
| 16 | `agent/16-devops` | Determinism Auditor & CI/Release | `integration/` smoke, CI gates, SLA regression harness | Cross-cutting QA |

### mesh-transport file ownership (Workstream B)

| Agent | Owns |
| --- | --- |
| 02 Transport | `lora_mac.rs`, `packet_translator.rs`, `identity_manager.rs`, `wifi_direct.rs`, `encryption_layer.rs`, `network.rs` |
| 03 Topology | `kademlia_routing.rs`, `topology.rs`, `stun_hole_punch.rs`, `ble_mesh.rs`, `simulation.rs` |
| 08 Cryptography | `vdf_sybil.rs` (+ sandbox VDF gate coordination) |
| 13 Security | `quarantine.rs`, `honeypot.rs`, `hash_chain.rs`, `thermal_aware.rs`, `battery_tracker.rs`, `vrf_assigner.rs` |

`mesh-transport/src/lib.rs` and `main.rs` are **integration surfaces** (re-export/init wiring only).

---

## Role Charters (Condensed)

1. **Architect** — change control, contracts, phase DoD, workstream board; no crate internals rewrites.
2. **LoRa & Reticulum** — physical/link layer; phone packet → LoRa → offline node (Phase 1 DoD).
3. **Kademlia Topology** — identity DHT, pathfinding, heavy-line hole punching.
4. **Captive Portal** — nginx portal (`client_max_body_size 150M`), docker compose, offline landing.
5. **Shakmaty Engine** — bitboards, wasm32-wasip1, ~836 Mnps, Syzygy mmap strategy.
6. **Merkle-Patricia Trie** — incremental trie, DAG-CBOR, active ledger ≤ 10 MB.
7. **ZK-SNARK Folding** — Nova-style constant-size verification proofs.
8. **VDF Cryptographer** — MinRoot sequential VDF (~10 min) for Ephemeral Job DIDs.
9. **CRDT Specialist** — yrs/YATA binary deltas; deterministic island merge.
10. **HTLC Developer** — escrowed credits + VDF-delayed cancellation.
11. **TWAMM Architect** — uplink OpenAPI bridge; TWAMM max-spread ≤ 2%.
12. **Governance** — WoT vouching, multi-sig decay, ZK-blinded edge voting.
13. **Anomaly / Security** — quarantine, honeypots, free-rider hash chains.
14. **Edge AI Quant** — symmetric static INT8, fixed-block pools, SITF tensors.
15. **Frontend UX** — chessboard, network status, offline-first PWA.
16. **Determinism Auditor** — cross-crate smoke, CI, <1% unverifiable protocol enforcement.

---

## Collaboration Protocol

1. Commit only within Primary Scope (plus owned files when sharing a crate).
2. Cross-crate APIs are defined in `docs/interface-contracts.md`; mock until peer is ready.
3. No cloud DBs, public DNS as sole locator, or fixed public IP control planes.
4. Unit tests per crate; `integration/` owns cross-crate smoke only.
5. Branch `agent/NN-*` from `origin/main`. **Never force-push `main`.**
6. SLA/contract conflicts → Architect (01) + Determinism Auditor (16).

---

## Crate Map ↔ Agents

```
frontend / captive-portal  →  04, 15
mesh-transport             →  02, 03, 08, 13
sandbox                    →  08, 14
engine                     →  05
consensus                  →  06, 07, 09
interop                    →  10, 11, 12, 14
docs / u / AGENTS / workstream / integration → 01, 16
```

## Related Docs

- [`workstream.md`](workstream.md)
- [`u/plan.md`](u/plan.md)
- [`u/full-info.md`](u/full-info.md)
- [`u/info.md`](u/info.md)
- [`docs/interface-contracts.md`](docs/interface-contracts.md)
- [`docs/sla-and-dod.md`](docs/sla-and-dod.md)
- [`docs/integration-smoke-plan.md`](docs/integration-smoke-plan.md)
