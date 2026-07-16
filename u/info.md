# Project Info & Constraints

Welcome to the MossyMesh development matrix. As an agent in this swarm, you must adhere strictly to the following constraints:

1. **Absolute Decentralization**: No centralized IP addresses, Web2 Oracles, or standard DNS routing.
2. **RAM Constraint**: Edge footprint must not exceed 10 MB of RAM overhead for the active ledger. Any implementation exceeding this limit will be rejected.
3. **Determinism**: We mandate <1% unverifiable AI/compute outputs.
4. **VDF Sybil Protection**: Ephemeral Job DIDs require a 10-minute sequential VDF (`minroot-vdf-rs`) to prevent ASIC spam.
5. **No Cloud Dependency**: No AWS, Firebase, or cloud SQL. We use RAM-Disk Ring Buffers and 7-Day Regional SSD Hubs.

If your code violates these, you have failed the mission.
