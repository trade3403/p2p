import copy
import unittest
from summarize_evidence import summarize


def reports():
    sample = dict(elapsed_ms=2000, rendered_avc_frames=20, encrypted_video_bytes=1000,
                  surface_consumed_frames=19, authenticated_rtt_ms=5, rendered_fps=10,
                  encrypted_video_bps=4000)
    metrics = {
        "system_network_outage": dict(default_network_lost=True, native_connection_retired=True,
            default_network_restored=True, new_authenticated_video=True, sustained_video=True,
            observed_network_loss_ms=1, observed_retire_ms=2,
            observed_default_network_restore_ms=3, observed_restore_to_video_ms=4,
            observed_retire_to_video_ms=5, network_change_events=2, new_relay_socket_opens=1),
        "multi_candidate_lan": dict(first_authenticated_hello=True, second_authenticated_hello=True,
            stable_second_candidate_video=True, verified_candidate_disconnect_recovers_public=True,
            candidate_attempts=2, first_video_packets=0, relay_socket_opens_during_batch=0,
            second_authenticated_pongs=1, attempt_ms=4, observed_upgrade_to_video_ms=5,
            fallback_first_video_ms=6),
        "lan_stalled_verification": dict(authenticated_hello_count=1, candidate_attempts=1,
            attempt_ms=4, relay_open_ms=1, first_video_ms=2, observed_total_ms=6),
        "lan_authenticated_heartbeat_timeout": dict(unanswered_pings=1,
            lan_video_bytes_during_silence=1, observed_silence_to_public_video_ms=4,
            observed_total_ms=5, relay_open_ms=1, first_video_ms=2),
        "lan_avc_fixture": dict(sample, relay_video_bytes=0, relay_socket_opens=0, surface_errors=0),
        "lan_avc_fallback_live_windows": dict(sample, lan_video_byte_delta=0, host_encoded_frame_delta=20),
    }
    return {"passed": True, "metrics": metrics}, {"passed": True, "native_passed": True, "metrics": metrics}


class EvidenceTests(unittest.TestCase):
    def test_valid_and_private_fields_excluded(self):
        native, job = reports()
        native["metrics"]["lan_avc_fixture"]["private"] = "PRIVATE_MARKER"
        result = summarize(native, job)
        self.assertNotIn("PRIVATE_MARKER", str(result))
        self.assertEqual(result["lan_avc_fixture_emulator_local"]["rendered_fps"], 10)

    def test_reject_rate_mismatch(self):
        native, job = reports()
        native["metrics"]["lan_avc_fixture"]["rendered_fps"] = 30
        with self.assertRaises(ValueError):
            summarize(native, job)

    def test_reject_boolean_counter_and_failed_job(self):
        native, job = reports()
        native["metrics"]["multi_candidate_lan"]["first_video_packets"] = False
        with self.assertRaises(ValueError):
            summarize(native, job)
        native, job = reports()
        job["passed"] = False
        with self.assertRaises(ValueError):
            summarize(native, job)

    def test_reject_inconsistent_report(self):
        native, job = reports()
        job = copy.deepcopy(job)
        job["metrics"]["lan_avc_fixture"]["elapsed_ms"] = 2001
        with self.assertRaises(ValueError):
            summarize(native, job)


if __name__ == "__main__":
    unittest.main()
