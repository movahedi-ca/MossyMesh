# Project Info & Constraints

Welcome to the MossyMesh development matrix. Every agent must adhere to these constraints.

## Hard SLAs

1. **Absolute Decentralization**: No centralized IP control planes, Web2 oracles, or standard public DNS as the sole locator.
2. **RAM Constraint**: Edge active ledger ≤ **10 MB**. Implementations that exceed this are rejected.
3. **Determinism**: < **1%** unverifiable AI/compute outputs.
4. **Reliability**: < **5%** job timeout rate in unstable environments.
5. **VDF Sybil Protection**: Ephemeral Job DIDs require a ~10-minute sequential MinRoot VDF.
6. **No Cloud Dependency**: No AWS, Firebase, or cloud SQL. Use RAM-disk ring buffers and regional SSD hubs (append-only + erasure coding).

## Where to Look

| Need | Document |
| --- | --- |
| Who owns what | [`AGENTS.md`](../AGENTS.md) |
| Cross-crate APIs | [`docs/interface-contracts.md`](../docs/interface-contracts.md) |
| Phase DoD checklists | [`docs/sla-and-dod.md`](../docs/sla-and-dod.md) |
| Smoke tests | [`docs/integration-smoke-plan.md`](../docs/integration-smoke-plan.md), crate `integration/` |
| Execution plan | [`u/plan.md`](plan.md) |
| Full architecture | [`u/full-info.md`](full-info.md) |

If your code violates the SLAs or contracts, it fails change control.
