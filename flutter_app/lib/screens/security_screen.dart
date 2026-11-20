import 'package:material_ui/material_ui.dart';

import '../app_theme.dart';
import '../widgets/p2p_widgets.dart';

class SecurityScreen extends StatefulWidget {
  const SecurityScreen({super.key});

  @override
  State<SecurityScreen> createState() => _SecurityScreenState();
}

class _SecurityScreenState extends State<SecurityScreen> {
  bool _remoteInput = true;
  bool _clipboard = false;
  bool _rememberDevice = false;
  bool _startOnLogin = false;

  @override
  Widget build(BuildContext context) {
    return ListView(
      children: [
        const PageHeading(
          eyebrow: 'SECURITY & PERMISSIONS',
          title: '安全与权限',
          subtitle: '明确控制远程输入、剪贴板和设备信任范围。',
          trailing: PrototypePill(),
        ),
        const SizedBox(height: 24),
        PanelCard(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              const Text('远程操作权限', style: TextStyle(fontSize: 18, fontWeight: FontWeight.w800)),
              const SizedBox(height: 5),
              const Text('每项能力独立授权。原型开关仅用于展示交互，不会修改 Windows 权限。',
                style: TextStyle(color: P2PColors.muted, fontSize: 12)),
              const SizedBox(height: 14),
              SwitchListTile.adaptive(
                contentPadding: EdgeInsets.zero,
                value: _remoteInput,
                onChanged: (value) => setState(() => _remoteInput = value),
                title: const Text('允许远程鼠标和键盘'),
                subtitle: const Text('允许已配对设备点击、拖动和输入文字。'),
              ),
              const Divider(color: P2PColors.outline),
              SwitchListTile.adaptive(
                contentPadding: EdgeInsets.zero,
                value: _clipboard,
                onChanged: (value) => setState(() => _clipboard = value),
                title: const Text('允许按需剪贴板访问'),
                subtitle: const Text('仅在用户主动点击读取或发送时传输文本。'),
              ),
            ],
          ),
        ),
        const SizedBox(height: 15),
        PanelCard(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              const Text('设备信任与启动', style: TextStyle(fontSize: 18, fontWeight: FontWeight.w800)),
              const SizedBox(height: 12),
              SwitchListTile.adaptive(
                contentPadding: EdgeInsets.zero,
                value: _rememberDevice,
                onChanged: (value) => setState(() => _rememberDevice = value),
                title: const Text('信任此设备'),
                subtitle: const Text('需由用户明确确认后，才会保存长期配对凭据。'),
              ),
              const Divider(color: P2PColors.outline),
              SwitchListTile.adaptive(
                contentPadding: EdgeInsets.zero,
                value: _startOnLogin,
                onChanged: (value) => setState(() => _startOnLogin = value),
                title: const Text('登录后启动'),
                subtitle: const Text('启动行为和无人值守输入权限应分别设置。'),
              ),
            ],
          ),
        ),
        const SizedBox(height: 15),
        PanelCard(
          child: const Column(
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
              _SecurityPoint(text: '首次配对必须由被控电脑用户批准。'),
              _SecurityPoint(text: '设备号只用于识别，动态验证码有效期为 120 秒。'),
              _SecurityPoint(text: '剪贴板不会自动监听或后台同步。'),
              _SecurityPoint(text: 'UAC、安全桌面和锁屏界面不在当前支持范围内。'),
            ],
          ),
        ),
      ],
    );
  }
}

class _SecurityPoint extends StatelessWidget {
  const _SecurityPoint({required this.text});
  final String text;

  @override
  Widget build(BuildContext context) {
    return Padding(
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
}
