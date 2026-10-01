# MossyMesh

MossyMesh is a Rust workspace of mesh-networking and compute libraries, plus one
daemon that wires them together and one HTTP gateway on top. It is not a mesh
network you can join today. This README describes what the code actually does,
checked against the tree at HEAD.

The old README (the "MessyMash Master Blueprint") described a long-term vision:
LoRa relays, ham-radio ZK bursts, 100 RVCH/day SLAs. None of that runs. It is
kept for reference in [docs/vision-blueprint.md](docs/vision-blueprint.md),
marked as future vision, not current capability.

## What works

**mesh-daemon binary.** Boots and serves. A CI smoke test boots the real binary
and asserts `GET /api/v1/health` returns 200 (`devops/smoke-boot.sh`).

**Real node-to-node p2p path.** Set `MESH_LISTEN_ADDR` (for example
`127.0.0.1:19091`) and the daemon runs a live libp2p swarm: TCP transport,
Noise encryption, Yamux multiplexing, active ping. A second daemon dials it
with `MESH_PEER_ADDR` set to the first node's full multiaddr ending in
`/p2p/<peer-id>`. A CI smoke test boots two daemons, dials one to the other,
and requires real ping packets over real sockets in both directions
(`devops/smoke-p2p.sh`). Supporting env vars: `MESH_NODE_SEED` (64 hex chars,
pins the peer identity), `MESH_STATUS_FILE` (JSONL status output),
`MESH_GATEWAY_BIND` (HTTP bind address).

**HTTP gateway.** Axum server serving `GET /api/v1/health`,
`POST /api/v1/submit_job`, and `/api-docs/openapi.json` with Swagger UI. Auth
and rate limiting exist on the job endpoint. It binds loopback and fails closed
otherwise.

**Chess engine (`engine` crate).** Real shakmaty 0.27 bitboards: legal move
generation, checkmate detection, as a library.

**Consensus trie (`consensus` crate).** Hand-rolled Merkle-Patricia trie over
Blake3, radix-16, as a library. Note: the old README credited `trie-db` and
`ipld-core`; the code uses neither.

**VDF Sybil gate (`mesh-transport`).** Real Wesolowski VDF over RSA-2048 with
logarithmic verification. Caveat, stated plainly: RSA-2048 needs a trusted
setup. The factors were claimed destroyed in 2001; anyone who learns them can
evaluate the VDF instantly, mint identity proofs without the delay, and the
Sybil gate collapses. The zero-trusted-setup alternative (Pietrzak over class
groups) is archived as follow-up work. Full math and the rotation plan are in
[docs/math-wesolowski-vdf.md](docs/math-wesolowski-vdf.md).

## What is partial

**HTTP job queue.** `POST /api/v1/submit_job` returns 200 and appends the job
to an in-memory `VecDeque`. Nothing in the running daemon drains it. Jobs are
accepted, not executed.

**Frontend (Vite + React + TypeScript, PWA).** Builds clean and renders a
chessboard. Its backend contract is broken in two places: after a 200 from
`/api/v1/submit_job` it shows "Move confirmed by swarm", while the move sits in
the undrained outbox; and it fetches `/api/v1/engine_eval`, an endpoint the
backend does not define, which 404s.

**Sandbox.** The default is an honest, documented host *simulation* of the WASI
surface: fixed-block heap, bounded aux stack. Native WAMR FFI exists behind a
feature gate and is not linked in the default build.

**BLE peripheral.** Real peripheral code (567 lines) plus a hardware
abstraction layer and 25 tests. Not wired into the daemon, and never tested
against real radio hardware.

**TWAMM / HTLC (`interop` crate).** The math is real and tested: spread caps,
order slicing, timelocks. There is no market, no counterparty, and no network
for it to run on.

**Kademlia DHT, LoRa, WiFi-Direct, STUN-less hole punch (`mesh-transport`).**
Real library helpers with tests. The daemon does not use them for live traffic;
the only live transport is the libp2p path above.

## What is a prototype

**SNARK / Nova-style folding (`consensus` crate).** Deterministic mock provers.
The code says so, and so does this README. No real nova-snark proofs are
produced.

**Captive portal.** Static files only. No portal logic runs.

## What is unwired

**`governance` crate.** Compiles, 47 tests (multisig, voting, staking,
web-of-trust). The daemon never initializes it.

**`ai` crate.** Compiles, 29 tests. The Vulkan compute backend is a stub, labeled
as one in the code. The daemon never initializes it.

## What is a demo

When `MESH_LISTEN_ADDR` is not set, the daemon boots in demo mode: it inserts
8 fake peers (`mesh://boot/1` through `mesh://boot/8`) into an in-memory table
and runs a scripted WiFi-Direct negotiation with hardcoded weights (local 950
vs fake peer 150, so the local node always wins). The Kademlia, BLE, and
hole-punch boot lines describe in-memory sketches, not live routing or live
radio. Treat that output as a demo, not a network.

## Build, test, run

```sh
cargo build --workspace          # one binary: mesh-daemon
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
./target/debug/mesh-daemon       # demo boot + HTTP gateway
# Real p2p (two terminals):
MESH_LISTEN_ADDR=127.0.0.1:19091 MESH_STATUS_FILE=/tmp/a.jsonl ./target/debug/mesh-daemon
MESH_LISTEN_ADDR=127.0.0.1:19092 MESH_PEER_ADDR=/ip4/127.0.0.1/tcp/19091/p2p/<peer-id-from-/tmp/a.jsonl> \
  MESH_STATUS_FILE=/tmp/b.jsonl ./target/debug/mesh-daemon
```

Frontend: `cd frontend && npm ci && npm run build`.

Smoke tests: `devops/smoke-boot.sh` (daemon boot + `GET /api/v1/health` -> 200),
`devops/smoke-p2p.sh` (two-node libp2p ping). Both run in CI.

## Workspace layout

`mesh-transport` (networking, VDF, identity), `consensus` (trie, mocks for
SNARK folding, erasure coding, CRDT), `engine` (shakmaty chess), `sandbox`
(WASI surface), `interop` (HTTP gateway, OpenAPI docs, TWAMM/HTLC math),
`governance`, `ai`, `integration` (cross-crate smoke), `frontend`,
`captive-portal`, `devops`, `docs`.
