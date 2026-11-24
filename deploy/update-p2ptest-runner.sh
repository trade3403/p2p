#!/usr/bin/env bash
# Update ONLY the already-running p2ptest Compose service on the dedicated runner.
set -Eeuo pipefail
umask 077

fail() { printf '::error::%s\n' "$*" >&2; exit 1; }
[[ "${RUNNER_NAME:-}" == "ubuntu-linux-relay" ]] || fail "Wrong runner; no changes made"
[[ "${GITHUB_REF:-}" == "refs/heads/feature/v0.20-public-remote" ]] || fail "Wrong branch"
[[ "${GITHUB_SHA:-}" =~ ^[0-9a-f]{40}$ ]] || fail "Invalid commit SHA"
[[ "$(git rev-parse HEAD)" == "$GITHUB_SHA" ]] || fail "Checkout does not match requested commit"

if docker info >/dev/null 2>&1; then
  D=(docker)
elif sudo -n docker info >/dev/null 2>&1; then
  D=(sudo -n docker)
else
  fail "No existing Docker permission; no host changes made"
fi
"${D[@]}" compose version
command -v python3 >/dev/null || fail "Python 3 required"
command -v flock >/dev/null || fail "flock required"

state="$HOME/.local/state/p2p-relay-deploy"
mkdir -p "$state/backups"
chmod 700 "$state" "$state/backups"
exec 9>"$state/deploy.lock"
flock -n 9 || fail "Another relay deployment is active"

# Docker Compose labels, port and persistent volume are checked BEFORE any mutation.
project=p2p-relay-test
service=relay-test
mapfile -t ids < <("${D[@]}" ps --filter "label=com.docker.compose.project=$project" \
  --filter "label=com.docker.compose.service=$service" --format '{{.ID}}')
[[ "${#ids[@]}" == 1 ]] || fail "Expected exactly one running staging Compose container; refuse to guess or create a new stack"
cid="${ids[0]}"
meta="$("${D[@]}" inspect --format '{{json .HostConfig.PortBindings}}' "$cid")"
mounts="$("${D[@]}" inspect --format '{{json .Mounts}}' "$cid")"
python3 - "$meta" "$mounts" <<'PY'
import json,sys
ports,mounts=map(json.loads,sys.argv[1:])
assert ports == {"8080/tcp":[{"HostIp":"127.0.0.1","HostPort":"18080"}]}, "Unexpected port bindings; refuse update"
assert len(mounts)==1 and mounts[0]["Type"]=="volume" and mounts[0]["Name"]=="p2p-relay-test_relay_test_data" and mounts[0]["Destination"]=="/data" and mounts[0]["RW"], "Unexpected data mount; refuse update"
PY

# Preserve the existing running container's exact bearer token; never display it.
TEST_RELAY_ACCESS_TOKEN="$("${D[@]}" inspect --format '{{range .Config.Env}}{{println .}}{{end}}' "$cid" | python3 -c 'import sys; v=[s.rstrip("\n").split("=",1)[1] for s in sys.stdin if s.startswith("RELAY_ACCESS_TOKEN=")]; assert len(v)==1 and 32<=len(v[0])<=512; print(v[0],end="")')"
[[ -n "$TEST_RELAY_ACCESS_TOKEN" ]] || fail "Existing relay credential unavailable"
export TEST_RELAY_ACCESS_TOKEN
# Existing configuration is preserved and no .env is sourced or rewritten.
existing_store="$("${D[@]}" inspect --format '{{range .Config.Env}}{{println .}}{{end}}' "$cid" | grep '^DEVICE_STORE_PATH=' || true)"
[[ "$existing_store" == "DEVICE_STORE_PATH=/data/devices.json" ]] || fail "Unexpected existing device registry path"

probe() {
  python3 - <<'PY'
import os, urllib.request, urllib.error, json
token=os.environ["TEST_RELAY_ACCESS_TOKEN"]
for base in ("http://127.0.0.1:18080","https://p2ptest.aikissgo.com"):
    with urllib.request.urlopen(base+"/healthz",timeout=10) as r:
        assert r.status==200 and r.read().strip()==b"ok", "Health response invalid"
    req=urllib.request.Request(base+"/api/v1/devices/register",data=b"{}",headers={
        "Authorization":"Bearer "+token,"Content-Type":"application/json","X-Forwarded-Proto":"https"})
    try:
        with urllib.request.urlopen(req,timeout=10):
            raise AssertionError("Invalid registration unexpectedly accepted")
    except urllib.error.HTTPError as e:
        assert e.code==400 and json.loads(e.read()).get("error")=="invalid device registration", "Unexpected registration response"
    print(base+": health and authenticated pairing route OK")
PY
}
echo "Checking existing staging relay"
probe || fail "Existing local/public service verification failed; no deployment attempted"

previous="$("${D[@]}" inspect --format '{{.Image}}' "$cid")"
[[ "$previous" =~ ^sha256:[0-9a-f]{64}$ ]] || fail "Unexpected previous image ID"
backup="$state/backups/${GITHUB_RUN_ID}-${GITHUB_RUN_ATTEMPT}"
mkdir -p "$backup"
printf '%s\n' "$previous" > "$backup/previous-image"
if "${D[@]}" exec "$cid" test -f /data/devices.json; then
  "${D[@]}" exec "$cid" cat /data/devices.json > "$backup/devices.json"
  python3 -c 'import json,sys; json.load(open(sys.argv[1]))' "$backup/devices.json"
  echo "Device registry backed up locally; original volume remains in place."
else
  echo "No existing registry file; persistent volume will still be retained."
fi

image="p2p-relay-test:v20-${GITHUB_SHA}"
rollback="p2p-relay-test:rollback-${GITHUB_RUN_ID}-${GITHUB_RUN_ATTEMPT}"
"${D[@]}" tag "$previous" "$rollback"
"${D[@]}" build --label "org.opencontainers.image.revision=$GITHUB_SHA" -t "$image" ./relay

temp="$(mktemp -d)"
override="$temp/image.yml"
cleanup() { rm -rf -- "$temp"; unset TEST_RELAY_ACCESS_TOKEN; }
trap cleanup EXIT
set_image() {
  printf 'services:\n  relay-test:\n    image: %s\n' "$1" > "$override"
}
compose() {
  "${D[@]}" compose --project-name "$project" \
    -f "$GITHUB_WORKSPACE/deploy/compose.test.yml" -f "$override" "$@"
}
set_image "$image"
compose config --quiet
# No proxy, firewall, host service or unrelated container changes.
# Do not remove the named volume or prune the rollback image.
rollback_on_error() {
  local status=$?
  trap - ERR
  echo "::error::New staging image did not pass deployment; restoring previous image"
  set_image "$rollback"
  if compose up -d --no-build --no-deps --force-recreate "$service"; then
    for i in $(seq 1 12); do
      if probe; then echo "Previous relay restored"; exit "$status"; fi
      sleep 3
    done
  fi
  echo "::error::Rollback health not confirmed; manual inspection required. Original volume and backup retained."
  exit 1
}
trap rollback_on_error ERR
compose up -d --no-build --no-deps --force-recreate "$service"
ready=0
for i in $(seq 1 20); do
  if probe; then ready=1; break; fi
  sleep 3
done
[[ "$ready" == 1 ]]
new_id="$(compose ps -q "$service")"
[[ -n "$new_id" ]]
actual="$("${D[@]}" inspect --format '{{.Image}}' "$new_id")"
expected="$("${D[@]}" image inspect --format '{{.Id}}' "$image")"
[[ "$actual" == "$expected" ]]
trap - ERR
printf 'Deployed commit: %s\nImage: %s\nRunner: %s\n' "$GITHUB_SHA" "$actual" "$RUNNER_NAME"
printf '### p2ptest Relay deployment\n- Commit: %s\n- Runner: %s\n- Local and public health and authenticated pairing probe: PASS\n- Previous image and persistent registry retained.\n' "$GITHUB_SHA" "$RUNNER_NAME" >> "$GITHUB_STEP_SUMMARY"
