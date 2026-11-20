import 'dart:io';

import 'package:material_ui/material_ui.dart';
import 'package:window_manager/window_manager.dart';

import 'app_shell.dart';
import 'app_theme.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();

  if (Platform.isWindows) {
    final window = WindowManager.instance.getCurrent();
    if (window != null) {
      window.title = 'P2P Remote';
      window.titleBarStyle = TitleBarStyle.hidden;
      window.minimumSize = const Size(980, 640).toNative();
      window.hasShadow = true;
    }
  }

  runApp(const P2PRemoteApp());
}

class P2PRemoteApp extends StatelessWidget {
  const P2PRemoteApp({super.key});

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'P2P Remote',
      debugShowCheckedModeBanner: false,
      theme: buildP2PTheme(),
      home: const AppShell(),
    );
  }
}
