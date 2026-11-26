import 'dart:convert';

import 'performance_export.dart';

/// Bounded, explicitly started diagnostic samples, never raw bridge snapshots.
class PerformanceSeries {
  PerformanceSeries({required this.host, required this.startedAt});

  static const maxSamples = 300;
  static const intervalMs = 2000;
  final bool host;
  final DateTime startedAt;
  final _samples = <Map<String, dynamic>>[];
  String? _stopReason;

  int get count => _samples.length;
  bool get active => _stopReason == null;
  String? get stopReason => _stopReason;

  bool add(Map<String, dynamic> snapshot, {
    required int elapsedMs, required DateTime capturedAt,
  }) {
    if (!active) return false;
    if (elapsedMs < 0 ||
        (_samples.isNotEmpty && elapsedMs <= (_samples.last['elapsed_ms'] as int))) {
      return false;
    }
    final encoded = host
        ? exportHostPerformanceSnapshot(snapshot, capturedAt: capturedAt)
        : exportPerformanceSnapshot(snapshot, capturedAt: capturedAt);
    final sample = jsonDecode(encoded) as Map<String, dynamic>;
    sample['elapsed_ms'] = elapsedMs;
    _samples.add(sample);
    if (count >= maxSamples) stop('sample_limit');
    return true;
  }

  void stop(String reason) {
    _stopReason ??= reason;
  }

  String export() => const JsonEncoder.withIndent('  ').convert({
    'schema_version': 1,
    'sample_kind': host ? 'host_series' : 'viewer_series',
    'started_at_utc': startedAt.toUtc().toIso8601String(),
    'nominal_interval_ms': intervalMs,
    'sample_limit': maxSamples,
    'stop_reason': _stopReason,
    'samples': _samples,
  });
}
