# Engine: `wasm32-wasip1` build notes

Scope: `engine/` crate (shakmaty bitboards) for deterministic mesh / WAMR sandbox loads.

**Host builds must stay default.** Do not set a workspace default target to WASM.
`cargo test -p engine` and `cargo build -p engine` always mean **native host**.

Related: [`cross-compile.md`](cross-compile.md) §4 (short pointer), sandbox WAMR (`sandbox/`), SLA RAM cap ≤ 10 MB.

---

## 1. Prerequisites

```bash
# From workspace root (MossyMesh)
rustup target add wasm32-wasip1
```

Optional local runner for `cargo test --target wasm32-wasip1` (not required for a pure compile check):

```bash
# Linux / macOS (example)
curl https://wasmtime.dev/install.sh -sSf | bash
# or: cargo install wasmtime-cli
```

Windows: install [Wasmtime](https://wasmtime.dev/) and ensure `wasmtime` is on `PATH`.

---

## 2. Build (library only)

Default features are WASM-safe (no Syzygy mmap / OS file tablebases):

```bash
# Debug
cargo build -p engine --target wasm32-wasip1

# Release (sandbox / benchmark artifact)
cargo build -p engine --release --target wasm32-wasip1
```

Artifact (rlib + deps under target; no `cdylib` export surface yet):

```text
target/wasm32-wasip1/release/libengine.rlib
```

**Do not** enable native-only features for WASI:

| Feature | Host | `wasm32-wasip1` |
| --- | --- | --- |
| (default) | OK | OK |
| `syzygy` | OK (file tables) | Avoid — file/mmap I/O |
| `syzygy-mmap` | OK (native mmap) | **Do not use** |

```bash
# Native Syzygy only (host)
cargo build -p engine --features syzygy
cargo build -p engine --features syzygy-mmap
```

---

## 3. Optional cargo config

A sample fragment lives at [`cargo-config-engine-wasm.toml`](cargo-config-engine-wasm.toml).

It is **not** applied automatically. Either:

1. Merge snippets into workspace `.cargo/config.toml` when you need a WASM test runner, **or**
2. Point Cargo at the sample for a one-off:

```bash
# Unix
CARGO_HOME_CONFIG=...  # prefer explicit merge; see sample file comments

# Practical one-off: use --config (Cargo 1.39+)
cargo build -p engine --release --target wasm32-wasip1 \
  --config 'target.wasm32-wasip1.runner="wasmtime"'
```

**Never** put this in the sample as a default host target:

```toml
# BAD — breaks host cargo test / default builds
# [build]
# target = "wasm32-wasip1"
```

---

## 4. Helper scripts

From workspace root:

```bash
# Unix / Git Bash
chmod +x devops/build-engine-wasm.sh
./devops/build-engine-wasm.sh           # release
./devops/build-engine-wasm.sh --debug   # debug
```

```powershell
# Windows PowerShell
.\devops\build-engine-wasm.ps1
.\devops\build-engine-wasm.ps1 -Debug
```

Scripts only add the target (if missing) and run `cargo build -p engine … --target wasm32-wasip1`. They do not change workspace default features or host toolchain.

---

## 5. Mnps bench how-to

`engine::benchmark_mnps()` measures real throughput over a **fixed** workload:

1. `perft(startpos, 4)` — move-gen tree (~197_281 nodes)
2. `negamax_search(startpos, 3)` — eval + search nodes

Nodes are summed; wall time is `std::time::Instant`.  
**~836 Mnps** is an aspirational native reference only — **never hard-code it**. WASM is expected to be lower. Report measured values.

### 5.1 Host (native) — recommended

```bash
# Library tests (includes benchmark_returns_finite_positive)
cargo test -p engine --release

# Focused bench tests with printed output (if you add eprintln in a local branch)
cargo test -p engine benchmark --release -- --nocapture

# Dedicated example: print measured Mnps
cargo run -p engine --example mnps_bench --release
```

Example output shape:

```text
workload: perft(d4)+negamax(d3) startpos
nodes:    198xxx
seconds:  0.0xx
mnps:     12.34   # measured — do not compare to a hard-coded 836
```

### 5.2 Host — quick unit path

```bash
cargo test -p engine benchmark_mnps_is_measured -- --nocapture
```

### 5.3 WASM / WASI

Compile-only (CI / smoke that the crate is WASI-ready):

```bash
cargo build -p engine --release --target wasm32-wasip1
```

If you have `wasmtime` and a future WASI-linked binary/export that calls `benchmark_mnps`, run under the runner. Today the crate is a **library**; Mnps numbers for PoC are collected on **native host** via the example/tests, while WASM proves **compile + sandbox load** (WAMR workstream).

Optional test under WASI (requires runner configured):

```bash
cargo test -p engine --target wasm32-wasip1 -- --nocapture
```

### 5.4 Interpreting results

| Context | Expectation |
| --- | --- |
| Native release, strong CPU | Highest measured Mnps; may approach aspirational ~836 only with further bitboard/kernel work |
| Native debug | Much lower; not for SLA claims |
| `wasm32-wasip1` / WAMR | Lower than native; still must be deterministic for mesh replay |
| SLA | Determinism + offline sandbox — not a hard Mnps floor in CI |

---

## 6. Sandbox / WAMR notes

- Bounded stack / fixed pools live in the **sandbox** workstream (`-z stack-size=N`, fixed-block pools).
- Do not raise the edge **10 MB** ledger/RAM budget without SLA review.
- Future WASM exports (`evaluate_move`, `get_best_move`) are contract-level (see `docs/interface-contracts.md`); this doc only covers building the `engine` crate for `wasm32-wasip1`.

---

## 7. Checklist

- [ ] `rustup target add wasm32-wasip1`
- [ ] `cargo build -p engine --release --target wasm32-wasip1` succeeds with **default** features
- [ ] `cargo test -p engine` still green on **host** (no default-target change)
- [ ] Mnps reported via `mnps_bench` / `benchmark_mnps_detailed()` — measured, not hard-coded
- [ ] No `syzygy-mmap` on WASI builds
