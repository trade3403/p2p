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
}
