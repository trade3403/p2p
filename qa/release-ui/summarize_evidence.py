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
    stages = ["start", "relay_baseline", "lan_settle", "lan_stable", "relay_resumed", "passed"]
    require([row.get("stage") for row in rows] == stages, "unexpected G1 stage sequence")
    require(all(row.get("scope") == "windows_runner_loopback_ws_jpeg" for row in rows),
            "invalid G1 scope")
    data = {row["stage"]: row["data"] for row in rows}
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
            "phases": output}


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
               "role": role, "recording": recording(report), "requested_stability_seconds": duration}
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
    else:
        require(role in ("windows", "android"), "invalid role")
        summary = summarize_release(json.loads(raw), role)
    encoded = json.dumps(summary, indent=2, allow_nan=False)
    Path(destination).write_text(encoded + "\n", encoding="utf-8")
    print(encoded)


if __name__ == "__main__":
    main()
