import 'dart:async';
import 'package:flutter/foundation.dart';

import 'package:material_ui/material_ui.dart';

import '../app_theme.dart';
import '../services/remote_controller.dart';
import '../widgets/p2p_widgets.dart';

class NetworkScreen extends StatefulWidget {
  const NetworkScreen({super.key});

  @override
  State<NetworkScreen> createState() => _NetworkScreenState();
}

class _NetworkScreenState extends State<NetworkScreen> {
  final _server = TextEditingController(text: 'wss://p2ptest.aikissgo.com');
  final _token = TextEditingController();
  bool _saving = false;
  Timer? _hostMetricsTimer;

  @override
  void initState() {
    super.initState();
    final controller = P2PRemoteController.instance;
    _hostMetricsTimer = Timer.periodic(const Duration(seconds: 2), (_) {
      if (controller.bridgeAvailable && controller.hostRunning && !controller.isAndroid) {
        unawaited(controller.refreshSnapshot());
      }
    });
    unawaited(controller.initialize().then((_) {
      if (mounted && _server.text == 'wss://p2ptest.aikissgo.com') {
        _server.text = controller.relayUrl;
        setState(() {});
      }
    }));
  }

  @override
  void dispose() {
    _hostMetricsTimer?.cancel();
    _server.dispose();
    _token.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final controller = P2PRemoteController.instance;
    return AnimatedBuilder(
      animation: controller,
      builder: (context, _) {
        return ListView(
          children: [
            const PageHeading(
              eyebrow: 'NETWORK',
              title: '网络连接',
              subtitle: '配置真实 Relay 地址和访问凭据。',
              trailing: StatusDot(color: P2PColors.mint, label: '本机配置'),
            ),
            const SizedBox(height: 24),
            PanelCard(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  const Text('中转服务器', style: TextStyle(fontSize: 18, fontWeight: FontWeight.w800)),
                  const SizedBox(height: 6),
                  Text(
                    defaultTargetPlatform == TargetPlatform.android
                        ? '使用 HTTPS/WSS 地址。访问口令由 Android Keystore 加密保存，不会返回 Flutter 页面。'
                        : '使用 HTTPS/WSS 地址。访问口令会保存在 Windows DPAPI 加密凭据库中，不会返回 Flutter 页面.',
                    style: const TextStyle(color: P2PColors.muted, fontSize: 12, height: 1.5),
                  ),
                  const SizedBox(height: 18),
                  Semantics(
                    identifier: 'p2p_relay_server',
                    child: TextField(
                    controller: _server,
                    enabled: controller.bridgeAvailable && !_saving && !controller.hostRunning && !controller.viewerRequested && !controller.pairingActive,
                    keyboardType: TextInputType.url,
                    decoration: const InputDecoration(
                      labelText: 'Relay HTTPS / WSS 地址',
                      hintText: 'wss://relay.example.com',
                    ),
                    ),
                  ),
                  const SizedBox(height: 12),
                  Semantics(
                    identifier: 'p2p_relay_token',
                    child: TextField(
                    controller: _token,
                    enabled: controller.bridgeAvailable && !_saving && !controller.hostRunning && !controller.viewerRequested && !controller.pairingActive,
                    obscureText: true,
                    decoration: InputDecoration(
                      labelText: 'Relay Access Token',
                      hintText: controller.tokenConfigured ? '已加密保存；留空可保留现有口令' : '输入中转访问口令',
                    ),
                    ),
                  ),
                  const SizedBox(height: 14),
                  Row(
                    children: [
                      Expanded(
                        child: FilledButton.icon(
                          onPressed: controller.bridgeAvailable && !_saving && !controller.hostRunning && !controller.viewerRequested && !controller.pairingActive
                              ? _saveConfiguration
                              : null,
                          icon: _saving
                              ? const SizedBox(width: 16, height: 16, child: CircularProgressIndicator(strokeWidth: 2))
                              : const Icon(Icons.save_rounded),
                          label: Text(_saving ? '保存中…' : '保存中转配置'),
                        ),
                      ),
                    ],
                  ),
                  const SizedBox(height: 12),
                  _NetworkStatusRow(
                    icon: Icons.cloud_done_rounded,
                    title: 'Relay 地址',
                    value: controller.relayUrl,
                  ),
                  Semantics(
                    identifier: controller.tokenConfigured
                        ? 'p2p_relay_token_saved' : 'p2p_relay_token_missing',
                    child: _NetworkStatusRow(
                      icon: Icons.key_rounded,
                      title: '访问口令',
                      value: controller.tokenConfigured
                          ? (defaultTargetPlatform == TargetPlatform.android ? '已通过 Android Keystore 加密保存' : '已通过 DPAPI 加密保存')
                          : '尚未配置',
                    ),
                  ),
                  _NetworkStatusRow(
                    icon: Icons.lock_rounded,
                    title: '本机凭据库',
                    value: controller.snapshot['vault_error'] is String
                        ? '读取失败，已拒绝覆盖'
                        : defaultTargetPlatform == TargetPlatform.android ? 'Android Keystore' : 'Windows 当前用户 DPAPI',
                  ),
                  if (controller.snapshot['vault_error'] is String)
                    Padding(
                      padding: const EdgeInsets.only(top: 10),
                      child: Text(
                        controller.snapshot['vault_error'] as String,
                        style: const TextStyle(color: P2PColors.amber, fontSize: 11),
                      ),
                    ),
                  if (!controller.bridgeAvailable)
                    Padding(
                      padding: const EdgeInsets.only(top: 10),
                      child: Text(controller.bridgeError,
                        style: const TextStyle(color: P2PColors.amber, fontSize: 12)),
                    ),
                  if (controller.lastEvent.isNotEmpty)
                    Padding(
                      padding: const EdgeInsets.only(top: 10),
                      child: Text(controller.lastEvent,
                        style: const TextStyle(color: P2PColors.muted, fontSize: 11)),
                    ),
                ],
              ),
            ),
            const SizedBox(height: 15),
            PanelCard(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  const Text('当前连接策略', style: TextStyle(fontSize: 18, fontWeight: FontWeight.w800)),
                  const SizedBox(height: 10),
                  const Text(
                    '数字配对经 WSS 完成。同一局域网尝试加密 LAN 直连，失败或断线时自动回退 VPS。直连期间仍可保留中转连接用于恢复；视频数据不应持续经 VPS 发送。',
                    style: TextStyle(color: P2PColors.muted, fontSize: 12, height: 1.55),
                  ),
                  if (controller.hostRunning) ...[
                    _NetworkStatusRow(icon: Icons.lan_rounded, title: '本机 LAN 会话',
                      value: controller.snapshot['host_lan_active'] == true ? '已验证直连' : '未激活'),
                    _NetworkStatusRow(icon: Icons.cloud_done_rounded, title: '本机 Relay WebSocket',
                      value: _socketLabel(controller.snapshot['host_relay_socket_open'])),
                    _NetworkStatusRow(icon: Icons.info_outline, title: '本次共享 Relay 建连次数',
                      value: controller.snapshot['host_relay_socket_opens']?.toString() ?? '待测'),
                    _NetworkStatusRow(icon: Icons.swap_vert, title: 'LAN 视频发送',
                      value: _bytes(controller.snapshot['host_lan_video_bytes'])),
                    _NetworkStatusRow(icon: Icons.cloud_done_rounded, title: 'VPS 视频发送',
                      value: _bytes(controller.snapshot['host_relay_video_bytes'])),
                    _NetworkStatusRow(icon: Icons.info_outline, title: 'LAN / VPS 已发送视频包',
                      value: '${controller.snapshot['host_lan_video_frames'] ?? '待测'} / ${controller.snapshot['host_relay_video_frames'] ?? '待测'}'),
                    _NetworkStatusRow(icon: Icons.info_outline, title: '抑制 VPS 视频包',
                      value: controller.snapshot['host_relay_suppressed_frames']?.toString() ?? '待测'),
                  ],
                  if (controller.viewerRequested) ...[
                    _NetworkStatusRow(icon: Icons.lan_rounded, title: '当前控制端传输',
                      value: controller.transport),
                    if (controller.isAndroid) ...[
                      _NetworkStatusRow(icon: Icons.info_outline, title: '连接阶段',
                        value: controller.transportPhaseLabel),
                      _NetworkStatusRow(icon: Icons.timer_outlined, title: 'LAN 尝试耗时',
                        value: _millis(controller.snapshot['lan_attempt_ms'])),
                      _NetworkStatusRow(icon: Icons.info_outline, title: '最近 LAN 回退原因',
                        value: controller.lanFallbackReasonLabel),
                      if (controller.snapshot['lan_fallback_started'] == true) ...[
                        _NetworkStatusRow(icon: Icons.timer_outlined, title: '回退至 WSS 建连',
                          value: _millis(controller.snapshot['lan_fallback_relay_open_ms'])),
                        _NetworkStatusRow(icon: Icons.timer_outlined, title: '回退至首帧画面',
                          value: _millis(controller.snapshot['lan_fallback_first_video_ms'])),
                      ],
                    ],
                    _NetworkStatusRow(icon: Icons.swap_vert, title: 'LAN 视频接收',
                      value: _bytes(controller.snapshot['viewer_lan_video_bytes'])),
                    _NetworkStatusRow(icon: Icons.cloud_done_rounded, title: 'VPS 视频接收',
                      value: _bytes(controller.snapshot['viewer_relay_video_bytes'])),
                  ],
                  const Text('WebSocket 已连接不代表对端已经接收画面。累计加密视频包字节不含 TCP/TLS、握手和元数据；并非网卡总流量。',
                    style: TextStyle(color: P2PColors.muted, fontSize: 11)),
                ],
              ),
            ),
          ],
        );
      },
    );
  }

  String _socketLabel(Object? value) =>
      value == true ? '已建立' : value == false ? '未建立' : '待测';

  String _millis(Object? value) =>
      value is num && value >= 0 ? '${value.toInt()} ms' : '待测';

  String _bytes(Object? value) {
    if (value is! num || value < 0) return '待测';
    return '${value.toInt()} B';
  }

  Future<void> _saveConfiguration() async {
    setState(() => _saving = true);
    final ok = await P2PRemoteController.instance.configureRelay(_server.text, _token.text);
    if (!mounted) return;
    setState(() => _saving = false);
    ScaffoldMessenger.of(context).showSnackBar(
      SnackBar(
        content: Text(ok
            ? (defaultTargetPlatform == TargetPlatform.android ? '中转配置已通过 Android Keystore 保存' : '中转配置已通过 DPAPI 保存')
            : P2PRemoteController.instance.lastEvent),
      ),
    );
    if (ok) _token.clear();
  }
}

class _NetworkStatusRow extends StatelessWidget {
  const _NetworkStatusRow({
    required this.icon,
    required this.title,
    required this.value,
  });

  final IconData icon;
  final String title;
  final String value;

  @override
  Widget build(BuildContext context) => Padding(
        padding: const EdgeInsets.symmetric(vertical: 8),
        child: Row(
          children: [
            Icon(icon, size: 18, color: P2PColors.mint),
            const SizedBox(width: 11),
            Expanded(child: Text(title, style: const TextStyle(fontWeight: FontWeight.w600))),
            Flexible(
              child: Text(
                value,
                textAlign: TextAlign.end,
                style: const TextStyle(color: P2PColors.muted, fontSize: 11),
              ),
            ),
          ],
        ),
      );
}
