import 'package:flutter/material.dart';

import '../app_page.dart';
import '../app_theme.dart';
import '../widgets/p2p_widgets.dart';

class DevicesScreen extends StatelessWidget {
  const DevicesScreen({super.key, required this.onNavigate});

  final ValueChanged<AppPage> onNavigate;

  @override
  Widget build(BuildContext context) {
    return ListView(
      children: [
        const PageHeading(
          eyebrow: 'DEVICE LIBRARY',
          title: '我的设备',
          subtitle: '管理已配对的电脑，查看在线状态并发起远程连接。',
          trailing: PrototypePill(),
        ),
        const SizedBox(height: 25),
        PanelCard(
          padding: const EdgeInsets.all(20),
          child: Row(
            children: [
              Container(
                width: 46,
                height: 46,
                decoration: BoxDecoration(
                  color: P2PColors.mintDeep,
                  borderRadius: BorderRadius.circular(14),
                ),
                child: const Icon(Icons.add_link_rounded, color: P2PColors.mint),
              ),
              const SizedBox(width: 14),
              const Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text('添加新设备', style: TextStyle(fontWeight: FontWeight.w800)),
                    SizedBox(height: 4),
                    Text('使用 9 位设备号和 6 位动态验证码完成首次配对。',
                      style: TextStyle(color: P2PColors.muted, fontSize: 12)),
                  ],
                ),
              ),
              FilledButton(
                onPressed: () => onNavigate(AppPage.remote),
                child: const Text('开始配对'),
              ),
            ],
          ),
        ),
        const SizedBox(height: 23),
        const SectionTitle(title: '已保存设备  ·  演示列表'),
        const SizedBox(height: 13),
        LayoutBuilder(
          builder: (context, constraints) {
            final columns = constraints.maxWidth > 1050 ? 3 : constraints.maxWidth > 640 ? 2 : 1;
            final width = (constraints.maxWidth - (columns - 1) * 14) / columns;
            return Wrap(
              spacing: 14,
              runSpacing: 14,
              children: [
                SizedBox(width: width, child: DeviceTile(
                  name: '办公室电脑',
                  detail: 'Windows 11 Pro · 设备号 419049723',
                  online: true,
                  icon: Icons.desktop_windows_rounded,
                  onConnect: () => onNavigate(AppPage.remote),
                )),
                SizedBox(width: width, child: DeviceTile(
                  name: '家用电脑',
                  detail: 'Windows 11 · 设备号 286104935',
                  online: true,
                  icon: Icons.computer_rounded,
                  onConnect: () => onNavigate(AppPage.remote),
                )),
                SizedBox(width: width, child: DeviceTile(
                  name: '设计笔记本',
                  detail: 'Windows 10 · 设备号 735190462',
                  online: false,
                  icon: Icons.laptop_windows_rounded,
                  onConnect: () => _showOffline(context),
                )),
              ],
            );
          },
        ),
        const SizedBox(height: 18),
        const Text(
          '原型说明：设备列表当前为静态样例，尚未读取 Relay 注册设备或 Android Keystore / Windows DPAPI 数据。',
          style: TextStyle(color: P2PColors.muted, fontSize: 11),
        ),
      ],
    );
  }

  static void _showOffline(BuildContext context) {
    ScaffoldMessenger.of(context).showSnackBar(
      const SnackBar(content: Text('演示设备当前标记为离线。')),
    );
  }
}
