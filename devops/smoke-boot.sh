#!/usr/bin/env bash
# Boot smoke test for the real mesh-daemon binary.
#
# Regression guard: run_http_server once registered /api-docs/openapi.json
# twice and Axum panicked, killing the daemon on every boot. No unit test
# ever executed main(), so this boots the actual binary and asserts the
# HTTP gateway answers /api/v1/health with HTTP 200.
#
# Usage: ./devops/smoke-boot.sh [path-to-mesh-daemon]
# Env:   SMOKE_PORT (default 18080)
set -euo pipefail

BIN="${1:-./target/debug/mesh-daemon}"
PORT="${SMOKE_PORT:-18080}"
export MESH_GATEWAY_BIND="127.0.0.1:${PORT}"
LOG="$(mktemp /tmp/mesh-daemon-smoke.XXXXXX.log)"

if [ ! -x "$BIN" ]; then
  echo "smoke-boot: binary not found or not executable: $BIN" >&2
  exit 1
fi

"$BIN" >"$LOG" 2>&1 &
DAEMON_PID=$!
cleanup() { kill "$DAEMON_PID" 2>/dev/null || true; rm -f "$LOG"; }
trap cleanup EXIT

HEALTH_URL="http://127.0.0.1:${PORT}/api/v1/health"
READY=0
for _ in $(seq 1 60); do
  if curl -sf --max-time 2 "$HEALTH_URL" >/dev/null 2>&1; then
    READY=1
    break
  fi
  if ! kill -0 "$DAEMON_PID" 2>/dev/null; then
    echo "smoke-boot: FAIL - daemon exited during boot. Log:" >&2
    cat "$LOG" >&2
    exit 1
  fi
  sleep 1
done

if [ "$READY" != "1" ]; then
  echo "smoke-boot: FAIL - /api/v1/health never answered within 60s. Log:" >&2
  cat "$LOG" >&2
  exit 1
fi

BODY="$(curl -sf --max-time 5 "$HEALTH_URL")"
if [ "$BODY" != "Mesh Island Active" ]; then
  echo "smoke-boot: FAIL - unexpected health body: $BODY" >&2
  exit 1
fi

echo "smoke-boot: OK - daemon booted, GET /api/v1/health -> 200 'Mesh Island Active'"
