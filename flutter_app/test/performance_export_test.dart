import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:p2p_remote_ui/services/performance_export.dart';

void main() {
  test('exports actual scalar values without inventing missing measurements', () {
    final result = jsonDecode(exportPerformanceSnapshot({
      'rendered_fps': 9.99,
      'viewer_lan_video_bytes': 0,
      'rtt_ms': -1,
      'measured_video_bps': double.nan,
      'host_metadata_current': false,
      'avc_decoder_recovering': true,
      'active_video_codec': 'H264',
      'viewer_transport_phase': 'lan_video',
    }, capturedAt: DateTime.utc(2026, 1, 1))) as Map;
    final metrics = result['metrics'] as Map;
    expect(result['schema_version'], 1);
    expect(result['captured_at_utc'], '2026-01-01T00:00:00.000Z');
    expect(metrics['rendered_fps'], 9.99);
    expect(metrics['viewer_lan_video_bytes'], 0);
    expect(metrics['rtt_ms'], isNull);
    expect(metrics['measured_video_bps'], isNull);
    expect(metrics['host_relay_video_bytes'], isNull);
    expect(metrics['host_metadata_current'], false);
    expect(metrics['avc_decoder_recovering'], true);
    expect(metrics['active_video_codec'], 'H264');
    expect(metrics['viewer_transport_phase'], 'lan_video');
  });

  test('excludes arbitrary strings, credentials, clipboard and nested data', () {
    final result = exportPerformanceSnapshot({
      'pairing_code': 'SECRET_MARKER',
      'clipboard_text': 'SECRET_MARKER',
      'relay_token': 'SECRET_MARKER',
      'device_number': 'SECRET_MARKER',
      'video_encoder': 'SECRET_MARKER',
      'active_video_codec': 'SECRET_MARKER',
      'lan_fallback_reason': 'SECRET_MARKER',
      'rtt_ms': {'secret': 'SECRET_MARKER'},
    }, capturedAt: DateTime.utc(2026));
    expect(result, isNot(contains('SECRET_MARKER')));
    final metrics = (jsonDecode(result) as Map)['metrics'] as Map;
    expect(metrics['active_video_codec'], isNull);
    expect(metrics['lan_fallback_reason'], isNull);
    expect(metrics['rtt_ms'], isNull);
    expect(metrics.containsKey('pairing_code'), false);
  });

  test('host export uses only local counters and keeps zero distinct from missing', () {
    final result = jsonDecode(exportHostPerformanceSnapshot({
      'host_running': true,
      'host_lan_active': true,
      'host_relay_socket_open': true,
      'host_lan_video_bytes': 12345,
      'host_relay_video_bytes': 0,
      'host_relay_suppressed_frames': 12,
      'host_lan_video_frames': -1,
      'host_relay_socket_opens': 1.5,
      'rendered_fps': 30,
      'relay_token': 'PRIVATE_MARKER',
    }, capturedAt: DateTime.utc(2026))) as Map;
    final metrics = result['metrics'] as Map;
    expect(result['sample_kind'], 'host_snapshot');
    expect(metrics['host_lan_video_bytes'], 12345);
    expect(metrics['host_relay_video_bytes'], 0);
    expect(metrics['host_lan_active'], true);
    expect(metrics['host_relay_socket_open'], true);
    expect(metrics['host_relay_suppressed_frames'], 12);
    expect(metrics['host_lan_video_frames'], isNull);
    expect(metrics['host_relay_socket_opens'], isNull);
    expect(metrics.containsKey('rendered_fps'), false);
    expect(result.toString(), isNot(contains('PRIVATE_MARKER')));
  });

  test('resource and actual backend exports use safe fields and preserve unknown', () {
    final metrics = (jsonDecode(exportPerformanceSnapshot({
      'viewer_requested': true, 'viewer_connected': true,
      'host_metadata_current': true, 'active_video_codec': 'H264',
      'video_encoder': 'Media Foundation hardware', 'capture_backend': 'DXGI',
      'avc_decoder_hardware': false, 'avc_decoder_recovering': false,
      'process_cpu_ms': 123, 'process_rss_bytes': 4096,
      'process_gpu_utilization_percent': 42, 'avc_decoder': 'PRIVATE_MARKER',
    }, capturedAt: DateTime.utc(2026))) as Map)['metrics'] as Map;
    expect(metrics['process_cpu_ms'], 123);
    expect(metrics['process_rss_bytes'], 4096);
    expect(metrics['process_gpu_utilization_percent'], isNull);
    expect(metrics['video_encoder'], 'Media Foundation hardware');
    expect(metrics['capture_backend'], 'DXGI');
    expect(metrics['decoder_backend_class'], 'software');
    expect(metrics.toString(), isNot(contains('PRIVATE_MARKER')));
    final missing = (jsonDecode(exportPerformanceSnapshot({
      'viewer_requested': true, 'viewer_connected': true,
      'host_metadata_current': false, 'active_video_codec': 'H264',
      'video_encoder': 'OpenH264 software', 'avc_decoder_hardware': true,
      'avc_decoder_recovering': true, 'process_cpu_ms': -1, 'process_rss_bytes': 0,
    }, capturedAt: DateTime.utc(2026))) as Map)['metrics'] as Map;
    for (final key in ['video_encoder', 'decoder_backend_class',
      'process_cpu_ms', 'process_rss_bytes']) {
      expect(missing[key], isNull);
    }
  });

  test('stopped or unknown host never exports stale counters', () {
    for (final running in [false, null]) {
      final result = jsonDecode(exportHostPerformanceSnapshot({
        'host_running': running,
        'host_lan_active': true,
        'host_lan_video_bytes': 200,
        'process_cpu_ms': 100,
        'process_rss_bytes': 4096,
      }, capturedAt: DateTime.utc(2026))) as Map;
      final metrics = result['metrics'] as Map;
      expect(metrics['host_running'], running);
      expect(metrics['host_lan_active'], isNull);
      expect(metrics['host_lan_video_bytes'], isNull);
      expect(metrics['process_cpu_ms'], isNull);
      expect(metrics['process_rss_bytes'], isNull);
    }
  });
}
