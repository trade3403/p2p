# P2P Remote v0.2 — Native Windows + Android + Self-hosted Relay

Private personal-use remote desktop project. **This is a test build, not an audited release.**

## What changed in V0.2

- Windows: native **Rust + Iced** application with modern dark UI, screenshot capture, mouse/text input, explicit control toggle.
- Android: native **Kotlin + Jetpack Compose** interface, picture viewer with pointer mapping, text input.
- VPS: lightweight **Go WebSocket relay** running behind Caddy TLS. No fixed home IP and no router port forwarding.
- Screen frames and commands are encrypted **end-to-end AES-256-GCM** before the relay; the relay only transports opaque bytes.
- GitHub Actions builds Windows portable EXE, Android debug APK, and tests the Go relay.
- V0.1 C# code stays in `windows/` for history and fallback; V0.2 source is in `desktop/`.

## Important: no VPS has been provisioned for you

You must have your own VPS, a domain name pointing to its public IP, and Docker Engine + Docker Compose. **CI building a relay container is not deploying it to a server**. Do not share VPS root passwords or API tokens in issues, commits, or chat.

### 1. VPS setup

```bash
# clone your private GitHub repo after authenticating to GitHub
cd p2p
cp deploy/.env.example deploy/.env
# edit .env: RELAY_DOMAIN points to this VPS; RELAY_ACCESS_TOKEN is a new random 64-character hex string
# generate token via: openssl rand -hex 32
docker compose --env-file deploy/.env -f deploy/compose.yml up -d --build
# Caddy requests a TLS certificate for your domain; DNS and TCP 80/443 must be reachable.
```

Open `https://YOUR_DOMAIN/healthz` and verify response `ok`.

### 2. Windows

Download **P2PRemote-Windows-v0.2.0-test** from the latest successful **V0.2 Rust / Compose / Relay** workflow run under Actions. Unzip and run `P2PRemote.exe` **only on a Windows machine you own or are authorized to access**.

Enter `wss://YOUR_DOMAIN` and the same `RELAY_ACCESS_TOKEN`, click **启动远程服务**, and copy the 64-character session secret displayed by the app.

### 3. Android

Download **P2PRemote-Android-v0.2.0-debug** from that same run, install on your own Android phone, and enter the WSS address, relay token, and Windows session secret. Tap **连接电脑**. To allow mouse/text input you must toggle authorization on the Windows app. Closing the Windows app stops the service.

### Key notes

- The relay token protects your VPS against unauthenticated relay usage; the separate session key encrypts video and commands end-to-end. Never put either token in a URL, screenshot, source control or CI logs.
- Both clients connect *outbound* to the VPS. The desktop uses WSS; the Android application rejects cleartext endpoints.
- Only a **single host and single viewer** per room. No remote login screen, UAC, Windows service, wake-on-LAN, unattended startup, H.264, audio or file transfer.
- V0.2 video is **JPEG ~6 FPS** to validate the networking and UI rewrite. Target H.264/WebRTC is later. Performance has not been bench-tested.
- This is test software: possible disconnects, data loss, errors. Test on non-sensitive data.
- No commercial branding, payment system, telemetry or external servers other than your configured VPS.

## Repository structure

| Folder | Purpose |
|---|---|
| `desktop/` | Windows Rust + Iced host |
| `android/` | Android Kotlin + Compose controller |
| `relay/` | Go relay (WebSocket transport only) |
| `deploy/` | Caddy + Compose TLS deployment |
| `.github/workflows/v02.yml` | CI for three modules |
| `windows/` | V0.1 legacy WinForms implementation (preserved) |

## Git workflow

- `main` protected as stable; develop on `feature/v0.2-rust-native`, open PR, merge after CI.
- CI artifacts are **test builds**, not signed production releases.
- Create `v0.2.0-rc1` only after both builds pass and manual test is performed. Formal `v0.2.0` after validation.

See `docs/V0.2-ACCEPTANCE.md` for criteria and outstanding work.
