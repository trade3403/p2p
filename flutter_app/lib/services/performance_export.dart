import 'dart:convert';

/// Exports only explicitly approved scalar diagnostics, never the raw snapshot.
/// Missing or invalid data remains null; zero is a measured/reported zero.
String exportPerformanceSnapshot(Map<String, dynamic> snapshot,
    {required DateTime capturedAt}) {
  const numeric = [
    'avc_rendered_frames', 'rendered_fps', 'measured_video_bps', 'rtt_ms',
    'relay_socket_opens', 'relay_retry_events', 'video_target_bps',
    'video_target_fps', 'avc_decoder_drops', 'capture_avg_us',
    'encode_avg_us', 'send_avg_us', 'fresh_frames', 'reused_frames',
    'resize_cache_hits', 'yuv_cache_hits', 'host_encoded_frames',
    'lan_attempt_ms', 'lan_candidate_attempts', 'network_change_events',
    'network_retry_wakes', 'lan_fallback_relay_open_ms',
    'lan_fallback_first_video_ms', 'viewer_lan_video_bytes',
    'viewer_relay_video_bytes', 'host_lan_video_bytes',
    'host_relay_video_bytes', 'host_relay_suppressed_frames',
    'host_lan_video_frames', 'host_relay_video_frames',
    'host_relay_socket_opens',
  ];
  const flags = [
    'viewer_connected', 'host_metadata_current', 'network_observer_ready',
    'lan_fallback_started', 'host_relay_socket_open', 'avc_decoder_hardware',
  ];
  const enums = <String, List<String>>{
    'active_video_codec': ['none', 'JPEG', 'H264'],
    'viewer_transport_phase': [
      'idle', 'lan_verifying', 'lan_verified', 'lan_video', 'relay_connecting',
      'relay_open', 'relay_video', 'relay_retry', 'udp_video', 'failed',
    ],
    'lan_fallback_reason': [
      'none', 'invalid_endpoint', 'connect_or_verify_timeout',
      'connect_or_verify_failure', 'closed_before_verified', 'peer_closed',
      'transport_failure', 'video_watchdog', 'lan_heartbeat_timeout',
      'lan_heartbeat_send_failed', 'lan_subnet_changed',
    ],
  };
  final metrics = <String, Object?>{};
  for (final key in numeric) {
    final value = snapshot[key];
    metrics[key] = value is num && value.isFinite && value >= 0 ? value : null;
  }
  for (final key in flags) {
    final value = snapshot[key];
    metrics[key] = value is bool ? value : null;
  }
  for (final entry in enums.entries) {
    final value = snapshot[entry.key];
    metrics[entry.key] = entry.value.contains(value) ? value : null;
  }
  return const JsonEncoder.withIndent('  ').convert({
    'schema_version': 1,
    'captured_at_utc': capturedAt.toUtc().toIso8601String(),
    'sample_kind': 'viewer_snapshot',
    'metrics': metrics,
  });
}

 
/// Local Host counters only. A stopped Host may retain stale bridge fields,
/// so those fields are unavailable unless the native running flag is true.
String exportHostPerformanceSnapshot(Map<String, dynamic> snapshot,
    {required DateTime capturedAt}) {
  final running = snapshot['host_running'];
  final metrics = <String, Object?>{
    'host_running': running is bool ? running : null,
  };
  for (final key in [
    'host_lan_video_bytes', 'host_relay_video_bytes',
    'host_lan_video_frames', 'host_relay_video_frames',
    'host_relay_suppressed_frames', 'host_relay_socket_opens',
  ]) {
    final value = snapshot[key];
    metrics[key] = running == true && value is int && value >= 0 ? value : null;
  }
  for (final key in ['host_lan_active', 'host_relay_socket_open']) {
    final value = snapshot[key];
    metrics[key] = running == true && value is bool ? value : null;
  }
  return const JsonEncoder.withIndent('  ').convert({
    'schema_version': 1,
    'captured_at_utc': capturedAt.toUtc().toIso8601String(),
    'sample_kind': 'host_snapshot',
    'metrics': metrics,
  });
}
