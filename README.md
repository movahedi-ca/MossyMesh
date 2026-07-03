# MessyMash Master Blueprint (Version 8.0)
### *Asynchronous, Censorship-Resistant, Serverless Supercomputer & Open-Source Chess PoC*
### Official Website: MessyMash.com
## 1. Project Integration Management (The Charter)
**Mission Statement:** To build a self-healing mesh that turns any collection of phones, Raspberry Pis, PCs, and LoRa radios into a unified, decentralized compute grid operating completely independently of traditional ISPs, DNS servers, and fiat currencies.
**Proof of Concept (PoC):** The MessyMash offline-capable Chess application utilizing the shakmaty engine (capable of ~836 Mnps bitboard evaluation) to stress-test perfect state-transition determinism across highly heterogeneous hardware.
### Strict System SLAs & Constraints
 * **Target Capacity:** 100 Resilient Verifiable Compute-Hours (RVCH) per day per 20-node island, with zero upstream internet.
 * **Determinism Guarantee:** Less than 1% unverifiable AI/Compute outputs (Perfect Cross-Device Determinism).
 * **Reliability:** Less than 5% job timeout rate in highly unstable physical environments.
 * **Edge Footprint:** Strict maximum of 10 MB RAM overhead for the active ledger on edge devices.
**Integrated Change Control:** Any architectural changes to the cryptographic stack or memory allocations must be mathematically proven not to violate the 10 MB RAM edge constraint or the cross-device determinism SLA before merging.
## 2. Project Scope Management (Architecture & Baselines)
### In-Scope Technical Stack

| Architectural Layer | Core Technologies | Mesh Implementation & Guardrails |
| :--- | :--- | :--- |
| **Frontend Layer** | React, TypeScript, Vite, vite-plugin-pwa | Serves an offline-first PWA via a Captive Portal. nginx configured with client_max_body_size 150M for asset transfers. |
| **Application Logic** | Rust, shakmaty, shakmaty-syzygy, yrs | Core chess bitboard evaluation paired with memory-mapped endgame tablebases and YATA conflict-free replication. |
| **Execution Sandbox** | WAMR (wasm32-wasip1), WASI | Enforces **Symmetric Static INT8 Quantization** and **Fixed-Block Memory Pools** via wasm_runtime_full_init. Bounded aux stack (-z stack-size=N). |
| **Transport Layer** | reticulum-rs, lxmf-rs, Kademlia DHT | Replaces IP with identity-based routing. Heavy lines use STUN-less hole punching; lightweight uses LoRa (CSMA/CA) & BLE. |
| **Ledger Consensus** | trie-db, ipld-core, serde_ipld_dagcbor | Incremental Merkle-Patricia Trie datastore utilizing serialized compact DAG-CBOR formats and cryptographic pointers. |
| **State Compression** | nova-snark | Recursive ZK-SNARK folding scheme over Pallas/Vesta curves. Keeps proofs constant-sized to drop old ledger histories. |
| **AI Processing** | SITF, Edge PagedAttention, Vulkan Compute | Standardized tensor formats, disk-mapped context windows, and deterministic GPU computing for high-compute tiers. | <br> ### Out-of-Scope (Exclusions) <br> * Any reliance on centralized IP addresses, Web2 Oracles, or standard DNS routing. <br> * Centralized cloud databases (e.g., AWS, Firebase). Data availability must rely on RAM-Disk Ring Buffers (Ephemeral DA with LRU eviction) and 7-Day Regional SSD Hubs utilizing Append-Only Logs secured by Reed-Solomon Erasure Coding. <br> ## 3. Project Schedule Management (WBS & Roadmap) <br> Execution is sequenced over an iterative, critical path timeline based on a 10-hour/week commitment.




| Project Phase | Focus & Deliverables | Definition of Done (DoD) & Acceptance Criteria |
| :--- | :--- | :--- |
| **Phase 1: Transport** | Offline Wi-Fi domains, Captive Portal redirection, and reticulum-rs daemon builds. | A smartphone test packet successfully translates to a LoRa transmission and routes to an offline node using Kademlia DHT pathfinding. |
| **Phase 2: Sandbox** | Integration of minroot-vdf-rs and WAMR environment deployment. | WAMR strictly enforces the 10 MB RAM cap via wasm_runtime_full_init and creating an Ephemeral Job DID requires burning a 10-minute sequential VDF. |
| **Phase 3: Consensus** | Deployment of trie-db, nova-snark, and yrs CRDT-based merging architectures. | Edge nodes successfully verify the ledger via a sub-megabyte constant proof and disconnected islands merge data deterministically via binary deltas. |
| **Phase 4: Logic** | Compile shakmaty loop to wasm32-wasip1 and bring lxmf-rs messaging online. | The WASM chess engine benchmarks at ~836 Mnps. Escrowed credits use Hashed Timelock Contracts (HTLCs) protected by VDF-Delayed Cancellation. |
| **Phase 5: Interop** | UI layout serving, Reticulum_AsyncAPI_rs endpoints, and TWAMM orchestration. | Reconnecting to the internet spins up an OpenAPI gateway, bridging local liquidity to a global AMM using a TWAMM with a strict 2% max-spread cap. | <br> ## 4. Project Cost & Resource Management <br> ### Initial Hardware Baseline


| Quantity | Device Tier | Core Hardware Target | Estimated Cost (USD) |
| :--- | :--- | :--- | :--- |
| 2 Units | **Pi-Tier (Genesis Nodes)** | Raspberry Pi Zero 2 W, Raspberry Pi 4, or 5 | ~$150.00 |
| 3 Units | **Edge / IoT Tier** | ESP32 Microcontrollers with SX1262 LoRa transceivers | ~$60.00 |
| 1 Unit | **Regional Hub** | NVMe-equipped High-Capacity Mini PC | ~$250.00 |
| – | **Physical Layer Gear** | Power banks, HF Ham Radio links, high-gain antennas | ~$150.00 |
| 12 Mos | **SaaS & Tooling** | Pro-tier AI assistants and developer workspace subscriptions | ~$360.00 / yr |
| **Total** |  | **Initial Outlay & Baseline Projection** | **~$970.00** | <br> ### Labor Baseline (Sweat Equity) <br> 10 hours/week over 2.5 years equates to ~1,250 development hours. Evaluated at a senior system architect market rate ($100/hr), the total sweat equity project baseline valuation is **$125,000**. <br> ## 5. Project Quality Management (QA & Control) <br> To enforce the <1% unverifiable output SLA, the following protocol checks are automated in code: <br> * **Intelligent VRF Assignment:** Tasks are routed via a Commit-and-Reveal seed using Least-Loaded-First logic, Battery-Curve Weighting (heavy layers to AC-powered nodes), and Thermal-Aware routing (deprioritizing CPUs over 75°C). <br> * **Dynamic Triangulation:** The VRF assigns 3 Primary and 2 Standby workers per job. Standbys instantly replace dropping primaries without requiring a DAG restart. <br> * **Free-Rider Prevention:** Each node must submit Cryptographic Hash Chains of their WASM execution trace to prove actual computation occurred, rather than simple data forwarding. <br> * **Hardware Quarantine:** Tensors undergo Statistical Anomaly Detection. Nodes failing 3 checks are forced into Quarantine to run a 1-hour hardware diagnostic benchmark checking for silent CPU decay. <br> * **Cartel Eradication:** Hubs silently replay historically verified jobs via Onion-Routed Honeypots. Unproven cartels agreeing on fake hashes are instantly slashed and banned. <br> ## 6. Project Risk Management (Risk Register)

| Risk Event | Impact | Probability | Mitigation Strategy (Response Plan) |
| :--- | :--- | :--- | :--- |
| **Apple iOS Wi-Fi Drops (Kernel Panics)** | Critical | High | **Mitigate:** Mandate sudo rpi-update patches; force systemd-timesyncd offline time sync to local NTP prior to deployment. |
| **Upstream Dependency Abandonment** | High | Medium | **Mitigate:** "Fork and Maintain" strategy. Vendor or fork core crates like reticulum-rs directly to the project organization to shield against bit-rot. |
| **ASIC/GPU Spam Farms (Sybil Attacks)** | High | Medium | **Mitigate:** Require memory-hard MinRoot VDF calculations (x \to x^{1/5} \pmod p) to generate Ephemeral Job DIDs, neutralizing parallel hardware. |
| **Storage Bloat Exhausting Edge RAM** | High | Medium | **Mitigate:** Utilize nova-snark MicroSpartan preprocessing to ensure verification circuits remain constant-sized (~10,000 gates). |
| **Regional SSD Hub Destruction** | High | Low | **Mitigate:** Securely anchor 200-byte ZK-SNARK ledger proofs to neighboring macro-islands via High-Frequency (Ham) radio bursts up to 300 miles away. |
| **Solo Developer Burnout** | Critical | High | **Mitigate:** Adhere strictly to the 10-hour/week allocation constraint. Enforce sequential, phase-by-phase completion to minimize context-switching. |

## 7. Procurement, Governance & Stakeholders
### Procurement Strategy (Make vs. Buy)
 * **Open-Source Integration over Custom Build:** To maximize productivity under a strict hobby time allocation, the project explicitly rejects "Not Invented Here" syndrome. The core engine integrates battle-tested components (shakmaty for bitboards and yrs for CRDT document updates) to save hundreds of custom engineering hours.
 * **Sneakernet Procurement Logistics:** Large assets (like 4GB AI Base Models) physically bypass severe radio frequency limits using air-gapped human couriers with High-Capacity USB Flash Drives. Subsequent delta updates use Radio-transmitted AI LoRA Weight Patching (<10MB).
### Governance & Economy
 * **Web of Trust (WoT) Onboarding:** New nodes require a voucher who locks Quadratic Staking collateral. Vouchers are financially slashed if their invitees behave maliciously.
 * **Liquid DAO Governance:** The network initiates with a 3-of-5 admin multi-sig that mathematically decays to zero authority over 90 days. Control transitions to ZK-Blinded Voting by verified edge nodes.
 * **Incentives:** Genesis nodes operating entirely offline earn Retroactive AMM Liquidity Mining points, resulting in airdropped governance tokens upon internet reconnection.
```
