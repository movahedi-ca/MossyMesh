# MossyMesh / MessyMash DevOps

Edge deployment notes for Raspberry Pi genesis nodes, cross-compilation for
ARM and ESP32, and captive-portal packaging.

## Contents

| Path | Purpose |
| --- | --- |
| `deploy-pi.sh` | Build portal assets + run compose on a Pi (or rsync from laptop) |
| `cross-compile.md` | ARM (Pi) and ESP32 toolchains, Cargo targets, tips |
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
| **Cargo test (workspace lib)** | `cargo test --workspace --lib` on `ubuntu-latest` / Rust stable (30m timeout, cargo cache) |
| Docker portal image | Builds `captive-portal` image after frontend jobs |

**Rust gate notes:** CI uses `--lib` only (unit tests in library crates), not
`--all-targets`, so bin/integration tests that need RF hardware or long runtime
do not block the monorepo. If a single crate is known broken, exclude it with
`cargo test --workspace --lib --exclude <crate>` in the workflow and list it
here — do not paper over failures with `continue-on-error`.

**Currently excluded:** none.
