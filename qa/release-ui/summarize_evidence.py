"""Independent validation of measured Host and release UI evidence.

No network calls, simulated metrics, credentials, raw UI text or recording paths
in the output. Samples are observations, not display latency measurements.
"""
import json
import math
import sys
from pathlib import Path


def require(ok, label):
    if not ok:
        raise ValueError(label)


def integer(value, label):
    require(type(value) is int and value >= 0, "invalid integer: " + label)
    return value


def number(value, label):
    require(type(value) in (int, float) and math.isfinite(value) and value >= 0,
            "invalid number: " + label)
    return value


def counter(data, key):
    return integer(data.get(key), key)


def delta(before, after, key):
    first, last = counter(before, key), counter(after, key)
    require(last >= first, "counter reset: " + key)
    return last - first


def summarize_g1(rows):
    stages = ["start", "relay_baseline", "lan_settle", "lan_stable",
              "clipboard_ownership", "relay_resumed", "passed"]
    require([row.get("stage") for row in rows] == stages, "unexpected G1 stage sequence")
    require(all(row.get("scope") == "windows_runner_loopback_ws_jpeg" for row in rows),
            "invalid G1 scope")
    data = {row["stage"]: row["data"] for row in rows}
    ownership = data["clipboard_ownership"]
    require(ownership.get("lan_applied") is True and ownership.get("relay_blocked") is True,
            "clipboard ownership evidence incomplete")
    output = {}
    for stage, active, transport in (
        ("relay_baseline", False, "relay"), ("lan_stable", True, "lan"),
        ("relay_resumed", False, "relay"),
    ):
        phase = data[stage]
        before, after = phase["before"], phase["after"]
        require(after.get("active") is active and after.get("relay_socket_open") is True,
                "invalid Host transport state")
        counters = {key: delta(before, after, key) for key in (
            "lan_bytes", "lan_frames", "relay_bytes", "relay_frames", "suppressed",
        )}
        other = "relay" if transport == "lan" else "lan"
        require(counters[transport + "_bytes"] > 0 and counters[transport + "_frames"] > 0,
                "no measured video progress")
        require(counters[other + "_bytes"] == 0 and counters[other + "_frames"] == 0,
                "unexpected other-transport video")
        peer = phase[transport]
        require(counter(peer, "bytes") == counters[transport + "_bytes"]
                and counter(peer, "frames") == counters[transport + "_frames"]
                and peer.get("pong") is True, "peer video counters or heartbeat mismatch")
        if active:
            require(counter(phase["relay"], "bytes") == 0
                    and counter(phase["relay"], "frames") == 0
                    and counters["suppressed"] > 0, "missing relay suppression")
        require(counter(after, "relay_socket_opens") == 1, "unexpected relay reopen")
        output[stage] = counters
    final = data["passed"]["host"]
    require(final.get("active") is False and final.get("relay_socket_open") is False
            and counter(final, "relay_socket_opens") == 1, "Host socket not retired")
    return {"schema_version": 1, "scope": "Windows runner loopback WebSocket JPEG",
            "phases": output, "clipboard_ownership": {"lan_applied": True, "relay_blocked": True}}



ENCODE_STAGES = ("resize_cpu", "color_i420", "color_nv12", "codec_software", "codec_mf_cpu", "codec_mf_gpu", "codec_jpeg", "gpu_capture_convert", "encoder_init")
def summarize_encode_stages(samples):
    previous = None
    for sample in samples:
        stages = sample["timing"].get("encode_stages")
        require(isinstance(stages, dict) and set(stages) == set(ENCODE_STAGES),
                "missing or unknown encode stages")
        for key in ENCODE_STAGES:
            total = counter(stages[key], "total_us")
            count = counter(stages[key], "samples")
            require(count > 0 or total == 0, "nonzero time without stage samples")
            if previous is not None:
                dt = delta(previous[key], stages[key], "total_us")
                dc = delta(previous[key], stages[key], "samples")
                require(dc > 0 or dt == 0, "stage time increased without samples")
        previous = stages
    first = samples[0]["timing"]["encode_stages"]
    last = samples[-1]["timing"]["encode_stages"]
    result = {}
    for key in ENCODE_STAGES:
        total = delta(first[key], last[key], "total_us")
        count = delta(first[key], last[key], "samples")
        result[key] = dict(total_us=total, samples=count, mean_us=total/count if count else None)
    require(sum(result[k]["samples"] for k in ("codec_software","codec_mf_cpu","codec_mf_gpu")) > 0,
            "no actual AVC codec operations")
    return result


def summarize_g5b2(rows, require_encode_stages=False):
    """Recompute stable LAN measurements from the authenticated observer's raw
    counters. Metadata observation times and receiver windows are distinct."""
    stages = ["start", "relay_baseline", "lan_startup", "lan_stable", "relay_resumed", "passed"]
    require([row.get("stage") for row in rows] == stages, "unexpected AVC handoff stage sequence")
    require(all(row.get("scope") == "windows_runner_loopback_ws_avc_handoff" for row in rows),
            "invalid AVC handoff scope")
    data = {row["stage"]: row["data"] for row in rows}
    phase = data["lan_stable"]
    before, after = phase["before"], phase["after"]
    for snap in (before, after):
        require(snap.get("active") is True and snap.get("relay_socket_open") is True
                and counter(snap, "relay_socket_opens") == 1, "invalid stable LAN ownership")
    counts = {key: delta(before, after, key) for key in (
        "lan_bytes", "lan_frames", "relay_bytes", "relay_frames", "suppressed")}
    require(counts["relay_bytes"] == counts["relay_frames"] == 0
            and counts["suppressed"] > 0, "relay suppression evidence missing")
    lan, relay = phase["lan"], phase["relay"]
    require(all(counter(relay, key) == 0 for key in ("bytes", "packets", "jpeg_packets"))
            and relay.get("pong") is False, "relay transmitted during LAN ownership")
    packets, received_bytes, decoded = (counter(lan, k) for k in ("packets", "bytes", "decoded_vcl"))
    window = counter(lan, "elapsed_ms")
    require(window > 0 and packets >= 2 and received_bytes > 0
            and 2 <= decoded <= counter(lan, "vcl_packets") <= packets
            and 0 < counter(lan, "decoded_predictive") <= decoded
            and counter(lan, "decoded_idr") == 1 and counter(lan, "jpeg_packets") == 0
            and lan.get("pong") is True and counter(lan, "ws_pings") > 0,
            "invalid actual AVC receive evidence")
    # Host and receiver windows have independently sampled boundaries.
    # Preserve differences explicitly rather than pretending atomic equality.
    require(counts["lan_frames"] >= 2 and counts["lan_bytes"] > 0, "no Host LAN progress")
    dimensions = lan["dimensions"]
    require(isinstance(dimensions, list) and len(dimensions) == 2
            and all(integer(v, "dimension") > 0 and v % 2 == 0 for v in dimensions),
            "invalid AVC dimensions")
    samples = lan["timing_samples"]
    require(isinstance(samples, list) and len(samples) >= 2, "missing timing metadata")
    keys = ("capture", "encode", "send", "queue_wait")
    backends = ("DXGI", "screenshots compatibility")
    encoders = ("OpenH264 software", "Media Foundation hardware", "Media Foundation D3D11 texture")
    previous = None
    for sample in samples:
        observed = counter(sample, "observed_ms")
        require(observed <= window and sample.get("encoder") in encoders
                and sample.get("capture_backend") in backends, "invalid metadata observation")
        timing = sample["timing"]
        counter(timing, "queue_drops")
        for key in keys:
            counter(timing[key], "total_us")
            counter(timing[key], "samples")
        require(timing["send"]["samples"] == timing["queue_wait"]["samples"],
                "send and queue denominator mismatch")
        if previous is not None:
            require(observed > previous["observed_ms"], "metadata time not increasing")
            delta(previous["timing"], timing, "queue_drops")
            for key in keys:
                for field in ("total_us", "samples"):
                    delta(previous["timing"][key], timing[key], field)
        previous = sample
    first, last = samples[0], samples[-1]
    result = {}
    for key in keys:
        total = delta(first["timing"][key], last["timing"][key], "total_us")
        count = delta(first["timing"][key], last["timing"][key], "samples")
        require(count > 0, "no stage samples: " + key)
        result[key] = {"total_us": total, "samples": count, "mean_us": total / count}
    result.update(metadata_samples=len(samples),
                  elapsed_observation_ms=last["observed_ms"]-first["observed_ms"],
                  queue_drops=delta(first["timing"], last["timing"], "queue_drops"),
                  encoder_first=first["encoder"], encoder_last=last["encoder"],
                  capture_backend_first=first["capture_backend"], capture_backend_last=last["capture_backend"])
    if require_encode_stages or any("encode_stages" in s["timing"] for s in samples):
        stages = summarize_encode_stages(samples)
        reported_stages = phase["performance"]["session_timing"].get("encode_stages")
        require(isinstance(reported_stages, dict) and set(reported_stages) == set(stages),
                "missing reported encode stages")
        for key, expected in stages.items():
            actual = reported_stages[key]
            for field in ("total_us", "samples"):
                require(counter(actual, field) == expected[field], "encode stage count mismatch")
            if expected["mean_us"] is None:
                require(actual.get("mean_us") is None, "unobserved stage mean must be null")
            else:
                require(math.isclose(number(actual.get("mean_us"), key), expected["mean_us"],
                                     rel_tol=1e-9, abs_tol=1e-9), "encode stage mean mismatch")
    else:
        stages = None
    rates = dict(receiver_window_ms=window, received_avc_packets=packets, decoded_vcl=decoded,
                 encrypted_avc_bytes=received_bytes, received_avc_packets_per_second=packets*1000/window,
                 decoded_vcl_per_second=decoded*1000/window, encrypted_avc_bps=received_bytes*8000/window)
    def verify_number(actual, expected, label):
        if type(expected) is int:
            require(integer(actual, label) == expected, "inconsistent count: " + label)
        else:
            require(math.isclose(number(actual, label), expected, rel_tol=1e-9, abs_tol=1e-9),
                    "inconsistent rate: " + label)
    reported = phase["performance"]
    require(reported.get("scope") == "windows_runner_loopback_real_capture_session_timing",
            "invalid performance scope")
    for key, value in rates.items():
        verify_number(reported.get(key), value, key)
    reported_timing = reported["session_timing"]
    for key, value in result.items():
        if isinstance(value, dict):
            for field, expected in value.items():
                verify_number(reported_timing[key].get(field), expected, key + "." + field)
        elif isinstance(value, str):
            require(reported_timing.get(key) == value, "inconsistent backend: " + key)
        else:
            verify_number(reported_timing.get(key), value, key)
    resumed = data["relay_resumed"]
    rb, ra = resumed["before"], resumed["after"]
    require(rb.get("active") is False and ra.get("active") is False
            and rb.get("relay_socket_open") is True and ra.get("relay_socket_open") is True
            and counter(rb, "relay_socket_opens") == counter(ra, "relay_socket_opens") == 1,
            "relay socket not reused")
    require(all(delta(rb, ra, key) == 0 for key in ("lan_bytes", "lan_frames", "suppressed"))
            and delta(rb, ra, "relay_frames") >= 2 and delta(rb, ra, "relay_bytes") > 0,
            "invalid relay resume counters")
    peer = resumed["relay"]
    require(counter(peer, "packets") >= 2 and counter(peer, "bytes") > 0
            and counter(peer, "jpeg_packets") == 0 and counter(peer, "decoded_idr") == 1
            and counter(peer, "decoded_vcl") == counter(peer, "vcl_packets")
            and counter(peer, "decoded_predictive") > 0 and peer.get("pong") is True,
            "invalid resumed AVC evidence")
    final = data["passed"]["host"]
    require(final.get("active") is False and final.get("relay_socket_open") is False
            and counter(final, "relay_socket_opens") == 1, "Host socket not retired")
    if stages is not None:
        result["encode_stages"] = stages
    return {"schema_version": 2 if stages is not None else 1, "scope": "Windows runner loopback real capture; not physical display latency",
            "dimensions": dimensions, **rates, "session_timing": result, "host_delta": counts,
            "host_minus_received_bytes": counts["lan_bytes"]-received_bytes,
            "host_minus_received_packets": counts["lan_frames"]-packets,
            "relay_resumed_first_decoded_idr_observation_ms": counter(peer, "first_decoded_idr_ms")}


def live(sample):
    require(sample.get("codec") == "H264" and sample.get("transport_phase") == "relay_video"
            and sample.get("host_metadata_current") == "true", "not live authenticated AVC")
    result = {key: counter(sample, key) for key in (
        "frames", "bps", "rtt", "relay_socket_opens", "relay_retry_events", "drops",
        "host_encoded_frames",
    )}
    result["fps"] = number(sample.get("fps"), "fps")
    require(result["fps"] > 0 and result["bps"] > 0 and result["frames"] > 0,
            "no live AVC throughput")
    return result


def recording(report):
    record = report["recording"]
    frames = counter(record, "frames")
    duration = number(record.get("duration_seconds"), "recording duration")
    require(frames >= 16 and duration > 0 and counter(record, "capture_fps") == 4,
            "invalid recording evidence")
    return {"frames": frames, "duration_seconds": duration, "capture_fps": 4}


def summarize_release(report, role):
    require(report.get("passed") is True and report.get("role") == role, "release UI did not pass")
    duration = counter(report, "requested_stability_seconds")
    require(duration in (300, 1800), "unexpected stability duration")
    summary = {"schema_version": 1, "scope": "release UI on CI runners; not physical display latency",
               "role": role, "requested_stability_seconds": duration}
    if report.get("recording_enabled") is False:
        require("recording" not in report and "partial_recording_frames" not in report,
                "unexpected recording evidence when disabled")
        summary["recording_enabled"] = False
    else:
        # Backward-compatible validation of historical recordings only.
        summary["recording_enabled"] = True
        summary["recording"] = recording(report)
    if role == "windows":
        elapsed = number(report.get("peer_stability_elapsed_seconds"), "peer duration")
        count = counter(report, "peer_stability_sample_count")
        require(elapsed >= duration and count == duration // 30
                and report.get("avc_stability_text_verified") is True, "Windows peer evidence incomplete")
        summary["peer_stability"] = {"elapsed_seconds": elapsed, "sample_count": count,
                                     "remote_text_verified": True}
        return summary

    first, second = (report["actual_avc_metrics"][key] for key in ("first", "second"))
    live(first)
    live_second = live(second)
    transport = {}
    for prefix in ("viewer", "host"):
        key = prefix + "_relay_video_bytes"
        transport[key + "_delta"] = delta(first, second, key)
        require(transport[key + "_delta"] > 0, "no public video progress")
        key = prefix + "_lan_video_bytes"
        require(counter(first, key) == counter(second, key) == 0, "unexpected LAN video")
    summary["initial_public_avc"] = {**live_second, **transport}
    stable = report["avc_stability"]
    require(counter(stable, "minimum_elapsed_seconds") == duration
            and counter(stable, "expected_samples") == duration // 30, "wrong sample target")
    samples = stable["samples"]
    require(len(samples) == duration // 30 and stable.get("remote_text_verified") is True,
            "incomplete stability evidence")
    baseline = live(stable["baseline"])
    previous = baseline
    last_time = 0
    measured = []
    for index, raw in enumerate(samples, 1):
        sample = live(raw)
        elapsed = number(raw.get("elapsed_seconds"), "sample elapsed")
        require(elapsed > last_time and elapsed >= index * 30 - .001, "invalid sample timing")
        require(sample["relay_socket_opens"] == baseline["relay_socket_opens"]
                and sample["relay_retry_events"] == baseline["relay_retry_events"], "unexpected reconnect")
        require(sample["frames"] > previous["frames"] and
                sample["host_encoded_frames"] > previous["host_encoded_frames"] and
                sample["drops"] >= previous["drops"], "counter progress invalid")
        measured.append({"elapsed_seconds": elapsed, **sample})
        previous, last_time = sample, elapsed
    elapsed = number(stable.get("elapsed_seconds"), "total elapsed")
    require(elapsed >= duration and elapsed >= last_time, "short stability interval")
    frames = previous["frames"] - baseline["frames"]
    drops = previous["drops"] - baseline["drops"]
    rtts = [sample["rtt"] for sample in measured]
    for key, expected in (("rendered_frame_delta", frames), ("decoder_drop_delta", drops),
                          ("rtt_min_ms", min(rtts)), ("rtt_max_ms", max(rtts))):
        require(counter(stable, key) == expected, "inconsistent summary: " + key)
    # Original FPS uses unrounded elapsed time; the report rounds to milliseconds.
    reported_fps = number(stable.get("observed_average_render_fps"), "mean FPS")
    require(math.isclose(reported_fps, frames / elapsed, rel_tol=0.00001),
            "inconsistent mean FPS")
    summary["stability"] = {"elapsed_seconds": elapsed, "sample_count": len(measured),
                            "rendered_frame_delta": frames, "decoder_drop_delta": drops,
                            "average_output_fps": frames / elapsed,
                            "rtt_min_ms": min(rtts), "rtt_max_ms": max(rtts),
                            "remote_text_verified": True, "samples": measured}
    return summary


def main():
    role, source, destination = sys.argv[1:]
    raw = Path(source).read_text(encoding="utf-8")
    if role == "g1":
        summary = summarize_g1([json.loads(line) for line in raw.splitlines() if line.strip()])
    elif role in ("g5b2", "g6"):
        summary = summarize_g5b2([json.loads(line) for line in raw.splitlines() if line.strip()],
                                  require_encode_stages=role=="g6")
    else:
        require(role in ("windows", "android"), "invalid role")
        summary = summarize_release(json.loads(raw), role)
    encoded = json.dumps(summary, indent=2, allow_nan=False)
    Path(destination).write_text(encoded + "\n", encoding="utf-8")
    print(encoded)


if __name__ == "__main__":
    main()
