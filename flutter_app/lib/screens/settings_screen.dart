import 'package:material_ui/material_ui.dart';

import '../app_theme.dart';
import '../app_page.dart';
import '../widgets/p2p_widgets.dart';

class SettingsScreen extends StatelessWidget {
  const SettingsScreen({super.key, required this.onNavigate});

  final ValueChanged<AppPage> onNavigate;

  @override
  Widget build(BuildContext context) {
    return ListView(
      children: [
        const PageHeading(
          eyebrow: 'PREFERENCES',
          title: '设置',
          subtitle: '查看当前应用配置与本机信息。',
          trailing: StatusDot(color: P2PColors.mint, label: 'V0.20 原生桥接'),
        ),
        const SizedBox(height: 24),
        const PanelCard(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text('应用体验', style: TextStyle(fontSize: 18, fontWeight: FontWeight.w800)),
              SizedBox(height: 13),
              _SettingRow(
                icon: Icons.dark_mode_rounded,
                title: '外观',
                subtitle: '当前固定深色主题',
                value: '深色',
              ),
              Divider(color: P2PColors.outline),
              _SettingRow(
                icon: Icons.language_rounded,
                title: '语言',
                subtitle: '当前固定界面语言',
                value: '简体中文',
              ),
              Divider(color: P2PColors.outline),
              _SettingRow(
                icon: Icons.notifications_active_outlined,
                title: '连接通知',
                subtitle: '通知设置暂未接入',
                value: '未提供设置',
              ),
            ],
          ),
        ),
        const SizedBox(height: 15),
        PanelCard(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              const Text('更多设置', style: TextStyle(fontSize: 18, fontWeight: FontWeight.w800)),
              const SizedBox(height: 10),
              _NavigationSetting(
                icon: Icons.shield_outlined,
                title: '安全与权限',
                subtitle: '远程输入、剪贴板和设备信任',
                onTap: () => onNavigate(AppPage.security),
              ),
              const Divider(color: P2PColors.outline),
              Semantics(
                identifier: 'p2p_settings_network',
                child: _NavigationSetting(
                  icon: Icons.hub_rounded,
                  title: '网络连接',
                  subtitle: 'Relay、局域网直连和诊断',
                  onTap: () => onNavigate(AppPage.network),
                ),
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
                        Text('V0.20 Android Native Bridge', style: TextStyle(color: P2PColors.muted, fontSize: 11)),
                      ],
                    ),
                  ),
                  const Icon(Icons.science_outlined, color: P2PColors.amber),
                ],
              ),
              const SizedBox(height: 16),
              const Text(
                'Windows 端使用 Rust Host/Viewer；Android Flutter 端通过 Kotlin MethodChannel/EventChannel 接入数字配对、AES-GCM、JPEG/H.264 画面和远程输入。Windows 托盘及安装程序已构建；公网远控仍需实测验收。',
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

class _NavigationSetting extends StatelessWidget {
  const _NavigationSetting({
    required this.icon,
    required this.title,
    required this.subtitle,
    required this.onTap,
  });

  final IconData icon;
  final String title;
  final String subtitle;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    return ListTile(
      contentPadding: EdgeInsets.zero,
      leading: Icon(icon, color: P2PColors.mint),
      title: Text(title, style: const TextStyle(fontWeight: FontWeight.w700)),
      subtitle: Text(subtitle, style: const TextStyle(color: P2PColors.muted, fontSize: 11)),
      trailing: const Icon(Icons.chevron_right_rounded, color: P2PColors.muted),
      onTap: onTap,
    );
  }
}
