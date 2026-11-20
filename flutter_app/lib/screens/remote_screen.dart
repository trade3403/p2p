import 'dart:math' as math;

import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:material_ui/material_ui.dart';

import '../app_theme.dart';
import '../services/remote_controller.dart';
import '../widgets/p2p_widgets.dart';

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
  double _lastX = 0.5;
  double _lastY = 0.5;
  DateTime _lastMoveSent = DateTime.fromMillisecondsSinceEpoch(0);

  P2PRemoteController get _controller => P2PRemoteController.instance;

  @override
  void dispose() {
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
        return ListView(
          children: [
            PageHeading(
              eyebrow: 'REMOTE SESSION  /  V0.18',
              title: '远程控制',
              subtitle: '通过端到端加密会话查看桌面，并发送鼠标、键盘和剪贴板操作。',
              trailing: StatusDot(
                color: connected ? P2PColors.mint : P2PColors.muted,
                label: connected ? '已连接' : '未连接',
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
                        child: TextField(
                          controller: _deviceNumber,
                          enabled: !connected && !_controller.pairingActive,
                          keyboardType: TextInputType.number,
                          inputFormatters: [FilteringTextInputFormatter.digitsOnly, LengthLimitingTextInputFormatter(9)],
                          onChanged: (_) => setState(() {}),
                          decoration: const InputDecoration(labelText: '9位设备号', hintText: '输入电脑上的设备号'),
                        ),
                      ),
                      const SizedBox(width: 10),
                      Expanded(
                        child: TextField(
                          controller: _pairingCode,
                          enabled: !connected && !_controller.pairingActive,
                          keyboardType: TextInputType.number,
                          obscureText: true,
                          inputFormatters: [FilteringTextInputFormatter.digitsOnly, LengthLimitingTextInputFormatter(6)],
                          onChanged: (_) => setState(() {}),
                          decoration: const InputDecoration(labelText: '6位动态验证码', hintText: '一次性验证码'),
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
                        onPressed: connected
                            ? null
                            : _controller.pairingActive
                                ? () => _controller.cancelPairing()
                                : _deviceNumber.text.length == 9 && _pairingCode.text.length == 6
                                    ? () => _controller.requestPairing(_deviceNumber.text, _pairingCode.text)
                                    : null,
                        icon: Icon(_controller.pairingActive ? Icons.close_rounded : Icons.link_rounded),
                        label: Text(_controller.pairingActive ? '取消配对' : '请求连接'),
                      ),
                      if (connected)
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
                      onPressed: connected || _legacyKey.text.length != 64
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
                        child: Listener(
                          behavior: HitTestBehavior.opaque,
                          onPointerHover: (event) => _movePointer(event.localPosition, surfaceSize),
                          onPointerMove: (event) => _movePointer(event.localPosition, surfaceSize),
                          onPointerDown: (event) => _pointerDown(event, surfaceSize),
                          onPointerUp: _pointerUp,
                          onPointerCancel: (_) => _releasePointer(),
                          onPointerSignal: (event) {
                            if (event is PointerScrollEvent && connected) {
                              final delta = event.scrollDelta.dy;
                              if (delta.abs() >= 1) _controller.sendScroll(delta > 0 ? -1 : 1);
                            }
                          },
                          child: SizedBox(
                            width: width,
                            height: height,
                            child: ColoredBox(
                              color: const Color(0xFF070D16),
                              child: _controller.h264Active && _controller.videoTextureId != null
                                  ? Texture(
                                      textureId: _controller.videoTextureId!,
                                      filterQuality: FilterQuality.low,
                                    )
                                  : _controller.frame != null
                                      ? Image.memory(
                                          _controller.frame!,
                                          fit: BoxFit.contain,
                                          gaplessPlayback: true,
                                          filterQuality: FilterQuality.low,
                                        )
                                      : Center(
                                      child: Column(
                                        mainAxisSize: MainAxisSize.min,
                                        children: [
                                          Icon(
                                            connected ? Icons.hourglass_empty_rounded : Icons.desktop_access_disabled_rounded,
                                            color: P2PColors.muted,
                                            size: 42,
                                          ),
                                          const SizedBox(height: 12),
                                          Text(
                                            connected ? '等待远程桌面画面…' : '连接成功后，远程画面会显示在这里',
                                            style: const TextStyle(color: P2PColors.muted, fontSize: 13),
                                          ),
                                        ],
                                      ),
                                    ),
                            ),
                          ),
                        ),
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
                          onPressed: _controller.probeStun,
                          icon: const Icon(Icons.network_check_rounded),
                          label: const Text('检测 STUN'),
                        ),
                        FilledButton.icon(
                          onPressed: connected && _controller.tokenConfigured && !_controller.udpAttempted
                              ? _controller.startUdpPunch
                              : null,
                          icon: const Icon(Icons.bolt_rounded),
                          label: const Text('尝试加密 UDP 预览'),
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
                              enabled: connected,
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
                                  onPressed: connected && _textToSend.text.isNotEmpty
                                      ? () async {
                                          final value = _textToSend.text;
                                          if (await _controller.sendText(value)) _textToSend.clear();
                                        }
                                      : null,
                                  child: const Text('发送文字'),
                                ),
                                for (final key in const ['enter', 'backspace', 'tab', 'escape', 'ctrl_a', 'ctrl_c', 'ctrl_v'])
                                  OutlinedButton(
                                    onPressed: connected ? () => _controller.sendKey(key) : null,
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
                              onPressed: connected ? () => _controller.getClipboardText() : null,
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
                              enabled: connected,
                              onChanged: (_) => setState(() {}),
                              maxLines: 2,
                              maxLength: 4096,
                              decoration: const InputDecoration(labelText: '发送到远程剪贴板', counterText: ''),
                            ),
                            const SizedBox(height: 7),
                            FilledButton(
                              onPressed: connected && _clipboardToSend.text.isNotEmpty
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
    if (!_controller.viewerConnected) return;
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
    if (!_controller.viewerConnected) return;
    final position = _normalized(event.localPosition, surface);
    if (position == null) return;
    _lastX = position.$1;
    _lastY = position.$2;
    if ((event.buttons & kSecondaryMouseButton) != 0) {
      _controller.sendPointer('right_click', _lastX, _lastY);
      return;
    }
    _pressed = true;
    _controller.sendPointer('down', _lastX, _lastY);
  }

  void _pointerUp(PointerUpEvent event) => _releasePointer();

  void _releasePointer() {
    if (!_pressed) return;
    _pressed = false;
    _controller.sendPointer('up', _lastX, _lastY);
  }
}
