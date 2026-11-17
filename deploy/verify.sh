#!/usr/bin/env bash
# Diagnostics and acceptance checks for the Linux WSS relay stack.
set -Eeuo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

RELAY_DOMAIN="${RELAY_DOMAIN:-p2p.aikissgo.com}"
COMPOSE_PROJECT_NAME="${COMPOSE_PROJECT_NAME:-p2p-remote}"
DEPLOY_DIR="${DEPLOY_DIR:-${SCRIPT_DIR}}"
ENV_FILE="${ENV_FILE:-${DEPLOY_DIR}/.env}"
STRICT="${STRICT:-0}"

pass=0
fail=0
warns=0

log()  { printf '[verify] %s\n' "$*"; }
ok()   { printf '[verify] PASS: %s\n' "$*"; pass=$((pass + 1)); }
bad()  { printf '[verify] FAIL: %s\n' "$*" >&2; fail=$((fail + 1)); }
warn() { printf '[verify] WARN: %s\n' "$*" >&2; warns=$((warns + 1)); }

usage() {
  cat <<'EOF'
Usage: bash deploy/verify.sh [options]

Options:
  --domain NAME          Domain to probe (default: p2p.aikissgo.com)
  --env-file PATH        Path to deploy/.env
  --strict               Treat warnings as failures
  -h, --help             Show help

Checks:
  - compose file validity (with a throwaway token if .env absent)
  - relay container not publishing host :8080
  - local /healthz via Docker network
  - unauthorized /relay returns 401
  - public HTTPS /healthz returns ok (when DNS/TLS ready)
EOF
}

compose() {
  docker compose --project-name "${COMPOSE_PROJECT_NAME}" --env-file "$1" -f "${DEPLOY_DIR}/compose.yml" "${@:2}"
}

check_compose_config() {
  local tmpenv
  tmpenv="$(mktemp)"
  if [[ -f "${ENV_FILE}" ]]; then
    cp "${ENV_FILE}" "${tmpenv}"
  else
    cat >"${tmpenv}" <<EOF
RELAY_DOMAIN=${RELAY_DOMAIN}
RELAY_ACCESS_TOKEN=$(openssl rand -hex 32)
RELAY_IMAGE_TAG=p2p-relay:verify
EOF
  fi
  chmod 0600 "${tmpenv}"
  if compose "${tmpenv}" config >/dev/null; then
    ok "compose config validates"
  else
    bad "compose config invalid"
  fi
  # Ensure 8080 is not published.
  if compose "${tmpenv}" config | grep -E 'published:[" ]*8080|0\.0\.0\.0:8080|:::8080' >/dev/null; then
    bad "relay port 8080 appears published to the host"
  else
    ok "relay 8080 is not published on the host (Docker network only)"
  fi
  rm -f "${tmpenv}"
}

check_containers() {
  if ! command -v docker >/dev/null; then
    warn "docker not available; skipping runtime checks"
    return
  fi
  if ! docker compose version >/dev/null 2>&1; then
    warn "docker compose missing; skipping runtime checks"
    return
  fi
  if [[ ! -f "${ENV_FILE}" ]]; then
    warn "no ${ENV_FILE}; skipping live container probes"
    return
  fi
  if compose "${ENV_FILE}" ps --status running 2>/dev/null | grep -q relay; then
    ok "relay container is running"
  else
    bad "relay container is not running"
    return
  fi

  # Probe healthz over the compose network without exposing 8080 on the host.
  local health
  health="$(docker compose --project-name "${COMPOSE_PROJECT_NAME}" --env-file "${ENV_FILE}" -f "${DEPLOY_DIR}/compose.yml" \
    run --rm --no-deps --entrypoint /bin/sh caddy -c 'wget -qO- http://relay:8080/healthz' 2>/dev/null || true)"
  if [[ "${health}" == "ok" ]]; then
    ok "internal http://relay:8080/healthz => ok"
  else
    # Fallback: publish nothing; use docker exec into relay via a tiny curl sidecar if image lacks wget.
    health="$(docker run --rm --network "${COMPOSE_PROJECT_NAME}_p2p_internal" curlimages/curl:8.11.1 -fsS http://relay:8080/healthz 2>/dev/null || true)"
    if [[ "${health}" == "ok" ]]; then
      ok "internal healthz via curl sidecar => ok"
    else
      bad "internal healthz failed"
    fi
  fi

  local code
  code="$(docker run --rm --network "${COMPOSE_PROJECT_NAME}_p2p_internal" curlimages/curl:8.11.1 \
    -s -o /dev/null -w '%{http_code}' \
    "http://relay:8080/relay?room=$(printf 'a%.0s' {1..64})&role=viewer" 2>/dev/null || echo 000)"
  if [[ "${code}" == "401" ]]; then
    ok "unauthorized /relay returns 401"
  else
    bad "unauthorized /relay returned HTTP ${code}, expected 401"
  fi

  # Host publish check
  if docker compose --project-name "${COMPOSE_PROJECT_NAME}" --env-file "${ENV_FILE}" -f "${DEPLOY_DIR}/compose.yml" \
      port relay 8080 >/dev/null 2>&1; then
    bad "compose reports a host mapping for relay:8080"
  else
    ok "no host port mapping for relay:8080"
  fi
}

check_public_https() {
  local url="https://${RELAY_DOMAIN}/healthz"
  local body="" code="000" tmp
  tmp="$(mktemp)"
  if code="$(curl -sS -o "${tmp}" -w '%{http_code}' --connect-timeout 10 --max-time 30 "${url}" 2>/dev/null)"; then
    body="$(tr -d '\r\n' <"${tmp}" || true)"
  else
    code="000"
  fi
  rm -f "${tmp}"
  if [[ "${code}" == "200" && "${body}" == "ok" ]]; then
    ok "public ${url} => 200 ok"
  else
    warn "public ${url} not ready (HTTP ${code}). Confirm DNS A/AAAA, security group 80/443, and Caddy TLS."
  fi
}

check_dns() {
  local resolved=""
  if command -v dig >/dev/null 2>&1; then
    resolved="$(dig +short A "${RELAY_DOMAIN}"; dig +short AAAA "${RELAY_DOMAIN}")"
  elif command -v getent >/dev/null 2>&1; then
    resolved="$(getent ahosts "${RELAY_DOMAIN}" | awk '{print $1}' | sort -u)"
  fi
  if [[ -n "${resolved}" ]]; then
    ok "DNS ${RELAY_DOMAIN} resolves"
    log "DNS answers:"$'\n'"${resolved}"
  else
    warn "DNS ${RELAY_DOMAIN} did not resolve from this host"
  fi
}

main() {
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --domain) RELAY_DOMAIN="$2"; shift 2 ;;
      --env-file) ENV_FILE="$2"; shift 2 ;;
      --strict) STRICT=1; shift ;;
      -h|--help) usage; exit 0 ;;
      *) printf 'unknown arg: %s\n' "$1" >&2; usage; exit 1 ;;
    esac
  done

  [[ -f "${DEPLOY_DIR}/compose.yml" ]] || { bad "compose.yml missing"; exit 1; }
  check_compose_config
  check_dns
  check_containers
  check_public_https

  log "summary: pass=${pass} fail=${fail} warn=${warns}"
  if [[ "${fail}" -gt 0 ]]; then
    exit 1
  fi
  if [[ "${STRICT}" == "1" && "${warns}" -gt 0 ]]; then
    exit 1
  fi
}

main "$@"
