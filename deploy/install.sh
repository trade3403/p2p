#!/usr/bin/env bash
# First-time (and re-entrant) Linux deploy for P2P Remote WSS relay + Caddy.
# Does NOT auto-deploy to production from CI. Run manually on the VPS after DNS is ready.
set -Eeuo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

DEFAULT_DOMAIN="p2p.aikissgo.com"
INSTALL_DIR="${INSTALL_DIR:-/opt/p2p-remote}"
RELAY_DOMAIN="${RELAY_DOMAIN:-$DEFAULT_DOMAIN}"
GIT_REF="${GIT_REF:-}"
REPO_URL="${REPO_URL:-}"
SKIP_DNS_CHECK="${SKIP_DNS_CHECK:-0}"
ASSUME_YES="${ASSUME_YES:-0}"
COMPOSE_PROJECT_NAME="${COMPOSE_PROJECT_NAME:-p2p-remote}"

log()  { printf '[install] %s\n' "$*"; }
warn() { printf '[install] WARN: %s\n' "$*" >&2; }
die()  { printf '[install] ERROR: %s\n' "$*" >&2; exit 1; }

usage() {
  cat <<'EOF'
Usage: sudo bash deploy/install.sh [options]

Options:
  --domain NAME          Relay domain (default: p2p.aikissgo.com)
  --install-dir PATH     Install directory (default: /opt/p2p-remote)
  --git-ref REF          Checkout this git ref/SHA into --install-dir
  --repo-url URL         Git remote used with --git-ref (SSH recommended)
  --skip-dns-check       Do not fail when DNS does not point at this host
  --yes                  Non-interactive confirmations where safe
  -h, --help             Show help

Environment overrides: RELAY_DOMAIN, INSTALL_DIR, GIT_REF, REPO_URL,
SKIP_DNS_CHECK, ASSUME_YES, COMPOSE_PROJECT_NAME.

Security notes:
  - Never pass a real RELAY_ACCESS_TOKEN on the command line.
  - Existing deploy/.env tokens are preserved and never overwritten.
  - Port 8080 stays on the Docker network only; public ports are 80/443.
  - This script does not rewrite UFW / cloud security-group rules.
EOF
}

require_root() {
  if [[ "${EUID}" -ne 0 ]]; then
    die "run as root (sudo bash deploy/install.sh)"
  fi
}

detect_os() {
  if [[ ! -r /etc/os-release ]]; then
    die "unsupported OS: /etc/os-release missing"
  fi
  # shellcheck disable=SC1091
  source /etc/os-release
  case "${ID:-}:${VERSION_ID:-}" in
    ubuntu:22.04|ubuntu:24.04|debian:12|debian:13) ;;
    ubuntu:*|debian:*)
      warn "OS ${PRETTY_NAME:-unknown} is not in the tested matrix (Ubuntu 22.04/24.04); continuing"
      ;;
    *)
      die "unsupported OS '${PRETTY_NAME:-unknown}'. Use Ubuntu 22.04/24.04 or Debian."
      ;;
  esac
  case "$(uname -m)" in
    x86_64|amd64|aarch64|arm64) ;;
    *) die "unsupported architecture $(uname -m)" ;;
  esac
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "missing required command: $1"
}

confirm() {
  local prompt="$1"
  if [[ "${ASSUME_YES}" == "1" ]]; then
    return 0
  fi
  read -r -p "${prompt} [y/N] " answer
  [[ "${answer}" == "y" || "${answer}" == "Y" ]]
}

ensure_docker() {
  if command -v docker >/dev/null 2>&1 && docker info >/dev/null 2>&1; then
    log "reusing existing Docker daemon"
  else
    warn "Docker is not available. Official install will be attempted if you confirm."
    if ! confirm "Install Docker Engine via get.docker.com?"; then
      die "Docker is required"
    fi
    need_cmd curl
    curl -fsSL https://get.docker.com | sh
  fi
  if ! docker compose version >/dev/null 2>&1; then
    die "docker compose plugin missing; install docker-compose-plugin without breaking other containers"
  fi
  need_cmd openssl
  need_cmd curl
  need_cmd dig || need_cmd getent || true
}

resolve_workdir() {
  if [[ -n "${GIT_REF}" ]]; then
    need_cmd git
    [[ -n "${REPO_URL}" ]] || die "--git-ref requires --repo-url (use SSH deploy key; do not hardcode tokens)"
    mkdir -p "$(dirname "${INSTALL_DIR}")"
    if [[ -d "${INSTALL_DIR}/.git" ]]; then
      log "updating existing checkout at ${INSTALL_DIR} to ${GIT_REF}"
      git -C "${INSTALL_DIR}" fetch --tags --force origin
      git -C "${INSTALL_DIR}" checkout --detach "${GIT_REF}"
    else
      log "cloning ${REPO_URL} @ ${GIT_REF} into ${INSTALL_DIR}"
      git clone --no-checkout "${REPO_URL}" "${INSTALL_DIR}"
      git -C "${INSTALL_DIR}" checkout --detach "${GIT_REF}"
    fi
    WORKDIR="${INSTALL_DIR}"
  else
    WORKDIR="${REPO_ROOT}"
    log "using current repository tree at ${WORKDIR}"
  fi
  DEPLOY_DIR="${WORKDIR}/deploy"
  [[ -f "${DEPLOY_DIR}/compose.yml" ]] || die "compose.yml not found under ${DEPLOY_DIR}"
  [[ -f "${DEPLOY_DIR}/Caddyfile" ]] || die "Caddyfile not found under ${DEPLOY_DIR}"
}

write_env_if_needed() {
  local env_file="${DEPLOY_DIR}/.env"
  if [[ -f "${env_file}" ]]; then
    log "preserving existing ${env_file} (token will NOT be rotated)"
    # Ensure domain can be updated without touching token.
    if grep -q '^RELAY_DOMAIN=' "${env_file}"; then
      sed -i "s/^RELAY_DOMAIN=.*/RELAY_DOMAIN=${RELAY_DOMAIN}/" "${env_file}"
    else
      printf '\nRELAY_DOMAIN=%s\n' "${RELAY_DOMAIN}" >>"${env_file}"
    fi
    chmod 0600 "${env_file}"
    return
  fi
  local token
  token="$(openssl rand -hex 32)"
  umask 077
  cat >"${env_file}" <<EOF
RELAY_DOMAIN=${RELAY_DOMAIN}
RELAY_ACCESS_TOKEN=${token}
RELAY_IMAGE_TAG=p2p-relay:$(git -C "${WORKDIR}" rev-parse --short HEAD 2>/dev/null || echo local)
EOF
  chmod 0600 "${env_file}"
  log "created ${env_file} (mode 0600). Copy RELAY_ACCESS_TOKEN to clients via a secure channel."
  log "token fingerprint (sha256 first 12): $(printf '%s' "${token}" | sha256sum | awk '{print substr($1,1,12)}')"
}

dns_check() {
  if [[ "${SKIP_DNS_CHECK}" == "1" ]]; then
    warn "skipping DNS check"
    return
  fi
  local resolved=""
  if command -v dig >/dev/null 2>&1; then
    resolved="$(dig +short A "${RELAY_DOMAIN}" | tr '\n' ' ')"
    resolved+=" $(dig +short AAAA "${RELAY_DOMAIN}" | tr '\n' ' ')"
  elif command -v getent >/dev/null 2>&1; then
    resolved="$(getent ahostsv4 "${RELAY_DOMAIN}" | awk '{print $1}' | sort -u | tr '\n' ' ')"
  fi
  resolved="$(echo "${resolved}" | xargs || true)"
  if [[ -z "${resolved}" ]]; then
    warn "DNS for ${RELAY_DOMAIN} did not resolve. Set A/AAAA at your DNS provider before claiming TLS success."
    warn "Continuing to build locally; HTTPS verification may fail until DNS propagates."
    return
  fi
  log "DNS ${RELAY_DOMAIN} -> ${resolved}"
  local host_ips
  host_ips="$(hostname -I 2>/dev/null || true)"
  local matched=0
  local ip
  for ip in ${resolved}; do
    if echo " ${host_ips} " | grep -q " ${ip} "; then
      matched=1
    fi
  done
  if [[ "${matched}" -ne 1 ]]; then
    warn "resolved addresses for ${RELAY_DOMAIN} do not clearly match this host (${host_ips})."
    warn "Confirm DNS A/AAAA and cloud security group (TCP 80/443) before production use."
  fi
}

backup_compose_state() {
  local stamp
  stamp="$(date -u +%Y%m%dT%H%M%SZ)"
  BACKUP_DIR="${DEPLOY_DIR}/backups/${stamp}"
  mkdir -p "${BACKUP_DIR}"
  cp -a "${DEPLOY_DIR}/compose.yml" "${BACKUP_DIR}/compose.yml"
  cp -a "${DEPLOY_DIR}/Caddyfile" "${BACKUP_DIR}/Caddyfile"
  if [[ -f "${DEPLOY_DIR}/.env" ]]; then
    # Copy env for rollback of non-secret structure only on this host; keep mode 0600.
    cp -a "${DEPLOY_DIR}/.env" "${BACKUP_DIR}/.env"
    chmod 0600 "${BACKUP_DIR}/.env"
  fi
  log "backup saved to ${BACKUP_DIR}"
}

compose() {
  docker compose --project-name "${COMPOSE_PROJECT_NAME}" --env-file "${DEPLOY_DIR}/.env" -f "${DEPLOY_DIR}/compose.yml" "$@"
}

bring_up() {
  log "validating compose config"
  compose config >/dev/null
  local git_sha
  git_sha="$(git -C "${WORKDIR}" rev-parse HEAD 2>/dev/null || echo unknown)"
  log "building and starting stack (git=${git_sha})"
  if ! compose up -d --build; then
    die "compose up failed; previous volumes/config left intact under ${DEPLOY_DIR}"
  fi
  compose ps
  log "probing internal healthz via compose network (8080 stays unpublished)"
  local health=""
  local attempt
  for attempt in $(seq 1 30); do
    health="$(compose run --rm --no-deps --entrypoint /bin/sh caddy -c 'wget -qO- http://relay:8080/healthz' 2>/dev/null || true)"
    if [[ "${health}" == "ok" ]]; then
      log "internal healthz OK (attempt ${attempt})"
      break
    fi
    sleep 1
  done
  if [[ "${health}" != "ok" ]]; then
    warn "internal healthz not confirmed yet; continue with public HTTPS check / verify.sh"
  fi
}

verify_https() {
  local url="https://${RELAY_DOMAIN}/healthz"
  log "checking ${url}"
  if curl -fsS --connect-timeout 10 --max-time 30 "${url}" | tee /tmp/p2p-healthz.out | grep -qx 'ok'; then
    log "HTTPS healthz OK"
  else
    warn "HTTPS healthz not confirmed yet. DNS/TLS may still be propagating."
    warn "Re-run: bash deploy/verify.sh --domain ${RELAY_DOMAIN}"
  fi
}

print_firewall_hints() {
  cat <<EOF

Firewall guidance (NOT applied automatically — confirm each rule yourself):

  # Cloud security group / panel:
  #   Allow TCP 80, 443 from the Internet
  #   Allow TCP 22 only from admin IPs
  #   Do NOT publish 8080 or Windows LAN 43821 to the public Internet

  # Optional host UFW (review before enabling; wrong rules can lock you out of SSH):
  #   sudo ufw allow OpenSSH
  #   sudo ufw allow 80/tcp
  #   sudo ufw allow 443/tcp
  #   sudo ufw enable

Client defaults:
  Windows/Android WSS base URL: wss://${RELAY_DOMAIN}
  Health check: https://${RELAY_DOMAIN}/healthz
  Relay token: read from ${DEPLOY_DIR}/.env (root only). Never paste into GitHub issues.

EOF
}

main() {
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --domain) RELAY_DOMAIN="$2"; shift 2 ;;
      --install-dir) INSTALL_DIR="$2"; shift 2 ;;
      --git-ref) GIT_REF="$2"; shift 2 ;;
      --repo-url) REPO_URL="$2"; shift 2 ;;
      --skip-dns-check) SKIP_DNS_CHECK=1; shift ;;
      --yes) ASSUME_YES=1; shift ;;
      -h|--help) usage; exit 0 ;;
      *) die "unknown argument: $1" ;;
    esac
  done

  require_root
  detect_os
  ensure_docker
  resolve_workdir
  write_env_if_needed
  dns_check
  backup_compose_state
  bring_up
  verify_https
  print_firewall_hints
  log "install finished at $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  log "deployment metadata: domain=${RELAY_DOMAIN} workdir=${WORKDIR} git=$(git -C "${WORKDIR}" rev-parse HEAD 2>/dev/null || echo unknown)"
}

main "$@"
