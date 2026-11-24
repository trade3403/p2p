import 'package:material_ui/material_ui.dart';

import '../app_page.dart';
import '../app_theme.dart';
import '../services/remote_controller.dart';
import '../widgets/p2p_widgets.dart';

class DevicesScreen extends StatelessWidget {
  const DevicesScreen({super.key, required this.onNavigate});

  final ValueChanged<AppPage> onNavigate;

  @override
  Widget build(BuildContext context) {
    final controller = P2PRemoteController.instance;
    return AnimatedBuilder(
      animation: controller,
      builder: (context, _) {
        return ListView(
          children: [
            const PageHeading(
              eyebrow: 'DEVICE LIBRARY',
              title: '我的设备',
              subtitle: '通过数字配对连接设备；多设备本地加密库正在迁移。',
              trailing: StatusDot(color: P2PColors.amber, label: '迁移中'),
            ),
            const SizedBox(height: 24),
            PanelCard(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  const Text('当前远程会话', style: TextStyle(fontSize: 18, fontWeight: FontWeight.w800)),
                  const SizedBox(height: 9),
                  Row(
                    children: [
                      Icon(
                        controller.viewerConnected ? Icons.desktop_windows_rounded : Icons.devices_other_rounded,
                        color: P2PColors.mint,
                        size: 24,
                      ),
                      const SizedBox(width: 12),
                      Expanded(
                        child: Text(
                          controller.viewerConnected || controller.viewerRequested || controller.pairingActive ? controller.viewerStatus : '当前没有活动的远程会话',
                          style: const TextStyle(color: P2PColors.muted, fontSize: 12),
                        ),
                      ),
                      if (controller.viewerConnected || controller.viewerRequested || controller.pairingActive)
                        OutlinedButton(
                          onPressed: () => controller.disconnectViewer(),
                          child: const Text('断开'),
                        ),
                    ],
                  ),
                  const SizedBox(height: 15),
                  FilledButton.icon(
                    onPressed: () => onNavigate(AppPage.remote),
                    icon: const Icon(Icons.add_link_rounded),
                    label: const Text('通过设备号配对'),
                  ),
                ],
              ),
            ),
            const SizedBox(height: 15),
            const PanelCard(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text('设备库迁移说明', style: TextStyle(fontSize: 17, fontWeight: FontWeight.w800)),
                  SizedBox(height: 10),
                  Text(
                    '旧版 Windows 和 Android 客户端各自保存单设备凭据。目前保持现有 DPAPI / Android Keystore 存储不变，再迁移为带稳定本地 ID 的多设备加密集合。完成迁移前，不展示演示设备，也不会自动覆盖旧凭据。',
                    style: TextStyle(color: P2PColors.muted, fontSize: 12, height: 1.55),
                  ),
                ],
              ),
            ),
          ],
        );
      },
    );
  }
}
