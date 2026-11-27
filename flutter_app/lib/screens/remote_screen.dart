import 'dart:math' as math;

import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:material_ui/material_ui.dart';

import '../app_theme.dart';
import '../services/remote_controller.dart';
import '../services/performance_export.dart';
import '../widgets/p2p_widgets.dart';
import '../widgets/remote_keyboard.dart';

class RemoteScreen extends StatefulWidget {
  const RemoteScreen({super.key});

  @override
  State<RemoteScreen> createState() => _RemoteScreenState();
}

class _RemoteScreenState extends State<RemoteScreen> {
  final _deviceNumber = TextEditingController();
  final _pairingCode = TextEditingController();
  final _textToSend = TextEditingController();
  final _clipboardToSend = TextEditingController();
  final _legacyKey = TextEditingController();
  bool _showLegacy = false;
  bool _pressed = false;
  int? _activePointer;
  double _lastX = 0.5;
  double _lastY = 0.5;
  DateTime _lastMoveSent = DateTime.fromMillisecondsSinceEpoch(0);

  P2PRemoteController get _controller => P2PRemoteController.instance;

  @override
  void dispose() {
    _releasePointer();
    _deviceNumber.dispose();
    _pairingCode.dispose();
    _textToSend.dispose();
    _clipboardToSend.dispose();
    _legacyKey.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return AnimatedBuilder(
      animation: _controller,
      builder: (context, _) {
        final connected = _controller.viewerConnected;
        final requested = _controller.viewerRequested;
        final inputAllowed = connected && _controller.viewerAllowsInput;
        final clipboardAllowed = connected && _controller.viewerAllowsClipboard;
        return ListView(
          children: [
            PageHeading(
              eyebrow: 'REMOTE SESSION  /  V0.21',
              title: '远程控制',
              subtitle: '通过端到端加密会话查看桌面，并发送鼠标、键盘和剪贴板操作。',
              trailing: StatusDot(
                color: connected ? P2PColors.mint : P2PColors.muted,
                label: connected ? '画面已验证' : requested ? '连接或恢复中' : '未连接',
              ),
            ),
            const SizedBox(height: 20),
            PanelCard(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  const Text('连接 Windows 电脑', style: TextStyle(fontSize: 17, fontWeight: FontWeight.w800)),
                  const SizedBox(height: 6),
                  const Text(
                    '首次连接需要电脑端显示动态验证码，并由电脑使用者明确批准。',
                    style: TextStyle(color: P2PColors.muted, fontSize: 12),
                  ),
                  const SizedBox(height: 14),
                  Row(
                    children: [
                      Expanded(
                        child: Semantics(
                          identifier: 'p2p_remote_device_number',
                          child: TextField(
                          controller: _deviceNumber,
                          enabled: !requested && !_controller.pairingActive,
                          keyboardType: TextInputType.number,
                          inputFormatters: [FilteringTextInputFormatter.digitsOnly, LengthLimitingTextInputFormatter(9)],
                          onChanged: (_) => setState(() {}),
                          decoration: const InputDecoration(labelText: '9位设备号', hintText: '输入电脑上的设备号'),
                          ),
                        ),
                      ),
                      const SizedBox(width: 10),
                      Expanded(
                        child: Semantics(
                          identifier: 'p2p_remote_pairing_code',
                          child: TextField(
                          controller: _pairingCode,
                          enabled: !requested && !_controller.pairingActive,
                          keyboardType: TextInputType.number,
                          obscureText: true,
                          inputFormatters: [FilteringTextInputFormatter.digitsOnly, LengthLimitingTextInputFormatter(6)],
                          onChanged: (_) => setState(() {}),
                          decoration: const InputDecoration(labelText: '6位动态验证码', hintText: '一次性验证码'),
                          ),
                        ),
                      ),
                    ],
                  ),
                  const SizedBox(height: 12),
                  Wrap(
                    spacing: 9,
                    runSpacing: 8,
                    children: [
                      FilledButton.icon(
                        onPressed: !_controller.bridgeAvailable || !_controller.tokenConfigured || requested
                            ? null
                            : _controller.pairingActive
                                ? () => _controller.cancelPairing()
                                : _deviceNumber.text.length == 9 && _pairingCode.text.length == 6
                                    ? () => _controller.requestPairing(_deviceNumber.text, _pairingCode.text)
                                    : null,
                        icon: Icon(_controller.pairingActive ? Icons.close_rounded : Icons.link_rounded),
                        label: Text(_controller.pairingActive ? '取消配对' : '请求连接'),
                      ),
                      if (requested || connected)
                        OutlinedButton.icon(
                          onPressed: () => _controller.disconnectViewer(),
                          icon: const Icon(Icons.link_off_rounded),
                          label: const Text('断开连接'),
                        ),
                      TextButton(
                        onPressed: () => setState(() => _showLegacy = !_showLegacy),
                        child: Text(_showLegacy ? '收起旧版密钥' : '旧版64位密钥连接'),
                      ),
                    ],
                  ),
                  if (_showLegacy) ...[
                    const SizedBox(height: 10),
                    TextField(
                      controller: _legacyKey,
                      obscureText: true,
                      maxLength: 64,
                      onChanged: (_) => setState(() {}),
                      decoration: const InputDecoration(
                        labelText: '64位十六进制会话密钥',
                        counterText: '',
                      ),
                    ),
                    FilledButton(
                      onPressed: !_controller.bridgeAvailable || !_controller.tokenConfigured || requested || _controller.pairingActive || !RegExp(r'^[0-9a-fA-F]{64}$').hasMatch(_legacyKey.text)
                          ? null
                          : () => _controller.connectLegacy(_legacyKey.text),
                      child: const Text('使用旧版密钥连接'),
                    ),
                  ],
                  const SizedBox(height: 8),
                  Text(
                    _controller.viewerStatus,
                    style: TextStyle(
                      color: connected ? P2PColors.mint : P2PColors.muted,
                      fontSize: 12,
                    ),
                  ),
                  if (!_controller.tokenConfigured)
                    const Text('请先在网络连接中保存 Relay 访问口令。'),
                  if (requested || connected)
                    Text('${_controller.transport} · RTT ${_controller.rttMs?.toString() ?? "待测"} ms'),
                  if (!_controller.bridgeAvailable)
                    Padding(
                      padding: const EdgeInsets.only(top: 7),
                      child: Text(_controller.bridgeError,
                        style: const TextStyle(color: P2PColors.amber, fontSize: 11)),
                    ),
                ],
              ),
            ),
            const SizedBox(height: 14),
            PanelCard(
              padding: const EdgeInsets.all(12),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Row(
                    children: [
                      const Expanded(
                        child: Text('远程桌面', style: TextStyle(fontSize: 16, fontWeight: FontWeight.w800)),
                      ),
                      if (connected)
                        Semantics(
                          identifier: 'p2p_remote_fullscreen',
                          label: '全屏远控',
                          button: true,
                          child: IconButton(
                            tooltip: '全屏远控',
                            onPressed: _openFullscreen,
                            icon: const Icon(Icons.fullscreen_rounded),
                          ),
                        ),
                      if (connected && _controller.monitorCount > 1)
                        DropdownButton<int>(
                          value: _controller.selectedMonitor.clamp(0, _controller.monitorCount - 1).toInt(),
                          items: List.generate(
                            _controller.monitorCount,
                            (index) => DropdownMenuItem(value: index, child: Text('显示器 ${index + 1}')),
                          ),
                          onChanged: (index) {
                            if (index != null) _controller.selectMonitor(index);
                          },
                        ),
                      if (connected)
                        Text(
                          '${_controller.frameWidth}×${_controller.frameHeight}',
                          style: const TextStyle(color: P2PColors.muted, fontSize: 11),
                        ),
                    ],
                  ),
                  const SizedBox(height: 10),
                  LayoutBuilder(
                    builder: (context, constraints) {
                      final width = constraints.maxWidth;
                      final ratio = _controller.frameWidth > 0 && _controller.frameHeight > 0
                          ? _controller.frameWidth / _controller.frameHeight
                          : 16 / 9;
                      final height = math.min(width / ratio, 620.0);
                      final surfaceSize = Size(width, height);
                      return ClipRRect(
                        borderRadius: BorderRadius.circular(12),
                        child: _videoSurface(surfaceSize),
                      );
                    },
                  ),
                  const SizedBox(height: 8),
                  const Text(
                    '点击：鼠标左键 · 拖动：按住并移动 · 右键：鼠标右键 · 滚轮：远程滚动',
                    style: TextStyle(color: P2PColors.muted, fontSize: 11),
                  ),
                ],
              ),
            ),
            if (_controller.isAndroid) ...[
              const SizedBox(height: 14),
              PanelCard(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    const Text('网络诊断与可选 UDP 预览',
                      style: TextStyle(fontWeight: FontWeight.w800, fontSize: 15)),
                    const SizedBox(height: 8),
                    Wrap(
                      spacing: 8,
                      runSpacing: 8,
                      children: [
                        OutlinedButton.icon(
                          onPressed: _controller.bridgeAvailable ? _controller.probeStun : null,
                          icon: const Icon(Icons.network_check_rounded),
                          label: const Text('检测 STUN'),
                        ),
                        FilledButton.icon(
                          onPressed: null,
                          icon: const Icon(Icons.bolt_rounded),
                          label: const Text('UDP 预览暂未接入新版 Host'),
                        ),
                      ],
                    ),
                    const SizedBox(height: 8),
                    Text(_controller.stunReport,
                      style: const TextStyle(color: P2PColors.muted, fontSize: 11)),
                    const SizedBox(height: 4),
                    Text(_controller.udpReport,
                      style: const TextStyle(color: P2PColors.muted, fontSize: 11)),
                  ],
                ),
              ),
            ],
            const SizedBox(height: 14),
            LayoutBuilder(
              builder: (context, constraints) {
                final columns = constraints.maxWidth > 850 ? 2 : 1;
                final itemWidth = (constraints.maxWidth - (columns - 1) * 14) / columns;
                return Wrap(
                  spacing: 14,
                  runSpacing: 14,
                  children: [
                    SizedBox(
                      width: itemWidth,
                      child: PanelCard(
                        child: Column(
                          crossAxisAlignment: CrossAxisAlignment.start,
                          children: [
                            const Text('键盘与文字', style: TextStyle(fontWeight: FontWeight.w800, fontSize: 15)),
                            const SizedBox(height: 10),
                            TextField(
                              controller: _textToSend,
                              enabled: inputAllowed,
                              onChanged: (_) => setState(() {}),
                              maxLength: 250,
                              decoration: const InputDecoration(
                                labelText: '发送文字到远程输入框',
                                counterText: '',
                              ),
                            ),
                            const SizedBox(height: 8),
                            Wrap(
                              spacing: 7,
                              runSpacing: 7,
                              children: [
                                FilledButton(
                                  onPressed: inputAllowed && _textToSend.text.isNotEmpty
                                      ? () async {
                                          final value = _textToSend.text;
                                          if (await _controller.sendText(value)) _textToSend.clear();
                                        }
                                      : null,
                                  child: const Text('发送文字'),
                                ),
                                for (final key in const ['enter', 'backspace', 'tab', 'escape', 'ctrl_a', 'ctrl_c', 'ctrl_v'])
                                  OutlinedButton(
                                    onPressed: inputAllowed ? () => _controller.sendKey(key) : null,
                                    child: Text(key.replaceAll('_', ' ')),
                                  ),
                              ],
                            ),
                          ],
                        ),
                      ),
                    ),
                    SizedBox(
                      width: itemWidth,
                      child: PanelCard(
                        child: Column(
                          crossAxisAlignment: CrossAxisAlignment.start,
                          children: [
                            const Text('剪贴板', style: TextStyle(fontWeight: FontWeight.w800, fontSize: 15)),
                            const SizedBox(height: 8),
                            const Text(
                              '仅在你主动点击时读取或发送。被控电脑必须单独授权剪贴板。',
                              style: TextStyle(color: P2PColors.muted, fontSize: 11),
                            ),
                            const SizedBox(height: 10),
                            OutlinedButton.icon(
                              onPressed: clipboardAllowed ? () => _controller.getClipboardText() : null,
                              icon: const Icon(Icons.download_rounded),
                              label: const Text('读取远程剪贴板'),
                            ),
                            if (_controller.clipboardText.isNotEmpty) ...[
                              const SizedBox(height: 7),
                              SelectableText(
                                _controller.clipboardText,
                                style: const TextStyle(color: P2PColors.muted, fontSize: 12),
                              ),
                            ],
                            const SizedBox(height: 10),
                            TextField(
                              controller: _clipboardToSend,
                              enabled: clipboardAllowed,
                              onChanged: (_) => setState(() {}),
                              maxLines: 2,
                              maxLength: 4096,
                              decoration: const InputDecoration(labelText: '发送到远程剪贴板', counterText: ''),
                            ),
                            const SizedBox(height: 7),
                            FilledButton(
                              onPressed: clipboardAllowed && _clipboardToSend.text.isNotEmpty
                                  ? () => _controller.setClipboardText(_clipboardToSend.text)
                                  : null,
                              child: const Text('发送剪贴板文字'),
                            ),
                          ],
                        ),
                      ),
                    ),
                  ],
                );
              },
            ),
            if (_controller.lastEvent.isNotEmpty) ...[
              const SizedBox(height: 12),
              Text(_controller.lastEvent,
                style: const TextStyle(color: P2PColors.muted, fontSize: 11)),
            ],
          ],
        );
      },
    );
  }

  Widget _videoSurface(Size size) {
    final connected = _controller.viewerConnected;
    final ratio = _controller.frameWidth > 0 && _controller.frameHeight > 0
        ? _controller.frameWidth / _controller.frameHeight : 16 / 9;
    return SizedBox(
      width: size.width,
      height: size.height,
      child: ColoredBox(
        color: const Color(0xFF070D16),
        child: Center(
          child: AspectRatio(
            aspectRatio: ratio,
            child: LayoutBuilder(
              builder: (context, constraints) {
                final actual = Size(constraints.maxWidth, constraints.maxHeight);
                return Semantics(
                  container: true,
                  identifier: 'p2p_remote_video',
                  label: '远程画面操作区域',
                  excludeSemantics: true,
                  child: Listener(
                    behavior: HitTestBehavior.opaque,
                    onPointerHover: (event) => _movePointer(event.localPosition, actual),
                    onPointerMove: (event) {
                      if (_activePointer == event.pointer) {
                        _movePointer(event.localPosition, actual);
                      }
                    },
                    onPointerDown: (event) => _pointerDown(event, actual),
                    onPointerUp: (event) {
                      if (_activePointer == event.pointer) {
                        _movePointer(event.localPosition, actual);
                        _releasePointer();
                      }
                    },
                    onPointerCancel: (event) {
                      if (_activePointer == event.pointer) _releasePointer();
                    },
                    onPointerSignal: (event) {
                      if (event is PointerScrollEvent && connected && _controller.viewerAllowsInput) {
                        if (event.scrollDelta.dy.abs() >= 1) {
                          _controller.sendScroll(event.scrollDelta.dy > 0 ? -1 : 1);
                        }
                      }
                    },
                    child: SizedBox.expand(
                      child: _controller.h264Active && _controller.videoTextureId != null
                          ? Texture(textureId: _controller.videoTextureId!, filterQuality: FilterQuality.low)
                          : _controller.frame != null
                              ? Image.memory(_controller.frame!, fit: BoxFit.contain,
                                  gaplessPlayback: true, filterQuality: FilterQuality.low)
                              : Center(child: Text(connected ? '等待远程桌面画面…' : '连接成功后，远程画面会显示在这里')),
                    ),
                  ),
                );
              },
            ),
          ),
        ),
      ),
    );
  }

  Future<void> _openFullscreen() async {
    _releasePointer();
    final android = _controller.isAndroid;
    if (android) {
      await SystemChrome.setPreferredOrientations([
        DeviceOrientation.landscapeLeft, DeviceOrientation.landscapeRight,
      ]);
    }
    if (!mounted) {
      if (android) await SystemChrome.setPreferredOrientations([]);
      return;
    }
    try {
      await Navigator.of(context).push<void>(
      MaterialPageRoute<void>(
        fullscreenDialog: true,
        builder: (routeContext) => AnimatedBuilder(
          animation: _controller,
          builder: (context, _) => Scaffold(
            backgroundColor: const Color(0xFF070D16),
            body: SafeArea(
              child: Column(
                children: [
                  Wrap(
                    alignment: WrapAlignment.center,
                    crossAxisAlignment: WrapCrossAlignment.center,
                    spacing: 6,
                    children: [
                      TextButton(onPressed: () => Navigator.of(routeContext).pop(),
                        child: const Text('退出全屏')),
                      TextButton(
                        onPressed: () => _showVideoMetrics(routeContext),
                        child: const Text('视频性能'),
                      ),
                      TextButton(
                        onPressed: _controller.viewerConnected && _controller.viewerAllowsInput
                            ? () => _fullscreenText(routeContext) : null,
                        child: const Text('远程输入文字'),
                      ),
                      TextButton(
                        onPressed: _controller.viewerRequested || _controller.viewerConnected
                            ? () async {
                                _releasePointer();
                                await _controller.disconnectViewer();
                                if (routeContext.mounted) Navigator.of(routeContext).pop();
                              } : null,
                        child: const Text('断开连接'),
                      ),
                    ],
                  ),
                  RemoteKeyboard(controller: _controller),
                  Text(_controller.viewerStatus, style: const TextStyle(fontSize: 12)),
                  Expanded(child: LayoutBuilder(
                    builder: (context, constraints) => _videoSurface(
                      Size(constraints.maxWidth, constraints.maxHeight),
                    ),
                  )),
                ],
              ),
            ),
          ),
        ),
      ),
      );
    } finally {
      _releasePointer();
      if (android) await SystemChrome.setPreferredOrientations([]);
    }
  }

  Future<void> _showVideoMetrics(BuildContext routeContext) async {
    final s = _controller.snapshot;
    final capturedAt = DateTime.now().toUtc();
    final codec = s['active_video_codec'] as String? ??
        (_controller.viewerConnected ? (s['h264_active'] == true ? 'H264' : 'JPEG') : 'none');
    final hardware = s['avc_decoder_hardware'] == true
        ? 'hardware' : s['avc_decoder_hardware'] == false ? 'software' : 'unknown';
    final details = 'codec=$codec\n'
        'frames=${s['avc_rendered_frames'] ?? 0}\n'
        'fps=${s['rendered_fps'] ?? 0}\n'
        'bps=${s['measured_video_bps'] ?? 0}\n'
        'rtt=${s['rtt_ms'] ?? -1}\n'
        'relay_socket_opens=${s['relay_socket_opens'] ?? 0}\n'
        'relay_retry_events=${s['relay_retry_events'] ?? 0}\n'
        'target_bps=${s['video_target_bps'] ?? 0}\n'
        'target_fps=${s['video_target_fps'] ?? 0}\n'
        'drops=${s['avc_decoder_drops'] ?? 0}\n'
        'encoder=${s['video_encoder'] ?? 'unknown'}\n'
        'decoder=${s['avc_decoder'] ?? 'unknown'}\n'
        'decoder_mode=$hardware\n'
        'decoder_recovering=${s['avc_decoder_recovering'] ?? 'unknown'}\n'
        'capture_backend=${s['capture_backend'] ?? 'unknown'}\n'
        'capture_avg_us=${s['capture_avg_us'] ?? 0}\n'
        'encode_avg_us=${s['encode_avg_us'] ?? 0}\n'
        'send_avg_us=${s['send_avg_us'] ?? 0}\n'
        'fresh_frames=${s['fresh_frames'] ?? 0}\n'
        'reused_frames=${s['reused_frames'] ?? 0}\n'
        'resize_cache_hits=${s['resize_cache_hits'] ?? 0}\n'
        'yuv_cache_hits=${s['yuv_cache_hits'] ?? 0}\n'
        'host_encoded_frames=${s['host_encoded_frames'] ?? 0}\n'
        'transport=${_controller.transport}\n'
        'transport_phase=${s['viewer_transport_phase'] ?? 'unknown'}\n'
        'host_metadata_current=${s['host_metadata_current'] ?? false}\n'
        'lan_attempt_ms=${s['lan_attempt_ms'] ?? -1}\n'
        'lan_candidate_attempts=${s['lan_candidate_attempts'] ?? 0}\n'
        'network_observer_ready=${s['network_observer_ready'] ?? false}\n'
        'network_change_events=${s['network_change_events'] ?? 0}\n'
        'network_retry_wakes=${s['network_retry_wakes'] ?? 0}\n'
        'lan_fallback_reason=${s['lan_fallback_reason'] ?? 'none'}\n'
        'lan_fallback_started=${s['lan_fallback_started'] ?? false}\n'
        'lan_fallback_relay_open_ms=${s['lan_fallback_relay_open_ms'] ?? -1}\n'
        'lan_fallback_first_video_ms=${s['lan_fallback_first_video_ms'] ?? -1}\n'
        'viewer_lan_video_bytes=${s['viewer_lan_video_bytes'] ?? -1}\n'
        'viewer_relay_video_bytes=${s['viewer_relay_video_bytes'] ?? -1}\n'
        'host_lan_video_bytes=${s['host_lan_video_bytes'] ?? -1}\n'
        'host_relay_video_bytes=${s['host_relay_video_bytes'] ?? -1}\n'
        'host_relay_suppressed_frames=${s['host_relay_suppressed_frames'] ?? -1}\n'
        'host_lan_video_frames=${s['host_lan_video_frames'] ?? -1}\n'
        'host_relay_video_frames=${s['host_relay_video_frames'] ?? -1}\n'
        'host_relay_socket_open=${s['host_relay_socket_open'] ?? 'unknown'}\n'
        'host_relay_socket_opens=${s['host_relay_socket_opens'] ?? -1}';
    await showDialog<void>(
      context: routeContext,
      builder: (dialogContext) => AlertDialog(
        title: const Text('实际视频性能'),
        content: SingleChildScrollView(
          child: Semantics(
            identifier: 'p2p_video_metrics',
            child: Text(details, style: const TextStyle(fontSize: 11)),
          ),
        ),
        actions: [
          TextButton(
            onPressed: () async {
              await Clipboard.setData(ClipboardData(
                text: exportPerformanceSnapshot(s, capturedAt: capturedAt),
              ));
              if (dialogContext.mounted) {
                ScaffoldMessenger.of(routeContext).showSnackBar(
                  const SnackBar(content: Text('性能 JSON 已复制到本机剪贴板')),
                );
              }
            },
            child: const Text('复制性能 JSON'),
          ),
          TextButton(onPressed: () => Navigator.of(dialogContext).pop(),
            child: const Text('关闭性能数据')),
        ],
      ),
    );
  }

  Future<void> _fullscreenText(BuildContext routeContext) async {
    // Landscape keyboards can occupy most of the short dimension. A full
    // route with a compact horizontal form keeps the input and send action
    // above the IME, instead of shrinking the content of an AlertDialog.
    await Navigator.of(routeContext).push<void>(
      MaterialPageRoute<void>(
        fullscreenDialog: true,
        builder: (_) => _RemoteTextPage(controller: _controller),
      ),
    );
  }

  (double, double)? _normalized(Offset point, Size surface) {
    final width = _controller.frameWidth.toDouble();
    final height = _controller.frameHeight.toDouble();
    if (width <= 0 || height <= 0 || surface.width <= 0 || surface.height <= 0) return null;
    final scale = math.min(surface.width / width, surface.height / height);
    final shownWidth = width * scale;
    final shownHeight = height * scale;
    final left = (surface.width - shownWidth) / 2;
    final top = (surface.height - shownHeight) / 2;
    if (point.dx < left || point.dy < top ||
        point.dx > left + shownWidth || point.dy > top + shownHeight) {
      return null;
    }
    return (
      ((point.dx - left) / shownWidth).clamp(0.0, 1.0).toDouble(),
      ((point.dy - top) / shownHeight).clamp(0.0, 1.0).toDouble(),
    );
  }

  void _movePointer(Offset point, Size surface) {
    if (!_controller.viewerConnected || !_controller.viewerAllowsInput) return;
    final now = DateTime.now();
    if (now.difference(_lastMoveSent).inMilliseconds < 25) return;
    final position = _normalized(point, surface);
    if (position == null) return;
    _lastMoveSent = now;
    _lastX = position.$1;
    _lastY = position.$2;
    _controller.sendPointer('move', _lastX, _lastY);
  }

  void _pointerDown(PointerDownEvent event, Size surface) {
    if (!_controller.viewerConnected || !_controller.viewerAllowsInput || _activePointer != null) return;
    final position = _normalized(event.localPosition, surface);
    if (position == null) return;
    _lastX = position.$1;
    _lastY = position.$2;
    if ((event.buttons & kSecondaryMouseButton) != 0) {
      _controller.sendPointer('right_click', _lastX, _lastY);
      return;
    }
    _pressed = true;
    _activePointer = event.pointer;
    _controller.sendPointer('down', _lastX, _lastY);
  }

  void _releasePointer() {
    if (!_pressed) return;
    _pressed = false;
    _activePointer = null;
    _controller.sendPointer('up', _lastX, _lastY);
  }
}

class _RemoteTextPage extends StatefulWidget {
  const _RemoteTextPage({required this.controller});
  final P2PRemoteController controller;

  @override
  State<_RemoteTextPage> createState() => _RemoteTextPageState();
}

class _RemoteTextPageState extends State<_RemoteTextPage> {
  final _text = TextEditingController();
  bool _sending = false;

  @override
  void dispose() {
    _text.dispose();
    super.dispose();
  }

  Future<void> _send() async {
    if (_sending || _text.text.trim().isEmpty ||
        !widget.controller.viewerConnected || !widget.controller.viewerAllowsInput) {
      return;
    }
    setState(() => _sending = true);
    try {
      final ok = await widget.controller.sendText(_text.text);
      if (!mounted) return;
      if (ok) {
        FocusScope.of(context).unfocus();
        Navigator.of(context).pop();
      } else {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(content: Text(widget.controller.lastEvent)),
        );
      }
    } finally {
      if (mounted) setState(() => _sending = false);
    }
  }

  @override
  Widget build(BuildContext context) => Scaffold(
    resizeToAvoidBottomInset: true,
    body: SafeArea(
      child: AnimatedBuilder(
        animation: widget.controller,
        builder: (context, _) {
          final allowed = widget.controller.viewerConnected && widget.controller.viewerAllowsInput;
          return SingleChildScrollView(
            padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 4),
            child: Center(
              child: ConstrainedBox(
                constraints: const BoxConstraints(maxWidth: 960),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Row(
                      children: [
                        TextButton(onPressed: _sending ? null : () => Navigator.of(context).pop(),
                          child: const Text('取消')),
                        const Expanded(child: Text('远程输入文字', style: TextStyle(fontWeight: FontWeight.w700))),
                      ],
                    ),
                    Row(
                      children: [
                        Expanded(
                          child: Semantics(
                            identifier: 'p2p_remote_text',
                            child: TextField(
                              controller: _text,
                              enabled: allowed && !_sending,
                              maxLength: 250,
                              textInputAction: TextInputAction.send,
                              onSubmitted: (_) => _send(),
                              decoration: const InputDecoration(
                                labelText: '远程文字内容',
                                counterText: '',
                                isDense: true,
                              ),
                            ),
                          ),
                        ),
                        const SizedBox(width: 8),
                        FilledButton(
                          onPressed: allowed && !_sending ? _send : null,
                          child: const Text('发送远程文字'),
                        ),
                      ],
                    ),
                  ],
                ),
              ),
            ),
          );
        },
      ),
    ),
  );
}
