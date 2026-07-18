# Welcome to the MossyMesh Wiki!

**MossyMesh** is an offline-first, decentralized chess engine that runs entirely on a mesh captive portal. It allows users to play chess (and eventually other turn-based applications) completely offline by syncing moves through physical mesh relays (LoRa, BLE, and Wi-Fi Direct).

## 🌍 The Vision
Traditional multiplayer games require central servers and internet access. MossyMesh imagines a world where digital communities can interact and sync state even when completely isolated from the upstream internet. 

We achieve this by embedding a full WebAssembly sandboxed engine (`shakmaty`) inside a Rust-based routing daemon, allowing nearby devices to connect to an offline Wi-Fi access point, load the React PWA via a Captive Portal, and play.

## 📚 Key Concepts

- **Captive Portal:** Connect to the `MossyMesh_Local` Wi-Fi to immediately load the app without an app store or internet.
- **Mesh Transport:** If your opponent is far away, moves are routed through Kademlia DHT over LoRa or BLE.
- **Ledger Compression:** All game states are compressed into a deterministic Merkle-Patricia Trie that strictly remains under 10 MB on edge nodes.
- **OpenAPI Interop:** When a node reconnects to the upstream internet, the OpenAPI Gateway wakes up to bridge offline credits to global AMMs.

## 📖 Wiki Navigation

- **[Architecture Overview](Architecture.md)**
- **[Networking & Transport Layer](Networking_Layer.md)**
- **[Consensus & Ledger](Consensus_and_Ledger.md)**
- **[Sandbox & WASM Execution](Sandbox_and_WASM.md)**
- **[DeFi & Interop Bridging](DeFi_and_Interop.md)**
- **[Developer Guide](Developer_Guide.md)**
- **[Agent Grid & Collaboration](Agent_Grid.md)**
