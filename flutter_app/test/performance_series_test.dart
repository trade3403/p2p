import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:p2p_remote_ui/services/performance_series.dart';

void main() {
  final now = DateTime.utc(2026, 1, 1);
  test('series keeps actual elapsed time and excludes private fields', () {
    final series = PerformanceSeries(host: false, startedAt: now);
    expect(series.add({'rendered_fps': 9.9, 'pairing_code': 'PRIVATE_MARKER'},
        elapsedMs: 17, capturedAt: now), true);
    expect(series.add({}, elapsedMs: 17, capturedAt: now), false);
    expect(series.add({'viewer_relay_video_bytes': 0},
        elapsedMs: 2410, capturedAt: now), true);
    series.stop('user_stop');
    expect(series.add({}, elapsedMs: 5000, capturedAt: now), false);
    final report = jsonDecode(series.export()) as Map;
    final samples = report['samples'] as List;
    expect(report['sample_kind'], 'viewer_series');
    expect(report['stop_reason'], 'user_stop');
    expect(samples.length, 2);
    expect(samples[0]['elapsed_ms'], 17);
    expect(samples[1]['elapsed_ms'], 2410);
    expect(samples[1]['metrics']['viewer_relay_video_bytes'], 0);
    expect(series.export(), isNot(contains('PRIVATE_MARKER')));
  });

  test('host series is bounded and never overwrites stop reason', () {
    final series = PerformanceSeries(host: true, startedAt: now);
    for (var i = 0; i < PerformanceSeries.maxSamples; i++) {
      expect(series.add({'host_running': true, 'host_lan_video_bytes': i},
          elapsedMs: i * 2000, capturedAt: now), true);
    }
    expect(series.active, false);
    expect(series.stopReason, 'sample_limit');
    expect(series.add({}, elapsedMs: 999999, capturedAt: now), false);
    series.stop('user_stop');
    expect(series.stopReason, 'sample_limit');
    expect(series.count, PerformanceSeries.maxSamples);
  });

  test('host interval rates use actual elapsed time and observed LAN endpoints', () {
    final series = PerformanceSeries(host: true, startedAt: now);
    Map<String, dynamic> sample(int lan, int relay) => {
      'host_running': true, 'host_lan_active': true,
      'host_relay_socket_open': true,
      'host_lan_video_bytes': lan, 'host_relay_video_bytes': relay,
    };
    series.add(sample(100, 50), elapsedMs: 10, capturedAt: now);
    series.add(sample(1100, 50), elapsedMs: 2510, capturedAt: now);
    series.add(sample(2100, 100), elapsedMs: 4510, capturedAt: now);
    series.add(sample(10, 0), elapsedMs: 6510, capturedAt: now);
    series.add(sample(100, 0), elapsedMs: 12510, capturedAt: now);
    final report = jsonDecode(series.export()) as Map;
    final analysis = report['analysis'] as Map;
    final intervals = analysis['intervals'] as List;
    expect(intervals[0]['duration_ms'], 2500);
    expect(intervals[0]['rates_bps_or_count_per_second']['host_lan_video_bytes'], 3200);
    expect(intervals[0]['counter_deltas']['host_relay_video_bytes'], 0);
    expect(intervals[2]['decreased_counters'], contains('host_lan_video_bytes'));
    expect(intervals[2]['counter_deltas']['host_lan_video_bytes'], isNull);
    expect(intervals[3]['long_sampling_gap'], true);
    expect(analysis['lan_endpoint_observed_intervals'], 2);
    expect(analysis['lan_endpoint_observed_zero_relay_intervals'], 1);
    expect(analysis['lan_endpoint_observed_nonzero_relay_intervals'], 1);
    expect(analysis['connected_rtt_mean_ms'], isNull);
  });

  test('viewer transport changes and missing metrics are not LAN evidence', () {
    final series = PerformanceSeries(host: false, startedAt: now);
    series.add({
      'viewer_connected': true, 'viewer_transport_phase': 'lan_video',
      'viewer_lan_video_bytes': 100, 'viewer_relay_video_bytes': 0,
      'viewer_lan_video_frames': 5, 'avc_rendered_frames': 10, 'rtt_ms': 10,
    }, elapsedMs: 0, capturedAt: now);
    series.add({
      'viewer_connected': true, 'viewer_transport_phase': 'relay_video',
      'viewer_lan_video_bytes': 200, 'viewer_lan_video_frames': 9,
      'avc_rendered_frames': 20, 'rtt_ms': 30,
    }, elapsedMs: 2000, capturedAt: now);
    series.add({'viewer_connected': false, 'rtt_ms': 1000},
        elapsedMs: 4000, capturedAt: now);
    final analysis = (jsonDecode(series.export()) as Map)['analysis'] as Map;
    final intervals = analysis['intervals'] as List;
    expect(intervals[0]['same_observed_transport'], false);
    expect(intervals[0]['counter_deltas']['viewer_relay_video_bytes'], isNull);
    expect(intervals[0]['rates_bps_or_count_per_second']['viewer_lan_video_frames'], 2);
    expect(intervals[0]['rates_bps_or_count_per_second']['avc_rendered_frames'], 5);
    expect(analysis['lan_endpoint_observed_intervals'], 0);
    expect(analysis['connected_rtt_sample_count'], 2);
    expect(analysis['connected_rtt_min_ms'], 10);
    expect(analysis['connected_rtt_mean_ms'], 20);
    expect(analysis['connected_rtt_max_ms'], 30);
  });

}
