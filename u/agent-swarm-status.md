# MossyMesh 16-Agent Swarm — 2026-07-17

**Mode:** caveman (short, ship, push)
**Goal:** finish open PR boxes, never lose work near weekly limit

## Open boxes (MUST close)

1. **P4-WASM** — Engine compiles `wasm32-wasip1` AND loads in sandbox
2. **P4-MSG** — Mesh messaging: job distribute + result collect

## Agent map

| # | Branch | Mission |
|---|--------|---------|
| 01 | agent/01-architect | contracts, SLA boxes, ownership, DoD check when green |
| 02 | agent/02-transport | **P4-MSG core**: messaging module job distribute |
| 03 | agent/03-network-topology | **P4-MSG**: route jobs via kademlia/topology |
| 04 | agent/04-portal | portal e2e polish; no block on P4 |
| 05 | agent/05-logic | **P4-WASM core**: cdylib/exports, wasip1 build |
| 06 | agent/06-consensus | trie tests; commit receipts for job results |
| 07 | agent/07-state-compression | fold proofs for job receipts |
| 08 | agent/08-cryptography | VDF gate stays green with messaging path |
| 09 | agent/09-database | CRDT merge of job result islands |
| 10 | agent/10-escrow | HTLC path still works with job pipeline |
| 11 | agent/11-defi | TWAMM regression only |
| 12 | agent/12-governance | multisig/vote regression only |
| 13 | agent/13-security | hash-chain on job results; quarantine hooks |
| 14 | agent/14-ai | **P4-WASM sandbox load** real/module path |
| 15 | agent/15-frontend | UX status for job submit if needed |
| 16 | agent/16-devops | integration smoke for P4-WASM + P4-MSG; CI |
| **PUSH** | (all branches) | **continuous push** every ~60s — never lose WIP |

## Rules

1. Own scope only (AGENTS.md).
2. Commit often. Push often.
3. No force-push main.
4. Branch from origin/main: `agent/NN-*`.
5. When P4 green: agent 01 checks boxes in `docs/sla-and-dod.md` + CHANGELOG.
6. Tests: `cargo test -p <crate>` before claim done.

## Definition of done (boxes)

### P4-WASM done when
- `cargo build -p engine --target wasm32-wasip1` succeeds (cdylib → `.wasm` preferred)
- sandbox can `Job::load` / `admit_and_load` those module bytes (magic `\0asm` validated)
- integration smoke covers load path
- SLA checkbox checked

### P4-MSG done when
- `mesh-transport` has job distribute + result collect API
- unit tests green
- integration or transport test covers path
- SLA checkbox checked
