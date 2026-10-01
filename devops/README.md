# MossyMesh / MessyMash DevOps

Edge deployment notes for Raspberry Pi genesis nodes, cross-compilation for
ARM and ESP32, and captive-portal packaging.

## Contents

| Path | Purpose |
| --- | --- |
| `deploy-pi.sh` | Build portal assets + run compose on a Pi (or rsync from laptop) |
| `cross-compile.md` | ARM (Pi) and ESP32 toolchains, Cargo targets, tips |
| `engine-wasm.md` | `engine` crate → `wasm32-wasip1` build notes + Mnps bench how-to |
| `build-engine-wasm.sh` / `.ps1` | Optional helper: build `engine` for WASI (host defaults unchanged) |
| `cargo-config-engine-wasm.toml` | Optional Cargo fragment (runner only; not auto-applied) |
| `hostapd-dnsmasq.notes.md` | Optional Wi-Fi AP + DNS hijack for captive portal |

## Quick start (Pi)

```bash
# On the Pi (or after copying the repo)
chmod +x devops/deploy-pi.sh
./devops/deploy-pi.sh
```

Portal listens on port 80 by default (`PORTAL_PORT` overrides).

## Captive portal constraints

- nginx `client_max_body_size 150M` for large asset / LoRA-patch transfers
- iOS / Android connectivity checks redirect to `/` (see `captive-portal/nginx.conf`)
- Chess PWA served at `/app/`

## CI

GitHub Actions (`.github/workflows/ci.yml`) on push/PR to `main` (and push to
`agent/**`):

| Job | What it runs |
| --- | --- |
| Frontend + portal | `npm ci` + build for `frontend` and `captive-portal` |
| **Cargo test (workspace lib)** | `cargo test --locked --workspace --lib` on `ubuntu-latest` / Rust stable (30m timeout, cargo cache) |
| cargo check + test (windows-msvc) | `cargo check --locked --workspace --all-targets` and `cargo test --locked --workspace --lib` on `windows-latest` (issue #148) |
| Feature matrix | documented non-default combos: `sandbox --features wamr`, `engine --features syzygy` / `syzygy-mmap` (issue #147) |
| Integration smoke tests | `cargo test --locked -p integration`, default and `--features transport` (issue #172) |
| Docker portal image | Builds the `captive-portal` image, then runs it and probes `/healthz` and `/app/` (15m timeout, issue #151) |

**Rust gate notes:** `rust-lib` runs `--lib` only (unit tests in library crates)
so bin/integration tests that need RF hardware or long runtime do not block
the monorepo; `rust-windows` additionally compiles `--all-targets` and runs
the lib suite on `windows-latest`, and the `integration-smoke` job covers the
cross-crate suite. All cargo invocations use `--locked` so CI tests exactly
the committed `Cargo.lock` (issue #152). If a single crate is known broken,
exclude it with `--exclude <crate>` in the workflow and list it here, do not
paper over failures with `continue-on-error`.

**Currently excluded:** none.
