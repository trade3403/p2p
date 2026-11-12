# P2P Remote V0.5 — Explicit Trusted Home-PC Pairing (test build)

A private Windows Rust/Iced host + Android Kotlin Compose viewer + self-hosted Go HTTPS/WSS relay. **This is an unsigned development test build, not an audited unattended-access service.**

## New in V0.5

- **Windows current-user trusted pairing:** click **加密保存可信配对** to protect the relay bearer token and 32-byte session key using Windows DPAPI (CurrentUser). The existing public relay URL remains in regular preferences.
- **Explicit login auto-start:** optionally tick **我同意 Windows 用户登录后自动启动并连接 VPS** and Save. Adds only a per-user HKCU Run registry entry (`P2PRemote.exe --autostart`), never a hidden system service. No admin privileges, no unattended access before Windows user login.
- **Explicit persistent input consent:** a separate opt-in **我同意下次自动连接后允许已配对手机控制鼠标/键盘** controls whether input authorization is restored at the next *user-login* auto-connect. Off by default. Clipboard permission never auto-enables.
- **Revoke:** Windows **撤销所有已记住的配对** stops the session, turns input and clipboard off, removes saved DPAPI credentials, disables per-user login startup, and rotates the encryption session key. Old Android profiles cannot join the new room.
- **Android local trusted computer:** button to store one PC profile using an AES-256-GCM key in AndroidKeyStore. Only ciphertext is stored in private SharedPreferences; explicit **连接已保存的家庭电脑** connects by user request. **清除此手机** removes local ciphertext, but is **not** a Windows-side revocation.
- Closing the visible Windows app stops remote streaming and input. No invisible persistence after exit.
- **No new server-side identity** in this iteration. The existing relay still has one bearer token plus a single shared session secret; this is not per-device public-key authentication, enterprise device management, or a remote account system.

## Security and scope

Use only your own or explicitly authorized devices. The pairing secret is a password; anyone with the relay token **and** secret could connect. Never share it in screenshots or GitHub. Windows DPAPI protects data under the current user; malware running as that user could still access it. Android Keystore does not automatically require biometric authentication. Lock-screen control, UAC secure desktop, unattended pre-login Windows service, system tray, audio, files, and H.264 hardware streaming **are not implemented**.

### How to test V0.5

1. Deploy the Go relay on your own Linux VPS, configure DNS + Caddy HTTPS, and keep port 8080 private. Copy `deploy/.env.example` to `deploy/.env`, configure `RELAY_DOMAIN` and generate `RELAY_ACCESS_TOKEN` via `openssl rand -hex 32`.
2. Start Windows V0.5 `P2PRemote.exe` and enter the WSS URL and relay token. Tick either optional login/start or persistent-input consent (both default off).
3. Click **加密保存可信配对**. You can also manually click **启动远程服务**. Do not expose the session key publicly.
4. Open Android V0.5, enter the same WSS address, bearer token and 64-hex session key. Tap **加密保存配对** for one-click future use.
5. On Android tap **连接已保存的家庭电脑**. If persistent-input consent is off, allow remote input only from the running Windows UI, as before.
6. For the login-start scenario, sign out and back into the *same Windows user account*. If opted in, the visible desktop program starts and initiates the relay connection. Android still requires an explicit tap to connect.
7. Revoke from Windows to immediately stop and rotate the room key. Clearing Android local data alone does not revoke a Windows-side trusted pair.

**Important:** The Windows EXE path is recorded at time of opt-in. Moving the portable EXE afterward will break auto-start until you re-save the profile.

## Build and artifacts

In private GitHub repository `chengzi1994999/p2p`, open **Actions → V0.5 Windows Android Relay → a successful run → Artifacts**.

- `P2PRemote-Windows-v0.5.0-test`: Windows Rust/Iced EXE ZIP, SHA-256 checksum.
- `P2PRemote-Android-v0.5.0-debug`: Android Compose debug APK and SHA-256 checksum.
- Go relay `go test -race` and Docker build are also validated.

These are **unsigned test binaries**. Building them does not deploy your VPS or prove device-to-device performance. No VPS domain or access credentials are stored in Git.

## Git versioning

Development branch: `feature/v0.5-unattended-access` with PR and CI before merging to `main`. V0.1–V0.4 source stays in Git history. A stable tag/release must await real Windows/Android/VPS acceptance.

See [docs/V0.5-ACCEPTANCE.md](docs/V0.5-ACCEPTANCE.md).
