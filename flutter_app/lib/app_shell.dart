import 'package:material_ui/material_ui.dart';

import 'app_theme.dart';
import 'app_page.dart';
import 'screens/dashboard_screen.dart';
import 'screens/devices_screen.dart';
import 'screens/network_screen.dart';
import 'screens/remote_screen.dart';
import 'screens/security_screen.dart';
import 'screens/settings_screen.dart';
import 'widgets/p2p_widgets.dart';

class AppShell extends StatefulWidget {
  const AppShell({super.key});

  @override
  State<AppShell> createState() => _AppShellState();
}

class _AppShellState extends State<AppShell> {
  AppPage _page = AppPage.home;

  static const _navigation = <(AppPage, String, IconData)>[
    (AppPage.home, '首页', Icons.space_dashboard_rounded),
    (AppPage.remote, '远程控制', Icons.desktop_windows_rounded),
    (AppPage.devices, '设备管理', Icons.devices_rounded),
    (AppPage.security, '安全与权限', Icons.shield_outlined),
    (AppPage.network, '网络连接', Icons.hub_rounded),
    (AppPage.settings, '设置', Icons.settings_rounded),
  ];

  void _navigate(AppPage page) => setState(() => _page = page);

  Widget _buildPage() {
    return switch (_page) {
      AppPage.home => DashboardScreen(onNavigate: _navigate),
      AppPage.remote => const RemoteScreen(),
      AppPage.devices => DevicesScreen(onNavigate: _navigate),
      AppPage.security => const SecurityScreen(),
      AppPage.network => const NetworkScreen(),
      AppPage.settings => SettingsScreen(onNavigate: _navigate),
    };
  }

  @override
  Widget build(BuildContext context) {
    return LayoutBuilder(
      builder: (context, constraints) {
        final desktop = constraints.maxWidth >= 980;
        final content = Center(
          child: ConstrainedBox(
            constraints: const BoxConstraints(maxWidth: 1480),
            child: Padding(
              padding: EdgeInsets.fromLTRB(
                desktop ? 30 : 18,
                desktop ? 28 : 18,
                desktop ? 30 : 18,
                24,
              ),
              child: _buildPage(),
            ),
          ),
        );

        if (desktop) {
          return Scaffold(
            body: Row(
              children: [
                SizedBox(
                  width: 238,
                  child: _DesktopSidebar(
                    items: _navigation,
                    selected: _page,
                    onSelected: _navigate,
                  ),
                ),
                Expanded(child: content),
              ],
            ),
          );
        }

        const mobilePages = [
          AppPage.home,
          AppPage.devices,
          AppPage.remote,
          AppPage.settings,
        ];
        const mobileLabels = ['首页', '设备', '远控', '我的'];
        const mobileIcons = [
          Icons.home_rounded,
          Icons.devices_rounded,
          Icons.desktop_windows_rounded,
          Icons.person_rounded,
        ];
        final selectedMobileIndex = mobilePages.indexOf(_page);
        return Scaffold(
          body: SafeArea(child: content),
          bottomNavigationBar: NavigationBar(
            selectedIndex: selectedMobileIndex < 0 ? 3 : selectedMobileIndex,
            onDestinationSelected: (index) => _navigate(mobilePages[index]),
            destinations: List.generate(
              mobilePages.length,
              (index) => NavigationDestination(
                icon: Icon(mobileIcons[index]),
                label: mobileLabels[index],
              ),
            ),
          ),
        );
      },
    );
  }
}

class _DesktopSidebar extends StatelessWidget {
  const _DesktopSidebar({
    required this.items,
    required this.selected,
    required this.onSelected,
  });

  final List<(AppPage, String, IconData)> items;
  final AppPage selected;
  final ValueChanged<AppPage> onSelected;

  @override
  Widget build(BuildContext context) {
    return Container(
      decoration: const BoxDecoration(
        color: P2PColors.sidebar,
        border: Border(right: BorderSide(color: P2PColors.outline, width: 0.7)),
      ),
      child: SafeArea(
        child: Padding(
          padding: const EdgeInsets.fromLTRB(19, 22, 15, 18),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Row(
                children: [
                  Container(
                    width: 42,
                    height: 42,
                    decoration: BoxDecoration(
                      borderRadius: BorderRadius.circular(13),
                      gradient: const LinearGradient(
                        colors: [P2PColors.mint, Color(0xFF3E91C7)],
                        begin: Alignment.topLeft,
                        end: Alignment.bottomRight,
                      ),
                    ),
                    child: const Icon(Icons.bolt_rounded, color: Color(0xFF08211F), size: 25),
                  ),
                  const SizedBox(width: 11),
                  const Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text('P2P Remote', style: TextStyle(fontSize: 17, fontWeight: FontWeight.w900)),
                      SizedBox(height: 2),
                      Text('REMOTE DESKTOP', style: TextStyle(fontSize: 9, color: P2PColors.muted, letterSpacing: 1.2)),
                    ],
                  ),
                ],
              ),
              const SizedBox(height: 38),
              const Text('工作空间', style: TextStyle(color: P2PColors.muted, fontSize: 11, fontWeight: FontWeight.w700)),
              const SizedBox(height: 13),
              for (final item in items) ...[
                _SidebarItem(
                  icon: item.$3,
                  label: item.$2,
                  selected: selected == item.$1,
                  onTap: () => onSelected(item.$1),
                ),
                const SizedBox(height: 6),
              ],
              const Spacer(),
              const Divider(color: P2PColors.outline),
              const SizedBox(height: 12),
              const StatusDot(color: P2PColors.amber, label: '原型模式 · 尚未连接服务'),
              const SizedBox(height: 9),
              const Text('V0.17 Flutter UI Preview', style: TextStyle(color: P2PColors.muted, fontSize: 10)),
            ],
          ),
        ),
      ),
    );
  }
}

class _SidebarItem extends StatelessWidget {
  const _SidebarItem({
    required this.icon,
    required this.label,
    required this.selected,
    required this.onTap,
  });

  final IconData icon;
  final String label;
  final bool selected;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    return AnimatedContainer(
      duration: const Duration(milliseconds: 160),
      decoration: BoxDecoration(
        color: selected ? const Color(0xFF193B49) : Colors.transparent,
        borderRadius: BorderRadius.circular(12),
        border: Border.all(
          color: selected ? const Color(0xFF2C6E6A) : Colors.transparent,
        ),
      ),
      child: ListTile(
        dense: true,
        visualDensity: const VisualDensity(vertical: -1),
        contentPadding: const EdgeInsets.symmetric(horizontal: 12),
        leading: Icon(icon, size: 19, color: selected ? P2PColors.mint : P2PColors.muted),
        title: Text(
          label,
          style: TextStyle(
            color: selected ? P2PColors.text : P2PColors.muted,
            fontSize: 13,
            fontWeight: selected ? FontWeight.w800 : FontWeight.w500,
          ),
        ),
        trailing: selected
            ? const Icon(Icons.circle, size: 6, color: P2PColors.mint)
            : null,
        onTap: onTap,
      ),
    );
  }
}
