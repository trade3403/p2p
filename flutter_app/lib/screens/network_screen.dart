import 'package:material_ui/material_ui.dart';

import '../app_theme.dart';
import '../widgets/p2p_widgets.dart';

class NetworkScreen extends StatelessWidget {
  const NetworkScreen({super.key});

  @override
  Widget build(BuildContext context) {
    return ListView(
      children: [
        const PageHeading(
          eyebrow: 'NETWORK',
          title: '网络连接',
          subtitle: '查看中转服务、局域网直连和当前链路状态。',
          trailing: PrototypePill(),
        ),
        const SizedBox(height: 24),
        const PanelCard(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text('中转服务器', style: TextStyle(fontSize: 18, fontWeight: FontWeight.w800)),
              SizedBox(height: 5),
              Text('测试环境地址仅作为界面样例，不会在原型中发起网络请求。',
                style: TextStyle(color: P2PColors.muted, fontSize: 12)),
              SizedBox(height: 18),
              Text('WSS 地址', style: TextStyle(color: P2PColors.muted, fontSize: 12)),
              SizedBox(height: 7),
              SelectableText('wss://p2ptest.aikissgo.com',
                style: TextStyle(color: P2PColors.mint, fontSize: 15, fontWeight: FontWeight.w700)),
              SizedBox(height: 17),
              _NetworkRow(icon: Icons.cloud_done_rounded, title: 'Relay 中转', value: '演示状态 · 未检测'),
              _NetworkRow(icon: Icons.lan_rounded, title: '局域网直连', value: '等待设备发现'),
              _NetworkRow(icon: Icons.public_rounded, title: 'STUN 探测', value: '尚未执行'),
              _NetworkRow(icon: Icons.lock_rounded, title: '传输加密', value: 'AES-256-GCM · 协议保持不变'),
            ],
          ),
        ),
        const SizedBox(height: 15),
        PanelCard(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              const Text('连接策略', style: TextStyle(fontSize: 18, fontWeight: FontWeight.w800)),
              const SizedBox(height: 10),
              const Text('优先尝试可用的局域网 / P2P 路径；无法直连时回退到现有 WSS Relay。实际策略由 Rust / Android 原生连接核心执行。',
                style: TextStyle(color: P2PColors.muted, fontSize: 13, height: 1.55)),
              const SizedBox(height: 15),
              OutlinedButton.icon(
                onPressed: () => ScaffoldMessenger.of(context).showSnackBar(
                  const SnackBar(content: Text('原型模式：网络诊断接口尚未接入。')),
                ),
                icon: const Icon(Icons.network_check_rounded),
                label: const Text('运行网络诊断'),
              ),
            ],
          ),
        ),
      ],
    );
  }
}

class _NetworkRow extends StatelessWidget {
  const _NetworkRow({required this.icon, required this.title, required this.value});

  final IconData icon;
  final String title;
  final String value;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 9),
      child: Row(
        children: [
          Icon(icon, size: 18, color: P2PColors.mint),
          const SizedBox(width: 12),
          Expanded(child: Text(title, style: const TextStyle(fontWeight: FontWeight.w600))),
          Flexible(child: Text(value, textAlign: TextAlign.end,
            style: const TextStyle(color: P2PColors.muted, fontSize: 12))),
        ],
      ),
    );
  }
}
