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

GitHub Actions (`.github/workflows/ci.yml`) builds `frontend` and
`captive-portal`, best-effort `cargo check` per workspace crate, and the
portal Docker image.
