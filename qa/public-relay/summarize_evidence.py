"""Validate measured E2E evidence and emit a numeric-only, scoped summary."""
import json
import math
import sys
from pathlib import Path


def require(condition, label):
    if not condition:
        raise ValueError(label)


def integers(section, keys):
    result = {}
    for key in keys:
        value = section.get(key)
        require(type(value) is int and value >= 0, "invalid counter: " + key)
        result[key] = value
    return result


def sample(section):
    result = integers(section, (
        "elapsed_ms", "rendered_avc_frames", "encrypted_video_bytes",
        "surface_consumed_frames", "authenticated_rtt_ms",
    ))
    elapsed = result["elapsed_ms"]
    require(elapsed > 0, "invalid sample duration")
    for key in ("rendered_avc_frames", "encrypted_video_bytes", "surface_consumed_frames"):
        require(result[key] > 0, "no measured progress: " + key)
    for key, expected in (
        ("rendered_fps", result["rendered_avc_frames"] * 1000 / elapsed),
        ("encrypted_video_bps", result["encrypted_video_bytes"] * 8000 / elapsed),
    ):
        value = section.get(key)
        require(type(value) in (float, int) and math.isfinite(value)
                and math.isclose(value, expected, rel_tol=1e-8, abs_tol=1e-8),
                "inconsistent measured rate: " + key)
        result[key] = expected
    return result


def summarize(native, job):
    require(native.get("passed") is True and job.get("passed") is True
            and job.get("native_passed") is True, "E2E not passed")
    metrics = native["metrics"]
    require(job.get("metrics") == metrics, "native and job metrics differ")
    outage = metrics["system_network_outage"]
    require(all(outage.get(k) is True for k in (
        "default_network_lost", "native_connection_retired", "default_network_restored",
        "new_authenticated_video", "sustained_video",
    )), "missing outage evidence")
    outage_result = integers(outage, (
        "observed_network_loss_ms", "observed_retire_ms",
        "observed_default_network_restore_ms", "observed_restore_to_video_ms",
        "observed_retire_to_video_ms", "network_change_events", "new_relay_socket_opens",
    ))
    require(outage_result["new_relay_socket_opens"] > 0
            and outage_result["network_change_events"] >= 2, "no recovery progress")
    require(outage_result["observed_retire_ms"] >= outage_result["observed_network_loss_ms"]
            and outage_result["observed_restore_to_video_ms"] >=
            outage_result["observed_default_network_restore_ms"], "invalid outage timing")
    candidate = metrics["multi_candidate_lan"]
    require(all(candidate.get(k) is True for k in (
        "first_authenticated_hello", "second_authenticated_hello",
        "stable_second_candidate_video", "verified_candidate_disconnect_recovers_public",
    )), "missing candidate evidence")
    candidate_result = integers(candidate, (
        "candidate_attempts", "first_video_packets", "relay_socket_opens_during_batch",
        "second_authenticated_pongs", "attempt_ms", "observed_upgrade_to_video_ms",
        "fallback_first_video_ms",
    ))
    require(candidate_result["candidate_attempts"] == 2
            and candidate_result["first_video_packets"] == 0
            and candidate_result["relay_socket_opens_during_batch"] == 0
            and candidate_result["second_authenticated_pongs"] > 0, "invalid candidate counters")
    stalled = integers(metrics["lan_stalled_verification"], (
        "authenticated_hello_count", "candidate_attempts", "attempt_ms",
        "relay_open_ms", "first_video_ms", "observed_total_ms",
    ))
    heartbeat = integers(metrics["lan_authenticated_heartbeat_timeout"], (
        "unanswered_pings", "lan_video_bytes_during_silence",
        "observed_silence_to_public_video_ms", "observed_total_ms",
        "relay_open_ms", "first_video_ms",
    ))
    require(stalled["authenticated_hello_count"] == 1 and stalled["candidate_attempts"] == 1
            and heartbeat["unanswered_pings"] == 1
            and heartbeat["lan_video_bytes_during_silence"] > 0, "missing failure/recovery progress")
    for section in (stalled, heartbeat):
        require(section["first_video_ms"] >= section["relay_open_ms"], "invalid fallback timing")
    fixture = metrics["lan_avc_fixture"]
    fixture_result = sample(fixture)
    fixture_result.update(integers(fixture, ("relay_video_bytes", "relay_socket_opens", "surface_errors")))
    require(all(fixture_result[k] == 0 for k in (
        "relay_video_bytes", "relay_socket_opens", "surface_errors",
    )), "fixture relay or surface invariant failed")
    fallback = metrics["lan_avc_fallback_live_windows"]
    fallback_result = sample(fallback)
    fallback_result.update(integers(fallback, ("lan_video_byte_delta", "host_encoded_frame_delta")))
    require(fallback_result["lan_video_byte_delta"] == 0
            and fallback_result["host_encoded_frame_delta"] > 0, "fallback transport invariant failed")
    return {
        "schema_version": 1,
        "scope": "Android emulator and CI Windows; not physical LAN or display latency",
        "system_network_outage": outage_result,
        "multi_candidate_lan": candidate_result,
        "lan_stalled_verification": stalled,
        "lan_authenticated_heartbeat_timeout": heartbeat,
        "lan_avc_fixture_emulator_local": fixture_result,
        "lan_avc_fallback_live_windows_public": fallback_result,
    }


def main():
    native_path, job_path, output_path = map(Path, sys.argv[1:])
    summary = summarize(json.loads(native_path.read_text(encoding="utf-8")),
                        json.loads(job_path.read_text(encoding="utf-8")))
    encoded = json.dumps(summary, indent=2, allow_nan=False)
    output_path.write_text(encoded + "\n", encoding="utf-8")
    print(encoded)


if __name__ == "__main__":
    main()
