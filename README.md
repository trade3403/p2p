# P2P Remote V0.9 — Authenticated Cross-Network UDP Connectivity Test

Private Windows Rust/Iced desktop host, Android Kotlin/Jetpack Compose controller, self-hosted Go WebSocket relay and optional Go STUN Binding service.

**V0.9 actively attempts an authenticated UDP hole-punch between the two devices.** It is a 12-second connectivity test, **not full internet P2P remote desktop**: JPEG/H.264 screen frames, mouse and clipboard remain on the existing WSS VPS relay (or the V0.7 LAN WebSocket). Do not confuse STUN-mapped IPs or successful UDP test packets with media transport migration.

## New in V0.9

- **Windows opt-in**: new checkbox to permit a trusted Android controller to start a limited UDP connectivity test. OFF by default; user must enable before starting the Windows host.
- **Android explicit test button**: while connected to the VPS relay, tap **测试公网 UDP 直连**. The app sends a versioned encrypted request (message kind 10) to Windows over the existing WSS link. Android cannot silently start a probe on its own.
- **Encrypted candidate exchange**: both clients bind an IPv4 UDP socket, make a STUN Binding query to your own VPS UDP 3478 using that SAME socket, and share the observed IP:port over AES-256-GCM WebSocket messages. Candidate exchange never places the session key or relay token in query parameters, logs or UDP.
- **Authenticated UDP challenge/response**: each endpoint sends a fresh random 16-byte challenge in a GCM-authenticated datagram (kind 11). The peer replies in kind 12 with that challenge. A peer accepts success only after its own challenge returns over UDP from the validated endpoint, and the responder lingers briefly to help both sides finish.
- **Secure failure / no takeover**: probes stop after 12 seconds, on disconnect or when the owner stops the Windows host; failure does not disturb the WSS video/control session. There are no unauthenticated UDP mouse or clipboard commands.
- **No server schema change**: the Go relay already forwards encrypted WebSocket binary packets as opaque bytes. V0.9 includes a real bidirectional localhost WebSocket integration test verifying kind-10 signaling is delivered unchanged.
- **Test coverage**: Rust validates candidate addresses/roles, AES message-type tampering, same-socket STUN test inherited from V0.8; Kotlin validates candidate addresses and AES tampering; Go tests opaque two-way signaling plus STUN UDP loopback.

## Deploy your own VPS

Normal WSS relay remains the fallback and is required for this cross-network test:

```bash
cp deploy/.env.example deploy/.env
# Fill RELAY_DOMAIN and a new RELAY_ACCESS_TOKEN; never commit this file.
docker compose --env-file deploy/.env -f deploy/compose.yml up -d --build
```

To enable V0.8/V0.9 **optional STUN UDP Binding** on the same VPS:

```bash
docker compose --env-file deploy/.env \
  -f deploy/compose.yml -f deploy/compose.stun.yml \
  up -d --build
```

Permit UDP **3478** to the VPS in the provider firewall and apply an external rate limit. STUN port 3478 is NOT TURN and does not forward media. Port 8080 should remain private behind Caddy, and Windows LAN TCP 43821 must NOT be opened to the public Internet.

## How to verify V0.9

1. Use Windows V0.9 and Android V0.9 on devices you own, with a working WSS VPS and optional STUN 3478 configured. Windows should show a valid WSS connection first.
2. In Windows, check **允许已配对 Android 发起 UDP 直连测试** before you start the service. Keep mouse/keyboard consent separate.
3. Android connects to the same relay with your existing credentials and 64-hex session key; it must show `VPS WSS 中转` (not LAN).
4. Tap **测试公网 UDP 直连** on Android. The phone sends an authenticated request; both ends perform STUN on their own still-open UDP sockets, exchange encrypted candidates, and send encrypted HELLO/ACK packets for up to 12 seconds.
5. **Success** means the peer returned the current nonce over the authenticated UDP channel; it does NOT mean that video or input has migrated. The UI explicitly states screen transfer is still over VPS.
6. **Timeout or disabled Windows consent** leaves the existing remote session available over VPS; cellular and symmetric NAT may prevent UDP pinhole formation.
7. Restart or reconnect a WSS session to retry. A single bounded probe is permitted per authenticated session.
8. Manually test Wi-Fi vs 5G, separate routers, Windows firewalls, NAT scenarios and 8-hour relayed session stability.

## Security and interoperability

- Candidate signaling type 10 travels inside the same end-to-end encrypted WebSocket session and is not readable by the VPS. UDP kinds 11 and 12 use GCM with the header byte as authenticated AAD.
- Public UDP connectivity is explicitly consented to on Windows, and explicitly initiated by Android. No stealth service or automatic network-wide scanning.
- UDP probes are sent only to a syntactically validated, role-matched IPv4 candidate received over authenticated signaling. No DNS resolution or arbitrary hostnames in candidate packets.
- New UDP sockets are destroyed when the test ends; the WSS video/control path continues unaffected.
- STUN is unauthenticated network discovery, not user authentication. The project uses shared per-session AES key authentication for peer proof; full per-device public-key identity is still future work.
- The test is not standards-complete ICE/STUN/TURN. It does not support real cross-NAT media migration, packet retransmission, congestion control, secure QUIC fragmentation, IPv6 or every NAT/firewall topology.
- No Windows login/lock-screen/UAC bypass. CPU OpenH264 encoding remains the existing fallback and may not meet consumer remote-desktop latency targets.

## Build and version governance

Private GitHub repository: `chengzi1994999/p2p`. V0.9 development branch: `feature/v0.9-udp-connectivity` based on `feature/v0.8-network-diagnostics`, because V0.8 CI remained blocked.

GitHub Actions workflow `.github/workflows/v09.yml` generates, **only after it passes**:
- `P2PRemote-Windows-v0.9.0-test` (portable Windows Rust EXE in ZIP, SHA-256).
- `P2PRemote-Android-v0.9.0-debug` (debug APK and SHA-256).

**Important blocker:** Recent private-repo Actions runs for V0.8 returned `failure` before runner steps, with no logs. This may be an account runner allocation/billing/policy issue; the exact cause is unconfirmed. If V0.9 hits the same condition, artifacts are NOT available and the draft PR must remain unmerged. See [docs/V0.8-CI-BLOCKER.md](docs/V0.8-CI-BLOCKER.md) and [docs/V0.9-ACCEPTANCE.md](docs/V0.9-ACCEPTANCE.md).

### Proven vs unproven

A green compile proves only that tests compile/pass and artifacts are created. **Real cross-network UDP punching, true video transport switching, actual latency and stability have NOT been validated on your devices.** Do not label V0.9 as stable until those tests are complete.

Past V0.1–V0.8 history remains accessible through Git; do not squash or force-push the main branch.
