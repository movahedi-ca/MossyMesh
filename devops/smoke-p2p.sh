#!/usr/bin/env bash
# P2P smoke test: boots TWO real mesh-daemon processes in REAL p2p mode and
# asserts they connect and exchange real ping packets over real sockets.
#
# Regression guard for the Oct 2026 operational review finding: mesh
# networking was theater (listen/dial/bootstrap helpers had zero callers,
# boot invented fake peers, no node ever sent a packet). This boots two
# real daemons, dials B -> A, and requires both status files to show
# peer_connected plus at least one real ping round trip each.
#
# Usage: ./devops/smoke-p2p.sh [path-to-mesh-daemon]
set -euo pipefail

BIN="${1:-./target/debug/mesh-daemon}"
LOG_A="$(mktemp /tmp/mesh-p2p-a.XXXXXX.log)"
LOG_B="$(mktemp /tmp/mesh-p2p-b.XXXXXX.log)"
STATUS_A="$(mktemp /tmp/mesh-p2p-status-a.XXXXXX.jsonl)"
STATUS_B="$(mktemp /tmp/mesh-p2p-status-b.XXXXXX.jsonl)"

if [ ! -x "$BIN" ]; then
  echo "smoke-p2p: binary not found or not executable: $BIN" >&2
  exit 1
fi

DAEMON_A_PID=""
DAEMON_B_PID=""
cleanup() {
  [ -n "$DAEMON_A_PID" ] && kill "$DAEMON_A_PID" 2>/dev/null || true
  [ -n "$DAEMON_B_PID" ] && kill "$DAEMON_B_PID" 2>/dev/null || true
  rm -f "$LOG_A" "$LOG_B" "$STATUS_A" "$STATUS_B"
}
trap cleanup EXIT

fail() {
  echo "smoke-p2p: FAIL - $1" >&2
  echo "--- daemon A log ---" >&2
  cat "$LOG_A" >&2
  echo "--- daemon B log ---" >&2
  cat "$LOG_B" >&2
  exit 1
}

# Boot daemon A (no bootstrap peer): it listens and publishes its peer id.
MESH_GATEWAY_BIND=127.0.0.1:18091 \
MESH_LISTEN_ADDR=127.0.0.1:19091 \
MESH_STATUS_FILE="$STATUS_A" \
  "$BIN" >"$LOG_A" 2>&1 &
DAEMON_A_PID=$!

# Wait for A's listening line and extract its peer id (python3, no jq).
A_PEER=""
for _ in $(seq 1 60); do
  if ! kill -0 "$DAEMON_A_PID" 2>/dev/null; then
    fail "daemon A exited during boot"
  fi
  A_PEER="$(python3 - "$STATUS_A" <<'EOF' 2>/dev/null || true
import json, sys
try:
    for line in open(sys.argv[1]):
        try:
            obj = json.loads(line)
        except ValueError:
            continue
        if obj.get("event") == "listening" and obj.get("peer_id"):
            print(obj["peer_id"])
            break
except FileNotFoundError:
    pass
EOF
)"
  [ -n "$A_PEER" ] && break
  sleep 1
done
[ -n "$A_PEER" ] || fail "daemon A never wrote a listening line within 60s"

# Boot daemon B, dialing A's full multiaddr incl. /p2p/<peer-id>.
MESH_GATEWAY_BIND=127.0.0.1:18092 \
MESH_LISTEN_ADDR=127.0.0.1:19092 \
MESH_PEER_ADDR="/ip4/127.0.0.1/tcp/19091/p2p/${A_PEER}" \
MESH_STATUS_FILE="$STATUS_B" \
  "$BIN" >"$LOG_B" 2>&1 &
DAEMON_B_PID=$!

# Poll up to 90s until BOTH status files show peer_connected and a ping.
READY=0
for _ in $(seq 1 90); do
  if ! kill -0 "$DAEMON_A_PID" 2>/dev/null; then
    fail "daemon A exited after B booted"
  fi
  if ! kill -0 "$DAEMON_B_PID" 2>/dev/null; then
    fail "daemon B exited during boot"
  fi
  if python3 - "$STATUS_A" "$STATUS_B" <<'EOF'
import json, sys
def events(path):
    out = set()
    pings = 0
    try:
        for line in open(path):
            try:
                obj = json.loads(line)
            except ValueError:
                continue
            ev = obj.get("event")
            if ev == "peer_connected":
                out.add(ev)
            elif ev == "ping" and isinstance(obj.get("rtt_ms"), (int, float)):
                pings += 1
    except FileNotFoundError:
        pass
    return out, pings
a_ev, a_ping = events(sys.argv[1])
b_ev, b_ping = events(sys.argv[2])
ok = ("peer_connected" in a_ev and "peer_connected" in b_ev
      and a_ping >= 1 and b_ping >= 1)
sys.exit(0 if ok else 1)
EOF
  then
    READY=1
    break
  fi
  sleep 1
done

[ "$READY" = "1" ] || fail "no real connection+ping round trip within 90s"

A_PING="$(grep -c '"event":"ping"' "$STATUS_A" || true)"
B_PING="$(grep -c '"event":"ping"' "$STATUS_B" || true)"
echo "smoke-p2p: OK - two real daemons connected; A saw ${A_PING} ping(s), B saw ${B_PING} ping(s) over real sockets"
