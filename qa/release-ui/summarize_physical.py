"""Offline paired performance-series validator. No device access or recording."""
import argparse
import json
import math
from datetime import datetime, timedelta
from pathlib import Path

MIN_WINDOW_MS = 10_000
MAX_GAP_MS = 4_000
MAX_SAMPLES = 300


def require(condition, label):
    if not condition:
        raise ValueError(label)


def uint(value, label):
    require(type(value) is int and value >= 0, label)
    return value


def utc(value):
    require(isinstance(value, str), "missing UTC timestamp")
    try:
        parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError:
        raise ValueError("invalid UTC timestamp") from None
    require(parsed.utcoffset() == timedelta(0), "timestamp not UTC")
    return parsed


def phase(metrics, role):
    if role == "host":
        if metrics.get("host_running") is not True:
            return "unknown"
        if metrics.get("host_lan_active") is True:
            return "lan"
        if (metrics.get("host_lan_active") is False
                and metrics.get("host_relay_socket_open") is True):
            return "relay"
    elif metrics.get("viewer_connected") is True:
        return {"lan_video": "lan", "relay_video": "relay"}.get(
            metrics.get("viewer_transport_phase"), "unknown")
    return "unknown"


def analyze(series, role, transport):
    require(role in ("host", "viewer") and transport in ("lan", "relay"), "invalid role or transport")
    require(isinstance(series, dict) and type(series.get("schema_version")) is int
            and series["schema_version"] == 1
            and series.get("sample_kind") == role + "_series", "wrong series schema or role")
    started = utc(series.get("started_at_utc"))
    require(type(series.get("nominal_interval_ms")) is int
            and series["nominal_interval_ms"] == 2000
            and type(series.get("sample_limit")) is int
            and series["sample_limit"] == MAX_SAMPLES, "unexpected sampling configuration")
    require(series.get("stop_reason") in ("user", "manual", "sample_limit", "user_stop"),
            "series not explicitly completed")
    samples = series.get("samples")
    require(isinstance(samples, list) and 2 <= len(samples) <= MAX_SAMPLES, "invalid sample count")
    counters = [f"{role}_{path}_video_{unit}" for path in ("lan", "relay")
                for unit in ("bytes", "frames")]
    sockets = "host_relay_socket_opens" if role == "host" else "relay_socket_opens"
    counters.append(sockets)
    if role == "viewer":
        counters.extend(["avc_rendered_frames", "avc_decoder_drops", "relay_retry_events"])
    else:
        counters.append("host_relay_suppressed_frames")
    points = []
    rtts = []
    for sample in samples:
        require(isinstance(sample, dict) and type(sample.get("schema_version")) is int
                and sample["schema_version"] == 1
                and sample.get("sample_kind") == role + "_snapshot", "invalid sample schema")
        elapsed = uint(sample.get("elapsed_ms"), "invalid elapsed time")
        stamp = utc(sample.get("captured_at_utc"))
        require(stamp >= started, "sample UTC before start")
        m = sample.get("metrics")
        require(isinstance(m, dict) and phase(m, role) == transport, "unexpected observed transport")
        if role == "viewer":
            require(m.get("active_video_codec") == "H264", "viewer not reporting active AVC")
            require(m.get("avc_decoder_recovering") is False, "decoder recovery or missing state")
            rtt = m.get("rtt_ms")
            require(type(rtt) in (int, float) and math.isfinite(rtt) and rtt >= 0, "invalid RTT")
            rtts.append(rtt)
        values = {key: uint(m.get(key), "missing or invalid counter: " + key) for key in counters}
        points.append((elapsed, stamp, values))
    active = f"{role}_{transport}_video_"
    other = f"{role}_{'relay' if transport == 'lan' else 'lan'}_video_"
    totals = {key: 0 for key in counters}
    for (a_ms, a_utc, a), (b_ms, b_utc, b) in zip(points, points[1:]):
        require(0 < b_ms - a_ms <= MAX_GAP_MS, "nonmonotonic sample or long sampling gap")
        require(b_utc >= a_utc, "wall clock moved backwards")
        delta = {key: b[key] - a[key] for key in counters}
        require(all(v >= 0 for v in delta.values()), "counter reset")
        require(delta[active + "bytes"] > 0 and delta[active + "frames"] > 0,
                "no active video progress")
        require(delta[other + "bytes"] == delta[other + "frames"] == 0,
                "inactive transport carried video")
        require(delta[sockets] == 0, "relay socket opened during stable window")
        if role == "viewer":
            require(delta["avc_rendered_frames"] > 0, "no AVC decoder output")
            require(delta["relay_retry_events"] == 0, "relay retry during stable window")
        for key, value in delta.items():
            totals[key] += value
    duration = points[-1][0] - points[0][0]
    require(duration >= MIN_WINDOW_MS, "stable observation shorter than ten seconds")
    result = {
        "sample_count": len(points),
        "duration_ms": duration,
        "counter_deltas": totals,
        "active_encrypted_video_bps": totals[active + "bytes"] * 8000 / duration,
        "active_video_packets_per_second": totals[active + "frames"] * 1000 / duration,
        "max_sample_gap_ms": max(b[0] - a[0] for a, b in zip(points, points[1:])),
    }
    if role == "viewer":
        result.update(avc_output_per_second=totals["avc_rendered_frames"] * 1000 / duration,
                      rtt_observation_min_ms=min(rtts), rtt_observation_max_ms=max(rtts),
                      rtt_observation_mean_ms=sum(rtts) / len(rtts))
    return result, points[0][1], points[-1][1]


def summarize(host, viewer, transport):
    h, hs, he = analyze(host, "host", transport)
    v, vs, ve = analyze(viewer, "viewer", transport)
    # UTC is only a rough correlation aid; device clocks are not synchronized here.
    overlap = max(0, int((min(he, ve) - max(hs, vs)).total_seconds() * 1000))
    return {
        "schema_version": 1,
        "scope": "imported diagnostic series; device provenance and same session not independently verified",
        "transport": transport,
        "stable_observation_checks_passed": True,
        "physical_device_acceptance_verified": False,
        "recording_enabled": False,
        "utc_overlap_hint_ms": overlap,
        "host": h,
        "viewer": v,
        "limitations": [
            "Endpoint observations cannot exclude transitions between samples.",
            "Host and viewer windows are independent; UTC overlap does not establish synchronization or session identity.",
            "Video counters are application transport counters, not NIC traffic; packet rate and decoder output are not display FPS.",
            "RTT is not display latency; CPU/GPU/RAM and hardware provenance are not validated by this schema.",
        ],
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", required=True, type=Path)
    parser.add_argument("--viewer", required=True, type=Path)
    parser.add_argument("--transport", required=True, choices=("lan", "relay"))
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    if args.output.resolve() in (args.host.resolve(), args.viewer.resolve()):
        parser.error("output must not overwrite either input")
    try:
        result = summarize(json.loads(args.host.read_text(encoding="utf-8-sig")),
                           json.loads(args.viewer.read_text(encoding="utf-8-sig")), args.transport)
    except (ValueError, TypeError, OSError, OverflowError):
        # Do not print raw report values or paths. No partial success report.
        parser.exit(1, "Invalid or incomplete paired diagnostic evidence.\n")
    args.output.write_text(json.dumps(result, indent=2, allow_nan=False) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
