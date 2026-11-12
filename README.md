# P2P Remote V0.6 — H.264 Video Channel (Test Build)

Private Windows Rust + Iced remote host, Android Kotlin/Jetpack Compose viewer, and self-hosted Go WebSocket relay.

**V0.6 introduces a real, opt-in H.264 video data path. The Windows encoder is OpenH264 software encoding, not GPU hardware encoding.** Android decodes using the system MediaCodec API (hardware support depends on the phone). JPEG remains the default compatibility mode. Neither mode is a security-audited commercial product.

## Changes from V0.5

- Windows: new frame-producer thread with bounded two-frame queue. Capture/encoding no longer blocks the async socket loop responsible for mouse, clipboard and connection commands.
- Windows: H.264 Annex-B access units via OpenH264 v0.9.9, limited to 1280×720; JPEG quality 58 remains available. Codec initializes on demand when the user toggles H.264 on. It is reset when switching displays or video formats, and every 30 frames for periodic SPS/PPS/IDR recovery.
- Android: MediaCodec AVC decoder rendering directly to TextureView. JPEG decoding and interaction controls continue to work.
- Protocol: AES-256-GCM encrypted type 5: `uint16BE width | uint16BE height | Annex-B H.264 access unit`; old JPEG type 1, control type 2, clipboard type 3, monitors type 4 remain intact.
- FPS presets: 6 / 12 / 20 requested FPS in H.264 mode; JPEG stays at approximately 6 FPS. Actual throughput depends on CPU, encoder, relay bandwidth, Android decoder and network.
- Sender queue drops stale/overflow frames instead of growing memory indefinitely; drop counter and sent packet counter shown in Windows UI.
- V0.5 trusted pairing (Windows DPAPI, Android Keystore) and explicit host permission controls remain unchanged.

## Install

From this private repository, open **Actions → V0.6 Windows Android Relay**, choose the same successful run and download its two Artifacts:

- `P2PRemote-Windows-v0.6.0-test` — ZIP containing portable `P2PRemote.exe` plus SHA256 hash.
- `P2PRemote-Android-v0.6.0-debug` — debug-signed installable APK plus SHA256 hash.

Android and Windows should both be V0.6 to use the new video packet kind; v0.5 Android does not decode type 5. Start in JPEG mode, validate a working connection, then enable H.264 on Windows.

## VPS setup is still required

A Linux VPS with DNS and Docker Compose is not automatically provisioned by GitHub Actions. In the checked-out repository:

```bash
cp deploy/.env.example deploy/.env
# configure RELAY_DOMAIN and a fresh RELAY_ACCESS_TOKEN; keep .env out of Git!
docker compose --env-file deploy/.env -f deploy/compose.yml up -d --build
curl -fsS https://YOUR_DOMAIN/healthz
```

The VPS must terminate TLS through Caddy and must **not** expose relay TCP 8080 publicly. Windows and Android both use `wss://YOUR_DOMAIN`, the same relay token, and the owner's session key. No static home IP or router port forwarding is needed.

## H.264 test checklist

1. Run Windows V0.6 and Android V0.6 on devices you own. Connect in JPEG mode and confirm the screen and explicit remote-input authorization work.
2. Enable **H.264 软件编码** in Windows while the session is active.
3. Android should display `H.264 · Android MediaCodec` and render the video in the same screen area. The first picture might need up to a few seconds until the next codec configuration/keyframe.
4. Switch from H.264 back to JPEG, change monitor, disconnect/reconnect, and verify the view recovers.
5. Change requested H.264 FPS among 6, 12 and 20; inspect sent/dropped counters. Numbers are *requests*, not measured FPS.
6. To compare mobile vs LAN, log end-to-end latency, CPU, GPU utilization, frame loss, audio absence, bandwidth and 8-hour stability. These measurements have **not** been performed in CI.

## Security and compatibility

- All video and input payloads use the existing authenticated AES-256-GCM layer. A public VPS only forwards encrypted packets; the session key must be shared privately.
- Remote input requires Windows owner's explicit permission. Windows login pre-desktop, UAC secure desktop and unattended service operation are **not** supported.
- OpenH264 source is permissively licensed (BSD-2-Clause). H.264 patent and distribution obligations may need separate review before commercial distribution.
- V0.6 does **not** implement Windows GPU/NVENC/AMF/QSV or Media Foundation hardware *encoding*. Android's system MediaCodec may select hardware or software based on device capabilities.
- JPEG and H.264 paths are both best-effort prototype implementations; remote function, animation smoothness and codec compatibility must be verified on the actual devices.
- V0.1–V0.5 history remains in Git; development branch `feature/v0.6-h264-video`. No stable release tag until on-device verification.

See [docs/V0.6-ACCEPTANCE.md](docs/V0.6-ACCEPTANCE.md).
