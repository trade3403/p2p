import 'package:material_ui/material_ui.dart';

import '../app_theme.dart';
import '../services/remote_controller.dart';
import '../widgets/p2p_widgets.dart';

class SecurityScreen extends StatelessWidget {
  const SecurityScreen({super.key});

  @override
  Widget build(BuildContext context) {
    final controller = P2PRemoteController.instance;
    return AnimatedBuilder(
      animation: controller,
      builder: (context, _) {
        return ListView(
          children: [
            const PageHeading(
              eyebrow: 'SECURITY & PERMISSIONS',
              title: '安全与权限',
              subtitle: '远程操作权限由被控电脑本机用户明确授权。',
              trailing: StatusDot(color: P2PColors.mint, label: '本机控制'),
            ),
            const SizedBox(height: 24),
            PanelCard(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  const Text('远程操作权限', style: TextStyle(fontSize: 18, fontWeight: FontWeight.w800)),
                  const SizedBox(height: 5),
                  Text(
                    controller.hostRunning
                        ? '当前共享会话的权限状态。关闭权限会立即阻止后续远程输入或剪贴板操作。'
                        : '先启动本机共享，再按需授权远程输入和剪贴板。',
                    style: const TextStyle(color: P2PColors.muted, fontSize: 12),
                  ),
                  const SizedBox(height: 12),
                  SwitchListTile.adaptive(
                    contentPadding: EdgeInsets.zero,
                    value: controller.allowInput,
                    onChanged: controller.bridgeAvailable && controller.hostRunning
                        ? (value) => controller.setRemoteInputAllowed(value)
                        : null,
                    title: const Text('允许远程鼠标和键盘'),
                    subtitle: const Text('允许已批准的控制端点击、拖动和输入文字。'),
                  ),
                  const Divider(color: P2PColors.outline),
                  SwitchListTile.adaptive(
                    contentPadding: EdgeInsets.zero,
                    value: controller.allowClipboard,
                    onChanged: controller.bridgeAvailable && controller.hostRunning
                        ? (value) => controller.setClipboardAllowed(value)
                        : null,
                    title: const Text('允许按需剪贴板访问'),
                    subtitle: const Text('仅在控制端主动读取或发送时传输文字。'),
                  ),
                  if (!controller.hostRunning)
                    const Padding(
                      padding: EdgeInsets.only(top: 8),
                      child: Text('本机共享未启动，权限开关暂不可用。',
                        style: TextStyle(color: P2PColors.amber, fontSize: 11)),
                    ),
                ],
              ),
            ),
            const SizedBox(height: 15),
            const PanelCard(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Row(
                    children: [
                      Icon(Icons.verified_user_outlined, color: P2PColors.mint),
                      SizedBox(width: 10),
                      Text('安全边界', style: TextStyle(fontSize: 17, fontWeight: FontWeight.w800)),
                    ],
                  ),
                  SizedBox(height: 12),
                  _SecurityPoint(text: '首次数字配对必须由被控电脑用户批准。'),
                  _SecurityPoint(text: '设备号用于识别；6位动态验证码有效期为120秒。'),
                  _SecurityPoint(text: 'Windows 长期凭据由当前用户 DPAPI 加密保存；Android Relay 口令由 Android Keystore 加密保存。'),
                  _SecurityPoint(text: '剪贴板不会自动监听或后台同步。'),
                  _SecurityPoint(text: 'UAC、安全桌面和锁屏界面不在当前支持范围内。'),
                ],
              ),
            ),
          ],
        );
      },
    );
  }
}

class _SecurityPoint extends StatelessWidget {
  const _SecurityPoint({required this.text});
  final String text;

  @override
  Widget build(BuildContext context) => Padding(
        padding: const EdgeInsets.only(bottom: 9),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            const Icon(Icons.check_circle_outline_rounded, color: P2PColors.mint, size: 17),
            const SizedBox(width: 9),
            Expanded(child: Text(text, style: const TextStyle(color: P2PColors.muted, fontSize: 12))),
          ],
        ),
      );
}
