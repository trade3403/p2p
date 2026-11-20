import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:p2p_remote_ui/app_shell.dart';
import 'package:p2p_remote_ui/app_theme.dart';

void main() {
  testWidgets('mobile shell navigates between primary screens', (tester) async {
    await tester.view.setPhysicalSize(const Size(800, 900));
    await tester.view.setDevicePixelRatio(1);
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);

    await tester.pumpWidget(
      MaterialApp(
        theme: buildP2PTheme(),
        home: const AppShell(),
      ),
    );

    expect(find.text('设备概览'), findsOneWidget);
    expect(find.text('UI 原型 · 演示数据'), findsOneWidget);

    await tester.tap(find.text('设备').last);
    await tester.pumpAndSettle();
    expect(find.text('我的设备'), findsOneWidget);

    await tester.tap(find.text('远控').last);
    await tester.pumpAndSettle();
    expect(find.text('远程控制'), findsOneWidget);
    expect(find.text('视频流和远控事件尚未接入'), findsOneWidget);

    await tester.tap(find.text('我的').last);
    await tester.pumpAndSettle();
    expect(find.text('设置'), findsOneWidget);
  });
}
