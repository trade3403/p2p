# P2P Remote V0.7 — LAN Direct First, VPS Relay Fallback

Private Windows Rust/Iced host + Android Kotlin Compose viewer with AES-256-GCM end-to-end encrypted remote desktop. **Development test build, not a stable commercial release.**

## What's new

- **Real local-network P2P:** Windows can explicitly enable an inbound LAN WebSocket listener on TCP **43821**. It accepts only RFC1918/private IPv4 peers (and loopback for tests), and only while the owner has manually started the app and enabled the feature.
- **Encrypted key proof before video:** Android's first WebSocket message is an AES-GCM authenticated type-6 `P2P-LAN-HELLO-v1`; Windows rejects it unless it was encrypted with the correct current session key. Only after verification does Windows reply with encrypted type-7 `P2P-LAN-READY-v1` and begin streaming. Neither the session key nor the VPS bearer token is placed in the LAN URL or headers.
- **Automatic LAN-to-VPS fallback:** Android can optionally save the host's private IPv4 address (for example `192.168.1.100`). On an explicit Connect tap it tries `ws://HOST:43821/direct` first. After a failed socket/4-second handshake, or if an established LAN connection drops, the app falls back to the existing `wss://VPS/relay` link when a valid VPS token exists. The fallback is displayed in the UI.
- **Direct-only mode** is possible with no VPS token, when a valid private IPv4 is entered and Windows LAN permission is enabled. It cannot work across independent mobile/cellular networks.
- The full-screen JPEG/H.264, mouse/keyboard permissions, manually requested clipboard transfer, Windows DPAPI trust and Android Keystore trust remain intact.
- Unit tests: RFC1918 IP policy, encrypted handshake validation and a real localhost WebSocket handshake on Windows; JVM tests for Android private IPv4 input.

## Important networking limitation

**This release does NOT implement Internet NAT traversal/ICE/STUN/TURN or automatic cross-subnet P2P.** It provides actual **same-LAN direct** connection without VPS and automatically chooses VPS when the direct connection fails. For Android on 5G outside home, the VPS relay is still required. Many home APs enable client isolation, which can prevent even same-Wi-Fi direct connections.

The Android app enables cleartext HTTP/WebSocket support **only to permit an explicitly configured RFC1918 LAN WebSocket**, and all LAN application messages use authenticated AES-256-GCM. The code rejects public IPs and DNS names as LAN-direct endpoints. Because `usesCleartextTraffic` is globally enabled at the OS policy level, this requires an independent security review before commercial release. Public relay connections are always `wss://`; the relay bearer token is never sent over `ws://`.

## Install

From the private repo `chengzi1994999/p2p`, open GitHub **Actions → V0.7 Windows Android Relay → a successful run → Artifacts**:

- `P2PRemote-Windows-v0.7.0-test`: ZIP including portable Windows Rust EXE and SHA-256 checksum.
- `P2PRemote-Android-v0.7.0-debug`: debug-signed APK and checksum.

Both must be V0.7. V0.6 clients can still use the original WSS relay transport, but do **not** understand the new LAN proof messages.

## Same-Wi-Fi direct test (no VPS required)

1. Both devices must be on the same private LAN. Open Windows `ipconfig` to see the host IPv4 address, e.g. `192.168.1.100`.
2. Start Windows V0.7. Tick **允许局域网加密直连（TCP 43821）** and start remote service. Windows may prompt for a firewall rule; allow TCP 43821 **only on trusted Private networks** for this test, not on Public profiles. Never forward port 43821 on your router.
3. Open Android V0.7, enter the Windows private IPv4 and the matching 64-character E2E session key. WSS server/token are optional in direct-only mode. Tap Connect.
4. Android should show **局域网 P2P 直连** after encrypted proof, followed by JPEG/H.264 frame view. Remote input still requires explicit owner authorization in Windows.
5. With an owned VPS configured, block port 43821 or leave the private network. Android should report switching to **VPS WSS 中转**. Relay must be deployed, reachable and configured before this will succeed.

## VPS relay (fallback mode)

Formal production domain: **`p2p.aikissgo.com`**. Windows and Android **new installs** default to `wss://p2p.aikissgo.com`. Saved user settings / trusted pairings are **not** silently overwritten.

Linux deploy scripts live under `deploy/` (`install.sh`, `update.sh`, `verify.sh`, `README-LINUX.md`). The server is **not automatically deployed by GitHub CI** — run install on the VPS after DNS is ready, or use a protected GitHub Environment later. See [deploy/README-LINUX.md](deploy/README-LINUX.md).

```bash
# On the Ubuntu VPS (after DNS A record points here):
sudo bash deploy/install.sh --domain p2p.aikissgo.com
# Or manually:
cp deploy/.env.example deploy/.env
# Set RELAY_DOMAIN=p2p.aikissgo.com and RELAY_ACCESS_TOKEN=$(openssl rand -hex 32)
docker compose --env-file deploy/.env -f deploy/compose.yml up -d --build
```

Both devices initiate outbound WSS to the VPS, so a home broadband static IP or port forwarding is unnecessary for the fallback path. Never expose Go relay port 8080 directly to the Internet.

**Auth model:** `RELAY_ACCESS_TOKEN` is a **shared** VPS access token for your personal devices — **not** a production multi-tenant / per-device IAM system. End-to-end AES-256-GCM session keys remain separate client secrets.

## Safety and limitations

- LAN transport uses `ws://` at the network layer; confidentiality and authorization are provided by AES-256-GCM encrypted payloads and a mandatory key-proof handshake. Do not send credentials or personal information in plaintext outside the encrypted protocol.
- The Windows listener accepts private IPv4 peers only, requires the owner to enable LAN for the current session, and accepts a single LAN viewer per listener at a time.
- Do not interpret "LAN P2P" as NAT hole-punching: different routers or 5G require a VPS relay until ICE/STUN and compatible secure UDP transport are implemented.
- Session-key sharing is still manual or via locally encrypted trusted profiles; there is no independent per-device identity at the relay.
- Closing the visible Windows application or stopping the session ends streaming. Windows lock screen/UAC and pre-login services remain unsupported.
- Windows H.264 uses OpenH264 software encoding, not GPU NVENC/QSV/AMF; video performance must be measured on real devices.
- Real Windows firewall rules, multiple home routers, Android device compatibility, long-run stability and WAN fallback are **not** tested by GitHub Actions.

See [docs/V0.7-ACCEPTANCE.md](docs/V0.7-ACCEPTANCE.md). V0.1–V0.6 are retained in Git history.
