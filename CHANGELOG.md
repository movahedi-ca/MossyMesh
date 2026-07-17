# Changelog

Notable mainline merges and features (newest first). Scope is high-level; see git history for full detail.

## Unreleased (docs)

- Refresh `docs/sla-and-dod.md` deliverable checkboxes against verified mainline code.

## 2026-07 — Mainline feature landings

### Consensus / state compression
- **Merkle-Patricia Trie** (Blake3, DAG-CBOR-ish nodes), inclusion proofs, island merge, `MAX_LEDGER_SIZE` (10_000_000).
- **Constant-size SNARK folding** mock (200-byte public anchor; real `nova-snark` optional) + fold/verify APIs.

### Cryptography / sandbox admit
- **MinRoot VDF** (production modulus + ~50M-step delay target) for Sybil-resistant Ephemeral Job DIDs.
- Sandbox **job admit gate** (`VdfReceipt` / `JobDid`) with stable error codes; hash-PoW stub for tests.
- Identity AEAD (ChaCha20-Poly1305) and commit-reveal **VRF assigner**.

### Sandbox
- **Fixed-block pool** enforcing `MEM_LIMIT = 10 MiB`; deterministic OOM on over-alloc.
- Symmetric static **INT8** quant helpers; pure-Rust host sim + optional `wamr` feature surface.

### Interop / credits
- **HTLC** escrow with claim / refund / **VDF-delayed cancel**.
- **TWAMM** stream engine with hard **≤ 2%** max-spread (`MAX_SPREAD_BPS = 200`).
- OpenAPI gateway dormant offline; activates on internet reconnect without dropping island APIs.

### Transport
- Identity-based routing, **Kademlia** XOR DHT (sim), LoRa MAC + smartphone packet translator.
- Captive portal nginx (`client_max_body_size 150M`) + offline PWA shell.

### Engine / integration
- Deterministic chess legal moves, eval, negamax search, measured **Mnps** harness (836 aspirational).
- Expanded **integration** smoke (MEM_LIMIT, MPT prove/merge, SNARK fold, HTLC types, job pipeline stub).

### Still open (SLA checkboxes left unchecked)
- Real engine **wasm32-wasip1** binary loaded in sandbox (host sim stub only today).
- Full mesh **messaging** path for job distribute / result collect (beyond identity announce + REST stub).
