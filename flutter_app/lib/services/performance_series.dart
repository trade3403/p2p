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
    'analysis': _analysis(),
  });

  String _phase(Map metrics) {
    if (host) {
      if (metrics['host_running'] != true) return 'unknown';
      if (metrics['host_lan_active'] == true) return 'lan';
      if (metrics['host_lan_active'] == false &&
          metrics['host_relay_socket_open'] == true) {
        return 'relay';
      }
      return 'unknown';
    }
    if (metrics['viewer_connected'] != true) return 'unknown';
    switch (metrics['viewer_transport_phase']) {
      case 'lan_video': return 'lan';
      case 'relay_video': return 'relay';
      default: return 'unknown';
    }
  }

  Map<String, Object?> _analysis() {
    final prefix = host ? 'host' : 'viewer';
    final counters = [
      '${prefix}_lan_video_bytes', '${prefix}_relay_video_bytes',
      '${prefix}_lan_video_frames', '${prefix}_relay_video_frames',
      if (host) 'host_relay_suppressed_frames',
      if (!host) 'avc_rendered_frames',
    ];
    final intervals = <Map<String, Object?>>[];
    final rtts = <num>[];
    var lanObserved = 0;
    var lanZeroRelay = 0;
    var lanNonzeroRelay = 0;
    for (final sample in _samples) {
      final metrics = sample['metrics'] as Map;
      final rtt = metrics['rtt_ms'];
      if (!host && metrics['viewer_connected'] == true &&
          rtt is num && rtt.isFinite && rtt >= 0) {
        rtts.add(rtt);
      }
    }
    for (var i = 1; i < _samples.length; i++) {
      final before = _samples[i - 1];
      final after = _samples[i];
      final a = before['metrics'] as Map;
      final b = after['metrics'] as Map;
      final elapsed = (after['elapsed_ms'] as int) - (before['elapsed_ms'] as int);
      final gap = elapsed > intervalMs * 2;
      final startPhase = _phase(a);
      final endPhase = _phase(b);
      final sameObservedPhase = startPhase == endPhase && startPhase != 'unknown';
      final deltas = <String, int?>{};
      final rates = <String, double?>{};
      final resets = <String>[];
      for (final key in counters) {
        final first = a[key];
        final last = b[key];
        int? delta;
        if (first is int && last is int && first >= 0 && last >= 0) {
          if (last >= first) {
            delta = last - first;
          } else {
            resets.add(key);
          }
        }
        deltas[key] = delta;
        if (key.endsWith('_bytes')) {
          rates[key] = delta == null ? null : delta * 8000.0 / elapsed;
        } else {
          rates[key] = delta == null ? null : delta * 1000.0 / elapsed;
        }
      }
      final lanBytes = deltas['${prefix}_lan_video_bytes'];
      final relayBytes = deltas['${prefix}_relay_video_bytes'];
      // Endpoint observations only, never proof of uninterrupted LAN.
      final lanObservedInterval = sameObservedPhase && startPhase == 'lan' &&
          !gap && resets.isEmpty && lanBytes != null && lanBytes > 0 &&
          relayBytes != null;
      if (lanObservedInterval) {
        lanObserved++;
        if (relayBytes == 0) {
          lanZeroRelay++;
        } else {
          lanNonzeroRelay++;
        }
      }
      intervals.add({
        'start_elapsed_ms': before['elapsed_ms'],
        'end_elapsed_ms': after['elapsed_ms'],
        'duration_ms': elapsed,
        'long_sampling_gap': gap,
        'start_observed_transport': startPhase,
        'end_observed_transport': endPhase,
        'same_observed_transport': sameObservedPhase,
        'decreased_counters': resets,
        'counter_deltas': deltas,
        'rates_bps_or_count_per_second': rates,
        'lan_endpoint_observation_eligible': lanObservedInterval,
      });
    }
    rtts.sort();
    return {
      'interval_count': intervals.length,
      'intervals': intervals,
      'lan_endpoint_observed_intervals': lanObserved,
      'lan_endpoint_observed_zero_relay_intervals': lanZeroRelay,
      'lan_endpoint_observed_nonzero_relay_intervals': lanNonzeroRelay,
      'connected_rtt_sample_count': rtts.length,
      'connected_rtt_min_ms': rtts.isEmpty ? null : rtts.first,
      'connected_rtt_max_ms': rtts.isEmpty ? null : rtts.last,
      'connected_rtt_mean_ms': rtts.isEmpty
          ? null : rtts.fold<num>(0, (sum, value) => sum + value) / rtts.length,
    };
  }

}
