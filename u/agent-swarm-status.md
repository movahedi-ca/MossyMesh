# MossyMesh 16-Agent Swarm — 2026-07-17

**Mode:** caveman (short, ship, push)
**Goal:** finish open PR boxes, never lose work near weekly limit
**Last architect poll:** 2026-07-17 (agent/01-architect)

## Open boxes (MUST close)

1. **P4-WASM** — Engine compiles `wasm32-wasip1` AND loads in sandbox → **OPEN**
2. **P4-MSG** — Mesh messaging: job distribute + result collect → **OPEN** (WIP seen, not landed)

## Poll snapshot

| Box | Evidence | Status |
|-----|----------|--------|
| P4-WASM | engine wasip1 notes still rlib-oriented; sandbox WIP mentions `wasm_module` / magic validate in dirty trees but **not proven on origin/main**; host sim accepts `\0asm` stubs | **OPEN** — 05 + 14 + 16 |
| P4-MSG | WIP `messaging.rs` / JobMeshBus seen in shared worktree at times; **not stable on origin/agent/02 or main** | **OPEN** — 02 commit+push; 03 route; 16 smoke |

**Do not check SLA boxes until:** code on branch/main, `cargo test` green, smoke plan SMK-09 / SMK-10 satisfied.

## Agent map

| # | Branch | Mission | Notes |
|---|--------|---------|-------|
| 01 | agent/01-architect | contracts, SLA boxes, ownership, DoD check when green | contracts + smoke plan P4 paths updated |
| 02 | agent/02-transport | **P4-MSG core**: messaging module job distribute | **ship messaging.rs** + wire `lib.rs` + push |
| 03 | agent/03-network-topology | **P4-MSG**: route jobs via kademlia/topology | feed `NodeId` list into distribute |
| 04 | agent/04-portal | portal e2e polish; no block on P4 | |
| 05 | agent/05-logic | **P4-WASM core**: cdylib/exports, wasip1 build | need loadable `.wasm` artifact |
| 06 | agent/06-consensus | trie tests; commit receipts for job results | |
| 07 | agent/07-state-compression | fold proofs for job receipts | |
| 08 | agent/08-cryptography | VDF gate stays green with messaging path | |
| 09 | agent/09-database | CRDT merge of job result islands | |
| 10 | agent/10-escrow | HTLC path still works with job pipeline | |
| 11 | agent/11-defi | TWAMM regression only | |
| 12 | agent/12-governance | multisig/vote regression only | |
| 13 | agent/13-security | hash-chain on job results; quarantine hooks | |
| 14 | agent/14-ai | **P4-WASM sandbox load** real/module path | magic `\0asm` validate + load engine bytes |
| 15 | agent/15-frontend | UX status for job submit if needed | |
| 16 | agent/16-devops | integration smoke SMK-09 + SMK-10; CI | |
| **PUSH** | (all branches) | **continuous push** every ~60s — never lose WIP | shared worktree thrash — prefer dedicated worktrees |

## Rules

1. Own scope only (AGENTS.md).
2. Commit often. Push often.
3. No force-push main.
4. Branch from origin/main: `agent/NN-*`.
5. When P4 green: agent 01 checks boxes in `docs/sla-and-dod.md` + CHANGELOG.
6. Tests: `cargo test -p <crate>` before claim done.
7. **Architect never rewrites crate internals** — docs/contracts/integration plan only.

## Definition of done (boxes)

### P4-WASM done when
- `cargo build -p engine --target wasm32-wasip1` succeeds (cdylib → `.wasm` preferred)
- sandbox can `Job::load` / `admit_and_load` those module bytes (magic `\0asm` validated)
- integration smoke covers load path (SMK-09)
- SLA checkbox checked by 01 + CHANGELOG Still open cleared

### P4-MSG done when
- `mesh-transport` has job distribute + result collect API (`messaging` module)
- unit tests green
- integration or transport test covers path (SMK-10)
- SLA checkbox checked by 01 + CHANGELOG Still open cleared

## Architect actions log

| When | Action |
|------|--------|
| poll | Peers not proven on origin — boxes stay unchecked |
| docs | `interface-contracts.md` §2a messaging + §2b wasm load |
| docs | `integration-smoke-plan.md` SMK-09 / SMK-10 |
| warn | Shared `C:\Users\mhmov\MossyMesh` thrash — agents steal checkout; use worktrees |
| next | Re-poll when 02/05/14 push; then check boxes |
