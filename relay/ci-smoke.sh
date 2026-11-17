#!/usr/bin/env bash
# CI-only smoke test. Never bind a public interface or operate production Docker.
set -Eeuo pipefail
cd "$(dirname "$0")"
tmp="$(mktemp -d)"
pid=""
cleanup() {
  if [[ -n "$pid" ]]; then
    kill "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
  fi
  rm -rf "$tmp"
}
trap cleanup EXIT

# Choose an ephemeral local-only port, without requiring root or Docker access.
port="$(python3 - <<'PY'
import socket
with socket.socket() as s:
    s.bind(('127.0.0.1', 0))
    print(s.getsockname()[1])
PY
)"
token="$(python3 - <<'PY'
import secrets
print(secrets.token_hex(32))
PY
)"
echo "Building relay and starting loopback-only smoke service"
go build -trimpath -o "$tmp/p2p-relay-smoke" .
STUN_ENABLED=false RELAY_ACCESS_TOKEN="$token" "$tmp/p2p-relay-smoke" -listen "127.0.0.1:$port" >"$tmp/server.log" 2>&1 &
pid="$!"
base="http://127.0.0.1:$port"

ready=false
for attempt in $(seq 1 40); do
  if ! kill -0 "$pid" 2>/dev/null; then
    echo "Relay exited before becoming healthy" >&2
    cat "$tmp/server.log" >&2
    exit 1
  fi
  if curl --silent --show-error --max-time 1 "$base/healthz" >"$tmp/health" 2>/dev/null; then
    ready=true
    break
  fi
  sleep 0.25
done
if [[ "$ready" != true ]] || [[ "$(cat "$tmp/health")" != "ok" ]]; then
  echo "Relay health check failed" >&2
  cat "$tmp/server.log" >&2
  exit 1
fi
room="$(printf '%064d' 0)"
unauth="$(curl --silent --output /dev/null --write-out '%{http_code}' --max-time 3 "$base/relay?room=$room&role=viewer")"
if [[ "$unauth" != 401 ]]; then
  echo "Expected 401 for unauthenticated relay; got $unauth" >&2
  exit 1
fi
badroom="$(curl --silent --output /dev/null --write-out '%{http_code}' --max-time 3 -H "Authorization: Bearer $token" "$base/relay?room=invalid&role=host")"
if [[ "$badroom" != 400 ]]; then
  echo "Expected 400 for malformed room; got $badroom" >&2
  exit 1
fi
echo "PASS: loopback healthz=200, no token=401, malformed room=400"
echo "WebSocket forwarding is exercised separately by go test -race ./..."
