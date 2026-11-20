import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../app_shell.dart';
import '../app_theme.dart';
import '../widgets/p2p_widgets.dart';

class DashboardScreen extends StatelessWidget {
  const DashboardScreen({super.key, required this.onNavigate});

  final ValueChanged<AppPage> onNavigate;

  @override
  Widget build(BuildContext context) {
    return ListView(
      children: [
        const PageHeading(
          eyebrow: 'P2P REMOTE  /  V0.17',
          title: '设备概览',
          subtitle: '安全连接你的设备，随时随地访问远程桌面。',
          trailing: PrototypePill(),
        ),
        const SizedBox(height: 26),
        const _LocalDeviceCard(),
        const SizedBox(height: 18),
        LayoutBuilder(
          builder: (context, constraints) {
            final columns = constraints.maxWidth > 1050 ? 3 : constraints.maxWidth > 650 ? 2 : 1;
            final width = (constraints.maxWidth - (columns - 1) * 14) / columns;
            return Wrap(
              spacing: 14,
              runSpacing: 14,
              children: [
                SizedBox(width: width, child: _QuickConnectCard(onPreview: () => onNavigate(AppPage.remote))),
                SizedBox(width: width, child: const _OverviewCard(
                  icon: Icons.shield_rounded,
                  title: '安全状态',
                  value: '权限受控',
                  detail: '输入与剪贴板需单独授权',
                  accent: P2PColors.mint,
                )),
                SizedBox(width: width, child: const _OverviewCard(
                  icon: Icons.hub_rounded,
                  title: '连接方式',
                  value: 'P2P 优先',
                  detail: '直连不可用时使用加密中转',
                  accent: P2PColors.blue,
                )),
              ],
            );
          },
        ),
        const SizedBox(height: 28),
        SectionTitle(
          title: '最近设备',
          actionLabel: '查看全部',
          onAction: () => onNavigate(AppPage.devices),
        ),
        const SizedBox(height: 13),
        LayoutBuilder(
          builder: (context, constraints) {
            final columns = constraints.maxWidth > 1050 ? 3 : constraints.maxWidth > 620 ? 2 : 1;
            final width = (constraints.maxWidth - (columns - 1) * 14) / columns;
            return Wrap(
              spacing: 14,
              runSpacing: 14,
              children: [
                SizedBox(
                  width: width,
                  child: DeviceTile(
                    name: '办公室电脑',
                    detail: 'Windows 11 · 上次连接 今天 09:18',
                    online: true,
                    icon: Icons.desktop_windows_rounded,
                    onConnect: () => onNavigate(AppPage.remote),
                  ),
                ),
                SizedBox(
                  width: width,
                  child: DeviceTile(
                    name: '家用电脑',
                    detail: 'Windows 11 · 上次连接 昨天 21:42',
                    online: true,
                    icon: Icons.computer_rounded,
                    onConnect: () => onNavigate(AppPage.remote),
                  ),
                ),
                SizedBox(
                  width: width,
                  child: DeviceTile(
                    name: '设计笔记本',
                    detail: 'Windows 10 · 上次在线 2 小时前',
                    online: false,
                    icon: Icons.laptop_windows_rounded,
                    onConnect: () => _showDemoMessage(context),
                  ),
                ),
              ],
            );
          },
        ),
        const SizedBox(height: 26),
        const Text(
          '以上设备号、状态和时间均为 UI 演示数据，不代表真实 Relay 状态。',
          style: TextStyle(color: P2PColors.muted, fontSize: 11),
        ),
      ],
    );
  }

  static void _showDemoMessage(BuildContext context) {
    ScaffoldMessenger.of(context).showSnackBar(
      const SnackBar(content: Text('当前是界面原型，尚未连接真实设备服务。')),
    );
  }
}

class _LocalDeviceCard extends StatelessWidget {
  const _LocalDeviceCard();

  @override
  Widget build(BuildContext context) {
    return PanelCard(
      padding: const EdgeInsets.all(26),
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
                    Text('本机设备', style: TextStyle(color: P2PColors.muted, fontSize: 13)),
                    SizedBox(height: 5),
                    Text('让其他设备安全连接到这台电脑', style: TextStyle(color: P2PColors.text, fontSize: 17, fontWeight: FontWeight.w800)),
                  ],
                ),
              ),
              Container(
                padding: const EdgeInsets.symmetric(horizontal: 11, vertical: 8),
                decoration: BoxDecoration(
                  color: const Color(0xFF183E39),
                  border: Border.all(color: const Color(0xFF2A665C)),
                  borderRadius: BorderRadius.circular(30),
                ),
                child: const StatusDot(color: P2PColors.mint, label: '原型状态'),
              ),
            ],
          ),
          const SizedBox(height: 25),
          LayoutBuilder(
            builder: (context, constraints) {
              if (constraints.maxWidth < 640) {
                return const Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    _NumberBlock(
                      label: '9 位设备号',
                      value: '419 049 723',
                      helper: '设备号用于识别设备，不是访问凭据。',
                    ),
                    SizedBox(height: 22),
                    _NumberBlock(
                      label: '6 位动态验证码',
                      value: '582 614',
                      helper: '演示有效期 02:00 · 成功配对后失效。',
                      amber: true,
                    ),
                  ],
                );
              }
              return const Row(
                children: [
                  Expanded(
                    child: _NumberBlock(
                      label: '9 位设备号',
                      value: '419 049 723',
                      helper: '设备号用于识别设备，不是访问凭据。',
                    ),
                  ),
                  SizedBox(width: 26),
                  SizedBox(height: 82, child: VerticalDivider(color: P2PColors.outline, width: 1)),
                  SizedBox(width: 26),
                  Expanded(
                    child: _NumberBlock(
                      label: '6 位动态验证码',
                      value: '582 614',
                      helper: '演示有效期 02:00 · 成功配对后失效。',
                      amber: true,
                    ),
                  ),
                ],
              );
            },
          ),
          const SizedBox(height: 23),
          const Wrap(
            spacing: 10,
            runSpacing: 10,
            children: [
              _CopyButton(value: '419049723', label: '复制设备号'),
              _CopyButton(value: '582614', label: '复制验证码'),
            ],
          ),
        ],
      ),
    );
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
  Widget build(BuildContext context) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Text(label, style: const TextStyle(color: P2PColors.muted, fontSize: 12)),
        const SizedBox(height: 8),
        SelectableText(
          value,
          style: TextStyle(
            color: amber ? P2PColors.amber : P2PColors.mint,
            fontSize: 31,
            fontWeight: FontWeight.w900,
            letterSpacing: 2.3,
            fontFeatures: const [FontFeature.tabularFigures()],
          ),
        ),
        const SizedBox(height: 5),
        Text(helper, style: const TextStyle(color: P2PColors.muted, fontSize: 11)),
      ],
    );
  }
}

class _CopyButton extends StatelessWidget {
  const _CopyButton({required this.value, required this.label});

  final String value;
  final String label;

  @override
  Widget build(BuildContext context) {
    return OutlinedButton.icon(
      onPressed: () async {
        await Clipboard.setData(ClipboardData(text: value));
        if (context.mounted) {
          ScaffoldMessenger.of(context).showSnackBar(
            SnackBar(content: Text('$label（演示数据）已复制')),
          );
        }
      },
      icon: const Icon(Icons.copy_rounded, size: 16),
      label: Text(label),
    );
  }
}

class _QuickConnectCard extends StatefulWidget {
  const _QuickConnectCard({required this.onPreview});

  final VoidCallback onPreview;

  @override
  State<_QuickConnectCard> createState() => _QuickConnectCardState();
}

class _QuickConnectCardState extends State<_QuickConnectCard> {
  final _device = TextEditingController();
  final _code = TextEditingController();

  @override
  void dispose() {
    _device.dispose();
    _code.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return PanelCard(
      padding: const EdgeInsets.all(19),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const Row(
            children: [
              Icon(Icons.link_rounded, color: P2PColors.mint),
              SizedBox(width: 9),
              Text('快速连接', style: TextStyle(fontWeight: FontWeight.w800, fontSize: 16)),
            ],
          ),
          const SizedBox(height: 14),
          TextField(
            controller: _device,
            keyboardType: TextInputType.number,
            maxLength: 9,
            decoration: const InputDecoration(
              labelText: '9 位设备号',
              hintText: '输入对方设备号',
              counterText: '',
            ),
          ),
          const SizedBox(height: 9),
          TextField(
            controller: _code,
            keyboardType: TextInputType.number,
            maxLength: 6,
            obscureText: true,
            decoration: const InputDecoration(
              labelText: '6 位动态验证码',
              hintText: '输入一次性验证码',
              counterText: '',
            ),
          ),
          const SizedBox(height: 12),
          SizedBox(
            width: double.infinity,
            child: FilledButton.icon(
              onPressed: () {
                ScaffoldMessenger.of(context).showSnackBar(
                  const SnackBar(content: Text('UI 原型：真实配对接口将在后续阶段接入。')),
                );
                widget.onPreview();
              },
              icon: const Icon(Icons.arrow_forward_rounded, size: 18),
              label: const Text('连接预览'),
            ),
          ),
        ],
      ),
    );
  }
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
  Widget build(BuildContext context) {
    return PanelCard(
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
}
