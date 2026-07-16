# MossyMesh Master Documentation

*This document serves as the single source of truth for the swarm, combining the README charter and workstream definitions.*

## The Mission
To build a self-healing mesh that turns any collection of phones, Raspberry Pis, PCs, and LoRa radios into a unified, decentralized compute grid operating completely independently of traditional ISPs, DNS servers, and fiat currencies. Our Proof of Concept is the offline-capable MessyMash Chess application utilizing `shakmaty`.

## Core Technologies
- **Frontend**: React, TypeScript, Vite, vite-plugin-pwa, nginx (150M body size)
- **Engine Logic**: Rust, shakmaty, wasm32-wasip1
- **Sandbox**: WAMR, WASI, Symmetric Static INT8 Quantization
- **Transport**: reticulum-rs, lxmf-rs, Kademlia DHT
- **Consensus**: trie-db, nova-snark, yrs
- **AI Processing**: SITF, Edge PagedAttention, Vulkan Compute

## Collaboration Rules
- **Directory Isolation**: Only touch files within your assigned Directory Scope (e.g., Workstream A only touches `/frontend` and `/captive-portal`).
- **Interface Contracts**: Mock dependent APIs if the adjacent workstream hasn't finished them yet.
- **No Centralized Assumptions**: Adhere to the "Out-of-Scope" rules. 
- **Testing**: Include isolated unit tests.
