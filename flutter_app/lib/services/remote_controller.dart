import 'dart:async';
import 'dart:convert';
import 'dart:ffi';
import 'dart:io';

import 'package:ffi/ffi.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

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
  static const MethodChannel _androidBridge = MethodChannel('com.p2p.remote/bridge');
  static const EventChannel _androidEvents = EventChannel('com.p2p.remote/events');

  P2PRemoteController._();

  static final P2PRemoteController instance = P2PRemoteController._();

  // Retain the native library handle for the lifetime of the FFI function pointers.
  // ignore: unused_field
  DynamicLibrary? _library;
  _CommandDart? _command;
  _PollDart? _pollEvent;
  _FreeStringDart? _freeString;
  _SizeDart? _frameSize;
  _VersionDart? _frameVersion;
  _CopyFrameDart? _copyFrame;
  Timer? _eventTimer;
  Timer? _frameTimer;
  StreamSubscription<dynamic>? _androidEventSubscription;
  int? videoTextureId;
  bool h264Active = false;
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

  bool get isAndroid => Platform.isAndroid;
  bool get hostRunning => _snapshot['host_running'] == true;
  bool get viewerConnected => _snapshot['viewer_connected'] == true;
  bool get viewerRequested => _snapshot['viewer_requested'] == true;
  bool get viewerAllowsInput => _snapshot['viewer_allow_input'] == true;
  bool get viewerAllowsClipboard => _snapshot['viewer_allow_clipboard'] == true;
  String get transport => (_snapshot['transport'] as String?) ?? (viewerConnected ? 'WSS 中转' : '未连接');
  String get transportPhaseLabel {
    switch (_snapshot['viewer_transport_phase']) {
      case 'lan_verifying': return 'LAN 连接 / 加密验证中';
      case 'lan_verified': return 'LAN 已验证，等待画面';
      case 'lan_video': return 'LAN 正在接收画面';
      case 'relay_connecting': return 'VPS WSS 连接中';
      case 'relay_open': return 'VPS WSS 已连接，等待画面';
      case 'relay_video': return 'VPS 正在接收画面';
      case 'relay_retry': return 'VPS 等待重连';
      case 'udp_video': return 'UDP 预览，控制走 WSS';
      case 'failed': return '连接失败 / 已停止';
      case 'idle': return '未连接';
      default: return '待测';
    }
  }
  String get lanFallbackReasonLabel {
    switch (_snapshot['lan_fallback_reason']) {
      case 'invalid_endpoint': return '连接地址无效';
      case 'connect_or_verify_timeout': return 'LAN 连接或验证超时';
      case 'connect_or_verify_failure': return 'LAN 连接或验证失败';
      case 'closed_before_verified': return 'LAN 验证前关闭';
      case 'peer_closed': return 'LAN 对端关闭';
      case 'transport_failure': return 'LAN 传输中断';
      case 'video_watchdog': return 'LAN 画面更新超时';
      case 'lan_heartbeat_timeout': return 'LAN 加密心跳超时';
      case 'lan_heartbeat_send_failed': return 'LAN 加密心跳发送失败';
      case 'lan_subnet_changed': return 'LAN 本地网段变化';
      default: return '未记录';
    }
  }
  int? get rttMs => (_snapshot['rtt_ms'] as num?)?.toInt();
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
    if (value is Map) return (value['count'] as num?)?.toInt() ?? 1;
    return 1;
  }

  int get selectedMonitor {
    final value = _snapshot['monitors'];
    if (value is Map) return (value['selected'] as num?)?.toInt() ?? 0;
    return 0;
  }

  String get clipboardText => (_snapshot['clipboard_text'] as String?) ?? '';
  String get udpReport => (_snapshot['udp_report'] as String?) ?? '尚未执行公网 UDP 双端验证';
  String get stunReport => (_snapshot['stun_report'] as String?) ?? 'STUN UDP 尚未探测';
  bool get udpAttempted => _snapshot['udp_attempted'] == true;

  Future<void> initialize() async {
    if (_initialized || _initializing) return;
    _initializing = true;
    try {
      if (Platform.isAndroid) {
        _androidEventSubscription = _androidEvents.receiveBroadcastStream().listen(
          _handleAndroidEvent,
          onError: (Object error) {
            bridgeError = 'Android 原生事件通道异常：$error';
            notifyListeners();
          },
        );
        final response = await _androidBridge.invokeMapMethod<String, dynamic>('snapshot');
        if (response == null) throw StateError('Android 原生桥接没有返回状态');
        _applyResponse(response);
        bridgeAvailable = true;
        bridgeError = '';
        _initialized = true;
        notifyListeners();
        return;
      }
      if (!Platform.isWindows) {
        bridgeError = '当前平台尚未提供原生远控桥接。';
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
      bridgeError = Platform.isAndroid
          ? '无法初始化 Android Kotlin 远控桥接：$error'
          : '无法加载 Windows Rust 远控核心：$error';
      _initialized = true;
    } finally {
      _initializing = false;
      notifyListeners();
    }
  }

  Future<Map<String, dynamic>> _invoke(Map<String, Object?> command) async {
    if (Platform.isAndroid) {
      try {
        final method = command['method'] as String;
        final arguments = Map<String, Object?>.from(command)..remove('method');
        final decoded = await _androidBridge.invokeMapMethod<String, dynamic>(method, arguments);
        if (decoded == null) {
          return <String, dynamic>{'ok': false, 'error': 'Android bridge returned a null response'};
        }
        _applyResponse(decoded);
        return decoded;
      } catch (error) {
        lastEvent = 'Android 原生桥接调用失败：$error';
        notifyListeners();
        return <String, dynamic>{'ok': false, 'error': lastEvent};
      }
    }
    if (!bridgeAvailable || _command == null || _freeString == null) {
      lastEvent = bridgeError;
      notifyListeners();
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
        _applyResponse(decoded);
        return decoded;
      } finally {
        _freeString!(output);
      }
    } finally {
      malloc.free(input);
    }
  }

  void _applyResponse(Map<String, dynamic> decoded) {
    final snapshot = decoded['snapshot'];
    if (snapshot is Map) {
      _snapshot = Map<String, dynamic>.from(snapshot);
      final texture = _snapshot['texture_id'];
      videoTextureId = texture is num ? texture.toInt() : null;
      h264Active = _snapshot['h264_active'] == true;
      if (_snapshot['viewer_connected'] != true) {
        frame = null;
        frameWidth = 0;
        frameHeight = 0;
        _lastFrameVersion = 0;
      } else {
        frameWidth = (_snapshot['frame_width'] as num?)?.toInt() ?? frameWidth;
        frameHeight = (_snapshot['frame_height'] as num?)?.toInt() ?? frameHeight;
      }
    }
    if (decoded['ok'] != true) {
      lastEvent = (decoded['error'] as String?) ?? 'Native command failed';
    }
    notifyListeners();
  }

  void _handleAndroidEvent(dynamic raw) {
    if (raw is! Map) return;
    final type = raw['type'];
    if (type == 'snapshot') {
      final snapshot = raw['snapshot'];
      if (snapshot is Map) {
        _snapshot = Map<String, dynamic>.from(snapshot);
        final texture = raw['texture_id'] ?? _snapshot['texture_id'];
        videoTextureId = texture is num ? texture.toInt() : null;
        h264Active = _snapshot['h264_active'] == true;
        if (_snapshot['viewer_connected'] != true) {
          frame = null;
          frameWidth = 0;
          frameHeight = 0;
        } else {
          frameWidth = (_snapshot['frame_width'] as num?)?.toInt() ?? frameWidth;
          frameHeight = (_snapshot['frame_height'] as num?)?.toInt() ?? frameHeight;
        }
      }
      notifyListeners();
    } else if (type == 'frame') {
      final data = raw['frame'];
      if (data is Uint8List) {
        frame = data;
      } else if (data is List<int>) {
        frame = Uint8List.fromList(data);
      }
      frameWidth = (raw['width'] as num?)?.toInt() ?? frameWidth;
      frameHeight = (raw['height'] as num?)?.toInt() ?? frameHeight;
      h264Active = false;
      notifyListeners();
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

  Future<bool> probeStun() async =>
      (await _invoke(<String, Object?>{
        'method': 'probeSTUN',
        'server': relayUrl,
      }))['ok'] == true;

  Future<bool> startUdpPunch() async =>
      (await _invoke(const <String, Object?>{'method': 'startUdpPunch'}))['ok'] == true;

  Future<bool> getClipboardText() async =>
      (await _invoke(const <String, Object?>{'method': 'getClipboardText'}))['ok'] == true;

  Future<bool> setClipboardText(String text) async =>
      (await _invoke(<String, Object?>{'method': 'setClipboardText', 'text': text}))['ok'] == true;

  void _drainEvents() {
    final poll = _pollEvent;
    final free = _freeString;
    if (!bridgeAvailable || poll == null || free == null) return;
    var changed = false;
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
        changed = true;
      } catch (_) {
        // Malformed status events are ignored; the command/snapshot path is authoritative.
      } finally {
        free(pointer);
      }
    }
    if (changed) {
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
    _androidEventSubscription?.cancel();
    super.dispose();
  }
}
