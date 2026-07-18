# SLAs & Definition of Done (Phases 1–5)

**Owner:** Architect (`agent/01-architect`) with Determinism Auditor (`agent/16-devops`)  
**Charter thresholds:** root `README.md`.

---

## Global SLAs (All Phases)

| ID | SLA | Target | Measurement |
| --- | --- | --- | --- |
| SLA-RAM | Active ledger footprint | ≤ **10 MB** (10_000_000 bytes; sandbox `MEM_LIMIT` = 10 MiB) | Probes; reject unbounded active set |
| SLA-DET | Verifiable compute | < **1%** unverifiable outputs | Trace hash + re-exec / proof sample |
| SLA-TO | Job reliability | < **5%** timeout | Telemetry (sim OK early) |
| SLA-CAP | Capacity | **100 RVCH/day** / 20-node island, zero upstream | Bench (Phase 4+) |
| SLA-DEC | Decentralization | No sole public DNS / fixed IP CP / cloud DB | Design + dependency audit |

### Quality protocol (continuous)

- VRF: 3 primary + 2 standby; battery- and thermal-aware (deprioritize >75°C).
- Hash-chain execution traces.
- Anomaly → quarantine after 3 fails → 1h diagnostic.
- Honeypot replay for cartels.

### Change control

Crypto or memory allocation changes require written proof (doc or test) that SLA-RAM and SLA-DET remain intact before `main`.

---

## Phase 1 — Transport

**Focus:** Offline Wi-Fi domains, captive portal, reticulum-style daemon, LoRa/BLE/Kademlia.

### Deliverables

- [x] Captive portal serves offline PWA (nginx body size ≥ 150M).
- [x] Identity-based routing (no public DNS required for island traffic).
- [x] Smartphone test packet → translator → LoRa MAC framing.
- [x] Kademlia pathfinding to offline node (sim acceptable).
- [x] Unit tests for MAC/translate/DHT modules.

### Acceptance

| ID | Pass condition |
| --- | --- |
| P1-DoD-1 | Test packet translates and routes via Kademlia to second offline peer (hw or sim). |
| P1-DoD-2 | Device joining Wi-Fi island gets portal UI without upstream Internet. |
| P1-DoD-3 | No required cloud DB or public DNS for island routing. |
| P1-DoD-4 | `integration` init path runs without panic (stub OK). |

---

## Phase 2 — Sandbox

**Focus:** WAMR/WASI, fixed-block pools, MinRoot VDF job DIDs.

### Deliverables

- [x] WAMR (or faithful stub → WAMR) with bounded linear memory.
- [x] Symmetric static INT8 policy hooks.
- [x] Ephemeral Job DID gated by ~10-minute sequential VDF.
- [x] `allocate` hard-fails past cap with deterministic error.

### Acceptance

| ID | Pass condition |
| --- | --- |
| P2-DoD-1 | Guest heap ≤ 10 MB; over-alloc → `Err`, not UB. |
| P2-DoD-2 | Job DID requires successful MinRoot VDF verify. |
| P2-DoD-3 | OOM and missing-export codes stable across nodes. |
| P2-DoD-4 | Sandbox + VDF unit tests green; integration execute path returns stub bytes. |

---

## Phase 3 — Consensus

**Focus:** trie-db, nova-snark folding, yrs/CRDT island merge.

### Deliverables

- [x] Incremental Merkle-Patricia (or equivalent) active ledger.
- [x] Constant-size (or bounded sub-MB) edge verification proof.
- [x] Deterministic merge of divergent islands.
- [x] Active ledger size accounting vs SLA-RAM.

### Acceptance

| ID | Pass condition |
| --- | --- |
| P3-DoD-1 | Peer verifies update via sub-megabyte constant/bounded proof. |
| P3-DoD-2 | Two partitions converge to identical root after merge. |
| P3-DoD-3 | Active ledger ≤ 10 MB under documented workload. |
| P3-DoD-4 | insert → prove → verify round-trip in tests. |

---

## Phase 4 — Logic (Engine + Messaging)

**Focus:** shakmaty → wasm32-wasip1, messaging, HTLC escrow.

### Deliverables

- [ ] Engine compiles to wasm32-wasip1 and loads in sandbox. <!-- SMK-09; 05+14+16; unchecked until proven -->
- [x] Deterministic legal moves + evaluation.
- [x] Mnps harness (~836 target on reference profile).
- [x] HTLC + VDF-delayed cancellation.
- [ ] Mesh messaging for job distribute / result collect. <!-- SMK-10; 02+03+16; unchecked until proven -->

### Acceptance

| ID | Pass condition |
| --- | --- |
| P4-DoD-1 | Same FEN corpus → identical outputs on two hosts in sandbox. |
| P4-DoD-2 | Benchmark ~836 Mnps on agreed profile (or justified gap). |
| P4-DoD-3 | Escrow open → work → claim/refund tested; VDF cancel enforced. |
| P4-DoD-4 | Sampled re-exec mismatch < 1% on chess job corpus. |

**Close criteria (architect):** check boxes only when SMK-09 / SMK-10 in [`integration-smoke-plan.md`](integration-smoke-plan.md) pass on landed code; update [`CHANGELOG.md`](../CHANGELOG.md) Still open. Contracts: [`interface-contracts.md`](interface-contracts.md) §2a–2b.

---

## Phase 5 — Interop

**Focus:** Mature UI serving, AsyncAPI endpoints, TWAMM on reconnect.

### Deliverables

- [x] Stable AsyncAPI/OpenAPI submit/sync/health.
- [x] Uplink gateway without abandoning island autonomy.
- [x] TWAMM bridge with **≤ 2%** max-spread.
- [x] Frontend wired to host API for chess + network status.

### Acceptance

| ID | Pass condition |
| --- | --- |
| P5-DoD-1 | With uplink, gateway serves; without, island APIs still work. |
| P5-DoD-2 | Simulated trade path enforces max-spread ≤ 2%. |
| P5-DoD-3 | Captive-portal chess works offline E2E against local host. |
| P5-DoD-4 | SLA-RAM/DET/TO checks automated in CI smoke (even if simulated). |

---

## PR Acceptance Checklist (Architect View)

1. [ ] Touches only allowed directory scope for the agent branch.
2. [ ] No new centralized DNS/IP/cloud DB dependency.
3. [ ] Interface changes reflected in `docs/interface-contracts.md`.
4. [ ] Unit tests added/updated for changed behavior.
5. [ ] `integration` smoke still designed to pass (or updated).
6. [ ] SLA-RAM / SLA-DET impact noted when crypto or memory touched.
7. [ ] No secrets committed.

---

## Phase → Workstream Map

| Phase | Primary workstreams |
| --- | --- |
| 1 Transport | A Frontend/Portal, B Mesh Transport |
| 2 Sandbox | E Sandbox, B (VDF), AI hooks |
| 3 Consensus | C Consensus |
| 4 Logic | D Engine, F HTLC, B messaging |
| 5 Interop | F Interop/TWAMM, A Frontend polish |
