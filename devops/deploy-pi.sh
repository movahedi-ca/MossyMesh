#!/usr/bin/env bash
# Deploy MessyMash captive portal + frontend on a Raspberry Pi edge node.
# Usage: ./devops/deploy-pi.sh [--bind] [--port 8080]
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

MODE="build"   # build | bind
PORTAL_PORT="${PORTAL_PORT:-80}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --bind) MODE="bind"; shift ;;
    --port) PORTAL_PORT="$2"; shift 2 ;;
    -h|--help)
      echo "Usage: $0 [--bind] [--port N]"
      echo "  default: docker compose multi-stage build"
      echo "  --bind:  npm build + nginx bind mounts (faster iteration)"
      exit 0
      ;;
    *) echo "Unknown arg: $1" >&2; exit 1 ;;
  esac
done

export PORTAL_PORT

need() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "Missing dependency: $1" >&2
    exit 1
  }
}

need docker

if [[ "$MODE" == "bind" ]]; then
  need npm
  echo "==> Building captive-portal"
  (cd captive-portal && npm ci && npm run build)
  echo "==> Building frontend (base /app/)"
  (cd frontend && npm ci && npx tsc -b && npx vite build --base=/app/)
  echo "==> Starting nginx bind profile on :${PORTAL_PORT}"
  docker compose --profile bind up -d portal-bind
else
  echo "==> Building and starting multi-stage portal image on :${PORTAL_PORT}"
  docker compose up -d --build portal
fi

echo "==> Health check"
for i in 1 2 3 4 5; do
  if curl -fsS "http://127.0.0.1:${PORTAL_PORT}/healthz" >/dev/null; then
    echo "OK  portal healthy at http://0.0.0.0:${PORTAL_PORT}/"
    echo "    chess app:          http://0.0.0.0:${PORTAL_PORT}/app/"
    exit 0
  fi
  sleep 2
done

echo "WARN: healthz did not succeed yet; check: docker compose logs" >&2
exit 1
