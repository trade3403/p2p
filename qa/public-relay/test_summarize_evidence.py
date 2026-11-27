import copy
import unittest
from summarize_evidence import summarize, resolution_observation


# Real historical Android artifact 15236289034, run 46080387539.
# The old schema has reported deltas only, not exact raw boundaries.
def historical_switches():
    return [dict(width=w, height=h, predictive_only_packets=5,
        predictive_only_render_delta=0, predictive_only_connected=False,
        predictive_only_h264_active=False, predictive_only_codec="none",
        predictive_only_phase="lan_verified", predictive_only_received_avc_bytes=b,
        recovered_codec="H264", recovered_phase="lan_video", render_delta=r,
        surface_delta=r, recovery_ms=t, same_socket=True, relay_socket_opens=0,
        relay_video_bytes=0, surface_errors=0)
        for w,h,b,r,t in ((640,360,51432,4,402),(320,240,23592,3,404))]


def synthetic_switches():
    # Constructed counter-consistency unit-test inputs, NOT measured raw evidence.
    switches = historical_switches()
    previous = (320,240)
    for s in switches:
        target = (s["width"],s["height"])
        def point(size, playing, rendered, received):
            return dict(width=size[0],height=size[1],connected=playing,h264_active=playing,
                codec="H264" if playing else "none", phase="lan_video" if playing else "lan_verified",
                rendered=rendered,received_avc_bytes=received,lan_video_bytes=received,
                relay_socket_opens=0,relay_video_bytes=0)
        end_bytes=1000+s["predictive_only_received_avc_bytes"]
        s["raw"] = dict(before=point(previous,True,10,900),
            blocked_start=dict(point(target,False,10,1000),fixture_sent=10),
            blocked_end=dict(point(target,False,10,end_bytes),fixture_sent=15,authenticated_hellos=1),
            recovered=dict(point(target,True,10+s["render_delta"],end_bytes+1000),
                surface_frames=20+s["surface_delta"],authenticated_hellos=1,surface_errors=0,
                observed_ms=100+s["recovery_ms"]),
            initial_authenticated_hellos=1,resume_surface_frames=20,resume_ms=100)
        previous=target
    return switches


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
        "lan_avc_resolution_switches": synthetic_switches(),
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

    def test_historical_observations_and_missing_raw_gate(self):
        legacy=historical_switches()
        self.assertEqual(resolution_observation(legacy[0],640,360)["recovery_ms"],402)
        self.assertEqual(resolution_observation(legacy[1],320,240)["predictive_only_received_avc_bytes"],23592)
        native,job=reports()
        native["metrics"]["lan_avc_resolution_switches"]=legacy
        with self.assertRaisesRegex(ValueError,"missing raw"):
            summarize(native,job)

    def test_raw_resolution_cross_checks(self):
        for point,key,value in (
            ("blocked_start","fixture_sent",11),
            ("blocked_end","rendered",11),
            ("blocked_end","received_avc_bytes",999),
            ("blocked_end","connected",True),
            ("blocked_end","relay_video_bytes",1),
            ("recovered","authenticated_hellos",2),
            ("recovered","surface_errors",1),
            ("recovered","observed_ms",99),
            ("recovered","surface_frames",20),
            ("recovered","width",320),
        ):
            with self.subTest(point=point,key=key):
                native,job=reports()
                native["metrics"]["lan_avc_resolution_switches"][0]["raw"][point][key]=value
                with self.assertRaises(ValueError):
                    summarize(native,job)

    def test_reject_missing_or_reordered_switches(self):
        for change in (lambda s: s.pop(),lambda s: s.reverse()):
            native,job=reports()
            change(native["metrics"]["lan_avc_resolution_switches"])
            with self.assertRaises(ValueError):
                summarize(native,job)

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
