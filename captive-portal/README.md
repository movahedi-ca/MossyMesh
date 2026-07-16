# MessyMash Captive Portal

Offline Wi-Fi landing page for MossyMesh / MessyMash mesh islands.

When a phone or laptop joins the AP, OS connectivity checks are intercepted by
`nginx.conf` and redirected here. Users can open the chess PWA at `/app/`.

## Stack

- React + TypeScript + Vite (portal UI)
- nginx (`client_max_body_size 150M`)
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
(cd frontend && npm ci && npx tsc -b && npx vite build --base=/app/)
docker compose -f captive-portal/docker-compose.yml up
```

## Full multi-stage image

```bash
# from repo root
docker compose up --build
```

- Portal: `http://localhost/`
- Chess app: `http://localhost/app/`
- Health: `http://localhost/healthz`

## Captive probes

nginx redirects common iOS / Android / Windows / Firefox checks to `/`
(see `nginx.conf`). That triggers the system captive-portal sheet so users
see MessyMash instead of a broken internet error.
