import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:material_ui/material_ui.dart';

import '../app_page.dart';
import '../app_theme.dart';
import '../services/remote_controller.dart';
import '../widgets/p2p_widgets.dart';

class DashboardScreen extends StatelessWidget {
  const DashboardScreen({super.key, required this.onNavigate});

  final ValueChanged<AppPage> onNavigate;

  @override
  Widget build(BuildContext context) {
    final controller = P2PRemoteController.instance;
    return AnimatedBuilder(
      animation: controller,
      builder: (context, _) {
        return ListView(
          children: [
            PageHeading(
              eyebrow: 'P2P REMOTE  /  V0.18',
              title: '设备概览',
              subtitle: '管理本机共享服务，或连接已授权的远程电脑。',
              trailing: StatusDot(
                color: controller.hostRunning || controller.viewerConnected
                    ? P2PColors.mint
                    : P2PColors.muted,
                label: controller.hostRunning
                    ? '本机共享运行中'
                    : controller.viewerConnected
                        ? '远程会话已连接'
                        : '未连接',
              ),
            ),
            const SizedBox(height: 24),
            if (defaultTargetPlatform == TargetPlatform.android)
              const _AndroidBridgeNotice()
            else
              _LocalDeviceCard(onSettings: () => onNavigate(AppPage.network)),
            const SizedBox(height: 16),
            LayoutBuilder(
              builder: (context, constraints) {
                final columns = constraints.maxWidth > 1050 ? 3 : constraints.maxWidth > 650 ? 2 : 1;
                final width = (constraints.maxWidth - (columns - 1) * 14) / columns;
                return Wrap(
                  spacing: 14,
                  runSpacing: 14,
                  children: [
                    SizedBox(
                      width: width,
                      child: _OverviewCard(
                        icon: Icons.shield_rounded,
                        title: '远程输入',
                        value: controller.allowInput ? '已授权' : '未授权',
                        detail: '由本机用户明确控制',
                        accent: controller.allowInput ? P2PColors.mint : P2PColors.amber,
                      ),
                    ),
                    SizedBox(
                      width: width,
                      child: _OverviewCard(
                        icon: Icons.content_paste_rounded,
                        title: '剪贴板',
                        value: controller.allowClipboard ? '已授权' : '未授权',
                        detail: '仅按需读取或发送',
                        accent: controller.allowClipboard ? P2PColors.mint : P2PColors.amber,
                      ),
                    ),
                    SizedBox(
                      width: width,
                      child: _OverviewCard(
                        icon: Icons.hub_rounded,
                        title: '连接服务',
                        value: controller.tokenConfigured ? '凭据已保存' : '尚未配置',
                        detail: controller.relayUrl,
                        accent: controller.tokenConfigured ? P2PColors.blue : P2PColors.amber,
                      ),
                    ),
                  ],
                );
              },
            ),
            const SizedBox(height: 22),
            PanelCard(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  const Text('远程控制', style: TextStyle(fontSize: 17, fontWeight: FontWeight.w800)),
                  const SizedBox(height: 5),
                  const Text(
                    '输入对方电脑的9位设备号和6位动态验证码。首次连接仍需被控电脑批准。',
                    style: TextStyle(color: P2PColors.muted, fontSize: 12),
                  ),
                  const SizedBox(height: 14),
                  FilledButton.icon(
                    onPressed: () => onNavigate(AppPage.remote),
                    icon: const Icon(Icons.desktop_windows_rounded),
                    label: const Text('打开远程控制'),
                  ),
                ],
              ),
            ),
            if (!controller.bridgeAvailable) ...[
              const SizedBox(height: 16),
              PanelCard(
                child: Text(
                  controller.bridgeError,
                  style: const TextStyle(color: P2PColors.amber, fontSize: 12),
                ),
              ),
            ],
          ],
        );
      },
    );
  }
}

class _AndroidBridgeNotice extends StatelessWidget {
  const _AndroidBridgeNotice();

  @override
  Widget build(BuildContext context) {
    return const PanelCard(
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text('Android 控制端', style: TextStyle(fontSize: 18, fontWeight: FontWeight.w800)),
          SizedBox(height: 7),
          Text(
            'Android 原生远控功能仍由现有 Kotlin 客户端提供。Flutter 页面与 Android 会话、MediaCodec 纹理的桥接正在 V0.18 后续阶段接入。',
            style: TextStyle(color: P2PColors.muted, fontSize: 12, height: 1.5),
          ),
        ],
      ),
    );
  }
}

class _LocalDeviceCard extends StatelessWidget {
  const _LocalDeviceCard({required this.onSettings});

  final VoidCallback onSettings;

  @override
  Widget build(BuildContext context) {
    final controller = P2PRemoteController.instance;
    return AnimatedBuilder(
      animation: controller,
      builder: (context, _) {
        final running = controller.hostRunning;
        final hasCode = controller.pairingCode.isNotEmpty;
        return PanelCard(
          padding: const EdgeInsets.all(25),
          gradient: const LinearGradient(
            colors: [Color(0xFF1A3549), Color(0xFF17273D), Color(0xFF142238)],
            begin: Alignment.topLeft,
            end: Alignment.bottomRight,
          ),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Row(
                children: [
                  const Expanded(
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Text('本机 Windows 设备', style: TextStyle(color: P2PColors.muted, fontSize: 13)),
                        SizedBox(height: 5),
                        Text('启动共享后，其他设备可通过数字配对连接。',
                          style: TextStyle(color: P2PColors.text, fontSize: 17, fontWeight: FontWeight.w800)),
                      ],
                    ),
                  ),
                  StatusDot(
                    color: running ? P2PColors.mint : P2PColors.muted,
                    label: running ? '共享已启动' : '共享已停止',
                  ),
                ],
              ),
              const SizedBox(height: 20),
              if (controller.deviceNumber.isNotEmpty || hasCode)
                LayoutBuilder(
                  builder: (context, constraints) {
                    final compact = constraints.maxWidth < 620;
                    final items = [
                      _NumberBlock(
                        label: '9位设备号',
                        value: controller.deviceNumber.isEmpty ? '注册中…' : controller.deviceNumber,
                        helper: '用于识别设备，不是访问凭据。',
                      ),
                      _NumberBlock(
                        label: '6位动态验证码',
                        value: hasCode ? controller.pairingCode : '等待生成…',
                        helper: '由中转服务定时轮换；仅用于首次授权。',
                        amber: true,
                      ),
                    ];
                    return compact
                        ? Column(
                            crossAxisAlignment: CrossAxisAlignment.start,
                            children: [items[0], const SizedBox(height: 18), items[1]],
                          )
                        : Row(
                            children: [
                              Expanded(child: items[0]),
                              const SizedBox(width: 24),
                              const SizedBox(height: 78, child: VerticalDivider(color: P2PColors.outline)),
                              const SizedBox(width: 24),
                              Expanded(child: items[1]),
                            ],
                          );
                  },
                )
              else
                const Text(
                  '尚未启动共享，因此还没有设备号或动态验证码。',
                  style: TextStyle(color: P2PColors.muted, fontSize: 12),
                ),
              const SizedBox(height: 14),
              Wrap(
                spacing: 9,
                runSpacing: 9,
                children: [
                  if (controller.deviceNumber.isNotEmpty)
                    OutlinedButton.icon(
                      onPressed: () => _copy(context, controller.deviceNumber, '设备号'),
                      icon: const Icon(Icons.copy_rounded, size: 15),
                      label: const Text('复制设备号'),
                    ),
                  if (hasCode)
                    OutlinedButton.icon(
                      onPressed: () => _copy(context, controller.pairingCode, '验证码'),
                      icon: const Icon(Icons.copy_rounded, size: 15),
                      label: const Text('复制验证码'),
                    ),
                  if (!controller.tokenConfigured)
                    OutlinedButton.icon(
                      onPressed: onSettings,
                      icon: const Icon(Icons.settings_rounded, size: 15),
                      label: const Text('配置中转服务'),
                    ),
                  FilledButton.icon(
                    onPressed: !controller.bridgeAvailable || (!running && !controller.tokenConfigured)
                        ? onSettings
                        : () => running ? controller.stopHost() : controller.startHost(),
                    icon: Icon(running ? Icons.stop_rounded : Icons.play_arrow_rounded),
                    label: Text(running ? '停止共享' : '启动本机共享'),
                  ),
                ],
              ),
              const SizedBox(height: 12),
              Text(
                controller.hostStatus,
                style: const TextStyle(color: P2PColors.muted, fontSize: 12),
              ),
              if (controller.pendingRequestId != null) ...[
                const SizedBox(height: 14),
                Container(
                  padding: const EdgeInsets.all(14),
                  decoration: BoxDecoration(
                    color: const Color(0xFF302A22),
                    borderRadius: BorderRadius.circular(12),
                    border: Border.all(color: const Color(0xFF75542D)),
                  ),
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      const Text('待确认的连接请求', style: TextStyle(fontWeight: FontWeight.w800)),
                      const SizedBox(height: 5),
                      Text(
                        '控制端指纹：${controller.pendingViewerFingerprint ?? "未知设备"}',
                        style: const TextStyle(color: P2PColors.muted, fontSize: 12),
                      ),
                      const SizedBox(height: 10),
                      Wrap(
                        spacing: 8,
                        children: [
                          FilledButton(
                            onPressed: () => controller.approvePairing(controller.pendingRequestId!),
                            child: const Text('允许本次连接'),
                          ),
                          OutlinedButton(
                            onPressed: () => controller.denyPairing(controller.pendingRequestId!),
                            child: const Text('拒绝'),
                          ),
                        ],
                      ),
                    ],
                  ),
                ),
              ],
            ],
          ),
        );
      },
    );
  }

  static Future<void> _copy(BuildContext context, String value, String label) async {
    await Clipboard.setData(ClipboardData(text: value));
    if (context.mounted) {
      ScaffoldMessenger.of(context).showSnackBar(SnackBar(content: Text('$label已复制')));
    }
  }
}

class _NumberBlock extends StatelessWidget {
  const _NumberBlock({
    required this.label,
    required this.value,
    required this.helper,
    this.amber = false,
  });

  final String label;
  final String value;
  final String helper;
  final bool amber;

  @override
  Widget build(BuildContext context) => Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(label, style: const TextStyle(color: P2PColors.muted, fontSize: 12)),
          const SizedBox(height: 8),
          SelectableText(
            value,
            style: TextStyle(
              color: amber ? P2PColors.amber : P2PColors.mint,
              fontSize: 30,
              fontWeight: FontWeight.w900,
              letterSpacing: 2,
            ),
          ),
          const SizedBox(height: 5),
          Text(helper, style: const TextStyle(color: P2PColors.muted, fontSize: 11)),
        ],
      );
}

class _OverviewCard extends StatelessWidget {
  const _OverviewCard({
    required this.icon,
    required this.title,
    required this.value,
    required this.detail,
    required this.accent,
  });

  final IconData icon;
  final String title;
  final String value;
  final String detail;
  final Color accent;

  @override
  Widget build(BuildContext context) => PanelCard(
        padding: const EdgeInsets.all(19),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Icon(icon, color: accent, size: 23),
            const SizedBox(height: 14),
            Text(title, style: const TextStyle(color: P2PColors.muted, fontSize: 12)),
            const SizedBox(height: 4),
            Text(value, style: const TextStyle(fontSize: 19, fontWeight: FontWeight.w900)),
            const SizedBox(height: 4),
            Text(detail, style: const TextStyle(color: P2PColors.muted, fontSize: 11)),
          ],
        ),
      );
}
