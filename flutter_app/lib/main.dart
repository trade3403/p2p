import 'package:flutter/material.dart';

import 'app_shell.dart';
import 'app_theme.dart';

void main() {
  WidgetsFlutterBinding.ensureInitialized();
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
