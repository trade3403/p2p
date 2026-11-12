# P2P Remote V0.3 · Windows Rust + Android Compose

**Development test build, not a security-audited commercial release.**

P2P Remote lets you remotely view and (after explicit host authorization) control your own Windows PC from an Android device through a self-hosted HTTPS/WSS relay. Both devices initiate outbound connections, so a home broadband connection does **not** require a static IP or router port-forwarding.

## V0.3 changes

- Windows: Rust + Iced native desktop, automatic reconnection with bounded backoff (1–30 seconds), a visible stop control, and status messages.
- Android: Kotlin + Jetpack Compose, network-failure reconnection, 20-second stalled-frame detection, a stop/cancel button, and clearer status indicators.
- Both apps remember **only the non-sensitive relay address**. The relay bearer token and 64-character session key are held only in memory during an active session; enter them again after restarting.
- End-to-end AES-256-GCM now authenticates **both packet contents and packet kind** (type is bound using AEAD AAD). V0.3 and V0.2 clients are **not protocol-compatible**. Upgrade both together.
- Go relay includes additional unit tests for unauthorized and malformed join requests. Docker Compose/Caddy deployment is retained.
- GitHub Actions now runs Windows Rust tests/build, Android unit tests/APK build, and Go race/security tests + Docker image build.

## Test-build downloads

Private GitHub repository → **Actions → V0.3 Windows Android Relay → successful run → Artifacts**.

Artifacts:
- `P2PRemote-Windows-v0.3.0-test`: ZIP containing portable `P2PRemote.exe`.
- `P2PRemote-Android-v0.3.0-debug`: debug installable APK.
- Both packages include SHA-256 checksums. These builds are **unsigned test packages**.

## VPS deployment (manual, server not provisioned)

A publicly reachable Linux VPS, a domain you control, DNS pointing to its public IP, Docker Engine, and Docker Compose are needed. Never paste or commit root passwords, signing keys, or relay access tokens.

```bash
git clone git@github.com:chengzi1994999/p2p.git
cd p2p
cp deploy/.env.example deploy/.env
# Edit deploy/.env with RELAY_DOMAIN and a newly generated RELAY_ACCESS_TOKEN
# Generate a token on the VPS: openssl rand -hex 32
docker compose --env-file deploy/.env -f deploy/compose.yml up -d --build
curl -fsS https://YOUR_DOMAIN/healthz
```

Caddy terminates HTTPS/WSS; the Go relay's internal port **8080 must not be exposed publicly**.

## Connect Windows to Android

1. Start `P2PRemote.exe` on your own Windows PC. Enter `wss://YOUR_DOMAIN` and `RELAY_ACCESS_TOKEN`; click **启动远程服务**.
2. Keep the **64-character session secret** visible only to yourself. The relay cannot decrypt the screen.
3. Install Android debug APK. Enter the same WSS address, relay token, and session secret. Tap **连接电脑**.
4. When the Android app displays the Windows screen, enable **允许 Android 端操作** in the Windows app if input control is needed; it is disabled by default.
5. Disconnect from either side to end the session. Android and Windows reconnect **only while you have explicitly requested an active session**.

### What a green CI result proves

A green workflow confirms **code compilation and automated tests**. It does not prove that a real VPS is deployed, an APK launches on every Android phone, input works across UAC/lock screens, the program is free of bugs, or that latency meets product targets. You must complete the manual acceptance checklist at [docs/V0.3-ACCEPTANCE.md](docs/V0.3-ACCEPTANCE.md).

### Current limitations

- Video still uses JPEG at ~6 FPS, not H.264 hardware encode; no audio, file transfer or multi-monitor switching.
- Android currently supports only a **single active session**; closing the app ends it. No stealth/background remote access.
- Relay credentials and session secret intentionally **are not stored**; copying them requires a trusted method.
- Automatic retry handles transient network errors, not incorrect credentials (401/403 stops retry).
- No unattended Windows service, secure-desktop access, P2P bypass or wake-on-LAN.
- Commercial distribution requires independent dependency-license and security audits, code-signing, and real device tests.

## Repository structure

| Path | Purpose |
|---|---|
| `desktop/` | Rust/Iced Windows host, encryption, reconnect state |
| `android/` | Kotlin Compose viewer, retry policy, Android tests |
| `relay/` | Go WebSocket relay; opaque encrypted data only |
| `deploy/` | Docker Compose + Caddy HTTPS deployment |
| `.github/workflows/v03.yml` | Current CI test-build workflow |
| `windows/` | V0.1 legacy C# host preserved for Git history |

## Git versions

- `main`: last merged build-verified version
- `feature/v0.3-stable-remote`: V0.3 development and testing
- `v0.3.0-rc1`: create only after actual two-client testing; a green CI alone does not warrant a stable release tag.

Private, personal-use source. No license granting others permission to redistribute is included.
