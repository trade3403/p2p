# P2P Remote V0.8 — STUN Network Diagnostics + Encrypted RTT

Private, personal-use Windows Rust/Iced host + Android Kotlin Compose controller + Go WebSocket relay. **This is an unsigned test build.** The implementation has not been tested against a real VPS or Windows/Android devices.

## V0.8 delivers

1. **Optional self-hosted STUN Binding server (UDP 3478).** The Go relay supports RFC 8489-style IPv4 Binding requests/responses with XOR-MAPPED-ADDRESS, exact transaction-ID echo, input validation, and bounded reply size. It is **disabled by default** and must be enabled using the additional Compose file `deploy/compose.stun.yml`.
2. **Windows public UDP mapping probe.** A visible "探测公网 UDP 映射" button sends a single STUN Binding request to the same host as your WSS relay, over UDP 3478. It reports observed public IPv4:port and RTT (or a clear failure); no secret, bearer token, desktop video or clipboard data are sent.
3. **Android public UDP mapping probe.** A "检测公网 UDP" button performs the equivalent test off the UI thread, rejects malformed responses and matches the 96-bit random transaction ID.
4. **Actual encrypted remote-session signaling RTT.** Every ~5 seconds while connected, Android sends an AES-256-GCM authenticated 8-byte nonce, packet kind 8 (PING). The Windows host echoes it as kind 9 (PONG) through the active LAN-direct or VPS relay transport. Android shows the resulting application-level RTT in milliseconds.
5. **V0.7 transport remains.** On trusted LAN, Android can directly connect to Windows TCP 43821 after encrypted proof; when unavailable it falls back to the existing WSS VPS relay. Windows OpenH264 software video, Android MediaCodec, clipboard permission and trusted pairing remain intact.
6. **Automated tests** for Go STUN protocol parsing and actual localhost UDP roundtrip, Rust STUN parsing and UDP test, Android JVM STUN parsing, plus existing end-to-end encrypted protocol and app builds.

## Critical limitation: this is NOT public NAT traversal

**A STUN Binding response describes an observed UDP address/port for one temporary socket; it does not prove that a second device can connect to it.** V0.8 closes the diagnostic UDP socket after receiving a response. It does **not** implement ICE candidate exchange, persistent UDP sockets, simultaneous hole punching, secure UDP/QUIC media transport, or TURN. Public P2P success cannot be claimed until those components and tests exist.

For Android on 5G when away from home, use your private VPS WSS relay. Home broadband still does not need a fixed public IP for the VPS transport.

## Download test builds

Open the private GitHub repository `chengzi1994999/p2p` → **Actions** → **V0.8 Windows Android Relay** → a successful run → **Artifacts**.

- `P2PRemote-Windows-v0.8.0-test`: ZIP containing portable `P2PRemote.exe`, and SHA-256 checksum.
- `P2PRemote-Android-v0.8.0-debug`: Android debug APK and SHA-256 checksum.

Both are test artifacts, not signed production installers. For RTT measurement the host must be updated to V0.8 because earlier versions do not reply to the new PING message type.

## VPS setup — optional STUN diagnostics

Normal HTTPS/WSS relay remains configured using `deploy/compose.yml`:

```bash
cp deploy/.env.example deploy/.env
# Edit deploy/.env: RELAY_DOMAIN and RELAY_ACCESS_TOKEN.
# Generate a new access token: openssl rand -hex 32
docker compose --env-file deploy/.env -f deploy/compose.yml up -d --build
```

STUN is **not** enabled by that command. If you choose to expose UDP STUN on an authorized VPS, add the override:

```bash
docker compose --env-file deploy/.env \
  -f deploy/compose.yml -f deploy/compose.stun.yml \
  up -d --build
```

This starts Go's optional IPv4 STUN listener on UDP 3478, published by Docker as `3478:3478/udp`. Permit UDP 3478 through your VPS provider firewall **only if needed**, and apply an external rate limit to reduce abuse. This server is a basic Binding diagnostic, **not a fully RFC-8489-compliant internet STUN service** or TURN relay. The result reveals public IP/port to the requesting client only; the server does not log client addresses or traffic contents.

For an external network: `curl -fsS https://YOUR_DOMAIN/healthz` verifies the HTTPS relay only; **it does not test UDP STUN**. Use the new client diagnostic buttons to verify actual UDP response.

## Use V0.8

1. Start the Windows application on your own PC, configure your WSS server and bearer token if using a VPS. Consent is required before remote mouse and keyboard operations.
2. Open the Android V0.8 app; enter WSS, bearer token and session key, or use an already saved trusted profile. Android may try LAN direct before the VPS.
3. When connected, Android displays the current transport and its encrypted **signaling RTT**; this is not video decode/render latency.
4. On each platform, tap the STUN diagnostic button. A successful result shows only a **temporary observed address/port**. Failures generally indicate missing UDP 3478 service, DNS/UDP firewall restrictions, NAT/proxy changes or an invalid response.
5. Compare LAN-direct versus WSS relay RTT on owned devices and run repeated 5G/Wi-Fi tests. No guarantees are made about real-world latency from GitHub Actions.

## Security

- STUN tests are user-triggered only. No private session key, VPS token or desktop content is included in packets.
- STUN uses unauthenticated UDP as designed for discovery; keep server exposure optional and firewall rate-limited. The STUN result must never be treated as proof of a remote device's identity.
- Actual remote commands/video remain encrypted with existing AES-256-GCM session key. Input and clipboard require explicit owner permission.
- Android's V0.7 private-LAN WebSocket uses cleartext at the transport layer only for private IPv4, with mandatory AES-GCM application authentication. Android manifest currently enables cleartext globally for this test feature; a scoped policy or TLS/QUIC design is needed before commercial release.
- No Windows lock-screen or UAC secure desktop bypass, stealth background control, P2P Internet transport, audio, full file transfer, paid SDK or commercial signed release.

See [docs/V0.8-ACCEPTANCE.md](docs/V0.8-ACCEPTANCE.md).
