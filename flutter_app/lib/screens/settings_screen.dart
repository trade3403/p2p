import 'package:flutter/material.dart';

import '../app_theme.dart';
import '../widgets/p2p_widgets.dart';

class SettingsScreen extends StatelessWidget {
  const SettingsScreen({super.key});

  @override
  Widget build(BuildContext context) {
    return ListView(
      children: [
        const PageHeading(
          eyebrow: 'PREFERENCES',
          title: '设置',
          subtitle: '管理应用外观、启动行为和本机信息。',
          trailing: PrototypePill(),
        ),
        const SizedBox(height: 24),
        PanelCard(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              const Text('应用体验', style: TextStyle(fontSize: 18, fontWeight: FontWeight.w800)),
              const SizedBox(height: 13),
              const _SettingRow(
                icon: Icons.dark_mode_rounded,
                title: '外观',
                subtitle: '深色主题 · P2P Remote 专属配色',
                value: '深色',
              ),
              const Divider(color: P2PColors.outline),
              const _SettingRow(
                icon: Icons.language_rounded,
                title: '语言',
                subtitle: '应用界面语言',
                value: '简体中文',
              ),
              const Divider(color: P2PColors.outline),
              const _SettingRow(
                icon: Icons.notifications_active_outlined,
                title: '连接通知',
                subtitle: '连接请求、断开和安全提醒',
                value: '默认',
              ),
            ],
          ),
        ),
        const SizedBox(height: 15),
        PanelCard(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              const Text('关于 P2P Remote', style: TextStyle(fontSize: 18, fontWeight: FontWeight.w800)),
              const SizedBox(height: 14),
              Row(
                children: [
                  Container(
                    width: 48,
                    height: 48,
                    decoration: BoxDecoration(
                      borderRadius: BorderRadius.circular(14),
                      gradient: const LinearGradient(
                        colors: [P2PColors.mint, Color(0xFF3E91C7)],
                      ),
                    ),
                    child: const Icon(Icons.bolt_rounded, color: Color(0xFF08211F), size: 27),
                  ),
                  const SizedBox(width: 13),
                  const Expanded(
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Text('P2P Remote', style: TextStyle(fontWeight: FontWeight.w900, fontSize: 16)),
                        SizedBox(height: 3),
                        Text('V0.17 Flutter UI Prototype', style: TextStyle(color: P2PColors.muted, fontSize: 11)),
                      ],
                    ),
                  ),
                  const Icon(Icons.science_outlined, color: P2PColors.amber),
                ],
              ),
              const SizedBox(height: 16),
              const Text(
                '此版本仅用于确认视觉设计和交互结构。真实设备、配对、视频流、权限和系统托盘仍由现有原生核心提供，完成桥接前不会在此界面伪装为已连接。',
                style: TextStyle(color: P2PColors.muted, fontSize: 12, height: 1.55),
              ),
            ],
          ),
        ),
      ],
    );
  }
}

class _SettingRow extends StatelessWidget {
  const _SettingRow({
    required this.icon,
    required this.title,
    required this.subtitle,
    required this.value,
  });

  final IconData icon;
  final String title;
  final String subtitle;
  final String value;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 12),
      child: Row(
        children: [
          Icon(icon, color: P2PColors.mint, size: 20),
          const SizedBox(width: 13),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(title, style: const TextStyle(fontWeight: FontWeight.w700)),
                const SizedBox(height: 3),
                Text(subtitle, style: const TextStyle(color: P2PColors.muted, fontSize: 11)),
              ],
            ),
          ),
          Text(value, style: const TextStyle(color: P2PColors.muted, fontSize: 12)),
        ],
      ),
    );
  }
}
