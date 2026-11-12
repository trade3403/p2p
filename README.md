# P2P Remote V0.10 — Encrypted UDP JPEG Preview (Development Test)

Private Windows Rust/Iced host + Android Kotlin/Jetpack Compose controller + Go VPS relay. This branch builds on V0.9's authenticated UDP peer connectivity tests. **V0.10 is a bounded UDP video-preview experiment, not V1.0 and not a production-grade P2P remote desktop.**

## New in V0.10

- **True encrypted UDP image transport:** when both peers complete the V0.9 AES-GCM challenge/ACK handshake, Android sends an encrypted preview-start request (kind 13). Windows streams an experimental **JPEG-only, 640×360 max, approximately 3 FPS, maximum 20 seconds** preview on the same still-open, STUN-tested UDP socket. No direct TCP LAN or VPS media forwarding is needed during those preview frames.
- **Separate explicit Windows permission:** the original "允许 UDP 连通性测试" and a new **"额外允许 UDP JPEG 预览"** switch are both **OFF by default**. They must be selected while the host is stopped. Neither setting turns on mouse/keyboard permissions.
- **Authenticated UDP video fragments:** packets are individually AES-256-GCM encrypted (kind 14), with a bounded frame ID, fragment count/index and JPEG byte length. Limits: **96 KiB per JPEG**, **900 bytes per fragment**, **110 fragments per JPEG** and less than 1 KiB encrypted UDP payload. Each fragment is independently authenticated.
- **Android JPEG reassembly:** a single bounded in-flight frame, out-of-order fragments, duplicate filtering, replay suppression by increasing frame ID, 1.5-second reassembly timeout and JPEG header validation. Only complete, authenticated JPEG frames are displayed.
- **End-to-end delivery acknowledgement:** Android sends AES-GCM authenticated frame-complete ACKs (kind 15). If Windows receives no valid ACK for **3 seconds**, it automatically stops UDP preview and returns to VPS WSS. UDP preview also stops after **20 seconds**, on Windows host stop, WebSocket session drop, or LAN session takeover.
- **CPU/backpressure:** while UDP preview is active, the full-size WSS video encoder pauses instead of wasting CPU generating unused frames. Authenticated remote mouse, clipboard, monitor controls and session signaling remain on WSS (not UDP).
- **Android automatic display fallback:** the last successfully received UDP preview image is shown in the existing remote-view panel; when UDP ends, normal JPEG/H.264 frames from VPS WSS restore the display. The UI clearly identifies which path is active.
- **Protocol checks:** Rust unit tests for encrypted UDP framing/fragment limits and tamper rejection; Android JVM tests for reordering, replay, malformed packets, wrong key, incomplete and timed-out reassembly.

## Security model

Existing per-session AES-GCM secret authenticates kind 10 candidate signaling over WSS, kind 11 HELLO, kind 12 ACK, kind 13 preview-start, kind 14 image chunks and kind 15 complete-frame receipt. A server forwarding the encrypted WSS candidate packet does **not** receive the plaintext JPEG or session key. An observer of UDP datagrams sees packet length and network endpoints, but not frame bytes.

The experimental receiver only accepts packets from the authenticated peer IP and port. Neither endpoint executes mouse, keyboard, file transfer, clipboard or OS-level commands from any UDP datagram. A shared session key still needs secure handling; this is not independently verified per-device public-key authentication. Bandwidth and stability must be verified against real owned devices and a VPS before any wider deployment.

## VPS setup

A deployed WSS VPS is still **required** for cross-network signaling, authentication and fallback. Additionally, both devices need to reach the **optional STUN UDP 3478 service** from V0.8:

```bash
cp deploy/.env.example deploy/.env
# Configure relay domain and a fresh access token. Never commit .env.
docker compose --env-file deploy/.env \
  -f deploy/compose.yml -f deploy/compose.stun.yml \
  up -d --build
```

The VPS provider firewall must permit UDP 3478 when STUN is opted in. Keep relay 8080 private behind TLS-terminating Caddy. **Do not** expose the Windows LAN TCP 43821 port to the public Internet. Some networks (symmetric NAT, CGNAT, carrier firewalls) prevent UDP hole punching; in those cases the WSS remote desktop still works.

## How to test on your devices

1. Install **Windows V0.10 and Android V0.10 from the same successful Actions run**, after CI is operational. These are unsigned EXE and debug APK test builds.
2. Start Windows on your own logged-in desktop. Check **允许已配对 Android 发起 UDP 直连测试** and separately check **额外允许 UDP JPEG 预览**. Start service with your WSS VPS credentials.
3. Android connects over **VPS WSS**, not local LAN, with the matching relay token and 64-hex session key. Use **测试公网 UDP 直连**.
4. If V0.9 STUN and encrypted UDP HELLO/ACK succeed on both sides, Windows starts a **maximum 20-second** JPEG preview after the Android encrypted kind-13 request.
5. Android should show a count of preview frames and display **公网加密 UDP JPEG 预览**. The mouse, keyboard and clipboard still use WSS.
6. After 20 seconds, if 3 seconds of complete-frame ACKs are missed, or after disconnect, the app should resume VPS video automatically.
7. Test UDP port blocked, wrong key, disabled Windows permission, high packet loss, Wi-Fi↔5G, symmetric NAT, unresponsive decoder, app stop, and Windows window close. These real-device test cases have **not** been performed by GitHub.
8. Compare FPS, loss, latency, CPU, uplink bandwidth and memory with the existing WSS video path.

## Current version and CI state

V0.10 branch: `feature/v0.10-udp-jpeg-preview`. Parent: `feature/v0.9-udp-connectivity`. The last successfully CI-built `main` remains V0.7 until V0.8/V0.9/V0.10 are individually validated.

Workflow: `.github/workflows/v10.yml`. When GitHub-hosted runners work again, this is intended to generate:
- `P2PRemote-Windows-v0.10.0-test` (portable Rust EXE in ZIP and SHA-256 checksum)
- `P2PRemote-Android-v0.10.0-debug` (debug APK and SHA-256 checksum)
- Go relay STUN/race tests and Docker build checks

**GitHub Actions blocker:** V0.8 and V0.9 Jobs repeatedly failed **before the first step** with empty steps/logs; root cause not confirmed (see [Issue #9](https://github.com/chengzi1994999/p2p/issues/9)). Never claim V0.10 Windows or Android executables exist until all jobs run, pass and publish actual artifacts.

This implementation does **not** claim complete ICE/TURN/QUIC, 1080p30 hardware H.264, seamless arbitrary-length WAN P2P remote desktop, reliability under all NAT types, audio, files or production security compliance. See [docs/V0.10-ACCEPTANCE.md](docs/V0.10-ACCEPTANCE.md).
