# P2P Remote V0.4 — Windows Rust + Android Compose + VPS relay

**Closed-source private development test build. Do not use on computers you do not own or have permission to access.**

## What changed in V0.4

- **Multi-monitor**: Windows Rust host enumerates displays, sends authenticated screen-count and selected-screen metadata; Android provides Previous/Next display controls. The active display is used for screen capture and mouse mapping.
- **Input improvements**: Android adds a right-click button and scroll-up/scroll-down controls. Remote input still requires explicit authorization on Windows.
- **Explicit clipboard text sharing**: Windows has a **separate, default-off clipboard permission**. Android can manually send text to the Windows clipboard or request the current Windows text, then copy the result to Android only on a separate tap. No silent clipboard polling, history capture, or automatic synchronization.
- **Protocol**: existing AES-256-GCM authenticated packets retained. New type 3 carries encrypted clipboard text; type 4 carries encrypted monitor metadata; control messages use the existing authenticated type 2. Max clipboard payload 4096 UTF-8 bytes.
- **CI**: Windows Rust Release EXE, Android Compose debug APK, Go relay Docker smoke build and tests. Every artifact is a TEST PACKAGE, not a code-signed commercial release.

## Install and test

Download artifacts from GitHub **Actions → V0.4 Windows Android Relay → latest green successful run**:
- `P2PRemote-Windows-v0.4.0-test` (ZIP containing portable EXE and SHA-256 hash).
- `P2PRemote-Android-v0.4.0-debug` (APK and SHA-256 hash).

Use the **same version on both devices**. Install the test Windows EXE on your own Windows PC and the debug APK on your Android device. Both require a VPS relay running `deploy/compose.yml` behind Caddy and a WSS certificate. CI does not automatically deploy a VPS.

### Connection steps

1. On the VPS, create `deploy/.env` from `deploy/.env.example`, configure your relay domain and a new `openssl rand -hex 32` token; run:
   ```bash
   docker compose --env-file deploy/.env -f deploy/compose.yml up -d --build
   ```
2. Start Windows `P2PRemote.exe`, input `wss://YOUR_DOMAIN` and VPS bearer token, then start service. The screen-sharing session key appears locally; do not expose it publicly.
3. On Android, input the same address, relay token, and 64-digit session key, and connect.
4. For mouse/keyboard right-click/scroll, enable **允许 Android 端操作鼠标及输入文字** on Windows.
5. For clipboard actions, separately enable **允许 Android 端按需读写 Windows 剪贴板文字**. Clipboard sharing does not start automatically.
6. Android can select a monitor; Windows can also cycle monitors. Screen coordinates follow the selected display.
7. Stop either app to end the connection; no stealth start or consent bypass.

The VPS relay transports encrypted bytes; it does not have your screen or clipboard session key. It **does** receive a relay bearer token for access control, so protect that token separately. Home broadband can have dynamic/private IP because both clients connect outbound to your VPS.

## Safety and known limitations

- No automatic clipboard sync or background monitoring. Clipboard read is explicit and guarded; do not transmit passwords or sensitive secrets.
- No file transfer in V0.4. File transfer requires secure file picker, antivirus validation, user confirmation and bounded transfer quotas; tracked for a later dedicated milestone.
- JPEG image transfer remains ~6 FPS; H.264 hardware encoding is not yet delivered. The VPS can add latency/bandwidth cost.
- Windows lock-screen/UAC/secure desktop and unattended Windows service are not supported.
- Requires actual VPS + Windows/Android device tests for functional acceptance. Green GitHub Actions alone proves only compilation/unit tests.
- Windows and Android V0.4 must be used together for new operations; avoid mixing old clients.

See [docs/V0.4-ACCEPTANCE.md](docs/V0.4-ACCEPTANCE.md). Legacy V0.1/V0.2/V0.3 remain in Git history.

## Source layout

- `desktop/`: Rust + Iced Windows host; monitor capture and clipboard permissions.
- `android/`: Kotlin + Jetpack Compose viewer and explicit remote-operations toolbar.
- `relay/`: Go WSS relay behind Caddy (`deploy/`).
- `.github/workflows/v04.yml`: test build on private GitHub repository.

**Version governance:** `main` stays build-verified; development happens on `feature/v0.4-remote-operations`. Use Pull Requests and run CI before merging. No production release tag before on-device verification.
