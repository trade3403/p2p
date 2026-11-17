#!/usr/bin/env bash
# Safely update an existing P2P Remote Linux relay deployment to a pinned git ref.
set -Eeuo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

INSTALL_DIR="${INSTALL_DIR:-/opt/p2p-remote}"
GIT_REF="${GIT_REF:-}"
COMPOSE_PROJECT_NAME="${COMPOSE_PROJECT_NAME:-p2p-remote}"
ASSUME_YES="${ASSUME_YES:-0}"

log()  { printf '[update] %s\n' "$*"; }
warn() { printf '[update] WARN: %s\n' "$*" >&2; }
die()  { printf '[update] ERROR: %s\n' "$*" >&2; exit 1; }

usage() {
  cat <<'EOF'
Usage: sudo bash deploy/update.sh --git-ref <sha-or-tag> [options]

Options:
  --git-ref REF          Required. Exact commit SHA or tag to deploy
  --install-dir PATH     Checkout directory (default: /opt/p2p-remote)
  --yes                  Skip confirmation prompt
  -h, --help             Show help

Behavior:
  - Fetches and checks out the pinned ref (detached HEAD)
  - Preserves deploy/.env (never rotates RELAY_ACCESS_TOKEN)
  - Backs up compose/Caddyfile/.env before recreate
  - Builds image tagged with the short SHA (no floating latest deploy)
  - On failure, previous volumes remain; restore from deploy/backups/
EOF
}

confirm() {
  if [[ "${ASSUME_YES}" == "1" ]]; then return 0; fi
  read -r -p "$1 [y/N] " answer
  [[ "${answer}" == "y" || "${answer}" == "Y" ]]
}

main() {
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --git-ref) GIT_REF="$2"; shift 2 ;;
      --install-dir) INSTALL_DIR="$2"; shift 2 ;;
      --yes) ASSUME_YES=1; shift ;;
      -h|--help) usage; exit 0 ;;
      *) die "unknown argument: $1" ;;
    esac
  done

  [[ "${EUID}" -eq 0 ]] || die "run as root"
  [[ -n "${GIT_REF}" ]] || die "--git-ref is required (pin an exact SHA/tag; do not deploy drifting latest)"
  command -v docker >/dev/null || die "docker missing"
  docker compose version >/dev/null || die "docker compose plugin missing"
  command -v git >/dev/null || die "git missing"

  local workdir
  if [[ -d "${INSTALL_DIR}/.git" ]]; then
    workdir="${INSTALL_DIR}"
  elif [[ -d "${REPO_ROOT}/.git" ]]; then
    workdir="${REPO_ROOT}"
  else
    die "no git checkout found at ${INSTALL_DIR} or ${REPO_ROOT}"
  fi

  local deploy_dir="${workdir}/deploy"
  [[ -f "${deploy_dir}/.env" ]] || die "missing ${deploy_dir}/.env — run install.sh first"
  [[ -f "${deploy_dir}/compose.yml" ]] || die "missing compose.yml"

  if ! confirm "Update ${workdir} to ${GIT_REF} and recreate containers?"; then
    die "aborted"
  fi

  local stamp backup
  stamp="$(date -u +%Y%m%dT%H%M%SZ)"
  backup="${deploy_dir}/backups/${stamp}"
  mkdir -p "${backup}"
  cp -a "${deploy_dir}/compose.yml" "${deploy_dir}/Caddyfile" "${deploy_dir}/.env" "${backup}/"
  chmod 0600 "${backup}/.env"
  log "backup at ${backup}"

  local prev
  prev="$(git -C "${workdir}" rev-parse HEAD)"
  log "previous HEAD ${prev}"
  git -C "${workdir}" fetch --tags --force origin || warn "git fetch failed; continuing with local objects if ref exists"
  if ! git -C "${workdir}" checkout --detach "${GIT_REF}"; then
    die "checkout ${GIT_REF} failed; working tree unchanged enough to retry; backup at ${backup}"
  fi
  local new_sha short_sha
  new_sha="$(git -C "${workdir}" rev-parse HEAD)"
  short_sha="$(git -C "${workdir}" rev-parse --short HEAD)"
  log "now at ${new_sha}"

  # Keep token; refresh image tag annotation only.
  if grep -q '^RELAY_IMAGE_TAG=' "${deploy_dir}/.env"; then
    sed -i "s/^RELAY_IMAGE_TAG=.*/RELAY_IMAGE_TAG=p2p-relay:${short_sha}/" "${deploy_dir}/.env"
  else
    printf '\nRELAY_IMAGE_TAG=p2p-relay:%s\n' "${short_sha}" >>"${deploy_dir}/.env"
  fi
  chmod 0600 "${deploy_dir}/.env"

  if ! docker compose --project-name "${COMPOSE_PROJECT_NAME}" --env-file "${deploy_dir}/.env" -f "${deploy_dir}/compose.yml" config >/dev/null; then
    warn "compose config invalid after checkout; restoring compose files from backup"
    cp -a "${backup}/compose.yml" "${backup}/Caddyfile" "${deploy_dir}/"
    git -C "${workdir}" checkout --detach "${prev}" || true
    die "update aborted; restored compose files"
  fi

  if ! docker compose --project-name "${COMPOSE_PROJECT_NAME}" --env-file "${deploy_dir}/.env" -f "${deploy_dir}/compose.yml" up -d --build; then
    warn "compose up failed. Volumes preserved. To roll back compose files: cp ${backup}/* ${deploy_dir}/"
    die "update failed at ${new_sha}"
  fi

  log "update complete: ${prev} -> ${new_sha}"
  log "run: bash deploy/verify.sh"
}

main "$@"
