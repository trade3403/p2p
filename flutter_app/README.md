# P2P Remote V0.18 — Flutter + native remote core

This branch starts the real-feature integration from the approved V0.17 Flutter UI.

## Windows integration implemented in this phase

- Flutter calls a narrow C ABI exported by `desktop/src/lib.rs` and `desktop/src/flutter_bridge.rs`.
- The Rust library reuses the existing Windows Host, Viewer, numeric pairing, AES-GCM protocol, DPAPI trusted profile, Win32 input injection, monitor selection and clipboard authorization.
- The Dashboard starts/stops the real local Host, displays the actual 9-digit device number and rotating 6-digit pairing code, and handles explicit pairing approval/denial.
- Network Settings saves the Relay URL and access token through the existing Windows DPAPI vault. The token is not returned to Dart.
- Remote Session requests numeric pairing, receives bounded latest JPEG frames, and sends mouse, keyboard, text, monitor and explicit clipboard commands.
- The C ABI keeps Rust channels, atomics, cryptographic keys and session state private. Video frames are exposed as a bounded latest-frame buffer, not an unbounded event queue.

## Not yet complete

- Android Flutter MethodChannel/EventChannel bridge. The existing Kotlin/Compose Android client remains the working Android implementation and is not replaced by this Windows-only integration build.
- Android MediaCodec-to-Flutter texture lifecycle.
- Multi-device DPAPI / Android Keystore collection migration.
- Windows tray lifecycle and installer/portable packaging parity with the legacy Iced application.
- Physical Windows-to-Windows and Android-to-Windows acceptance.

## Build on Windows

The V0.18 workflow generates the Flutter Windows runner, builds the Rust shared library, copies `p2p_remote.dll` beside the Flutter executable, then runs:

```powershell
cargo test --manifest-path desktop/Cargo.toml
cargo build --manifest-path desktop/Cargo.toml --lib --release
cd flutter_app
flutter pub get
flutter analyze
flutter test
flutter build windows --release
```

The application requires the Rust DLL in the same directory as `p2p_remote_ui.exe`.

## Security and compatibility

- No Go Relay API, WSS room/role derivation, AES-GCM framing, pairing signatures, X25519/HKDF domains or approval semantics are changed.
- Windows long-term credentials remain protected by DPAPI for the current user.
- Remote input and clipboard permissions default to disabled and must be explicitly enabled on the Host.
- The Flutter UI never receives the long-term Relay token or private signing key.
- No production or test Relay deployment is performed by this branch.
