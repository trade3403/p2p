import 'dart:async';
import 'dart:convert';
import 'dart:ffi';
import 'dart:io';
import 'dart:typed_data';

import 'package:ffi/ffi.dart';
import 'package:flutter/foundation.dart';

typedef _InitNative = Int32 Function();
typedef _InitDart = int Function();
typedef _CommandNative = Pointer<Utf8> Function(Pointer<Utf8>);
typedef _CommandDart = Pointer<Utf8> Function(Pointer<Utf8>);
typedef _PollNative = Pointer<Utf8> Function();
typedef _PollDart = Pointer<Utf8> Function();
typedef _FreeStringNative = Void Function(Pointer<Utf8>);
typedef _FreeStringDart = void Function(Pointer<Utf8>);
typedef _SizeNative = IntPtr Function();
typedef _SizeDart = int Function();
typedef _VersionNative = Uint64 Function();
typedef _VersionDart = int Function();
typedef _CopyFrameNative = IntPtr Function(
  Pointer<Uint8>,
  IntPtr,
  Pointer<Uint32>,
  Pointer<Uint32>,
  Pointer<Uint64>,
);
typedef _CopyFrameDart = int Function(
  Pointer<Uint8>,
  int,
  Pointer<Uint32>,
  Pointer<Uint32>,
  Pointer<Uint64>,
);

/// Typed Dart façade over the existing Rust Windows Host/Viewer engine.
/// Secrets are passed only into the Rust process boundary and persisted there
/// with DPAPI; the UI receives display-safe snapshots and JPEG frames.
class P2PRemoteController extends ChangeNotifier {
  P2PRemoteController._();

  static final P2PRemoteController instance = P2PRemoteController._();

  DynamicLibrary? _library;
  _CommandDart? _command;
  _PollDart? _pollEvent;
  _FreeStringDart? _freeString;
  _SizeDart? _frameSize;
  _VersionDart? _frameVersion;
  _CopyFrameDart? _copyFrame;
  Timer? _eventTimer;
  Timer? _frameTimer;
  bool _initialized = false;
  bool _initializing = false;
  bool bridgeAvailable = false;
  String bridgeError = '原生桥接尚未初始化';
  Map<String, dynamic> _snapshot = <String, dynamic>{};
  Uint8List? frame;
  int frameWidth = 0;
  int frameHeight = 0;
  int _lastFrameVersion = 0;
  String lastEvent = '';

  bool get hostRunning => _snapshot['host_running'] == true;
  bool get viewerConnected => _snapshot['viewer_connected'] == true;
  bool get pairingActive => _snapshot['pairing_active'] == true;
  bool get tokenConfigured => _snapshot['token_configured'] == true;
  bool get allowInput => _snapshot['allow_input'] == true;
  bool get allowClipboard => _snapshot['allow_clipboard'] == true;
  String get relayUrl => (_snapshot['relay_url'] as String?) ?? 'wss://p2ptest.aikissgo.com';
  String get hostStatus => (_snapshot['host_status'] as String?) ?? '未启动';
  String get viewerStatus => (_snapshot['viewer_status'] as String?) ?? '未连接';
  String get deviceNumber => (_snapshot['device_number'] as String?) ?? '';
  String get pairingCode => (_snapshot['pairing_code'] as String?) ?? '';
  int get pairingExpiresAt => (_snapshot['pairing_expires_at'] as num?)?.toInt() ?? 0;
  String? get pendingRequestId => _snapshot['pending_pairing_request_id'] as String?;
  String? get pendingViewerFingerprint => _snapshot['pending_viewer_fingerprint'] as String?;
  Map<String, dynamic> get snapshot => Map<String, dynamic>.unmodifiable(_snapshot);

  int get monitorCount {
    final value = _snapshot['monitors'];
    if (value is Map<String, dynamic>) return (value['count'] as num?)?.toInt() ?? 1;
    return 1;
  }

  int get selectedMonitor {
    final value = _snapshot['monitors'];
    if (value is Map<String, dynamic>) return (value['selected'] as num?)?.toInt() ?? 0;
    return 0;
  }

  String get clipboardText => (_snapshot['clipboard_text'] as String?) ?? '';

  Future<void> initialize() async {
    if (_initialized || _initializing) return;
    _initializing = true;
    try {
      if (!Platform.isWindows) {
        bridgeError = Platform.isAndroid
            ? 'Android 原生会话桥接正在接入；当前保留原生 Android 远控客户端。'
            : '当前平台尚未提供原生远控桥接。';
        bridgeAvailable = false;
        _initialized = true;
        notifyListeners();
        return;
      }
      final library = DynamicLibrary.open('p2p_remote.dll');
      final init = library.lookupFunction<_InitNative, _InitDart>('p2p_bridge_init');
      _command = library.lookupFunction<_CommandNative, _CommandDart>('p2p_bridge_command_json');
      _pollEvent = library.lookupFunction<_PollNative, _PollDart>('p2p_bridge_poll_event');
      _freeString = library.lookupFunction<_FreeStringNative, _FreeStringDart>('p2p_bridge_free_string');
      _frameSize = library.lookupFunction<_SizeNative, _SizeDart>('p2p_bridge_latest_frame_size');
      _frameVersion = library.lookupFunction<_VersionNative, _VersionDart>('p2p_bridge_latest_frame_version');
      _copyFrame = library.lookupFunction<_CopyFrameNative, _CopyFrameDart>('p2p_bridge_copy_latest_frame');
      if (init() != 1) throw StateError('Rust native bridge initialization failed');
      _library = library;
      bridgeAvailable = true;
      bridgeError = '';
      _initialized = true;
      await refreshSnapshot();
      _eventTimer = Timer.periodic(const Duration(milliseconds: 60), (_) => _drainEvents());
      _frameTimer = Timer.periodic(const Duration(milliseconds: 33), (_) => _readLatestFrame());
    } catch (error) {
      bridgeAvailable = false;
      bridgeError = '无法加载 Windows Rust 远控核心：$error';
      _initialized = true;
    } finally {
      _initializing = false;
      notifyListeners();
    }
  }

  Future<Map<String, dynamic>> _invoke(Map<String, Object?> command) async {
    if (!bridgeAvailable || _command == null || _freeString == null) {
      return <String, dynamic>{'ok': false, 'error': bridgeError};
    }
    final input = jsonEncode(command).toNativeUtf8();
    try {
      final output = _command!(input);
      if (output == nullptr) return <String, dynamic>{'ok': false, 'error': 'Rust bridge returned a null response'};
      try {
        final decoded = jsonDecode(output.toDartString());
        if (decoded is! Map<String, dynamic>) {
          return <String, dynamic>{'ok': false, 'error': 'Invalid native response'};
        }
        final snapshot = decoded['snapshot'];
        if (snapshot is Map<String, dynamic>) _snapshot = snapshot;
        if (decoded['ok'] != true) {
          lastEvent = (decoded['error'] as String?) ?? 'Native command failed';
        }
        notifyListeners();
        return decoded;
      } finally {
        _freeString!(output);
      }
    } finally {
      malloc.free(input);
    }
  }

  Future<void> refreshSnapshot() async {
    final response = await _invoke(const <String, Object?>{'method': 'snapshot'});
    if (response['ok'] != true && bridgeAvailable) {
      lastEvent = (response['error'] as String?) ?? '无法读取原生状态';
    }
  }

  Future<bool> configureRelay(String server, String token) async =>
      (await _invoke(<String, Object?>{
        'method': 'configureRelay',
        'server': server.trim(),
        'token': token.trim(),
      }))['ok'] == true;

  Future<bool> startHost() async =>
      (await _invoke(const <String, Object?>{'method': 'startHost'}))['ok'] == true;

  Future<bool> stopHost() async =>
      (await _invoke(const <String, Object?>{'method': 'stopHost'}))['ok'] == true;

  Future<bool> setRemoteInputAllowed(bool allowed) async =>
      (await _invoke(<String, Object?>{
        'method': 'setRemoteInputAllowed',
        'allowed': allowed,
      }))['ok'] == true;

  Future<bool> setClipboardAllowed(bool allowed) async =>
      (await _invoke(<String, Object?>{
        'method': 'setClipboardAllowed',
        'allowed': allowed,
      }))['ok'] == true;

  Future<bool> approvePairing(String requestId) async =>
      (await _invoke(<String, Object?>{
        'method': 'approvePairingRequest',
        'requestId': requestId,
      }))['ok'] == true;

  Future<bool> denyPairing(String requestId) async =>
      (await _invoke(<String, Object?>{
        'method': 'denyPairingRequest',
        'requestId': requestId,
      }))['ok'] == true;

  Future<bool> requestPairing(String deviceNumber, String code) async =>
      (await _invoke(<String, Object?>{
        'method': 'requestPairing',
        'deviceNumber': deviceNumber,
        'pairingCode': code,
      }))['ok'] == true;

  Future<bool> cancelPairing() async =>
      (await _invoke(const <String, Object?>{'method': 'cancelPairing'}))['ok'] == true;

  Future<bool> connectLegacy(String sessionKey) async =>
      (await _invoke(<String, Object?>{
        'method': 'connectLegacy',
        'sessionKey': sessionKey,
      }))['ok'] == true;

  Future<bool> disconnectViewer() async =>
      (await _invoke(const <String, Object?>{'method': 'disconnectViewer'}))['ok'] == true;

  Future<bool> sendPointer(String kind, double x, double y) async =>
      (await _invoke(<String, Object?>{
        'method': 'sendPointer',
        'kind': kind,
        'x': x.clamp(0.0, 1.0),
        'y': y.clamp(0.0, 1.0),
      }))['ok'] == true;

  Future<bool> sendScroll(int delta) async =>
      (await _invoke(<String, Object?>{'method': 'sendScroll', 'delta': delta}))['ok'] == true;

  Future<bool> sendText(String text) async =>
      (await _invoke(<String, Object?>{'method': 'sendText', 'text': text}))['ok'] == true;

  Future<bool> sendKey(String key) async =>
      (await _invoke(<String, Object?>{'method': 'sendKey', 'key': key}))['ok'] == true;

  Future<bool> selectMonitor(int index) async =>
      (await _invoke(<String, Object?>{'method': 'selectMonitor', 'index': index}))['ok'] == true;

  Future<bool> getClipboardText() async =>
      (await _invoke(const <String, Object?>{'method': 'getClipboardText'}))['ok'] == true;

  Future<bool> setClipboardText(String text) async =>
      (await _invoke(<String, Object?>{'method': 'setClipboardText', 'text': text}))['ok'] == true;

  void _drainEvents() {
    final poll = _pollEvent;
    final free = _freeString;
    if (!bridgeAvailable || poll == null || free == null) return;
    for (var i = 0; i < 64; i++) {
      final pointer = poll();
      if (pointer == nullptr) break;
      try {
        final raw = pointer.toDartString();
        final event = jsonDecode(raw);
        if (event is Map<String, dynamic>) {
          lastEvent = (event['message'] as String?) ?? (event['type'] as String?) ?? raw;
        } else {
          lastEvent = raw;
        }
      } catch (_) {
        // Malformed status events are ignored; the command/snapshot path is authoritative.
      } finally {
        free(pointer);
      }
    }
    if (lastEvent.isNotEmpty) {
      unawaited(refreshSnapshot());
    }
  }

  void _readLatestFrame() {
    if (!bridgeAvailable || _frameSize == null || _frameVersion == null || _copyFrame == null) return;
    try {
      final version = _frameVersion!();
      if (version == 0 || version == _lastFrameVersion) return;
      final size = _frameSize!();
      if (size <= 0 || size > 3 * 1024 * 1024) return;
      final bytes = calloc<Uint8>(size);
      final width = calloc<Uint32>();
      final height = calloc<Uint32>();
      final copiedVersion = calloc<Uint64>();
      try {
        final copied = _copyFrame!(bytes, size, width, height, copiedVersion);
        if (copied <= 0 || copied > size || copiedVersion.value == _lastFrameVersion) return;
        frame = Uint8List.fromList(bytes.asTypedList(copied));
        frameWidth = width.value;
        frameHeight = height.value;
        _lastFrameVersion = copiedVersion.value;
        notifyListeners();
      } finally {
        calloc.free(bytes);
        calloc.free(width);
        calloc.free(height);
        calloc.free(copiedVersion);
      }
    } catch (error) {
      lastEvent = '读取远程画面失败：$error';
      notifyListeners();
    }
  }

  @override
  void dispose() {
    _eventTimer?.cancel();
    _frameTimer?.cancel();
    super.dispose();
  }
}
