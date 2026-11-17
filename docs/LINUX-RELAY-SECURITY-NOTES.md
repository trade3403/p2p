# Linux Relay security notes (V0.7 + aikissgo domain)

## Fixed / verified in this branch

- Bearer comparison uses `crypto/subtle.ConstantTimeCompare`.
- Invalid `room` / `role` → 400; missing/wrong token → 401.
- `X-Forwarded-Proto: https` is trusted **only** from loopback or private (Docker) peers; spoofing from a public IP yields 426.
- WebSocket `Origin` must be empty (native clients). Browser Origins are rejected.
- Coarse per-IP connect attempt limiter (30/min).
- Read limit 3 MiB, read/write deadlines, ping/pong, send-queue backpressure (drop on full).
- Room map cleaned when both peers leave.
- `/healthz` returns only `ok\n` (no token/rooms/peers).
- Compose keeps `8080` on the Docker network; host publishes 80/443 only.

## Explicit non-goals / residual risks

- Shared `RELAY_ACCESS_TOKEN` is **not** multi-tenant IAM.
- Android `usesCleartextTraffic` remains globally enabled for RFC1918 LAN `ws://` only; treat as an **independent** hardening follow-up before commercial release (not expanded in this domain change).
- No coturn/WebRTC/ICE in this version.

## Secrets policy

Never commit real tokens, session keys, SSH keys, or GitHub PATs. Generate tokens on the VPS with `openssl rand -hex 32` and distribute out-of-band.
