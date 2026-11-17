# V0.11 staging relay: p2ptest.aikissgo.com

This is a **test environment**, not a production deployment. DNS is managed by the owner and reported to point at the Ubuntu server; live DNS, TLS, firewall and reverse-proxy configuration have **not** been verified by CI.

## Separation and binding

- Production remains `p2p.aikissgo.com` and is not touched by this staging Compose project.
- Staging Relay runs in Compose project `p2p-relay-test`; only TCP `127.0.0.1:18080` on the Ubuntu host is published. No STUN is enabled for the staging instance.
- The existing HTTPS reverse proxy must route `p2ptest.aikissgo.com` to this *staging* upstream, preserving WebSocket Upgrade and passing `X-Forwarded-Proto: https`.
- Staging and production **must not share** `RELAY_ACCESS_TOKEN`.
- Do not copy production `.env` or publish test tokens to GitHub.
- Changes to the production proxy require a maintenance/approval process; CI does not modify it.

## Bootstrap on Ubuntu (operator action, once)

Use a deployment checkout separate from `~/actions-runner/_work`; Actions checkout is ephemeral and can be wiped by the next job:

```bash
git clone https://github.com/chengzi1994999/p2p.git ~/p2p-relay-test-source
cd ~/p2p-relay-test-source
git fetch origin
git switch --track origin/feature/v0.11-windows-viewer-font
```

Create a separate token outside Git. The command below stores the generated token in the local environment file with restricted permissions:

```bash
cd ~/p2p-relay-test-source
umask 077
printf 'TEST_RELAY_ACCESS_TOKEN=%s\n' "$(openssl rand -hex 32)" > deploy/.env.test
docker compose -p p2p-relay-test --env-file deploy/.env.test -f deploy/compose.test.yml config --quiet
docker compose -p p2p-relay-test --env-file deploy/.env.test -f deploy/compose.test.yml up -d --build
curl --fail --silent --show-error http://127.0.0.1:18080/healthz
```

**Caution:** Current GitHub Actions user `ubuntu` cannot access `/var/run/docker.sock`. This is intentional on a production-adjacent runner. Run the initial Docker Compose deployment using an independently authorized server administrator rather than granting the Actions runner Docker access. The Docker commands above do not install system services automatically.

## HTTPS reverse proxy

First identify **where your existing TCP 80/443 proxy runs**, using `sudo ss -ltnp | grep -E ':(80|443) '` and `docker ps --format '{{.Names}} {{.Ports}}'`. Do not start another proxy on those ports. Use the matching option:

### A. Existing Caddy runs directly on the Ubuntu host

Add this site block to the existing Caddy configuration, without replacing its production site:

```caddyfile
p2ptest.aikissgo.com {
    encode zstd gzip
    reverse_proxy 127.0.0.1:18080
}
```

After confirming syntax and backing up the existing config, reload Caddy through its existing administration method. Do not replace the `p2p.aikissgo.com` block.

### B. Existing Caddy runs inside a Docker container

The published upstream is deliberately bound only to host loopback, so **`127.0.0.1:18080` inside Caddy's container cannot reach it**. Do not simply copy option A into a containerized Caddyfile.

Instead, attach the *staging Relay container* and *existing Caddy container* to an explicitly shared private Docker network, and route to `relay-test:8080`. This requires reviewing the current Caddy Compose project and its network names before making changes. Never expose port 18080 on `0.0.0.0` just to get around this limitation.

## Client configuration

Development-branch V0.11 Windows and Android first-run defaults are now `wss://p2ptest.aikissgo.com`. Previously saved client settings can override the new default; manually set the server field to that exact URL on an existing installation. Use the **test** `RELAY_ACCESS_TOKEN` in both clients.

The server WebSocket endpoint is `wss://p2ptest.aikissgo.com/relay?room=<64 lowercase hexadecimal characters>&role=host|viewer`. The client adds this path/query. Do not enter `/relay` when the UI asks for the base relay server address.

## Verification after staging is deployed

```bash
curl -fSs https://p2ptest.aikissgo.com/healthz
curl -s -o /dev/null -w '%{http_code}\n' https://p2ptest.aikissgo.com/relay?room=0000000000000000000000000000000000000000000000000000000000000000\&role=viewer
```

The first request should return `ok`, the second should be HTTP `401` without a token. Then validate Windows/Android WSS and display/control using owned devices. An HTTP response alone is not proof that WebSocket traffic and remote control work.

## Rollback and cleanup

```bash
cd ~/p2p-relay-test-source
docker compose -p p2p-relay-test --env-file deploy/.env.test -f deploy/compose.test.yml down
```

Remove only the added staging virtual host from your existing reverse proxy. Never tear down the production Compose project.

## CI limitations

V0.11 CI performs offline Compose syntax verification and local loopback Relay smoke tests. CI does **not** deploy staging, verify DNS propagation or manage a production Caddy instance. A manual admin step is required before the test hostname can actually serve traffic.
