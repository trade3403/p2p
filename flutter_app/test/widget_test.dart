import 'package:material_ui/material_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:p2p_remote_ui/app_shell.dart';
import 'package:p2p_remote_ui/app_theme.dart';

void main() {
  testWidgets('mobile shell navigates between primary screens', (tester) async {
    tester.view.physicalSize = const Size(800, 900);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);

    await tester.pumpWidget(
      MaterialApp(
        theme: buildP2PTheme(),
        home: const AppShell(showNativeWindowFrame: false),
      ),
    );

    expect(find.text('设备概览'), findsOneWidget);
    expect(find.text('P2P REMOTE  /  V0.20'), findsOneWidget);

    await tester.tap(find.text('设备').last);
    await tester.pumpAndSettle();
    expect(find.text('我的设备'), findsOneWidget);

    await tester.tap(find.text('远控').last);
    await tester.pumpAndSettle();
    expect(find.text('远程控制'), findsOneWidget);
    expect(find.text('连接 Windows 电脑'), findsOneWidget);
    expect(find.text('远程桌面'), findsOneWidget);

    await tester.tap(find.text('我的').last);
    await tester.pumpAndSettle();
    expect(find.text('设置'), findsOneWidget);

    await tester.tap(find.text('安全与权限'));
    await tester.pumpAndSettle();
    expect(find.text('允许远程鼠标和键盘'), findsOneWidget);

    await tester.tap(find.text('我的').last);
    await tester.pumpAndSettle();
    await tester.tap(find.text('网络连接'));
    await tester.pumpAndSettle();
    expect(find.text('中转服务器'), findsOneWidget);
  });
}
