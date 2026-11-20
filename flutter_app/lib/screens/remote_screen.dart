import 'package:flutter/material.dart';

import '../app_theme.dart';
import '../widgets/p2p_widgets.dart';

class RemoteScreen extends StatelessWidget {
  const RemoteScreen({super.key});

  @override
  Widget build(BuildContext context) {
    return ListView(
      children: [
        PageHeading(
          eyebrow: 'REMOTE SESSION',
          title: '远程控制',
          subtitle: '连接电脑后，在这里查看桌面并安全操作。',
          trailing: const PrototypePill(),
        ),
        const SizedBox(height: 22),
        PanelCard(
          padding: const EdgeInsets.fromLTRB(18, 15, 18, 16),
          child: LayoutBuilder(
            builder: (context, constraints) {
              final compact = constraints.maxWidth < 700;
              final device = const Row(
                mainAxisSize: MainAxisSize.min,
                children: [
                  Icon(Icons.desktop_windows_rounded, color: P2PColors.mint, size: 20),
                  SizedBox(width: 9),
                  Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text('办公室电脑', style: TextStyle(fontWeight: FontWeight.w800)),
                      SizedBox(height: 2),
                      Text('演示会话 · 尚未建立连接', style: TextStyle(color: P2PColors.muted, fontSize: 11)),
                    ],
                  ),
                ],
              );
              final actions = Wrap(
                spacing: 7,
                runSpacing: 7,
                children: [
                  _ToolbarButton(icon: Icons.fullscreen_rounded, label: '全屏'),
                  _ToolbarButton(icon: Icons.keyboard_rounded, label: '键盘'),
                  _ToolbarButton(icon: Icons.mouse_rounded, label: '鼠标模式'),
                  _ToolbarButton(icon: Icons.monitor_rounded, label: '显示器'),
                  _ToolbarButton(icon: Icons.high_quality_rounded, label: '画质'),
                  _ToolbarButton(icon: Icons.content_paste_rounded, label: '剪贴板'),
                  _ToolbarButton(icon: Icons.folder_open_rounded, label: '文件'),
                ],
              );
              if (compact) {
                return Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [device, const SizedBox(height: 15), actions],
                );
              }
              return Row(
                children: [
                  device,
                  const Spacer(),
                  Flexible(child: actions),
                ],
              );
            },
          ),
        ),
        const SizedBox(height: 14),
        AspectRatio(
          aspectRatio: 16 / 9,
          child: Container(
            decoration: BoxDecoration(
              color: const Color(0xFF070D16),
              borderRadius: BorderRadius.circular(18),
              border: Border.all(color: P2PColors.outline),
              gradient: const LinearGradient(
                colors: [Color(0xFF101D2D), Color(0xFF080F19)],
                begin: Alignment.topLeft,
                end: Alignment.bottomRight,
              ),
            ),
            child: Stack(
              children: [
                Positioned(
                  top: 16,
                  left: 18,
                  child: Row(
                    children: [
                      Container(
                        width: 9,
                        height: 9,
                        decoration: const BoxDecoration(color: P2PColors.amber, shape: BoxShape.circle),
                      ),
                      const SizedBox(width: 8),
                      const Text('等待连接', style: TextStyle(color: P2PColors.muted, fontSize: 12)),
                    ],
                  ),
                ),
                const Center(
                  child: Column(
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      Icon(Icons.desktop_access_disabled_rounded, size: 54, color: Color(0xFF55708D)),
                      SizedBox(height: 16),
                      Text('远程画面将在这里显示', style: TextStyle(fontSize: 18, fontWeight: FontWeight.w800)),
                      SizedBox(height: 7),
                      Text('Flutter UI 原型 · 视频流和远控事件尚未接入',
                        style: TextStyle(color: P2PColors.muted, fontSize: 12)),
                    ],
                  ),
                ),
                Positioned(
                  right: 16,
                  bottom: 14,
                  child: Container(
                    padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 7),
                    decoration: BoxDecoration(
                      color: const Color(0xCC101D2D),
                      borderRadius: BorderRadius.circular(9),
                      border: Border.all(color: P2PColors.outline),
                    ),
                    child: const Text('适配比例 16:9  ·  100%',
                      style: TextStyle(color: P2PColors.muted, fontSize: 10)),
                  ),
                ),
              ],
            ),
          ),
        ),
        const SizedBox(height: 15),
        LayoutBuilder(
          builder: (context, constraints) {
            final columns = constraints.maxWidth > 900 ? 2 : 1;
            final width = (constraints.maxWidth - (columns - 1) * 14) / columns;
            return Wrap(
              spacing: 14,
              runSpacing: 14,
              children: [
                SizedBox(
                  width: width,
                  child: const PanelCard(
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Text('控制提示', style: TextStyle(fontWeight: FontWeight.w800, fontSize: 16)),
                        SizedBox(height: 10),
                        _HintRow(icon: Icons.ads_click_rounded, text: '单击画面：鼠标左键'),
                        _HintRow(icon: Icons.open_with_rounded, text: '按住拖动：移动窗口或选择内容'),
                        _HintRow(icon: Icons.pinch_rounded, text: '双指缩放：调整远程画面比例'),
                        _HintRow(icon: Icons.keyboard_rounded, text: '虚拟键盘：向远程电脑发送按键'),
                      ],
                    ),
                  ),
                ),
                SizedBox(
                  width: width,
                  child: PanelCard(
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        const Text('连接状态', style: TextStyle(fontWeight: FontWeight.w800, fontSize: 16)),
                        const SizedBox(height: 13),
                        const _HintRow(icon: Icons.lock_outline_rounded, text: '端到端加密：等待会话建立'),
                        const _HintRow(icon: Icons.speed_rounded, text: '延迟：暂无数据'),
                        const _HintRow(icon: Icons.tune_rounded, text: '远程输入：等待被控端授权'),
                        const SizedBox(height: 12),
                        OutlinedButton.icon(
                          onPressed: () => ScaffoldMessenger.of(context).showSnackBar(
                            const SnackBar(content: Text('原型模式：断开/连接控制尚未接入。')),
                          ),
                          icon: const Icon(Icons.link_off_rounded),
                          label: const Text('断开会话'),
                        ),
                      ],
                    ),
                  ),
                ),
              ],
            );
          },
        ),
      ],
    );
  }
}

class _ToolbarButton extends StatelessWidget {
  const _ToolbarButton({required this.icon, required this.label});

  final IconData icon;
  final String label;

  @override
  Widget build(BuildContext context) {
    return OutlinedButton.icon(
      onPressed: () => ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text('$label：原生远控桥接将在后续阶段接入。')),
      ),
      icon: Icon(icon, size: 15),
      label: Text(label, style: const TextStyle(fontSize: 11)),
      style: OutlinedButton.styleFrom(
        minimumSize: const Size(0, 37),
        padding: const EdgeInsets.symmetric(horizontal: 10),
      ),
    );
  }
}

class _HintRow extends StatelessWidget {
  const _HintRow({required this.icon, required this.text});

  final IconData icon;
  final String text;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.only(bottom: 10),
      child: Row(
        children: [
          Icon(icon, size: 17, color: P2PColors.mint),
          const SizedBox(width: 10),
          Expanded(child: Text(text, style: const TextStyle(color: P2PColors.muted, fontSize: 12))),
        ],
      ),
    );
  }
}
