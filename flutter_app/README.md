# P2P Remote V0.17 Flutter UI Prototype

This is the first isolated Flutter UI implementation based on the approved P2P Remote visual direction.

## Current scope

- Responsive Windows-style dashboard and mobile navigation.
- Home, devices, remote-session preview, security, network and settings pages.
- Shared dark navy / mint design tokens.
- Demo data only. The UI is **not connected** to the Rust Host/Viewer, Android session controller, Android Keystore, Windows DPAPI or Go Relay yet.
- Buttons that would start real remote sessions intentionally report that native integration is pending; they do not fake a successful connection.

The existing Iced Windows application, Kotlin/Jetpack Compose Android application and Go Relay remain unchanged and are still the functional baseline.

## Generate platform runners

Flutter platform runner folders are generated locally so the UI prototype does not overwrite the current native Windows and Android projects:

```bash
cd flutter_app
flutter create --platforms=windows,android --project-name p2p_remote_ui .
flutter pub get
flutter analyze
flutter test
```

Run on Windows:

```bash
flutter run -d windows
```

Run on an attached Android device:

```bash
flutter devices
flutter run -d <device-id>
```

## Migration rule

Do not copy production secrets into Dart. Windows DPAPI and Android Keystore remain native security boundaries. Do not change the Relay protocol or deploy any server as part of the UI prototype.
