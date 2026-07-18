# MessyMash Captive Portal

Offline Wi-Fi landing page for MossyMesh / MessyMash mesh islands.

When a phone or laptop joins the AP, OS connectivity checks are intercepted by
`nginx.conf` and redirected here. Users can open the chess PWA at `/app/`.

## Stack

- React + TypeScript + Vite (portal UI)
- nginx (`client_max_body_size 150M`)
- Reverse proxy: `/api/*` → mesh interop host (`host.docker.internal:8787`)
- Docker Compose (root `docker-compose.yml` or this directory)

## Local develop

```bash
npm ci
npm run dev
```

## Build & serve with nginx (bind mounts)

```bash
# from repo root
(cd captive-portal && npm ci && npm run build)
(cd frontend && npm ci && npm run build)   # base=/app/ (vite default on build)
docker compose -f captive-portal/docker-compose.yml up
```

**Note:** volume paths in `captive-portal/docker-compose.yml` are relative to
`captive-portal/` (`./dist`, `../frontend/dist`). Always run compose from a
context where those paths resolve (repo checkout intact).

Root bind profile (same layout, preferred on Pi):

```bash
# from repo root — also used by devops/deploy-pi.sh --bind
(cd captive-portal && npm ci && npm run build)
(cd frontend && npm ci && npm run build)
docker compose --profile bind up portal-bind
```

## Full multi-stage image

```bash
# from repo root
docker compose up --build portal
# or: ./devops/deploy-pi.sh
```

| URL | Purpose |
| --- | --- |
| `http://localhost/` | Portal landing |
| `http://localhost/app/` | Chess PWA (offline-capable) |
| `http://localhost/healthz` | nginx liveness (`ok`) |
| `http://localhost/api/v1/health` | Mesh host (502 if host down) |
| `http://localhost/api/v1/submit_job` | Job enqueue (POST, host when up) |

## Mesh API proxy

nginx upstream `mesh_api` targets **`host.docker.internal:8787`**. Compose sets
`extra_hosts: host.docker.internal:host-gateway` so Linux Docker can reach a
process on the host.

| Deploy mode | Upstream |
| --- | --- |
| Docker bridge (default) | `host.docker.internal:8787` |
| `network_mode: host` (Pi AP) | Edit `nginx.conf` → `127.0.0.1:8787` |

When the host is down, `/api/*` returns **502**. The PWA chess UI stays fully
usable offline and treats that as island mode.

## Captive probes

nginx redirects common iOS / Android / Windows / Firefox checks to `/`
(see `nginx.conf`). That triggers the system captive-portal sheet so users
see MessyMash instead of a broken internet error.
