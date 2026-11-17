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

## HTTPS reverse proxy: host Nginx (confirmed by server inspection)

The Ubuntu host currently runs **Nginx directly** on TCP 80/443. There is no Caddy binary, Caddy service or running Caddy container. **Do not start the repository's production `deploy/compose.yml` on this VPS**, because its Caddy would conflict with the existing Nginx listeners.

We ship a separate staging Nginx site in `deploy/nginx.p2ptest.conf`. The upstream points to the loopback-only test container at `127.0.0.1:18080`, and forwards WebSocket Upgrade plus `X-Forwarded-Proto`.

### One-time administrator setup

First build and start the staging relay as described above. The service should answer `curl -fSs http://127.0.0.1:18080/healthz` with `ok`.

Back up existing Nginx config and check for a pre-existing staging site before proceeding:

```bash
sudo nginx -t
sudo nginx -T 2>&1 | grep -n 'server_name.*p2ptest.aikissgo.com' || true
```

If the hostname is **already configured**, review/modify that site instead of adding a duplicate.

When no existing staging site exists, install only the new configuration file:

```bash
cd ~/p2p-relay-test-source
sudo cp deploy/nginx.p2ptest.conf /etc/nginx/sites-available/p2ptest.aikissgo.com
sudo ln -s /etc/nginx/sites-available/p2ptest.aikissgo.com /etc/nginx/sites-enabled/p2ptest.aikissgo.com
sudo nginx -t && sudo systemctl reload nginx
```

If Nginx does not use Debian-style `sites-available/sites-enabled`, inspect the existing `nginx.conf` includes and use that host's established convention instead. Do **not** change the existing production site configuration.

DNS is reported to already resolve to this server. With TCP 80/443 reachable and Certbot Nginx integration installed, issue and install HTTPS for this **test hostname only**:

```bash
sudo certbot --nginx -d p2ptest.aikissgo.com --redirect
sudo nginx -t && sudo systemctl reload nginx
curl -fSs https://p2ptest.aikissgo.com/healthz
```

On Ubuntu, install the **official distribution Certbot/Nginx plugin packages** if `certbot --nginx` is unavailable. Do not assume this is already installed. The initial Nginx site is HTTP-only to allow certificate bootstrap; **do not use the test access token until HTTPS is working**. Certbot should create the HTTPS vhost and redirect HTTP traffic to HTTPS. Inspect the generated SSL config to confirm `location /` still proxies to `127.0.0.1:18080` with WebSocket headers. Check automatic renewal with `sudo certbot renew --dry-run`.

### Nginx TLS behavior

The Relay's WebSocket service refuses insecure external requests. Nginx must preserve `X-Forwarded-Proto: https` on TLS connections; the site template uses `$scheme`. Clients then connect to `wss://p2ptest.aikissgo.com` (base URL).

If `curl https://p2ptest.aikissgo.com/healthz` works but WebSocket upgrade does not, inspect the HTTPS `server` block generated by Certbot for these directives:

```nginx
proxy_http_version 1.1;
proxy_set_header Upgrade $http_upgrade;
proxy_set_header Connection "upgrade";
proxy_set_header X-Forwarded-Proto $scheme;
proxy_read_timeout 90s;
```

**Security:** the staging Relay is experimental and exposed to the Internet through Nginx. Use a distinct strong test token, restrict access where feasible, and do not assume the endpoint is production-hardened. Nginx reload and Docker deployment are operator actions, not performed by the CI Runner.

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

V0.11 CI performs offline Compose syntax verification and local loopback Relay smoke tests. CI does **not** deploy staging, verify DNS propagation or manage the host Nginx server. A manual admin step is required before the test hostname can actually serve traffic.
