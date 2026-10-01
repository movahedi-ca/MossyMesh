# MossyMesh Issue Tracker

This document tracks known issues, technical debt, and environmental blockers for the MossyMesh project.

## 🐛 Build & Environment Issues

### 1. Windows MinGW GCC Linker Failure (`High Priority`)
- **Description:** The Rust workspace fails to compile on Windows when evaluating build scripts for `proc-macro2`, `serde`, and `zerocopy`. The `x86_64-w64-mingw32-gcc` linker throws errors: `unable to find library -lgcc_eh` and `unable to find library -lgcc`.
- **Impact:** Prevents any local `cargo check`, `cargo build`, or `clippy` analysis on the backend.
- **Proposed Fix:** Reinstall MSYS2/MinGW-w64 and ensure `libgcc_eh.a` is in the PATH, or configure `.cargo/config.toml` to force the MSVC linker instead of GNU for host build scripts.

### 2. Missing WASM Engine Compilation Pipeline (`Medium Priority`)
- **Description:** The `engine` crate is written to be executed in the `sandbox` via WebAssembly (WAMR). However, there is no automated step (e.g., a `build.rs` script or Makefile) to actually compile the engine to `wasm32-wasip1` before the `sandbox` attempts to load it.
- **Impact:** If the daemon starts, it will crash trying to load an `engine.wasm` file that doesn't exist yet.
- **Proposed Fix:** Add an explicit build script to `sandbox` that invokes `cargo build -p engine --target wasm32-wasip1 --release` and copies the resulting `.wasm` file into the `sandbox/assets/` directory.

## 🏗️ Architectural & Implementation Gaps

### 3. Disconnected DHT Payload Routing (`Medium Priority`)
- **Description:** The new Axum HTTP server in `interop` accepts the `/api/v1/submit_job` POST payload, but it currently just prints the payload to the console (`"Routing job payload [...]"`).
- **Impact:** Frontend chess moves are not actually being submitted to the Kademlia DHT or the WASM sandbox yet.
- **Proposed Fix:** Wire the `submit_job_handler` to explicitly invoke the `WAMR` sandbox execution context and dispatch the job struct via `mesh-transport`.

### 4. WebSocket Sync Loop is Stubbed (`Low Priority`)
- **Description:** The function `handle_websocket` in `interop/src/lib.rs` contains a mocked event loop that ticks 3 times and intentionally breaks the connection to simulate a disconnect.
- **Impact:** Offline nodes currently have no real way of persisting a WebSocket connection when internet is re-established.
- **Proposed Fix:** Implement `tokio-tungstenite` to maintain a robust, persistent connection with exponential backoff for reconnects when the `internet_reconnected` flag is true.

## 🧹 Code Quality & CI

### 5. Missing CI/CD Workflows (`Low Priority`)
- **Description:** There are no GitHub Actions (or equivalent CI pipelines) enforcing `cargo clippy`, `cargo fmt`, or running cross-compilation checks.
- **Impact:** Platform-specific build errors (like the Windows MinGW issue) slip into the `main` branch undetected.
- **Proposed Fix:** Add a `.github/workflows/rust.yml` file to test native compilation (`x86_64-unknown-linux-gnu`, `x86_64-pc-windows-msvc`) and WASM compilation (`wasm32-wasip1`).
